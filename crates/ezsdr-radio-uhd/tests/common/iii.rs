//! Bench preparation (iii) after note 48 (2026-11-06): the pure judgements of the hardware tests that the runbook's
//! blocks (b) to (f) read. Each test prints the verdict these functions return; the decision is here, so that a wrong
//! comparison fails a unit test without a USRP.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration as Wall;

use ezsdr_kernel::manifest::Manifest;
use ezsdr_radio_uhd::{Device, Dir, Iq, Settings};
use serde_json::{Value as Json, json};

use super::MCR;
use super::measurement::TrialClass;
use super::n47;
use super::raw::{TICKS_PER_MS, align_to, d9_channel, drain, iq, pin_slot_a, raw_rx, raw_tx, read, send_burst, tone_wave, wait_burst_ack};
use super::tone::{PER_1MSPS, Verdict, contiguous_windows_at, first_ms_frequency, window_tone_at};
use super::{effective_layout, events_of, section, set_selector_override};
use ezsdr_radio::staging::Bookings;

/// The transmit faults a Run reports: its `radio.TIME_ERROR` and `radio.TX_UNDERFLOW` events, and the async section's
/// reports that are not a flush's (UR-28: a flush's underflow is recorded with `tx_flush` and emits no event).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Faults {
    pub time_errors: usize,
    pub underflows: usize,
    pub async_faults: usize,
}

/// The faults of one Run's manifest.
pub fn run_faults(manifest: &Manifest) -> Faults {
    Faults {
        time_errors: events_of(manifest, "radio.TIME_ERROR").len(),
        underflows: events_of(manifest, "radio.TX_UNDERFLOW").len(),
        async_faults: section(manifest, "async").as_array().into_iter().flatten().filter(|row| is_async_fault(row)).count(),
    }
}

/// Whether an async row is a device fault of a burst (not the flush's): a late burst, an underflow of either kind, or a
/// sequence error.
fn is_async_fault(row: &Json) -> bool {
    row["tx_flush"] != true && matches!(row["code"].as_str(), Some("TimeError" | "Underflow" | "UnderflowInPacket" | "SeqError" | "SeqErrorInBurst"))
}

/// The verdict on one Run's faults. A device underflow or async fault is a FAIL. A `radio.TIME_ERROR` beyond the
/// `expected` count is a FAIL. Fewer than expected is INCONCLUSIVE: a negative control that did not show its fault
/// proves nothing about the fault being absent.
pub fn fault_verdict(faults: Faults, expected_time_errors: usize) -> Verdict {
    if faults.underflows > 0 || faults.async_faults > 0 {
        Verdict::Fail(format!("{} radio.TX_UNDERFLOW and {} device fault report(s) in the async section", faults.underflows, faults.async_faults))
    } else if faults.time_errors > expected_time_errors {
        Verdict::Fail(format!("{} radio.TIME_ERROR, {expected_time_errors} expected", faults.time_errors))
    } else if faults.time_errors < expected_time_errors {
        Verdict::Inconclusive(format!("{} radio.TIME_ERROR of the {expected_time_errors} the negative control expects: the control did not show its fault", faults.time_errors))
    } else {
        Verdict::Pass(format!("{} radio.TIME_ERROR as expected; no radio.TX_UNDERFLOW; no device fault report", faults.time_errors))
    }
}

/// The faults of several Runs, each with its own expected `radio.TIME_ERROR` count (a negative control expects one): the
/// first FAIL, else INCONCLUSIVE when a Run is, else PASS. Each Run is named in the text.
pub fn faults_verdict(runs: &[(&str, Faults, usize)]) -> Verdict {
    let mut detail = Vec::new();
    let mut inconclusive = false;
    for (name, faults, expected) in runs {
        match fault_verdict(*faults, *expected) {
            Verdict::Fail(text) => return Verdict::Fail(format!("{name}: {text}")),
            Verdict::Inconclusive(text) => {
                inconclusive = true;
                detail.push(format!("{name}: {text}"));
            }
            Verdict::Pass(text) => detail.push(format!("{name}: {text}")),
        }
    }
    let text = detail.join("; ");
    if inconclusive { Verdict::Inconclusive(text) } else { Verdict::Pass(text) }
}

/// `hw_s1_short_first_sends` for one case: how many of the run's device bursts' BurstAcks came before the long burst's
/// start (`acks_before`, the short one's) and how many at or after it (`acks_after`). PASS needs one each and no fault.
pub fn short_first_send_verdict(samples: usize, faults: Faults, acks_before: usize, acks_after: usize) -> Verdict {
    if faults.time_errors > 0 || faults.underflows > 0 || faults.async_faults > 0 {
        return Verdict::Fail(format!("{samples}-sample first send: {} TIME_ERROR, {} TX_UNDERFLOW, {} device fault report(s)", faults.time_errors, faults.underflows, faults.async_faults));
    }
    if acks_before == 1 && acks_after == 1 {
        Verdict::Pass(format!("{samples}-sample first send: no TIME_ERROR, no TX_UNDERFLOW, one BurstAck for each burst"))
    } else {
        Verdict::Inconclusive(format!("{samples}-sample first send: {acks_before} BurstAck(s) for the short burst and {acks_after} for the long one: each needs exactly one"))
    }
}

/// The `radio.TIME_ERROR` causes of a Run, in order (the payload's `cause`; empty when it has none).
pub fn time_error_causes(manifest: &Manifest) -> Vec<String> {
    events_of(manifest, "radio.TIME_ERROR").iter().map(|event| event.payload["cause"].as_str().unwrap_or_default().to_owned()).collect()
}

/// `hw_b8_burst_one_sample_after_a_burst`'s negative control: `expected_late` `radio.TIME_ERROR` (0 or 1), each with cause
/// `late` (UR-21), and no device fault (`fault_verdict`). Another cause is a FAIL.
pub fn late_control_verdict(faults: Faults, causes: &[String], expected_late: usize) -> Verdict {
    match causes.iter().find(|cause| cause.as_str() != "late") {
        Some(cause) => Verdict::Fail(format!("a radio.TIME_ERROR with cause {cause:?}, not \"late\"")),
        None => fault_verdict(faults, expected_late),
    }
}

/// The receive restart's leading fill, measured on one trial's analysed span: `zeros` samples that are exactly zero
/// in fc32 at its start (the wire's zeros, since an all-zero sc16 word converts to exactly 0.0), and the samples from
/// the first non-zero sample to the first sample at 10 % and at 90 % of the steady magnitude.
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Fill {
    pub zeros: usize,
    pub ramp10: usize,
    pub ramp90: usize,
    pub steady: f64,
}

/// The fill of a span of `samples` (tick, sample) at `per` ticks a sample. None when the span has a gap or holds no non-zero
/// sample. The steady magnitude is the median over the last half, so the ramp always reaches 90 % of it.
pub fn restart_fill(samples: &[(i64, Iq)], per: i64) -> Option<Fill> {
    if samples.len() < 4 || samples.windows(2).any(|pair| pair[1].0 - pair[0].0 != per) {
        return None;
    }
    let magnitudes: Vec<f64> = samples.iter().map(|(_, s)| f64::from(s[0]).hypot(f64::from(s[1]))).collect();
    let mut steady_half = magnitudes[magnitudes.len() / 2..].to_vec();
    steady_half.sort_by(f64::total_cmp);
    let steady = steady_half[steady_half.len() / 2];
    if steady <= 0.0 {
        return None;
    }
    let zeros = samples.iter().take_while(|(_, s)| s[0] == 0.0 && s[1] == 0.0).count();
    let first = magnitudes.iter().position(|m| *m > 0.0)?;
    let reach = |fraction: f64| magnitudes.iter().enumerate().skip(first).find(|(_, m)| **m >= fraction * steady).map(|(index, _)| index);
    Some(Fill { zeros, ramp10: reach(0.1)? - first, ramp90: reach(0.9)? - first, steady })
}

/// One trial of `hw_iii_raw_rx_restart_fill`: whether the old stream drained quietly before the rate was applied, the rate
/// the device read back, the planned and the first block's tick, whether the analysed span is continuously covered and
/// free of overflow, the tone found in its steady window, and the fill.
#[derive(Debug, Clone)]
pub struct RestartTrial {
    pub drained: bool,
    pub asked: f64,
    pub applied: f64,
    pub planned: i64,
    pub first_tick: Option<i64>,
    pub covered: bool,
    pub tone_hz: Option<f64>,
    pub fill: Option<Fill>,
}

/// Why a trial does not count, or None when it does: the rate read back must be the asked one, the span covered, a steady
/// tone at `tone_hz` found in it, and the fill measured.
pub fn incomplete_reason(trial: &RestartTrial, tone_hz: f64, tolerance_hz: f64) -> Option<String> {
    if !trial.drained {
        Some("the old stream did not drain quietly: no rate was applied".to_owned())
    } else if (trial.applied - trial.asked).abs() > trial.asked.abs() * 1e-9 + 1e-6 {
        Some(format!("rate mismatch (asked {}, read back {})", trial.asked, trial.applied))
    } else if !trial.covered {
        Some("the span is not continuously covered".to_owned())
    } else if !trial.tone_hz.is_some_and(|hz| (hz - tone_hz).abs() <= tolerance_hz) {
        Some(format!("no steady tone at {tone_hz:.0} Hz in the steady window"))
    } else if trial.fill.is_none() {
        Some("no fill measured (no non-zero sample, or 90 % never reached)".to_owned())
    } else {
        None
    }
}

/// The fills of the trials that count (`incomplete_reason` is None): the summary and the verdict both read these.
pub fn complete_fills(trials: &[RestartTrial], tone_hz: f64, tolerance_hz: f64) -> Vec<Fill> {
    trials.iter().filter(|trial| incomplete_reason(trial, tone_hz, tolerance_hz).is_none()).filter_map(|trial| trial.fill).collect()
}

/// The verdict on one receive rate of `hw_iii_raw_rx_restart_fill`. FAIL only when a trial's first block is not at its
/// planned tick. PASS needs three complete trials. Anything else is INCONCLUSIVE, with the reason.
pub fn restart_rate_verdict(rate_hz: f64, trials: &[RestartTrial], tone_hz: f64, tolerance_hz: f64) -> Verdict {
    if let Some(trial) = trials.iter().find(|trial| trial.first_tick.is_some_and(|tick| tick != trial.planned)) {
        return Verdict::Fail(format!("{rate_hz:.0} S/s: a first block is at {:?} for the planned tick {}", trial.first_tick, trial.planned));
    }
    let fills = complete_fills(trials, tone_hz, tolerance_hz);
    if fills.len() < 3 {
        let reasons: Vec<String> = trials.iter().filter_map(|trial| incomplete_reason(trial, tone_hz, tolerance_hz)).collect();
        return Verdict::Inconclusive(format!("{rate_hz:.0} S/s: {} of {} trials complete (three needed): {}", fills.len(), trials.len(), reasons.join("; ")));
    }
    let zeros: Vec<usize> = fills.iter().map(|fill| fill.zeros).collect();
    let ramps: Vec<usize> = fills.iter().map(|fill| fill.ramp90).collect();
    Verdict::Pass(format!("{rate_hz:.0} S/s: {} complete trials; zeros {zeros:?}; samples to 90 % {ramps:?}", fills.len()))
}

/// The receive rates of `hw_iii_raw_rx_restart_fill`: 200 MHz / rate is an integer for each.
pub const RESTART_RATES: [f64; 8] = [1e6, 2e6, 5e6, 10e6, 25e6, 50e6, 100e6, 200e6];

/// The restarts per rate.
pub const RESTARTS: usize = 5;

/// The tone the probe's source sounds, 100 kHz from the carrier.
pub const RESTART_TONE_HZ: f64 = 100e3;

