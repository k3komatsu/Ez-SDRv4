//! bt2's drop storm (cell C7, S8): a repeat whose every part is handed at its resume point to a device that is already past it, so that the device drops it again and
//! the repeat keeps resuming. The storm is read from the Module's bounded records (the counters are exact, the kept rows are the bound's) and the probe's rows; the
//! capture decides only that nothing was radiated after the first drop.
//!
//! A storm that did not reach the bound is INCONCLUSIVE, not a failure: it did not exercise what it is for. CLEAN is the bound kept exactly: the kept resume rows,
//! the omitted count, the counters, no held close, silence after the first drop and the events naming the rows they can.

#![allow(dead_code)]

use ezsdr_kernel::session::Outcome;

use super::c7::{self, Storm};
use super::c7_rf::{self, Capture, Wave};
use super::c7_verdict::{self, Class, Fail};
use super::*;

/// The Module's default per-kind record bound, restated here so that a change of the Module's constant fails a test instead of being followed; the Manifest's own
/// declared `rows_per_kind` is checked AGAINST it.
pub const DEFAULT_BOUND: usize = 1_000;

/// Everything the storm's verdict reads, taken from one finished storm.
pub struct StormEvidence {
    pub fine: bool,
    pub incomplete: Vec<String>,
    pub probe_complete: bool,
    pub booking_admitted: bool,
    /// `common::omitted_rows`: the Provider's bounded sections, one `<section>.<kind>: <count>` line each.
    pub omitted: Vec<String>,
    /// The `tx_resume` rows the `timing` section kept (read with `section_unchecked`: the bound is the subject).
    pub rows_kept: usize,
    /// `record_bound.omitted.timing.tx_resume`, 0 when absent.
    pub omitted_resumes: i64,
    pub stats_resumes: i64,
    /// The blocks the capture's data link dropped (`c7_verdict::link_drops`).
    pub link_drops: u64,
    /// `Common::check` for each kept row, in the Manifest's order (the same `expected_r` and `r_check` as a trial row, over the sends of the row's own incident); an
    /// `ambiguous` text is a row whose own incident lacks the data the rule needs.
    pub row_checks: Vec<Result<(), String>>,
    /// The Module's per-kind bound this Run recorded (`record_bound.rows_per_kind`).
    pub bound: usize,
    /// Any probe `tx_send` of class close with `held_ns > 0`.
    pub close_held: bool,
    /// The first held send's `at`: the first drop.
    pub held_first_at: Option<i64>,
    /// The kept rows' `resumes_at`.
    pub row_plans: Vec<i64>,
    /// The accepted payload Starts after the control (`c7::start_ticks`): index 0 is the held first Start.
    pub start_ticks: Vec<i64>,
    /// Each kept row's resume point as a tick: `tx_origin + resumes_at * per`, in the Manifest's order.
    pub row_ticks: Vec<i64>,
    /// The `resumes_at` of every delivered TIME_ERROR/TX_UNDERFLOW event that has a plan, whole Run.
    pub events: Vec<i64>,
    /// The repeat's lattice origin, a transmit sample's tick (`tx_origin + start * per`).
    pub lattice_tick: i64,
    /// The probe's rows the Run kept (`storm.probe.len()`), against the probe's 65 536-row record.
    pub probe_rows: usize,
    /// `ProbeDevice::dropped_rows` after the Run: rows the probe's record could not keep.
    pub probe_dropped: usize,
    pub control_at: Option<i64>,
    pub c0: i64,
    pub stop_tick: i64,
    /// The samples the capture was booked for: `sink.capture_samples`.
    pub scheduled_samples: i64,
    pub capture: Capture,
    pub wave: Wave,
    pub fake: bool,
}

