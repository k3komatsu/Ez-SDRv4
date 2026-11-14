//! Maintenance table 3, sub-step C: the chunked late burst, cell C1 of `plan/phase8/bench.md` (AGENTS §9: a hardware test whose expectation comes from
//! a rule has a rehearsal on the FakeDevice). One lifted body, `late_chunked_cell`, runs on the USRP (`hardware.rs`, `hw_mt3_c1_chunked_late_burst`) and
//! on `FakeDevice` (`fake.rs`, the rehearsal); the verdict is the pure `late_chunked_verdict`, so the evidence it needs is a plain value that its unit
//! tests build by hand.
//!
//! The rule under test (UR-33, spec 18): the X310's radio drops a late packet at one sample per tick and the DUC carries a burst's first time to its end,
//! so a device burst whose first timed packet arrives late is dropped whole, through its EOB, with no BurstAck (VERIFIED in the FPGA source, and on the
//! bench for one 20 000-sample send at 1 MS/s; the Module's chunked shape, other rates and board #2 are INFERRED until this cell runs).
//!
//! A trial sends an on-time control burst of the same chunked shape and hears it (the receiver and the loopback work, and the tone is the one the
//! late burst would carry), then the late burst, whose first sample's time is `LATE_MS` before the host's reference instant (the host sends the first chunk at or
//! after it, so the first send is at least that late). The late burst is heard only if some of it played. Nothing heard proves nothing without the control,
//! complete receive coverage of the late burst's interval, and a last chunk handed at least `LAST_CHUNK_MARGIN_TICKS` (1 ms) before its own time (a device that dropped
//! each late packet alone would play it), so the verdict is INCONCLUSIVE without them. The control's reports are read after the late burst, so nothing
//! delays the late send; reports are attributed by their ticks.

#![allow(dead_code)]

use std::time::{Duration as Wall, Instant};

use ezsdr_radio::segment::send_cap;
use ezsdr_radio::x3xx::PACKETS;
use ezsdr_radio_uhd::{Device, Iq, RxRecv, TxCode, TxReport};
use serde_json::{Value as Json, json};

use super::observations::HeardInterval;
use super::raw::{Read, TICKS_PER_MS, align_to, drain_without_clock, raw_rx, raw_tx, read_with, tone_wave};
use super::tone::{QUIET_CHECK_MIN_POWER, Verdict, contiguous_windows_at, mean_power, window_tone_at};
use super::{MCR, iii};

/// The rates cell C1 runs at (bench.md: C2 adds 100 and 200 MS/s and board #2).
pub const C1_RATES: [f64; 2] = [1e6, 10e6];
/// Trials per rate on the bench.
pub const C1_TRIALS: usize = 10;
/// How long before the host's reference instant the late burst's first sample is timed: the host sends the first chunk at or after that instant, so the
/// first send is at least this late (the verdict checks what matters, that the last chunk was handed well before its own time: `Evidence::late_last`).
pub const LATE_MS: i64 = 1;
/// The transmit tone's offset from the carrier at every rate: the 100 kHz the other bench tools use, so the carrier plus the tone stays inside `SAFE_TX_HZ`.
const TONE_HZ: f64 = 100e3;
/// How long before its own time the late burst's last chunk must have been handed: 1 ms. The check measures when the host's send returned, not when the
/// packet reached the radio, so the margin covers the host-to-radio delay and the read's own latency (INFERRED to be far under it).
const LAST_CHUNK_MARGIN_TICKS: i64 = TICKS_PER_MS;
/// Sends per burst: the Module's chunks (`segment::send_cap`) of a 20 ms burst at 1 MS/s.
pub const CHUNKS: usize = 8;
/// The Module's in-flight window (`x3xx` timing, 10 ms): a chunk is sent once it is inside it, as the Module does.
const WINDOW_TICKS: i64 = 10 * TICKS_PER_MS;
/// The samples of a tone window (the probes' window, `tone.rs`).
const WINDOW: usize = 25;
const TX_TIMEOUT: Wall = Wall::from_millis(500);

