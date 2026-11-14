//! Maintenance table 3, cell C4 of `plan/phase8/bench.md` (UR-33, Fable M3): the cascade boundary. Does the X310 end a drain no later than the fake's limit? One lifted
//! body, `cascade_cell`, runs on the USRP (`hardware.rs`, `hw_mt3_c4_cascade_boundary`) and on `FakeDevice` (`fake.rs`, the rehearsal); the verdicts are the pure
//! `cascade_outcome` (one trial) and `sweep_verdict` (the sweep).
//!
//! The rule (UR-33, spec 18): a burst A whose first timed packet is late is dropped whole, and the DUC consumes its samples (the drain) at one sample per tick,
//! so a burst B handed behind it is late too until the drain ends. The fake ends a drain at `drain end + (pad + g)` (`src/device.rs` `drain_limit`: a share of the
//! samples, two clocks a send, the guard and the DUC's wait a burst), its own margin over the device. The cell measures where the device really ends it, against that limit.
//!
//! `t0` is the tick of A's first TimeError, which is the radio time when the radio found A's first packet late (`radio_tx_core.v:280-284, 337-355, 450-459`, VERIFIED:
//! `radio_time_d1` in `ST_TIME_CHECK`): the start of the drain as the radio sees it, not A's own timestamp and not a host arrival time. The fake's drain starts when it takes the packet.
//! So `t0 + S x per` is the drain's end up to one sample period and a few master clocks (`uncertainty_ticks`), and `delta` is a coordinate measured from it, not an
//! asserted end of the device's drain.

#![allow(dead_code)]

use std::time::{Duration as Wall, Instant};

use ezsdr_radio_uhd::{Device, Iq, TxCode, TxReport};
use serde_json::{Value as Json, json};

use super::MCR;
use super::mt3::{self, Plan, Sent};
use super::raw::{TICKS_PER_MS, align_to};
use super::tone::Verdict;

/// The rates cell C4 runs at.
pub const C4_RATES: [f64; 2] = [1e6, 10e6];
/// Trials per delta on the bench (AGENTS §9: at least 10 near a boundary).
pub const C4_TRIALS: usize = 10;
/// Decided (played or dropped) trials a delta at or past the fake's limit needs for the sweep to be more than INCONCLUSIVE.
pub const MIN_DECIDED: usize = 10;
/// Root ticks a microsecond.
pub const US: i64 = MCR / 1_000_000;
/// How far before the drain reaches it each of A's sends (after the first, which makes `t0`) must have been handed: a send that is late lets the drain wait for data
/// (`radio_tx_core.v:399-426`), which lengthens it, so the host would be measured, not the device.
const A_MARGIN_TICKS: i64 = TICKS_PER_MS;
/// How long before `t0 + S x per` B must have been handed: B is then queued behind the drain (INFERRED: the transport's delay is not measured).
const B_MARGIN_TICKS: i64 = 2 * TICKS_PER_MS;
/// A's first sample is timed this long before the host's reference instant: the burst is late by at least that much (C1's shape).
const A_LATE_TICKS: i64 = TICKS_PER_MS;
/// "A few master clocks" of the uncertainty between `t0` and the fake's drain start (INFERRED: the radio's pipeline between a packet's arrival and its time check).
const CLOCKS: i64 = 4;

/// The comparison between the device's and the fake's drain carries this: a sample period (the fake's drain starts at a tick of its own, B's time is on the sample grid)
/// and a few master clocks.
pub fn uncertainty_ticks(per: i64) -> i64 {
    per + CLOCKS
}

/// The samples of `nanos` at `ratio` root ticks a sample, rounded up, as the fake counts them (`Radio::samples_in`, restated here: the test states the fake's formula itself,
/// so that a change to the fake's pad fails the rehearsal rather than being followed).
fn samples_in(nanos: i128, ratio: i64) -> i64 {
    let ticks = (nanos * i128::from(MCR) + 999_999_999) / 1_000_000_000;
    i64::try_from((ticks + i128::from(ratio) - 1) / i128::from(ratio)).expect("a sample count")
}

/// The ticks past A's drain end at which the fake stops calling an idle timed packet late (`Radio::drain_limit` less the drain's end): `(pad + g)` samples, the pad
/// being a hundredth of A's `samples`, two samples a send (`packets`), and for each closed burst `bursts` the guard `g` and the DUC's wait of 10 µs; `g` is the larger of
/// 3 samples and 15 µs. `ratio` is root ticks a sample.
pub fn fake_limit_ticks(ratio: i64, samples: usize, packets: usize, bursts: usize) -> i64 {
    let g = samples_in(15_000, ratio).max(3);
    let pad = (samples as i64 + 99) / 100 + 2 * packets as i64 + bursts as i64 * (g + samples_in(10_000, ratio));
    (pad + g) * ratio
}

