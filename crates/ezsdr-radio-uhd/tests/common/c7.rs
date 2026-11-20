//! bt2's cell C7 (`plan/phase8/bench.md`): a repeat whose device burst is dropped (its first send reaches the device late, or the device starves mid-play)
//! resumes at `R = first lattice point >= max(busy + m, now + lead)` (UR-22, #106). One trial body, `trial`, runs in a Session through the real Module, on the
//! USRP (`hardware.rs`) and on `FakeDevice` (`fake.rs`: AGENTS §9, a hardware test whose expectation comes from a rule has a rehearsal). The expectation
//! of `R` is `expected_r`, a restatement of the Module's rule that reads only what the Session itself did (the probe's rows), with the test's own `m`: a hook the
//! Module ignores, or a hold that does not move `busy`, fails an exact comparison of every row.
//!
//! The verdict table and the RF detector are not here yet (B3); `Trial` carries the evidence they read.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration as Wall;

use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::session::SessionAction;
use ezsdr_kernel::spec::{Scalar, Value};
use ezsdr_kernel::time::TimePoint;
use ezsdr_radio_uhd::Device;
use serde_json::Value as Json;

use super::n47s5::tx_origin_per;
use super::probe_device::{Hold, ProbeDevice, TxHold};
use super::*;

/// UR-22's margin when no hook replaces it (`profile::RESUME_MARGIN_NS`, restated: a change of the constant must fail a test, not be followed).
pub const DEFAULT_MARGIN_NS: i64 = 10_000_000;
/// The device lead (`profile::DEVICE_LEAD_NS`, 2 ms) in nanoseconds.
pub const LEAD_NS: i64 = 2_000_000;
/// The samples of a device burst's close (`segment::CLOSE_SAMPLES`).
pub const CLOSE_SAMPLES: i64 = 2;
/// The capture's length in milliseconds; the first 20 ms are the quiet floor.
pub const CAPTURE_MS: i64 = 360;
/// The accepted Starts after the control that the window follows at most: the cell's first Start, the resume and 7 retries. A window that followed every Start
/// would run an endless drop chain to the deadline and overflow the Module's 1 000-row record bound; a trial that has retried this often is an UNSETTLED datum
/// anyway.
pub const MAX_FOLLOWED_STARTS: usize = 9;

/// The two ways the device burst of a repeat is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The first send reaches the device after the repeat's start: the device drops the whole device burst.
    LateStart,
    /// A send in the middle of the play is held so long that the device runs dry.
    Starved,
}

/// How the stall is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stall {
    /// The held send stays in its call this long (the bench's cells).
    For(Wall),
    /// The held send returns at the instant that puts `busy + m` `side_ns` past a lattice point of the repeat (a negative `side_ns` is below it), so that R's
    /// rounding is booked on one side of the boundary by construction, whatever instant the Module handed the send over.
    Aligned { side_ns: i64 },
}

#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub mode: Mode,
    pub rate: f64,
    /// The PN's period in microseconds: `FAKE_PERIOD_US` for the rehearsals, `HW_PERIOD_US` for the bench.
    pub period_us: i64,
    /// The hook's margin; `None` builds the Module as a user's Run does (no hook).
    pub margin_ns: Option<i64>,
    pub stall: Stall,
    /// The start lead in microseconds; `None` is the mode's default (6 500 for `LateStart`, 10 000 for `Starved`, which ignores it).
    pub lead_us: Option<i64>,
    /// LateStart only: the second `Start` after arming - the resumed part's - is held this long, so that it reaches the device late and is dropped (S4').
    pub retry_hold: Option<Wall>,
}

impl Cell {
    pub fn margin(&self) -> i64 {
        self.margin_ns.unwrap_or(DEFAULT_MARGIN_NS)
    }
}

/// The PN period of the fake rehearsals, in microseconds: they keep the exact pins at 100 µs (the fake's drain limit is about 70 µs).
pub const FAKE_PERIOD_US: i64 = 100;
/// The PN period of the hardware tests, in microseconds: 1 ms, so that the oracle's mapping uncertainty (0.05 to 0.3 ms) is a small part of a period.
pub const HW_PERIOD_US: i64 = 1_000;

/// The samples of one PN period: `period_us` microseconds at `rate`.
pub fn period(rate: f64, period_us: i64) -> usize {
    (rate * period_us as f64 / 1e6).round() as usize
}

/// `period_us` microseconds of a zero-mean QPSK PN at the rate, peak 0.495 (bench.md's RF safety: at most 0.5), so that a lattice period is `period_us`
/// at every rate and a DC or leakage offset adds nothing to a correlation with it.
pub fn wave(rate: f64, period_us: i64) -> Vec<(f32, f32)> {
    let n = period(rate, period_us);
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let raw: Vec<(f64, f64)> = (0..n)
        .map(|_| {
            let bits = next();
            ((if bits & 1 == 0 { 1.0 } else { -1.0 }), (if bits & 2 == 0 { 1.0 } else { -1.0 }))
        })
        .collect();
    let (mean_i, mean_q) = (raw.iter().map(|s| s.0).sum::<f64>() / n as f64, raw.iter().map(|s| s.1).sum::<f64>() / n as f64);
    let centred: Vec<(f64, f64)> = raw.iter().map(|s| (s.0 - mean_i, s.1 - mean_q)).collect();
    let peak = centred.iter().map(|s| s.0.hypot(s.1)).fold(0.0, f64::max);
    centred.iter().map(|s| ((s.0 / peak * 0.495) as f32, (s.1 / peak * 0.495) as f32)).collect()
}

/// A send cap in samples (`segment::send_cap`): a quarter of the in-flight window, 32..=20 000 samples (F11).
pub fn send_cap(rate: f64) -> usize {
    let per = MCR as f64 / rate;
    ((2_000_000.0 / per / 4.0) as usize).clamp(32, 20_000)
}

// ---------------------------------------------------------------- the rule, restated (pure)

/// One whole send of the Module as the probe saw it, in root ticks: when it returned, when its first sample plays, how long it plays, and how far the
/// Module's own stamp of that return can lie from `returned`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sent {
    pub returned: i64,
    pub first: i64,
    pub ticks: i64,
    /// How far the Module's own stamp of this send's return can lie from `returned`, in root ticks: the probe's mapping uncertainty, plus the Module's
    /// clock read width in effect at that return, plus the delay to the Module's sample, bounded by the next TX send's entry or by the decision.
    pub err: i64,
}

/// `busy <- max(busy, returned + shift, first) + ticks` over the sends in order (`TxBook::note_sent`, UR-22): one recursion, replayed at each stamp's nominal
/// value, its lower bound and its upper bound.
fn shifted_busy_replay(sends: &[Sent], shift: impl Fn(&Sent) -> i64) -> i64 {
    sends.iter().fold(0, |busy, send| busy.max(send.returned + shift(send)).max(send.first) + send.ticks)
}

/// `busy <- max(busy, returned, first) + ticks` over the sends in order (`TxBook::note_sent`, UR-22), at the nominal returns.
pub fn busy_replay(sends: &[Sent]) -> i64 {
    shifted_busy_replay(sends, |_| 0)
}

/// The replay with every send's return at its lower (`returned - err`) and its upper (`returned + err`) bound: the replay is monotone in each return, so a send
/// that does not set `busy` adds nothing at either bound.
pub fn busy_bounds(sends: &[Sent]) -> (i64, i64) {
    (shifted_busy_replay(sends, |send| -send.err), shifted_busy_replay(sends, |send| send.err))
}

/// The `at` tick of every accepted payload Start after the first `skip` of them, in row order: exactly the rows `c7_verdict::evidence` selects for `starts`.
pub fn start_ticks(rows: &[Json], skip: usize) -> Vec<i64> {
    rows.iter()
        .filter(|row| row["call"] == "tx_send" && row["class"] == "start" && row["result"]["accepted_samples"].as_i64().unwrap_or(0) > 0)
        .skip(skip)
        .map(|row| row["at"].as_i64().unwrap())
        .collect()
}

/// The allowance for the probe's mapping, in root ticks (1 µs), added to the measured anchor bracket: on the X310 a bracket is 0.05 to 0.15 ms, so the interval
/// `expected_r` allows follows the measured bracket of each trial.
pub const MAP_TOLERANCE_TICKS: i64 = 200;

/// The half width of the widest clock read the Module accepts, in root ticks (0.5 ms): half of `authority.rs`'s `WIDEST_BRACKET` (1 ms), which a read wider than is
/// discarded. The Module keeps a read's midpoint as its anchor and extrapolates from it, so its error at a decision is at most this half width.
pub const MODULE_READ_HALF_MAX_TICKS: i64 = 100_000;

/// The repeat's lattice on the transmit stream: sample 0 is at root tick `tx_origin`, a sample lasts `per` ticks, and the lattice is `origin + j * period` samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lattice {
    pub tx_origin: i64,
    pub per: i64,
    pub origin: i64,
    pub period: i64,
}

/// The terms of one resume point, root ticks and samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terms {
    /// The nominal `busy` the replay gives.
    pub busy: i64,
    pub unaligned: i64,
    pub decided: i64,
    /// The nominal resume point as a transmit sample index: the first lattice point at or after `max(unaligned, decided)`.
    pub r: i64,
    /// The first lattice point at or after `max(busy_lo + margin, decided)`: the lowest point the sends' stamps allow.
    pub lo: i64,
    /// The first lattice point at or after `max(busy_hi + margin, decided)`: the highest point they allow.
    pub hi: i64,
    /// How far the interval around the nominal `busy` reaches, in root ticks: `max(busy - busy_lo, busy_hi - busy)`, what the datum prints.
    pub uncertainty: i64,
    /// Whether the whole width of the stamps' bounds exceeds one lattice period, so that the allowed set holds several points the oracle cannot tell apart (exactly one
    /// period is resolved): a datum; the check still requires R in `lo..=hi`.
    pub unresolved: bool,
}

