//! Stage 4b's S9 bench tool (`plan/phase8/bench.md`): a receive Stop and start booked after an overflow start, classified after
//! the fact from the probe's `rx_recv` rows (cell A: the booking completed inside the restart gap, after the overflow result and
//! before the stream could have resumed; cell B: the booking met the backlog the reader was returning), the verdict on one trial's
//! events, and the decision line of one target cell. The pure judgements are unit-tested at the bottom; `s9_trial_with` books the
//! Stop and start on one Session Run (its hardware path is `s9_trial`), and `s9_run` runs a plan's cells.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration as Wall;

use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::module_api::{Link, LinkDescriptor, ModuleError};
use ezsdr_kernel::session::SessionAction;
use ezsdr_kernel::spec::Scalar;
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockRef, DataLink, DataLinkDecl, DropCarry, PublishOutcome};
use ezsdr_kernel::time::TimePoint;
use ezsdr_radio::payloads::{RxOverflowCause, RxOverflowPayload};
use ezsdr_radio_uhd::Device;
use serde_json::{Value as Json, json};

use super::measurement;

/// The successor's first block as the Provider published it to the link (its index on its own clock and the gap flags it carries).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirstBlock {
    pub index: u64,
    pub gap_before: bool,
    pub seq_discontinuity: bool,
}

/// The first block published per SampleClock.
pub type FirstBlocks = Arc<Mutex<BTreeMap<ClockDomainId, FirstBlock>>>;

/// `ezsdr_link_host::HostLinkModule` that records each SampleClock's first published block.
pub struct FirstBlockLink {
    inner: ezsdr_link_host::HostLinkModule,
    seen: FirstBlocks,
}

impl FirstBlockLink {
    pub fn new(seen: FirstBlocks) -> FirstBlockLink {
        FirstBlockLink { inner: ezsdr_link_host::HostLinkModule::new(), seen }
    }
}

impl Link for FirstBlockLink {
    fn descriptor(&self) -> &LinkDescriptor {
        self.inner.descriptor()
    }

    fn create(&self, decl: &DataLinkDecl) -> Result<Arc<dyn DataLink>, ModuleError> {
        Ok(Arc::new(Recording { inner: self.inner.create(decl)?, seen: self.seen.clone() }))
    }
}

/// The link itself, with each SampleClock's first published block recorded before it is queued.
struct Recording {
    inner: Arc<dyn DataLink>,
    seen: FirstBlocks,
}

impl DataLink for Recording {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        let header = block.header();
        let domain = header.first_sample_time.domain();
        if let Some(index) = header.first_sample_time.ticks_in(domain).ok().and_then(|ticks| u64::try_from(ticks).ok()) {
            self.seen.lock().unwrap().entry(domain).or_insert(FirstBlock {
                index,
                gap_before: header.flags.contains(BlockFlags::GAP_BEFORE),
                seq_discontinuity: header.flags.contains(BlockFlags::SEQ_DISCONTINUITY),
            });
        }
        self.inner.publish(block)
    }

    fn receive(&self) -> Option<(BlockRef, DropCarry)> {
        self.inner.receive()
    }

    fn drops(&self) -> u64 {
        self.inner.drops()
    }

    fn take_drop_carry(&self) -> DropCarry {
        self.inner.take_drop_carry()
    }

    fn policy(&self) -> BackPressure {
        self.inner.policy()
    }
}

/// Which of S9's cells a booking falls in, or why the trace cannot say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    A,
    B,
    Inconclusive(&'static str),
}

/// A booking after the overflow result that a `samples` call begun before the booking completed may have overtaken: it cannot be
/// shown to have met the restart gap, so the cell is open.
pub const S9_RESUMED: &str = "the stream had resumed before the booking";

/// More than one non-sequence overflow row in the trace: the stall's own overflow cannot be told from one the load caused, so the
/// trial is open (S4's rule: a trial with an overflow besides its own is INCONCLUSIVE).
pub const S9_MORE_OVERFLOWS: &str = "an overflow besides the stall's";
/// The booking ended too close to UHD's own restart: the Module's stop at the cut may have come after UHD's restart START ran.
pub const S9_NEAR_RESTART: &str = "booked within a receive wait of the device's own restart";
/// A `rx_recv` row without unsigned host stamps cannot be placed in time.
pub const S9_UNSTAMPED: &str = "an rx_recv row has no host stamps";
/// No `rx_stop` row (with host stamps) after the booking: the stop that cleared UHD's restart cannot be placed in time.
pub const S9_NO_STOP: &str = "no rx_stop row with host stamps after the booking";
/// A row of the booking's stop pair recorded an error: the physical stop that cell A's premise needs did not take effect.
pub const S9_STOP_FAILED: &str = "a stop of the booking's pair failed";

/// UHD 4.10.0.0 queues its restart `START_CONTINUOUS` at `get_time_now() + OVERRUN_RESTART_DELAY` with `OVERRUN_RESTART_DELAY` = 0.05 s
/// (`host/lib/rfnoc/radio_control_impl.cpp:48` the constant, `:105-125` the restart-request handler, `time_spec` at `:117`). The request
/// is made while the streamer returns the overflow to the reader, so the restart is at least this long after the `rx_recv` call that
/// returned the overflow result began (VERIFIED in the UHD 4.10 source, design/18 §2).
pub const UHD_OVERRUN_RESTART_DELAY_NS: u64 = 50_000_000;

/// Slack kept between the end of a booking and the restart (two milliseconds), on top of one receive wait: uhd-rx clamps each receive wait to at
/// least 1 ms (`rx.rs` `recv_timeout`) and UHD truncates the wait to whole milliseconds (`rx_streamer_impl.hpp:154`), so the wait that ends the
/// stream can overshoot the span by up to 1 ms; one more millisecond covers the `rx_stop` pair and `get_time_now()`'s read latency.
pub const S9_RESTART_MARGIN_NS: u64 = 2_000_000;

/// The Module's receive wait: one block plus the delivery allowance (`rx.rs` `quiet_span`), in ns (rounded up).
pub fn s9_span_ns(block_len: u64, rate_hz: u64) -> u64 {
    block_len.saturating_mul(1_000_000_000).div_ceil(rate_hz.max(1)).saturating_add(ezsdr_radio_uhd::profile::DELIVERY_ALLOWANCE_NS.unsigned_abs())
}

/// How long after the overflow result's call began a booking may end and still have certainly come before the device's restart
/// reached the Module: the restart delay less one receive wait less the margin (0 when none is left).
pub fn s9_booking_window_ns(restart_after_ns: u64, span_ns: u64) -> u64 {
    restart_after_ns.saturating_sub(span_ns).saturating_sub(S9_RESTART_MARGIN_NS)
}

/// The retuned cells' target: below the 1 000 MHz tuning-path edge of both boards (UBX `db_ubx.cpp` 800–1000 MHz / 1000–1500 MHz branches,
/// OBX `obx_expert.cpp`), while every Session starts at the board's default of 1 GHz (`Board::default_frequency_hz`), so the retune
/// crosses the edge.
pub const S9_RETUNE_HZ: f64 = measurement::RX_START_LEAD_RETUNE_HZ.0;

/// A gap of at least the stall's length between two `rx_recv` rows: the reader resumed (a gap of exactly `stall_ns` counts).
pub fn is_resume(previous_after: u64, before: u64, stall_ns: u64) -> bool {
    before >= previous_after.saturating_add(stall_ns)
}

/// One `rx_recv` row as the classifier reads it.
pub struct Recv {
    pub before: u64,
    pub after: u64,
    pub kind: String,
    pub out_of_sequence: bool,
    pub first_tick: Option<i64>,
    pub samples: Option<u64>,
}

/// One row as the classifier reads it, or `None` for another call or a row without unsigned host stamps.
pub fn recv_row(row: &Json) -> Option<Recv> {
    if row["call"] != "rx_recv" {
        return None;
    }
    let before = row["host_before_ns"].as_u64()?;
    let after = row["host_after_ns"].as_u64()?;
    Some(Recv {
        before,
        after,
        kind: row["outcome"]["kind"].as_str().unwrap_or_default().to_owned(),
        out_of_sequence: row["outcome"]["out_of_sequence"].as_bool().unwrap_or(false),
        first_tick: row["outcome"]["first_tick"].as_i64(),
        samples: row["outcome"]["samples"].as_u64(),
    })
}

/// The `rx_recv` rows in order, each with its host stamps and its outcome; a row without unsigned host stamps is skipped.
pub fn recv_rows(rows: &[Json]) -> Vec<Recv> {
    rows.iter().filter_map(recv_row).collect()
}

/// The overflow result: an overflow row that is not a sequence error.
fn is_result(row: &Recv) -> bool {
    row.kind == "overflow" && !row.out_of_sequence
}

/// The first resume (a gap of the stall's length) and the overflow result after it, as indices into `rows`: `(resume, result)`.
fn locate(rows: &[Recv], stall_ns: u64) -> Result<(usize, usize), &'static str> {
    let resume = (1..rows.len()).find(|&index| is_resume(rows[index - 1].after, rows[index].before, stall_ns)).ok_or("no gap of the stall's length in the trace")?;
    let result = rows[resume..].iter().position(is_result).ok_or("no overflow result after the stall")?;
    Ok((resume, resume + result))
}

/// The overflow result row's host stamps `(before, after)`, as the classifier finds it.
pub fn s9_result_stamps(rows: &[Json], stall_ns: u64) -> Option<(u64, u64)> {
    let rows = recv_rows(rows);
    let (_, result) = locate(&rows, stall_ns).ok()?;
    Some((rows[result].before, rows[result].after))
}

/// One sample's index on a stream's lattice as the Provider publishes it (`lattice_index`), or `None` when the tick is before the origin
/// (the Provider drops that packet). S9 uses it for the old stream's first undelivered sample (`s9_first_undelivered`).
pub fn s9_first_index(origin: i64, first_tick: i64, ticks_per_sample: i64) -> Option<u64> {
    if ticks_per_sample <= 0 || first_tick < origin {
        return None;
    }
    u64::try_from(ezsdr_radio_uhd::lattice_index(first_tick - origin, ticks_per_sample)).ok()
}

/// The booked origin of the successor's start: the `start_index`th (0-based) `rx_start` row's `at`, kept only when the row began at or
/// after the pre-stamp (an earlier start is the stream's first, not the successor's).
pub fn s9_successor_origin(rows: &[Json], start_index: usize, pre_stamp_ns: u64) -> Option<i64> {
    rows.iter().filter(|row| row["call"] == "rx_start").nth(start_index).filter(|row| row["host_before_ns"].as_u64().is_some_and(|before| before >= pre_stamp_ns)).and_then(|row| row["at"].as_i64())
}

/// The old stream's first sample not delivered when the overflow result arrived: the end of the last `samples` call before the result row, on the lattice of `old_origin` (root ticks), or `None` when no such call or its first tick/count is missing.
pub fn s9_first_undelivered(rows: &[Json], stall_ns: u64, old_origin: i64, ticks_per_sample: i64) -> Option<u64> {
    let recvs = recv_rows(rows);
    let (_, result_index) = locate(&recvs, stall_ns).ok()?;
    let last = recvs[..result_index].iter().rev().find(|row| row.kind == "samples")?;
    Some(s9_first_index(old_origin, last.first_tick?, ticks_per_sample)? + last.samples?)
}

/// One old clock as the judgement reads it: its domain and its origin and end in root ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegClock {
    pub domain: ClockDomainId,
    pub origin: i64,
    pub end: Option<i64>,
}

/// The tail the old segment owes and what is wrong with where the delivered tail event stands. `expected` is the old clock's end index
/// less the first undelivered sample (`None` when the end or the sample is unknown or the end is before it); `fault` names the first of:
/// the tail event's clock is not the old clock; its tick is not the first undelivered sample; the old clock's end is not the applied
/// stop's cut; the old clock's end is before the booked Stop's instant. `cut_verified` is true only when the end equals the applied cut
/// and is at or after the booked Stop's instant (the Provider books the Stop at its own `now`, no earlier, and the cut is at or after
/// what ends it: a lower bound only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TailJudgement {
    pub expected: Option<u64>,
    pub fault: Option<&'static str>,
    pub cut_verified: bool,
}