/// The deltas of the sweep, in root ticks, ascending: coarse -20, -10, 50 and 100 µs, fine 0 to 30 µs in 1 µs steps, the fake's own limit, the limit plus the
/// comparison's uncertainty (`uncertainty_ticks(per)`: the first delta at which a dropped B fails the sweep, so the device's drain cannot end between it and the
/// next point unseen) and the limit plus 20 µs. Each is aligned up to the sample grid when B's time is made (`align_to`).
pub fn grid(limit: i64, per: i64) -> Vec<i64> {
    let mut deltas: Vec<i64> = [-20, -10, 50, 100].iter().chain(&(0..=30).collect::<Vec<i64>>()).map(|us| us * US).chain([limit, limit + uncertainty_ticks(per), limit + 20 * US]).collect();
    deltas.sort_unstable();
    deltas.dedup();
    deltas
}

/// Everything `cascade_outcome` reads of one trial. Ticks are device ticks.
#[derive(Clone, Debug)]
pub struct CascadeEvidence {
    /// Root ticks a sample.
    pub per: i64,
    /// A's first sample's time.
    pub a_first: i64,
    /// A's duration, `S x per`.
    pub a_ticks: i64,
    /// The sends A took when it was planned (`S` over the chunk), against the sends it took.
    pub planned_a_sends: usize,
    /// A's sends, each with its first sample's time and the device time read once it was handed.
    pub a_sends: Vec<Sent>,
    /// The tick of A's first TimeError (`None`: there was none, and B was not sent).
    pub t0: Option<i64>,
    /// B's first sample's time.
    pub b_first: i64,
    /// The device time read once B had been handed (`None`: it was not handed).
    pub b_handed: Option<i64>,
    /// Every report of the trial, in order.
    pub reports: Vec<TxReport>,
    /// Failed device calls.
    pub errors: Vec<String>,
}

/// What B did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Played,
    Dropped,
    Inconclusive(String),
}

/// The outcome of one trial. INCONCLUSIVE when the measurement is not valid: a failed call, no TimeError for A (it was not late: there is no drain to measure), A taking more
/// sends than planned (the fake's pad counts a packet a send), a send of A after the first handed less than 1 ms before the drain reached it (the drain waits for data, so the
/// host's stall would have lengthened it: the pad would be blamed for the host), B not handed 2 ms before `t0 + S x per` (not known to be queued behind the drain: INFERRED), an
/// Underflow, `UnderflowInPacket` or sequence-error report (a partly played B must not count as dropped), a report without a tick, or a BurstAck before B's first sample (A
/// is never acknowledged). Then: B **played** when a BurstAck came at or after B's first sample; **dropped** when none did and a TimeError did; anything else INCONCLUSIVE.
pub fn cascade_outcome(e: &CascadeEvidence) -> Outcome {
    let mut reasons: Vec<String> = e.errors.iter().map(|error| format!("a device call failed: {error}")).collect();
    let Some(t0) = e.t0 else {
        reasons.push("no TimeError for A: it was not late, so there is no drain".to_owned());
        return Outcome::Inconclusive(reasons.join("; "));
    };
    if e.a_sends.len() != e.planned_a_sends {
        reasons.push(format!("A took {} sends, not the planned {}: the fake's pad counts a packet a send", e.a_sends.len(), e.planned_a_sends));
    }
    let starved: Vec<(i64, i64)> = e.a_sends.iter().skip(1).filter(|send| send.handed + A_MARGIN_TICKS > t0 + (send.at - e.a_first)).map(|send| (send.at - e.a_first, send.handed)).collect();
    if !starved.is_empty() {
        reasons.push(format!("A's sends (offset, handed) {starved:?} were handed less than 1 ms before the drain reached them: the host may have starved the drain"));
    }
    match e.b_handed {
        Some(handed) if handed + B_MARGIN_TICKS <= t0 + e.a_ticks => {}
        other => reasons.push(format!("B was not handed 2 ms before t0 + S x per = {} ({other:?}): not known to be queued behind the drain (INFERRED margin)", t0 + e.a_ticks)),
    }
    if e.reports.iter().any(|r| matches!(r.code, TxCode::Underflow | TxCode::UnderflowInPacket | TxCode::SeqError | TxCode::SeqErrorInBurst)) {
        reasons.push("an Underflow or sequence-error report: a partly played B must not count as dropped".to_owned());
    }
    if e.reports.iter().any(|r| r.tick.is_none() && matches!(r.code, TxCode::BurstAck | TxCode::TimeError)) {
        reasons.push("a report without a tick cannot be attributed to A or B".to_owned());
    }
    let acks = |from: i64| e.reports.iter().filter(|r| r.code == TxCode::BurstAck && r.tick.is_some_and(|tick| tick >= from)).count();
    if e.reports.iter().any(|r| r.code == TxCode::BurstAck && r.tick.is_some_and(|tick| tick < e.b_first)) {
        reasons.push("a BurstAck before B's first sample: A is never acknowledged".to_owned());
    }
    let time_errors = e.reports.iter().filter(|r| r.code == TxCode::TimeError && r.tick.is_some_and(|tick| tick >= e.b_first)).count();
    if !reasons.is_empty() {
        return Outcome::Inconclusive(reasons.join("; "));
    }
    match (acks(e.b_first) > 0, time_errors > 0) {
        (true, false) => Outcome::Played,
        (false, true) => Outcome::Dropped,
        (true, true) => Outcome::Inconclusive("a BurstAck and a TimeError both at or after B's first sample".to_owned()),
        (false, false) => Outcome::Inconclusive("neither a BurstAck nor a TimeError at or after B's first sample".to_owned()),
    }
}