/// One receive rate of the probe: its trials, and the observation it printed.
pub struct RateFill {
    pub rate: f64,
    pub trials: Vec<RestartTrial>,
    pub observed: Json,
}

/// `hw_iii_raw_rx_restart_fill`'s probe on `device`: raw `Device` calls, no Session. Slot A is pinned and both directions are
/// set to 1 MS/s; each of `rates` gets `restarts` trials (`restart_fill_trials`). Each rate's observation is printed as one
/// `III fill observation` line, every trial with its completeness and reason, so the log keeps all of them.
pub fn restart_fill_probe(device: Arc<dyn Device>, rates: &[f64], restarts: usize) -> Vec<RateFill> {
    let hz = pin_slot_a(&*device);
    raw_tx(&*device, 1e6, hz);
    raw_rx(&*device, 1e6, hz);
    let fills = rates
        .iter()
        .map(|&rate| {
            let (trials, observed) = restart_fill_trials(device.clone(), rate, restarts);
            println!("III fill observation {observed}");
            RateFill { rate, trials, observed }
        })
        .collect();
    device.rx_stop(0).ok();
    let _ = drain(&*device);
    fills
}

/// min, median and max of `values` (sorted in place), `None` when empty.
fn min_median_max(values: &mut [f64]) -> Option<(f64, f64, f64)> {
    values.sort_by(f64::total_cmp);
    Some((*values.first()?, values[values.len() / 2], *values.last()?))
}

/// The trials at one receive rate: `restarts` timed receive starts on a streamer already open at that rate, each with the
/// 100 kHz tone sounding from 20 ms before the planned start (50 ms ahead of it). Each trial reads to `max(4 000 samples,
/// 2 ms)` past the planned start and keeps only that span, so memory stays bounded at 200 MS/s (the probe's block retention
/// is left off). Returns the trials and their observation; the summary counts the complete trials only.
fn restart_fill_trials(device: Arc<dyn Device>, rate: f64, restarts: usize) -> (Vec<RestartTrial>, Json) {
    let per = (MCR as f64 / rate) as i64;
    let span = (4_000 * per).max(2 * TICKS_PER_MS);
    device.rx_stop(0).ok();
    let stale = drain(&*device);
    if !stale.quiet {
        let undrained = RestartTrial { drained: false, asked: rate, applied: f64::NAN, planned: 0, first_tick: None, covered: false, tone_hz: None, fill: None };
        return (vec![undrained; restarts], json!({ "rate_hz": rate, "incomplete": "the old stream did not drain quietly; no rate was applied", "stale_blocks": stale.read.blocks.len() }));
    }
    let applied = device.apply(Dir::Rx, d9_channel(), &Settings { rate: Some(rate), ..Settings::default() }).unwrap();
    let tone = iq(&tone_wave(1e6, 60_000, RESTART_TONE_HZ));
    let mut trials = Vec::new();
    let mut observed = Vec::new();
    for _ in 0..restarts {
        let planned = align_to(device.time_now().unwrap() + 50 * TICKS_PER_MS, per);
        let source_at = align_to(device.time_now().unwrap() + 20 * TICKS_PER_MS, 200);
        let got = std::thread::scope(|scope| {
            let source = scope.spawn(|| send_burst(&*device, &tone, Some(source_at)));
            device.rx_start(0, planned).unwrap();
            let got = read(&*device, Wall::from_secs(2), Some(per), |r| r.end(per).is_some_and(|end| end >= planned + span));
            device.rx_stop(0).unwrap();
            source.join().expect("the tone source's send");
            got
        });
        let (ack, _) = wait_burst_ack(&*device, Wall::from_secs(3));
        let span_samples: Vec<(i64, Iq)> = got.samples.iter().copied().filter(|(tick, _)| *tick >= planned && *tick < planned + span).collect();
        let first_tick = got.blocks.first().map(|(tick, _)| *tick);
        let contiguous = span_samples.windows(2).all(|pair| pair[1].0 - pair[0].0 == per);
        let covered = first_tick.is_some() && contiguous && span_samples.len() as i64 == span / per && got.other.iter().all(|(_, other)| other == "Timeout");
        let tone_hz = first_ms_frequency(&span_samples, planned + span - TICKS_PER_MS, per);
        let fill = restart_fill(&span_samples, per);
        let trial = RestartTrial { drained: true, asked: rate, applied: applied.rate, planned, first_tick, covered, tone_hz, fill };
        let reason = incomplete_reason(&trial, RESTART_TONE_HZ, n47::TONE_TOLERANCE_HZ);
        observed.push(json!({
            "planned": planned, "first_tick": first_tick, "source_at": source_at, "source_ack": ack, "applied": applied.rate, "covered": covered,
            "other_outcomes": got.other.iter().filter(|(_, other)| other != "Timeout").collect::<Vec<_>>(), "tone_hz": tone_hz,
            "fill": fill.map(|fill| json!({ "zeros": fill.zeros, "ramp10": fill.ramp10, "ramp90": fill.ramp90, "steady": fill.steady })),
            "complete": reason.is_none(), "reason": reason,
        }));
        trials.push(trial);
        device.rx_stop(0).ok();
        let _ = drain(&*device);
    }
    let fills = complete_fills(&trials, RESTART_TONE_HZ, n47::TONE_TOLERANCE_HZ);
    let mut zeros: Vec<f64> = fills.iter().map(|fill| fill.zeros as f64).collect();
    let mut ramp: Vec<f64> = fills.iter().map(|fill| fill.ramp90 as f64).collect();
    let ns = |samples: f64| samples * 1e9 / rate;
    let (zeros_range, ramp_range) = (min_median_max(&mut zeros), min_median_max(&mut ramp));
    println!(
        "rx restart fill at {rate:.0} S/s over {} complete trials of {}: exact zeros in fc32 {:?} samples ({:?} ns); samples to 90 % {:?} ({:?} ns); applied {}",
        fills.len(),
        trials.len(),
        zeros_range,
        zeros_range.map(|(a, b, c)| (ns(a), ns(b), ns(c))),
        ramp_range,
        ramp_range.map(|(a, b, c)| (ns(a), ns(b), ns(c))),
        applied.rate
    );
    (trials, json!({ "rate_hz": rate, "applied_rate": applied.rate, "per": per, "span_ticks": span, "restarts": observed, "zeros_samples": zeros_range, "ramp90_samples": ramp_range }))
}

/// The transmit-side judgement of one stage (the shared check of `hw_tx200_hot_path_probe` and `hw_iii_load_200msps`):
/// the class is Success; each transmit stream accepted samples and reported no async failure; each receive stream had no
/// overflow; no transmit underflow; and every link's feed dropped no block. A link whose drop count was not read is not
/// a success, since the claim is about what the Module delivered.
pub fn stage_trial(class: TrialClass, observation: &Json) -> Result<(), String> {
    if class != TrialClass::Success {
        return Err(format!("class {class:?}"));
    }
    let streams: Vec<&Json> = observation["streams"].as_array().into_iter().flatten().collect();
    if streams.is_empty() {
        return Err("no stream in the observation".to_owned());
    }
    for row in &streams {
        match row["direction"].as_str() {
            Some("tx") => {
                if !row["actual_samples"].as_u64().is_some_and(|n| n > 0) {
                    return Err("no accepted transmit samples in the observation".to_owned());
                }
                if let Some(failure) = row["async_failures"].as_array().into_iter().flatten().next() {
                    return Err(format!("an async failure: {failure}"));
                }
            }
            Some("rx") => {
                if observation["rx_overflow"] == true {
                    return Err("a receive overflow".to_owned());
                }
            }
            _ => return Err("a stream with no direction".to_owned()),
        }
    }
    let underflows = observation["tx_underflow_events"].as_u64().unwrap_or_else(|| streams.iter().filter_map(|row| row["tx_underflow_events"].as_u64()).sum());
    if underflows != 0 {
        return Err(format!("{underflows} transmit underflow(s)"));
    }
    match observation["link_drops"].as_array() {
        Some(drops) if !drops.is_empty() => {
            if let Some(drop) = drops.iter().find(|drop| drop.as_u64() != Some(0)) {
                return Err(format!("the Module's feed did not report zero dropped blocks: {drop}"));
            }
        }
        _ => return Err("the link drop counts were not read".to_owned()),
    }
    Ok(())
}

/// The bench's NICs, from `EZSDR_BENCH_NIC` (whitespace-separated; the runbook sets one per board).
pub fn bench_nics() -> Vec<String> {
    std::env::var("EZSDR_BENCH_NIC").map(|text| text.split_whitespace().map(str::to_owned).collect()).unwrap_or_default()
}

/// The layout that puts a probe on slot A alone: the device's first receive and transmit elements, which name slot A's
/// front ends (the X300's channel 0). Installed for the thread, it replaces `EZSDR_MEASUREMENT_SELECTOR` for the
/// probes that pin it, so a second board in slot B is never transmitted on.
pub fn pin_to_slot_a(device: &dyn ezsdr_radio_uhd::Device) -> Result<(), String> {
    let (rx, tx) = effective_layout(device);
    let first = |elements: &Json| elements.as_array().and_then(|elements| elements.first()).cloned().ok_or("the device layout has no element".to_owned());
    let (rx0, tx0) = (first(&rx)?, first(&tx)?);
    if !tx0.as_str().is_some_and(|element| element.contains(":A:")) {
        return Err(format!("the first transmit element {tx0} is not slot A"));
    }
    set_selector_override(Some(json!({ "rx": [rx0], "tx": [tx0] })));
    let (rx, tx) = effective_layout(device);
    if rx.as_array().map_or(0, Vec::len) != 1 || tx.as_array().map_or(0, Vec::len) != 1 {
        return Err(format!("slot A alone should give one receive and one transmit channel, the layout gives rx {rx} and tx {tx}"));
    }
    Ok(())
}

// ---------------------------------------------------------------- stage 5 bench (2026-11-07)
//
// The judgements of the `hw_n47s5_*` tests and of the changed `hw_b8_*` tests (bench.md, 2026-11-07). The record's fronts,
// `J(e)` and the stop bound are read from `ezsdr_radio::staging` and `ezsdr_radio_uhd::profile`, never copied here.

/// A booking that should play is accepted only when the Kernel admits it and the Provider refuses none of its Actions
/// (`refusals`, `n47s5::refusals_of`). A refusal is a FAIL, and its reasons are in the text.
pub fn admission_verdict(admitted: bool, refusals: &[String]) -> Verdict {
    if !admitted {
        Verdict::Fail("the booking was refused at receipt".to_owned())
    } else if !refusals.is_empty() {
        Verdict::Fail(format!("the Provider refused a booking that should play: {refusals:?}"))
    } else {
        Verdict::Pass("admitted and not refused".to_owned())
    }
}

/// A booking that is admitted, not refused by the Provider (`refusals`, `n47s5::refusals_of`), raises no fault and plays on time.
/// The on-time part needs a timestamped onset in the capture (S7): without one the result is INCONCLUSIVE and the text says what was shown.
pub fn booking_verdict(admitted: bool, refusals: &[String], faults: Faults, onset_offset: Option<i64>, delay: Option<i64>, tolerance: i64) -> Verdict {
    let admission = admission_verdict(admitted, refusals);
    if matches!(admission, Verdict::Fail(_)) {
        return admission;
    }
    let clean = fault_verdict(faults, 0);
    if matches!(clean, Verdict::Fail(_)) {
        return clean;
    }
    match (onset_offset, delay) {
        (None, _) => Verdict::Inconclusive("admitted and no reported faults; the on-time part is INCONCLUSIVE: no tone onset in the capture (or a hole before it)".to_owned()),
        (Some(_), None) => Verdict::Inconclusive("admitted and no reported faults; the on-time part is INCONCLUSIVE: the loop delay is not measured (no onset of the trial's first burst)".to_owned()),
        (Some(offset), Some(delay)) if (offset - delay).abs() > tolerance => {
            Verdict::Fail(format!("the tone starts {offset} samples after its booked instant; the loop's delay is {delay} ± {tolerance}"))
        }
        (Some(offset), Some(delay)) => Verdict::Pass(format!("admitted, no reported faults, the tone starts {offset} samples after its booked instant (loop delay {delay} ± {tolerance})")),
    }
}