pub fn s9_tail_judgement(old: &SegClock, ticks_per_sample: i64, first_undelivered: Option<u64>, applied_cut: Option<i64>, booked_at: Option<i64>, tail: TimePoint) -> TailJudgement {
    let expected = match (old.end, first_undelivered) {
        (Some(end), Some(first)) if ticks_per_sample > 0 => u64::try_from(ezsdr_radio_uhd::lattice_index(end - old.origin, ticks_per_sample)).ok().and_then(|end_index| end_index.checked_sub(first)),
        _ => None,
    };
    let cut_verified = old.end.is_some_and(|end| applied_cut == Some(end) && booked_at.is_some_and(|booked| end >= booked));
    if tail.domain() != old.domain {
        return TailJudgement { expected, fault: Some("the tail is not on the old stream's clock"), cut_verified };
    }
    if let Some(first) = first_undelivered
        && u64::try_from(tail.ticks_in(old.domain).unwrap_or(-1)).ok() != Some(first)
    {
        return TailJudgement { expected, fault: Some("the tail is not at the old stream's first undelivered sample"), cut_verified };
    }
    if let Some(cut) = applied_cut
        && old.end != Some(cut)
    {
        return TailJudgement { expected, fault: Some("the old clock's end is not the applied stop's cut"), cut_verified };
    }
    if let Some(booked) = booked_at
        && let Some(end) = old.end
        && end < booked
    {
        return TailJudgement { expected, fault: Some("the old clock's end is before the booked Stop's instant"), cut_verified };
    }
    TailJudgement { expected, fault: None, cut_verified }
}

/// The cell the booking's host stamps and the trace fall in. A is certain only when the pre-stamp is strictly after the overflow
/// result's `after`, no `samples` call from the old stream began before the booking completed (one that began earlier may have
/// returned the restart's data), the booking ended inside `window_ns` of the result call, and the physical stop pair ended at least
/// the margin before UHD's own restart; B only when the post-stamp is strictly before its `before` and a post-stall samples row ended
/// before the pre-stamp (the booking met the backlog the reader was returning).
pub fn s9_classify(pre_stamp_ns: u64, post_stamp_ns: u64, rows: &[Json], stall_ns: u64, window_ns: u64, restart_after_ns: u64, rows_complete: bool) -> Cell {
    if !rows_complete {
        return Cell::Inconclusive("the probe dropped rows");
    }
    if rows.iter().any(|row| row["call"] == "rx_recv" && recv_row(row).is_none()) {
        return Cell::Inconclusive(S9_UNSTAMPED);
    }
    let recvs = recv_rows(rows);
    if recvs.iter().filter(|row| is_result(row)).count() > 1 {
        return Cell::Inconclusive(S9_MORE_OVERFLOWS);
    }
    let (resume, result_index) = match locate(&recvs, stall_ns) {
        Ok(found) => found,
        Err(reason) => return Cell::Inconclusive(reason),
    };
    let result = &recvs[result_index];
    if pre_stamp_ns > result.after {
        // A `samples` call that began before the booking was complete may have returned the restart's data in the meantime; the
        // successor's own calls (those from its `rx_start` call on) are not the old stream's.
        let successor_start = rows.iter().filter(|row| row["call"] == "rx_start").filter_map(|row| row["host_before_ns"].as_u64()).find(|&before| before >= pre_stamp_ns);
        // The physical stop pair: the `rx_stop` rows begun at or after the pre-stamp and, when the successor's start is in the trace,
        // before it. A row whose result records an error is no stop that took effect, so it is not the premise (whatever its stamps).
        let pair: Vec<&Json> = rows
            .iter()
            .filter(|row| row["call"] == "rx_stop")
            .filter(|row| row["host_before_ns"].as_u64().is_some_and(|before| before >= pre_stamp_ns && successor_start.is_none_or(|start| before < start)))
            .collect();
        if pair.iter().any(|row| !row["result"].is_null()) {
            return Cell::Inconclusive(S9_STOP_FAILED);
        }
        // The last pair row's end is when the device was certainly stopped.
        let stop_end = pair.last().and_then(|row| row["host_after_ns"].as_u64());
        let Some(stop_end) = stop_end else { return Cell::Inconclusive(S9_NO_STOP) };
        let bound = post_stamp_ns.max(stop_end);
        let resumed_before = successor_start.map_or(bound, |start| start.min(bound));
        if recvs[result_index + 1..].iter().any(|row| row.kind == "samples" && row.before < resumed_before) {
            return Cell::Inconclusive(S9_RESUMED);
        }
        // The Module's stop at the cut must come before UHD's own restart reached it: the booking ended inside `window_ns` of the result call, and the physical stop pair's end is at least the margin before the earliest restart after the result call began.
        return if post_stamp_ns < result.before.saturating_add(window_ns) && stop_end < result.before.saturating_add(restart_after_ns).saturating_sub(S9_RESTART_MARGIN_NS) {
            Cell::A
        } else {
            Cell::Inconclusive(S9_NEAR_RESTART)
        };
    }
    let backlog = recvs[resume..].iter().any(|row| row.kind == "samples" && row.after < pre_stamp_ns);
    if post_stamp_ns < result.before && backlog {
        return Cell::B;
    }
    Cell::Inconclusive("the booking is neither certainly after the overflow result nor inside the backlog the reader was returning")
}

/// The booking state the reader's trace shows as it grows: the watcher reads only the rows added since its last look, so that a
/// 1 ms poll over a trace of tens of thousands of rows stays cheap. The rules are the classifier's: the same row reader and the
/// same `is_resume`, `backlog` from a post-resume `samples` row and `overflow` from a post-resume overrun result.
#[derive(Clone, Debug, Default)]
pub struct S9Watch {
    seen: usize,
    last_after: Option<u64>,
    stall_ns: u64,
    pub resumed: bool,
    pub backlog: bool,
    pub overflow: bool,
}

impl S9Watch {
    pub fn new(stall_ns: u64) -> S9Watch {
        S9Watch { stall_ns, ..S9Watch::default() }
    }

    /// Reads `rows[self.seen..]` only (then advances its mark) and keeps the first resume and what followed it.
    pub fn feed(&mut self, rows: &[Json]) {
        let Some(fresh) = rows.get(self.seen..) else { return };
        for recv in fresh.iter().filter_map(recv_row) {
            if !self.resumed && self.last_after.is_some_and(|previous| is_resume(previous, recv.before, self.stall_ns)) {
                self.resumed = true;
            }
            if self.resumed && recv.kind == "samples" {
                self.backlog = true;
            }
            if self.resumed && is_result(&recv) {
                self.overflow = true;
            }
            self.last_after = Some(recv.after);
        }
        self.seen = rows.len();
    }
}

/// What one S9 trial showed: the successor's first sample and published block, the old segment's tail, and the reports the Run delivered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct S9Trial {
    /// A sample came back from the device after the booking's `rx_start` (the probe's row): the successor ran.
    pub sample_seen: bool,
    /// The successor's first block as published (D1); `None` when the link saw none on the successor's clock.
    pub first_block: Option<FirstBlock>,
    /// The old clock's end was checked against the applied stop's cut AND the booked Stop's instant (D3), and both held.
    pub cut_verified: bool,
    /// `lost` of every RX_OVERFLOW (cause overrun) the Run delivered. The classifier admits one non-sequence overflow row only, so these are the old segment's one report (two events from one row would be a defect).
    pub tails: Vec<u64>,
    /// What the old segment owes: its clock's end sample less the first sample not delivered; `None` when there is not exactly one
    /// tail or it cannot be computed.
    pub expected_tail: Option<u64>,
    /// What is wrong with where the one tail event stands (its clock, its tick, or the old clock's end against the applied cut).
    pub tail_fault: Option<&'static str>,
    /// RX_OVERFLOW events (cause sequence) at the successor's first block: the cascade race (UR-19). A host drop elsewhere in the Run is not counted (`sequence_events`).
    pub sequence_overflows: usize,
    /// Every RX_OVERFLOW event (cause sequence) the Run delivered, wherever it is (a reported host drop; for the record only).
    pub sequence_events: usize,
    /// START_LEAD_SHORT events.
    pub start_lead_short: usize,
    /// LATE_COMMAND events.
    pub late_command: usize,
    /// The Provider's `rx_before_origin` count.
    pub rx_before_origin: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass(String),
    Fail(String),
    Inconclusive(String),
}

/// The verdict on one trial: every broken rule is named in one FAIL, so that one report carries them all.
pub fn s9_verdict(cell: &Cell, trial: &S9Trial) -> Verdict {
    if let Cell::Inconclusive(reason) = cell {
        return Verdict::Inconclusive((*reason).to_owned());
    }
    let mut failures: Vec<String> = Vec::new();
    if !trial.sample_seen {
        failures.push("no successor sample".to_owned());
    }
    if trial.tails.is_empty() {
        failures.push("the tail is absent".to_owned());
    }
    if trial.tails.len() > 1 {
        failures.push("more than one tail".to_owned());
    }
    if let ([tail], Some(expected)) = (trial.tails.as_slice(), trial.expected_tail)
        && expected != *tail
    {
        failures.push("the tail's lost is wrong".to_owned());
    }
    if let Some(fault) = trial.tail_fault {
        failures.push(fault.to_owned());
    }
    if trial.sequence_overflows > 0 {
        failures.push("a sequence-error overflow at the successor's first block (the cascade race)".to_owned());
    }
    if trial.rx_before_origin > 0 {
        failures.push("old-stream samples reached the successor before its origin (the cascade race)".to_owned());
    }
    match cell {
        Cell::A => {
            if let Some(block) = trial.first_block {
                if block.index != 0 {
                    failures.push("the successor's first published block is not on time".to_owned());
                }
                if block.gap_before {
                    failures.push("the successor's first published block carries GAP_BEFORE".to_owned());
                }
                if block.seq_discontinuity {
                    failures.push("the successor's first published block carries SEQ_DISCONTINUITY".to_owned());
                }
            }
            if trial.late_command > 0 {
                failures.push("LATE_COMMAND in cell A (the stop-at-the-cut fallback)".to_owned());
            }
        }
        Cell::B => {
            if trial.first_block.is_some_and(|block| block.index > 0) && trial.late_command == 0 {
                failures.push("the start missed its origin with no LATE_COMMAND (a START_LEAD_SHORT start keeps its origin: RM-25)".to_owned());
            }
        }
        Cell::Inconclusive(_) => {}
    }
    if !failures.is_empty() {
        return Verdict::Fail(failures.join("; "));
    }
    if trial.sample_seen && trial.first_block.is_none() {
        return Verdict::Inconclusive("the successor's first published block was not observed".to_owned());
    }
    if trial.tails.len() == 1 && trial.expected_tail.is_none() {
        return Verdict::Inconclusive("the expected tail cannot be computed".to_owned());
    }
    if trial.tails.len() == 1 && !trial.cut_verified {
        return Verdict::Inconclusive("the old clock's cut cannot be verified against the applied stop and the booked Stop".to_owned());
    }
    Verdict::Pass(format!("START_LEAD_SHORT {}, LATE_COMMAND {}", trial.start_lead_short, trial.late_command))
}

/// The decision line of one target cell: every trial's class and verdict is counted, and the first FAIL's reason is named.
pub fn s9_cell_decision(name: &str, target: &Cell, trials: &[(Cell, Verdict)]) -> String {
    let classify = |wanted: &Cell| trials.iter().filter(|(cell, _)| cell == wanted).count();
    let (a, b) = (classify(&Cell::A), classify(&Cell::B));
    let cell_inconclusive = trials.len() - a - b;
    let hits = classify(target);
    let (mut pass, mut verdict_inconclusive) = (0, 0);
    let mut failures: Vec<&str> = Vec::new();
    for (_, verdict) in trials {
        match verdict {
            Verdict::Pass(_) => pass += 1,
            Verdict::Fail(reason) => failures.push(reason),
            Verdict::Inconclusive(_) => verdict_inconclusive += 1,
        }
    }
    let target = match target {
        Cell::A => "A",
        Cell::B => "B",
        Cell::Inconclusive(_) => "INCONCLUSIVE",
    };
    let mut line = format!(
        "S9 decision {name}: {} trials, target {target} reached {hits}; classified A {a}, B {b}, INCONCLUSIVE {cell_inconclusive}; verdicts pass {pass}, FAIL {}, inconclusive {verdict_inconclusive}",
        trials.len(),
        failures.len()
    );
    if let Some(first) = failures.first() {
        line.push_str(&format!("; first FAIL: {first}"));
    }
    line
}

// ---------------------------------------------------------------- S9's trial: one Session Run, one stall, one booking

/// Which state of the reader's trace a booking waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S9Target {
    A,
    B,
}

/// One S9 trial: the receive rate, the reader's stall and where in it the booking is made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S9Plan {
    pub rate_hz: u64,
    pub stall_ms: u64,
    pub after_ms: u64,
    pub restart_after_ns: u64,
    pub retune: bool,
    pub target: S9Target,
}

/// One S9 trial's outcome: the cell its stamps fall in, what the Run showed, and the verdict.
#[derive(Clone, Debug)]
pub struct S9Outcome {
    pub cell: Cell,
    pub trial: S9Trial,
    pub verdict: Verdict,
    pub pre_stamp_ns: u64,
    pub post_stamp_ns: u64,
    pub window_ns: u64,
    pub result_ns: Option<(u64, u64)>,
    pub rows: usize,
}

