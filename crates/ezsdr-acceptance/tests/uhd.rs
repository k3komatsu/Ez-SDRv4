//! The Vision §59 rehearsal on `FakeDevice` (plan/phase7/00-overview.md §7, §8): the
//! Phase 2–6 experiments under a UHD profile, assembled by the server's catalogue, with
//! the fake device in place of a USRP. The same steps on an X310 are bench B3–B7.

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use ezsdr_acceptance::{experiments, rig};
use ezsdr_kernel::coordinator::{self, RunHandle};
use ezsdr_kernel::event::EventKind;
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::manifest::{ArtifactRef, Manifest};
use ezsdr_kernel::run::Lease;
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::BurstRecord;
use ezsdr_kernel::time::TimePoint;
use ezsdr_radio_uhd::{Device, FakeConfig, FakeDevice};
use serde_json::{json, Value as Json};
use support::{artifact, finish_at, modules, root, section, spec_run, T0};

/// Root ticks per millisecond: the X310's 200 MHz master clock (UR-7).
const MS: i64 = 200_000;

fn fake() -> Arc<dyn Device> {
    Arc::new(FakeDevice::new(FakeConfig::default()))
}

fn envelope() -> Json {
    json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999_000_000.0, "hi_hz": 1_001_000_000.0 }], "max_gain_db": 0.0, "tx_enabled": [true] } })
}

fn uhd_spec_run(temp: &rig::TempDir, spec: &Json, inputs: BTreeMap<ContentHash, Vec<u8>>) -> RunHandle {
    let profile = rig::uhd_profile(&temp.0, false, envelope());
    coordinator::start_spec_run(spec, &profile, rig::assemble_on(&profile, inputs, fake())).expect("Spec Run starts")
}

fn uhd_session(temp: &rig::TempDir) -> RunHandle {
    let profile = rig::uhd_profile(&temp.0, true, envelope());
    let mut run = coordinator::connect(&profile, rig::assemble_on(&profile, BTreeMap::new(), fake()), Lease::attached()).expect("Session connects");
    let t0 = run.start_instant().expect("a start instant");
    run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + MS)).unwrap();
    run
}

/// Runs a Spec Run for `ms` after T0, in the device's time, and finishes it.
fn finish_after(mut run: RunHandle, ms: i64) -> Manifest {
    let t0 = run.start_instant().expect("a start instant");
    let _ = run.run_until_end(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ms * MS));
    run.finish()
}

fn written() -> EventKind {
    EventKind::parse(ezsdr_sink::CAPTURE_WRITTEN).unwrap()
}

/// Waits up to three seconds of the device's time for the next capture the Session writes.
fn wait_capture(run: &mut RunHandle, from: usize) {
    let now = run.now();
    let found = run.wait_for(&[written()], from, TimePoint::new(now.domain(), now.ticks_in(now.domain()).unwrap() + 3_000 * MS)).unwrap();
    assert!(found.is_some(), "no capture was written");
}

fn capture(n: i64, at: Option<TimePoint>) -> SessionAction {
    SessionAction::Vocabulary {
        ns: Namespace::parse("sink").unwrap(),
        verb: Ident::parse("capture").unwrap(),
        target: ResourceId::parse("rec").unwrap(),
        at,
        params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::from(n))]),
    }
}

fn set(key: &str, value: Value) -> SessionAction {
    SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse(key).unwrap(), value }
}

fn admitted(run: &mut RunHandle, action: SessionAction, waveform: Option<&[u8]>) {
    let entry = run.submit(action, waveform).unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "{:?}", entry.outcome);
}

/// A QPSK pseudo-noise waveform, so that each offset of it is unambiguous.
fn pn(n: usize) -> Vec<(f32, f32)> {
    let mut state: u32 = 0x1234_5678;
    (0..n)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let bit = |b: u32| if state >> b & 1 == 1 { 0.4 } else { -0.4 };
            (bit(30), bit(29))
        })
        .collect()
}

fn capture_artifact(manifest: &Manifest) -> &ArtifactRef {
    manifest.artifacts.iter().find(|a| a.id.as_str().starts_with("rec")).unwrap_or_else(|| panic!("no capture: {:?}", manifest.artifacts))
}

fn samples(artifact: &ArtifactRef) -> Vec<(f32, f32)> {
    rig::read_capture(artifact, 1).remove(0)
}