/// One delta's trials, counted, with the effective delta (B's time aligned up to the sample grid, from `t0 + S x per`) of each dropped one: the sweep judges a trial by where B
/// really was, not by the nominal delta of its cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub delta: i64,
    pub played: usize,
    pub dropped: usize,
    pub inconclusive: usize,
    pub dropped_at: Vec<i64>,
}

impl Cell {
    /// The cell of `delta` from its trials' (effective delta, outcome).
    pub fn of(delta: i64, trials: &[(i64, Outcome)]) -> Cell {
        let count = |want: &Outcome| trials.iter().filter(|(_, outcome)| outcome == want).count();
        let (played, dropped) = (count(&Outcome::Played), count(&Outcome::Dropped));
        let dropped_at = trials.iter().filter(|(_, outcome)| *outcome == Outcome::Dropped).map(|(effective, _)| *effective).collect();
        Cell { delta, played, dropped, inconclusive: trials.len() - played - dropped, dropped_at }
    }

    fn line(&self) -> String {
        format!("{} us: {} played, {} dropped, {} inconclusive", self.delta as f64 / US as f64, self.played, self.dropped, self.inconclusive)
    }
}

/// The verdict of the sweep against the fake's `limit` (ticks past A's drain end) and the `uncertainty` of the comparison. FAIL if any trial whose effective delta (B's
/// time as it was aligned to the sample grid) is at least the limit plus the uncertainty was dropped: the device ended a drain later than the fake does, so the fake is looser than the device (UR-33 broken). INCONCLUSIVE if no delta
/// reaches the limit, or one that does has fewer than `MIN_DECIDED` decided trials. PASS otherwise; a dropped trial inside the uncertainty past the limit is counted in the
/// text, not failed. The deltas below the limit are the device's own boundary, a measurement and not a verdict term.
pub fn sweep_verdict(cells: &[Cell], limit: i64, uncertainty: i64) -> Verdict {
    let beyond: Vec<&Cell> = cells.iter().filter(|cell| cell.delta >= limit).collect();
    let late: Vec<String> = cells.iter().filter(|cell| cell.dropped_at.iter().any(|at| *at >= limit + uncertainty)).map(|cell| cell.line()).collect();
    if !late.is_empty() {
        return Verdict::Fail(format!("B was dropped at or past the fake's limit {limit} ticks plus the uncertainty {uncertainty}: the device ends a drain later than the fake ({late:?})"));
    }
    let thin: Vec<String> = beyond.iter().filter(|cell| cell.played + cell.dropped < MIN_DECIDED).map(|cell| cell.line()).collect();
    if beyond.is_empty() || !thin.is_empty() {
        return Verdict::Inconclusive(format!("the deltas at or past the fake's limit {limit} ticks need {MIN_DECIDED} decided trials each: {} cell(s) at or past it, thin: {thin:?}", beyond.len()));
    }
    let inside: usize = cells.iter().flat_map(|cell| &cell.dropped_at).filter(|at| **at >= limit && **at < limit + uncertainty).count();
    Verdict::Pass(format!("no B was dropped at or past the fake's limit {limit} ticks plus the uncertainty {uncertainty}; {inside} dropped trial(s) inside the uncertainty past the limit"))
}

/// The bracket and its mixed cells: the last delta whose decided trials were all dropped, the first above it whose were all played, and every cell with both between.
/// A measurement of the device's own boundary.
pub fn boundary_summary(cells: &[Cell]) -> String {
    let all_dropped = cells.iter().filter(|cell| cell.dropped > 0 && cell.played == 0).map(|cell| cell.delta).max();
    let all_played = cells.iter().filter(|cell| cell.played > 0 && cell.dropped == 0 && all_dropped.is_none_or(|dropped| cell.delta > dropped)).map(|cell| cell.delta).min();
    let us = |delta: Option<i64>| delta.map_or("none".to_owned(), |delta| format!("{} us", delta as f64 / US as f64));
    let mixed: Vec<String> = cells.iter().filter(|cell| cell.played > 0 && cell.dropped > 0).map(Cell::line).collect();
    format!("last all-dropped delta {}, first all-played delta {}, mixed cells {mixed:?}", us(all_dropped), us(all_played))
}

/// `t0` from one report: its tick when it is a TimeError at or after A's own first sample (an earlier one is the last trial's leftover, not A's).
pub fn a_time_error(report: &TxReport, a_first: i64) -> Option<i64> {
    report.tick.filter(|tick| report.code == TxCode::TimeError && *tick >= a_first)
}