/// `hw_n47s5_front_after_a_short_burst` for one case: the second burst is booked at the record's front. The `bursts` device bursts
/// (the calibration, the head and the booked tone) are each acknowledged once (`acks`), and the booking is judged as [`booking_verdict`].
#[allow(clippy::too_many_arguments)]
pub fn front_case_verdict(admitted: bool, refusals: &[String], faults: Faults, acks: usize, bursts: usize, onset_offset: Option<i64>, delay: Option<i64>, tolerance: i64) -> Verdict {
    match booking_verdict(admitted, refusals, faults, onset_offset, delay, tolerance) {
        Verdict::Pass(text) if acks != bursts => Verdict::Inconclusive(format!("{text}; {acks} BurstAck(s) for {bursts} bursts, each needs one")),
        other => other,
    }
}

/// The raw front and gap probe's outcome for one trial (an observation, not a judgement): complete when both bursts were acknowledged.
pub fn gap_trial_outcome(acks: usize) -> &'static str {
    if acks == 2 { "complete" } else { "incomplete" }
}

/// T3 (b)'s Session case (S3): the record's first sample after a Stop is the Provider's bound from its own receipt of the Stop, which the
/// Manifest does not record, so a booking at the Kernel's instant can be refused by the host latency alone (FakeDevice: refused at the
/// Kernel's bound, accepted 0.2 ms later). Only the overlap refusal is INCONCLUSIVE and printed; any other refusal is a FAIL. The raw case
/// (b) is the decisive one.
pub fn stop_booking_verdict(admitted: bool, refusals: &[String], faults: Faults, onset_offset: Option<i64>, delay: Option<i64>, tolerance: i64) -> Verdict {
    if admitted {
        match refusals {
            [] => {}
            [reason] if reason.contains("overlaps an accepted booking") => {
                return Verdict::Inconclusive(format!(
                    "refused at the Kernel's stop sample {refusals:?}: the Provider's bound follows its own receipt of the Stop (not recorded), so the host latency is not shown"
                ));
            }
            other => return Verdict::Fail(format!("refused at the Kernel's stop sample for another reason, or more than once: {other:?}")),
        }
    }
    booking_verdict(admitted, refusals, faults, onset_offset, delay, tolerance)
}

/// The late-arrival branch of T3 (f), the first evidence read for a booking that is not refused (its `refusals` empty). `late_by_ns`
/// are the amounts of the `radio.TIME_ERROR`s (cause late, outcome drop) whose target is the booking's instant. When the booking was
/// dropped as late and no other fault shows, it was late after its deadline, at receipt or at dispatch, and the case is not exercised.
/// `None` when the branch does not apply (no late drop of the booking, a refusal, or any other fault). The late drop is the same
/// `TIME_ERROR` at receipt and at dispatch (`control.rs`, `tx.rs`), so the text names both places.
pub fn late_arrival_verdict(late_by_ns: &[i64], refusals: &[String], faults: Faults) -> Option<Verdict> {
    let late = *late_by_ns.first()?;
    (refusals.is_empty() && faults.time_errors == late_by_ns.len() && faults.underflows == 0 && faults.async_faults == 0)
        .then(|| Verdict::Inconclusive(format!("C was late at receipt or at dispatch (late_by {late} ns, after its deadline): host latency or a late dispatch, case not exercised")))
}

/// The held tail (`hw_n47s5_held_tail_and_stop`, case f): the device's end-of-burst sends (`data_eobs`, from the ProbeDevice rows;
/// `None` when no send was recorded) and the acknowledgements, one per device burst.
pub fn close_verdict(faults: Faults, data_eobs: Option<usize>, expected_eobs: usize, acks: usize) -> Verdict {
    let clean = fault_verdict(faults, 0);
    if matches!(clean, Verdict::Fail(_)) {
        return clean;
    }
    let Some(eobs) = data_eobs else {
        return Verdict::Inconclusive("no transmit send was recorded: the end-of-burst count is not shown".to_owned());
    };
    if eobs != expected_eobs {
        return Verdict::Fail(format!("{eobs} device end-of-burst send(s), {expected_eobs} expected"));
    }
    if acks != eobs {
        return Verdict::Inconclusive(format!("{acks} BurstAck(s) for {eobs} device burst(s): one each is needed"));
    }
    Verdict::Pass(format!("{eobs} device burst(s) as expected, each acknowledged once, no reported faults"))
}

/// A booking the record refuses at receipt (RM-15): the Kernel admits the Action, and the Provider refuses it with one
/// `radio.COMMAND_REJECTED` naming that Action (RM-22). `rejections` are the reasons of the events that name the entry's
/// dispatched Actions (`n47s5::refusals_of`); none means the Provider did not refuse it.
pub fn refusal_verdict(rejections: &[String]) -> Verdict {
    match rejections {
        [reason] if reason.contains("overlaps an accepted booking") => Verdict::Pass(format!("refused at receipt: {reason}")),
        [] => Verdict::Fail("the booking was not refused at receipt: no radio.COMMAND_REJECTED names its Action".to_owned()),
        other => Verdict::Fail(format!("refused for another reason, or more than once: {other:?}")),
    }
}

/// One repeat switch's evidence (`hw_n47s5_repeat_switch`): the `switches` row's outcome, the faults, the seams the device recorded
/// at the switch (`None` when no send was recorded), the coherent tone windows `(start, Hz)` and every power window `(start, power)`.
pub struct SwitchTrial<'a> {
    /// The Provider's refusals of the switch (`n47s5::refusals_of`): a refused switch that should apply is a FAIL.
    pub refusals: &'a [String],
    /// The `switches` row's outcome; `None` when the Manifest has no row (a refused switch writes none, `control.rs`).
    pub row: Option<&'a str>,
    /// The switch's boundary in root ticks, before the loop delay; `None` when the row has none.
    pub boundary: Option<i64>,
    /// The loop delay of this trial in root ticks (its first repeat's onset); `None` when the capture shows no onset.
    pub delay: Option<i64>,
    /// The transition's half-width in root ticks: the loop delay's spread plus the DUC's rise (INFERRED).
    pub margin: i64,
    pub before_hz: f64,
    pub after_hz: f64,
    pub tolerance_hz: f64,
    pub faults: Faults,
    pub seams: Option<usize>,
    pub tone: &'a [(i64, f64)],
    pub power: &'a [(i64, f64)],
}

/// The repeat switch's verdict (S4, S5): the tone is the old frequency before the transition and the new one after it, nothing
/// else changes within 5 ms on each side of the boundary, the power holds across the boundary, and the device sees no end-of-burst
/// or start-of-burst at the switch.
pub fn switch_verdict(trial: &SwitchTrial) -> Verdict {
    if !trial.refusals.is_empty() {
        return Verdict::Fail(format!("the Provider refused the switch, which should apply: {:?}", trial.refusals));
    }
    let Some(row) = trial.row else {
        return Verdict::Fail("the switches section has no row".to_owned());
    };
    let clean = fault_verdict(trial.faults, 0);
    if matches!(clean, Verdict::Fail(_)) {
        return clean;
    }
    match trial.seams {
        None => return Verdict::Inconclusive("no transmit send was recorded: the seam at the switch is not shown".to_owned()),
        Some(seams) if seams > 0 => return Verdict::Fail(format!("{seams} end-of-burst or start-of-burst send(s) at the switch: the repeat was closed and restarted")),
        Some(_) => {}
    }
    if row == "uncertain" {
        return Verdict::Inconclusive("the switch row is \"uncertain\": the send that carried the boundary ended in an exception, so whether the new waveform was sent is unknown".to_owned());
    }
    if row != "applied" {
        return Verdict::Fail(format!("the switch row is {row:?}, expected \"applied\""));
    }
    let Some(boundary) = trial.boundary else {
        return Verdict::Fail("the switch row has no boundary".to_owned());
    };
    let Some(delay) = trial.delay else {
        return Verdict::Inconclusive("the loop delay is not measured: the capture shows no onset of the first repeat".to_owned());
    };
    let (b, m) = (boundary + delay, trial.margin);
    let starts: Vec<i64> = trial.power.iter().map(|(t, _)| *t).collect();
    if !covered(&starts, b - 5 * TICKS_PER_MS, b + 5 * TICKS_PER_MS) {
        return Verdict::Inconclusive("the capture has a hole within 5 ms of the boundary: the transition is not shown".to_owned());
    }
    let steady_window = |t: i64| t >= b - 5 * TICKS_PER_MS && t < b - m;
    let before = trial.tone.iter().filter(|(t, _)| steady_window(*t)).count();
    let after = trial.tone.iter().filter(|(t, _)| *t >= b + m && *t < b + 5 * TICKS_PER_MS).count();
    if before < 10 || after < 10 {
        return Verdict::Inconclusive(format!("the tone is not found on both sides of the boundary ({before} windows before, {after} after)"));
    }
    if let Some((t, hz)) = trial.tone.iter().find(|(t, hz)| {
        (*t >= b - 5 * TICKS_PER_MS && *t < b + 5 * TICKS_PER_MS)
            && ((*t < b - m && (hz - trial.before_hz).abs() > trial.tolerance_hz) || (*t >= b + m && (hz - trial.after_hz).abs() > trial.tolerance_hz))
    }) {
        return Verdict::Fail(format!("a {hz:.0} Hz window at tick {t}, outside the transition [{}, {}): the tone changes elsewhere", b - m, b + m));
    }
    let steady: Vec<f64> = trial.power.iter().filter(|(t, _)| steady_window(*t)).map(|(_, p)| *p).collect();
    if steady.len() < 10 {
        return Verdict::Inconclusive(format!("only {} power windows before the boundary", steady.len()));
    }
    let steady = steady.iter().sum::<f64>() / steady.len() as f64;
    let dip = trial.power.iter().filter(|(t, _)| *t >= b - m && *t < b + m).map(|(_, p)| *p).fold(f64::INFINITY, f64::min);
    if dip.is_infinite() {
        return Verdict::Inconclusive("no power window across the boundary".to_owned());
    }
    if dip <= steady / 4.0 {
        return Verdict::Fail(format!("the power falls to {dip:.2e} across the boundary, against {steady:.2e} before it: a close or a gap"));
    }
    Verdict::Pass(format!("the row is applied; the tone is {} Hz before and {} Hz after the boundary; the power holds ({dip:.2e} against {steady:.2e}); no seam", trial.before_hz, trial.after_hz))
}

/// `hw_n47s5_untimed` (1): an untimed repeat started again after a Stop is admitted and transmits (a tone found after it).
pub fn untimed_repeat_verdict(admitted: bool, refusals: &[String], tone_found: bool, faults: Faults) -> Verdict {
    if !admitted {
        return Verdict::Fail("the untimed repeat after the Stop was refused".to_owned());
    }
    if !refusals.is_empty() {
        return Verdict::Fail(format!("the Provider refused the untimed repeat, which should play: {refusals:?}"));
    }
    let clean = fault_verdict(faults, 0);
    if matches!(clean, Verdict::Fail(_)) {
        return clean;
    }
    if tone_found {
        Verdict::Pass("admitted, a tone after it, no reported faults".to_owned())
    } else {
        Verdict::Inconclusive("admitted and no reported faults; no tone after it: the transmit is not shown".to_owned())
    }
}

