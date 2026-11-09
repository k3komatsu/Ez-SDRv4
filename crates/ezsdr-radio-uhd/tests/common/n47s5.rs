//! Note 47 stage 5's bench sequences (T2's Module case, T3, T4, T5, T6's edge), shared by `hardware.rs` (on the USRP) and `fake.rs` (a
//! rehearsal on `FakeDevice`). A sequence takes the Session it runs in (and the `ProbeDevice` it reads the sends from); the pure decisions
//! are in `iii.rs`. The loop's delay is measured in each trial from the onset of its first burst (S5); nothing assumes a constant.

#![allow(dead_code)]

use std::sync::Arc;

use ezsdr_kernel::coordinator::RunHandle;
use ezsdr_kernel::event::ActionId;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::session::{LogEntry, Outcome};
use ezsdr_kernel::spec::Scalar;
use ezsdr_kernel::time::TimePoint;
use ezsdr_radio::payloads::CommandRejectedPayload;
use ezsdr_radio_uhd::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS};
use ezsdr_radio_uhd::{Device, Iq};
use serde_json::Value as Json;

use super::iii;
use super::probe_device::ProbeDevice;
use super::raw::{TICKS_PER_MS, tone_wave};
use super::tone::{PER_1MSPS, Verdict};
use super::*;

/// B6's loop delay (INFERRED). The stage 5 verdicts measure their own per trial; this is printed beside them and used by the older B tests.
pub const LOOPBACK_DELAY_SAMPLES: i64 = 44;

pub const ONSET_TOLERANCE: i64 = 50;

pub const TONE_TOLERANCE_HZ: f64 = 10e3;

/// The half-width of a repeat switch's transition: 500 µs (INFERRED: the loop delay's spread plus the DUC's rise).
pub const SWITCH_MARGIN_NS: i64 = 500_000;

/// T6's edge: 20 windows of the repeat set its steady power; 5 quiet windows (0.5 ms at 1 Msps) must follow the edge. That is longer
/// than the 100-sample power window and the loop's delay (INFERRED: 44 samples), and shorter than the in-flight window (10 ms).
pub const STOP_STEADY_WINDOWS: usize = 20;
pub const STOP_QUIET_WINDOWS: usize = 5;

/// The root ticks a span of `ns` nanoseconds takes at the master clock.
pub fn ns_ticks(ns: i64) -> i64 {
    ns * MCR / 1_000_000_000
}

/// The current instant on the root clock.
pub fn now_root(run: &RunHandle) -> i64 {
    let now = run.now();
    now.ticks_in(now.domain()).unwrap()
}

/// The running transmit clock's origin (root ticks) and root ticks a sample.
pub fn tx_origin_per(run: &RunHandle) -> (i64, i64) {
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.to_string().ends_with("usrp/tx/s0") && r.ended_at.is_none()).unwrap();
    (clock.origin.ticks_in(clock.root).unwrap(), clock.root_ticks_per_tick.num() as i64)
}

/// The `rec` capture of a Manifest as root-tick samples. Each run of valid samples keeps its own first instant (SC-30): a hole stays
/// a hole, and no sample after it is placed as if the hole were not there.
pub fn captured_samples(manifest: &Manifest) -> Vec<(i64, Iq)> {
    let (origin, per) = rx_clock(manifest);
    let artifact = capture_of(manifest, "rec");
    let map = &artifact.continuity[0];
    let spans: Vec<(i64, usize)> = map.valid[0].iter().map(|segment| (origin + segment.start.ticks_in(map.domain).unwrap() * per, segment.len as usize)).collect();
    let samples: Vec<Iq> = read_capture(&artifact, 1).remove(0).into_iter().map(|(re, im)| [re, im]).collect();
    iii::span_samples(&samples, &spans, per).expect("the capture holds the samples its continuity map names")
}

/// The 25-sample windows of the `rec` capture at the 1 Msps receive rate of these tests.
pub fn windows_of(manifest: &Manifest) -> Vec<iii::Window> {
    iii::windows(&captured_samples(manifest), PER_1MSPS)
}

/// The mean power of each 100-sample window of the `rec` capture that holds 100 consecutive samples, with its first instant (T6).
pub fn captured_power(manifest: &Manifest) -> Vec<(i64, f64)> {
    captured_samples(manifest)
        .chunks_exact(100)
        .filter(|chunk| chunk[99].0 - chunk[0].0 == 99 * PER_1MSPS)
        .map(|chunk| (chunk[0].0, chunk.iter().map(|(_, s)| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2)).sum::<f64>() / 100.0))
        .collect()
}