/// Everything `late_chunked_verdict` reads of one trial. Ticks are device ticks; the sent intervals are on the transmit axis, the receive blocks and the
/// heard intervals on the receive axis, which the loopback delays.
#[derive(Clone, Debug)]
pub struct Evidence {
    /// Root ticks a sample.
    pub per: i64,
    /// Root ticks of one send (the chunk).
    pub chunk_ticks: i64,
    /// The late burst's expected interval `[first sample, end)`.
    pub late: (i64, i64),
    /// The control burst's expected interval.
    pub control: (i64, i64),
    /// Failed device calls (`rx_start`, a send, a report read).
    pub errors: Vec<String>,
    /// The receive blocks `(first tick, samples)` in the order they came.
    pub blocks: Vec<(i64, usize)>,
    /// Each receive outcome that was not samples, with the stream position it came at (the end of the last block before it; `None` before the first).
    pub other: Vec<Option<i64>>,
    /// The tone heard, on the receive axis.
    pub heard: Vec<HeardInterval>,
    /// Every transmit report of the trial, in order.
    pub reports: Vec<TxReport>,
    /// The late burst's last send: the device time read when it had been handed, and its own first sample's time. A device that drops each late packet alone
    /// plays a chunk handed before its own time, so silence means something only if this one was handed at least `LAST_CHUNK_MARGIN_TICKS` before it (`None`: it
    /// was not handed).
    pub late_last: Option<(i64, i64)>,
}

/// The delay the control tone arrived with: its heard onset less its first sample's tick, when the control was heard (at least 90 % of its samples, its
/// onset within a tone window before to one chunk after its first sample's tick). A window that straddles the onset can read as tone (a mixed window's coherence
/// is not 1, but it can pass the bar) and a clean one starts up to a window after it, so the delay is read to about 25 samples either way.
fn control_delay(e: &Evidence) -> Option<i64> {
    let (from, to) = e.control;
    let slack = WINDOW as i64 * e.per;
    let heard: Vec<&HeardInterval> = e.heard.iter().filter(|iv| iv.first_tick >= from - slack && iv.first_tick < to + e.chunk_ticks).collect();
    let samples: usize = heard.iter().map(|iv| iv.samples).sum();
    let onset = heard.iter().map(|iv| iv.first_tick).min()?;
    (samples as i64 * 10 >= (to - from) / e.per * 9 && onset < from + e.chunk_ticks).then_some((onset - from).max(0))
}

/// Whether the receive blocks cover `[from, to)` without a gap: one run of blocks that each start where the one before ended.
fn covers(blocks: &[(i64, usize)], per: i64, from: i64, to: i64) -> bool {
    let mut run: Option<(i64, i64)> = None;
    for (first, len) in blocks {
        let end = first + *len as i64 * per;
        run = match run {
            Some((start, last)) if last == *first => Some((start, end)),
            _ => Some((*first, end)),
        };
        if run.is_some_and(|(start, last)| start <= from && last >= to) {
            return true;
        }
    }
    false
}

fn is_late_report(code: TxCode) -> bool {
    matches!(code, TxCode::TimeError | TxCode::Underflow | TxCode::UnderflowInPacket)
}

/// The verdict of one trial, in this order. FAIL is positive evidence and holds whatever the coverage: a BurstAck whose tick is at or after the late
/// burst's first sample (the control has its own, before), or a heard interval that overlaps the late burst's interval, the loopback delay taken off,
/// not only one inside it. INCONCLUSIVE when the evidence that makes silence mean something is missing: a failed call, receive blocks that do not cover
/// the interval plus a chunk each side without a gap, a non-sample receive outcome inside that range, a control that was not heard or was itself late,
/// a report whose tick cannot attribute it, a last chunk not handed 1 ms before its own time, or no TimeError (the burst was not late). PASS only with all of it,
/// nothing heard in the interval and no BurstAck.
pub fn late_chunked_verdict(e: &Evidence) -> Verdict {
    let (from, to) = e.late;
    let delay = control_delay(e);
    let shift = delay.unwrap_or(0);
    let acks: Vec<i64> = e.reports.iter().filter(|r| r.code == TxCode::BurstAck).filter_map(|r| r.tick).filter(|tick| *tick >= from).collect();
    if !acks.is_empty() {
        return Verdict::Fail(format!("a BurstAck for the late burst at tick(s) {acks:?}: a late burst is dropped without one"));
    }
    let played: Vec<(i64, i64)> = e.heard.iter().map(|iv| (iv.first_tick - shift, iv.last_tick - shift + e.per)).filter(|(first, end)| *first < to && *end > from).collect();
    if !played.is_empty() {
        return Verdict::Fail(format!("heard {played:?} (loopback delay {shift} ticks taken off) inside the late burst's interval {from}..{to}: part of a late burst played"));
    }
    let mut reasons: Vec<String> = e.errors.iter().map(|error| format!("a device call failed: {error}")).collect();
    let (want_from, want_to) = (from + shift - e.chunk_ticks, to + shift + e.chunk_ticks);
    if !covers(&e.blocks, e.per, want_from, want_to) {
        reasons.push(format!("receive blocks do not cover {want_from}..{want_to} without a gap"));
    }
    if e.other.iter().any(|at| at.is_some_and(|at| at >= want_from && at < want_to)) {
        reasons.push("a receive outcome other than samples came inside that range".to_owned());
    }
    if delay.is_none() {
        reasons.push("the control burst was not heard".to_owned());
    }
    match e.late_last {
        Some((handed, own)) if handed + LAST_CHUNK_MARGIN_TICKS <= own => {}
        other => reasons.push(format!(
            "the late burst's last send was not handed 1 ms before its own time {other:?} (handed, own): a device that drops late packets one by one could have dropped only the early ones"
        )),
    }
    if e.reports.iter().any(|r| r.tick.is_none() && (r.code == TxCode::BurstAck || is_late_report(r.code))) {
        reasons.push("a report without a tick cannot be attributed to the control or the late burst".to_owned());
    }
    if e.reports.iter().any(|r| is_late_report(r.code) && r.tick.is_some_and(|tick| tick < from)) {
        reasons.push("the control burst was itself late or starved".to_owned());
    }
    if !e.reports.iter().any(|r| r.code == TxCode::TimeError && r.tick.is_some_and(|tick| tick >= from)) {
        reasons.push("no TimeError: the burst was not late, so nothing was proved".to_owned());
    }
    if reasons.is_empty() {
        Verdict::Pass(format!("no 100 kHz tone window was detected in the late burst's interval, no BurstAck for it, a TimeError came; the control was heard (delay {shift} ticks)"))
    } else {
        Verdict::Inconclusive(reasons.join("; "))
    }
}

