//! Maintenance table 3, cell C6 of `plan/phase8/bench.md` (Fable M2): the held tail starves. One lifted body, `held_tail_cell`, runs on the USRP
//! (`hardware.rs`, `hw_mt3_c6_held_tail_starves`) and on `FakeDevice` (`fake.rs`, the rehearsal); the verdict is the pure `held_tail_verdict`.
//!
//! The Module holds the last sample of a burst back and closes the burst with two samples (the held tail and a zero, `src/provider/tx.rs` `close`; the
//! raw helper `raw::send_burst` documents the same shape), timed at the tail's own instant. If the host is late with that close, the device runs dry
//! first. The rule under test (INFERRED from the FPGA source, `radio_tx_core.v:359-370, 399-410`): the radio reports an Underflow when it runs dry, then
//! drops the late close with a TimeError, and a burst that never got its EOB has no BurstAck. The cell replaces host load by a deterministic hold: the
//! close is handed only once the device time is 1 ms past its instant, which is what a late host looks like to the device.

#![allow(dead_code)]

use ezsdr_radio_uhd::{Device, Iq, TxCode, TxReport};
use serde_json::{Value as Json, json};

use super::raw::{TICKS_PER_MS, align_to};
use super::tone::Verdict;
use super::{MCR, mt3};

/// The rates cell C6 runs at.
pub const C6_RATES: [f64; 2] = [1e6, 10e6];
/// Trials per rate on the bench.
pub const C6_TRIALS: usize = 10;
/// How long past its own instant the close is held before it is handed: the device is then 1 ms dry, as a late host leaves it.
const HOLD_TICKS: i64 = TICKS_PER_MS;
/// How long before its own time the head's last send must have been handed: a host stall on the head starves it early, which is not the held tail.
const HEAD_MARGIN_TICKS: i64 = TICKS_PER_MS;

/// Everything `held_tail_verdict` reads of one trial. Ticks are device ticks.
#[derive(Clone, Debug)]
pub struct TailEvidence {
    /// The burst's first sample's time.
    pub first: i64,
    /// The head's last sample's instant: the burst's `S - 1` samples are the head.
    pub head_last: i64,
    /// Root ticks of one send (the chunk).
    pub chunk_ticks: i64,
    /// The close's own time, the tail's instant (`first + (S - 1)` samples).
    pub close_at: i64,
    /// The head's last send: the device time read when it had been handed, and its own first sample's time (`None`: the head was not handed).
    pub head_last_send: Option<(i64, i64)>,
    /// The device time read just before the close was handed (`None`: it was not handed).
    pub close_read: Option<i64>,
    /// Failed device calls (a send, a time read, a report read).
    pub errors: Vec<String>,
    /// The transmit channel's reports, in order.
    pub reports: Vec<TxReport>,
}

fn is_underflow(code: TxCode) -> bool {
    matches!(code, TxCode::Underflow | TxCode::UnderflowInPacket)
}

/// A TimeError of this burst: one at or after its first sample (an earlier one is an earlier trial's, like an ACK before it).
fn burst_time_error(report: &TxReport, first: i64) -> bool {
    report.code == TxCode::TimeError && report.tick.is_some_and(|tick| tick >= first)
}

