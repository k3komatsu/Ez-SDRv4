//! bt2's C7 cells (`plan/phase8/bench.md`, "C7: bt2"): one mode x one rate x one margin is a cell; a trial is one Session of `c7::trial` whose verdict
//! `c7_verdict` decides. The runner prints the decision lines: one `C7 trial` line per trial with the raw numbers the verdict does not read, one `C7 start`
//! line per accepted Start after the first and one `C7 retry` line per retry, then the cell's summary and its `VERDICT c7` line. A trial is CLEAN, a datum
//! (RETRIED, UNSETTLED) or a reason it cannot be read (INCONCLUSIVE) or a named failure; INCONCLUSIVE fails a cell as FAIL does, while RETRIED and
//! UNSETTLED only change the cell's PASS wording. The verdict is `c7_verdict`'s and is not changed here.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration as Wall;

use ezsdr_radio_uhd::Device;
use serde_json::Value as Json;

use super::TempDir;
use super::c7::{self, Cell, Mode, Resume, Stall, Terms, Trial};
use super::c7_rf::{self, Control, PartReading};
use super::c7_storm;
use super::c7_verdict::{self, Class, Evidence, Fail, Readings};
use super::probe_device::ProbeDevice;
use super::tone::angle_difference_deg;

/// The rates of the bench grid.
pub const RATES: [f64; 2] = [1e6, 10e6];
/// The margins of the bench grid and the trials each cell runs, ns.
pub const MARGINS: [(i64, usize); 3] = [(10_000_000, 10), (1_000_000, 20), (200_000, 20)];

/// Root ticks a microsecond (the master clock rate is 200 MHz). The lines print µs, the Session ticks.
const TICKS_PER_US: i64 = super::MCR / 1_000_000;

/// One accepted Start after the first drop, with the reports either side of it: the drain's split at the start.
#[derive(Clone, Debug, PartialEq)]
pub struct StartDatum {
    /// The start's index in `Evidence::starts` (the first is 0 and not printed).
    pub j: usize,
    pub a: i64,
    pub played: Option<bool>,
    pub d_before: Option<i64>,
    pub d_after: Option<i64>,
    pub a_minus_dbefore_us: Option<i64>,
    pub dafter_minus_a_us: Option<i64>,
}

/// One row's expected R as data: the actual resume tick, the interval's allowed lattice points, and whether the check decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RRow {
    /// The row's actual resume point, in root ticks.
    pub r_act: i64,
    /// The lowest and highest allowed lattice points, in root ticks.
    pub lo: i64,
    pub hi: i64,
    /// How many allowed lattice points the interval holds.
    pub allowed: i64,
    pub uncertainty_us: i64,
    pub unresolved: bool,
}

/// One resume row's expectation, as the lines print it.
#[derive(Clone, Debug, PartialEq)]
pub struct RowExpectation {
    pub r_minus_busy_us: i64,
    pub m_bound: bool,
    pub r_row: RRow,
}

/// One resume row's data: its close's return, and its expectation when its history is not ambiguous.
#[derive(Clone, Debug, PartialEq)]
pub struct RowDatum {
    pub close_returned: Option<i64>,
    pub expectation: Option<RowExpectation>,
}

/// All raw numbers of one trial that the lines print; the verdict reads none of them.
#[derive(Clone, Debug, PartialEq)]
pub struct Datum {
    pub rows: usize,
    pub retries: usize,
    /// The PN period in microseconds.
    pub period_us: i64,
    /// The held send's accepted samples, and whether it was a full send of the Module.
    pub n1: i64,
    pub n1_full: bool,
    /// The first resume row's rule terms.
    pub m_bound: Option<bool>,
    pub m_informative: Option<bool>,
    pub allowed: Option<i64>,
    pub gap_us: Option<i64>,
    /// Every resume row's numbers (`None`: the row's history is ambiguous).
    pub row_data: Vec<RowDatum>,
    /// The rows whose expectation was ambiguous.
    pub ambiguous: usize,
    /// The first `TimeError` at or after the held start and before the first resume point, against the held call's return, and its endpoints
    /// (missing when none qualifies).
    pub delta_first_us: Option<i64>,
    /// The first `TimeError` at or after the held start and before the first resume point (missing when none qualifies).
    pub t1: Option<i64>,
    pub r1: Option<i64>,
    pub return_minus_at_us: Option<i64>,
    pub held_ns: Option<u64>,
    /// The observed lattice origin minus the booked `start`, in samples (`Common::origin`): 0 when the booking was not moved.
    pub start_moved_samples: i64,
    pub starts: Vec<StartDatum>,
    pub reports_after_a_played_start: usize,
    pub report_gap_us: Option<i64>,
    pub phase_vs_control_deg: Option<f64>,
    pub phase_vs_prestall_deg: Option<f64>,
    pub g_c1_over_t: Option<f64>,
}

impl Datum {
    /// Row 1's `R - busy`, the effective margin of the first incident (`None` when there is no row or row 1's history is ambiguous).
    pub fn first_r_minus_busy_us(&self) -> Option<i64> {
        self.row_data.first().and_then(|row| row.expectation.as_ref()).map(|row| row.r_minus_busy_us)
    }
}

#[cfg(test)]
impl Datum {
    /// A clean trial's numbers, for the lines' pure tests.
    pub fn example() -> Datum {
        Datum {
            rows: 1,
            retries: 0,
            period_us: c7::FAKE_PERIOD_US,
            n1: 2_500,
            n1_full: true,
            m_bound: Some(true),
            m_informative: Some(true),
            allowed: Some(1),
            gap_us: Some(1_200),
            row_data: vec![RowDatum {
                close_returned: Some(33_000_000),
                expectation: Some(RowExpectation {
                    r_minus_busy_us: 9_000,
                    m_bound: true,
                    r_row: RRow { r_act: 34_000_000, lo: 34_000_000, hi: 34_000_000, allowed: 1, uncertainty_us: 1, unresolved: false },
                }),
            }],
            ambiguous: 0,
            delta_first_us: Some(-40),
            t1: Some(8_000),
            r1: Some(16_000),
            return_minus_at_us: Some(40_000),
            held_ns: Some(40_000_000),
            start_moved_samples: 0,
            starts: vec![StartDatum {
                j: 1,
                a: 34_000_000,
                played: Some(true),
                d_before: Some(26_000_000),
                d_after: Some(34_200_000),
                a_minus_dbefore_us: Some(40_000),
                dafter_minus_a_us: Some(1_000),
            }],
            reports_after_a_played_start: 1,
            report_gap_us: Some(20_000),
            phase_vs_control_deg: Some(2.0),
            phase_vs_prestall_deg: None,
            g_c1_over_t: Some(12.5),
        }
    }
}