/// The tone windows of `samples` (25 samples, each one spacing apart) as intervals: coherent over 0.9 (`tone.rs`'s monitor bar), at `tone_hz` within a fifth
/// and above the 4-LSB amplitude floor (`QUIET_CHECK_MIN_POWER`), so noise and the FakeDevice's ramp (a coherent window at 0 Hz) are not a tone. Windows
/// that follow one another join; ticks are the samples' own.
pub fn tone_intervals(samples: &[(i64, Iq)], per: i64, tone_hz: f64) -> Vec<HeardInterval> {
    let mut intervals: Vec<HeardInterval> = Vec::new();
    for w in contiguous_windows_at(samples, per) {
        let (coherence, hz) = window_tone_at(w, per);
        if coherence <= 0.9 || (hz - tone_hz).abs() > tone_hz / 5.0 || mean_power(w) <= QUIET_CHECK_MIN_POWER {
            continue;
        }
        let (first, last) = (w[0].0, w[WINDOW - 1].0);
        match intervals.last_mut() {
            Some(open) if open.last_tick + per == first => {
                open.last_tick = last;
                open.samples += WINDOW;
            }
            _ => intervals.push(HeardInterval { first_tick: first, last_tick: last, samples: WINDOW }),
        }
    }
    intervals
}

/// Holds until the device's time reaches `tick`, on the device's own clock (the manual one on the fake, the real one on the USRP). `bench1115::wait_device_time`
/// does not fit: it is private and sleeps on the thread, which would never move a manual clock this thread holds. A failed time read is retried for 50 ms
/// (it is transient on a device that recovers) and then is an error: the caller must not send on a wait that did not happen, or a burst meant to be late goes out on time.
fn wait_until(device: &dyn Device, tick: i64) -> Result<(), String> {
    let mut failed = 0;
    loop {
        match device.time_now() {
            Ok(now) if now >= tick => return Ok(()),
            Ok(_) => device.host().sleep(Wall::from_micros(200)),
            Err(error) if failed >= 50 => return Err(format!("time_now: {}", error.message)),
            Err(_) => {
                failed += 1;
                device.host().sleep(Wall::from_millis(1));
            }
        }
    }
}

/// Sends `wave` as one device burst of timed sends of `chunk` samples (SOB on the first, EOB on the last, each with its first sample's time), each once
/// it is inside the in-flight window, as the Module does; a short send's rest follows with its own time. Returns the last send's `(device time read once it was
/// handed, its own first sample's time)`.
fn send_chunked(device: &dyn Device, wave: &[Iq], first_at: i64, per: i64, chunk: usize) -> Result<(i64, i64), String> {
    let (mut sent, mut last) = (0, (0, 0));
    while sent < wave.len() {
        let n = chunk.min(wave.len() - sent);
        let at = first_at + sent as i64 * per;
        wait_until(device, at - WINDOW_TICKS)?;
        let took = device.tx_send(0, &[&wave[sent..sent + n]], Some(at), sent == 0, sent + n == wave.len(), TX_TIMEOUT).map_err(|error| error.message)?;
        if took == 0 {
            return Err(format!("the device took none of the samples sent at sample {sent}"));
        }
        last = (device.time_now().map_err(|error| error.message)?, at);
        sent += took;
    }
    Ok(last)
}

