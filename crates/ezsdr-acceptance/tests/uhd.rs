//! The Vision §59 rehearsal on `FakeDevice` (plan/phase7/00-overview.md §7, §8): the
//! Phase 2–6 experiments under a UHD profile, assembled by the server's catalogue, with
//! the fake device in place of a USRP. The same steps on an X310 are bench B3–B7.

mod support;

// The shared per-test wall watchdog (plan 30 §7, R05): a manual clock a mutant freezes would
// hang the test until the harness's timeout; this aborts instead. One implementation, shared
// with `ezsdr-radio-uhd`'s `tests/common` and `ezsdr-server`'s `tests/protocol.rs`.
#[path = "../../ezsdr-radio-uhd/tests/common/watchdog.rs"]
mod watchdog;

// The Run a UHD test drives (note 51 §6.2): names a real-time bound at the call that met it. `run.rs` takes `creation_limit` and `infra` from this root.
#[path = "../../ezsdr-radio-uhd/tests/common/run.rs"]
mod run;

use std::collections::BTreeMap;
use std::sync::Arc;

use ezsdr_acceptance::{experiments, rig};
use ezsdr_kernel::event::EventKind;
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::manifest::{ArtifactRef, Manifest};
use ezsdr_kernel::run::{Lease, StopCause, Termination};
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Scalar, Value};
use ezsdr_kernel::stream::BurstRecord;
use ezsdr_kernel::time::TimePoint;
use ezsdr_radio::payloads::{RxOverflowCause, RxOverflowPayload};
use ezsdr_radio_uhd::{Device, FakeConfig, FakeDevice, FakeFault};
use run::{Declared, Expect, Run};
use serde_json::{Value as Json, json};
use support::{T0, artifact, finish_at, modules, output, resource, root, section, source, spec_run};
use watchdog::infra::{creation_limit, infra, rig_delay, timing};

/// Root ticks per millisecond: the X310's 200 MHz master clock (UR-7).
const MS: i64 = 200_000;

fn fake() -> Arc<dyn Device> {
    fake_device()
}

fn fake_device() -> Arc<FakeDevice> {
    // The smoke set runs on the manual clock (plan 30 §4.4), which is what the watchdog
    // bounds (R05): every Run of this file builds its device here, before it starts.
    watchdog::watch();
    Arc::new(FakeDevice::new(FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..FakeConfig::default() }))
}

fn envelope() -> Json {
    json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999_000_000.0, "hi_hz": 1_001_000_000.0 }], "max_gain_db": 0.0, "tx_enabled": [true] } })
}

fn uhd_spec_run(temp: &rig::TempDir, spec: &Json, inputs: BTreeMap<ContentHash, Vec<u8>>) -> Run {
    let profile = rig::uhd_profile(&temp.0, false, envelope());
    Run::start_spec(spec, &profile, rig::assemble_on(&profile, inputs, fake()), Declared::default()).expect("Spec Run starts")
}

fn uhd_session(temp: &rig::TempDir) -> Run {
    uhd_session_asking(temp, support::rx0)
}

/// A Session that receives and transmits on c0, both asked for at `connect` (RS-12).
fn uhd_loopback_session(temp: &rig::TempDir) -> Run {
    uhd_session_asking(temp, support::rx0_tx0)
}

fn uhd_session_asking(temp: &rig::TempDir, asking: fn(&ezsdr_kernel::coordinator::Assembly) -> ezsdr_kernel::session::InitialRequests) -> Run {
    let profile = rig::uhd_profile(&temp.0, true, envelope());
    let assembly = rig::assemble_on(&profile, BTreeMap::new(), fake());
    let mut run = Run::connect(&profile, &asking(&assembly), assembly, Lease::attached(), Declared::default()).expect("Session connects");
    let t0 = run.start_instant().expect("a start instant");
    run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + MS)).unwrap();
    run
}

/// Runs a Spec Run for `ms` after T0, in the device's time, and finishes it.
fn finish_after(mut run: Run, ms: i64) -> Manifest {
    let t0 = run.start_instant().expect("a start instant");
    let _ = run.run_until_end(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ms * MS));
    run.finish()
}

/// A Mock Spec Run of `spec` under the `x3xx` profile with `environment`, advanced to
/// `horizon_ms` after T0 (the acceptance Mock root is 1 GHz, so a millisecond is 1 000 000
/// ticks). `advance_to` schedules its horizon as a wakeup: `run_until_end` would run the round at
/// the Mock's next own wakeup — past a 140 ms gap — before it compares the horizon.
fn mock_spec_manifest(temp: &rig::TempDir, spec: &Json, environment: Json, horizon_ms: i64) -> Manifest {
    let profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, environment);
    let mut run = spec_run(temp, spec, &profile, BTreeMap::new());
    let clock = root(&run);
    // `Ended` is the expected answer for a Run the Policy stopped before its horizon.
    let _ = run.advance_to(TimePoint::new(clock, T0 + horizon_ms * 1_000_000));
    run.finish()
}

/// A UHD Spec Run on `device`, its binding asking for `block_len` samples a receive call (the
/// one input a test reads a device's prefix from: a scripted fault fires at the next call).
fn uhd_spec_run_on(temp: &rig::TempDir, spec: &Json, block_len: i64, device: Arc<dyn Device>, declared: Declared) -> Run {
    let mut profile = rig::uhd_profile(&temp.0, false, envelope());
    profile["bindings"]["radio"]["selector"]["block_len"] = json!(block_len);
    let assembly = rig::assemble_on(&profile, BTreeMap::new(), device);
    Run::start_spec(spec, &profile, assembly, declared).expect("Spec Run starts")
}