impl Terms {
    /// Whether the interval pins one lattice point: the Module's own reading decides, or the mapping's uncertainty does not cross a point.
    pub fn pinned(&self) -> bool {
        self.lo == self.hi
    }

    /// How many lattice points the interval allows.
    pub fn allowed(&self, period: i64) -> i64 {
        (self.hi - self.lo) / period + 1
    }
}

/// `R`: the first point `origin + j * period` (samples) at or after the first sample whose instant is at or after `max(busy + margin, decided + lead)`, with the stream's
/// samples at `tx_origin + k * per` root ticks. `busy` comes from host stamps mapped through the probe's anchor, so it lies between `busy_lo` and `busy_hi`; the lead
/// bound is the Module's own clock and exact. `lo` and `hi` are the same rounding of `busy_lo + m` and `busy_hi + m`; the device's front is left out: it is `H + 5`
/// samples, far below `busy + m` in the cells. When the stamps' whole width exceeds a lattice period the interval holds several points and `unresolved` is set: the
/// oracle cannot tell them apart, and accepts every one of them (and none outside).
pub fn expected_r(busy_lo: i64, busy: i64, busy_hi: i64, margin_ticks: i64, now: i64, lead_ticks: i64, lattice: &Lattice) -> Terms {
    let Lattice { tx_origin, per, origin: lattice_origin, period } = *lattice;
    let (unaligned, decided) = (busy + margin_ticks, now + lead_ticks);
    let at = |ticks: i64| {
        let bound = (ticks - tx_origin + per - 1).div_euclid(per);
        lattice_origin + (bound - lattice_origin + period - 1).div_euclid(period) * period
    };
    let r = at(unaligned.max(decided));
    let lo = at((busy_lo + margin_ticks).max(decided));
    let hi = at((busy_hi + margin_ticks).max(decided));
    let uncertainty = (busy - busy_lo).max(busy_hi - busy);
    let unresolved = busy_hi - busy_lo > period * per;
    Terms { busy, unaligned, decided, r, lo, hi, uncertainty, unresolved }
}

/// The expectation of the resume decided at `at` over the sends its decision counted: the rule restated with the test's margin `m`.
fn terms_of(sends: &[Sent], at: i64, margin_ns: i64, lattice: &Lattice) -> Terms {
    let (lo, hi) = busy_bounds(sends);
    expected_r(lo, busy_replay(sends), hi, ns_to_ticks_ceil(margin_ns), at, ns_to_ticks_ceil(LEAD_NS), lattice)
}

/// The rule's decision on one resume point: `Err` when `resumes_at` is not on the lattice, and `Err` outside `terms.lo..=terms.hi` otherwise. When the stamps' whole
/// width exceeds a period (`terms.unresolved`) the interval holds several points, the oracle cannot say which one is the rule's, and every one of them passes; a point
/// outside the interval never does, whatever the width.
pub fn r_check(terms: &Terms, resumes_at: i64, start: i64, period: i64) -> Result<(), String> {
    if (resumes_at - start).rem_euclid(period) != 0 {
        return Err(format!("R {resumes_at} is not on the lattice (origin {start}, period {period})"));
    }
    if terms.lo <= resumes_at && resumes_at <= terms.hi {
        Ok(())
    } else {
        let away = if resumes_at < terms.lo { (terms.lo - resumes_at) / period } else { (resumes_at - terms.hi) / period };
        Err(format!(
            "R {resumes_at} is not a lattice point in {}..={}, {away} lattice point(s) outside (decided at {}; uncertainty {}): {terms:?}",
            terms.lo, terms.hi, terms.decided, terms.uncertainty
        ))
    }
}

/// A device-time and host-time pair of `ProbeDevice::clock_anchor`: the host stamps of the probe's axis map to root ticks through the pair's midpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub tick: i64,
    pub host_mid_ns: i128,
    /// Half the round trip of the read, in nanoseconds.
    pub bracket_ns: u64,
    /// The largest half round trip of all the reads the anchor was chosen from, in nanoseconds: the best read maps, but the worst is the honest uncertainty.
    pub worst_bracket_ns: u64,
}

/// The largest bracket of `brackets`: the best read maps, but the worst is the honest uncertainty of the mapping.
pub fn worst_bracket(brackets: &[u64]) -> u64 {
    brackets.iter().copied().max().unwrap_or(0)
}

impl Anchor {
    /// The anchor of the smallest round trip of `reads` anchor reads of the probe (the first on a tie), with the largest round trip of them as its worst bracket.
    pub fn read(probe: &ProbeDevice, reads: usize) -> Anchor {
        let anchors: Vec<Anchor> = (0..reads)
            .map(|_| {
                let (tick, before, after) = probe.clock_anchor().expect("the device's time");
                Anchor { tick, host_mid_ns: (before as i128 + after as i128) / 2, bracket_ns: (after - before) / 2, worst_bracket_ns: (after - before) / 2 }
            })
            .collect();
        let worst = worst_bracket(&anchors.iter().map(|anchor| anchor.bracket_ns).collect::<Vec<_>>());
        let mut best = *anchors.iter().min_by_key(|anchor| anchor.bracket_ns).unwrap();
        best.worst_bracket_ns = worst;
        best
    }

    /// The root tick of a stamp of the probe's axis, as the Module's authority extrapolates (floor).
    pub fn map(&self, host_ns: u64) -> i64 {
        self.tick + ((host_ns as i128 - self.host_mid_ns) * MCR as i128).div_euclid(1_000_000_000) as i64
    }

    /// The stamp of the probe's axis of a root tick (nearest nanosecond).
    pub fn host_at(&self, tick: i64) -> u64 {
        (self.host_mid_ns + ((tick - self.tick) as i128 * 1_000_000_000 + MCR as i128 / 2).div_euclid(MCR as i128)).max(0) as u64
    }

    /// The bracket in root ticks (a ceiling).
    pub fn bracket_ticks(&self) -> i64 {
        (self.bracket_ns as i128 * MCR as i128 / 1_000_000_000) as i64 + 1
    }

    /// The worst bracket in root ticks (a ceiling), as `bracket_ticks` for the best one.
    pub fn worst_ticks(&self) -> i64 {
        (self.worst_bracket_ns as i128 * MCR as i128 / 1_000_000_000) as i64 + 1
    }
}

/// A duration in nanoseconds as root ticks, rounded up (a ceiling: a bound at a whole tick).
pub fn ns_to_ticks_ceil(ns: i64) -> i64 {
    ((ns as i128 * MCR as i128 + 999_999_999) / 1_000_000_000) as i64
}

/// The widest read the Module's Authority accepts, in nanoseconds (`authority.rs`'s `WIDEST_BRACKET`, 1 ms): a wider one is discarded. Also the window of the clamp
/// below.
const WIDEST_READ_NS: u64 = 1_000_000;

/// The clock reads of `rows` the Module could have accepted, in row order: `(exit_ns, width_ns)` of every `time_now` read of a `device_call` whose result is a tick and
/// whose whole call, `host_exit_ns - host_entered_ns`, is at most 1 ms. The Authority brackets the whole probe call (`authority.rs` `DeviceTime::read`), so the probe's
/// entry and exit stamps are the bracket, not the inner call's `host_before_ns` and `host_after_ns`; a row without them is not a Module read.
fn module_reads(rows: &[Json]) -> Vec<(u64, u64)> {
    rows.iter()
        .filter(|row| row["call"] == "time_now" && row["purpose"] == "device_call" && row["result"]["value"].is_i64())
        .filter_map(|row| {
            let entered = row["host_entered_ns"].as_u64()?;
            let exit = row["host_exit_ns"].as_u64()?;
            let width = exit.checked_sub(entered)?;
            (width <= WIDEST_READ_NS).then_some((exit, width))
        })
        .collect()
}

/// The half width, in root ticks (a ceiling), of the widest read that bounds the Module's stamp of a send that returned at `returned_ns` (the probe's axis): the read in
/// effect when the window `[returned_ns - 1 ms, returned_ns]` opened and every read that completed in it. No read that completed after the return counts (the Module
/// stamped the return, not a later instant, with what it had). The Module keeps a read's midpoint as its anchor, so a stamp is wrong by at most that read's half width;
/// and `DeviceTime::extrapolate` never lets a stamp fall behind an earlier one (`last.fetch_max`), so a wide read that a narrow one replaced still holds the stamp ahead
/// for up to its own half width (at most 0.5 ms) after the narrow read landed: the one-millisecond window covers it. `MODULE_READ_HALF_MAX_TICKS` when no read qualifies.
fn read_half_ticks(reads: &[(u64, u64)], returned_ns: u64) -> i64 {
    let opened = returned_ns.saturating_sub(WIDEST_READ_NS);
    let in_effect = reads.iter().filter(|(exit, _)| *exit <= opened).max_by_key(|(exit, _)| *exit);
    let inside = reads.iter().filter(|(exit, _)| *exit > opened && *exit <= returned_ns);
    in_effect.into_iter().chain(inside).map(|(_, width)| *width).max().map_or(MODULE_READ_HALF_MAX_TICKS, |width| ((width as i128 * MCR as i128 + 1_999_999_999).div_euclid(2_000_000_000)) as i64)
}