/// One S9 trial on a fresh Session Run: a reader stall `after_ms` into the stream, a booking made once the probe's trace reaches
/// `plan.target`, and the classification and verdict of what followed. One Run per trial, because the stall fires once per Run and
/// `restart_trial`'s `clear_rows` would destroy the stall's gap and the overflow result.
///
/// Cell B needs the device's drain after the stall (the backlog the reader returns), which only the USRP has: the FakeDevice reports
/// the overflow at once and has no backlog, so its rehearsal reaches cell A only. A rehearsal cannot hold the reader on the
/// manual-clock fake: the held reader is a busy participant, so the clock stops and uhd-control's poll never ends, and the Session's
/// first booking meets the Kernel's KC-21a budget (5000 ms). The rehearsal therefore relies on the classifier to say when its premise
/// failed: `Cell::Inconclusive(S9_RESUMED)` names a booking the stream's restart overtook and `Cell::Inconclusive(S9_NEAR_RESTART)` one
/// booked too close to the device's own restart, and only those outcomes are retried.
pub fn s9_trial(device: Arc<dyn Device>, plan: &S9Plan) -> Result<S9Outcome, String> {
    s9_trial_with(device, plan, |_| {})
}

/// The S9 Session's initial requests: receive channel c0 in stream 0 and the receive rate, and no frequency. The Session therefore starts at the board's default
/// (1 GHz, RM-5), so the retune's target `S9_RETUNE_HZ` crosses the 1 GHz band edge; a frequency added here would move the starting point.
fn s9_initial(profile: &Json, rate_hz: u64) -> Result<ezsdr_kernel::session::InitialRequests, String> {
    let rate = Scalar::try_from(rate_hz as f64).map_err(|()| "S9's receive rate is not a finite number".to_owned())?;
    Ok(super::with_initial(super::rx0(profile), &[("radio.rx.s0.sample_rate_hz", rate)]))
}

/// `s9_trial` with one hook run once the booking state was reached and before the booking's stamps are read: a rehearsal's
/// `overfill` makes the trace incomplete through the real body.
pub fn s9_trial_with(device: Arc<dyn Device>, plan: &S9Plan, before_booking: impl FnOnce(&super::probe_device::ProbeDevice)) -> Result<S9Outcome, String> {
    let probe = Arc::new(super::probe_device::ProbeDevice::new(device.clone()));
    let dir = super::TempDir::new();
    let profile = super::bench_profile(&*probe, &dir, json!({}), json!({}), true);
    let block_len = profile["bindings"]["radio"]["selector"]["block_len"].as_u64().unwrap_or(u64::from(ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN));
    let window_ns = s9_booking_window_ns(plan.restart_after_ns, s9_span_ns(block_len, plan.rate_hz));
    let initial = s9_initial(&profile, plan.rate_hz)?;
    let mut assembly = super::assembly(&profile, probe.clone(), BTreeMap::new(), |radio| radio.with_rx_stall(Wall::from_millis(plan.after_ms), Wall::from_millis(plan.stall_ms)));
    // D1: the links a test creates through the Assembly are the objects the Kernel attaches to the Provider's ports, so a recording
    // wrapper around the host Link sees each published block without new production surface.
    let seen: FirstBlocks = Default::default();
    for link in assembly.links.values_mut() {
        *link = Box::new(FirstBlockLink::new(seen.clone()));
    }
    let mut run = super::session_on(&profile, &initial, assembly);
    super::past_t0(&mut run, super::ms(100));

    let stall_ns = plan.stall_ms.saturating_mul(1_000_000);
    let mut watch = S9Watch::new(stall_ns);
    let bound_ms = i64::try_from(plan.after_ms.saturating_add(plan.stall_ms).saturating_add(3_000)).map_err(|_| "S9's booking bound exceeds i64 ms".to_owned())?;
    let reached = measurement::advance_until(&mut run, bound_ms, |_| {
        probe.with_rows(|rows| {
            watch.feed(rows);
            match plan.target {
                S9Target::A => watch.overflow,
                S9Target::B => watch.backlog && !watch.overflow,
            }
        })
    });
    if !reached {
        let reason = "the booking state was not reached";
        let rows = probe.rows().len();
        drop(run.finish());
        return Ok(S9Outcome {
            cell: Cell::Inconclusive(reason),
            trial: S9Trial::default(),
            verdict: Verdict::Inconclusive(reason.to_owned()),
            pre_stamp_ns: 0,
            post_stamp_ns: 0,
            window_ns: 0,
            result_ns: None,
            rows,
        });
    }
    before_booking(&probe);
    let start_index = probe.count("rx_start");
    let pre_stamp_ns = probe.host_ns();
    let values = plan.retune.then_some((S9_RETUNE_HZ, 3.0));
    let booked = measurement::book_receive_stop(&mut run, values, 1).and_then(|()| {
        let start = run.submit(super::verb("start_rx", super::resource("radio/rx/s0"), None, &[]), None).map_err(|error| error.to_string())?;
        if super::admitted(&start) { Ok(()) } else { Err(format!("the receive start was not admitted: {:?}", start.outcome)) }
    });
    let post_stamp_ns = probe.host_ns();
    booked?;

    let _ = measurement::advance_until(&mut run, measurement::DEVICE_WAIT_MS, |_| super::probe_device::rx_start_and_sample(&probe, start_index).is_some());
    super::wait(&mut run, super::ms(100));
    let manifest = run.finish();
    let rows = probe.rows();
    let cell = s9_classify(pre_stamp_ns, post_stamp_ns, &rows, stall_ns, window_ns, plan.restart_after_ns, probe.rows_complete().is_ok());
    let trial = s9_extract(&manifest, &probe, start_index, pre_stamp_ns, plan, device.master_clock_rate(), &seen);
    let verdict = s9_verdict(&cell, &trial);
    Ok(S9Outcome { cell, trial, verdict, pre_stamp_ns, post_stamp_ns, window_ns, result_ns: s9_result_stamps(&rows, stall_ns), rows: rows.len() })
}

/// The successor's first sample, the old segment's tail and the reports one finished S9 Run delivered. Only an `rx_start` row issued
/// after the booking began (its `host_before_ns` at or after the pre-stamp) can be the successor's: the first stream's start came
/// before it. Sequence events count against the verdict only at the successor's first block (`s9_split_overflows`).
fn s9_extract(manifest: &Manifest, probe: &super::probe_device::ProbeDevice, start_index: usize, pre_stamp_ns: u64, plan: &S9Plan, master_clock_hz: u64, seen: &FirstBlocks) -> S9Trial {
    let mut trial = S9Trial::default();
    let ticks_per_sample = i64::try_from(master_clock_hz / plan.rate_hz).unwrap_or(0);
    let successor_origin = probe.with_rows(|rows| s9_successor_origin(rows, start_index, pre_stamp_ns));
    if let Some(row) = super::probe_device::rx_start_and_sample(probe, start_index)
        && row["rx_start"]["host_before_ns"].as_u64() >= Some(pre_stamp_ns)
        && let (Some(origin), Some(first_tick)) = (row["rx_start"]["at"].as_i64(), row["rx_sample"]["outcome"]["first_tick"].as_i64())
        && ticks_per_sample > 0
        && first_tick >= origin
    {
        trial.sample_seen = true;
    }
    // D1: the successor's clock is the usrp/rx/s0 record whose origin is the booked instant; the link recorded the block it published first.
    if let Some(origin) = successor_origin
        && let Some(clock) = manifest.clocks.sample_clocks.iter().find(|clock| clock.stream.to_string().ends_with("usrp/rx/s0") && clock.origin.ticks_in(clock.root).ok() == Some(origin))
    {
        trial.first_block = seen.lock().unwrap().get(&clock.domain).copied();
    }
    let mut overflows = Vec::new();
    for event in &manifest.events.delivered {
        if event.kind == super::kind("radio.RX_OVERFLOW") {
            let Ok(payload) = RxOverflowPayload::from_payload(&event.payload) else { continue };
            overflows.push((payload.cause, payload.lost, event.time));
        } else if event.kind == super::kind("radio.LATE_COMMAND") {
            trial.late_command += 1;
        } else if event.kind == super::kind("radio.START_LEAD_SHORT") {
            trial.start_lead_short += 1;
        }
    }
    let split = s9_split_overflows(&overflows);
    trial.sequence_events = split.sequence_events;
    trial.sequence_overflows = split.first_block_sequences;
    trial.tails = split.tails;
    trial.rx_before_origin = super::section_unchecked(manifest, "stats")["rx_before_origin"].as_u64().unwrap_or(0);
    if trial.tails.len() == 1 {
        // A is the first stream's clock: the sample clock with the smallest origin in root ticks. Fewer than two clocks means there is
        // no successor to tell it apart from, so the tail stays not computable. Its record is kept for the booked Stop's root.
        let old = if manifest.clocks.sample_clocks.len() >= 2 {
            manifest
                .clocks
                .sample_clocks
                .iter()
                .filter_map(|clock| {
                    let origin = clock.origin.ticks_in(clock.root).ok()?;
                    let end = clock.ended_at.and_then(|end| end.ticks_in(clock.root).ok());
                    Some((origin, clock, SegClock { domain: clock.domain, origin, end }))
                })
                .min_by_key(|(origin, _, _)| *origin)
                .map(|(_, record, seg)| (record, seg))
        } else {
            None
        };
        if let (Some((record, old)), Some(tail_time)) = (old, split.tail_time) {
            let stall_ns = plan.stall_ms.saturating_mul(1_000_000);
            let first = probe.with_rows(|rows| s9_first_undelivered(rows, stall_ns, old.origin, ticks_per_sample));
            let applied_cut = super::section_unchecked(manifest, "applied").as_array().and_then(|rows| rows.iter().find(|row| row["key"] == "rx_stop")).and_then(|row| row["at"]["ticks"].as_i64());
            // D3: the trial books an untimed Stop; its logged instant (RS-15) is the booked Stop's time, read on the old clock's root.
            let booked_at = manifest.action_log.iter().find(|entry| matches!(&entry.action, SessionAction::Stop { target: Some(_) })).and_then(|entry| entry.time.ticks_in(record.root).ok());
            let judgement = s9_tail_judgement(&old, ticks_per_sample, first, applied_cut, booked_at, tail_time);
            trial.expected_tail = judgement.expected;
            trial.tail_fault = judgement.fault;
            trial.cut_verified = judgement.cut_verified;
        }
    }
    trial
}

/// What one S9 Run's `RX_OVERFLOW` events owe the verdict.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct S9Overflows {
    /// `lost` of every overrun-cause event, in delivery order.
    pub tails: Vec<u64>,
    /// The time of the last overrun-cause event: its domain is the old segment's SampleClock, its tick the first sample not delivered.
    pub tail_time: Option<TimePoint>,
    /// Every sequence-cause event the Run delivered, wherever it is: a host drop the Provider reported correctly is not a defect.
    pub sequence_events: usize,
    /// The sequence-cause events at the successor's first block: in a SampleClock other than the old segment's, at its tick 0. That is the cascade
    /// race `bench.md` names (the successor's first block `GAP_BEFORE | SEQ_DISCONTINUITY`); the first block is stamped at the stream's `expected` = 0
    /// (`rx.rs`, UR-19).
    pub first_block_sequences: usize,
}

/// Splits the Run's `RX_OVERFLOW` events, as `(cause, lost, time)` in delivery order.
pub fn s9_split_overflows(events: &[(RxOverflowCause, u64, TimePoint)]) -> S9Overflows {
    let mut split = S9Overflows::default();
    for (cause, lost, time) in events {
        match cause {
            RxOverflowCause::Overrun => {
                split.tails.push(*lost);
                split.tail_time = Some(*time);
            }
            RxOverflowCause::Sequence => split.sequence_events += 1,
        }
    }
    // shortcut: with no tail the old segment's domain is unknown and any sequence event at tick 0 counts; the verdict fails that trial for its absent tail anyway.
    let old = split.tail_time.map(TimePoint::domain);
    split.first_block_sequences =
        events.iter().filter(|(cause, _, time)| *cause == RxOverflowCause::Sequence && Some(time.domain()) != old && time.ticks_in(time.domain()).is_ok_and(|ticks| ticks == 0)).count();
    split
}

// ---------------------------------------------------------------- S9's cells: the two rates by retune by target

/// The stalls S9's rate cells use: the same stalls S4's rate cells use for these rates (`S4_RATE_STALLS_MS`).
pub const S9_RATE_STALLS_MS: [(u64, u64); 2] = [(10_000_000, 2_000), (100_000_000, 500)];