/// The storm's class, by the specification's order: the INCONCLUSIVE reasons first, then the named failures, then CLEAN.
pub fn storm_verdict(e: &StormEvidence) -> Class {
    // 1. The preconditions the later checks rest on: every reason that applies, in the specification's order.
    let mut reasons: Vec<String> = Vec::new();
    if !e.fine {
        reasons.push("the Run did not end Fine".to_owned());
    }
    if e.link_drops > 0 {
        reasons.push(format!("the capture link dropped {} blocks", e.link_drops));
    }
    reasons.extend(e.incomplete.iter().cloned());
    if !e.probe_complete {
        reasons.push("the probe is incomplete".to_owned());
    }
    if !e.booking_admitted {
        reasons.push("booking refused".to_owned());
    }
    let control = match e.control_at {
        Some(at) => match c7_rf::analyse_control(&e.capture, &e.wave, e.c0, at) {
            Ok(control) => Some(control),
            Err(text) => {
                reasons.push(text);
                None
            }
        },
        None => {
            reasons.push("no control start in the probe".to_owned());
            None
        }
    };
    // The capture is one continuous run from its own first sample to the scheduled end: a hole anywhere, also between the control and the held Start, is
    // INCONCLUSIVE (an absent capture is already a reason of its own).
    let scheduled_end = e.capture.start() + (e.scheduled_samples - 1) * e.capture.per;
    if e.capture.end() > 0 && !e.capture.covers(e.capture.start(), scheduled_end) {
        if e.capture.end() < scheduled_end {
            reasons.push(format!("the capture is shorter than the scheduled {} samples", e.scheduled_samples));
        } else {
            reasons.push("hole in the capture".to_owned());
        }
    }
    let ambiguous: Vec<&String> = e.row_checks.iter().filter_map(|check| check.as_ref().err()).filter(|text| text.starts_with("ambiguous")).collect();
    if let Some(first) = ambiguous.first() {
        reasons.push(format!("{} kept row(s) lack the data the R rule needs; the first: {first}", ambiguous.len()));
    }
    if e.stats_resumes <= DEFAULT_BOUND as i64 {
        reasons.push("the storm did not reach the bound".to_owned());
    }
    if e.held_first_at.is_none() {
        reasons.push("no held start in the probe".to_owned());
    }
    if !reasons.is_empty() {
        return Class::Inconclusive(reasons);
    }

    // 2. The named failures, in the specification's order.
    if e.bound != DEFAULT_BOUND || e.rows_kept != DEFAULT_BOUND || e.omitted_resumes != e.stats_resumes - DEFAULT_BOUND as i64 {
        return Class::Fail(
            Fail::BoundWrong,
            format!("a declared bound of {} (expected {}), {} kept resume rows and {} omitted, for {} resumes", e.bound, DEFAULT_BOUND, e.rows_kept, e.omitted_resumes, e.stats_resumes),
        );
    }
    if let Err(text) = kept_rows_are_the_first(&e.row_ticks, &e.start_ticks) {
        return Class::Fail(Fail::BoundWrong, text);
    }
    if let Some((index, text)) = e.row_checks.iter().enumerate().find_map(|(index, check)| check.as_ref().err().map(|text| (index, text))) {
        return Class::Fail(Fail::ResumeOffRule, format!("kept row {index}: {text}"));
    }
    if e.close_held {
        return Class::Fail(Fail::CloseHeld, "a close send was held".to_owned());
    }
    let (control, from) = (control.expect("a control was analysed above"), e.held_first_at.expect("a held start was found above"));
    match c7_rf::silence(&e.capture, &e.wave, &control, from, scheduled_end, e.lattice_tick) {
        Ok(Some(tick)) => return Class::Fail(Fail::StormRadiated, format!("heard RF at {tick} after the first drop at {from}")),
        Ok(None) => {}
        Err(text) => return Class::Inconclusive(vec![text]),
    }
    match c7_rf::quiet(&e.capture, &control, from, scheduled_end) {
        Ok(Some((tick, power))) => return Class::Fail(Fail::StormRadiated, format!("RF power {power:.3e} (threshold {:.3e}) at {tick} after the first drop at {from}", control.tp)),
        Ok(None) => {}
        Err(text) => return Class::Inconclusive(vec![text]),
    }
    if let Some(last) = e.row_plans.iter().copied().max() {
        if let Some(event) = e.events.iter().find(|event| **event <= last && !e.row_plans.contains(event)) {
            return Class::Fail(Fail::EventsOutsideRows, format!("event plan {event} is no kept row's resume point (the kept rows end at {last})"));
        }
    }
    if e.events.len() > e.stats_resumes as usize {
        return Class::Fail(Fail::EventsExceedResumes, format!("{} resume events for {} resumes", e.events.len(), e.stats_resumes));
    }
    if e.fake && e.events.len() + 1 < e.stats_resumes as usize {
        return Class::Fail(Fail::EventsMissing, format!("{} resume events for {} resumes on the fake (one per drop, but the first may be `expired`)", e.events.len(), e.stats_resumes));
    }
    Class::Clean
}