/// One trial at `rate`: A (a 20 ms chunked burst of the Module, its first sample 1 ms before the host's reference instant: C1's late burst), the reports read until A's first
/// TimeError (`t0`), then B (one timed burst of one chunk, SOB and EOB in one send) at `t0 + S x per + delta` on the sample grid, handed at once. No receive.
fn cascade_trial(device: &dyn Device, rate: f64, wave: &[Iq], chunk: usize, delta: i64, rate_errors: &[String]) -> CascadeEvidence {
    let per = MCR / rate as i64;
    let d0 = device.time_now().expect("the device's time");
    let reference = align_to(d0 + 60 * TICKS_PER_MS, per);
    let a_first = align_to(reference - A_LATE_TICKS, per);
    let a_ticks = wave.len() as i64 * per;
    let mut e = CascadeEvidence {
        per,
        a_first,
        a_ticks,
        planned_a_sends: wave.len().div_ceil(chunk),
        a_sends: Vec::new(),
        t0: None,
        b_first: 0,
        b_handed: None,
        reports: Vec::new(),
        errors: rate_errors.to_vec(),
    };
    if let Err(error) = mt3::wait_until(device, reference) {
        e.errors.push(format!("the wait for A: {error}"));
        return e;
    }
    let plan = Plan { first_at: a_first, per, chunk, head: chunk, eob: true, each: true, first: false, channels: 1 };
    match mt3::send_chunked(device, wave, plan) {
        Ok(sent) => e.a_sends = sent,
        Err(error) => e.errors.push(format!("A: {error}")),
    }
    // The first TimeError at or after A's own first sample is A's: earlier reports are the last trial's leftovers.
    let begun = Instant::now();
    while e.t0.is_none() && begun.elapsed() < Wall::from_secs(2) {
        match device.tx_async(0, Wall::from_millis(20)) {
            Ok(Some(report)) => {
                e.reports.push(report);
                e.t0 = a_time_error(&report, a_first);
            }
            Ok(None) => {}
            Err(error) => {
                e.errors.push(format!("tx_async: {}", error.message));
                break;
            }
        }
    }
    let Some(t0) = e.t0 else { return e };
    e.b_first = align_to(t0 + a_ticks + delta, per);
    match device.tx_send(0, &[&wave[..chunk]], Some(e.b_first), true, true, Wall::from_millis(500)) {
        Ok(_) => match device.time_now() {
            Ok(handed) => e.b_handed = Some(handed),
            Err(error) => e.errors.push(format!("time_now after B: {}", error.message)),
        },
        Err(error) => e.errors.push(format!("B: {}", error.message)),
    }
    if let Err(error) = mt3::wait_until(device, e.b_first + chunk as i64 * per + 2 * TICKS_PER_MS) {
        e.errors.push(format!("the wait for B's reports: {error}"));
    }
    let more = mt3::read_reports(device, &mut e.errors);
    e.reports.extend(more);
    e
}

/// One trial of the sweep: its nominal delta, B's delta from `t0 + S x per` on the grid (`None` when B was not sent), what it read and what B did.
pub struct CascadeTrial {
    pub delta: i64,
    pub trial: usize,
    pub evidence: CascadeEvidence,
}

impl CascadeTrial {
    pub fn outcome(&self) -> Outcome {
        cascade_outcome(&self.evidence)
    }

    /// B's delta from `t0 + S x per` after it was aligned up to the sample grid.
    pub fn effective_delta(&self) -> Option<i64> {
        self.evidence.t0.map(|t0| self.evidence.b_first - (t0 + self.evidence.a_ticks))
    }

    /// The trial as the bench logs it.
    pub fn trace(&self, rate: f64) -> Json {
        let e = &self.evidence;
        json!({
            "rate_hz": rate, "delta_ticks": self.delta, "trial": self.trial, "effective_delta_ticks": self.effective_delta(), "t0": e.t0, "b_first": e.b_first, "b_handed": e.b_handed,
            "a_first": e.a_first, "a_ticks": e.a_ticks, "a_sends": e.a_sends.iter().map(|send| [send.at, send.handed]).collect::<Vec<_>>(), "planned_a_sends": e.planned_a_sends,
            "a_min_margin_ticks": e.t0.and_then(|t0| e.a_sends.iter().skip(1).map(|send| t0 + (send.at - e.a_first) - send.handed).min()),
            "errors": e.errors, "reports": e.reports.iter().map(|r| json!([format!("{:?}", r.code), r.tick])).collect::<Vec<_>>(),
        })
    }
}

/// The sweep at `rate` on `device`: every delta of `deltas`, `trials` times each (interleaved: one pass over the deltas a trial, so a slow drift of the host does not
/// land on one delta), opening its transmit streamer (1 GHz, 0 dB, `raw_tx_on` refuses anything outside bench.md's RF envelope) and closing it again. No receive.
/// The caller is the only driver of the device's clock while it runs.
pub fn cascade_cell(device: &dyn Device, rate: f64, deltas: &[i64], trials: usize) -> Vec<CascadeTrial> {
    let _driver = device.host().drive();
    let errors = mt3::open_tx_checked(device, rate);
    let chunk = mt3::chunk_at(rate);
    let wave = mt3::tone_burst(rate, mt3::burst_samples(rate));
    let out = (0..trials)
        .flat_map(|trial| deltas.iter().map(move |&delta| (trial, delta)))
        .map(|(trial, delta)| CascadeTrial { delta, trial, evidence: cascade_trial(device, rate, &wave, chunk, delta, &errors) })
        .collect();
    device.tx_close(0);
    out
}