/// The UHD Spec Run's T0 in device ticks, read from a throwaway Run of the same Spec on a fresh
/// fake: a scripted fault is the fake's device time since its zero, and T0 is known only after a
/// Run has armed. The manual clock runs at real time while its participants are idle (host.rs
/// M5), so two builds of one Run differ by their construction time; a caller places a fault far
/// enough from a call boundary that the difference cannot move it, and asserts the difference is
/// inside that margin.
fn spec_t0(spec: &Json) -> i64 {
    let temp = rig::TempDir::new("uhd-59-t0");
    let run = uhd_spec_run(&temp, spec, BTreeMap::new());
    let t0 = run.start_instant().expect("a start instant");
    let ticks = t0.ticks_in(t0.domain()).unwrap();
    let _ = finish_after(run, 1);
    ticks
}

/// The fake's device time `ticks` after its zero, as a scripted fault takes it: `FakeDevice::ticks_of`
/// turns it into master-clock ticks (5 ns a tick at 200 MHz), so the nanosecond value is exact.
fn device_ns(ticks: i64) -> std::time::Duration {
    std::time::Duration::from_nanos((i128::from(ticks) * 1_000_000_000 / (MS as i128 * 1_000)) as u64)
}

/// A manual-clock fake with scripted faults.
fn fake_with_faults(faults: Vec<FakeFault>) -> Arc<dyn Device> {
    watchdog::watch();
    Arc::new(FakeDevice::new(FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, faults, ..FakeConfig::default() }))
}

/// Asserts a Run's T0 is within `margin` of the probe's: the fault a test placed from the probe's
/// T0 stays where it was derived.
fn started_within(run: &Run, t0: i64, margin: i64) {
    let started = run.start_instant().expect("a start instant");
    let ticks = started.ticks_in(started.domain()).unwrap();
    // A drift past the margin is a missed timing precondition (the manual clock runs at real time while its participants idle), not a verdict on the code under test: the runners re-run the test alone.
    if (ticks - t0).abs() >= margin {
        timing(&format!("the UHD Run's T0 drifted {} ticks from the probe's (margin {margin})", ticks - t0));
    }
    assert!((ticks - t0).abs() < margin, "the UHD Run's T0 drifted {} ticks from the probe's (margin {margin})", ticks - t0);
}

fn written() -> EventKind {
    EventKind::parse(ezsdr_sink::CAPTURE_WRITTEN).unwrap()
}

/// Waits up to three seconds of the device's time for the next capture the Session writes.
fn wait_capture(run: &mut Run, from: usize) {
    let now = run.now();
    let found = run.wait_for(&[written()], from, TimePoint::new(now.domain(), now.ticks_in(now.domain()).unwrap() + 3_000 * MS)).unwrap();
    assert!(found.is_some(), "no capture was written");
}

fn capture(n: i64, at: Option<TimePoint>) -> SessionAction {
    SessionAction::Vocabulary {
        ns: Namespace::parse("sink").unwrap(),
        verb: Ident::parse("capture").unwrap(),
        target: output("rec"),
        at,
        params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::from(n))]),
    }
}

fn set(key: &str, value: Value) -> SessionAction {
    SessionAction::SetParameter { target: resource("radio"), key: Key::parse(key).unwrap(), value }
}

fn admitted(run: &mut Run, action: SessionAction, waveform: Option<&[u8]>) {
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
    manifest.artifacts.iter().find(|(output, _)| output.as_str() == "rec").and_then(|(_, list)| list.first()).unwrap_or_else(|| panic!("no capture: {:?}", manifest.artifacts))
}

fn samples(artifact: &ArtifactRef) -> Vec<(f32, f32)> {
    rig::read_capture(artifact, 1).remove(0)
}

#[test]
fn uhd_59_one_spec_mock_and_uhd_profiles() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(10_000));
    let temp = rig::TempDir::new("uhd-59-mock");
    let mock_profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
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
        assert_eq!(artifact(manifest, "rec", 0).size_bytes, 80_000, "10 000 cf32 samples");
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
    let bursts: Vec<BurstRecord> = serde_json::from_value(section(&manifest, "usrp", "ezsdr.radio.uhd.bursts").clone()).unwrap();
    assert_eq!(bursts.len(), 1, "{bursts:?}");
    assert_eq!(bursts[0].samples, 1_000);
    let clock = manifest.clocks.sample_clocks.iter().find(|r| r.domain == bursts[0].target.domain()).expect("the burst's SampleClock");
    let n = clock.root_ticks_per_tick.num() as i64;
    assert_eq!(
        clock.origin.ticks_in(clock.origin.domain()).unwrap() + bursts[0].target.ticks_in(bursts[0].target.domain()).unwrap() * n,
        t0.ticks_in(t0.domain()).unwrap() + 5_000 * n,
        "5 000 samples after T0"
    );
    assert!(!manifest.events.delivered.iter().any(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR));
}