/// The half width, in root ticks (a ceiling), the Module's stamp of a send that returned at `returned_ns` can be wrong by (`read_half_ticks` over `module_reads`).
pub fn module_read_half_ticks(rows: &[Json], returned_ns: u64) -> i64 {
    read_half_ticks(&module_reads(rows), returned_ns)
}

/// The slack of [`history`]'s time comparison: the probe's own uncertainty plus the Module's longest accepted clock read. Wide on purpose: the ordering is decided by the
/// close's ordinal, and the time comparison is only a sanity check.
pub fn history_slack(probe_uncertainty: i64) -> i64 {
    probe_uncertainty + MODULE_READ_HALF_MAX_TICKS
}

/// A probe's accepted non-flush sends in probe order, prepared once for the many resume rows of one Session (a storm keeps a thousand): each send's mapped numbers, the
/// Module's read half width at its return (`read_half_ticks`) and the delay to the next send's entry. `history` takes the prefix a resume row's decision counted.
pub struct Sends {
    rows: Vec<SendRow>,
}

/// One accepted send: whether it is a close, its mapped numbers (`err` not yet set), the Module's read half width at its return and the delay to the next send's entry
/// (`None` for the last send).
struct SendRow {
    close: bool,
    sent: Sent,
    read_half: i64,
    to_next: Option<i64>,
}

impl Sends {
    pub fn new(rows: &[Json], anchor: &Anchor, per: i64) -> Sends {
        let reads = module_reads(rows);
        let accepted: Vec<&Json> = rows.iter().filter(|row| row["call"] == "tx_send" && row["class"] != "flush" && row["result"]["accepted_samples"].as_i64().unwrap_or(0) > 0).collect();
        let entered = |row: &Json| row["host_entered_ns"].as_u64().expect("a tx_send row of the probe has host_entered_ns");
        let rows = accepted
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let returned_ns = row["host_after_ns"].as_u64().unwrap();
                let sent = Sent { returned: anchor.map(returned_ns), first: row["at"].as_i64().unwrap(), ticks: row["result"]["accepted_samples"].as_i64().unwrap() * per, err: 0 };
                SendRow {
                    close: row["class"] == "close",
                    sent,
                    read_half: read_half_ticks(&reads, returned_ns),
                    to_next: accepted.get(index + 1).map(|next| (anchor.map(entered(next)) - sent.returned).max(0)),
                }
            })
            .collect();
        Sends { rows }
    }

    /// The sends the Module's `busy` counted before resume row `i`: up to and including close number `armed_closes + i + 1` (1-based, counting accepted closes from the
    /// start of the probe). A missing close, or one whose mapped return is later than `at + slack`, is ambiguous evidence: the decision cannot follow a close it did not see
    /// yet. The slack is [`history_slack`]: the ordering is decided by the close's ordinal, so this comparison is wide on purpose (it allows the Module's last accepted
    /// clock read before the decision). Each send carries its own stamp error: the probe's uncertainty, the Module's read half width at that return, and the delay to the
    /// Module's own sample, bounded by the next send's entry or, for the close paired with the decision, by the decision.
    pub fn history(&self, armed_closes: usize, i: usize, at: i64, probe_uncertainty: i64) -> Result<Vec<Sent>, String> {
        let slack = history_slack(probe_uncertainty);
        let target = armed_closes + i + 1;
        let Some(end) = self.rows.iter().enumerate().filter(|(_, row)| row.close).nth(target - 1).map(|(index, _)| index) else {
            let seen = self.rows.iter().filter(|row| row.close).count();
            return Err(format!("ambiguous: resume row {i} has no close number {target} in the probe ({seen} accepted closes)"));
        };
        if self.rows[end].sent.returned > at + slack {
            return Err(format!("ambiguous: resume row {i}'s close returned at {}, later than its decision {at} plus the slack {slack}", self.rows[end].sent.returned));
        }
        Ok(self.rows[..=end]
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let offset = if index == end { (at - row.sent.returned).max(0) } else { row.to_next.unwrap_or(0) };
                Sent { err: probe_uncertainty + row.read_half + offset, ..row.sent }
            })
            .collect())
    }
}

/// `Sends::new(rows, ..).history(..)`: the sends of one resume row of a probe's rows, for a caller that needs one.
pub fn history(rows: &[Json], anchor: &Anchor, armed_closes: usize, i: usize, at: i64, probe_uncertainty: i64, per: i64) -> Result<Vec<Sent>, String> {
    Sends::new(rows, anchor, per).history(armed_closes, i, at, probe_uncertainty)
}

// ---------------------------------------------------------------- the trial

/// What a finished Session leaves that both a cell trial and a storm read: the Manifest, the probe's rows and the rule's terms.
pub struct Common {
    pub manifest: Manifest,
    pub probe: Vec<Json>,
    pub anchor: Anchor,
    /// The anchor read after the Run's last action; `probe_uncertainty` takes the larger worst bracket of the two and the drift between them.
    pub anchor_after: Anchor,
    /// The root tick of transmit sample 0 and the root ticks a sample (`n47s5::tx_origin_per`).
    pub tx_origin: i64,
    pub per: i64,
    /// The cell's repeat: its first start (a transmit sample, also its lattice's origin) and its period in samples.
    pub start: i64,
    pub period: i64,
    /// The PN's period in microseconds (the cell's, or the storm's argument).
    pub period_us: i64,
    /// The root tick of the capture's first sample (+0 ms).
    pub c0: i64,
    /// The Run's root time at the instant the final Stop was submitted.
    pub stop_tick: i64,
    /// How many accepted close rows the probe held when the hold was armed: resume row `i` pairs with the close after them.
    pub armed_closes: usize,
    /// How many accepted payload Start rows the probe held when the hold was armed: the control's own start, which `starts` drops.
    pub control_starts: usize,
    /// The `at` tick of the control repeat's first accepted start, taken when the hold was armed.
    pub control: Option<i64>,
    /// Evidence a complete trial needed and did not have; empty for a trial the rule can judge.
    pub incomplete: Vec<String>,
    /// `probe.rows_complete()` after the Run finished.
    pub rows_complete: Result<(), String>,
    /// `ProbeDevice::dropped_rows()` after the Run finished: rows the probe's record could not keep.
    pub probe_dropped: usize,
    /// The cell repeat's booking in the Session's action log, when it was booked: a refused one (a start too near its lead) is evidence, not a trial of the rule.
    pub booking: Option<ezsdr_kernel::session::LogEntry>,
}

/// What a trial leaves: the common record and the cell it ran.
pub struct Trial {
    pub cell: Cell,
    pub common: Common,
    /// Aligned stall only: the lattice point the stall was aimed at (a transmit sample) and the side asked.
    pub aimed: Option<(i64, i64)>,
}

impl Deref for Trial {
    type Target = Common;

    fn deref(&self) -> &Common {
        &self.common
    }
}

/// What a storm leaves: the common record and its capture's length in milliseconds (a storm's capture is longer than a cell's).
pub struct Storm {
    pub common: Common,
    pub capture_ms: i64,
}

impl Deref for Storm {
    type Target = Common;

    fn deref(&self) -> &Common {
        &self.common
    }
}

/// A `tx_resume` row of the Module's `timing` section: `from`, `resumes_at` in transmit samples, `at` the decision instant in root ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resume {
    pub cause: String,
    pub from: i64,
    pub resumes_at: i64,
    pub at: i64,
}

impl Common {
    /// The observed lattice origin: the first accepted payload Start after the control, as a transmit sample. The booked `start` can be moved by
    /// `SendAsapAndFlag` (`provider/tx.rs` `dispatch_start`), and the Module rounds R on the moved booking's lattice; `start` when no payload Start was accepted.
    pub fn origin(&self) -> i64 {
        start_ticks(&self.probe, self.control_starts).first().map_or(self.start, |at| (at - self.tx_origin).div_euclid(self.per))
    }

    pub fn resumes(&self) -> Vec<Resume> {
        let Some(rows) = self.manifest.section(&usrp(), "ezsdr.radio.uhd.timing").and_then(|section| section.as_array()) else { return Vec::new() };
        rows.iter()
            .filter(|row| row["what"] == "tx_resume")
            .map(|row| Resume {
                cause: row["cause"].as_str().unwrap().to_owned(),
                from: row["from"].as_i64().unwrap(),
                resumes_at: row["resumes_at"].as_i64().unwrap(),
                at: row["at"].as_i64().unwrap(),
            })
            .collect()
    }

    /// The margins the Manifest records (a `timing` row `resume_margin`, written when the builder set one); empty when the section is absent.
    pub fn margin_rows(&self) -> Vec<i64> {
        self.manifest
            .section(&usrp(), "ezsdr.radio.uhd.timing")
            .and_then(|section| section.as_array())
            .map_or_else(Vec::new, |rows| rows.iter().filter(|row| row["what"] == "resume_margin").map(|row| row["ns"].as_i64().unwrap()).collect())
    }

    fn accepted(row: &Json) -> i64 {
        row["result"]["accepted_samples"].as_i64().unwrap_or(0)
    }

    /// The probe's transmit sends of class `class`, in order, after the control (the ones the hold was armed for).
    pub fn sends_of(&self, class: &str) -> Vec<&Json> {
        self.probe.iter().filter(|row| row["call"] == "tx_send" && row["class"] == class).collect()
    }

    /// The cell's device bursts' first sends (the control's is before the armed hold: `held_after_arming`).
    pub fn held(&self) -> Option<&Json> {
        self.probe.iter().find(|row| row["call"] == "tx_send" && row["held_ns"].as_u64().is_some_and(|held| held > 0))
    }