/// One finished trial: its class and the raw numbers of `datum`.
#[derive(Clone, Debug)]
pub struct TrialRecord {
    pub class: Class,
    pub datum: Datum,
}

/// The device's drop reports, of any of the three codes, with a tick.
fn report_tick(code: &str, tick: Option<i64>) -> Option<i64> {
    c7_verdict::is_drop(code).then_some(tick).flatten()
}

/// `D_before` and `D_after` of a part whose accepted start is at `a`: the largest report tick strictly below `a`, the smallest at or after it.
fn drain(reports: &[(String, Option<i64>)], a: i64) -> (Option<i64>, Option<i64>) {
    let mut before: Option<i64> = None;
    let mut after: Option<i64> = None;
    for (code, tick) in reports {
        let Some(tick) = report_tick(code, *tick) else { continue };
        if tick < a {
            before = Some(before.map_or(tick, |largest| largest.max(tick)));
        } else {
            after = Some(after.map_or(tick, |smallest| smallest.min(tick)));
        }
    }
    (before, after)
}

/// The median of `values` (sorted, the upper middle for an even count).
fn median(values: &[i64]) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2])
}

/// `min/median/max` of `values`, `-` when there are none.
fn stats(values: &[i64]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    let min = *values.iter().min().unwrap();
    let max = *values.iter().max().unwrap();
    format!("{min}/{}/{max}", median(values).unwrap())
}

fn number(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| value.to_string())
}

fn flag(value: Option<bool>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| value.to_string())
}

fn real(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| format!("{value:.3}"))
}

/// `Mode` as the lines name it.
fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::LateStart => "late_start",
        Mode::Starved => "starved",
    }
}

/// The rate as the lines name it (`1MS/s`, `10MS/s`).
fn rate_name(rate: f64) -> String {
    format!("{}MS/s", rate / 1e6)
}

/// The margin in ms as the lines name it (`10`, `1`, `0.2`).
fn margin_name(margin_ns: i64) -> String {
    if margin_ns % 1_000_000 == 0 { (margin_ns / 1_000_000).to_string() } else { format!("{}.{}", margin_ns / 1_000_000, (margin_ns % 1_000_000) / 100_000) }
}

/// The prefix every line of a trial shares: its board, mode, rate, PN period, margin and index.
fn prefix(board: &str, cell: &Cell, index: usize) -> String {
    format!("{board} {} {} pn={}us m={} #{index}", mode_name(cell.mode), rate_name(cell.rate), cell.period_us, margin_name(cell.margin()))
}

/// The last heard part of the trial: the final accepted Start whose first period is heard.
fn last_heard(readings: &Readings) -> Option<&PartReading> {
    readings.parts.iter().rev().find_map(|part| part.as_ref().ok().filter(|part| part.w0_heard))
}

/// The pre-stall periods' phase of the repeat: the circular mean of the gain-phase windows at `s0.at + d + k * period`, k = 1..=19.
fn prestall_phase(e: &Evidence, control: &Control, s0: &c7_verdict::StartRow) -> Option<f64> {
    let span = e.n * e.per;
    let (mut re, mut im) = (0.0, 0.0);
    for k in 1..=19 {
        let (_, phase) = e.capture.gain_phase(&e.wave, s0.at + control.d + k as i64 * span)?;
        let radians = phase.to_radians();
        re += radians.cos();
        im += radians.sin();
    }
    Some(im.atan2(re).to_degrees())
}

/// The first report whose code is `TimeError` with a tick at or after `at` and, when `before` is `Some(limit)`, before that limit.
fn first_time_error(reports: &[(String, Option<i64>)], at: i64, before: Option<i64>) -> Option<i64> {
    reports.iter().find_map(|(code, tick)| {
        let tick = tick.filter(|tick| code == "TimeError" && *tick >= at && before.is_none_or(|limit| *tick < limit))?;
        Some(tick)
    })
}