#[test]
fn uhd_59_one_spec_mock_and_uhd_profiles() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(10_000));
    let temp = rig::TempDir::new("uhd-59-mock");
    let mock_profile = rig::spec_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &mock_profile, BTreeMap::new());
    let clock = root(&run);
    let mock = finish_at(run, clock, T0 + 20_000_000);
    let uhd_temp = rig::TempDir::new("uhd-59-uhd");
    let uhd = finish_after(uhd_spec_run(&uhd_temp, &spec, BTreeMap::new()), 50);
    // The same Spec document, byte for byte; only the profile changed (§58 #1, §59).
    assert_eq!(mock.spec.hash, uhd.spec.hash);
    assert_eq!(mock.spec.body, uhd.spec.body);
    assert!(modules(&mock).contains(&"ezsdr.radio.mock"));
    assert!(modules(&uhd).contains(&"ezsdr.radio.uhd"));
    assert_eq!(uhd.run.execution_class, Some(ezsdr_kernel::module_api::ExecutionClass::HardwareInLoop));
    for manifest in [&mock, &uhd] {
        assert_eq!(artifact(manifest, "rec").size_bytes, 80_000, "10 000 cf32 samples");
    }
}

#[test]
fn uhd_59_receive_capture_starts_at_the_requested_index() {
    let mut spec = experiments::receive(1, 1.0e6, 1.0e9, None);
    spec["schedule"] = json!([]);
    let spec = experiments::with_timed_capture(spec, 10_000, 50_000);
    let temp = rig::TempDir::new("uhd-59-index");
    let manifest = finish_after(uhd_spec_run(&temp, &spec, BTreeMap::new()), 100);
    let map = &capture_artifact(&manifest).continuity[0];
    assert_eq!((map.first.ticks_in(map.first.domain()).unwrap(), map.end.ticks_in(map.end.domain()).unwrap()), (50_000, 60_000));
}

#[test]
fn uhd_59_a_scheduled_burst_is_recorded() {
    let wave = pn(1_000);
    let (bytes, waveform) = experiments::waveform_of(&wave);
    let spec = experiments::transmit(1.0e6, &waveform, false, "drop_and_flag", 5_000, None);
    let temp = rig::TempDir::new("uhd-59-burst");
    let run = uhd_spec_run(&temp, &spec, BTreeMap::from([(waveform.hash, bytes)]));
    let t0 = run.start_instant().unwrap();
    let manifest = finish_after(run, 50);
    let bursts: Vec<BurstRecord> = serde_json::from_value(section(&manifest, "ezsdr.radio.uhd.usrp.bursts").clone()).unwrap();
    assert_eq!(bursts.len(), 1, "{bursts:?}");
    assert_eq!(bursts[0].samples, 1_000);
    let clock = manifest.clocks.sample_clocks.iter().find(|r| r.domain == bursts[0].target.domain()).expect("the burst's SampleClock");
    let n = clock.root_ticks_per_tick.num() as i64;
    assert_eq!(clock.origin.ticks_in(clock.origin.domain()).unwrap() + bursts[0].target.ticks_in(bursts[0].target.domain()).unwrap() * n, t0.ticks_in(t0.domain()).unwrap() + 5_000 * n, "5 000 samples after T0");
    assert!(!manifest.events.delivered.iter().any(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR));
}

#[test]
fn uhd_59_a_session_retune_outside_the_rf_envelope_is_rejected() {
    let temp = rig::TempDir::new("uhd-59-rf");
    let mut run = uhd_session(&temp);
    admitted(&mut run, set("radio.tx.channels", Value::from(1)), None);
    let entry = run.submit(set("radio.tx.frequency_hz", Value::num(2.4e9).unwrap()), None).unwrap();
    let Outcome::Rejected { violations } = &entry.outcome else { panic!("outside the band was admitted") };
    assert!(violations.iter().any(|v| v.check.as_str() == "radio.rf_envelope" && v.key.as_ref().is_some_and(|k| k.as_str() == "radio.tx.frequency_hz")));
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("radio.tx.frequency_hz").unwrap()], Value::num(1.0e9).unwrap());
    let manifest = run.finish();
    assert!(manifest.action_log.iter().any(|logged| logged.seq == entry.seq && matches!(logged.outcome, Outcome::Rejected { .. })), "the rejection is logged");
}