/// The verdict of one trial. FAIL is positive evidence: a BurstAck at or after the burst's first sample (the close was dropped, so none is due), or, in report
/// order, a TimeError before the first Underflow, or a TimeError whose tick is before the first Underflow's tick (the device must run dry before it can drop the
/// close). INCONCLUSIVE when the order cannot be read: a failed call, an Underflow or a TimeError missing (both are required, with ticks), the close not handed 1 ms
/// after its instant, the head's last send not handed 1 ms before its own time, a sequence error report (tickless included), a leftover Underflow does not count (only one at or after the burst's first sample), or the first Underflow earlier than the head's last sample instant less one chunk (the head starved early: a host stall, not the held tail).
/// PASS with an Underflow at the head's end, a TimeError after it in order and tick, and no BurstAck.
pub fn held_tail_verdict(e: &TailEvidence) -> Verdict {
    let acks: Vec<i64> = e.reports.iter().filter(|r| r.code == TxCode::BurstAck).filter_map(|r| r.tick).filter(|tick| *tick >= e.first).collect();
    if !acks.is_empty() {
        return Verdict::Fail(format!("a BurstAck at tick(s) {acks:?}: the close was dropped, so the burst has no end to acknowledge"));
    }
    let underflow = e.reports.iter().position(|r| is_underflow(r.code) && r.tick.is_none_or(|tick| tick >= e.first));
    if let Some(at) = underflow {
        if e.reports[..at].iter().any(|r| burst_time_error(r, e.first)) {
            return Verdict::Fail("a TimeError came before the first Underflow in report order: the device dropped the close before it ran dry".to_owned());
        }
        if let Some(tick) = e.reports[at].tick {
            if e.reports.iter().any(|r| burst_time_error(r, e.first) && r.tick.is_some_and(|late| late < tick)) {
                return Verdict::Fail(format!("a TimeError's tick is before the first Underflow's tick {tick}"));
            }
        }
    }
    let mut reasons: Vec<String> = e.errors.iter().map(|error| format!("a device call failed: {error}")).collect();
    if e.reports.iter().any(|r| matches!(r.code, TxCode::SeqError | TxCode::SeqErrorInBurst)) {
        reasons.push("a sequence error (UHD reports a lost transport packet without a tick): the head may have starved from it, not from the held tail".to_owned());
    }
    match underflow.map(|at| e.reports[at].tick) {
        None => reasons.push("no Underflow: the device did not report running dry".to_owned()),
        Some(None) => reasons.push("the Underflow has no tick".to_owned()),
        Some(Some(tick)) if tick < e.head_last - e.chunk_ticks => {
            reasons.push(format!("the head starved early: the Underflow's tick {tick} is more than a chunk before the head's last sample {}", e.head_last))
        }
        Some(Some(_)) => {}
    }
    if !e.reports.iter().any(|r| burst_time_error(r, e.first)) {
        reasons.push("no TimeError: the late close was not reported dropped".to_owned());
    }
    if e.reports.iter().any(|r| r.tick.is_none() && (r.code == TxCode::TimeError || r.code == TxCode::BurstAck)) {
        reasons.push("a report without a tick cannot be ordered".to_owned());
    }
    match e.head_last_send {
        Some((handed, own)) if handed + HEAD_MARGIN_TICKS <= own => {}
        other => reasons.push(format!("the head's last send was not handed 1 ms before its own time {other:?} (handed, own): a host stall on the head is not the held tail")),
    }
    match e.close_read {
        Some(read) if read >= e.close_at + HOLD_TICKS => {}
        other => reasons.push(format!("the close was not handed 1 ms after its instant {} (device time read before it: {other:?})", e.close_at)),
    }
    if reasons.is_empty() { Verdict::Pass("an Underflow at the head's end, then a TimeError, no BurstAck".to_owned()) } else { Verdict::Inconclusive(reasons.join("; ")) }
}

/// One trial at `rate` with the burst `wave` (`S` samples): the head, its first `S - 1` samples in the Module's chunks on time (SOB on the first, no EOB), then
/// the Module's close of two samples (the held tail and a zero, SOB false, EOB true) timed at the tail's instant and handed once the device time is 1 ms past
/// it. Every report is read at the end.
fn held_tail_trial(device: &dyn Device, rate: f64, wave: &[Iq], chunk: usize, rate_errors: &[String]) -> TailEvidence {
    let per = MCR / rate as i64;
    let d0 = device.time_now().expect("the device's time");
    let first = align_to(d0 + 60 * TICKS_PER_MS, per);
    let (head_last, close_at) = (first + (wave.len() as i64 - 2) * per, first + (wave.len() as i64 - 1) * per);
    let mut errors = rate_errors.to_vec();
    let head = mt3::Plan { first_at: first, per, chunk, head: chunk, eob: false, each: false, first: false, channels: 1 };
    let (mut head_last_send, mut close_read) = (None, None);
    match mt3::send_chunked(device, &wave[..wave.len() - 1], head) {
        Err(error) => errors.push(format!("the head: {error}")),
        Ok(sent) => match mt3::wait_until(device, close_at + HOLD_TICKS) {
            Err(error) => errors.push(format!("the hold before the close: {error}")),
            Ok(read) => {
                head_last_send = sent.last().map(|last| (last.handed, last.at));
                close_read = Some(read);
                if let Err(error) = device.tx_send(0, &[&[wave[wave.len() - 1], [0.0, 0.0]]], Some(close_at), false, true, std::time::Duration::from_millis(500)) {
                    errors.push(format!("the close: {}", error.message));
                }
            }
        },
    }
    let reports = mt3::read_reports(device, &mut errors).into_iter().filter(|r| r.channel == 0).collect();
    TailEvidence { first, head_last, chunk_ticks: chunk as i64 * per, close_at, head_last_send, close_read, errors, reports }
}