/// Every report that is there, until three quiet reads of 20 ms.
fn read_reports(device: &dyn Device, errors: &mut Vec<String>) -> Vec<TxReport> {
    let begun = Instant::now();
    let (mut quiet, mut out) = (0, Vec::new());
    while begun.elapsed() < Wall::from_secs(2) && quiet < 3 {
        match device.tx_async(0, Wall::from_millis(20)) {
            Ok(Some(report)) => {
                quiet = 0;
                out.push(report);
            }
            Ok(None) => quiet += 1,
            Err(error) => {
                errors.push(format!("tx_async: {}", error.message));
                break;
            }
        }
    }
    out
}

/// What the receive thread read: `raw::read_with`'s blocks and samples, the stream position each other outcome came at (the end of the last block before
/// it, `None` before the first) and the failed calls.
struct RxLog {
    blocks: Vec<(i64, usize)>,
    samples: Vec<(i64, Iq)>,
    other: Vec<Option<i64>>,
    errors: Vec<String>,
}

/// The positions of the outcomes that were not samples, read off the order `Read::events` keeps.
fn rx_log(read: Read, per: i64) -> RxLog {
    let (mut blocks, mut end, mut other, mut errors) = (read.blocks.iter(), None, Vec::new(), Vec::new());
    for event in &read.events {
        if event == "samples" {
            end = blocks.next().map(|(first, len)| first + *len as i64 * per);
        } else {
            other.push(end);
            if event.starts_with("Failed") {
                errors.push(format!("rx_recv: {event}"));
            }
        }
    }
    RxLog { blocks: read.blocks, samples: read.samples, other, errors }
}

/// The receive thread is a participant of the device's clock like the sending thread (`late_chunked_cell`'s): the clock stands while it is descheduled, so
/// a loaded host cannot make the fake report the reader as 100 ms behind (an `Overflow`, which would turn the trial INCONCLUSIVE). A real clock registers
/// nothing. It reads until the blocks reach `end_tick`, or a call fails (a closed streamer fails at once and would be polled to the limit), or 10 s.
fn read_rx(device: &dyn Device, per: i64, n: usize, end_tick: i64) -> RxLog {
    let _driver = device.host().drive();
    let read = read_with(device, 0, n, Wall::from_secs(10), Some(per), |read| read.end(per).is_some_and(|end| end >= end_tick) || read.events.last().is_some_and(|event| event.starts_with("Failed")));
    rx_log(read, per)
}

/// One trial of the cell at `rate`: receive from 20 ms on, the control burst at 60 ms, the late burst's first sample timed `late_ms` before the host's
/// reference instant (130 ms; nothing but the control's sends comes between), and the receive read to two chunks and a ms past the late burst's end by a
/// thread of its own, so the receiver never waits on a send. Every report is read at the end.
fn late_chunked_trial(device: &dyn Device, rate: f64, wave: &[Iq], chunk: usize, late_ms: i64) -> Evidence {
    let per = MCR / rate as i64;
    let chunk_ticks = chunk as i64 * per;
    let d0 = device.time_now().expect("the device's time");
    let (rx_at, control_at, reference) = (align_to(d0 + 20 * TICKS_PER_MS, per), align_to(d0 + 60 * TICKS_PER_MS, per), align_to(d0 + 130 * TICKS_PER_MS, per));
    let late_at = reference - late_ms * TICKS_PER_MS;
    let len = wave.len() as i64 * per;
    let (control, late) = ((control_at, control_at + len), (late_at, late_at + len));
    let read_to = late.1 + 2 * chunk_ticks + TICKS_PER_MS;
    let mut errors = Vec::new();
    if let Err(error) = device.rx_start(0, rx_at) {
        errors.push(format!("rx_start: {}", error.message));
    }
    let n = (rate / 1_000.0) as usize;
    let mut late_last = None;
    let log = std::thread::scope(|scope| {
        let reader = scope.spawn(|| read_rx(device, per, n.max(1_000), read_to));
        if let Err(error) = send_chunked(device, wave, control_at, per, chunk) {
            errors.push(format!("control burst: {error}"));
        }
        match wait_until(device, reference) {
            Ok(()) => match send_chunked(device, wave, late_at, per, chunk) {
                Ok(last) => late_last = Some(last),
                Err(error) => errors.push(format!("late burst: {error}")),
            },
            Err(error) => errors.push(format!("the wait for the late burst: {error}")),
        }
        if let Err(error) = wait_until(device, read_to + TICKS_PER_MS) {
            errors.push(format!("the wait for the receive: {error}"));
        }
        // Polled on the device's clock: a join would hold this thread busy, and a manual clock does not run while a participant is busy.
        while !reader.is_finished() {
            device.host().sleep(Wall::from_millis(1));
        }
        reader.join().expect("the receive thread")
    });
    let reports = read_reports(device, &mut errors);
    if let Err(error) = device.rx_stop(0) {
        errors.push(format!("rx_stop: {}", error.message));
    }
    let _ = drain_without_clock(device, Wall::from_millis(10), Wall::from_millis(300));
    errors.extend(log.errors);
    Evidence { per, chunk_ticks, late, control, errors, blocks: log.blocks, other: log.other, heard: tone_intervals(&log.samples, per, TONE_HZ), reports, late_last }
}