#[test]
fn uhd_59_a_value_outside_the_rf_envelope_is_refused_at_connect_and_at_runtime() {
    // Note 47 stage 4: the envelope is checked on the initial values and on a transmit value during the Run (RM-19, RM-21, RM-27: `cold`). The
    // receive form is the next test.
    let (_, refused) = session_on(&[("radio.tx.c0.stream", Scalar::Int(0)), ("radio.tx.c0.frequency_hz", num(2.4e9))], envelope(), 10, refused());
    assert!(
        matches!(refused.state(), ezsdr_kernel::run::RunState::CleanedUp { termination: ezsdr_kernel::run::Termination::Failed { reason, .. } } if reason.contains("radio.rf_envelope")),
        "{:?}",
        refused.state()
    );
    let _ = refused.finish();
    let temp = rig::TempDir::new("uhd-59-rf");
    let mut run = uhd_loopback_session(&temp);
    let entry = run.submit(set("radio.tx.c0.frequency_hz", Value::num(2.4e9).unwrap()), None).unwrap();
    let Outcome::Rejected { violations } = &entry.outcome else { panic!("a runtime radio value change was admitted") };
    assert!(violations.iter().any(|violation| violation.check.as_str() == "radio.rf_envelope"), "{violations:?}");
    assert!(run.effective()[&Ident::parse("radio").unwrap()].get(&Key::parse("radio.tx.c0.frequency_hz").unwrap()) != Some(&Value::num(2.4e9).unwrap()));
    let manifest = run.finish();
    assert!(manifest.action_log.iter().any(|logged| logged.seq == entry.seq && matches!(logged.outcome, Outcome::Rejected { .. })), "the rejection is logged");
}

#[test]
fn uhd_59_a_receive_antenna_outside_the_rf_envelope_is_refused_on_a_stopped_stream_without_a_device_call() {
    // §58 #16 on the UHD Module, receive form: on a stopped stream an antenna outside the envelope is refused at admission, logged, and never
    // dispatched, so the device is not touched (RM-19, RM-21); one inside it is staged.
    let mut envelope = envelope();
    envelope["radio.rf_envelope"]["antenna_ports"] = json!(["RX2"]);
    let (device, mut run) = session_on(&[("radio.rx.c0.stream", Scalar::Int(0))], envelope, 10, Declared::default());
    admitted(&mut run, SessionAction::Stop { target: Some(resource("radio/rx/s0")) }, None);
    let calls = device.calls().len();
    let entry = run.submit(set("radio.rx.c0.antenna", Value::from("TX/RX")), None).unwrap();
    let Outcome::Rejected { violations } = &entry.outcome else { panic!("an antenna outside the envelope was admitted: {:?}", entry.outcome) };
    assert!(violations.iter().any(|v| v.check.as_str() == "radio.rf_envelope"), "{violations:?}");
    admitted(&mut run, set("radio.rx.c0.antenna", Value::from("RX2")), None);
    assert!(!device.calls()[calls..].iter().any(|call| call.starts_with("apply")), "{:?}", device.calls());
    let manifest = run.finish();
    assert!(manifest.action_log.iter().any(|logged| logged.seq == entry.seq && matches!(logged.outcome, Outcome::Rejected { .. })), "the rejection is logged");
    assert!(!manifest.events.delivered.iter().any(|e| e.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED), "nothing was dispatched, so the Provider refused nothing");
}

#[test]
fn uhd_57_the_session_loopback_captures_what_it_transmits() {
    let temp = rig::TempDir::new("uhd-57");
    let mut run = uhd_loopback_session(&temp);
    let wave = pn(1_000);
    let (bytes, _) = experiments::waveform_of(&wave);
    let repeat = SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: resource("radio/tx/s0"), at: None, params: BTreeMap::new() };
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

// ---------------------------------------------------------------- connect's initial streams (RS-12, RM-30)

/// A Session on a FakeDevice the test keeps, asking `requests` of the radio at `connect`, advanced
/// `ms` past T0; the device's call trace and the Run.
fn session_on(requests: &[(&str, Scalar)], envelope: Json, ms: i64, declared: Declared) -> (Arc<FakeDevice>, Run) {
    session_with_layout(requests, envelope, ms, None, declared)
}

/// As [`session_on`], the radio's `rx` and `tx` lists replaced by `layout` when given.
fn session_with_layout(requests: &[(&str, Scalar)], envelope: Json, ms: i64, layout: Option<(Json, Json)>, declared: Declared) -> (Arc<FakeDevice>, Run) {
    let temp = rig::TempDir::new("uhd-initial");
    let mut profile = rig::uhd_profile(&temp.0, true, envelope);
    if let Some((rx, tx)) = layout {
        profile["bindings"]["radio"]["selector"]["rx"] = rx;
        profile["bindings"]["radio"]["selector"]["tx"] = tx;
    }
    let device = fake_device();
    let assembly = rig::assemble_on(&profile, BTreeMap::new(), device.clone());
    let mut run = Run::connect(&profile, &support::asking("radio", requests), assembly, Lease::attached(), declared).expect("the Session's entry is valid");
    if matches!(run.state(), ezsdr_kernel::run::RunState::Running {}) {
        let t0 = run.start_instant().expect("a start instant");
        run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ms * MS)).unwrap();
    }
    drop(temp);
    (device, run)
}

/// A Session its test expects refused at `connect` (RM-31, RM-33, RM-19).
fn refused() -> Declared {
    Declared { end: Some(Expect::Refused), ..Declared::default() }
}

fn first(calls: &[String], prefix: &str) -> usize {
    calls.iter().position(|call| call.starts_with(prefix)).unwrap_or_else(|| panic!("no call starts with {prefix:?}: {calls:?}"))
}