/// For each (rate, stall) the same values and a retune, targeting A then B: `trials` trials a cell.
pub fn s9_cells(trials: usize, rate_stalls_ms: &[(u64, u64)], after_ms: u64) -> Vec<(S9Plan, usize)> {
    let mut cells = Vec::new();
    for (rate_hz, stall_ms) in rate_stalls_ms {
        for retune in [false, true] {
            for target in [S9Target::A, S9Target::B] {
                cells.push((S9Plan { rate_hz: *rate_hz, stall_ms: *stall_ms, after_ms, restart_after_ns: UHD_OVERRUN_RESTART_DELAY_NS, retune, target }, trials));
            }
        }
    }
    cells
}

/// What S9 ran: each cell with its planned trial count and its outcomes, and why it stopped early.
#[derive(Debug, Default)]
pub struct S9Run {
    pub cells: Vec<(S9Plan, usize, Vec<S9Outcome>)>,
    pub stopped: Option<String>,
}

/// Runs the cells' trials in order through `s9_trial`, printing a row per trial. Stops at the first `Err`, or when `latched` names a
/// reason; the cells after it are left with fewer trials than planned.
pub fn s9_run(device: &Arc<dyn Device>, cells: &[(S9Plan, usize)], mut latched: impl FnMut() -> Option<String>) -> S9Run {
    let mut run = S9Run::default();
    for (plan, trials) in cells {
        let mut outcomes = Vec::new();
        for _ in 0..*trials {
            if run.stopped.is_some() {
                break;
            }
            match s9_trial(device.clone(), plan) {
                Err(error) => run.stopped = Some(format!("{} MS/s stall {} ms: {error}", plan.rate_hz / 1_000_000, plan.stall_ms)),
                Ok(outcome) => {
                    println!("S9 trial {}", trial_row(plan, &outcome));
                    outcomes.push(outcome);
                    if let Some(reason) = latched() {
                        run.stopped = Some(format!("the device handle is latched ({reason})"));
                    }
                }
            }
        }
        run.cells.push((plan.clone(), *trials, outcomes));
    }
    if let Some(reason) = &run.stopped {
        println!("S9 stop: {reason}; no later trial runs on this device: power-cycle the board and tell the owner");
    }
    run
}

fn target_name(target: S9Target) -> &'static str {
    match target {
        S9Target::A => "A",
        S9Target::B => "B",
    }
}

fn cell_name(cell: &Cell) -> String {
    match cell {
        Cell::A => "A".to_owned(),
        Cell::B => "B".to_owned(),
        Cell::Inconclusive(reason) => format!("INCONCLUSIVE: {reason}"),
    }
}

fn verdict_name(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Pass(text) => format!("PASS: {text}"),
        Verdict::Fail(text) => format!("FAIL: {text}"),
        Verdict::Inconclusive(text) => format!("INCONCLUSIVE: {text}"),
    }
}

fn trial_row(plan: &S9Plan, outcome: &S9Outcome) -> Json {
    json!({
        "rate_hz": plan.rate_hz, "stall_ms": plan.stall_ms, "retune": plan.retune, "target": target_name(plan.target),
        "cell": cell_name(&outcome.cell), "verdict": verdict_name(&outcome.verdict),
        "sample_seen": outcome.trial.sample_seen, "first_index": outcome.trial.first_block.map(|block| block.index),
        "first_gap_before": outcome.trial.first_block.map(|block| block.gap_before), "first_seq_discontinuity": outcome.trial.first_block.map(|block| block.seq_discontinuity),
        "cut_verified": outcome.trial.cut_verified, "tails": outcome.trial.tails, "expected_tail": outcome.trial.expected_tail, "tail_fault": outcome.trial.tail_fault,
        "sequence_overflows": outcome.trial.sequence_overflows, "sequence_events": outcome.trial.sequence_events, "start_lead_short": outcome.trial.start_lead_short,
        "late_command": outcome.trial.late_command, "rx_before_origin": outcome.trial.rx_before_origin,
        "pre_stamp_ns": outcome.pre_stamp_ns, "post_stamp_ns": outcome.post_stamp_ns, "rows": outcome.rows,
        "result_before_ns": outcome.result_ns.map(|(before, _)| before), "result_after_ns": outcome.result_ns.map(|(_, after)| after),
        "window_ns": outcome.window_ns,
    })
}