/// The root tick of the first transmitted burst in a Manifest: the quiet floor is the capture before it.
pub fn first_burst_root(manifest: &Manifest) -> i64 {
    let (origin, per) = stream_clock(manifest, "usrp/tx/s0");
    section(manifest, "bursts").as_array().unwrap().iter().map(|row| origin + target_sample(row) * per).min().unwrap()
}

/// The tone windows of a Manifest's `rec` capture above the quiet floor before its first transmitted burst (`iii::tone_windows`).
pub fn tone_windows_of(manifest: &Manifest) -> Vec<iii::Window> {
    iii::tone_windows(&windows_of(manifest), first_burst_root(manifest))
}

/// The first onset of a tone at `hz` at or after `from` (root ticks), on the trial's capture (`iii::tone_onset`, S7).
pub fn tone_onset_of(manifest: &Manifest, from: i64, hz: f64) -> Option<i64> {
    iii::tone_onset(&windows_of(manifest), first_burst_root(manifest), from, hz, TONE_TOLERANCE_HZ)
}

/// The loop's delay of a trial in root ticks, from the onset at `hz` of its first burst (S5); `None` when the capture shows no onset.
pub fn first_delay(manifest: &Manifest, hz: f64) -> Option<i64> {
    let target = first_burst_root(manifest);
    tone_onset_of(manifest, target - ms(2), hz).map(|onset| onset - target)
}

/// `first_delay` in samples of the 1 Msps receive clock: the unit the booking verdicts compare their onsets in.
pub fn first_delay_samples(manifest: &Manifest, hz: f64) -> Option<i64> {
    first_delay(manifest, hz).map(|delay| delay / PER_1MSPS)
}

/// The applied target of a `bursts` row, in samples of its transmit clock.
pub fn target_sample(row: &Json) -> i64 {
    row["target"]["ticks"].as_i64().unwrap()
}

/// The reasons of the `radio.COMMAND_REJECTED` events that name one of the entry's dispatched Actions, booking refusals only (RM-22):
/// the Provider refuses an Action the Kernel has admitted, and the refusal is judged from its event, not from the admission.
pub fn refusals_of(manifest: &Manifest, entry: &LogEntry) -> Vec<String> {
    let dispatched: Vec<ActionId> = match &entry.outcome {
        Outcome::Admitted { dispatched, .. } => dispatched.clone(),
        _ => Vec::new(),
    };
    events_of(manifest, ezsdr_radio::kinds::COMMAND_REJECTED)
        .iter()
        .filter_map(|event| serde_json::from_value::<CommandRejectedPayload>(event.payload.clone()).ok())
        .filter(|payload| payload.kind == "tx_burst" && payload.action.is_some_and(|action| dispatched.contains(&action)))
        .map(|payload| payload.reason)
        .collect()
}

/// The late amounts (ns) of the `radio.TIME_ERROR`s that dropped the booking at `sample` (the target's sample) as late (`cause` late,
/// `outcome` drop). The late-arrival branch of T3 (f) reads them (`iii::late_arrival_verdict`).
pub fn late_arrivals(manifest: &Manifest, sample: i64) -> Vec<i64> {
    events_of(manifest, "radio.TIME_ERROR")
        .iter()
        .filter(|event| event.payload["cause"] == "late" && event.payload["outcome"] == "drop" && event.payload["target"]["ticks"].as_i64() == Some(sample))
        .filter_map(|event| event.payload["late_by_ns"].as_i64())
        .collect()
}

/// The device's acknowledgements of data bursts (flushes left out).
pub fn burst_acks(manifest: &Manifest) -> usize {
    section(manifest, "async").as_array().unwrap().iter().filter(|row| row["code"] == "BurstAck" && row["tx_flush"] != true).count()
}

/// The Session of a transmit trial: `profile` with its transmit sample rate asked at connect (note 47 stage 2), or the profile's own rate.
pub fn transmit_run(profile: &Json, device: Arc<dyn Device>, rate: Option<f64>) -> RunHandle {
    let initial = match rate {
        Some(rate) => with_initial(rx0_tx0(profile), &[("radio.tx.s0.sample_rate_hz", Scalar::try_from(rate).unwrap())]),
        None => rx0_tx0(profile),
    };
    let mut run = session_with(profile, device, &initial);
    past_t0(&mut run, ms(1));
    past_tx_origin(&mut run);
    run
}