/// Whether the rows the Manifest kept are the first rows: kept row `i` (0-based) pairs with the accepted Start number `i + 1` (index 0 is the held first Start).
/// The rows are the first ones when row `i`'s resume point is at or before that Start and, when row `i + 1` is kept, that Start is strictly before row `i + 1`.
/// `Err` names the row and the two ticks.
pub fn kept_rows_are_the_first(row_ticks: &[i64], start_ticks: &[i64]) -> Result<(), String> {
    for (i, &row) in row_ticks.iter().enumerate() {
        let Some(&start) = start_ticks.get(i + 1) else {
            return Err(format!("kept row {i} has no paired Start"));
        };
        if row > start {
            return Err(format!("kept row {i} at {row} is after its paired Start at {start}"));
        }
        if let Some(&next) = row_ticks.get(i + 1) {
            if start >= next {
                return Err(format!("kept row {} at {next} is before the Start at {start}", i + 1));
            }
        }
    }
    Ok(())
}

/// The evidence of a finished storm. `fake` is true on the `FakeDevice`, whose idle sample is its ramp.
pub fn storm_evidence(storm: &Storm, fake: bool) -> StormEvidence {
    let manifest = &storm.manifest;
    let missing = c7_verdict::missing_evidence(manifest, &["timing", "stats", "record_bound"]);
    let record_bound = manifest.section(&usrp(), "ezsdr.radio.uhd.record_bound").cloned().unwrap_or_default();
    let timing = manifest.section(&usrp(), "ezsdr.radio.uhd.timing").cloned().unwrap_or_default();
    let stats = manifest.section(&usrp(), "ezsdr.radio.uhd.stats").cloned().unwrap_or_default();
    let rows_kept = timing.as_array().map_or(0, |rows| rows.iter().filter(|row| row["what"] == "tx_resume").count());
    let omitted_resumes = record_bound["omitted"]["timing"]["tx_resume"].as_i64().unwrap_or(0);
    let stats_resumes = stats["tx_resumes"].as_i64().unwrap_or(0);
    let bound = record_bound["rows_per_kind"].as_u64().unwrap_or(0) as usize;
    let close_held = storm.probe.iter().any(|row| row["call"] == "tx_send" && row["class"] == "close" && row["held_ns"].as_u64().is_some_and(|held| held > 0));
    let held_first_at = storm.probe.iter().find(|row| row["call"] == "tx_send" && row["held_ns"].as_u64().is_some_and(|held| held > 0)).and_then(|row| row["at"].as_i64());
    let resumes = storm.resumes();
    let row_plans = resumes.iter().map(|row| row.resumes_at).collect();
    let row_checks = storm.checks(&resumes, c7::DEFAULT_MARGIN_NS);
    let rate = MCR as f64 / storm.per as f64;
    let mut incomplete = storm.incomplete.clone();
    incomplete.extend(missing);
    StormEvidence {
        fine: failure(manifest).is_empty(),
        incomplete,
        probe_complete: storm.rows_complete.is_ok(),
        booking_admitted: storm.booking.as_ref().is_some_and(|entry| matches!(entry.outcome, Outcome::Admitted { .. })),
        omitted: omitted_rows(manifest),
        rows_kept,
        omitted_resumes,
        stats_resumes,
        link_drops: c7_verdict::link_drops(manifest),
        row_checks,
        bound,
        close_held,
        held_first_at,
        row_plans,
        start_ticks: c7::start_ticks(&storm.probe, storm.control_starts),
        row_ticks: resumes.iter().map(|row| storm.tx_origin + row.resumes_at * storm.per).collect(),
        events: storm.event_resume_points(),
        lattice_tick: storm.tx_origin + storm.origin() * storm.per,
        probe_rows: storm.probe.len(),
        probe_dropped: storm.probe_dropped,
        control_at: storm.control,
        c0: storm.c0,
        stop_tick: storm.stop_tick,
        scheduled_samples: (rate * storm.capture_ms as f64 / 1_000.0) as i64,
        capture: if c7_verdict::capture_missing(manifest) {
            Capture::new(Vec::new(), storm.per, storm.period as usize, fake)
        } else {
            Capture::new(n47s5::captured_samples(manifest), storm.per, storm.period as usize, fake)
        },
        wave: Wave::new(&c7::wave(rate, storm.period_us)),
        fake,
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
    const DELAY: i64 = 44 * PER;
    const CONTROL_AT: i64 = 25 * MS;
    const CAPTURE_END: i64 = 600 * MS;
    const FIRST: i64 = 130 * MS;
    const BOUND: usize = DEFAULT_BOUND;
    const RESUMES: i64 = 1_006;

    /// The control repeat only; nothing after `FIRST` is heard.
    fn base() -> Synth {
        base_at(c7::FAKE_PERIOD_US)
    }

    /// The control repeat only at a PN of `period_us`; nothing after `FIRST` is heard.
    fn base_at(period_us: i64) -> Synth {
        let mut synth = Synth::with_period(RATE, period_us, 0, CAPTURE_END);
        synth.play(CONTROL_AT + DELAY, (550 * 100 / period_us) as usize, 1.0, 0.0);
        synth
    }

    /// A storm's evidence: `resumes` drops at `FIRST + j * SPAN`, `bound` rows kept, every drop named by an event.
    fn evidence(synth: &Synth, resumes: i64, bound: usize, fake: bool) -> StormEvidence {
        evidence_at(synth, resumes, bound, fake, c7::FAKE_PERIOD_US)
    }

    /// The storm's evidence at a PN of `period_us`: `resumes` drops at `FIRST + j * span`, `bound` rows kept, every drop named by an event.
    fn evidence_at(synth: &Synth, resumes: i64, bound: usize, fake: bool, period_us: i64) -> StormEvidence {
        let span = c7::period(RATE, period_us) as i64 * PER;
        let plans: Vec<i64> = (1..=resumes).map(|j| (FIRST + j * span) / PER).collect();
        StormEvidence {
            fine: true,
            incomplete: Vec::new(),
            probe_complete: true,
            booking_admitted: true,
            omitted: Vec::new(),
            rows_kept: bound,
            omitted_resumes: resumes - bound as i64,
            stats_resumes: resumes,
            link_drops: 0,
            row_checks: vec![Ok(()); bound],
            bound,
            close_held: false,
            held_first_at: Some(FIRST),
            row_plans: plans.iter().copied().take(bound).collect(),
            start_ticks: (0..=resumes).map(|j| FIRST + j * span).collect(),
            row_ticks: plans.iter().take(bound).map(|plan| plan * PER).collect(),
            events: plans,
            lattice_tick: FIRST,
            probe_rows: 4_000,
            probe_dropped: 0,
            control_at: Some(CONTROL_AT),
            c0: 0,
            stop_tick: CAPTURE_END,
            scheduled_samples: CAPTURE_END / PER,
            capture: synth.capture(fake),
            wave: Wave::new(&c7::wave(RATE, period_us)),
            fake,
        }
    }

    fn good() -> StormEvidence {
        evidence(&base(), RESUMES, BOUND, false)
    }

    fn assert_class(class: Class, expected: Class) {
        assert_eq!(class, expected, "{}", c7_verdict::describe(&class));
    }

    #[test]
    fn c7_storm_clean() {
        assert_class(storm_verdict(&good()), Class::Clean);
    }

    // ---------------------------------------------------------------- the INCONCLUSIVE reasons, each otherwise CLEAN

    #[test]
    fn c7_storm_inconclusive_not_fine() {
        let mut e = good();
        e.fine = false;
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["the Run did not end Fine".to_owned()]));
    }

    #[test]
    fn c7_storm_inconclusive_incomplete_text() {
        let mut e = good();
        e.incomplete = vec!["the control's close is not in the probe".to_owned()];
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["the control's close is not in the probe".to_owned()]));
    }

    #[test]
    fn c7_storm_missing_evidence_is_inconclusive_not_a_panic() {
        // No capture and no record bound: the evidence is the reasons, and the verdict returns them without touching a missing part.
        let mut e = good();
        e.incomplete = vec!["no capture artifact `rec` in the Manifest".to_owned(), "no `record_bound` section in the Manifest".to_owned()];
        e.capture = c7_rf::Capture::new(Vec::new(), PER, N as usize, false);
        e.bound = 0;
        e.stats_resumes = 0;
        match storm_verdict(&e) {
            Class::Inconclusive(reasons) => {
                assert!(reasons.contains(&"no capture artifact `rec` in the Manifest".to_owned()), "{reasons:?}");
                assert!(reasons.contains(&"no `record_bound` section in the Manifest".to_owned()), "{reasons:?}");
            }
            other => panic!("{}", c7_verdict::describe(&other)),
        }
    }

    #[test]
    fn c7_storm_inconclusive_probe_incomplete() {
        let mut e = good();
        e.probe_complete = false;
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["the probe is incomplete".to_owned()]));
    }

    #[test]
    fn c7_storm_inconclusive_booking_refused() {
        let mut e = good();
        e.booking_admitted = false;
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["booking refused".to_owned()]));
    }

    #[test]
    fn c7_storm_inconclusive_control_missing() {
        let mut e = good();
        e.control_at = None;
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["no control start in the probe".to_owned()]));
    }

    #[test]
    fn c7_storm_inconclusive_control_not_separable() {
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.dc(CONTROL_AT + DELAY, CONTROL_AT + DELAY + SPAN, 0.5);
        synth.play(CONTROL_AT + DELAY + SPAN, 549, 1.0, 0.0);
        let e = evidence(&synth, RESUMES, BOUND, false);
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["the control is not separable from the floor".to_owned()]));
    }

    #[test]
    fn c7_storm_inconclusive_hole_in_the_capture() {
        let mut synth = base();
        synth.hole(FIRST + 10 * SPAN, FIRST + 10 * SPAN + PER);
        let e = evidence(&synth, RESUMES, BOUND, false);
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["hole in the capture".to_owned()]));
    }

    #[test]
    fn c7_storm_inconclusive_a_hole_between_the_control_and_the_held_start() {
        // bt2 fix round 2, item 6: the capture is one continuous run from its own first sample, not from the held Start: a hole at 100 ms is after the control's analysis
        // and before the first drop at 130 ms.
        let mut synth = base();
        synth.hole(100 * MS, 100 * MS + PER);
        let e = evidence(&synth, RESUMES, BOUND, false);
        assert!(e.capture.covers(FIRST, e.capture.end()), "the part from the held Start on is whole");
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["hole in the capture".to_owned()]));
    }

    #[test]
    fn c7_storm_inconclusive_the_capture_link_dropped_blocks() {
        // bt2 fix round 2, item 4(b): its own reason, before the hole it leaves.
        let mut e = good();
        e.link_drops = 7;
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["the capture link dropped 7 blocks".to_owned()]));
        let mut synth = base();
        synth.hole(FIRST + 10 * SPAN, FIRST + 10 * SPAN + PER);
        let mut e = evidence(&synth, RESUMES, BOUND, false);
        e.link_drops = 7;
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["the capture link dropped 7 blocks".to_owned(), "hole in the capture".to_owned()]));
    }

    #[test]
    fn c7_storm_inconclusive_a_kept_row_whose_incident_lacks_the_data_the_rule_needs() {
        // bt2 fix round 2, item 3: the row's own incident has no close to pair it with (the sparse event): never passed on ordering alone.
        let mut e = good();
        e.row_checks[7] = Err("ambiguous: resume row 7 has no close number 8 in the probe (7 accepted closes)".to_owned());
        e.row_checks[9] = Err("ambiguous: resume row 9 has no close number 10 in the probe (7 accepted closes)".to_owned());
        assert_class(
            storm_verdict(&e),
            Class::Inconclusive(vec!["2 kept row(s) lack the data the R rule needs; the first: ambiguous: resume row 7 has no close number 8 in the probe (7 accepted closes)".to_owned()]),
        );
    }

    #[test]
    fn c7_storm_inconclusive_a_capture_shorter_than_scheduled() {
        // F5: the sink returns a partial artifact at Stop, contiguous only through 250 ms, while the capture was booked for 600 ms.
        let mut synth = Synth::new(RATE, 0, 250 * MS);
        synth.play(CONTROL_AT + DELAY, (550 * 100 / c7::FAKE_PERIOD_US) as usize, 1.0, 0.0);
        let e = evidence(&synth, RESUMES, BOUND, false);
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["the capture is shorter than the scheduled 600000 samples".to_owned()]));
    }

    #[test]
    fn c7_storm_a_capture_one_sample_short_is_complete_and_two_short_is_not() {
        // F5: the scheduled end is the booked count's last sample, so one sample less is still covered and two less is the truncated capture.
        let mut one = Synth::new(RATE, 0, CAPTURE_END - PER);
        one.play(CONTROL_AT + DELAY, (550 * 100 / c7::FAKE_PERIOD_US) as usize, 1.0, 0.0);
        let one = evidence(&one, RESUMES, BOUND, false);
        assert_class(storm_verdict(&one), Class::Clean);
        let mut two = Synth::new(RATE, 0, CAPTURE_END - 2 * PER);
        two.play(CONTROL_AT + DELAY, (550 * 100 / c7::FAKE_PERIOD_US) as usize, 1.0, 0.0);
        let two = evidence(&two, RESUMES, BOUND, false);
        assert_class(storm_verdict(&two), Class::Inconclusive(vec!["the capture is shorter than the scheduled 600000 samples".to_owned()]));
    }

    #[test]
    fn c7_storm_inconclusive_short_of_the_bound() {
        let mut e = good();
        e.stats_resumes = BOUND as i64;
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["the storm did not reach the bound".to_owned()]));
    }

    #[test]
    fn c7_storm_inconclusive_no_held_start() {
        let mut e = good();
        e.held_first_at = None;
        assert_class(storm_verdict(&e), Class::Inconclusive(vec!["no held start in the probe".to_owned()]));
    }

    // ---------------------------------------------------------------- the named failures, each otherwise CLEAN

    #[test]
    fn c7_storm_fail_bound_wrong() {
        let mut kept = good();
        kept.rows_kept = BOUND + 1;
        assert_class(storm_verdict(&kept), Class::Fail(Fail::BoundWrong, "a declared bound of 1000 (expected 1000), 1001 kept resume rows and 6 omitted, for 1006 resumes".to_owned()));
        let mut omitted = good();
        omitted.omitted_resumes = 2;
        assert_class(storm_verdict(&omitted), Class::Fail(Fail::BoundWrong, "a declared bound of 1000 (expected 1000), 1000 kept resume rows and 2 omitted, for 1006 resumes".to_owned()));
    }

    #[test]
    fn c7_storm_fail_a_manifest_declaring_another_bound_is_not_clean() {
        // F6: the Manifest under test declares 500 and keeps 500 of 1 200, so its own numbers agree with each other; only the Module's default 1 000 says it is wrong.
        let mut e = good();
        e.bound = 500;
        e.rows_kept = 500;
        e.omitted_resumes = 700;
        e.stats_resumes = 1_200;
        assert_class(storm_verdict(&e), Class::Fail(Fail::BoundWrong, "a declared bound of 500 (expected 1000), 500 kept resume rows and 700 omitted, for 1200 resumes".to_owned()));
    }

    #[test]
    fn c7_storm_fail_substituted_kept_row() {
        // F6: kept row 3 is replaced by a tick from later in the storm, and no event names it any more.
        let mut e = good();
        e.events.remove(3);
        e.row_ticks[3] = e.start_ticks[6];
        let (row, start) = (e.row_ticks[3], e.start_ticks[4]);
        assert_class(storm_verdict(&e), Class::Fail(Fail::BoundWrong, format!("kept row 3 at {row} is after its paired Start at {start}")));
    }

    #[test]
    fn c7_storm_fail_a_kept_row_off_its_own_rule_range() {
        // bt2 fix round 2, item 3: kept row 3's R is another value inside its Start interval (ordering holds) but off the range its own incident's rule allows.
        let mut e = good();
        e.row_checks[3] = Err("R 123 is not a lattice point in 150..=151 (decided at 7; uncertainty 1)".to_owned());
        assert_class(storm_verdict(&e), Class::Fail(Fail::ResumeOffRule, "kept row 3: R 123 is not a lattice point in 150..=151 (decided at 7; uncertainty 1)".to_owned()));
    }

    #[test]
    fn c7_storm_fail_close_held() {
        let mut e = good();
        e.close_held = true;
        assert_class(storm_verdict(&e), Class::Fail(Fail::CloseHeld, "a close send was held".to_owned()));
    }

    #[test]
    fn c7_storm_fail_storm_radiated() {
        let mut synth = base();
        let leak = FIRST + DELAY + 2 * SPAN;
        synth.play(leak, 2, 1.0, 0.0);
        let e = evidence(&synth, RESUMES, BOUND, false);
        assert_class(storm_verdict(&e), Class::Fail(Fail::StormRadiated, format!("heard RF at {leak} after the first drop at {FIRST}")));
    }

    #[test]
    fn c7_the_storm_fails_off_lattice_pn_radiated_after_the_first_drop_at_the_hardware_period() {
        // F4: at the bench's 1 ms PN, 3 periods a quarter period off the repeat's lattice after the first drop are under the correlation's floor and over the
        // power test's threshold.
        let period_us = c7::HW_PERIOD_US;
        let clean = evidence_at(&base_at(period_us), RESUMES, BOUND, false, period_us);
        assert_class(storm_verdict(&clean), Class::Clean);
        let span = c7::period(RATE, period_us) as i64 * PER;
        let mut synth = base_at(period_us);
        let leak = FIRST + DELAY + 2 * span + span / 4;
        synth.play(leak, 3, 1.0, 0.0);
        let e = evidence_at(&synth, RESUMES, BOUND, false, period_us);
        match storm_verdict(&e) {
            Class::Fail(Fail::StormRadiated, text) => assert!(text.starts_with("RF power "), "{text}"),
            other => panic!("{}", c7_verdict::describe(&other)),
        }
    }

    #[test]
    fn c7_storm_fail_event_outside_the_kept_rows() {
        let mut e = good();
        e.events.insert(1, e.row_plans[1] + 1);
        let (event, last) = (e.row_plans[1] + 1, *e.row_plans.last().unwrap());
        assert_class(storm_verdict(&e), Class::Fail(Fail::EventsOutsideRows, format!("event plan {event} is no kept row's resume point (the kept rows end at {last})")));
    }

    #[test]
    fn c7_storm_fail_events_exceed_resumes() {
        let mut e = good();
        let extra = *e.row_plans.last().unwrap() + 1_000;
        e.events.push(extra);
        assert_class(storm_verdict(&e), Class::Fail(Fail::EventsExceedResumes, "1007 resume events for 1006 resumes".to_owned()));
    }

    #[test]
    fn c7_storm_fail_events_missing_on_the_fake() {
        let mut e = evidence(&base(), RESUMES, BOUND, true);
        e.events.truncate(4);
        assert_class(storm_verdict(&e), Class::Fail(Fail::EventsMissing, "4 resume events for 1006 resumes on the fake (one per drop, but the first may be `expired`)".to_owned()));
    }

    #[test]
    fn c7_storm_events_missing_is_clean_off_the_fake() {
        let mut e = good();
        e.events.truncate(4);
        assert_class(storm_verdict(&e), Class::Clean);
    }

    #[test]
    fn c7_the_kept_rows_are_the_first_rows() {
        let rows = [100, 200, 300, 400, 500];
        let starts = [50, 150, 250, 350, 450, 550];
        assert_eq!(kept_rows_are_the_first(&rows, &starts), Ok(()), "the accepted case");
        let mut later = rows;
        later[3] = 460;
        assert_eq!(kept_rows_are_the_first(&later, &starts), Err("kept row 3 at 460 is after its paired Start at 450".to_owned()), "a substituted later row");
        let mut earlier = rows;
        earlier[3] = 200;
        assert_eq!(kept_rows_are_the_first(&earlier, &starts), Err("kept row 3 at 200 is before the Start at 350".to_owned()), "a substituted earlier row");
        assert_eq!(kept_rows_are_the_first(&rows, &[50, 150, 250, 350, 450]), Err("kept row 4 has no paired Start".to_owned()), "a missing paired Start");
        assert_eq!(kept_rows_are_the_first(&[900], &[800, 1_000]), Ok(()), "the last kept row has no next row to compare");
    }
}
