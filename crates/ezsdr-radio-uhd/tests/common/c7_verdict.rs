//! bt2's verdict for cell C7: the evidence a finished trial leaves (section 3 of the B3 specification) and the class the B3 grid gives it. The capture is read
//! only through `c7_rf` (the control's delay and gain, the part's first period and steady periods, the silence region and the pre-stall RF); every other
//! term is the Session's own record (the Manifest's `timing`, `async`, `stats` and `rejected` sections and the probe's rows).
//!
//! The order is the specification's: the INCONCLUSIVE reasons a verdict's later checks rest on are collected first and returned together; the structural
//! failures follow in their own order; the grid (rows x final Start: none, heard, silent) is read last. A verdict never prints: the caller prints `describe`.

#![allow(dead_code)]

use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::session::Outcome;
use serde_json::Value as Json;

use super::c7::{self, Mode, Resume, Trial};
use super::c7_rf::{self, Capture, Control, PartReading, Wave};
use super::*;

/// The 5 ms "tail" threshold (INFERRED, plan Q15): how long past a drop a Run must have run before nothing heard is `DroppedNeverResumed` rather than a datum.
pub const OPEN_NS: i64 = 5_000_000;

/// One accepted payload Start after the hold was armed, in root ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartRow {
    pub at: i64,
    pub accepted: i64,
    /// The Start's own `tx_send` row's mapped return, in root ticks.
    pub returned: i64,
}

/// The probe's held send: its class, the transmit sample it starts at, what the device took, and its host stamps mapped through the anchor to root ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldSend {
    pub class: String,
    pub at: Option<i64>,
    pub accepted: i64,
    pub entered: i64,
    pub returned: i64,
    pub held_ns: u64,
}

/// Everything the verdict reads, taken from one finished trial: the Manifest's termination, sections and events, the probe's rows, and the capture through
/// the detector.
#[derive(Clone)]
pub struct Evidence {
    pub mode: Mode,
    pub rate: f64,
    /// Samples of one period of the repeat's PN.
    pub n: i64,
    /// Root ticks a transmit or receive sample.
    pub per: i64,
    /// The root tick of transmit sample 0.
    pub tx_origin: i64,
    /// The repeat's lattice origin, a transmit sample.
    pub origin: i64,
    /// A full send of the Module, in samples.
    pub cap_samples: i64,
    /// The anchor's uncertainty, in ticks, for the two mapped comparisons.
    pub bracket: i64,
    pub fine: bool,
    pub device_error: bool,
    pub rx_overflow: bool,
    /// The blocks the capture's data link dropped (the Manifest's `links` record): a drop-oldest link in front of a starved capture thread leaves a hole in the
    /// capture, which the readers would report as a hole.
    pub link_drops: u64,
    pub omitted: bool,
    pub probe_complete: bool,
    pub incomplete: Vec<String>,
    pub booking_admitted: bool,
    /// The Module's `tx_resume` rows.
    pub rows: Vec<Resume>,
    /// `Trial::check` per row, the terms dropped.
    pub row_checks: Vec<Result<(), String>>,
    pub stats_resumes: i64,
    pub stats_gap: i64,
    /// The `resumes_at` of each `radio.TIME_ERROR` / `radio.TX_UNDERFLOW` event that carries a plan.
    pub events: Vec<Option<i64>>,
    pub holds: Vec<HeldSend>,
    /// How many held sends the cell arms: 1, plus one when the cell holds the resumed part's own Start (S4').
    pub expected_holds: usize,
    /// The accepted payload Starts after arming, in order.
    pub starts: Vec<StartRow>,
    /// The device's asynchronous reports: `(code, tick)`.
    pub reports: Vec<(String, Option<i64>)>,
    pub stop_tick: i64,
    /// The control repeat's first accepted start, in root ticks.
    pub control_at: Option<i64>,
    pub c0: i64,
    pub capture: Capture,
    pub wave: Wave,
}

/// The named failures of the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Counters,
    EventWithoutRow,
    ResumeOffRule,
    LatticeOff,
    StartedEarly,
    SilenceViolated,
    PhaseDiscontinuous,
    LateRestart,
    NoResume,
    DroppedNeverResumed,
    BoundWrong,
    StormRadiated,
    CloseHeld,
    EventsOutsideRows,
    EventsExceedResumes,
    EventsMissing,
}

impl Fail {
    /// The variant's name, as the cell's decision lines print it.
    pub fn name(self) -> &'static str {
        match self {
            Fail::Counters => "Counters",
            Fail::EventWithoutRow => "EventWithoutRow",
            Fail::ResumeOffRule => "ResumeOffRule",
            Fail::LatticeOff => "LatticeOff",
            Fail::StartedEarly => "StartedEarly",
            Fail::SilenceViolated => "SilenceViolated",
            Fail::PhaseDiscontinuous => "PhaseDiscontinuous",
            Fail::LateRestart => "LateRestart",
            Fail::NoResume => "NoResume",
            Fail::DroppedNeverResumed => "DroppedNeverResumed",
            Fail::BoundWrong => "BoundWrong",
            Fail::StormRadiated => "StormRadiated",
            Fail::CloseHeld => "CloseHeld",
            Fail::EventsOutsideRows => "EventsOutsideRows",
            Fail::EventsExceedResumes => "EventsExceedResumes",
            Fail::EventsMissing => "EventsMissing",
        }
    }
}

/// The verdict: CLEAN, a datum (RETRIED, UNSETTLED), or a reason it cannot be read (INCONCLUSIVE) or a named failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Class {
    Clean,
    Retried(usize),
    Unsettled(usize),
    Inconclusive(Vec<String>),
    Fail(Fail, String),
}

/// The repeat's silence starts here: the held send's time in a late-start cell (the whole device burst was dropped), the starved row's handed sample in a
/// starved one (the device had nothing past it).
fn region_from(e: &Evidence) -> Option<i64> {
    match e.mode {
        Mode::LateStart => e.holds.first().and_then(|held| held.at),
        Mode::Starved => e.rows.first().map(|row| e.tx_origin + row.from * e.per),
    }
}

pub fn is_drop(code: &str) -> bool {
    code == "TimeError" || code.starts_with("Underflow")
}

/// Whether the device reported a `TimeError` at or after `at`.
fn drop_after(e: &Evidence, at: i64) -> bool {
    e.reports.iter().any(|(code, tick)| code == "TimeError" && tick.is_some_and(|tick| tick >= at))
}

/// Whether the device reported a drop (`TimeError` or an `Underflow`) at or after `at`.
fn evidence_after(e: &Evidence, at: i64) -> bool {
    e.reports.iter().any(|(code, tick)| is_drop(code) && tick.is_some_and(|reported| reported >= at))
}

/// A coverage hole is one INCONCLUSIVE reason, however many readers report it.
fn push_hole(reasons: &mut Vec<String>, text: String) {
    if !reasons.contains(&text) {
        reasons.push(text);
    }
}

/// Every capture reading a trial's verdict and datum share: the control, and one part per `Evidence::starts` (the same index).
pub struct Readings {
    pub control: Result<Control, String>,
    /// Empty when the control is unresolved: no part can be read without it.
    pub parts: Vec<Result<PartReading, String>>,
}

/// Reads the capture once: the control's delay and gain, then each accepted Start's part.
pub fn read(e: &Evidence) -> Readings {
    let control = match e.control_at {
        Some(at) => c7_rf::analyse_control(&e.capture, &e.wave, e.c0, at),
        None => Err("no control start in the probe".to_owned()),
    };
    let parts = match &control {
        Ok(control) => e.starts.iter().map(|start| c7_rf::read_part(&e.capture, &e.wave, control, start.at)).collect(),
        Err(_) => Vec::new(),
    };
    Readings { control, parts }
}