/// One trial of T2's Module case. A calibration burst of 2 000 samples at 100 kHz starts at 15 ms, the head of `samples` samples at
/// 40 ms, and a 10 000-sample 100 kHz tone at the record's front after it (`iii::head_front`); all are booked before the head is handed.
/// The calibration burst is the trial's first burst, so its onset is the loop delay of the trial (S5); the capture starts at 5 ms (S7).
pub struct FrontTrial {
    pub admitted: bool,
    /// The Provider's refusals of the booked tone (`refusals_of`): a refused booking that should play is a FAIL.
    pub refusals: Vec<String>,
    pub manifest: Manifest,
    /// The tone's booked instant in root ticks.
    pub burst: i64,
}

pub fn front_trial(mut run: RunHandle, samples: usize, rate: f64) -> FrontTrial {
    let (calibration, _) = waveform_of(&tone_wave(rate, 2_000, 100e3));
    let (head, _) = waveform_of(&vec![(0.4f32, 0.0f32); samples]);
    let (tone, _) = waveform_of(&tone_wave(rate, 10_000, 100e3));
    let cal = tx_at(&run, ms(15));
    let a = tx_at(&run, ms(40));
    let domain = a.domain();
    let a_k = a.ticks_in(domain).unwrap();
    let b = TimePoint::new(domain, a_k + iii::head_front(0, samples as i64));
    let capture_at = after_ticks(&run, ms(5));
    assert!(admitted(&run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(80_000))]), None).unwrap()));
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(cal), &[]), Some(&calibration)).unwrap()));
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(a), &[]), Some(&head)).unwrap()));
    let entry = run.submit(verb("send", resource("radio/tx/s0"), Some(b), &[]), Some(&tone)).unwrap();
    let burst = transmit_root(&run, &b);
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let manifest = run.finish();
    FrontTrial { admitted: admitted(&entry), refusals: refusals_of(&manifest, &entry), manifest, burst }
}

/// T2's verdict for one trial: admitted and not refused, the device's three acknowledgements (calibration, head, tone), the faults, and
/// the tone's onset at the booked instant against the trial's loop delay (S7).
pub fn front_outcome(trial: &FrontTrial) -> Verdict {
    let onset = tone_onset_of(&trial.manifest, trial.burst - ms(2), 100e3).map(|t| (t - trial.burst) / PER_1MSPS);
    iii::front_case_verdict(trial.admitted, &trial.refusals, iii::run_faults(&trial.manifest), burst_acks(&trial.manifest), 3, onset, first_delay_samples(&trial.manifest, 100e3), ONSET_TOLERANCE)
}

/// One trial of T4: a 100 kHz repeat of 1 100 samples, then the switch to a 200 kHz repeat of 1 100 samples, untimed (`timed` false) or
/// at a future instant (`timed` true). The capture starts before the first repeat is submitted (S5); the probe records the device's sends.
pub struct SwitchEvidence {
    pub manifest: Manifest,
    pub probe: Arc<ProbeDevice>,
    pub admitted: bool,
    pub refusals: Vec<String>,
}

/// The first repeat is timed 15 ms after the capture starts (S5): the capture covers the first burst's target − 2 ms, so the loop's
/// delay is measured in every trial. The switch follows 20 ms later (untimed, or timed at 40 ms).
pub fn repeat_switch_trial(mut run: RunHandle, probe: Arc<ProbeDevice>, timed: bool) -> SwitchEvidence {
    let (first, _) = waveform_of(&tone_wave(1e6, 1_100, 100e3));
    let (second, _) = waveform_of(&tone_wave(1e6, 1_100, 200e3));
    let capture_at = after_ticks(&run, ms(1));
    assert!(admitted(&run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(120_000))]), None).unwrap()));
    assert!(admitted(&run.submit(verb("start_repeat", resource("radio/tx/s0"), Some(tx_at(&run, ms(15))), &[]), Some(&first)).unwrap()));
    wait(&mut run, ms(20));
    let at = timed.then(|| tx_at(&run, ms(40)));
    let mark = [("radio.repeat_operation", ezsdr_kernel::spec::Value::Scalar(ezsdr_kernel::spec::Scalar::from("switch")))];
    let entry = run.submit(verb("start_repeat", resource("radio/tx/s0"), at, &mark), Some(&second)).unwrap();
    wait(&mut run, ms(60));
    let _ = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None);
    wait(&mut run, ms(20));
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let manifest = run.finish();
    SwitchEvidence { refusals: refusals_of(&manifest, &entry), admitted: admitted(&entry), manifest, probe }
}