/// All raw numbers of one finished trial: read from the Session's record, the probe's rows and the capture, once.
pub fn datum(trial: &Trial, e: &Evidence, readings: &Readings) -> Datum {
    let margin_ns = trial.cell.margin();
    let mut row_data = Vec::new();
    let mut first_terms: Option<Terms> = None;
    for (index, row) in e.rows.iter().enumerate() {
        let (close_returned, expectation) = match trial.sent_for_row(index, row) {
            Ok(sends) => {
                let terms = trial.expected_from(&sends, row, margin_ns);
                if index == 0 {
                    first_terms = Some(terms);
                }
                let expectation = RowExpectation {
                    r_minus_busy_us: (e.tx_origin + row.resumes_at * e.per - terms.busy) / TICKS_PER_US,
                    m_bound: trial.m_bound(row, &terms),
                    r_row: RRow {
                        r_act: e.tx_origin + row.resumes_at * e.per,
                        lo: e.tx_origin + terms.lo * e.per,
                        hi: e.tx_origin + terms.hi * e.per,
                        allowed: terms.allowed(trial.period),
                        uncertainty_us: terms.uncertainty / TICKS_PER_US,
                        unresolved: terms.unresolved,
                    },
                };
                (sends.last().map(|sent| sent.returned), Some(expectation))
            }
            Err(_) => (None, None),
        };
        row_data.push(RowDatum { close_returned, expectation });
    }
    let ambiguous = row_data.iter().filter(|row| row.expectation.is_none()).count();
    let first_row: Option<&Resume> = e.rows.first();
    let (m_bound, m_informative, allowed, gap_us) = match (first_row, first_terms) {
        (Some(row), Some(terms)) => {
            (Some(trial.m_bound(row, &terms)), Some(trial.m_informative(row, &terms)), Some(terms.allowed(trial.period)), Some((row.resumes_at - row.from) * e.per / TICKS_PER_US))
        }
        _ => (None, None, None, None),
    };
    let held = e.holds.first();
    let n1 = held.map_or(0, |held| held.accepted);
    let (delta_first_us, t1, r1) = match (e.mode, held) {
        (Mode::LateStart, Some(held)) => {
            let r1 = held.returned;
            let before = e.rows.first().map(|row| e.tx_origin + row.resumes_at * e.per);
            let t1 = held.at.and_then(|at| first_time_error(&e.reports, at, before));
            (t1.map(|t1| delta_first_us(t1, r1)), t1, Some(r1))
        }
        _ => (None, None, None),
    };
    let return_minus_at_us = match (e.mode, held) {
        (Mode::LateStart, Some(held)) => held.at.map(|at| (held.returned - at) / TICKS_PER_US),
        _ => None,
    };
    let control = readings.control.as_ref().ok();
    let mut starts = Vec::new();
    let mut reports_after_a_played_start = 0;
    for (j, start) in e.starts.iter().enumerate().skip(1) {
        let (d_before, d_after) = drain(&e.reports, start.at);
        let played = readings.parts.get(j).and_then(|part| part.as_ref().ok()).map(|part| part.w0_heard);
        if played == Some(true) && d_after.is_some() {
            reports_after_a_played_start += 1;
        }
        starts.push(StartDatum {
            j,
            a: start.at,
            played,
            d_before,
            d_after,
            a_minus_dbefore_us: d_before.map(|before| (start.at - before) / TICKS_PER_US),
            dafter_minus_a_us: d_after.map(|after| (after - start.at) / TICKS_PER_US),
        });
    }
    let report_gap_us = e.starts.get(1).and_then(|first| {
        let ticks: Vec<i64> = e.reports.iter().filter_map(|(code, tick)| report_tick(code, *tick)).filter(|tick| *tick < first.at).collect();
        if ticks.len() < 6 {
            return None;
        }
        let last = &ticks[ticks.len() - 6..];
        let gaps: Vec<i64> = last.windows(2).map(|pair| (pair[1] - pair[0]) / TICKS_PER_US).collect();
        median(&gaps)
    });
    let heard = last_heard(readings);
    let phase_vs_control_deg = match (control, &heard) {
        (Some(control), Some(part)) => Some(angle_difference_deg(part.w0_phase, control.phase_c)),
        _ => None,
    };
    let phase_vs_prestall_deg = match (e.mode, control, &heard, e.starts.first()) {
        (Mode::Starved, Some(control), Some(part), Some(s0)) => prestall_phase(e, control, s0).map(|phase| angle_difference_deg(part.w0_phase, phase)),
        _ => None,
    };
    Datum {
        rows: e.rows.len(),
        retries: e.rows.len().saturating_sub(1),
        period_us: trial.period_us,
        n1,
        n1_full: n1 == e.cap_samples,
        m_bound,
        m_informative,
        allowed,
        gap_us,
        row_data,
        ambiguous,
        delta_first_us,
        t1,
        r1,
        return_minus_at_us,
        held_ns: held.map(|held| held.held_ns),
        start_moved_samples: trial.origin() - trial.start,
        starts,
        reports_after_a_played_start,
        report_gap_us,
        phase_vs_control_deg,
        phase_vs_prestall_deg,
        g_c1_over_t: control.map(|control| control.g_c1 / control.t),
    }
}

/// The trial's lines: its `C7 trial` line, a `C7 r_unresolved` line for each unresolved row, a `C7 start` line for every accepted Start after the first,
/// and a `C7 retry` line for every resume row after the first.
pub fn trial_lines(board: &str, cell: &Cell, index: usize, rec: &TrialRecord, e: &Evidence) -> Vec<String> {
    let prefix = prefix(board, cell, index);
    let datum = &rec.datum;
    let bracket_us = e.bracket / TICKS_PER_US;
    let r_unresolved = datum.row_data.iter().filter_map(|row| row.expectation.as_ref()).filter(|row| row.r_row.unresolved).count();
    let mut lines = vec![format!(
        "C7 trial {prefix}: {} | rows={} retries={} n1={} n1_full={} m_bound={} m_informative={} allowed={} r_unresolved={} gap_us={} r_minus_busy_first_us={} delta_first_us={} (t1 = {}, r1 = {} +- {}) return_minus_at_us={} +- {} held_ns={} start_moved_samples={} g_c1_over_t={} phase_vs_control_deg={} phase_vs_prestall_deg={}",
        c7_verdict::describe(&rec.class),
        datum.rows,
        datum.retries,
        datum.n1,
        datum.n1_full,
        flag(datum.m_bound),
        flag(datum.m_informative),
        number(datum.allowed),
        r_unresolved,
        number(datum.gap_us),
        number(datum.first_r_minus_busy_us()),
        number(datum.delta_first_us),
        number(datum.t1),
        number(datum.r1),
        if datum.r1.is_some() { e.bracket.to_string() } else { "-".to_owned() },
        number(datum.return_minus_at_us),
        if datum.return_minus_at_us.is_some() { bracket_us.to_string() } else { "-".to_owned() },
        number(datum.held_ns.map(|held| held as i64)),
        datum.start_moved_samples,
        real(datum.g_c1_over_t),
        real(datum.phase_vs_control_deg),
        real(datum.phase_vs_prestall_deg),
    )];
    for (index, row) in datum.row_data.iter().enumerate() {
        if let Some(row) = row.expectation.as_ref().map(|expectation| &expectation.r_row) {
            if row.unresolved {
                lines.push(format!(
                    "C7 r_unresolved {prefix} row={}: R_act={} interval=[{}, {}] allowed={} uncertainty_us={} period_us={} (the interval holds several lattice points; the R check accepts every one of them and none outside)",
                    index + 1,
                    row.r_act,
                    row.lo,
                    row.hi,
                    row.allowed,
                    row.uncertainty_us,
                    datum.period_us,
                ));
            }
        }
    }
    let print_start = |start: &StartDatum| {
        format!(
            "C7 start {prefix} j={}: A={} returned={} played={} D_before={} D_after={} A_minus_Dbefore_us={} Dafter_minus_A_us={} (raw distances: reports may stop before the drain ends, F7)",
            start.j,
            start.a,
            e.starts.get(start.j).map_or_else(|| "-".to_owned(), |row| row.returned.to_string()),
            flag(start.played),
            start.d_before.map_or_else(|| "none".to_owned(), |tick| tick.to_string()),
            start.d_after.map_or_else(|| "none".to_owned(), |tick| tick.to_string()),
            start.a_minus_dbefore_us.map_or_else(|| "none".to_owned(), |us| us.to_string()),
            start.dafter_minus_a_us.map_or_else(|| "none".to_owned(), |us| us.to_string()),
        )
    };
    lines.extend(datum.starts.iter().map(print_start));
    let retries: Vec<(usize, &Resume)> = e.rows.iter().enumerate().map(|(index, row)| (index + 1, row)).filter(|(row, _)| *row >= 2).collect();
    let print_retry = |(row, resume): &(usize, &Resume)| {
        let dropped = datum.starts.get(row - 2);
        let dropped_start = e.starts.get(row - 1);
        let dropped_held_ns = dropped_start.and_then(|start| e.holds.iter().find(|held| held.at == Some(start.at))).map_or(0, |held| held.held_ns);
        format!(
            "C7 retry {prefix} row={row}: cause={} from={} resumes_at={} at={} close_returned={} dropped_start_A={} dropped_start_returned={} dropped_start_held_ns={} D_before={} D_after={} A_minus_Dbefore_us={} Dafter_minus_A_us={} r_minus_busy_us={} m_bound={}",
            resume.cause,
            resume.from,
            resume.resumes_at,
            resume.at,
            number(datum.row_data.get(row - 1).and_then(|row| row.close_returned)),
            dropped.map_or_else(|| "-".to_owned(), |start| start.a.to_string()),
            dropped_start.map_or_else(|| "-".to_owned(), |start| start.returned.to_string()),
            dropped_held_ns,
            dropped.and_then(|start| start.d_before).map_or_else(|| "none".to_owned(), |tick| tick.to_string()),
            dropped.and_then(|start| start.d_after).map_or_else(|| "none".to_owned(), |tick| tick.to_string()),
            dropped.and_then(|start| start.a_minus_dbefore_us).map_or_else(|| "none".to_owned(), |us| us.to_string()),
            dropped.and_then(|start| start.dafter_minus_a_us).map_or_else(|| "none".to_owned(), |us| us.to_string()),
            number(datum.row_data.get(row - 1).and_then(|row| row.expectation.as_ref()).map(|row| row.r_minus_busy_us)),
            flag(datum.row_data.get(row - 1).and_then(|row| row.expectation.as_ref()).map(|row| row.m_bound)),
        )
    };
    lines.extend(retries.iter().map(print_retry));
    lines
}