/// The cells of a sweep, by delta ascending.
pub fn cells_of(trials: &[CascadeTrial]) -> Vec<Cell> {
    let mut deltas: Vec<i64> = trials.iter().map(|trial| trial.delta).collect();
    deltas.sort_unstable();
    deltas.dedup();
    let of = |delta: i64| trials.iter().filter(|trial| trial.delta == delta).map(|trial| (trial.effective_delta().unwrap_or(delta), trial.outcome())).collect::<Vec<_>>();
    deltas.into_iter().map(|delta| Cell::of(delta, &of(delta))).collect()
}

/// The fake's limit for A at `rate` as the cell sends it: `S` samples in `S` over the chunk sends, one closed burst.
pub fn planned_limit(rate: f64) -> i64 {
    fake_limit_ticks(MCR / rate as i64, mt3::burst_samples(rate), mt3::burst_samples(rate).div_ceil(mt3::chunk_at(rate)), 1)
}

/// The bench's grid at `rate`: `grid` around the fake's limit for A as the cell sends it.
pub fn planned_grid(rate: f64) -> Vec<i64> {
    grid(planned_limit(rate), MCR / rate as i64)
}

/// Runs the sweep over `deltas` (the bench passes `planned_grid(rate)`) and prints its lines: `MT3 C4 trial` a trial, `MT3 C4 cell` a delta, `MT3 C4 boundary`, then
/// `VERDICT mt3 c4`; returns the cells and `Some(text)` when the sweep is not PASS.
pub fn report_sweep(device: &dyn Device, rate: f64, deltas: &[i64], trials: usize) -> (Vec<Cell>, Option<String>) {
    let limit = planned_limit(rate);
    println!(
        "MT3 C4 {} MS/s: the fake's limit for A is {limit} ticks ({} us) past its drain end; t0 is A's first TimeError tick (the radio's time check of A's first packet, a measurement coordinate), B is handed at once, an INFERRED 2 ms margin before t0 + S x per",
        rate / 1e6,
        limit / US
    );
    let run = cascade_cell(device, rate, deltas, trials);
    for trial in &run {
        println!("MT3 C4 trial {} MS/s delta {} us #{}: {:?} {}", rate / 1e6, trial.delta as f64 / US as f64, trial.trial, trial.outcome(), trial.trace(rate));
    }
    let cells = cells_of(&run);
    for cell in &cells {
        println!("MT3 C4 cell {} MS/s delta {}", rate / 1e6, cell.line());
    }
    println!("MT3 C4 boundary {} MS/s: {}", rate / 1e6, boundary_summary(&cells));
    let verdict = sweep_verdict(&cells, limit, uncertainty_ticks(MCR / rate as i64));
    println!("VERDICT mt3 c4 {} MS/s: {verdict:?}", rate / 1e6);
    let failed = (!matches!(verdict, Verdict::Pass(_))).then(|| format!("{} MS/s: {verdict:?}", rate / 1e6));
    (cells, failed)
}

#[cfg(test)]
mod tests {
    use super::super::mt3::tests::report;
    use super::*;

    const PER: i64 = 200;
    const T0: i64 = 13_000_000;
    const A_FIRST: i64 = 12_800_000;
    const A_TICKS: i64 = 4_000_000;

    /// 1 MS/s, A of 8 sends of 2 500 samples (500 000 ticks) handed as the window wait hands them (the later ones 10 ms ahead of their own time), t0 where the radio found
    /// the first late, B timed 100 000 ticks past the drain's end and handed at once, a TimeError for each of A's sends and B dropped.
    fn dropped() -> CascadeEvidence {
        let a_sends = (0..8).map(|k| Sent { at: A_FIRST + k * 500_000, handed: if k == 0 { T0 - 100_000 } else { T0 + 100_000 } }).collect();
        let b_first = T0 + A_TICKS + 100_000;
        CascadeEvidence {
            per: PER,
            a_first: A_FIRST,
            a_ticks: A_TICKS,
            planned_a_sends: 8,
            a_sends,
            t0: Some(T0),
            b_first,
            b_handed: Some(T0 + 1_000_000),
            reports: (0..8).map(|k| report(TxCode::TimeError, Some(T0 + k * 500_000))).chain([report(TxCode::TimeError, Some(b_first))]).collect(),
            errors: Vec::new(),
        }
    }

    fn played() -> CascadeEvidence {
        let mut e = dropped();
        e.reports.pop();
        e.reports.push(report(TxCode::BurstAck, Some(e.b_first + 500_000 + 1)));
        e
    }