pub fn repeat_switch_verdict(trial: &SwitchEvidence) -> Verdict {
    if !trial.admitted {
        return Verdict::Fail("the switch was refused at receipt".to_owned());
    }
    let row = section(&trial.manifest, "switches").as_array().and_then(|rows| rows.first()).cloned();
    let outcome = row.as_ref().map(|row| row["outcome"].as_str().unwrap_or("").to_owned());
    let boundary = row.as_ref().and_then(|row| row["moved_to"].as_i64().or(row["boundary"].as_i64()));
    let (origin, per) = stream_clock(&trial.manifest, "usrp/tx/s0");
    let windows = windows_of(&trial.manifest);
    let delay = first_delay(&trial.manifest, 100e3);
    let tone: Vec<(i64, f64)> = iii::tone_windows(&windows, first_burst_root(&trial.manifest)).iter().map(|w| (w.start, w.hz)).collect();
    let power: Vec<(i64, f64)> = windows.iter().map(|w| (w.start, w.power)).collect();
    let flags: Vec<(bool, bool)> = data_sends(&trial.probe).iter().map(|row| (row["sob"] == true, row["eob"] == true)).collect();
    let seams = (!flags.is_empty()).then(|| iii::seam_count(&flags));
    println!("T4 switch: row {outcome:?}, boundary {boundary:?} samples (before the loop delay), loop delay {delay:?} ticks, seams {seams:?}, sends {flags:?}, refusals {:?}", trial.refusals);
    if let (Some(boundary), Some(delay)) = (boundary, delay) {
        let b = origin + boundary * per + delay;
        let near: Vec<String> = tone.iter().filter(|(t, _)| (t - b).abs() <= 10 * TICKS_PER_MS).map(|(t, hz)| format!("({:.1}, {hz:.0})", (t - b) as f64 * 1e3 / TICKS_PER_MS as f64)).collect();
        let far: Vec<f64> = tone.iter().filter(|(t, _)| (t - b).abs() > 10 * TICKS_PER_MS).map(|(_, hz)| *hz).collect();
        let range =
            if far.is_empty() { "none".to_owned() } else { format!("{:.0} to {:.0} Hz", far.iter().cloned().fold(f64::INFINITY, f64::min), far.iter().cloned().fold(f64::NEG_INFINITY, f64::max)) };
        println!("T4 switch tone windows near the boundary (us, Hz): [{}]; {} farther, Hz {range}", near.join(", "), far.len());
    }
    iii::switch_verdict(&iii::SwitchTrial {
        refusals: &trial.refusals,
        row: outcome.as_deref(),
        boundary: boundary.map(|boundary| origin + boundary * per),
        delay,
        margin: ns_ticks(SWITCH_MARGIN_NS),
        before_hz: 100e3,
        after_hz: 200e3,
        tolerance_hz: TONE_TOLERANCE_HZ,
        faults: iii::run_faults(&trial.manifest),
        seams,
        tone: &tone,
        power: &power,
    })
}

/// A stream clock of a Manifest, by the stream's path suffix: its origin (root ticks) and root ticks a sample.
pub fn stream_clock(manifest: &Manifest, suffix: &str) -> (i64, i64) {
    let clock = manifest.clocks.sample_clocks.iter().find(|r| r.stream.to_string().ends_with(suffix)).unwrap();
    (clock.origin.ticks_in(clock.root).unwrap(), clock.root_ticks_per_tick.num() as i64)
}

/// The receive SampleClock's origin (root ticks) and root ticks a sample.
pub fn rx_clock(manifest: &Manifest) -> (i64, i64) {
    stream_clock(manifest, "usrp/rx/s0")
}

/// The root tick of a transmit instant on the running transmit clock.
pub fn transmit_root(run: &RunHandle, at: &TimePoint) -> i64 {
    let (origin, per) = tx_origin_per(run);
    origin + at.ticks_in(at.domain()).unwrap() * per
}

/// The transmit sends of a probe, flushes left out. Zero-length sends stay in: a closing one is judged by its count (T2).
pub fn data_sends(probe: &ProbeDevice) -> Vec<Json> {
    probe.rows().into_iter().filter(|row| row["call"] == "tx_send" && row["tx_flush"] != true).collect()
}