    /// The uncertainty of a host stamp mapped to root ticks: the worst bracket of the two anchors, the drift between them, and the mapping tolerance.
    pub fn probe_uncertainty(&self) -> i64 {
        let bracket = self.anchor.worst_ticks().max(self.anchor_after.worst_ticks());
        let drift = (self.anchor.map(self.anchor_after.host_mid_ns as u64) - self.anchor_after.tick).abs();
        bracket + drift + MAP_TOLERANCE_TICKS
    }

    /// The accepted samples of the held send (a `LateStart` cell's held Start).
    pub fn first_send_samples(&self) -> i64 {
        self.held().map_or(0, Self::accepted)
    }

    /// The reports of the device the Module recorded: `(code, tick)`; empty when the section is absent.
    pub fn reports(&self) -> Vec<(String, Option<i64>)> {
        let Some(rows) = self.manifest.section(&usrp(), "ezsdr.radio.uhd.async").and_then(|section| section.as_array()) else { return Vec::new() };
        rows.iter().filter(|row| row["tx_flush"] != true).map(|row| (row["code"].as_str().unwrap().to_owned(), row["tick"].as_i64())).collect()
    }

    /// The sample indices of the delivered `radio.TIME_ERROR` and `radio.TX_UNDERFLOW` events that carry `resumes_at` (a repeat that resumed).
    pub fn event_resume_points(&self) -> Vec<i64> {
        let mut points = Vec::new();
        for kind in ["radio.TIME_ERROR", "radio.TX_UNDERFLOW"] {
            for event in events_of(&self.manifest, kind) {
                let Some(value) = event.payload.get("resumes_at").filter(|value| !value.is_null()) else { continue };
                let point: TimePoint = serde_json::from_value(value.clone()).unwrap();
                points.push(point.ticks_in(point.domain()).unwrap());
            }
        }
        points
    }

    /// The sends the Module's `busy` counted before resume row `i`'s decision (T3's flush apart), in probe order up to its own close, each with its stamp error.
    pub fn sent_for_row(&self, i: usize, row: &Resume) -> Result<Vec<Sent>, String> {
        history(&self.probe, &self.anchor, self.armed_closes, i, row.at, self.probe_uncertainty(), self.per)
    }

    /// The expectation of resume row `i`: the rule restated over the probe's own rows and the test's margin `m`.
    pub fn expected(&self, i: usize, row: &Resume, margin_ns: i64) -> Result<Terms, String> {
        let sends = self.sent_for_row(i, row)?;
        Ok(self.expected_from(&sends, row, margin_ns))
    }

    /// The repeat's lattice on the transmit stream, at the observed origin.
    fn lattice(&self) -> Lattice {
        Lattice { tx_origin: self.tx_origin, per: self.per, origin: self.origin(), period: self.period }
    }

    /// The expectation of `row` over sends already gathered for its decision (`sent_for_row`).
    pub fn expected_from(&self, sends: &[Sent], row: &Resume, margin_ns: i64) -> Terms {
        terms_of(sends, row.at, margin_ns, &self.lattice())
    }

    /// `Ok` when the Module's resume point `row.resumes_at` is a lattice point of this Session within the rule's interval `lo..=hi` with the test's margin `m` (when the
    /// stamps' whole width exceeds a period the interval holds several points and any of them passes).
    pub fn check(&self, i: usize, row: &Resume, margin_ns: i64) -> Result<Terms, String> {
        let terms = self.expected(i, row, margin_ns)?;
        r_check(&terms, row.resumes_at, self.origin(), self.period)?;
        Ok(terms)
    }

    /// `check` of every row of `rows` (row `i` of the Module's record is `rows[i]`), the probe's sends and the lattice prepared once: a storm keeps a thousand rows.
    pub fn checks(&self, rows: &[Resume], margin_ns: i64) -> Vec<Result<(), String>> {
        let (sends, lattice, uncertainty) = (Sends::new(&self.probe, &self.anchor, self.per), self.lattice(), self.probe_uncertainty());
        rows.iter()
            .enumerate()
            .map(|(i, row)| {
                let terms = terms_of(&sends.history(self.armed_closes, i, row.at, uncertainty)?, row.at, margin_ns, &lattice);
                r_check(&terms, row.resumes_at, lattice.origin, self.period)
            })
            .collect()
    }
}

impl Trial {
    /// Whether R is set by `busy + m` and not by the lead: a whole lattice period or more past the lead bound.
    pub fn m_bound(&self, row: &Resume, terms: &Terms) -> bool {
        (self.tx_origin + row.resumes_at * self.per) - terms.decided >= self.period * self.per
    }

    /// Whether the trial can tell `m` from the lead: its first send is a full chunk and R is m-bound.
    pub fn m_informative(&self, row: &Resume, terms: &Terms) -> bool {
        self.first_send_samples() == send_cap(self.cell.rate) as i64 && self.m_bound(row, terms)
    }
}

fn hold_of(cell: &Cell, anchor: &Anchor, until_tick: Option<i64>) -> Hold {
    match (cell.stall, until_tick) {
        (Stall::For(wall), _) => Hold::For(wall),
        (Stall::Aligned { .. }, Some(tick)) => Hold::Until(anchor.host_at(tick)),
        (Stall::Aligned { .. }, None) => unreachable!("an aligned stall is planned from the repeat's lattice"),
    }
}

fn to(run: &mut Run, tick: i64) {
    let now = now_root_of(run);
    let _ = run.advance_to(after_ticks(run, tick - now));
}

fn now_root_of(run: &Run) -> i64 {
    let now = run.now();
    now.ticks_in(now.domain()).unwrap()
}

fn stop(run: &mut Run) {
    let _ = run.submit(SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None);
}

fn us(n: i64) -> i64 {
    n * MCR / 1_000_000
}

/// The first half of a trial, for the cell trial and the storm: the Run, its capture (the first `capture_ms`) booked 2 ms ahead, a control repeat playing 25 ms to
/// 80 ms (what the RF decisions of B3 take their delay and gain from), stopped at +80 ms, and the counts and instants the cell's own repeat is booked from.
pub struct Opened {
    pub run: Run,
    pub anchor: Anchor,
    pub wave: Vec<u8>,
    pub cap: i64,
    pub tx_origin: i64,
    pub c0: i64,
    pub period: i64,
    pub per: i64,
    pub incomplete: Vec<String>,
    pub armed_closes: usize,
    pub control: Option<i64>,
    pub control_starts: usize,
}

/// The capacity of the capture's data link in a C7 Session, in blocks: the link in front of the capture sink drops its oldest block when it is full (`drop_oldest`), and
/// a sink thread that a loaded host starves for longer than the link holds leaves a hole in the capture, which the verdict reports as such. A block is 2 000 samples
/// (`profile::DEFAULT_BLOCK_LEN`), 200 us at 10 MS/s, so 1 024 blocks hold 204.8 ms at 10 MS/s (2.05 s at 1 MS/s), more than the 200 ms stall the rehearsals are asked to
/// survive, against the profile's 64 blocks (12.8 ms at 10 MS/s). The memory is 1 024 x 2 000 samples x 8 bytes (two `f32`), 16.4 MB at most, on the bench as on the fake.
pub const CAPTURE_LINK_CAPACITY: u64 = 1_024;

/// `profile` with its capture binding's feed capacity (when it has a feed) raised to `CAPTURE_LINK_CAPACITY`.
fn with_capture_capacity(profile: &Json) -> Json {
    let mut profile = profile.clone();
    if let Some(feed) = profile["bindings"]["rec"].get_mut("feed") {
        feed["capacity"] = serde_json::json!(CAPTURE_LINK_CAPACITY);
    }
    profile
}

/// Opens the Run of a trial on `probe` and plays and stops the control repeat; a missing control close or a refused submit is recorded in `incomplete`, and the
/// caller still books its repeat, waits and finishes, so that its evidence exists.
fn open(profile: &Json, probe: &Arc<ProbeDevice>, margin: Option<i64>, rate: f64, period_us: i64, capture_ms: i64) -> Opened {
    let profile = &with_capture_capacity(profile);
    let initial = with_initial(rx0_tx0(profile), &[("radio.tx.s0.sample_rate_hz", Scalar::try_from(rate).unwrap()), ("radio.rx.s0.sample_rate_hz", Scalar::try_from(rate).unwrap())]);
    let device: Arc<dyn Device> = probe.clone();
    let mut run = session_on(
        profile,
        &initial,
        assembly(profile, device, BTreeMap::new(), move |radio| match margin {
            Some(ns) => radio.with_measurement_resume_margin(ns),
            None => radio,
        }),
    );
    past_t0(&mut run, ms(1));
    past_tx_origin(&mut run);
    let anchor = Anchor::read(probe, 3);
    let (period, per) = (period(rate, period_us) as i64, MCR / rate as i64);
    let (wave, cap) = (waveform_of(&wave(rate, period_us)).0, send_cap(rate) as i64);
    let (tx_origin, _) = tx_origin_per(&run);
    let c0 = now_root_of(&run) + ms(2);
    let sample_count = (rate * capture_ms as f64 / 1_000.0) as i64;
    let mut incomplete = Vec::new();
    if let Err(error) = run.submit(verb("capture", output("rec"), Some(after_ticks(&run, c0 - now_root_of(&run))), &[("sink.capture_samples", Value::from(sample_count))]), None) {
        incomplete.push(error.to_string());
    }
    if incomplete.is_empty() {
        // The control: a repeat on time at +25 ms, stopped at +80 ms.
        let control_at = tx_at(&run, c0 + ms(25) - now_root_of(&run));
        if let Err(error) = run.submit(verb("start_repeat", resource("radio/tx/s0"), Some(control_at), &[]), Some(&wave)) {
            incomplete.push(error.to_string());
        }
        to(&mut run, c0 + ms(80));
        stop(&mut run);
        to(&mut run, c0 + ms(88));
        if !probe.rows().iter().any(|row| row["call"] == "tx_send" && row["class"] == "close") {
            incomplete.push("the control's close is not in the probe".to_owned());
        }
    }
    let mut armed_closes = 0;
    let mut control_starts = 0;
    let mut control = None;
    if incomplete.is_empty() {
        let rows = probe.rows();
        armed_closes = rows.iter().filter(|row| row["call"] == "tx_send" && row["class"] == "close" && row["result"]["accepted_samples"].as_i64().unwrap_or(0) > 0).count();
        control_starts = rows.iter().filter(|row| row["call"] == "tx_send" && row["class"] == "start" && row["result"]["accepted_samples"].as_i64().unwrap_or(0) > 0).count();
        control = rows.iter().find(|row| row["call"] == "tx_send" && row["class"] == "start" && row["result"]["accepted_samples"].as_i64().unwrap_or(0) > 0).and_then(|row| row["at"].as_i64());
    }
    Opened { run, anchor, wave, cap, tx_origin, c0, period, per, incomplete, armed_closes, control, control_starts }
}