/// `hw_n47s5_untimed` (2): an untimed burst booked while a timed one plays is admitted, placed by the record at or after the
/// playing burst's front (`placed_at`, its applied target), and played on time against that target (`onset_offset`).
#[allow(clippy::too_many_arguments)]
pub fn untimed_burst_verdict(
    admitted: bool,
    refusals: &[String],
    placed_at: Option<i64>,
    earlier_front: i64,
    faults: Faults,
    onset_offset: Option<i64>,
    delay: Option<i64>,
    tolerance: i64,
) -> Verdict {
    if !admitted {
        return Verdict::Fail("the untimed burst was refused".to_owned());
    }
    if !refusals.is_empty() {
        return Verdict::Fail(format!("the Provider refused the untimed burst, which should play: {refusals:?}"));
    }
    match placed_at {
        None => Verdict::Inconclusive("the placed start is not in the bursts section".to_owned()),
        Some(at) if at < earlier_front => Verdict::Fail(format!("placed at tick {at}, before the playing burst's front {earlier_front}")),
        Some(_) => booking_verdict(true, refusals, faults, onset_offset, delay, tolerance),
    }
}

/// `hw_b8_stop_end` (S1): where the loop stops hearing the repeat (`heard_until`) against the repeat's recorded end (the `bursts`
/// row's target plus its samples, `None` when the Manifest has no such row); `tolerance` is the edge resolution plus the loop delay's spread.
pub fn stop_end_verdict(heard_until: Option<i64>, recorded_end: Option<i64>, tolerance: i64) -> Verdict {
    match (heard_until, recorded_end) {
        (Some(heard), Some(end)) if (heard - end).abs() <= tolerance => Verdict::Pass(format!("the loop stops hearing the repeat {} ticks from its recorded end (tolerance {tolerance})", heard - end)),
        (Some(heard), Some(end)) => Verdict::Fail(format!("the loop stops hearing the repeat {} ticks from its recorded end (tolerance {tolerance})", heard - end)),
        (None, _) => Verdict::Inconclusive("the power edge or the loop delay is not found in the capture".to_owned()),
        (_, None) => Verdict::Inconclusive("the repeat's recorded end is not in the Manifest".to_owned()),
    }
}

/// `hw_b8_stop_end`'s second check, printed and not judged (S1): the recorded end is at or before the Stop plus the in-flight window,
/// with the host's delay between the submission and the control's instant as the allowance.
pub fn stop_bound_text(recorded_end: Option<i64>, stop: i64, in_flight: i64, host_delay: i64) -> String {
    match recorded_end {
        Some(end) => {
            format!("recorded end {} ticks after the Stop; the in-flight window is {in_flight} and the host delay {host_delay}: within the window {}", end - stop, end <= stop + in_flight + host_delay)
        }
        None => "no recorded end".to_owned(),
    }
}

/// `hw_p70_…` (T2): the one-sample burst leaves the next burst's onset where the two-sample control leaves it. Each argument is the
/// onset in samples from its own booked instant; `tolerance` is 50 samples, as the test has always used.
pub fn onset_control_verdict(one: Option<i64>, control: Option<i64>, tolerance: i64) -> Verdict {
    match (one, control) {
        (Some(a), Some(b)) if (a - b).abs() <= tolerance => Verdict::Pass(format!("the onset differs from the control's by {} samples", a - b)),
        (Some(a), Some(b)) => Verdict::Fail(format!("the one-sample burst moved the next burst's onset by {} samples", a - b)),
        _ => Verdict::Inconclusive("no tone onset in one of the trials: the positive signal is missing".to_owned()),
    }
}

/// The counts `hw_p72_…` judges (T1, T2, T3 of the production path).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct TimedSends {
    pub refused: usize,
    pub untimed: usize,
    pub short_closing: usize,
    pub payload_sends: usize,
    pub closing: usize,
    pub flushes: usize,
    pub acked: usize,
}

/// `hw_p72_…`'s verdict: the faults first, then every send timed, every closing send at least two samples, the flushes
/// accepted and acknowledged, and enough payload and closing sends to show it.
pub fn timed_sends_verdict(faults: Verdict, sends: &TimedSends) -> Verdict {
    if !matches!(faults, Verdict::Pass(_)) {
        return faults;
    }
    if sends.refused > 0 {
        Verdict::Fail(format!("T3: {} flush(es) refused", sends.refused))
    } else if sends.untimed > 0 {
        Verdict::Fail(format!("T1: {} send(s) carry no time", sends.untimed))
    } else if sends.short_closing > 0 {
        Verdict::Fail(format!("T2: {} closing send(s) with fewer than two samples", sends.short_closing))
    } else if sends.payload_sends < 2 {
        Verdict::Inconclusive(format!("only {} send(s) carried samples: no multi-chunk evidence", sends.payload_sends))
    } else if sends.closing == 0 {
        Verdict::Inconclusive("no closing send was recorded: T2 not shown".to_owned())
    } else if sends.flushes == 0 {
        Verdict::Inconclusive("no flush was sent: T3 not shown".to_owned())
    } else if sends.acked < sends.flushes {
        Verdict::Inconclusive(format!("{} of {} flushes acknowledged: T3 not shown", sends.acked, sends.flushes))
    } else {
        Verdict::Pass(format!("{} sends, each timed; {} closing send(s) with two or more samples; {} flush(es) acknowledged", sends.payload_sends, sends.closing, sends.acked))
    }
}

/// The closed short gap at `rate` S/s on the device's 200 MHz root, `max(3, ceil(15 us x rate))` (RM-15), as the Providers arm their record.
pub fn gap_at(rate: f64) -> i64 {
    ezsdr_radio::staging::closed_short_gap(MCR as u64, (MCR as f64 / rate).round() as u64, 1)
}

/// The record of a transmit stream at `rate` S/s on the device's 200 MHz root, armed with [`gap_at`] as the Providers arm theirs.
pub fn armed_record(rate: f64) -> Bookings {
    let mut bookings = Bookings::unarmed();
    bookings.arm(gap_at(rate));
    bookings
}

/// The shape a long close must have for the 3-sample margin check (step F): among the transmit sends `sends` of a probe (`ProbeDevice::rows`,
/// flushes left out), the send before the last start of burst is the held tail and one zero, two samples on every channel with
/// end-of-burst and no start, and that start is timed exactly `gap` samples (`per` ticks each) past the close's exclusive end. `Err` says what
/// differed, so the trial reports INCONCLUSIVE: a delayed turn that sends the last payload with its own end-of-burst closes differently.
pub fn long_close_shape(sends: &[Json], per: i64, gap: i64) -> Result<(), String> {
    let starts: Vec<usize> = sends.iter().enumerate().filter(|(_, row)| row["sob"] == true).map(|(i, _)| i).collect();
    let (Some(&second), true) = (starts.last(), starts.len() >= 2) else {
        return Err(format!("{} start(s) of burst, two needed", starts.len()));
    };
    let close = &sends[second - 1];
    let counts: Vec<u64> = close["sample_counts"].as_array().map(|counts| counts.iter().filter_map(Json::as_u64).collect()).unwrap_or_default();
    if close["eob"] != true || close["sob"] == true || counts.is_empty() || counts.iter().any(|count| *count != 2) {
        return Err(format!("the send before the second burst is not a two-sample close: {close}"));
    }
    let (Some(close_at), Some(start_at)) = (close["at"].as_i64(), sends[second]["at"].as_i64()) else {
        return Err("the close or the second burst is not timed".to_owned());
    };
    let found = (start_at - close_at) / per - 2;
    if (start_at - close_at) % per != 0 || found != gap {
        return Err(format!("the second burst starts {found} samples past the close's end, {gap} planned"));
    }
    Ok(())
}

/// The record's front of a standalone head of `len` samples at `start` on a stream at `rate` (RM-15, `staging.rs` `front`), read through
/// `Bookings`: the head is accepted and the first sample a booking after it may take is asked of the record.
pub fn head_front(start: i64, len: i64, rate: f64) -> i64 {
    let mut bookings = armed_record(rate);
    bookings.accept(start, Some(len), 0, |_| 0, i64::MIN).expect("the first booking is accepted");
    bookings.first_free(start, false, 0, |_| 0, i64::MIN).expect("a front")
}

/// One 25-sample window of a capture: its start in root ticks, its frequency, whether it carries a coherent tone (coherence
/// over 0.5, as `tone::first_ms_tone`), and its mean power.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub start: i64,
    pub hz: f64,
    pub tone: bool,
    pub power: f64,
}

/// The windows of `samples` (root ticks, `ticks_per_sample` apart) that the capture holds whole.
pub fn windows(samples: &[(i64, Iq)], ticks_per_sample: i64) -> Vec<Window> {
    contiguous_windows_at(samples, ticks_per_sample)
        .map(|w| {
            let (coherence, hz) = window_tone_at(w, ticks_per_sample);
            let power = w.iter().map(|(_, s)| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2)).sum::<f64>() / w.len() as f64;
            Window { start: w[0].0, hz, tone: coherence > 0.5, power }
        })
        .collect()
}

/// The start of the first tone window at or after `from` at `hz` (within `tolerance_hz`), in root ticks, of the windows above the
/// quiet floor before `quiet_until` (`tone_windows`). `None` when a hole in the capture lies between `from` and it: the tone may
/// have begun in the hole, so the onset is not shown.
pub fn tone_onset(windows: &[Window], quiet_until: i64, from: i64, hz: f64, tolerance_hz: f64) -> Option<i64> {
    let onset = tone_windows(windows, quiet_until).iter().find(|w| w.start >= from && (w.hz - hz).abs() <= tolerance_hz)?.start;
    let starts: Vec<i64> = windows.iter().map(|w| w.start).collect();
    covered(&starts, from, onset).then_some(onset)
}

/// The tone windows above the quiet floor: coherent, at four times the mean power of the windows that start before `quiet_until` (the
/// capture before the first transmitted sample). LSB noise is coherent too (`tone.rs` `classify_tone`), so a coherent window alone is not a tone.
pub fn tone_windows(windows: &[Window], quiet_until: i64) -> Vec<Window> {
    let quiet: Vec<f64> = windows.iter().filter(|w| w.start < quiet_until).map(|w| w.power).collect();
    let floor = if quiet.is_empty() { 0.0 } else { quiet.iter().sum::<f64>() / quiet.len() as f64 };
    windows.iter().filter(|w| w.tone && w.power > 4.0 * floor).copied().collect()
}

/// The 25-sample window at 1 Msps in root ticks: the stride of consecutive windows of a capture.
pub const WINDOW_TICKS: i64 = 25 * PER_1MSPS;

/// Whether the windows starting at `starts` (ascending, `WINDOW_TICKS` long) hold every instant of `[from, to)`: a hole in the
/// capture, a span the windows do not cover, breaks it.
pub fn covered(starts: &[i64], from: i64, to: i64) -> bool {
    let mut at = from;
    while at < to {
        match starts.iter().find(|start| **start <= at && at < **start + WINDOW_TICKS) {
            Some(start) => at = start + WINDOW_TICKS,
            None => return false,
        }
    }
    true
}

/// The samples of a capture with their root ticks, span by span (SC-30): `spans` are `(first root tick, sample count)` in the order
/// the capture holds them, one per run of valid samples, so a hole stays a hole. `None` when the spans do not add up to the samples.
pub fn span_samples<T: Copy>(samples: &[T], spans: &[(i64, usize)], per: i64) -> Option<Vec<(i64, T)>> {
    if spans.iter().map(|(_, len)| len).sum::<usize>() != samples.len() {
        return None;
    }
    let mut taken = 0;
    let mut out = Vec::with_capacity(samples.len());
    for (first, len) in spans {
        out.extend((0..*len).map(|k| (first + k as i64 * per, samples[taken + k])));
        taken += len;
    }
    Some(out)
}