/// T5 (1): an untimed repeat, stopped, then an untimed repeat again. The second is admitted and its tone is heard, with no fault.
pub fn untimed_repeat_outcome(mut run: RunHandle) -> Verdict {
    let (first, _) = waveform_of(&tone_wave(1e6, 1_100, 100e3));
    let (second, _) = waveform_of(&tone_wave(1e6, 1_100, 200e3));
    let capture_at = after_ticks(&run, ms(1));
    assert!(admitted(&run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(120_000))]), None).unwrap()));
    let started = run.submit(verb("start_repeat", resource("radio/tx/s0"), None, &[]), Some(&first)).unwrap();
    wait(&mut run, ms(20));
    let _ = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None);
    wait(&mut run, ms(5));
    let again = run.submit(verb("start_repeat", resource("radio/tx/s0"), None, &[]), Some(&second)).unwrap();
    wait(&mut run, ms(60));
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let manifest = run.finish();
    let tone_found = tone_windows_of(&manifest).iter().any(|w| (w.hz - 200e3).abs() <= TONE_TOLERANCE_HZ);
    let first_refusals = refusals_of(&manifest, &started);
    let first_ok = if admitted(&started) && first_refusals.is_empty() {
        Verdict::Pass("the first untimed repeat is admitted and not refused".to_owned())
    } else {
        Verdict::Fail(format!("the first untimed repeat was refused: {first_refusals:?}"))
    };
    iii::all_of(vec![first_ok, iii::untimed_repeat_verdict(admitted(&again), &refusals_of(&manifest, &again), tone_found, iii::run_faults(&manifest))])
}

/// T5 (2): an untimed 1 000-sample burst booked while a timed 30 000-sample one plays. It is placed after the timed one (the record's
/// front, `head_front`), admitted, and its tone is at its applied target (S7), against the loop delay of the timed burst (S5).
pub fn untimed_burst_outcome(mut run: RunHandle) -> Verdict {
    let (timed, _) = waveform_of(&tone_wave(1e6, 30_000, 100e3));
    let (untimed, _) = waveform_of(&tone_wave(1e6, 1_000, 200e3));
    let (origin, per) = tx_origin_per(&run);
    let capture_at = after_ticks(&run, ms(1));
    assert!(admitted(&run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(120_000))]), None).unwrap()));
    let a = tx_at(&run, ms(40));
    let a_k = a.ticks_in(a.domain()).unwrap();
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(a), &[]), Some(&timed)).unwrap()));
    let a_root = transmit_root(&run, &a);
    while now_root(&run) < a_root + ms(5) {
        wait(&mut run, ms(1) / 4);
    }
    let b = run.submit(verb("send", resource("radio/tx/s0"), None, &[]), Some(&untimed)).unwrap();
    wait(&mut run, ms(80));
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let manifest = run.finish();
    let bursts = section(&manifest, "bursts").as_array().unwrap().clone();
    let placed = bursts.iter().find(|row| row["samples"] == 1_000).map(target_sample);
    let onset = placed.and_then(|k| tone_onset_of(&manifest, origin + k * per - ms(2), 200e3).map(|t| (t - (origin + k * per)) / per));
    let delay = first_delay_samples(&manifest, 100e3);
    println!("T5 (2): the untimed burst placed at sample {placed:?} after the timed one at {a_k}; onset {onset:?}; loop delay {delay:?} samples");
    iii::untimed_burst_verdict(admitted(&b), &refusals_of(&manifest, &b), placed, iii::head_front(a_k, 30_000), iii::run_faults(&manifest), onset, delay, ONSET_TOLERANCE)
}