/// The class of `e`, by the specification's order of evaluation.
pub fn verdict(e: &Evidence) -> Class {
    verdict_with(e, &read(e))
}

/// The class of `e` from capture readings taken once.
pub fn verdict_with(e: &Evidence, readings: &Readings) -> Class {
    let mode = e.mode;
    let m = e.rows.len();
    let open = c7::ns_to_ticks_ceil(OPEN_NS);

    // 1. The preconditions the later checks rest on: every reason that applies, in the specification's order.
    let mut reasons: Vec<String> = Vec::new();
    if !e.fine {
        reasons.push("the Run did not end Fine".to_owned());
    }
    if e.device_error {
        reasons.push("a device error row".to_owned());
    }
    if e.rx_overflow {
        reasons.push("RX_OVERFLOW".to_owned());
    }
    if e.link_drops > 0 {
        reasons.push(format!("the capture link dropped {} blocks", e.link_drops));
    }
    if e.omitted || !e.probe_complete {
        reasons.push("rows omitted or the probe is incomplete".to_owned());
    }
    reasons.extend(e.incomplete.iter().cloned());
    if !e.booking_admitted {
        reasons.push("booking refused".to_owned());
    }
    for check in &e.row_checks {
        if let Err(text) = check {
            if text.starts_with("ambiguous") {
                reasons.push(text.clone());
            }
        }
    }
    let control = match &readings.control {
        Ok(control) => Some(control),
        Err(text) => {
            reasons.push(text.clone());
            None
        }
    };
    let class = match mode {
        Mode::LateStart => "start",
        Mode::Starved => "continuation",
    };
    let held = match e.holds.first() {
        Some(held) if e.holds.len() == e.expected_holds && held.class == class && (mode == Mode::Starved || (held.accepted > 0 && held.at.is_some())) => Some(held),
        _ => None,
    };
    if held.is_none() {
        reasons.push("the hold did not fire as the cell says".to_owned());
    }
    let unpaired = !e.starts.is_empty() && e.starts.len() - 1 > m;
    if unpaired {
        reasons.push("unpaired start".to_owned());
    }
    let final_index = if unpaired || e.starts.is_empty() || e.starts.len() - 1 < m { None } else { Some(if m == 0 { 0 } else { m }) };
    let final_start = final_index.map(|index| &e.starts[index]);
    // The held send's time: the start's own tick, or the held call's entry (the device runs dry while the call is still held).
    let held_time = held.and_then(|held| match mode {
        Mode::LateStart => held.at,
        Mode::Starved => Some(held.entered - e.bracket),
    });
    // A starved repeat with no resume: the held call's return is the earliest instant the Module can decide at.
    let starved_tail = held.map(|held| e.stop_tick - held.returned);
    // How long the Run ran past the last drop: the grid's tail, and the UNSETTLED exits' own condition.
    let tail = match e.rows.last() {
        Some(row) => e.stop_tick - (e.tx_origin + row.resumes_at * e.per),
        None => {
            let time = held_time.unwrap_or(e.c0);
            let last = e.reports.iter().rev().find(|(code, tick)| is_drop(code) && tick.is_some_and(|tick| tick >= time));
            match last {
                Some((_, Some(tick))) => e.stop_tick - *tick,
                _ => e.stop_tick - e.c0,
            }
        }
    };

    if let Some(held) = held {
        match mode {
            Mode::LateStart => {
                let shown_late = held.at.is_some_and(|at| held.returned - e.bracket > at);
                if !shown_late {
                    reasons.push("the held start was not shown late (returned within the bracket of its time)".to_owned());
                }
                if m >= 1 {
                    let cause = e.rows[0].cause.as_str();
                    if !matches!(cause, "late_at_device" | "expired") {
                        reasons.push("the first resume's cause is not a late start's".to_owned());
                    }
                }
            }
            Mode::Starved => {
                // The starvation's evidence is the RF before the stall; the Underflow report is printed as data, not required.
                if let (Some(control), Some(before)) = (&control, held_time) {
                    if !c7_rf::prestall_rf(&e.capture, &e.wave, control, before) {
                        reasons.push("no RF before the stall".to_owned());
                    }
                }
                if m >= 1 && !matches!(e.rows[0].cause.as_str(), "underflow" | "expired") {
                    reasons.push("the first resume's cause is not a starvation's".to_owned());
                }
            }
        }
    }

    // The final Start's reading decides the column; a hole in the coverage is INCONCLUSIVE, never silence.
    let mut reading: Option<&PartReading> = None;
    let mut heard_final = false;
    if let (Some(index), Ok(_)) = (final_index, &readings.control) {
        match &readings.parts[index] {
            Ok(part) => {
                heard_final = part.w0_heard;
                reading = Some(part);
            }
            Err(text) => push_hole(&mut reasons, text.clone()),
        }
    }
    // The gap the drop evidence says is silent is scanned: the heard column up to the final Start's root tick, the two UNSETTLED exits up to the Run's stop.
    // Every other exit returns without scanning (a late-heard part, a NoResume cell: RF there is legitimate).
    let unsettled = m >= 1 && tail < open && ((final_start.is_none() && m >= 2) || final_start.is_some_and(|start| !heard_final && drop_after(e, start.at)));
    let gap_to = match control {
        Some(control) if m >= 1 && heard_final => final_start.map(|start| start.at + control.d),
        Some(control) if unsettled => Some(e.stop_tick + control.d),
        _ => None,
    };
    // The correlation on the repeat's lattice is the more sensitive test when the leak is aligned; the power test reads a leak whatever its phase. A leak is
    // whichever finds RF first, and a hole in either is INCONCLUSIVE, never silence.
    let mut leak: Option<String> = None;
    if let (Some(control), Some(gap_to), Some(from)) = (control, gap_to, region_from(e)) {
        let to = gap_to - control.d;
        match c7_rf::silence(&e.capture, &e.wave, control, from, gap_to, e.tx_origin + e.origin * e.per) {
            Ok(Some(tick)) => leak = Some(format!("heard {tick} inside the gap [{from}, {to})")),
            Ok(None) => {}
            Err(text) => push_hole(&mut reasons, text),
        }
        if leak.is_none() {
            match c7_rf::quiet(&e.capture, control, from, gap_to) {
                Ok(Some((tick, power))) => leak = Some(format!("RF power {power:.3e} (threshold {:.3e}) at {tick} inside the gap [{from}, {to})", control.tp)),
                Ok(None) => {}
                Err(text) => push_hole(&mut reasons, text),
            }
        }
    }
    // A starved repeat with no resume: its first Start was heard before the stall, so the RF after the held send's return decides the cell (not a gate on the
    // Underflow report, which stays printed as data).
    let mut starved_heard: Option<bool> = None;
    if mode == Mode::Starved && m == 0 && final_start.is_some() && starved_tail.is_some_and(|tail| tail >= open) {
        if let (Some(control), Some(held)) = (control, held) {
            match c7_rf::silence(&e.capture, &e.wave, control, held.returned, e.stop_tick + control.d, e.tx_origin + e.origin * e.per) {
                Ok(found) => starved_heard = Some(found.is_some()),
                Err(text) => push_hole(&mut reasons, text),
            }
        }
    }
    // A heard final part needs its first period and 19 steady ones: the Module's Stop closes the burst after what it has already handed over.
    if heard_final && final_start.is_some_and(|start| start.at + 20 * e.n * e.per > e.stop_tick) {
        reasons.push("the Run stopped before the final part's 20 periods".to_owned());
    }
    if !reasons.is_empty() {
        return Class::Inconclusive(reasons);
    }

    // 2. The structural failures, in the specification's order.
    if !e.omitted {
        let gap: i64 = e.rows.iter().map(|row| row.resumes_at - row.from).sum();
        if e.stats_resumes != m as i64 || e.stats_gap != gap {
            return Class::Fail(Fail::Counters, "counters disagree with rows".to_owned());
        }
    }
    for event in e.events.iter().flatten() {
        if !e.rows.iter().any(|row| row.resumes_at == *event) {
            return Class::Fail(Fail::EventWithoutRow, format!("event plan {event} is no row's resume point"));
        }
    }
    for check in &e.row_checks {
        if let Err(text) = check {
            return Class::Fail(Fail::ResumeOffRule, text.clone());
        }
    }
    for row in &e.rows {
        if (row.resumes_at - e.origin).rem_euclid(e.n) != 0 {
            return Class::Fail(Fail::LatticeOff, format!("R {} is off the lattice", e.tx_origin + row.resumes_at * e.per));
        }
    }
    for start in e.starts.iter().skip(1) {
        let sample = (start.at - e.tx_origin).div_euclid(e.per);
        if (start.at - e.tx_origin).rem_euclid(e.per) != 0 || (sample - e.origin).rem_euclid(e.n) != 0 {
            return Class::Fail(Fail::LatticeOff, format!("R {} is off the lattice", start.at));
        }
    }
    for j in 1..=m.min(e.starts.len().saturating_sub(1)) {
        let at = e.starts[j].at;
        let r = e.tx_origin + e.rows[j - 1].resumes_at * e.per;
        if at < r {
            return Class::Fail(Fail::StartedEarly, format!("a start of burst at {at} before its resume point {r}"));
        }
    }

    // 3. The grid.
    let dropped = "the last part was dropped and the Run ran >= 5 ms past it with no resume".to_owned();
    match (m, final_start) {
        (0, None) => Class::Inconclusive(vec!["no incident".to_owned()]),
        (0, Some(_)) if mode == Mode::Starved => {
            if starved_tail.is_some_and(|tail| tail < open) {
                Class::Inconclusive(vec!["ended before a resume".to_owned()])
            } else if starved_heard == Some(true) {
                Class::Inconclusive(vec!["no incident".to_owned()])
            } else {
                Class::Fail(Fail::DroppedNeverResumed, dropped)
            }
        }
        (0, Some(_)) if heard_final => Class::Inconclusive(vec!["no incident".to_owned()]),
        (0, Some(_)) => {
            if !evidence_after(e, held_time.unwrap_or(e.c0)) {
                Class::Inconclusive(vec!["no incident".to_owned()])
            } else if tail < open {
                Class::Inconclusive(vec!["ended before a resume".to_owned()])
            } else {
                Class::Fail(Fail::DroppedNeverResumed, dropped)
            }
        }
        (_, None) => {
            if tail >= open {
                Class::Inconclusive(vec!["no start after resume".to_owned()])
            } else if m == 1 {
                Class::Inconclusive(vec!["ended before R + 5 ms".to_owned()])
            } else {
                if let Some(text) = leak {
                    return Class::Fail(Fail::SilenceViolated, text);
                }
                Class::Unsettled(m - 1)
            }
        }
        (_, Some(_)) if heard_final => {
            if let Some(text) = leak {
                return Class::Fail(Fail::SilenceViolated, text);
            }
            let part = reading.as_ref().expect("a heard final part is read");
            if !part.steady_ok {
                return Class::Fail(Fail::PhaseDiscontinuous, format!("the waveform at R is not the lattice's (gain {:.3}, phase {:.1} vs control)", part.worst_gain_ratio, part.worst_phase_deg));
            }
            if m == 1 { Class::Clean } else { Class::Retried(m - 1) }
        }
        (_, Some(start)) => {
            if drop_after(e, start.at) {
                if tail < open {
                    if let Some(text) = leak {
                        return Class::Fail(Fail::SilenceViolated, text);
                    }
                    Class::Unsettled(m)
                } else {
                    Class::Fail(Fail::DroppedNeverResumed, dropped)
                }
            } else if let Some(k) = reading.as_ref().and_then(|part| part.late_by_periods) {
                Class::Fail(Fail::LateRestart, format!("RF started {k} period(s) after the accepted start"))
            } else {
                Class::Fail(Fail::NoResume, format!("the repeat is silent after its last start {}", start.at))
            }
        }
    }
}