/// One trial of the cell: its rate, its number and what it read.
pub struct TailTrial {
    pub rate: f64,
    pub trial: usize,
    pub evidence: TailEvidence,
}

impl TailTrial {
    /// The trial as the bench logs it: the times and every report in order, so the verdict can be read back from the log.
    pub fn trace(&self) -> Json {
        let e = &self.evidence;
        json!({
            "rate_hz": self.rate, "trial": self.trial, "first": e.first, "head_last": e.head_last, "close_at": e.close_at, "head_last_handed_and_own_time": e.head_last_send, "close_read_before_the_send": e.close_read,
            "chunk_ticks": e.chunk_ticks, "errors": e.errors, "reports": e.reports.iter().map(|r| json!([format!("{:?}", r.code), r.tick])).collect::<Vec<_>>(),
        })
    }
}

/// The cell at `rate` on `device`, which opens its transmit streamer (1 GHz, 0 dB, channel 0: `raw_tx_on` refuses anything outside bench.md's RF envelope; a
/// rate other than the requested one makes every trial INCONCLUSIVE) and closes it again. No receive. The caller is the only driver of the device's clock.
pub fn held_tail_cell(device: &dyn Device, rate: f64, trials: usize) -> Vec<TailTrial> {
    let _driver = device.host().drive();
    let errors = mt3::open_tx_checked(device, rate);
    let chunk = mt3::chunk_at(rate);
    let wave = mt3::tone_burst(rate, mt3::burst_samples(rate));
    let out = (0..trials).map(|trial| TailTrial { rate, trial, evidence: held_tail_trial(device, rate, &wave, chunk, &errors) }).collect();
    device.tx_close(0);
    out
}

/// The cell's verdict: PASS only if every trial passes, FAIL if any fails, else INCONCLUSIVE; no trial is INCONCLUSIVE.
pub fn tail_cell_verdict(trials: &[TailTrial]) -> Verdict {
    mt3::cell_of(trials.iter().map(|trial| held_tail_verdict(&trial.evidence)).collect())
}

/// Runs the cell and prints its lines (`MT3 C6 trace`, `VERDICT mt3 c6` a trial, `MT3 C6 cell`); `Some(text)` when the cell is not PASS.
pub fn report_tail_cell(device: &dyn Device, rate: f64, trials: usize) -> Option<String> {
    let trials = held_tail_cell(device, rate, trials);
    for trial in &trials {
        println!("MT3 C6 trace {}", trial.trace());
        println!("VERDICT mt3 c6 {} MS/s trial {}: {:?}", rate / 1e6, trial.trial, held_tail_verdict(&trial.evidence));
    }
    let verdict = tail_cell_verdict(&trials);
    println!("MT3 C6 cell {} MS/s: {verdict:?}", rate / 1e6);
    (!matches!(verdict, Verdict::Pass(_))).then(|| format!("{} MS/s: {verdict:?}", rate / 1e6))
}

#[cfg(test)]
mod tests {
    use super::super::mt3::tests::report;
    use super::*;

    const PER: i64 = 200;
    const CHUNK: i64 = 500_000;
    const FIRST: i64 = 12_000_000;
    /// 20 000 samples at 1 MS/s: the head's last sample is the 19 999th, the close's instant the 20 000th's.
    const HEAD_LAST: i64 = FIRST + 19_998 * PER;
    const CLOSE_AT: i64 = FIRST + 19_999 * PER;

    /// An Underflow where the head ran dry, a TimeError 1.2 ms after it, the close handed 1 ms after its instant.
    fn passing() -> TailEvidence {
        TailEvidence {
            first: FIRST,
            head_last: HEAD_LAST,
            chunk_ticks: CHUNK,
            close_at: CLOSE_AT,
            head_last_send: Some((HEAD_LAST - 10 * TICKS_PER_MS, HEAD_LAST)),
            close_read: Some(CLOSE_AT + HOLD_TICKS),
            errors: Vec::new(),
            reports: vec![report(TxCode::Underflow, Some(CLOSE_AT)), report(TxCode::TimeError, Some(CLOSE_AT + 240_000))],
        }
    }

    fn assert_kind(e: &TailEvidence, want: &str, what: &str) {
        held_tail_verdict(e).assert_kind(want, what);
    }