fn num(x: f64) -> Scalar {
    Scalar::Num(ezsdr_kernel::spec::Finite::new(x).unwrap())
}

#[test]
fn uhd_59_a_default_session_configures_and_opens_nothing() {
    // RM-30: no stream, so no streamer is opened and no front end configured, and the recorder
    // linked to `rx0` hears nothing; the Session still connects.
    let (device, run) = session_on(&[], envelope(), 100, Declared::default());
    assert!(matches!(run.state(), ezsdr_kernel::run::RunState::Running {}), "{:?}", run.state());
    let calls = device.calls();
    for prefix in ["apply rx", "apply tx", "rx_open", "tx_open", "rx_start", "tx_send"] {
        assert!(!calls.iter().any(|call| call.starts_with(prefix)), "{prefix}: {calls:?}");
    }
    let manifest = run.finish();
    assert!(manifest.clocks.sample_clocks.is_empty() && manifest.artifacts.is_empty(), "{:?}", manifest.clocks.sample_clocks);
}

#[test]
fn uhd_59_initial_values_are_applied_before_the_streamers_open() {
    // What a Session asks at connect is configured in prepare, ahead of the arm that opens the
    // streamers, and the streams start and send afterwards (UR-12, UR-13, UR-15).
    let envelope = json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999_000_000.0, "hi_hz": 1_001_000_000.0 }], "max_gain_db": 20.0, "tx_enabled": [true] } });
    let requests = [
        ("radio.rx.c0.stream", Scalar::Int(0)),
        ("radio.rx.s0.sample_rate_hz", num(2.0e6)),
        ("radio.rx.c0.frequency_hz", num(1.0005e9)),
        ("radio.rx.c0.gain_db", num(10.0)),
        ("radio.rx.c0.antenna", Scalar::from("TX/RX")),
        ("radio.tx.c0.stream", Scalar::Int(0)),
        ("radio.tx.s0.sample_rate_hz", num(2.0e6)),
        ("radio.tx.c0.frequency_hz", num(1.0005e9)),
        ("radio.tx.c0.gain_db", num(5.0)),
    ];
    // The transmit channel is in a stream, so a receive antenna on TX/RX would break RM-31: ask RX2.
    let requests: Vec<_> = requests.into_iter().map(|(key, value)| if key == "radio.rx.c0.antenna" { (key, Scalar::from("RX2")) } else { (key, value) }).collect();
    let (device, mut run) = session_on(&requests, envelope, 5, Declared::default());
    assert!(matches!(run.state(), ezsdr_kernel::run::RunState::Running {}), "{:?}", run.state());
    let calls = device.calls();
    let (apply_rx, apply_tx) = (first(&calls, "apply rx 0 rate=2000000 freq=1000500000 gain=10 antenna=RX2"), first(&calls, "apply tx 0 rate=2000000 freq=1000500000 gain=5"));
    let (open_rx, open_tx) = (first(&calls, "rx_open"), first(&calls, "tx_open"));
    assert!(apply_rx < open_rx && apply_rx < open_tx && apply_tx < open_rx && apply_tx < open_tx, "every initial value is applied before any streamer opens: {calls:?}");
    assert!(open_rx < first(&calls, "rx_start"), "the receive streamer starts after it opens: {calls:?}");
    let _ = &mut run;
    let manifest = run.finish();
    assert_eq!(manifest.action_log.len(), 0, "no call but connect asked for any of it");
    let rx = manifest.clocks.sample_clocks.iter().find(|c| c.stream.path() == "usrp/rx/s0").expect("the receive clock");
    assert_eq!((rx.nominal_rate.num(), rx.nominal_rate.den()), (2_000_000, 1), "the first clock has the asked rate");
}

#[test]
fn uhd_59_a_named_selector_calls_the_device_as_the_unnamed_one() {
    // RM-28: the same Session on the same layout written plain and with names: the device sees the same
    // calls, none carrying a name, and the Manifest has the same clocks.
    let envelope = json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999_000_000.0, "hi_hz": 1_001_000_000.0 }], "max_gain_db": 20.0, "tx_enabled": [true, true] } });
    let requests = [
        ("radio.rx.c1.stream", Scalar::Int(0)),
        ("radio.rx.s0.sample_rate_hz", num(2.0e6)),
        ("radio.rx.c1.frequency_hz", num(1.0005e9)),
        ("radio.rx.c1.gain_db", num(10.0)),
        ("radio.rx.c1.antenna", Scalar::from("RX2")),
        ("radio.tx.c0.stream", Scalar::Int(0)),
        ("radio.tx.s0.sample_rate_hz", num(2.0e6)),
        ("radio.tx.c0.frequency_hz", num(1.0005e9)),
        ("radio.tx.c0.gain_db", num(5.0)),
    ];
    let run = |layout: (Json, Json)| {
        let (device, run) = session_with_layout(&requests, envelope.clone(), 5, Some(layout), Declared::default());
        assert!(matches!(run.state(), ezsdr_kernel::run::RunState::Running {}), "{:?}", run.state());
        let manifest = run.finish();
        // `rx_start`'s argument is a device time read from the wall clock; the call itself is compared.
        let calls: Vec<String> = device.calls().into_iter().map(|call| if call.starts_with("rx_start") { "rx_start".to_owned() } else { call }).collect();
        // The clocks' origins are wall-clock device times; their streams and rates are the Session's.
        let clocks: Vec<(String, u64)> = manifest.clocks.sample_clocks.iter().map(|c| (c.stream.path().to_owned(), c.nominal_rate.num())).collect();
        (calls, clocks)
    };
    let plain = run((json!(["0:B:ubx:0", "0:A:ubx:0"]), json!(["0:A:ubx:0", "0:B:ubx:0"])));
    let named = run((json!(["0:B:ubx:0#side-rx", "0:A:ubx:0#main-rx"]), json!(["0:A:ubx:0#main-tx", "0:B:ubx:0"])));
    assert_eq!(plain.0, named.0, "the same device calls");
    assert_eq!(plain.1, named.1, "the same clocks");
    assert!(first(&named.0, "apply rx 1") < first(&named.0, "rx_open"), "{:?}", named.0);
    assert!(!named.0.iter().any(|call| call.contains('#')), "a name reached the device: {:?}", named.0);
}