/// One trial of a cell: its rate, its number and what it read.
pub struct Trial {
    pub rate: f64,
    pub trial: usize,
    pub evidence: Evidence,
}

impl Trial {
    /// The trial as the bench logs it: the intervals, the receive coverage and the reports by code, so a verdict can be read back from the log.
    pub fn trace(&self) -> Json {
        let e = &self.evidence;
        let count = |code: TxCode| e.reports.iter().filter(|r| r.code == code).count();
        json!({
            "rate_hz": self.rate, "trial": self.trial, "per_ticks": e.per, "chunk_ticks": e.chunk_ticks, "late": [e.late.0, e.late.1], "control": [e.control.0, e.control.1],
            "control_delay_ticks": control_delay(e), "late_last_handed_and_own_time": e.late_last, "errors": e.errors,
            "blocks": e.blocks.len(), "blocks_first": e.blocks.first(), "blocks_end": e.blocks.last().map(|(first, len)| first + *len as i64 * e.per),
            "other_positions": e.other, "heard": e.heard.iter().map(|iv| [iv.first_tick, iv.last_tick, iv.samples as i64]).collect::<Vec<_>>(),
            "reports": { "BurstAck": count(TxCode::BurstAck), "TimeError": count(TxCode::TimeError), "Underflow": count(TxCode::Underflow) + count(TxCode::UnderflowInPacket) },
            "late_time_errors": e.reports.iter().filter(|r| r.code == TxCode::TimeError && r.tick.is_some_and(|tick| tick >= e.late.0)).count(),
            "acks_at_or_after_late": e.reports.iter().filter(|r| r.code == TxCode::BurstAck && r.tick.is_some_and(|tick| tick >= e.late.0)).count(),
        })
    }
}

/// The cell at `rate` on `device`, which opens its streamers (1 GHz, 0 dB, channel 0: `raw_tx` refuses anything outside bench.md's RF envelope) and closes
/// them again. The caller is the only driver of the device's clock while it runs.
pub fn late_chunked_cell(device: &dyn Device, rate: f64, trials: usize, late_ms: i64) -> Vec<Trial> {
    let _driver = device.host().drive();
    raw_rx(device, rate, 1e9);
    raw_tx(device, rate, 1e9);
    let chunk = usize::try_from(send_cap(WINDOW_TICKS, MCR / rate as i64, &PACKETS)).expect("a send is a positive count");
    let wave: Vec<Iq> = tone_wave(rate, CHUNKS * chunk, TONE_HZ).into_iter().map(|(re, im)| [re, im]).collect();
    let out = (0..trials).map(|trial| Trial { rate, trial, evidence: late_chunked_trial(device, rate, &wave, chunk, late_ms) }).collect();
    device.rx_close(0);
    device.tx_close(0);
    out
}