/// The start of the last window of a repeat's power that still carries it (T6, S1): `power` holds `(start, mean power)` of
/// consecutive windows `step` apart. The steady power is the mean of the first `steady` windows; the edge is the last window above a
/// quarter of it, and it is judged only when the steady power is over four times the quiet power that follows it (the noise floor
/// measured after the edge) and `quiet` quiet windows follow the edge before the capture ends. A noise-only capture, a capture that
/// ends in signal and a hole before the edge have no edge.
pub fn stop_edge(power: &[(i64, f64)], step: i64, steady: usize, quiet: usize) -> Option<i64> {
    if power.len() < steady || steady == 0 {
        return None;
    }
    let head = power.iter().take(steady).map(|(_, p)| p).sum::<f64>() / steady as f64;
    let last = power.iter().rposition(|(_, p)| *p > head / 4.0)?;
    let tail = &power[last + 1..];
    if tail.len() < quiet || power[..=last + quiet].windows(2).any(|pair| pair[1].0 - pair[0].0 != step) {
        return None;
    }
    let noise = tail.iter().map(|(_, p)| p).sum::<f64>() / tail.len() as f64;
    (head > 4.0 * noise).then_some(power[last].0)
}

/// T3 (b) raw (S3): the late reports after a burst closed with the Module's two zeros at cursor `c`, and the next burst at `c + offset`
/// (`trials` are `(offset, async codes)`). PASS when every trial at `c+5`, the front (the close's end `c+2` plus three), has two
/// acknowledgements and no late or underflow report; FAIL when one of them is late or underflows; INCONCLUSIVE when a trial lacks its
/// acknowledgements. `c+3` and `c+4` are observations the caller prints and does not judge here (the sweep of 2026-11-10 needs one sample
/// at 50 MS/s and below and two or three at 100 and 200 MS/s).
pub fn close_pair_verdict(trials: &[(i64, Vec<String>)]) -> Verdict {
    let judged: Vec<&(i64, Vec<String>)> = trials.iter().filter(|(offset, _)| *offset == 5).collect();
    if judged.is_empty() {
        return Verdict::Inconclusive("no trial at c+5".to_owned());
    }
    let late: Vec<String> = judged
        .iter()
        .filter_map(|(offset, codes)| codes.iter().find(|code| matches!(code.as_str(), "TimeError" | "Underflow" | "UnderflowInPacket")).map(|code| format!("c+{offset}: {code}")))
        .collect();
    if !late.is_empty() {
        return Verdict::Fail(format!("the next burst after the close is late or underflows: {late:?}"));
    }
    let incomplete = judged.iter().filter(|(_, codes)| codes.iter().filter(|code| code.as_str() == "BurstAck").count() != 2).count();
    if incomplete > 0 {
        Verdict::Inconclusive(format!("{incomplete} of {} trial(s) at c+5 without two acknowledgements", judged.len()))
    } else {
        Verdict::Pass(format!("{} trial(s) at c+5, each with two acknowledgements and no late or underflow report", judged.len()))
    }
}

/// The device seams at a repeat switch (S4): each data send's (start of burst, end of burst) flags in order. The first send's start
/// (the Module's own start) and the last send's end (the final Stop) are not seams; any other start or end is.
pub fn seam_count(flags: &[(bool, bool)]) -> usize {
    let last = flags.len().saturating_sub(1);
    flags.iter().enumerate().filter(|(i, (sob, eob))| (*sob && *i > 0) || (*eob && *i < last)).count()
}

/// Several verdicts of one claim: a FAIL in any is the claim's FAIL, else an INCONCLUSIVE, else a PASS. The texts are joined.
pub fn all_of(verdicts: Vec<Verdict>) -> Verdict {
    let text = verdicts
        .iter()
        .map(|verdict| match verdict {
            Verdict::Pass(text) | Verdict::Inconclusive(text) | Verdict::Fail(text) => text.as_str(),
        })
        .collect::<Vec<_>>()
        .join("; ");
    if verdicts.iter().any(|verdict| matches!(verdict, Verdict::Fail(_))) {
        Verdict::Fail(text)
    } else if verdicts.iter().any(|verdict| matches!(verdict, Verdict::Inconclusive(_))) {
        Verdict::Inconclusive(text)
    } else {
        Verdict::Pass(text)
    }
}

/// The raw trials of a booking after a first burst (T2's raw side, T3 (h)): each trial's async codes in order. FAIL on a late or
/// underflow report; INCONCLUSIVE when a trial does not show two acknowledgements (one per burst) or there is no trial; else PASS.
pub fn raw_pair_verdict(trials: &[Vec<String>]) -> Verdict {
    if trials.is_empty() {
        return Verdict::Inconclusive("no trial was run".to_owned());
    }
    let late: Vec<&String> = trials.iter().flatten().filter(|code| matches!(code.as_str(), "TimeError" | "Underflow" | "UnderflowInPacket")).collect();
    if !late.is_empty() {
        return Verdict::Fail(format!("{} late or underflow report(s): {late:?}", late.len()));
    }
    let incomplete = trials.iter().filter(|codes| codes.iter().filter(|code| code.as_str() == "BurstAck").count() != 2).count();
    if incomplete > 0 {
        Verdict::Inconclusive(format!("{incomplete} of {} trial(s) without two acknowledgements", trials.len()))
    } else {
        Verdict::Pass(format!("{} trial(s), each with two acknowledgements and no late or underflow report", trials.len()))
    }
}

/// `hw_txp_b0_send_sizes` (T1): the first timed send takes at most the window's chunk, each continuation takes the chunk or what is
/// left of the waveform, and every closing send has two samples or more.
pub fn repeat_sizes_verdict(sizes: &[usize], closing: &[usize], chunk: usize, waveform_len: usize) -> Verdict {
    if sizes.is_empty() {
        return Verdict::Fail("the repeat sent nothing".to_owned());
    }
    if let Some(short) = closing.iter().find(|count| **count < 2) {
        return Verdict::Fail(format!("a closing send has {short} sample(s): T2"));
    }
    let mut offset = 0;
    for (index, size) in sizes.iter().copied().enumerate() {
        let expected = chunk.min(waveform_len - offset);
        let fits = if index == 0 { (1..=expected).contains(&size) } else { size == expected };
        if !fits {
            return Verdict::Fail(format!("send {index} takes {size} samples at waveform offset {offset}, {expected} expected: {sizes:?}"));
        }
        offset = (offset + size) % waveform_len;
    }
    Verdict::Pass(format!("{} payload sends sized as the window allows; closing sends {closing:?}", sizes.len()))
}

#[cfg(test)]
mod tests {
    use super::super::tone::PER_1MSPS;
    use super::*;

    fn span(values: &[(f32, f32)], per: i64) -> Vec<(i64, Iq)> {
        values.iter().enumerate().map(|(i, (re, im))| (1_000 + i as i64 * per, [*re, *im])).collect()
    }

    /// Eight steps of `steady / 8` (exact in binary): the first step is 12.5 %, the eighth is `steady`.
    fn ramp(steady: f32) -> Vec<(f32, f32)> {
        (0..8).map(|i| (steady * (i as f32 + 1.0) / 8.0, 0.0)).collect()
    }

    #[test]
    fn a_fill_counts_zeros_then_the_ramp_to_ten_and_ninety_percent() {
        let mut values = vec![(0.0, 0.0); 13];
        values.extend(ramp(1.0));
        values.extend(vec![(1.0, 0.0); 40]);
        let fill = restart_fill(&span(&values, PER_1MSPS), PER_1MSPS).expect("the fill is measured");
        assert_eq!(fill.zeros, 13);
        assert_eq!(fill.steady, 1.0);
        // The first non-zero sample is index 13 (12.5 %, so 10 % is reached there); 90 % is reached at the eighth step, index 20.
        assert_eq!((fill.ramp10, fill.ramp90), (0, 7));
    }

    #[test]
    fn a_span_with_no_zeros_has_none_and_its_ramp_starts_at_the_first_sample() {
        let mut values = ramp(1.0);
        values.extend(vec![(1.0, 0.0); 40]);
        let fill = restart_fill(&span(&values, PER_1MSPS), PER_1MSPS).expect("the fill is measured");
        assert_eq!(fill.zeros, 0);
        assert_eq!(fill.ramp90, 7);
    }

    #[test]
    fn a_span_whose_steady_magnitude_is_zero_has_no_fill() {
        // The median of the last half (20 samples) is zero when 11 of them are zero: no steady magnitude to reach 90 % of.
        let mut values = vec![(0.0, 0.0); 31];
        values.extend(vec![(0.5, 0.0); 9]);
        assert!(restart_fill(&span(&values, PER_1MSPS), PER_1MSPS).is_none());
    }

    #[test]
    fn a_gap_in_the_span_has_no_fill() {
        let mut samples = span(&[(0.5, 0.0); 40], PER_1MSPS);
        samples[20].0 += 7;
        assert!(restart_fill(&samples, PER_1MSPS).is_none());
    }

    #[test]
    fn a_span_that_is_all_zero_has_no_fill() {
        assert!(restart_fill(&span(&[(0.0, 0.0); 40], PER_1MSPS), PER_1MSPS).is_none());
    }

    fn trial(applied: f64, first: Option<i64>, covered: bool, tone: Option<f64>, fill: Option<Fill>) -> RestartTrial {
        RestartTrial { drained: true, asked: 1e6, applied, planned: 500, first_tick: first, covered, tone_hz: tone, fill }
    }

    fn filled() -> Option<Fill> {
        Some(Fill { zeros: 13, ramp10: 2, ramp90: 8, steady: 0.5 })
    }

    #[test]
    fn a_rate_passes_with_three_complete_trials_at_their_planned_tick() {
        let good = trial(1e6, Some(500), true, Some(100e3), filled());
        assert!(matches!(restart_rate_verdict(1e6, &[good.clone(), good.clone(), good], 100e3, 10e3), Verdict::Pass(_)));
    }

    #[test]
    fn a_first_block_off_its_planned_tick_fails_even_in_an_incomplete_trial() {
        let late = trial(1e6, Some(560), false, None, None);
        assert!(matches!(restart_rate_verdict(1e6, &[late], 100e3, 10e3), Verdict::Fail(_)));
    }

    #[test]
    fn a_rate_mismatch_never_passes() {
        let mismatch = trial(2e6, Some(500), true, Some(100e3), filled());
        let verdict = restart_rate_verdict(1e6, &[mismatch.clone(), mismatch.clone(), mismatch], 100e3, 10e3);
        assert!(matches!(verdict, Verdict::Inconclusive(ref text) if text.contains("rate mismatch")), "{verdict:?}");
    }

    #[test]
    fn two_complete_trials_are_inconclusive() {
        let good = trial(1e6, Some(500), true, Some(100e3), filled());
        let missing = trial(1e6, None, false, None, None);
        let verdict = restart_rate_verdict(1e6, &[good.clone(), good, missing], 100e3, 10e3);
        assert!(matches!(verdict, Verdict::Inconclusive(ref text) if text.contains("2 of 3")), "{verdict:?}");
    }

    #[test]
    fn an_uncovered_span_is_not_complete() {
        let gap = trial(1e6, Some(500), false, Some(100e3), filled());
        let verdict = restart_rate_verdict(1e6, &[gap.clone(), gap.clone(), gap], 100e3, 10e3);
        assert!(matches!(verdict, Verdict::Inconclusive(ref text) if text.contains("not continuously covered")), "{verdict:?}");
    }

    #[test]
    fn a_trial_whose_old_stream_did_not_drain_is_not_complete() {
        let undrained = RestartTrial { drained: false, ..trial(1e6, Some(500), true, Some(100e3), filled()) };
        let verdict = restart_rate_verdict(1e6, &[undrained.clone(), undrained.clone(), undrained], 100e3, 10e3);
        assert!(matches!(verdict, Verdict::Inconclusive(ref text) if text.contains("did not drain")), "{verdict:?}");
    }