    fn inconclusive(e: &CascadeEvidence, what: &str) {
        let outcome = cascade_outcome(e);
        assert!(matches!(outcome, Outcome::Inconclusive(_)), "{what}: {outcome:?}");
    }

    #[test]
    fn mt3_c4_outcome_a_timed_error_at_b_is_dropped_and_an_ack_at_b_is_played() {
        assert_eq!(cascade_outcome(&dropped()), Outcome::Dropped);
        assert_eq!(cascade_outcome(&played()), Outcome::Played);
        // B's TimeError carries a tick at or after B's first sample, whatever the drain's own ticks were: at B.first exactly counts.
        let mut at = dropped();
        let b_first = at.b_first;
        at.reports.last_mut().unwrap().tick = Some(b_first);
        assert_eq!(cascade_outcome(&at), Outcome::Dropped);
        let mut before = dropped();
        before.reports.last_mut().unwrap().tick = Some(b_first - 1);
        inconclusive(&before, "a TimeError one tick before B's first sample is A's, so B has no report");
        let mut ack_at = played();
        ack_at.reports.last_mut().unwrap().tick = Some(b_first);
        assert_eq!(cascade_outcome(&ack_at), Outcome::Played, "an ACK at B.first counts");
        let mut both = played();
        both.reports.push(report(TxCode::TimeError, Some(b_first + 1)));
        inconclusive(&both, "a BurstAck and a TimeError both for B");
        let mut neither = dropped();
        neither.reports.pop();
        inconclusive(&neither, "nothing reported for B");
    }

    #[test]
    fn mt3_c4_outcome_without_a_time_error_for_a_there_is_no_drain() {
        let mut none = dropped();
        none.t0 = None;
        none.b_handed = None;
        inconclusive(&none, "A drew no TimeError");
        let mut failed = dropped();
        failed.errors = vec!["B: the stream is not open".to_owned()];
        inconclusive(&failed, "a failed call");
        let mut a_acked = dropped();
        a_acked.reports.insert(0, report(TxCode::BurstAck, Some(T0)));
        inconclusive(&a_acked, "a BurstAck before B's first sample: A is never acknowledged");
    }

    #[test]
    fn mt3_c4_outcome_a_stalled_host_or_a_late_b_is_inconclusive() {
        // The drain reaches A's send 3 at t0 + 1 500 000: handed exactly 1 ms (200 000 ticks) before is enough, 1 tick short is not.
        let mut enough = dropped();
        enough.a_sends[3].handed = T0 + 1_500_000 - 200_000;
        assert_eq!(cascade_outcome(&enough), Outcome::Dropped);
        let mut short = dropped();
        short.a_sends[3].handed = T0 + 1_500_000 - 200_000 + 1;
        inconclusive(&short, "a send of A handed 1 tick under 1 ms before the drain reached it");
        let mut first = dropped();
        first.a_sends[0].handed = T0 + 10;
        assert_eq!(cascade_outcome(&first), Outcome::Dropped, "the first send makes t0: it is not held to the margin");
        let mut extra = dropped();
        extra.a_sends.push(Sent { at: A_FIRST + 4_000_000, handed: T0 });
        inconclusive(&extra, "a ninth send (a short one): the fake counts a packet a send");
        // B must be handed 2 ms (400 000 ticks) before t0 + S x per.
        let mut edge = dropped();
        edge.b_handed = Some(T0 + A_TICKS - 400_000);
        assert_eq!(cascade_outcome(&edge), Outcome::Dropped);
        let mut late = dropped();
        late.b_handed = Some(T0 + A_TICKS - 400_000 + 1);
        inconclusive(&late, "B handed 1 tick under 2 ms before the drain's end");
        let mut not_handed = dropped();
        not_handed.b_handed = None;
        inconclusive(&not_handed, "B was not handed");
    }

    #[test]
    fn mt3_c4_outcome_an_underflow_a_sequence_error_or_a_tickless_report_is_inconclusive() {
        for code in [TxCode::Underflow, TxCode::UnderflowInPacket, TxCode::SeqError, TxCode::SeqErrorInBurst] {
            let mut e = dropped();
            e.reports.insert(2, report(code, Some(T0)));
            inconclusive(&e, &format!("{code:?}: a partly played B must not count as dropped"));
        }
        let mut tickless = dropped();
        tickless.reports.insert(2, report(TxCode::TimeError, None));
        inconclusive(&tickless, "a TimeError without a tick");
        let mut tickless_ack = played();
        tickless_ack.reports.insert(2, report(TxCode::BurstAck, None));
        inconclusive(&tickless_ack, "a BurstAck without a tick");
    }