/// The decision lines of the run: one per cell, with the run's stop reason on the cells it left short of their plan.
pub fn s9_decisions(run: &S9Run) -> Vec<String> {
    run.cells
        .iter()
        .map(|(plan, planned, outcomes)| {
            let target = match plan.target {
                S9Target::A => Cell::A,
                S9Target::B => Cell::B,
            };
            let name = format!("{} MS/s stall {} ms {} target {}", plan.rate_hz / 1_000_000, plan.stall_ms, if plan.retune { "retune" } else { "same" }, target_name(plan.target));
            let trials: Vec<(Cell, Verdict)> = outcomes.iter().map(|outcome| (outcome.cell.clone(), outcome.verdict.clone())).collect();
            let mut line = s9_cell_decision(&name, &target, &trials);
            if let Some(reason) = &run.stopped
                && outcomes.len() < *planned
            {
                line.push_str(&format!("; stopped: {reason}"));
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ezsdr_kernel::contract::{DataContractId, PortRef};
    use ezsdr_kernel::id::{ClockDomainId, DataLinkId};
    use ezsdr_kernel::spec::Ident;
    use ezsdr_kernel::stream::{BlockHeader, ChannelMask, Direction, SampleBlock};
    use serde_json::json;

    /// The stall of the pure tests: a small number, since nothing here is timed.
    const STALL_NS: u64 = 1_000;

    /// The booking window of the pure tests: an A case must end before the base trace's result `before` 1 100 plus this 500, i.e. 1 600.
    const WINDOW_NS: u64 = 500;

    /// The restart delay of the pure tests: the physical stop pair must end before the result's `before` 1 100 plus 2 010 000 less the
    /// 2 ms margin, i.e. 11 100.
    const RESTART_NS: u64 = 2_010_000;

    fn recv(before: u64, after: u64, outcome: Json) -> Json {
        json!({ "call": "rx_recv", "host_before_ns": before, "host_after_ns": after, "outcome": outcome })
    }

    fn samples(before: u64, after: u64) -> Json {
        recv(before, after, json!({ "kind": "samples", "first_tick": 0, "samples": 1_000 }))
    }

    fn samples_of(before: u64, after: u64, first_tick: i64, count: u64) -> Json {
        recv(before, after, json!({ "kind": "samples", "first_tick": first_tick, "samples": count }))
    }

    fn overflow(before: u64, after: u64, out_of_sequence: bool) -> Json {
        recv(before, after, json!({ "kind": "overflow", "out_of_sequence": out_of_sequence }))
    }

    fn at(domain: u32, ticks: i64) -> TimePoint {
        TimePoint::new(ClockDomainId::local(domain), ticks)
    }

    fn event(cause: RxOverflowCause, lost: u64, time: TimePoint) -> (RxOverflowCause, u64, TimePoint) {
        (cause, lost, time)
    }

    /// The base trace: pre-stall samples rows at 0..10 and 10..20, the resume at 1 020 (1 020 - 20 = 1 000, exactly the stall),
    /// post-stall samples rows at 1 020..1 030 and 1 050..1 060, the overflow result at 1 100..1 110 and the resumed stream's
    /// samples row at 1 300..1 310. Every A case below has a post-stamp before 1 300, the resumed row's `before`.
    fn trace() -> Vec<Json> {
        vec![samples(0, 10), samples(10, 20), samples(1_020, 1_030), samples(1_050, 1_060), overflow(1_100, 1_110, false), samples(1_300, 1_310)]
    }

    /// The classifier with a stop pair of its own after the pre-stamp, so that every existing A case keeps meaning what it meant; the
    /// row's end (pre + 2) is far below the 11 100 the restart rule allows.
    fn classify(pre: u64, post: u64, rows: &[Json]) -> Cell {
        let mut rows = rows.to_vec();
        rows.push(json!({ "call": "rx_stop", "stream": 0, "host_before_ns": pre + 1, "host_after_ns": pre + 2, "result": null }));
        s9_classify(pre, post, &rows, STALL_NS, WINDOW_NS, RESTART_NS, true)
    }

    /// The classifier on exactly the rows given, for the cases whose stop pair is the subject.
    fn classify_raw(pre: u64, post: u64, rows: &[Json], window_ns: u64) -> Cell {
        s9_classify(pre, post, rows, STALL_NS, window_ns, RESTART_NS, true)
    }

    const NEITHER: &str = "the booking is neither certainly after the overflow result nor inside the backlog the reader was returning";
    const NO_GAP: &str = "no gap of the stall's length in the trace";
    const NO_RESULT: &str = "no overflow result after the stall";
    const RESUMED: &str = S9_RESUMED;
    const MORE: &str = S9_MORE_OVERFLOWS;
    const NEAR: &str = S9_NEAR_RESTART;
    const UNSTAMPED: &str = S9_UNSTAMPED;

    #[test]
    fn s9_classifies_a_booking_by_its_stamps_and_the_overflow_result() {
        // 1. A: the pre-stamp 1 200 is after the overflow row's after 1 110, and the resumed row's before 1 300 is not before its
        // post-stamp 1 210.
        assert_eq!(classify(1_200, 1_210, &trace()), Cell::A, "case 1: a booking certainly after the result is A");
        // 2. the pre-stamp 1 110 equals the overflow row's after 1 110: strictly later is A, so this is not A.
        assert_eq!(classify(1_110, 1_120, &trace()), Cell::Inconclusive(NEITHER), "case 2: an equal pre-stamp is not A");
        // 3. B: the post-stamp 1 050 is before the overflow row's before 1 100, and the resume row's after 1 030 is before the pre-stamp 1 040.
        assert_eq!(classify(1_040, 1_050, &trace()), Cell::B, "case 3: a booking inside the backlog is B");
        // 4. the post-stamp 1 100 equals the overflow row's before 1 100: strictly earlier is B, so this is not B.
        assert_eq!(classify(1_040, 1_100, &trace()), Cell::Inconclusive(NEITHER), "case 4: an equal post-stamp is not B");
        // 5. a booking inside the gap, while the reader sleeps: no post-stall samples row has its after before the pre-stamp 500.
        assert_eq!(classify(500, 600, &trace()), Cell::Inconclusive(NEITHER), "case 5: a booking while the reader sleeps is inconclusive");
        // 6. a booking just before the stall: the pre-stall samples row 10..20 is before the pre-stamp 25, but no post-stall one is.
        assert_eq!(classify(25, 30, &trace()), Cell::Inconclusive(NEITHER), "case 6: a booking before the stall is inconclusive");
        // 7. a pair straddling the result: the pre-stamp 1 070 has the backlog behind it (1 060 < 1 070) and is not after the result's 1 110,
        // the post-stamp 1 105 is past the result's before 1 100 (and equals neither boundary).
        assert_eq!(classify(1_070, 1_105, &trace()), Cell::Inconclusive(NEITHER), "case 7: a pair straddling the result is inconclusive");
        // 8. the base trace's gap is exactly 1 000 (1 020 - 20): it is the resume, so the base trace is A. One tick less is no resume.
        assert_eq!(classify(1_200, 1_210, &trace()), Cell::A, "case 8: a gap of exactly the stall is a resume");
        let mut short = trace();
        short[2]["host_before_ns"] = json!(1_019); // 1 019 - 20 = 999 < 1 000
        assert_eq!(classify(1_200, 1_210, &short), Cell::Inconclusive(NO_GAP), "case 8: a gap one tick shorter is no resume");
        // 9. no gap in the trace at all (the largest is 30 - 20 = 10).
        let no_gap = vec![samples(0, 10), samples(20, 30), overflow(40, 50, false)];
        assert_eq!(classify(100, 110, &no_gap), Cell::Inconclusive(NO_GAP), "case 9: a trace without a gap is inconclusive");
        // 10. an out-of-sequence overflow at 1 040..1 045 is not the result: the real one at 1 100..1 110 makes this B (post 1 050 < 1 100,
        // resume after 1 030 < pre 1 040). With only the out-of-sequence one, no result is found.
        let mut sequence_first = trace();
        sequence_first.insert(3, overflow(1_040, 1_045, true)); // after the resume row
        assert_eq!(classify(1_040, 1_050, &sequence_first), Cell::B, "case 10: a sequence-error overflow does not decide");
        let only_sequence = vec![samples(0, 10), samples(10, 20), samples(1_020, 1_030), overflow(1_100, 1_110, true), samples(1_120, 1_130)];
        assert_eq!(classify(1_200, 1_210, &only_sequence), Cell::Inconclusive(NO_RESULT), "case 10: a trace whose only overflow is a sequence error is inconclusive");
        // 11. an overflow before the resume is not the result: rows 0..10, overflow 10..20, samples 20..30, resume 1 030 (1 030 - 30 = 1 000),
        // post-stall samples 1 030..1 040 and 1 050..1 060: no overflow after the resume.
        let pre_stall_overflow = vec![samples(0, 10), overflow(10, 20, false), samples(20, 30), samples(1_030, 1_040), samples(1_050, 1_060)];
        assert_eq!(classify(1_070, 1_080, &pre_stall_overflow), Cell::Inconclusive(NO_RESULT), "case 11: a pre-stall overflow does not decide");
        // 12. no overflow row at all: the base trace without its result row still has its resume.
        let mut no_overflow = trace();
        no_overflow.remove(4);
        assert_eq!(classify(1_200, 1_210, &no_overflow), Cell::Inconclusive(NO_RESULT), "case 12: no overflow row is inconclusive");
        // 13. an incomplete record cannot be classified.
        assert_eq!(s9_classify(1_200, 1_210, &trace(), STALL_NS, WINDOW_NS, RESTART_NS, false), Cell::Inconclusive("the probe dropped rows"), "case 13: a dropped row makes the trace inconclusive");
        // 14. foreign rows inside the gap (a time_now and an rx_stop) do not split it: the base trace still classifies A.
        let mut foreign = trace();
        foreign.insert(2, json!({ "call": "time_now", "purpose": "x", "host_before_ns": 500, "host_after_ns": 510, "result": { "value": 1 } }));
        foreign.insert(3, json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 600, "host_after_ns": 610, "result": null }));
        assert_eq!(classify(1_200, 1_210, &foreign), Cell::A, "case 14: a foreign row inside the gap does not split it");
        // 15. a booking that ended after the stream may have resumed: the pre-stamp 1 200 is after the overflow row's after 1 110 (the old
        // rule called this A), but the post-stamp 1 350 is past the resumed row's before 1 300, so that call had begun before the booking
        // completed and may have returned the restart's data first.
        assert_eq!(classify(1_200, 1_350, &trace()), Cell::Inconclusive(RESUMED), "case 15: a booking that ends after the stream may have resumed is inconclusive");
        // 16. the boundary: the post-stamp 1 300 equals the resumed row's before 1 300; strictly earlier is required to count as resumed,
        // so the booking was complete before that call began and met the gap.
        assert_eq!(classify(1_200, 1_300, &trace()), Cell::A, "case 16: an equal resumed-row before leaves the booking inside the gap");
        // 17. a booking made after the restart's data: the pre-stamp 1 400 is past the overflow row's after 1 110 and the resumed row's
        // after 1 310; the booking cannot be inside the gap.
        assert_eq!(classify(1_400, 1_410, &trace()), Cell::Inconclusive(RESUMED), "case 17: a booking after the resumed data is inconclusive");
        // 18. the resumed check reads the row's `before`, not its `after`: the post-stamp 1 305 is past the resumed row's before 1 300 but
        // before its after 1 310, so only the `before` reading catches that the call had begun before the booking completed.
        assert_eq!(classify(1_200, 1_305, &trace()), Cell::Inconclusive(RESUMED), "case 18: a post-stamp inside the resumed row is resumed");
        // 19. the backlog reads the row's `after`, not its `before`: the resume row 1 020..1 030 ended at 1 030, past the pre-stamp 1 025,
        // so the booking did not meet that backlog; its `before` 1 020 would call it backlog and make this B.
        let backlog_row = vec![samples(0, 10), samples(10, 20), samples(1_020, 1_030), overflow(1_100, 1_110, false)];
        assert_eq!(classify(1_025, 1_050, &backlog_row), Cell::Inconclusive(NEITHER), "case 19: a backlog row that ended after the pre-stamp is not backlog");
        // 20. the successor's own samples are not resumed: its start at 1 250 (after the pre-stamp 1 200) makes the first resumed-looking row
        // 1 300..1 310 the successor's, so the booking at 1 350 is still cell A; without the successor rule it is RESUMED.
        let mut successor = trace();
        successor.insert(5, json!({ "call": "rx_start", "stream": 0, "at": 0, "host_before_ns": 1_250, "host_after_ns": 1_260, "result": null }));
        assert_eq!(classify(1_200, 1_350, &successor), Cell::A, "case 20: the successor's own samples are not the old stream's");
        // 21. a start issued before the booking (at 5, before the pre-stamp 1 200) is not the successor's: the 1 300 row is the old stream's.
        let mut first_start = trace();
        first_start.insert(0, json!({ "call": "rx_start", "stream": 0, "at": 0, "host_before_ns": 5, "host_after_ns": 6, "result": null }));
        assert_eq!(classify(1_200, 1_350, &first_start), Cell::Inconclusive(RESUMED), "case 21: a start before the booking is not the successor's");
        // 22. the window's bound is the result's `before` 1 100 + 500 = 1 600, strictly: 1 599 is A, 1 600 is not (the result's `after`
        // 1 110 would put the bound at 1 610).
        let mut quiet = trace();
        quiet.pop();
        assert_eq!(classify(1_200, 1_599, &quiet), Cell::A, "case 22: a booking one nanosecond inside the window is A");
        assert_eq!(classify(1_200, 1_600, &quiet), Cell::Inconclusive(NEAR), "case 22: a booking at the window's bound is not A");
        assert_eq!(classify(1_200, 1_700, &quiet), Cell::Inconclusive(NEAR), "case 22: a booking past the window is not A");
        // 23. a second non-sequence overflow makes the stall's own report indistinguishable from the load's: open, after the result
        // (1 320..1 330) and before the stall (0..5).
        let mut second_overflow = trace();
        second_overflow.push(overflow(1_320, 1_330, false));
        assert_eq!(classify(1_200, 1_210, &second_overflow), Cell::Inconclusive(MORE), "case 23: a second overflow after the result is open");
        let mut early_overflow = trace();
        early_overflow.insert(0, overflow(0, 5, false));
        assert_eq!(classify(1_200, 1_210, &early_overflow), Cell::Inconclusive(MORE), "case 23: an overflow before the stall is open");
        // 24. a sequence-error overflow is not the stall's report: the base trace plus one at 1 320..1 330 is still A.
        let mut sequence_after = trace();
        sequence_after.push(overflow(1_320, 1_330, true));
        assert_eq!(classify(1_200, 1_210, &sequence_after), Cell::A, "case 24: a sequence-error overflow does not count");
        // 25. a rx_recv row without unsigned host stamps cannot be placed in time: the trace is open.
        let mut unstamped = trace();
        unstamped.push(json!({ "call": "rx_recv", "outcome": { "kind": "samples" } }));
        assert_eq!(classify(1_200, 1_210, &unstamped), Cell::Inconclusive(UNSTAMPED), "case 25: a rx_recv row without host stamps is open");
    }

    #[test]
    fn s9_first_index_is_the_lattice_index_the_provider_publishes() {
        // (offset, n) → (offset + n / 2).div_euclid(n); the old floor gave offset.div_euclid(n).
        // Origin 0, first tick 1 at n = 2: (1 + 1) / 2 = 1, the Provider's published index (the floor gave 0).
        assert_eq!(s9_first_index(0, 1, 2), Some(1), "a first raw tick one root tick off the origin is index 1, not 0");
        // 0: (0 + 1) / 2 = 0.
        assert_eq!(s9_first_index(0, 0, 2), Some(0), "the origin itself is index 0");
        // 3: (3 + 1) / 2 = 2 (the floor gave 1).
        assert_eq!(s9_first_index(0, 3, 2), Some(2), "three root ticks round to the nearest sample");
        // Origin 10, first tick 10: (0 + 1) / 2 = 0.
        assert_eq!(s9_first_index(10, 10, 2), Some(0), "the origin is index 0 wherever it is");
        // First tick 9 is before the origin 10: the Provider drops the packet.
        assert_eq!(s9_first_index(10, 9, 2), None, "a tick before the origin has no published block");
        // n 0: no lattice.
        assert_eq!(s9_first_index(0, 5, 0), None, "no ticks per sample is no index");
    }

    #[test]
    fn s9_successor_origin_is_the_booked_rx_starts_at() {
        let rows =
            vec![json!({ "call": "rx_start", "at": 100, "host_before_ns": 10 }), json!({ "call": "rx_start", "at": 500, "host_before_ns": 50 }), json!({ "call": "rx_recv", "host_before_ns": 60 })];
        // The second start (index 1) began at the pre-stamp 50: its booked origin is 500.
        assert_eq!(s9_successor_origin(&rows, 1, 50), Some(500), "the start at the pre-stamp is the successor's");
        // Index 0 began at 10, before the pre-stamp 50: that is the stream's first start, not the successor's.
        assert_eq!(s9_successor_origin(&rows, 0, 50), None, "a start before the pre-stamp is not the successor's");
        // A missing index and a row without `at` have no origin.
        assert_eq!(s9_successor_origin(&rows, 2, 0), None, "no third start");
        let no_at = vec![json!({ "call": "rx_start", "host_before_ns": 50 })];
        assert_eq!(s9_successor_origin(&no_at, 0, 0), None, "a start without at has no origin");
    }

    #[test]
    fn s9_a_first_block_off_the_lattice_by_a_tick_fails_cell_a() {
        // The successor's booked origin is 0 and its first raw tick is 1 at n = 2: the Provider publishes index 1 (rounding, not floor), so
        // the successor's first published block is late and discontinuous. (`s9_first_index` still derives the old stream's first
        // undelivered index; the verdict reads the header the link saw.)
        let index = s9_first_index(0, 1, 2).unwrap();
        assert_eq!(index, 1, "one root tick off a two-tick sample is index 1");
        let trial = S9Trial { first_block: Some(FirstBlock { index, gap_before: true, seq_discontinuity: true }), ..good() };
        match s9_verdict(&Cell::A, &trial) {
            Verdict::Fail(text) => assert!(text.contains("not on time"), "{text}"),
            other => panic!("a successor first block off the lattice must FAIL cell A: {other:?}"),
        }
    }

    #[test]
    fn s9_a_published_first_block_must_be_on_time_and_gapless_in_cell_a() {
        let block = |index, gap_before, seq_discontinuity| FirstBlock { index, gap_before, seq_discontinuity };
        let cell_a = |first_block| s9_verdict(&Cell::A, &S9Trial { first_block: Some(first_block), ..good() });
        assert_eq!(cell_a(block(1, false, false)), Verdict::Fail("the successor's first published block is not on time".to_owned()), "index 1 is late");
        assert_eq!(cell_a(block(0, true, false)), Verdict::Fail("the successor's first published block carries GAP_BEFORE".to_owned()), "a first block carrying a gap is not gapless");
        assert_eq!(
            cell_a(block(0, false, true)),
            Verdict::Fail("the successor's first published block carries SEQ_DISCONTINUITY".to_owned()),
            "a first block carrying a sequence discontinuity is not gapless"
        );
        assert_eq!(
            cell_a(block(0, true, true)),
            Verdict::Fail("the successor's first published block carries GAP_BEFORE; the successor's first published block carries SEQ_DISCONTINUITY".to_owned()),
            "both flags are named in one FAIL"
        );
        assert_eq!(cell_a(block(0, false, false)), Verdict::Pass("START_LEAD_SHORT 0, LATE_COMMAND 0".to_owned()), "index 0 with no flags passes cell A");
        // Cell B: a published index 2 is a missed origin unless a LATE_COMMAND reports it.
        let shifted = FirstBlock { index: 2, gap_before: false, seq_discontinuity: false };
        assert_eq!(
            s9_verdict(&Cell::B, &S9Trial { first_block: Some(shifted), ..good() }),
            Verdict::Fail("the start missed its origin with no LATE_COMMAND (a START_LEAD_SHORT start keeps its origin: RM-25)".to_owned()),
            "cell B with a late published block and no LATE_COMMAND fails"
        );
        assert_eq!(
            s9_verdict(&Cell::B, &S9Trial { first_block: Some(shifted), late_command: 1, ..good() }),
            Verdict::Pass("START_LEAD_SHORT 0, LATE_COMMAND 1".to_owned()),
            "cell B with a reported late start passes"
        );
    }

    #[test]
    fn s9_a_successor_block_the_link_never_saw_is_inconclusive_not_a_pass() {
        // The successor's sample came but the link recorded no block for its clock: the trial cannot say the block was on time.
        assert_eq!(
            s9_verdict(&Cell::A, &S9Trial { first_block: None, ..good() }),
            Verdict::Inconclusive("the successor's first published block was not observed".to_owned()),
            "an unobserved first block is open, not a pass"
        );
        // No sample at all is a failure of the successor's start, not an open observation.
        assert_eq!(s9_verdict(&Cell::A, &S9Trial { sample_seen: false, ..good() }), Verdict::Fail("no successor sample".to_owned()), "no successor sample fails");
    }

    /// One small receive block as `rx.rs` builds it: one channel, one sample, host memory.
    fn block(domain: u32, first: i64, flags: BlockFlags) -> BlockRef {
        let header = BlockHeader {
            first_sample_time: at(domain, first),
            len: 1,
            channels: 1,
            direction: Direction::Rx,
            valid: ChannelMask::full(1),
            flags,
            lost: flags.contains(BlockFlags::GAP_BEFORE).then_some(1),
            contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        };
        Arc::new(SampleBlock::new_host(header, ezsdr_hostmem::HOST_MEMORY, Arc::from(vec![0u8; 8]), 8).unwrap())
    }

    /// A declared host link, as `tests/fake.rs` builds one.
    fn data_link_decl() -> DataLinkDecl {
        DataLinkDecl {
            id: DataLinkId::local(0),
            from: PortRef { component: Ident::parse("radio").unwrap(), port: Ident::parse("rx0").unwrap() },
            to: PortRef { component: Ident::parse("rec").unwrap(), port: Ident::parse("in").unwrap() },
            contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
            policy: BackPressure::DropOldest,
            capacity: 4,
        }
    }

    #[test]
    fn s9_the_link_records_the_first_published_block_of_each_clock() {
        let seen: FirstBlocks = Default::default();
        let link = FirstBlockLink::new(seen.clone()).create(&data_link_decl()).unwrap();
        let first = block(1, 0, BlockFlags::NONE);
        let later = block(1, 100, BlockFlags::GAP_BEFORE);
        let successor = block(2, 3, BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY);
        for queued in [first.clone(), later.clone(), successor.clone()] {
            assert_eq!(link.publish(queued), PublishOutcome::Accepted);
        }
        assert_eq!(
            *seen.lock().unwrap(),
            BTreeMap::from([
                (ClockDomainId::local(1), FirstBlock { index: 0, gap_before: false, seq_discontinuity: false }),
                (ClockDomainId::local(2), FirstBlock { index: 3, gap_before: true, seq_discontinuity: true }),
            ]),
            "one entry per clock, holding the first block it published"
        );
        let received: Vec<BlockRef> = (0..3).map(|_| link.receive().expect("a queued block").0).collect();
        assert_eq!(
            received.iter().map(|block| block.header().first_sample_time).collect::<Vec<_>>(),
            [at(1, 0), at(1, 100), at(2, 3)],
            "the wrapper queues every block in the order it was published and changes nothing"
        );
    }

    #[test]
    fn s9_a_stop_pair_that_ends_after_the_restart_is_inconclusive_not_a_cascade_fail() {
        // The premise of cell A is the physical stop pair, not the booking's post-stamp. The base trace without its resumed samples row (its
        // 1 300 row would begin before the stop end and make the trace RESUMED first); result before 1 100, restart 2 010 000, margin 2 000 000:
        // the stop pair must end before 1 100 + 10 000 = 11 100.
        let mut rows = trace();
        let _ = rows.pop();
        rows.push(json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 1_201, "host_after_ns": 11_100, "result": null }));
        // Post 1 250 < 1 100 + 5 000 = 6 100 (the booking's window passes); 11 100 < 11 100 is false: the stop may have cleared the restart too late.
        assert_eq!(classify_raw(1_200, 1_250, &rows, 5_000), Cell::Inconclusive(S9_NEAR_RESTART), "a stop pair ending at the earliest restart is inconclusive");
        // 11 099 < 11 100: the stop was certainly before the restart, and the booking is inside the window.
        let mut before = rows.clone();
        before.last_mut().unwrap()["host_after_ns"] = json!(11_099);
        assert_eq!(classify_raw(1_200, 1_250, &before, 5_000), Cell::A, "a stop pair ending one ns before the bound is cell A");
    }

    #[test]
    fn s9_a_trace_without_a_stop_pair_after_the_booking_is_inconclusive() {
        // The base trace carries no rx_stop row at all.
        assert_eq!(classify_raw(1_200, 1_210, &trace(), WINDOW_NS), Cell::Inconclusive(S9_NO_STOP), "no stop row is no premise for A");
        // A stop that began before the pre-stamp is not the booking's pair.
        let mut early = trace();
        early.push(json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 1_199, "host_after_ns": 1_205, "result": null }));
        assert_eq!(classify_raw(1_200, 1_210, &early, WINDOW_NS), Cell::Inconclusive(S9_NO_STOP), "a stop before the booking cannot be placed after it");
        // A stop row without its end stamp cannot be placed in time.
        let mut unstamped = trace();
        unstamped.push(json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 1_201, "result": null }));
        assert_eq!(classify_raw(1_200, 1_210, &unstamped, WINDOW_NS), Cell::Inconclusive(S9_NO_STOP), "a stop row without host_after_ns is no end");
    }

    #[test]
    fn s9_a_failed_stop_of_the_pair_is_not_the_cell_a_premise() {
        // A booking after the result (pre 1 200, post 1 210) whose Ok pair ends at 1 280 is cell A; a pair row whose `result` records an
        // error is not a stop that took effect.
        let ok = json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 1_201, "host_after_ns": 1_280, "result": null });
        let failed = json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 1_202, "host_after_ns": 1_281, "result": "the device refused the stop" });
        let pair = |first: &Json, second: &Json| {
            let mut rows = trace();
            rows.push(first.clone());
            rows.push(second.clone());
            classify_raw(1_200, 1_210, &rows, WINDOW_NS)
        };
        assert_eq!(pair(&failed, &ok), Cell::Inconclusive(S9_STOP_FAILED), "a failed first stop followed by an Ok second is not the premise");
        assert_eq!(pair(&ok, &failed), Cell::Inconclusive(S9_STOP_FAILED), "a failed last stop is not the premise");
        // The only pair row failed, and it has no end stamp: STOP_FAILED is decided before S9_NO_STOP needs the end.
        let mut failed_only = trace();
        failed_only.push(json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 1_202, "result": "the device refused the stop" }));
        assert_eq!(classify_raw(1_200, 1_210, &failed_only, WINDOW_NS), Cell::Inconclusive(S9_STOP_FAILED), "a lone failed stop says STOP_FAILED, not NO_STOP");
        // A failed stop outside the pair does not count: one before the pre-stamp and one at or after the successor's start, with the pair Ok.
        let mut outside = trace();
        outside.push(json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 1_199, "host_after_ns": 1_205, "result": "an earlier stop" }));
        outside.push(ok.clone());
        outside.push(json!({ "call": "rx_start", "stream": 0, "at": 0, "host_before_ns": 1_290, "host_after_ns": 1_295, "result": null }));
        outside.push(json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 1_300, "host_after_ns": 1_310, "result": "a later stop" }));
        assert_eq!(classify_raw(1_200, 1_210, &outside, WINDOW_NS), Cell::A, "a failed stop outside the pair is not the pair's");
    }

    #[test]
    fn s9_a_samples_call_begun_before_the_stop_pair_ended_is_the_restart_resuming() {
        // The old rule compared the samples row with the post-stamp alone. Here the samples row 1 260..1 270 begins before the stop pair ended
        // at 1 280, and before any successor start: it may be the restart's data, not the old stream's.
        let mut rows = trace();
        *rows.last_mut().unwrap() = samples(1_260, 1_270);
        rows.push(json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 1_201, "host_after_ns": 1_280, "result": null }));
        // Post 1 210 < 1 100 + 5 000 = 6 100 and stop end 1 280 < 11 100: without the resumed rule this would be A; the samples row's before
        // 1 260 < max(1 210, 1 280) = 1 280 decides it RESUMED.
        assert_eq!(classify_raw(1_200, 1_210, &rows, 5_000), Cell::Inconclusive(S9_RESUMED), "a samples call begun before the stop pair ended is the restart resuming");
    }

    #[test]
    fn s9_a_successors_own_rows_are_still_not_the_old_streams() {
        // The stop pair ends at 1 280; the successor's start begins at 1 290, so the pair's last row before that start is the stop. Its samples
        // row at 1 300 begins after the stop end: the resumed rule does not read the successor's own data.
        let mut rows = trace();
        let _ = rows.pop();
        rows.push(json!({ "call": "rx_stop", "stream": 0, "host_before_ns": 1_201, "host_after_ns": 1_280, "result": null }));
        rows.push(json!({ "call": "rx_start", "stream": 0, "at": 0, "host_before_ns": 1_290, "host_after_ns": 1_295, "result": null }));
        rows.push(samples(1_300, 1_310));
        // Post 1 210 < 1 100 + 500 = 1 600 and stop end 1 280 < 11 100: cell A.
        assert_eq!(classify_raw(1_200, 1_210, &rows, WINDOW_NS), Cell::A, "the successor's own samples are not the old stream's resume");
    }

    #[test]
    fn s9_watch_agrees_with_the_classifier_on_a_trace_fed_in_pieces() {
        let rows = trace();
        // The gap is between rows 1 and 2; the backlog rows are 2 and 3: the trace is fed as prefixes split inside the gap and inside the backlog.
        let mut watch = S9Watch::new(STALL_NS);
        watch.feed(&rows[..2]);
        assert!(!watch.resumed && !watch.backlog && !watch.overflow, "before the resume row the watch has seen nothing: {watch:?}");
        watch.feed(&rows[..4]);
        assert!(watch.resumed && watch.backlog && !watch.overflow, "the resume and the last backlog row, no result yet: {watch:?}");
        watch.feed(&rows);
        assert!(watch.resumed && watch.backlog && watch.overflow, "the rest of the trace gives the result: {watch:?}");
        let mut foreign = S9Watch::new(STALL_NS);
        let mut interleaved = rows.clone();
        interleaved.insert(2, json!({ "call": "time_now", "host_before_ns": 500, "host_after_ns": 510 }));
        foreign.feed(&interleaved[..3]);
        foreign.feed(&interleaved[..5]);
        foreign.feed(&interleaved);
        assert!(foreign.resumed && foreign.backlog && foreign.overflow, "a foreign row inside the gap does not change the watch: {foreign:?}");
        // A sequence-error overflow after the resume is not the result.
        let mut sequence = S9Watch::new(STALL_NS);
        sequence.feed(&[samples(0, 10), samples(10, 20), samples(1_020, 1_030), overflow(1_040, 1_050, true)]);
        assert!(!sequence.overflow, "an out-of-sequence overflow does not set the result: {sequence:?}");
    }

    #[test]
    fn s9_a_sequence_event_counts_only_at_the_successors_first_block() {
        // The old segment is domain 1 and the successor domain 2; the stall's own event is an overrun at the old segment's tick 5 000.
        let tail = event(RxOverflowCause::Overrun, 20_000, at(1, 5_000));
        // 1. a host drop before the stall (at 700), one in the backlog (at 3 000) and one on a later successor block (at 4 096): none is the
        // successor's first block, which is tick 0 of domain 2.
        let elsewhere = vec![event(RxOverflowCause::Sequence, 0, at(1, 700)), event(RxOverflowCause::Sequence, 0, at(1, 3_000)), tail, event(RxOverflowCause::Sequence, 0, at(2, 4_096))];
        assert_eq!(
            s9_split_overflows(&elsewhere),
            S9Overflows { tails: vec![20_000], tail_time: Some(at(1, 5_000)), sequence_events: 3, first_block_sequences: 0 },
            "case 1: sequence events before the stall, in the backlog and on a later successor block are not the race"
        );
        // 2. the same events plus one at the successor's first block (domain 2, tick 0), placed before the tail in the list: the scan's order
        // does not matter, so the count is 4 sequence events and 1 first-block event.
        let mut first_block = elsewhere.clone();
        first_block.insert(2, event(RxOverflowCause::Sequence, 0, at(2, 0)));
        assert_eq!(
            s9_split_overflows(&first_block),
            S9Overflows { tails: vec![20_000], tail_time: Some(at(1, 5_000)), sequence_events: 4, first_block_sequences: 1 },
            "case 2: a sequence event at the successor's tick 0 is the race, wherever it sits in the list"
        );
        // 3. a sequence event at tick 0 of the old segment's own domain (1) is not the successor's block: 0.
        let old_domain = vec![event(RxOverflowCause::Sequence, 0, at(1, 0)), tail];
        assert_eq!(
            s9_split_overflows(&old_domain),
            S9Overflows { tails: vec![20_000], tail_time: Some(at(1, 5_000)), sequence_events: 1, first_block_sequences: 0 },
            "case 3: tick 0 of the old segment's domain is not the successor's first block"
        );
        // 4. two overrun-cause events [10 at domain 1 tick 0, 20 at domain 2 tick 5] are both tails, not sequence events: the last tail's domain
        // is 2, so the event at domain 1 tick 0 would match the first-block test if the cause were not consulted.
        let two_tails = vec![event(RxOverflowCause::Overrun, 10, at(1, 0)), event(RxOverflowCause::Overrun, 20, at(2, 5))];
        assert_eq!(
            s9_split_overflows(&two_tails),
            S9Overflows { tails: vec![10, 20], tail_time: Some(at(2, 5)), sequence_events: 0, first_block_sequences: 0 },
            "case 4: an overrun-cause event is never a sequence event, even at tick 0 of a domain other than the last tail's"
        );
        // 5. no tail: the old segment's domain is unknown, so the documented shortcut counts a sequence event at tick 0 of domain 2.
        let no_tail = vec![event(RxOverflowCause::Sequence, 0, at(2, 0))];
        assert_eq!(
            s9_split_overflows(&no_tail),
            S9Overflows { tails: Vec::new(), tail_time: None, sequence_events: 1, first_block_sequences: 1 },
            "case 5: with no tail any sequence event at tick 0 counts"
        );
        // 6. no events at all: the default.
        assert_eq!(s9_split_overflows(&[]), S9Overflows::default(), "case 6: no events split to the default");
    }

    /// The base trial: sample seen, the first published block on time and gapless, the cut verified, one tail of 20 000, which is also what it owes.
    fn good() -> S9Trial {
        S9Trial {
            sample_seen: true,
            first_block: Some(FirstBlock { index: 0, gap_before: false, seq_discontinuity: false }),
            cut_verified: true,
            tails: vec![20_000],
            expected_tail: Some(20_000),
            ..S9Trial::default()
        }
    }

    #[test]
    fn s9_verdicts_need_the_successor_block_and_the_tail() {
        let mut reported = good();
        reported.start_lead_short = 2;
        assert_eq!(s9_verdict(&Cell::A, &reported), Verdict::Pass("START_LEAD_SHORT 2, LATE_COMMAND 0".to_owned()), "A pass reports the START_LEAD_SHORT count");
        let mut absent = good();
        absent.tails.clear();
        assert_eq!(s9_verdict(&Cell::A, &absent), Verdict::Fail("the tail is absent".to_owned()), "A without the tail fails");
        let mut two = good();
        two.tails.push(20_001);
        assert_eq!(s9_verdict(&Cell::A, &two), Verdict::Fail("more than one tail".to_owned()), "A with two tails fails");
        let mut wrong = good();
        wrong.tails = vec![19_999];
        assert_eq!(s9_verdict(&Cell::A, &wrong), Verdict::Fail("the tail's lost is wrong".to_owned()), "A with a wrong tail fails");
        let mut late = good();
        late.late_command = 1;
        assert_eq!(s9_verdict(&Cell::A, &late), Verdict::Fail("LATE_COMMAND in cell A (the stop-at-the-cut fallback)".to_owned()), "A with LATE_COMMAND fails");
        let mut shifted = good();
        shifted.first_block = Some(FirstBlock { index: 3, gap_before: false, seq_discontinuity: false });
        assert_eq!(s9_verdict(&Cell::A, &shifted), Verdict::Fail("the successor's first published block is not on time".to_owned()), "A whose first block is not on time fails");
        let mut missing = good();
        missing.sample_seen = false;
        assert_eq!(s9_verdict(&Cell::A, &missing), Verdict::Fail("no successor sample".to_owned()), "A without a successor sample fails");
        let mut sequence = good();
        sequence.sequence_overflows = 1;
        assert_eq!(
            s9_verdict(&Cell::A, &sequence),
            Verdict::Fail("a sequence-error overflow at the successor's first block (the cascade race)".to_owned()),
            "A with a sequence-error overflow at the successor's first block fails"
        );
        let mut before_origin = good();
        before_origin.rx_before_origin = 1;
        assert_eq!(
            s9_verdict(&Cell::A, &before_origin),
            Verdict::Fail("old-stream samples reached the successor before its origin (the cascade race)".to_owned()),
            "A with old-stream samples before the origin fails"
        );

        assert_eq!(s9_verdict(&Cell::B, &good()), Verdict::Pass("START_LEAD_SHORT 0, LATE_COMMAND 0".to_owned()), "B on time passes");
        let mut late_b = good();
        late_b.first_block = Some(FirstBlock { index: 3, gap_before: false, seq_discontinuity: false });
        late_b.late_command = 1;
        assert_eq!(s9_verdict(&Cell::B, &late_b), Verdict::Pass("START_LEAD_SHORT 0, LATE_COMMAND 1".to_owned()), "B with a reported late start passes");
        let mut short_b = good();
        short_b.first_block = Some(FirstBlock { index: 3, gap_before: false, seq_discontinuity: false });
        short_b.start_lead_short = 1;
        assert_eq!(
            s9_verdict(&Cell::B, &short_b),
            Verdict::Fail("the start missed its origin with no LATE_COMMAND (a START_LEAD_SHORT start keeps its origin: RM-25)".to_owned()),
            "B whose START_LEAD_SHORT shifted the first block fails: the start kept its origin"
        );
        let mut short_b0 = good();
        short_b0.start_lead_short = 1;
        assert_eq!(s9_verdict(&Cell::B, &short_b0), Verdict::Pass("START_LEAD_SHORT 1, LATE_COMMAND 0".to_owned()), "B with a reported short lead that kept its origin passes");
        let mut unreported = good();
        unreported.first_block = Some(FirstBlock { index: 3, gap_before: false, seq_discontinuity: false });
        assert_eq!(
            s9_verdict(&Cell::B, &unreported),
            Verdict::Fail("the start missed its origin with no LATE_COMMAND (a START_LEAD_SHORT start keeps its origin: RM-25)".to_owned()),
            "B with an unreported missed start fails"
        );
        let mut absent_b = good();
        absent_b.tails.clear();
        assert_eq!(s9_verdict(&Cell::B, &absent_b), Verdict::Fail("the tail is absent".to_owned()), "B without the tail fails");
        let mut wrong_b = good();
        wrong_b.tails = vec![19_999];
        assert_eq!(s9_verdict(&Cell::B, &wrong_b), Verdict::Fail("the tail's lost is wrong".to_owned()), "B with a wrong tail fails");
        let mut sequence_b = good();
        sequence_b.sequence_overflows = 1;
        assert_eq!(
            s9_verdict(&Cell::B, &sequence_b),
            Verdict::Fail("a sequence-error overflow at the successor's first block (the cascade race)".to_owned()),
            "B with a sequence-error overflow at the successor's first block fails"
        );
        let mut host_drops = good();
        host_drops.sequence_events = 3;
        assert_eq!(
            s9_verdict(&Cell::A, &host_drops),
            Verdict::Pass("START_LEAD_SHORT 0, LATE_COMMAND 0".to_owned()),
            "A with host drops reported elsewhere in the Run passes: only the successor's first block is the race"
        );
        assert_eq!(
            s9_verdict(&Cell::B, &host_drops),
            Verdict::Pass("START_LEAD_SHORT 0, LATE_COMMAND 0".to_owned()),
            "B with host drops reported elsewhere in the Run passes: only the successor's first block is the race"
        );
        let mut before_origin_b = good();
        before_origin_b.rx_before_origin = 1;
        assert_eq!(
            s9_verdict(&Cell::B, &before_origin_b),
            Verdict::Fail("old-stream samples reached the successor before its origin (the cascade race)".to_owned()),
            "B with old-stream samples before the origin fails"
        );
        let mut missing_b = good();
        missing_b.sample_seen = false;
        assert_eq!(s9_verdict(&Cell::B, &missing_b), Verdict::Fail("no successor sample".to_owned()), "B without a successor sample fails");

        let mut broken = good();
        broken.tails.clear();
        broken.sequence_overflows = 1;
        assert_eq!(s9_verdict(&Cell::Inconclusive("x"), &broken), Verdict::Inconclusive("x".to_owned()), "an inconclusive cell is judged by nothing else");
        let mut unknown = good();
        unknown.expected_tail = None;
        assert_eq!(s9_verdict(&Cell::A, &unknown), Verdict::Inconclusive("the expected tail cannot be computed".to_owned()), "one tail with no expectation is inconclusive");
        let mut unknown_absent = good();
        unknown_absent.expected_tail = None;
        unknown_absent.tails.clear();
        assert_eq!(s9_verdict(&Cell::A, &unknown_absent), Verdict::Fail("the tail is absent".to_owned()), "no tail with no expectation still fails: the tail is absent");
        let several = S9Trial { sample_seen: false, tails: Vec::new(), expected_tail: Some(20_000), sequence_overflows: 1, ..S9Trial::default() };
        let reason = match s9_verdict(&Cell::A, &several) {
            Verdict::Fail(reason) => reason,
            other => panic!("several broken rules must fail: {other:?}"),
        };
        assert!(reason.contains("no successor sample") && reason.contains("the tail is absent") && reason.contains("a sequence-error overflow"), "one FAIL names every broken rule: {reason}");
    }

    #[test]
    fn s9_decision_counts_classes_and_verdicts() {
        let trials = vec![
            (Cell::A, Verdict::Pass("START_LEAD_SHORT 0, LATE_COMMAND 0".to_owned())),
            (Cell::A, Verdict::Fail("the tail is absent".to_owned())),
            (Cell::B, Verdict::Pass("START_LEAD_SHORT 0, LATE_COMMAND 1".to_owned())),
            (Cell::Inconclusive("x"), Verdict::Inconclusive("x".to_owned())),
            (Cell::B, Verdict::Inconclusive("the expected tail cannot be computed".to_owned())),
        ];
        let line = s9_cell_decision("s9", &Cell::A, &trials);
        assert!(line.contains("5 trials"), "the line names the trial count: {line}");
        assert!(line.contains("target A reached 2"), "the line counts the target's hits: {line}");
        assert!(line.contains("classified A 2, B 2, INCONCLUSIVE 1"), "the line counts every class: {line}");
        assert!(line.contains("verdicts pass 2, FAIL 1, inconclusive 2"), "the line counts every verdict: {line}");
        assert!(line.contains("; first FAIL: the tail is absent"), "the line names the first FAIL's reason: {line}");
        let no_fail: Vec<(Cell, Verdict)> = trials.into_iter().filter(|(_, verdict)| !matches!(verdict, Verdict::Fail(_))).collect();
        assert!(!s9_cell_decision("s9", &Cell::A, &no_fail).contains("first FAIL"), "a line without a FAIL has no first FAIL");
    }

    #[test]
    fn s9_cells_are_the_two_rates_by_retune_by_target() {
        let cells = s9_cells(20, &S9_RATE_STALLS_MS, 100);
        assert_eq!(cells.len(), 8, "two rates, both retune values, two targets");
        assert!(cells.iter().all(|(plan, trials)| *trials == 20 && plan.after_ms == 100), "every cell plans 20 trials at 100 ms: {cells:?}");
        assert!(cells.iter().all(|(plan, _)| plan.restart_after_ns == 50_000_000), "every cell judges the booking against UHD's 50 ms restart delay: {cells:?}");
        let shape: Vec<(u64, u64, bool, S9Target)> = cells.iter().map(|(plan, _)| (plan.rate_hz, plan.stall_ms, plan.retune, plan.target)).collect();
        assert_eq!(
            shape,
            [
                (10_000_000, 2_000, false, S9Target::A),
                (10_000_000, 2_000, false, S9Target::B),
                (10_000_000, 2_000, true, S9Target::A),
                (10_000_000, 2_000, true, S9Target::B),
                (100_000_000, 500, false, S9Target::A),
                (100_000_000, 500, false, S9Target::B),
                (100_000_000, 500, true, S9Target::A),
                (100_000_000, 500, true, S9Target::B),
            ],
            "each (rate, stall) runs both retune values, targeting A then B"
        );
    }

    #[test]
    fn s9_window_and_span_come_from_the_block_the_rate_and_the_restart_delay() {
        // span = block * 1e9 / rate, rounded up, plus the 3 ms delivery allowance (rx.rs `quiet_span`).
        // 2 000 * 1e9 / 10e6 = 200 000; + 3 000 000 = 3 200 000.
        assert_eq!(s9_span_ns(2_000, 10_000_000), 3_200_000, "2 000 samples at 10 MS/s");
        // 2 000 * 1e9 / 100e6 = 20 000; + 3 000 000 = 3 020 000.
        assert_eq!(s9_span_ns(2_000, 100_000_000), 3_020_000, "2 000 samples at 100 MS/s");
        // 65 536 * 1e9 / 1e6 = 65 536 000; + 3 000 000 = 68 536 000.
        assert_eq!(s9_span_ns(65_536, 1_000_000), 68_536_000, "65 536 samples at 1 MS/s");
        // the window = restart - span - the 2 ms margin.
        // 50 000 000 - 3 020 000 - 2 000 000 = 44 980 000.
        assert_eq!(s9_booking_window_ns(50_000_000, 3_020_000), 44_980_000, "UHD's 50 ms restart less a 100 MS/s span");
        // 140 000 000 - 5 000 000 - 2 000 000 = 133 000 000.
        assert_eq!(s9_booking_window_ns(140_000_000, 5_000_000), 133_000_000, "the fake's 140 ms restart less a 5 ms span");
        // 5 000 000 - 3 000 000 - 2 000 000 = 0: the margin exactly uses up the slack.
        assert_eq!(s9_booking_window_ns(5_000_000, 3_000_000), 0, "a restart one span plus the margin away leaves no window");
        // 5 000 001 - 3 000 000 - 2 000 000 = 1: one nanosecond of slack is the window.
        assert_eq!(s9_booking_window_ns(5_000_001, 3_000_000), 1, "a nanosecond past the margin is the window");
        // 3 500 000 - 3 000 000 = 500 000, less the 2 000 000 margin saturates at 0.
        assert_eq!(s9_booking_window_ns(3_500_000, 3_000_000), 0, "a restart inside one span and the margin leaves no window");
        assert_eq!(UHD_OVERRUN_RESTART_DELAY_NS, 50_000_000, "UHD 4.10's OVERRUN_RESTART_DELAY is 0.05 s");
        assert_eq!(
            S9_RESTART_MARGIN_NS, 2_000_000,
            "the margin covers uhd-rx's 1 ms minimum receive wait, which UHD truncates to whole milliseconds, and 1 ms for the rx_stop pair and get_time_now()"
        );
    }

    #[test]
    fn s9_result_stamps_are_the_result_rows_before_and_after() {
        assert_eq!(s9_result_stamps(&trace(), STALL_NS), Some((1_100, 1_110)), "the base trace's result row is 1 100..1 110");
        // The no-gap trace of case 9 has no resume, so it has no result after one.
        let no_gap = vec![samples(0, 10), samples(20, 30), overflow(40, 50, false)];
        assert_eq!(s9_result_stamps(&no_gap, STALL_NS), None, "a trace without a resume has no result row to place");
    }

    #[test]
    fn s9_a_tail_on_the_successor_s_clock_is_a_fault() {
        // The old clock: domain 1, origin 0, end 2 000 root ticks at n = 2 → end index (2 000 + 1) / 2 = 1 000. First undelivered 300,
        // so the old segment owes 1 000 - 300 = 700. The booked Stop is at 2 000, the applied cut is 2 000: the cut is verified.
        let old = SegClock { domain: ClockDomainId::local(1), origin: 0, end: Some(2_000) };
        // The tail event is on the successor's clock (domain 2, tick 0): the old code took the event's own domain and computed B's loss.
        assert_eq!(
            s9_tail_judgement(&old, 2, Some(300), Some(2_000), Some(2_000), at(2, 0)),
            TailJudgement { expected: Some(700), fault: Some("the tail is not on the old stream's clock"), cut_verified: true },
            "a tail on the successor's clock is a fault"
        );
        // On the old domain but one sample past the first undelivered sample.
        assert_eq!(
            s9_tail_judgement(&old, 2, Some(300), Some(2_000), Some(2_000), at(1, 301)),
            TailJudgement { expected: Some(700), fault: Some("the tail is not at the old stream's first undelivered sample"), cut_verified: true },
            "a tail one sample past the first undelivered sample is a fault"
        );
        // On the old domain at the first undelivered sample: 1 000 - 300 = 700.
        assert_eq!(
            s9_tail_judgement(&old, 2, Some(300), Some(2_000), Some(2_000), at(1, 300)),
            TailJudgement { expected: Some(700), fault: None, cut_verified: true },
            "the old tail at its first undelivered sample is sound"
        );
        // An end of 1 998 does not match the applied cut 2 000: its index is (1 998 + 1) / 2 = 999, so 999 - 300 = 699.
        let short = SegClock { end: Some(1_998), ..old.clone() };
        assert_eq!(
            s9_tail_judgement(&short, 2, Some(300), Some(2_000), Some(2_000), at(1, 300)),
            TailJudgement { expected: Some(699), fault: Some("the old clock's end is not the applied stop's cut"), cut_verified: false },
            "a shortened old clock is not the cut the Module applied"
        );
        // No first undelivered sample: the expected loss is unknown and the tick check is skipped; the domain matches and the end is the cut.
        assert_eq!(
            s9_tail_judgement(&old, 2, None, Some(2_000), Some(2_000), at(1, 300)),
            TailJudgement { expected: None, fault: None, cut_verified: true },
            "an unknown sample leaves the loss unknown and no fault"
        );
    }

    #[test]
    fn s9_tail_judgement_checks_the_cut_against_the_booked_stop() {
        // End 2 000 root ticks at n = 2: index (2 000 + 1) / 2 = 1 000; first undelivered 300, so the old segment owes 700.
        let old = SegClock { domain: ClockDomainId::local(1), origin: 0, end: Some(2_000) };
        // The booked Stop's instant 2 001 is after the end 2 000: the cut came before the Stop arrived, so the Module did not cut where booked.
        assert_eq!(
            s9_tail_judgement(&old, 2, Some(300), Some(2_000), Some(2_001), at(1, 300)),
            TailJudgement { expected: Some(700), fault: Some("the old clock's end is before the booked Stop's instant"), cut_verified: false },
            "an end before the booked instant is a fault and leaves the cut unverified"
        );
        // Equal is allowed: a lattice instant can coincide with the booked Stop's.
        assert_eq!(
            s9_tail_judgement(&old, 2, Some(300), Some(2_000), Some(2_000), at(1, 300)),
            TailJudgement { expected: Some(700), fault: None, cut_verified: true },
            "an end at the booked instant is verified"
        );
        // Above is verified too: the Provider books the Stop at its own `now`, no earlier, and a late cut is bounded only by the cell-A window.
        assert_eq!(
            s9_tail_judgement(&old, 2, Some(300), Some(2_000), Some(1_999), at(1, 300)),
            TailJudgement { expected: Some(700), fault: None, cut_verified: true },
            "an end after the booked instant is verified"
        );
        // A missing applied cut or booked instant leaves the cut unverified without a fault.
        assert_eq!(s9_tail_judgement(&old, 2, Some(300), None, Some(2_000), at(1, 300)), TailJudgement { expected: Some(700), fault: None, cut_verified: false }, "no applied cut is unverified");
        assert_eq!(s9_tail_judgement(&old, 2, Some(300), Some(2_000), None, at(1, 300)), TailJudgement { expected: Some(700), fault: None, cut_verified: false }, "no booked instant is unverified");
        // An applied cut different from the end keeps its own fault.
        assert_eq!(
            s9_tail_judgement(&old, 2, Some(300), Some(1_998), Some(2_000), at(1, 300)),
            TailJudgement { expected: Some(700), fault: Some("the old clock's end is not the applied stop's cut"), cut_verified: false },
            "an end off the applied cut is a fault"
        );
    }

    #[test]
    fn s9_first_undelivered_reads_the_last_samples_call_before_the_overflow_result() {
        // Rows: 0..10 delivers 1 000 samples from index 0 (0..1 000); 10..20 delivers 500 from root tick 2 000 at n = 2 — index
        // (2 000 + 1) / 2 = 1 000, so it ends at 1 500. The result row is the first row after the 1 000 ns stall (1 020 - 20 = 1 000).
        let rows = vec![samples_of(0, 10, 0, 1_000), samples_of(10, 20, 2_000, 500), overflow(1_020, 1_030, false)];
        assert_eq!(s9_first_undelivered(&rows, STALL_NS, 0, 2), Some(1_500), "the last samples call before the result ends at 1 500");
        // A post-resume backlog row before the result: index (3 000 + 1) / 2 = 1 500, plus 100 = 1 600.
        let backlog = vec![samples_of(0, 10, 0, 1_000), samples_of(10, 20, 2_000, 500), samples_of(1_020, 1_030, 3_000, 100), overflow(1_100, 1_110, false)];
        assert_eq!(s9_first_undelivered(&backlog, STALL_NS, 0, 2), Some(1_600), "a backlog row before the result is the last delivery");
        // No samples row before the result: the sequence overflow and the result are the only rows.
        let none = vec![overflow(0, 10, true), overflow(1_020, 1_030, false)];
        assert_eq!(s9_first_undelivered(&none, STALL_NS, 0, 2), None, "no samples call before the result has no first undelivered sample");
        // A samples row after the result is ignored: the last delivery before the result is still 1 500.
        let after = vec![samples_of(0, 10, 0, 1_000), samples_of(10, 20, 2_000, 500), overflow(1_020, 1_030, false), samples_of(1_300, 1_310, 4_000, 100)];
        assert_eq!(s9_first_undelivered(&after, STALL_NS, 0, 2), Some(1_500), "a samples row after the result is not the old stream's last delivery");
    }

    #[test]
    fn s9_a_tail_fault_is_a_verdict_failure() {
        let trial = S9Trial { tail_fault: Some("the tail is not on the old stream's clock"), ..good() };
        assert_eq!(s9_verdict(&Cell::A, &trial), Verdict::Fail("the tail is not on the old stream's clock".to_owned()), "a tail fault fails the verdict in so many words");
    }

    #[test]
    fn s9_a_trial_with_an_unverified_cut_is_inconclusive_not_a_pass() {
        // The cut was not checked against both the applied stop and the booked Stop: a shortened capture cannot pass on the rest.
        assert_eq!(
            s9_verdict(&Cell::A, &S9Trial { cut_verified: false, ..good() }),
            Verdict::Inconclusive("the old clock's cut cannot be verified against the applied stop and the booked Stop".to_owned()),
            "an unverified cut is open"
        );
        assert_eq!(s9_verdict(&Cell::A, &good()), Verdict::Pass("START_LEAD_SHORT 0, LATE_COMMAND 0".to_owned()), "a verified cut passes");
    }

    #[test]
    fn s9_retune_target_crosses_the_one_gigahertz_edge_from_the_default() {
        let edge_hz = 1.0e9;
        assert!(S9_RETUNE_HZ < edge_hz, "the retune target 999.5 MHz is below the 1 GHz tuning-path edge: {S9_RETUNE_HZ}");
        for board in [ezsdr_radio::x3xx::Board::Ubx, ezsdr_radio::x3xx::Board::Obx] {
            assert!(board.default_frequency_hz() >= 1.0e9, "{board:?} starts every Session at or above 1 GHz: {}", board.default_frequency_hz());
        }
    }

    #[test]
    fn s9_initial_requests_carry_the_stream_and_the_rate_and_no_frequency() {
        let initial = s9_initial(&json!({}), 10_000_000).unwrap();
        let radio = &initial[&Ident::parse("radio").unwrap()];
        let mut keys: Vec<String> = radio.keys().map(|key| key.as_str().to_owned()).collect();
        keys.sort();
        assert_eq!(
            keys,
            ["radio.rx.c0.stream".to_owned(), "radio.rx.s0.sample_rate_hz".to_owned()],
            "the initial requests book receive channel c0 in stream 0 and the receive rate, and nothing else: {keys:?}"
        );
        assert!(
            !keys.iter().any(|key| key.contains("frequency")),
            "no frequency is booked: the Session starts at the board's 1 GHz default, and the retune's target S9_RETUNE_HZ (999.5 MHz) crosses the 1 GHz band edge: {keys:?}"
        );
    }

    #[test]
    fn s9_trial_row_prints_the_result_stamps_and_the_window() {
        let plan = S9Plan { rate_hz: 10_000_000, stall_ms: 2_000, after_ms: 100, restart_after_ns: UHD_OVERRUN_RESTART_DELAY_NS, retune: false, target: S9Target::A };
        let outcome = S9Outcome {
            cell: Cell::A,
            trial: S9Trial { sequence_events: 3, sequence_overflows: 1, ..S9Trial::default() },
            verdict: Verdict::Pass("START_LEAD_SHORT 0, LATE_COMMAND 0".to_owned()),
            pre_stamp_ns: 1_200,
            post_stamp_ns: 1_210,
            window_ns: 500,
            result_ns: Some((1_100, 1_110)),
            rows: 6,
        };
        let row = trial_row(&plan, &outcome);
        assert_eq!(row["result_before_ns"], 1_100, "the row prints the result's before stamp: {row}");
        assert_eq!(row["result_after_ns"], 1_110, "the row prints the result's after stamp: {row}");
        assert_eq!(row["window_ns"], 500, "the row prints the booking window it was judged against: {row}");
        assert_eq!(row["sequence_events"], 3, "the row prints every sequence event the Run delivered: {row}");
        assert_eq!(row["sequence_overflows"], 1, "the row prints the successor's first-block sequence count: {row}");
        let none = S9Outcome { result_ns: None, ..outcome };
        let row = trial_row(&plan, &none);
        assert!(row["result_before_ns"].is_null() && row["result_after_ns"].is_null(), "a trial without a result row prints null stamps: {row}");
        assert_eq!(row["window_ns"], 500, "the window is printed with or without a result: {row}");
    }
}