    #[test]
    fn a_missing_tone_is_inconclusive_with_its_reason() {
        let silent = trial(1e6, Some(500), true, None, filled());
        let verdict = restart_rate_verdict(1e6, &[silent.clone(), silent.clone(), silent], 100e3, 10e3);
        assert!(matches!(verdict, Verdict::Inconclusive(ref text) if text.contains("no steady tone")), "{verdict:?}");
    }

    #[test]
    fn a_fault_is_a_fail_and_a_missing_negative_control_is_inconclusive() {
        let clean = Faults { time_errors: 0, underflows: 0, async_faults: 0 };
        assert!(matches!(fault_verdict(clean, 0), Verdict::Pass(_)));
        assert!(matches!(fault_verdict(Faults { time_errors: 1, ..clean }, 0), Verdict::Fail(_)));
        assert!(matches!(fault_verdict(Faults { underflows: 1, ..clean }, 0), Verdict::Fail(_)));
        assert!(matches!(fault_verdict(Faults { async_faults: 1, ..clean }, 0), Verdict::Fail(_)));
        assert!(matches!(fault_verdict(clean, 1), Verdict::Inconclusive(_)));
        assert!(matches!(fault_verdict(Faults { time_errors: 1, ..clean }, 1), Verdict::Pass(_)));
    }

    #[test]
    fn several_runs_fail_on_the_first_fault_and_name_the_run() {
        let clean = Faults { time_errors: 0, underflows: 0, async_faults: 0 };
        assert!(matches!(faults_verdict(&[("heard", clean, 0), ("looped", clean, 0)]), Verdict::Pass(_)));
        let verdict = faults_verdict(&[("heard", clean, 0), ("looped", Faults { time_errors: 2, ..clean }, 0)]);
        assert!(matches!(verdict, Verdict::Fail(ref text) if text.starts_with("looped:")), "{verdict:?}");
        assert!(matches!(faults_verdict(&[("control", clean, 0), ("one", clean, 1)]), Verdict::Inconclusive(_)));
        assert!(matches!(faults_verdict(&[("control", clean, 1), ("looped", Faults { underflows: 1, ..clean }, 0)]), Verdict::Fail(_)));
    }

    #[test]
    fn a_short_first_send_passes_with_one_ack_each_and_fails_on_a_fault() {
        let clean = Faults { time_errors: 0, underflows: 0, async_faults: 0 };
        assert!(matches!(short_first_send_verdict(2, clean, 1, 1), Verdict::Pass(_)));
        assert!(matches!(short_first_send_verdict(2, clean, 0, 1), Verdict::Inconclusive(_)));
        assert!(matches!(short_first_send_verdict(2, clean, 2, 1), Verdict::Inconclusive(_)));
        assert!(matches!(short_first_send_verdict(2, Faults { underflows: 9, ..clean }, 1, 1), Verdict::Fail(_)));
    }

    fn stage_observation(direction: &str, rx_overflow: bool, drops: Json) -> Json {
        json!({
            "streams": [{ "direction": direction, "actual_samples": 10, "async_failures": [] }],
            "rx_overflow": rx_overflow, "tx_underflow_events": 0, "link_drops": drops
        })
    }

    #[test]
    fn a_receive_stage_passes_without_overflow_or_drops() {
        assert!(stage_trial(TrialClass::Success, &stage_observation("rx", false, json!([0]))).is_ok());
        let mut unread = stage_observation("rx", false, json!([0]));
        unread.as_object_mut().unwrap().remove("link_drops");
        assert!(stage_trial(TrialClass::Success, &unread).is_err());
    }

    #[test]
    fn a_receive_stage_with_an_overflow_or_a_drop_is_not_a_success() {
        assert!(stage_trial(TrialClass::Success, &stage_observation("rx", true, json!([0]))).is_err());
        assert!(stage_trial(TrialClass::Success, &stage_observation("rx", false, json!([3]))).is_err());
        assert!(stage_trial(TrialClass::Success, &stage_observation("rx", false, json!([null]))).is_err());
        assert!(stage_trial(TrialClass::DeviceFailure, &stage_observation("rx", false, json!([0]))).is_err());
    }

    #[test]
    fn a_late_control_needs_its_expected_late_cause_and_no_fault() {
        let clean = Faults { time_errors: 1, underflows: 0, async_faults: 0 };
        let late = vec!["late".to_owned()];
        assert!(matches!(late_control_verdict(clean, &late, 1), Verdict::Pass(_)));
        assert!(matches!(late_control_verdict(Faults { time_errors: 0, ..clean }, &[], 0), Verdict::Pass(_)));
        assert!(matches!(late_control_verdict(Faults { time_errors: 0, ..clean }, &[], 1), Verdict::Inconclusive(_)));
        assert!(matches!(late_control_verdict(clean, &["unclosed_burst".to_owned()], 1), Verdict::Fail(_)));
        assert!(matches!(late_control_verdict(clean, &late, 0), Verdict::Fail(_)));
        assert!(matches!(late_control_verdict(Faults { async_faults: 1, ..clean }, &late, 1), Verdict::Fail(_)));
    }

    #[test]
    fn a_summary_counts_the_complete_trials_only() {
        let good = trial(1e6, Some(500), true, Some(100e3), filled());
        let uncovered = RestartTrial { fill: Some(Fill { zeros: 99, ramp10: 1, ramp90: 99, steady: 1.0 }), ..trial(1e6, Some(500), false, Some(100e3), None) };
        let fills = complete_fills(&[good.clone(), uncovered, good], 100e3, 10e3);
        assert_eq!(fills.iter().map(|fill| fill.zeros).collect::<Vec<_>>(), vec![13, 13]);
    }

    #[test]
    fn a_flush_report_is_not_a_device_fault() {
        assert!(!is_async_fault(&json!({ "code": "TimeError", "tx_flush": true })));
        assert!(is_async_fault(&json!({ "code": "TimeError" })));
        assert!(!is_async_fault(&json!({ "code": "BurstAck" })));
    }

    #[test]
    fn a_transmit_stage_needs_accepted_samples_and_no_underflow() {
        assert!(stage_trial(TrialClass::Success, &stage_observation("tx", false, json!([0]))).is_ok());
        let mut underflow = stage_observation("tx", false, json!([0]));
        underflow["tx_underflow_events"] = json!(2);
        assert!(stage_trial(TrialClass::Success, &underflow).is_err());
        let mut silent = stage_observation("tx", false, json!([0]));
        silent["streams"][0]["actual_samples"] = json!(0);
        assert!(stage_trial(TrialClass::Success, &silent).is_err());
    }

    #[test]
    fn a_stage_needs_a_stream_a_direction_and_no_underflow_in_any_stream() {
        assert!(stage_trial(TrialClass::Success, &json!({ "streams": [], "link_drops": [0] })).is_err());
        assert!(stage_trial(TrialClass::Success, &stage_observation("both", false, json!([0]))).is_err());
        let per_stream = |events: u64| {
            let mut observation = stage_observation("tx", false, json!([0]));
            observation.as_object_mut().unwrap().remove("tx_underflow_events");
            observation["streams"][0]["tx_underflow_events"] = json!(events);
            observation
        };
        assert!(stage_trial(TrialClass::Success, &per_stream(0)).is_ok());
        assert!(stage_trial(TrialClass::Success, &per_stream(1)).is_err());
    }
}

#[cfg(test)]
mod stage5_tests {
    use super::*;

    const CLEAN: Faults = Faults { time_errors: 0, underflows: 0, async_faults: 0 };
    const DELAY: i64 = 44;
    const TOL: i64 = 50;
    const MARGIN: i64 = 100_000;
    const B: i64 = 50_000_000;

    /// Coherent 100 kHz windows before `B` and 200 kHz after it, one every 5 000 ticks (25 samples at 1 MS/s).
    fn tone() -> Vec<(i64, f64)> {
        let mut windows: Vec<(i64, f64)> = (0..200).map(|i| (B - 5 * TICKS_PER_MS + i * 5_000, 100e3)).filter(|(t, _)| *t < B - MARGIN).collect();
        windows.extend((0..200).map(|i| (B + MARGIN + i * 5_000, 200e3)).filter(|(t, _)| *t < B + 5 * TICKS_PER_MS));
        windows
    }

    fn power(level_in_transition: f64) -> Vec<(i64, f64)> {
        (0..400).map(|i| (B - 5 * TICKS_PER_MS + i * 5_000, if (B - MARGIN..B + MARGIN).contains(&(B - 5 * TICKS_PER_MS + i * 5_000)) { level_in_transition } else { 0.16 })).collect()
    }