    #[test]
    fn mt3_c4_t0_is_a_time_error_at_or_after_a_s_first_sample() {
        assert_eq!(a_time_error(&report(TxCode::TimeError, Some(A_FIRST)), A_FIRST), Some(A_FIRST));
        assert_eq!(a_time_error(&report(TxCode::TimeError, Some(A_FIRST - 1)), A_FIRST), None, "the last trial's leftover");
        assert_eq!(a_time_error(&report(TxCode::BurstAck, Some(A_FIRST + 10)), A_FIRST), None);
        assert_eq!(a_time_error(&report(TxCode::Underflow, Some(A_FIRST + 10)), A_FIRST), None);
        assert_eq!(a_time_error(&report(TxCode::TimeError, None), A_FIRST), None);
    }

    #[test]
    fn mt3_c4_the_limit_is_the_fakes_formula() {
        // The lib test `ur_33_the_fake_s_drain_pad_counts_two_samples_per_packet` (device.rs): 20 000 samples in 200 sends of one burst at 1 MS/s, limit +640 samples.
        assert_eq!(fake_limit_ticks(200, 20_000, 200, 1), 640 * 200);
        // Eight chunks of 2 500: a pad of 200 + 16 + (15 + 10) and a guard of 15, +256 samples (`assert_drain_band(&[2_500; 8], 248, 256)`).
        assert_eq!(fake_limit_ticks(200, 20_000, 8, 1), 256 * 200);
        // 10 MS/s (20 ticks a sample): the guard is 150 samples, the DUC's wait 100, a pad of 2 000 + 20 + 250, +2 420 samples.
        assert_eq!(fake_limit_ticks(20, 200_000, 10, 1), 2_420 * 20);
        assert_eq!([1e6, 10e6].map(planned_limit), [256 * 200, 2_420 * 20]);
        assert_eq!(uncertainty_ticks(200), 204);
    }

    #[test]
    fn mt3_c4_the_grid_is_the_stated_one() {
        let limit = 256 * 200;
        let deltas = grid(limit, PER);
        assert_eq!(deltas.len(), 4 + 31 + 3);
        assert_eq!(&deltas[..3], &[-20 * US, -10 * US, 0]);
        assert!([-20, -10, 0, 1, 15, 30, 50, 100].iter().all(|us| deltas.contains(&(us * US))), "{deltas:?}");
        assert!(deltas.contains(&limit) && deltas.contains(&(limit + 20 * US)));
        assert!(deltas.contains(&(limit + 204)), "the first delta at which a dropped B fails the sweep: the limit plus a sample and 4 clocks");
        assert_eq!(grid(limit, 20).iter().filter(|delta| **delta > limit && **delta < limit + 20 * US).count(), 1, "at 10 MS/s the uncertainty is 24 ticks");
        assert!(grid(limit, 20).contains(&(limit + 24)));
        assert!(deltas.windows(2).all(|pair| pair[0] < pair[1]), "ascending and without a repeat");
        assert_eq!(grid(50 * US, PER).len(), 4 + 31 + 2, "a limit that is a grid point is not repeated");
    }

    /// A cell whose trials sit at exactly the nominal delta (no alignment).
    fn cell(delta_us: i64, played: usize, dropped: usize, inconclusive: usize) -> Cell {
        cell_at(delta_us * US, played, dropped, inconclusive)
    }

    fn cell_at(delta: i64, played: usize, dropped: usize, inconclusive: usize) -> Cell {
        Cell { delta, played, dropped, inconclusive, dropped_at: vec![delta; dropped] }
    }

    const LIMIT: i64 = 256 * US;
    const UNC: i64 = 204;

    /// A sweep whose device ends its drain near 10 us: dropped before, played after, one mixed cell; the cells at the limit and past it all played.
    fn passing_sweep() -> Vec<Cell> {
        vec![cell(-20, 0, 10, 0), cell(0, 0, 10, 0), cell(10, 4, 6, 0), cell(20, 10, 0, 0), cell(100, 10, 0, 0), cell(256, 10, 0, 0), cell(276, 10, 0, 0)]
    }

    #[test]
    fn mt3_c4_sweep_passes_when_the_device_ends_its_drain_before_the_fakes_limit() {
        let verdict = sweep_verdict(&passing_sweep(), LIMIT, UNC);
        assert!(matches!(verdict, Verdict::Pass(_)), "{verdict:?}");
        // The deltas below the limit are measurements: dropped trials there are no term, whatever their number.
        let mut below = passing_sweep();
        below[4] = cell(100, 0, 10, 0);
        assert!(matches!(sweep_verdict(&below, LIMIT, UNC), Verdict::Pass(_)), "a device that still drops at 100 us has a drain shorter than the fake's limit");
    }