/// `hw_b8_burst_one_sample_after_a_burst` for one case: a 30 000-sample burst A plays, and a 1 000-sample burst is booked at sample
/// `offset` after A's start (30 001 is A's end + 1, refused at receipt by the Provider, RM-15; 30 002 plays on time). `late_booking`
/// books it after A's last buffer went out (8 ms before A's end).
pub fn one_sample_after_a_burst(mut run: RunHandle, offset: i64, late_booking: bool) -> Verdict {
    wait(&mut run, ms(1));
    let a = tx_at(&run, ms(40));
    let (first, _) = waveform_of(&pn(30_000));
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(a), &[]), Some(&first)).unwrap()));
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.to_string().ends_with("usrp/tx/s0") && r.ended_at.is_none()).unwrap();
    let a_end = clock.origin.ticks_in(clock.root).unwrap() + (a.ticks_in(clock.domain).unwrap() + 30_000) * clock.root_ticks_per_tick.num() as i64;
    if late_booking {
        while run.now().ticks_in(clock.root).unwrap() < a_end - ms(8) {
            wait(&mut run, ms(1) / 4);
        }
    }
    let b = TimePoint::new(a.domain(), a.ticks_in(clock.domain).unwrap() + offset);
    let (second, _) = waveform_of(&pn(1_000));
    let entry = run.submit(verb("send", resource("radio/tx/s0"), Some(b), &[]), Some(&second)).unwrap();
    wait(&mut run, ms(80));
    let manifest = run.finish();
    println!(
        "B8 burst at end + {} samples, booked late {late_booking}: {:?}; refusals {:?}; TIME_ERROR {:?}; async {}; bursts {}",
        offset - 30_000,
        entry.outcome,
        refusals_of(&manifest, &entry),
        events_of(&manifest, "radio.TIME_ERROR").iter().map(|e| e.payload.clone()).collect::<Vec<_>>(),
        section(&manifest, "async"),
        section(&manifest, "bursts")
    );
    let faults = iii::late_control_verdict(iii::run_faults(&manifest), &iii::time_error_causes(&manifest), 0);
    let refusals = refusals_of(&manifest, &entry);
    if offset == 30_001 { iii::all_of(vec![iii::refusal_verdict(&refusals), faults]) } else { iii::all_of(vec![iii::admission_verdict(admitted(&entry), &refusals), faults]) }
}

/// One trial of T3 (f): a 30 000-sample tone burst A at `a`, ending at `e` (its held tail closes at `J(e)` = `e` − 5 ms, `join_by` in
/// `src/provider/core.rs`, from the record's device lead and delivery allowance), and a 10 000-sample 100 kHz tone C at `e + c_offset`.
/// C is booked before `J(e)` (`after_join` None), or after it: submitted `after_join` ticks after `J(e)` (Some(0) is the first wait step
/// after `J(e)`). The Run's dir is kept until the verdict is read.
pub struct HeldTrial {
    pub manifest: Manifest,
    pub probe: Arc<ProbeDevice>,
    pub admitted: bool,
    /// The refusals of C that name its Action (`refusals_of`).
    pub refusals: Vec<String>,
    /// The late amounts (ns) of the drops of C as late (`late_arrivals`).
    pub late: Vec<i64>,
    pub c_root: i64,
    _dir: TempDir,
}

pub fn held_tail_trial(mut run: RunHandle, probe: Arc<ProbeDevice>, dir: TempDir, c_offset: i64, after_join: Option<i64>) -> HeldTrial {
    let (a_wave, _) = waveform_of(&tone_wave(1e6, 30_000, 200e3));
    let (c_wave, _) = waveform_of(&tone_wave(1e6, 10_000, 100e3));
    let (origin, per) = tx_origin_per(&run);
    let a = tx_at(&run, ms(40));
    let domain = a.domain();
    let e = a.ticks_in(domain).unwrap() + 30_000;
    let e_root = origin + e * per;
    let j_root = e_root - ns_ticks(DEVICE_LEAD_NS + DELIVERY_ALLOWANCE_NS);
    let capture_at = after_ticks(&run, ms(10));
    assert!(admitted(&run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(120_000))]), None).unwrap()));
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(a), &[]), Some(&a_wave)).unwrap()));
    if let Some(lag) = after_join {
        while now_root(&run) < j_root + lag {
            wait(&mut run, ms(1) / 4);
        }
    }
    let submit_after_join = now_root(&run) - j_root;
    let c = TimePoint::new(domain, e + c_offset);
    let entry = run.submit(verb("send", resource("radio/tx/s0"), Some(c), &[]), Some(&c_wave)).unwrap();
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let manifest = run.finish();
    let late = late_arrivals(&manifest, e + c_offset);
    println!(
        "T3 (f) C at e+{c_offset}, after_join {after_join:?}: submitted {:.3} ms after J(e); refusals {:?}; late drops {late:?} ns",
        submit_after_join as f64 / TICKS_PER_MS as f64,
        refusals_of(&manifest, &entry)
    );
    HeldTrial { refusals: refusals_of(&manifest, &entry), admitted: admitted(&entry), late, manifest, probe, c_root: origin + (e + c_offset) * per, _dir: dir }
}