/// The cell's verdict: PASS only if every trial passes, FAIL if any fails, else INCONCLUSIVE; no trial is INCONCLUSIVE.
pub fn cell_verdict(trials: &[Trial]) -> Verdict {
    if trials.is_empty() {
        return Verdict::Inconclusive("no trial was run".to_owned());
    }
    iii::all_of(trials.iter().map(|trial| late_chunked_verdict(&trial.evidence)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PER: i64 = 200;
    const CHUNK: i64 = 500_000;
    const DELAY: i64 = 44 * PER;
    /// The delay the verdict reads off the control: its onset is 5 samples into the first tone window.
    const MEASURED: i64 = DELAY + 5 * PER;

    fn report(code: TxCode, tick: Option<i64>) -> TxReport {
        TxReport { code, tick, channel: 0 }
    }

    /// 1 MS/s, 2 500-sample chunks, 8 per burst: a control at 10 000 000 and a late burst at 30 000 000, 4 000 000 ticks (20 000 samples) each, the loopback
    /// 44 samples late, the receive from 2 000 000 to 40 000 000 in 1 ms blocks, the control heard, nothing else heard, the control's BurstAck and two TimeErrors.
    fn passing() -> Evidence {
        let blocks = (0..190).map(|k| (2_000_000 + k * 200_000, 1_000)).collect();
        let control = (10_000_000, 14_000_000);
        Evidence {
            per: PER,
            chunk_ticks: CHUNK,
            late: (30_000_000, 34_000_000),
            control,
            errors: Vec::new(),
            blocks,
            other: Vec::new(),
            heard: vec![HeardInterval { first_tick: control.0 + DELAY + 5 * PER, last_tick: control.1 + DELAY - 5 * PER, samples: 19_990 }],
            reports: vec![report(TxCode::BurstAck, Some(14_000_000)), report(TxCode::TimeError, Some(30_000_000)), report(TxCode::TimeError, Some(30_500_000))],
            late_last: Some((31_000_000, 33_500_000)),
        }
    }

    fn kind(verdict: &Verdict) -> &'static str {
        match verdict {
            Verdict::Pass(_) => "PASS",
            Verdict::Inconclusive(_) => "INCONCLUSIVE",
            Verdict::Fail(_) => "FAIL",
        }
    }

    fn assert_kind(e: &Evidence, want: &str, what: &str) {
        let verdict = late_chunked_verdict(e);
        assert_eq!(kind(&verdict), want, "{what}: {verdict:?}");
    }

    /// A heard interval of `samples` samples starting at `first`.
    fn heard(first: i64, samples: i64) -> HeardInterval {
        HeardInterval { first_tick: first, last_tick: first + (samples - 1) * PER, samples: samples as usize }
    }

    #[test]
    fn mt3_c1_verdict_passes_with_complete_evidence() {
        assert_kind(&passing(), "PASS", "complete coverage, the control heard, nothing heard in the late interval, no BurstAck for it, a TimeError");
    }

    #[test]
    fn mt3_c1_verdict_absent_receive_is_inconclusive_not_pass() {
        // The control and everything else as in the passing case: only the receive evidence is missing.
        let mut no_blocks = passing();
        no_blocks.blocks.clear();
        assert_kind(&no_blocks, "INCONCLUSIVE", "no receive block at all");
        let mut failed = passing();
        failed.errors = vec!["rx_start: the stream is not open".to_owned()];
        assert_kind(&failed, "INCONCLUSIVE", "rx_start failed");
    }

    #[test]
    fn mt3_c1_verdict_incomplete_receive_is_inconclusive() {
        let late = passing().late;
        let mut gap = passing();
        gap.blocks.retain(|(first, _)| *first != 32_000_000);
        assert_eq!(gap.blocks.len(), 189);
        assert_kind(&gap, "INCONCLUSIVE", "a block inside the interval is missing");
        let mut ends_early = passing();
        ends_early.blocks.truncate(160);
        assert_kind(&ends_early, "INCONCLUSIVE", "the blocks end (at 34 000 000) before the interval's end and its margin");
        let mut no_margin_after = passing();
        no_margin_after.blocks.retain(|(first, _)| *first + 200_000 < late.1 + MEASURED + CHUNK);
        assert_kind(&no_margin_after, "INCONCLUSIVE", "the margin after the interval is missing by under one block");
        let mut no_margin_before = passing();
        no_margin_before.blocks.retain(|(first, _)| *first > late.0 + MEASURED - CHUNK);
        assert_kind(&no_margin_before, "INCONCLUSIVE", "the margin before the interval is missing by under one block");
        let mut exactly = passing();
        exactly.blocks.clear();
        let mut at = late.0 + MEASURED - CHUNK;
        while at < late.1 + MEASURED + CHUNK {
            exactly.blocks.push((at, 1_000));
            at += 200_000;
        }
        assert_kind(&exactly, "PASS", "blocks that cover the range exactly, the control heard");
    }

    #[test]
    fn mt3_c1_verdict_without_the_control_is_inconclusive() {
        let mut not_heard = passing();
        not_heard.heard.clear();
        assert_kind(&not_heard, "INCONCLUSIVE", "the control was not heard");
        let mut half = passing();
        half.heard = vec![heard(10_000_000 + DELAY, 10_000)];
        assert_kind(&half, "INCONCLUSIVE", "half of the control was heard");
        let mut late_control = passing();
        late_control.reports.push(report(TxCode::TimeError, Some(11_000_000)));
        assert_kind(&late_control, "INCONCLUSIVE", "the control itself drew a TimeError");
    }

    #[test]
    fn mt3_c1_verdict_a_receive_outcome_inside_the_range_is_inconclusive() {
        let mut inside = passing();
        inside.other = vec![Some(31_000_000)];
        assert_kind(&inside, "INCONCLUSIVE", "an overflow in the interval");
        let mut margin = passing();
        margin.other = vec![Some(30_000_000 - CHUNK + MEASURED)];
        assert_kind(&margin, "INCONCLUSIVE", "an overflow at the margin's first tick");
        let mut outside = passing();
        outside.other = vec![None, Some(3_000_000), Some(39_000_000)];
        assert_kind(&outside, "PASS", "outcomes before the first block, before the margin and after it");
    }

    #[test]
    fn mt3_c1_verdict_a_heard_interval_inside_the_late_interval_fails() {
        let mut inside = passing();
        inside.heard.push(heard(30_000_000 + DELAY + 500_000, 2_500));
        assert_kind(&inside, "FAIL", "the first chunk of the late burst was heard");
        let mut incomplete = inside.clone();
        incomplete.blocks.truncate(160);
        assert_kind(&incomplete, "FAIL", "partial transmission is positive evidence whatever the coverage");
        let mut no_control = inside.clone();
        no_control.heard.remove(0);
        assert_kind(&no_control, "FAIL", "and whatever the control");
    }

    #[test]
    fn mt3_c1_verdict_a_heard_interval_that_straddles_the_late_interval_fails() {
        let late = passing().late;
        // Starts 10 samples before the late burst's first sample (the delay taken off) and runs 200 samples into it.
        let mut from_before = passing();
        from_before.heard.push(heard(late.0 + DELAY - 10 * PER, 210));
        assert_kind(&from_before, "FAIL", "a heard interval over the start of the late interval");
        // Starts 100 samples before its end and runs past it.
        let mut past_the_end = passing();
        past_the_end.heard.push(heard(late.1 + DELAY - 100 * PER, 300));
        assert_kind(&past_the_end, "FAIL", "a heard interval over the end of the late interval");
        // Ends exactly where the late interval begins: touching is not overlapping.
        let mut touching = passing();
        touching.heard.push(heard(late.0 + MEASURED - 100 * PER, 100));
        assert_kind(&touching, "PASS", "a heard interval that ends where the late interval starts");
        // The loopback delay is taken off: a tone that arrives before the delayed start is not the late burst's.
        let mut before_the_delay = passing();
        before_the_delay.heard.push(heard(late.0 + 5 * PER, 20));
        assert_kind(&before_the_delay, "PASS", "a tone inside the delay after the late interval's first tick");
    }

    #[test]
    fn mt3_c1_verdict_a_burst_ack_attributed_by_tick() {
        let mut late_ack = passing();
        late_ack.reports.push(report(TxCode::BurstAck, Some(34_000_000)));
        assert_kind(&late_ack, "FAIL", "a BurstAck at the late burst's end");
        let mut first_sample = passing();
        first_sample.reports.push(report(TxCode::BurstAck, Some(30_000_000)));
        assert_kind(&first_sample, "FAIL", "a BurstAck at the late burst's first tick");
        let mut incomplete = late_ack.clone();
        incomplete.blocks.clear();
        assert_kind(&incomplete, "FAIL", "a BurstAck fails whatever the coverage");
        // The passing case already carries the control's BurstAck (tick 14 000 000): it is the control's by its tick.
        let mut before = passing();
        before.reports.push(report(TxCode::BurstAck, Some(29_999_999)));
        assert_kind(&before, "PASS", "a BurstAck one tick before the late burst's first tick is the control's");
        let mut tickless = passing();
        tickless.reports.push(report(TxCode::BurstAck, None));
        assert_kind(&tickless, "INCONCLUSIVE", "a BurstAck without a tick cannot be attributed");
    }

    #[test]
    fn mt3_c1_verdict_a_last_chunk_handed_after_its_own_time_is_inconclusive() {
        // The late burst's last chunk is timed 33 500 000; a device that drops late packets one by one drops only the chunks that are late at their own time.
        let mut after = passing();
        after.late_last = Some((33_500_001, 33_500_000));
        assert_kind(&after, "INCONCLUSIVE", "the last chunk was handed after its own time: silence could be per-packet drops of the early chunks");
        let mut at = passing();
        at.late_last = Some((33_500_000, 33_500_000));
        assert_kind(&at, "INCONCLUSIVE", "handed at its own time is not 1 ms before it");
        let mut short = passing();
        short.late_last = Some((33_300_001, 33_500_000));
        assert_kind(&short, "INCONCLUSIVE", "handed 199 999 ticks (under 1 ms) before its own time");
        let mut before = passing();
        before.late_last = Some((33_300_000, 33_500_000));
        assert_kind(&before, "PASS", "handed exactly 1 ms (200 000 ticks) before its own time");
        let mut none = passing();
        none.late_last = None;
        assert_kind(&none, "INCONCLUSIVE", "the last send was not handed");
        let mut played = after.clone();
        played.heard.push(heard(31_000_000 + MEASURED, 2_500));
        assert_kind(&played, "FAIL", "what was heard still fails, whenever the last chunk was handed");
    }

    #[test]
    fn mt3_c1_verdict_without_a_time_error_is_inconclusive() {
        let mut none = passing();
        none.reports.retain(|r| r.code != TxCode::TimeError);
        assert_kind(&none, "INCONCLUSIVE", "no TimeError: the burst was not late");
        let mut control_only = passing();
        control_only.reports.retain(|r| r.code != TxCode::TimeError);
        control_only.reports.push(report(TxCode::TimeError, Some(11_000_000)));
        assert_kind(&control_only, "INCONCLUSIVE", "a TimeError before the late burst is the control's, not the late burst's");
        let mut tickless = passing();
        tickless.reports.retain(|r| r.code != TxCode::TimeError);
        tickless.reports.push(report(TxCode::TimeError, None));
        assert_kind(&tickless, "INCONCLUSIVE", "a TimeError without a tick is not evidence of the late burst");
    }

    #[test]
    fn mt3_c1_the_tone_detector_hears_a_tone_and_neither_noise_nor_a_ramp() {
        let per = 200;
        let tone = |from: usize, n: usize, amplitude: f32| -> Vec<(i64, Iq)> {
            (from..from + n)
                .map(|i| {
                    let phase = 2.0 * std::f64::consts::PI * 100e3 * (i as f64 / 1e6);
                    (i as i64 * per, [amplitude * phase.cos() as f32, amplitude * phase.sin() as f32])
                })
                .collect()
        };
        // A tone at the loopback's level over 100 samples hears as one interval of four windows.
        assert_eq!(tone_intervals(&tone(0, 100, 0.01), per, 100e3), vec![HeardInterval { first_tick: 0, last_tick: 99 * per, samples: 100 }]);
        // Below the amplitude floor it is not heard (about 1 LSB).
        assert!(tone_intervals(&tone(0, 100, 0.00003), per, 100e3).is_empty());
        // At another frequency it is not heard.
        assert!(tone_intervals(&tone(0, 100, 0.01), per, 300e3).is_empty());
        // The fake's ramp (real, rising, at 0 Hz) is coherent and loud but not the tone.
        let ramp: Vec<(i64, Iq)> = (0..100).map(|i| (i * per, [(i as f32 * 100.0 % 65_536.0) / 65_536.0 + 0.1, 0.0])).collect();
        assert!(tone_intervals(&ramp, per, 100e3).is_empty());
        // A tone whose phase jitters (coherence about 0.7, at the right frequency and level) is not the tone.
        let jittery: Vec<(i64, Iq)> = (0..100)
            .map(|i| {
                let phase = 2.0 * std::f64::consts::PI * 100e3 * (i as f64 / 1e6) + 0.4 * if i % 2 == 0 { 1.0 } else { -1.0 };
                (i as i64 * per, [0.01 * phase.cos() as f32, 0.01 * phase.sin() as f32])
            })
            .collect();
        assert!(tone_intervals(&jittery, per, 100e3).is_empty());
        // A hole in the ticks breaks the window.
        let mut holed = tone(0, 100, 0.01);
        holed.iter_mut().skip(40).for_each(|(tick, _)| *tick += per);
        assert_eq!(tone_intervals(&holed, per, 100e3).len(), 2, "window 0, then windows 2 and 3 after the hole: window 1 straddles it");
    }
}
