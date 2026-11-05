//! The Vision §59 rehearsal on `FakeDevice` (plan/phase7/00-overview.md §7, §8): the
//! Phase 2–6 experiments under a UHD profile, assembled by the server's catalogue, with
//! the fake device in place of a USRP. The same steps on an X310 are bench B3–B7.

mod support;

// The shared per-test wall watchdog (plan 30 §7, R05): a manual clock a mutant freezes would
// hang the test until the harness's timeout; this aborts instead. One implementation, shared
// with `ezsdr-radio-uhd`'s `tests/common` and `ezsdr-server`'s `tests/protocol.rs`.
#[path = "../../ezsdr-radio-uhd/tests/common/watchdog.rs"]
mod watchdog;

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
use ezsdr_kernel::spec::{Ident, Key, Namespace, Scalar, Value};
use ezsdr_kernel::stream::BurstRecord;
use ezsdr_kernel::time::TimePoint;
use ezsdr_radio_uhd::{Device, FakeConfig, FakeDevice};
use serde_json::{Value as Json, json};
use support::{T0, artifact, finish_at, modules, output, resource, root, section, spec_run};

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

fn uhd_spec_run(temp: &rig::TempDir, spec: &Json, inputs: BTreeMap<ContentHash, Vec<u8>>) -> RunHandle {
    let profile = rig::uhd_profile(&temp.0, false, envelope());
    coordinator::start_spec_run(spec, &profile, rig::assemble_on(&profile, inputs, fake())).expect("Spec Run starts")
}

fn uhd_session(temp: &rig::TempDir) -> RunHandle {
    uhd_session_asking(temp, support::rx0)
}

/// A Session that receives and transmits on c0, both asked for at `connect` (RS-12).
fn uhd_loopback_session(temp: &rig::TempDir) -> RunHandle {
    uhd_session_asking(temp, support::rx0_tx0)
}

fn uhd_session_asking(temp: &rig::TempDir, asking: fn(&ezsdr_kernel::coordinator::Assembly) -> ezsdr_kernel::session::InitialRequests) -> RunHandle {
    let profile = rig::uhd_profile(&temp.0, true, envelope());
    let assembly = rig::assemble_on(&profile, BTreeMap::new(), fake());
    let mut run = coordinator::connect(&profile, &asking(&assembly), assembly, Lease::attached()).expect("Session connects");
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
        target: output("rec"),
        at,
        params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::from(n))]),
    }
}

fn set(key: &str, value: Value) -> SessionAction {
    SessionAction::SetParameter { target: resource("radio"), key: Key::parse(key).unwrap(), value }
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
fn uhd_59_a_value_outside_the_rf_envelope_is_refused_at_connect_and_a_transmit_runtime_change_has_no_class() {
    // Note 47 stage 3: the envelope is checked on the initial values; a transmit change during the Run is refused as a key with no update class
    // until stage 4 (RM-27). The receive form is the next test.
    let (_, refused) = session_on(&[("radio.tx.c0.stream", Scalar::Int(0)), ("radio.tx.c0.frequency_hz", num(2.4e9))], envelope(), 10);
    assert!(
        matches!(refused.state(), ezsdr_kernel::run::RunState::CleanedUp { termination: ezsdr_kernel::run::Termination::Failed { reason, .. } } if reason.contains("radio.rf_envelope")),
        "{:?}",
        refused.state()
    );
    let _ = refused.finish();
    let temp = rig::TempDir::new("uhd-59-rf");
    let mut run = uhd_session(&temp);
    let entry = run.submit(set("radio.tx.c0.frequency_hz", Value::num(2.4e9).unwrap()), None).unwrap();
    let Outcome::Rejected { violations } = &entry.outcome else { panic!("a runtime radio value change was admitted") };
    assert!(violations.iter().any(|violation| violation.check.as_str() == "ezsdr.update_class" && violation.reason.contains("declares no update class")), "{violations:?}");
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
    let (device, mut run) = session_on(&[("radio.rx.c0.stream", Scalar::Int(0))], envelope, 10);
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
fn session_on(requests: &[(&str, Scalar)], envelope: Json, ms: i64) -> (Arc<FakeDevice>, RunHandle) {
    session_with_layout(requests, envelope, ms, None)
}

/// As [`session_on`], the radio's `rx` and `tx` lists replaced by `layout` when given.
fn session_with_layout(requests: &[(&str, Scalar)], envelope: Json, ms: i64, layout: Option<(Json, Json)>) -> (Arc<FakeDevice>, RunHandle) {
    let temp = rig::TempDir::new("uhd-initial");
    let mut profile = rig::uhd_profile(&temp.0, true, envelope);
    if let Some((rx, tx)) = layout {
        profile["bindings"]["radio"]["selector"]["rx"] = rx;
        profile["bindings"]["radio"]["selector"]["tx"] = tx;
    }
    let device = fake_device();
    let assembly = rig::assemble_on(&profile, BTreeMap::new(), device.clone());
    let mut run = coordinator::connect(&profile, &support::asking("radio", requests), assembly, Lease::attached()).expect("the Session's entry is valid");
    if matches!(run.state(), ezsdr_kernel::run::RunState::Running {}) {
        let t0 = run.start_instant().expect("a start instant");
        run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ms * MS)).unwrap();
    }
    drop(temp);
    (device, run)
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
    let (device, run) = session_on(&[], envelope(), 100);
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
    let (device, mut run) = session_on(&requests, envelope, 5);
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
        let (device, run) = session_with_layout(&requests, envelope.clone(), 5, Some(layout));
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
    let (device, run) = session_on(&[("radio.tx.c0.stream", Scalar::Int(0))], envelope(), 50);
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
    let (device, run) = session_on(&[("radio.tx.c0.stream", Scalar::Int(0)), ("radio.rx.c0.stream", Scalar::Int(0)), ("radio.rx.c0.antenna", Scalar::from("TX/RX"))], envelope(), 5);
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
        let (device, run) = session_on(requests, envelope, 5);
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
    let (_, run) = session_on(&[("radio.tx.c0.stream", Scalar::Int(0))], band, 5);
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