/// The printed line of a verdict: the datum with its count, the reasons joined, or the failure's own text.
pub fn describe(class: &Class) -> String {
    match class {
        Class::Clean => "CLEAN".to_owned(),
        Class::Retried(retries) => {
            format!("RETRIED({retries}): the repeat resumed after {retries} retries; the last part is phase-continuous on the lattice (a datum, not a failure)")
        }
        Class::Unsettled(dropped) => format!("UNSETTLED({dropped}): {dropped} dropped resumed part(s), nothing heard from a part by the end of the trial (a datum, not a failure)"),
        Class::Inconclusive(reasons) => format!("INCONCLUSIVE: {}", reasons.join("; ")),
        Class::Fail(_, text) => format!("FAIL: {text}"),
    }
}

/// The blocks the Run's data links dropped, from the Manifest's `links` (a link whose count cleanup never read counts none).
pub fn link_drops(manifest: &Manifest) -> u64 {
    manifest.links.iter().map(|link| link.drops.unwrap_or(0)).sum()
}

/// Whether the Manifest has no usable `rec` capture: no artifact, no continuity map, or no valid span in it.
pub fn capture_missing(manifest: &Manifest) -> bool {
    manifest
        .artifacts
        .iter()
        .find(|(output, _)| output.as_str() == "rec")
        .and_then(|(_, list)| list.first())
        .and_then(|artifact| artifact.continuity.first())
        .is_none_or(|map| map.valid.iter().all(Vec::is_empty))
}

/// The evidence a finished trial's Manifest must carry and does not: one reason per missing part.
pub fn missing_evidence(manifest: &Manifest, sections: &[&str]) -> Vec<String> {
    let mut reasons = Vec::new();
    if capture_missing(manifest) {
        reasons.push("no capture artifact `rec` in the Manifest".to_owned());
    }
    for section in sections {
        if manifest.section(&usrp(), &format!("ezsdr.radio.uhd.{section}")).is_none() {
            reasons.push(format!("no `{section}` section in the Manifest"));
        }
    }
    reasons
}