/// One trial of cell `cell` in a Session on `probe` (a `ProbeDevice` over the device), the Run of `profile`. Times are relative to `c0`, the capture's first sample:
/// the capture (360 ms) is booked 2 ms ahead; a control repeat plays 25 ms to 80 ms; the hold is armed once the control's close is in the probe, and the cell's
/// repeat is booked: a late start `lead_us` ahead of its start (6.5 ms by default, so that its first send is a full chunk) at +130 ms, a starved repeat 10 ms ahead
/// of its start at +90 ms (so that its first send is short and its stall midway, 60 ms after the first start). It runs until the latest accepted Start after the
/// control (the cell's first, then each part after a drop, among the first `MAX_FOLLOWED_STARTS` of them) has had 30 ms (or +300 ms), stops the repeat and reads
/// the Manifest once the capture is written. A
/// missing control close or a refused submit is recorded in `incomplete`; the trial still stops, waits for the capture and finishes, so that its evidence exists.
pub fn trial(profile: &Json, probe: Arc<ProbeDevice>, cell: Cell) -> Trial {
    let Opened { mut run, anchor, wave, cap, tx_origin, c0, period, per, mut incomplete, armed_closes, control, control_starts } =
        open(profile, &probe, cell.margin_ns, cell.rate, cell.period_us, CAPTURE_MS);
    // The cell's repeat, and the stall armed for it.
    let (book_at, lead, aim) = match cell.mode {
        Mode::LateStart => {
            let lead = us(cell.lead_us.unwrap_or(6_500));
            (c0 + ms(130) - lead, lead, c0 + ms(175))
        }
        Mode::Starved => (c0 + ms(90), ms(10), c0 + ms(215)),
    };
    to(&mut run, book_at);
    let start_point = tx_at(&run, lead);
    let start = start_point.ticks_in(start_point.domain()).unwrap();
    let mut aimed = None;
    let mut until = None;
    if let Stall::Aligned { side_ns } = cell.stall {
        // `busy` ends at the held send's return plus the sends' samples (the held send's cap and the close's two); aim `busy + m` at the lattice point `L` past `aim`.
        let lattice = period * per;
        let start_root = tx_origin + start * per;
        let l = start + (aim - start_root + lattice - 1).div_euclid(lattice) * period;
        until = Some(tx_origin + l * per + ns_to_ticks_ceil(side_ns) - (cap + CLOSE_SAMPLES) * per - ns_to_ticks_ceil(cell.margin()));
        aimed = Some((l, side_ns));
    }
    let mut booking = None;
    if incomplete.is_empty() {
        let hold = hold_of(&cell, &anchor, until);
        let mut holds = vec![match cell.mode {
            Mode::LateStart => TxHold::Start { nth: 1, hold },
            Mode::Starved => TxHold::Midplay { after: Wall::from_millis(60), hold },
        }];
        match (cell.mode, cell.retry_hold) {
            (Mode::LateStart, Some(retry)) => holds.push(TxHold::Start { nth: 2, hold: Hold::For(retry) }),
            (Mode::Starved, Some(_)) => panic!("retry_hold is a late-start cell's (the second Start after arming, held late; S4')"),
            _ => {}
        }
        probe.arm_hold(holds);
        match run.submit(verb("start_repeat", resource("radio/tx/s0"), Some(start_point), &[]), Some(&wave)) {
            Ok(entry) => booking = Some(entry),
            Err(error) => incomplete.push(error.to_string()),
        }
    }
    // Until the latest accepted Start after the control has had 30 ms, or +300 ms.
    let mut latest = None;
    if incomplete.is_empty() {
        while now_root_of(&run) < c0 + ms(300) {
            wait(&mut run, ms(5));
            let ticks = probe.with_rows(|rows| start_ticks(rows, control_starts));
            // The cell's first Start and the parts after a drop: the window follows the latest of them, so that the part just handed has its 30 ms; it follows
            // only the first `MAX_FOLLOWED_STARTS`, so that a chain that never settles still ends.
            if ticks.len() >= 2 {
                latest = ticks.iter().take(MAX_FOLLOWED_STARTS).copied().max();
            }
            if latest.is_some_and(|at| now_root_of(&run) >= at + ms(30)) {
                break;
            }
        }
    }
    let stop_tick = now_root_of(&run);
    stop(&mut run);
    to(&mut run, c0 + ms(CAPTURE_MS + 5));
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let anchor_after = Anchor::read(&probe, 3);
    let manifest = run.finish();
    let rows_complete = probe.rows_complete();
    Trial {
        cell,
        common: Common {
            manifest,
            probe: probe.rows(),
            anchor,
            anchor_after,
            tx_origin,
            per,
            start,
            period,
            period_us: cell.period_us,
            c0,
            stop_tick,
            armed_closes,
            control_starts,
            control,
            incomplete,
            rows_complete,
            probe_dropped: probe.dropped_rows(),
            booking,
        },
        aimed,
    }
}

/// The storm's capture: long enough for the 20 s fake rehearsal to stop well inside it (the capture is written once its own 600 ms are in).
pub const STORM_CAPTURE_MS: i64 = 600;