    #[test]
    fn mt3_c4_sweep_fails_when_b_is_dropped_past_the_fakes_limit_and_the_uncertainty() {
        let mut past = passing_sweep();
        past[6] = cell(276, 9, 1, 0);
        assert!(matches!(sweep_verdict(&past, LIMIT, UNC), Verdict::Fail(_)), "one drop 20 us past the limit");
        // Exactly the limit plus the uncertainty fails; one tick under it is inside the uncertainty and is counted, not failed.
        let edge = |delta: i64| {
            let mut cells = passing_sweep();
            cells[5] = cell_at(delta, 9, 1, 0);
            sweep_verdict(&cells, LIMIT, UNC)
        };
        assert!(matches!(edge(LIMIT + UNC), Verdict::Fail(_)));
        let inside = edge(LIMIT + UNC - 1);
        assert!(matches!(&inside, Verdict::Pass(text) if text.contains("1 dropped trial(s) inside")), "{inside:?}");
        let at_limit = edge(LIMIT);
        assert!(matches!(&at_limit, Verdict::Pass(text) if text.contains("1 dropped trial(s) inside")), "{at_limit:?}");
        // A FAIL stands whatever else is thin.
        let mut thin_and_failing = past.clone();
        thin_and_failing[5] = cell(256, 1, 0, 9);
        assert!(matches!(sweep_verdict(&thin_and_failing, LIMIT, UNC), Verdict::Fail(_)));
    }

    #[test]
    fn mt3_c4_sweep_judges_a_trial_by_its_effective_delta() {
        // The cell's nominal delta is the limit (a drop there is inside the uncertainty), but one trial's B was aligned up to the limit plus the uncertainty: that drop fails.
        let mut cells = passing_sweep();
        cells[5].dropped = 1;
        cells[5].played = 9;
        cells[5].dropped_at = vec![LIMIT + UNC];
        assert!(matches!(sweep_verdict(&cells, LIMIT, UNC), Verdict::Fail(_)));
        cells[5].dropped_at = vec![LIMIT + UNC - 1];
        let inside = sweep_verdict(&cells, LIMIT, UNC);
        assert!(matches!(&inside, Verdict::Pass(text) if text.contains("1 dropped trial(s) inside")), "{inside:?}");
        // A drop whose effective delta is below the limit (a nominal delta of the limit cannot be, but a cell can mix) is a measurement, not counted inside.
        cells[5].dropped_at = vec![LIMIT - 1];
        let below = sweep_verdict(&cells, LIMIT, UNC);
        assert!(matches!(&below, Verdict::Pass(text) if text.contains("0 dropped trial(s) inside")), "{below:?}");
    }

    #[test]
    fn mt3_c4_sweep_needs_ten_decided_trials_at_and_past_the_limit() {
        let mut thin = passing_sweep();
        thin[5] = cell(256, 5, 4, 1);
        assert!(matches!(sweep_verdict(&thin, LIMIT, UNC), Verdict::Inconclusive(_)), "9 decided at the limit");
        let mut thin_past = passing_sweep();
        thin_past[6] = cell(276, 3, 0, 7);
        assert!(matches!(sweep_verdict(&thin_past, LIMIT, UNC), Verdict::Inconclusive(_)), "3 decided past the limit");
        let mut exact = passing_sweep();
        exact[6] = cell(276, 6, 0, 0);
        assert!(matches!(sweep_verdict(&exact, LIMIT, UNC), Verdict::Inconclusive(_)), "6 trials in all");
        let mut ten = passing_sweep();
        ten[5] = cell(256, 6, 4, 0);
        assert!(matches!(sweep_verdict(&ten, LIMIT, UNC), Verdict::Pass(_)), "10 decided trials at the limit are enough, and a drop inside the uncertainty is not a FAIL");
        let mut below_thin = passing_sweep();
        below_thin[0] = cell(-20, 0, 1, 9);
        assert!(matches!(sweep_verdict(&below_thin, LIMIT, UNC), Verdict::Pass(_)), "a thin cell below the limit is only a measurement");
        assert!(matches!(sweep_verdict(&passing_sweep()[..5], LIMIT, UNC), Verdict::Inconclusive(_)), "no delta reaches the limit");
        assert!(matches!(sweep_verdict(&[], LIMIT, UNC), Verdict::Inconclusive(_)));
    }

    #[test]
    fn mt3_c4_the_boundary_summary_names_the_bracket_and_the_mixed_cells() {
        let summary = boundary_summary(&passing_sweep());
        assert!(summary.starts_with("last all-dropped delta 0 us, first all-played delta 20 us, mixed cells [\"10 us: 4 played, 6 dropped, 0 inconclusive\"]"), "{summary}");
        let none = boundary_summary(&[cell(0, 0, 0, 10)]);
        assert_eq!(none, "last all-dropped delta none, first all-played delta none, mixed cells []");
        // An all-played cell below the last all-dropped one is not the bracket's upper end.
        let odd = boundary_summary(&[cell(-20, 10, 0, 0), cell(0, 0, 10, 0), cell(20, 10, 0, 0)]);
        assert!(odd.starts_with("last all-dropped delta 0 us, first all-played delta 20 us"), "{odd}");
    }

    #[test]
    fn mt3_c4_cells_count_the_outcomes_of_a_delta() {
        let trials = [(5, Outcome::Played), (6, Outcome::Dropped), (7, Outcome::Dropped), (5, Outcome::Inconclusive("x".to_owned()))];
        assert_eq!(Cell::of(5, &trials), Cell { delta: 5, played: 1, dropped: 2, inconclusive: 1, dropped_at: vec![6, 7] });
    }
}