    #[test]
    fn mt3_c6_verdict_passes_an_underflow_then_a_time_error_and_no_ack() {
        assert_kind(&passing(), "PASS", "Underflow, TimeError after it, no BurstAck");
        let mut in_packet = passing();
        in_packet.reports[0] = report(TxCode::UnderflowInPacket, Some(CLOSE_AT));
        assert_kind(&in_packet, "PASS", "an UnderflowInPacket is the device running dry too");
        let mut several = passing();
        several.reports.push(report(TxCode::TimeError, Some(CLOSE_AT + 480_000)));
        assert_kind(&several, "PASS", "more TimeErrors after the Underflow (R4's discard of the rest of a packet)");
        let mut same_tick = passing();
        same_tick.reports[1] = report(TxCode::TimeError, Some(CLOSE_AT));
        assert_kind(&same_tick, "PASS", "a TimeError at the Underflow's own tick is not before it");
        let mut earlier_ack = passing();
        earlier_ack.reports.push(report(TxCode::BurstAck, Some(FIRST - 1)));
        assert_kind(&earlier_ack, "PASS", "a BurstAck before the burst's first sample is an earlier burst's");
    }

    #[test]
    fn mt3_c6_verdict_a_time_error_before_the_burst_is_an_earlier_trials() {
        // Like an ACK before the burst's first sample, a TimeError before it is not this burst's: it neither fails the order nor counts as the one that is required.
        let mut leftover = passing();
        leftover.reports.insert(0, report(TxCode::TimeError, Some(FIRST - 1)));
        assert_kind(&leftover, "PASS", "a leftover TimeError ahead of the Underflow does not fail the order");
        let mut tick_before = passing();
        tick_before.reports.push(report(TxCode::TimeError, Some(FIRST - 1)));
        assert_kind(&tick_before, "PASS", "a leftover TimeError whose tick is before the Underflow's does not fail");
        let mut only_leftover = passing();
        only_leftover.reports[1] = report(TxCode::TimeError, Some(FIRST - 1));
        assert_kind(&only_leftover, "INCONCLUSIVE", "the only TimeError is an earlier trial's: this burst's close was not reported dropped");
        let mut at_first = passing();
        at_first.reports[1] = report(TxCode::TimeError, Some(FIRST));
        assert_kind(&at_first, "FAIL", "a TimeError at the burst's first sample is the burst's, and before the Underflow's tick");
    }

    #[test]
    fn mt3_c6_verdict_a_sequence_error_is_inconclusive() {
        // A lost transport packet starves the head with a SeqError (no tick) and then an Underflow and TimeErrors that look like the held tail's.
        for code in [TxCode::SeqError, TxCode::SeqErrorInBurst] {
            for tick in [None, Some(CLOSE_AT - 1_000)] {
                let mut e = passing();
                e.reports.insert(0, report(code, tick));
                assert_kind(&e, "INCONCLUSIVE", &format!("{code:?} with tick {tick:?} ahead of the Underflow"));
            }
        }
        let mut after = passing();
        after.reports.push(report(TxCode::SeqError, None));
        assert_kind(&after, "INCONCLUSIVE", "a sequence error after the TimeError");
        let mut ack = passing();
        ack.reports.push(report(TxCode::BurstAck, Some(CLOSE_AT + 1)));
        ack.reports.push(report(TxCode::SeqError, None));
        assert_kind(&ack, "FAIL", "a BurstAck is positive evidence whatever else came");
    }

    #[test]
    fn mt3_c6_verdict_an_underflow_before_the_burst_is_an_earlier_trials() {
        let mut leftover = passing();
        leftover.reports.insert(0, report(TxCode::Underflow, Some(FIRST - 1)));
        assert_kind(&leftover, "PASS", "a leftover Underflow ahead of the burst's own does not take the first Underflow's place");
        let mut only = passing();
        only.reports[0] = report(TxCode::Underflow, Some(FIRST - 1));
        assert_kind(&only, "INCONCLUSIVE", "the only Underflow is an earlier trial's: this burst's head was not shown running dry");
        let mut tickless = passing();
        tickless.reports[0] = report(TxCode::Underflow, None);
        assert_kind(&tickless, "INCONCLUSIVE", "a tickless Underflow is still found, and has no tick");
    }

