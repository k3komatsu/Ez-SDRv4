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
use super::tone::{Verdict, first_ms_frequency};
use super::{effective_layout, events_of, section, set_selector_override};

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