/// The verdict of a held-tail trial that plays C: the late-arrival branch first (`iii::late_arrival_verdict`), then the device's
/// end-of-burst sends (flushes left out) against the device bursts expected, the acknowledgements, and C's onset at its booked instant
/// against the trial's loop delay (S7).
pub fn held_played_verdict(trial: &HeldTrial, expected_bursts: usize) -> Verdict {
    let faults = iii::run_faults(&trial.manifest);
    if let Some(verdict) = iii::late_arrival_verdict(&trial.late, &trial.refusals, faults) {
        return verdict;
    }
    let eobs = data_sends(&trial.probe).iter().filter(|row| row["eob"] == true).count();
    let onset = tone_onset_of(&trial.manifest, trial.c_root - ms(2), 100e3).map(|t| (t - trial.c_root) / PER_1MSPS);
    iii::all_of(vec![
        iii::close_verdict(faults, Some(eobs), expected_bursts, burst_acks(&trial.manifest)),
        iii::booking_verdict(trial.admitted, &trial.refusals, faults, onset, first_delay_samples(&trial.manifest, 200e3), ONSET_TOLERANCE),
    ])
}

/// The verdict of a held-tail trial whose C is booked at e+1 after J(e): refused at receipt by the Provider (the late-arrival branch
/// first: a late drop is host latency), and A alone plays.
pub fn held_refused_verdict(trial: &HeldTrial) -> Verdict {
    let faults = iii::run_faults(&trial.manifest);
    if let Some(verdict) = iii::late_arrival_verdict(&trial.late, &trial.refusals, faults) {
        return verdict;
    }
    let eobs = data_sends(&trial.probe).iter().filter(|row| row["eob"] == true).count();
    iii::all_of(vec![iii::refusal_verdict(&trial.refusals), iii::close_verdict(faults, Some(eobs), 1, burst_acks(&trial.manifest))])
}

/// One trial of T3 (b): a 100 kHz repeat stopped by `Stop`; the next burst, a 1 000-sample 200 kHz tone, is booked at the record's first
/// sample after the Stop (`staging::stop_sample`, RM-16, with the in-flight window of the profile). The Stop's instant is the Stop
/// entry's `time` (INFERRED: root ticks; the Stop is the control's `now`, `control.rs:310–321`).
pub struct StopTrial {
    pub manifest: Manifest,
    pub probe: Arc<ProbeDevice>,
    pub admitted: bool,
    /// The Provider's refusals of the next burst (`refusals_of`).
    pub refusals: Vec<String>,
    /// The Stop's instant in root ticks (its entry's `time`).
    pub stop_root: i64,
    pub next_root: i64,
    _dir: TempDir,
}

/// The repeat is timed 15 ms after the capture starts (S5, as `repeat_switch_trial`), and the Stop follows 30 ms after its submission.
pub fn stop_next_trial(mut run: RunHandle, probe: Arc<ProbeDevice>, dir: TempDir) -> StopTrial {
    let (repeat, _) = waveform_of(&tone_wave(1e6, 1_100, 100e3));
    let (next, _) = waveform_of(&tone_wave(1e6, 1_000, 200e3));
    let capture_at = after_ticks(&run, ms(1));
    assert!(admitted(&run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(120_000))]), None).unwrap()));
    assert!(admitted(&run.submit(verb("start_repeat", resource("radio/tx/s0"), Some(tx_at(&run, ms(15))), &[]), Some(&repeat)).unwrap()));
    wait(&mut run, ms(30));
    let stop = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None).unwrap();
    let stop_root = stop.time.ticks_in(stop.time.domain()).unwrap();
    let (origin, per) = tx_origin_per(&run);
    let domain = tx_at(&run, ms(40)).domain();
    let in_flight = ns_ticks(ezsdr_radio::x3xx::timing(&ezsdr_radio_uhd::profile::terms()).tx_in_flight_ns);
    let next_k = ezsdr_radio::staging::stop_sample(stop_root, in_flight, |t| sample_at_or_after(origin, per, t));
    let entry = run.submit(verb("send", resource("radio/tx/s0"), Some(TimePoint::new(domain, next_k)), &[]), Some(&next)).unwrap();
    wait(&mut run, ms(80));
    let manifest = run.finish();
    StopTrial { refusals: refusals_of(&manifest, &entry), manifest, probe, admitted: admitted(&entry), stop_root, next_root: origin + next_k * per, _dir: dir }
}