#[test]
fn uhd_59_membership_alone_sends_nothing() {
    // A transmit channel in a stream is configured and its streamer opened, and no waveform sample
    // is sent while nothing is repeated or transmitted (RM-30).
    let (device, run) = session_on(&[("radio.tx.c0.stream", Scalar::Int(0))], envelope(), 50, Declared::default());
    let calls = device.calls();
    assert!(first(&calls, "apply tx 0") < first(&calls, "tx_open"), "{calls:?}");
    // The only send is the four-zero burst a streamer's opening makes to clear stale header state (UR-22).
    let sends: Vec<_> = calls.iter().filter(|call| call.starts_with("tx_send")).collect();
    assert_eq!(sends, ["tx_send 0 n=4 at=- sob=true eob=true"], "no waveform sample went out: {calls:?}");
    let manifest = run.finish();
    assert!(!manifest.events.delivered.iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::TX_UNDERFLOW), "no underflow: nothing was due");
}

#[test]
fn uhd_59_a_refused_initial_request_configures_nothing() {
    // RM-31 judged at Validate: a transmit channel in a stream with its board's receive antenna on
    // TX/RX; no device call follows, as on the Mock (`v58_connect_refuses_what_the_layout_cannot_carry_at_validate`).
    let (device, run) = session_on(&[("radio.tx.c0.stream", Scalar::Int(0)), ("radio.rx.c0.stream", Scalar::Int(0)), ("radio.rx.c0.antenna", Scalar::from("TX/RX"))], envelope(), 5, refused());
    assert!(matches!(run.state(), ezsdr_kernel::run::RunState::CleanedUp { .. }), "{:?}", run.state());
    let manifest = run.finish();
    assert!(matches!(manifest.termination.reason, ezsdr_kernel::run::Termination::Failed { stage: ezsdr_kernel::run::Stage::Validate, .. }), "{:?}", manifest.termination.reason);
    assert!(manifest.admission.rejected.iter().any(|item| item.reason.starts_with("RM-31") && item.reason.contains("transmit channel c0")), "{:?}", manifest.admission.rejected);
    let calls = device.calls();
    for prefix in ["apply rx", "apply tx", "rx_open", "tx_open", "rx_start"] {
        assert!(!calls.iter().any(|call| call.starts_with(prefix)), "{prefix}: {calls:?}");
    }
}

#[test]
fn uhd_59_the_rf_envelope_judges_an_assigned_transmit_channel_at_validate() {
    // RM-33: a transmit channel asked for at connect is judged on its band and its enable like any
    // other. The check does not look at receive bands, so the case is a transmit channel.
    let refused = |requests: &[(&str, Scalar)], envelope: Json, what: &str| {
        let (device, run) = session_on(requests, envelope, 5, refused());
        let manifest = run.finish();
        assert!(matches!(manifest.termination.reason, ezsdr_kernel::run::Termination::Failed { stage: ezsdr_kernel::run::Stage::Validate, .. }), "{what}: {:?}", manifest.termination.reason);
        assert!(manifest.admission.violations.iter().any(|v| v.check.as_str() == "radio.rf_envelope"), "{what}: {:?}", manifest.admission.violations);
        let calls = device.calls();
        for prefix in ["apply tx", "tx_open", "tx_send"] {
            assert!(!calls.iter().any(|call| call.starts_with(prefix)), "{what}: {prefix}: {calls:?}");
        }
    };
    let band = json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999_000_000.0, "hi_hz": 1_001_000_000.0 }], "max_gain_db": 0.0, "tx_enabled": [true] } });
    refused(&[("radio.tx.c0.stream", Scalar::Int(0)), ("radio.tx.c0.frequency_hz", num(2.0e9))], band.clone(), "outside the band");
    let disabled = json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999_000_000.0, "hi_hz": 1_001_000_000.0 }], "max_gain_db": 0.0, "tx_enabled": [false] } });
    refused(&[("radio.tx.c0.stream", Scalar::Int(0))], disabled, "transmit not enabled");
    // The same request inside the band and enabled connects.
    let (_, run) = session_on(&[("radio.tx.c0.stream", Scalar::Int(0))], band, 5, Declared::default());
    assert!(matches!(run.state(), ezsdr_kernel::run::RunState::Running {}), "{:?}", run.state());
    let _ = run.finish();
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
    let rx = run.sample_clocks().into_iter().find(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap()).expect("the receive SampleClock");
    let n = rx.root_ticks_per_tick.num() as i64;
    // A sample index 100 ms ahead of the Run's time (v3's capture at a sample index).
    let index = (run.now().ticks_in(run.now().domain()).unwrap() - rx.origin.ticks_in(rx.origin.domain()).unwrap()) / n + 100_000;
    let from = run.events(0).len();
    admitted(&mut run, capture(100, Some(TimePoint::new(rx.domain, index))), None);
    wait_capture(&mut run, from);
    let manifest = run.finish();
    let map = &capture_artifact(&manifest).continuity[0];
    let (first, end) = (map.first.ticks_in(map.first.domain()).unwrap(), map.end.ticks_in(map.end.domain()).unwrap());
    // A test thread delayed between the index and the request makes the capture start
    // later; what holds either way is that it starts no earlier than asked and holds its
    // 100 samples. The exact start is pinned on a Spec by
    // `uhd_59_receive_capture_starts_at_the_requested_index`.
    assert!(first >= index, "the first sample at {first}, asked {index}");
    assert_eq!(end - first, 100, "the map names the samples it holds");
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