    fn trial<'a>(tone: &'a [(i64, f64)], power: &'a [(i64, f64)], row: &'a str, seams: Option<usize>) -> SwitchTrial<'a> {
        SwitchTrial { refusals: &[], row: Some(row), boundary: Some(B), delay: Some(0), margin: MARGIN, before_hz: 100e3, after_hz: 200e3, tolerance_hz: 10e3, faults: CLEAN, seams, tone, power }
    }

    fn refused() -> Vec<String> {
        vec!["UR-21: a repeat switch needs a playing repeat and no pending switch".to_owned()]
    }

    #[test]
    fn a_coherent_window_in_the_quiet_floor_is_not_a_tone() {
        let quiet = |start: i64, tone: bool, power: f64| Window { start, hz: 0.0, tone, power };
        let windows = [quiet(0, true, 1.0), quiet(5_000, false, 1.0), quiet(10_000, true, 1.0), quiet(20_000, true, 40.0)];
        let kept: Vec<i64> = tone_windows(&windows, 15_000).iter().map(|w| w.start).collect();
        assert_eq!(kept, vec![20_000], "the coherent LSB windows before the signal are not tones; the signal's window is");
    }

    #[test]
    fn a_clean_switch_passes_and_a_phase_transition_at_the_boundary_is_not_a_seam() {
        let (tone, power) = (tone(), power(0.16));
        assert!(matches!(switch_verdict(&trial(&tone, &power, "applied", Some(0))), Verdict::Pass(_)));
    }

    #[test]
    fn a_tone_off_its_frequency_away_from_the_boundary_fails() {
        let (mut tone, power) = (tone(), power(0.16));
        tone[10].1 = 150e3;
        assert!(matches!(switch_verdict(&trial(&tone, &power, "applied", Some(0))), Verdict::Fail(ref text) if text.contains("changes elsewhere")));
    }

    #[test]
    fn windows_far_from_the_boundary_do_not_fail_the_switch() {
        let (mut tone, power) = (tone(), power(0.16));
        tone.push((B - 20 * TICKS_PER_MS, 491_925.0));
        tone.push((B + 30 * TICKS_PER_MS, 0.0));
        assert!(matches!(switch_verdict(&trial(&tone, &power, "applied", Some(0))), Verdict::Pass(_)));
    }

    #[test]
    fn a_power_dip_across_the_boundary_fails() {
        let (tone, power) = (tone(), power(0.0));
        assert!(matches!(switch_verdict(&trial(&tone, &power, "applied", Some(0))), Verdict::Fail(ref text) if text.contains("falls")));
    }

    #[test]
    fn a_short_gap_the_windows_miss_fails_on_the_send_seam() {
        let (tone, power) = (tone(), power(0.16));
        assert!(matches!(switch_verdict(&trial(&tone, &power, "applied", Some(1))), Verdict::Fail(ref text) if text.contains("restarted")));
        assert!(matches!(switch_verdict(&trial(&tone, &power, "applied", None)), Verdict::Inconclusive(_)));
    }

    #[test]
    fn a_switch_not_applied_fails_and_no_tone_is_inconclusive() {
        let (tone, power) = (tone(), power(0.16));
        assert!(matches!(switch_verdict(&trial(&tone, &power, "cancelled", Some(0))), Verdict::Fail(_)));
        assert!(matches!(switch_verdict(&trial(&[], &power, "applied", Some(0))), Verdict::Inconclusive(_)));
    }

    #[test]
    fn an_uncertain_switch_is_inconclusive_and_only_a_seam_or_a_refusal_still_fails_it() {
        // note 49 stage C2 (Q2): the bench cannot tell either; the send's exception is not the switch's failure.
        let (tone, power) = (tone(), power(0.16));
        assert!(matches!(switch_verdict(&trial(&tone, &power, "uncertain", Some(0))), Verdict::Inconclusive(ref text) if text.contains("uncertain")));
        assert!(matches!(switch_verdict(&trial(&tone, &power, "uncertain", Some(1))), Verdict::Fail(ref text) if text.contains("restarted")));
        assert!(matches!(switch_verdict(&SwitchTrial { refusals: &refused(), ..trial(&tone, &power, "uncertain", Some(0)) }), Verdict::Fail(_)));
    }

    #[test]
    fn a_booking_passes_only_admitted_fault_free_and_on_its_onset() {
        assert!(matches!(booking_verdict(true, &[], CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Pass(_)));
        assert!(matches!(booking_verdict(true, &[], CLEAN, None, Some(DELAY), TOL), Verdict::Inconclusive(ref text) if text.contains("on-time part")));
        assert!(matches!(booking_verdict(true, &[], CLEAN, Some(DELAY + 200), Some(DELAY), TOL), Verdict::Fail(_)));
        assert!(matches!(booking_verdict(false, &[], CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)));
        assert!(matches!(booking_verdict(true, &[], Faults { time_errors: 1, ..CLEAN }, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)));
        assert!(matches!(booking_verdict(true, &[], Faults { underflows: 1, ..CLEAN }, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)));
    }

    #[test]
    fn a_provider_refusal_of_a_booking_that_should_play_is_a_fail() {
        let refusal = vec!["UR-21: the burst overlaps an accepted booking".to_owned()];
        let verdict = booking_verdict(true, &refusal, CLEAN, Some(DELAY), Some(DELAY), TOL);
        assert!(matches!(verdict, Verdict::Fail(ref text) if text.contains("overlaps")), "{verdict:?}");
        assert!(matches!(front_case_verdict(true, &refusal, CLEAN, 3, 3, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)));
        assert!(matches!(untimed_repeat_verdict(true, &refusal, true, CLEAN), Verdict::Fail(_)));
        assert!(matches!(untimed_burst_verdict(true, &refusal, Some(1_000), 1_000, CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)));
        assert!(matches!(untimed_burst_verdict(true, &refusal, None, 1_000, CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)), "a refused burst is not placed: a FAIL, not INCONCLUSIVE");
        assert!(matches!(switch_verdict(&SwitchTrial { refusals: &refusal, ..trial(&tone(), &power(0.16), "applied", Some(0)) }), Verdict::Fail(_)));
    }

    #[test]
    fn a_refusal_at_the_kernels_stop_sample_is_inconclusive_and_a_refusal_elsewhere_is_a_fail() {
        let refusal = vec!["UR-21: the burst overlaps an accepted booking".to_owned()];
        let at_stop = stop_booking_verdict(true, &refusal, CLEAN, Some(DELAY), Some(DELAY), TOL);
        assert!(matches!(at_stop, Verdict::Inconclusive(ref text) if text.contains("overlaps")), "{at_stop:?}");
        let other = vec!["UR-21: the burst starts before a booking that may already be handed".to_owned()];
        assert!(matches!(stop_booking_verdict(true, &other, CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)), "another reason");
        let twice = vec![refusal[0].clone(), refusal[0].clone()];
        assert!(matches!(stop_booking_verdict(true, &twice, CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)), "more than one refusal");
        assert!(matches!(stop_booking_verdict(false, &refusal, CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)), "not admitted");
        assert!(matches!(stop_booking_verdict(true, &[], CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Pass(_)));
        assert!(matches!(stop_booking_verdict(true, &[], CLEAN, Some(DELAY + 200), Some(DELAY), TOL), Verdict::Fail(_)), "a late onset stays a FAIL");
    }

    #[test]
    fn a_late_arrival_is_inconclusive_only_when_nothing_else_is_wrong() {
        let late = Faults { time_errors: 1, ..CLEAN };
        let verdict = late_arrival_verdict(&[500_000], &[], late);
        assert!(matches!(verdict, Some(Verdict::Inconclusive(ref text)) if text.contains("host latency")), "{verdict:?}");
        assert!(late_arrival_verdict(&[], &[], late).is_none(), "no late drop of the booking");
        assert!(late_arrival_verdict(&[500_000], &["UR-21: the burst overlaps an accepted booking".to_owned()], late).is_none(), "a refusal names the booking: it arrived on time");
        assert!(late_arrival_verdict(&[500_000], &[], Faults { time_errors: 2, ..CLEAN }).is_none(), "another TIME_ERROR stays a FAIL");
        assert!(late_arrival_verdict(&[500_000], &[], Faults { underflows: 1, ..late }).is_none(), "an underflow stays a FAIL");
    }

    #[test]
    fn a_front_case_needs_two_acknowledgements() {
        assert!(matches!(front_case_verdict(true, &[], CLEAN, 3, 3, Some(DELAY), Some(DELAY), TOL), Verdict::Pass(_)));
        assert!(matches!(front_case_verdict(true, &[], CLEAN, 2, 3, Some(DELAY), Some(DELAY), TOL), Verdict::Inconclusive(_)));
        assert!(matches!(front_case_verdict(false, &[], CLEAN, 3, 3, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)));
        assert_eq!(gap_trial_outcome(2), "complete");
        assert_eq!(gap_trial_outcome(1), "incomplete");
    }

    #[test]
    fn a_held_tail_closes_once_per_device_burst_and_a_refusal_names_the_overlap() {
        assert!(matches!(close_verdict(CLEAN, Some(2), 2, 2), Verdict::Pass(_)));
        assert!(matches!(close_verdict(CLEAN, Some(1), 2, 1), Verdict::Fail(_)), "the held tail's end-of-burst is missing");
        assert!(matches!(close_verdict(CLEAN, None, 2, 0), Verdict::Inconclusive(_)));
        assert!(matches!(close_verdict(CLEAN, Some(2), 2, 1), Verdict::Inconclusive(_)));
        assert!(matches!(close_verdict(Faults { async_faults: 1, ..CLEAN }, Some(2), 2, 2), Verdict::Fail(_)));
        let overlap = vec!["UR-21: the burst overlaps an accepted booking".to_owned()];
        assert!(matches!(refusal_verdict(&overlap), Verdict::Pass(_)));
        assert!(matches!(refusal_verdict(&[]), Verdict::Fail(_)));
        assert!(matches!(refusal_verdict(&["UR-21: the burst begins before the device's start-up".to_owned()]), Verdict::Fail(_)));
    }

    #[test]
    fn an_untimed_repeat_and_burst_pass_only_where_the_record_places_them() {
        assert!(matches!(untimed_repeat_verdict(true, &[], true, CLEAN), Verdict::Pass(_)));
        assert!(matches!(untimed_repeat_verdict(true, &[], false, CLEAN), Verdict::Inconclusive(_)));
        assert!(matches!(untimed_repeat_verdict(false, &[], true, CLEAN), Verdict::Fail(_)));
        assert!(matches!(untimed_burst_verdict(true, &[], Some(1_000), 1_000, CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Pass(_)));
        assert!(matches!(untimed_burst_verdict(true, &[], Some(999), 1_000, CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Fail(_)));
        assert!(matches!(untimed_burst_verdict(true, &[], None, 1_000, CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Inconclusive(_)));
    }

    #[test]
    fn a_stop_end_is_judged_against_the_recorded_end_within_its_tolerance() {
        assert!(matches!(stop_end_verdict(Some(1_000), Some(1_000 + 2_000), 2_000), Verdict::Pass(_)));
        assert!(matches!(stop_end_verdict(Some(1_000), Some(1_000 + 2_001), 2_000), Verdict::Fail(_)));
        assert!(matches!(stop_end_verdict(None, Some(1_000), 2_000), Verdict::Inconclusive(_)));
        assert!(matches!(stop_end_verdict(Some(1_000), None, 2_000), Verdict::Inconclusive(_)));
        assert!(stop_bound_text(Some(10), 0, 5, 0).contains("within the window false"));
    }

    #[test]
    fn a_one_sample_burst_must_leave_the_onset_where_the_control_does() {
        assert!(matches!(onset_control_verdict(Some(44), Some(44), 50), Verdict::Pass(_)));
        assert!(matches!(onset_control_verdict(Some(120), Some(44), 50), Verdict::Fail(_)));
        assert!(matches!(onset_control_verdict(None, Some(44), 50), Verdict::Inconclusive(_)));
    }

    #[test]
    fn a_timed_send_run_passes_only_with_all_its_sends_timed_and_acknowledged() {
        let sends = TimedSends { refused: 0, untimed: 0, short_closing: 0, payload_sends: 3, closing: 1, flushes: 1, acked: 1 };
        assert!(matches!(timed_sends_verdict(Verdict::Pass(String::new()), &sends), Verdict::Pass(_)));
        assert!(matches!(timed_sends_verdict(Verdict::Pass(String::new()), &TimedSends { untimed: 1, ..sends }), Verdict::Fail(_)));
        assert!(matches!(timed_sends_verdict(Verdict::Pass(String::new()), &TimedSends { short_closing: 1, ..sends }), Verdict::Fail(_)));
        assert!(matches!(timed_sends_verdict(Verdict::Pass(String::new()), &TimedSends { acked: 0, ..sends }), Verdict::Inconclusive(_)));
        assert!(matches!(timed_sends_verdict(Verdict::Fail(String::new()), &sends), Verdict::Fail(_)));
    }

    #[test]
    fn a_repeat_sends_its_window_sized_chunks_and_closing_sends_have_two_samples() {
        assert!(matches!(repeat_sizes_verdict(&[2_500, 2_500, 2_500], &[2], 2_500, 7_500), Verdict::Pass(_)));
        assert!(matches!(repeat_sizes_verdict(&[2_500, 2_000], &[], 2_500, 7_500), Verdict::Fail(_)), "the second chunk is the chunk or what is left");
        assert!(matches!(repeat_sizes_verdict(&[2_500], &[1], 2_500, 7_500), Verdict::Fail(_)));
        assert!(matches!(repeat_sizes_verdict(&[], &[], 2_500, 7_500), Verdict::Fail(_)));
    }

    #[test]
    fn the_record_front_is_the_gap_past_the_transmitted_end_of_a_head() {
        // At 1 MS/s the gap is 15 samples; a head longer than 32 fronts at its end plus four.
        assert_eq!(head_front(0, 1, 1e6), 19);
        assert_eq!(head_front(0, 3, 1e6), 19);
        assert_eq!(head_front(0, 4, 1e6), 19);
        assert_eq!(head_front(0, 10, 1e6), 25);
        assert_eq!(head_front(0, 32, 1e6), 47);
        assert_eq!(head_front(0, 30_000, 1e6), 30_004);
        // The gap follows the rate: max(3, ceil(15 us x rate)), written out here from the arithmetic, not from `closed_short_gap`.
        for (rate, gap) in [(1e6, 15), (2e6, 30), (200e6 / 70.0, 43), (10e6, 150), (100e6, 1_500), (200e6, 3_000)] {
            for (n, padded) in [(1i64, 4i64), (2, 4), (3, 4), (4, 4), (10, 10), (32, 32)] {
                assert_eq!(head_front(0, n, rate) - padded, gap, "a standalone head of {n} samples at {rate} S/s");
            }
        }
    }

    #[test]
    fn a_booking_after_a_head_the_stop_cuts_waits_for_its_front() {
        // R5-m5 (h): the head at c−1 starts at the Stop's cut t and is dropped; the region holds it to its own front (RM-16), not to c+5 = stop.
        let (c, stop) = (1_000, 1_005);
        let mut bookings = armed_record(1e6);
        bookings.accept(c - 1, Some(1), 0, |_| 0, i64::MIN).unwrap();
        bookings.halt(c - 1, stop);
        // The head is handed over (start c − 1 < stop − 5), so the region reaches its own front, c + 3 + 15 = c + 18 (RM-16).
        for k in 3..18 {
            assert!(bookings.accept(c + k, Some(3), 0, |_| 0, i64::MIN).is_err(), "a booking at c + {k} is inside the head's front");
        }
        assert!(bookings.accept(c + 18, Some(3), 0, |_| 0, i64::MIN).is_ok(), "c + 18 is the front");
    }

    #[test]
    fn an_onset_is_the_first_coherent_window_at_the_frequency() {
        let at = |start: i64, hz: f64, tone: bool| Window { start, hz, tone, power: 1.0 };
        let w = WINDOW_TICKS;
        let stream = [at(0, 0.0, true), at(w, 100e3, false), at(2 * w, 100e3, true), at(3 * w, 200e3, true)];
        assert_eq!(tone_onset(&stream, 0, 0, 100e3, 10e3), Some(2 * w), "a non-coherent window is not the tone");
        assert_eq!(tone_onset(&stream, 0, 2 * w + 1, 100e3, 10e3), None, "the window before `from` is not searched");
        assert_eq!(tone_onset(&stream, 0, 0, 200e3, 10e3), Some(3 * w));
    }

    #[test]
    fn a_hole_between_the_instant_and_the_onset_is_not_an_onset() {
        let at = |start: i64| Window { start, hz: 100e3, tone: true, power: 1.0 };
        let w = WINDOW_TICKS;
        let holed = [at(3 * w)];
        assert_eq!(tone_onset(&holed, 0, 0, 100e3, 10e3), None, "the tone may have begun in the hole");
        assert_eq!(tone_onset(&[at(0), at(w)], 0, 0, 100e3, 10e3), Some(0));
    }

    #[test]
    fn a_span_covers_only_the_instants_its_windows_hold() {
        let starts = [0, WINDOW_TICKS, 3 * WINDOW_TICKS, 4 * WINDOW_TICKS];
        assert!(covered(&starts, 0, 2 * WINDOW_TICKS), "two adjacent windows");
        assert!(covered(&[0], WINDOW_TICKS - 1, WINDOW_TICKS), "a window covers its last instant");
        assert!(!covered(&starts, 0, 4 * WINDOW_TICKS), "the hole at window 2 breaks coverage");
        assert!(covered(&starts, 3 * WINDOW_TICKS, 5 * WINDOW_TICKS));
        assert!(covered(&starts, 7, 7), "an empty interval is covered");
    }

    #[test]
    fn samples_of_separate_spans_keep_their_own_first_ticks() {
        let samples: Vec<Iq> = (0..5).map(|k| [k as f32, 0.0]).collect();
        let spanned = span_samples(&samples, &[(1_000, 2), (9_000, 3)], 200).unwrap();
        let ticks: Vec<i64> = spanned.iter().map(|(t, _)| *t).collect();
        assert_eq!(ticks, vec![1_000, 1_200, 9_000, 9_200, 9_400], "the second span does not continue the first");
        assert_eq!(spanned[2].1, samples[2]);
        assert!(span_samples(&samples, &[(0, 2)], 200).is_none(), "spans that do not add up to the samples are refused");
    }

    #[test]
    fn a_switch_with_a_hole_at_the_boundary_or_no_measured_delay_is_inconclusive() {
        let (tone, power) = (tone(), power(0.16));
        let holed: Vec<(i64, f64)> = power.iter().copied().filter(|(t, _)| !(B - 1_000_000..B + 1_000_000).contains(t)).collect();
        assert!(matches!(switch_verdict(&trial(&tone, &holed, "applied", Some(0))), Verdict::Inconclusive(ref text) if text.contains("hole")));
        let undelayed = SwitchTrial { delay: None, ..trial(&tone, &power, "applied", Some(0)) };
        assert!(matches!(switch_verdict(&undelayed), Verdict::Inconclusive(ref text) if text.contains("loop delay")));
    }

    #[test]
    fn a_refused_switch_fails_with_its_reason_even_when_it_wrote_no_row() {
        let (tone, power) = (tone(), power(0.16));
        let refusal = refused();
        let refused_switch = SwitchTrial { refusals: &refusal, row: None, boundary: None, ..trial(&tone, &power, "applied", Some(0)) };
        assert!(matches!(switch_verdict(&refused_switch), Verdict::Fail(ref text) if text.contains("repeat switch needs")), "{:?}", switch_verdict(&refused_switch));
        let no_row = SwitchTrial { row: None, boundary: None, ..trial(&tone, &power, "applied", Some(0)) };
        assert!(matches!(switch_verdict(&no_row), Verdict::Fail(ref text) if text.contains("no row")));
    }

    #[test]
    fn a_booking_without_a_measured_delay_is_not_on_time() {
        assert!(matches!(booking_verdict(true, &[], CLEAN, Some(DELAY), None, TOL), Verdict::Inconclusive(ref text) if text.contains("loop delay")));
        assert!(matches!(booking_verdict(true, &[], CLEAN, Some(DELAY), Some(DELAY), TOL), Verdict::Pass(_)), "the delay is the trial's, not a constant");
    }

    #[test]
    fn a_stop_edge_needs_signal_then_quiet_windows_before_the_capture_ends() {
        let step = 100;
        let series = |levels: &[f64]| -> Vec<(i64, f64)> { levels.iter().enumerate().map(|(k, p)| (k as i64 * step, *p)).collect() };
        let signal_then_quiet: Vec<f64> = [vec![1.0; 20], vec![1.0; 10], vec![0.01; 6]].concat();
        assert_eq!(stop_edge(&series(&signal_then_quiet), step, 20, 5), Some(29 * step), "the last window of signal");
        assert_eq!(stop_edge(&series(&[vec![1.0; 20], vec![0.01; 4]].concat()), step, 20, 5), None, "too few quiet windows after the edge");
        assert_eq!(stop_edge(&series(&[vec![1.0; 20], vec![0.01; 5]].concat()), step, 20, 5), Some(19 * step), "exactly the quiet windows");
        assert_eq!(stop_edge(&series(&vec![0.01; 40]), step, 20, 5), None, "noise only: no signal to end");
        assert_eq!(stop_edge(&series(&vec![1.0; 40]), step, 20, 5), None, "a capture that ends in signal");
        let mut holed = series(&signal_then_quiet);
        holed.retain(|(t, _)| *t != 25 * step);
        assert_eq!(stop_edge(&holed, step, 20, 5), None, "a hole before the edge is not judged");
    }

    #[test]
    fn a_long_close_has_the_held_tail_and_one_zero_and_a_second_burst_three_samples_past_it() {
        let send = |at: i64, count: u64, sob: bool, eob: bool| json!({ "call": "tx_send", "at": at, "sample_counts": [count], "sob": sob, "eob": eob });
        let per = 200;
        let first = send(1_000, 20_000, true, false);
        let close = send(1_000 + 19_999 * per, 2, false, true);
        let end = 1_000 + 20_001 * per;
        assert_eq!(long_close_shape(&[first.clone(), close.clone(), send(end + 3 * per, 1_000, true, true)], per, 3), Ok(()));
        assert!(long_close_shape(&[first.clone(), close.clone(), send(end + 4 * per, 1_000, true, true)], per, 3).is_err(), "four past is not the margin");
        assert!(
            long_close_shape(&[first.clone(), send(1_000 + 19_998 * per, 20_000, false, true), send(end + 2 * per, 1_000, true, true)], per, 3).is_err(),
            "the last payload sent with its own end-of-burst"
        );
        assert!(long_close_shape(&[first, close], per, 3).is_err(), "one burst only");
    }

    #[test]
    fn a_close_is_judged_at_c_plus_five_and_c_plus_three_and_four_are_only_printed() {
        let ok = |codes: &[&str]| codes.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>();
        assert!(matches!(close_pair_verdict(&[(5, ok(&["BurstAck", "BurstAck"])), (5, ok(&["BurstAck", "BurstAck"]))]), Verdict::Pass(_)));
        assert!(
            matches!(close_pair_verdict(&[(3, ok(&["BurstAck", "TimeError", "BurstAck"])), (5, ok(&["BurstAck", "BurstAck"]))]), Verdict::Pass(_)),
            "a late c+3 is an observation, not the verdict"
        );
        assert!(matches!(close_pair_verdict(&[(5, ok(&["BurstAck", "TimeError", "BurstAck"]))]), Verdict::Fail(_)));
        assert!(matches!(close_pair_verdict(&[(5, ok(&["BurstAck", "UnderflowInPacket"]))]), Verdict::Fail(_)));
        assert!(matches!(close_pair_verdict(&[(5, ok(&["BurstAck"]))]), Verdict::Inconclusive(_)));
        assert!(matches!(close_pair_verdict(&[(4, ok(&["BurstAck", "BurstAck"]))]), Verdict::Inconclusive(_)), "no judged trial");
    }

    #[test]
    fn the_windows_of_a_capture_carry_their_tone_and_power() {
        let tone: Vec<(i64, Iq)> =
            (0..50).map(|i| (i * 200, [0.4 * (2.0 * std::f32::consts::PI * 100e3 * i as f32 / 1e6).cos(), 0.4 * (2.0 * std::f32::consts::PI * 100e3 * i as f32 / 1e6).sin()])).collect();
        let found = windows(&tone, 200);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|w| w.tone && (w.hz - 100e3).abs() < 1e3), "{found:?}");
        assert!(found.iter().all(|w| (w.power - 0.16).abs() < 1e-3), "{found:?}");
    }

    #[test]
    fn a_seam_is_a_start_or_an_end_other_than_the_first_start_and_the_last_end() {
        assert_eq!(seam_count(&[(true, false), (false, false), (false, true)]), 0);
        assert_eq!(seam_count(&[(true, false), (false, true), (true, false), (false, true)]), 2);
        assert_eq!(seam_count(&[(true, false)]), 0);
        assert_eq!(seam_count(&[]), 0);
    }

    #[test]
    fn several_verdicts_of_one_claim_fail_first_then_are_inconclusive() {
        assert!(matches!(all_of(vec![Verdict::Pass("a".into()), Verdict::Fail("b".into()), Verdict::Inconclusive("c".into())]), Verdict::Fail(ref text) if text == "a; b; c"));
        assert!(matches!(all_of(vec![Verdict::Pass("a".into()), Verdict::Inconclusive("c".into())]), Verdict::Inconclusive(_)));
        assert!(matches!(all_of(vec![Verdict::Pass("a".into())]), Verdict::Pass(_)));
    }

    #[test]
    fn a_raw_pair_passes_only_with_two_acknowledgements_and_no_late_report() {
        let ok = |codes: &[&str]| codes.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>();
        assert!(matches!(raw_pair_verdict(&[ok(&["BurstAck", "BurstAck"])]), Verdict::Pass(_)));
        assert!(matches!(raw_pair_verdict(&[ok(&["BurstAck", "TimeError", "BurstAck"])]), Verdict::Fail(_)));
        assert!(matches!(raw_pair_verdict(&[ok(&["BurstAck", "UnderflowInPacket"])]), Verdict::Fail(_)));
        assert!(matches!(raw_pair_verdict(&[ok(&["BurstAck"])]), Verdict::Inconclusive(_)));
        assert!(matches!(raw_pair_verdict(&[]), Verdict::Inconclusive(_)));
    }
}