/// The signed distance, in microseconds, from the held send's return `r1` to the first report `t1` (negative when the report precedes the return).
fn delta_first_us(t1: i64, r1: i64) -> i64 {
    (t1 - r1) / TICKS_PER_US
}

/// One trial's retry count added to the `by r` histogram.
fn count_by_retries(by_retries: &mut Vec<(usize, usize)>, retries: usize) {
    match by_retries.iter_mut().find(|(by, _)| *by == retries) {
        Some((_, seen)) => *seen += 1,
        None => by_retries.push((retries, 1)),
    }
}

/// The counts of the cell's classes: clean, retried trials, the retries they total, the trials by retry count, unsettled, inconclusive, fail.
#[allow(clippy::type_complexity)]
fn cell_counts(records: &[TrialRecord]) -> (usize, usize, usize, Vec<(usize, usize)>, usize, usize, usize) {
    let (mut clean, mut retried, mut retries, mut unsettled, mut inconclusive, mut fail) = (0, 0, 0, 0, 0, 0);
    let mut by_retries: Vec<(usize, usize)> = Vec::new();
    for record in records {
        match &record.class {
            Class::Clean => clean += 1,
            Class::Retried(_) => {
                retried += 1;
                retries += record.datum.retries;
                count_by_retries(&mut by_retries, record.datum.retries);
            }
            Class::Unsettled(_) => {
                unsettled += 1;
                retries += record.datum.retries;
                count_by_retries(&mut by_retries, record.datum.retries);
            }
            Class::Inconclusive(_) => inconclusive += 1,
            Class::Fail(_, _) => fail += 1,
        }
    }
    by_retries.sort_unstable();
    (clean, retried, retries, by_retries, unsettled, inconclusive, fail)
}

/// `name: count` entries in the order they first appear.
fn counted(entries: impl Iterator<Item = String>) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for entry in entries {
        match counts.iter_mut().find(|(name, _)| *name == entry) {
            Some((_, count)) => *count += 1,
            None => counts.push((entry, 1)),
        }
    }
    counts
}

fn counted_text(counts: &[(String, usize)]) -> String {
    counts.iter().map(|(name, count)| format!("{name}: {count}")).collect::<Vec<_>>().join(", ")
}

/// The cell's lines: its `C7 cell` summary and its `VERDICT c7` line.
pub fn cell_lines(board: &str, mode: Mode, rate: f64, period_us: i64, margin_ns: i64, records: &[TrialRecord]) -> Vec<String> {
    let n = records.len();
    let (clean, retried, retries, by_retries, unsettled, inconclusive, fail) = cell_counts(records);
    let reasons = counted(records.iter().flat_map(|record| match &record.class {
        Class::Inconclusive(reasons) => reasons.clone(),
        _ => Vec::new(),
    }));
    let failures = counted(records.iter().filter_map(|record| match &record.class {
        Class::Fail(fail, _) => Some(fail.name().to_owned()),
        _ => None,
    }));
    let n1_full = records.iter().filter(|record| record.datum.n1_full).count();
    let m_bound = records.iter().filter(|record| record.datum.m_bound == Some(true)).count();
    let m_informative = records.iter().filter(|record| record.datum.m_informative == Some(true)).count();
    let allowed: Vec<i64> = records.iter().filter_map(|record| record.datum.allowed).collect();
    let r_unresolved: usize = records.iter().map(|record| record.datum.row_data.iter().filter_map(|row| row.expectation.as_ref()).filter(|row| row.r_row.unresolved).count()).sum();
    let r_unresolved_trials = records.iter().filter(|record| record.datum.row_data.iter().filter_map(|row| row.expectation.as_ref()).any(|row| row.r_row.unresolved)).count();
    let drains: Vec<i64> = records.iter().flat_map(|record| &record.datum.starts).filter(|start| start.played == Some(true)).filter_map(|start| start.a_minus_dbefore_us).collect();
    let reports_after: usize = records.iter().map(|record| record.datum.reports_after_a_played_start).sum();
    let r_minus_busy: Vec<i64> = records.iter().flat_map(|record| &record.datum.row_data).filter_map(|row| row.expectation.as_ref()).map(|row| row.r_minus_busy_us).collect();
    let r_minus_busy_first: Vec<i64> = records.iter().filter_map(|record| record.datum.first_r_minus_busy_us()).collect();
    let gaps: Vec<i64> = records.iter().filter_map(|record| record.datum.gap_us).collect();
    let report_gaps: Vec<i64> = records.iter().filter_map(|record| record.datum.report_gap_us).collect();
    let deltas: Vec<i64> = records.iter().filter_map(|record| record.datum.delta_first_us).collect();
    let phases: Vec<f64> = records.iter().filter_map(|record| record.datum.phase_vs_control_deg).collect();
    let phase_spread = super::tone::circular_spread_deg(&phases).map_or_else(|| "-".to_owned(), |spread| format!("{spread:.1}"));
    let by_retries = by_retries.iter().map(|(count, seen)| format!("{count}:{seen}")).collect::<Vec<_>>().join(" ");
    let head = format!("{board} {} {} pn={period_us}us m={}", mode_name(mode), rate_name(rate), margin_name(margin_ns));
    let mut lines = vec![format!(
        "C7 cell {head}: n={n} clean={clean} retried={retried} (retries total {retries}; by r: {by_retries}) unsettled={unsettled} inconclusive={inconclusive} ({}) fail={fail} ({}); n1_full={n1_full}/{n}; m_bound={m_bound}/{n}; m_informative={m_informative}/{n}; allowed min/median/max={}; r_unresolved_rows={r_unresolved} in {r_unresolved_trials}/{n} trials; A_minus_Dbefore_us min/median/max over played starts={} (raw: reports may stop before the drain ends, F7); reports_after_a_played_start={reports_after}; r_minus_busy_us min/median/max={}; r_minus_busy_first_us min/median/max={}; gap_us min/median/max={}; report_gap_us median={}; delta_first_us min/median/max={} (signed; no class uses it); phase_spread_deg={phase_spread}",
        counted_text(&reasons),
        counted_text(&failures),
        stats(&allowed),
        stats(&drains),
        stats(&r_minus_busy),
        stats(&r_minus_busy_first),
        stats(&gaps),
        number(median(&report_gaps)),
        stats(&deltas),
    )];
    let verdict = if fail > 0 {
        format!("FAIL: {}", counted_text(&failures))
    } else if inconclusive > 0 {
        format!("INCONCLUSIVE: {}", counted_text(&reasons))
    } else if retried + unsettled == 0 {
        "PASS (contract)".to_owned()
    } else {
        format!("PASS (contract); retries in {retried}/{n} trials, unsettled in {unsettled}/{n} -- read the C7 retry lines")
    };
    lines.push(format!("VERDICT c7 {head}: {verdict}"));
    lines
}