/// RM-14, KC-28 (stage 4a): a Session admits an untimed burst at its Provider's declared command lead, 5 ms after the Action's instant on the
/// UHD Module, as on MockRadio (`v58.rs`).
#[test]
fn s4a_an_untimed_session_burst_is_admitted_at_now_plus_the_declared_lead_on_the_uhd_module() {
    let temp = rig::TempDir::new("s4a-declared-lead-uhd");
    let mut run = uhd_loopback_session(&temp);
    // `uhd_loopback_session` has advanced 1 ms past T0; the transmit clock's origin is before that.
    let (bytes, _) = experiments::waveform_of(&pn(1_000));
    let repeat = SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: resource("radio/tx/s0"), at: None, params: BTreeMap::new() };
    let entry = run.submit(repeat, Some(&bytes)).unwrap();
    let Outcome::Admitted { coercions, .. } = &entry.outcome else { panic!("{:?}", entry.outcome) };
    let at = coercions.iter().find(|coercion| coercion.key == Key::parse("ezsdr.action.at").unwrap()).expect("the untimed burst is given an instant");
    let now = entry.time;
    assert_eq!(at.applied, Value::Scalar(Scalar::Int(now.ticks_in(now.domain()).unwrap() + 5 * MS)));
}

#[test]
fn ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end() {
    // RM-15, RM-16 (sequence 1) on UHD: an untimed repeat submitted right after a transmit stream's Stop is placed past the Stop region, with
    // no TIME_ERROR and no refusal (the same test as the Mock's, in repeat_switch.rs).
    let temp = rig::TempDir::new("ea-16-untimed-after-stop-uhd");
    let mut run = uhd_loopback_session(&temp);
    let wave = pn(1_000);
    let (bytes, _) = experiments::waveform_of(&wave);
    let repeat = SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: resource("radio/tx/s0"), at: None, params: BTreeMap::new() };
    admitted(&mut run, repeat.clone(), Some(&bytes));
    let now = run.now();
    run.advance_to(TimePoint::new(now.domain(), now.ticks_in(now.domain()).unwrap() + 20 * MS)).unwrap();
    // The Stop's own log stamp, taken before the Provider's receipt, not a time read after `submit` returned: a delayed thread reads a later time (note 51 §3).
    let stopped = run.submit(SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None).unwrap();
    assert!(matches!(stopped.outcome, Outcome::Admitted { .. }), "{:?}", stopped.outcome);
    let now = stopped.time;
    let stop_root = now.ticks_in(now.domain()).unwrap();
    let record = run.sample_clocks().into_iter().find(|r| r.stream.path().ends_with("tx/s0")).expect("the transmit SampleClock");
    let origin = record.origin.ticks_in(record.origin.domain()).unwrap();
    let stop = (stop_root - origin) / record.root_ticks_per_tick.num() as i64;
    admitted(&mut run, repeat, Some(&bytes));
    let _ = run.run_until_end(TimePoint::new(now.domain(), stop_root + 50 * MS));
    let manifest = run.finish();
    let rows = section(&manifest, "usrp", "ezsdr.radio.uhd.bursts").as_array().unwrap();
    assert_eq!(rows.len(), 2, "the repeat and the one after the Stop: {rows:?}");
    let gap = rows[1]["target"]["ticks"].as_i64().unwrap() - stop;
    // The window is 10 ms to within its rounding onto the sample grid (KL-13, KL-15): 9 988 samples here, 10 003 on the Mock.
    assert!(gap >= 9_900, "the repeat starts past the Stop region, {gap} samples after the Stop: {rows:?}");
    assert!(rows[1]["late_by"].is_null(), "an untimed repeat reports no lateness: {rows:?}");
    assert!(manifest.events.delivered.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR && event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED));
}

// ---------------------------------------------------------------- 4b-2b: the tail reaches the Manifest (S2c, S7)

/// The x3xx profile's receive restart gap in nanoseconds (MR-17).
fn gap_ns() -> i64 {
    ezsdr_radio::x3xx::timing().overflow_restart_gap_ns
}