/// The evidence of a finished trial. `fake_ramp` is true on the `FakeDevice`, whose idle sample is its ramp.
pub fn evidence(trial: &Trial, fake_ramp: bool) -> Evidence {
    let manifest = &trial.manifest;
    let rate = trial.cell.rate;
    let per = trial.per;
    let missing = missing_evidence(manifest, &["timing", "async", "stats", "rejected"]);
    let accepted = |row: &Json| row["result"]["accepted_samples"].as_i64().unwrap_or(0);
    let holds: Vec<HeldSend> = trial
        .probe
        .iter()
        .filter(|row| row["call"] == "tx_send" && row["held_ns"].as_u64().is_some_and(|held| held > 0))
        .map(|row| HeldSend {
            class: row["class"].as_str().unwrap_or_default().to_owned(),
            at: row["at"].as_i64(),
            accepted: accepted(row),
            entered: trial.anchor.map(row["host_entered_ns"].as_u64().unwrap()),
            returned: trial.anchor.map(row["host_after_ns"].as_u64().unwrap()),
            held_ns: row["held_ns"].as_u64().unwrap(),
        })
        .collect();
    let starts: Vec<StartRow> = trial
        .probe
        .iter()
        .filter(|row| row["call"] == "tx_send" && row["class"] == "start" && accepted(row) > 0)
        .skip(trial.control_starts)
        .map(|row| StartRow { at: row["at"].as_i64().unwrap(), accepted: accepted(row), returned: trial.anchor.map(row["host_after_ns"].as_u64().unwrap()) })
        .collect();
    let rows = trial.resumes();
    let row_checks = rows.iter().enumerate().map(|(index, row)| trial.check(index, row, trial.cell.margin()).map(|_| ())).collect();
    let stats = manifest.section(&usrp(), "ezsdr.radio.uhd.stats").cloned().unwrap_or(Json::Null);
    let mut incomplete = trial.incomplete.clone();
    incomplete.extend(missing);
    Evidence {
        mode: trial.cell.mode,
        rate,
        n: trial.period,
        per,
        tx_origin: trial.tx_origin,
        origin: trial.origin(),
        cap_samples: c7::send_cap(rate) as i64,
        bracket: trial.probe_uncertainty(),
        fine: failure(manifest).is_empty(),
        device_error: manifest.section(&usrp(), "ezsdr.radio.uhd.rejected").and_then(|section| section.as_array()).is_some_and(|rows| rows.iter().any(|row| row["error"].is_string())),
        rx_overflow: !events_of(manifest, "radio.RX_OVERFLOW").is_empty(),
        link_drops: link_drops(manifest),
        omitted: !omitted_rows(manifest).is_empty(),
        probe_complete: trial.rows_complete.is_ok(),
        incomplete,
        booking_admitted: trial.booking.as_ref().is_some_and(|entry| matches!(entry.outcome, Outcome::Admitted { .. })),
        rows,
        row_checks,
        stats_resumes: stats["tx_resumes"].as_i64().unwrap_or(0),
        stats_gap: stats["tx_gap_samples"].as_i64().unwrap_or(0),
        events: trial.event_resume_points().into_iter().map(Some).collect(),
        holds,
        expected_holds: 1 + usize::from(trial.cell.retry_hold.is_some()),
        starts,
        reports: trial.reports(),
        stop_tick: trial.stop_tick,
        control_at: trial.control,
        c0: trial.c0,
        capture: if capture_missing(manifest) {
            Capture::new(Vec::new(), per, trial.period as usize, fake_ramp)
        } else {
            Capture::new(n47s5::captured_samples(manifest), per, trial.period as usize, fake_ramp)
        },
        wave: Wave::new(&c7::wave(rate, trial.period_us)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::c7_rf::synth::Synth;
    use super::super::{MCR, c7};
    use super::*;

    const RATE: f64 = 1e6;
    const PER: i64 = 200;
    const N: i64 = 100;
    const SPAN: i64 = N * PER;
    const MS: i64 = MCR / 1_000;
    const NS_PER_TICK: i64 = 1_000_000_000 / MCR;
    /// The loop delay of the fixtures' control, in samples.
    const DELAY: i64 = 44 * PER;
    const MARGIN: i64 = 10 * MS;
    const BRACKET: i64 = 200;
    /// A full first send at 1 MS/s (`c7::send_cap(1e6)`).
    const CAP: i64 = 2_500;
    const CAPTURE_END: i64 = c7::CAPTURE_MS * MS;
    const CONTROL_AT: i64 = 25 * MS;
    /// The late-start cell's repeat starts here (its lattice origin).
    const LATE_AT: i64 = 130 * MS;
    /// The starved cell's repeat starts here.
    const STARVED_AT: i64 = 100 * MS;

    /// Which column of the grid a fixture is in: no Start after the last resume, the final Start heard, or the final Start silent.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Last {
        None,
        Heard,
        Silent,
    }

    /// One cell's consistent schedule, and the capture it is heard in.
    struct Fixture {
        mode: Mode,
        /// The PN's period in microseconds: `FAKE_PERIOD_US` unless a test builds the hardware period's lattice.
        period_us: i64,
        /// The repeat's lattice origin, a transmit sample.
        origin: i64,
        held: HeldSend,
        starts: Vec<StartRow>,
        rows: Vec<Resume>,
        reports: Vec<(String, Option<i64>)>,
        stop_tick: i64,
        last: Last,
    }

    impl Fixture {
        fn new(mode: Mode, m: usize, last: Last) -> Fixture {
            Fixture::new_at(mode, m, last, c7::FAKE_PERIOD_US)
        }

        /// The cell on a PN of `period_us`: the heard part plays 30 ms (300 periods at 100 us) and the starved pre-stall part `499 * 100 / period_us` periods
        /// (49.9 ms at 100 us, 49 ms at 1 ms), so only the schedule's lattice changes.
        fn new_at(mode: Mode, m: usize, last: Last, period_us: i64) -> Fixture {
            let n = c7::period(RATE, period_us) as i64;
            let span = n * PER;
            let play = DELAY + 300 * 100 / period_us * span;
            let stall = STARVED_AT + DELAY + 499 * 100 / period_us * span;
            let first = match mode {
                Mode::LateStart => LATE_AT,
                Mode::Starved => STARVED_AT,
            };
            let (entered, returned) = match mode {
                Mode::LateStart => (first, first + 40 * MS),
                Mode::Starved => (first + 30 * MS, first + 52 * MS),
            };
            let held = HeldSend {
                class: match mode {
                    Mode::LateStart => "start",
                    Mode::Starved => "continuation",
                }
                .to_owned(),
                at: Some(first),
                accepted: CAP,
                entered,
                returned,
                held_ns: ((returned - entered) * NS_PER_TICK) as u64,
            };
            let mut starts = Vec::new();
            if !(m == 0 && last == Last::None) {
                starts.push(StartRow { at: first, accepted: CAP, returned: first - 2_000_000 });
            }
            // The held drop's report; a starved cell reports the underflow in the stall.
            let mut reports = vec![match mode {
                Mode::LateStart => ("TimeError".to_owned(), Some(first)),
                Mode::Starved => ("Underflow".to_owned(), Some(stall)),
            }];
            let mut rows: Vec<Resume> = Vec::new();
            let mut previous_at = first;
            let mut previous_from = match mode {
                Mode::LateStart => first / PER,
                Mode::Starved => stall / PER,
            };
            for j in 1..=m {
                // R_j: the first lattice point at or after the previous part's end plus the margin.
                let after = match (mode, j) {
                    (Mode::Starved, 1) => stall + MARGIN,
                    _ => previous_at + play + MARGIN,
                };
                let at = first + (after - first + span - 1).div_euclid(span) * span;
                rows.push(Resume {
                    cause: match mode {
                        Mode::LateStart => "late_at_device",
                        Mode::Starved => "underflow",
                    }
                    .to_owned(),
                    from: previous_from,
                    resumes_at: at / PER,
                    at: match j {
                        1 => returned,
                        _ => previous_at,
                    },
                });
                if j < m || last != Last::None {
                    starts.push(StartRow { at, accepted: CAP, returned: at - 2_000_000 });
                }
                if j < m || last == Last::Silent {
                    reports.push(("TimeError".to_owned(), Some(at)));
                }
                previous_at = at;
                previous_from = at / PER;
            }
            let anchor = rows.last().map_or(entered - BRACKET, |row| row.resumes_at * PER);
            Fixture { mode, period_us, origin: first / PER, held, starts, rows, reports, stop_tick: anchor + 30 * MS, last }
        }

        /// The capture of the cell: the control, a starved repeat's pre-stall part, and the one heard part.
        fn synth(&self) -> Synth {
            let mut synth = Synth::with_period(RATE, self.period_us, 0, CAPTURE_END);
            synth.play(CONTROL_AT + DELAY, (550 * 100 / self.period_us) as usize, 1.0, 0.0);
            if self.mode == Mode::Starved {
                synth.play(self.starts[0].at + DELAY, (499 * 100 / self.period_us) as usize, 1.0, 0.0);
            }
            if self.last == Last::Heard && !self.rows.is_empty() {
                synth.play(self.starts.last().unwrap().at + DELAY, (300 * 100 / self.period_us) as usize, 1.0, 0.0);
            }
            synth
        }

        fn evidence(&self, synth: &Synth, fake_ramp: bool) -> Evidence {
            Evidence {
                mode: self.mode,
                rate: RATE,
                n: c7::period(RATE, self.period_us) as i64,
                per: PER,
                tx_origin: 0,
                origin: self.origin,
                cap_samples: CAP,
                bracket: BRACKET,
                fine: true,
                device_error: false,
                rx_overflow: false,
                link_drops: 0,
                omitted: false,
                probe_complete: true,
                incomplete: Vec::new(),
                booking_admitted: true,
                rows: self.rows.clone(),
                row_checks: vec![Ok(()); self.rows.len()],
                stats_resumes: self.rows.len() as i64,
                stats_gap: self.rows.iter().map(|row| row.resumes_at - row.from).sum(),
                events: self.rows.iter().map(|row| Some(row.resumes_at)).collect(),
                holds: vec![self.held.clone()],
                expected_holds: 1,
                starts: self.starts.clone(),
                reports: self.reports.clone(),
                stop_tick: self.stop_tick,
                control_at: Some(CONTROL_AT),
                c0: 0,
                capture: synth.capture(fake_ramp),
                wave: Wave::new(&c7::wave(RATE, self.period_us)),
            }
        }
    }

    fn good(mode: Mode, m: usize, last: Last) -> (Evidence, Synth) {
        let fixture = Fixture::new(mode, m, last);
        let synth = fixture.synth();
        (fixture.evidence(&synth, false), synth)
    }

    /// The tick of resume row `index`.
    fn r_tick(e: &Evidence, index: usize) -> i64 {
        e.tx_origin + e.rows[index].resumes_at * e.per
    }

    fn assert_class(class: Class, expected: Class) {
        assert_eq!(class, expected, "{}", describe(&class));
    }

    const DROPPED: &str = "the last part was dropped and the Run ran >= 5 ms past it with no resume";

    #[test]
    fn c7_verdict_clean_late_start() {
        // The CLEAN cell, and the "good PN starting exactly at A + d stays CLEAN" fixture: the last part's PN begins at the final Start's A + d, so the
        // silence region's last examined window (ending at A + d) is silent. The Start is exactly at R, which StartedEarly must accept.
        let (e, _) = good(Mode::LateStart, 1, Last::Heard);
        assert_eq!(e.starts[1].at, r_tick(&e, 0), "the fixture's Start is exactly at R");
        assert_class(verdict(&e), Class::Clean);
    }

    #[test]
    fn c7_verdict_clean_starved() {
        let (e, _) = good(Mode::Starved, 1, Last::Heard);
        assert_class(verdict(&e), Class::Clean);
    }

    #[test]
    fn c7_verdict_m0_none_is_no_incident() {
        let (e, _) = good(Mode::LateStart, 0, Last::None);
        assert_class(verdict(&e), Class::Inconclusive(vec!["no incident".to_owned()]));
    }

    #[test]
    fn c7_verdict_late_start_m0_heard_is_no_incident() {
        // A late-start repeat whose first Start is heard: one incident, nothing after it.
        let fixture = Fixture::new(Mode::LateStart, 0, Last::Heard);
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY, 550, 1.0, 0.0);
        synth.play(fixture.starts[0].at + DELAY, 300, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Inconclusive(vec!["no incident".to_owned()]));
    }

    #[test]
    fn c7_verdict_starved_m0_silent_after_the_stall_is_dropped_never_resumed() {
        // A starved repeat with no resume whose first Start is heard: the device ran dry at the stall and nothing was heard after it.
        let (e, _) = good(Mode::Starved, 0, Last::Heard);
        assert_class(verdict(&e), Class::Fail(Fail::DroppedNeverResumed, DROPPED.to_owned()));
    }

    #[test]
    fn c7_verdict_starved_m0_short_tail_is_ended_before_a_resume() {
        let (mut e, _) = good(Mode::Starved, 0, Last::Heard);
        e.stop_tick = e.holds[0].returned + 2 * MS;
        assert_class(verdict(&e), Class::Inconclusive(vec!["ended before a resume".to_owned()]));
    }

    #[test]
    fn c7_verdict_starved_m0_rf_after_the_stall_is_no_incident() {
        // The capture plays PN after the held send's return: the device did not run dry, so there is no incident.
        let fixture = Fixture::new(Mode::Starved, 0, Last::Heard);
        let (span, guard) = (N * PER, c7_rf::SILENCE_GUARD_US * MCR / 1_000_000);
        let j = (fixture.held.returned + guard - STARVED_AT + span - 1).div_euclid(span);
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY, 550, 1.0, 0.0);
        synth.play(STARVED_AT + DELAY, 499, 1.0, 0.0);
        synth.play(STARVED_AT + DELAY + j * span, 2, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Inconclusive(vec!["no incident".to_owned()]));
    }

    #[test]
    fn c7_verdict_starved_m0_the_ring_down_inside_the_stall_is_not_rf_after_it() {
        // PN in the stall after the held call's entry (a ring-down), silent after its return: the device ran dry, so the cell is dropped.
        let fixture = Fixture::new(Mode::Starved, 0, Last::Heard);
        let (span, guard) = (N * PER, c7_rf::SILENCE_GUARD_US * MCR / 1_000_000);
        let j = (fixture.held.entered + guard - STARVED_AT + span - 1).div_euclid(span);
        let ring = STARVED_AT + DELAY + j * span;
        assert!(ring >= fixture.held.entered && ring < fixture.held.returned, "the ring-down is inside the stall: {ring}");
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY, 550, 1.0, 0.0);
        synth.play(STARVED_AT + DELAY, 499, 1.0, 0.0);
        synth.play(ring, 2, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Fail(Fail::DroppedNeverResumed, DROPPED.to_owned()));
    }

    #[test]
    fn c7_verdict_starved_m0_without_an_underflow_report_is_still_dropped() {
        // The Underflow report is data: without it, the RF after the stall still decides the cell.
        let (mut e, _) = good(Mode::Starved, 0, Last::Heard);
        e.reports.clear();
        assert_class(verdict(&e), Class::Fail(Fail::DroppedNeverResumed, DROPPED.to_owned()));
    }

    #[test]
    fn c7_verdict_m0_silent_dropped_never_resumed() {
        let (e, _) = good(Mode::LateStart, 0, Last::Silent);
        assert_class(verdict(&e), Class::Fail(Fail::DroppedNeverResumed, DROPPED.to_owned()));
    }

    #[test]
    fn c7_verdict_m0_silent_short_tail() {
        let (mut e, _) = good(Mode::LateStart, 0, Last::Silent);
        e.stop_tick = e.holds[0].at.unwrap() + 2 * MS;
        assert_class(verdict(&e), Class::Inconclusive(vec!["ended before a resume".to_owned()]));
    }

    #[test]
    fn c7_verdict_m0_silent_no_evidence() {
        let (mut e, _) = good(Mode::LateStart, 0, Last::Silent);
        e.reports.clear();
        assert_class(verdict(&e), Class::Inconclusive(vec!["no incident".to_owned()]));
    }

    #[test]
    fn c7_verdict_m1_none_short_tail() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::None);
        e.stop_tick = r_tick(&e, 0) + 2 * MS;
        assert_class(verdict(&e), Class::Inconclusive(vec!["ended before R + 5 ms".to_owned()]));
    }

    #[test]
    fn c7_verdict_m1_none_long_tail() {
        let (e, _) = good(Mode::LateStart, 1, Last::None);
        assert_class(verdict(&e), Class::Inconclusive(vec!["no start after resume".to_owned()]));
    }

    #[test]
    fn c7_verdict_a_silent_final_part_is_judged_on_its_ten_promised_periods() {
        // bt2 fix round 2, item 7: the capture ends exactly at the ten periods a silent first period promises after the final Start's A + d (`read_part` covers
        // no more for a silent part). The verdict is the silent part's, not a coverage error: the final Start is silent and the Run ran on with no resume.
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Silent);
        let span = c7::period(RATE, fixture.period_us) as i64 * PER;
        let end = fixture.starts.last().unwrap().at + DELAY + 10 * span;
        let mut synth = Synth::with_period(RATE, fixture.period_us, 0, end);
        synth.play(CONTROL_AT + DELAY, (550 * 100 / fixture.period_us) as usize, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Fail(Fail::DroppedNeverResumed, DROPPED.to_owned()));
    }

    #[test]
    fn c7_verdict_m1_silent_short_tail_is_unsettled() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Silent);
        e.stop_tick = r_tick(&e, 0) + 2 * MS;
        assert_class(verdict(&e), Class::Unsettled(1));
    }

    #[test]
    fn c7_verdict_m1_silent_long_tail_is_dropped_never_resumed() {
        let (e, _) = good(Mode::LateStart, 1, Last::Silent);
        assert_class(verdict(&e), Class::Fail(Fail::DroppedNeverResumed, DROPPED.to_owned()));
    }

    #[test]
    fn c7_verdict_m1_silent_old_reports_only_is_no_resume() {
        // Only the held start's own report, before the final Start: reports before it are not the final part's drop (Terra 3).
        let (mut e, _) = good(Mode::LateStart, 1, Last::Silent);
        e.reports = vec![("TimeError".to_owned(), e.holds[0].at)];
        assert_class(verdict(&e), Class::Fail(Fail::NoResume, format!("the repeat is silent after its last start {}", e.starts[1].at)));
    }

    #[test]
    fn c7_verdict_m1_silent_one_period_late_is_late_restart() {
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let final_at = fixture.starts.last().unwrap().at;
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY, 550, 1.0, 0.0);
        synth.play(final_at + DELAY + SPAN, 300, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Fail(Fail::LateRestart, "RF started 1 period(s) after the accepted start".to_owned()));
    }

    #[test]
    fn c7_verdict_m2_none_short_tail() {
        let (mut e, _) = good(Mode::LateStart, 2, Last::None);
        e.stop_tick = r_tick(&e, 1) + 2 * MS;
        assert_class(verdict(&e), Class::Unsettled(1));
    }

    #[test]
    fn c7_verdict_m2_none_long_tail() {
        let (e, _) = good(Mode::LateStart, 2, Last::None);
        assert_class(verdict(&e), Class::Inconclusive(vec!["no start after resume".to_owned()]));
    }

    #[test]
    fn c7_verdict_m4_none_short_tail() {
        let (mut e, _) = good(Mode::LateStart, 4, Last::None);
        e.stop_tick = r_tick(&e, 3) + 2 * MS;
        assert_class(verdict(&e), Class::Unsettled(3));
    }

    #[test]
    fn c7_verdict_m4_none_long_tail() {
        let (e, _) = good(Mode::LateStart, 4, Last::None);
        assert_class(verdict(&e), Class::Inconclusive(vec!["no start after resume".to_owned()]));
    }

    #[test]
    fn c7_verdict_m2_heard_is_retried() {
        let (e, _) = good(Mode::LateStart, 2, Last::Heard);
        assert_class(verdict(&e), Class::Retried(1));
    }

    #[test]
    fn c7_verdict_m4_heard_is_retried() {
        let (e, _) = good(Mode::LateStart, 4, Last::Heard);
        assert_class(verdict(&e), Class::Retried(3));
    }

    #[test]
    fn c7_verdict_a_final_start_5_ms_before_the_stop_has_not_played_its_20_periods() {
        // A 1 ms PN: a heard part is read from its first period and 19 steady ones (20 periods, 20 ms), but the Run stops 5 ms after the final Start.
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let final_at = fixture.starts.last().unwrap().at;
        let mut synth = Synth::with_period(RATE, c7::HW_PERIOD_US, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY, 30, 1.0, 0.0);
        synth.play(final_at + DELAY, 30, 1.0, 0.0);
        let mut e = fixture.evidence(&synth, false);
        e.n = c7::period(RATE, c7::HW_PERIOD_US) as i64;
        e.wave = Wave::new(&c7::wave(RATE, c7::HW_PERIOD_US));
        e.stop_tick = final_at + 5 * MS;
        assert_class(verdict(&e), Class::Inconclusive(vec!["the Run stopped before the final part's 20 periods".to_owned()]));
    }

    #[test]
    fn c7_verdict_m2_silent_is_unsettled() {
        let (mut e, _) = good(Mode::LateStart, 2, Last::Silent);
        e.stop_tick = r_tick(&e, 1) + 2 * MS;
        assert_class(verdict(&e), Class::Unsettled(2));
    }

    // ---------------------------------------------------------------- the INCONCLUSIVE reasons, each otherwise CLEAN

    #[test]
    fn c7_verdict_inconclusive_not_fine() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.fine = false;
        assert_class(verdict(&e), Class::Inconclusive(vec!["the Run did not end Fine".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_device_error() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.device_error = true;
        assert_class(verdict(&e), Class::Inconclusive(vec!["a device error row".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_rx_overflow() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.rx_overflow = true;
        assert_class(verdict(&e), Class::Inconclusive(vec!["RX_OVERFLOW".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_the_capture_link_dropped_blocks() {
        // bt2 fix round 2, item 4(b): a link drop is its own reason, and it comes before the hole it leaves, which the readers would report as the hole.
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.link_drops = 3;
        assert_class(verdict(&e), Class::Inconclusive(vec!["the capture link dropped 3 blocks".to_owned()]));
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let mut synth = fixture.synth();
        let final_at = fixture.starts.last().unwrap().at;
        synth.hole(final_at + DELAY + 10 * SPAN, final_at + DELAY + 10 * SPAN + PER);
        let mut e = fixture.evidence(&synth, false);
        e.link_drops = 3;
        assert_class(verdict(&e), Class::Inconclusive(vec!["the capture link dropped 3 blocks".to_owned(), "hole in the coverage".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_omitted_rows() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.omitted = true;
        assert_class(verdict(&e), Class::Inconclusive(vec!["rows omitted or the probe is incomplete".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_probe_incomplete() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.probe_complete = false;
        assert_class(verdict(&e), Class::Inconclusive(vec!["rows omitted or the probe is incomplete".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_incomplete_text() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.incomplete = vec!["the control's close is not in the probe".to_owned()];
        assert_class(verdict(&e), Class::Inconclusive(vec!["the control's close is not in the probe".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_booking_refused() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.booking_admitted = false;
        assert_class(verdict(&e), Class::Inconclusive(vec!["booking refused".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_ambiguous_check() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.row_checks[0] = Err("ambiguous: resume row 0 has no close number 3 in the probe (2 accepted closes)".to_owned());
        assert_class(verdict(&e), Class::Inconclusive(vec!["ambiguous: resume row 0 has no close number 3 in the probe (2 accepted closes)".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_control_missing() {
        // No control in the capture: the lag is unresolved against the onset.
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(fixture.starts.last().unwrap().at + DELAY, 300, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Inconclusive(vec!["the loop delay is not resolved against the control's onset".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_control_not_separable() {
        // The control's first period is a DC offset (power in the onset test, no correlation with the zero-mean PN): its gain is under 4 t.
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.dc(CONTROL_AT + DELAY, CONTROL_AT + DELAY + SPAN, 0.5);
        synth.play(CONTROL_AT + DELAY + SPAN, 549, 1.0, 0.0);
        synth.play(fixture.starts.last().unwrap().at + DELAY, 300, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Inconclusive(vec!["the control is not separable from the floor".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_loop_delay_unresolved() {
        // The control's true delay is d + n samples: the PN's periodicity ties the peak at d, a whole period from the onset.
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY + SPAN, 550, 1.0, 0.0);
        synth.play(fixture.starts.last().unwrap().at + DELAY, 300, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Inconclusive(vec!["the loop delay is not resolved against the control's onset".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_no_hold() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.holds.clear();
        assert_class(verdict(&e), Class::Inconclusive(vec!["the hold did not fire as the cell says".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_two_holds() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        let held = e.holds[0].clone();
        e.holds.push(held);
        assert_class(verdict(&e), Class::Inconclusive(vec!["the hold did not fire as the cell says".to_owned()]));
    }

    #[test]
    fn c7_verdict_two_expected_holds_fire_as_the_cell_says() {
        // S4': the resumed part's own Start is held too, so the cell arms two holds.
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        let held = e.holds[0].clone();
        e.holds.push(held);
        e.expected_holds = 2;
        assert_class(verdict(&e), Class::Clean);
        e.expected_holds = 1;
        assert_class(verdict(&e), Class::Inconclusive(vec!["the hold did not fire as the cell says".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_wrong_hold_class() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.holds[0].class = "continuation".to_owned();
        assert_class(verdict(&e), Class::Inconclusive(vec!["the hold did not fire as the cell says".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_held_start_returned_within_the_bracket() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.holds[0].returned = e.holds[0].at.unwrap() + e.bracket;
        assert_class(verdict(&e), Class::Inconclusive(vec!["the held start was not shown late (returned within the bracket of its time)".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_first_cause_late_start() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.rows[0].cause = "underflow".to_owned();
        assert_class(verdict(&e), Class::Inconclusive(vec!["the first resume's cause is not a late start's".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_first_cause_starvation() {
        let (mut e, _) = good(Mode::Starved, 1, Last::Heard);
        e.rows[0].cause = "late_at_device".to_owned();
        assert_class(verdict(&e), Class::Inconclusive(vec!["the first resume's cause is not a starvation's".to_owned()]));
    }

    #[test]
    fn c7_verdict_an_expired_first_incident_needs_no_timeerror_report() {
        // The Module's `expired` row plus the host stamps that the held send returned after its time are the late-start evidence; the device's report may be lost.
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.rows[0].cause = "expired".to_owned();
        e.reports.clear();
        assert_class(verdict(&e), Class::Clean);
    }

    #[test]
    fn c7_verdict_inconclusive_no_rf_before_the_stall() {
        // The control and the part after R, but no RF in the last 100 ms before the stall: the starvation's evidence is missing.
        let fixture = Fixture::new(Mode::Starved, 1, Last::Heard);
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY, 550, 1.0, 0.0);
        synth.play(fixture.starts.last().unwrap().at + DELAY, 300, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Inconclusive(vec!["no RF before the stall".to_owned()]));
    }

    #[test]
    fn c7_verdict_underflow_absent_with_rf_is_clean() {
        // The Underflow report is data: with the RF before the stall present, its absence changes nothing.
        let (mut e, _) = good(Mode::Starved, 1, Last::Heard);
        e.reports.clear();
        assert_class(verdict(&e), Class::Clean);
    }

    #[test]
    fn c7_verdict_inconclusive_hole_in_the_coverage() {
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let mut synth = fixture.synth();
        let final_at = fixture.starts.last().unwrap().at;
        synth.hole(final_at + DELAY + 10 * SPAN, final_at + DELAY + 10 * SPAN + PER);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Inconclusive(vec!["hole in the coverage".to_owned()]));
    }

    #[test]
    fn c7_verdict_inconclusive_unpaired_start() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        let at = e.starts[1].at + SPAN;
        e.starts.push(StartRow { at, accepted: CAP, returned: at - 2_000_000 });
        assert_class(verdict(&e), Class::Inconclusive(vec!["unpaired start".to_owned()]));
    }

    // ---------------------------------------------------------------- the named failures, each otherwise CLEAN

    #[test]
    fn c7_verdict_fail_counters() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.stats_resumes += 1;
        assert_class(verdict(&e), Class::Fail(Fail::Counters, "counters disagree with rows".to_owned()));
    }

    #[test]
    fn c7_verdict_fail_event_without_row() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.events.push(Some(9_999));
        assert_class(verdict(&e), Class::Fail(Fail::EventWithoutRow, "event plan 9999 is no row's resume point".to_owned()));
    }

    #[test]
    fn c7_verdict_a_row_without_an_event_is_clean() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.events.clear();
        assert_class(verdict(&e), Class::Clean);
    }

    #[test]
    fn c7_verdict_fail_resume_off_rule() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.row_checks[0] = Err("R 170100 is not the rule's allowed lattice points 170000..=170300".to_owned());
        assert_class(verdict(&e), Class::Fail(Fail::ResumeOffRule, "R 170100 is not the rule's allowed lattice points 170000..=170300".to_owned()));
    }

    #[test]
    fn c7_verdict_fail_lattice_off_a_row() {
        // One row one sample off its lattice point, with the counters and the event that name it moved with it.
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.rows[0].resumes_at += 1;
        e.stats_gap += 1;
        e.events[0] = Some(e.rows[0].resumes_at);
        let tick = e.tx_origin + e.rows[0].resumes_at * e.per;
        assert_class(verdict(&e), Class::Fail(Fail::LatticeOff, format!("R {tick} is off the lattice")));
    }

    #[test]
    fn c7_verdict_fail_lattice_off_a_start() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.starts[1].at += PER;
        let tick = e.starts[1].at;
        assert_class(verdict(&e), Class::Fail(Fail::LatticeOff, format!("R {tick} is off the lattice")));
    }

    #[test]
    fn c7_verdict_fail_started_early() {
        let (mut e, _) = good(Mode::LateStart, 1, Last::Heard);
        e.starts[1].at -= SPAN;
        let (at, r) = (e.starts[1].at, r_tick(&e, 0));
        assert_class(verdict(&e), Class::Fail(Fail::StartedEarly, format!("a start of burst at {at} before its resume point {r}")));
    }

    #[test]
    fn c7_verdict_fail_silence_violated() {
        // Two periods of PN leakage in the gap, on a silence window's own grid.
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let mut synth = fixture.synth();
        let leak = fixture.held.at.unwrap() + DELAY + 2 * SPAN;
        synth.play(leak, 2, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        assert_class(verdict(&e), Class::Fail(Fail::SilenceViolated, format!("heard {leak} inside the gap [{}, {})", fixture.held.at.unwrap(), e.starts[1].at)));
    }

    #[test]
    fn c7_the_verdict_fails_off_lattice_pn_radiated_in_the_gap_at_the_hardware_period() {
        // F4a: the PN radiated for 3 periods a quarter period off the repeat's lattice, 5 periods into the gap, then a correct resume: the correlation on
        // the lattice misses it and the power test hears it. The same fixture without the leak is the control that the 1 ms schedule is consistent.
        let fixture = Fixture::new_at(Mode::LateStart, 1, Last::Heard, c7::HW_PERIOD_US);
        let clean = fixture.evidence(&fixture.synth(), false);
        assert_class(verdict(&clean), Class::Clean);
        let span = c7::period(RATE, c7::HW_PERIOD_US) as i64 * PER;
        let leak = fixture.held.at.unwrap() + DELAY + 5 * span + span / 4;
        let mut synth = fixture.synth();
        synth.play(leak, 3, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        match verdict(&e) {
            Class::Fail(Fail::SilenceViolated, text) => assert!(text.starts_with("RF power "), "{text}"),
            other => panic!("{}", describe(&other)),
        }
    }

    #[test]
    fn c7_the_verdict_fails_an_intermediate_part_that_radiated_before_an_unsettled_end() {
        // F4b: the final Start was dropped again and the Run stopped 3 ms later (UNSETTLED(1)); the part it dropped radiated 5 periods on the lattice from the
        // first resumed part's start, in a gap the heard column never scans.
        let fixture = Fixture::new_at(Mode::LateStart, 2, Last::None, c7::HW_PERIOD_US);
        let mut baseline = fixture.evidence(&fixture.synth(), false);
        baseline.stop_tick = r_tick(&baseline, 1) + 3 * MS;
        assert_class(verdict(&baseline), Class::Unsettled(1));
        let leak = fixture.starts[1].at + DELAY;
        let mut synth = fixture.synth();
        synth.play(leak, 5, 1.0, 0.0);
        let mut e = fixture.evidence(&synth, false);
        e.stop_tick = r_tick(&e, 1) + 3 * MS;
        match verdict(&e) {
            Class::Fail(Fail::SilenceViolated, text) => {
                assert_eq!(text, format!("heard {leak} inside the gap [{}, {})", fixture.held.at.unwrap(), e.stop_tick), "{text}")
            }
            other => panic!("{}", describe(&other)),
        }
    }

    #[test]
    fn c7_the_verdict_fails_a_gap_leak_before_a_silent_dropped_again_final_start() {
        // F4c: the final Start is silent, was dropped again and the Run stopped 3 ms later (UNSETTLED(1)); 3 periods a quarter period off the lattice,
        // 5 periods after the held start's first loopback sample.
        let fixture = Fixture::new_at(Mode::LateStart, 1, Last::Silent, c7::HW_PERIOD_US);
        let mut baseline = fixture.evidence(&fixture.synth(), false);
        baseline.stop_tick = r_tick(&baseline, 0) + 3 * MS;
        assert_class(verdict(&baseline), Class::Unsettled(1));
        let span = c7::period(RATE, c7::HW_PERIOD_US) as i64 * PER;
        let leak = fixture.held.at.unwrap() + DELAY + 5 * span + span / 4;
        let mut synth = fixture.synth();
        synth.play(leak, 3, 1.0, 0.0);
        let mut e = fixture.evidence(&synth, false);
        e.stop_tick = r_tick(&e, 0) + 3 * MS;
        match verdict(&e) {
            Class::Fail(Fail::SilenceViolated, text) => assert!(text.starts_with("RF power "), "{text}"),
            other => panic!("{}", describe(&other)),
        }
    }

    #[test]
    fn c7_the_verdict_scans_no_legitimate_exit() {
        // F4d: the scans do not eat the legitimate exits. The existing LateRestart fixture's shape (`c7_verdict_m1_silent_one_period_late_is_late_restart`) at
        // the hardware period: a silent final Start with no drop report after it, its PN 3 periods late and the Run running 30 ms past the Start.
        let fixture = Fixture::new_at(Mode::LateStart, 1, Last::Silent, c7::HW_PERIOD_US);
        let span = c7::period(RATE, c7::HW_PERIOD_US) as i64 * PER;
        let final_at = fixture.starts.last().unwrap().at;
        let mut synth = Synth::with_period(RATE, c7::HW_PERIOD_US, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY, 30, 1.0, 0.0);
        synth.play(final_at + DELAY + 3 * span, 30, 1.0, 0.0);
        let mut e = fixture.evidence(&synth, false);
        e.reports = vec![("TimeError".to_owned(), fixture.held.at)];
        assert_class(verdict(&e), Class::Fail(Fail::LateRestart, "RF started 3 period(s) after the accepted start".to_owned()));
    }

    #[test]
    fn c7_verdict_the_fake_ramp_with_a_wrap_in_the_gap_is_clean() {
        // The fake's idle ramp with its wrap inside the gap, baseline on: silence, so CLEAN.
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let (from, end) = (fixture.held.at.unwrap(), fixture.starts[1].at + DELAY);
        let wrap = 2 * 65_536 * PER;
        assert!(from < wrap && wrap < end, "the ramp wraps inside the gap: {from} < {wrap} < {end}");
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.ramp(0, CAPTURE_END);
        synth.play_over_ramp(CONTROL_AT + DELAY, 550, 1.0, 0.0);
        synth.play_over_ramp(fixture.starts[1].at + DELAY, 300, 1.0, 0.0);
        let e = fixture.evidence(&synth, true);
        assert_class(verdict(&e), Class::Clean);
    }

    #[test]
    fn c7_verdict_fail_phase_discontinuous_three_samples_off() {
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let final_at = fixture.starts.last().unwrap().at;
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY, 550, 1.0, 0.0);
        synth.play(final_at + DELAY + 3 * PER, 300, 1.0, 0.0);
        let e = fixture.evidence(&synth, false);
        match verdict(&e) {
            Class::Fail(Fail::PhaseDiscontinuous, text) => assert!(text.starts_with("the waveform at R is not the lattice's (gain 0.0"), "{text}"),
            other => panic!("{}", describe(&other)),
        }
    }

    #[test]
    fn c7_verdict_fail_phase_discontinuous_forty_degrees_off() {
        let fixture = Fixture::new(Mode::LateStart, 1, Last::Heard);
        let final_at = fixture.starts.last().unwrap().at;
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY, 550, 1.0, 0.0);
        synth.play(final_at + DELAY, 300, 1.0, 40.0);
        let e = fixture.evidence(&synth, false);
        match verdict(&e) {
            Class::Fail(Fail::PhaseDiscontinuous, text) => {
                assert!(text.starts_with("the waveform at R is not the lattice's (gain 1.000, phase 40.0"), "{text}")
            }
            other => panic!("{}", describe(&other)),
        }
    }

    // ---------------------------------------------------------------- the printed line and the retry record

    #[test]
    fn c7_verdict_every_class_has_a_line() {
        assert_eq!(describe(&Class::Clean), "CLEAN");
        assert_eq!(describe(&Class::Retried(2)), "RETRIED(2): the repeat resumed after 2 retries; the last part is phase-continuous on the lattice (a datum, not a failure)");
        assert_eq!(describe(&Class::Unsettled(3)), "UNSETTLED(3): 3 dropped resumed part(s), nothing heard from a part by the end of the trial (a datum, not a failure)");
        assert_eq!(describe(&Class::Inconclusive(vec!["the Run did not end Fine".to_owned(), "booking refused".to_owned()])), "INCONCLUSIVE: the Run did not end Fine; booking refused");
        for (fail, text) in [
            (Fail::Counters, "counters disagree with rows"),
            (Fail::EventWithoutRow, "event plan 9999 is no row's resume point"),
            (Fail::ResumeOffRule, "R 170100 is not the rule's allowed lattice points 170000..=170300"),
            (Fail::LatticeOff, "R 34020200 is off the lattice"),
            (Fail::StartedEarly, "a start of burst at 34000000 before its resume point 34020000"),
            (Fail::SilenceViolated, "heard 26048800 inside the gap [26000000, 34020000)"),
            (Fail::PhaseDiscontinuous, "the waveform at R is not the lattice's (gain 0.059, phase 0.0 vs control)"),
            (Fail::LateRestart, "RF started 1 period(s) after the accepted start"),
            (Fail::NoResume, "the repeat is silent after its last start 34020000"),
            (Fail::DroppedNeverResumed, DROPPED),
        ] {
            assert_eq!(describe(&Class::Fail(fail, text.to_owned())), format!("FAIL: {text}"));
        }
    }

    #[test]
    fn c7_verdict_ignores_the_retry_record() {
        // The retry record is a datum: the hold's length, a later return, and the earlier drops' reports (all before the final Start) change no class.
        let (e, _) = good(Mode::LateStart, 2, Last::Heard);
        let mut other = e.clone();
        other.holds[0].held_ns = 7;
        other.holds[0].returned = other.holds[0].at.unwrap() + 100 * e.bracket;
        other.reports = vec![("TimeError".to_owned(), Some(e.starts[1].at)), ("TimeError".to_owned(), Some(e.starts[1].at + PER))];
        assert_class(verdict(&e), Class::Retried(1));
        assert_class(verdict(&other), Class::Retried(1));
    }
}