/// A storm: a repeat whose every part is handed at its resume point to a device that is already past it (every `Start` held `hold`), so that it drops them all and
/// keeps resuming. It books the cell's repeat as the late-start cell does (6.5 ms of lead, its start at +130 ms), arms the storm's hold once the control's close is in
/// the probe, runs to `c0 + storm_ms`, stops the repeat and reads the Manifest once the capture is written.
pub fn storm_trial(profile: &Json, probe: Arc<ProbeDevice>, rate: f64, period_us: i64, hold: Wall, storm_ms: i64) -> Storm {
    let Opened { mut run, anchor, wave, cap: _, tx_origin, c0, period, per, mut incomplete, armed_closes, control, control_starts } = open(profile, &probe, None, rate, period_us, STORM_CAPTURE_MS);
    let lead = us(6_500);
    to(&mut run, c0 + ms(130) - lead);
    let start_point = tx_at(&run, lead);
    let start = start_point.ticks_in(start_point.domain()).unwrap();
    let mut booking = None;
    if incomplete.is_empty() {
        probe.arm_hold(vec![TxHold::EveryStart(Hold::For(hold))]);
        match run.submit(verb("start_repeat", resource("radio/tx/s0"), Some(start_point), &[]), Some(&wave)) {
            Ok(entry) => booking = Some(entry),
            Err(error) => incomplete.push(error.to_string()),
        }
    }
    if incomplete.is_empty() {
        while now_root_of(&run) < c0 + ms(storm_ms) {
            wait(&mut run, ms(5));
        }
    }
    let stop_tick = now_root_of(&run);
    stop(&mut run);
    let end = (c0 + ms(STORM_CAPTURE_MS + 5)).max(now_root_of(&run));
    to(&mut run, end);
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let anchor_after = Anchor::read(&probe, 3);
    let manifest = run.finish();
    let rows_complete = probe.rows_complete();
    Storm {
        common: Common {
            manifest,
            probe: probe.rows(),
            anchor,
            anchor_after,
            tx_origin,
            per,
            start,
            period,
            period_us,
            c0,
            stop_tick,
            armed_closes,
            control_starts,
            control,
            incomplete,
            rows_complete,
            probe_dropped: probe.dropped_rows(),
            booking,
        },
        capture_ms: STORM_CAPTURE_MS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One sample = 200 root ticks, origin 0, the lattice `20 030 + period * j`.
    fn lattice(period: i64) -> Lattice {
        Lattice { tx_origin: 0, per: 200, origin: 20_030, period }
    }

    /// The identity mapping: one root tick is 5 ns, every bracket is zero.
    fn zero_anchor() -> Anchor {
        Anchor { tick: 0, host_mid_ns: 0, bracket_ns: 0, worst_bracket_ns: 0 }
    }

    /// One successful `ProbeDevice` clock read of the Module (`device_call`), `width` nanoseconds wide.
    fn time_now(before: u64, width: u64) -> Json {
        serde_json::json!({ "call": "time_now", "purpose": "device_call", "host_entered_ns": before, "host_exit_ns": before + width, "result": { "value": 1 } })
    }

    #[test]
    fn c7_the_wave_is_a_zero_mean_pn_of_the_requested_period_within_half_scale() {
        for (period_us, rate, n) in [(FAKE_PERIOD_US, 1e6, 100), (FAKE_PERIOD_US, 10e6, 1_000), (HW_PERIOD_US, 1e6, 1_000), (HW_PERIOD_US, 10e6, 10_000)] {
            let w = wave(rate, period_us);
            assert_eq!((w.len(), period(rate, period_us)), (n, n), "{rate}: {period_us} us");
            assert!(w.iter().all(|s| (s.0 as f64).hypot(s.1 as f64) <= 0.495 + 1e-6), "bench.md's RF safety: at most 0.5");
            let mean = (w.iter().map(|s| s.0 as f64).sum::<f64>() / n as f64, w.iter().map(|s| s.1 as f64).sum::<f64>() / n as f64);
            assert!(mean.0.abs() < 1e-6 && mean.1.abs() < 1e-6, "{rate}: {period_us} us: zero mean {mean:?}");
            assert_eq!(w, wave(rate, period_us), "deterministic");
        }
    }

    /// bt2 fix round 2, item 4(a): a C7 Session's capture link holds a 200 ms stall of the capture thread at 10 MS/s (against the shared profile's 64 blocks, 12.8 ms),
    /// and `open` applies it to a profile that has a feed.
    #[test]
    fn c7_the_capture_link_holds_a_200_ms_stall_at_10_msps_and_the_c7_session_applies_it() {
        // A receive block is 2 000 samples (`profile::DEFAULT_BLOCK_LEN`, restated): 200 us at 10 MS/s.
        let block_us = 2_000.0 / 10.0;
        assert!(CAPTURE_LINK_CAPACITY as f64 * block_us >= 200_000.0, "{CAPTURE_LINK_CAPACITY} blocks hold {} us at 10 MS/s", CAPTURE_LINK_CAPACITY as f64 * block_us);
        let dir = TempDir::new();
        let shared = profile(&dir, serde_json::json!({}), serde_json::json!({}), true);
        assert_eq!(shared["bindings"]["rec"]["feed"]["capacity"], 64, "the shared profile keeps its 64 blocks");
        assert_eq!(with_capture_capacity(&shared)["bindings"]["rec"]["feed"]["capacity"], CAPTURE_LINK_CAPACITY);
        let no_feed = profile(&dir, serde_json::json!({}), serde_json::json!({}), false);
        assert!(with_capture_capacity(&no_feed)["bindings"]["rec"].get("feed").is_none(), "a profile with no feed gets none");
    }

    #[test]
    fn c7_the_send_cap_is_a_quarter_of_the_window_and_at_most_twenty_thousand_samples() {
        assert_eq!((send_cap(1e6), send_cap(10e6)), (2_500, 20_000));
    }

    /// The numbers of `mt3d_a_core_margin_sets_r_and_the_fakes_drain_limit_lies_between_the_998_and_999_cases` (period-100 waveform, k0 = 20 030, one sample = 200 root ticks):
    /// the first send returned at 17 ms, plays from k0 for 2 500 samples; the close returned at 21 ms; the decision instant is 21 ms.
    #[test]
    fn c7_expected_r_pins_the_mt3d_hand_numbers() {
        let per = 200;
        let sends = [Sent { returned: 17 * 200_000, first: 20_030 * per, ticks: 2_500 * per, err: 0 }, Sent { returned: 21 * 200_000, first: 22_530 * per, ticks: 2 * per, err: 0 }];
        let busy = busy_replay(&sends);
        let (lo, hi) = (busy - MAP_TOLERANCE_TICKS, busy + MAP_TOLERANCE_TICKS);
        assert_eq!(busy, 22_532 * per);
        for (margin_us, r) in [(10_000, 32_630), (1_000, 23_630), (999, 23_630), (998, 23_530), (200, 23_030)] {
            let terms = expected_r(lo, busy, hi, margin_us * 200, 21 * 200_000, 2 * 200_000, &lattice(100));
            assert_eq!(terms.r, r, "m {margin_us} us: {terms:?}");
            assert!(terms.lo <= terms.r && terms.r <= terms.hi, "m {margin_us} us: r is in the interval: {terms:?}");
        }
        // V5's period 1 000: R = 33 030 (the first 20 030 + 1 000 j at or after 32 532); period 12 502: busy + m is the lattice point itself.
        assert_eq!(expected_r(lo, busy, hi, 10_000 * 200, 21 * 200_000, 2 * 200_000, &lattice(1_000)).r, 33_030);
        assert_eq!(expected_r(lo, busy, hi, 10_000 * 200, 21 * 200_000, 2 * 200_000, &lattice(12_502)).r, 32_532);
    }

    /// The interval `expected_r` allows, from its definition: one point each side of a lattice boundary, one point wholly inside a period, a lead bound, and an
    /// uncertainty wider than a period (the X310's 0.15 ms bracket against the 100 µs lattice).
    #[test]
    fn c7_the_resume_interval_spans_a_boundary_and_pins_a_point_inside_a_period() {
        // One sample = 200 ticks, lattice 20 030 + 100 j: L = 23 530 is at 4 706 000 ticks, and a lattice period is 20 000 ticks.
        let (period, per, point) = (100, 200, 23_530);
        let l = point * per;
        let at = |unaligned: i64, decided: i64, uncertainty: i64| expected_r(unaligned - uncertainty, unaligned, unaligned + uncertainty, 0, decided, 0, &lattice(period));
        let allowed = |terms: &Terms| (terms.lo..=terms.hi).step_by(period as usize).collect::<Vec<_>>();

        // Ten ticks below L, uncertainty 200: [L - 210, L + 190] holds L and the next point; L - period is refused.
        let below = at(l - 10, 0, MAP_TOLERANCE_TICKS);
        assert_eq!((below.lo, below.r, below.hi), (point, point, point + period), "{below:?}");
        assert_eq!((below.allowed(period), allowed(&below)), (2, vec![point, point + period]), "{below:?}");
        assert!(!allowed(&below).contains(&(point - period)));

        // Ten ticks above L: rounding up of u - 200 is L, of u + 200 is L + period, and the nominal r is L + period.
        let above = at(l + 10, 0, MAP_TOLERANCE_TICKS);
        assert_eq!((above.lo, above.r, above.hi), (point, point + period, point + period), "{above:?}");
        assert_eq!((above.allowed(period), allowed(&above)), (2, vec![point, point + period]), "{above:?}");

        // Wholly inside a period: both sides round up to L + period, so the interval pins exactly one answer.
        let inside = at(l + 10_000, 0, MAP_TOLERANCE_TICKS);
        assert_eq!((inside.lo, inside.r, inside.hi), (point + period, point + period, point + period), "{inside:?}");
        assert!(inside.pinned() && inside.allowed(period) == 1, "{inside:?}");

        // Lead-bound: max(unaligned -+ uncertainty, decided) is decided, the Module's exact reading, so the interval is pinned too.
        let lead = at(l - 100_000, l + 5_000, MAP_TOLERANCE_TICKS);
        assert_eq!((lead.lo, lead.r, lead.hi), (point + period, point + period, point + period), "{lead:?}");
        assert!(lead.pinned() && lead.allowed(period) == 1, "{lead:?}");

        // An uncertainty of 30 200 ticks (0.15 ms at 5 ns a tick plus the 1 µs allowance) about L: [L - 30 200, L + 30 200] allows L - period, L,
        // L + period and L + 2*period; four points.
        let wide = at(l, 0, 30_200);
        assert_eq!((wide.lo, wide.r, wide.hi), (point - period, point, point + 2 * period), "{wide:?}");
        assert_eq!((wide.allowed(period), allowed(&wide).len() as i64), (4, 4), "{wide:?}");
        assert!(wide.allowed(period) >= 3 && !wide.pinned(), "{wide:?}");
        assert!(wide.unresolved, "30 200 ticks exceed half of the 20 000-tick period: {wide:?}");
    }

    #[test]
    fn c7_the_resume_interval_is_unresolved_when_the_uncertainty_exceeds_half_a_period() {
        // A period of 1 000 samples at 200 ticks: 200 000 ticks, so half is 100 000.
        let busy = 20_030 * 200;
        let at = |uncertainty: i64| expected_r(busy - uncertainty, busy, busy + uncertainty, 0, 21 * 200_000, 0, &lattice(1_000));
        assert!(!at(100_000).unresolved, "exactly half a period is resolved: {:?}", at(100_000));
        assert!(at(100_001).unresolved, "one tick past half a period is unresolved: {:?}", at(100_001));
        let narrow = at(30_200);
        assert!(!narrow.unresolved, "{narrow:?}");
        assert!((1..=2).contains(&narrow.allowed(1_000)), "a resolved narrow uncertainty allows one or two points: {narrow:?}");
        // The old four-point case: 30 200 ticks against the 100 us lattice (20 000 ticks) exceed half a period, so the points cannot be told apart.
        assert!(expected_r(busy - 30_200, busy, busy + 30_200, 0, 21 * 200_000, 0, &lattice(100)).unresolved);
    }

    #[test]
    fn c7_the_r_check_requires_the_interval_resolved_or_not() {
        // Lattice 100 + 10 j: 100 and 110 are on it, 120 is one period above `hi`, 115 is off it.
        let resolved = Terms { busy: 0, unaligned: 0, decided: 0, r: 100, lo: 100, hi: 110, uncertainty: 5, unresolved: false };
        assert_eq!(r_check(&resolved, 110, 100, 10), Ok(()), "an R inside lo..=hi is the rule's");
        assert!(r_check(&resolved, 120, 100, 10).is_err(), "an R one period above hi is off the rule");
        // Unresolved (the bounds are wider than a period): every lattice point inside the interval passes and the points outside still do not.
        let unresolved = Terms { unresolved: true, lo: 100, hi: 120, ..resolved };
        assert_eq!(r_check(&unresolved, 110, 100, 10), Ok(()), "unresolved: a point inside the interval passes");
        assert_eq!(r_check(&unresolved, 120, 100, 10), Ok(()), "unresolved: hi itself passes");
        assert!(r_check(&unresolved, 130, 100, 10).is_err(), "unresolved: a point one period above hi is off the rule");
        assert!(r_check(&unresolved, 90, 100, 10).is_err(), "unresolved: a point one period below lo is off the rule");
        assert!(r_check(&unresolved, 115, 100, 10).is_err(), "unresolved still requires the lattice");
    }

    /// Terra's sequence (bt2 fix round 2, item 2): the stamps allow R from 150 ms to 152 ms (three points of a 1 ms lattice, so the width exceeds a period and the
    /// interval is unresolved); a Module that resumes at 200 ms, on the lattice, is off the rule and is not CLEAN.
    #[test]
    fn c7_an_unresolved_interval_refuses_a_lattice_point_outside_it() {
        let lattice = wide_lattice();
        let ms = |n: i64| n * 200_000;
        // busy + m rounds up to 150 ms at the low bound and to 152 ms at the high one; the decision is long before.
        let terms = expected_r(ms(150) - 100, ms(151), ms(152), 0, 0, 0, &lattice);
        assert!(terms.unresolved, "{terms:?}");
        let at = |t: i64| t / 200;
        assert_eq!((terms.lo, terms.hi), (at(ms(150)), at(ms(152))), "{terms:?}");
        for inside in [150, 151, 152] {
            assert_eq!(r_check(&terms, at(ms(inside)), 100_000, 1_000), Ok(()), "{inside} ms is inside");
        }
        for (outside, away) in [(149, 1), (153, 1), (200, 48)] {
            let refused = r_check(&terms, at(ms(outside)), 100_000, 1_000);
            assert!(
                refused.as_ref().is_err_and(|text| text.contains(&format!("{away} lattice point(s) outside"))),
                "{outside} ms is on the lattice, {away} point(s) outside the interval: {refused:?}"
            );
        }
    }

    /// The sends the Module's `busy` counted before a resume row: its own close by ordinal, not a later one that returned inside the slack.
    #[test]
    fn c7_history_pairs_each_resume_with_its_own_close_not_a_later_one_inside_the_slack() {
        // The anchor makes one root tick 5 ns: `host_after_ns = returned * 5` maps back exactly.
        let anchor = Anchor { tick: 0, host_mid_ns: 0, bracket_ns: 0, worst_bracket_ns: 0 };
        let row = |class: &str, accepted: i64, returned: i64, at: i64| serde_json::json!({ "call": "tx_send", "class": class, "at": at, "host_entered_ns": (returned * 5) as u64, "host_after_ns": (returned * 5) as u64, "result": { "accepted_samples": accepted } });
        let rows = [
            row("start", 100, 100, 0),
            row("close", 2, 1_000, 900),     // close 1: the control's
            row("start", 100, 2_000, 1_500), // the cell's
            row("continuation", 100, 3_000, 2_500),
            row("close", 2, 4_000, 3_500),   // close 2: paired with resume row 0
            row("start", 100, 4_500, 4_100), // the resumed part
            row("close", 2, 4_600, 4_500),   // close 3: returns inside the slack after the decision
        ];
        let sent = history(&rows, &anchor, 1, 0, 4_000, 1_000, 1).unwrap();
        assert_eq!(sent.iter().map(|sent| sent.returned).collect::<Vec<_>>(), [100, 1_000, 2_000, 3_000, 4_000], "up to the first close after the armed ones, not the later one in the slack");
        assert_eq!(sent.iter().map(|sent| sent.ticks).collect::<Vec<_>>(), [100, 2, 100, 100, 2], "accepted samples times `per`");
        assert_eq!(sent[1].first, 900);

        let missing = history(&rows[..4], &anchor, 1, 0, 4_000, 1_000, 1).unwrap_err();
        assert!(missing.starts_with("ambiguous"), "{missing}");

        let mut late = rows;
        late[4] = row("close", 2, 4_000 + history_slack(1_000) + 1, 3_500);
        let late = history(&late, &anchor, 1, 0, 4_000, 1_000, 1).unwrap_err();
        assert!(late.starts_with("ambiguous") && late.contains("later than its decision"), "{late}");
    }

    /// A successful Module clock read as the probe records it: the whole call `entered..exit`, the inner call a little inside it.
    fn whole_read(entered: u64, exit: u64) -> Json {
        serde_json::json!({ "call": "time_now", "purpose": "device_call", "host_entered_ns": entered, "host_before_ns": entered + 1_000, "host_after_ns": exit - 1_000, "host_exit_ns": exit, "result": { "value": 1 } })
    }

    /// The Module's stamp of a send that returned at `returned_ns` is bounded by the widest read that was in effect when the one-millisecond window before the return opened
    /// and that completed inside it; a read that completed after the return counts for nothing (bt2 fix round 2, item 1). One root tick is 5 ns.
    #[test]
    fn c7_the_module_read_half_width_is_the_widest_read_in_the_window_before_the_return() {
        let ticks = |ns: u64| (ns as i64 + 9) / 10;
        let returned = 50_000_000;
        // A wide read (900 us) long before, a narrow one (20 us) 0.3 ms before the return: the clamp holds the stamp ahead by the wide read's half width.
        let clamp = [whole_read(40_000_000, 40_900_000), whole_read(49_680_000, 49_700_000)];
        assert_eq!(module_read_half_ticks(&clamp, returned), ticks(900_000), "the superseded wide read still bounds the stamp: {clamp:?}");
        // The same reads with the narrow one landing before the window opened (1.5 ms before the return): the wide one is gone.
        let settled = [whole_read(40_000_000, 40_900_000), whole_read(48_480_000, 48_500_000)];
        assert_eq!(module_read_half_ticks(&settled, returned), ticks(20_000), "a narrow read in effect for the whole window: its own half width");
        // A wide read in effect over the whole window (nothing newer): its half width.
        assert_eq!(module_read_half_ticks(&[whole_read(40_000_000, 40_900_000)], returned), ticks(900_000));
        // Terra's sequence: a wide read in effect at the return, a narrow one just after it: the narrow one does not replace the wide one.
        let after = [whole_read(40_000_000, 40_800_000), whole_read(50_020_000, 50_040_000)];
        assert_eq!(module_read_half_ticks(&after, returned), ticks(800_000), "a narrow read that completed after the return is not used: {after:?}");
        // And a wide read that completed after the return is not used either (no look-ahead).
        let wide_after = [whole_read(49_900_000, 49_920_000), whole_read(50_100_000, 50_900_000)];
        assert_eq!(module_read_half_ticks(&wide_after, returned), ticks(20_000), "a wide read after the return does not widen the stamp: {wide_after:?}");
        // No read at all: the Module's widest accepted read.
        assert_eq!(module_read_half_ticks(&[], returned), MODULE_READ_HALF_MAX_TICKS);
        assert_eq!(module_read_half_ticks(&[whole_read(50_100_000, 50_140_000)], returned), MODULE_READ_HALF_MAX_TICKS, "only a read after the return: none qualifies");
        // The width is the whole call: the inner stamps of this row (a 1 us margin each side) are narrower than its entry and exit.
        let whole = whole_read(49_000_000, 49_800_000);
        assert_eq!(module_read_half_ticks(std::slice::from_ref(&whole), returned), ticks(800_000), "the whole call's width, not the inner call's {whole}");
        // The reads the Authority discards (wider than 1 ms, or an error, or another purpose) are not the Module's anchors.
        let discarded = [
            whole_read(48_900_000, 49_901_000),
            serde_json::json!({ "call": "time_now", "purpose": "device_call", "host_entered_ns": 49_100_000u64, "host_exit_ns": 49_110_000u64, "result": { "error": "lost", "lost": true } }),
            serde_json::json!({ "call": "time_now", "purpose": "h4_anchor", "host_entered_ns": 49_200_000u64, "host_exit_ns": 49_210_000u64, "result": { "value": 1 } }),
        ];
        assert_eq!(module_read_half_ticks(&discarded, returned), MODULE_READ_HALF_MAX_TICKS, "no read qualifies: {discarded:?}");
        // Exactly 1 ms is accepted.
        assert_eq!(module_read_half_ticks(&[whole_read(48_000_000, 49_000_000)], returned), ticks(1_000_000));
        assert_eq!(module_read_half_ticks(&[time_now(48_000_000, 60_000)], returned), ticks(60_000));
    }

    #[test]
    fn c7_the_anchors_worst_bracket_is_the_widest_read() {
        assert_eq!(worst_bracket(&[40, 120, 80]), 120, "the best read maps, the worst is the honest uncertainty");
        assert_eq!(worst_bracket(&[]), 0);
    }

    #[test]
    fn c7_history_accepts_a_close_that_returned_a_module_read_error_after_the_decision() {
        // One root tick is 5 ns. The decision is at tick 4 000; the Module's own last clock read can place its close up to 100 000 ticks past it.
        let anchor = zero_anchor();
        let at = 4_000;
        let close = |returned: i64| serde_json::json!({ "call": "tx_send", "class": "close", "at": 3_500, "host_entered_ns": (returned * 5) as u64, "host_after_ns": (returned * 5) as u64, "result": { "accepted_samples": 2 } });
        let paired = history(&[close(at + 90_000)], &anchor, 0, 0, at, 0, 1);
        assert!(paired.is_ok(), "a close 90 000 ticks after the decision is within the Module's read error: {paired:?}");
        let late = history(&[close(at + 200_000)], &anchor, 0, 0, at, 0, 1).unwrap_err();
        assert!(late.starts_with("ambiguous"), "a close 200 000 ticks after the decision is ambiguous: {late}");
        assert_eq!(history_slack(0), MODULE_READ_HALF_MAX_TICKS);
        assert_eq!(history_slack(500_000), 500_000 + MODULE_READ_HALF_MAX_TICKS, "the Module's read half width is part of the slack");
    }

    #[test]
    fn c7_busy_follows_the_modules_recursion_with_the_return_of_each_send() {
        // A send returned after its first sample is due (r = 21 000 > x = 20 030) counts from its return; one returned before it counts from its first sample.
        let late = busy_replay(&[Sent { returned: 21_000, first: 20_030, ticks: 2_500, err: 0 }, Sent { returned: 21_010, first: 22_530, ticks: 2, err: 0 }]);
        assert_eq!(late, 23_502, "r-bound: 21 000 + 2 500 = 23 500, then the close from max(23 500, 21 010, 22 530)");
        let early = busy_replay(&[Sent { returned: 17_000, first: 20_030, ticks: 2_500, err: 0 }, Sent { returned: 21_000, first: 22_530, ticks: 2, err: 0 }]);
        assert_eq!(early, 22_532, "x-bound");
    }

    /// The hardware lattice of the one-incident fixtures: 1 ms at 1 MS/s, sample 0 at root tick 0, the points at 100 000 + 1 000 j samples.
    fn wide_lattice() -> Lattice {
        Lattice { tx_origin: 0, per: 200, origin: 100_000, period: 1_000 }
    }

    /// The rows of the one-incident fixtures: a wide read, the held Start, a narrow read and the close that is paired with the decision at 28 150 000.
    fn held_send_rows(wide_before: u64, wide_width: u64) -> Vec<Json> {
        vec![
            time_now(wide_before, wide_width),
            serde_json::json!({ "call": "tx_send", "class": "start", "at": 20_000_000, "host_entered_ns": 101_000_000u64, "host_after_ns": 140_648_000u64, "result": { "accepted_samples": 2_500 } }),
            time_now(140_700_000, 10_000),
            serde_json::json!({ "call": "tx_send", "class": "close", "at": 20_500_000, "host_entered_ns": 140_720_000u64, "host_after_ns": 140_730_000u64, "result": { "accepted_samples": 2 } }),
        ]
    }

    /// The interval bounds the stamps that set `busy`, each at its own error, not the Module's clock read at the decision: the held send returned inside a wide
    /// read (half 40 000 ticks) and a narrow read replaced it before the decision, so the old interval (the narrow read's half, 1 000) was 145 000..=145 000 and
    /// refused the Module's own R 144 000 off by 15 400 ticks, inside the wide read's half width.
    #[test]
    fn c7_the_r_interval_bounds_the_stamps_that_set_busy_not_the_read_at_the_decision() {
        let anchor = zero_anchor();
        let rows = held_send_rows(100_000_000, 400_000);
        let sends = history(&rows, &anchor, 0, 0, 28_150_000, 0, 200).unwrap();
        assert_eq!(
            sends,
            [Sent { returned: 28_129_600, first: 20_000_000, ticks: 500_000, err: 54_400 }, Sent { returned: 28_146_000, first: 20_500_000, ticks: 400, err: 44_000 },],
            "each send's own stamp error: 40 000 + 14 400 for the held Start; 40 000 + 4 000 for the close, which returned 20 us after the narrow read replaced the wide one, so the wide read's half width (the clamp) still bounds it"
        );
        assert_eq!(busy_replay(&sends), 28_630_000);
        assert_eq!(busy_bounds(&sends), (28_575_600, 28_684_400));
        let terms = expected_r(28_575_600, busy_replay(&sends), 28_684_400, 200_000, 28_150_000, 400_000, &wide_lattice());
        assert_eq!(terms.unaligned, 28_830_000);
        assert_eq!((terms.lo, terms.r, terms.hi), (144_000, 145_000, 145_000), "{terms:?}");
        assert_eq!(terms.uncertainty, 54_400);
        assert!(!terms.unresolved);
        assert_eq!(terms.allowed(1_000), 2);
        assert!(r_check(&terms, 144_000, 100_000, 1_000).is_ok(), "the Module's R with its own stamp of the held return 35 000 ticks earlier is in the interval");
        assert!(r_check(&terms, 145_000, 100_000, 1_000).is_ok());
        assert!(r_check(&terms, 143_000, 100_000, 1_000).is_err(), "one point below lo is refused");
        assert!(r_check(&terms, 146_000, 100_000, 1_000).is_err(), "one point above hi is refused");
    }

    /// Terra's sequence (bt2 fix round 2, item 1): a wide read (400 us) is in effect when the held Start returns, and a narrow one (10 us) completes 52 us after the
    /// return. The oracle used to take the last read that completed within the anchor's bracket of the return, the narrow one, so its interval for the held send's stamp
    /// was 1 000 + 14 400 ticks wide and refused the Module's own R 144 000; it now bounds the stamp by the wide read in effect (40 000 + 14 400) and accepts it.
    #[test]
    fn c7_a_narrow_read_just_after_the_held_return_does_not_narrow_the_interval_of_the_wide_read_in_effect() {
        let anchor = zero_anchor();
        let rows = held_send_rows(100_000_000, 400_000);
        let sends = history(&rows, &anchor, 0, 0, 28_150_000, 0, 200).unwrap();
        let interval = |sends: &[Sent]| {
            let (lo, hi) = busy_bounds(sends);
            expected_r(lo, busy_replay(sends), hi, 200_000, 28_150_000, 400_000, &wide_lattice())
        };
        let mut narrow = sends.clone();
        narrow[0].err = 1_000 + 14_400;
        let before = interval(&narrow);
        assert_eq!((before.lo, before.hi), (145_000, 145_000), "the narrow read's half width pins one point: {before:?}");
        assert!(r_check(&before, 144_000, 100_000, 1_000).is_err(), "and refused the Module's own R");
        let after = interval(&sends);
        assert_eq!((after.lo, after.hi), (144_000, 145_000), "{after:?}");
        assert_eq!(r_check(&after, 144_000, 100_000, 1_000), Ok(()));
    }

    /// A wide read over the held send (exactly the 1 ms the Module accepts, half 100 000 ticks) widens the interval past a period: unresolved, so the oracle accepts
    /// every point of the interval, but a point outside it or off the lattice is still refused.
    #[test]
    fn c7_a_wide_read_over_the_held_send_makes_the_interval_unresolved() {
        let anchor = zero_anchor();
        let rows = held_send_rows(99_000_000, 1_000_000);
        let sends = history(&rows, &anchor, 0, 0, 28_150_000, 0, 200).unwrap();
        assert_eq!(sends[0].err, 114_400, "the 1 ms read's half (100 000) plus 14 400: {sends:?}");
        assert_eq!(busy_replay(&sends), 28_630_000);
        assert_eq!(busy_bounds(&sends), (28_515_600, 28_744_400));
        let terms = expected_r(28_515_600, busy_replay(&sends), 28_744_400, 200_000, 28_150_000, 400_000, &wide_lattice());
        assert!(terms.unresolved, "228 800 ticks of width exceed the 200 000-tick period: {terms:?}");
        assert_eq!(terms.uncertainty, 114_400);
        assert_eq!((terms.lo, terms.hi), (144_000, 145_000), "{terms:?}");
        assert!(r_check(&terms, 144_000, 100_000, 1_000).is_ok() && r_check(&terms, 145_000, 100_000, 1_000).is_ok(), "unresolved: every point of the interval passes");
        assert!(r_check(&terms, 146_000, 100_000, 1_000).is_err(), "unresolved: a lattice point outside the interval does not");
        assert!(r_check(&terms, 143_000, 100_000, 1_000).is_err(), "unresolved: nor one below it");
        assert!(r_check(&terms, 144_500, 100_000, 1_000).is_err(), "unresolved still requires the lattice");
    }

    /// Each send's return is replayed at its own error, so a stale send's huge error does not widen the busy the next send sets.
    #[test]
    fn c7_busy_bounds_replay_each_stamp_at_its_own_error_so_a_stale_send_drops_out() {
        let sends = [Sent { returned: 1_000, first: 900, ticks: 2, err: 5_000_000 }, Sent { returned: 20_000_000, first: 19_000_000, ticks: 500, err: 100 }];
        assert_eq!(busy_replay(&sends), 20_000_500);
        assert_eq!(busy_bounds(&sends), (20_000_400, 20_000_600));
    }

    #[test]
    fn c7_the_anchor_maps_host_stamps_to_ticks_and_back() {
        let anchor = Anchor { tick: 1_000_000, host_mid_ns: 5_000_000, bracket_ns: 100, worst_bracket_ns: 100 };
        assert_eq!(anchor.map(5_000_000), 1_000_000);
        assert_eq!(anchor.map(6_000_000), 1_000_000 + 200_000);
        assert_eq!(anchor.map(4_999_999), 1_000_000 - 1, "5 ns a tick: floor");
        assert_eq!(anchor.host_at(1_200_000), 6_000_000);
        assert_eq!(anchor.map(anchor.host_at(1_234_567)), 1_234_567);
    }
}