/// One Provider's Manifest as S2c reads it: the capture holds exactly the delivered prefix
/// `[0, kf)` and is partial, one nine-byte `RX_OVERFLOW` sits at sample `kf` of the segment's
/// SampleClock with `lost` the distance to the clock's end, the capture carries no mark, and the
/// counter row for the stream's node counts one. `horizon` is the sample the Run's end cuts at;
/// `exact` pins it where the Provider's own clock is stepped (the Mock), while a device-paced
/// Provider's end follows the horizon by the coordinator's own work in real time, which the
/// manual clock counts (host.rs M5), so it is read from the record with only its lower bound
/// asserted. That the cut is inside the gap and not at the restart is pinned without wall time:
/// the capture holds exactly `[0, kf)` and no mark, and the one tail's `lost` is the positive
/// distance from `kf` to the recorded end.
fn assert_cut_overflow(manifest: &Manifest, node: &str, kf: i64, horizon: i64, exact: bool, what: &str) {
    let capture = artifact(manifest, "rec", 0);
    assert!(capture.partial, "{what}: the Run ended while the capture was open");
    // The capture holds the delivered prefix and nothing after it: `kf * 8` bytes of cf32.
    assert_eq!(capture.size_bytes, kf as u64 * 8, "{what}: the capture holds [0, {kf})");
    let map = &capture.continuity[0];
    assert_eq!((map.first.ticks_in(map.first.domain()).unwrap(), map.end.ticks_in(map.end.domain()).unwrap()), (0, kf), "{what}: one run, [0, {kf})");
    assert_eq!(capture.continuity.len(), 1, "{what}: the prefix is contiguous");
    assert!(capture.marks.is_empty(), "{what}: the tail is stamped at the capture's exclusive end (RS-30): {:?}", capture.marks);
    let overflows: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(overflows.len(), 1, "{what}: one tail: {:?}", manifest.events.delivered);
    let event = overflows[0];
    assert_eq!(event.source, source(node), "{what}: the stream's own node");
    assert_eq!(event.payload.as_array().map(Vec::len), Some(9), "{what}: RM-24's nine bytes: {}", event.payload);
    let payload = RxOverflowPayload::from_payload(&event.payload).unwrap();
    assert_eq!(payload.cause, RxOverflowCause::Overrun, "{what}");
    let clock = manifest.clocks.sample_clocks.iter().find(|record| record.stream.path() == node).unwrap_or_else(|| panic!("{what}: no receive clock"));
    assert_eq!(event.time.domain(), clock.domain, "{what}: the tail is stamped in the segment's SampleClock");
    assert_eq!(event.time.ticks_in(clock.domain).unwrap(), kf, "{what}: at the first sample not delivered");
    let origin = clock.origin.ticks_in(clock.origin.domain()).unwrap();
    let at = clock.ended_at.unwrap_or_else(|| panic!("{what}: the Run's end ends the clock")).ticks_in(clock.root).unwrap();
    let sample = (at - origin) / clock.root_ticks_per_tick.num() as i64;
    if exact {
        assert_eq!(sample, horizon, "{what}: the Run's end at the horizon");
    } else {
        assert!(sample >= horizon, "{what}: the Run's end is at the {horizon}-sample horizon, not before it: {sample}");
    }
    assert!(payload.lost > 0, "{what}: the report was spent by the cut, not carried by a block");
    assert_eq!(payload.lost as i64, sample - kf, "{what}: `lost` is the cut's distance from the first sample not delivered");
    let counter = manifest.events.counters.iter().find(|row| row.source == source(node) && row.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).unwrap_or_else(|| panic!("{what}: no counter row"));
    assert_eq!(counter.count, 1, "{what}");
}

/// The same Provider's control: the Run ran past the restart, so the block after it carried the
/// report inside the capture, and exactly one mark is recorded (RS-30).
fn assert_one_mark(manifest: &Manifest, what: &str) {
    let capture = artifact(manifest, "rec", 0);
    assert!(!capture.partial, "{what}: the 100 000-sample capture completed");
    assert_eq!(capture.size_bytes, 800_000, "{what}: 100 000 cf32 samples");
    assert_eq!(capture.marks.len(), 1, "{what}: the block-carried event is inside the capture: {:?}", capture.marks);
    assert_eq!(capture.marks[0].kind.as_str(), ezsdr_radio::kinds::RX_OVERFLOW, "{what}");
}