    #[test]
    fn mt3_c6_verdict_a_head_that_was_handed_late_is_inconclusive() {
        let own = HEAD_LAST;
        let mut short = passing();
        short.head_last_send = Some((own - TICKS_PER_MS + 1, own));
        assert_kind(&short, "INCONCLUSIVE", "the head's last send handed 1 tick under 1 ms before its own time: a host stall, not the held tail");
        let mut edge = passing();
        edge.head_last_send = Some((own - TICKS_PER_MS, own));
        assert_kind(&edge, "PASS", "handed exactly 1 ms before");
        let mut none = passing();
        none.head_last_send = None;
        assert_kind(&none, "INCONCLUSIVE", "the head was not handed");
    }

    #[test]
    fn mt3_c6_verdict_a_burst_ack_fails() {
        let mut ack = passing();
        ack.reports.push(report(TxCode::BurstAck, Some(CLOSE_AT + 1)));
        assert_kind(&ack, "FAIL", "a BurstAck after the Underflow and the TimeError");
        let mut at_first = passing();
        at_first.reports.push(report(TxCode::BurstAck, Some(FIRST)));
        assert_kind(&at_first, "FAIL", "a BurstAck at the burst's first sample");
        let mut incomplete = ack.clone();
        incomplete.close_read = None;
        assert_kind(&incomplete, "FAIL", "positive evidence whatever the hold");
        let mut tickless = passing();
        tickless.reports.push(report(TxCode::BurstAck, None));
        assert_kind(&tickless, "INCONCLUSIVE", "a BurstAck without a tick cannot be ordered");
    }

    #[test]
    fn mt3_c6_verdict_a_time_error_before_the_underflow_fails() {
        let mut order = passing();
        order.reports.swap(0, 1);
        assert_kind(&order, "FAIL", "the TimeError comes first in report order");
        let mut tick = passing();
        tick.reports[1] = report(TxCode::TimeError, Some(CLOSE_AT - 1));
        assert_kind(&tick, "FAIL", "the TimeError comes second but its tick is before the Underflow's");
        let mut ahead = passing();
        ahead.reports = vec![report(TxCode::TimeError, Some(CLOSE_AT + 240_000)), report(TxCode::UnderflowInPacket, Some(CLOSE_AT))];
        assert_kind(&ahead, "FAIL", "an UnderflowInPacket after the TimeError");
    }

    #[test]
    fn mt3_c6_verdict_needs_both_an_underflow_and_a_time_error() {
        let mut no_underflow = passing();
        no_underflow.reports.remove(0);
        assert_kind(&no_underflow, "INCONCLUSIVE", "a TimeError alone: the device was not shown running dry");
        let mut no_time_error = passing();
        no_time_error.reports.remove(1);
        assert_kind(&no_time_error, "INCONCLUSIVE", "an Underflow alone: the vacuous order proves nothing");
        let mut none = passing();
        none.reports.clear();
        assert_kind(&none, "INCONCLUSIVE", "no report at all");
        let mut tickless_underflow = passing();
        tickless_underflow.reports[0] = report(TxCode::Underflow, None);
        assert_kind(&tickless_underflow, "INCONCLUSIVE", "an Underflow without a tick");
        let mut tickless_time_error = passing();
        tickless_time_error.reports[1] = report(TxCode::TimeError, None);
        assert_kind(&tickless_time_error, "INCONCLUSIVE", "a TimeError without a tick");
    }

    #[test]
    fn mt3_c6_verdict_a_close_that_was_not_held_or_a_head_that_starved_early_is_inconclusive() {
        let mut short = passing();
        short.close_read = Some(CLOSE_AT + HOLD_TICKS - 1);
        assert_kind(&short, "INCONCLUSIVE", "the close was handed 1 tick short of 1 ms after its instant");
        let mut not_handed = passing();
        not_handed.close_read = None;
        assert_kind(&not_handed, "INCONCLUSIVE", "the close was not handed");
        let mut early = passing();
        early.reports[0] = report(TxCode::Underflow, Some(HEAD_LAST - CHUNK - 1));
        early.reports[1] = report(TxCode::TimeError, Some(HEAD_LAST));
        assert_kind(&early, "INCONCLUSIVE", "the Underflow is more than a chunk before the head's last sample: the head starved early");
        let mut edge = passing();
        edge.reports[0] = report(TxCode::Underflow, Some(HEAD_LAST - CHUNK));
        edge.reports[1] = report(TxCode::TimeError, Some(HEAD_LAST));
        assert_kind(&edge, "PASS", "an Underflow exactly one chunk before the head's last sample");
        let mut failed = passing();
        failed.errors = vec!["tx_async: the stream is not open".to_owned()];
        assert_kind(&failed, "INCONCLUSIVE", "a failed call");
    }
}