/// Runs one cell: `trials` Sessions on `device`, each in a capture directory of its own, prints every line and returns the records and the lines.
#[allow(clippy::too_many_arguments)]
pub fn run_cell(
    device: Arc<dyn Device>,
    profile_of: &dyn Fn(&TempDir) -> Json,
    board: &str,
    mode: Mode,
    rate: f64,
    period_us: i64,
    margin_ns: i64,
    trials: usize,
    fake_ramp: bool,
) -> (Vec<TrialRecord>, Vec<String>) {
    let mut records = Vec::new();
    let mut lines = Vec::new();
    for index in 0..trials {
        let dir = TempDir::new();
        let probe = Arc::new(ProbeDevice::new(device.clone()));
        let trial = c7::trial(&profile_of(&dir), probe, Cell { mode, rate, period_us, margin_ns: Some(margin_ns), stall: Stall::For(Wall::from_millis(40)), lead_us: None, retry_hold: None });
        let e = c7_verdict::evidence(&trial, fake_ramp);
        let readings = c7_verdict::read(&e);
        let class = c7_verdict::verdict_with(&e, &readings);
        let record = TrialRecord { class: class.clone(), datum: datum(&trial, &e, &readings) };
        for line in trial_lines(board, &trial.cell, index, &record, &e) {
            println!("{line}");
            lines.push(line);
        }
        records.push(record);
    }
    for line in cell_lines(board, mode, rate, period_us, margin_ns, &records) {
        println!("{line}");
        lines.push(line);
    }
    (records, lines)
}

/// The storm's summary line: what it counted and the class it got.
pub fn storm_line(board: &str, e: &c7_storm::StormEvidence, held_starts: usize, class: &Class) -> String {
    format!(
        "C7 storm {board}: resumes={} rows_kept={} omitted={} events={} held_starts={held_starts} probe_rows={} probe_dropped={} {}",
        e.stats_resumes,
        e.rows_kept,
        e.omitted_resumes,
        e.events.len(),
        e.probe_rows,
        e.probe_dropped,
        c7_verdict::describe(class)
    )
}

/// The storm's length in milliseconds: the hardware storm and its fake rehearsal run this long; a 60 s storm overflows the probe's 65 536-row record, measured on the fake.
pub const STORM_MS: i64 = 20_000;

/// Runs one storm: a Session of `c7::storm_trial` on `device` (1 MS/s, a Start held 12 ms every cycle, `STORM_MS`), prints its line and its VERDICT, and returns
/// the failure texts (a FAIL or an INCONCLUSIVE).
pub fn run_storm(device: Arc<dyn Device>, profile_of: &dyn Fn(&TempDir) -> Json, board: &str, period_us: i64, fake: bool) -> Vec<String> {
    let dir = TempDir::new();
    let probe = Arc::new(ProbeDevice::new(device));
    let storm = c7::storm_trial(&profile_of(&dir), probe, 1e6, period_us, Wall::from_millis(12), STORM_MS);
    let e = c7_storm::storm_evidence(&storm, fake);
    let class = c7_storm::storm_verdict(&e);
    let held_starts = storm.probe.iter().filter(|row| row["call"] == "tx_send" && row["class"] == "start" && row["held_ns"].as_u64().is_some_and(|held| held > 0)).count();
    println!("{}", storm_line(board, &e, held_starts, &class));
    let verdict = match &class {
        Class::Clean => format!("VERDICT c7 {board} storm: PASS (contract)"),
        Class::Inconclusive(reasons) => format!("VERDICT c7 {board} storm: INCONCLUSIVE: {}", reasons.join("; ")),
        Class::Fail(_, text) => format!("VERDICT c7 {board} storm: FAIL: {text}"),
        Class::Retried(retries) => format!("VERDICT c7 {board} storm: PASS (contract); retried {retries}"),
        Class::Unsettled(dropped) => format!("VERDICT c7 {board} storm: PASS (contract); unsettled {dropped}"),
    };
    println!("{verdict}");
    match class {
        Class::Clean => Vec::new(),
        _ => vec![verdict],
    }
}