#[test]
fn uhd_57_the_session_loopback_captures_what_it_transmits() {
    let temp = rig::TempDir::new("uhd-57");
    let mut run = uhd_session(&temp);
    let wave = pn(1_000);
    let (bytes, _) = experiments::waveform_of(&wave);
    admitted(&mut run, set("radio.tx.channels", Value::from(1)), None);
    let repeat = SessionAction::Vocabulary {
        ns: Namespace::parse("radio").unwrap(),
        verb: Ident::parse("start_repeat").unwrap(),
        target: ResourceId::parse("radio/tx").unwrap(),
        at: None,
        params: BTreeMap::new(),
    };
    admitted(&mut run, repeat, Some(&bytes));
    let from = run.events(0).len();
    let now = run.now();
    // Ahead of the Run's time, past the repeat's restart lead (EA-17, UR-25).
    admitted(&mut run, capture(3_000, Some(TimePoint::new(now.domain(), now.ticks_in(now.domain()).unwrap() + 100 * MS))), None);
    wait_capture(&mut run, from);
    let manifest = run.finish();
    let captured = samples(capture_artifact(&manifest));
    assert_eq!(captured.len(), 3_000);
    // QPSK takes four values, so the offset is found from a run of 32 samples.
    let offset = (0..1_000).find(|k| (0..32).all(|i| captured[i] == wave[(k + i) % 1_000])).expect("the capture holds the waveform");
    for (index, sample) in captured.iter().enumerate() {
        assert_eq!(*sample, wave[(offset + index) % 1_000], "captured sample {index}");
    }
}

#[test]
fn uhd_61_01_repeat_is_continuous_across_the_wrap() {
    let wave = pn(1_000);
    let (bytes, waveform) = experiments::waveform_of(&wave);
    let spec = experiments::with_timed_capture(experiments::transmit(1.0e6, &waveform, true, "send_asap_and_flag", 10_000, None), 5_000, 10_500);
    let temp = rig::TempDir::new("uhd-61-01");
    let manifest = finish_after(uhd_spec_run(&temp, &spec, BTreeMap::from([(waveform.hash, bytes)])), 100);
    // Five wraps of the waveform, sample for sample: the repeat starts at 10 000, the
    // capture at 10 500 (v3 behaviour 1).
    let captured = samples(capture_artifact(&manifest));
    assert_eq!(captured.len(), 5_000);
    for (index, sample) in captured.iter().enumerate() {
        assert_eq!(*sample, wave[(500 + index) % 1_000], "captured sample {index}");
    }
}

#[test]
fn uhd_61_02_capture_starts_at_the_requested_sample_index() {
    let temp = rig::TempDir::new("uhd-61-02");
    let mut run = uhd_session(&temp);
    // The receive SampleClock is registered at its first block (RM-25, UR-17).
    let first_block = TimePoint::new(run.now().domain(), run.now().ticks_in(run.now().domain()).unwrap() + 10 * MS);
    run.advance_to(first_block).unwrap();
    let rx = run.sample_clocks().into_iter().find(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).expect("the receive SampleClock");
    let n = rx.root_ticks_per_tick.num() as i64;
    // A sample index 100 ms ahead of the Run's time (v3's capture at a sample index).
    let index = (run.now().ticks_in(run.now().domain()).unwrap() - rx.origin.ticks_in(rx.origin.domain()).unwrap()) / n + 100_000;
    let from = run.events(0).len();
    admitted(&mut run, capture(100, Some(TimePoint::new(rx.domain, index))), None);
    wait_capture(&mut run, from);
    let manifest = run.finish();
    let map = &capture_artifact(&manifest).continuity[0];
    assert_eq!((map.first.ticks_in(map.first.domain()).unwrap(), map.end.ticks_in(map.end.domain()).unwrap()), (index, index + 100));
}

#[test]
fn uhd_61_03_timed_start_of_tx_and_capture() {
    let wave = pn(1_000);
    let (bytes, waveform) = experiments::waveform_of(&wave);
    let spec = experiments::with_timed_capture(experiments::transmit(1.0e6, &waveform, false, "drop_and_flag", 5_000, None), 1_000, 5_000);
    let temp = rig::TempDir::new("uhd-61-03");
    let manifest = finish_after(uhd_spec_run(&temp, &spec, BTreeMap::from([(waveform.hash, bytes)])), 50);
    // The burst and the capture both start at sample 5 000 after T0: the capture is the burst.
    let capture = capture_artifact(&manifest);
    assert_eq!(capture.continuity[0].valid[0][0].start.ticks_in(capture.continuity[0].valid[0][0].start.domain()).unwrap(), 5_000);
    assert_eq!(samples(capture), wave);
}