/// The verdict of T3 (b)'s Session case: the booking's admission and refusals (`iii::stop_booking_verdict`, S3), the faults, and the next
/// tone's onset against the repeat's loop delay.
pub fn stop_next_verdict(trial: &StopTrial) -> Verdict {
    let onset = tone_onset_of(&trial.manifest, trial.next_root - ms(2), 200e3).map(|t| (t - trial.next_root) / PER_1MSPS);
    iii::stop_booking_verdict(trial.admitted, &trial.refusals, iii::run_faults(&trial.manifest), onset, first_delay_samples(&trial.manifest, 100e3), ONSET_TOLERANCE)
}

/// The sample of a clock at or after root tick `root`, on a clock whose sample `k` is at `origin + k * per` (the record's `at_or_after`).
pub fn sample_at_or_after(origin: i64, per: i64, root: i64) -> i64 {
    (root - origin + per - 1).div_euclid(per)
}

/// T6's edge in the trial's capture: the root tick the loop stops hearing the repeat, its last 100-sample window's end less the loop
/// delay (`first_delay`). The edge is searched from the first repeat's onset + 1 ms on, so its steady windows are the repeat's signal.
/// `None` when no edge is shown (`iii::stop_edge`) or the delay is not measured.
pub fn heard_until(manifest: &Manifest) -> Option<i64> {
    let delay = first_delay(manifest, 100e3)?;
    let from = first_burst_root(manifest) + delay + ms(1);
    let power: Vec<(i64, f64)> = captured_power(manifest).into_iter().filter(|(start, _)| *start >= from).collect();
    let edge = iii::stop_edge(&power, 100 * PER_1MSPS, STOP_STEADY_WINDOWS, STOP_QUIET_WINDOWS)?;
    Some(edge + 100 * PER_1MSPS - delay)
}

/// The repeat's recorded end in root ticks: the first `bursts` row's applied target plus its transmitted samples (`None` when the
/// Manifest has no row).
pub fn recorded_end(manifest: &Manifest) -> Option<i64> {
    let (origin, per) = stream_clock(manifest, "usrp/tx/s0");
    section(manifest, "bursts").as_array()?.first().map(|row| origin + (target_sample(row) + row["samples"].as_i64().unwrap()) * per)
}

/// One trial of T6 (`hw_b8_stop_end`): a 100 kHz repeat of 1 000 samples, timed 15 ms after the capture starts (S5), and a `Stop` 70 ms
/// after the capture's start. `submit_root` is the instant the Stop is submitted, `stop_root` the Stop's own instant (its entry's `time`).
pub struct StopEnd {
    pub manifest: Manifest,
    pub admitted: bool,
    pub refusals: Vec<String>,
    pub submit_root: i64,
    pub stop_root: i64,
    _dir: TempDir,
}

pub fn stop_end_trial(mut run: RunHandle, dir: TempDir) -> StopEnd {
    let (bytes, _) = waveform_of(&tone_wave(1e6, 1_000, 100e3));
    let at = after_ticks(&run, ms(5));
    assert!(admitted(&run.submit(verb("capture", output("rec"), Some(at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(100_000))]), None).unwrap()));
    let repeat = run.submit(verb("start_repeat", resource("radio/tx/s0"), Some(tx_at(&run, ms(15))), &[]), Some(&bytes)).unwrap();
    let _ = run.advance_to(TimePoint::new(at.domain(), at.ticks_in(at.domain()).unwrap() + ms(70)));
    let submit_root = now_root(&run);
    let stop = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None).unwrap();
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let manifest = run.finish();
    StopEnd { refusals: refusals_of(&manifest, &repeat), admitted: admitted(&repeat), manifest, submit_root, stop_root: stop.time.ticks_in(stop.time.domain()).unwrap(), _dir: dir }
}

/// T6's tolerance in root ticks: the 100-sample edge resolution plus the 50-sample loop-delay spread (both INFERRED), at the transmit clock.
pub fn stop_end_tolerance(manifest: &Manifest) -> i64 {
    (100 + 50) * stream_clock(manifest, "usrp/tx/s0").1
}

/// T6's verdict for one trial: the repeat is admitted and not refused (a FAIL otherwise), and the loop stops hearing it at its recorded end
/// within the tolerance (`iii::stop_end_verdict`).
pub fn stop_end_outcome(trial: &StopEnd) -> Verdict {
    iii::all_of(vec![iii::admission_verdict(trial.admitted, &trial.refusals), iii::stop_end_verdict(heard_until(&trial.manifest), recorded_end(&trial.manifest), stop_end_tolerance(&trial.manifest))])
}