/// Runs the bench grid for one mode: every rate of `RATES` with every cell of `MARGINS`, in that order. Every cell's lines are printed; the failures are
/// collected afterwards, one VERDICT line per cell that has a FAIL or an INCONCLUSIVE trial.
pub fn run_bench(device: Arc<dyn Device>, profile_of: &dyn Fn(&TempDir) -> Json, board: &str, mode: Mode, period_us: i64, fake_ramp: bool) -> Vec<String> {
    let mut failures = Vec::new();
    for &rate in &RATES {
        for &(margin_ns, trials) in &MARGINS {
            let (records, lines) = run_cell(device.clone(), profile_of, board, mode, rate, period_us, margin_ns, trials, fake_ramp);
            let bad = records.iter().any(|record| matches!(record.class, Class::Fail(_, _) | Class::Inconclusive(_)));
            if bad {
                if let Some(verdict) = lines.iter().rev().find(|line| line.starts_with("VERDICT c7 ")) {
                    failures.push(verdict.clone());
                }
            }
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(mode: Mode) -> Cell {
        Cell { mode, rate: 1e6, period_us: c7::FAKE_PERIOD_US, margin_ns: Some(10_000_000), stall: Stall::For(Wall::from_millis(40)), lead_us: None, retry_hold: None }
    }

    fn record(class: Class) -> TrialRecord {
        TrialRecord { class, datum: Datum::example() }
    }

    /// A record whose datum carries the trial's retry count.
    fn record_with_retries(class: Class, retries: usize) -> TrialRecord {
        let mut datum = Datum::example();
        datum.retries = retries;
        TrialRecord { class, datum }
    }

    /// The little `Evidence` that `trial_lines` needs: the anchor's bracket, and whatever a line reads.
    fn evidence_example() -> Evidence {
        Evidence {
            mode: Mode::LateStart,
            rate: 1e6,
            n: 100,
            per: 200,
            tx_origin: 0,
            origin: 0,
            cap_samples: 2_500,
            bracket: 200,
            fine: true,
            device_error: false,
            rx_overflow: false,
            link_drops: 0,
            omitted: false,
            probe_complete: true,
            incomplete: Vec::new(),
            booking_admitted: true,
            rows: Vec::new(),
            row_checks: Vec::new(),
            stats_resumes: 0,
            stats_gap: 0,
            events: Vec::new(),
            holds: Vec::new(),
            expected_holds: 1,
            starts: Vec::new(),
            reports: Vec::new(),
            stop_tick: 0,
            control_at: None,
            c0: 0,
            capture: c7_rf::Capture::new(Vec::new(), 200, 100, false),
            wave: c7_rf::Wave::new(&c7::wave(1e6, c7::FAKE_PERIOD_US)),
        }
    }

    #[test]
    fn c7_cells_a_clean_cell_prints_pass_contract() {
        let records = [record(Class::Clean), record(Class::Clean)];
        let lines = cell_lines("bench", Mode::LateStart, 1e6, c7::FAKE_PERIOD_US, 10_000_000, &records);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("C7 cell bench late_start 1MS/s pn=100us m=10: n=2 clean=2 retried=0"), "{}", lines[0]);
        assert_eq!(lines[1], "VERDICT c7 bench late_start 1MS/s pn=100us m=10: PASS (contract)");
    }

    #[test]
    fn c7_cells_retried_and_unsettled_trials_print_the_counts_and_never_a_bare_pass() {
        let records = [record_with_retries(Class::Retried(1), 1), record_with_retries(Class::Retried(2), 2), record_with_retries(Class::Unsettled(2), 2)];
        let lines = cell_lines("bench", Mode::Starved, 10e6, c7::FAKE_PERIOD_US, 200_000, &records);
        assert!(lines[0].contains("retried=2 (retries total 5; by r: 1:1 2:2) unsettled=1"), "{}", lines[0]);
        assert!(lines[0].starts_with("C7 cell bench starved 10MS/s pn=100us m=0.2: "), "{}", lines[0]);
        assert!(!lines[1].ends_with(": PASS (contract)"), "{}", lines[1]);
        assert_eq!(lines[1], "VERDICT c7 bench starved 10MS/s pn=100us m=0.2: PASS (contract); retries in 2/3 trials, unsettled in 1/3 -- read the C7 retry lines");
    }

    #[test]
    fn c7_cells_a_fail_names_the_failure_and_wins_over_inconclusive() {
        let records = [
            record(Class::Fail(Fail::StartedEarly, "a start of burst at 1 before its resume point 2".to_owned())),
            record(Class::Fail(Fail::StartedEarly, "a start of burst at 3 before its resume point 4".to_owned())),
            record(Class::Inconclusive(vec!["RX_OVERFLOW".to_owned()])),
        ];
        let lines = cell_lines("bench", Mode::LateStart, 1e6, c7::FAKE_PERIOD_US, 1_000_000, &records);
        assert!(lines[0].contains("inconclusive=1 (RX_OVERFLOW: 1) fail=2 (StartedEarly: 2)"), "{}", lines[0]);
        assert_eq!(lines[1], "VERDICT c7 bench late_start 1MS/s pn=100us m=1: FAIL: StartedEarly: 2");
        assert_eq!(Fail::StartedEarly.name(), "StartedEarly");
    }

    #[test]
    fn c7_cells_inconclusive_counts_reasons() {
        let records = [record(Class::Inconclusive(vec!["no control start in the probe".to_owned(), "booking refused".to_owned()])), record(Class::Inconclusive(vec!["booking refused".to_owned()]))];
        let lines = cell_lines("bench", Mode::Starved, 1e6, c7::FAKE_PERIOD_US, 10_000_000, &records);
        assert!(lines[0].contains("inconclusive=2 (no control start in the probe: 1, booking refused: 2)"), "{}", lines[0]);
        assert_eq!(lines[1], "VERDICT c7 bench starved 1MS/s pn=100us m=10: INCONCLUSIVE: no control start in the probe: 1, booking refused: 2");
    }

    #[test]
    fn c7_cells_the_delta_first_is_printed_signed_with_its_endpoints() {
        let mut datum = Datum::example();
        datum.delta_first_us = Some(-40);
        datum.t1 = Some(8_000);
        datum.r1 = Some(16_000);
        let record = TrialRecord { class: Class::Clean, datum };
        assert_eq!(delta_first_us(8_000, 16_000), -40, "t1 before r1 is negative");
        let e = evidence_example();
        let lines = trial_lines("bench", &cell(Mode::LateStart), 0, &record, &e);
        assert!(lines[0].contains("gap_us=1200 r_minus_busy_first_us=9000 delta_first_us=-40 (t1 = 8000, r1 = 16000 +- 200)"), "{}", lines[0]);
    }

    #[test]
    fn c7_the_trial_line_prints_the_first_rows_margin_on_every_trial() {
        let e = evidence_example();
        let lines = trial_lines("bench", &cell(Mode::LateStart), 0, &record(Class::Clean), &e);
        assert!(lines[0].contains("r_minus_busy_first_us=9000"), "{}", lines[0]);

        let expectation = |r_minus_busy_us, r_act| RowExpectation { r_minus_busy_us, m_bound: true, r_row: RRow { r_act, lo: r_act, hi: r_act, allowed: 1, uncertainty_us: 1, unresolved: false } };
        let mut datum = Datum::example();
        datum.rows = 2;
        datum.retries = 1;
        datum.row_data = vec![
            RowDatum { close_returned: Some(33_000_000), expectation: Some(expectation(9_000, 34_000_000)) },
            RowDatum { close_returned: Some(33_020_000), expectation: Some(expectation(4_000, 34_020_000)) },
        ];
        let mut e = evidence_example();
        e.rows = vec![
            Resume { cause: "late_at_device".to_owned(), from: 170_000, resumes_at: 170_100, at: 33_900_000 },
            Resume { cause: "late_at_device".to_owned(), from: 170_100, resumes_at: 170_200, at: 34_010_000 },
        ];
        let record = TrialRecord { class: Class::Retried(1), datum: datum.clone() };
        let lines = trial_lines("bench", &cell(Mode::LateStart), 0, &record, &e);
        assert!(lines[0].contains("r_minus_busy_first_us=9000"), "the first row, not the second: {}", lines[0]);
        let retry = lines.iter().find(|line| line.starts_with("C7 retry ") && line.contains(" row=2:")).expect("row 2's retry line");
        assert!(retry.contains("r_minus_busy_us=4000"), "{retry}");

        let cells = cell_lines("bench", Mode::LateStart, 1e6, c7::FAKE_PERIOD_US, 10_000_000, &[record]);
        assert!(cells[0].contains("r_minus_busy_us min/median/max=4000/9000/9000; r_minus_busy_first_us min/median/max=9000/9000/9000"), "{}", cells[0]);

        let mut no_rows = Datum::example();
        no_rows.rows = 0;
        no_rows.row_data.clear();
        let e = evidence_example();
        let lines = trial_lines("bench", &cell(Mode::LateStart), 0, &TrialRecord { class: Class::Clean, datum: no_rows }, &e);
        assert!(lines[0].contains("r_minus_busy_first_us=-"), "{}", lines[0]);

        let mut ambiguous = Datum::example();
        ambiguous.row_data[0].expectation = None;
        let lines = trial_lines("bench", &cell(Mode::LateStart), 0, &TrialRecord { class: Class::Clean, datum: ambiguous }, &e);
        assert!(lines[0].contains("r_minus_busy_first_us=-"), "{}", lines[0]);
    }

    #[test]
    fn c7_the_first_time_error_is_the_one_before_the_first_resume_point() {
        let reports = vec![("TimeError".to_owned(), Some(100)), ("TimeError".to_owned(), Some(900))];
        assert_eq!(first_time_error(&reports, 50, Some(500)), Some(100));
        assert_eq!(first_time_error(&[("TimeError".to_owned(), Some(900))], 50, Some(500)), None, "the first drop's report is missing");
        assert_eq!(first_time_error(&[("TimeError".to_owned(), Some(900))], 50, None), Some(900));
        assert_eq!(first_time_error(&reports, 50, Some(100)), None, "a report exactly at before is excluded");
        assert_eq!(first_time_error(&reports, 100, Some(500)), Some(100), "a report exactly at at is included");
        let other = [("Underflow".to_owned(), Some(100)), ("TimerError".to_owned(), Some(120))];
        assert_eq!(first_time_error(&other, 50, Some(500)), None, "only TimeError counts");
    }

    #[test]
    fn c7_cells_the_trial_line_prints_how_far_the_booking_moved() {
        let mut datum = Datum::example();
        datum.start_moved_samples = 37;
        let record = TrialRecord { class: Class::Clean, datum };
        let lines = trial_lines("bench", &cell(Mode::LateStart), 0, &record, &evidence_example());
        assert!(lines[0].contains("start_moved_samples=37"), "{}", lines[0]);
    }

    #[test]
    fn c7_cells_the_drain_distances_split_at_the_start_and_a_report_at_the_start_belongs_to_after() {
        let reports = vec![
            ("TimeError".to_owned(), Some(100)),
            ("Underflow".to_owned(), Some(300)),
            ("UnderflowInPacket".to_owned(), Some(400)),
            ("TimeError".to_owned(), Some(500)),
            ("Underflow".to_owned(), None),
            ("RxOverflow".to_owned(), Some(600)),
        ];
        let (before, after) = drain(&reports, 400);
        assert_eq!((before, after), (Some(300), Some(400)), "a report exactly at A belongs to after; tick-less and other codes are not drains");
        assert_eq!(drain(&reports, 350), (Some(300), Some(400)));
        assert_eq!(drain(&reports, 50), (None, Some(100)), "no report before the first");
        assert_eq!(drain(&reports, 700), (Some(500), None), "no report after the last");
    }

    #[test]
    fn c7_cells_no_line_says_a_drain_cleared() {
        let mut datum = Datum::example();
        datum.rows = 2;
        datum.retries = 1;
        let row = RowDatum {
            close_returned: Some(33_000_000),
            expectation: Some(RowExpectation {
                r_minus_busy_us: 9_000,
                m_bound: true,
                r_row: RRow { r_act: 34_000_000, lo: 34_000_000, hi: 34_000_000, allowed: 1, uncertainty_us: 1, unresolved: false },
            }),
        };
        datum.row_data = vec![row.clone(), row];
        let record = TrialRecord { class: Class::Retried(1), datum };
        let mut e = evidence_example();
        e.starts = vec![c7_verdict::StartRow { at: 34_000_000, accepted: 2_500, returned: 33_900_000 }, c7_verdict::StartRow { at: 34_020_000, accepted: 2_500, returned: 33_920_000 }];
        e.rows = vec![
            Resume { cause: "late_at_device".to_owned(), from: 170_000, resumes_at: 170_100, at: 33_900_000 },
            Resume { cause: "late_at_device".to_owned(), from: 170_100, resumes_at: 170_200, at: 34_010_000 },
        ];
        let lines = trial_lines("bench", &cell(Mode::LateStart), 0, &record, &e);
        assert!(lines.iter().any(|line| line.starts_with("C7 retry ")), "{lines:?}");
        for line in &lines {
            assert!(!line.contains("cleared"), "{line}");
        }
    }

    #[test]
    fn c7_cells_every_start_and_every_retry_is_printed() {
        let mut datum = Datum::example();
        datum.rows = 8;
        datum.retries = 7;
        datum.starts = (1..=7)
            .map(|j| StartDatum {
                j,
                a: 34_000_000 + j as i64 * 20_000,
                played: Some(true),
                d_before: Some(33_000_000),
                d_after: Some(34_100_000),
                a_minus_dbefore_us: Some(50_000),
                dafter_minus_a_us: Some(500),
            })
            .collect();
        datum.row_data = (0..8)
            .map(|index| RowDatum {
                close_returned: Some(30_000_000 + index as i64 * 1_000),
                expectation: Some(RowExpectation {
                    r_minus_busy_us: 9_000,
                    m_bound: true,
                    r_row: RRow { r_act: 34_000_000, lo: 34_000_000, hi: 34_000_000, allowed: 1, uncertainty_us: 1, unresolved: false },
                }),
            })
            .collect();
        let record = TrialRecord { class: Class::Retried(7), datum };
        let mut e = evidence_example();
        e.starts = (0..=7).map(|j| c7_verdict::StartRow { at: 34_000_000 + j as i64 * 20_000, accepted: 2_500, returned: 33_000_000 + j as i64 * 20_000 }).collect();
        e.rows = (0..8).map(|index| Resume { cause: "late_at_device".to_owned(), from: 170_000 + index as i64, resumes_at: 170_100 + index as i64, at: 33_900_000 + index as i64 * 20_000 }).collect();
        let lines = trial_lines("bench", &cell(Mode::LateStart), 0, &record, &e);
        let starts: Vec<&String> = lines.iter().filter(|line| line.starts_with("C7 start ")).collect();
        assert_eq!(starts.len(), 7, "every start is printed: {lines:?}");
        assert!(starts.iter().any(|line| line.contains(" j=7:")), "{starts:?}");
        let retries: Vec<&String> = lines.iter().filter(|line| line.starts_with("C7 retry ")).collect();
        assert_eq!(retries.len(), 7, "every retry is printed: {lines:?}");
        assert!(!lines.iter().any(|line| line.contains("not printed")), "{lines:?}");
        let row2 = retries.first().unwrap();
        assert!(row2.contains("close_returned=30001000"), "{row2}");
        assert!(row2.contains("dropped_start_A=34020000"), "{row2}");
        assert!(row2.contains("dropped_start_returned=33020000"), "{row2}");
    }

    #[test]
    fn c7_cells_an_unresolved_row_prints_its_interval_and_a_resolved_one_does_not() {
        let mut datum = Datum::example();
        datum.rows = 2;
        datum.retries = 1;
        datum.period_us = 100;
        datum.row_data = vec![
            RowDatum {
                close_returned: Some(33_000_000),
                expectation: Some(RowExpectation {
                    r_minus_busy_us: 9_000,
                    m_bound: true,
                    r_row: RRow { r_act: 34_000_000, lo: 34_000_000, hi: 34_000_000, allowed: 1, uncertainty_us: 1, unresolved: false },
                }),
            },
            RowDatum {
                close_returned: Some(33_020_000),
                expectation: Some(RowExpectation {
                    r_minus_busy_us: 9_000,
                    m_bound: true,
                    r_row: RRow { r_act: 34_020_000, lo: 34_000_000, hi: 34_040_000, allowed: 3, uncertainty_us: 302, unresolved: true },
                }),
            },
        ];
        let record = TrialRecord { class: Class::Clean, datum };
        let e = evidence_example();
        let lines = trial_lines("bench", &cell(Mode::LateStart), 0, &record, &e);
        assert!(lines[0].contains("allowed=1 r_unresolved=1 gap_us=1200 r_minus_busy_first_us=9000"), "{}", lines[0]);
        assert!(
            lines.contains(
                &"C7 r_unresolved bench late_start 1MS/s pn=100us m=10 #0 row=2: R_act=34020000 interval=[34000000, 34040000] allowed=3 uncertainty_us=302 period_us=100 (the interval holds several lattice points; the R check accepts every one of them and none outside)".to_owned()
            ),
            "{lines:?}"
        );
        assert!(!lines.iter().any(|line| line.contains("row=1:")), "the resolved row prints no line: {lines:?}");
    }

    #[test]
    fn c7_cells_the_storm_line_names_what_it_counted() {
        let e = c7_storm::StormEvidence {
            fine: true,
            incomplete: Vec::new(),
            probe_complete: true,
            booking_admitted: true,
            omitted: Vec::new(),
            rows_kept: 1_000,
            omitted_resumes: 203,
            stats_resumes: 1_203,
            link_drops: 0,
            row_checks: Vec::new(),
            bound: 1_000,
            close_held: false,
            held_first_at: Some(1),
            row_plans: Vec::new(),
            start_ticks: Vec::new(),
            row_ticks: Vec::new(),
            events: vec![1; 1_203],
            lattice_tick: 0,
            probe_rows: 65_535,
            probe_dropped: 1,
            control_at: None,
            c0: 0,
            stop_tick: 0,
            scheduled_samples: 600_000,
            capture: c7_rf::Capture::new(Vec::new(), 200, 100, false),
            wave: c7_rf::Wave::new(&c7::wave(1e6, c7::FAKE_PERIOD_US)),
            fake: true,
        };
        assert_eq!(storm_line("fake", &e, 1_203, &Class::Clean), "C7 storm fake: resumes=1203 rows_kept=1000 omitted=203 events=1203 held_starts=1203 probe_rows=65535 probe_dropped=1 CLEAN");
        assert_eq!(
            storm_line("fake", &e, 1_203, &Class::Inconclusive(vec!["the storm did not reach the bound".to_owned()])),
            "C7 storm fake: resumes=1203 rows_kept=1000 omitted=203 events=1203 held_starts=1203 probe_rows=65535 probe_dropped=1 INCONCLUSIVE: the storm did not reach the bound"
        );
    }

    #[test]
    fn c7_cells_the_bench_grid_is_two_rates_three_margins_with_ten_twenty_twenty_trials() {
        assert_eq!(RATES, [1e6, 10e6]);
        assert_eq!(MARGINS, [(10_000_000, 10), (1_000_000, 20), (200_000, 20)]);
    }
}