#[test]
fn uhd_59_a_cut_overflow_reaches_the_manifest_and_marks_no_capture_alike() {
    // S2c: one Spec with a capture, an overrun early in the first segment, and the Run ended
    // inside the 140 ms gap, so the segment is cut with the report unspent. On both Providers the
    // report reaches the Manifest as one nine-byte `RX_OVERFLOW` at the first sample not
    // delivered (`kf`), the capture holds the prefix `[0, kf)`, and the tail's stamp is the
    // capture's exclusive end: no mark (RS-30). A fault is the fake's device time since its
    // zero, so its placement comes from a throwaway Run's T0 — and the fake reports it only at a
    // receive call's start, so that call's size is the window the placement may drift in:
    // 20 000 samples = 20 ms at 1 MS/s, with the fault at its middle, survives 10 ms of the
    // manual clock's real-time construction difference (host.rs M5; `started_within` asserts the
    // margin). The Mock's own clock is stepped, so its fault at exactly 1 ms needs no margin.
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }] });
    // The Mock's block at 2 000 samples is cut at the fault's instant (MR-20), so its prefix is
    // [0, 1 000) and kf = 1 000 = 1 ms at 1 MS/s; its end at 20 ms makes `lost` 19 000.
    let temp = rig::TempDir::new("uhd-59-cut-mock");
    let mock = mock_spec_manifest(&temp, &spec, faults.clone(), 20);
    let mock_control = mock_spec_manifest(&temp, &spec, faults, (gap_ns() + 1_000_000 + 100_000_000 + 60_000_000) / 1_000_000);
    // The UHD: the first call delivers [0, 20 000) — 20 ms — and the second, whose next sample
    // is at 20 ms, reports the overrun, so kf = 20 000. The Run ends 40 ms after T0, inside the
    // gap: `lost` is the distance from kf to the end the Manifest records.
    let t0 = spec_t0(&spec);
    let uhd_temp = rig::TempDir::new("uhd-59-cut-uhd");
    let run = uhd_spec_run_on(&uhd_temp, &spec, 20_000, fake_with_faults(vec![FakeFault::Overflow(device_ns(t0 + 10 * MS))]), Declared::default());
    started_within(&run, t0, 10 * MS);
    let uhd = finish_after(run, 40);
    let run = uhd_spec_run_on(&uhd_temp, &spec, 20_000, fake_with_faults(vec![FakeFault::Overflow(device_ns(t0 + 10 * MS))]), Declared::default());
    started_within(&run, t0, 10 * MS);
    let uhd_control = finish_after(run, (gap_ns() + 1_000_000 + 100_000_000 + 60_000_000) / 1_000_000);
    // The shape first: the overrun reached each Provider, the Mock's row with the profile's
    // restart gap, and the UHD's counter once.
    let rows = section(&mock, "mock", "ezsdr.radio.mock.faults").as_array().expect("the Mock's fault rows");
    assert_eq!((rows.len(), rows[0]["applied"].as_bool(), rows[0]["lost"].as_u64()), (1, Some(true), Some(gap_ns() as u64 / 1_000)), "the Mock's overrun applied with the profile's restart gap");
    assert_eq!(section(&uhd, "usrp", "ezsdr.radio.uhd.stats")["rx_overflows"], 1, "the UHD received the report");
    assert_cut_overflow(&mock, "mock/rx/s0", 1_000, 20_000, true, "the Mock");
    assert_cut_overflow(&uhd, "usrp/rx/s0", 20_000, 40_000, false, "the UHD");
    assert_one_mark(&mock_control, "the Mock's control");
    assert_one_mark(&uhd_control, "the UHD's control");
}

#[test]
fn uhd_59_a_loss_after_an_overrun_stops_the_run_and_keeps_the_tail() {
    // S7 on the UHD: an overrun at 10 ms after T0 leaves the second call's report pending (the
    // first call delivered [0, 20 000), 20 ms), and the device is lost at 30 ms, found by the
    // next receive call (the 23 ms wait for the restart, 20 000 samples and the 3 ms allowance).
    // The default Policy aborts the Run with `DEVICE_LOST`; the tail survives it: one
    // `RX_OVERFLOW` at kf = 20 000 with `lost` the distance to the cut the loss booked — the
    // clock's recorded end, which is not 30 000 itself.
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let t0 = spec_t0(&spec);
    let temp = rig::TempDir::new("uhd-59-tail-loss");
    let faults = vec![FakeFault::Overflow(device_ns(t0 + 10 * MS)), FakeFault::Lost(device_ns(t0 + 30 * MS))];
    let run = uhd_spec_run_on(&temp, &spec, 20_000, fake_with_faults(faults), Declared::default()).expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
    started_within(&run, t0, 10 * MS);
    let manifest = finish_after(run, 60);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: EventKind::parse(EventKind::DEVICE_LOST).unwrap() } });
    let overflows: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(overflows.len(), 1, "one tail: {:?}", manifest.events.delivered);
    let event = overflows[0];
    assert_eq!(event.source, source("usrp/rx/s0"));
    assert_eq!(event.payload.as_array().map(Vec::len), Some(9), "RM-24's nine bytes: {}", event.payload);
    let payload = RxOverflowPayload::from_payload(&event.payload).unwrap();
    assert_eq!(payload.cause, RxOverflowCause::Overrun);
    let kf = 20_000;
    let clock = manifest.clocks.sample_clocks.iter().find(|record| record.stream.path() == "usrp/rx/s0").expect("the receive clock");
    assert_eq!(event.time.domain(), clock.domain);
    assert_eq!(event.time.ticks_in(clock.domain).unwrap(), kf, "the first sample not delivered");
    // The shape: the capture is partial and holds exactly that prefix, and the tail's stamp is
    // its exclusive end, so it carries no mark (RS-30).
    let capture = artifact(&manifest, "rec", 0);
    assert!(capture.partial, "the loss ends the Run while the capture is open");
    assert_eq!(capture.size_bytes, 160_000, "the capture holds the delivered prefix [0, 20 000)");
    assert!(capture.marks.is_empty(), "the tail is stamped at the capture's exclusive end: {:?}", capture.marks);
    let origin = clock.origin.ticks_in(clock.origin.domain()).unwrap();
    let end = clock.ended_at.expect("the loss ends the stream").ticks_in(clock.root).unwrap();
    let sample = (end - origin) / clock.root_ticks_per_tick.num() as i64;
    assert!(sample >= 30_000, "the loss's cut is at or after its instant 30 000, not before it: {sample}");
    assert_eq!(payload.lost, (sample - kf) as u64, "the tail reaches the cut the loss booked");
    assert!(payload.lost > 0);
    assert_eq!(manifest.events.delivered.iter().filter(|event| event.kind.as_str() == EventKind::DEVICE_LOST).count(), 1, "one DEVICE_LOST");
}
