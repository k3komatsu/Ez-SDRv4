//! Spec 18 §6: every UR rule through the real coordinator on `FakeDevice`, with an
//! Assembly built as the server's catalogue builds it (the capture Sink, the host
//! Link, `DeviceAuthority` and `UhdRadio` on one device), unless a test says "unit".
//! The device's clock is the manual one (plan 30 §4.4), so a test's waits and its
//! timing assertions are that clock's; see tests/README.md for the rules and for
//! timing-precondition diagnostics.

mod common;
mod infra_fixtures;

use ezsdr_kernel::spec::Scalar;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding};
use ezsdr_kernel::coordinator::RunHandleError;
use ezsdr_kernel::event::{Action, ActionId, Event, EventCollector, EventKind, EventSource, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ResourceId, RunId};
use ezsdr_kernel::manifest::{ArtifactRef, Manifest};
use ezsdr_kernel::module_api::{ActionReceiver, ActionSubmitter, AttachedPort, Authority, Endpoint, ExecutionClass, Link, ModuleErrorKind, ModuleRegistry, PrepareContext, Provider, Role};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::run::{RunState, Stage, StopCause, Termination};
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BurstEnd, DataLinkDecl};
use ezsdr_kernel::time::{ClockRegistry, Duration, EpochRef, RelativeBudget, TimePoint};
use ezsdr_radio::payloads::{TimeErrorOutcome, TimeErrorPayload};
use ezsdr_radio_uhd::{ClockMode, Device, DeviceAuthority, Dir, FakeConfig, FakeDevice, FakeFault, TxCode, UhdRadio};
use serde_json::{Value as Json, json};

use common::rig::{AuthorityGuard, assert_joined, assert_stands_still};
use common::*;

// ---------------------------------------------------------------- identity (UR-1…UR-5)

#[test]
fn ur_16_calls_drops_only_the_stop_that_follows_its_stream_s_open() {
    // The helper behind the experiment-traffic assertions: a refused open before the real one, and a stop after a start, do not hide an experiment stop.
    let device = FakeDevice::new(FakeConfig { clock: ClockMode::Manual, faults: vec![FakeFault::RxOpenFails], ..FakeConfig::default() });
    assert!(device.rx_open(0, &[0]).is_err());
    device.rx_open(0, &[0]).unwrap();
    device.rx_stop(0).unwrap();
    device.rx_start(0, 4_000_000).unwrap();
    device.rx_stop(0).unwrap();
    assert_eq!(calls(&device, "rx_"), ["rx_open 0 [0]", "rx_open 0 [0]", "rx_start 0 4000000", "rx_stop 0 now"], "{:?}", device.calls());
    // No open before it: a stop with a start between is not setup, and another stream's calls do not decide.
    let device = FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() });
    device.rx_open(0, &[0]).unwrap();
    device.rx_start(0, 4_000_000).unwrap();
    device.rx_stop(0).unwrap();
    assert_eq!(calls(&device, "rx_stop"), ["rx_stop 0 now"], "{:?}", device.calls());
}

#[test]
fn ur_01_the_descriptor_registers() {
    let mut registry = ModuleRegistry::new();
    let (mut checks, mut kinds) = (AdmissionCheckRegistry::new(), EventKindRegistry::with_kernel_kinds());
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
    registry.register(ezsdr_radio_uhd::descriptor(), ezsdr_radio_uhd::factories()).unwrap();
    let descriptor = registry.modules().find(|d| d.id.as_str() == "ezsdr.radio.uhd").unwrap();
    assert_eq!(descriptor.roles, [Role::Provider, Role::Authority]);
    assert_eq!(descriptor.impl_hash, Some(ContentHash::of_bytes(b"ezsdr.radio.uhd 0.4.0")));
    assert_eq!(descriptor.version, ezsdr_kernel::module_api::Version::new(0, 4, 0));
    let mut old = ModuleRegistry::new();
    let mut vocabulary = ezsdr_radio::vocabulary();
    vocabulary.version = ezsdr_kernel::module_api::Version::new(1, 5, 0);
    old.register_vocabulary(vocabulary).unwrap();
    assert!(old.register(ezsdr_radio_uhd::descriptor(), ezsdr_radio_uhd::factories()).is_err());
}

#[test]
fn ur_02_device_is_object_safe_and_shared() {
    fn shared<T: Send + Sync + ?Sized>() {}
    shared::<dyn Device>();
    let device: Arc<dyn Device> = fake(FakeConfig::default());
    assert_eq!(device.master_clock_rate(), MCR as u64);
}

#[test]
fn ur_03_time_specs_round_trip() {
    for tick in [0, 1, 199_999_999, 200_000_000, 1i64 << 53, -1] {
        let (full, frac) = ezsdr_radio_uhd::to_time_spec(tick, MCR as u64);
        assert_eq!(ezsdr_radio_uhd::from_time_spec(full, frac, MCR as u64).unwrap(), tick, "{tick}");
    }
    assert_eq!(ezsdr_radio_uhd::to_time_spec(-1, MCR as u64), (-1, 0.999999995));
    assert!(ezsdr_radio_uhd::from_time_spec(i64::MAX / 2, 0.0, MCR as u64).is_err());
}

#[cfg(not(feature = "uhd"))]
#[test]
fn ur_04_open_without_the_feature_refuses() {
    let error = ezsdr_radio_uhd::open("addr=192.168.40.2").err().unwrap();
    assert!(error.contains("feature `uhd`"), "{error}");
}

fn binding(selector: Json, profile: Option<Json>) -> Binding {
    let mut selector = selector;
    if let Some(object) = selector.as_object_mut() {
        object.entry("rx").or_insert(json!(["0:A:ubx:0", "0:B:ubx:0"]));
        object.entry("tx").or_insert(json!(["0:A:ubx:0", "0:B:ubx:0"]));
    }
    let mut doc = json!({ "module": uhd_module(), "selector": selector });
    if let Some(profile) = profile {
        doc["profile"] = profile;
    }
    serde_json::from_value(doc).unwrap()
}

#[test]
fn ur_05_from_binding_refusals() {
    let device = fake(FakeConfig::default());
    let refuse = |b: Binding, device: Arc<FakeDevice>| UhdRadio::from_binding(&b, device).err().unwrap().message;
    let ok = json!({ "args": ARGS });
    assert!(UhdRadio::from_binding(&binding(ok.clone(), Some(x3xx())), device.clone()).is_ok());
    let mut other = binding(ok.clone(), Some(x3xx()));
    other.module.id = ezsdr_kernel::id::ModuleId::parse("ezsdr.radio.mock").unwrap();
    assert!(refuse(other, device.clone()).starts_with("UR-5:"));
    assert!(refuse(binding(ok.clone(), None), device.clone()).starts_with("UR-5:"));
    let foreign = json!({ "name": "ideal", "version": { "major": 2, "minor": 0, "patch": 0 } });
    assert!(refuse(binding(ok.clone(), Some(foreign)), device.clone()).starts_with("UR-5:"));
    let mut fed = binding(ok.clone(), Some(x3xx()));
    fed.feed = serde_json::from_value(json!({ "port": { "component": "radio", "port": "rx0" }, "policy": "drop_oldest", "capacity": 4 })).unwrap();
    assert!(refuse(fed, device.clone()).starts_with("UR-5:"));
    for selector in [
        json!({ "args": ARGS, "fake": true }),
        json!({ "args": ARGS, "block_len": 0 }),
        json!({ "args": ARGS, "clock_source": "atomic" }),
        json!({ "args": ARGS, "id": "a/b" }),
        json!({ "args": ARGS, "id": "" }),
        json!({ "id": "usrp" }),
        json!({ "args": ARGS, "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"], "image": ["xx"] }),
        json!({ "args": ARGS, "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"], "link_gbps": [2] }),
        json!({ "args": ARGS, "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"], "master_clock_hz": [184_320_000] }),
        json!({ "args": ARGS, "rx": [], "tx": [] }),
    ] {
        assert!(refuse(binding(selector.clone(), Some(x3xx())), device.clone()).starts_with("UR-5:"), "{selector}");
    }
    // The id is only a node path now (sections are filed by source, RS-39): any one segment.
    assert!(UhdRadio::from_binding(&binding(json!({ "args": ARGS, "id": "Dev-1.a" }), Some(x3xx())), device.clone()).is_ok());
    let slow = fake(FakeConfig { master_clock_rate: 184_320_000, ..FakeConfig::default() });
    assert_eq!(refuse(binding(ok, Some(x3xx())), slow), "UR-5: profile x3xx needs a 200 MHz master clock; the device runs at 184320000 Hz");
}

/// The selector of a device with one board in slot A (UR-5, design 33 D5).
fn one_board(board: &str) -> Json {
    json!({ "args": ARGS, "rx": [format!("0:A:{board}:0")], "tx": [format!("0:A:{board}:0")] })
}

#[test]
fn ur_09_the_obx_is_one_channel_from_10_mhz_to_8_4_ghz_defaulting_to_1_ghz() {
    let selector = one_board("obx");
    let radio = UhdRadio::from_binding(&binding(selector.clone(), Some(x3xx())), fake(one_obx())).unwrap();
    let capability = |name: &str| radio.instance().tree.capabilities[&Key::parse(name).unwrap()].clone();
    let range = |min, max| ezsdr_kernel::spec::CapabilityValue::Range { min, max };
    assert_eq!(capability("radio.rx.c0.frequency_hz"), range(Scalar::try_from(1e7).unwrap(), Scalar::try_from(8.4e9).unwrap()));
    assert_eq!(capability("radio.rx.c0.stream"), range(Scalar::from(-1), Scalar::from(0)));
    assert_eq!(capability("radio.tx.c0.frequency_hz"), range(Scalar::try_from(1e7).unwrap(), Scalar::try_from(8.4e9).unwrap()));
    assert_eq!(capability("radio.tx.c0.stream"), range(Scalar::from(-1), Scalar::from(0)));
    assert_eq!(capability("radio.rx.c0.phase_behavior_on_retune"), ezsdr_kernel::spec::CapabilityValue::One { value: Scalar::from("random_unless_timed_tune".to_owned()) });
    assert!(!radio.instance().tree.capabilities.contains_key(&Key::parse("radio.rx.coherent").unwrap()));
    assert_eq!(radio.instance().profile.as_ref().unwrap().name, "x3xx");
    let dir = TempDir::new();
    let mut doc = profile(&dir, selector, json!({}), true);
    doc["bindings"]["radio"]["profile"] = x3xx();
    let mut run = session(&doc, fake(one_obx()));
    past_t0(&mut run, ms(1));
    let manifest = run.finish();
    let applied = section(&manifest, "applied").as_array().unwrap().iter().find(|r| r["key"] == "radio.rx.c0.frequency_hz").cloned().unwrap();
    assert_eq!(applied["claimed"], 1e9);
    let timing = &section(&manifest, "envelope")["timing"];
    assert_eq!(timing["min_timed_command_lead_ns"], 5_000_000);
    assert_eq!(timing["startup_latency_ns"], 2_000_000_000i64);
    assert_eq!(timing["overflow_restart_gap_ns"], 50_000_000);
    assert_eq!(timing["rx_start_lead_ns"], 50_000_000);
    assert_eq!(timing["tx_in_flight_ns"], 10_000_000);
}

#[test]
fn ur_05_the_profile_must_be_the_device_s_front_ends() {
    let refuse = |selector: Json, config: FakeConfig| UhdRadio::from_binding(&binding(selector, Some(x3xx())), fake(config)).err().unwrap().message;
    assert!(UhdRadio::from_binding(&binding(one_board("cbx"), Some(x3xx())), fake(one_cbx())).is_ok());
    let boards = |front_ends: Vec<&'static str>| FakeConfig { front_ends, ..FakeConfig::default() };
    let two = |board: &str| json!({ "args": ARGS, "rx": [format!("0:A:{board}:0"), format!("0:B:{board}:0")], "tx": [format!("0:A:{board}:0"), format!("0:B:{board}:0")] });
    assert_eq!(refuse(one_board("ubx"), one_cbx()), "UR-5: the layout's rx channel 0 is `CBX-120 RX`, not `ubx` front end 0");
    assert_eq!(refuse(two("ubx"), boards(vec!["UBX", "CBX"])), "UR-5: the layout's rx channel 1 is `CBX RX`, not `ubx` front end 0");
    // The spec is set before the channel map is read (UR-5, H1): a layout naming more
    // channels than the device's slots has is refused by `set_subdev_spec` itself, with
    // UHD's own text.
    assert_eq!(refuse(two("ubx"), boards(vec!["UBX"])), "UR-5: uhd_usrp_set_rx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev B:0");
    assert_eq!(refuse(one_board("cbx"), FakeConfig::default()), "UR-5: the layout's rx channel 0 is `UBX RX`, not `cbx` front end 0");
    assert_eq!(refuse(one_board("cbx"), boards(vec![])), "UR-5: uhd_usrp_set_rx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev A:0");
    // The transmit side is checked the same way, and so is a device whose two directions
    // name boards of different kinds.
    assert!(UhdRadio::from_binding(&binding(one_board("obx"), Some(x3xx())), fake(one_obx())).is_ok());
    assert!(UhdRadio::from_binding(&binding(two("ubx"), Some(x3xx())), fake(boards(vec!["UBX", "UBX"]))).is_ok());
    assert_eq!(refuse(one_board("ubx"), one_obx()), "UR-5: the layout's rx channel 0 is `OBX RX`, not `ubx` front end 0");
    assert_eq!(refuse(one_board("obx"), FakeConfig::default()), "UR-5: the layout's rx channel 0 is `UBX RX`, not `obx` front end 0");
    assert_eq!(refuse(one_board("obx"), one_cbx()), "UR-5: the layout's rx channel 0 is `CBX-120 RX`, not `obx` front end 0");
    let mixed = json!({ "args": ARGS, "rx": ["0:A:ubx:0"], "tx": ["0:A:obx:0"] });
    assert_eq!(
        refuse(mixed, FakeConfig::default()),
        "UR-5: the layout's `rx` element at position 0 (timekeeper 0, slot A, `ubx` front end 0) sits in a slot that also carries `obx`; a duplex board carries both directions and admits no other board there"
    );
}

#[test]
fn ur_09_the_cbx_is_one_channel_from_1_2_ghz_defaulting_to_2_45_ghz() {
    let selector = one_board("cbx");
    let radio = UhdRadio::from_binding(&binding(selector.clone(), Some(x3xx())), fake(one_cbx())).unwrap();
    let capability = |name: &str| radio.instance().tree.capabilities[&Key::parse(name).unwrap()].clone();
    let range = |min, max| ezsdr_kernel::spec::CapabilityValue::Range { min, max };
    assert_eq!(capability("radio.rx.c0.frequency_hz"), range(Scalar::try_from(1.2e9).unwrap(), Scalar::try_from(6e9).unwrap()));
    assert_eq!(capability("radio.rx.c0.stream"), range(Scalar::from(-1), Scalar::from(0)));
    assert_eq!(capability("radio.tx.c0.frequency_hz"), range(Scalar::try_from(1.2e9).unwrap(), Scalar::try_from(6e9).unwrap()));
    assert_eq!(capability("radio.tx.c0.stream"), range(Scalar::from(-1), Scalar::from(0)));
    // A CBX has random phase on retune; the layout does not declare `radio.rx.coherent`.
    assert_eq!(capability("radio.rx.c0.phase_behavior_on_retune"), ezsdr_kernel::spec::CapabilityValue::One { value: Scalar::from("random".to_owned()) });
    assert!(!radio.instance().tree.capabilities.contains_key(&Key::parse("radio.rx.coherent").unwrap()));
    // A Session that sets no frequency tunes to the default, inside the CBX's range.
    let dir = TempDir::new();
    let mut doc = profile(&dir, selector, json!({}), true);
    doc["bindings"]["radio"]["profile"] = x3xx();
    let mut run = session(&doc, fake(one_cbx()));
    past_t0(&mut run, ms(1));
    let manifest = run.finish();
    let applied = section(&manifest, "applied").as_array().unwrap().iter().find(|r| r["key"] == "radio.rx.c0.frequency_hz").cloned().unwrap();
    assert_eq!(applied["claimed"], 2.45e9);
}

#[test]
fn ur_06_the_provider_refuses_another_root() {
    // Another device's root, whether its time was set at once or at a PPS edge, with its
    // clock internal or locked (UR-7).
    for (clock_source, time_source) in [("internal", "internal"), ("internal", "external"), ("external", "external")] {
        let dir = TempDir::new();
        let profile = profile(&dir, json!({ "authority_args": "addr=10.0.0.9", "clock_source": clock_source, "time_source": time_source }), json!({}), false);
        let manifest = spec_run_ended(&receive_spec(1, 1e6, 1e9, None), &profile, fake(FakeConfig::default()), BTreeMap::new());
        assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Prepare, .. }), "{clock_source} {time_source}");
        assert!(failure(&manifest).contains("UR-6: the primary root is not this device's"), "{clock_source} {time_source}: {}", failure(&manifest));
    }
}

// ---------------------------------------------------------------- the Authority (UR-7, UR-8)

fn authority_on(device: Arc<FakeDevice>, time_source: &str) -> (DeviceAuthority, Arc<FakeDevice>, ClockDomainId) {
    let clocks = Arc::new(ClockRegistry::new());
    let authority = DeviceAuthority::new(device.clone(), clocks, "internal", time_source, ARGS).unwrap();
    let root = authority.root();
    (authority, device, root)
}

/// The Authority's own tests on the smoke set's manual clock (plan 30 §4.4): their point is
/// device/virtual time, so the device's clock is what they wait on and measure.
fn authority(config: FakeConfig, time_source: &str) -> (DeviceAuthority, Arc<FakeDevice>, ClockDomainId) {
    authority_on(fake(config), time_source)
}

/// The Authority's own tests whose point is a real host latency (plan 30 §4.4): the wake of a
/// waiting `next_wakeup` and a callback that must not hold the schedule lock.
fn authority_real(config: FakeConfig, time_source: &str) -> (DeviceAuthority, Arc<FakeDevice>, ClockDomainId) {
    authority_on(fake_real(config), time_source)
}

#[test]
fn ur_07_the_authority_paces_to_the_device() {
    let (authority, device, root) = authority(FakeConfig::default(), "internal");
    let time = authority.time();
    // The device's own clock, which is what pacing means (plan 30 §4.4); the wall bound is a
    // watchdog only.
    let begun = device.host().now();
    let wall = Instant::now();
    let at = TimePoint::new(root, time.now(root).unwrap().ticks_in(root).unwrap() + ms(30));
    time.schedule(at, Box::new(|_| {})).unwrap();
    assert_eq!(authority.next_wakeup(), Some(at));
    assert!(device.host().now() - begun >= Wall::from_millis(29), "the device's clock advanced only {:?}", device.host().now() - begun);
    assert!(wall.elapsed() < Wall::from_secs(10), "{:?}", wall.elapsed());
    assert!(time.now(root).unwrap().ticks_in(root).unwrap() >= at.ticks_in(root).unwrap());
}

#[test]
fn ur_07_schedule_wakes_a_waiting_next_wakeup() {
    // Every nap is at most 20 ms (UR-7), so a missed wake only makes the waiter late by
    // up to one nap: the instant is 1 ms ahead, and the median lateness of 20 tries
    // tells a woken waiter (well under 1 ms) from one that sleeps out its nap (~10 ms).
    // A real host latency is the point, so this one keeps the real clock (plan 30 §4.4).
    let (authority, _, root) = authority_real(FakeConfig::default(), "internal");
    let authority = Arc::new(authority);
    let time = authority.time();
    let mut late = Vec::new();
    for _ in 0..20 {
        let far = time.schedule(TimePoint::new(root, time.now(root).unwrap().ticks_in(root).unwrap() + ms(1_000)), Box::new(|_| {})).unwrap();
        let waiter = {
            let a = authority.clone();
            std::thread::spawn(move || (a.next_wakeup(), Instant::now()))
        };
        std::thread::sleep(Wall::from_millis(7));
        let near = TimePoint::new(root, time.now(root).unwrap().ticks_in(root).unwrap() + ms(1));
        let due = Instant::now() + Wall::from_millis(1);
        time.schedule(near, Box::new(|_| {})).unwrap();
        let (woke, at) = waiter.join().unwrap();
        assert_eq!(woke, Some(near));
        late.push(at.saturating_duration_since(due));
        time.cancel(far);
    }
    late.sort();
    assert!(late[10] < Wall::from_millis(4), "median lateness {:?} of {late:?}", late[10]);
}

#[test]
fn ur_07_now_is_monotonic_across_a_reanchor() {
    // The real clock (plan 30 §4.4): the monotonic guard is `now`'s against the real clock's
    // bracket error. A re-anchor's read is bracketed in real time, so its midpoint can land
    // behind the last `now` returned. The fake steps its clock back 50 ms, more than the 1 ms
    // between two reads below, so a `now` that followed the anchor back shows if a re-anchor
    // lands in the window. A diagnostic of the integration: the guard itself, row U06, is
    // pinned without a window by `src/authority.rs`'s
    // `ur_07_now_does_not_follow_an_anchor_that_lands_behind_it` (R13).
    let (authority, device, root) = authority_real(FakeConfig { faults: vec![FakeFault::StepBack(Wall::from_millis(50), ms(50))], ..FakeConfig::default() }, "internal");
    let time = authority.time();
    let mut last = time.now(root).unwrap().ticks_in(root).unwrap();
    // The window is the real clock: a re-anchor runs every 100 ms of it.
    let until = device.host().now() + Wall::from_millis(400);
    while device.host().now() < until {
        let now = time.now(root).unwrap().ticks_in(root).unwrap();
        assert!(now >= last, "{now} < {last}");
        last = now;
        device.host().sleep(Wall::from_millis(1));
    }
}

// `ur_07_a_read_bracketed_wider_than_1_ms_is_discarded` and the first anchor's rule are unit
// tests in `src/authority.rs`, where a participant can hold the clock while the Authority is
// built (UR-7, plan 30).

#[test]
fn ur_07_the_time_is_set_at_the_next_pps_with_an_external_source() {
    let (_, device, _) = authority(FakeConfig::default(), "external");
    let calls = device.calls();
    let sources = calls.iter().position(|c| c == "set_sources internal external").unwrap();
    let zero = calls.iter().position(|c| c == "set_time_zero pps").unwrap();
    assert!(sources < zero, "{calls:?}");
}

#[test]
fn ur_07_the_root_s_epoch_is_the_pps_edge_with_external_sources() {
    // UR-6, UR-7, TM-3: the epoch is a PPS edge only when both sources are not internal (a
    // GPSDO locks as an external reference does), so that every whole second of the root is
    // one; `set_by` names how the time was set.
    let (now, pps) = (format!("ezsdr.radio.uhd.set_time_now:{ARGS}"), format!("ezsdr.radio.uhd.set_time_unknown_pps:{ARGS}"));
    for (clock_source, time_source, epoch) in [
        ("external", "external", EpochRef::Pps { set_by: pps.clone() }),
        ("gpsdo", "gpsdo", EpochRef::Pps { set_by: pps.clone() }),
        ("internal", "external", EpochRef::Arbitrary { set_by: pps.clone() }),
        ("external", "internal", EpochRef::Arbitrary { set_by: now.clone() }),
        ("internal", "internal", EpochRef::Arbitrary { set_by: now.clone() }),
    ] {
        let clocks = Arc::new(ClockRegistry::new());
        let authority = DeviceAuthority::new(fake(FakeConfig::default()), clocks.clone(), clock_source, time_source, ARGS).unwrap();
        let kind = clocks.get(authority.root()).unwrap().kind;
        assert!(matches!(kind, ezsdr_kernel::time::ClockDomainKind::Root { epoch: ref e, .. } if *e == epoch), "{clock_source} {time_source}: {kind:?}");
    }
}

#[test]
fn ur_07_schedule_accepts_an_instant_already_passed() {
    let (authority, device, root) = authority(FakeConfig::default(), "internal");
    let time = authority.time();
    let fired = time.now(root).unwrap();
    time.schedule(fired, Box::new(|_| {})).unwrap();
    assert_eq!(authority.next_wakeup(), Some(fired));
    device.host().sleep(Wall::from_millis(6));
    let passed = TimePoint::new(root, time.now(root).unwrap().ticks_in(root).unwrap() - ms(1));
    time.schedule(passed, Box::new(|_| {})).unwrap();
    // The instant it returns is the property; how fast is no verdict here (R21).
    assert_eq!(authority.next_wakeup(), Some(passed));
    assert!(matches!(time.schedule(TimePoint::new(root, fired.ticks_in(root).unwrap() - 1), Box::new(|_| {})), Err(ezsdr_kernel::time::TimeError::InPast { .. })));
}

#[test]
fn ur_07_callbacks_run_outside_the_schedule_lock() {
    // MA-29 as KG-3 amends it (the exit review's UR-7 clause): a callback may schedule at
    // its own instant, fired in the same wakeup, and another thread may schedule while a
    // callback runs. `next_wakeup` runs on a thread so that a callback holding the lock
    // fails the test in seconds instead of hanging it. The callback runs on until another
    // thread's `schedule` has returned, so that schedule cannot have waited for it: a handshake,
    // not a wall interval, decides it (R21), so the smoke set's manual clock serves.
    let (authority, _device, root) = authority(FakeConfig::default(), "internal");
    let time = authority.time();
    let t = TimePoint::new(root, time.now(root).unwrap().ticks_in(root).unwrap() + ms(20));
    let fired = Arc::new(Mutex::new(Vec::new()));
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (proceed, proceed_rx) = std::sync::mpsc::channel::<()>();
    let (inner, log) = (time.clone(), fired.clone());
    time.schedule(
        t,
        Box::new(move |at| {
            log.lock().unwrap().push("first");
            let log2 = log.clone();
            inner.schedule(at, Box::new(move |_| log2.lock().unwrap().push("second"))).unwrap();
            started_tx.send(()).unwrap();
            let _ = proceed_rx.recv();
            log.lock().unwrap().push("first ends");
        }),
    )
    .unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || done_tx.send(authority.next_wakeup()).unwrap());
    started_rx.recv_timeout(Wall::from_secs(3)).expect("the first callback ran");
    let (scheduled_tx, scheduled_rx) = std::sync::mpsc::channel();
    let other = time.clone();
    std::thread::spawn(move || {
        let _ = scheduled_tx.send(other.schedule(TimePoint::new(root, t.ticks_in(root).unwrap() + ms(1_000)), Box::new(|_| {})).is_ok());
    });
    assert_eq!(scheduled_rx.recv_timeout(Wall::from_secs(3)).ok(), Some(true), "another thread's schedule waited for the running callback");
    let _ = proceed.send(());
    let woke = done_rx.recv_timeout(Wall::from_secs(3)).expect("next_wakeup returned");
    assert_eq!(woke, Some(t));
    assert_eq!(*fired.lock().unwrap(), vec!["first", "first ends", "second"]);
}

#[test]
fn ur_07_cancel_does_not_remove_another_roots_callback() {
    let clocks = Arc::new(ClockRegistry::new());
    let a = DeviceAuthority::new(fake(FakeConfig::default()), clocks.clone(), "internal", "internal", "fake-a").unwrap();
    let b = DeviceAuthority::new(fake(FakeConfig::default()), clocks, "internal", "internal", "fake-b").unwrap();
    let fired = Arc::new(Mutex::new(false));
    let flag = fired.clone();
    // KG-9 permits a past instant before any callback has fired. No host deadline
    // is needed to test handle identity, and both queues get the same (tick, seq).
    let foreign = a.time().schedule(TimePoint::new(a.root(), 0), Box::new(|_| {})).unwrap();
    let own = b
        .time()
        .schedule(
            TimePoint::new(b.root(), 0),
            Box::new(move |_| {
                *flag.lock().unwrap() = true;
            }),
        )
        .unwrap();
    assert_ne!(foreign.root, own.root);
    assert_eq!((foreign.ticks, foreign.seq), (own.ticks, own.seq));
    assert!(!b.time().cancel(foreign));
    assert_eq!(b.next_wakeup(), Some(TimePoint::new(b.root(), 0)));
    assert!(*fired.lock().unwrap());
    assert!(a.time().cancel(foreign));
    assert!(!b.time().cancel(own));
}

#[test]
fn ur_07_cancel_wakes_a_waiting_next_wakeup() {
    // As above: without the wake, the waiter returns only when its nap ends. A real host
    // latency is the point, so this one keeps the real clock (plan 30 §4.4).
    let (authority, _, root) = authority_real(FakeConfig::default(), "internal");
    let authority = Arc::new(authority);
    let time = authority.time();
    let mut late = Vec::new();
    for _ in 0..20 {
        let handle = time.schedule(TimePoint::new(root, time.now(root).unwrap().ticks_in(root).unwrap() + ms(1_000)), Box::new(|_| {})).unwrap();
        let waiter = {
            let a = authority.clone();
            std::thread::spawn(move || (a.next_wakeup(), Instant::now()))
        };
        std::thread::sleep(Wall::from_millis(7));
        let begun = Instant::now();
        assert!(time.cancel(handle));
        let (woke, at) = waiter.join().unwrap();
        assert_eq!(woke, None);
        late.push(at.saturating_duration_since(begun));
    }
    late.sort();
    assert!(late[10] < Wall::from_millis(3), "median delay {:?} of {late:?}", late[10]);
}

#[test]
fn ur_07_now_tracks_the_device_while_no_call_runs() {
    // UR-7: a read of the Authority's clock lies inside the bracket of two of the device's own,
    // one before it and one after, so a delayed test thread widens the bracket and never the
    // verdict. The device does not drift here: the reading a drifting device leaves a re-anchor,
    // and `uhd-clock` that never re-anchors, are the unit test's
    // (`src/authority.rs`, `ur_07_uhd_clock_re_anchors_follow_a_drifting_device`), whose two
    // reads are made with the clock held and so do not depend on when a thread runs either.
    let (authority, device, root) = authority(FakeConfig::default(), "internal");
    let time = authority.time();
    // No `next_wakeup` and no `wait_until` for 2 s: only `uhd-clock`'s own reads anchor it.
    device.host().sleep(Wall::from_secs(2));
    let before = device.time_now().unwrap();
    let ours = time.now(root).unwrap().ticks_in(root).unwrap();
    let after = device.time_now().unwrap();
    assert!(before <= ours && ours <= after, "{ours} ticks outside [{before}, {after}]");
}

#[test]
fn ur_08_the_relations_bracket_a_device_read() {
    let (authority, _, root) = authority(FakeConfig::default(), "internal");
    let relations = authority.relations();
    assert_eq!(relations.iter().map(|r| r.target()).collect::<Vec<_>>(), [ClockDomainId::HOST_MONOTONIC, ClockDomainId::UTC]);
    for relation in &relations {
        assert_eq!(relation.source(), root);
        assert!(relation.uncertainty().ticks_in(relation.uncertainty().domain()).unwrap() >= 0 && relation.uncertainty().ticks_in(relation.uncertainty().domain()).unwrap() <= 10_000_000);
    }
    assert_eq!(relations[0].method(), "ezsdr.radio.uhd.host_bracket");
    assert_eq!(relations[1].method(), "ezsdr.radio.uhd.host_utc_bracket");
}

// ---------------------------------------------------------------- the profile and the instance (UR-9…UR-11)

fn provider() -> UhdRadio {
    UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), fake(FakeConfig::default())).unwrap()
}

#[test]
fn ur_09_profile_values_reach_the_capabilities_and_the_envelope_section() {
    let radio = provider();
    let instance = radio.instance();
    let capability = |name: &str| instance.tree.capabilities[&Key::parse(name).unwrap()].clone();
    let one = |v: Scalar| ezsdr_kernel::spec::CapabilityValue::One { value: v };
    assert_eq!(capability("radio.timing.min_timed_command_lead_ns"), one(Scalar::from(5_000_000)));
    assert_eq!(capability("radio.timing.startup_latency_ns"), one(Scalar::from(2_000_000_000)));
    assert_eq!(capability("radio.timing.overflow_restart_gap_ns"), one(Scalar::from(50_000_000)));
    assert_eq!(capability("radio.timing.rx_start_lead_ns"), one(Scalar::from(50_000_000)));
    assert_eq!(capability("radio.timing.tx_in_flight_ns"), one(Scalar::from(10_000_000)));
    assert_eq!(capability("radio.tx.repeat_max_samples"), one(Scalar::from(4_294_967_295)));
    assert_eq!(capability("radio.tx.repeat_align_samples"), one(Scalar::from(1)));
    assert_eq!(capability("radio.rx.block_len"), one(Scalar::from(2_000)));
    assert_eq!(capability("radio.perf.wire_bytes_per_sample"), one(Scalar::from(4)));
    assert_eq!(capability("radio.tx.c0.path_delay_samples"), one(Scalar::from(45)));
    let envelope = &section_of(instance, "envelope");
    assert_eq!(envelope["profile"]["name"], "x3xx");
    assert_eq!(envelope["timing"]["min_timed_command_lead_ns"], 5_000_000);
    assert_eq!(envelope["timing"]["startup_latency_ns"], 2_000_000_000i64);
    assert_eq!(envelope["timing"]["overflow_restart_gap_ns"], 50_000_000);
    assert_eq!(envelope["timing"]["rx_start_lead_ns"], 50_000_000);
    assert_eq!(envelope["timing"]["tx_in_flight_ns"], 10_000_000);
    assert_eq!(envelope["performance"]["rx_bytes_per_s"], 100_000_000);
    assert_eq!(section_of(instance, "device")["fake"], true);
}

#[test]
fn measurement_candidate_propagates_all_timing_fields_to_the_run_manifest() {
    let candidate = ezsdr_radio::TimingEnvelope {
        min_timed_command_lead_ns: 11_000_001,
        startup_latency_ns: 12_000_002,
        overflow_restart_gap_ns: 17_000_004,
        rx_start_lead_ns: 19_000_005,
        tx_in_flight_ns: 29_000_007,
    };
    let dir = TempDir::new();
    let doc = profile(&dir, json!({}), json!({}), false);
    let device = fake(FakeConfig::default());
    let run = Run::start_spec(&receive_spec(1, 1e6, 1e9, Some(100)), &doc, assembly(&doc, device, BTreeMap::new(), |radio| radio.with_measurement_timing(candidate)), Declared::default()).unwrap();
    let manifest = captured(run);
    let timing = &section(&manifest, "envelope")["timing"];
    for (field, value) in [
        ("min_timed_command_lead_ns", candidate.min_timed_command_lead_ns),
        ("startup_latency_ns", candidate.startup_latency_ns),
        ("overflow_restart_gap_ns", candidate.overflow_restart_gap_ns),
        ("rx_start_lead_ns", candidate.rx_start_lead_ns),
        ("tx_in_flight_ns", candidate.tx_in_flight_ns),
    ] {
        assert_eq!(timing[field], value, "{field}");
    }
}

#[test]
fn measurement_fake_rehearsal_round_trips_and_only_fails_source() {
    let device = fake(FakeConfig::default());
    let (heard, _) = rehearse_txrx_and_repeat(device.clone(), true);
    let records = bursts(&heard);
    let burst = &records[0];
    let artifact = capture_of(&heard, "rec");
    let continuity = &artifact.continuity[0];
    let target = burst.target.ticks_in(burst.target.domain()).unwrap();
    let first = continuity.first.ticks_in(continuity.domain).unwrap();
    let end = continuity.end.ticks_in(continuity.domain).unwrap();
    let observation = measurement::observation(
        "fake_txrx_run",
        "tests/fake.rs::measurement_fake_rehearsal_round_trips_and_only_fails_source",
        Some("txrx".to_owned()),
        1,
        json!({
            "burst_target": target,
            "burst_samples": burst.samples,
            "capture_first": first,
            "capture_end": end,
            "capture_gaps": continuity.gaps.len(),
        }),
    )
    .unwrap();
    assert_eq!(observation.result["burst_target"], target);
    assert_eq!(observation.result["burst_samples"], burst.samples);
    assert_eq!(observation.result["capture_first"], first);
    assert_eq!(observation.result["capture_end"], end);
    assert_eq!(observation.result["capture_gaps"], continuity.gaps.len());

    let dir = TempDir::new();
    let doc = bench_profile(&*device, &dir, json!({}), bench_envelope(&*device), false);
    let binding = measurement::radio_binding(&doc).unwrap();
    let uhd = measurement::description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
    let condition = ezsdr_radio::measurement::Condition {
        id: "txrx".to_owned(),
        streams: vec![measurement::stream_condition(ezsdr_kernel::stream::Direction::Rx, 0, vec![0], 1e6), measurement::stream_condition(ezsdr_kernel::stream::Direction::Tx, 0, vec![0], 1e6)],
        concurrent: vec![measurement::stream_ref(ezsdr_kernel::stream::Direction::Rx, 0), measurement::stream_ref(ezsdr_kernel::stream::Direction::Tx, 0)],
        reopened: false,
    };
    let record = measurement::build_record(
        ezsdr_radio::measurement::Source::Fake,
        &binding,
        &*device,
        &measurement::rehearsal_metadata(),
        measurement::MeasurementGroup { conditions: vec![condition], values: vec![], observations: vec![observation] },
        &uhd,
    )
    .unwrap();
    let path = dir.0.join("fake-measurement.json");
    measurement::write_record(&path, &record).unwrap();
    let round_trip: ezsdr_radio::measurement::Record = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(round_trip, record);
    assert_eq!(round_trip.observations[0].result["burst_target"], target, "the observation must come from the Run's burst record");
    assert_eq!(round_trip.observations[0].result["capture_first"], first, "the observation must come from the Run's capture continuity");
    assert!(round_trip.values.iter().all(|value| matches!(value, ezsdr_radio::measurement::ValueRecord::Unmeasured { reason, .. } if !reason.trim().is_empty())));
    let violations = ezsdr_radio::measurement::compare(&round_trip, &uhd);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert_eq!(violations[0].key, "source");
}

#[test]
fn provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements() {
    // The whole group list runs through a guard on the device: none of it makes the guard latch (an open of a stream still open,
    // an `apply` error, a timeout).
    let inner = fake(one_obx());
    let guard = Arc::new(measurement::GuardedDevice::new(inner.clone(), measurement::Latch::new()));
    let device: Arc<dyn Device> = guard.clone();
    let plan = measurement::MeasurementPlan::rehearsal();
    let dir = TempDir::new();
    let doc = bench_profile(&*device, &dir, json!({}), bench_envelope(&*device), false);
    let binding = measurement::radio_binding(&doc).unwrap();
    let description = measurement::description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
    let mut group = measurement::MeasurementGroup::default();
    for (name, measure, keys) in measurement::provider_groups() {
        let supplied = measure(device.clone(), &plan).unwrap();
        let found: Vec<_> = supplied
            .values
            .iter()
            .map(|row| match row {
                ezsdr_radio::measurement::ValueRecord::Measured { key, .. } | ezsdr_radio::measurement::ValueRecord::Unmeasured { key, .. } => key.as_str().to_owned(),
            })
            .collect();
        assert_eq!(found, keys, "the keys a failed `{name}` group marks unmeasured are the ones it supplies");
        group.append(supplied).unwrap();
    }
    let (raw_group, stopped, restore_error) = measurement::measure_raw_group(device.clone(), &description, &plan).unwrap();
    assert_eq!((stopped, restore_error), (None, None));
    group.append(raw_group).unwrap();

    let assert_stage = |stage: &Json, trials: usize| {
        let rows = stage["trials"].as_array().expect("every candidate has trial rows");
        assert_eq!(rows.len(), trials);
        assert!(rows.iter().all(|row| row["class"].as_str().is_some_and(|class| !class.is_empty())));
    };
    let assert_stages = |stages: &Json, count: usize, trials: usize| {
        let stages = stages.as_array().expect("candidate stages are an array");
        assert_eq!(stages.len(), count);
        for stage in stages {
            assert_stage(stage, trials);
        }
    };
    let result = |name: &str| group.observations.iter().find(|row| row.name == name).map(|row| &row.result).unwrap();
    assert_eq!(guard.latched(), None, "no group of the list makes the guard latch");
    assert_eq!(result("provider_overflow_restart_gap_with_tx")["observation_only"], true);
    assert_stages(&result("provider_min_timed_command_lead")["candidates"], plan.timed_command_leads_ns.len(), plan.step_trials);
    let one_channel_row = |prefix: &str, row: &&ezsdr_radio::measurement::Observation| row.name.strip_prefix(prefix).is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()));
    let throughput: Vec<_> = group.observations.iter().filter(|row| one_channel_row("provider_rx_throughput_", row)).collect();
    assert_eq!(throughput.len(), plan.rx_rates_hz.len());
    assert!(
        !group.observations.iter().any(|row| row.name.contains("throughput") && row.name.contains("duplex")),
        "no throughput stage runs both directions: the pair of the two limits is tried once, after both values"
    );
    // The pair, once both throughput values are decided: one stream each way on the one-channel conditions, no value.
    let pair = measurement::measure_duplex_at_limits(device.clone(), &plan, measurement::duplex_limits(&group), 4).unwrap();
    assert!(pair.values.is_empty() && pair.observations[0].result["ran"] == true, "{:?}", pair.observations[0].result);
    for row in throughput {
        assert_stage(&row.result["stage"], 1);
    }
    let tx_throughput: Vec<_> = group.observations.iter().filter(|row| one_channel_row("provider_tx_throughput_", row)).collect();
    assert_eq!(tx_throughput.len(), plan.tx_throughput_rates_hz.len());
    // Each transmit run's burst keeps its device-send count, for the bench's question of how many 1 000-sample sends one burst takes.
    for row in &tx_throughput {
        assert!(row.result["burst_blocks"].as_array().is_some_and(|blocks| blocks.len() == 1 && blocks[0].as_u64().is_some_and(|n| n >= 1)), "{:?}", row.result);
    }
    for row in tx_throughput {
        assert_stage(&row.result["stage"], 1);
        for key in ["burst_target", "burst_start_tick", "burst_ack_count", "burst_ack_tick", "ack_elapsed_samples", "actual_samples"] {
            assert!(row.result.contains_key(key), "TX observation omits {key}: {:?}", row.result);
        }
    }
    let overflow = result("provider_overflow_restart_gap");
    let overflow_rates = overflow["rate_measurements"].as_array().expect("overflow rates are an array");
    assert_eq!(overflow_rates.len(), plan.overflow_rates_hz.len());
    for rate in overflow_rates {
        let trials = rate["trials"].as_array().expect("overflow trials are an array");
        assert_eq!(trials.len(), plan.overflow_trials);
        assert!(trials.iter().all(|row| row["class"].as_str().is_some_and(|class| !class.is_empty())));
        assert!(rate.get("maximum_restart_after_resume_ns").is_some());
        assert!(trials.iter().all(|row| row.get("restart_after_resume_ns").is_some() && row.get("restart_after_resume_reason").is_some() && row.get("gaps").is_some()));
    }
    let duplex = result("provider_full_duplex")["trials"].as_array().unwrap();
    assert_eq!(duplex.len(), plan.loopback_trials);
    assert!(duplex.iter().all(|row| row["class"].as_str().is_some_and(|class| !class.is_empty())));

    let coercion: Vec<_> = group.observations.iter().filter(|row| row.name.starts_with("raw_coercion_")).collect();
    assert_eq!(coercion.len(), 10);
    for row in &coercion {
        let points = row.result["points"].as_array().unwrap();
        assert_eq!(row.trials as usize, points.len().max(1), "{}", row.name);
        let expected = match row.name.as_str() {
            "raw_coercion_rx_c0_gain_db" | "raw_coercion_tx_c0_gain_db" => 64,
            "raw_coercion_rx_c0_gain_step_db" | "raw_coercion_tx_c0_gain_step_db" => 316,
            "raw_coercion_rx_c0_antenna" => description.rx[0].antennas.len(),
            "raw_coercion_tx_c0_antenna" => description.tx[0].antennas.len(),
            "raw_coercion_rx_c0_frequency_hz" | "raw_coercion_tx_c0_frequency_hz" => plan.frequency_points,
            "raw_coercion_rx_s0_sample_rate_hz" | "raw_coercion_tx_s0_sample_rate_hz" => match &description.rates {
                ezsdr_radio::device::Grid::Values(values) => values.len(),
                other => panic!("the x3xx sample-rate profile must be enumerable: {other:?}"),
            },
            other => panic!("unexpected raw coercion observation {other}"),
        };
        assert_eq!(points.len(), expected, "{}", row.name);
        for point in points {
            for field in ["requested", "readback", "error"] {
                assert!(point.get(field).is_some(), "{} point omits {field}: {point}", row.name);
            }
        }
        if row.name == "raw_coercion_tx_c0_frequency_hz" {
            let beyond = points.iter().filter(|point| point["requested"].as_f64().is_some_and(|hz| !SAFE_TX_HZ.contains(&hz))).count();
            assert!(beyond > 0, "the frequency sweep covers the whole profile grid, not the transmit safe band");
        }
    }

    let calls_before_restore = inner.calls().len();
    measurement::restore_measurement_defaults(&*device, &description, &measurement::RawGroup::with_limit(Wall::from_secs(60))).unwrap();
    let calls = inner.calls();
    let restored = &calls[calls_before_restore..];
    assert_eq!(restored.len(), description.rx.len() + description.tx.len());
    for (dir, channels) in [(Dir::Rx, &description.rx), (Dir::Tx, &description.tx)] {
        for (channel_index, channel) in channels.iter().enumerate() {
            let expected = format!("apply {} {channel_index} rate={} freq=1000000000 gain=0 antenna={}", dir.name(), ezsdr_radio::channels::DEFAULT_SAMPLE_RATE_HZ, channel.default_antenna,);
            assert!(restored.contains(&expected), "missing restored settings: {expected}; {restored:?}");
        }
    }

    // The transmit frequency sweep runs at gain 0 and ends back on the safe 1 GHz; no streamer is opened for it.
    let calls = inner.calls();
    let first_sweep = calls.iter().position(|call| call.starts_with("apply tx 0 rate=- freq=10000000 gain=-")).expect("the transmit sweep starts at the grid's low end");
    assert_eq!(calls[first_sweep - 1], "apply tx 0 rate=- freq=- gain=0 antenna=-");
    let last_sweep = calls.iter().rposition(|call| call.starts_with("apply tx 0 rate=- freq=8400000000 gain=-")).expect("the transmit sweep ends at the grid's high end");
    assert_eq!(calls[last_sweep + 1], "apply tx 0 rate=- freq=1000000000 gain=- antenna=-");

    let measured_keys: std::collections::BTreeSet<_> = group
        .values
        .iter()
        .map(|row| match row {
            ezsdr_radio::measurement::ValueRecord::Measured { key, .. } | ezsdr_radio::measurement::ValueRecord::Unmeasured { key, .. } => key.as_str().to_owned(),
        })
        .collect();
    assert!(!measured_keys.contains("radio.timing.overflow_restart_gap_ns"), "overflow contributes observations but no value row");
    assert_eq!(
        measured_keys,
        [
            "radio.full_duplex",
            "radio.hardware_time",
            "radio.perf.rx_bytes_per_s",
            "radio.perf.tx_bytes_per_s",
            "radio.timing.min_timed_command_lead_ns",
            "radio.timing.rx_start_lead_ns",
            "radio.rx.c0.gain_db",
            "radio.rx.c0.gain_step_db",
            "radio.rx.c0.antenna",
            "radio.rx.s0.sample_rate_hz",
            "radio.tx.c0.gain_db",
            "radio.tx.c0.gain_step_db",
            "radio.tx.c0.antenna",
            "radio.tx.s0.sample_rate_hz",
            "radio.rx.c0.frequency_hz",
            "radio.rx.c0.frequency_step_hz",
            "radio.tx.c0.frequency_hz",
            "radio.tx.c0.frequency_step_hz",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );

    for key in ["radio.rx.c0.frequency_hz", "radio.tx.c0.frequency_hz"] {
        assert!(group.values.iter().any(|row| matches!(row, ezsdr_radio::measurement::ValueRecord::Measured { key: found, worst: ezsdr_radio::measurement::Worst::Grid(ezsdr_radio::measurement::GridValue::Step { step, .. }), .. }
            if found.as_str() == key && *step == 1.0)), "{key}: the fake reads every frequency back, so the sweep is a value before the fake override");
    }
    let record = measurement::build_record(ezsdr_radio::measurement::Source::Fake, &binding, &*device, &measurement::rehearsal_metadata(), group, &description).unwrap();
    assert!(record.values.iter().all(|row| matches!(row, ezsdr_radio::measurement::ValueRecord::Unmeasured { reason, .. } if !reason.trim().is_empty())));
    assert!(record.values.iter().any(|row| matches!(row,
        ezsdr_radio::measurement::ValueRecord::Unmeasured { key, reason }
            if key.as_str() == "radio.timing.overflow_restart_gap_ns" && reason.contains("pending the first hardware bench reading"))));
    for key in &measured_keys {
        assert!(record.values.iter().any(|row| matches!(row,
            ezsdr_radio::measurement::ValueRecord::Unmeasured { key: found, reason }
                if found.as_str() == key && reason == "FakeDevice は実機の証拠ではない")));
    }
    let path = dir.0.join("provider-measurement-rehearsal.json");
    measurement::write_record(&path, &record).unwrap();
    let round_trip: ezsdr_radio::measurement::Record = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(round_trip, record);
    let violations = ezsdr_radio::measurement::compare(&round_trip, &description);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert_eq!(violations[0].key, "source");
}

fn raw_group_fixture(config: FakeConfig) -> (Arc<FakeDevice>, ezsdr_radio::channels::Description, measurement::MeasurementPlan) {
    let device = fake(config);
    let dir = TempDir::new();
    let doc = bench_profile(&*device, &dir, json!({}), bench_envelope(&*device), false);
    let binding = measurement::radio_binding(&doc).unwrap();
    let description = measurement::description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
    (device, description, measurement::MeasurementPlan::rehearsal())
}

fn unmeasured_reason<'a>(group: &'a measurement::MeasurementGroup, key: &str) -> &'a str {
    group
        .values
        .iter()
        .find_map(|row| match row {
            ezsdr_radio::measurement::ValueRecord::Unmeasured { key: found, reason } if found.as_str() == key => Some(reason.as_str()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{key} is not an unmeasured row"))
}

#[test]
fn raw_group_sends_nothing_after_its_first_failed_apply() {
    // The first untimed receive apply is the first gain request: it fails, and that apply is the
    // device's last call: no later request, no restore.
    let (device, description, plan) = raw_group_fixture(FakeConfig { faults: vec![FakeFault::RxApplyFails], ..one_obx() });
    let (group, stopped, restore_error) = measurement::measure_raw_group(device.clone(), &description, &plan).unwrap();
    let stopped = stopped.expect("the group stopped on the failed apply");
    assert!(stopped.contains("rx channel 0 Gain request 0") && stopped.contains("scripted refusal"), "{stopped}");
    assert_eq!(restore_error, None, "a skipped restore is not a restore error");
    assert_eq!(device.calls().len(), 1, "{:?}", device.calls());
    for key in measurement::raw_group_keys(&description) {
        let reason = unmeasured_reason(&group, &key);
        assert!(reason.contains("stopped the raw group") && reason.contains("scripted refusal"), "{key}: {reason}");
    }
    let restore = &group.observations.iter().find(|row| row.name == "raw_restore").unwrap().result;
    assert_eq!(restore["status"], "skipped");
}

#[test]
fn the_raw_group_deadline_is_checked_between_two_device_calls() {
    // The second untimed apply moves the group's elapsed time past its limit (injected, not slept):
    // the third is never sent.
    let (device, description, plan) = raw_group_fixture(one_obx());
    let probe = Arc::new(probe_device::ProbeDevice::new(device));
    let raw = Arc::new(measurement::RawGroup::with_limit(Wall::from_secs(3_600)));
    let injected = raw.clone();
    probe.before_untimed_apply(move |count| {
        if count == 2 {
            injected.advance_elapsed(Wall::from_secs(7_200))
        }
    });
    let group = measurement::measure_coercion(probe.clone(), &description, &plan, &raw).unwrap();
    assert!(raw.failure().is_some_and(|reason| reason.contains("wall-clock limit")));
    let log = probe.control_log();
    assert_eq!(log.iter().filter(|call| **call == "apply").count(), 2, "{log:?}");
    assert_eq!(log.last(), Some(&"apply"), "no call after the one that crossed the deadline: {log:?}");
    assert!(unmeasured_reason(&group, "radio.rx.c0.gain_db").contains("wall-clock limit"));
}

/// The raw group's record run as the caller runs it: the groups, the raw group last, the record's
/// writing (which panics when `write_panics`), and the lease dropped as the run unwinds.
fn run_record_flow(
    probe: &Arc<probe_device::ProbeDevice>,
    description: &ezsdr_radio::channels::Description,
    binding: &ezsdr_kernel::binding::Binding,
    plan: &measurement::MeasurementPlan,
    write_panics: bool,
) -> (Option<String>, Option<String>) {
    let (mut stopped, mut restored) = (None, None);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut lease = measurement::DeviceLease::new(probe.clone());
        let mut groups = measurement::GroupRun::new(None);
        (stopped, restored) = measurement::run_raw_group_leased(&mut lease, &mut groups, description, plan);
        let (described, serial) = (probe.describe(), probe.serial().unwrap());
        let record =
            measurement::build_record_with_identity(ezsdr_radio::measurement::Source::Fake, binding, described, serial, &measurement::rehearsal_metadata(), groups.take(), description).unwrap();
        assert!(!record.values.is_empty());
        if write_panics {
            panic!("the record could not be written");
        }
    }));
    assert_eq!(outcome.is_err(), write_panics);
    (stopped, restored)
}

#[test]
fn a_lease_leaks_the_handle_when_the_run_unwinds_before_it_is_marked_clean() {
    // A panic while a group's failure is reported (before the raw group decides) must not drop it.
    let (device, _description, _plan) = raw_group_fixture(one_obx());
    let probe = Arc::new(probe_device::ProbeDevice::new(device));
    let log = probe.log_handle();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lease = measurement::DeviceLease::new(probe.clone());
        panic!("reporting a failed group");
    }));
    assert!(outcome.is_err());
    drop(probe);
    assert!(!log.lock().unwrap().contains(&"drop"), "no teardown before the run is marked clean");
}

#[test]
fn a_failed_restore_leaks_the_handle_even_when_the_record_s_writing_panics() {
    let (device, description, plan) = raw_group_fixture(one_obx());
    let dir = TempDir::new();
    let binding = measurement::radio_binding(&bench_profile(&*device, &dir, json!({}), bench_envelope(&*device), false)).unwrap();
    let probe = Arc::new(probe_device::ProbeDevice::new(device));
    let log = probe.log_handle();
    probe.fail_restoring_apply();
    let (stopped, restored) = run_record_flow(&probe, &description, &binding, &plan, true);
    assert!(restored.is_some_and(|error| error.contains("scripted restore failure")), "the restore failed");
    assert!(stopped.is_some_and(|reason| reason.contains("scripted restore failure")), "the group's state is read after the restore");
    drop(probe);
    assert!(!log.lock().unwrap().contains(&"drop"), "no teardown on the failed handle: {:?}", log.lock().unwrap().last());
    // The same run with nothing failed drops the handle as usual, also as it unwinds.
    let (device, description, plan) = raw_group_fixture(one_obx());
    let binding = measurement::radio_binding(&bench_profile(&*device, &dir, json!({}), bench_envelope(&*device), false)).unwrap();
    let probe = Arc::new(probe_device::ProbeDevice::new(device));
    let log = probe.log_handle();
    assert_eq!(run_record_flow(&probe, &description, &binding, &plan, true), (None, None));
    drop(probe);
    assert!(log.lock().unwrap().contains(&"drop"));
}

#[test]
fn the_record_is_assembled_from_an_identity_read_before_the_groups_with_no_device() {
    let (device, description, _) = raw_group_fixture(one_obx());
    let dir = TempDir::new();
    let binding = measurement::radio_binding(&bench_profile(&*device, &dir, json!({}), bench_envelope(&*device), false)).unwrap();
    let (described, serial) = (device.describe(), device.serial().unwrap());
    drop(device); // the record needs no device any more: its identity is already in hand
    let record = measurement::build_record_with_identity(
        ezsdr_radio::measurement::Source::Fake,
        &binding,
        described,
        serial.clone(),
        &measurement::rehearsal_metadata(),
        measurement::MeasurementGroup::default(),
        &description,
    )
    .unwrap();
    assert_eq!(record.device.serial, serial);
}

#[test]
fn ur_7_retries_only_the_reference_clock_not_locking() {
    let lock = format!("{} to its external source", ezsdr_radio_uhd::REFERENCE_DID_NOT_LOCK);
    let attempts = std::cell::Cell::new(0);
    let ok_third = open_with_pll_retries(
        || {
            attempts.set(attempts.get() + 1);
            if attempts.get() < 3 { Err(lock.clone()) } else { Ok(attempts.get()) }
        },
        3,
        Wall::ZERO,
    );
    assert_eq!((ok_third, attempts.get()), (Ok(3), 3));
    attempts.set(0);
    let exhausted: Result<(), String> = open_with_pll_retries(
        || {
            attempts.set(attempts.get() + 1);
            Err(lock.clone())
        },
        3,
        Wall::ZERO,
    );
    assert_eq!((exhausted.is_err(), attempts.get()), (true, 4), "the first try and three retries");
    attempts.set(0);
    let other: Result<(), String> = open_with_pll_retries(
        || {
            attempts.set(attempts.get() + 1);
            Err("UR-7: the time reset failed".to_owned())
        },
        3,
        Wall::ZERO,
    );
    assert_eq!((other.unwrap_err().as_str(), attempts.get()), ("UR-7: the time reset failed", 1), "another error is returned at once");
    // The Authority's own message for the fake's PLL fault is the one retried.
    let device = fake(FakeConfig { faults: vec![FakeFault::ReferenceDoesNotLock], ..FakeConfig::default() });
    let error = DeviceAuthority::new(device, Arc::new(ClockRegistry::new()), "external", "internal", ARGS).err().expect("the fault fails the open");
    assert!(error.contains(ezsdr_radio_uhd::REFERENCE_DID_NOT_LOCK), "{error}");
}

#[test]
fn a_probe_s_long_arrays_are_summarised_and_their_rows_go_to_a_side_file() {
    let dir = TempDir::new();
    let output = dir.0.join("record.json");
    let rows: Vec<Json> =
        (0..500).map(|index| json!({ "call": if index % 2 == 0 { "rx_recv" } else { "tx_send" }, "ns": index, "error": if index == 7 { json!("boom") } else { Json::Null } })).collect();
    let numbers: Vec<Json> = (0..100).map(|index| json!(index)).collect();
    let mut groups = measurement::GroupRun::new(Some(&output));
    groups.probe("hw_big", || measurement::push_raw_observation("hw_big", 1, json!({ "rows": rows, "numbers": numbers, "short": [1, 2, 3], "nested": [{ "inner": rows }] })));
    let measured = groups.take();
    let result = &measured.observations[0].result;
    assert_eq!(result["rows"]["summarised_rows"], 500);
    assert_eq!(result["rows"]["counts"]["call"]["rx_recv"], 250);
    assert_eq!(result["rows"]["outliers"][0]["ns"], 7);
    assert_eq!(result["rows"]["first"].as_array().unwrap().len(), 3);
    assert_eq!((result["numbers"]["min"].as_f64(), result["numbers"]["median"].as_f64(), result["numbers"]["max"].as_f64()), (Some(0.0), Some(50.0), Some(99.0)));
    assert_eq!(result["short"], json!([1, 2, 3]));
    assert_eq!(result["nested"][0]["inner"]["summarised_rows"], 500, "an array inside a short array is summarised too");
    assert!(serde_json::to_vec(&measured.observations[0]).unwrap().len() < 10_000, "the record keeps no rows");
    let file = std::fs::read_to_string(dir.0.join("record.json.rows").join("hw_big.jsonl")).unwrap();
    assert_eq!(file.lines().count(), 500 + 100 + 500);
    let first: Json = file.lines().map(|line| serde_json::from_str::<Json>(line).unwrap()).find(|line| line["path"] == "rows").unwrap();
    assert_eq!((first["index"].as_u64(), first["row"]["ns"].as_u64()), (Some(0), Some(0)));
    assert!(measured.observations[0].note.as_deref().is_some_and(|note| note.contains("hw_big.jsonl")));
    let partial: Json = serde_json::from_slice(&std::fs::read(groups.partial_path().unwrap()).unwrap()).unwrap();
    assert!(serde_json::to_vec(&partial).unwrap().len() < 10_000, "the partial file has the same summary");
}

#[test]
fn coercion_stops_after_its_first_failed_apply() {
    let (device, description, plan) = raw_group_fixture(FakeConfig { faults: vec![FakeFault::RxApplyFails], ..one_obx() });
    let raw = measurement::RawGroup::new(&plan);
    let group = measurement::measure_coercion(device.clone(), &description, &plan, &raw).unwrap();
    assert_eq!(device.calls().len(), 1, "the first receive gain request failed and nothing followed: {:?}", device.calls());
    assert!(raw.failure().is_some_and(|reason| reason.contains("rx channel 0 Gain request 0")));
    for key in ["radio.rx.c0.gain_db", "radio.rx.c0.frequency_hz", "radio.tx.c0.antenna", "radio.tx.s0.sample_rate_hz"] {
        assert!(unmeasured_reason(&group, key).contains("stopped the raw group"), "{key}");
    }
}

#[test]
fn coercion_stopped_midway_keeps_what_it_measured_and_marks_the_rest_unmeasured() {
    // 64 gain, 316 gain-step and 2 antenna requests precede the receive frequency sweep; the 5th of its requests fails.
    let (device, description, plan) = raw_group_fixture(one_obx());
    let probe = Arc::new(probe_device::ProbeDevice::new(device));
    probe.fail_untimed_apply(64 + 316 + 2 + 5);
    let raw = measurement::RawGroup::new(&plan);
    let group = measurement::measure_coercion(probe.clone(), &description, &plan, &raw).unwrap();
    assert_eq!(probe.control_log().last(), Some(&"apply_failed"), "nothing was sent after the failed apply");
    let measured = |key: &str| group.values.iter().any(|row| matches!(row, ezsdr_radio::measurement::ValueRecord::Measured { key: found, .. } if found.as_str() == key));
    for key in ["radio.rx.c0.gain_db", "radio.rx.c0.gain_step_db", "radio.rx.c0.antenna"] {
        assert!(measured(key), "{key} was complete before the failure");
    }
    assert!(unmeasured_reason(&group, "radio.rx.c0.frequency_hz").contains("scripted apply failure"));
    for key in ["radio.rx.s0.sample_rate_hz", "radio.tx.c0.gain_db", "radio.tx.c0.frequency_hz"] {
        assert!(unmeasured_reason(&group, key).contains("stopped the raw group"), "{key}");
    }
}

#[test]
fn coercion_opens_no_streamer() {
    let (device, description, plan) = raw_group_fixture(one_obx());
    let probe = Arc::new(probe_device::ProbeDevice::new(device));
    measurement::measure_coercion(probe.clone(), &description, &plan, &measurement::RawGroup::new(&plan)).unwrap();
    let log = probe.control_log();
    assert!(log.contains(&"apply") && !log.contains(&"rx_open") && !log.contains(&"tx_open"), "the sweeps are untimed applies on a device with no streamer");
}

#[test]
fn raw_group_limit_stops_every_later_call() {
    let (device, description, plan) = raw_group_fixture(one_obx());
    let over = measurement::RawGroup::with_limit(Wall::ZERO);
    let group = measurement::measure_coercion(device.clone(), &description, &plan, &over).unwrap();
    assert!(over.failure().is_some_and(|reason| reason.contains("wall-clock limit")));
    assert!(device.calls().is_empty(), "the limit was already passed: {:?}", device.calls());
    assert!(unmeasured_reason(&group, "radio.rx.c0.gain_db").contains("wall-clock limit"));
    assert!(unmeasured_reason(&group, "radio.tx.c0.frequency_hz").contains("wall-clock limit"));
    let latch = measurement::RawGroup::with_limit(Wall::from_secs(60));
    latch.fail("first");
    latch.fail("second");
    assert_eq!(latch.failure().as_deref(), Some("first"), "only the first failure is kept");
    let mut short = measurement::MeasurementPlan::rehearsal();
    short.raw_group_limit_ms = 1;
    let raw = measurement::RawGroup::new(&short);
    std::thread::sleep(Wall::from_millis(10));
    assert!(raw.halted(), "the plan's limit is the group's limit");
}

#[test]
fn raw_group_restores_after_a_panic_in_it() {
    // An untimed apply panics in the coercion sweep (no failure was latched); the group unwinds and its guard restores.
    let (device, description, plan) = raw_group_fixture(one_obx());
    let probe = Arc::new(probe_device::ProbeDevice::new(device.clone()));
    probe.before_untimed_apply(|count| {
        if count == 2 {
            panic!("scripted panic in an untimed apply")
        }
    });
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measurement::measure_raw_group(probe.clone(), &description, &plan)));
    assert!(outcome.is_err());
    let calls = device.calls();
    let restored = |dir: &str, antenna: &str| format!("apply {dir} 0 rate={} freq=1000000000 gain=0 antenna={antenna}", ezsdr_radio::channels::DEFAULT_SAMPLE_RATE_HZ);
    assert_eq!(calls.last(), Some(&restored("tx", &description.tx[0].default_antenna)), "{calls:?}");
    assert!(calls.contains(&restored("rx", &description.rx[0].default_antenna)), "{calls:?}");
}

#[test]
fn group_run_keeps_the_other_groups_after_an_error_or_a_panic() {
    let (device, description, plan) = raw_group_fixture(one_obx());
    let dir = TempDir::new();
    let output = dir.0.join("record.json");
    let mut groups = measurement::GroupRun::new(Some(&output));
    let keys = |keys: &[&str]| keys.iter().map(|key| (*key).to_owned()).collect::<Vec<_>>();
    groups.run("fine", &keys(&["radio.timing.min_timed_command_lead_ns"]), || measurement::measure_min_timed_command_lead(device.clone(), &plan));
    groups.run("errors", &keys(&["radio.perf.rx_bytes_per_s"]), || Err("the device refused".to_owned()));
    groups.run("panics", &keys(&["radio.full_duplex", "radio.hardware_time"]), || -> Result<_, String> {
        let seven = std::hint::black_box(7);
        panic!("boom {seven}")
    });
    groups.probe("probe_panics", || panic!("probe boom"));
    groups.probe("probe_fine", || measurement::push_raw_observation("probe_fine", 1, json!({ "x": 1 })));
    assert_eq!(groups.failed().iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["errors", "panics", "probe_panics"]);
    assert_eq!(groups.failed()[2].1, "panic: probe boom");
    assert_eq!(groups.probes().iter().map(String::as_str).collect::<Vec<_>>(), ["probe_fine", "probe_panics"]);
    let partial = groups.partial_path().unwrap().to_owned();
    assert_eq!(partial, dir.0.join("record.json.partial.json"));
    let written: Json = serde_json::from_slice(&std::fs::read(&partial).unwrap()).unwrap();
    assert_eq!(written["completed"], json!(["fine", "probe_fine"]));
    assert_eq!(written["failed"].as_array().unwrap().len(), 3);
    let measured = groups.take();
    let reason = |key: &str| unmeasured_reason(&measured, key).to_owned();
    assert!(reason("radio.perf.rx_bytes_per_s").contains("the device refused"));
    assert!(reason("radio.full_duplex").contains("panic: boom 7") && reason("radio.hardware_time").contains("panic: boom 7"));
    assert!(
        measured.values.iter().any(|row| matches!(row, ezsdr_radio::measurement::ValueRecord::Measured { key, .. } if key.as_str() == "radio.timing.min_timed_command_lead_ns")
            || matches!(row, ezsdr_radio::measurement::ValueRecord::Unmeasured { key, .. } if key.as_str() == "radio.timing.min_timed_command_lead_ns")),
        "the fine group's row stays"
    );
    for name in ["group_failure_errors", "group_failure_panics", "group_failure_probe_panics", "provider_min_timed_command_lead", "probe_fine"] {
        assert!(measured.observations.iter().any(|row| row.name == name), "{name}");
    }
    let binding = measurement::radio_binding(&bench_profile(&*device, &dir, json!({}), bench_envelope(&*device), false)).unwrap();
    let record = measurement::build_record(ezsdr_radio::measurement::Source::Fake, &binding, &*device, &measurement::rehearsal_metadata(), measured, &description).unwrap();
    measurement::write_record(&output, &record).unwrap();
    groups.remove_partial();
    assert!(!partial.exists(), "the partial file goes once the record is written");
    let violations = ezsdr_radio::measurement::compare(&record, &description);
    assert_eq!(violations.len(), 1, "{violations:?}");
}

#[test]
fn rejected_runs_are_admission_rejections_not_group_failures() {
    // 400 MS/s is more than the bench profile's 10 GbE link carries: validate refuses the Run, which has no T0.
    let (device, _, mut plan) = raw_group_fixture(one_obx());
    plan.rx_rates_hz = vec![390_625, 400_000_000];
    plan.tx_throughput_rates_hz = vec![1_000_000, 400_000_000];
    let rx = measurement::measure_rx_throughput(device.clone(), &plan).unwrap();
    let tx = measurement::measure_tx_throughput(device.clone(), &plan).unwrap();
    for (group, name, key) in [(&rx, "provider_rx_throughput_400000000", "radio.perf.rx_bytes_per_s"), (&tx, "provider_tx_throughput_400000000", "radio.perf.tx_bytes_per_s")] {
        let row = group.observations.iter().find(|row| row.name == name).unwrap();
        assert_eq!(row.result["stage"]["trials"][0]["class"], "AdmissionRejection", "{:?}", row.result);
        assert!(row.result["run_termination"].as_str().is_some_and(|text| !text.is_empty()));
        let accepted = group.observations.iter().find(|row| row.name == name.replace("400000000", if name.contains("_tx_") { "1000000" } else { "390625" })).unwrap();
        assert!(!accepted.result.contains_key("run_termination"), "the rate below the link's capacity ran");
        let wire = 4.0 * if name.contains("_tx_") { 1_000_000.0 } else { 390_625.0 };
        assert!(
            group.values.iter().any(|row| matches!(row, ezsdr_radio::measurement::ValueRecord::Measured { key: found, worst: ezsdr_radio::measurement::Worst::Number(bytes), .. }
            if found.as_str() == key && *bytes == wire)),
            "{key}: the accepted rate is the capacity and the refused one is not claimed"
        );
    }
    plan.overflow_rates_hz = vec![390_625, 400_000_000];
    plan.overflow_stall_ms = vec![300, 300];
    let overflow = measurement::measure_overflow_restart_gap(device, &plan).unwrap();
    let rates = overflow.observations[0].result["rate_measurements"].as_array().unwrap();
    assert_eq!(rates[1]["trials"][0]["class"], "AdmissionRejection");
    assert!(rates[1]["trials"][0]["run_termination"].as_str().is_some_and(|text| !text.is_empty()));
    assert!(rates[0]["trials"][0]["run_termination"].is_null());
}

#[test]
fn measurement_defaults_restoration_stops_at_the_first_apply_error() {
    let device = fake(FakeConfig { faults: vec![FakeFault::RxApplyFails], ..one_obx() });
    let dir = TempDir::new();
    let doc = bench_profile(&*device, &dir, json!({}), bench_envelope(&*device), false);
    let binding = measurement::radio_binding(&doc).unwrap();
    let description = measurement::description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
    let calls_before = device.calls().len();
    let raw = measurement::RawGroup::with_limit(Wall::from_secs(60));

    let error = measurement::restore_measurement_defaults(&*device, &description, &raw).unwrap_err();
    assert!(error.contains("could not restore Rx channel 0"), "{error}");
    let calls = device.calls();
    let restored = &calls[calls_before..];
    assert_eq!(restored.len(), 1, "no channel is restored after the first apply error: {restored:?}");
    assert!(raw.failure().is_some_and(|reason| reason.contains("could not restore Rx channel 0")));
    assert!(measurement::restore_measurement_defaults(&*device, &description, &raw).is_err());
    assert_eq!(device.calls().len(), calls_before + 1, "a stopped group restores nothing");
}

#[test]
fn ur_10_instance() {
    let radio = provider();
    let instance = radio.instance();
    assert!(!instance.driving.stepped);
    assert_eq!(instance.min_command_lead, Some(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000)));
    assert!(instance.arm_after.is_empty());
    assert_eq!(instance.profile.as_ref().unwrap().name, "x3xx");
    assert_eq!(instance.id, ResourceId::parse("usrp").unwrap());
    assert_eq!(instance.tree.memory_domains, [ezsdr_hostmem::HOST_MEMORY]);
}

fn request(pairs: &[(&str, Scalar)]) -> ezsdr_kernel::module_api::Requested {
    ezsdr_kernel::module_api::Requested {
        resource: ResourceId::parse("usrp").unwrap(),
        constraints: pairs.iter().map(|(k, v)| (Key::parse(k).unwrap(), Constraint::Eq { value: v.clone() })).collect(),
    }
}

#[test]
fn ur_11_coerce_is_the_descriptions() {
    let radio = provider();
    let report = radio.coerce(&request(&[("radio.rx.s0.sample_rate_hz", Scalar::try_from(19.5e6).unwrap())])).unwrap();
    assert_eq!(report.applied[&Key::parse("radio.rx.s0.sample_rate_hz").unwrap()], Value::num(20e6).unwrap());
    assert_eq!(report.coercions.len(), 1);
    assert!(!radio.coerce(&request(&[("radio.rx.c0.frequency_hz", Scalar::try_from(7e9).unwrap())])).unwrap().rejected.is_empty());
    assert!(!radio.coerce(&request(&[("radio.rx.c2.frequency_hz", Scalar::try_from(1e9).unwrap())])).unwrap().rejected.is_empty());
}

#[test]
fn ur_11_a_transmit_rate_below_one_msps_is_rejected() {
    // KL-40: a transmit stream on `x3xx` starts at 1 MS/s: 0.5 MS/s is rejected, not coerced to it; receive keeps 0.5 MS/s.
    let radio = provider();
    let key = Key::parse("radio.tx.s0.sample_rate_hz").unwrap();
    let report = radio.coerce(&request(&[("radio.tx.s0.sample_rate_hz", Scalar::try_from(500_000.0).unwrap())])).unwrap();
    assert_eq!(report.rejected.len(), 1, "{:?}", report.rejected);
    assert_eq!(report.rejected[0].key, key);
    assert!(report.coercions.is_empty() && !report.applied.contains_key(&key));
    let report = radio.coerce(&request(&[("radio.tx.s0.sample_rate_hz", Scalar::try_from(1e6).unwrap())])).unwrap();
    assert!(report.rejected.is_empty() && report.coercions.is_empty());
    let report = radio.coerce(&request(&[("radio.rx.s0.sample_rate_hz", Scalar::try_from(500_000.0).unwrap())])).unwrap();
    assert!(report.rejected.is_empty() && report.coercions.is_empty(), "{report:?}");
}

// ---------------------------------------------------------------- prepare, arm, start (UR-12…UR-16)

struct NoActions;
impl ActionReceiver for NoActions {
    fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
        None
    }
}
struct NoSubmit;
impl ActionSubmitter for NoSubmit {
    fn submit(&self, _: Action) -> Result<ActionId, Vec<ezsdr_kernel::binding::Violation>> {
        Ok(ActionId(1))
    }
}

/// A unit `prepare` of the UHD Provider with a hand-built context.
fn direct_prepare(radio: &mut UhdRadio, device: Arc<FakeDevice>, class: ExecutionClass, links: Vec<AttachedPort>, constraints: &[(&str, Scalar)]) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
    let clocks = Arc::new(ClockRegistry::new());
    let authority = DeviceAuthority::new(device, clocks.clone(), "internal", "internal", ARGS).unwrap();
    let mut registry = ModuleRegistry::new();
    let (mut checks, mut kinds) = (AdmissionCheckRegistry::new(), EventKindRegistry::with_kernel_kinds());
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
    let pairs: Vec<_> = ["usrp", "usrp/rx/s0", "usrp/tx/s0", "usrp/rx/s1", "usrp/tx/s1", "usrp/rx/s2", "usrp/tx/s2", "usrp/rx/s3", "usrp/tx/s3"]
        .iter()
        .flat_map(|s| kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k)))
        .collect();
    let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 1024, &Policy::default()));
    let ctx = {
        let mut ctx = PrepareContext::testing(EventSource::Node { node: ResourceId::parse("usrp").unwrap() }, authority.time(), clocks, events, Arc::new(NoActions), Arc::new(NoSubmit));
        ctx.run = RunId::from_string("unit".to_owned());
        ctx.class = class;
        ctx.environment = Arc::new(BTreeMap::new());
        ctx.inputs = Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new());
        ctx.links = links;
        ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
        ctx
    };
    let fragment = Fragment {
        id: Ident::parse("radio").unwrap(),
        instance: ezsdr_radio_uhd::module_ref(),
        role: Role::Provider,
        content: json!({ "selector": {}, "requested": request(constraints) }),
        after: Vec::new(),
    };
    radio.prepare(&fragment, ctx).map(|_| ())
}

fn attached(policy: ezsdr_kernel::stream::BackPressure) -> AttachedPort {
    attached_on("rx0", policy)
}

/// A receive link on a layout stream's port — `rx0`, `rx1` (RM-29).
fn attached_on(port: &str, policy: ezsdr_kernel::stream::BackPressure) -> AttachedPort {
    let decl = DataLinkDecl {
        id: ezsdr_kernel::id::DataLinkId::local(0),
        from: ezsdr_kernel::contract::PortRef { component: Ident::parse("radio").unwrap(), port: Ident::parse(port).unwrap() },
        to: ezsdr_kernel::contract::PortRef { component: Ident::parse("rec").unwrap(), port: Ident::parse("in").unwrap() },
        contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        policy,
        capacity: 4,
    };
    let link = ezsdr_link_host::HostLinkModule::new().create(&decl).unwrap();
    AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse(port).unwrap(), Endpoint::StreamOut(link))
}

/// `attached_on`, with the test's own handle of the link, so that it can read the blocks a
/// stream publishes (the fake's per-channel loopback, UR-33).
fn attached_reading(port: &str) -> (AttachedPort, Arc<dyn ezsdr_kernel::stream::DataLink>) {
    let decl = DataLinkDecl {
        id: ezsdr_kernel::id::DataLinkId::local(0),
        from: ezsdr_kernel::contract::PortRef { component: Ident::parse("radio").unwrap(), port: Ident::parse(port).unwrap() },
        to: ezsdr_kernel::contract::PortRef { component: Ident::parse("rec").unwrap(), port: Ident::parse("in").unwrap() },
        contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        policy: ezsdr_kernel::stream::BackPressure::DropOldest,
        // Every block of the Run, so that a test can read each segment's own.
        capacity: 4096,
    };
    let link = ezsdr_link_host::HostLinkModule::new().create(&decl).unwrap();
    (AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse(port).unwrap(), Endpoint::StreamOut(link.clone())), link)
}

/// A queue the test pushes Actions into, as the coordinator dispatches them: stamped with the device clock at the push, so that an Action is received at the first poll strictly after it
/// (R2, `common::rig::Stamped`).
struct Pushed {
    actions: rig::Stamped<Action>,
    given: std::sync::atomic::AtomicU64,
}
impl Pushed {
    fn new(clock: ezsdr_radio_uhd::HostClock) -> Pushed {
        Pushed { actions: rig::Stamped::new(Some(clock)), given: std::sync::atomic::AtomicU64::new(0) }
    }
}
impl ActionReceiver for Pushed {
    // The coordinator's resolution (KC-23): the test's target is the node it resolves to,
    // `<resource>[/<path>]`, so a test may name a stream's node; ids count from 1, as the
    // Kernel's do (KC-24).
    fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
        let id = || ActionId(self.given.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1);
        self.actions.take().map(|action| {
            let node = action.target().and_then(|target| match target {
                Target::Resource { resource, path } if path.is_empty() => ResourceId::parse(resource.as_str()).ok(),
                Target::Resource { resource, path } => ResourceId::parse(&format!("{resource}/{path}")).ok(),
                _ => None,
            });
            ezsdr_kernel::module_api::Dispatched::new(id(), action, node)
        })
    }
}

/// The Provider driven without the coordinator: prepared, armed and started at `S`,
/// with Actions pushed into its queue and events read from its collector.
struct Direct {
    /// First, so that it drops before the Authority: `DeviceAuthority::drop` waits in real time for `uhd-clock`, and a thread still registered (busy) would hold the clock that `uhd-clock` may be
    /// asleep on (note 51 §4.2). On a real clock (`Gated`) it registers nothing.
    _driver: ezsdr_radio_uhd::host::Driver,
    radio: UhdRadio,
    device: Arc<FakeDevice>,
    queue: Arc<Pushed>,
    events: Arc<EventCollector>,
    time: Arc<dyn ezsdr_kernel::time::TimeAuthority>,
    root: ClockDomainId,
    clocks: Arc<ClockRegistry>,
    delivered: Vec<Event>,
    _authority: AuthorityGuard,
}

impl Direct {
    fn with_links(config: FakeConfig, constraints: &[(&str, Scalar)], links: Vec<AttachedPort>) -> Direct {
        Direct::build(config, constraints, links, json!({}))
    }

    /// With selector values beside `args` (UR-5).
    fn build(config: FakeConfig, constraints: &[(&str, Scalar)], links: Vec<AttachedPort>, selector: Json) -> Direct {
        Direct::build_with(config, constraints, links, selector, BTreeMap::new())
    }

    /// `build` with the Run's Command inputs, for a test that books a waveform (UR-21).
    fn build_with(config: FakeConfig, constraints: &[(&str, Scalar)], links: Vec<AttachedPort>, selector: Json, inputs: BTreeMap<ContentHash, Vec<u8>>) -> Direct {
        let mut direct = Direct::armed_with(config, constraints, links, selector, inputs);
        direct.start();
        direct
    }

    /// The Provider prepared and armed, before `start` (UR-15): what a segment's declaration
    /// and a streamer's opening can be observed on without any block (RM-25).
    fn armed(config: FakeConfig, constraints: &[(&str, Scalar)], links: Vec<AttachedPort>, selector: Json) -> Direct {
        Direct::armed_with(config, constraints, links, selector, BTreeMap::new())
    }

    fn armed_with(config: FakeConfig, constraints: &[(&str, Scalar)], links: Vec<AttachedPort>, selector: Json, inputs: BTreeMap<ContentHash, Vec<u8>>) -> Direct {
        let device = fake(config);
        Direct::armed_on(device.clone(), device, constraints, links, selector, inputs)
    }

    fn armed_on(device: Arc<FakeDevice>, tested: Arc<dyn Device>, constraints: &[(&str, Scalar)], links: Vec<AttachedPort>, selector: Json, inputs: BTreeMap<ContentHash, Vec<u8>>) -> Direct {
        let clocks = Arc::new(ClockRegistry::new());
        let authority = DeviceAuthority::new(tested.clone(), clocks.clone(), "internal", "internal", ARGS).unwrap_or_else(|error| {
            // The Authority's first read has a real-time bound (UR-7, 5 s): an expiry is the host's, so it is a marker (note 51 §6.2).
            if error.contains("did not come within") {
                infra(&format!("the rig's Authority: {error}"));
            }
            panic!("{error}")
        });
        let (time, root) = (authority.time(), authority.root());
        let mut registry = ModuleRegistry::new();
        let (mut checks, mut kinds) = (AdmissionCheckRegistry::new(), EventKindRegistry::with_kernel_kinds());
        ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
        // The instance's own nodes under either key set (RM-29): a layout's events come
        // from its stream nodes.
        let pairs: Vec<_> = ["usrp", "usrp/rx/s0", "usrp/tx/s0", "usrp/rx/s1", "usrp/tx/s1", "usrp/rx/s2", "usrp/tx/s2", "usrp/rx/s3", "usrp/tx/s3"]
            .iter()
            .flat_map(|s| kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k)))
            .collect();
        let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()));
        let host = tested.host();
        let queue = Arc::new(Pushed::new(host.clone()));
        let ctx = {
            let mut ctx = PrepareContext::testing(EventSource::Node { node: ResourceId::parse("usrp").unwrap() }, time.clone(), clocks.clone(), events.clone(), queue.clone(), Arc::new(NoSubmit));
            ctx.run = RunId::from_string("direct".to_owned());
            ctx.class = ExecutionClass::HardwareInLoop;
            ctx.environment = Arc::new(BTreeMap::new());
            ctx.inputs = Arc::new(inputs.into_iter().map(|(hash, bytes)| (hash, Arc::<[u8]>::from(bytes))).collect::<BTreeMap<_, _>>());
            ctx.links = links;
            ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
            ctx
        };
        let mut keys = json!({ "args": ARGS });
        keys.as_object_mut().unwrap().extend(selector.as_object().cloned().unwrap_or_default());
        let mut radio = UhdRadio::from_binding(&binding(keys, Some(x3xx())), tested).unwrap();
        // RM-30: no channel is in a stream unless a request puts it there; these runs receive on c0.
        let mut constraints = constraints.to_vec();
        if radio.instance().tree.capabilities.contains_key(&Key::parse("radio.rx.c0.stream").unwrap()) && !constraints.iter().any(|(name, _)| *name == "radio.rx.c0.stream") {
            constraints.push(("radio.rx.c0.stream", Scalar::from(0)));
        }
        let fragment = Fragment {
            id: Ident::parse("radio").unwrap(),
            instance: ezsdr_radio_uhd::module_ref(),
            role: Role::Provider,
            content: json!({ "selector": {}, "requested": request(&constraints) }),
            after: Vec::new(),
        };
        radio.prepare(&fragment, ctx).unwrap();
        radio.arm().unwrap();
        // Last, so that no error or unwind path above holds a live registration; the Authority's first read (5 s), `prepare` and `arm` stay outside it.
        let driver = host.drive();
        Direct { _driver: driver, radio, device, queue, events, time, root, clocks, delivered: Vec::new(), _authority: AuthorityGuard::new(authority) }
    }

    /// Starts the armed Provider at T0 and waits a millisecond past it (UR-15).
    fn start(&mut self) {
        let t0 = self.time.now(self.root).unwrap().ticks_in(self.root).unwrap() + ms(2_000) + ms(1);
        self.radio.start(Some(TimePoint::new(self.root, t0))).unwrap();
        self.wait_until(t0 + ms(1));
    }

    /// Advances the device clock to `tick` as an idle participant, then ends the wait (R1).
    fn wait_until(&mut self, tick: i64) {
        self.time.wait_until(TimePoint::new(self.root, tick)).unwrap();
        self.end_wait();
    }

    /// The end of every wait that ends on an instant the test chose (R1, test plan 1c "M3"; `until` ends on an observed state, which is whole once seen): one microsecond more on the clock, so every
    /// participant due at the instant the wait ended on has run (M4 holds the instant until each has). The order of the test thread and a poll at the instant *after* it is the queue's, not the wait's:
    /// an Action is received at the first poll strictly after its push (R2, `rig::Stamped`).
    fn end_wait(&self) {
        self.device.host().sleep(Wall::from_micros(1));
    }

    fn now(&self) -> i64 {
        self.time.now(self.root).unwrap().ticks_in(self.root).unwrap()
    }

    fn push(&self, action: Action) {
        self.queue.actions.push(action);
    }

    fn update(&self, key: &str, value: Value, class: ezsdr_kernel::module_api::UpdateClass, at: Option<i64>) {
        self.update_at("usrp", key, value, class, at);
    }

    /// An update whose target resolves to `node` (KC-23): `usrp`, or a stream's node such
    /// as `usrp/rx/s0` (RM-29).
    fn update_at(&self, node: &str, key: &str, value: Value, class: ezsdr_kernel::module_api::UpdateClass, at: Option<i64>) {
        self.push(Action::UpdateParameter {
            target: resource(node),
            key: Key::parse(key).unwrap(),
            value,
            class,
            at: at.map(|t| ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(self.root, t))),
        });
    }

    /// A `Stop` whose target resolves to `node` (KC-23, UR-26).
    fn stop_at(&self, node: &str) {
        self.push(Action::Stop { target: Some(resource(node)) });
    }

    /// `start_rx` at `node`, at `at` (RM-12, RM-21).
    fn start_rx_at(&self, node: &str, at: Option<i64>) {
        self.push(Action::Command {
            target: resource(node),
            verb: Ident::parse("start_rx").unwrap(),
            params: BTreeMap::new(),
            at: at.map(|t| ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(self.root, t))),
        });
    }

    /// A `TxBurst` at `node`, its waveform an input of the Run (UR-21); `at` is in the
    /// transmit stream's SampleClock, which the burst must name (UR-21).
    fn burst_at(&self, node: &str, waveform: &ezsdr_kernel::manifest::ArtifactRef, repeat: bool, at: TimePoint) {
        self.push(Action::TxBurst {
            target: resource(node),
            waveform: waveform.clone(),
            repeat,
            at: ezsdr_kernel::time::AbsoluteDeadline::new(at),
            requested_at: None,
            late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
            metadata: BTreeMap::new(),
        });
    }

    fn settle(&mut self, wall: Wall) -> Vec<Event> {
        // The test thread is a participant (`_driver`): idle in this wait, so the device's time advances; between waits it is busy and the clock stands (note 51 §4.2).
        self.device.host().sleep(wall);
        self.end_wait();
        self.delivered.extend(self.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)));
        self.delivered.clone()
    }

    /// Advances the fake host clock until the observed state is reached, within 5 s of device time (the test's own verdict) and 10 s of real time (the host's: the marker).
    fn until(&mut self, what: &str, reached: impl FnMut(&mut Self) -> bool) {
        self.until_within(what, Wall::from_secs(5), Wall::from_secs(10), reached);
    }

    /// `horizon` is device time, the one the state must be reached in: a plain failure. `bound` is real time, which a stopped clock never ends: its expiry says nothing about the code under test,
    /// so it writes the marker (note 51 §6.2). The horizon is checked first, so a failure that took `horizon` of device time stays a verdict. Arguments only so that a fixture can expire either.
    fn until_within(&mut self, what: &str, horizon: Wall, bound: Wall, mut reached: impl FnMut(&mut Self) -> bool) {
        let begun = self.device.host().now();
        let deadline = Instant::now() + bound;
        while !reached(self) {
            assert!(self.device.host().now() - begun < horizon, "{what} did not come within {horizon:?} of device time: {:?}", self.device.calls());
            if Instant::now() >= deadline {
                infra(&format!("the rig's wait for {what} did not end within {bound:?} of real time"));
                panic!("timed out waiting for {what}: {:?}", self.device.calls());
            }
            self.device.host().sleep(Wall::from_millis(1));
            self.delivered.extend(self.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)));
        }
    }

    fn of(&mut self, name: &str) -> Vec<Event> {
        self.delivered.extend(self.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)));
        self.delivered.iter().filter(|e| e.kind == kind(name)).cloned().collect()
    }

    fn finish(mut self) -> ezsdr_kernel::module_api::ProviderInstance {
        self.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
        self.radio.cleanup();
        self.radio.instance().clone()
    }
}

/// An instance's Manifest section `suffix` (UR-30).
fn section_of(instance: &ezsdr_kernel::module_api::ProviderInstance, suffix: &str) -> Json {
    instance.sections[&Namespace::parse(&format!("ezsdr.radio.uhd.{suffix}")).unwrap()].clone()
}

/// The `timing` section's `plan` row (UR-30), each stream's segments in stream order.
fn plan_row(timing: &Json) -> Json {
    let plans: Vec<&Json> = timing.as_array().unwrap().iter().filter(|row| row["what"] == "plan").collect();
    assert_eq!(plans.len(), 1, "UR-30: exactly one plan row: {timing}");
    plans[0].clone()
}

/// The plan's rows of `dir`'s stream `k`, in order (UR-30).
fn plan_stream(plan: &Json, dir: &str, k: u64) -> Vec<Json> {
    plan[dir].as_array().unwrap().iter().filter(|row| row["stream"] == k).cloned().collect()
}

#[test]
fn ur_12_prepare_refusals() {
    let device = fake(FakeConfig::default());
    let mut radio = provider();
    let error = direct_prepare(&mut radio, device.clone(), ExecutionClass::Simulation, Vec::new(), &[]).unwrap_err();
    assert_eq!(error.kind, ModuleErrorKind::Unsupported);
    assert!(error.message.starts_with("UR-12: ezsdr.radio.uhd runs HardwareInLoop or Hardware"));
    let mut radio = provider();
    direct_prepare(&mut radio, device.clone(), ExecutionClass::HardwareInLoop, Vec::new(), &[]).unwrap();
    let again = direct_prepare(&mut radio, device.clone(), ExecutionClass::HardwareInLoop, Vec::new(), &[]).unwrap_err();
    assert!(again.message.contains("already called"), "{}", again.message);
    let mut radio = provider();
    let block = direct_prepare(&mut radio, device.clone(), ExecutionClass::HardwareInLoop, vec![attached(ezsdr_kernel::stream::BackPressure::Block)], &[]).unwrap_err();
    assert!(block.message.contains("Block"), "{}", block.message);
    let unknown_device = fake(FakeConfig { serial: "ur12-unknown-key".to_owned(), ..FakeConfig::default() });
    let mut radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), unknown_device.clone()).unwrap();
    let applies = |device: &FakeDevice| device.calls().iter().filter(|call| call.starts_with("apply")).count();
    let before = applies(&unknown_device);
    let unknown = direct_prepare(&mut radio, unknown_device.clone(), ExecutionClass::HardwareInLoop, Vec::new(), &[("radio.rx.c2.frequency_hz", Scalar::try_from(1e9).unwrap())]).unwrap_err();
    assert_eq!(unknown.kind, ModuleErrorKind::Rejected);
    assert!(unknown.message.contains("RM-4:"), "{}", unknown.message);
    assert_eq!(applies(&unknown_device), before, "prepare called the device before rejecting the unknown key: {:?}", unknown_device.calls());
}

#[test]
fn ur_12_a_tx_only_layout_declares_no_receive_clock() {
    // UR-12, design 33 D5: the Module configures a direction, and declares its stream's
    // SampleClock, only when its channel set is non-empty. A `tx`-only layout has no receive
    // stream and declares no receive clock.
    let direct = Direct::build(FakeConfig::default(), &[("radio.tx.c0.stream", Scalar::from(0))], Vec::new(), json!({ "rx": [], "tx": ["0:A:ubx:0"] }));
    let clocks: Vec<String> = direct.clocks.sample_clock_records().iter().map(|record| record.stream.path().to_owned()).collect();
    assert_eq!(clocks, ["usrp/tx/s0"], "a layout with no receive front end declares no receive clock");
    let instance = direct.finish();
    let applied = section_of(&instance, "applied");
    assert!(applied.as_array().unwrap().iter().all(|row| !row["key"].as_str().unwrap_or("").starts_with("radio.rx")), "no receive channel is configured: {applied}");
}

#[test]
fn ur_12_the_applied_values_are_recorded() {
    let dir = TempDir::new();
    let mut spec = receive_spec(1, 1e6, 2.4e9, Some(2_000));
    spec["resources"]["radio"]["requires"]["radio.rx.c0.gain_db"] = json!({ "kind": "eq", "value": 10.3 });
    let manifest = captured(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), fake(FakeConfig::default()), BTreeMap::new()));
    let applied = section(&manifest, "applied").as_array().unwrap().clone();
    let row = |key: &str| applied.iter().find(|r| r["key"] == key).unwrap_or_else(|| panic!("{key}: {applied:?}")).clone();
    assert_eq!(row("radio.rx.c0.frequency_hz")["claimed"], 2.4e9);
    assert_eq!(row("radio.rx.c0.frequency_hz")["read_back"], 2.4e9);
    assert_eq!(row("radio.rx.c0.gain_db")["claimed"], 10.5);
    assert_eq!(row("radio.rx.c0.gain_db")["read_back"], 10.5);
}

#[test]
fn ur_12_a_rate_the_device_does_not_apply_is_refused() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::WrongRate { claimed: 20e6, applied: 19.9e6, nth: 0 }], ..FakeConfig::default() });
    let manifest = spec_run_ended(&receive_spec(1, 20e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Prepare, .. }));
    assert!(failure(&manifest).contains("UR-12: the device applied 19900000 S/s for the claimed 20000000"), "{}", failure(&manifest));
}

#[test]
fn ur_12_every_advertised_rate_is_accepted() {
    // UR-9 advertises 200 MHz / N for UHD's decimations N (design 33 D5); UR-12 accepts
    // each (Review L, P0-2).
    let selector = std::collections::BTreeMap::from([
        (Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
        (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
    ]);
    let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
    let description = ezsdr_radio::channels::Description::new(&layout, 2_000, &ezsdr_radio_uhd::profile::terms());
    let ezsdr_radio::device::Grid::Values(rates) = description.rates else { panic!("a Values grid") };
    assert_eq!(rates.len(), 639);
    for rate in &rates {
        let n = ezsdr_radio_uhd::decimation(200_000_000, *rate).unwrap_or_else(|| panic!("{rate} refused"));
        assert_eq!(200_000_000.0 / n as f64, *rate);
    }
    // 200 MHz / 257 is not a decimation the DDC can make (design 33 D5), and 513 is past
    // every one; 19.5 Msps is off the grid.
    assert_eq!(ezsdr_radio_uhd::decimation(200_000_000, 200e6 / 257.0), None);
    assert_eq!(ezsdr_radio_uhd::decimation(200_000_000, 200e6 / 258.0), Some(258));
    assert_eq!(ezsdr_radio_uhd::decimation(200_000_000, 200e6 / 2040.0), Some(2040));
    assert_eq!(ezsdr_radio_uhd::decimation(200_000_000, 19.5e6), None);
    assert_eq!(ezsdr_radio_uhd::decimation(200_000_000, 200e6 / 513.0), None);
    // The fake clips a rate onto the grid as UHD's `coerce_decim` does (UR-33): 200e6/257
    // lies between 256 and 258, and the nearer decimation wins, ties to the smaller.
    assert_eq!(ezsdr_radio::x3xx::clip_decimation(200_000_000, 200e6 / 257.0), 256);
    assert_eq!(ezsdr_radio::x3xx::clip_decimation(200_000_000, 200e6 / 257.4), 258);
    assert_eq!(ezsdr_radio::x3xx::clip_decimation(200_000_000, 200e6 / 2041.0), 2040);
    let fake_device = FakeDevice::new(FakeConfig::default());
    let applied = fake_device.apply(ezsdr_radio_uhd::Dir::Rx, 0, &ezsdr_radio_uhd::Settings { rate: Some(200e6 / 257.0), ..ezsdr_radio_uhd::Settings::default() }).unwrap();
    assert_eq!(applied.rate, 200e6 / 256.0, "the fake clips like the device");
    let applied = fake_device.apply(ezsdr_radio_uhd::Dir::Rx, 0, &ezsdr_radio_uhd::Settings { rate: Some(200e6 / 2041.0), ..ezsdr_radio_uhd::Settings::default() }).unwrap();
    assert_eq!(applied.rate, 200e6 / 2040.0, "and to the top of the grid");
    // 200 MHz / 3 prepares and captures.
    let dir = TempDir::new();
    let manifest = captured(spec_run(&receive_spec(1, 200e6 / 3.0, 1e9, Some(1_000)), &profile(&dir, json!({ "link_gbps": [10] }), json!({}), false), fake(FakeConfig::default()), BTreeMap::new()));
    assert_eq!(capture_of(&manifest, "rec").size_bytes, 8_000, "{}", failure(&manifest));
    assert_eq!(manifest.clocks.sample_clocks[0].root_ticks_per_tick.num(), 3);
    // ...and a Session asks for 200 MHz / 7 at connect.
    let dir = TempDir::new();
    let profile = profile(&dir, json!({ "link_gbps": [10] }), json!({}), true);
    let mut initial = rx0(&profile);
    initial.get_mut(&Ident::parse("radio").unwrap()).unwrap().insert(Key::parse("radio.rx.s0.sample_rate_hz").unwrap(), Scalar::Num(ezsdr_kernel::spec::Finite::new(200e6 / 7.0).unwrap()));
    let mut run = session_with(&profile, fake(FakeConfig::default()), &initial);
    past_t0(&mut run, ms(1));
    wait(&mut run, ms(200));
    let manifest = run.finish();
    assert!(rejections(&manifest).is_empty(), "{:?}", rejections(&manifest));
    let new = manifest.clocks.sample_clocks.iter().find(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap()).expect("the receive clock");
    assert_eq!(new.root_ticks_per_tick.num(), 7);
}

#[test]
fn ur_13_arm_cases() {
    let dir = TempDir::new();
    let unlocked = fake(FakeConfig { faults: vec![FakeFault::Unlocked(Wall::ZERO)], ..FakeConfig::default() });
    let manifest = spec_run_ended(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({ "clock_source": "external" }), json!({}), false), unlocked, BTreeMap::new());
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Arm, .. }));
    assert!(failure(&manifest).contains("UR-13: the external reference is not locked"));
    let run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({ "clock_source": "external" }), json!({}), false), fake(FakeConfig::default()), BTreeMap::new());
    running(&run);
    let _ = run.finish();
}

#[test]
fn ur_15_start_cases() {
    let dir = TempDir::new();
    let early = profile(&dir, json!({}), json!({ "ezsdr.time": { "class": "hardware_in_loop", "start_lead_ns": 1_000_000_000u64 } }), false);
    let manifest = spec_run_ended(&receive_spec(1, 1e6, 1e9, None), &early, fake(FakeConfig::default()), BTreeMap::new());
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Arm, .. }));
    assert!(failure(&manifest).contains("UR-15: the start at"), "{}", failure(&manifest));
    assert_eq!(events_of(&manifest, "radio.LATE_COMMAND").len(), 1);
    let manifest = captured(spec_run(&receive_spec(1, 1e6, 1e9, Some(1_000)), &profile(&dir, json!({}), json!({}), false), fake(FakeConfig::default()), BTreeMap::new()));
    let first = section(&manifest, "timing").as_array().unwrap().iter().find(|r| r["what"] == "first_rx_block").cloned().unwrap();
    assert_eq!(first["index"], 0);
}

#[test]
fn ur_15_every_clock_starts_on_its_lattice() {
    let dir = TempDir::new();
    let odd = profile(&dir, json!({}), json!({ "ezsdr.time": { "class": "hardware_in_loop", "start_lead_ns": 2_000_000_003u64 } }), false);
    let spec = with_tx(receive_spec(1, 1e6, 1e9, Some(1_000)), 1e6);
    let manifest = captured(spec_run(&spec, &odd, fake(FakeConfig::default()), BTreeMap::new()));
    assert_eq!(manifest.clocks.sample_clocks.len(), 2);
    for record in &manifest.clocks.sample_clocks {
        assert_eq!(record.origin.ticks_in(record.root).unwrap() % 200, 0, "{record:?}");
    }
}

/// F4 (design-notes §17): a lost device is marked as such before anything could free
/// its streamers, so that `UhdDevice` frees neither them nor itself.
fn assert_marked_lost_before_closed(device: &FakeDevice) {
    let calls = device.calls();
    let marked = calls.iter().position(|c| c == "mark_lost").unwrap_or_else(|| panic!("the device was never marked lost: {calls:?}"));
    assert!(calls.iter().filter(|c| *c == "mark_lost").count() == 1, "{calls:?}");
    assert!(calls[..marked].iter().all(|c| c != "close_streams"), "closed before it was marked lost: {calls:?}");
}

#[test]
fn ur_16_a_panicking_thread_is_a_lost_device() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::RxPanic(Wall::from_millis(2_200))], ..FakeConfig::default() });
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new()).expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
    let result = run.run_until_end(after(&run, ms(5_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
    assert!(section(&manifest, "rejected").as_array().unwrap().iter().any(|r| r["thread"] == "uhd-rx-s0"));
    assert_marked_lost_before_closed(&device);
}

fn wedged(dir: &TempDir) -> (Manifest, Arc<FakeDevice>, Wall) {
    // TxBlocks is an intentional permanent host stop: it keeps the real clock (plan 30 §4.4).
    let device = fake_real(FakeConfig { faults: vec![FakeFault::TxBlocks], ..FakeConfig::default() });
    let (bytes, waveform) = waveform_of(&tone(1_000));
    let spec = with_burst(with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &waveform, true, "send_asap_and_flag", 1_000);
    let mut run = spec_run(&spec, &profile(dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(waveform.hash.clone(), bytes)])).join_is_subject();
    running(&run);
    past_t0(&mut run, ms(300));
    let begun = Instant::now();
    let manifest = run.finish();
    (manifest, device, begun.elapsed())
}

#[test]
fn ur_16_a_wedged_thread_does_not_wedge_stop() {
    let dir = TempDir::new();
    let (manifest, _, took) = wedged(&dir);
    assert!(took < Wall::from_secs(3), "{took:?}");
    assert!(section(&manifest, "rejected").as_array().unwrap().iter().any(|r| r["thread"] == "uhd-tx-s0"), "{:?}", section(&manifest, "rejected"));
}

#[test]
fn ur_16_cleanup_keeps_the_streamer_of_a_detached_thread() {
    let dir = TempDir::new();
    let (manifest, device, _) = wedged(&dir);
    assert!(calls(&device, "close_streams").is_empty());
    assert!(section(&manifest, "rejected").as_array().unwrap().iter().any(|r| r.get("leaked").is_some()));
}

// ---------------------------------------------------------------- receive (UR-17…UR-20)

fn receive_run(config: FakeConfig, channels: i64, samples: i64) -> (Manifest, TempDir) {
    let dir = TempDir::new();
    let manifest = captured(spec_run(&receive_spec(channels, 1e6, 1e9, Some(samples)), &profile(&dir, json!({}), json!({}), false), fake(config), BTreeMap::new()));
    (manifest, dir)
}

#[test]
fn ur_17_blocks_carry_the_device_timestamps() {
    let (manifest, _dir) = receive_run(FakeConfig::default(), 1, 10_000);
    let capture = capture_of(&manifest, "rec");
    let map = &capture.continuity[0];
    assert_eq!(map.first.ticks_in(map.domain).unwrap(), 0);
    assert!(map.gaps.is_empty(), "{:?}", map.gaps);
    let origin = manifest.clocks.sample_clocks[0].origin.ticks_in(manifest.clocks.sample_clocks[0].root).unwrap();
    let samples = read_capture(&capture, 1).remove(0);
    assert_eq!(samples.len(), 10_000);
    for (k, (re, _)) in samples.iter().enumerate() {
        let tick = origin + k as i64 * 200;
        assert_eq!(*re, ((tick / 200).rem_euclid(65_536)) as f32 / 65_536.0, "sample {k}");
    }
}

#[test]
fn ur_17_a_missed_start_restarts_with_a_gap() {
    let (manifest, _dir) = receive_run(FakeConfig { faults: vec![FakeFault::LateStart], ..FakeConfig::default() }, 1, 2_000);
    let late = events_of(&manifest, "radio.LATE_COMMAND");
    assert_eq!(late.len(), 1, "{late:?}");
    let payload: ezsdr_radio::payloads::LateCommandPayload = serde_json::from_value(late[0].payload.clone()).unwrap();
    let t0 = manifest.clocks.sample_clocks[0].origin.ticks_in(manifest.clocks.sample_clocks[0].root).unwrap();
    assert_eq!(payload.requested.ticks_in(payload.requested.domain()).unwrap(), t0);
    assert!(payload.applied.ticks_in(payload.applied.domain()).unwrap() - t0 >= ms(50) && payload.applied.ticks_in(payload.applied.domain()).unwrap() % 200 == 0, "{payload:?}");
    // TM-13b as KG-14 amends it: the origin stays T0, so the first sample the capture
    // holds is at the restart's index, after the first block's GAP_BEFORE.
    let map = &capture_of(&manifest, "rec").continuity[0];
    assert_eq!(map.first.ticks_in(map.domain).unwrap(), (payload.applied.ticks_in(payload.applied.domain()).unwrap() - t0) / 200);
}

#[test]
fn ur_17_blocks_reach_every_link() {
    let dir = TempDir::new();
    let mut spec = receive_spec(1, 1e6, 1e9, Some(3_000));
    let mut second = spec["outputs"][0].clone();
    second["id"] = json!("rec2");
    spec["outputs"].as_array_mut().unwrap().push(second);
    let mut doc = profile(&dir, json!({}), json!({}), false);
    doc["bindings"]["rec2"] = doc["bindings"]["rec"].clone();
    let mut placement = doc["placements"]["links"][0].clone();
    placement["to"]["component"] = json!("rec2");
    doc["placements"]["links"].as_array_mut().unwrap().push(placement);
    let mut run = spec_run(&spec, &doc, fake(FakeConfig::default()), BTreeMap::new());
    let horizon = after(&run, ms(10_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], run.events(0).iter().position(|e| e.kind == kind("sink.CAPTURE_WRITTEN")).unwrap() + 1, horizon);
    let manifest = run.finish();
    let a = read_capture(&capture_of(&manifest, "rec"), 1);
    let b = read_capture(&capture_of(&manifest, "rec2"), 1);
    assert_eq!(a, b);
}

// ---------------------------------------------------------------- transmit (UR-21…UR-23)

/// A Session with one transmit channel enabled and T0 passed.
/// Runs `f` on its own thread and returns its value, failing the test if it does not finish
/// within `bound`: a mutant that freezes or deadlocks the Provider must fail the test, not
/// hang the suite (plan 30 §7's guard).
fn bounded<T: Send + 'static>(what: &str, bound: Wall, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::spawn(move || {
        // A failure inside `f` fails the test at once; only a frozen or deadlocked Run pays
        // the bound.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|_| ());
        let _ = tx.send(result);
    });
    match rx.recv_timeout(bound) {
        Ok(Ok(value)) => value,
        Ok(Err(())) => panic!("{what} failed"),
        Err(_) => {
            infra(&format!("{what} did not finish within {bound:?}"));
            panic!("{what} did not finish within {bound:?}")
        }
    }
}

fn tx_session(config: FakeConfig) -> (Run, Arc<FakeDevice>, TempDir) {
    let dir = TempDir::new();
    let device = fake(config);
    let profile = profile(&dir, json!({}), json!({}), true);
    let mut run = session_with(&profile, device.clone(), &rx0_tx0(&profile));
    past_t0(&mut run, ms(1));
    past_tx_origin(&mut run);
    (run, device, dir)
}

/// The same transmit Session on any `Device` (a test wrapper, plan 30 §7).
fn tx_session_on(device: Arc<dyn Device>) -> (Run, TempDir) {
    let dir = TempDir::new();
    let profile = profile(&dir, json!({}), json!({}), true);
    let mut run = session_with(&profile, device, &rx0_tx0(&profile));
    past_t0(&mut run, ms(1));
    past_tx_origin(&mut run);
    (run, dir)
}

/// A Spec Run advanced to `ticks` after T0, then finished. The Run's clock is the device's,
/// which may already be past a `now()`-relative horizon before T0 (plan 30 §4.4).
fn finish_spec(mut run: Run, ticks: i64) -> Manifest {
    let t0 = run.start_instant().expect("a Spec Run starts");
    let _ = run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ticks));
    run.finish()
}

/// The transmit sample `lead` root ticks from now, on the running transmit clock.
fn tx_at(run: &Run, lead: i64) -> TimePoint {
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx/s0").unwrap() && r.ended_at.is_none()).unwrap();
    let n = clock.root_ticks_per_tick.num() as i64;
    TimePoint::new(clock.domain, (run.now().ticks_in(clock.root).unwrap() + lead - clock.origin.ticks_in(clock.root).unwrap() + n - 1).div_euclid(n))
}

fn send_at(run: &mut Run, verb_name: &str, at: Option<TimePoint>, samples: &[(f32, f32)]) -> ezsdr_kernel::session::LogEntry {
    let (bytes, _) = waveform_of(samples);
    run.submit(verb(verb_name, resource("radio/tx/s0"), at, &[]), Some(&bytes)).unwrap()
}

fn send(run: &mut Run, verb_name: &str, lead: Option<i64>, samples: &[(f32, f32)]) -> ezsdr_kernel::session::LogEntry {
    let at = lead.map(|lead| tx_at(run, lead));
    send_at(run, verb_name, at, samples)
}

/// A second Spec input under its own name, for a Spec whose schedule names two waveforms
/// (KC-9).
fn second_waveform(samples: &[(f32, f32)]) -> (Vec<u8>, ArtifactRef) {
    let (bytes, _) = waveform_of(samples);
    let reference = ezsdr_kernel::manifest::ingest_input(Ident::parse("waveform2").unwrap(), Namespace::parse("ezsdr.input").unwrap(), format!("mem:{}", ContentHash::of_bytes(&bytes)), &bytes);
    (bytes, reference)
}

fn time_errors(manifest: &Manifest) -> Vec<TimeErrorPayload> {
    events_of(manifest, "radio.TIME_ERROR").iter().map(|e| serde_json::from_value(e.payload.clone()).unwrap()).collect()
}

fn rejections(manifest: &Manifest) -> Vec<String> {
    section(manifest, "rejected").as_array().unwrap().iter().filter_map(|r| r["reason"].as_str().map(str::to_owned)).collect()
}

/// The reason of the first `COMMAND_REJECTED` in `events` (RM-22).
fn rejection_reason(events: &[Event]) -> String {
    events.first().and_then(|event| event.payload["reason"].as_str()).unwrap_or_else(|| panic!("no rejection: {events:?}")).to_owned()
}

#[test]
fn ur_25_a_first_burst_with_the_connect_values_makes_no_apply() {
    // M1 (P5): arm publishes generation 0, the connect values' generation, so a burst with no new value applies nothing.
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    let applies = |device: &FakeDevice| device.calls().iter().filter(|call| call.starts_with("apply tx")).count();
    let before = applies(&device);
    let _ = send(&mut run, "send", Some(ms(20)), &tone(10));
    wait(&mut run, ms(40));
    let _ = run.finish();
    assert!(device.calls().iter().any(|call| call.starts_with("tx_send ")), "the burst is sent: {:?}", device.calls());
    assert_eq!(applies(&device), before, "{:?}", device.calls());
}

#[test]
fn ur_21_a_refused_configure_refuses_its_burst_through_the_kernel() {
    // S7 through a Session (R5, M3, RS-16): a refused configure refuses its burst with the burst's Action id, and nothing is sent; a corrected value after the burst's booked end is taken
    // and the next burst is configured with it on the same clock and sent. The burst is untimed and each wait is far past any placement and length, so no delay of the test thread can
    // move what is asserted (note 51 §3); the window of the refused burst's span is `ur_21_a_refused_configure_keeps_its_burst_s_span_until_its_booked_end_on_the_direct_rig`.
    const REFUSED: f64 = 2.0e9;
    const CORRECTED: f64 = 3.0e9;
    let (mut run, device, _dir) = tx_session(FakeConfig { faults: vec![FakeFault::TxApplyRefused(0, REFUSED)], ..FakeConfig::default() });
    let tx_clocks = |run: &Run| run.sample_clocks().into_iter().filter(|record| record.stream.path() == "usrp/tx/s0").map(|record| record.domain).collect::<Vec<_>>();
    let clocks = tx_clocks(&run);
    let action_of = |entry: &ezsdr_kernel::session::LogEntry| match &entry.outcome {
        Outcome::Admitted { dispatched, .. } => dispatched[0].0,
        other => panic!("not admitted: {other:?}"),
    };
    let value = |run: &mut Run, frequency: f64| run.submit(set("radio.tx.c0.frequency_hz", Value::num(frequency).unwrap()), None).unwrap();
    assert!(admitted(&value(&mut run, REFUSED)));
    let first = send(&mut run, "send", None, &tone(10_000));
    let first_id = action_of(&first);
    wait(&mut run, ms(60));
    assert!(admitted(&value(&mut run, CORRECTED)));
    let sent_before = tx_experiment_sends(&device, "tx_send").len();
    let next = send(&mut run, "send", None, &tone(2_000));
    assert!(admitted(&next));
    wait(&mut run, ms(60));
    let clocks_after = tx_clocks(&run);
    let manifest = run.finish();
    let rejected: Vec<_> = events_of(&manifest, "radio.COMMAND_REJECTED").iter().map(|event| (event.payload["kind"].clone(), event.payload["action"].clone())).collect();
    assert_eq!(rejected, [(json!("tx_burst"), json!(first_id))], "{rejected:?} {:?}", device.calls());
    assert_eq!(sent_before, 0, "the refused burst is not sent: {:?}", device.calls());
    let (configured, sent) = (
        device.calls().iter().position(|call| call.starts_with("apply tx 0 rate=- freq=3000000000")),
        device.calls().iter().position(|call| call.starts_with("tx_send ") && !call.contains(" at=- sob=true eob=true")),
    );
    assert!(matches!((configured, sent), (Some(configured), Some(sent)) if sent > configured), "the next burst is configured with the corrected value, then sent: {:?}", device.calls());
    assert_eq!(clocks_after, clocks, "the next burst is on the same clock");
}

#[test]
fn ur_21_a_refused_configure_keeps_its_burst_s_span_until_its_booked_end_on_the_direct_rig() {
    // S7's window (UR-21): a refused burst's span stays counted until its booked end. A corrected value before it is refused, one at the end is taken, and the next burst is configured with it
    // and sent on the same clock. On the Direct rig the device clock stands while the test thread works, so no delay of it moves a value off its side of the end.
    const REFUSED: f64 = 2.0e9;
    const CORRECTED: f64 = 3.0e9;
    let (bytes, waveform) = waveform_of(&tone(10_000));
    let (bytes_next, waveform_next) = waveform_of(&tone(2_000));
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes), (waveform_next.hash.clone(), bytes_next)]);
    let mut direct = Direct::build_with(
        FakeConfig { faults: vec![FakeFault::TxApplyRefused(0, REFUSED)], ..FakeConfig::default() },
        &[("radio.tx.c0.stream", Scalar::from(0))],
        Vec::new(),
        json!({ "rx": [], "tx": ["0:A:ubx:0"] }),
        inputs,
    );
    direct.settle(Wall::from_millis(50));
    let record = clock_of(&direct, "usrp/tx/s0");
    let cold = ezsdr_kernel::module_api::UpdateClass::Cold;
    direct.update("radio.tx.c0.frequency_hz", Value::num(REFUSED).unwrap(), cold, None);
    // The burst is 10 ms long and starts 10 ms ahead: the window of its booking is 10 ms to 20 ms after the push.
    burst_in(&direct, "usrp/tx/s0", &record, &waveform, false, ms(10));
    direct.settle(Wall::from_millis(12));
    direct.update("radio.tx.c0.frequency_hz", Value::num(CORRECTED).unwrap(), cold, None);
    direct.settle(Wall::from_micros(1_500));
    direct.settle(Wall::from_millis(9));
    direct.update("radio.tx.c0.frequency_hz", Value::num(CORRECTED).unwrap(), cold, None);
    direct.settle(Wall::from_millis(1));
    let sent_before = tx_experiment_sends(&direct.device, "tx_send").len();
    burst_in(&direct, "usrp/tx/s0", &record, &waveform_next, false, ms(10));
    direct.settle(Wall::from_millis(30));
    let rejected: Vec<_> = direct.of("radio.COMMAND_REJECTED").iter().map(|event| (event.payload["kind"].clone(), event.payload["action"].clone())).collect();
    let calls = direct.device.calls();
    let clocks_after = direct.clocks.sample_clock_records().into_iter().filter(|record| record.stream.path() == "usrp/tx/s0").map(|record| record.domain).collect::<Vec<_>>();
    direct.finish();
    // The queue numbers Actions from 1: the REFUSED value is 1, the first burst 2, the early value 3.
    assert_eq!(rejected, [(json!("tx_burst"), json!(2)), (json!("update_parameter"), json!(3))], "{rejected:?} {calls:?}");
    assert_eq!(sent_before, 0, "the refused burst is not sent: {calls:?}");
    let (configured, sent) =
        (calls.iter().position(|call| call.starts_with("apply tx 0 rate=- freq=3000000000")), calls.iter().position(|call| call.starts_with("tx_send ") && !call.contains(" at=- sob=true eob=true")));
    assert!(matches!((configured, sent), (Some(configured), Some(sent)) if sent > configured), "the next burst is configured with the corrected value, then sent: {calls:?}");
    assert_eq!(clocks_after, [record.domain], "the next burst is on the same clock");
}

#[test]
fn ur_21_txburst_refusals() {
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    // Not a multiple of 8 · channels.
    let odd = vec![0u8; 12];
    let _ = run.submit(verb("send", resource("radio/tx/s0"), None, &[]), Some(&odd)).unwrap();
    // A non-finite component.
    let _ = send(&mut run, "send", Some(ms(20)), &[(f32::NAN, 0.0)]);
    wait(&mut run, ms(20));
    let manifest = run.finish();
    let reasons = rejections(&manifest);
    for want in ["UR-21: the waveform size", "UR-21: a waveform sample is not finite"] {
        assert!(reasons.iter().any(|r| r.starts_with(want)), "{want}: {reasons:?}");
    }
    let rejected = events_of(&manifest, "radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 2, "{rejected:?}");
    // MA-14: a burst refused at receipt names the Action it answers.
    assert!(rejected.iter().all(|e| e.payload["action"].is_u64()), "{rejected:?}");

    // A start equal to a held burst's: two bursts at one fixed offset, both booked before
    // the Run starts, so that the test thread's own delay cannot move the second (rules
    // (1) and (2); the receipt-time booking is what decides).
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let (bytes, waveform) = waveform_of(&tone(10));
    let hash = waveform.hash.clone();
    let target = serde_json::to_value(resource("radio/tx/s0")).unwrap();
    let mut spec = with_tx(receive_spec(1, 1e6, 1e9, None), 1e6);
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 10_000 }, "action": { "kind": "tx_burst", "target": target.clone(),
            "waveform": waveform.clone(), "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 10_000 }, "action": { "kind": "tx_burst", "target": target,
            "waveform": waveform, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } }
    ]);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device, BTreeMap::from([(hash, bytes)]));
    let t0 = run.start_instant().expect("a Spec Run starts");
    let _ = run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ms(20)));
    let manifest = run.finish();
    let reasons = rejections(&manifest);
    assert!(reasons.iter().any(|r| r.starts_with("UR-21: the burst overlaps an accepted booking")), "{reasons:?}");
    let rejected = events_of(&manifest, "radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert!(rejected.iter().all(|e| e.payload["action"].is_u64()), "{rejected:?}");
}

#[test]
fn ur_21_late_policies_on_the_device_lead() {
    // The manual clock (plan 30 §4.4): the booking's verdicts are deterministic device-time
    // facts — the 1 ms `send` (`drop_and_flag`) is dropped and the 0.5 ms `start_repeat`
    // (`send_asap_and_flag`) is moved, both at the receipt site (`control.rs`'s `book_burst`
    // applies the 2 ms device lead). Whether the moved repeat is moved *again* at uhd-tx's
    // first hand-over depends on real time passing between the booking and the hand-over
    // (pinned on the controlled clock in `src/provider/tx.rs`), which the manual clock does
    // not promise — on a busy Provider it is frozen away, so the verdict can be absent. That
    // re-decision is not asserted here.
    let (mut run, _device, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "send", Some(ms(1)), &tone(10));
    let second_at = tx_at(&run, ms(1) / 2);
    let entry = send_at(&mut run, "start_repeat", Some(second_at), &tone(10));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(50));
    let _ = run.submit(SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None);
    wait(&mut run, ms(20));
    // A reservation whose delay cannot change the verdict is `send_asap_and_flag`: a delayed
    // booking moves it and says so, instead of dropping it (rule (3)). Its on-time start is
    // the case above; the device lead's own boundary is the bench's to measure (B8).
    let third_at = tx_at(&run, ms(20));
    let entry = send_at(&mut run, "start_repeat", Some(third_at), &tone(10));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(60));
    let manifest = run.finish();
    let errors = time_errors(&manifest);
    let outcomes: Vec<_> = errors.iter().map(|p| p.outcome).collect();
    // The booking's own verdicts: Drop for the 1 ms burst, SendAsap for the moved repeat.
    assert_eq!(&outcomes[..2], [TimeErrorOutcome::Drop, TimeErrorOutcome::SendAsap], "{errors:#?}");
    // Only moved repeats may follow, each naming its own request: the second is moved again
    // at uhd-tx's hand-over, and a delayed booking of the third moves it (rule (3)); neither
    // is a `Drop`.
    assert!(errors[2..].iter().all(|p| p.outcome == TimeErrorOutcome::SendAsap && (p.target == second_at || p.target == third_at)), "{errors:#?}");
    // The third was booked and played: the burst is on the device with the request kept when
    // the booking moved it, at or after the request.
    let records = bursts(&manifest);
    assert_eq!(records.len(), 2, "{records:?}");
    let third = &records[1];
    assert!(third.target.ticks_in(third.target.domain()).unwrap() >= third_at.ticks_in(third_at.domain()).unwrap(), "{third:?} requested {third_at:?}");
    assert!(third.requested_target.is_none() || third.requested_target == Some(third_at), "{third:?} requested {third_at:?}");
}

// `ur_21_an_untimed_send_is_placed_after_its_delivery` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37): the Kernel stamps an untimed Action on the
// test thread, so a fake test's own delay spends its slack. What the fake keeps is what no thread's timing can change.
#[test]
fn ur_21_untimed_sends_are_each_played() {
    // KG-8 end to end, on the manual clock: 20 untimed sends, each admitted, are each played. A send the test thread's delay leaves
    // late is placed after its delivery (RM-15): no send is dropped, and none carries a TIME_ERROR.
    let (mut run, _, _dir) = tx_session(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() });
    for _ in 0..20 {
        let entry = send(&mut run, "send", None, &tone(100));
        assert!(admitted(&entry), "{entry:?}");
        wait(&mut run, ms(5));
    }
    wait(&mut run, ms(30));
    let manifest = run.finish();
    assert_eq!(bursts(&manifest).len(), 20, "{:?}", time_errors(&manifest));
    assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
}

/// RM-15, RM-16 (sequence 2, UHD): an untimed repeat received while a repeat plays is refused as an overlap, with no `TIME_ERROR`;
/// the first repeat keeps playing.
#[test]
fn ur_21_s18_an_untimed_repeat_on_a_playing_repeat_is_refused_without_time_error() {
    let (mut run, device, _dir) = tx_session(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() });
    assert!(admitted(&send(&mut run, "start_repeat", None, &tone(1_000))));
    assert!(admitted(&send(&mut run, "start_repeat", None, &tone(500))));
    wait(&mut run, ms(40));
    let manifest = run.finish();
    let rejected = rejections(&manifest);
    assert!(rejected.iter().any(|r| r.starts_with("UR-21: the burst overlaps an accepted booking")), "{rejected:?}");
    assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
    let sends = tx_experiment_sends(&device, "tx_send");
    assert_eq!(sends.iter().filter(|c| c.contains("sob=true")).count(), 1, "{sends:?}");
}

/// RM-37, RM-15 (sequence 7, UHD): an untimed switch takes the first boundary of the replaced repeat's grid at or after its receipt's
/// window and device lead (`repeat_boundary`, the record's function), as a timed switch does (S8).
#[test]
fn ur_21_s19_untimed_switch_takes_the_first_grid_boundary_after_its_receipt_window() {
    let (mut run, _device, _dir) = tx_session(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() });
    // A 1 024-sample period: the receipt's bound lands about 400 samples into the period. On a 1 000-sample grid it lands
    // about 40 samples short of a boundary, and load can carry it onto one, where the mutant gives the same boundary.
    assert!(admitted(&send(&mut run, "start_repeat", Some(ms(10)), &tone(1_024))));
    wait(&mut run, ms(20));
    let (bytes, _) = waveform_of(&tone(500));
    let mark = [("radio.repeat_operation", Value::Scalar(Scalar::from("switch")))];
    let entry = run.submit(verb("start_repeat", resource("radio/tx/s0"), None, &mark), Some(&bytes)).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(40));
    let manifest = run.finish();
    assert!(rejections(&manifest).is_empty(), "{:?}", rejections(&manifest));
    let rows = section(&manifest, "switches").as_array().unwrap().clone();
    assert_eq!(rows.len(), 1, "{rows:?}");
    // 200 root ticks per transmit sample at 1 MS/s, as `ur_23_a_stop_ends_the_burst_within_the_in_flight_window`. The window is 10 ms
    // and the lead 2 ms (`DEVICE_LEAD_NS`): the bound is the receipt's sample plus 12 000 samples, rounded up to a sample.
    let origin_ticks = tx_clock_origin(&manifest, 0);
    let grid = serde_json::to_value(bursts(&manifest)[0].target).unwrap()["ticks"].as_i64().unwrap();
    let receipt = rows[0]["selected_at"]["ticks"].as_i64().unwrap() - origin_ticks;
    let bound = (receipt + 12_000 * 200 + 199).div_euclid(200);
    assert_eq!(rows[0]["boundary"].as_i64().unwrap(), ezsdr_radio::staging::repeat_boundary(grid, 1_024, bound).unwrap());
    // #86: an untimed switch has no requested instant (RS-19), so its new repeat's record carries no requested target.
    assert_eq!(bursts(&manifest)[1].requested_target, None, "{:?}", bursts(&manifest));
}

#[test]
fn ur_22_repeat_is_continuous_across_the_wrap() {
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let wave = tone(1_000);
    let (bytes, waveform) = waveform_of(&wave);
    let spec = with_burst(with_tx(receive_spec(1, 1e6, 1e9, Some(4_000)), 1e6), &waveform, true, "send_asap_and_flag", 1_000);
    let manifest = captured({
        let mut spec = spec;
        spec["outputs"][0]["params"] = json!({});
        spec["schedule"].as_array_mut().unwrap().push(json!({
            "at": { "clock": "radio", "offset_ticks": 1_000 },
            "action": { "kind": "update_parameter", "target": serde_json::to_value(output("rec")).unwrap(),
                        "key": "sink.capture_samples", "value": 4_000 }
        }));
        spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(waveform.hash.clone(), bytes)]))
    });
    let samples = read_capture(&capture_of(&manifest, "rec"), 1).remove(0);
    for (i, (re, im)) in samples.iter().enumerate() {
        assert_eq!((*re, *im), wave[i % 1_000], "sample {i}");
    }
    assert_eq!(device.restarts(), 0);
    let record = &bursts(&manifest)[0];
    assert!(record.wraps >= 3, "{record:?}");
    assert!(events_of(&manifest, "radio.TX_UNDERFLOW").is_empty());
    let sends = tx_experiment_sends(&device, "tx_send");
    assert!(!sends.is_empty(), "the repeat reached the device: {:?}", device.calls());
    assert!(sends.iter().all(|call| !call.contains(" at=- ")), "every payload send is timed: {sends:?}");
}

#[test]
fn ur_22_a_single_burst_ends_with_end_of_burst() {
    // Booked in the Spec, before the Run starts (rule (1)): a burst at 20 ms.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let (bytes, waveform) = waveform_of(&tone(3_000));
    let spec = with_burst(with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &waveform, false, "drop_and_flag", 20_000);
    let manifest = finish_spec(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(waveform.hash.clone(), bytes)])), ms(60));
    require_stream_timing(&manifest, "single burst", true);
    let sends = tx_experiment_sends(&device, "tx_send");
    assert!(sends.last().is_some_and(|s| s.ends_with("eob=true")), "{sends:?}");
    let records = bursts(&manifest);
    assert_eq!(records.len(), 1, "records={records:?}; sends={sends:?}; rejected={}", section(&manifest, "rejected"));
    assert_eq!(records[0].end, BurstEnd::Eob);
    assert_eq!(records[0].samples, 3_000);
}

#[test]
fn timing_precondition_failure_reports_evidence_and_still_fails() {
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    // At most 1 ms ahead of receipt, below the real 2 ms device lead.
    let _ = send(&mut run, "send", Some(ms(1)), &tone(10));
    let manifest = run.finish();
    assert!(time_errors(&manifest).iter().any(|p| p.outcome == TimeErrorOutcome::Drop));
    let failure = std::panic::catch_unwind(|| require_stream_timing(&manifest, "diagnostic regression", true)).expect_err("a missed prerequisite must fail, not skip or pass the trial");
    let message = failure.downcast_ref::<String>().unwrap();
    assert!(message.contains("TIMING_PRECONDITION_UNMET (diagnostic regression)"));
    assert!(message.contains("radio.TIME_ERROR") && message.contains("drop"));
}

#[test]
fn ur_23_a_repeat_holds_its_span_until_a_stop() {
    // All three are booked in the Spec, before the Run starts (rule (1)): a repeat at 10 ms, a
    // burst at 20 ms inside the repeat's open span (refused at receipt, S5), and a Stop at 40 ms.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let (one, a) = waveform_of(&tone(1_000));
    let (two, b) = second_waveform(&tone(500));
    let (ha, hb) = (a.hash.clone(), b.hash.clone());
    let target = serde_json::to_value(resource("radio/tx/s0")).unwrap();
    let mut spec = with_tx(receive_spec(1, 1e6, 1e9, None), 1e6);
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 10_000 }, "action": { "kind": "tx_burst", "target": target.clone(),
            "waveform": a, "repeat": true, "late_policy": "send_asap_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 20_000 }, "action": { "kind": "tx_burst", "target": target.clone(),
            "waveform": b, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 40_000 }, "action": { "kind": "stop", "target": target } }
    ]);
    let manifest = finish_spec(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(ha, one), (hb, two)])), ms(60));
    let rejected = rejections(&manifest);
    assert!(rejected.iter().any(|r| r.starts_with("UR-21: the burst overlaps an accepted booking")), "{rejected:?}");
    let records = bursts(&manifest);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].end, BurstEnd::Stop, "{records:?}");
    assert!(time_errors(&manifest).is_empty());
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError"), "{:?}", device.calls());
    // One device burst, one start-of-burst, and no timed start inside the open burst.
    let sends = tx_experiment_sends(&device, "tx_send");
    assert_eq!(sends.iter().filter(|c| c.contains("sob=true")).count(), 1, "{sends:?}");
    assert!(sends.iter().all(|call| !call.contains(" at=- ")), "every payload send is timed: {sends:?}");
    assert_eq!(device.unended_bursts(), 0, "a burst started inside one without end-of-burst: {:?}", device.calls());
}

#[test]
fn ur_23_a_stop_ends_the_burst_within_the_in_flight_window() {
    // The bound itself — the record's end no later than the stop plus the in-flight window —
    // is pinned on the controlled clock in `src/provider/tx.rs`: here a delayed test thread
    // moves the receipt, and the record's end with it, so no upper bound read from this side
    // stays true (rule (4)). What this side still asserts does not depend on the gap.
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "start_repeat", Some(ms(10)), &tone(1_000));
    wait(&mut run, ms(60));
    // The Stop's own receipt instant, as the Run logs it (RS-15), not a time read before the
    // submission: a delayed test thread moves the receipt and the record's end together.
    let stopped = run.submit(SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None).unwrap();
    let stop_at = stopped.time.ticks_in(stopped.time.domain()).unwrap();
    wait(&mut run, ms(40));
    let manifest = run.finish();
    let record = &bursts(&manifest)[0];
    assert_eq!(record.end, BurstEnd::Stop);
    let origin = tx_clock_origin(&manifest, 0);
    let end = origin + (record.target.ticks_in(record.target.domain()).unwrap() + record.samples as i64) * 200;
    // The stop recalls nothing already handed over (RM-16): its ledger instant is a lower
    // bound whichever way the test thread was delayed.
    assert!(end >= stop_at, "the record ends {} ms before the stop", (stop_at - end) / ms(1));
    // The device burst is closed with an end-of-burst after the stop, and nothing runs dry.
    let sends = tx_experiment_sends(&device, "tx_send");
    assert_eq!(sends.iter().filter(|call| call.ends_with("eob=true")).count(), 1, "{sends:?}");
    assert_eq!(device.unended_bursts(), 0, "a burst started inside one without end-of-burst: {:?}", device.calls());
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "Underflow"), "{:?}", device.calls());
}

// ---------------------------------------------------------------- updates (UR-24…UR-26)

// `ur_24_the_device_queue_order_is_the_effective_order` is a unit test in
// `src/provider/control.rs`, on the controlled clock: its case, a command earlier than one
// already released, was left to host timing here, which the manual clock no longer provides.

#[test]
fn ur_25_tx_channels_from_zero_transmits_the_next_burst() {
    for _ in 0..5 {
        let (mut run, device, _dir) = tx_session(FakeConfig::default());
        let entry = send(&mut run, "start_repeat", None, &tone(500));
        assert!(admitted(&entry), "{entry:?}");
        wait(&mut run, ms(40));
        let _ = run.finish();
        let calls = device.calls();
        let configured = calls.iter().position(|c| c.starts_with("apply tx 0 rate=1000000")).unwrap();
        let sent = calls.iter().position(|c| c.starts_with("tx_send") && c.contains("sob=true") && !c.contains("at=- sob=true eob=true")).unwrap();
        assert!(configured < sent);
    }
}

#[test]
fn ur_23_a_burst_after_a_stop_targets_the_one_clock() {
    // UR-23, RM-25 (S11): a burst booked after a Stop is on the transmit stream's clock registered at arm; the Stop registers none.
    let (mut run, _device, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "start_repeat", None, &tone(500));
    wait(&mut run, ms(20));
    let entry = run.submit(SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(5));
    // `send_asap_and_flag`: a burst the test thread's own delay leaves late is moved with a TIME_ERROR, not dropped as a refusal the assertion below would read.
    let entry = send(&mut run, "start_repeat", Some(ms(10)), &tone(100));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(40));
    let manifest = run.finish();
    let clocks = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/tx/s0").unwrap()).count();
    assert_eq!(clocks, 1, "{:?}", manifest.clocks.sample_clocks);
    assert!(rejections(&manifest).is_empty(), "{:?}", rejections(&manifest));
}

#[test]
fn ur_26_stop_actions() {
    // UR-26: a `Stop` of the transmit stream ends its burst, of the receive stream cuts it at
    // its booking, of the device does both, and of another target is refused.
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "start_repeat", Some(ms(10)), &tone(100));
    wait(&mut run, ms(30));
    for target in ["radio/tx/s0", "radio/rx/s0", "radio", "radio/gpio"] {
        let entry = run.submit(SessionAction::Stop { target: Some(resource(target)) }, None).unwrap();
        assert!(admitted(&entry), "{target}: {entry:?}");
        wait(&mut run, ms(5));
    }
    // A Command this Provider does not serve (`start_rx` of the transmit stream) is refused
    // under the kind `command`.
    let _ = run.submit(verb("start_rx", resource("radio/tx/s0"), None, &[]), None).unwrap();
    wait(&mut run, ms(20));
    let manifest = run.finish();
    assert!(section(&manifest, "rejected").as_array().unwrap().iter().any(|r| r["kind"] == "command"));
    assert_eq!(bursts(&manifest)[0].end, BurstEnd::Stop);
    assert!(section(&manifest, "applied").as_array().unwrap().iter().any(|r| r["key"] == "rx_stop"));
    assert!(rejections(&manifest).iter().any(|r| r.starts_with("UR-26: the Stop target is not this device")), "{:?}", rejections(&manifest));
}

#[test]
fn ur_26_an_unnumbered_key_is_refused_at_runtime() {
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::with_links(FakeConfig::default(), &[], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    let mark = direct.device.calls().len();
    direct.update("radio.rx.antenna", Value::from("TX/RX"), Cold, None);
    direct.until("the unknown-key refusal", |direct| !direct.of("radio.COMMAND_REJECTED").is_empty());
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejection_reason(&rejected), "UR-26: RM-21: only a channel's frequency, gain or antenna is staged");
    assert!(!direct.device.calls()[mark..].iter().any(|call| call.starts_with("apply")), "{:?}", direct.device.calls());
    let _ = direct.finish();
}

#[test]
fn ur_26_an_abort_publishes_nothing_after_the_stop_instant() {
    // RM-16: under abort the receive stream ends at the first sample not yet delivered
    // when uhd-rx learns of the stop (Review L, L12). The link keeps the newest blocks
    // (drop-oldest). Blocks of 20 ms, so that uhd-rx is almost surely inside a receive wait
    // when the stop begins, and the block it then receives, straddling the stop instant,
    // is discarded whole.
    let port = attached(ezsdr_kernel::stream::BackPressure::DropOldest);
    let Endpoint::StreamOut(link) = &port.endpoint else { unreachable!() };
    let link = link.clone();
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![port], json!({ "block_len": 20_000 }));
    direct.settle(Wall::from_millis(50));
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Abort).unwrap();
    let mut end = None;
    while let Some((block, _)) = link.receive() {
        let header = block.header();
        end = Some(header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() + i64::from(header.len));
    }
    direct.radio.cleanup();
    let instance = direct.radio.instance().clone();
    let timing = &section_of(&instance, "timing");
    let stop = timing.as_array().unwrap().iter().find(|r| r["what"] == "stop").unwrap()["at"].as_i64().unwrap();
    let origin = direct
        .clocks
        .sample_clock_records()
        .iter()
        .find(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap())
        .unwrap()
        .origin
        .ticks_in(direct.clocks.sample_clock_records().iter().find(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap()).unwrap().root)
        .unwrap();
    let end = origin + end.expect("blocks were published") * 200;
    assert!(end < stop, "published up to {end}, stopped at {stop}");
    // One stop, recorded once: not again when the tail reaches the cut (Review N, N4).
    assert_eq!(calls(&direct.device, "rx_stop").len(), 1, "{:?}", direct.device.calls());
    let applied = &section_of(&instance, "applied");
    assert_eq!(applied.as_array().unwrap().iter().filter(|r| r["key"] == "rx_stop").count(), 1, "{applied}");
}

#[test]
fn ur_25_a_receive_session_follows_its_recorded_plan() {
    // Spec 22, VH-8: the receive path end to end — uhd-control's booking, uhd-rx's thread, the
    // streamer and the device's commands — carries out the plan `Provider::stop` records: after two
    // `Stop`/`start_rx` pairs and the Run's stop, the plan has three segments,
    // in order and apart, and every SampleClock is one of them, in order, ended at its cut — or,
    // when uhd-rx had delivered past a `Stop`'s cut before it was booked, at the first sample it
    // had not (RM-16's floor) —, the end its stop row records; the stream is stopped untimed;
    // one streamer. Under load a segment may get no clock: its start missed (UR-17's restart)
    // with no sample before its cut, which is allowed, so the test holds whatever the load.
    // The rules that place the segments are the timeline's (`rm_26_the_timeline_cases`, layers
    // 1–3).
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device.clone());
    past_t0(&mut run, ms(1));
    wait(&mut run, ms(100));
    for _ in 0..2 {
        assert!(admitted(&run.submit(SessionAction::Stop { target: Some(resource("radio/rx/s0")) }, None).unwrap()));
        wait(&mut run, ms(20));
        assert!(admitted(&run.submit(verb("start_rx", resource("radio/rx/s0"), None, &[]), None).unwrap()));
        wait(&mut run, ms(100));
    }
    let manifest = run.finish();
    let timing = section(&manifest, "timing").clone();
    let plan = plan_row(&timing);
    let segments: Vec<(i64, Option<i64>)> = plan["rx"].as_array().unwrap().iter().map(|s| (s["origin"].as_i64().unwrap(), s["end"].as_i64())).collect();
    assert_eq!(segments.len(), 3, "{plan}");
    for pair in segments.windows(2) {
        assert!(pair[0].1.is_some_and(|end| end <= pair[1].0), "segments out of order or overlapping: {plan}");
    }
    assert!(segments[2].1.is_some(), "the Run's stop ends the last: {plan}");
    let clocks: Vec<(i64, Option<i64>)> = manifest
        .clocks
        .sample_clocks
        .iter()
        .filter(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap())
        .map(|r| (r.origin.ticks_in(r.root).unwrap(), r.ended_at.map(|t| t.ticks_in(r.root).unwrap())))
        .collect();
    let stops: Vec<i64> = section(&manifest, "applied").as_array().unwrap().iter().filter(|r| r["key"] == "rx_stop").filter_map(|r| r["at"]["ticks"].as_i64()).collect();
    assert!(!clocks.is_empty(), "{timing:?}");
    let mut next = 0;
    for (origin, end) in &clocks {
        let index =
            segments[next..].iter().position(|(o, _)| o == origin).map(|i| next + i).unwrap_or_else(|| panic!("a clock at {origin} that is no segment of the plan, or out of order: {timing:?}"));
        next = index + 1;
        let (planned, until) = (segments[index].1.unwrap(), segments.get(next).map_or(i64::MAX, |(o, _)| *o));
        let end = end.unwrap_or_else(|| panic!("a clock not ended: {timing:?}"));
        assert!(end >= planned && end <= until, "a clock ends at {end}, its segment's cut {planned}: {timing:?}");
        assert!(stops.contains(&end), "no stop row at the clock's end {end}: {timing:?}");
    }
    let calls = device.calls();
    assert_eq!(calls.iter().filter(|c| c.starts_with("rx_open")).count(), 1, "one streamer: {calls:?}");
    assert!(calls.iter().filter(|c| c.starts_with("rx_stop")).all(|c| c == "rx_stop 0 now"), "a timed receive stop: {calls:?}");
    assert!(rejections(&manifest).is_empty(), "{:?}", rejections(&manifest));
}

#[test]
fn ur_26_cleanup_is_idempotent() {
    let mut radio = provider();
    radio.cleanup();
    radio.cleanup();
}

// ---------------------------------------------------------------- device reports (UR-27…UR-29)

#[test]
fn ur_27_a_lost_reference_is_clock_lost() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::Unlocked(Wall::from_millis(2_300))], ..FakeConfig::default() });
    let mut run =
        spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({ "clock_source": "external" }), json!({}), false), device, BTreeMap::new()).expecting(Expect::Policy("radio.CLOCK_LOST"));
    let result = run.run_until_end(after(&run, ms(5_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind("radio.CLOCK_LOST") } });
    assert_eq!(events_of(&manifest, "radio.CLOCK_LOST")[0].payload, json!({ "reference": "frequency" }));
    assert_eq!(events_of(&manifest, "radio.CLOCK_LOST")[0].source, usrp(), "RM-11's source is the device node");
}

#[test]
fn ur_28_underflows_are_tx_underflow() {
    let config =
        FakeConfig { faults: vec![FakeFault::TxReport(Wall::from_millis(2_100), TxCode::Underflow), FakeFault::TxReport(Wall::from_millis(2_100), TxCode::SeqError)], ..FakeConfig::default() };
    let (mut run, _, _dir) = tx_session(config);
    wait(&mut run, ms(200));
    let manifest = run.finish();
    let causes: Vec<_> = events_of(&manifest, "radio.TX_UNDERFLOW").iter().map(|e| e.payload["cause"].clone()).collect();
    assert_eq!(causes, [json!("starved"), json!("lost")]);
    let codes: Vec<_> = section(&manifest, "async").as_array().unwrap().iter().map(|r| r["code"].clone()).collect();
    assert!(codes.contains(&json!("Underflow")) && codes.contains(&json!("SeqError")), "{codes:?}");
}

#[test]
fn ur_28_a_failed_report_read_is_not_an_empty_one() {
    // A failing read is counted with its first text, never taken for "no report" (UR-3).
    let (mut run, _, _dir) = tx_session(FakeConfig { faults: vec![FakeFault::TxAsyncFails(Wall::from_millis(2_100), false)], ..FakeConfig::default() });
    wait(&mut run, ms(200));
    let manifest = run.finish();
    assert!(section(&manifest, "stats")["tx_errors"].as_i64().unwrap() > 1, "the read failed repeatedly");
    let notes: Vec<_> = section(&manifest, "rejected").as_array().unwrap().iter().filter(|r| r["call"] == "tx_async").cloned().collect();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0]["error"].as_str().unwrap().contains("recv_async_msg"), "{notes:?}");
    assert!(events_of(&manifest, EventKind::DEVICE_LOST).is_empty());

    // One that reports the device gone is UR-29's loss, though every other call succeeds.
    let (run, _, _dir) = tx_session(FakeConfig { faults: vec![FakeFault::TxAsyncFails(Wall::from_millis(2_100), true)], ..FakeConfig::default() });
    let mut run = run.expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
    let result = run.run_until_end(after(&run, ms(5_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
}

#[test]
fn ur_28_the_device_reports_a_late_burst() {
    // uhd-tx hands the start of burst over in time by its clock; the fake sees it 20 ms
    // later, past its time, and reports TimeError (VE-3's late_at_device).
    let (mut run, _device, _dir) = tx_session(FakeConfig { faults: vec![FakeFault::SlowSend(Wall::from_millis(20))], ..FakeConfig::default() });
    // A reservation whose delay cannot change the verdict is `send_asap_and_flag` (rule
    // (3)): a booking the test thread's delay made late is moved, not dropped.
    let _ = send(&mut run, "start_repeat", Some(ms(8)), &tone(100));
    wait(&mut run, ms(100));
    let manifest = run.finish();
    let late: Vec<_> = time_errors(&manifest).into_iter().filter(|p| p.outcome == TimeErrorOutcome::LateAtDevice).collect();
    assert_eq!(late.len(), 1, "{:?}", time_errors(&manifest));
    let record = &bursts(&manifest)[0];
    assert_eq!(late[0].target, record.target, "the late report names the moved start");
    assert!(late[0].late_by_ns > 0);
}

#[test]
fn ur_29_a_lost_device_aborts_the_run() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::Lost(Wall::from_millis(2_200))], ..FakeConfig::default() });
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new()).expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
    let result = run.run_until_end(after(&run, ms(5_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
    assert_eq!(events_of(&manifest, EventKind::DEVICE_LOST)[0].source, usrp());
    assert_marked_lost_before_closed(&device);
}

#[test]
fn ur_29_a_silent_stream_is_a_lost_device() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::Silence(Wall::from_millis(2_100))], ..FakeConfig::default() });
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new()).expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
    // The property is about device time: the Run must end before its 8 s horizon, which
    // `Err(Ended)` says, and `after_silence` below reads when the Provider lost the device.
    let result = run.run_until_end(after(&run, ms(8_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    let lost = &events_of(&manifest, EventKind::DEVICE_LOST)[0];
    assert!(lost.payload["message"].as_str().unwrap().contains("UR-29"));
    // Within 1 to 2 s of the silence, which begins 2.1 s after the time was set (L14).
    let after_silence = lost.time.ticks_in(lost.time.domain()).unwrap() - ms(2_100);
    assert!(after_silence >= ms(1_000) && after_silence <= ms(2_000), "{} ms", after_silence / ms(1));
    assert_marked_lost_before_closed(&device);
}

#[test]
fn ur_29_a_dead_link_that_fails_without_lost_is_a_lost_device() {
    // Review S, S-B1 and TG-S1: a link that answers nothing, its calls failing as UHD's
    // `op_timeout` (not lost), in a transmit-only Session with nothing sent: the device
    // reads failing for 1 s make the device lost, and it is marked lost (F4).
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::Unreachable(Wall::from_millis(2_500))], ..FakeConfig::default() });
    let mut doc = profile(&dir, json!({}), json!({}), true);
    doc["bindings"]["rec"].as_object_mut().unwrap().remove("feed");
    doc.as_object_mut().unwrap().remove("placements");
    doc["bindings"].as_object_mut().unwrap().remove("rec");
    let mut run = session(&doc, device.clone()).expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
    past_t0(&mut run, ms(1));
    let result = run.run_until_end(after(&run, ms(6_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    let lost = events_of(&manifest, EventKind::DEVICE_LOST);
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert!(lost[0].payload["message"].as_str().unwrap().contains("UR-29: the device time reads have failed for 1 s"), "{lost:?}");
    // Within 2 s of the link's death (the reads every 500 ms; 100 ms each on the fake).
    let after_death = lost[0].time.ticks_in(lost[0].time.domain()).unwrap() - ms(2_500);
    assert!((ms(1_000)..=ms(2_000)).contains(&after_death), "{} ms", after_death / ms(1));
    assert_marked_lost_before_closed(&device);
}

#[test]
fn ur_29_time_reads_that_fail_apart_are_not_a_lost_device() {
    // Review T, TG-T1: time reads failing for 600 ms twice, 1.5 s apart (each catching one
    // or two of uhd-control's reads, 500 ms apart), each followed by reads that succeed,
    // start the 1 s count again: no DEVICE_LOST.
    let dir = TempDir::new();
    let window = Wall::from_millis(600);
    let faults = vec![FakeFault::TimeReadFailsFor(Wall::from_millis(2_500), window), FakeFault::TimeReadFailsFor(Wall::from_millis(4_000), window)];
    let device = fake(FakeConfig { faults, ..FakeConfig::default() });
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new());
    let result = run.run_until_end(after(&run, ms(5_000)));
    let manifest = run.finish();
    assert!(events_of(&manifest, EventKind::DEVICE_LOST).is_empty(), "{result:?} {:?}", events_of(&manifest, EventKind::DEVICE_LOST));
    let failed = section(&manifest, "timing").as_array().unwrap().iter().filter(|r| r["what"] == "time_read_failed").count();
    assert!(failed >= 2, "{}", section(&manifest, "timing"));
}

#[test]
fn ur_29_a_failing_reference_sensor_alone_is_not_a_lost_device() {
    // Review T, NB-T1: with the reference monitored, `ref_locked` failing while the time
    // reads succeed is recorded, not a lost device.
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::RefLockedFails(Wall::from_millis(2_300))], ..FakeConfig::default() });
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({ "clock_source": "external" }), json!({}), false), device, BTreeMap::new());
    let result = run.run_until_end(after(&run, ms(5_000)));
    let manifest = run.finish();
    assert!(events_of(&manifest, EventKind::DEVICE_LOST).is_empty(), "{result:?} {:?}", events_of(&manifest, EventKind::DEVICE_LOST));
    assert!(section(&manifest, "timing").as_array().unwrap().iter().any(|r| r["what"] == "ref_locked_failed"), "{}", section(&manifest, "timing"));
}

#[test]
fn ur_29_a_lost_device_is_found_while_idle() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::Lost(Wall::from_millis(2_500))], ..FakeConfig::default() });
    let mut doc = profile(&dir, json!({}), json!({}), true);
    doc["bindings"]["rec"].as_object_mut().unwrap().remove("feed");
    doc.as_object_mut().unwrap().remove("placements");
    doc["bindings"].as_object_mut().unwrap().remove("rec");
    let mut run = session(&doc, device.clone()).expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
    past_t0(&mut run, ms(1));
    let result = run.run_until_end(after(&run, ms(5_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    assert_eq!(events_of(&manifest, EventKind::DEVICE_LOST).len(), 1);
    assert_marked_lost_before_closed(&device);
}

// ---------------------------------------------------------------- records (UR-30, UR-31)

#[test]
fn ur_30_the_sections_are_written() {
    let (manifest, _dir) = receive_run(FakeConfig::default(), 1, 1_000);
    for suffix in ["device", "envelope", "applied", "bursts", "async", "rejected", "switches", "timing", "stats"] {
        assert!(manifest.section(&usrp(), &format!("ezsdr.radio.uhd.{suffix}")).is_some(), "{suffix}");
    }
    let stats = section(&manifest, "stats");
    for key in ["rx_blocks", "rx_samples", "rx_overflows", "rx_errors", "rx_off_lattice", "rx_overlapping", "link_drops_seen", "tx_bursts", "tx_samples", "tx_errors"] {
        assert!(stats[key].is_i64(), "{key}");
    }
    assert!(stats["rx_samples"].as_i64().unwrap() >= 1_000);
}

#[test]
fn ur_31_fidelity_follows_the_device() {
    let (manifest, _dir) = receive_run(FakeConfig::default(), 1, 1_000);
    assert_eq!(manifest.run.fidelity, ezsdr_kernel::module_api::Fidelity::NONE);
    assert_eq!(manifest.run.execution_class, Some(ExecutionClass::HardwareInLoop));
}

// ---------------------------------------------------------------- the fake (UR-33)

#[test]
fn ur_33_no_document_selects_the_fake_device() {
    for args in ["fake", "type=fake"] {
        assert!(ezsdr_radio_uhd::open(args).is_err(), "{args}");
    }
    let refused = UhdRadio::from_binding(&binding(json!({ "args": ARGS, "fake": true }), Some(x3xx())), fake(FakeConfig::default()));
    assert!(refused.err().unwrap().message.contains("unsupported selector key `fake`"));
}

#[test]
fn ur_33_the_front_end_table_follows_the_subdev_spec() {
    // UR-33: the device's channels are its current subdev spec's, named as UHD names them.
    // A Basic/LF board carries the receive front ends `0` and `1` (`db_basic_and_lf.hpp`),
    // and the default spec is every front end the boards have.
    let dir = ezsdr_radio_uhd::Dir::Rx;
    let device = fake(FakeConfig { front_ends: vec!["BasicRX"], ..FakeConfig::default() });
    assert_eq!(device.channels(dir), 2);
    assert_eq!(device.front_end(dir, 0).unwrap(), "BasicRX (0)");
    assert_eq!(device.front_end(dir, 1).unwrap(), "BasicRX (1)");
    assert_eq!(device.channels(ezsdr_radio_uhd::Dir::Tx), 0, "a BasicRX carries no transmit front end");
    device.set_subdev_spec(dir, "A:1").unwrap();
    assert_eq!(device.channels(dir), 1);
    assert_eq!(device.front_end(dir, 0).unwrap(), "BasicRX (1)");
    assert!(device.front_end(dir, 1).is_err(), "the spec has one channel");
    let device = fake(FakeConfig::default());
    assert_eq!(device.channels(dir), 2, "slot A and slot B, one UBX front end each");
    assert_eq!(device.front_end(dir, 0).unwrap(), "UBX RX");
    assert_eq!(device.front_end(dir, 1).unwrap(), "UBX RX");
    device.set_subdev_spec(dir, "B:0").unwrap();
    assert_eq!(device.channels(dir), 1);
    assert_eq!(device.front_end(dir, 0).unwrap(), "UBX RX");
    // The spec's own order is the channel order.
    let device = fake(FakeConfig { front_ends: vec!["BasicRX"], ..FakeConfig::default() });
    device.set_subdev_spec(dir, "A:1 A:0").unwrap();
    assert_eq!(device.front_end(dir, 0).unwrap(), "BasicRX (1)");
    assert_eq!(device.front_end(dir, 1).unwrap(), "BasicRX (0)");
    // A BasicTX: one transmit front end, no receive one.
    let device = fake(FakeConfig { front_ends: vec!["LFTX"], ..FakeConfig::default() });
    assert_eq!(device.channels(ezsdr_radio_uhd::Dir::Tx), 1);
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Tx, 0).unwrap(), "LFTX (0)");
    assert_eq!(device.channels(dir), 0, "an LFTX carries no receive front end");
    // A front end that stays keeps its settings across a spec (UR-33, INFERRED: UHD's
    // channels are the radio blocks', whose registers a channel map does not touch).
    let device = fake(FakeConfig { front_ends: vec!["BasicRX"], ..FakeConfig::default() });
    let set = ezsdr_radio_uhd::Settings { freq: Some(2e9), ..ezsdr_radio_uhd::Settings::default() };
    device.apply(dir, 1, &set).unwrap();
    device.set_subdev_spec(dir, "A:1").unwrap();
    assert_eq!(device.apply(dir, 0, &ezsdr_radio_uhd::Settings::default()).unwrap().freq, 2e9, "A:1's setting stayed with it");
}

#[test]
fn ur_33_a_subdev_spec_uhd_would_refuse_is_refused() {
    // UR-33: a pair the table has not, a bare front end with no slot, a repeated pair, an
    // empty spec and a malformed markup are refused, the lookup error in UHD's own words
    // (`multi_usrp_rfnoc.cpp:1226-1254`) and the malformed markup's with `uhd_subdev_spec_make`
    // as its call, which is where UHD parses it (`subdev_spec.cpp:19-20,58-59`: tokens split
    // on ASCII spaces alone, and a token of three or more `:`-parts is refused).
    let dir = ezsdr_radio_uhd::Dir::Rx;
    let device = fake(FakeConfig::default());
    let refuse = |markup: &str| device.set_subdev_spec(dir, markup).err().unwrap().message;
    assert_eq!(refuse("A:1"), "uhd_usrp_set_rx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev A:1", "a pair the table has not",);
    assert_eq!(refuse("0"), "uhd_usrp_set_rx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev :0", "a bare front end names no slot",);
    assert_eq!(refuse("A:0 A:0"), "uhd_usrp_set_rx_subdev_spec: UHD error 43: ValueError: duplicate subdev A:0", "a repeated pair",);
    assert_eq!(refuse(""), "uhd_usrp_set_rx_subdev_spec: UHD error 43: ValueError: empty subdev-spec markup string", "an empty spec",);
    assert_eq!(refuse("A:0:1"), "uhd_subdev_spec_make: UHD error 43: ValueError: invalid subdev-spec markup string: A:0:1", "a malformed markup",);
    assert_eq!(refuse("A:0\tB:0"), "uhd_subdev_spec_make: UHD error 43: ValueError: invalid subdev-spec markup string: A:0\tB:0", "a tab is no separator: UHD splits on spaces alone",);
    assert_eq!(device.channels(dir), 2, "a refused spec changes nothing");
}

#[test]
fn ur_33_a_transmit_send_keeps_the_streamer_s_channel_count() {
    // UR-33: as `UhdDevice` refuses them before the C call (UR-3), a send carries one buffer
    // per channel of its streamer and none shorter than the first.
    let device = fake(FakeConfig::default());
    device.tx_open(0, &[0, 1]).unwrap();
    let wave = vec![[0.1f32, 0.0]; 8];
    let short = vec![[0.1f32, 0.0]; 4];
    assert!(device.tx_send(0, &[&wave], None, false, false, Wall::from_secs(1)).is_err(), "one buffer for two channels");
    assert!(device.tx_send(0, &[&wave, &short], None, false, false, Wall::from_secs(1)).is_err(), "a short channel buffer");
    assert!(device.tx_send(0, &[&wave, &wave], None, false, false, Wall::from_secs(1)).is_ok());
}

#[test]
fn ur_33_one_channel_is_opened_by_one_streamer() {
    // UR-33: the graph refuses a second streamer on one front end — the radio block's port is
    // already connected (`graph.cpp:179-197`, "Attempting to reconnect ? port!"), the same
    // channel twice in one streamer included —, a stream's own reopen replaces its
    // predecessor and a refused reopen leaves the stream with none (as `UhdDevice`'s
    // does), and `close_streams` frees every streamer.
    let device = fake(FakeConfig::default());
    device.rx_open(0, &[0]).unwrap();
    let error = device.rx_open(1, &[0]).err().unwrap().message;
    assert!(error.contains("Attempting to reconnect output port!"), "{error}");
    let error = device.rx_open(0, &[0, 0]).err().unwrap().message;
    assert!(error.contains("Attempting to reconnect output port!"), "the same channel twice: {error}");
    assert!(matches!(device.rx_recv(0, 1, Wall::from_millis(1), false), ezsdr_radio_uhd::RxRecv::Failed(_)), "a refused reopen leaves the stream with none");
    device.rx_open(0, &[0]).unwrap();
    assert!(device.rx_open(0, &[2]).is_err(), "a channel the device has not");
    device.rx_open(0, &[0]).unwrap();
    device.tx_open(0, &[0]).unwrap();
    let error = device.tx_open(1, &[0]).err().unwrap().message;
    assert!(error.contains("Attempting to reconnect input port!"), "{error}");
    let error = device.tx_open(0, &[0, 0]).err().unwrap().message;
    assert!(error.contains("Attempting to reconnect input port!"), "the same channel twice: {error}");
    let out = [[0.0f32, 0.0]; 4];
    assert!(device.tx_send(0, &[&out], None, false, false, Wall::from_millis(1)).is_err(), "a refused reopen leaves the transmit stream with none");
    device.tx_open(0, &[0]).unwrap();
    device.tx_open(1, &[1]).unwrap();
    assert!(device.tx_open(2, &[1]).is_err(), "channel 1 is stream 1's");
    assert!(device.tx_open(0, &[0, 1]).is_err(), "channel 1 is stream 1's");
    assert!(device.take_abort().is_some_and(|open| open.starts_with("rx_open(1, [0])")), "the first open that would abort a real device's process");
    // Every streamer is freed: a channel one of them held can be opened again.
    device.close_streams();
    device.rx_open(2, &[0, 1]).unwrap();
    device.tx_open(2, &[0, 1]).unwrap();
}

#[test]
#[should_panic(expected = "on UHD 4.10 this aborts the process")]
fn ur_33_an_open_of_a_held_front_end_fails_the_test_at_drop() {
    // UR-33: on UHD 4.10 an open of a front end another stream's streamer holds is refused,
    // but it cuts the holder's edge and the process aborts later (bench-results.md,
    // 2026-10-21); the fake's drop fails a test that makes one and does not take it.
    let device = fake(FakeConfig::default());
    device.tx_open(0, &[0]).unwrap();
    assert!(device.tx_open(1, &[0]).is_err());
}

#[test]
fn ur_33_setting_a_spec_disconnects_that_direction_s_streamers() {
    // UR-33: UHD's `set_*_subdev_spec` rebuilds the direction's channel map and disconnects
    // its chains; the other direction's streamers stay. Each direction for itself.
    let device = fake(FakeConfig::default());
    device.rx_open(0, &[0]).unwrap();
    device.tx_open(0, &[0]).unwrap();
    device.set_subdev_spec(ezsdr_radio_uhd::Dir::Rx, "A:0").unwrap();
    assert!(matches!(device.rx_recv(0, 1, Wall::from_millis(1), false), ezsdr_radio_uhd::RxRecv::Failed(_)), "the receive streamer went with the map");
    let out = [[0.0f32, 0.0]; 4];
    assert!(device.tx_send(0, &[&out], None, false, false, Wall::from_millis(1)).is_ok(), "the transmit streamer stays");
    // The transmit side the same: its streamer goes with its map, the receive one stays.
    let device = fake(FakeConfig::default());
    device.rx_open(0, &[0]).unwrap();
    device.tx_open(0, &[0]).unwrap();
    device.set_subdev_spec(ezsdr_radio_uhd::Dir::Tx, "A:0").unwrap();
    assert!(device.tx_send(0, &[&out], None, false, false, Wall::from_millis(1)).is_err(), "the transmit streamer went with its map");
    assert!(matches!(device.rx_recv(0, 1, Wall::from_millis(1), false), ezsdr_radio_uhd::RxRecv::Timeout), "the receive streamer stays: its call waits, not fails");
}

#[test]
fn ur_33_rates_report_the_grid_and_the_pre_chain_one() {
    // UR-33: the default is the profile's rate grid as UHD's `(start, stop, step)` ranges;
    // a fake built with `rates_before_open` reports UHD's pre-chain answer `[(1.0, 1.0,
    // 0.0)]` until a streamer opened the channel (INFERRED, design 33 D5; BP0 measures it).
    let dir = ezsdr_radio_uhd::Dir::Rx;
    let device = fake(FakeConfig::default());
    let rates = device.rates(dir, 0).unwrap();
    assert_eq!(rates.len(), 639, "the X3xx grid's values");
    assert!(rates.iter().all(|(start, stop, step)| start == stop && *step == 0.0), "{:?}", &rates[..3]);
    assert!(rates.windows(2).all(|pair| pair[0].0 < pair[1].0), "ascending by rate");
    assert_eq!(rates.last().unwrap().0, 200_000_000.0, "the highest rate is the master clock");
    assert!(!rates.iter().any(|(rate, ..)| (rate - 200_000_000.0 / 257.0).abs() < 1.0), "257 is no decimation");
    assert!(device.rates(dir, 2).is_err(), "a channel the device has not");
    let device = fake(FakeConfig { rates_before_open: true, ..FakeConfig::default() });
    assert_eq!(device.rates(dir, 0).unwrap(), vec![(1.0, 1.0, 0.0)]);
    device.rx_open(0, &[0]).unwrap();
    assert_eq!(device.rates(dir, 0).unwrap().len(), 639, "the chain is connected now");
    // A rebuilt channel map's chains are new: its channels count as unconnected again.
    device.set_subdev_spec(dir, "A:0").unwrap();
    assert_eq!(device.rates(dir, 0).unwrap(), vec![(1.0, 1.0, 0.0)]);
}

#[test]
fn ur_33_the_serial_is_the_config_s() {
    // UR-33: the device's motherboard serial is the config's; design 33 D5 tells two
    // bindings of one device apart by it. The default is one serial per `FakeDevice`, so
    // that the two devices a test binary's parallel tests build are two devices.
    let (a, b) = (fake(FakeConfig::default()), fake(FakeConfig::default()));
    assert_ne!(a.serial().unwrap(), b.serial().unwrap());
    assert_eq!(fake(FakeConfig { serial: "F00BA2".to_owned(), ..FakeConfig::default() }).serial().unwrap(), "F00BA2");
}

#[test]
fn ur_33_a_real_fake_s_time_starts_at_its_own_creation() {
    // R07 (plan 30 stage A): a real-clock FakeDevice counts its time from its own creation,
    // not from the process-wide host origin, so one built later has not already spent time
    // and a `Lost(d)` deadline is `d` after its own creation, not after the process's.
    // Each age is bracketed by the process-wide real clock's readings around the creation and
    // around the read, so nothing here depends on how promptly this thread runs (R12); the
    // zero itself is pinned by `device.rs`'s `ur_33_a_real_fake_s_time_counts_from_its_own_creation`.
    let first = fake_real(FakeConfig::default());
    first.host().sleep(Wall::from_millis(100));
    let host = first.host();
    let before = host.now();
    let second = fake_real(FakeConfig { faults: vec![FakeFault::Lost(Wall::from_millis(200))], ..FakeConfig::default() });
    let read = second.time_now();
    let since = host.now() - before;
    match read {
        Ok(ticks) => {
            let age = Wall::from_nanos(ticks as u64 * 1_000_000_000 / MCR as u64);
            assert!(age <= since, "the later fake's time {age:?} is older than the {since:?} since its creation: the process's clock");
        }
        Err(_) => assert!(since >= Wall::from_millis(200), "the later fake was lost {since:?} after its own creation, before its 200 ms"),
    }
    second.host().sleep(Wall::from_millis(250));
    assert!(second.time_now().is_err(), "the Lost deadline did not count from the later fake's own creation");
    // The first's own clock is untouched by the second's (its own origin, its own time).
    assert!(first.time_now().unwrap() >= ms(100), "the first fake lost its own time");
}

// ---------------------------------------------------------------- the rehearsal (UR-34)
// Each step on the fake's two UBX and on the bench's one OBX (`x3xx`, an OBX layout); B3 on one CBX.

#[test]
fn rehearsal_b3_receive_at_t0() {
    let _ = rehearse_receive_at_t0(fake(FakeConfig::default()));
}

#[test]
fn rehearsal_b4_capture_at_a_sample_index() {
    let _ = rehearse_capture_at_a_sample_index(fake(FakeConfig::default()));
}

// `rehearsal_b5_overflow`'s overflow, the block that resumes after it and the `OverflowRestart`
// gap they make are the unit test of that name in `src/provider/rx.rs`, on the controlled clock
// (design note 37): the bench's own rehearsal, with its capture write-out, is `hw_b5_overflow`.

#[test]
fn rehearsal_b6_txrx_and_repeat() {
    let _ = rehearse_txrx_and_repeat(fake(FakeConfig::default()), true);
}

#[test]
fn a_duplex_pair_that_does_not_flow_says_the_two_values_were_not_shown_to_hold() {
    let device = fake(obx_and_cbx());
    let plan = two_rate_plan();
    let limits = measurement::DuplexLimits {
        rx_condition: "rx-test".to_owned(),
        rx: vec![measurement::StreamPlan { channels: vec![0], rate_hz: 390_625 }],
        tx_condition: "tx-test".to_owned(),
        tx: vec![measurement::StreamPlan { channels: vec![0], rate_hz: 1_000_000 }],
    };
    let group = with_selector(two_channel_selector(), || {
        measurement::with_injected_fault(measurement::InjectedFault { starve_rx: Some(0), ..Default::default() }, || {
            measurement::measure_duplex_at_limits(device.clone(), &plan, Ok(limits), 4).unwrap()
        })
    });
    let row = &group.observations[0];
    assert!(group.values.is_empty());
    assert_ne!(row.result["class"], "Success", "{:?}", row.result);
    assert!(row.result["verdict"].as_str().is_some_and(|verdict| verdict.contains("not shown")), "{:?}", row.result["verdict"]);
    assert!(
        row.result["streams"].as_array().is_some_and(|streams| streams.iter().any(|stream| stream["direction"] == "rx" && stream["class"] != "Success")),
        "the failed stream is named: {:?}",
        row.result["streams"]
    );
}

#[test]
fn the_ma1_phase_is_read_against_the_waveform_and_a_weak_or_unsteady_loop_is_inconclusive() {
    let wave = common::transmit_samples(1_000);
    let rotate = |deg: f64, gain: f32| -> Vec<(f32, f32)> {
        let (c, s) = ((deg.to_radians().cos()) as f32, (deg.to_radians().sin()) as f32);
        wave.iter().map(|(re, im)| (gain * (re * c - im * s), gain * (re * s + im * c))).collect()
    };
    let mut samples = vec![(0.0f32, 0.0f32); 100];
    samples.extend(rotate(40.0, 0.02));
    let phase = tone::burst_phase(&samples, &wave, 100).unwrap();
    assert!((phase.deg - 40.0).abs() < 0.1 && (phase.gain - 0.02).abs() < 1e-4, "{phase:?}");
    assert!(phase.halves_deg.iter().all(|half| (half - 40.0).abs() < 0.1));
    // A phase that moves across the burst shows in its halves.
    let mut moving = vec![(0.0f32, 0.0f32); 100];
    moving.extend(&rotate(0.0, 0.02)[..500]);
    moving.extend(&rotate(90.0, 0.02)[500..]);
    let moved = tone::burst_phase(&moving, &wave, 100).unwrap();
    assert!(tone::angle_difference_deg(moved.halves_deg[0], moved.halves_deg[1]) > 80.0 && !tone::phase_is_steady(&moved), "{moved:?}");
    assert!(tone::phase_is_steady(&phase), "{phase:?}");
    assert!(tone::burst_phase(&samples[..500], &wave, 100).is_none(), "a capture that ends inside the burst has no phase");
    // The verdict: a readable phase, a loop under the gain threshold, and a moving phase.
    assert!((tone::phase_verdict(&phase).unwrap() - 40.0).abs() < 0.1);
    let mut weak = vec![(0.0f32, 0.0f32); 100];
    weak.extend(rotate(40.0, 0.001));
    let weak = tone::burst_phase(&weak, &wave, 100).unwrap();
    assert!(tone::phase_verdict(&weak).unwrap_err().contains("too weak"), "{weak:?}");
    assert!(tone::phase_verdict(&moved).unwrap_err().contains("not steady"), "{moved:?}");
    // The spread is the largest circular difference, across the wrap at 180 degrees.
    assert_eq!(tone::circular_spread_deg(&[170.0, -170.0]), Some(20.0));
    assert_eq!(tone::circular_spread_deg(&[10.0, 40.0, 25.0]), Some(30.0));
    assert_eq!(tone::circular_spread_deg(&[10.0]), None);
}

#[test]
fn rehearsal_b7_refusal_holds_on_a_two_board_layout() {
    // Each transmit channel has its own band, and RM-19 admits a centre inside any of them: with an OBX at 1 GHz and a CBX at 2.45 GHz the
    // bands are 999–1001 MHz and 2.449–2.451 GHz, and B7's refused retune (1.5 GHz) is outside both.
    with_selector(two_channel_selector(), || {
        let device = fake(obx_and_cbx());
        let envelope = bench_envelope(&*device);
        assert_eq!(envelope["radio.rf_envelope"]["allowed_bands"].as_array().map(Vec::len), Some(2));
        let _ = rehearse_session_loopback(device, true);
    });
}

#[test]
fn rehearsal_b7_session_loopback() {
    let _ = rehearse_session_loopback(fake(FakeConfig::default()), true);
}

#[test]
fn rehearsal_b3_receive_at_t0_on_one_cbx() {
    let manifest = rehearse_receive_at_t0(fake(one_cbx()));
    let frequency = section(&manifest, "applied").as_array().unwrap().iter().find(|r| r["key"] == "radio.rx.c0.frequency_hz").cloned().unwrap();
    assert_eq!(frequency["claimed"], 2.45e9, "the bench frequency is the CBX board's default");
}

#[test]
fn rehearsal_b3_receive_at_t0_on_one_obx() {
    let manifest = rehearse_receive_at_t0(fake(one_obx()));
    assert_eq!(section(&manifest, "envelope")["profile"]["name"], "x3xx");
}

#[test]
fn rehearsal_b4_capture_at_a_sample_index_on_one_obx() {
    let _ = rehearse_capture_at_a_sample_index(fake(one_obx()));
}

#[test]
fn rehearsal_b6_txrx_and_repeat_on_one_obx() {
    let _ = rehearse_txrx_and_repeat(fake(one_obx()), true);
}

#[test]
fn rehearsal_b7_session_loopback_on_one_obx() {
    let _ = rehearse_session_loopback(fake(one_obx()), true);
}

// ---------------------------------------------------------------- the bench's correlation
// B6 on the bench (bench-results.md, session 2): the burst heard at sample 44 with a loop
// gain of 0.0152 (−36.4 dB) at a 24.6° LO phase, which `correlate` must find whatever
// the phase, and a capture of noise alone, in which it must find nothing.

/// Gaussian-like noise of RMS `rms` per sample, deterministic (a sum of four uniforms).
fn bench_noise(n: usize, rms: f32) -> Vec<(f32, f32)> {
    let mut state: u32 = 0x9e37_79b9;
    let mut uniform = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f32::from(u16::try_from(state >> 16).unwrap()) / 65_536.0 - 0.5
    };
    // Four uniforms on [-0.5, 0.5) sum to variance 1/3 per component.
    let scale = rms / (2.0f32 / 3.0).sqrt();
    (0..n).map(|_| ((0..4).map(|_| uniform()).sum::<f32>() * scale, (0..4).map(|_| uniform()).sum::<f32>() * scale)).collect()
}

fn bench_loop(gain: f32, phase_deg: f32, delay: usize) -> Vec<(f32, f32)> {
    let wave = pn(1_000);
    let (sin, cos) = phase_deg.to_radians().sin_cos();
    let mut samples = bench_noise(5_000, 0.003);
    for (i, (re, im)) in wave.iter().enumerate() {
        samples[delay + i].0 += gain * (re * cos - im * sin);
        samples[delay + i].1 += gain * (re * sin + im * cos);
    }
    samples
}

#[test]
fn correlate_finds_the_bench_loop_at_any_phase() {
    let wave = pn(1_000);
    for phase in [24.6, 90.0, 180.0, -120.0] {
        assert_eq!(correlate(&bench_loop(0.0152, phase, 44), &wave), Some(44), "the loop at {phase}°");
    }
    assert_eq!(correlate(&bench_noise(5_000, 0.003), &wave), None, "noise alone");
}

#[test]
fn back_to_back_sees_a_slip_in_a_cabled_repeat() {
    let wave = pn(1_000);
    let (sin, cos) = 24.6f32.to_radians().sin_cos();
    let turned = |slip: usize| -> Vec<(f32, f32)> {
        let mut samples = bench_noise(4_000, 0.003);
        for i in 44..4_000 {
            let (re, im) = wave[(i - 44 + if i >= 2_044 { slip } else { 0 }) % 1_000];
            samples[i].0 += 0.0152 * (re * cos - im * sin);
            samples[i].1 += 0.0152 * (re * sin + im * cos);
        }
        samples
    };
    let whole = back_to_back(&turned(0), &wave, 44);
    assert_eq!(whole.len(), 3);
    assert!(whole.iter().all(|(_, gain, phase)| (gain - 0.0152).abs() < 0.002 && (phase - 24.6).abs() < 5.0), "{whole:?}");
    let slipped = back_to_back(&turned(7), &wave, 44);
    assert!(slipped[2].1 < 0.5 * slipped[0].1, "a 7-sample slip at the third period: {slipped:?}");
}

// ---------------------------------------------------------------- Review N
// The fake's ways of being as strict as the X300 (UR-33) and the bench link the receive tests
// of B1 and B2 use. The raw cases — a timed stop, a stopped stream's tail and its whole timeout,
// a request cut short by a stop, and the transmit and report cases (timed starts, an empty
// end-of-burst, an underflow and its recovery, and per-stream reports) — are unit tests in
// `src/device.rs`, where every thread of a scenario is a participant of the fake's clock
// (plan 30 D2).

#[test]
fn ur_23_a_burst_alone_is_ended_before_it_runs_dry() {
    // Review N, B3's deferral: with no burst booked at its end, the device burst is
    // closed a device lead before its end, not left to underflow. Booked in the Spec
    // (rule (1)).
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let (bytes, waveform) = waveform_of(&tone(5_000));
    let spec = with_burst(with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &waveform, false, "drop_and_flag", 20_000);
    let manifest = finish_spec(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(waveform.hash.clone(), bytes)])), ms(80));
    let codes: Vec<_> = section(&manifest, "async").as_array().unwrap().iter().map(|r| r["code"].as_str().unwrap().to_owned()).collect();
    assert!(
        section(&manifest, "async").as_array().unwrap().iter().any(|row| row["code"] == "BurstAck" && row["tx_flush"] != true) && !codes.contains(&"Underflow".to_owned()),
        "{codes:?} {:?}",
        calls(&device, "tx_send")
    );
}

#[test]
fn ur_23_a_short_window_first_send_plays_clean_at_1_mhz() {
    // UR-23 (S3, #78): the first send of a 10 000-sample device burst at 1 MS/s waits for 32 samples of window space, so its open first
    // packet is not short enough to run dry (FakeDevice's rule at interpolation 80 or more) and the whole waveform plays.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let samples = tone(10_000);
    let (bytes, waveform) = waveform_of(&samples);
    let spec = with_burst(with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &waveform, false, "drop_and_flag", 20_000);
    let manifest = finish_spec(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(waveform.hash.clone(), bytes)])), ms(80));
    let rows = section(&manifest, "async").as_array().unwrap().clone();
    let codes: Vec<_> = rows.iter().map(|r| r["code"].as_str().unwrap().to_owned()).collect();
    assert!(!codes.iter().any(|code| code == "Underflow" || code == "TimeError"), "{codes:?}; sends: {:?}", calls(&device, "tx_send"));
    assert_eq!(rows.iter().filter(|row| row["code"] == "BurstAck" && row["tx_flush"] != true).count(), 1, "{codes:?}");
    let sends = tx_sends(&device);
    let played: Vec<_> = samples.iter().map(|s| Some([s.0, s.1])).collect();
    assert_eq!(device.transmitted(sends[0].at.unwrap(), samples.len()), played, "{:?}", calls(&device, "tx_send"));
}

#[test]
fn ur_23_a_burst_at_the_first_safe_sample_after_a_held_tail_close_is_played() {
    // T2's held-tail close sends tail+zero, so the record's first safe start is c + 4 (the close ends at c + 1, plus three). Both bursts are
    // booked in the Spec before the Run starts; A's device burst includes the closing zero.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let a_samples = tone(30_000);
    let (one, a) = waveform_of(&a_samples);
    let (two, b) = second_waveform(&vec![(-0.25f32, 0.125); 1_000]);
    let (ha, hb) = (a.hash.clone(), b.hash.clone());
    let target = serde_json::to_value(resource("radio/tx/s0")).unwrap();
    let mut spec = with_tx(receive_spec(1, 1e6, 1e9, None), 1e6);
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 10_000 }, "action": { "kind": "tx_burst", "target": target.clone(),
            "waveform": a, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 40_004 }, "action": { "kind": "tx_burst", "target": target,
            "waveform": b, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } }
    ]);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(ha, one), (hb, two)]));
    let t0 = run.start_instant().expect("a Spec Run starts");
    let root = t0.ticks_in(t0.domain()).unwrap();
    let _ = run.advance_to(TimePoint::new(t0.domain(), root + ms(100)));
    let manifest = run.finish();
    assert!(time_errors(&manifest).is_empty(), "{:?}; sends: {:?}", time_errors(&manifest), calls(&device, "tx_send"));
    // The first safe sample after A's two-sample close is c + 4.
    let sends = tx_sends(&device);
    let device_bursts: Vec<&[Sent]> = sends.split_inclusive(|s| s.eob).collect();
    assert_eq!(device_bursts.len(), 2, "{sends:?}");
    for (burst, at, len) in [(device_bursts[0], root + ms(10), 30_001), (device_bursts[1], root + ms(40) + 800, 1_001)] {
        assert!(burst[0].sob && burst[0].at == Some(at), "{sends:?}");
        assert!(burst[1..].iter().all(|s| !s.sob && s.at.is_some()), "{sends:?}");
        assert_eq!(burst.iter().map(|s| s.n).sum::<usize>(), len, "{sends:?}");
    }
    let tail_tick = root + ms(40) - 200;
    assert_eq!(device.transmitted(tail_tick, 2), vec![Some([a_samples[29_999].0, a_samples[29_999].1]), Some([0.0, 0.0])]);
    assert_eq!(device.unended_bursts(), 0);
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError" && r["code"] != "Underflow"), "{sends:?}");
    assert_eq!(bursts(&manifest).len(), 2, "{:?}", bursts(&manifest));
}

#[test]
fn ur_23_a_burst_at_e_while_the_held_sample_waits_continues_the_device_burst() {
    // UR-23 (F4-adj): B booked at A's exclusive end `e` before A's held sample goes continues the device burst: one SOB, no gap.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let a_samples = tone(30_000);
    let (one, a) = waveform_of(&a_samples);
    let (two, b) = second_waveform(&vec![(-0.25f32, 0.125); 1_000]);
    let (ha, hb) = (a.hash.clone(), b.hash.clone());
    let target = serde_json::to_value(resource("radio/tx/s0")).unwrap();
    let mut spec = with_tx(receive_spec(1, 1e6, 1e9, None), 1e6);
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 10_000 }, "action": { "kind": "tx_burst", "target": target.clone(),
            "waveform": a, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 40_000 }, "action": { "kind": "tx_burst", "target": target,
            "waveform": b, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } }
    ]);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(ha, one), (hb, two)]));
    let t0 = run.start_instant().expect("a Spec Run starts");
    let root = t0.ticks_in(t0.domain()).unwrap();
    let _ = run.advance_to(TimePoint::new(t0.domain(), root + ms(100)));
    let manifest = run.finish();
    assert!(time_errors(&manifest).is_empty(), "{:?}; sends: {:?}", time_errors(&manifest), calls(&device, "tx_send"));
    let sends = tx_sends(&device);
    assert_eq!(sends.iter().filter(|s| s.sob && s.at.is_some()).count(), 1, "a second SOB: {:?}", calls(&device, "tx_send"));
    assert_eq!(device.unended_bursts(), 0);
    assert_eq!(bursts(&manifest).len(), 2, "{:?}", bursts(&manifest));
}

/// A `tx_send` the fake saw, as its calls record it: `n` counts every sample in the send,
/// including EOB padding.
#[derive(Debug)]
struct Sent {
    n: usize,
    at: Option<i64>,
    sob: bool,
    eob: bool,
}

fn tx_sends(device: &FakeDevice) -> Vec<Sent> {
    tx_experiment_sends(device, "tx_send")
        .iter()
        .map(|c| {
            let field = |key: &str| c.split_whitespace().find_map(|f| f.strip_prefix(key));
            Sent { n: field("n=").unwrap().parse().unwrap(), at: field("at=").and_then(|t| t.parse().ok()), sob: field("sob=") == Some("true"), eob: field("eob=") == Some("true") }
        })
        .collect()
}

#[test]
fn ur_23_two_bursts_back_to_back_loop_back_whole() {
    // A burst booked ahead at another's end continues its device burst (§11 F3): on the
    // sample-exact loopback the capture is the two waveforms end to end, no sample lost,
    // doubled or moved. Both are booked before the first's last buffer goes out, so it
    // ends at the second (`ends_at_held`) and nothing is held back; a continuation booked
    // later, after a held sample, is `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it`
    // (Review P, NB-7).
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let first: Vec<(f32, f32)> = (0..1_000).map(|i| (0.1 + i as f32 / 4_000.0, 0.05)).collect();
    let second: Vec<(f32, f32)> = (0..500).map(|i| (-0.1 - i as f32 / 4_000.0, -0.05)).collect();
    let (one, a) = waveform_of(&first);
    let (two, _) = waveform_of(&second);
    // A second input needs its own name (KC-9).
    let b = ezsdr_kernel::manifest::ingest_input(Ident::parse("waveform2").unwrap(), Namespace::parse("ezsdr.input").unwrap(), format!("mem:{}", ContentHash::of_bytes(&two)), &two);
    let mut spec = with_tx(receive_spec(1, 1e6, 1e9, None), 1e6);
    let target = serde_json::to_value(resource("radio/tx/s0")).unwrap();
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 10_000 }, "action": { "kind": "tx_burst", "target": target, "waveform": a, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 11_000 }, "action": { "kind": "tx_burst", "target": target, "waveform": b, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 10_000 }, "action": { "kind": "update_parameter", "target": serde_json::to_value(output("rec")).unwrap(),
                    "key": "sink.capture_samples", "value": 1_500 } }
    ]);
    let inputs = BTreeMap::from([(a.hash.clone(), one), (b.hash.clone(), two)]);
    let manifest = captured(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), inputs));
    let samples = read_capture(&capture_of(&manifest, "rec"), 1).remove(0);
    let whole: Vec<_> = first.iter().chain(&second).copied().collect();
    assert_eq!(samples, whole, "{:?}", calls(&device, "tx_send"));
    let sends = tx_experiment_sends(&device, "tx_send");
    assert_eq!(sends.iter().filter(|c| c.contains("sob=true")).count(), 1, "{sends:?}");
    assert_eq!(sends.iter().filter(|c| c.ends_with("eob=true")).count(), 1, "{sends:?}");
    assert!(sends.iter().all(|call| !call.contains(" at=- ")), "every payload send is timed: {sends:?}");
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError" && r["code"] != "Underflow"), "{sends:?}");
}

#[test]
fn ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time() {
    // A one-sample burst's only buffer carries its start and time spec, so it is not held
    // back: it is played at its time, not untimed at the deadline (Review P, NB-3, TG-2).
    // Booked in the Spec at 100 ms, beyond the device lead, before the Run starts (rule (1)).
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let (bytes, waveform) = waveform_of(&[(0.5, -0.25)]);
    let spec = with_burst(with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &waveform, false, "drop_and_flag", 100_000);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(waveform.hash.clone(), bytes)]));
    let t0 = run.start_instant().expect("a Spec Run starts");
    let a_root = t0.ticks_in(t0.domain()).unwrap() + ms(100);
    let _ = run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ms(120)));
    let manifest = run.finish();
    assert_eq!(device.transmitted(a_root, 1), vec![Some([0.5, -0.25])], "{:?}", calls(&device, "tx_send"));
    let sends = tx_sends(&device);
    assert!(sends.iter().any(|send| send.sob && send.at == Some(a_root) && send.eob), "the one-sample burst is handed over as its own timed SOB/EOB packet: {sends:?}");
    assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
}

// `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took` is a unit test in `src/provider/tx.rs` on the controlled clock (design note 37): the fake's first send size is uhd-tx's poll phase.

/// A `FakeDevice` that records the thread its last handle is dropped on and the reads
/// after a flag, and can make `uhd-clock`'s reads slow (Review Q's D-1 probe).
struct Watched {
    inner: FakeDevice,
    slow: std::sync::atomic::AtomicBool,
    after: std::sync::atomic::AtomicBool,
    reads_after: std::sync::atomic::AtomicUsize,
    dropped_on: Arc<Mutex<Option<String>>>,
}

impl Drop for Watched {
    fn drop(&mut self) {
        *self.dropped_on.lock().unwrap() = Some(std::thread::current().name().unwrap_or("?").to_owned());
    }
}

impl Device for Watched {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.front_end(dir, chan)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        use std::sync::atomic::Ordering::SeqCst;
        if self.after.load(SeqCst) {
            self.reads_after.fetch_add(1, SeqCst);
        }
        if self.slow.load(SeqCst) && std::thread::current().name() == Some("uhd-clock") {
            std::thread::sleep(Wall::from_millis(60));
        }
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.inner.apply(d, c, s)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
}

fn watched() -> (Arc<Watched>, Arc<Mutex<Option<String>>>) {
    let on = Arc::new(Mutex::new(None));
    let device = Watched { inner: FakeDevice::new(FakeConfig::default()), slow: Default::default(), after: Default::default(), reads_after: Default::default(), dropped_on: on.clone() };
    (Arc::new(device), on)
}

#[test]
fn ur_07_a_reference_that_does_not_lock_is_named() {
    // design-notes §18: the X300's reference PLL that does not lock within UHD's 30 s is
    // refused in words the client can read, UHD's text kept.
    let device = fake(FakeConfig { faults: vec![FakeFault::ReferenceDoesNotLock], ..FakeConfig::default() });
    let Err(error) = DeviceAuthority::new(device, Arc::new(ClockRegistry::new()), "internal", "internal", "fake") else { panic!("no error") };
    assert!(error.starts_with("UR-7: the reference clock did not lock to its internal source within UHD's 30 s; connecting again usually succeeds ("), "{error}");
    assert!(error.contains("Reference Clock PLL failed to lock to internal source."), "{error}");
}

#[test]
fn ur_07_the_last_device_handle_is_not_dropped_on_uhd_clock() {
    // D-1 (design-notes §15): dropped in hw_b2's order while uhd-clock is inside a 60 ms
    // read, the last device handle goes on the dropping thread, not on uhd-clock, whose
    // drop would race UHD's static teardown at process exit.
    use std::sync::atomic::Ordering::SeqCst;
    for _ in 0..3 {
        let (device, on) = watched();
        let authority = DeviceAuthority::new(device.clone(), Arc::new(ClockRegistry::new()), "internal", "internal", "fake").unwrap();
        let time = authority.time();
        device.slow.store(true, SeqCst);
        std::thread::sleep(Wall::from_millis(130));
        drop(time);
        drop(authority);
        drop(device);
        std::thread::sleep(Wall::from_millis(200));
        let here = std::thread::current().name().map(str::to_owned);
        assert_eq!(*on.lock().unwrap(), here);
    }
}

#[test]
fn ur_07_no_device_read_follows_the_authority_s_drop() {
    // D-1: with a TimeAuthority handle outliving the Authority (the Kernel's, a
    // Provider's), uhd-clock makes no device read once `drop` has returned. That `drop`
    // returns is the shared watchdog's to bound, not a wall interval's here (a loaded host
    // stretched it past 50 ms; plan 30 D2); that it wakes uhd-clock at once instead of
    // waiting out its nap is pinned deterministically by the unit test
    // `authority::tests::ur_07_dropping_the_authority_ends_uhd_clock_while_the_manual_clock_is_stopped`.
    use std::sync::atomic::Ordering::SeqCst;
    watch();
    let (device, _) = watched();
    let authority = DeviceAuthority::new(device.clone(), Arc::new(ClockRegistry::new()), "internal", "internal", "fake").unwrap();
    let time = authority.time();
    std::thread::sleep(Wall::from_millis(50));
    drop(authority);
    device.after.store(true, SeqCst);
    std::thread::sleep(Wall::from_millis(350));
    assert_eq!(device.reads_after.load(SeqCst), 0);
    drop(time);
}

// ------------------------------------------------- the descheduled participant (plan 30 §7)
//
// The tests below need a Provider thread to be *busy* while the host deschedules it, so that
// the manual clock stops (M3): the "the host fell behind" a loaded machine produces, made
// deterministic. A test-local Device wrapper does it (plan 30 §7 forbids a new `FakeFault`).

/// Which call of a [`Descheduled`] wrapper the plan holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    TxSend,
    RxRecv,
    TxAsync,
}

/// What a [`Descheduled`] wrapper does when its step is reached (§7).
enum Deschedule {
    /// Hold the `skip`th timed `tx_send` carrying samples busy (0-based), then send.
    TxSend(usize, Wall),
    /// Hold the first `rx_recv` after a block (the stall's own position) busy.
    RxRecvAfterBlock(Wall),
    /// Hold the first `tx_async` with a non-zero timeout — the shutdown's ACK window — busy.
    ShutdownAck(Wall),
}

impl Deschedule {
    fn step(&self) -> Step {
        match self {
            Deschedule::TxSend(..) => Step::TxSend,
            Deschedule::RxRecvAfterBlock(_) => Step::RxRecv,
            Deschedule::ShutdownAck(_) => Step::TxAsync,
        }
    }

    fn busy(&self) -> Wall {
        match self {
            Deschedule::TxSend(_, d) | Deschedule::RxRecvAfterBlock(d) | Deschedule::ShutdownAck(d) => *d,
        }
    }
}

/// A `FakeDevice` whose chosen call is held in a busy `std::thread::sleep` once (§7).
struct Descheduled {
    inner: FakeDevice,
    plan: Mutex<Option<Deschedule>>,
    entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    /// The last `rx_recv` returned samples, so the next one is the stall's own position.
    after_block: std::sync::atomic::AtomicBool,
}

impl Descheduled {
    fn new(inner: FakeDevice) -> Descheduled {
        Descheduled { inner, plan: Mutex::new(None), entered: Mutex::new(None), after_block: Default::default() }
    }

    /// Arms the planned hold; the receiver fires when the hold begins.
    fn arm(&self, plan: Deschedule) -> std::sync::mpsc::Receiver<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.entered.lock().unwrap() = Some(tx);
        *self.plan.lock().unwrap() = Some(plan);
        rx
    }

    /// The inner fake's transmit sends, as `common::calls` reads them (UR-33).
    fn sends(&self) -> Vec<String> {
        self.inner.calls().into_iter().filter(|c| c.starts_with("tx_send")).collect()
    }

    /// Runs the planned busy hold when `step` is the armed one, signalling first. `condition`
    /// carries what a plan keyed on more than the call itself needs: the shutdown's non-zero
    /// report timeout, or that the last `rx_recv` returned a block.
    fn hold(&self, step: Step, condition: bool) -> bool {
        let busy = {
            let mut plan = self.plan.lock().unwrap();
            let armed = match plan.as_mut() {
                Some(p) if p.step() == step => match p {
                    Deschedule::ShutdownAck(_) | Deschedule::RxRecvAfterBlock(_) => condition,
                    Deschedule::TxSend(n, _) => {
                        if !condition {
                            false
                        } else if *n == 0 {
                            true
                        } else {
                            *n -= 1;
                            false
                        }
                    }
                },
                _ => false,
            };
            if !armed {
                return false;
            }
            plan.take().expect("armed above").busy()
        };
        if let Some(tx) = self.entered.lock().unwrap().take() {
            let _ = tx.send(());
        }
        std::thread::sleep(busy);
        true
    }
}

impl Device for Descheduled {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.front_end(dir, chan)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.inner.apply(d, c, s)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        use std::sync::atomic::Ordering::SeqCst;
        let after_block = self.after_block.swap(false, SeqCst);
        self.hold(Step::RxRecv, after_block);
        let got = self.inner.rx_recv(s, n, t, one_packet);
        if matches!(got, ezsdr_radio_uhd::RxRecv::Samples { .. }) {
            self.after_block.store(true, SeqCst);
        }
        got
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.hold(Step::TxSend, at.is_some());
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        if self.hold(Step::TxAsync, !t.is_zero()) {
            return Ok(None);
        }
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
}

/// §7's deterministic reproduction: a busy 200 ms inside uhd-tx's `tx_send` with a near start
/// already fixed. The manual clock stops while uhd-tx is busy (M3), so the burst stays on
/// time; the real clock runs, so the device reports it late (the fake's own late rule).
#[test]
fn ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one() {
    for (mode, late) in [(ClockMode::Manual, false), (ClockMode::Real, true)] {
        let device = Arc::new(Descheduled::new(FakeDevice::new(FakeConfig { clock: mode, ..FakeConfig::default() })));
        let entered = device.arm(Deschedule::TxSend(0, Wall::from_millis(200)));
        let (mut run, _dir) = tx_session_on(device.clone());
        let at = tx_at(&run, ms(30));
        // `send_asap_and_flag`, so that the test thread's own delay moves the booking
        // instead of dropping it; the deschedule is entered either way (rule (3)).
        let entry = send_at(&mut run, "start_repeat", Some(at), &tone(10_000));
        assert!(admitted(&entry), "{mode:?}: {entry:?}");
        entered.recv_timeout(Wall::from_secs(5)).unwrap_or_else(|_| {
            infra(&format!("{mode:?}: uhd-tx was not descheduled inside tx_send within 5 s of real time"));
            panic!("{mode:?}: uhd-tx was not descheduled inside tx_send")
        });
        let what = format!("{mode:?}: the descheduled send's Run");
        let manifest = bounded(&what, Wall::from_secs(30), move || {
            wait(&mut run, ms(150));
            run.finish()
        });
        let errors = time_errors(&manifest);
        assert_eq!(errors.iter().any(|p| p.outcome == TimeErrorOutcome::LateAtDevice), late, "{mode:?}: {errors:?}");
        if !late {
            assert_eq!(bursts(&manifest).len(), 1, "{mode:?}: {:?}", device.sends());
        }
    }
}

/// R02: the 100 ms window for the last BURST_ACK is the device's host clock (UR-26). A
/// participant held busy inside the shutdown freezes that clock, so a window measured on real
/// time would expire before the burst's end has played and the acknowledgement would be lost.
#[test]
fn ur_26_the_shutdown_s_last_burst_ack_waits_on_the_device_s_clock() {
    let device = Arc::new(Descheduled::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() })));
    let (mut run, _dir) = tx_session_on(device.clone());
    let at = tx_at(&run, ms(20));
    // SendAsapAndFlag: a booking the test thread's own delay made late is moved, not dropped
    // (plan 30 §5(c)).
    let entry = send_at(&mut run, "start_repeat", Some(at), &tone(30_000));
    assert!(admitted(&entry), "{entry:?}");
    let runner = device.clone();
    let (manifest, entered) = bounded("the transmit shutdown", Wall::from_secs(30), move || {
        wait(&mut run, ms(15));
        // The first non-zero report read is delayed and returns no report, leaving its ACK for the next poll.
        let entered = runner.arm(Deschedule::ShutdownAck(Wall::from_millis(200)));
        (run.finish(), entered)
    });
    assert!(entered.try_recv().is_ok(), "shutdown made its blocking TX report read");
    let sends = device.sends();
    let payload_start = sends
        .iter()
        .find_map(|call| {
            if !call.contains("sob=true") || call.contains("at=-") {
                return None;
            }
            call.split_whitespace().find_map(|field| field.strip_prefix("at=")?.parse::<i64>().ok())
        })
        .expect("the experiment payload was sent: {sends:?}");
    let close_at = sends
        .iter()
        .rev()
        .find_map(|call| {
            if !call.ends_with("eob=true") || call.contains("at=-") {
                return None;
            }
            call.split_whitespace().find_map(|field| field.strip_prefix("at=")?.parse::<i64>().ok())
        })
        .expect("shutdown sent a timed EOB for the experiment: {sends:?}");
    let burst_acks: Vec<i64> = section(&manifest, "async").as_array().unwrap().iter().filter(|row| row["code"] == "BurstAck" && row["channel"] == 0).filter_map(|row| row["tick"].as_i64()).collect();
    // The close is padded to two samples (UR-22), so its ACK is one tick past their end.
    assert!(
        burst_acks.contains(&(close_at + 2 * 200 + 1)),
        "the experiment EOB's BURST_ACK was lost; the earlier TX-open flush ack does not count: payload={payload_start}, close={close_at}, acks={burst_acks:?}, sends={sends:?}"
    );
}

/// UR-33 (plan 30 stage A): a participant that measures clock time across a loop waits on the
/// clock in every pass. `Tx::shutdown`'s 100 ms ACK window polls `tx_async`; a read that fails
/// at once (`TxAsyncFails`, not lost) would leave uhd-tx busy with no clock wait, so the
/// manual clock it measures stays stopped (M3), the loop never ends, and `Provider::stop`
/// detaches uhd-tx after `JOIN_BOUND` to burn a CPU for the rest of the process. With the
/// failed read waiting its timeout the window closes and uhd-tx joins (no "did not join").
#[test]
fn ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx() {
    let (mut run, _device, _dir) = tx_session(FakeConfig { faults: vec![FakeFault::TxAsyncFails(Wall::ZERO, false)], ..FakeConfig::default() });
    let at = tx_at(&run, ms(20));
    // `send_asap_and_flag`: a delayed booking is moved, not dropped (rule (3)); the shutdown
    // path under test does not depend on the burst, only on not losing it to the delay.
    let entry = send_at(&mut run, "start_repeat", Some(at), &tone(30_000));
    assert!(admitted(&entry), "{entry:?}");
    let manifest = bounded("the transmit shutdown", Wall::from_secs(30), move || {
        wait(&mut run, ms(15));
        run.finish()
    });
    let rejections = rejections(&manifest);
    assert!(!rejections.iter().any(|r| r.contains("did not join") || r.contains("timed out")), "{rejections:?}");
    // The failing read is still counted, with its first text (UR-28).
    assert!(section(&manifest, "stats")["tx_errors"].as_i64().unwrap() > 1, "{:?}", section(&manifest, "stats"));
}

/// R11's device: a manual-clock `FakeDevice` whose first `tx_async` with a timeout — the
/// shutdown's first ACK read; uhd-tx's own loop reads with none — reports the device lost.
/// Every Provider call after that is recorded (`mark_lost`, the loss's own step, aside); the
/// Authority's `uhd-clock` goes on re-anchoring until the Authority is dropped (UR-7).
struct LostAtShutdown {
    inner: FakeDevice,
    lost: std::sync::atomic::AtomicBool,
    timed_send: std::sync::atomic::AtomicBool,
    after: Mutex<Vec<&'static str>>,
}

impl LostAtShutdown {
    fn call(&self, name: &'static str) {
        if self.lost.load(std::sync::atomic::Ordering::SeqCst) && std::thread::current().name() != Some("uhd-clock") {
            self.after.lock().unwrap().push(name);
        }
    }
}

impl Device for LostAtShutdown {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.call("front_end");
        self.inner.front_end(dir, chan)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.call("set_sources");
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.call("set_time_zero");
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        self.call("time_now");
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.call("ref_locked");
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.call("apply");
        self.inner.apply(d, c, s)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.call("rx_open");
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.call("rx_start");
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.call("rx_stop");
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        self.call("rx_recv");
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.call("tx_open");
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.call("tx_send");
        let sent = self.inner.tx_send(s, b, at, sob, eob, t)?;
        if at.is_some() && sent > 0 {
            self.timed_send.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(sent)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.call("tx_async");
        if !t.is_zero() && self.timed_send.load(std::sync::atomic::Ordering::SeqCst) && !self.lost.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(ezsdr_radio_uhd::DeviceError { lost: true, message: "fake: the link went down during the shutdown".to_owned() });
        }
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.call("rx_close");
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.call("tx_close");
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.call("close_streams");
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
}

/// R11 (UR-26, UR-29, UR-33): a loss found by the shutdown's own first report read ends its
/// 100 ms ACK window. After it `reports` returns at once without reading, so without a clock
/// wait; a window that ran on until `until` would keep uhd-tx busy and the manual clock it
/// measures stopped (M3) for ever, and `Provider::stop` would detach it after `JOIN_BOUND`.
#[test]
fn ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window() {
    let device = Arc::new(LostAtShutdown {
        inner: FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }),
        lost: Default::default(),
        timed_send: Default::default(),
        after: Mutex::new(Vec::new()),
    });
    let (mut run, _dir) = tx_session_on(device.clone());
    // Untimed: no read of the Run's time, so no delay of the test thread can drop the burst before the premise (a timed send) holds.
    let _ = send(&mut run, "send", None, &tone(100));
    wait(&mut run, ms(80));
    let manifest = bounded("the transmit shutdown", Wall::from_secs(30), move || run.finish());
    assert!(device.lost.load(std::sync::atomic::Ordering::SeqCst), "the shutdown never read a report with a timeout");
    let rejections = rejections(&manifest);
    assert!(!rejections.iter().any(|r| r.contains("did not join")), "uhd-tx did not end after the loss: {rejections:?}");
    assert_eq!(*device.after.lock().unwrap(), Vec::<&str>::new(), "device calls after the loss");
}

/// S02: the receive stall's origin is the first block in the device's host clock (UR-34). A
/// participant descheduled for a real second inside its receive call freezes that clock, so a
/// stall measured from the wall clock would fire early and overflow the stream at the wrong
/// device instant — the overflow must come only once `after` and `duration` have elapsed there.
#[test]
fn ur_34_the_receive_stall_fires_after_its_device_clock_interval() {
    let dir = TempDir::new();
    let device = Arc::new(Descheduled::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() })));
    device.arm(Deschedule::RxRecvAfterBlock(Wall::from_millis(1_000)));
    let doc = bench_profile(&*device, &dir, json!({}), json!({}), false);
    let spec = receive_spec(1, 1e6, bench_hz(&*device), Some(30_000_000));
    let runner = device.clone();
    let manifest = bounded("the stalled receive", Wall::from_secs(60), move || {
        let mut run = Run::start_spec(&spec, &doc, assembly(&doc, runner, BTreeMap::new(), |r| r.with_rx_stall(Wall::from_millis(500), Wall::from_millis(300))), Declared::default()).unwrap();
        let _ = run.advance_to(after_ticks(&run, ms(2_000) + ms(1_800)));
        run.finish()
    });
    let clock = manifest.clocks.sample_clocks.iter().find(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap()).expect("the receive clock");
    let n = clock.root_ticks_per_tick.num() as i64;
    let origin = clock.origin.ticks_in(clock.root).unwrap();
    let first = section(&manifest, "timing").as_array().unwrap().iter().find(|r| r["what"] == "first_rx_block").expect("the first block").clone();
    let first = origin + first["index"].as_i64().unwrap() * n;
    let overflow = events_of(&manifest, "radio.RX_OVERFLOW").into_iter().next().expect("the stall provoked an overflow");
    // The overflow's time is where the reader left off, the stall's own firing: the gap begins
    // there and the resume is a `duration` later. A stall measured from the wall clock would
    // fire right after the 1 s busy interval, when almost no device time has passed.
    let at = origin + overflow.time.ticks_in(overflow.time.domain()).unwrap() * n;
    assert!(at - first >= ms(500) - ms(50), "the stall fired {} ms after the first block, before its device-clock interval had elapsed", (at - first) / ms(1));
}

// ------------------------------------------------ 1b-4b: per-stream state, threads and locks

#[test]
fn ur_16_a_panicking_thread_ends_a_two_stream_run_with_its_message() {
    // UR-16, RM-16 (design 33 D15): a panicking owner is recorded with its reason, and the
    // device loss it reports ends the whole instance — a Run with both directions' streams
    // included — with UR-16's message, once, before the device is marked lost.
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::RxPanic(Wall::from_millis(2_200))], ..FakeConfig::default() });
    let spec = with_tx(receive_spec(1, 1e6, 1e9, None), 1e6);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new()).expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
    let result = run.run_until_end(after(&run, ms(5_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
    let lost = events_of(&manifest, "ezsdr.DEVICE_LOST");
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert_eq!(lost[0].payload["message"], "a Provider thread panicked");
    let rejected = section(&manifest, "rejected").as_array().unwrap();
    assert!(rejected.iter().any(|r| r["thread"] == "uhd-rx-s0" && r["reason"] == "panicked"), "{rejected:?}");
    assert_marked_lost_before_closed(&device);
}

#[test]
fn ur_26_a_second_stop_does_nothing() {
    // UR-26 (design 33 D15): a second `Provider::stop` does nothing — no second stop row,
    // no second untimed stop of the receive stream.
    let mut direct = Direct::with_links(FakeConfig::default(), &[], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    direct.settle(Wall::from_millis(30));
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    assert_eq!(calls(&direct.device, "rx_stop").len(), 1, "{:?}", direct.device.calls());
    let instance = direct.finish();
    let timing = &section_of(&instance, "timing");
    let stops: Vec<_> = timing.as_array().unwrap().iter().filter(|r| r["what"] == "stop").collect();
    assert_eq!(stops.len(), 1, "{timing}");
    let rejected = &section_of(&instance, "rejected");
    assert!(!rejected.as_array().unwrap().iter().any(|r| r["reason"].as_str().is_some_and(|reason| reason.contains("did not join"))), "{rejected}");
}

/// A `FakeDevice` whose receive and transmit calls wait on a gate the test opens: both
/// Provider threads are held inside a device call, so neither joins within `JOIN_BOUND`, and
/// the order of the `rejected` rows is the order `Provider::stop` joins them in (UR-16, D15).
struct Gated {
    inner: Arc<FakeDevice>,
    held: Mutex<bool>,
    open: std::sync::Condvar,
    entered: Mutex<Option<std::sync::mpsc::Sender<&'static str>>>,
    arrivals: Mutex<std::sync::mpsc::Receiver<&'static str>>,
    /// The receive stream whose calls are held, when only one is (`None`: every stream).
    only: Option<usize>,
    /// Where each Provider thread that makes a gated call reports its end (S13, S14).
    ends: Mutex<Option<std::sync::mpsc::Sender<String>>>,
}

thread_local! {
    /// A test's end notification, installed on a Provider thread by its first gated call:
    /// the thread's TLS destructor sends its name when the thread ends.
    static END: RefCell<Option<End>> = const { RefCell::new(None) };
}

struct End {
    name: String,
    ended: std::sync::mpsc::Sender<String>,
}

impl Drop for End {
    fn drop(&mut self) {
        let _ = self.ended.send(self.name.clone());
    }
}

impl Gated {
    fn new(config: FakeConfig) -> Gated {
        let (entered, arrivals) = std::sync::mpsc::channel();
        Gated {
            inner: Arc::new(FakeDevice::new(config)),
            held: Mutex::new(false),
            open: std::sync::Condvar::new(),
            entered: Mutex::new(Some(entered)),
            arrivals: Mutex::new(arrivals),
            only: None,
            ends: Mutex::new(None),
        }
    }

    /// The device the test observes the calls on.
    fn inner(&self) -> Arc<FakeDevice> {
        self.inner.clone()
    }

    /// A `Gated` that holds only receive stream `k`'s calls (S14).
    fn only(config: FakeConfig, k: usize) -> Gated {
        Gated { only: Some(k), ..Gated::new(config) }
    }

    /// Reports on the returned channel the end of each Provider thread that makes a gated
    /// call after this, by its name: a test waits for a detached worker's end (S13, S14).
    fn ends(&self) -> std::sync::mpsc::Receiver<String> {
        let (ended, ends) = std::sync::mpsc::channel();
        *self.ends.lock().unwrap() = Some(ended);
        ends
    }

    /// Holds every device call from now on.
    fn hold(&self) {
        *self.held.lock().unwrap() = true;
    }

    /// Releases every held call.
    fn release(&self) {
        *self.held.lock().unwrap() = false;
        self.open.notify_all();
    }

    /// Waits until one held call whose name starts with each of `names` has begun.
    fn entered(&self, names: &[&str]) {
        let mut seen: Vec<&'static str> = Vec::new();
        while !names.iter().all(|name| seen.iter().any(|seen| seen.starts_with(name))) {
            let arrivals = self.arrivals.lock().unwrap();
            let name = arrivals.recv_timeout(Wall::from_secs(10)).unwrap_or_else(|_| panic!("no held call arrived: {seen:?}"));
            drop(arrivals);
            seen.push(name);
        }
    }

    /// Waits until `wanted` held calls whose names start with each prefix have begun (S13).
    fn entered_n(&self, wanted: &[(&str, usize)]) {
        let mut seen: Vec<&'static str> = Vec::new();
        while wanted.iter().any(|(name, n)| seen.iter().filter(|seen| seen.starts_with(name)).count() < *n) {
            let arrivals = self.arrivals.lock().unwrap();
            let name = arrivals.recv_timeout(Wall::from_secs(20)).unwrap_or_else(|_| panic!("no held call arrived: {seen:?}"));
            drop(arrivals);
            seen.push(name);
        }
    }

    /// Installs the end notification on this, a Provider thread's, first gated call.
    fn watch_end(&self) {
        let Some(ended) = self.ends.lock().unwrap().clone() else { return };
        END.with(|end| {
            let mut end = end.borrow_mut();
            if end.is_none() {
                *end = Some(End { name: std::thread::current().name().unwrap_or("a Provider thread").to_owned(), ended });
            }
        });
    }

    fn hold_call(&self, stream: usize, name: &'static str) {
        self.watch_end();
        if self.only.is_some_and(|only| only != stream || name.starts_with("tx_")) {
            return;
        }
        let mut held = self.held.lock().unwrap();
        if !*held {
            return;
        }
        if let Some(entered) = self.entered.lock().unwrap().as_ref() {
            let _ = entered.send(name);
        }
        while *held {
            held = self.open.wait(held).unwrap();
        }
    }
}

/// Waits on `ends` until every thread of `names` has ended (S13, S14).
fn ended(ends: &std::sync::mpsc::Receiver<String>, names: &[&str]) {
    let mut seen: Vec<String> = Vec::new();
    while !names.iter().all(|name| seen.iter().any(|seen| seen == name)) {
        let name = ends.recv_timeout(Wall::from_secs(20)).unwrap_or_else(|_| panic!("no worker ended: {seen:?}"));
        seen.push(name);
    }
}

impl Device for Gated {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.front_end(dir, chan)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.inner.apply(d, c, s)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        self.hold_call(s, "rx_recv");
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.hold_call(s, "tx_send");
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        // Held too: a burst a loaded host let underflow is not resumed (UR-22), but its
        // thread still reads reports every turn, so a test can still catch it in a call (#68).
        self.hold_call(s, "tx_async");
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
}

#[test]
fn ur_26_the_sending_threads_join_before_the_receiving_ones() {
    // RM-16, UR-16 (design 33 D15): `Provider::stop` joins the transmit streams' threads
    // before the receive streams' — a `stop` with both held inside a device call records the
    // transmit thread first. The real clock keeps uhd-control running while the two held
    // threads would otherwise freeze the manual one (UR-2, plan 30 §4.4).
    let device = Arc::new(Gated::new(FakeConfig::default()));
    let (run, _dir) = tx_session_on(device.clone());
    let mut run = run.join_is_subject();
    let entry = send(&mut run, "start_repeat", None, &tone(500));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(40));
    device.hold();
    device.entered(&["rx_recv", "tx_"]);
    let runner = device.clone();
    let manifest = bounded("the transmit join order", Wall::from_secs(30), move || run.finish());
    device.release();
    let threads: Vec<String> = section(&manifest, "rejected")
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["reason"].as_str().is_some_and(|reason| reason.contains("did not join")))
        .map(|r| r["thread"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(threads, ["uhd-tx-s0", "uhd-rx-s0"], "{:?}", section(&manifest, "rejected"));
    assert!(!runner.inner.calls().iter().any(|c| c == "close_streams"), "a detached thread's streamer was closed (UR-16)");
}

#[test]
fn ur_25_an_enable_from_no_stream_configures_once() {
    // UR-25, UR-12 (design 33 D15): an enable from no stream is configured by uhd-control
    // itself — one open and one untimed configuration — and the segment's owner does not
    // configure it again at its switch.
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    let entry = send(&mut run, "start_repeat", None, &tone(500));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(40));
    let manifest = run.finish();
    let calls = device.calls();
    assert_eq!(calls.iter().filter(|c| c.starts_with("tx_open 0 [0]")).count(), 1, "{calls:?}");
    assert_eq!(calls.iter().filter(|c| c.starts_with("apply tx 0 rate=1000000")).count(), 1, "{calls:?}");
    assert!(!bursts(&manifest).is_empty(), "{:?}", calls);
}

// `ur_25_an_enable_at_a_timed_update_s_instant_applies_it` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// ------------------------------------------------ Phase 8 stage 1b-4c: the layout's keys

#[test]
fn ur_05_keys_is_an_unknown_selector_key() {
    let error = UhdRadio::from_binding(&binding(json!({ "args": ARGS, "keys": "channel" }), Some(x3xx())), fake(FakeConfig::default())).err().unwrap();
    assert_eq!(error.kind, ModuleErrorKind::Rejected);
    assert_eq!(error.message, "UR-5: unsupported selector key `keys`");
}

/// The two directions' `set_subdev_spec` calls of a binding, in order (UR-5, H2).
fn spec_calls(device: &FakeDevice) -> Vec<String> {
    calls(device, "set_subdev_spec")
}

#[test]
fn ur_05_the_layout_builds_the_subdev_spec() {
    // UR-5, H2: a layout with elements sets each direction's subdev spec from them, in
    // logical order, before its channel map is read; the device's channels are that spec's.
    let device = fake(FakeConfig { front_ends: vec!["UBX", "BasicRX"], serial: "ur05-spec-a".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,
        "rx": ["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"], "tx": ["0:A:ubx:0"] });
    let radio = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
    let specs = spec_calls(&device);
    assert!(specs.contains(&"set_subdev_spec rx A:0 B:0 B:1".to_owned()), "{specs:?}");
    assert!(specs.contains(&"set_subdev_spec tx A:0".to_owned()), "{specs:?}");
    assert_eq!(specs.len(), 2, "{specs:?}");
    assert_eq!(device.channels(ezsdr_radio_uhd::Dir::Rx), 3);
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Rx, 1).unwrap(), "BasicRX (0)");
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Rx, 2).unwrap(), "BasicRX (1)");
    assert_eq!(section_of(radio.instance(), "device")["rx_channels"], 3, "the manifest's channels are the spec's, read after it");
    // The transmit side: an LFTX in slot B, its front end's number in the name (H5).
    let device = fake(FakeConfig { front_ends: vec!["UBX", "LFTX"], serial: "ur05-spec-b".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,  "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0", "0:B:lf_tx:0"] });
    let _ = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
    let specs = spec_calls(&device);
    assert!(specs.contains(&"set_subdev_spec rx A:0".to_owned()), "{specs:?}");
    assert!(specs.contains(&"set_subdev_spec tx A:0 B:0".to_owned()), "{specs:?}");
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Tx, 1).unwrap(), "LFTX (0)");
}

#[test]
fn ur_05_the_layout_s_order_is_the_device_s_channel_order() {
    // UR-5, H1: the spec's order is the channel order, so a layout that is not UHD's
    // default order (slot A first) maps channel 0 to slot B (RM-28).
    let device = fake(FakeConfig { front_ends: vec!["OBX", "UBX"], serial: "ur05-order".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,  "rx": ["0:B:ubx:0", "0:A:obx:0"], "tx": ["0:A:obx:0"] });
    let _ = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
    assert!(spec_calls(&device).contains(&"set_subdev_spec rx B:0 A:0".to_owned()), "{:?}", device.calls());
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Rx, 0).unwrap(), "UBX RX");
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Rx, 1).unwrap(), "OBX RX");
}

#[test]
fn ur_05_a_direction_without_elements_sets_no_spec() {
    // UR-5, H7: a direction with no element is not set — an empty spec is one the fake, and
    // UHD's RFNoC path, would take as something else — and it keeps the device's own map.
    let device = fake(FakeConfig { serial: "ur05-empty".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS, "rx": ["0:A:ubx:0"], "tx": [] });
    let _ = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
    assert!(!device.calls().iter().any(|call| call.starts_with("set_subdev_spec tx")), "{:?}", device.calls());
    assert_eq!(device.channels(ezsdr_radio_uhd::Dir::Tx), 2, "the map is the device's own");
    assert_eq!(spec_calls(&device), ["set_subdev_spec rx A:0"]);
}

#[test]
fn ur_05_a_layout_out_of_uhd_s_default_order_is_accepted() {
    // UR-5, RM-28: the prefix restriction is gone: slot B alone, and B before A, are the
    // channel order (design 33 D5); the parser's own cases live in `ezsdr-radio`'s tests.
    let device = fake(FakeConfig { serial: "ur05-nonprefix".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS, "rx": ["0:B:ubx:0", "0:A:ubx:0"], "tx": ["0:A:ubx:0"] });
    let _ = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
    assert!(spec_calls(&device).contains(&"set_subdev_spec rx B:0 A:0".to_owned()), "{:?}", device.calls());
    let device = fake(FakeConfig { serial: "ur05-nonprefix-b".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS, "rx": ["0:B:ubx:0"], "tx": ["0:A:ubx:0"] });
    let _ = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
    assert!(spec_calls(&device).contains(&"set_subdev_spec rx B:0".to_owned()), "{:?}", device.calls());
    assert_eq!(device.channels(ezsdr_radio_uhd::Dir::Rx), 1);
}

/// A `FakeDevice` that logs the calls its own log does not keep — `channels`, `front_end`,
/// `set_subdev_spec`, `serial`, `describe` — so that a test can compare their order across
/// the opening checks (UR-5).
struct Recording {
    inner: FakeDevice,
    log: Mutex<Vec<String>>,
    /// What `raise_thread_priority` answers: UHD's EPERM when a test sets it (UR-23).
    priority: Mutex<Option<ezsdr_radio_uhd::DeviceError>>,
}

impl Recording {
    fn new(config: FakeConfig) -> Recording {
        Recording { inner: FakeDevice::new(config), log: Mutex::new(Vec::new()), priority: Mutex::new(None) }
    }

    /// The names of the threads that asked for a real-time priority, in order.
    fn raisers(&self) -> Vec<String> {
        self.log().iter().filter_map(|line| line.strip_prefix("raise_thread_priority ").map(str::to_owned)).collect()
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn record(&self, entry: impl FnOnce() -> String) {
        self.log.lock().unwrap().push(entry());
    }
}

impl Device for Recording {
    fn describe(&self) -> Json {
        self.record(|| "describe".to_owned());
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize {
        self.record(|| format!("channels {}", dir.name()));
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.record(|| format!("front_end {} {chan}", dir.name()));
        self.inner.front_end(dir, chan)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.inner.apply(d, c, s)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.record(|| "serial".to_owned());
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.record(|| format!("set_subdev_spec {} {m}", d.name()));
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.record(|| format!("raise_thread_priority {}", std::thread::current().name().unwrap_or("")));
        self.priority.lock().unwrap().clone().map_or(Ok(()), Err)
    }
}

#[test]
fn ur_05_the_spec_is_set_before_the_channel_map_is_read() {
    // UR-5, H1: `from_binding` sets the spec before it reads `front_end`, so the front-end
    // check reads the mapping the spec made, and `describe` — the Manifest's channel
    // counts — is read after it.
    let device = Arc::new(Recording::new(FakeConfig { front_ends: vec!["BasicRX"], serial: "ur05-order-log".to_owned(), ..FakeConfig::default() }));
    let selector = json!({ "args": ARGS,  "rx": ["0:A:basic_rx:1"], "tx": [] });
    let _ = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
    let log = device.log();
    let position = |entry: &str| log.iter().position(|line| line == entry).unwrap_or_else(|| panic!("{entry} missing in {log:?}"));
    let spec = position("set_subdev_spec rx A:1");
    let front_end = position("front_end rx 0");
    let describe = position("describe");
    assert!(spec < front_end, "{log:?}");
    assert!(front_end < describe, "{log:?}");
}

#[test]
fn ur_05_a_layout_naming_front_end_one_is_set_before_the_front_ends_are_checked() {
    // UR-5, H5: a Basic/LF front end is checked with its number, and the spec that makes
    // `(1)` the device's channel 0 comes first: reading the map before the spec would see
    // `BasicRX (0)` and refuse a layout the device can serve.
    let device = fake(FakeConfig { front_ends: vec!["BasicRX"], serial: "ur05-fe-one".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,  "rx": ["0:A:basic_rx:1"], "tx": [] });
    let _ = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
    assert!(spec_calls(&device).contains(&"set_subdev_spec rx A:1".to_owned()), "{:?}", device.calls());
    assert_eq!(device.channels(ezsdr_radio_uhd::Dir::Rx), 1);
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Rx, 0).unwrap(), "BasicRX (1)");
}

#[test]
fn ur_05_the_front_end_check_reads_the_device_after_the_subdev_spec() {
    // UR-5, H1, H5: the check reads the mapping the spec made: a slot A CBX-120 answers
    // channel 0 by default, and a layout naming slot B with a UBX is accepted only because
    // the spec was set first (the read-before-spec order refuses it).
    let device = fake(FakeConfig { front_ends: vec!["CBX-120", "UBX"], serial: "ur05-check-order".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,  "rx": ["0:B:ubx:0"], "tx": [] });
    let _ = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
    assert!(spec_calls(&device).contains(&"set_subdev_spec rx B:0".to_owned()), "{:?}", device.calls());
    assert_eq!(device.channels(ezsdr_radio_uhd::Dir::Rx), 1);
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Rx, 0).unwrap(), "UBX RX");
}

#[test]
fn ur_05_a_board_that_differs_from_the_layout_is_refused() {
    // UR-5: the spec succeeds — the UBX's front end 0 is a real front end — but the name it
    // answers is not the layout's board; the refusal names the direction and channel and
    // both names, after the spec (the spec's call is in the device's log).
    let device = fake(FakeConfig { front_ends: vec!["UBX", "UBX"], serial: "ur05-board-rx".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,  "rx": ["0:B:basic_rx:0"], "tx": [] });
    let error = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).err().unwrap();
    assert_eq!(error.kind, ModuleErrorKind::Rejected);
    assert!(error.message.starts_with("UR-5:"), "{}", error.message);
    assert!(error.message.contains("rx channel 0"), "{}", error.message);
    assert!(error.message.contains("UBX RX") && error.message.contains("basic_rx"), "{}", error.message);
    assert!(spec_calls(&device).contains(&"set_subdev_spec rx B:0".to_owned()), "the refusal is after the spec: {:?}", device.calls());
    assert_eq!(device.channels(ezsdr_radio_uhd::Dir::Rx), 1);
    let device = fake(FakeConfig { front_ends: vec!["UBX", "UBX"], serial: "ur05-board-tx".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,  "rx": [], "tx": ["0:B:basic_tx:0"] });
    let error = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).err().unwrap();
    assert!(error.message.starts_with("UR-5:") && error.message.contains("tx channel 0") && error.message.contains("UBX TX") && error.message.contains("basic_tx"), "{}", error.message);
    assert_eq!(device.channels(ezsdr_radio_uhd::Dir::Tx), 1);
}

/// A `FakeDevice` whose `front_end` answers the number `(0)` for every Basic/LF name it
/// would return: a front end whose getter disagrees with its setter (UR-5's check).
struct Mislabelled {
    inner: FakeDevice,
}

impl Device for Mislabelled {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        let name = self.inner.front_end(dir, chan)?;
        let basic = ["BasicRX", "LFRX", "BasicTX", "LFTX"].iter().any(|base| name.starts_with(base));
        if basic {
            let base = name.split_once(" (").map_or(name.as_str(), |(base, _)| base);
            return Ok(format!("{base} (0)"));
        }
        Ok(name)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.inner.apply(d, c, s)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
}

#[test]
fn ur_05_a_basic_front_end_s_number_is_checked() {
    // UR-5, H5: a Basic/LF front end is checked down to the number its name carries; the
    // faithful fake always agrees, so a wrapper answers `(0)` for every claimed number — a
    // front end whose getter disagrees with its setter — and the check refuses it.
    let device = fake(FakeConfig { front_ends: vec!["BasicRX", "BasicRX"], serial: "ur05-fe-faithful".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,  "rx": ["0:B:basic_rx:1"], "tx": [] });
    let _ = UhdRadio::from_binding(&binding(selector.clone(), Some(x3xx())), device.clone()).unwrap();
    assert!(spec_calls(&device).contains(&"set_subdev_spec rx B:1".to_owned()), "{:?}", device.calls());
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Rx, 0).unwrap(), "BasicRX (1)");
    // The same layout through a device that answers `(0)`.
    let device: Arc<dyn Device> =
        Arc::new(Mislabelled { inner: FakeDevice::new(FakeConfig { front_ends: vec!["BasicRX", "BasicRX"], serial: "ur05-fe-mislabelled".to_owned(), ..FakeConfig::default() }) });
    let error = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device).err().unwrap();
    assert_eq!(error.kind, ModuleErrorKind::Rejected);
    assert!(error.message.contains("rx channel 0") && error.message.contains("BasicRX (0)") && error.message.contains("basic_rx"), "{}", error.message);
}

#[test]
fn ur_05_a_subdev_spec_the_device_refuses_is_refused_with_uhd_s_text() {
    // UR-5, H7, H9: a spec naming a pair the device has no front end for is the device's own
    // failure, with its text; a swallowed failure would leave the default map, whose name
    // matches the layout, and be accepted.
    let device = fake(FakeConfig { front_ends: vec!["UBX"], serial: "ur05-spec-fail-rx".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,  "rx": ["0:B:ubx:0"], "tx": [] });
    let error = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device).err().unwrap();
    assert_eq!(error.kind, ModuleErrorKind::Rejected);
    assert!(error.message.starts_with("UR-5:"), "{}", error.message);
    assert!(error.message.contains("uhd_usrp_set_rx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev B:0"), "{}", error.message);
    let device = fake(FakeConfig { front_ends: vec!["UBX"], serial: "ur05-spec-fail-tx".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,  "rx": [], "tx": ["0:B:ubx:0"] });
    let error = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device).err().unwrap();
    assert!(error.message.contains("uhd_usrp_set_tx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev B:0"), "{}", error.message);
}

#[test]
fn ur_05_each_channel_is_checked_at_its_own_device_channel() {
    // UR-5, H1, H5: each element is checked at its own device channel: a BasicRX with its
    // two front ends answers `(1)` at channel 0 and `(0)` at channel 1 when the spec says so.
    let device = fake(FakeConfig { front_ends: vec!["BasicRX"], serial: "ur05-each-channel".to_owned(), ..FakeConfig::default() });
    let selector = json!({ "args": ARGS,  "rx": ["0:A:basic_rx:1", "0:A:basic_rx:0"], "tx": [] });
    let _ = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
    assert!(spec_calls(&device).contains(&"set_subdev_spec rx A:1 A:0".to_owned()), "{:?}", device.calls());
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Rx, 0).unwrap(), "BasicRX (1)");
    assert_eq!(device.front_end(ezsdr_radio_uhd::Dir::Rx, 1).unwrap(), "BasicRX (0)");
}

// ------------------------------------------------ UR-5: one live instance per serial

#[test]
fn ur_05_a_second_binding_of_one_serial_is_refused() {
    // UR-5, H4: a second binding of one device would silently overwrite the first's subdev
    // spec, so one serial has one live instance; the refusal writes nothing to the device
    // (the serial is read before the spec), and dropping the first releases it.
    let serial = "DUP5-T2";
    let a = fake(FakeConfig { serial: serial.to_owned(), ..FakeConfig::default() });
    let first = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), a.clone()).unwrap();
    let before = a.calls().len();
    let error = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), a.clone()).err().unwrap();
    assert_eq!(error.kind, ModuleErrorKind::Rejected);
    assert!(error.message.starts_with("UR-5:") && error.message.contains(serial), "{}", error.message);
    assert_eq!(a.calls()[before..], ["serial"], "the refusal writes nothing to the device");
    // Another device of the same serial is refused too: the registry is keyed by the serial.
    let b = fake(FakeConfig { serial: serial.to_owned(), ..FakeConfig::default() });
    let error = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), b).err().unwrap();
    assert!(error.message.contains(serial), "{}", error.message);
    // The refusal does not release the first's registration.
    let c = fake(FakeConfig { serial: serial.to_owned(), ..FakeConfig::default() });
    assert!(UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), c).is_err(), "a refusal released the first's serial");
    drop(first);
    let d = fake(FakeConfig { serial: serial.to_owned(), ..FakeConfig::default() });
    assert!(UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), d).is_ok(), "the serial was not released on drop");
}

#[test]
fn ur_05_a_parallel_bind_of_one_serial_succeeds_once() {
    // UR-5 (Sol, major 3): the check and the insert are one lock, so two threads binding one
    // serial at the same time leave exactly one live instance; both results stay alive here,
    // so the loser's refusal cannot be a race with the winner's drop.
    let serial = "DUP5-T9";
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let device = fake(FakeConfig { serial: serial.to_owned(), ..FakeConfig::default() });
                barrier.wait();
                UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device)
            })
        })
        .collect();
    let results: Vec<Result<UhdRadio, ezsdr_kernel::module_api::ModuleError>> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1, "exactly one bind wins");
}

#[test]
fn ur_05_a_failed_binding_leaves_no_serial_registered() {
    // UR-5: the serial is read and registered before the master-clock check, and a binding
    // that then fails leaves no registration behind (the guard is dropped with the failure).
    let serial = "DUP5-T3";
    let a = fake(FakeConfig { serial: serial.to_owned(), master_clock_rate: 184_320_000, ..FakeConfig::default() });
    let error = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), a.clone()).err().unwrap();
    assert!(error.message.contains("needs a 200 MHz master clock"), "{}", error.message);
    assert!(!calls(&a, "serial").is_empty(), "the refused attempt read the serial");
    let b = fake(FakeConfig { serial: serial.to_owned(), ..FakeConfig::default() });
    assert!(UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), b).is_ok(), "the refused attempt left a registration");
}

/// A `FakeDevice` whose `serial` is the one call that fails: the rest of the device is
/// sound, so a swallowed serial error cannot hide behind a later call's (Sol, major 3).
struct SerialFails {
    inner: Arc<FakeDevice>,
}

impl Device for SerialFails {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.front_end(dir, chan)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        Err(ezsdr_radio_uhd::DeviceError::failed("fake: the serial read failed"))
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.inner.apply(d, c, s)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
}

#[test]
fn ur_05_a_serial_read_that_fails_refuses() {
    // UR-5: a serial that cannot be read is refused with the device's own reason, and the
    // refusal comes before any write — the sound device behind the wrapper sees nothing.
    let inner = fake(FakeConfig { serial: "DUP5-T4".to_owned(), ..FakeConfig::default() });
    let device: Arc<dyn Device> = Arc::new(SerialFails { inner: inner.clone() });
    let error = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device).err().unwrap();
    assert_eq!(error.kind, ModuleErrorKind::Rejected);
    assert!(error.message.starts_with("UR-5:"), "{}", error.message);
    assert!(error.message.contains("fake: the serial read failed"), "{}", error.message);
    assert!(inner.calls().is_empty(), "a write happened before the serial: {:?}", inner.calls());
    let healthy = fake(FakeConfig { serial: "DUP5-T4".to_owned(), ..FakeConfig::default() });
    assert!(UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), healthy).is_ok(), "the failed attempt left a registration");
}

#[test]
fn ur_05_a_radio_dropped_after_its_device_is_lost_releases_its_serial() {
    // UR-5: the registration holds the serial, not the device object's life; a drop after
    // the device is gone does not read it (UR-29's rule) and still releases the serial.
    let serial = "DUP5-T7";
    let a = fake(FakeConfig { serial: serial.to_owned(), faults: vec![FakeFault::Lost(Wall::from_millis(10))], ..FakeConfig::default() });
    let first = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), a.clone()).unwrap();
    a.host().sleep(Wall::from_millis(20));
    assert!(a.serial().is_err(), "the device is gone");
    drop(first);
    let b = fake(FakeConfig { serial: serial.to_owned(), ..FakeConfig::default() });
    assert!(UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), b).is_ok(), "the serial was not released");
}

#[test]
fn ur_05_two_default_fakes_bind_together() {
    // UR-33, UR-5: the default serial is one per `FakeDevice`, so the parallel tests of one
    // binary never bind one another's device.
    let a = fake(FakeConfig::default());
    let b = fake(FakeConfig::default());
    assert_ne!(a.serial().unwrap(), b.serial().unwrap());
    let r1 = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), a).unwrap();
    let r2 = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), b).unwrap();
    drop((r1, r2));
}

// ------------------------------------------------ UR-13: the rate set at the first open

/// A `FakeDevice` whose `rates` answer is replaced per direction after the inner call —
/// so its call is still recorded and its channel check still runs (design 33 D5: BP0
/// measures the hardware; a device with another grid is a test wrapper, not a `FakeConfig`).
struct RatesMisreported {
    inner: Arc<FakeDevice>,
    rx: Option<Vec<(f64, f64, f64)>>,
    tx: Option<Vec<(f64, f64, f64)>>,
    /// The direction whose `rates` read fails (UR-13's read-error refusal).
    failing: Option<ezsdr_radio_uhd::Dir>,
}

impl Device for RatesMisreported {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.front_end(dir, chan)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.inner.apply(d, c, s)
    }
    fn rates(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        if self.failing == Some(dir) {
            return Err(ezsdr_radio_uhd::DeviceError::failed("fake: the rates read failed"));
        }
        let answer = self.inner.rates(dir, chan)?;
        match dir {
            ezsdr_radio_uhd::Dir::Rx => Ok(self.rx.clone().unwrap_or(answer)),
            ezsdr_radio_uhd::Dir::Tx => Ok(self.tx.clone().unwrap_or(answer)),
        }
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
}

/// The profile's grid with one middle value moved off it: another device whose rates are
/// not the description's (UR-13).
fn off_grid() -> Vec<(f64, f64, f64)> {
    let mut grid = ezsdr_radio::x3xx::rates(200_000_000);
    let middle = grid.len() / 2;
    grid[middle] += 0.5;
    grid.into_iter().map(|rate| (rate, rate, 0.0)).collect()
}

#[test]
fn ur_13_arm_compares_the_rate_set_after_the_streamer_opens() {
    // UR-13, H3: the device's rates are compared once per direction, after its first
    // streamer opened — before an open the DDC's chain is not connected and UHD answers
    // `{1.0}` — and from_binding reads none; the Manifest keeps the answer.
    let dir = TempDir::new();
    let inner = fake(FakeConfig { rates_before_open: true, serial: "ur13-arm-reads".to_owned(), ..FakeConfig::default() });
    let device: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: None, tx: None, failing: None });
    let run = spec_run(&with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    running(&run);
    let log = inner.calls();
    let position = |call: &str| log.iter().position(|c| c == call).unwrap_or_else(|| panic!("{call} missing in {log:?}"));
    assert!(position("rx_open 0 [0]") < position("rates rx 0"), "{log:?}");
    assert!(position("tx_open 0 [0]") < position("rates tx 0"), "{log:?}");
    assert_eq!(calls(&inner, "rates rx 0"), ["rates rx 0"], "one comparison per direction");
    assert_eq!(calls(&inner, "rates tx 0"), ["rates tx 0"]);
    let first_open = ["rx_open 0 [0]", "tx_open 0 [0]"].iter().filter_map(|call| log.iter().position(|c| c == call)).min().unwrap();
    assert!(!log[..first_open].iter().any(|call| call.starts_with("rates")), "from_binding read rates: {log:?}");
    let manifest = run.finish();
    assert_eq!(section(&manifest, "device")["rates_checked"], json!({ "rx": true, "tx": true }));
}

#[test]
fn ur_13_arm_refuses_a_rate_set_the_description_does_not_have() {
    // UR-13: a rate set that is not the profile's — one middle value moved by 0.5 Hz, so
    // that ends and cardinality still match — is refused at `arm`, after the streamer
    // opened and was closed again by the cleanup.
    for tx in [false, true] {
        let dir = TempDir::new();
        let inner = fake(FakeConfig { serial: format!("ur13-arm-refuse-{tx}"), ..FakeConfig::default() });
        let device: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: (!tx).then(off_grid), tx: tx.then(off_grid), failing: None });
        let spec = if tx { with_tx(receive_spec(1, 1e6, 1e9, None), 1e6) } else { receive_spec(1, 1e6, 1e9, None) };
        let manifest = spec_run_ended(&spec, &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
        assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Arm, .. }), "tx={tx}: {:?}", manifest.termination.reason);
        let reason = failure(&manifest);
        assert!(reason.contains("UR-13") && reason.contains(if tx { "tx" } else { "rx" }) && reason.contains("rates"), "tx={tx}: {reason}");
        let log = inner.calls();
        let opened = log.iter().position(|c| c.starts_with(if tx { "tx_open" } else { "rx_open" })).expect("the streamer opened");
        let closed = log.iter().position(|c| c.starts_with("close_streams")).expect("the streamer was closed");
        assert!(opened < closed, "tx={tx}: {log:?}");
        assert_eq!(calls(&inner, if tx { "rates tx 0" } else { "rates rx 0" }).len(), 1, "{log:?}");
    }
}

#[test]
fn ur_13_arm_refuses_a_pre_chain_rate_set_after_the_open() {
    // UR-13: `{1.0}` after the streamer opened is not "not yet connected" — the chain is —
    // so it is refused like any other set that is not the profile's (INFERRED: BP0).
    let dir = TempDir::new();
    let inner = fake(FakeConfig { rates_before_open: true, serial: "ur13-pre-chain".to_owned(), ..FakeConfig::default() });
    let device: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: Some(vec![(1.0, 1.0, 0.0)]), tx: None, failing: None });
    let manifest = spec_run_ended(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Arm, .. }), "{:?}", manifest.termination.reason);
    assert!(failure(&manifest).contains("UR-13"), "{}", failure(&manifest));
    let log = inner.calls();
    assert_eq!(calls(&inner, "rx_open 0 [0]").len(), 1, "the refusal is after the open: {log:?}");
    let closed = log.iter().position(|call| call == "rx_close 0").expect("the rate refusal closed the streamer");
    let all_closed = log.iter().position(|call| call == "close_streams").expect("run cleanup");
    assert!(closed < all_closed, "the failed rate check closes before cleanup: {log:?}");
}

#[test]
fn ur_13_a_direction_the_arm_never_opened_is_not_compared() {
    // UR-13: a direction whose streamer `arm` did not open — here the receive side of a
    // Session with no link — is not compared, whatever its device would answer; the
    // Manifest says so.
    let dir = TempDir::new();
    let inner = fake(FakeConfig { rates_before_open: true, serial: "ur13-no-link".to_owned(), ..FakeConfig::default() });
    let device: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: Some(vec![(1.0, 1.0, 0.0)]), tx: Some(vec![(1.0, 1.0, 0.0)]), failing: None });
    let mut doc = profile(&dir, json!({}), json!({}), true);
    doc["bindings"]["rec"].as_object_mut().unwrap().remove("feed");
    doc.as_object_mut().unwrap().remove("placements");
    doc["bindings"].as_object_mut().unwrap().remove("rec");
    let mut run = session(&doc, device);
    past_t0(&mut run, ms(1));
    running(&run);
    let manifest = run.finish();
    assert_eq!(section(&manifest, "device")["rates_checked"], json!({ "rx": false, "tx": false }));
    assert!(calls(&inner, "rates").is_empty(), "{:?}", inner.calls());
}

#[test]
fn ur_13_arm_takes_the_rate_set_in_any_order() {
    // UR-13: the comparison is a set: the device's order is UHD's, which the bench fixes
    // (BP0), and a reversed answer is the same set.
    let dir = TempDir::new();
    let inner = fake(FakeConfig { serial: "ur13-order".to_owned(), ..FakeConfig::default() });
    let mut reversed: Vec<(f64, f64, f64)> = ezsdr_radio::x3xx::rates(200_000_000).into_iter().rev().map(|rate| (rate, rate, 0.0)).collect();
    reversed.dedup();
    let device: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: Some(reversed), tx: None, failing: None });
    let run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    running(&run);
    let manifest = run.finish();
    assert_eq!(section(&manifest, "device")["rates_checked"], json!({ "rx": true, "tx": false }));
}

#[test]
fn ur_13_a_transmit_stream_is_compared_with_the_whole_device_grid() {
    // UR-13, KL-40: the transmit grid the profile declares starts at 1 MS/s, but the device still reports every rate it can make, so the
    // comparison for a transmit stream is with the profile's whole device grid (`description.rates`): the whole set, in any order, passes;
    // a device that reports only the 200 transmit rates is not the profile's rate set and is refused at `arm`.
    let dir = TempDir::new();
    let inner = fake(FakeConfig { serial: "ur13-tx-whole".to_owned(), ..FakeConfig::default() });
    let mut reversed: Vec<(f64, f64, f64)> = ezsdr_radio::x3xx::rates(200_000_000).into_iter().rev().map(|rate| (rate, rate, 0.0)).collect();
    reversed.dedup();
    let device: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: None, tx: Some(reversed), failing: None });
    let run = spec_run(&with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    running(&run);
    assert_eq!(section(&run.finish(), "device")["rates_checked"], json!({ "rx": true, "tx": true }));

    let dir = TempDir::new();
    let inner = fake(FakeConfig { serial: "ur13-tx-restricted".to_owned(), ..FakeConfig::default() });
    let restricted: Vec<(f64, f64, f64)> = ezsdr_radio::x3xx::tx_rates(200_000_000).into_iter().map(|rate| (rate, rate, 0.0)).collect();
    let device: Arc<dyn Device> = Arc::new(RatesMisreported { inner, rx: None, tx: Some(restricted), failing: None });
    let manifest = spec_run_ended(&with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Arm, .. }), "{:?}", manifest.termination.reason);
    assert!(failure(&manifest).contains("UR-13"));
}

// ------------------------------------------------ 1b-4c: stream 0 of the layout's keys

/// The selector of a two-channel-UBX layout, both directions (RM-28).
fn layout_selector() -> Json {
    json!({  "rx": ["0:A:ubx:0", "0:B:ubx:0"], "tx": ["0:A:ubx:0", "0:B:ubx:0"] })
}

#[test]
fn ur_12_a_layout_s_stream_zero_declares_its_clock_and_opens_its_channels() {
    // UR-12, RM-29: `prepare` and `arm` declare stream 0's clock at `usrp/rx/s0`, read its
    // channel set from the membership, and open — and configure — exactly that set, each
    // channel with its own numbered settings; the record waits for the first block.
    let direct = Direct::armed(
        FakeConfig::default(),
        &[("radio.rx.c0.stream", Scalar::from(-1)), ("radio.rx.c1.stream", Scalar::from(0)), ("radio.rx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap())],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    assert_eq!(calls(&direct.device, "rx_open"), ["rx_open 0 [1]"], "{:?}", direct.device.calls());
    let declared = direct.clocks.declared_sample_clocks();
    let entry = declared.iter().find(|handle| handle.stream().path() == "usrp/rx/s0").expect("the stream 0 clock declared");
    assert_eq!(entry.root_ticks_per_tick(), ezsdr_kernel::time::Rational::new(100, 1).unwrap());
    assert!(declared.iter().all(|handle| handle.stream().path() != "usrp/rx"), "no direction-node clock: {declared:?}");
    assert!(direct.clocks.sample_clock_records().is_empty(), "no block yet, so no registration");
    let ports: Vec<String> = direct.radio.instance().tree.ports.iter().map(|port| port.name.as_str().to_owned()).collect();
    assert_eq!(ports, ["rx0", "rx1"]);
    let log = direct.device.calls();
    assert!(log.iter().any(|call| call == "apply rx 1 rate=2000000 freq=1000000000 gain=0 antenna=RX2"), "{log:?}");
    assert!(!log.iter().any(|call| call.starts_with("apply rx 0 ")), "c0 is in no stream: {log:?}");
    assert!(!log.iter().any(|call| call == "rx_open 0 []"), "{log:?}");
    let _ = direct.finish();
}

#[test]
fn ur_12_a_layout_s_stream_zero_applies_each_channel_s_own_settings() {
    // UR-12, UR-30: one stream's channels get their own settings — the stream's rate, each
    // channel's frequency, gain and antenna (design 33 D16) — under the concrete keys.
    let direct = Direct::armed(
        FakeConfig::default(),
        &[
            ("radio.rx.c0.stream", Scalar::from(0)),
            ("radio.rx.c1.stream", Scalar::from(0)),
            ("radio.rx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap()),
            ("radio.rx.c0.frequency_hz", Scalar::try_from(1e9).unwrap()),
            ("radio.rx.c0.gain_db", Scalar::from(5)),
            ("radio.rx.c0.antenna", Scalar::from("RX2")),
            ("radio.rx.c1.frequency_hz", Scalar::try_from(2.4e9).unwrap()),
            ("radio.rx.c1.gain_db", Scalar::try_from(7.5).unwrap()),
            ("radio.rx.c1.antenna", Scalar::from("TX/RX")),
        ],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    assert_eq!(calls(&direct.device, "rx_open"), ["rx_open 0 [0, 1]"]);
    assert_eq!(
        calls(&direct.device, "apply rx"),
        ["apply rx 0 rate=2000000 freq=1000000000 gain=5 antenna=RX2", "apply rx 1 rate=2000000 freq=2400000000 gain=7.5 antenna=TX/RX",],
        "{:?}",
        direct.device.calls()
    );
    let _ = direct.finish();
}

#[test]
fn ur_13_a_layout_s_transmit_stream_zero_opens_its_channel_list() {
    // UR-13, RM-29: the transmit side is the same: its streamer's channel list, its own
    // settings, and its clock registered at `arm` on the lattice (RM-25).
    let direct = Direct::armed(
        FakeConfig::default(),
        &[
            ("radio.tx.c0.stream", Scalar::from(-1)),
            ("radio.tx.c1.stream", Scalar::from(0)),
            ("radio.tx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap()),
            ("radio.tx.c1.frequency_hz", Scalar::try_from(2.4e9).unwrap()),
            ("radio.tx.c1.gain_db", Scalar::try_from(7.5).unwrap()),
            ("radio.tx.c1.antenna", Scalar::from("TX/RX")),
        ],
        Vec::new(),
        json!({  "rx": [], "tx": ["0:A:ubx:0", "0:B:ubx:0"] }),
    );
    assert_eq!(calls(&direct.device, "tx_open"), ["tx_open 0 [1]"]);
    assert!(calls(&direct.device, "apply tx").contains(&"apply tx 1 rate=2000000 freq=2400000000 gain=7.5 antenna=TX/RX".to_owned()), "{:?}", direct.device.calls());
    assert!(!calls(&direct.device, "apply tx").iter().any(|call| call.starts_with("apply tx 0 ")), "{:?}", direct.device.calls());
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].stream.path(), "usrp/tx/s0");
    assert_eq!(records[0].root_ticks_per_tick, ezsdr_kernel::time::Rational::new(100, 1).unwrap());
    let _ = direct.finish();
}

#[test]
fn ur_12_a_layout_declares_and_starts_every_stream() {
    // UR-12, UR-13, UR-15, RM-29 (design 33 D4, D15): every declared stream declares its
    // clock, opens its own streamer on its own port's links, is started at T0 and publishes
    // its own channels on its own clock — stream 1 as stream 0.
    let (rx0, link0) = attached_reading("rx0");
    let (rx1, link1) = attached_reading("rx1");
    let mut direct = Direct::build(
        FakeConfig::default(),
        &[
            ("radio.rx.c0.stream", Scalar::from(0)),
            ("radio.rx.c1.stream", Scalar::from(1)),
            ("radio.rx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap()),
            ("radio.rx.s1.sample_rate_hz", Scalar::try_from(1e6).unwrap()),
            ("radio.tx.c0.stream", Scalar::from(0)),
            ("radio.tx.c1.stream", Scalar::from(1)),
            ("radio.tx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap()),
            ("radio.tx.s1.sample_rate_hz", Scalar::try_from(1e6).unwrap()),
        ],
        vec![rx0, rx1],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(100));
    let log = direct.device.calls();
    for call in ["rx_open 0 [0]", "rx_open 1 [1]", "tx_open 0 [0]", "tx_open 1 [1]"] {
        assert_eq!(log.iter().filter(|c| c.as_str() == call).count(), 1, "{call}: {log:?}");
    }
    let starts: Vec<&String> = log.iter().filter(|c| c.starts_with("rx_start")).collect();
    let at = |call: &str| call.rsplit(' ').next().unwrap().to_owned();
    let zero = starts.iter().find(|c| c.starts_with("rx_start 0 ")).expect("stream 0's start");
    let one = starts.iter().find(|c| c.starts_with("rx_start 1 ")).expect("stream 1's start");
    assert_eq!(at(zero), at(one), "both streams start at T0: {log:?}");
    let records = direct.clocks.sample_clock_records();
    let record = |path: &str| records.iter().find(|record| record.stream.path() == path).unwrap_or_else(|| panic!("{path}: {records:?}"));
    assert_eq!(record("usrp/rx/s0").root_ticks_per_tick, ezsdr_kernel::time::Rational::new(100, 1).unwrap());
    assert_eq!(record("usrp/rx/s1").root_ticks_per_tick, ezsdr_kernel::time::Rational::new(200, 1).unwrap());
    let block = |link: &Arc<dyn ezsdr_kernel::stream::DataLink>, path: &str| {
        let block = link.receive().unwrap_or_else(|| panic!("a block on {path}")).0;
        assert_eq!(block.header().channels, 1, "{path}");
        assert_eq!(block.header().first_sample_time.domain(), record(path).domain, "{path}");
    };
    block(&link0, "usrp/rx/s0");
    block(&link1, "usrp/rx/s1");
    let _ = direct.finish();
}

#[test]
fn ur_12_a_layout_s_stream_zero_with_no_channel_declares_no_clock() {
    // UR-12, RM-29: a stream no channel is in declares no SampleClock, opens no streamer
    // and configures nothing — `rx_open(0, [])` would be channel 0 on the device —, while
    // its node and port stay in the tree.
    let direct = Direct::armed(
        FakeConfig::default(),
        &[("radio.rx.c0.stream", Scalar::from(-1)), ("radio.rx.c1.stream", Scalar::from(-1))],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    let log = direct.device.calls();
    assert!(!log.iter().any(|call| call.starts_with("rx_open") || call.starts_with("apply rx")), "{log:?}");
    assert!(direct.clocks.declared_sample_clocks().iter().all(|handle| handle.stream().path() != "usrp/rx/s0"), "{:?}", direct.clocks.declared_sample_clocks());
    let children: Vec<String> = direct.radio.instance().tree.children.iter().map(|child| child.id.path().to_owned()).collect();
    assert!(children.contains(&"usrp/rx/s0".to_owned()), "{children:?}");
    let _ = direct.finish();
}

#[test]
fn ur_12_a_layout_a_rate_the_device_does_not_apply_is_refused() {
    // UR-12: the stream's rate is read back per channel as ever; a device that applies
    // another rate halts the configuration before `arm`.
    let device = fake(FakeConfig { faults: vec![FakeFault::WrongRate { claimed: 2e6, applied: 1.9e6, nth: 0 }], serial: "ur12-layout-wrong-rate".to_owned(), ..FakeConfig::default() });
    let mut radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).unwrap();
    let constraints = [("radio.rx.c0.stream", Scalar::from(-1)), ("radio.rx.c1.stream", Scalar::from(0)), ("radio.rx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap())];
    let error = direct_prepare(&mut radio, device.clone(), ExecutionClass::HardwareInLoop, Vec::new(), &constraints).unwrap_err();
    assert!(error.message.contains("UR-12: the device applied") && error.message.contains("1900000") && error.message.contains("2000000"), "{}", error.message);
    assert!(calls(&device, "rx_open").is_empty(), "{:?}", device.calls());
}

#[test]
fn ur_12_a_layout_an_antenna_read_back_that_differs_is_refused() {
    // UR-12, D16: the antenna is read back after an untimed change and compared; a front
    // end whose getter disagrees halts the configuration (the record keeps both values).
    let device = fake(FakeConfig { faults: vec![FakeFault::WrongAntenna { claimed: "TX/RX", applied: "RX2" }], serial: "ur12-layout-antenna".to_owned(), ..FakeConfig::default() });
    let mut radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).unwrap();
    let constraints = [("radio.rx.c0.stream", Scalar::from(-1)), ("radio.rx.c1.stream", Scalar::from(0)), ("radio.rx.c1.antenna", Scalar::from("TX/RX"))];
    let error = direct_prepare(&mut radio, device.clone(), ExecutionClass::HardwareInLoop, Vec::new(), &constraints).unwrap_err();
    assert!(error.message.contains("UR-12") && error.message.contains("TX/RX") && error.message.contains("RX2"), "{}", error.message);
    assert!(error.message.contains("radio.rx.c1"), "{}", error.message);
    assert!(calls(&device, "rx_open").is_empty(), "{:?}", device.calls());
}

// ------------------------------------------------ 1b-4c: the cut, the floor and the targets

// `ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// `ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// `ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// ------------------------------------------------ 1b-4c: destinations, judgement, manifest

#[test]
fn ur_26_a_device_stop_stops_every_layout_stream() {
    // UR-26, D14: `Stop` of the instance's node stops every stream — a burst held for a
    // later instant is cancelled with its reason, and the receive stream ends. Both
    // book before the Run starts (rule (1)), so the burst is held when the Stop is booked
    // however long the test thread waited.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let (bytes, waveform) = waveform_of(&tone(2_000));
    let target = serde_json::to_value(resource("radio/tx/s0")).unwrap();
    let mut spec = with_tx(receive_spec(1, 1e6, 1e9, None), 1e6);
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 100_000 }, "action": { "kind": "tx_burst", "target": target,
            "waveform": waveform, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 50_000 }, "action": { "kind": "stop", "target": serde_json::to_value(resource("radio")).unwrap() } }
    ]);
    let manifest = finish_spec(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(waveform.hash.clone(), bytes)])), ms(150));
    let log = device.calls();
    assert!(tx_experiment_sends(&device, "tx_send ").is_empty(), "the cancelled burst never reached the device: {log:?}");
    assert!(log.iter().any(|call| call == "rx_stop 0 now"), "the receive stream ends: {log:?}");
    let records = &manifest.clocks.sample_clocks;
    let rx: Vec<_> = records.iter().filter(|record| record.stream == ResourceId::parse("usrp/rx/s0").unwrap()).collect();
    assert!(rx.first().is_some_and(|record| record.ended_at.is_some()), "the receive clock ends: {records:?}");
    let tx: Vec<_> = records.iter().filter(|record| record.stream == ResourceId::parse("usrp/tx/s0").unwrap()).collect();
    assert!(tx.iter().all(|record| record.ended_at.is_none()), "the Stop ends no transmit clock: {records:?}");
    let events = events_of(&manifest, "radio.COMMAND_REJECTED");
    assert!(events.iter().all(|event| event.payload["kind"] != "tx_burst"), "Stop reports no held-burst cancellation: {events:?}");
    assert!(events.iter().all(|event| !event.payload["reason"].as_str().unwrap_or("").contains("cancelled by Stop")), "{events:?}");
    let rejected: Vec<_> = section(&manifest, "rejected").as_array().unwrap().iter().filter(|row| row["kind"] == "tx_burst").collect();
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert_eq!(rejected[0]["reason"], "UR-26: cancelled by Stop");
    assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
}

#[test]
fn ur_26_d14_a_layout_instance_names_its_streams() {
    // RM-12, RM-29, D14: under the layout the instance's node and its streams' nodes
    // (`<id>/rx/s<k>`) are the targets; a stream node's `Stop` stops that stream alone (note 47
    // stage 2: no Run-time update names a stream).
    let (bytes, waveform) = waveform_of(&tone(2_000));
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes)]);
    let mut direct =
        Direct::build_with(FakeConfig::default(), &[("radio.tx.c0.stream", Scalar::from(0))], vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)], layout_selector(), inputs);
    direct.settle(Wall::from_millis(30));
    // 1. A burst on the transmit stream's node is taken and plays.
    let record = direct.clocks.sample_clock_records().into_iter().find(|record| record.stream.path() == "usrp/tx/s0").expect("the transmit clock");
    let k = (direct.now() + ms(30) - record.origin.ticks_in(record.root).unwrap()) / record.root_ticks_per_tick.num() as i64;
    direct.burst_at("usrp/tx/s0", &waveform, false, TimePoint::new(record.domain, k));
    direct.settle(Wall::from_millis(60));
    assert!(!tx_experiment_sends(&direct.device, "tx_send").is_empty(), "the burst plays: {:?}", direct.device.calls());
    // 2. `Stop` of a stream's node stops that stream alone: the receive side is cut, and
    //    the transmit side keeps its clock and its held burst, which still plays. The burst's
    //    being held at the Stop's instant is the controlled-clock unit test in
    //    `src/provider/control.rs`; here the count before the booking says it was not
    //    cancelled, whether or not a delayed test thread had already let it play.
    let before_held = tx_sends(&direct.device).len();
    let held = direct.now() + ms(30);
    let k = (held - record.origin.ticks_in(record.root).unwrap()) / record.root_ticks_per_tick.num() as i64;
    direct.burst_at("usrp/tx/s0", &waveform, false, TimePoint::new(record.domain, k));
    direct.settle(Wall::from_millis(5));
    let records = direct.clocks.sample_clock_records().len();
    direct.stop_at("usrp/rx/s0");
    direct.settle(Wall::from_millis(120));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_stop 0 now").len(), 1, "{log:?}");
    assert_eq!(direct.clocks.sample_clock_records().len(), records, "{log:?}");
    let cancelled: Vec<String> = direct.of("radio.COMMAND_REJECTED").iter().map(|event| event.payload["reason"].as_str().unwrap_or_default().to_owned()).collect();
    assert!(cancelled.iter().all(|reason| !reason.contains("cancelled by Stop")), "a receive stream's Stop cancels no transmit burst: {cancelled:?}");
    assert!(tx_sends(&direct.device).len() > before_held, "the held burst still plays: {log:?}");
    let stopped = direct.clocks.sample_clock_records();
    let rx: Vec<_> = stopped.iter().filter(|record| record.stream.path() == "usrp/rx/s0").collect();
    assert!(rx[0].ended_at.is_some(), "{stopped:?}");
    // 3. `start_rx` of the same node starts a new segment: a second receive clock.
    direct.start_rx_at("usrp/rx/s0", None);
    direct.settle(Wall::from_millis(200));
    let records = direct.clocks.sample_clock_records();
    let rx: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/rx/s0").collect();
    assert_eq!(rx.len(), 2, "a new receive clock: {records:?}");
    assert!(rx[1].ended_at.is_none(), "{records:?}");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty(), "{:?}", direct.device.calls());
    let _ = direct.finish();
}

#[test]
fn ur_26_d14_the_old_addresses_and_foreign_nodes_are_refused() {
    // UR-26, D14: under the layout the old `<id>/rx` and `<id>/tx` are no targets, and
    // neither is a node outside the instance's streams.
    use ezsdr_kernel::module_api::UpdateClass::HardwareTimed;
    let mut direct = Direct::build(FakeConfig::default(), &[("radio.tx.c0.stream", Scalar::from(0))], vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)], layout_selector());
    direct.settle(Wall::from_millis(30));
    let before = (direct.device.calls().len(), direct.clocks.sample_clock_records().len());
    let rejections = |direct: &mut Direct| -> Vec<Json> { direct.of("radio.COMMAND_REJECTED").into_iter().map(|event| event.payload).collect() };
    direct.update_at("usrp/rx", "radio.rx.c0.gain_db", Value::num(3.0).unwrap(), HardwareTimed, Some(direct.now() + ms(40)));
    direct.settle(Wall::from_millis(10));
    let rejected = rejections(&mut direct);
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert_eq!(rejected[0]["kind"], "update_parameter");
    assert_eq!(rejected[0]["reason"], "UR-26: the target is not this device");
    for node in ["usrp/rx", "usrp/tx", "usrp/gpio", "usrp/rx/s9"] {
        direct.stop_at(node);
    }
    direct.settle(Wall::from_millis(10));
    let rejected = rejections(&mut direct);
    assert_eq!(rejected.len(), 5, "{rejected:?}");
    assert!(rejected[1..].iter().all(|row| row["kind"] == "stop" && row["reason"] == "UR-26: the Stop target is not this device or its streams"), "{rejected:?}");
    for node in ["usrp", "usrp/tx/s0", "usrp/rx/s9"] {
        direct.start_rx_at(node, None);
    }
    direct.settle(Wall::from_millis(10));
    let rejected = rejections(&mut direct);
    assert_eq!(rejected.len(), 8, "{rejected:?}");
    assert!(rejected[5..].iter().all(|row| row["kind"] == "command" && row["reason"] == "UR-26: not supported by this Provider"), "{rejected:?}");
    let (bytes, waveform) = waveform_of(&tone(2_000));
    let _ = bytes;
    let record = direct.clocks.sample_clock_records().into_iter().find(|record| record.stream.path() == "usrp/tx/s0").unwrap();
    let at = TimePoint::new(record.domain, (direct.now() + ms(30) - record.origin.ticks_in(record.root).unwrap()) / record.root_ticks_per_tick.num() as i64);
    for node in ["usrp/tx", "usrp", "usrp/rx/s0", "usrp/tx/s9"] {
        direct.burst_at(node, &waveform, false, at);
    }
    direct.settle(Wall::from_millis(10));
    let rejected = rejections(&mut direct);
    assert_eq!(rejected.len(), 12, "{rejected:?}");
    assert!(rejected[8..].iter().all(|row| row["kind"] == "tx_burst" && row["reason"] == "UR-21: the target is not this device's transmit stream"), "{rejected:?}");
    direct.settle(Wall::from_millis(60));
    assert_eq!(direct.device.calls().len(), before.0, "a refusal changes nothing: {:?}", direct.device.calls());
    assert_eq!(direct.clocks.sample_clock_records().len(), before.1);
    let _ = direct.finish();
}

#[test]
fn ur_26_d14_a_channel_less_stream_keeps_the_mock_s_treatment() {
    // D14, RM-29: a stream node the layout holds no channel for: `start_rx` is accepted and
    // does nothing, a burst is refused for its missing channel — the same as MockRadio's
    // (MR-16's reason is its own; the kind is the target's).
    let (bytes, waveform) = waveform_of(&tone(2_000));
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes)]);
    let mut direct =
        Direct::build_with(FakeConfig::default(), &[("radio.tx.c0.stream", Scalar::from(0))], vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)], layout_selector(), inputs);
    direct.settle(Wall::from_millis(30));
    direct.stop_at("usrp/rx/s0");
    direct.settle(Wall::from_millis(80));
    let clocks = direct.clocks.sample_clock_records().len();
    direct.start_rx_at("usrp/rx/s1", None);
    direct.settle(Wall::from_millis(120));
    assert_eq!(direct.clocks.sample_clock_records().len(), clocks, "a channel-less start registers no clock");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty(), "a channel-less stream's start is accepted: {:?}", direct.device.calls());
    assert_eq!(calls(&direct.device, "rx_start").len(), 1, "no new start: {:?}", direct.device.calls());
    let timed: Vec<_> = direct.clocks.sample_clock_records().into_iter().filter(|record| record.stream.path() == "usrp/rx/s1").collect();
    assert!(timed.is_empty(), "no clock for a channel-less stream");
    let record = direct.clocks.sample_clock_records().into_iter().find(|record| record.stream.path() == "usrp/tx/s0").unwrap();
    let at = TimePoint::new(record.domain, (direct.now() + ms(30) - record.origin.ticks_in(record.root).unwrap()) / record.root_ticks_per_tick.num() as i64);
    direct.burst_at("usrp/tx/s1", &waveform, false, at);
    direct.settle(Wall::from_millis(5));
    let rejected: Vec<Json> = direct.of("radio.COMMAND_REJECTED").into_iter().map(|event| event.payload).collect();
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert_eq!(rejected[0]["kind"], "tx_burst");
    assert!(rejected[0]["reason"].as_str().unwrap().contains("no transmit channel"), "{rejected:?}");
    assert!(tx_experiment_sends(&direct.device, "tx_send").is_empty(), "the burst is not sent on stream 0: {:?}", direct.device.calls());
    let _ = direct.finish();
}

#[test]
fn ur_30_the_device_section_names_the_subdev_spec() {
    // UR-30: the Manifest's `device` section carries the markup each direction was set
    // with — null where none was — beside the describe() it already had.
    let device_section = |radio: &UhdRadio| section_of(radio.instance(), "device");
    let radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), fake(FakeConfig { serial: "ur30-spec-a".to_owned(), ..FakeConfig::default() })).unwrap();
    let device = device_section(&radio);
    assert_eq!(device["subdev_spec"]["rx"], "A:0 B:0");
    assert_eq!(device["subdev_spec"]["tx"], "A:0 B:0");
    assert_eq!(device["fake"], true);
    // The order is the layout's (RM-28), and the record is the string that was passed.
    let device2 = fake(FakeConfig { serial: "ur30-spec-b".to_owned(), ..FakeConfig::default() });
    let radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS, "rx": ["0:B:ubx:0", "0:A:ubx:0"] }), Some(x3xx())), device2.clone()).unwrap();
    assert_eq!(device_section(&radio)["subdev_spec"]["rx"], "B:0 A:0");
    assert!(calls(&device2, "set_subdev_spec").contains(&"set_subdev_spec rx B:0 A:0".to_owned()), "{:?}", device2.calls());
    // One element, and a direction with none.
    let radio =
        UhdRadio::from_binding(&binding(json!({ "args": ARGS, "rx": ["0:A:ubx:0"], "tx": [] }), Some(x3xx())), fake(FakeConfig { serial: "ur30-spec-c".to_owned(), ..FakeConfig::default() })).unwrap();
    let device = device_section(&radio);
    assert_eq!(device["subdev_spec"]["rx"], "A:0");
    assert!(device["subdev_spec"]["tx"].is_null(), "{device}");
    let radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS,  "rx": ["0:A:ubx:0"], "tx": [] }), Some(x3xx())), fake(FakeConfig { serial: "ur30-spec-d".to_owned(), ..FakeConfig::default() }))
        .unwrap();
    let device = device_section(&radio);
    assert_eq!(device["subdev_spec"]["rx"], "A:0");
    assert!(device["subdev_spec"]["tx"].is_null(), "keys: {device}");
}

#[test]
fn ur_30_a_named_selector_records_and_sets_the_unnamed_subdev_spec() {
    // RM-28: a name after `#` never reaches the device. The recorded spec is the string the device was given,
    // and the whole call log equals the plain selector's.
    let build = |serial: &str, selector: Json| {
        let device = fake(FakeConfig { serial: serial.to_owned(), ..FakeConfig::default() });
        let radio = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device.clone()).unwrap();
        (radio, device)
    };
    let (plain, plain_device) = build("ur30-named-plain", json!({ "args": ARGS, "rx": ["0:B:ubx:0", "0:A:ubx:0"], "tx": ["0:A:ubx:0", "0:B:ubx:0"] }));
    let (named, named_device) = build("ur30-named-named", json!({ "args": ARGS, "rx": ["0:B:ubx:0#in-b", "0:A:ubx:0#in-a"], "tx": ["0:A:ubx:0#out-a", "0:B:ubx:0"] }));
    let spec = |radio: &UhdRadio| section_of(radio.instance(), "device")["subdev_spec"].clone();
    assert_eq!(spec(&named), spec(&plain));
    assert_eq!(spec(&named)["rx"], "B:0 A:0");
    assert_eq!(spec(&named)["tx"], "A:0 B:0");
    let strip = |calls: Vec<String>| calls.into_iter().filter(|call| !call.starts_with("serial")).collect::<Vec<_>>();
    assert_eq!(strip(named_device.calls()), strip(plain_device.calls()), "the same device calls");
    assert!(calls(&named_device, "set_subdev_spec").contains(&"set_subdev_spec rx B:0 A:0".to_owned()), "{:?}", named_device.calls());
    assert!(!named_device.calls().iter().any(|call| call.contains('#')), "a name reached the device: {:?}", named_device.calls());
}

#[test]
fn ur_05_a_bad_selector_name_is_refused_before_the_device_is_touched() {
    let device = fake(FakeConfig { serial: "ur05-bad-name".to_owned(), ..FakeConfig::default() });
    for rx in [json!(["0:A:ubx:0#"]), json!(["0:A:ubx:0#7"]), json!(["0:A:ubx:0#a b"]), json!(["0:A:ubx:0#a", "0:B:ubx:0#a"])] {
        let error = UhdRadio::from_binding(&binding(json!({ "args": ARGS, "rx": rx.clone() }), Some(x3xx())), device.clone()).err().expect("refused");
        assert!(error.message.starts_with("UR-5: ") && error.message.contains("name"), "{rx}: {}", error.message);
    }
    assert!(!device.calls().iter().any(|call| call.starts_with("set_subdev_spec") || call.starts_with("serial")), "the device was touched: {:?}", device.calls());
}

#[test]
fn ur_30_the_device_section_records_the_serial() {
    // UR-30: the serial is the record that lets an operator tell two devices apart (H4).
    let device = fake(FakeConfig { serial: "F00BA2".to_owned(), ..FakeConfig::default() });
    let radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).unwrap();
    assert_eq!(section_of(radio.instance(), "device")["serial"], "F00BA2");
    assert_eq!(calls(&device, "serial"), ["serial"], "one read: {:?}", device.calls());
}

/// A whole-Run profile of the layout's keys (RM-29): its receive port is `rx0`, so the
/// placement and a Spec's feed name it.
fn layout_profile(dir: &TempDir) -> Json {
    profile(dir, json!({}), json!({}), false)
}

/// The layout's receive Spec: stream 0's channel 0 through the numbered keys (RM-27),
/// out of the `rx0` port.
fn layout_receive_spec(rate: f64, hz: f64, samples: i64) -> Json {
    receive_spec(1, rate, hz, Some(samples))
}

#[test]
fn ur_30_every_applied_key_is_declared_by_the_instance() {
    // UR-30 (Sol 6): every key an `applied` row records is one the instance declares — a
    // channel's setting under its channel key, a stream's rate under the stream's own key
    // (`radio.rx.s0.sample_rate_hz`), never a channel-numbered rate key.
    let mut direct = Direct::build(
        FakeConfig { serial: "ur30-applied-layout".to_owned(), ..FakeConfig::default() },
        &[("radio.rx.c0.frequency_hz", Scalar::try_from(1.1e9).unwrap())],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        json!({}),
    );
    direct.settle(Wall::from_millis(30));
    let instance = direct.finish();
    let applied = &section_of(&instance, "applied");
    let declared: std::collections::BTreeSet<String> = instance.tree.capabilities.keys().map(|key| key.as_str().to_owned()).collect();
    let rows: Vec<&Json> = applied.as_array().unwrap().iter().filter(|row| row.get("claimed").is_some()).collect();
    assert!(!rows.is_empty(), "the configuration was recorded");
    for row in &rows {
        let key = row["key"].as_str().unwrap();
        assert!(declared.contains(key), "the applied key {key} is not declared: {declared:?}");
    }
    assert!(rows.iter().any(|row| row["key"] == "radio.rx.s0.sample_rate_hz"), "the rate's row: {applied}");
    assert!(!rows.iter().any(|row| row["key"].as_str().unwrap().contains(".c0.sample_rate_hz")), "a rate is no channel's key: {applied}");
}

#[test]
fn ur_30_the_device_section_records_the_rate_check() {
    // UR-30, UR-13, item 14: the sealed Manifest's `device` section says per direction
    // whether its rates were compared — both fields, whatever the answer.
    let dir = TempDir::new();
    let doc = layout_profile(&dir);
    let manifest = captured(spec_run(&layout_receive_spec(1e6, 1e9, 1_000), &doc, fake(FakeConfig { serial: "ur30-rates-layout".to_owned(), ..FakeConfig::default() }), BTreeMap::new()));
    assert_eq!(section(&manifest, "device")["rates_checked"], json!({ "rx": true, "tx": false }), "failure: {}", failure(&manifest));
    // A Session with no receive link: the receive streamer is never opened, so nothing is
    // compared.
    let mut doc = profile(&dir, json!({}), json!({}), true);
    doc["placements"]["links"][0]["from"]["port"] = json!("rx0");
    doc["bindings"]["rec"].as_object_mut().unwrap().remove("feed");
    doc.as_object_mut().unwrap().remove("placements");
    doc["bindings"].as_object_mut().unwrap().remove("rec");
    let device = fake(FakeConfig { serial: "ur30-rates-linkless".to_owned(), ..FakeConfig::default() });
    let mut run = session(&doc, device.clone());
    past_t0(&mut run, ms(1));
    let manifest = run.finish();
    assert_eq!(section(&manifest, "device")["rates_checked"], json!({ "rx": false, "tx": false }));
    assert!(calls(&device, "rates").is_empty(), "{:?}", device.calls());
}

// ---------------------------------- 1b-4c: the review's cases (round 1 findings)

#[test]
fn ur_19_a_layout_receive_report_names_its_stream() {
    // UR-20, major 6: a receive report event comes from the stream's own node — with
    // `<id>/rx/s0` — and reaches the collector.
    let mut direct = Direct::build(
        FakeConfig { faults: vec![FakeFault::Overflow(Wall::from_millis(40))], ..FakeConfig::default() },
        &[],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(250));
    let overflows = direct.of("radio.RX_OVERFLOW");
    assert!(!overflows.is_empty(), "the overrun reached the collector: {:?}", direct.device.calls());
    let source = EventSource::Node { node: ResourceId::parse("usrp/rx/s0").unwrap() };
    assert!(overflows.iter().all(|event| event.source == source), "{:?}", overflows.iter().map(|event| format!("{:?}", event.source)).collect::<Vec<_>>());
    let _ = direct.finish();
}

#[test]
fn ur_28_a_layout_transmit_report_names_its_stream() {
    // UR-17, UR-28, major 6: a transmit asynchronous error comes from the transmit
    // stream's own node — `<id>/tx/s0` under the layout — and reaches the collector.
    let mut direct = Direct::build(
        FakeConfig { faults: vec![FakeFault::TxReport(Wall::from_millis(40), TxCode::Underflow)], ..FakeConfig::default() },
        &[("radio.tx.c0.stream", Scalar::from(0))],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(250));
    let underflows = direct.of("radio.TX_UNDERFLOW");
    assert!(!underflows.is_empty(), "the report reached the collector: {:?}", direct.device.calls());
    let source = EventSource::Node { node: ResourceId::parse("usrp/tx/s0").unwrap() };
    assert!(underflows.iter().all(|event| event.source == source), "{:?}", underflows.iter().map(|event| format!("{:?}", event.source)).collect::<Vec<_>>());
    let _ = direct.finish();
}

// `ur_25_a_transmit_refusal_from_no_stream_starts_no_clock` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).

#[test]
fn ur_05_cleanup_does_not_release_the_serial() {
    // UR-5, major 3: `cleanup` stops what runs; the serial is held while the instance
    // lives and released by its drop.
    let device = fake(FakeConfig { serial: "ur05-cleanup-holds".to_owned(), ..FakeConfig::default() });
    let mut radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).unwrap();
    assert!(UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).is_err(), "the serial is held");
    radio.cleanup();
    assert!(UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).is_err(), "cleanup alone releases nothing");
    drop(radio);
    assert!(UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).is_ok(), "the drop released it");
}

#[test]
fn ur_13_a_rate_read_that_fails_refuses() {
    // UR-13, major 5: a `rates` read that fails is a refusal with `UR-13:` — never
    // "no rates", which would let a device the profile cannot describe run.
    let dir = TempDir::new();
    let inner = fake(FakeConfig { serial: "ur13-read-fails".to_owned(), ..FakeConfig::default() });
    let device: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: None, tx: None, failing: Some(ezsdr_radio_uhd::Dir::Rx) });
    let manifest = spec_run_ended(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Arm, .. }), "{:?}", manifest.termination.reason);
    let reason = failure(&manifest);
    assert!(reason.contains("UR-13:") && reason.contains("rates read failed"), "{reason}");
    let log = inner.calls();
    let opened = log.iter().position(|call| call.starts_with("rx_open")).expect("the streamer opened");
    assert!(log.iter().position(|call| call.starts_with("close_streams")).is_some_and(|closed| opened < closed), "{log:?}");
}

#[test]
fn ur_26_an_unserved_command_is_refused_under_its_own_action_name() {
    // UR-26: a Command this Provider does not serve — `start_rx` with a parameter is none —
    // is refused, and its `rejected` row and event carry the action's kind `command`.
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.push(Action::Command {
        target: resource("usrp"),
        verb: Ident::parse("start_rx").unwrap(),
        params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::from(1))]),
        at: None,
    });
    direct.settle(Wall::from_millis(30));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert_eq!(rejected[0].payload["kind"], "command");
    assert_eq!(rejection_reason(&rejected), "UR-26: not supported by this Provider");
    let instance = direct.finish();
    let rows = &section_of(&instance, "rejected");
    assert!(rows.as_array().unwrap().iter().any(|row| row["kind"] == "command" && row["reason"] == "UR-26: not supported by this Provider"), "{rows}");
    assert!(rows.as_array().unwrap().iter().all(|row| row["kind"] != "peripheral_command"), "{rows}");
}

#[test]
fn ur_05_a_repeated_front_end_is_still_refused() {
    // UR-5, item 2's T7: with the prefix restriction gone, a layout naming one `(slot,
    // front end)` pair twice is refused, naming the position and the repeated pair.
    let device = fake(FakeConfig::default());
    for rx in [vec!["0:A:ubx:0", "0:A:ubx:0"], vec!["0:A:ubx:0", "0:B:ubx:0", "0:A:ubx:0"]] {
        let error = match UhdRadio::from_binding(&binding(json!({ "args": ARGS,  "rx": rx, "tx": [] }), Some(x3xx())), device.clone()) {
            Ok(_) => panic!("the repeated pair was accepted: {rx:?}"),
            Err(error) => error,
        };
        assert_eq!(error.kind, ModuleErrorKind::Rejected, "{rx:?}");
        assert!(error.message.contains("position") && error.message.contains("repeats"), "{rx:?}: {}", error.message);
    }
    // A Basic/LF board's two front ends are two pairs: accepted.
    let device = fake(FakeConfig { front_ends: vec!["BasicRX"], ..FakeConfig::default() });
    let radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS,  "rx": ["0:A:basic_rx:0", "0:A:basic_rx:1"], "tx": [] }), Some(x3xx())), device.clone()).unwrap();
    drop(radio);
    assert_eq!(calls(&device, "set_subdev_spec rx A:0 A:1").len(), 1, "{:?}", device.calls());
}

// ------------------------------------------------ 1b-4d-2: every declared stream

/// The constraints of two receive streams: c0 to s0 at 2 MS/s, c1 to s1 at 1 MS/s (S1).
fn rx_two() -> Vec<(&'static str, Scalar)> {
    vec![
        ("radio.rx.c0.stream", Scalar::from(0)),
        ("radio.rx.c1.stream", Scalar::from(1)),
        ("radio.rx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap()),
        ("radio.rx.s1.sample_rate_hz", Scalar::try_from(1e6).unwrap()),
    ]
}

/// The same for two transmit streams.
fn tx_two() -> Vec<(&'static str, Scalar)> {
    vec![
        ("radio.tx.c0.stream", Scalar::from(0)),
        ("radio.tx.c1.stream", Scalar::from(1)),
        ("radio.tx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap()),
        ("radio.tx.s1.sample_rate_hz", Scalar::try_from(1e6).unwrap()),
    ]
}

#[test]
fn ur_13_a_linkless_receive_stream_has_no_streamer() {
    // UR-13, RM-29 (S2): a receive stream whose port has no link opens no streamer, starts
    // nothing and declares no registered clock; stream 0 with its link is untouched.
    let mut direct = Direct::build(FakeConfig::default(), &rx_two(), vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)], layout_selector());
    direct.settle(Wall::from_millis(100));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_open 1").len(), 0, "{log:?}");
    assert_eq!(calls(&direct.device, "rx_start 1").len(), 0, "{log:?}");
    assert!(direct.clocks.sample_clock_records().iter().all(|record| record.stream.path() != "usrp/rx/s1"), "{:?}", direct.clocks.sample_clock_records());
    assert_eq!(calls(&direct.device, "rx_open 0 [0]"), ["rx_open 0 [0]"], "{log:?}");
    assert_eq!(calls(&direct.device, "rx_start 0").len(), 1, "{log:?}");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty());
    let _ = direct.finish();
}

#[test]
fn ur_26_a_stop_of_a_later_stream_stops_that_stream_alone() {
    // UR-26, RM-29 (S7): `Stop` of a stream's node stops that stream alone, `start_rx`
    // starts it again, and `<id>` stops both.
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &rx_two(), vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.stop_at("usrp/rx/s1");
    direct.settle(Wall::from_millis(120));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_stop 1 now").len(), 1, "{log:?}");
    assert_eq!(calls(&direct.device, "rx_stop 0").len(), 0, "{log:?}");
    let records = direct.clocks.sample_clock_records();
    assert!(records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").all(|record| record.ended_at.is_some()), "{records:?}");
    assert!(records.iter().find(|record| record.stream.path() == "usrp/rx/s0").unwrap().ended_at.is_none(), "{records:?}");
    let starts = calls(&direct.device, "rx_start 1").len();
    direct.start_rx_at("usrp/rx/s1", None);
    direct.settle(Wall::from_millis(200));
    assert_eq!(calls(&direct.device, "rx_start 1").len(), starts + 1, "`start_rx` starts the stream: {:?}", direct.device.calls());
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").count(), 2, "{records:?}");
    direct.stop_at("usrp");
    direct.settle(Wall::from_millis(120));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_stop 0 now").len(), 1, "the instance's stop reaches stream 0: {log:?}");
    assert_eq!(calls(&direct.device, "rx_stop 1 now").len(), 2, "and stream 1: {log:?}");
    let records = direct.clocks.sample_clock_records();
    assert!(records.iter().all(|record| record.ended_at.is_some()), "{records:?}");
    let _ = direct.finish();
}

/// An `rx_two` Session whose second start of stream 1 (`start_rx` after a Stop) is refused: the device applies 0.9 MS/s for the asked 1 MS/s.
fn refused_second_start_of_stream_1() -> (Direct, Arc<dyn ezsdr_kernel::stream::DataLink>) {
    let (r0, link0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig { faults: vec![FakeFault::WrongRate { claimed: 1e6, applied: 0.9e6, nth: 1 }], ..FakeConfig::default() }, &rx_two(), vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.stop_at("usrp/rx/s1");
    direct.settle(Wall::from_millis(60));
    while link0.receive().is_some() {}
    direct.start_rx_at("usrp/rx/s1", None);
    direct.settle(Wall::from_millis(300));
    (direct, link0)
}

#[test]
fn ur_25_a_refused_start_of_a_later_stream_halts_that_stream_alone() {
    // UR-25 (S8): a `start_rx` whose configure the device refuses halts its own stream; the other stream
    // keeps delivering. The stimulus is a Stop of stream 1 and its `start_rx` (note 47 stage 2).
    let (mut direct, link0) = refused_second_start_of_stream_1();
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert!(rejection_reason(&rejected).contains("UR-12: the device applied 900000 S/s"), "{rejected:?}");
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_start 1").len(), 1, "the refused start called no `rx_start`: {log:?}");
    let records = direct.clocks.sample_clock_records();
    let s1: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").collect();
    assert_eq!(s1.len(), 1, "the refused start registers no clock: {records:?}");
    assert!(s1[0].ended_at.is_some(), "{records:?}");
    assert!(records.iter().find(|record| record.stream.path() == "usrp/rx/s0").unwrap().ended_at.is_none(), "{records:?}");
    assert!(link0.receive().is_some(), "stream 0 still delivers after stream 1's refusal");
    let _ = direct.finish();
}

#[test]
fn ur_25_a_successful_start_clears_a_later_stream_s_halt() {
    // UR-25 (O2): a refusal halts stream 1 alone; the next `start_rx` that succeeds starts it again.
    let (mut direct, link0) = refused_second_start_of_stream_1();
    assert_eq!(direct.of("radio.COMMAND_REJECTED").len(), 1, "{:?}", direct.device.calls());
    let starts = calls(&direct.device, "rx_start 1").len();
    // RM-21: the halted stream is still streaming, so a start needs a `Stop` first.
    direct.stop_at("usrp/rx/s1");
    direct.settle(Wall::from_millis(60));
    direct.start_rx_at("usrp/rx/s1", None);
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_start 1").len(), starts + 1, "the halt is cleared: {log:?}");
    assert_eq!(direct.of("radio.COMMAND_REJECTED").len(), 1, "no second refusal: {log:?}");
    let records = direct.clocks.sample_clock_records();
    let s1: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").collect();
    assert_eq!(s1.len(), 2, "{records:?}");
    assert!(s1.iter().any(|record| record.ended_at.is_some()) && s1.iter().any(|record| record.ended_at.is_none()), "{records:?}");
    assert!(link0.receive().is_some(), "stream 0 delivers throughout");
    // RM-25: the refusal is booked on stream 1's own line, so its segment is left out of stream 1's plan (the first and the third segments remain).
    let instance = direct.finish();
    let plan = plan_row(&section_of(&instance, "timing"));
    assert_eq!(plan_stream(&plan, "rx", 1).len(), 2, "{plan}");
}

fn bench_link(latency_ms: u64) -> FakeConfig {
    FakeConfig { rx_latency: Wall::from_millis(latency_ms), rx_packet: Some(1_996), ..FakeConfig::default() }
}

#[test]
fn ur_25_a_start_booked_anywhere_in_a_long_receive_call_is_on_time() {
    // Review O, O-B2 (R-1): with a 168 ms block at 390 625 S/s, a Stop and `start_rx` booked at any phase of the receive
    // call in progress still switch before e₂; and with a 100-sample block, where the call in progress waits for a packet
    // (Review P, TG-3; the packet term itself is a margin here: design-notes §14).
    let cases = [(65_536, 1_996, 28), (100, 1_996, 1)];
    for (block_len, packet, step_ms, phase) in cases.into_iter().flat_map(|(b, k, s)| (0..6).map(move |p| (b, k, s, p))) {
        let port = attached(ezsdr_kernel::stream::BackPressure::DropOldest);
        let config = FakeConfig { rx_packet: Some(packet), ..bench_link(2) };
        let mut direct = Direct::build(config, &[("radio.rx.s0.sample_rate_hz", Scalar::try_from(390_625.0).unwrap())], vec![port], json!({ "block_len": block_len }));
        direct.settle(Wall::from_millis(200 + phase * step_ms));
        let booked = direct.now();
        direct.stop_at("usrp/rx/s0");
        direct.start_rx_at("usrp/rx/s0", None);
        direct.settle(Wall::from_millis(600));
        let late = direct.of("radio.LATE_COMMAND");
        let instance = direct.finish();
        let timing = &section_of(&instance, "timing");
        let switch = timing.as_array().unwrap().iter().rfind(|r| r["what"] == "rx_segment").cloned().unwrap_or_else(|| panic!("no switch: {timing}"));
        // UR-25's ready instant (spec 22, VH-4): the new origin is at least the booking plus the longer of a block and a
        // packet at the old rate, 3 ms and the start lead (Review Q, TG-Q3).
        let floor = ms(50) + block_len.max(packet) as i64 * 512 + ms(3);
        let origin = switch["origin"].as_i64().unwrap();
        assert!(origin - booked >= floor, "block {block_len} packet {packet} phase {phase}: booked {booked} {switch}");
        assert!(switch["at"].as_i64().unwrap() < origin, "block {block_len} packet {packet} phase {phase}: switched after e₂: {switch}");
        assert!(late.is_empty(), "block {block_len} packet {packet} phase {phase}: {late:?}");
    }
}

#[test]
fn ur_25_a_stopped_stream_ends_at_its_cut_and_its_late_tail_stays_off_the_new_clock() {
    // Review N, B1 and B2: on a 60 ms link a Stop then `start_rx` finds the old stream's tail still in flight when the new
    // clock begins. The stopped stream delivers every sample up to its cut however late they come (a Timeout after the cut
    // waits the quiet span before it ends the stream), and none of the tail is published on the new clock, whose first
    // block is not before the untimed stop.
    let (port, link) = attached_reading("rx0");
    let mut direct = Direct::build(bench_link(60), &[], vec![port], json!({}));
    direct.settle(Wall::from_millis(300));
    direct.stop_at("usrp/rx/s0");
    direct.start_rx_at("usrp/rx/s0", None);
    direct.settle(Wall::from_millis(800));
    let clocks = direct.clocks.sample_clock_records();
    let instance = direct.finish();
    let timing = section_of(&instance, "timing");
    let stopped = timing.as_array().unwrap().iter().find(|row| row["what"] == "rx_stop_untimed").expect("the untimed stop")["at"].as_i64().unwrap();
    let headers: Vec<_> = std::iter::from_fn(|| link.receive()).map(|(block, _)| block.header().clone()).collect();
    let rx: Vec<_> = clocks.iter().filter(|record| record.stream.path() == "usrp/rx/s0").collect();
    assert_eq!(rx.len(), 2, "{rx:?} {timing}");
    let (old, new) = (rx[0], rx[1]);
    let n = old.root_ticks_per_tick.num() as i64;
    let cut = (old.ended_at.expect("the stopped clock ended").ticks_in(old.root).unwrap() - old.origin.ticks_in(old.root).unwrap()) / n;
    let delivered = headers.iter().filter(|header| header.first_sample_time.domain() == old.domain).map(|header| header.first_sample_time.ticks_in(old.domain).unwrap() + i64::from(header.len)).max();
    assert_eq!(delivered, Some(cut), "the stopped stream's samples end at its cut");
    let first = headers.iter().find(|header| header.first_sample_time.domain() == new.domain).expect("a block on the new clock");
    let first_tick = new.origin.ticks_in(new.root).unwrap() + first.first_sample_time.ticks_in(new.domain).unwrap() * new.root_ticks_per_tick.num() as i64;
    assert!(first_tick >= stopped, "the new clock's first block at {first_tick} precedes the old stream's stop at {stopped}: {timing}");
}

impl Direct {
    /// Starts the armed Provider at a T0 `lead` ticks ahead, without waiting for it (a queued start for a distant origin, #67).
    fn start_ahead(&mut self, lead: i64) -> i64 {
        let t0 = self.now() + lead;
        self.radio.start(Some(TimePoint::new(self.root, t0))).unwrap();
        t0
    }

    /// `Provider::stop(mode)` and cleanup, and the instance they leave.
    fn end(&mut self, mode: ezsdr_kernel::module_api::StopMode) -> ezsdr_kernel::module_api::ProviderInstance {
        self.radio.stop(mode).unwrap();
        self.radio.cleanup();
        self.radio.instance().clone()
    }
}

/// Every `rejected` row that is a join that timed out, or a leak of streamers (UR-16).
fn join_failures(instance: &ezsdr_kernel::module_api::ProviderInstance) -> Vec<Json> {
    section_of(instance, "rejected")
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row.get("leaked").is_some() || row["reason"].as_str().is_some_and(|reason| reason.contains("did not join")))
        .cloned()
        .collect()
}

/// The start the device holds at `t0`, reopened on a fresh streamer and read through `t0` + 300 ms: only timeouts mean the stop cancelled it and
/// the close did not hand it on (#67; UR-33).
fn assert_no_start_survives(device: &FakeDevice, t0: i64, case: &str) {
    device.rx_open(0, &[0]).unwrap();
    while device.time_now().unwrap() < t0 + ms(300) {
        assert_eq!(device.rx_recv(0, 2_000, Wall::from_millis(100), false), ezsdr_radio_uhd::device::RxRecv::Timeout, "{case}: the cancelled start delivered: {:?}", device.calls());
    }
}

#[test]
fn ur_26_provider_stop_before_a_distant_t0_stops_before_it_closes() {
    // UR-16, UR-17, UR-26 (#67): `Provider::stop` meets a start queued for T0 — 2.1 s ahead, past the 1 s join; 400 ms ahead, inside it — and
    // uhd-rx stops the stream untimed at once: it joins, nothing is left detached or leaked, the stop comes before the close, no sample of
    // the start is delivered, and what the device held is gone. With T0 passed, the stop is after it, as before. (The profile's start lead
    // is 2 s, so a T0 nearer than that is reached by waiting.)
    use ezsdr_kernel::module_api::StopMode;
    for (mode, lead, passed) in [(StopMode::Orderly, 2_100, false), (StopMode::Abort, 2_100, false), (StopMode::Orderly, 400, false), (StopMode::Orderly, 0, true)] {
        let case = format!("{mode:?}, T0 {lead} ms ahead, passed {passed}");
        let (port, link) = attached_reading("rx0");
        let mut direct = Direct::armed(FakeConfig::default(), &[], vec![port], json!({}));
        let t0 = direct.start_ahead(ms(2_100));
        direct.settle(Wall::from_millis(if passed { 2_150 } else { 2_100 - lead as u64 + 20 }));
        let instance = direct.end(mode);
        assert_eq!(join_failures(&instance), Vec::<Json>::new(), "{case}");
        let calls = direct.device.calls();
        let at = |prefix: &str| calls.iter().position(|call| call.starts_with(prefix)).unwrap_or_else(|| panic!("{case}: no {prefix} in {calls:?}"));
        // The open's own stop (UR-16) precedes the start; the one this case is about follows it.
        let start = at("rx_start");
        let stop = start + calls[start..].iter().position(|call| call.starts_with("rx_stop")).unwrap_or_else(|| panic!("{case}: no stop after the start in {calls:?}"));
        assert!(stop < at("close_streams"), "{case}: {calls:?}");
        let stops: Vec<_> = section_of(&instance, "timing").as_array().unwrap().iter().filter(|row| row["what"] == "rx_stop_untimed").map(|row| row["at"].as_i64().unwrap()).collect();
        assert_eq!(stops.len(), 1, "{case}");
        if passed {
            assert!(stops[0] >= t0, "{case}: {stops:?}");
        } else {
            assert!(stops[0] < t0, "{case}: the stop at {} is not before T0 {t0}", stops[0]);
            assert!(link.receive().is_none(), "{case}: a sample of the cancelled start");
            assert_no_start_survives(&direct.device, t0, &case);
        }
    }
}

#[test]
fn ur_25_a_stop_between_start_rx_and_its_origin_leaves_no_trace() {
    // UR-25, UR-26, RM-25 (#67; the Session twin of `hw_56_stop_between_starts`): streaming, `Stop`, `start_rx`, a second `Stop` before that start's
    // origin, a third `start_rx`. The second segment's start is stopped: it has no clock and no block, and the third segment's first block
    // is its own first sample. uhd-rx reads the second `Stop` between receive calls, and the call it is in when the `Stop` comes ends at
    // the origin (the profile's 50 ms lead is shorter than the 100 ms timeout), so the stop of a segment this near is not observably before
    // its origin here; that it is, is `ur_26_a_cut_at_a_queued_origin_stops_the_stream_at_once` and layer 3's.
    let (port, link) = attached_reading("rx0");
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![port], json!({}));
    direct.settle(Wall::from_millis(100));
    direct.stop_at("usrp/rx/s0");
    direct.start_rx_at("usrp/rx/s0", None);
    direct.settle(Wall::from_millis(4));
    direct.stop_at("usrp/rx/s0");
    direct.start_rx_at("usrp/rx/s0", None);
    direct.settle(Wall::from_millis(800));
    let clocks = direct.clocks.sample_clock_records();
    let device = direct.device.clone();
    let instance = direct.finish();
    let calls = device.calls();
    let starts: Vec<(usize, i64)> = calls.iter().enumerate().filter_map(|(i, call)| Some((i, call.strip_prefix("rx_start 0 ")?.parse().ok()?))).collect();
    assert_eq!(starts.len(), 3, "the first stream's start, the cancelled one and the third: {calls:?}");
    for pair in starts.windows(2) {
        assert!(calls[pair[0].0 + 1..pair[1].0].iter().any(|call| call == "rx_stop 0 now"), "a start follows one that was not stopped: {calls:?}");
    }
    let timing = section_of(&instance, "timing");
    let stops: Vec<i64> = timing.as_array().unwrap().iter().filter(|row| row["what"] == "rx_stop_untimed").map(|row| row["at"].as_i64().unwrap()).collect();
    assert!(stops.len() >= 3, "the second segment is stopped: {stops:?}");
    let rx: Vec<_> = clocks.iter().filter(|record| record.stream.path() == "usrp/rx/s0").collect();
    assert_eq!(rx.len(), 2, "no clock for the cancelled segment: {rx:?}");
    let third = rx[1];
    assert_eq!(third.origin.ticks_in(third.root).unwrap(), starts[2].1);
    let headers: Vec<_> = std::iter::from_fn(|| link.receive()).map(|(block, _)| block.header().clone()).collect();
    let first = headers.iter().find(|header| header.first_sample_time.domain() == third.domain).expect("a block on the third clock");
    assert_eq!(first.first_sample_time.ticks_in(third.domain).unwrap(), 0, "the third segment's first block is its first sample");
    assert!(!headers.iter().any(|header| header.first_sample_time.domain() != third.domain && header.first_sample_time.domain() != rx[0].domain), "a block on no clock of the plan");
}

#[test]
fn ur_29_a_loss_while_the_start_is_queued_makes_stop_stop_nothing_more() {
    // UR-29 (#67): the device is lost while the start is queued for T0 2.1 s ahead; `Provider::stop` then ends the Provider without a device
    // stop call after the loss. The loss is published once, and the join stays inside its bound.
    use ezsdr_kernel::module_api::StopMode;
    let (port, _link) = attached_reading("rx0");
    let mut direct = Direct::armed(FakeConfig { faults: vec![FakeFault::Lost(Wall::from_millis(200))], ..FakeConfig::default() }, &[], vec![port], json!({}));
    direct.start_ahead(ms(2_100));
    direct.settle(Wall::from_millis(400));
    let lost = direct.of(EventKind::DEVICE_LOST).len();
    let instance = direct.end(StopMode::Orderly);
    assert_eq!(lost, 1, "the loss is published once");
    assert_eq!(join_failures(&instance), Vec::<Json>::new());
    let calls = direct.device.calls();
    let marked = calls.iter().position(|call| call == "mark_lost").expect("the device was marked lost");
    assert!(!calls[marked..].iter().any(|call| call.starts_with("rx_stop")), "a stop call after the loss: {calls:?}");
}

#[test]
fn the_overflow_gap_adds_a_condition_with_the_other_direction_running() {
    // The overflow gap has a companion group with a 10 MS/s stream of the other direction running. It is an
    // observation only; the solo group's conditions and values are those of the solo conditions alone.
    let device = fake(one_obx());
    let plan = measurement::MeasurementPlan::rehearsal();
    let overflow_solo = measurement::measure_overflow_restart_gap(device.clone(), &plan).unwrap();
    assert!(overflow_solo.conditions.iter().all(|condition| !condition.id.ends_with("-with-tx")));
    let overflow = measurement::measure_overflow_restart_gap_with_tx(device, &plan).unwrap();
    let extra = &overflow.observations[0];
    assert_eq!((extra.name.as_str(), extra.condition.as_deref()), ("provider_overflow_restart_gap_with_tx", Some("rx-1msps-with-tx")));
    assert!(overflow.conditions.iter().any(|condition| Some(condition.id.as_str()) == extra.condition.as_deref()));
    assert_eq!(extra.result["trials"].as_array().unwrap().len(), plan.overflow_trials);
    assert_eq!(extra.result["observation_only"], true);
    assert_eq!(extra.result["trials"][0]["companion"]["observation_only"], true, "{:?}", extra.result);
}

/// A burst at `lead` root ticks past now, on `record`'s clock (UR-21).
fn burst_in(direct: &Direct, node: &str, record: &ezsdr_kernel::time::SampleClockRecord, waveform: &ezsdr_kernel::manifest::ArtifactRef, repeat: bool, lead: i64) {
    let origin = record.origin.ticks_in(record.root).unwrap();
    let k = (direct.now() + lead - origin) / record.root_ticks_per_tick.num() as i64;
    direct.burst_at(node, waveform, repeat, TimePoint::new(record.domain, k));
}

fn clock_of(direct: &Direct, path: &str) -> ezsdr_kernel::time::SampleClockRecord {
    direct.clocks.sample_clock_records().into_iter().find(|record| record.stream.path() == path).unwrap_or_else(|| panic!("no clock {path}"))
}

#[test]
fn ur_21_a_burst_targets_its_own_transmit_stream() {
    // UR-21, RM-29 (S11): a burst is booked on its target stream's clock and sent by that
    // stream's owner alone; a stream with no channel is refused.
    let (bytes, waveform) = waveform_of(&tone(1_000));
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes)]);
    let mut constraints = tx_two();
    constraints.extend([("radio.rx.c0.stream", Scalar::from(0))]);
    let (r0, _l0) = attached_reading("rx0");
    let mut direct = Direct::build_with(FakeConfig::default(), &constraints, vec![r0], layout_selector(), inputs);
    direct.settle(Wall::from_millis(50));
    let s0 = clock_of(&direct, "usrp/tx/s0");
    let s1 = clock_of(&direct, "usrp/tx/s1");
    assert_ne!(s0.domain, s1.domain, "two transmit streams, two clocks");
    burst_in(&direct, "usrp/tx/s0", &s0, &waveform, false, ms(40));
    direct.settle(Wall::from_millis(80));
    let first = tx_experiment_sends(&direct.device, "tx_send 0").len();
    let none = tx_experiment_sends(&direct.device, "tx_send 1").len();
    assert!(first > 0 && none == 0, "only stream 0 sends: {:?}", direct.device.calls());
    burst_in(&direct, "usrp/tx/s1", &s1, &waveform, false, ms(40));
    direct.settle(Wall::from_millis(80));
    assert_eq!(tx_experiment_sends(&direct.device, "tx_send 0").len(), first, "stream 0 is not sent stream 1's burst: {:?}", direct.device.calls());
    assert!(tx_experiment_sends(&direct.device, "tx_send 1").len() > none, "stream 1 sends its own: {:?}", direct.device.calls());
    // A late burst's TIME_ERROR comes from stream 1's node. The boundary is taken after the
    // earlier bursts' own verdicts, so a delayed booking's TIME_ERROR on stream 0 is not the
    // late burst's: only the events after the boundary are judged.
    let seen = direct.of("radio.TIME_ERROR").len();
    let k = (direct.now() - ms(20) - s1.origin.ticks_in(s1.root).unwrap()) / s1.root_ticks_per_tick.num() as i64;
    let past = TimePoint::new(s1.domain, k);
    direct.burst_at("usrp/tx/s1", &waveform, false, past);
    direct.settle(Wall::from_millis(60));
    let errors = direct.of("radio.TIME_ERROR");
    let errors = &errors[seen..];
    assert!(!errors.is_empty(), "{:?}", direct.device.calls());
    assert!(errors.iter().all(|event| event.source == EventSource::Node { node: ResourceId::parse("usrp/tx/s1").unwrap() }), "{errors:?}");
    let _ = direct.finish();

    // (b) a stream with no channel has no burst.
    let (bytes, waveform) = waveform_of(&tone(1_000));
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes)]);
    let mut constraints = tx_two();
    constraints[0] = ("radio.tx.c0.stream", Scalar::from(-1));
    constraints.extend([("radio.rx.c0.stream", Scalar::from(0))]);
    let (r0, _l0) = attached_reading("rx0");
    let mut direct = Direct::build_with(FakeConfig::default(), &constraints, vec![r0], layout_selector(), inputs);
    direct.settle(Wall::from_millis(50));
    let s1 = clock_of(&direct, "usrp/tx/s1");
    burst_in(&direct, "usrp/tx/s0", &s1, &waveform, false, ms(40));
    direct.settle(Wall::from_millis(60));
    let rejected: Vec<Json> = direct.of("radio.COMMAND_REJECTED").into_iter().map(|event| event.payload).collect();
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert_eq!(rejected[0]["kind"], "tx_burst");
    assert!(rejected[0]["reason"].as_str().unwrap().starts_with("UR-21:") && rejected[0]["reason"].as_str().unwrap().contains("no transmit channel"), "{rejected:?}");
    assert!(tx_experiment_sends(&direct.device, "tx_send").is_empty(), "{:?}", direct.device.calls());
    let _ = direct.finish();
}

#[test]
fn ur_26_two_receive_streams_end_at_their_own_stops() {
    // UR-26, RM-16 (S12a): a Stop of one receive stream ends it at its own stop, and the
    // other still runs to the Orderly stop later; the per-stream delivered floor is S12b/c's.
    let constraints = rx_two();
    let (r0, link0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig { rx_latency: Wall::from_millis(50), ..FakeConfig::default() }, &constraints, vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.settle(Wall::from_millis(400));
    direct.stop_at("usrp/rx/s1");
    // Wait for stream 1's owner to finish, but not for stream 0's next (50 ms late)
    // packet: the abort must see the delivered floor stream 1's finish did or did not clear.
    // The packet between them is pinned by `ur_26_a_stream_s_delivered_is_cleared_alone`,
    // `ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts` and
    // `ur_26_an_end_takes_delivered_from_each_segment_s_own_stream` (design note 37).
    // The state, not a time bound: stream 1's own Stop ended it before the abort, so its end is not the abort's cut.
    direct.until("stream 1's own Stop to end it", |direct| direct.clocks.sample_clock_records().iter().any(|record| record.stream.path() == "usrp/rx/s1" && record.ended_at.is_some()));
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Abort).unwrap();
    direct.radio.cleanup();
    let instance = direct.radio.instance();
    let plan = plan_row(&section_of(instance, "timing"));
    let rx = plan["rx"].as_array().unwrap();
    assert_eq!(rx.len(), 2, "{plan}");
    let (s0, s1) = (rx[0]["end"].as_i64().unwrap(), rx[1]["end"].as_i64().unwrap());
    let delivered: i64 = std::iter::from_fn(|| link0.receive().map(|(block, _)| block.header().len as i64)).sum();
    let origin = rx[0]["origin"].as_i64().unwrap();
    let n = rx[0]["ticks_per_sample"][0].as_i64().unwrap();
    assert_eq!(s0, origin + delivered * n, "the abort cuts stream 0 at its own delivered samples: {delivered} vs {s0} in {plan}");
    // Stream 1's own Stop cuts it at its own instant, and the abort follows it: the recorded
    // stop's instant is later than both cuts, whichever way a delayed test thread shifted them.
    let abort_at = section_of(instance, "timing").as_array().unwrap().iter().find(|row| row["what"] == "stop").expect("the abort's row")["at"].as_i64().unwrap();
    assert!(s1 > rx[1]["origin"].as_i64().unwrap(), "stream 1's own Stop does not end its segment: {s1} in {plan}");
    assert!(s0 <= abort_at && s1 <= abort_at, "a cut is later than the abort: {s0}, {s1} vs {abort_at} in {plan}");
    let records = direct.clocks.sample_clock_records();
    let ended = |path: &str| records.iter().filter(|record| record.stream.path() == path).filter_map(|record| record.ended_at.map(|at| at.ticks_in(record.root).unwrap())).max();
    let (end0, end1) = (ended("usrp/rx/s0").expect("stream 0's clock ends"), ended("usrp/rx/s1").expect("stream 1's clock ends"));
    assert_eq!((end0, end1), (s0, s1), "each clock ends at its own plan's cut");
}

#[test]
fn ur_26_every_stream_s_threads_are_joined_in_order() {
    // UR-26, UR-16 (S13): `Provider::stop` joins every transmit stream before every
    // receive stream, in stream order, and one that is left detached keeps the device's
    // streams open.
    let (bytes, waveform) = waveform_of(&tone(1_000));
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes)]);
    let gated = Arc::new(Gated::new(FakeConfig::default()));
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut constraints = rx_two();
    constraints.extend(tx_two());
    let mut direct = Direct::armed_on(gated.inner(), gated.clone(), &constraints, vec![r0, r1], layout_selector(), inputs);
    direct.start();
    direct.settle(Wall::from_millis(50));
    let ends = gated.ends();
    let s0 = clock_of(&direct, "usrp/tx/s0");
    let s1 = clock_of(&direct, "usrp/tx/s1");
    burst_in(&direct, "usrp/tx/s0", &s0, &waveform, true, ms(30));
    burst_in(&direct, "usrp/tx/s1", &s1, &waveform, true, ms(30));
    direct.settle(Wall::from_millis(100));
    struct Release<'a>(&'a Gated);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    gated.hold();
    let release = Release(&gated);
    gated.entered_n(&[("rx_recv", 2), ("tx_", 2)]);
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    drop(release);
    ended(&ends, &["uhd-tx-s0", "uhd-tx-s1", "uhd-rx-s0", "uhd-rx-s1"]);
    direct.radio.cleanup();
    let rejected = &section_of(direct.radio.instance(), "rejected");
    let threads: Vec<String> =
        rejected.as_array().unwrap().iter().filter(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("did not join"))).map(|row| row["thread"].as_str().unwrap().to_owned()).collect();
    assert_eq!(threads, ["uhd-tx-s0", "uhd-tx-s1", "uhd-rx-s0", "uhd-rx-s1"], "{rejected}");
    assert_eq!(calls(&gated.inner, "close_streams").len(), 0, "a detached thread keeps the streams open: {:?}", gated.inner.calls());
}

#[test]
fn ur_16_a_wedged_later_stream_leaves_the_others_joined() {
    // UR-16, RM-29 (S14): only the wedged stream is recorded detached; the other threads
    // join, and `close_streams` (the whole device's) is not called.
    let gated = Arc::new(Gated::only(FakeConfig::default(), 1));
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut constraints = rx_two();
    constraints.extend([("radio.tx.c0.stream", Scalar::from(0))]);
    let mut direct = Direct::armed_on(gated.inner(), gated.clone(), &constraints, vec![r0, r1], layout_selector(), BTreeMap::new());
    direct.start();
    direct.settle(Wall::from_millis(50));
    let ends = gated.ends();
    struct Release<'a>(&'a Gated);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    gated.hold();
    let release = Release(&gated);
    gated.entered(&["rx_recv"]);
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    drop(release);
    ended(&ends, &["uhd-rx-s1"]);
    direct.radio.cleanup();
    let rejected = &section_of(direct.radio.instance(), "rejected");
    let threads: Vec<String> =
        rejected.as_array().unwrap().iter().filter(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("did not join"))).map(|row| row["thread"].as_str().unwrap().to_owned()).collect();
    assert_eq!(threads, ["uhd-rx-s1"], "{rejected}");
    assert_eq!(calls(&gated.inner, "close_streams").len(), 0, "{:?}", gated.inner.calls());
    assert_eq!(rejected.as_array().unwrap().iter().filter(|row| row["leaked"] == "streamers").count(), 1, "{rejected}");
}

#[test]
fn ur_16_a_panicking_later_stream_ends_the_run() {
    // UR-16, RM-29 (S15): a panic in stream 1's owner is that thread's, once a
    // DEVICE_LOST, and the other stream ends at the loss.
    struct PanicOnOne {
        inner: Arc<FakeDevice>,
    }
    impl ezsdr_radio_uhd::Device for PanicOnOne {
        fn describe(&self) -> Json {
            self.inner.describe()
        }
        fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
            self.inner.fidelity()
        }
        fn master_clock_rate(&self) -> u64 {
            self.inner.master_clock_rate()
        }
        fn channels(&self, d: ezsdr_radio_uhd::Dir) -> usize {
            self.inner.channels(d)
        }
        fn front_end(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
            self.inner.front_end(d, c)
        }
        fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
            self.inner.serial()
        }
        fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
            self.inner.set_subdev_spec(d, m)
        }
        fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
            self.inner.rates(d, c)
        }
        fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
            self.inner.set_sources(c, t)
        }
        fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
            self.inner.set_time_zero(p)
        }
        fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
            self.inner.time_now()
        }
        fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
            self.inner.ref_locked()
        }
        fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
            self.inner.apply(d, c, s)
        }
        fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
            self.inner.rx_open(s, c)
        }
        fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
            self.inner.rx_start(s, at)
        }
        fn rx_packet_samples(&self, s: usize) -> usize {
            self.inner.rx_packet_samples(s)
        }
        fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
            self.inner.rx_stop(s)
        }
        fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
            assert_ne!(s, 1, "fake: rx_recv panics");
            self.inner.rx_recv(s, n, t, one_packet)
        }
        fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
            self.inner.tx_open(s, c)
        }
        fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
            self.inner.tx_send(s, b, at, sob, eob, t)
        }
        fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
            self.inner.tx_async(s, t)
        }
        fn rx_close(&self, s: usize) {
            self.inner.rx_close(s)
        }
        fn tx_close(&self, s: usize) {
            self.inner.tx_close(s)
        }
        fn close_streams(&self) {
            self.inner.close_streams()
        }
        fn mark_lost(&self) {
            self.inner.mark_lost()
        }
        fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
            self.inner.raise_thread_priority()
        }
        fn host(&self) -> ezsdr_radio_uhd::HostClock {
            self.inner.host()
        }
    }
    let inner = fake(FakeConfig::default());
    let tested: Arc<dyn ezsdr_radio_uhd::Device> = Arc::new(PanicOnOne { inner: inner.clone() });
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::armed_on(inner.clone(), tested, &rx_two(), vec![r0, r1], layout_selector(), BTreeMap::new());
    direct.start();
    direct.settle(Wall::from_millis(500));
    let lost = direct.of("ezsdr.DEVICE_LOST");
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert_eq!(lost[0].payload["message"], "a Provider thread panicked");
    direct.radio.cleanup();
    let rejected = &section_of(direct.radio.instance(), "rejected");
    assert!(rejected.as_array().unwrap().iter().any(|row| row["thread"] == "uhd-rx-s1" && row["reason"] == "panicked"), "{rejected}");
    assert_marked_lost_before_closed(&inner);
}

/// A `FakeDevice` whose untimed `apply` of one receive channel is held after it returns
/// (S16a): the owner is stopped between its configuration and its timed start.
struct HoldAfterApply {
    inner: Arc<FakeDevice>,
    pending: Mutex<Option<usize>>,
    entered: std::sync::atomic::AtomicBool,
    released: std::sync::atomic::AtomicBool,
    timed_out: std::sync::atomic::AtomicBool,
}

impl HoldAfterApply {
    fn new(inner: Arc<FakeDevice>) -> HoldAfterApply {
        HoldAfterApply { inner, pending: Mutex::new(None), entered: false.into(), released: false.into(), timed_out: false.into() }
    }

    /// The next receive `apply` of channel `chan` holds its owner after the call.
    fn hold(&self, chan: usize) {
        *self.pending.lock().unwrap() = Some(chan);
    }

    /// The owner is inside its configuration and still held.
    fn held(&self) -> bool {
        self.entered.load(std::sync::atomic::Ordering::SeqCst) && !self.released.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn entered(&self) -> bool {
        self.entered.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Lets the owner go on; `false` if its own real-time bound had already ended the hold (the setup failed, not the code under test).
    fn release(&self) -> bool {
        self.released.store(true, std::sync::atomic::Ordering::SeqCst);
        !self.timed_out.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The hold is a wait on the device's clock, which the owner (a participant) spends idle: a driver that polls on the same clock can see `entered` while the owner is held, and the clock
    /// keeps running. Its bound is real time.
    fn wait_for_release(&self) {
        let deadline = Instant::now() + Wall::from_secs(10);
        while !self.released.load(std::sync::atomic::Ordering::SeqCst) {
            if Instant::now() >= deadline {
                self.timed_out.store(true, std::sync::atomic::Ordering::SeqCst);
                return;
            }
            self.inner.host().sleep(Wall::from_millis(1));
        }
    }
}

impl ezsdr_radio_uhd::Device for HoldAfterApply {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, d: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(d)
    }
    fn front_end(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.front_end(d, c)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        let applied = self.inner.apply(d, c, s)?;
        let hold = d == ezsdr_radio_uhd::Dir::Rx && *self.pending.lock().unwrap() == Some(c);
        if hold {
            *self.pending.lock().unwrap() = None;
            self.entered.store(true, std::sync::atomic::Ordering::SeqCst);
            self.wait_for_release();
        }
        Ok(applied)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
}

#[test]
fn ur_26_a_receive_start_queued_by_the_stop_is_issued_and_stopped_at_once() {
    // UR-26 (S16a, #67): the owner does not re-read `Provider::stop` after its configuration;
    // the start it queued is issued, and the plan's end cuts the segment at its origin: no
    // block, and the untimed stop at once.
    let inner = fake(FakeConfig::default());
    let spy = Arc::new(HoldAfterApply::new(inner.clone()));
    let (r0, _l0) = attached_reading("rx0");
    let (r1, link1) = attached_reading("rx1");
    let mut direct = Direct::armed_on(inner.clone(), spy.clone(), &rx_two(), vec![r0, r1], layout_selector(), BTreeMap::new());
    direct.start();
    direct.settle(Wall::from_millis(50));
    while link1.receive().is_some() {}
    spy.hold(1);
    // The Stop of stream 1 cuts its segment, and the `start_rx` after it queues the start whose configuration is held.
    direct.stop_at("usrp/rx/s1");
    direct.start_rx_at("usrp/rx/s1", None);
    direct.until("the owner's configuration", |_| spy.entered());
    // The ordering the test is about: the Provider's Stop comes while the owner is inside its configuration, and the owner is still held when the Stop has returned.
    assert!(spy.held(), "the owner is held in its configuration when the Stop is issued");
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    assert!(spy.held(), "the owner was still held in its configuration while the Stop ran");
    while link1.receive().is_some() {}
    if !spy.release() {
        infra("the held owner's own real-time bound ended its hold before the test released it");
        panic!("the hold ended by its own bound: the Stop did not come during the configuration");
    }
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_start 1").len(), 2, "the owner still issues its timed start: {log:?}");
    assert!(link1.receive().is_none(), "the end at the origin publishes nothing: {log:?}");
    assert_eq!(calls(&direct.device, "rx_stop 1 now").len(), 2, "the old segment's cut and the queued start's: {log:?}");
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").count(), 1, "no clock is registered without a block: {records:?}");
    assert!(log.iter().any(|call| call.starts_with("apply rx 1 ")), "{log:?}");
    direct.radio.cleanup();
}

#[test]
fn ur_25_a_receive_cut_counts_its_own_stream_s_packet_size() {
    // UR-25 (S18): the ready gap after a cut counts the cut's stream's packet size, not
    // stream 0's (which has no streamer and answers 0). The cut is a `Stop` and the next
    // segment a `start_rx`.
    let mut constraints = rx_two();
    constraints[2] = ("radio.rx.s0.sample_rate_hz", Scalar::try_from(1e6).unwrap());
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig { rx_packet: Some(4_000), ..FakeConfig::default() }, &constraints, vec![r1], layout_selector());
    direct.settle(Wall::from_millis(100));
    assert_eq!((direct.device.rx_packet_samples(0), direct.device.rx_packet_samples(1)), (0, 4_000), "stream 0 has no streamer, stream 1's packet is 4000");
    direct.stop_at("usrp/rx/s1");
    direct.start_rx_at("usrp/rx/s1", None);
    direct.settle(Wall::from_millis(700));
    let instance = direct.finish();
    let plan = plan_row(&section_of(&instance, "timing"));
    let rx = plan["rx"].as_array().unwrap();
    assert_eq!(rx.len(), 2, "{plan}");
    let gap = rx[1]["origin"].as_i64().unwrap() - rx[0]["end"].as_i64().unwrap();
    assert!((11_400_000..11_400_400).contains(&gap), "4000 samples at 200 ticks + 3 ms allowance + 50 ms lead, rounded to the lattice: {gap} in {plan}");
}

#[test]
fn ur_26_a_start_rx_of_a_channel_less_linked_stream_starts_nothing() {
    // UR-26 (R21): `start_rx` of a linked stream that holds no channel is accepted and
    // carries out nothing.
    let mut constraints = rx_two();
    constraints[1] = ("radio.rx.c1.stream", Scalar::from(-1));
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &constraints, vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.start_rx_at("usrp/rx/s1", None);
    direct.settle(Wall::from_millis(120));
    assert_eq!(calls(&direct.device, "rx_start 1").len(), 0, "{:?}", direct.device.calls());
    assert!(direct.clocks.sample_clock_records().iter().all(|record| record.stream.path() != "usrp/rx/s1"), "{:?}", direct.clocks.sample_clock_records());
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty());
    let _ = direct.finish();
}

#[test]
fn ur_26_a_stop_of_a_later_transmit_stream_ends_only_its_burst() {
    // UR-26 (T2): `Stop` of a transmit stream ends that stream's burst, keeps its clock,
    // and leaves the other stream's burst alone; a later burst on the stopped stream plays.
    let (bytes, waveform) = waveform_of(&tone(1_000));
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes)]);
    let mut constraints = tx_two();
    constraints.extend([("radio.rx.c0.stream", Scalar::from(0))]);
    let (r0, _l0) = attached_reading("rx0");
    let mut direct = Direct::build_with(FakeConfig::default(), &constraints, vec![r0], layout_selector(), inputs);
    direct.settle(Wall::from_millis(50));
    let s0 = clock_of(&direct, "usrp/tx/s0");
    let s1 = clock_of(&direct, "usrp/tx/s1");
    burst_in(&direct, "usrp/tx/s0", &s0, &waveform, true, ms(30));
    burst_in(&direct, "usrp/tx/s1", &s1, &waveform, true, ms(30));
    direct.settle(Wall::from_millis(80));
    direct.stop_at("usrp/tx/s1");
    direct.settle(Wall::from_millis(120));
    let stopped = tx_experiment_sends(&direct.device, "tx_send 1").len();
    let sent0 = tx_experiment_sends(&direct.device, "tx_send 0").len();
    direct.settle(Wall::from_millis(100));
    assert_eq!(tx_experiment_sends(&direct.device, "tx_send 1").len(), stopped, "stream 1's burst ended: {:?}", direct.device.calls());
    assert!(tx_experiment_sends(&direct.device, "tx_send 0").len() > sent0, "stream 0's burst goes on: {:?}", direct.device.calls());
    let s1_now = clock_of(&direct, "usrp/tx/s1");
    assert!(s1_now.ended_at.is_none(), "the clock stays: {s1_now:?}");
    burst_in(&direct, "usrp/tx/s1", &s1_now, &waveform, false, ms(40));
    direct.settle(Wall::from_millis(100));
    assert!(tx_experiment_sends(&direct.device, "tx_send 1").len() > stopped, "a later burst plays on the kept clock: {:?}", direct.device.calls());
    let _ = direct.finish();
}

#[test]
fn ur_26_a_stopped_later_stream_waits_for_start_rx() {
    // UR-26 (O1): a `Stop`ped stream is off: only `start_rx` starts it.
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &rx_two(), vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.stop_at("usrp/rx/s1");
    direct.settle(Wall::from_millis(120));
    let starts = calls(&direct.device, "rx_start 1").len();
    direct.settle(Wall::from_millis(300));
    assert_eq!(calls(&direct.device, "rx_start 1").len(), starts, "an off stream starts on its own: {:?}", direct.device.calls());
    direct.start_rx_at("usrp/rx/s1", None);
    direct.settle(Wall::from_millis(200));
    assert_eq!(calls(&direct.device, "rx_start 1").len(), starts + 1, "`start_rx` starts it: {:?}", direct.device.calls());
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").count(), 2, "{records:?}");
    let _ = direct.finish();
}

#[test]
fn ur_36_every_restart_is_configured_with_the_connect_values() {
    // UR-36, note 47: the segment a `start_rx` begins is configured by its own command, the second restart as the first,
    // and with the values the Run started with (here 2 GHz), not the profile's defaults: values change only at a start.
    let mut direct = Direct::build(FakeConfig::default(), &[("radio.rx.c0.frequency_hz", Scalar::try_from(2.0e9).unwrap())], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)], json!({}));
    direct.settle(Wall::from_millis(100));
    assert_eq!(direct.device.freq_at(Dir::Rx, 0, direct.now()), 2.0e9, "the connect value");
    for restart in 1..=2 {
        let mark = direct.device.calls().len();
        direct.stop_at("usrp/rx/s0");
        direct.start_rx_at("usrp/rx/s0", None);
        direct.settle(Wall::from_millis(200));
        assert!(direct.device.calls()[mark..].iter().any(|call| call.starts_with("apply rx")), "restart {restart} left its segment unconfigured: {:?}", &direct.device.calls()[mark..]);
        assert_eq!(direct.device.freq_at(Dir::Rx, 0, direct.now()), 2.0e9, "restart {restart} keeps the connect value");
    }
    let _ = direct.finish();
}

/// A `FakeDevice` whose `rx_start` of stream 1 fails once, after stream 0's owner has made
/// its first receive call (P1): `start` refuses, and cleanup joins that owner first. With
/// `lost`, that failure is a lost device (P1a).
struct OnceFails {
    inner: Arc<FakeDevice>,
    recv0: std::sync::atomic::AtomicBool,
    recv0_calls: std::sync::atomic::AtomicUsize,
    lost: bool,
}

impl ezsdr_radio_uhd::Device for OnceFails {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, d: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(d)
    }
    fn front_end(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.front_end(d, c)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.inner.apply(d, c, s)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        if s == 1 {
            let mut waits = 0;
            while !self.recv0.load(std::sync::atomic::Ordering::SeqCst) && waits < 2_000 {
                self.inner.host().sleep(Wall::from_millis(1));
                waits += 1;
            }
            assert!(self.recv0.load(std::sync::atomic::Ordering::SeqCst), "stream 0's owner began before stream 1's start");
            return Err(ezsdr_radio_uhd::DeviceError { lost: self.lost, message: "test: stream 1's start fails".to_owned() });
        }
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        if s == 0 {
            self.recv0.store(true, std::sync::atomic::Ordering::SeqCst);
            self.recv0_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
}

#[test]
fn ur_15_a_failed_later_start_closes_after_the_owners_joined() {
    // UR-15 (P1): stream 1's `rx_start` fails after stream 0's owner began. `start` is
    // refused, and cleanup joins that owner before it closes the device's streams.
    let inner = fake(FakeConfig::default());
    let once = Arc::new(OnceFails { inner: inner.clone(), recv0: std::sync::atomic::AtomicBool::new(false), recv0_calls: std::sync::atomic::AtomicUsize::new(0), lost: false });
    let tested: Arc<dyn ezsdr_radio_uhd::Device> = once.clone();
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::armed_on(inner.clone(), tested, &rx_two(), vec![r0, r1], layout_selector(), BTreeMap::new());
    let t0 = direct.now() + ms(2_000) + 1;
    let error = direct.radio.start(Some(TimePoint::new(direct.root, t0))).unwrap_err();
    assert_eq!(error.kind, ModuleErrorKind::Rejected, "{error:?}");
    assert!(error.message.starts_with("UR-15:"), "{}", error.message);
    let before = inner.calls();
    assert!(!calls(&inner, "rx_stop 0 now").is_empty(), "`start` stopped the owner it began: {before:?}");
    assert!(!before.iter().any(|call| call == "close_streams"), "`start` closed nothing: {before:?}");
    direct.radio.cleanup();
    let log = inner.calls();
    assert!(once.recv0_calls.load(std::sync::atomic::Ordering::SeqCst) >= 1, "stream 0's owner ran: {log:?}");
    let close = log.iter().position(|call| call == "close_streams").expect("the close");
    assert!(!calls(&inner, "rx_stop 0 now").is_empty(), "stream 0's owner was stopped: {log:?}");
    let stop = log.iter().rposition(|call| call == "rx_stop 0 now").unwrap();
    assert!(stop < close, "the owner is joined before the streams close: {log:?}");
    let rejected = &section_of(direct.radio.instance(), "rejected");
    assert!(!rejected.as_array().unwrap().iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("did not join"))), "{rejected}");
}

#[test]
fn ur_29_a_lost_later_start_is_published_before_the_owners_are_stopped() {
    // UR-29, UR-15 (P1a): stream 1's `rx_start` fails as a lost device after stream 0's
    // owner began. `start` publishes the loss once and refuses; the owner already begun is
    // stopped without a device call after the loss, and cleanup closes nothing.
    let inner = fake(FakeConfig::default());
    let once = Arc::new(OnceFails { inner: inner.clone(), recv0: std::sync::atomic::AtomicBool::new(false), recv0_calls: std::sync::atomic::AtomicUsize::new(0), lost: true });
    let tested: Arc<dyn ezsdr_radio_uhd::Device> = once.clone();
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::armed_on(inner.clone(), tested, &rx_two(), vec![r0, r1], layout_selector(), BTreeMap::new());
    let t0 = direct.now() + ms(2_000) + 1;
    let error = direct.radio.start(Some(TimePoint::new(direct.root, t0))).unwrap_err();
    assert_eq!(error.kind, ModuleErrorKind::Rejected, "{error:?}");
    let lost = direct.of("ezsdr.DEVICE_LOST");
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert_eq!(lost[0].payload["message"], "test: stream 1's start fails", "{lost:?}");
    direct.radio.cleanup();
    let log = inner.calls();
    assert_marked_lost_before_closed(&inner);
    let marked = log.iter().position(|call| call == "mark_lost").expect("the loss is marked");
    assert!(!log[marked..].iter().any(|call| call.starts_with("rx_stop")), "no device call follows the loss: {log:?}");
    assert!(!log.iter().any(|call| call == "close_streams"), "a lost device's streamers are not closed: {log:?}");
}

// ------------------------------------------------ 1b-4d-3: the plan and the reports name their stream

#[test]
fn ur_30_the_plan_names_every_stream_at_the_stop() {
    // UR-30, design 33 D3's6: after `Provider::stop`, a 2×2 layout's plan holds the rows
    // for every stream, each naming its stream, the command that began the segment (`null`
    // when no command began it) and its logical channels in block order; every receive
    // stream's last segment ends at the stop instant's sample, and a transmit one has none —
    // a stop ends the bursts, not the clock.
    let mut constraints = rx_two();
    constraints.extend(tx_two());
    let mut direct = Direct::build(
        FakeConfig::default(),
        &constraints,
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest), attached_on("rx1", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(50));
    // Receive stream 0's second segment, so that a row's `sequence` is a command's.
    direct.stop_at("usrp/rx/s0");
    direct.start_rx_at("usrp/rx/s0", None);
    direct.settle(Wall::from_millis(250));
    let clocks = direct.clocks.clone();
    let instance = direct.finish();
    let timing = section_of(&instance, "timing");
    let plan = plan_row(&timing);
    let stop = timing.as_array().unwrap().iter().find(|row| row["what"] == "stop").expect("the stop row")["at"].as_i64().unwrap();
    for k in [0u64, 1] {
        let rx = plan_stream(&plan, "rx", k);
        assert!(!rx.is_empty(), "stream {k} has no receive row: {plan}");
        assert_eq!(rx[0]["sequence"], Json::Null, "a first segment has no command: {plan}");
        assert_eq!(rx[0]["channels"], json!([k]), "each stream holds its own channel: {plan}");
        let last = rx.last().unwrap();
        let (origin, num, den) = (last["origin"].as_i64().unwrap(), last["ticks_per_sample"][0].as_i64().unwrap(), last["ticks_per_sample"][1].as_i64().unwrap());
        let cut = (stop - origin) * den;
        let end = origin + ((cut + num - 1) / num * num + den - 1) / den;
        assert_eq!(last["end"].as_i64(), Some(end), "the stop cuts stream {k} at its sample: {plan}");
        let tx = plan_stream(&plan, "tx", k);
        assert_eq!(tx[0]["sequence"], Json::Null, "a first segment has no command: {plan}");
        assert_eq!(tx[0]["channels"], json!([k]), "each stream holds its own channel: {plan}");
        let last = tx.last().unwrap();
        assert_eq!(last["end"], Json::Null, "a stop ends the transmit bursts, not the clock: {plan}");
        assert_eq!(last["channels"], json!([k]), "each stream holds its own channel: {plan}");
    }
    let rx0 = plan_stream(&plan, "rx", 0);
    assert_eq!(rx0.len(), 2, "the restart's segment and the first: {plan}");
    assert!(rx0[1]["sequence"].is_u64(), "the restart's segment names its command: {plan}");
    let records = clocks.sample_clock_records();
    for k in [0, 1] {
        let path = format!("usrp/tx/s{k}");
        let open: Vec<_> = records.iter().filter(|record| record.stream.path() == path.as_str()).collect();
        assert_eq!(open.len(), 1, "stream {k}'s clocks: {records:?}");
        assert!(open.last().unwrap().ended_at.is_none(), "a stop keeps stream {k}'s transmit clock: {records:?}");
    }
}

// ------------------------------------------------ stage 3c-3a: layouts, streams and conditions

/// An OBX in slot A and a CBX in slot B: a two-channel layout of two boards (RM-28 admits the pair).
fn obx_and_cbx() -> FakeConfig {
    FakeConfig { front_ends: vec!["OBX", "CBX-120"], ..FakeConfig::default() }
}

fn two_channel_selector() -> Json {
    json!({ "rx": ["0:A:obx:0", "0:B:cbx:0"], "tx": ["0:A:obx:0", "0:B:cbx:0"] })
}

/// The rehearsal plan with two grid rates, so that the mixed-rate condition has a pair.
fn two_rate_plan() -> measurement::MeasurementPlan {
    let mut plan = measurement::MeasurementPlan::rehearsal();
    plan.rx_rates_hz = vec![390_625, 781_250];
    plan.tx_throughput_rates_hz = vec![1_000_000, 2_000_000];
    plan
}

fn names(group: &measurement::MeasurementGroup) -> Vec<String> {
    group.observations.iter().map(|row| row.name.clone()).collect()
}

/// Runs one direction's throughput group of an OBX+CBX layout, checks each stage of `tags` and the record built from it.
fn rehearse_two_channel_group(dir: &str, tags: &[&str], one_rate: bool, capacity: f64) {
    let device = fake(obx_and_cbx());
    let mut plan = two_rate_plan();
    if one_rate {
        plan.rx_rates_hz.truncate(1);
        plan.tx_throughput_rates_hz.truncate(1);
    }
    let (group, description, binding) = with_selector(two_channel_selector(), || {
        let group = if dir == "rx" { measurement::measure_rx_throughput(device.clone(), &plan) } else { measurement::measure_tx_throughput(device.clone(), &plan) }.unwrap();
        let tmp = TempDir::new();
        let binding = measurement::radio_binding(&bench_profile(&*device, &tmp, json!({}), bench_envelope(&*device), false)).unwrap();
        let description = measurement::description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
        (group, description, binding)
    });
    assert_eq!((description.rx.len(), description.tx.len()), (2, 2));
    let ids: Vec<String> = group.conditions.iter().map(|condition| condition.id.clone()).filter(|id| id.matches('-').count() >= 2).collect();
    assert_eq!(ids, tags.iter().map(|tag| format!("{dir}-{tag}")).collect::<Vec<_>>(), "{dir}: {:?}", names(&group));
    for tag in tags {
        let row = group.observations.iter().find(|row| row.name == format!("provider_{dir}_throughput_{dir}-{tag}")).unwrap_or_else(|| panic!("{dir} {tag}: {:?}", names(&group)));
        let streams = row.result["streams"].as_array().unwrap();
        assert!(!streams.is_empty() && streams.iter().all(|stream| stream["class"] == "Success"), "{dir} {tag}: {:?}", row.result);
        assert!(row.result["streamers_opened_in_order"].as_array().is_some_and(|opened| !opened.is_empty()), "{:?}", row.result);
        if dir == "tx" {
            assert!(
                streams.iter().all(|stream| stream["burst_blocks"].as_array().is_some_and(|blocks| blocks.len() == 1 && blocks[0].as_u64().is_some_and(|n| n >= 1))),
                "{dir} {tag}: {:?}",
                row.result
            );
        }
        assert!(row.result["nic_bytes"].as_object().is_some_and(|nics| nics.is_empty()), "no NIC named, none read");
    }
    assert!(group.observations.iter().any(|row| row.name == format!("provider_{dir}_throughput_capacity")), "{:?}", names(&group));
    // Every stage succeeded on the fake, so the capacity is the largest total over all the conditions' stages.
    assert!(matches!(&group.values[..], [ezsdr_radio::measurement::ValueRecord::Measured { worst: ezsdr_radio::measurement::Worst::Number(bytes), .. }] if *bytes == capacity), "{:?}", group.values);
    let record = measurement::build_record(ezsdr_radio::measurement::Source::Fake, &binding, &*device, &measurement::rehearsal_metadata(), group, &description).unwrap();
    let violations = ezsdr_radio::measurement::compare(&record, &description);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert_eq!(violations[0].key, "source");
}

#[test]
fn two_channel_receive_throughput_runs_the_grouped_split_and_mixed_conditions() {
    // UR-34, design 41 §8: each condition's streams are judged on their own, and the record's conditions and rows
    // pass the comparison apart from its source.
    rehearse_two_channel_group("rx", &["grouped-390625hz", "grouped-781250hz", "split-390625hz", "split-781250hz", "mixed-781250hz-390625hz"], false, 6_250_000.0);
}

#[test]
fn reopened_receive_throughput_repeats_only_the_single_channel_sweep() {
    let device = fake(obx_and_cbx());
    let mut plan = two_rate_plan();
    plan.rx_rates_hz.truncate(1);
    let group = with_selector(two_channel_selector(), || measurement::measure_reopened_rx_throughput(device, &plan).unwrap());
    assert_eq!(group.conditions.len(), 1, "the reopen pass omits multichannel families");
    assert_eq!(group.conditions[0].streams.len(), 1);
    assert_eq!(group.conditions[0].streams[0].channels, vec![0]);
}

#[test]
fn two_channel_transmit_throughput_runs_the_grouped_split_and_mixed_conditions() {
    // Transmit starts at 1 MS/s on `x3xx` (KL-40): its two rates are 1 and 2 MS/s, and the capacity is two channels of 4-byte samples at 2 MS/s.
    rehearse_two_channel_group("tx", &["grouped-1m", "grouped-2m", "split-1m", "split-2m", "mixed-2m-1m"], false, 16_000_000.0);
}

#[test]
fn the_duplex_pair_runs_the_two_measured_limits_together_once_and_decides_no_value() {
    // UR-34 (stage 3e): after both throughput values, each direction on the streams that held its own value, at once.
    let device = fake(obx_and_cbx());
    let plan = two_rate_plan();
    let mut plan = plan;
    plan.tx_throughput_rates_hz = vec![1_000_000];
    let (limits, pair) = with_selector(two_channel_selector(), || {
        let mut group = measurement::measure_rx_throughput(device.clone(), &plan).unwrap();
        group.append(measurement::measure_tx_throughput(device.clone(), &plan).unwrap()).unwrap();
        let limits = measurement::duplex_limits(&group).unwrap();
        (limits.clone(), measurement::measure_duplex_at_limits(device.clone(), &plan, Ok(limits), 4).unwrap())
    });
    // The two directions' values were held by different streams (receive up to 781 250, transmit only 1 000 000 on offer).
    assert_eq!(limits.rx, [measurement::StreamPlan { channels: vec![0, 1], rate_hz: 781_250 }], "{limits:?}");
    assert_eq!(limits.tx, [measurement::StreamPlan { channels: vec![0, 1], rate_hz: 1_000_000 }], "{limits:?}");
    assert!(pair.values.is_empty(), "the pair decides no value");
    assert_eq!(pair.conditions.iter().map(|condition| condition.id.as_str()).collect::<Vec<_>>(), [measurement::DUPLEX_PAIR]);
    assert_eq!(pair.conditions[0].streams.len(), limits.rx.len() + limits.tx.len());
    let row = &pair.observations[0];
    assert_eq!(row.name, "provider_duplex_at_the_measured_limits");
    assert_eq!((row.result["ran"].clone(), row.result["class"].as_str()), (json!(true), Some("Success")), "{:?}", row.result);
    assert_eq!((row.result["rx_limit_condition"].as_str(), row.result["tx_limit_condition"].as_str()), (Some(limits.rx_condition.as_str()), Some(limits.tx_condition.as_str())));
    assert_ne!(limits.rx_condition, limits.tx_condition);
    assert_eq!(
        (row.result["rx_streams"][0]["rate_hz"].as_u64(), row.result["tx_streams"][0]["rate_hz"].as_u64()),
        (Some(781_250), Some(1_000_000)),
        "each direction runs on its own limit: {:?}",
        row.result
    );
    let classes: Vec<_> = row.result["streams"].as_array().unwrap().iter().map(|stream| (stream["direction"].as_str().unwrap().to_owned(), stream["class"].as_str().unwrap().to_owned())).collect();
    assert!(classes.iter().any(|(direction, _)| direction == "rx") && classes.iter().any(|(direction, _)| direction == "tx") && classes.iter().all(|(_, class)| class == "Success"), "{classes:?}");
    assert!(row.result["overlap"]["overlap_root_ticks"].as_i64().is_some_and(|ticks| ticks > 0), "the two directions overlapped: {:?}", row.result["overlap"]);
    assert!(row.result["overlap"]["tx_flow"][0]["samples_accepted_over_samples_of_the_duration"].as_f64().is_some_and(|share| share >= 1.0), "{:?}", row.result["overlap"]);
}

fn stage_of(id: &'static str, family: &'static str, rx: &[(&[usize], u64)], tx: &[(&[usize], u64)]) -> measurement::Stage {
    let streams = |rows: &[(&[usize], u64)]| rows.iter().map(|(channels, rate_hz)| measurement::StreamPlan { channels: channels.to_vec(), rate_hz: *rate_hz }).collect();
    measurement::Stage { id: id.to_owned(), family, rx: streams(rx), tx: streams(tx) }
}

/// One stage on the OBX+CBX layout, with `fault` injected into its streams.
fn run_two_channel_stage(stage: &measurement::Stage, fault: measurement::InjectedFault) -> Result<measurement::StageResult, String> {
    let device = fake(obx_and_cbx());
    let plan = two_rate_plan();
    with_selector(two_channel_selector(), || measurement::with_injected_fault(fault, || measurement::run_stage(device.clone(), &plan, stage, 4)))
}

fn stream_classes(result: &measurement::StageResult) -> Vec<(String, u64, String)> {
    result.observation["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["direction"].as_str().unwrap().to_owned(), row["stream"].as_u64().unwrap(), row["class"].as_str().unwrap().to_owned()))
        .collect()
}

#[test]
fn a_starved_stream_fails_its_stage_and_the_others_still_show_success() {
    // The starved stream's own samples fall short of its SampleClock's interval; the total does not hide it.
    let stage = stage_of("rx-split-test", "split", &[(&[0], 390_625), (&[1], 390_625)], &[]);
    let result = run_two_channel_stage(&stage, measurement::InjectedFault { starve_rx: Some(1), ..Default::default() }).unwrap();
    assert!(result.left_open, "the stage reports the streamers it left open");
    assert_eq!(stream_classes(&result), [("rx".to_owned(), 0, "Success".to_owned()), ("rx".to_owned(), 1, "Incomplete".to_owned())], "{:?}", result.observation);
    // The Provider gave the starved stream's Run up with its streamers open: the stage fails, and the handle is marked wedged.
    assert_eq!(result.class, measurement::TrialClass::DeviceFailure, "{:?}", result.observation);
    assert_eq!(result.observation["streamers_left_open"], json!(["rx0", "rx1"]));
}

#[test]
fn a_partial_send_fails_the_transmit_stage() {
    let stage = stage_of("tx-split-test", "split", &[], &[(&[0], 1_000_000), (&[1], 1_000_000)]);
    let result = run_two_channel_stage(&stage, measurement::InjectedFault { short_tx: Some(0), ..Default::default() }).unwrap();
    assert_eq!(result.class, measurement::TrialClass::DeviceFailure, "{:?}", result.observation);
    let classes = stream_classes(&result);
    assert_eq!(classes[0].2, "DeviceFailure", "{classes:?}");
    assert_eq!(classes[1].2, "Success", "the other stream's own evidence is kept: {classes:?}");
    assert!(result.observation["short_send_rejections"].as_array().is_some_and(|rows| !rows.is_empty()), "{:?}", result.observation);
    assert!(result.observation["streams"][0]["short_sends"].as_u64().unwrap() > 0);
}

#[test]
fn a_burst_acknowledgement_that_never_comes_leaves_the_stream_incomplete() {
    let stage = stage_of("tx-split-test", "split", &[], &[(&[0], 1_000_000), (&[1], 1_000_000)]);
    let result = run_two_channel_stage(&stage, measurement::InjectedFault { swallow_ack: Some(1), ..Default::default() }).unwrap();
    let classes = stream_classes(&result);
    assert_eq!((classes[0].2.as_str(), classes[1].2.as_str()), ("Success", "Incomplete"), "{:?}", result.observation);
    assert_eq!(result.observation["streams"][1]["burst_acks_found"], 0);
}

#[test]
fn one_unacknowledged_channel_of_a_stream_leaves_it_incomplete() {
    // A stream of two channels is acknowledged once per channel; with channel 1's acknowledgement missing the stream shows none.
    let stage = stage_of("tx-grouped-test", "grouped", &[], &[(&[0, 1], 1_000_000)]);
    let result = run_two_channel_stage(&stage, measurement::InjectedFault { swallow_ack_channel: Some(1), ..Default::default() }).unwrap();
    assert_eq!(stream_classes(&result), [("tx".to_owned(), 0, "Incomplete".to_owned())], "{:?}", result.observation);
    assert_eq!((result.observation["streams"][0]["burst_acks_found"].as_u64(), result.observation["streams"][0]["burst_acks_expected"].as_u64()), (Some(1), Some(2)));
}

#[test]
fn a_duplex_stage_fails_when_the_companion_direction_does_not_flow() {
    // The receive stream of the pair starves, so the stage is not a success; its own rows say which stream fell short.
    let stage = stage_of("tx-duplex-test", "duplex", &[(&[0], 390_625)], &[(&[0], 1_000_000)]);
    let result = run_two_channel_stage(&stage, measurement::InjectedFault { starve_rx: Some(0), ..Default::default() }).unwrap();
    assert_ne!(result.class, measurement::TrialClass::Success, "{:?}", result.observation);
    let classes = stream_classes(&result);
    assert_eq!(classes.iter().find(|(direction, ..)| direction == "rx").unwrap().2, "Incomplete", "{classes:?}");
}

#[test]
fn a_stage_above_the_admission_ceiling_is_an_admission_rejection_not_a_failure() {
    // Two channels at 200 MS/s are 1 600 MB/s, past one 10 GbE link's 1 250 MB/s ceiling (RM-32): validate refuses the Run.
    let stage = stage_of("rx-grouped-test", "grouped", &[(&[0, 1], 200_000_000)], &[]);
    let result = run_two_channel_stage(&stage, Default::default()).unwrap();
    assert_eq!(result.class, measurement::TrialClass::AdmissionRejection, "{:?}", result.observation);
    assert!(result.observation["run_termination"].as_str().is_some_and(|text| !text.is_empty()));
}

#[test]
fn the_candidate_ceiling_admits_what_the_profiles_link_limit_refuses() {
    // 200 + 100 MS/s is 1 200 MB/s: over the profile's 1 000 MB/s, under the candidate's 1 250 MB/s.
    let device = fake(obx_and_cbx());
    let dir = TempDir::new();
    let spec = {
        let mut spec = receive_spec(1, 2e8, 1e9, None);
        let requires = &mut spec["resources"]["radio"]["requires"];
        requires["radio.rx.c1.stream"] = json!({ "kind": "eq", "value": 1 });
        requires["radio.rx.s1.sample_rate_hz"] = json!({ "kind": "eq", "value": 1e8 });
        requires["radio.rx.c1.frequency_hz"] = json!({ "kind": "eq", "value": 2.45e9 });
        spec
    };
    let mut doc = with_selector(two_channel_selector(), || bench_profile(&*device, &dir, json!({}), json!({}), false));
    let sink = doc["bindings"]["rec"].clone();
    doc["bindings"]["rec1"] = sink;
    doc["placements"]["links"].as_array_mut().unwrap().push(json!({
        "link": { "id": "ezsdr.link.host", "version": { "major": 1, "minor": 0, "patch": 0 } },
        "from": { "component": "radio", "port": "rx1" }, "to": { "component": "rec1", "port": "in" }
    }));
    let mut spec = spec;
    spec["outputs"].as_array_mut().unwrap().push(json!({ "id": "rec1", "kind": "sink.capture", "params": {},
        "feed": { "port": { "component": "radio", "port": "rx1" }, "policy": "drop_oldest", "capacity": 64 } }));
    let refused = spec_run_ended(&spec, &doc, device.clone(), BTreeMap::new());
    assert!(matches!(refused.termination.reason, Termination::Failed { stage: Stage::Validate, .. }), "the profile's own limit refuses it: {:?}", refused.termination.reason);
    let raised = ezsdr_radio::x3xx::performance(10);
    let candidate = ezsdr_radio::PerformanceEnvelope { rx_bytes_per_s: 1_250_000_000, tx_bytes_per_s: 1_250_000_000, wire_bytes_per_sample: raised.wire_bytes_per_sample };
    let admitted = Run::start_spec(&spec, &doc, assembly(&doc, device, BTreeMap::new(), |radio| radio.with_measurement_performance(candidate)), Declared::default()).unwrap();
    assert!(admitted.start_instant().is_some(), "the candidate ceiling admits it: {:?}", admitted.state());
    admitted.finish();
}

#[test]
fn the_selector_environment_names_only_the_layout_image_and_links() {
    // Stage 3c-3a: an object of rx, tx, image, links and link_gbps only; the rest is refused before a device is opened.
    let selector = parse_selector(r#"{"rx":["0:A:obx:0"],"tx":["0:A:obx:0"],"image":["xg"],"links":[2],"link_gbps":[10]}"#).unwrap();
    assert_eq!(selector["links"], json!([2]));
    for bad in ["{\"args\":\"addr=1\"}", "{\"rx\":[],\"id\":\"x\"}", "{\"tx\":[\"0:A:obx:0\"]}", "{\"rx\":[\"0:A:obx:0\"],\"image\":[\"hg\"]}", "[1]", "\"rx\"", "not json", "", "null"] {
        assert!(parse_selector(bad).is_err(), "{bad}");
    }
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    for name in [
        "EZSDR_MEASUREMENT_MBOARD",
        "EZSDR_MEASUREMENT_FPGA_IMAGE",
        "EZSDR_MEASUREMENT_HOST_NAME",
        "EZSDR_MEASUREMENT_SETTINGS_SCRIPT",
        "EZSDR_MEASUREMENT_SETTINGS_VERSION",
        "EZSDR_MEASUREMENT_COMMIT",
    ] {
        env.insert(name.to_owned(), "x".to_owned());
    }
    env.insert("EZSDR_MEASUREMENT_LOAD".to_owned(), "none".to_owned());
    env.insert("EZSDR_MEASUREMENT_OUTPUT".to_owned(), format!("{}/record.json", std::env::temp_dir().display()));
    assert_eq!(measurement::metadata_from_env(&env).unwrap().selector, None);
    env.insert(measurement::SELECTOR_ENV.to_owned(), "{\"rx\":[],\"tx\":[]}".to_owned());
    assert_eq!(measurement::metadata_from_env(&env).unwrap().selector, Some(json!({ "rx": [], "tx": [] })));
    env.insert(measurement::SELECTOR_ENV.to_owned(), "{\"rx\":[\"0:B:cbx:0\"]}".to_owned());
    assert!(measurement::metadata_from_env(&env).is_err(), "one direction's layout without the other is refused");
    env.insert(measurement::SELECTOR_ENV.to_owned(), "{\"image\":[\"xg\"]}".to_owned());
    assert!(measurement::metadata_from_env(&env).is_ok(), "a selector with neither keeps the device's layout");
    env.insert(measurement::SELECTOR_ENV.to_owned(), "{\"nope\":1}".to_owned());
    assert!(measurement::metadata_from_env(&env).is_err(), "a refused selector stops the record before the device opens");
    env.insert(measurement::SELECTOR_ENV.to_owned(), String::new());
    assert!(measurement::metadata_from_env(&env).is_err());
}

#[test]
fn a_named_layout_element_has_its_boards_frequency_and_description() {
    // RM-28: the helpers read the board before the `#name`, and the Provider's own parse accepts the named layout.
    let device = fake(obx_and_cbx());
    let dir = TempDir::new();
    with_selector(json!({ "rx": ["0:A:obx:0#in", "0:B:cbx:0#aux"], "tx": ["0:A:obx:0#out", "0:B:cbx:0"] }), || {
        assert_eq!((channel_hz(&*device, Dir::Tx, 0), channel_hz(&*device, Dir::Tx, 1), channel_hz(&*device, Dir::Rx, 1)), (1e9, 2.45e9, 2.45e9));
        let doc = bench_profile(&*device, &dir, json!({}), json!({}), false);
        let binding = measurement::radio_binding(&doc).unwrap();
        measurement::description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
    });
}

#[test]
fn an_installed_selector_keeps_its_layout_and_gives_each_channel_its_own_band() {
    // The bench profile used to overwrite `rx` and `tx` with the device's slot-A layout; an explicit layout now stays, and
    // each transmit channel has its board's frequency, a band around it and an enable entry (RM-33).
    let device = fake(obx_and_cbx());
    let dir = TempDir::new();
    let plain = bench_profile(&*device, &dir, json!({}), json!({}), false);
    assert_eq!(plain["bindings"]["radio"]["selector"]["tx"], json!(["0:A:obx:0"]), "no selector: the device's slot-A layout, as before");
    assert_eq!(bench_hz(&*device), 1e9);
    assert_eq!(bench_envelope(&*device)["radio.rf_envelope"], json!({ "allowed_bands": [{ "lo_hz": 999e6, "hi_hz": 1001e6 }], "max_gain_db": 0.0, "tx_enabled": [true] }));
    with_selector(json!({ "rx": ["0:A:obx:0", "0:B:cbx:0"], "tx": ["0:A:obx:0", "0:B:cbx:0"], "image": ["xg"], "links": [2], "link_gbps": [10] }), || {
        let doc = bench_profile(&*device, &dir, json!({}), json!({}), false);
        let selector = &doc["bindings"]["radio"]["selector"];
        assert_eq!((&selector["rx"], &selector["tx"]), (&json!(["0:A:obx:0", "0:B:cbx:0"]), &json!(["0:A:obx:0", "0:B:cbx:0"])));
        assert_eq!((&selector["image"], &selector["links"], &selector["link_gbps"]), (&json!(["xg"]), &json!([2]), &json!([10])));
        // The Provider's own parse accepts it (RM-28), where a scalar `image` or a two-element `link_gbps` would be refused.
        let binding = measurement::radio_binding(&doc).unwrap();
        measurement::description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
        assert_eq!(selector["args"], ARGS, "the device's args are not the selector's to change");
        assert_eq!((channel_hz(&*device, Dir::Tx, 0), channel_hz(&*device, Dir::Tx, 1)), (1e9, 2.45e9));
        assert_eq!(
            bench_envelope(&*device)["radio.rf_envelope"],
            json!({
            "allowed_bands": [{ "lo_hz": 999e6, "hi_hz": 1001e6 }, { "lo_hz": 2.449e9, "hi_hz": 2.451e9 }], "max_gain_db": 0.0, "tx_enabled": [true, true] })
        );
    });
    assert_eq!(selector_override(), None, "the selector is removed when the closure ends");
    // Without a selector, a device with two boards of one kind has each transmit channel enabled in the envelope too.
    let pair = fake(FakeConfig { front_ends: vec!["OBX", "OBX"], ..FakeConfig::default() });
    assert_eq!(bench_envelope(&*pair)["radio.rf_envelope"]["tx_enabled"], json!([true, true]));
}

#[test]
fn a_selector_the_device_does_not_fit_is_the_groups_failure() {
    // UR-5: a layout naming a UBX on a device that has an OBX is refused when the first Run is assembled; the group fails
    // and the record keeps the failure.
    let device = fake(one_obx());
    let plan = measurement::MeasurementPlan::rehearsal();
    let mut groups = measurement::GroupRun::new(None);
    with_selector(json!({ "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"] }), || {
        groups.run("rx_throughput", &["radio.perf.rx_bytes_per_s".to_owned()], || measurement::measure_rx_throughput(device.clone(), &plan));
    });
    let failed = groups.failed();
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(failed[0].1.contains("UR-5"), "{failed:?}");
}

#[test]
fn a_companion_that_fails_after_a_solo_measurement_keeps_the_solo_value() {
    // The companion is its own group: here the latch holds after the solo group, so the companion is refused, and the solo value stays.
    let inner = fake(one_obx());
    let guard = Arc::new(measurement::GuardedDevice::new(inner, measurement::Latch::new()));
    let device: Arc<dyn Device> = guard.clone();
    let plan = measurement::MeasurementPlan::rehearsal();
    let mut groups = measurement::GroupRun::new(None);
    let solo_keys = vec!["radio.timing.min_timed_command_lead_ns".to_owned()];
    groups.run("min_timed_command_lead", &solo_keys, || measurement::measure_min_timed_command_lead(device.clone(), &plan));
    let _ = guard.note(Err::<(), _>(ezsdr_radio_uhd::DeviceError::failed("Control operation timed out")), false);
    groups.run("overflow_restart_gap_with_tx", &[], || measurement::measure_overflow_restart_gap_with_tx(device.clone(), &plan));
    let failed: Vec<_> = groups.failed().iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(failed, ["overflow_restart_gap_with_tx"], "only the companion group failed");
    let taken = groups.take();
    let solo = taken
        .values
        .iter()
        .find(|row| match row {
            ezsdr_radio::measurement::ValueRecord::Measured { key, .. } | ezsdr_radio::measurement::ValueRecord::Unmeasured { key, .. } => key.as_str() == "radio.timing.min_timed_command_lead_ns",
        })
        .expect("the solo group's row is kept");
    assert!(!matches!(solo, ezsdr_radio::measurement::ValueRecord::Unmeasured { reason, .. } if reason.contains("failed")), "{solo:?}");
}

#[test]
fn a_companion_group_is_skipped_with_an_observation_when_the_layout_lacks_the_opposite_direction() {
    let device = fake(one_obx());
    let plan = measurement::MeasurementPlan::rehearsal();
    let calls = device.calls().len();
    let group = with_selector(json!({ "rx": ["0:A:obx:0"], "tx": [] }), || measurement::measure_overflow_restart_gap_with_tx(device.clone(), &plan)).unwrap();
    assert!(group.conditions.is_empty() && group.values.is_empty());
    assert_eq!((group.observations[0].name.as_str(), group.observations[0].result["status"].as_str()), ("provider_overflow_restart_gap_with_tx", Some("skipped_no_opposite_direction")));
    assert_eq!(device.calls().len(), calls, "a skipped companion touches nothing");
}

#[test]
fn a_stage_that_wedges_the_handle_stops_the_group_and_caps_the_capacity() {
    // A streamer left open ends the sending: the stages after it are not run, and the capacity stops below them.
    let device = fake(obx_and_cbx());
    let plan = two_rate_plan();
    let group = with_selector(two_channel_selector(), || {
        measurement::with_injected_fault(measurement::InjectedFault { starve_rx: Some(0), ..Default::default() }, || measurement::measure_rx_throughput(device.clone(), &plan).unwrap())
    });
    let ran: Vec<_> = group.observations.iter().filter(|row| row.name.starts_with("provider_rx_throughput_rx-")).map(|row| row.name.as_str()).collect();
    assert_eq!(ran, ["provider_rx_throughput_rx-grouped-390625hz"], "no stage runs after the first wedged one: {:?}", names(&group));
    assert_eq!(
        group.conditions.iter().filter(|condition| condition.id.starts_with("rx-grouped") || condition.id.starts_with("rx-split") || condition.id.starts_with("rx-mixed")).count(),
        5,
        "the planned conditions are still declared"
    );
    // The wedged grouped stage failed and the stages after it did not run: the capacity stops at the passing one-channel stages,
    // named for a condition that passed, and the blocker is recorded apart.
    assert!(
        matches!(&group.values[..], [ezsdr_radio::measurement::ValueRecord::Measured { worst: ezsdr_radio::measurement::Worst::Number(bytes), condition, .. }]
        if *bytes == 4.0 * 390_625.0 && condition == "rx-390625hz"),
        "{:?}",
        group.values
    );
    let capacity = group.observations.iter().find(|row| row.name == "provider_rx_throughput_capacity").unwrap();
    assert_eq!(capacity.result["conditions"]["rx-grouped-390625hz"]["failed"][0]["class"], "DeviceFailure", "{:?}", capacity.result);
    assert_eq!(capacity.result["conditions"]["rx-split-390625hz"]["failed"][0]["class"], "NotRun", "{:?}", capacity.result);
    assert_eq!((capacity.result["blocker_condition"].as_str(), capacity.result["blocker_bytes_per_s"].as_u64()), (Some("rx-grouped-390625hz"), Some(3_125_000)), "{:?}", capacity.result);
}

/// Every call that would reach the device, once the guard latches; none of them may add to the inner device's call log.
fn assert_refused_without_reaching(guard: &measurement::GuardedDevice, device: &FakeDevice) {
    let before = device.calls().len();
    let settings = ezsdr_radio_uhd::Settings { rate: Some(1e6), freq: Some(1e9), gain: Some(0.0), antenna: Some("RX2".to_owned()) };
    assert!(guard.apply(Dir::Rx, 0, &settings).is_err());
    assert!(guard.rx_open(0, &[0]).is_err() && guard.tx_open(0, &[0]).is_err());
    assert!(guard.time_now().is_err() && guard.serial().is_err() && guard.front_end(Dir::Rx, 0).is_err());
    assert!(guard.set_sources("internal", "internal").is_err() && guard.rx_start(0, 0).is_err() && guard.rx_stop(0).is_err());
    assert!(matches!(guard.rx_recv(0, 100, Wall::from_millis(1), false), ezsdr_radio_uhd::RxRecv::Failed(_)));
    guard.rx_close(0);
    guard.tx_close(0);
    guard.close_streams();
    assert_eq!(device.calls().len(), before, "nothing reached the inner device after the latch: {:?}", &device.calls()[before..]);
}

#[test]
fn nothing_reaches_the_device_after_a_control_timeout() {
    let device = fake(one_obx());
    let guard = measurement::GuardedDevice::new(device.clone(), measurement::Latch::new());
    assert!(guard.serial().is_ok(), "an unlatched guard forwards");
    let _ = guard.note(Err::<(), _>(ezsdr_radio_uhd::DeviceError::failed("RuntimeError: Control operation timed out waiting for reply")), false);
    assert!(guard.latched().is_some_and(|reason| reason.contains("timed out")));
    assert_refused_without_reaching(&guard, &device);
}

#[test]
fn nothing_reaches_the_device_after_a_prepare_time_apply_failure() {
    // A Provider group whose first Run's configuration `apply` fails: the group fails and the guard holds the apply error.
    let device = fake(FakeConfig { faults: vec![FakeFault::RxApplyFails], ..one_obx() });
    let guard = Arc::new(measurement::GuardedDevice::new(device.clone(), measurement::Latch::new()));
    let plan = measurement::MeasurementPlan::rehearsal();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measurement::measure_rx_throughput(guard.clone(), &plan)));
    assert!(outcome.is_err() || outcome.unwrap().is_err() || guard.latched().is_some(), "the failing apply fails the group");
    assert!(guard.latched().is_some_and(|reason| reason.starts_with("apply failed")), "{:?}", guard.latched());
    assert_refused_without_reaching(&guard, &device);
}

#[test]
fn nothing_reaches_the_device_after_a_group_leaves_a_streamer_open() {
    let device = fake(one_obx());
    let guard = measurement::GuardedDevice::new(device.clone(), measurement::Latch::new());
    guard.rx_open(0, &[0]).unwrap();
    assert_eq!(guard.latch_if_streamers_open("a group"), Some("a group left streamers open: [\"rx0\"]".to_owned()));
    assert_refused_without_reaching(&guard, &device);
    let clean = measurement::GuardedDevice::new(device.clone(), measurement::Latch::new());
    clean.rx_open(0, &[0]).unwrap();
    clean.rx_close(0);
    assert_eq!(clean.latch_if_streamers_open("a group"), None, "a streamer closed again leaves nothing to latch");
}

static RAW_PROBES_RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn a_raw_probe() {
    RAW_PROBES_RUN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn raw_probes_are_inapplicable_when_channel_zero_is_not_slot_a_on_a_ubx_or_obx() {
    let applicable = |selector: Option<Json>| measurement::raw_probes_applicable(selector.as_ref());
    assert!(applicable(None).is_ok() && applicable(Some(json!({ "image": ["xg"] }))).is_ok());
    assert!(applicable(Some(json!({ "rx": ["0:A:obx:0", "0:B:cbx:0"], "tx": ["0:A:obx:0", "0:B:cbx:0"] }))).is_ok());
    assert!(applicable(Some(json!({ "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"] }))).is_ok());
    // A name after `#` is not part of the board (RM-28): the named layouts apply and are refused as the plain ones are.
    assert!(applicable(Some(json!({ "rx": ["0:A:ubx:0#in"], "tx": ["0:A:ubx:0#out"] }))).is_ok());
    assert!(applicable(Some(json!({ "rx": ["0:A:obx:0#in", "0:B:cbx:0#aux"], "tx": ["0:A:obx:0#out"] }))).is_ok());
    for bad in [
        json!({ "rx": ["0:B:cbx:0#in"] }),
        json!({ "tx": ["0:B:ubx:0#out"] }),
        json!({ "rx": ["0:A:cbx:0#in"], "tx": ["0:A:ubx:0#out"] }),
        json!({ "rx": ["0:A:ubx:0#in", "0:B:ubx:0"], "tx": ["0:B:obx:0#out"] }),
    ] {
        assert!(applicable(Some(bad.clone())).is_err(), "{bad}");
    }
    for bad in [json!({ "rx": ["0:B:cbx:0"] }), json!({ "tx": ["0:A:cbx:0"] }), json!({ "rx": ["0:A:obx:0"], "tx": ["0:B:obx:0"] }), json!({ "rx": ["0:A:basic_rx:0"] })] {
        assert!(applicable(Some(bad.clone())).is_err(), "{bad}");
    }
    let mut groups = measurement::GroupRun::new(None);
    let probes: [(&str, fn()); 2] = [("probe_a", a_raw_probe), ("probe_b", a_raw_probe)];
    measurement::run_raw_probes(&mut groups, &probes, applicable(Some(json!({ "rx": ["0:B:cbx:0"], "tx": ["0:B:cbx:0"] }))));
    assert_eq!(RAW_PROBES_RUN.load(std::sync::atomic::Ordering::SeqCst), 0, "no raw probe ran");
    assert!(groups.failed().is_empty(), "an inapplicable probe is not a failure: {:?}", groups.failed());
    let taken = groups.take();
    let statuses: Vec<_> = taken.observations.iter().map(|row| (row.name.as_str(), row.result["status"].as_str())).collect();
    assert_eq!(statuses, [("probe_a", Some("inapplicable_to_this_layout")), ("probe_b", Some("inapplicable_to_this_layout"))]);
    measurement::run_raw_probes(&mut groups, &probes, applicable(None));
    assert_eq!(RAW_PROBES_RUN.load(std::sync::atomic::Ordering::SeqCst), 2, "on an applicable layout they run");
}

#[test]
fn a_stage_between_the_profile_limit_and_the_candidate_ceiling_is_admitted() {
    // 200 + 100 MS/s is 1 200 MB/s: over the profile's 1 000 MB/s link, under the candidate ceiling `run_stage` passes to the Run.
    let stage = stage_of("rx-mixed-test", "mixed", &[(&[0], 200_000_000), (&[1], 100_000_000)], &[]);
    let result = run_two_channel_stage(&stage, Default::default()).unwrap();
    assert_ne!(result.class, measurement::TrialClass::AdmissionRejection, "{:?}", result.observation);
    assert_eq!(result.observation["admission_ceiling_bytes_per_s"], 1_250_000_000);
}

#[test]
fn the_one_channel_transmit_spec_carries_transmit_channel_zero_s_own_frequency() {
    // `with_tx` copies the receive channel's frequency; a CBX's transmit channel 0 is tuned to its own 2.45 GHz.
    let device = fake(one_cbx());
    let spec = measurement::tx_at_channel_zero_frequency(with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &*device);
    assert_eq!(spec["resources"]["radio"]["requires"]["radio.tx.c0.frequency_hz"], json!({ "kind": "eq", "value": 2.45e9 }));
    assert_eq!(spec["resources"]["radio"]["requires"]["radio.rx.c0.frequency_hz"], json!({ "kind": "eq", "value": 1e9 }), "only the transmit side changes");
}

#[test]
fn a_latched_guard_reads_nothing_from_the_device_and_leaks_it_when_dropped() {
    let device = fake(one_obx());
    let baseline = Arc::strong_count(&device);
    {
        let guard = measurement::GuardedDevice::new(device.clone(), measurement::Latch::new());
        assert_eq!(guard.describe(), device.describe(), "the description is read when the guard is made");
        guard.rx_close(0);
    }
    assert_eq!(Arc::strong_count(&device), baseline, "an unlatched guard hands the device back when dropped");
    let latch = measurement::Latch::new();
    let guard = measurement::GuardedDevice::new(device.clone(), latch.clone());
    let described = guard.describe();
    latch.set("a timeout");
    let calls = device.calls().len();
    assert_eq!((guard.describe(), guard.master_clock_rate()), (described, device.master_clock_rate()), "describe and the clock rate come from the cache");
    drop(guard);
    assert_eq!(Arc::strong_count(&device), baseline + 1, "a latched guard leaks its handle: no teardown reaches the device");
    assert_eq!(device.calls().len(), calls);
    // An unwinding probe that held a latched guard leaks it the same way.
    let latch = measurement::Latch::new();
    let before = Arc::strong_count(&device);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let guard = measurement::GuardedDevice::new(device.clone(), latch.clone());
        latch.set("an apply error");
        let _held = &guard;
        panic!("the probe fails");
    }));
    assert!(unwound.is_err());
    assert_eq!(Arc::strong_count(&device), before + 1, "the handle of a probe that unwound latched is leaked");
}

#[test]
fn a_latched_record_calls_no_opener() {
    let opened = std::cell::Cell::new(0);
    let opener = || -> Arc<dyn Device> {
        opened.set(opened.get() + 1);
        fake(one_obx())
    };
    // No record: the opener's device is returned as it is.
    let plain = measurement::guard_open(opener);
    assert_eq!(opened.get(), 1);
    drop(plain);
    let latch = measurement::Latch::new();
    measurement::install_record_latch(Some(latch.clone()));
    let guarded = measurement::guard_open(opener);
    assert_eq!(opened.get(), 2);
    assert!(guarded.serial().is_ok());
    latch.set("a probe's apply error");
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measurement::guard_open(opener)));
    let typed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measurement::open_record_guard(opener)));
    measurement::install_record_latch(None);
    assert!(refused.is_err() && typed.is_err(), "a latched record opens no device");
    assert_eq!(opened.get(), 2, "the opener was not called");
}

#[test]
fn record_reopen_requires_a_clear_latch_clean_lease_and_exclusive_owner() {
    let latch = measurement::Latch::new();
    measurement::install_record_latch(Some(latch.clone()));
    let opened = std::cell::Cell::new(0);
    let opener = || {
        let opened = &opened;
        move || -> Arc<dyn Device> {
            opened.set(opened.get() + 1);
            fake(one_obx())
        }
    };

    let latched = Arc::new(measurement::GuardedDevice::new(fake(one_obx()), latch.clone()));
    let mut latched_lease = measurement::DeviceLease::new(latched.clone());
    latched_lease.mark_clean();
    latch.set("a raw probe failed");
    assert!(latched_lease.reopen(latched, opener()).err().unwrap().contains("record latch"));
    assert_eq!(opened.get(), 0, "a latched record never calls its opener");

    measurement::install_record_latch(Some(measurement::Latch::new()));
    let unclean = Arc::new(measurement::GuardedDevice::new(fake(one_obx()), measurement::Latch::new()));
    let unclean_lease = measurement::DeviceLease::new(unclean.clone());
    assert!(!unclean_lease.is_clean());
    assert!(unclean_lease.reopen(unclean, opener()).err().unwrap().contains("not clean"));
    assert_eq!(opened.get(), 0, "an unclean lease never calls its opener");

    let extra_latch = measurement::Latch::new();
    measurement::install_record_latch(Some(extra_latch.clone()));
    let held = Arc::new(measurement::GuardedDevice::new(fake(one_obx()), extra_latch));
    let weak = Arc::downgrade(&held);
    let extra = held.clone();
    let mut held_lease = measurement::DeviceLease::new(held.clone());
    held_lease.mark_clean();
    assert_eq!(held_lease.strong_count(), 3);
    assert!(held_lease.reopen(held, opener()).err().unwrap().contains("2 strong owners"));
    assert_eq!(opened.get(), 0, "a residual Arc owner prevents reopening");
    assert!(weak.upgrade().is_some(), "the unforced owner is not closed");
    drop(extra);
    assert!(weak.upgrade().is_none());

    let clean_latch = measurement::Latch::new();
    measurement::install_record_latch(Some(clean_latch.clone()));
    let clean = Arc::new(measurement::GuardedDevice::new(fake(one_obx()), clean_latch.clone()));
    let old = Arc::downgrade(&clean);
    let mut lease = measurement::DeviceLease::new(clean.clone());
    lease.mark_clean();
    let reopened = lease
        .reopen(clean, || {
            opened.set(opened.get() + 1);
            assert!(old.upgrade().is_none(), "the prior GuardedDevice is dropped before open");
            fake(one_obx())
        })
        .unwrap();
    assert_eq!(opened.get(), 1);
    clean_latch.set("shared latch");
    assert!(reopened.latched().is_some(), "the reopened handle uses the record latch");
    drop(reopened);

    let lease_device = fake(one_obx());
    let mismatch_latch = measurement::Latch::new();
    measurement::install_record_latch(Some(mismatch_latch.clone()));
    let unrelated_guard = Arc::new(measurement::GuardedDevice::new(fake(one_obx()), mismatch_latch));
    let mut mismatched_lease = measurement::DeviceLease::new(lease_device);
    mismatched_lease.mark_clean();
    assert!(mismatched_lease.reopen(unrelated_guard, opener()).err().unwrap().contains("does not belong"));
    assert_eq!(opened.get(), 1, "a different guarded handle never opens another device");
    measurement::install_record_latch(None);
}

#[test]
fn restore_failure_and_unwound_group_refuse_reopen_without_opening_again() {
    for restore_fails in [true, false] {
        let latch = measurement::Latch::new();
        measurement::install_record_latch(Some(latch.clone()));
        let (device, description, plan) = raw_group_fixture(one_obx());
        let probe = Arc::new(probe_device::ProbeDevice::new(device));
        if restore_fails {
            probe.fail_restoring_apply();
        }
        let guard = Arc::new(measurement::GuardedDevice::new(probe, latch.clone()));
        let mut lease = measurement::DeviceLease::new(guard.clone());
        let mut groups = measurement::GroupRun::new(None);
        if !restore_fails {
            groups.run("unwound_group", &[], || panic!("scripted group unwind"));
        }
        let (stopped, restored) = measurement::run_raw_group_leased(&mut lease, &mut groups, &description, &plan);
        if restore_fails {
            assert!(restored.as_deref().is_some_and(|reason| reason.contains("scripted restore failure")));
            assert!(stopped.as_deref().is_some_and(|reason| reason.contains("scripted restore failure")));
            assert!(latch.reason().is_some(), "restore failure latches the handle");
        } else {
            assert!(groups.failed().iter().any(|(name, reason)| name == "unwound_group" && reason.contains("scripted group unwind")));
            assert_eq!((stopped, restored), (None, None));
            assert!(latch.reason().is_none());
        }
        assert!(!lease.is_clean(), "restore failure or a group unwind keeps the lease unclean");
        let opened = std::cell::Cell::new(0);
        let result = lease.reopen(guard, || {
            opened.set(opened.get() + 1);
            fake(one_obx())
        });
        assert!(result.is_err());
        assert_eq!(opened.get(), 0, "a failed or unclean run never calls the reopen opener");
    }
    measurement::install_record_latch(None);
}

#[test]
fn reopened_values_use_the_conservative_pair_and_failures_invalidate_base_values() {
    use ezsdr_radio::measurement::{Condition, ValueRecord, Worst};
    let group = |id: &str, key: &str, value: f64| {
        let condition = Condition { id: id.to_owned(), streams: vec![measurement::stream_condition(ezsdr_kernel::stream::Direction::Rx, 0, vec![0], 10e6)], concurrent: vec![], reopened: false };
        let measured = ValueRecord::Measured { key: Key::parse(key).unwrap(), worst: Worst::Number(value), trials: 1, condition: id.to_owned(), test: "tests/fake.rs::reopened_values".to_owned() };
        let row = measurement::observation("reopen_test", "tests/fake.rs::reopened_values", Some(id.to_owned()), 1, json!({ "condition_id": id })).unwrap();
        measurement::MeasurementGroup { conditions: vec![condition], values: vec![measured], observations: vec![row] }
    };
    let rx_key = "radio.perf.rx_bytes_per_s".to_owned();
    let lead_key = "radio.timing.rx_start_lead_ns".to_owned();
    let mut groups = measurement::GroupRun::new(None);
    groups.run("base", &[], || {
        let mut base = group("base-rx-throughput", &rx_key, 900.0);
        base.append(group("base-rx-lead", &lead_key, 5.0)).unwrap();
        Ok(base)
    });
    groups.run_reopened("reopened_rx_throughput", std::slice::from_ref(&rx_key), measurement::ReopenedMerge::Minimum, || Ok(group("rx-throughput", &rx_key, 800.0)));
    groups.run_reopened("reopened_rx_start_lead", std::slice::from_ref(&lead_key), measurement::ReopenedMerge::Maximum, || Ok(group("rx-lead", &lead_key, 10.0)));
    let measured = groups.take();
    assert_eq!(measured.values.len(), 2, "reopened values replace rather than duplicate keys");
    let numeric = |key: &str| {
        measured
            .values
            .iter()
            .find_map(|value| match value {
                ValueRecord::Measured { key: found, worst: Worst::Number(value), condition, .. } if found.as_str() == key => Some((*value, condition.as_str())),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(numeric(&rx_key), (800.0, "reopened-rx-throughput"));
    assert_eq!(numeric(&lead_key), (10.0, "reopened-rx-lead"));
    assert_eq!(measured.conditions.iter().filter(|condition| condition.reopened).count(), 2);
    assert!(measured.observations.iter().any(|row| row.name == "reopened_reopen_test"
        && row.condition.as_deref() == Some("reopened-rx-throughput")
        && row.result.iter().any(|(name, value)| name == "condition_id" && value.as_str() == Some("reopened-rx-throughput"))));

    let mut failed = measurement::GroupRun::new(None);
    failed.run("base", &[], || Ok(group("base-rx-throughput", &rx_key, 900.0)));
    failed.run_reopened("reopened_rx_throughput", std::slice::from_ref(&rx_key), measurement::ReopenedMerge::Minimum, || Err("reopened run failed".to_owned()));
    let values = failed.take().values;
    assert_eq!(values.len(), 1);
    assert!(matches!(&values[0], ValueRecord::Unmeasured { reason, .. } if reason.contains("reopened group")));

    let mut unmeasured = measurement::GroupRun::new(None);
    unmeasured.run("base", &[], || Ok(group("base-rx-throughput", &rx_key, 900.0)));
    unmeasured.run_reopened("reopened_rx_throughput", std::slice::from_ref(&rx_key), measurement::ReopenedMerge::Minimum, || {
        Ok(measurement::MeasurementGroup {
            conditions: vec![],
            values: vec![ValueRecord::Unmeasured { key: Key::parse(&rx_key).unwrap(), reason: "no successful reopened candidate".to_owned() }],
            observations: vec![],
        })
    });
    assert!(matches!(&unmeasured.take().values[0], ValueRecord::Unmeasured { reason, .. } if reason.contains("no successful reopened candidate")));

    let mut missing = measurement::GroupRun::new(None);
    missing.run("base", &[], || Ok(group("base-rx-throughput", &rx_key, 900.0)));
    missing.run_reopened("reopened_rx_throughput", std::slice::from_ref(&rx_key), measurement::ReopenedMerge::Minimum, || Ok(measurement::MeasurementGroup::default()));
    assert!(matches!(&missing.take().values[0], ValueRecord::Unmeasured { reason, .. } if reason.contains("exactly one")));

    let mut skipped = measurement::GroupRun::new(None);
    skipped.run("base", &[], || Ok(group("base-rx-throughput", &rx_key, 900.0)));
    skipped.skip_reopened(std::slice::from_ref(&rx_key), "the device still has 2 strong owners");
    let skipped = skipped.take();
    assert!(matches!(&skipped.values[0], ValueRecord::Unmeasured { reason, .. } if reason.contains("2 strong owners")));
    assert!(skipped.observations.iter().any(|row| row.name == "provider_reopen" && row.result.iter().any(|(name, value)| name == "status" && value.as_str() == Some("skipped"))));

    let mut base_unmeasured = measurement::GroupRun::new(None);
    base_unmeasured.run("base", &[], || {
        let mut base = group("base-rx-throughput", &rx_key, 900.0);
        base.values[0] = ValueRecord::Unmeasured { key: Key::parse(&rx_key).unwrap(), reason: "base trial incomplete".to_owned() };
        Ok(base)
    });
    base_unmeasured.run_reopened("reopened_rx_throughput", std::slice::from_ref(&rx_key), measurement::ReopenedMerge::Minimum, || Ok(group("rx-throughput", &rx_key, 800.0)));
    assert!(matches!(&base_unmeasured.take().values[0], ValueRecord::Unmeasured { reason, .. } if reason.contains("base measurement is unmeasured")));

    let mut base_non_numeric = measurement::GroupRun::new(None);
    base_non_numeric.run("base", &[], || {
        let mut base = group("base-rx-throughput", &rx_key, 900.0);
        base.values[0] = ValueRecord::Measured {
            key: Key::parse(&rx_key).unwrap(),
            worst: Worst::Text("not numeric".to_owned()),
            trials: 1,
            condition: "base-rx-throughput".to_owned(),
            test: "tests/fake.rs::reopened_values".to_owned(),
        };
        Ok(base)
    });
    base_non_numeric.run_reopened("reopened_rx_throughput", std::slice::from_ref(&rx_key), measurement::ReopenedMerge::Minimum, || Ok(group("rx-throughput", &rx_key, 800.0)));
    assert!(matches!(&base_non_numeric.take().values[0], ValueRecord::Unmeasured { reason, .. } if reason.contains("both be measured numeric values")));
}

#[test]
fn h4_fake_session_records_tx_headroom_and_load_with_no_run_time_update() {
    // The Session asks for rx/tx c0 at 10 MS/s at connect and repeats a burst; nothing changes at Run time.
    let selector = json!({ "rx": ["0:B:cbx:0"], "tx": ["0:B:cbx:0"] });
    let guard = Arc::new(measurement::GuardedDevice::new(fake_real(obx_and_cbx()), measurement::Latch::new()));
    let device: Arc<dyn Device> = guard.clone();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(device));
    let group = with_selector(selector, || measurement::measure_h4_load(probe.clone(), 200).unwrap());
    assert_eq!(guard.latch_if_streamers_open("H4 rehearsal"), None, "the Session closed its streamers");
    assert!(group.values.is_empty(), "H4 produces observations, not profile values");
    assert_eq!((group.conditions.len(), group.conditions[0].concurrent.len()), (1, 2));
    let result = Json::Object(group.observations[0].result.clone().into_iter().collect());
    assert!(matches!(result["status"].as_str(), Some("complete" | "partial")), "{result}");
    assert_eq!((result["master_clock_hz"].as_u64(), result["sample_rate_hz"].as_u64()), (Some(200_000_000), Some(10_000_000)));
    assert!(result["tx_payload_send_count"].as_u64().unwrap() > 0, "{result}");
    assert!(!result["tx_headroom"].as_array().unwrap().is_empty(), "{result}");
    assert_eq!(result["tx_headroom_proxy"].as_str(), Some("nominal next-sample submission margin; not RF execution time"));
    assert!(result["cpu_stat_during_probe"].is_object() && result["thread_policy_affinity"].is_object(), "{result}");
}

#[test]
fn h4_a_tx_underflow_makes_the_load_record_partial_and_converts_its_time() {
    let guard = Arc::new(measurement::GuardedDevice::new(fake_real(one_obx()), measurement::Latch::new()));
    let device: Arc<dyn Device> = guard;
    let probe = Arc::new(common::probe_device::ProbeDevice::new(device));
    probe.underflow_after_tx_send();
    let group = measurement::measure_h4_load(probe, 200).unwrap();
    let result = Json::Object(group.observations[0].result.clone().into_iter().collect());
    assert!(result["tx_underflow_events"].as_u64().unwrap() > 0, "{result}");
    assert!(result["tx_underflow_converted_events"].as_u64().unwrap() > 0, "the TX SampleClock report was converted to root time: {result}");
    assert_eq!(result["tx_underflow_unconvertible_events"].as_u64(), Some(0));
    assert_eq!(result["status"], "partial", "an underflow-terminated repeat cannot be complete");
}

#[test]
fn h4_load_self_test_reaps_pid_owned_workers_and_checks_the_memory_budget() {
    let script = format!("{}/../../plan/phase8/tools/h4-load.sh", env!("CARGO_MANIFEST_DIR"));
    let record_script = format!("{}/../../plan/phase8/tools/run-record.sh", env!("CARGO_MANIFEST_DIR"));
    for path in [&script, &record_script] {
        let syntax = std::process::Command::new("bash").args(["-n", path]).output().unwrap();
        assert!(syntax.status.success(), "bash -n {path}: {}", String::from_utf8_lossy(&syntax.stderr));
    }
    let self_test = std::process::Command::new("bash").arg(&script).arg("--self-test").output().unwrap();
    assert!(self_test.status.success(), "{}", String::from_utf8_lossy(&self_test.stderr));
    assert!(String::from_utf8_lossy(&self_test.stdout).contains("one-second load stopped and reaped PIDs"));
    let rejected = std::process::Command::new("bash").arg(&script).args(["--cpu", "1", "--mem", "10", "--duration", "1"]).output().unwrap();
    assert!(!rejected.status.success(), "a memory request over the cap must be refused before workers start");
    assert!(!String::from_utf8_lossy(&rejected.stdout).contains("H4_READY"));
    let help = std::process::Command::new("bash").arg(&record_script).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    for variable in ["EZSDR_MEASUREMENT_H4_CPU", "EZSDR_MEASUREMENT_H4_MEM", "EZSDR_MEASUREMENT_H4_DURATION"] {
        assert!(help.contains(variable), "run-record --help omits {variable}");
    }
}

#[test]
fn t200_bare_uhd_script_judges_a_run_and_bounds_it() {
    let script = format!("{}/../../plan/phase8/tools/t200-bare-uhd.sh", env!("CARGO_MANIFEST_DIR"));
    let self_check = std::process::Command::new("bash").arg(&script).arg("--self-check").output().unwrap();
    assert!(self_check.status.success(), "{}", String::from_utf8_lossy(&self_check.stderr));
    assert!(String::from_utf8_lossy(&self_check.stdout).contains("all cases pass"));
}

#[test]
fn an_open_of_channels_another_open_stream_holds_is_refused_before_the_device() {
    // Two candidates in a row: the first Run leaves stream 0's streamer open on channel 0, and stream 1 opened on channel 0 never reaches the device.
    let device = fake(obx_and_cbx());
    let guard = measurement::GuardedDevice::new(device.clone(), measurement::Latch::new());
    guard.rx_open(0, &[0]).unwrap();
    let calls = device.calls().len();
    assert!(guard.rx_open(1, &[0]).is_err());
    assert!(guard.latched().is_some_and(|reason| reason.contains("rx1 [0] was opened while rx0 still held [0]")), "{:?}", guard.latched());
    assert_eq!(device.calls().len(), calls, "the second open did not reach the device");
    assert_refused_without_reaching(&guard, &device);
    // An empty channel list is channel 0.
    let empty = measurement::GuardedDevice::new(device.clone(), measurement::Latch::new());
    empty.rx_open(0, &[]).unwrap();
    assert!(empty.rx_open(1, &[0]).is_err() && empty.latched().is_some());
}

#[test]
fn a_reopen_of_the_same_stream_and_a_disjoint_channel_set_are_allowed() {
    // `hw_b8_raw_restart_lead`'s sequence: open, start, stop, open the same stream again; UhdDevice drops the old streamer first.
    let device = fake(obx_and_cbx());
    let guard = measurement::GuardedDevice::new(device.clone(), measurement::Latch::new());
    guard.rx_open(0, &[0]).unwrap();
    let at = guard.time_now().unwrap() + 20_000_000;
    guard.rx_start(0, at).unwrap();
    guard.rx_stop(0).unwrap();
    guard.rx_open(0, &[0]).unwrap();
    guard.rx_open(0, &[0]).unwrap();
    assert_eq!(guard.latched(), None, "a same-stream re-open replaces its entry and is forwarded");
    // The two-channel conditions: one stream per channel, then a stream of both channels after both are closed.
    let split = measurement::GuardedDevice::new(device.clone(), measurement::Latch::new());
    split.rx_open(0, &[0]).unwrap();
    split.rx_open(1, &[1]).unwrap();
    split.tx_open(0, &[0]).unwrap();
    assert_eq!(split.latched(), None, "disjoint channels on other streams, and the other direction, are allowed");
    split.rx_close(0);
    split.rx_close(1);
    split.rx_open(0, &[0, 1]).unwrap();
    assert_eq!(split.latched(), None);
}

#[test]
fn a_forwarded_receive_timeout_or_loss_latches_and_blocks_later_calls() {
    for (message, lost) in [("Control operation timed out", false), ("the device is gone", true)] {
        let inner = fake(one_obx());
        let probe = Arc::new(common::probe_device::ProbeDevice::new(inner.clone()));
        let failed = ezsdr_radio_uhd::RxRecv::Failed(ezsdr_radio_uhd::DeviceError { lost, message: message.to_owned() });
        probe.script_rx(vec![failed]);
        let guard = measurement::GuardedDevice::new(probe.clone(), measurement::Latch::new());
        assert!(matches!(guard.rx_recv(0, 100, Wall::from_millis(1), false), ezsdr_radio_uhd::RxRecv::Failed(_)));
        assert!(guard.latched().is_some_and(|reason| reason.contains(message)), "{message}: {:?}", guard.latched());
        let rows = probe.control_log().len();
        assert!(matches!(guard.rx_recv(0, 100, Wall::from_millis(1), false), ezsdr_radio_uhd::RxRecv::Failed(_)));
        assert_eq!(probe.control_log().len(), rows, "the second receive did not reach the device");
        assert_refused_without_reaching(&guard, &inner);
    }
    // An ordinary receive failure does not latch.
    let inner = fake(one_obx());
    let probe = Arc::new(common::probe_device::ProbeDevice::new(inner));
    probe.script_rx(vec![ezsdr_radio_uhd::RxRecv::Failed(ezsdr_radio_uhd::DeviceError::failed("a bad packet"))]);
    let guard = measurement::GuardedDevice::new(probe, measurement::Latch::new());
    let _ = guard.rx_recv(0, 100, Wall::from_millis(1), false);
    assert_eq!(guard.latched(), None);
}

#[test]
fn the_waveforms_of_the_added_transmissions_stay_within_half_a_unit() {
    // `pn` swings both components by 0.4, which is a magnitude of 0.566; the added transmissions scale it under bench.md's 0.5.
    assert!(pn(1_000).iter().any(|(re, im)| re.hypot(*im) > 0.5));
    let (bytes, _) = transmit_waveform(1_000);
    let magnitudes: Vec<f32> = bytes.chunks(8).map(|pair| f32::from_le_bytes(pair[..4].try_into().unwrap()).hypot(f32::from_le_bytes(pair[4..].try_into().unwrap()))).collect();
    assert_eq!(magnitudes.len(), 1_000);
    assert!(magnitudes.iter().all(|magnitude| *magnitude <= 0.5) && magnitudes.iter().any(|magnitude| *magnitude > 0.4), "{magnitudes:?}");
}

// ---------------------------------------------------------------- D9a: raw probes (UR-24, UR-33)

#[test]
fn ur_24_d9_p4_c_closes_and_reopens_the_streamer_and_a_does_not() {
    // P4 C's close and reopen is checked on the calls, not on the DSP FIFO the fake no longer models.
    let device = fake(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() });
    let mut tune = |dir, chan, offset, _at| device.apply(dir, chan, &ezsdr_radio_uhd::Settings { freq: Some(1e9 + offset), ..Default::default() }).map(|_| offset);
    common::d9::open_raw(&*device, 0);
    for case in [common::d9::P4Case::DspOnly, common::d9::P4Case::DspOnlyReopen] {
        let before = device.calls().len();
        common::d9::p4_dsp_only(&*device, 0, &mut tune, case);
        let streamer: Vec<String> = device.calls()[before..].iter().filter(|call| call.starts_with("rx_close") || call.starts_with("rx_open")).cloned().collect();
        let expected: &[&str] = if case == common::d9::P4Case::DspOnlyReopen { &["rx_close 0", "rx_open 0 [0]"] } else { &[] };
        assert_eq!(streamer, expected, "{case:?}: the streamer is closed and reopened only in case C");
    }
}

#[test]
fn ur_33_d9_raw_probes_match_the_x3xx_tx_model() {
    let device = fake(FakeConfig::default());
    // An untimed tune: the fake has no timed one, and the bench's timed tune is `UhdDevice::tune_auto`.
    let mut tune = |dir, chan, offset, _at| device.apply(dir, chan, &ezsdr_radio_uhd::Settings { freq: Some(1e9 + offset), ..Default::default() }).map(|_| offset);

    // Bench-results.md §D9 (2026-10-29): P5 A is early with the stale header; the untimed-close
    // control has none; P5 C's acknowledged two-zero flush restores the transition at e.
    let p5 = common::d9::p5_stale_header(&*device, 0, &mut tune, &[common::d9::P5Case::StaleHeader, common::d9::P5Case::NoStaleHeader, common::d9::P5Case::FlushAfterReopen]);
    assert!(p5[0].old_header.is_some_and(|header| header >= p5[0].e));
    assert_eq!(p5[1].old_header, None);
    assert!(p5[2].flush_reports.iter().any(|report| report.code == TxCode::BurstAck && report.channel == 0));
    // FakeDevice cannot reproduce the RF loopback (its receive path forwards samples only when RX and
    // TX frequencies match), so the tone verdict is the bench's. No assertion on the fake's verdict:
    // it depends on wall-clock timing (2026-11-01).

    // Bench-results.md §D9: P6's chunks carry their first sample's time; starvation reports
    // Underflow then TimeError and the accepted late continuation does not reach loopback.
    let before_calls = device.calls().len();
    let (_, starvation) = common::d9::p6_every_chunk_timed(&*device, 0, &mut tune);
    assert!(device.calls()[before_calls..].iter().filter(|call| call.starts_with("tx_send 0 n=2000 ")).all(|call| !call.contains("at=-")), "P6 sends every chunk with its first sample's time");
    let codes = common::d9::report_codes(&starvation.reports);
    assert!(codes.contains(&TxCode::Underflow) && codes.contains(&TxCode::TimeError), "{codes:?}");
    // FakeDevice emits its deterministic ramp when the TX source is absent; the hardware
    // continuation classifier can count that ramp as signal, so reports and TX samples decide.
    assert!(device.transmitted(starvation.queued_end + 100 * 200, 1_900).iter().all(Option::is_none), "late continuation samples are absent from FakeDevice's loopback source");

    // Bench-results.md §D70: one-sample EOB has no ack and loses the next timed start; the
    // two-zero control is acknowledged and keeps the requested start.
    let d70 = common::d9::d70_one_sample_then_two(&*device, 0, &mut tune);
    // D70's FakeDevice verdict is the timed tone's BurstAck tick: an early start can fall between RX sample instants.
    assert!(!d70[0].priming_reports.iter().any(|report| report.code == TxCode::BurstAck));
    assert!(d70[1].priming_reports.iter().any(|report| report.code == TxCode::BurstAck && report.channel == 0));
    let ack_tick = |result: &common::d9::D70Result| result.reports.iter().find(|report| report.code == TxCode::BurstAck && report.channel == 0).and_then(|report| report.tick).unwrap();
    let failed_close_ack = ack_tick(&d70[0]);
    let reset_ack = ack_tick(&d70[1]);
    assert!(failed_close_ack < d70[0].start + 30 * ms(1) - ms(100), "D70 one-sample EOB leaves the next timed burst early: ack={failed_close_ack}, start={}", d70[0].start);
    assert!((reset_ack - (d70[1].start + 30 * ms(1))).abs() <= 200, "D70 two-zero control honors the requested start: ack={reset_ack}, start={}", d70[1].start);
}

#[test]
fn the_tone_classifier_finds_an_attenuated_shift_in_noise() {
    // No hardware: a 100 kHz tone of amplitude 0.008 (the loopback's attenuation of 0.4) over
    // noise, shifting to 150 kHz early (2 ms into the burst), on time (20 ms) or never; and noise alone.
    let (start, e) = (1_000_000i64, 1_000_000 + 20 * 200_000);
    let make = |shift_ms: Option<i64>, tone: bool| -> Vec<(i64, ezsdr_radio_uhd::Iq)> {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let mut phase = 0.0f64;
        (0..70_000i64)
            .map(|i| {
                let t = start - 5 * 200_000 + i * 200;
                let in_burst = t >= start && t < start + 60 * 200_000;
                let hz = if shift_ms.is_some_and(|ms| t >= start + ms * 200_000) { 150e3 } else { 100e3 };
                phase += 2.0 * std::f64::consts::PI * hz / 1e6;
                let amplitude = if tone && in_burst { 0.008 } else { 0.0 };
                (t, [(amplitude * phase.cos() + 0.006 * noise()) as f32, (amplitude * phase.sin() + 0.006 * noise()) as f32])
            })
            .collect()
    };
    let end = start + 60 * 200_000;
    let early = tone::classify_tone(&make(Some(2), true), start, end, e, 100e3);
    assert!(early.usable(), "{early:?}");
    let at = early.moves_at.expect("an early shift") as f64 / 200.0;
    assert!((at + 18_000.0).abs() < 1_500.0, "the tone leaves 100 kHz about 18 ms before e: {at} µs ({early:?})");
    let on_time = tone::classify_tone(&make(Some(20), true), start, end, e, 100e3);
    assert!(on_time.usable() && on_time.moves_at.is_some_and(|t| (t as f64 / 200.0).abs() < 1_500.0), "{on_time:?}");
    let never = tone::classify_tone(&make(None, true), start, end, e, 100e3);
    assert!(never.usable() && never.moves_at.is_none(), "{never:?}");
    let silent = tone::classify_tone(&make(Some(2), false), start, end, e, 100e3);
    assert!(!silent.usable() && silent.before * 2 < silent.before_of && silent.after * 2 < silent.after_of, "noise alone is not a tone: {silent:?}");
    assert!(tone::tone_verdict(&silent).starts_with("INCONCLUSIVE"));
    // A hole in the capture around an on-time shift: the packets' timestamps jump from 19 ms to
    // 40 ms into the burst, so the move is known only to lie in the hole.
    let holed = |from: i64, to: i64, shift: i64| -> Vec<(i64, ezsdr_radio_uhd::Iq)> {
        make(Some(shift), true).into_iter().filter(|(t, _)| *t < start + from * 200_000 || *t >= start + to * 200_000).collect()
    };
    let gapped = tone::classify_tone(&holed(19, 40, 20), start, end, e, 100e3);
    assert!(gapped.gapped && gapped.moves_at.is_none() && !gapped.usable(), "{gapped:?}");
    assert!(tone::tone_verdict(&gapped).starts_with("INCONCLUSIVE"));
    // A hole well after the shift does not hide it.
    let elsewhere = tone::classify_tone(&holed(40, 50, 20), start, end, e, 100e3);
    assert!(!elsewhere.gapped && elsewhere.moves_at.is_some_and(|t| (t as f64 / 200.0).abs() < 1_500.0), "{elsewhere:?}");
    // The bench's 0 dB receive gain: noise rounded to the sc16 LSB (a window's coherence is then anywhere from 0 to 1) under a
    // tone of 0.002. A move from the burst's first window is "at or before it" (D9 F7), not a gap.
    let make_lsb = |shift_ms: Option<i64>, tone: bool| -> Vec<(i64, ezsdr_radio_uhd::Iq)> {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let mut phase = 0.0f64;
        (0..70_000i64)
            .map(|i| {
                let t = start - 5 * 200_000 + i * 200;
                let in_burst = t >= start && t < start + 60 * 200_000;
                let hz = if shift_ms.is_some_and(|ms| t >= start + ms * 200_000) { 150e3 } else { 100e3 };
                phase += 2.0 * std::f64::consts::PI * hz / 1e6;
                let amplitude = if tone && in_burst { 0.002 } else { 0.0 };
                (t, [((amplitude * phase.cos() + (1.2 * noise()).round() / 32768.0) as f32), ((amplitude * phase.sin() + (1.2 * noise()).round() / 32768.0) as f32)])
            })
            .collect()
    };
    let lsb_on_time = tone::classify_tone(&make_lsb(Some(20), true), start, end, e, 100e3);
    assert!(lsb_on_time.usable() && lsb_on_time.moves_at.is_some_and(|t| (t as f64 / 200.0).abs() < 1_500.0), "{lsb_on_time:?}");
    let lsb_f7 = tone::classify_tone(&make_lsb(Some(0), true), start, end, e, 100e3);
    assert!(lsb_f7.usable() && !lsb_f7.gapped && lsb_f7.moves_at.is_some_and(|t| (t as f64 / 200.0 + 19_000.0).abs() < 1_500.0), "{lsb_f7:?}");
    assert!(lsb_f7.at_or_before && tone::tone_verdict(&lsb_f7).contains("at or before"), "{}", tone::tone_verdict(&lsb_f7));
    assert!(!lsb_on_time.at_or_before && !tone::tone_verdict(&lsb_on_time).contains("at or before"));
    let lsb_silent = tone::classify_tone(&make_lsb(Some(2), false), start, end, e, 100e3);
    assert!(!lsb_silent.usable() && lsb_silent.before * 2 < lsb_silent.before_of, "{lsb_silent:?}");
    // A capture with no windows before the burst has no floor.
    let short: Vec<_> = make(Some(2), true).into_iter().filter(|(t, _)| *t >= start).collect();
    assert!(!tone::classify_tone(&short, start, end, e, 100e3).usable());
    // Five quiet windows are too few to measure a floor.
    let few: Vec<_> = make(Some(2), true).into_iter().filter(|(t, _)| *t >= start - 625 * 200).collect();
    assert!(tone::classify_tone(&few, start, end, e, 100e3).floor.is_none());
}

#[test]
fn the_p6_loopback_count_needs_a_baseline_and_a_captured_continuation() {
    // No hardware: queued tone 0.002 from `start` to `queued_end`, a continuation after `next`, 1 sample per 200 ticks.
    let (start, queued_end) = (0i64, 20_000 * 200);
    let next = queued_end;
    let make = |continuation: bool, from: i64, to: i64| -> Vec<(i64, ezsdr_radio_uhd::Iq)> {
        (from..to)
            .map(|i| {
                let t = i * 200;
                let on = t < queued_end || (continuation && t >= next + 44 * 200 && t < next + 2_044 * 200);
                (t, [if on { 0.002 } else { 0.00003 }, 0.0])
            })
            .collect()
    };
    assert_eq!(tone::continuation_in_loopback(&make(true, 0, 23_000), start, queued_end, next), Ok(1_900));
    assert_eq!(tone::continuation_in_loopback(&make(false, 0, 23_000), start, queued_end, next), Ok(0));
    // The receive lost the queued part: no baseline, so a 0 would say nothing.
    assert!(tone::continuation_in_loopback(&make(true, 20_000, 23_000), start, queued_end, next).unwrap_err().contains("baseline"));
    assert!(tone::continuation_in_loopback(&make(true, 15_000, 23_000), start, queued_end, next).unwrap_err().contains("baseline"), "5 000 queued samples are too few");
    let silent_queue: Vec<_> = make(true, 0, 23_000).into_iter().map(|(t, s)| if t < queued_end { (t, [0.0, 0.0]) } else { (t, s) }).collect();
    assert!(tone::continuation_in_loopback(&silent_queue, start, queued_end, next).unwrap_err().contains("baseline"), "a zero median is no baseline");
    assert!(tone::continuation_in_loopback(&[], start, queued_end, next).is_err());
    // The capture stops before the continuation's interval.
    assert!(tone::continuation_in_loopback(&make(true, 0, 20_500), start, queued_end, next).unwrap_err().contains("captured"));
}

// ------------------------------------------------ TXP: the transmit owner's real-time priority (UR-23)

/// `Recording` as a `Device` the Provider and a wrapper can share, and the same value for the test to read.
fn recording(clock: ClockMode) -> Arc<Recording> {
    Arc::new(Recording::new(FakeConfig { clock, ..FakeConfig::default() }))
}

#[test]
fn ur_23_p1b_a_provider_run_raises_the_priority_of_each_transmit_thread_once() {
    let device = recording(ClockMode::Manual);
    let (mut run, _dir) = tx_session_on(device.clone());
    let at = tx_at(&run, ms(20));
    assert!(admitted(&send_at(&mut run, "start_repeat", Some(at), &tone(30_000))));
    wait(&mut run, ms(15));
    let manifest = run.finish();
    assert_eq!(device.raisers(), ["uhd-tx-s0"], "{:?}", device.log());
    assert_eq!(section(&manifest, "stats")["tx_realtime"], 1);
}

/// A refused priority behind the measurement's guard: no latch, the burst plays, one row in `rejected`.
#[test]
fn ur_23_g1_a_refused_priority_does_not_latch_the_measurement_guard() {
    let device = recording(ClockMode::Manual);
    *device.priority.lock().unwrap() = Some(ezsdr_radio_uhd::DeviceError { lost: true, message: "uhd_set_thread_priority: UHD error 31: EPERM".to_owned() });
    let guard = Arc::new(measurement::GuardedDevice::new(device.clone(), measurement::Latch::new()));
    let (mut run, _dir) = tx_session_on(guard.clone());
    let at = tx_at(&run, ms(20));
    assert!(admitted(&send_at(&mut run, "start_repeat", Some(at), &tone(30_000))));
    wait(&mut run, ms(15));
    let manifest = run.finish();
    assert_eq!(guard.latched(), None, "the refusal latched the guard");
    assert!(device.inner.calls().iter().any(|call| call.starts_with("tx_send ") && call.contains("sob=true")), "the burst reached the device: {:?}", device.inner.calls());
    let rows: Vec<_> = section(&manifest, "rejected").as_array().unwrap().iter().filter(|row| row["call"] == "set_thread_priority").cloned().collect();
    assert_eq!(rows.len(), 1, "{:?}", section(&manifest, "rejected"));
    assert!(rows[0]["error"].as_str().unwrap().contains("EPERM"), "{rows:?}");
    assert!(section(&manifest, "stats").get("tx_realtime").is_none());
    assert!(guard.serial().is_ok(), "a later call is not refused");
}

/// The same refusal on the guard alone: a latch would be seen here at once, not as a Run that never starts.
#[test]
fn ur_23_g1b_a_refused_priority_call_does_not_latch_the_guard() {
    let device = recording(ClockMode::Manual);
    *device.priority.lock().unwrap() = Some(ezsdr_radio_uhd::DeviceError { lost: true, message: "uhd_set_thread_priority: UHD error 31: EPERM".to_owned() });
    let guard = measurement::GuardedDevice::new(device.clone(), measurement::Latch::new());
    assert!(guard.raise_thread_priority().unwrap_err().message.contains("EPERM"));
    assert_eq!(guard.latched(), None, "the refusal latched the guard");
    assert!(guard.serial().is_ok(), "a later call is not refused");
}

/// Each wrapper forwards the call to the device behind it, on the calling thread.
#[test]
fn ur_23_g2_every_wrapper_forwards_the_priority_call() {
    fn raised_through(wrap: impl FnOnce(Arc<Recording>) -> Arc<dyn Device> + Send + 'static) -> Vec<String> {
        let device = recording(ClockMode::Real);
        let wrapper = wrap(device.clone());
        std::thread::Builder::new().name("uhd-tx-probe".to_owned()).spawn(move || wrapper.raise_thread_priority().unwrap()).unwrap().join().unwrap();
        device.raisers()
    }
    let expected = ["uhd-tx-probe".to_owned()];
    assert_eq!(raised_through(|inner| Arc::new(measurement::GuardedDevice::new(inner, measurement::Latch::new()))), expected, "GuardedDevice");
    assert_eq!(raised_through(|inner| Arc::new(measurement::CountingDevice::new(inner))), expected, "CountingDevice");
    assert_eq!(raised_through(|inner| Arc::new(probe_device::ProbeDevice::new(inner))), expected, "ProbeDevice");
}

/// A latched guard refuses the call like every other and the device sees nothing.
#[test]
fn ur_23_g3_a_latched_guard_refuses_the_priority_call() {
    let device = recording(ClockMode::Real);
    let latch = measurement::Latch::new();
    let guard = measurement::GuardedDevice::new(device.clone(), latch.clone());
    latch.set("an earlier loss");
    let error = guard.raise_thread_priority().unwrap_err();
    assert!(error.message.starts_with("latched ("), "{error:?}");
    assert!(device.raisers().is_empty());
}

// ---------------------------------------------------------------- note 47 stage 3: receive values on a stopped stream (RM-21, UR-26)

fn receive_applies(direct: &Direct) -> Vec<String> {
    receive_applies_of(&direct.device)
}

#[test]
fn ur_26_a_value_set_on_a_stopped_stream_reaches_the_device_before_the_next_start() {
    // S1: the staged value is applied once, by the owner, between the old segment's `rx_stop` and the new `rx_start`; nothing is applied when it is booked.
    let (port, _link) = attached_reading("rx0");
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![port], json!({}));
    direct.settle(Wall::from_millis(30));
    assert_eq!(receive_applies(&direct).len(), 1, "the first segment is configured by `prepare`: {:?}", direct.device.calls());
    assert!(receive_applies(&direct)[0].contains("freq=1000000000"), "{:?}", receive_applies(&direct));
    direct.stop_at("usrp/rx/s0");
    direct.until("the receive stop", |direct| !calls(&direct.device, "rx_stop 0").is_empty());
    direct.update("radio.rx.c0.frequency_hz", Value::num(2.4e9).unwrap(), ezsdr_kernel::module_api::UpdateClass::Cold, None);
    direct.settle(Wall::from_millis(30));
    assert_eq!(receive_applies(&direct).len(), 1, "a staged value is not applied when it is booked: {:?}", direct.device.calls());
    direct.start_rx_at("usrp/rx/s0", None);
    direct.until("the second start", |direct| calls(&direct.device, "rx_start 0").len() == 2);
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty());
    let log = direct.device.calls();
    let applies = receive_applies(&direct);
    assert_eq!(applies.len(), 2, "{log:?}");
    assert!(applies[1].contains("freq=2400000000"), "{applies:?}");
    let stop = log.iter().position(|call| call.starts_with("rx_stop 0")).unwrap();
    let apply = log.iter().rposition(|call| call.starts_with("apply rx 0")).unwrap();
    let start = log.iter().rposition(|call| call.starts_with("rx_start 0")).unwrap();
    assert!(stop < apply && apply < start, "rx_stop, apply, rx_start in that order: {log:?}");
    let _ = direct.finish();
}

#[test]
fn ur_26_a_value_for_a_streaming_stream_is_refused_and_the_next_start_uses_the_old_one() {
    // S2: one `COMMAND_REJECTED` naming the Action, no device call for it, and the later start applies what the stream had.
    let (port, _link) = attached_reading("rx0");
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![port], json!({}));
    direct.settle(Wall::from_millis(30));
    let before = direct.device.calls().len();
    direct.update("radio.rx.c0.frequency_hz", Value::num(2.4e9).unwrap(), ezsdr_kernel::module_api::UpdateClass::Cold, None);
    direct.until("the refusal", |direct| !direct.of("radio.COMMAND_REJECTED").is_empty());
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].payload["action"], 1);
    assert!(rejection_reason(&rejected).starts_with("UR-26: RM-21:") && rejection_reason(&rejected).contains("streaming"), "{rejected:?}");
    assert!(!direct.device.calls()[before..].iter().any(|call| call.starts_with("apply")), "{:?}", direct.device.calls());
    direct.stop_at("usrp/rx/s0");
    direct.start_rx_at("usrp/rx/s0", None);
    direct.until("the second start", |direct| calls(&direct.device, "rx_start 0").len() == 2);
    assert!(receive_applies(&direct)[1].contains("freq=1000000000"), "the refused value was not used: {:?}", receive_applies(&direct));
    let _ = direct.finish();
}

#[test]
fn ur_26_a_stopped_stream_s_value_leaves_a_running_peer_undisturbed() {
    // S5: stream 0 stopped (its value accepted, its start applies it), stream 1 streaming (its value refused): stream 1 gets no stop, no apply, no restart.
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &rx_two(), vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.stop_at("usrp/rx/s0");
    direct.update("radio.rx.c0.frequency_hz", Value::num(2.4e9).unwrap(), ezsdr_kernel::module_api::UpdateClass::Cold, None);
    direct.update("radio.rx.c1.frequency_hz", Value::num(2.4e9).unwrap(), ezsdr_kernel::module_api::UpdateClass::Cold, None);
    direct.start_rx_at("usrp/rx/s0", None);
    direct.until("stream 0's restart", |direct| calls(&direct.device, "rx_start 0").len() == 2);
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert_eq!(rejected[0].payload["action"], 3, "stream 1's value, the third Action");
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "apply rx 1").len(), 1, "stream 1 was configured by `prepare` alone: {log:?}");
    assert_eq!(calls(&direct.device, "rx_stop 1").len(), 0, "{log:?}");
    assert_eq!(calls(&direct.device, "rx_start 1").len(), 1, "{log:?}");
    assert!(calls(&direct.device, "apply rx 0")[1].contains("freq=2400000000"), "{log:?}");
    let _ = direct.finish();
}

#[test]
fn ur_25_a_refused_start_is_fixed_by_a_stop_a_new_value_and_a_start() {
    // S6 and the label of the start's refusal: the device refuses the second start of stream 1; the refusal is `COMMAND_REJECTED` of kind `command`
    // naming the `start_rx` Action (the second Action). The stream still counts as streaming, so a corrected value is refused until a `Stop`; then
    // the value is staged and the next start applies it.
    let (mut direct, _link0) = refused_second_start_of_stream_1();
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert_eq!((rejected[0].payload["kind"].as_str(), &rejected[0].payload["action"]), (Some("command"), &serde_json::json!(2)), "{rejected:?}");
    direct.update("radio.rx.c1.frequency_hz", Value::num(2.4e9).unwrap(), ezsdr_kernel::module_api::UpdateClass::Cold, None);
    direct.until("the value's refusal", |direct| direct.of("radio.COMMAND_REJECTED").len() == 2);
    assert!(rejection_reason(&direct.of("radio.COMMAND_REJECTED")[1..]).contains("streaming"));
    direct.stop_at("usrp/rx/s1");
    direct.update("radio.rx.c1.frequency_hz", Value::num(2.4e9).unwrap(), ezsdr_kernel::module_api::UpdateClass::Cold, None);
    direct.start_rx_at("usrp/rx/s1", None);
    direct.until("the corrected start", |direct| calls(&direct.device, "rx_start 1").len() == 2);
    assert_eq!(direct.of("radio.COMMAND_REJECTED").len(), 2, "the corrected value was staged");
    assert!(calls(&direct.device, "apply rx 1").last().unwrap().contains("freq=2400000000"), "{:?}", direct.device.calls());
    let _ = direct.finish();
}

#[test]
fn ur_26_a_start_booked_ahead_is_not_revised_by_an_earlier_one_and_still_delivers() {
    // RM-21, UR-26 (Sol's sequence): stop, `start_rx` at +150 ms (uhd-rx queues it), then `start_rx` at +60 ms before that origin. The second is
    // refused as the stream already counts as streaming, so the first is neither replaced nor skipped: one restart at its origin, and samples on the new clock.
    let (port, link) = attached_reading("rx0");
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![port], json!({}));
    direct.settle(Wall::from_millis(30));
    direct.stop_at("usrp/rx/s0");
    direct.until("the receive stop", |direct| !calls(&direct.device, "rx_stop 0").is_empty());
    let now = direct.now();
    direct.start_rx_at("usrp/rx/s0", Some(now + ms(150)));
    direct.until("the first start is queued", |direct| calls(&direct.device, "rx_start 0").len() == 2);
    direct.start_rx_at("usrp/rx/s0", Some(now + ms(60)));
    direct.until("the refusal", |direct| !direct.of("radio.COMMAND_REJECTED").is_empty());
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert_eq!((rejected[0].payload["kind"].as_str(), &rejected[0].payload["action"]), (Some("command"), &serde_json::json!(3)), "the second start, the third Action");
    assert!(rejection_reason(&rejected).contains("streaming"), "{rejected:?}");
    direct.settle(Wall::from_millis(400));
    assert_eq!(calls(&direct.device, "rx_start 0").len(), 2, "one restart: {:?}", direct.device.calls());
    let rx: Vec<_> = direct.clocks.sample_clock_records().into_iter().filter(|record| record.stream.path() == "usrp/rx/s0").collect();
    assert_eq!(rx.len(), 2, "{rx:?}");
    assert!(rx[1].ended_at.is_none() && rx[1].origin.ticks_in(rx[1].root).unwrap() >= now + ms(150), "the new clock begins at the first start's origin: {rx:?}");
    let headers: Vec<_> = std::iter::from_fn(|| link.receive()).map(|(block, _)| block.header().clone()).collect();
    assert!(headers.iter().any(|header| header.first_sample_time.domain() == rx[1].domain), "samples arrive on the new clock");
    let _ = direct.finish();
}

#[test]
fn ur_26_a_scheduled_receive_value_is_refused_by_the_provider_for_its_instant() {
    // A Spec's schedule of a receive value passes validate (`cold`) and carries an instant, which a staged value never has (RM-21): refused at run time.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let mut spec = receive_spec(1, 1e6, 1e9, Some(2_000));
    spec["schedule"] = json!([{
        "at": { "clock": "radio", "offset_ticks": 1_000 },
        "action": { "kind": "update_parameter", "target": serde_json::to_value(resource("radio")).unwrap(), "key": "radio.rx.c0.frequency_hz", "value": 2.4e9 }
    }]);
    let manifest = captured(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new()));
    let refused = events_of(&manifest, "radio.COMMAND_REJECTED");
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused[0].payload["reason"].as_str().unwrap().contains("instant"), "{refused:?}");
    assert_eq!(receive_applies_of(&device).len(), 1, "the value was not applied: {:?}", device.calls());
}

fn receive_applies_of(device: &FakeDevice) -> Vec<String> {
    calls(device, "apply rx")
}

#[test]
fn load200_duplex_stage_rehearses_on_the_fake() {
    // T4's duplex stage through `run_stage` at 1 MS/s for one second, on slot A alone: the bench's judgement (`iii::stage_trial`) on the
    // fake. The rate is low; the 200 MS/s bench run is `hw_iii_load_200msps`.
    let (device, description, _) = raw_group_fixture(one_obx());
    let wire_bytes = u64::try_from(description.performance.wire_bytes_per_sample).unwrap();
    let plan = measurement::MeasurementPlan { throughput_seconds: 1, tx_waveform_samples: 2_000, ..measurement::MeasurementPlan::rehearsal() };
    let channel = |rate_hz| measurement::StreamPlan { channels: vec![0], rate_hz };
    let stage = measurement::Stage { id: "rehearse-duplex".to_owned(), family: "load200", rx: vec![channel(1_000_000)], tx: vec![channel(1_000_000)] };
    common::iii::pin_to_slot_a(&*device).unwrap();
    let result = measurement::run_stage(device, &plan, &stage, wire_bytes).unwrap();
    assert!(!result.left_open, "{}", result.observation);
    assert_eq!(common::iii::stage_trial(result.class, &result.observation), Ok(()), "{}", result.observation);
}

#[test]
fn iii_restart_fill_probe_rehearses_on_the_fake() {
    // T2's probe (`common::iii::restart_fill_probe`) on FakeDevice, on the host clock because the probe waits in real time. The fake
    // returns the transmitted tone only when the transmit and receive rates are equal (device.rs `sample_at`); at any other rate it
    // returns a ramp. So the tone and the verdict are checked at 1 MS/s alone. At every rate the rehearsal checks the calls, each first
    // block at its planned tick and the rate read-back. The fake models no DDC fill, so the zeros and ramps are not checked.
    let device = fake_real(one_obx());
    device.set_time_zero(false).unwrap();
    for fill in common::iii::restart_fill_probe(device, &common::iii::RESTART_RATES, 3) {
        for trial in &fill.trials {
            assert_eq!(trial.first_tick, Some(trial.planned), "{:.0} S/s", fill.rate);
            assert!((trial.applied - fill.rate).abs() < 1.0, "{:.0} S/s read back {}", fill.rate, trial.applied);
        }
        if fill.rate == 1e6 {
            let outcome = common::iii::restart_rate_verdict(fill.rate, &fill.trials, common::iii::RESTART_TONE_HZ, common::n47::TONE_TOLERANCE_HZ);
            assert!(matches!(outcome, common::tone::Verdict::Pass(_)), "{outcome:?}");
        }
    }
}

#[test]
fn iii_n67_cancel_probe_rehearses_on_the_fake() {
    // #67's probe trial (`common::iii::cancel_trial`, the bench's `hw_67_stop_before_origin`) on a manual-clock FakeDevice at its three rates,
    // one trial a cell, against `iii::expected_cancel`: a rule change in the fake's start/stop/close model turns it red without hardware. The
    // test thread is a participant of the device's clock (so no host delay can make the fresh start late) and the clock runs at real speed
    // while it waits (M5), so the 12 trials take about 30 s of wall time. Every verdict is exact: an INCONCLUSIVE one fails the rehearsal.
    use common::iii::{Cancel, cancel_trial, cancel_trial_verdict, expected_cancel};
    use common::tone::Verdict;
    let (device, _driver) = common::iii::driven_fake(common::one_obx());
    device.set_time_zero(false).unwrap();
    for rate in [1e6, 10e6, 100e6] {
        common::raw::raw_rx(&*device, rate, 1e9);
        for cancel in [Cancel::Control, Cancel::StopBefore, Cancel::CloseBefore, Cancel::CloseThenStop] {
            let done = cancel_trial(&*device, cancel, rate);
            let outcome = cancel_trial_verdict(&done);
            let as_expected = match (&outcome, expected_cancel(cancel)) {
                (Verdict::Pass(_), None) => true,
                (Verdict::Fail(reason), Some(text)) => reason.contains(text),
                _ => false,
            };
            assert!(as_expected, "{} at {rate} S/s: {outcome:?} (expected {:?})\n{}", cancel.name(), expected_cancel(cancel), done.trace());
        }
    }
}

/// Note 47 stage 5's Module sequences (T2's front, T4, T5) on `FakeDevice`: the functions `hardware.rs` runs on the USRP, from
/// `common::n47s5`, so the rehearsal runs the code the bench runs. The fake cannot show the tones: before a transmit its receive samples
/// are a ramp (`device.rs` `sample_at`), above the tone's power, so every tone verdict is INCONCLUSIVE there, and at 2 MS/s it loops back
/// nothing (the rates differ). A rehearsal asserts that no verdict FAILs: admission, faults, the switch row and seams, and placement are
/// judged by the same pure functions as on the bench. The tone verdicts are unit-tested in `iii.rs`.
#[test]
fn n47s5_front_rehearsal_on_the_fake() {
    let mut rehearsal = common::Rehearsal::new();
    // The first four rates only: the fake's own `profile()` has the default 1 GbE link (100 MB/s), which refuses a stream at 100 and 200 MS/s
    // (RM-32: 400 and 800 MB/s needed). The bench profile sets `link_gbps: [10]`, so `hw_n47s5_front_after_a_short_burst` runs all six.
    for rate in &common::n47s5::FRONT_RATES[..4] {
        let rate = *rate;
        for samples in [1usize, 2, 3, 4, 10, 32] {
            let dir = TempDir::new();
            let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()), Some(rate));
            let trial = common::n47s5::front_trial(run, samples, rate);
            rehearsal.trial(&format!("T2 {samples} samples at {rate} S/s"), &trial.lateness, || {
                let outcome = common::n47s5::front_outcome(&trial);
                println!("rehearsal T2 {samples} samples at {rate} S/s: {outcome:?}");
                assert!(!matches!(outcome, common::tone::Verdict::Fail(_)), "{samples} samples at {rate} S/s: {outcome:?}");
            });
        }
    }
    rehearsal.finish();
}

#[test]
fn n47s5_long_close_rehearsal_on_the_fake() {
    // The long-close trial of the bench (`hw_n47s5_a_long_close_at_100_and_200_msps`) at the rates the fake's 1 GbE profile admits: no FAIL.
    let mut rehearsal = common::Rehearsal::new();
    for rate in [1e6, 2e6] {
        let dir = TempDir::new();
        let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
        let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), Some(rate));
        let trial = common::n47s5::long_close_trial(run, &probe, rate);
        rehearsal.trial(&format!("long close at {rate} S/s"), &trial.lateness, || {
            let outcome = common::n47s5::long_close_outcome(&trial);
            println!("rehearsal long close at {rate} S/s: {outcome:?}");
            assert!(!matches!(outcome, common::tone::Verdict::Fail(_)), "{rate} S/s: {outcome:?}");
            // The shape assertion runs on the fake too: the Module closed with the held tail and one zero, and the front is the margin's three samples.
            assert_eq!(common::iii::long_close_shape(&trial.sends, trial.per, 3), Ok(()), "{rate} S/s: {:?}", trial.sends);
        });
    }
    rehearsal.finish();
}

#[test]
fn n47s5_repeat_switch_rehearsal_on_the_fake() {
    let mut rehearsal = common::Rehearsal::new();
    for timed in [false, true] {
        let dir = TempDir::new();
        let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
        let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
        let trial = common::n47s5::repeat_switch_trial(run, probe, timed);
        rehearsal.trial(&format!("T4 timed {timed}"), &trial.lateness, || {
            let outcome = common::n47s5::repeat_switch_verdict(&trial);
            println!("rehearsal T4 timed {timed}: {outcome:?}");
            assert!(!matches!(outcome, common::tone::Verdict::Fail(_)), "timed {timed}: {outcome:?}");
            // The bench's expectations (E2a-3, #86): nothing refused, the row has no `moved_to`, and no record carries a requested target,
            // since the timed switch asks 40 ms ahead, past its receipt's bound (on its own grid point), and the untimed one asks nothing.
            assert!(trial.refusals.is_empty(), "timed {timed}: {:?}", trial.refusals);
            let rows = section(&trial.manifest, "switches").as_array().unwrap().clone();
            assert_eq!(rows.len(), 1, "timed {timed}: {rows:?}");
            assert!(rows[0].get("moved_to").is_none(), "timed {timed}: {rows:?}");
            let records = bursts(&trial.manifest);
            assert_eq!(records.len(), 2, "timed {timed}: the repeat and its successor: {records:?}");
            assert!(records.iter().all(|record| record.requested_target.is_none() && record.late_by.is_none()), "timed {timed}: {records:?}");
        });
    }
    rehearsal.finish();
}

#[test]
fn n47s5_untimed_rehearsal_on_the_fake() {
    let mut rehearsal = common::Rehearsal::new();
    let device = fake(FakeConfig::default());
    let dir = TempDir::new();
    let trial = common::n47s5::untimed_repeat_trial(common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), device.clone(), None));
    rehearsal.trial("T5 (1) repeat", &trial.lateness, || {
        let outcome = common::n47s5::untimed_repeat_verdict(&trial);
        println!("rehearsal T5 (1): {outcome:?}");
        assert!(!matches!(outcome, common::tone::Verdict::Fail(_)), "repeat: {outcome:?}");
    });
    let dir = TempDir::new();
    let trial = common::n47s5::untimed_burst_trial(common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), device, None));
    rehearsal.trial("T5 (2) burst", &trial.lateness, || {
        let outcome = common::n47s5::untimed_burst_verdict(&trial);
        println!("rehearsal T5 (2): {outcome:?}");
        assert!(!matches!(outcome, common::tone::Verdict::Fail(_)), "burst: {outcome:?}");
    });
    rehearsal.finish();
}

/// T3 (f) and (b) on `FakeDevice`, the functions `hardware.rs` runs on the USRP: each trial's directory is kept until its verdict is read.
/// Every case must be PASS or INCONCLUSIVE, never FAIL: on FakeDevice no tone is shown (`n47s5_front_rehearsal_on_the_fake`), so the played
/// cases are INCONCLUSIVE at best. C at e+1 after J(e) is submitted in the window (J(e), e + off·1 µs − 2 ms] (off = +1 sample at 1 MS/s, so about
/// (e − 5 ms, e − 2 ms] with J(e) = e − 5 ms): its receipt is on time and refused (PASS), or late and dropped (INCONCLUSIVE, late at receipt or at dispatch). The late case is rehearsed too: C at e+1 submitted at e − 0.5 ms, after the
/// window, must reach the late-arrival branch (`iii::late_arrival_verdict`), not a FAIL.
#[test]
fn n47s5_held_tail_and_stop_rehearsal_on_the_fake() {
    let mut rehearsal = common::Rehearsal::new();
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let continuation = common::n47s5::held_tail_trial(run, probe, dir, 0, None);
    rehearsal.trial("T3 (f) C at e, booked before J(e)", &continuation.lateness, || {
        let outcome = common::n47s5::held_played_verdict(&continuation, 1);
        println!("rehearsal T3 (f) C at e, booked before J(e): {outcome:?}");
        assert!(!matches!(outcome, common::tone::Verdict::Fail(_)), "{outcome:?}");
    });
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let separate = common::n47s5::held_tail_trial(run, probe, dir, 4, Some(0));
    rehearsal.trial("T3 (f) C at e+4 after J(e)", &separate.lateness, || {
        let outcome = common::n47s5::held_played_verdict(&separate, 2);
        println!("rehearsal T3 (f) C at e+4 after J(e): {outcome:?}");
        assert!(!matches!(outcome, common::tone::Verdict::Fail(_)), "{outcome:?}");
    });
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let refused = common::n47s5::held_tail_trial(run, probe, dir, 1, Some(0));
    rehearsal.trial("T3 (f) C at e+1 after J(e)", &refused.lateness, || {
        let outcome = common::n47s5::held_refused_verdict(&refused);
        println!("rehearsal T3 (f) C at e+1 after J(e): {outcome:?}");
        assert!(!matches!(outcome, common::tone::Verdict::Fail(_)), "{outcome:?}");
    });
    // The two deliberately late cases: C is late by construction and the shared verdict must say so. They are judged unless something other than C's own
    // late drop is late (`lateness_without_c_late`): an injected delay that also delays A is evidence and is not judged.
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let late = common::n47s5::held_tail_trial(run, probe, dir, 1, Some(ms(9) / 2));
    rehearsal.trial("T3 (f) C at e+1 submitted at e − 0.5 ms", &late.lateness_without_c_late(), || {
        let outcome = common::n47s5::held_refused_verdict(&late);
        println!("rehearsal T3 (f) C at e+1 submitted at e − 0.5 ms: {outcome:?}");
        assert!(matches!(outcome, common::tone::Verdict::Inconclusive(ref text) if text.contains("host latency")), "{outcome:?}");
    });
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let late = common::n47s5::held_tail_trial(run, probe, dir, 4, Some(ms(9) / 2));
    rehearsal.trial("T3 (f) C at e+4 submitted at e − 0.5 ms", &late.lateness_without_c_late(), || {
        let outcome = common::n47s5::held_played_verdict(&late, 2);
        println!("rehearsal T3 (f) C at e+4 submitted at e − 0.5 ms: {outcome:?}");
        assert!(matches!(outcome, common::tone::Verdict::Inconclusive(ref text) if text.contains("host latency")), "{outcome:?}");
    });
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let stop = common::n47s5::stop_next_trial(run, probe, dir, None);
    rehearsal.trial("T3 (b) Session Stop, N1 and N2", &stop.lateness, || {
        // N1 is refused as overlapping the Provider's bound (INCONCLUSIVE at best), and the Stop's own receipt row says what it had to do; N2 is past
        // the bound of any receipt within the margin, so it plays and the shared verdict says so.
        let outcome = common::n47s5::stop_next_verdict(&stop);
        println!("rehearsal T3 (b) Session, N1: {outcome:?}");
        assert!(!matches!(outcome, common::tone::Verdict::Fail(_)), "N1: {outcome:?}");
        assert_eq!(common::n47s5::stop_n1_check(&stop), Ok(()), "N1 against the Stop's receipt");
        let after_margin = common::n47s5::stop_after_margin_verdict(&stop);
        println!("rehearsal T3 (b) Session, N2: {after_margin:?}");
        assert!(!matches!(after_margin, common::tone::Verdict::Fail(_)), "N2: {after_margin:?}");
        assert_eq!(common::n47s5::stop_n2_check(&stop), Ok(()), "N2 plays");
    });
    rehearsal.finish();
}

/// The held tail's deliberately late case reads C's own late drop and `lateness_without_c_late` leaves out exactly that and C's deadline. These counts do
/// not depend on host timing (C is submitted past its deadline by construction), so a delayed run passes too.
#[test]
fn n47s5_held_tail_late_case_leaves_out_only_cs_own_late_drop() {
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let late = common::n47s5::held_tail_trial(run, probe, dir, 1, Some(ms(9) / 2));
    assert_eq!(late.late.len(), 1, "one late drop names C's target: {:?}", late.lateness.time_errors);
    let rest = late.lateness_without_c_late();
    assert_eq!(rest.time_errors.len(), late.lateness.time_errors.len() - 1, "C's drop is left out");
    let deadline_of = |lateness: &common::iii::Lateness, what: &str| lateness.booked.iter().find(|booked| booked.what == what).map(|booked| booked.deadline);
    assert_eq!(deadline_of(&rest, "C"), Some(None), "C's deadline is left out");
    assert_eq!(deadline_of(&rest, "A"), deadline_of(&late.lateness, "A"), "A's is not");
}

/// The Session Stop's N2 submitted late on purpose (N1 on time, then the host stalls): N2 is dropped as late, which is host latency on N2's verdict
/// line (INCONCLUSIVE, naming N2) and not N1's fault (not a FAIL), as on the bench. Run through `Rehearsal::trial` with N2's own deadline and late
/// drop left out, so a run in which anything else is late is not judged (a marker) and the assertions hold for the runs that are.
#[test]
fn n47s5_stop_n2_late_drop_is_host_latency_on_both_lines() {
    let mut rehearsal = common::Rehearsal::new();
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let stop = common::n47s5::stop_next_trial(run, probe, dir, Some(ms(1) / 2));
    assert_eq!(stop.n2_late.len(), 1, "N2 is dropped as late by construction: {:?}", stop.lateness.time_errors);
    rehearsal.trial("T3 (b) Session Stop, N2 submitted late", &stop.lateness_without_n2_late(), || {
        let n1 = common::n47s5::stop_next_verdict(&stop);
        assert!(!matches!(n1, common::tone::Verdict::Fail(_)), "N2's late drop is not N1's fault: {n1:?}");
        let n2 = common::n47s5::stop_after_margin_verdict(&stop);
        assert!(matches!(&n2, common::tone::Verdict::Inconclusive(text) if text.contains("N2 was late") && text.contains("host latency")), "{n2:?}");
    });
    rehearsal.finish();
}

/// The Stop trial's late N2 reads N2's own late drop and `lateness_without_n2_late` leaves out exactly that and N2's deadline. These counts do not
/// depend on host timing (N2 is submitted past its drop point by construction, with a margin of 1.5 ms of the manual clock), so a delayed run passes
/// too; it is the direct filter of the rows that make the rehearsal above skip (a skipped trial is a marker, not a kill).
#[test]
fn n47s5_stop_n2_late_case_leaves_out_only_n2s_own_late_drop() {
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let stop = common::n47s5::stop_next_trial(run, probe, dir, Some(ms(1) / 2));
    assert_eq!(stop.n2_late.len(), 1, "one late drop names N2's target: {:?}", stop.lateness.time_errors);
    let rest = stop.lateness_without_n2_late();
    assert_eq!(rest.time_errors.len(), stop.lateness.time_errors.len() - 1, "N2's drop is left out");
    let deadline_of = |lateness: &common::iii::Lateness, what: &str| lateness.booked.iter().find(|booked| booked.what == what).map(|booked| booked.deadline);
    assert_eq!(deadline_of(&rest, "N2"), Some(None), "N2's deadline is left out");
    assert_eq!(deadline_of(&rest, "N1"), deadline_of(&stop.lateness, "N1"), "N1's is not");
    assert_eq!(deadline_of(&rest, "repeat"), deadline_of(&stop.lateness, "repeat"), "nor the repeat's");
}

/// N1's line leaves out N2's own late drop and only that: with N1's own late drop added beside it (a copy of N2's `radio.TIME_ERROR` aimed at N1's
/// sample), N1's verdict is a FAIL. N2's drop is there by construction, so the result does not depend on host timing.
#[test]
fn n47s5_stop_n1_line_keeps_n1s_own_late_drop_beside_n2s() {
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let mut stop = common::n47s5::stop_next_trial(run, probe, dir, Some(ms(1) / 2));
    assert_eq!(stop.n2_late.len(), 1, "N2 is dropped as late by construction: {:?}", stop.lateness.time_errors);
    let mut n1_drop = common::events_of(&stop.manifest, "radio.TIME_ERROR").into_iter().next().expect("N2's TIME_ERROR");
    n1_drop.payload["target"]["ticks"] = json!(stop.next_k);
    stop.manifest.events.delivered.push(n1_drop);
    let n1 = common::n47s5::stop_next_verdict(&stop);
    assert!(matches!(n1, common::tone::Verdict::Fail(_)), "N1's own late drop beside N2's is N1's FAIL: {n1:?}");
}

/// T6 (`hw_b8_stop_end`) on FakeDevice: the bench's trial and verdict. The fake shows no edge: before a transmit its receive samples are a
/// ramp of mean power about 1/3 (`device.rs` `sample_at`), above the tone's 0.16 (amplitude 0.4), so no tone floor and no quiet power follow
/// the repeat. The verdict is INCONCLUSIVE there; the rehearsal asserts that it is not a FAIL.
#[test]
fn n47s5_stop_end_rehearsal_on_the_fake() {
    let mut rehearsal = common::Rehearsal::new();
    let dir = TempDir::new();
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()), None);
    let stop_end = common::n47s5::stop_end_trial(run, dir);
    rehearsal.trial("T6 stop end", &stop_end.lateness, || {
        let outcome = common::n47s5::stop_end_outcome(&stop_end);
        println!("rehearsal T6 stop end: {outcome:?}");
        assert!(!matches!(outcome, common::tone::Verdict::Fail(_)), "{outcome:?}");
    });
    rehearsal.finish();
}

/// T3 (f)'s refusal and `hw_b8_burst_one_sample_after_a_burst`'s boundary on `FakeDevice`: the Provider refuses a booking at one to three
/// samples past the end of a burst longer than 32 samples at receipt (its event names the Action the Kernel admitted), plays the one at four
/// past, and nothing plays late. Every case must PASS, so a front the bench test no longer expects fails here first.
#[test]
fn n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake() {
    let mut rehearsal = common::Rehearsal::new();
    for (late_booking, offset) in [(false, 30_001), (true, 30_001), (false, 30_003), (true, 30_003), (false, 30_004), (true, 30_004)] {
        let dir = TempDir::new();
        let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()), None);
        let trial = common::n47s5::one_sample_after_a_burst_trial(run, offset, late_booking);
        rehearsal.trial(&format!("B8 end + {}, booked late {late_booking}", offset - 30_000), &trial.lateness, || {
            let outcome = common::n47s5::one_sample_after_a_burst_verdict(&trial);
            println!("rehearsal B8 end + {}, booked late {late_booking}: {outcome:?}", offset - 30_000);
            assert!(matches!(outcome, common::tone::Verdict::Pass(_)), "booked late {late_booking}: {outcome:?}");
        });
    }
    rehearsal.finish();
}

// ------------------------------------------------ E3a-1: rehearsals of the bench bodies whose expectations the segmentation rules fix
//
// Each runs the bench's own body (`common::e3`, shared with `hardware.rs`) in a threaded Session on `fake(FakeConfig::default())`, whose clock
// is the manual one the Session advances: the instants a body books are fixed by the device time at its submits, so no assertion here depends on
// when a thread runs. They are counted as P4 rehearsals (note 51 §3) and are no mutation row's killer; the rules they rehearse are pinned by the
// frozen-rig tests of `tx/tests/e3.rs`.

/// `hw_s1_short_first_sends`: a burst of 1, 2, 3 or 4 samples is one padded device burst of four and the next burst starts clean. The fake
/// refuses an undersized first packet (`device.rs` `MIN_FIRST_SEND`), so a send that is not padded shows as a device fault and a FAIL.
#[test]
fn s1_short_first_sends_rehearsal_on_the_fake() {
    let mut rehearsal = common::Rehearsal::new();
    for samples in [1usize, 2, 3, 4] {
        let dir = TempDir::new();
        let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()), None);
        let trial = common::e3::short_first_trial(run, samples);
        rehearsal.trial(&format!("S1 {samples} samples"), &trial.lateness, || {
            let outcome = common::e3::short_first_verdict(&trial);
            println!("rehearsal S1 {samples} samples: {outcome:?}");
            assert!(matches!(outcome, common::tone::Verdict::Pass(_)), "{samples} samples: {outcome:?}");
        });
    }
    rehearsal.finish();
}

/// `hw_txp_b0_send_sizes`: a repeat's sends are the window's quarter (2 500 samples at 1 MS/s, 12 500 at 5 MS/s, typed here, not read from the
/// Module), each timed send after the first the whole chunk, every closing send of two samples or more.
#[test]
fn txp_b0_send_sizes_rehearsal_on_the_fake() {
    for (rate, chunk) in [(1e6, 2_500usize), (5e6, 12_500)] {
        let dir = TempDir::new();
        let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
        let profile = profile(&dir, json!({}), json!({}), true);
        let initial = with_initial(rx0_tx0(&profile), &[("radio.tx.c0.gain_db", Scalar::try_from(0.0).unwrap()), ("radio.tx.s0.sample_rate_hz", Scalar::try_from(rate).unwrap())]);
        let run = session_with(&profile, probe.clone(), &initial);
        let outcome = common::e3::repeat_sizes_case(run, &probe, rate, chunk);
        println!("rehearsal TXP B0 rate {rate}: {outcome:?}");
        assert!(matches!(outcome, common::tone::Verdict::Pass(_)), "rate {rate}: {outcome:?}");
    }
}

/// `hw_p70_one_sample_burst_then_timed_burst` on its production path, as far as the fake shows it (it shows no tone, so the bench's onset
/// comparison is not rehearsed): a one-sample burst and the four-sample control are both one padded device burst of four, and the burst booked
/// with it is sent at its own booked instant, a start of its own, the same distance after the first in both.
#[test]
fn p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake() {
    let mut rehearsal = common::Rehearsal::new();
    let mut trials = Vec::new();
    for samples in [1usize, 4] {
        let dir = TempDir::new();
        let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
        let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
        let pair = common::e3::one_sample_pair(run, &probe, samples, ms(40));
        rehearsal.trial(&format!("p70 {samples} samples"), &pair.lateness, || {
            let (sends, a_root, b_root) = (&pair.sends, pair.a_root, pair.b_root);
            let timed: Vec<&Json> = sends.iter().filter(|row| row["at"].is_i64()).collect();
            let a = timed.iter().find(|row| row["at"] == a_root).unwrap_or_else(|| panic!("{samples} samples: no send at A's instant {a_root}: {sends:?}"));
            assert_eq!((a["sample_counts"][0].as_u64(), a["sob"].as_bool(), a["eob"].as_bool()), (Some(4), Some(true), Some(true)), "{samples} samples: A is one padded device burst of four");
            let b = timed.iter().find(|row| row["at"] == b_root).unwrap_or_else(|| panic!("{samples} samples: no send at B's instant {b_root}: {sends:?}"));
            assert_eq!(b["sob"].as_bool(), Some(true), "{samples} samples: B starts its own device burst");
            trials.push((samples, b_root - a_root));
        });
    }
    // After `finish`, so that a trial that was not judged is the marker and not a failure of the comparison.
    rehearsal.finish();
    if let [a, b] = trials[..] {
        assert_eq!(a.1, b.1, "B is the same distance after A with a one-sample A and with the four-sample control: {trials:?}");
    } else {
        panic!("expected 2 trials: {trials:?}");
    }
}

/// `one_sample_pair` hands over the Manifest its `time_errors` were read from (note 51 stage 2c, minor a): p70's wrapper reads its lateness from there.
/// A is booked 1 ms ahead, past its deadline (the device lead, 2 ms), so the Provider drops it as late by construction and the Manifest has a
/// TIME_ERROR whatever the host does. What B did and whether A's burst row exists depend on the host, so they are not asserted.
#[test]
fn one_sample_pair_returns_the_manifest_its_time_errors_were_read_from() {
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let run = common::n47s5::transmit_run(&profile(&dir, json!({}), json!({}), true), probe.clone(), None);
    let pair = common::e3::one_sample_pair(run, &probe, 1, ms(1));
    assert!(pair.lateness.time_errors.iter().any(|(cause, outcome)| cause == "late" && outcome == "drop"), "A was dropped as late: {:?}", pair.lateness.time_errors);
    assert_eq!(pair.lateness.time_errors, common::iii::time_error_pairs(&pair.manifest));
    assert_eq!(pair.lateness.booked.iter().map(|booked| booked.what).collect::<Vec<_>>(), ["A", "B"]);
}

/// `hw_p72_timed_tx_chunks_are_timed_and_flushed`: every send of a 60 ms device burst carries its time (T1), every closing send has at least two
/// samples (T2), the burst's last sample is the held tail closed by one send of two samples, and the flush is acknowledged (T3: the fake does
/// acknowledge a flush, so the verdict is PASS; the acknowledgement is the device's behaviour, not a segmentation rule).
#[test]
fn p72_timed_tx_chunks_rehearsal_on_the_fake() {
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig::default())));
    let (spec, inputs) = common::e3::timed_chunks_spec(1e9);
    let manifest = captured(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), probe.clone(), inputs));
    let trial = common::e3::timed_chunks_case(&probe, &manifest);
    println!("rehearsal #72 1a: {:?}", trial.verdict);
    assert!(matches!(trial.verdict, common::tone::Verdict::Pass(_)), "T1 (every send timed), T2 (closing sends of two samples or more) and T3: {:?}", trial.verdict);
    assert_eq!(trial.closing, [json!([2])], "the held tail closes the burst with one send of two samples, not a direct close");
}

/// `hw_p72_…`'s verdict on a Session whose first flush (the open flush at `arm`, the first send that carries samples) is not acknowledged: a flush
/// without its `BurstAck` is a refused flush (UR-22), so T3 fails by name, whatever the retry rule does above it (#75: the open flush at `arm` is one
/// attempt while `OPEN_FLUSH_CONFIRMED` is off, and a recovery's retried flush rows would still count as refused).
#[test]
fn p72_a_lost_flush_ack_fails_the_verdict_on_the_fake() {
    let dir = TempDir::new();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(fake(FakeConfig { faults: vec![FakeFault::DropAckOfSend(0)], ..FakeConfig::default() })));
    let (spec, inputs) = common::e3::timed_chunks_spec(1e9);
    let manifest = captured(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), probe.clone(), inputs));
    let trial = common::e3::timed_chunks_case(&probe, &manifest);
    println!("rehearsal #72 1a, a lost flush acknowledgement: {:?}", trial.verdict);
    assert!(matches!(&trial.verdict, common::tone::Verdict::Fail(why) if why.contains("T3: 1 flush(es) refused")), "{:?}", trial.verdict);
}

// ------------------------------------------------ #92: a uhd-control panic is a DEVICE_LOST (UR-16)

/// A device whose `time_now` panics on uhd-control's thread once armed: the panic is put there by the test (UHD does not panic in `time_now`),
/// and reaches uhd-control at its next 500 ms device check.
struct PanicsInControl {
    inner: Arc<FakeDevice>,
    armed: Arc<std::sync::atomic::AtomicBool>,
}

impl ezsdr_radio_uhd::Device for PanicsInControl {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, d: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(d)
    }
    fn front_end(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.front_end(d, c)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        if self.armed.load(std::sync::atomic::Ordering::SeqCst) && std::thread::current().name() == Some("uhd-control") {
            panic!("fake: time_now panics on uhd-control");
        }
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.inner.apply(d, c, s)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
}

/// An Authority whose time panics on uhd-control's thread, once, when armed: a panic inside `book` and under the line lock, where `book` reads
/// the time (`DeviceAuthority::now` reads no device, so no device wrapper reaches it). The panic is injected, not evidence that the Authority
/// panics.
struct PanicsInBook {
    inner: DeviceAuthority,
    time: Arc<PanickingTime>,
}

struct PanickingTime {
    inner: Arc<dyn ezsdr_kernel::time::TimeAuthority>,
    armed: Arc<std::sync::atomic::AtomicBool>,
}

impl ezsdr_kernel::time::TimeAuthority for PanickingTime {
    fn primary_root(&self) -> ClockDomainId {
        self.inner.primary_root()
    }
    fn pacing(&self) -> ezsdr_kernel::module_api::Pacing {
        self.inner.pacing()
    }
    fn governs(&self, domain: ClockDomainId) -> bool {
        self.inner.governs(domain)
    }
    fn now(&self, domain: ClockDomainId) -> Result<TimePoint, ezsdr_kernel::time::TimeError> {
        if std::thread::current().name() == Some("uhd-control") && self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            panic!("fake: the Authority's time panics on uhd-control");
        }
        self.inner.now(domain)
    }
    fn wait_until(&self, t: TimePoint) -> Result<(), ezsdr_kernel::time::TimeError> {
        self.inner.wait_until(t)
    }
    fn schedule(&self, t: TimePoint, f: Box<dyn FnOnce(TimePoint) + Send>) -> Result<ezsdr_kernel::time::ScheduleHandle, ezsdr_kernel::time::TimeError> {
        self.inner.schedule(t, f)
    }
    fn cancel(&self, h: ezsdr_kernel::time::ScheduleHandle) -> bool {
        self.inner.cancel(h)
    }
}

impl Authority for PanicsInBook {
    fn descriptor(&self) -> &ezsdr_kernel::module_api::AuthorityDescriptor {
        self.inner.descriptor()
    }
    fn time(&self) -> Arc<dyn ezsdr_kernel::time::TimeAuthority> {
        self.time.clone()
    }
    fn next_wakeup(&self) -> Option<TimePoint> {
        self.inner.next_wakeup()
    }
    fn relations(&self) -> Vec<ezsdr_kernel::time::ClockRelation> {
        self.inner.relations()
    }
}

fn panicking_control_device(config: FakeConfig) -> (Arc<PanicsInControl>, Arc<std::sync::atomic::AtomicBool>) {
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    (Arc::new(PanicsInControl { inner: fake(config), armed: armed.clone() }), armed)
}

/// The Assembly of a Session whose uhd-control panics in `book`, holding the line, once the returned flag is set: a submit that waits for it
/// ends the Run by `DEVICE_LOST`.
fn panicking_book_assembly(profile: &Json, device: Arc<FakeDevice>) -> (ezsdr_kernel::coordinator::Assembly, Arc<std::sync::atomic::AtomicBool>) {
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook = armed.clone();
    let assembly = common::assembly_with_authority(
        profile,
        device,
        BTreeMap::new(),
        |r| r,
        move |authority| {
            let time = Arc::new(PanickingTime { inner: ezsdr_kernel::module_api::Authority::time(&authority), armed: hook });
            Box::new(PanicsInBook { inner: authority, time })
        },
    );
    (assembly, armed)
}

/// What a Run that lost its device to a uhd-control panic (UR-16) shows, whichever way it ended: one fatal `DEVICE_LOST` with the panic's message,
/// the panic in `rejected`, nothing that did not join, the device marked lost before it was closed, the plan once (UR-30), and every stream's clock
/// ended (a transmit clock that no Stop cut stays open after a loss of the device itself, so the check is of the receive ones).
fn assert_a_control_panic_ended_the_run(manifest: &Manifest, device: &FakeDevice) {
    let lost = events_of(manifest, EventKind::DEVICE_LOST);
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert_eq!((lost[0].payload["message"].as_str(), lost[0].severity), (Some("a Provider thread panicked"), ezsdr_kernel::event::Severity::Fatal));
    assert_eq!(lost[0].source, usrp());
    let rejected = section(manifest, "rejected").as_array().unwrap();
    assert!(rejected.iter().any(|row| row["thread"] == "uhd-control" && row["reason"] == "panicked"), "{rejected:?}");
    assert!(!rejected.iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("did not join"))), "{rejected:?}");
    assert_marked_lost_before_closed(device);
    plan_row(section(manifest, "timing"));
    assert_the_receive_clocks_ended(&manifest.clocks.sample_clocks, false);
}

/// Every receive clock the Run registered is ended; `registered` also requires one (uhd-rx registers it at its first block, which a Run
/// that is lost at once may not have reached).
fn assert_the_receive_clocks_ended(records: &[ezsdr_kernel::time::SampleClockRecord], registered: bool) {
    let receive: Vec<_> = records.iter().filter(|record| record.stream.path().starts_with("usrp/rx")).collect();
    assert!(!(registered && receive.is_empty()) && receive.iter().all(|record| record.ended_at.is_some()), "{records:?}");
}

/// The Run ends at the loss under the default Policy: its termination is the Policy's, not KC-21a's budget or a horizon.
fn assert_the_loss_ended_it(manifest: &Manifest) {
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
}

#[test]
fn ur_16_a_uhd_control_panic_ends_the_run_with_a_lost_device() {
    // The issue's sequence: a repeat plays, uhd-control panics at its next device check, and the Run ends at the loss, under the default Policy.
    let run_once = |loss: Option<Wall>| {
        let faults = loss.map(|at| vec![FakeFault::Lost(at)]).unwrap_or_default();
        let (tested, armed) = panicking_control_device(FakeConfig { faults, ..FakeConfig::default() });
        let (inner, probe) = (tested.inner.clone(), tested.inner.clone());
        let manifest = bounded("a Run whose uhd-control panics", Wall::from_secs(30), move || {
            let (run, _dir) = tx_session_on(tested);
            let mut run = run.expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
            let _ = send(&mut run, "start_repeat", Some(ms(8)), &tone(100));
            // The repeat is playing once the device has its first experiment send (not arm's four-zero flush).
            while tx_experiment_sends(&probe, "tx_send").is_empty() {
                wait(&mut run, ms(5));
            }
            if loss.is_none() {
                armed.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            let result = run.run_until_end(after(&run, ms(5_000)));
            assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
            run.finish()
        });
        (manifest, inner)
    };
    let (manifest, device) = run_once(None);
    assert_the_loss_ended_it(&manifest);
    let rejected = section(&manifest, "rejected").as_array().unwrap();
    assert!(rejected.iter().any(|row| row["thread"] == "uhd-control" && row["reason"] == "panicked"), "{rejected:?}");
    assert!(!rejected.iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("did not join"))), "{rejected:?}");
    assert_a_control_panic_ended_the_run(&manifest, &device);
    // A panic is no weaker an end than the loss the device itself reports: the same Run with `FakeFault::Lost` ends the same way, with the
    // same streams ended.
    let (reference, _) = run_once(Some(Wall::from_millis(2_300)));
    assert_eq!(reference.termination.reason, manifest.termination.reason);
    assert_eq!(
        reference.clocks.sample_clocks.iter().map(|record| record.ended_at.is_some()).collect::<Vec<_>>(),
        manifest.clocks.sample_clocks.iter().map(|record| record.ended_at.is_some()).collect::<Vec<_>>()
    );
}

#[test]
fn ur_16_a_uhd_control_panic_with_no_stream_sending_ends_the_run() {
    let (device, armed) = panicking_control_device(FakeConfig::default());
    let inner = device.inner.clone();
    let manifest = bounded("an idle Run whose uhd-control panics", Wall::from_secs(30), move || {
        let (run, _dir) = tx_session_on(device);
        let mut run = run.expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
        armed.store(true, std::sync::atomic::Ordering::SeqCst);
        let result = run.run_until_end(after(&run, ms(5_000)));
        assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
        run.finish()
    });
    assert_the_loss_ended_it(&manifest);
    assert_a_control_panic_ended_the_run(&manifest, &inner);
    // Only `arm`'s four-zero flush (UR-13) was sent.
    assert_eq!(calls(&inner, "tx_send"), ["tx_send 0 n=4 at=- sob=true eob=true"], "{:?}", inner.calls());
}

#[test]
fn ur_16_a_uhd_control_panic_while_a_submit_waits_ends_it_with_the_loss() {
    // KC-21a's own sequence: the Session's Stop is dispatched to uhd-control and the submit waits for it to be taken; uhd-control panics in
    // `book`, holding the line, so the Action is never acknowledged. The loss, not the 5 s budget, ends the Run.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let profile = profile(&dir, json!({}), json!({}), true);
    let (assembly, armed) = panicking_book_assembly(&profile, device.clone());
    let manifest = bounded("a submit that waits on a panicking uhd-control", Wall::from_secs(30), {
        let device = device.clone();
        move || {
            let mut run = session_on(&profile, &rx0_tx0(&profile), assembly).expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
            past_t0(&mut run, ms(1));
            past_tx_origin(&mut run);
            let _keep = device;
            armed.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = run.submit(SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None);
            run.finish()
        }
    });
    assert_the_loss_ended_it(&manifest);
    assert_a_control_panic_ended_the_run(&manifest, &device);
}

#[test]
fn ur_16_a_uhd_control_panic_under_a_continuing_policy_still_stops_the_repeat() {
    // A Policy that continues past `DEVICE_LOST` leaves the Run running: the Provider's own loss path must stop new work and end the
    // streams, before anything shuts the Provider down (Oracle Q of the test plan).
    const TX_CALLS: [&str; 7] = ["tx_send", "tx_open", "tx_close", "rx_open", "rx_start", "rx_recv", "close_streams"];
    // What one pass of each owner can still do after it read the device as healthy: uhd-tx's send and its held tail (tx.rs), and one receive
    // call of each of the two receive owners.
    const IN_FLIGHT: usize = 4;
    let new_work = |calls: &[String], after: usize| calls[after + 1..].iter().filter(|call| TX_CALLS.iter().any(|name| call.split(' ').next() == Some(name))).count();
    let send_size = |call: &String| call.split(' ').find_map(|part| part.strip_prefix("n=")).and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);

    let dir = TempDir::new();
    let (device, armed) = panicking_control_device(FakeConfig::default());
    let inner = device.inner.clone();
    let (bytes, waveform) = waveform_of(&tone(1_000));
    let mut spec = with_burst(with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &waveform, true, "send_asap_and_flag", 1_000);
    spec["policies"] = json!({ "failure": { "ezsdr.DEVICE_LOST": "continue" } });
    let doc = profile(&dir, json!({}), json!({}), false);
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes)]);
    let manifest = bounded("a Run that continues past a uhd-control panic", Wall::from_secs(60), {
        let inner = inner.clone();
        move || {
            let mut run = spec_run(&spec, &doc, device, inputs);
            running(&run);
            past_t0(&mut run, ms(300));
            while tx_experiment_sends(&inner, "tx_send").is_empty() {
                wait(&mut run, ms(5));
            }
            armed.store(true, std::sync::atomic::Ordering::SeqCst);
            while !inner.calls().iter().any(|call| call == "mark_lost") {
                wait(&mut run, ms(20));
            }
            let at_loss = inner.calls();
            let marked = at_loss.iter().position(|call| call == "mark_lost").unwrap();
            let largest = at_loss[..marked].iter().filter(|call| call.starts_with("tx_send")).map(send_size).max().unwrap_or(0);
            // Quiescence: the first window may hold what was already admitted; the second may not hold anything new.
            wait(&mut run, ms(50));
            let first = inner.calls();
            wait(&mut run, ms(50));
            let second = inner.calls();
            assert!(new_work(&first, marked) <= IN_FLIGHT, "{first:?}");
            assert_eq!(new_work(&second, marked), new_work(&first, marked), "new work was initiated after the owners could see the loss: {second:?}");
            let played = |calls: &[String]| calls[marked + 1..].iter().filter(|call| call.starts_with("tx_send")).map(send_size).sum::<usize>();
            assert!(played(&second) <= IN_FLIGHT * largest, "{second:?}");
            // The streams ended at the loss, before any stop of the Provider.
            assert_the_receive_clocks_ended(&run.sample_clocks(), true);
            run.finish()
        }
    });
    assert_a_control_panic_ended_the_run(&manifest, &inner);
}

// ---------------------------------------------------------------- the `Direct` rig (note 51 stage 1c)

fn one_stream_direct(config: FakeConfig) -> Direct {
    Direct::build(config, &[], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)], json!({}))
}

#[test]
fn direct_rig_refuses_a_fault_that_stops_the_manual_clock_for_good() {
    let refused = std::panic::catch_unwind(|| one_stream_direct(FakeConfig { faults: vec![FakeFault::TxBlocks], ..FakeConfig::default() }));
    let text = refused.err().and_then(|payload| payload.downcast_ref::<String>().cloned().or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_owned()))).expect("the constructor panics");
    assert!(text.contains("TxBlocks"), "{text}");
}

/// Runs the fixture `name` in a child process (the marker is written to the process's own stderr, so only a child shows it) and returns its stderr.
fn run_fixture(name: &str) -> String {
    let out = std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", name, "--test-threads=1", "--nocapture"]).env("EZSDR_RIG_FIXTURE", name).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success(), "the fixture fails: {}", unmarked(&stderr));
    stderr
}

/// The child's stderr as a failure message: its marker line is the child's, and a parent that printed it would be counted by the runner as a bound that expired in this test.
fn unmarked(stderr: &str) -> String {
    stderr.replace("EZSDR-INFRA", "child's marker")
}

fn fixture_runs(name: &str) -> bool {
    std::env::var("EZSDR_RIG_FIXTURE").is_ok_and(|fixture| fixture == name)
}

#[test]
fn direct_rig_until_writes_the_infra_marker_when_its_real_time_bound_expires() {
    let stderr = run_fixture("direct_rig_fixture_until_real_bound");
    assert!(stderr.contains("EZSDR-INFRA: direct_rig_fixture_until_real_bound: the rig's wait for an event that never comes did not end within"), "{stderr}");
}

/// Run by the test above in a child process; a no-op otherwise.
#[test]
fn direct_rig_fixture_until_real_bound() {
    if fixture_runs("direct_rig_fixture_until_real_bound") {
        one_stream_direct(FakeConfig::default()).until_within("an event that never comes", Wall::from_secs(60), Wall::from_millis(50), |_| false);
    }
}

#[test]
fn direct_rig_until_fails_at_its_device_time_horizon_without_a_marker() {
    let stderr = run_fixture("direct_rig_fixture_until_horizon");
    assert!(stderr.contains("an event that never comes did not come within"), "{}", unmarked(&stderr));
    assert!(!stderr.contains("EZSDR-INFRA"), "the horizon is the test's verdict, not the host's: {}", unmarked(&stderr));
}

/// Run by the test above in a child process; a no-op otherwise.
#[test]
fn direct_rig_fixture_until_horizon() {
    if fixture_runs("direct_rig_fixture_until_horizon") {
        one_stream_direct(FakeConfig::default()).until_within("an event that never comes", Wall::from_millis(50), Wall::from_secs(5), |_| false);
    }
}

/// A rig that is built but not started: nothing has run on its device clock past `prepare` and `arm`.
fn armed_direct(config: FakeConfig) -> Direct {
    Direct::armed(config, &[], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)], json!({}))
}

/// A Stop whose target is refused at its receipt (UR-26), so the `COMMAND_REJECTED` event's time is the instant the Action was received.
fn push_refused_stop(direct: &Direct) {
    direct.stop_at("usrp/gpio");
}

fn receipts(direct: &mut Direct) -> Vec<i64> {
    let root = direct.root;
    direct.of("radio.COMMAND_REJECTED").iter().map(|event| event.time.ticks_in(root).unwrap()).collect()
}

fn refusals(direct: &mut Direct) -> usize {
    direct.of("radio.COMMAND_REJECTED").len()
}

#[test]
fn direct_rig_the_device_clock_stands_still_while_the_test_thread_works_after_each_wait() {
    let mut direct = one_stream_direct(FakeConfig::default());
    let host = direct.device.host();
    assert_stands_still(&host, "built");
    direct.settle(Wall::from_millis(5));
    assert_stands_still(&host, "settle");
    push_refused_stop(&direct);
    direct.until("the refusal", |direct| refusals(direct) == 1);
    assert_stands_still(&host, "until");
    let tick = direct.now() + ms(3);
    direct.wait_until(tick);
    assert_stands_still(&host, "wait_until");
    let _ = direct.finish();
    let gone = host.now();
    std::thread::sleep(Wall::from_millis(20));
    assert!(host.now() > gone, "the clock runs again once the rig is gone");
}

#[test]
fn direct_rig_settle_advances_the_device_clock_by_its_duration_and_wait_until_reaches_its_tick() {
    let mut direct = armed_direct(FakeConfig::default());
    let host = direct.device.host();
    let before = host.now();
    direct.settle(Wall::from_millis(7));
    assert_eq!(host.now() - before, Wall::from_millis(7) + Wall::from_micros(1), "settle is the driver's own idle wait on the device clock, and ends one microsecond past its duration (R1)");
    let tick = direct.now() + ms(3);
    direct.wait_until(tick);
    let reached = direct.now();
    // One microsecond past the tick (a tick of the root clock is 5 ns, and the conversion to the host clock rounds by one): every participant due at the tick has run before the test continues.
    assert!(reached >= tick + ms(1) / 1_000 - 1 && reached < tick + ms(1), "wait_until returns just after its tick: {reached} for {tick}");
    // `start` books its first instant 2 s ahead and waits for it (UR-15): the same rule, on the one wait it makes.
    let begun = direct.now();
    direct.start();
    let started = direct.now();
    assert!(started >= begun + ms(2_002) + ms(1) / 1_000 - 1 && started < begun + ms(2_002) + ms(1), "start returns just after its first instant: {started} for {begun}");
    direct.finish();
}

#[test]
fn direct_rig_an_action_is_not_received_at_the_instant_it_was_pushed() {
    let mut direct = armed_direct(FakeConfig::default());
    push_refused_stop(&direct);
    assert!(direct.queue.recv().is_none(), "the driver is busy, so the clock stands at the instant of the push: a poll at it must not take the Action");
    direct.settle(Wall::from_micros(1));
    assert!(direct.queue.recv().is_some(), "the clock has moved past the push");
}

#[test]
fn direct_rig_an_action_pushed_after_wait_until_is_received_at_the_first_poll_after_its_tick() {
    let mut direct = armed_direct(FakeConfig::default());
    // uhd-control is spawned inside `start`, at an instant the clock stands at while this thread works, so its polls are at `s0 + k ms`: each tick below is an instant at which a poll is due, and
    // the receipt is the next one, every time, never the same instant.
    let s0 = direct.now();
    direct.radio.start(Some(TimePoint::new(direct.root, s0 + ms(2_001)))).unwrap();
    let ticks: Vec<i64> = (0..60).map(|k| 10 + 3 * k).collect();
    for tick in &ticks {
        direct.wait_until(s0 + ms(*tick));
        push_refused_stop(&direct);
    }
    direct.until("every refusal", |direct| refusals(direct) == ticks.len());
    for (receipt, tick) in receipts(&mut direct).iter().zip(&ticks) {
        // A receipt's tick is the Authority's extrapolation of the host clock: the nanosecond of the poll converted to root ticks, which rounds by one tick.
        assert!(receipt.abs_diff(s0 + ms(tick + 1)) <= 1, "the Stop pushed at {tick} ms is received at the poll one millisecond after it: {receipt} for {}", s0 + ms(tick + 1));
    }
    direct.finish();
}

#[test]
fn direct_rig_a_wait_that_ends_on_a_poll_instant_still_receives_at_the_next_poll() {
    let mut direct = armed_direct(FakeConfig::default());
    let host = direct.device.host();
    let (s0, g0) = (direct.now(), host.now());
    direct.radio.start(Some(TimePoint::new(direct.root, s0 + ms(2_001)))).unwrap();
    let mut pushed_on = Vec::new();
    for k in 1..=12usize {
        // The next poll instant at least 2 ms ahead. `settle` ends one microsecond past its duration (R1): this one ends exactly on the poll, where the driver's wake and the poll share the instant.
        let m = (host.now() - g0).as_millis() as u64 + 2;
        let grid = g0 + Wall::from_millis(m);
        direct.settle(grid - Wall::from_micros(1) - host.now());
        assert_eq!(host.now(), grid, "the wait ended on a poll instant");
        push_refused_stop(&direct);
        pushed_on.push(m as i64);
        direct.until("the refusal", |direct| refusals(direct) == k);
    }
    for (receipt, m) in receipts(&mut direct).iter().zip(&pushed_on) {
        assert!(receipt.abs_diff(s0 + ms(m + 1)) <= 1, "the Stop pushed at poll {m} is received at the poll after it: {receipt} for {}", s0 + ms(m + 1));
    }
    direct.finish();
}

#[test]
fn direct_rig_pushes_after_the_test_thread_was_away_are_received_at_the_same_instant() {
    let mut direct = one_stream_direct(FakeConfig::default());
    push_refused_stop(&direct);
    std::thread::sleep(Wall::from_millis(200));
    push_refused_stop(&direct);
    direct.until("both refusals", |direct| refusals(direct) == 2);
    let received = receipts(&mut direct);
    assert_eq!(received[0], received[1], "the clock stood while the test thread was away: {received:?}");
    direct.finish();
}

#[test]
fn direct_rig_two_rigs_on_one_thread_move_only_the_clock_the_thread_waits_on() {
    let mut a = armed_direct(FakeConfig::default());
    let mut b = armed_direct(FakeConfig::default());
    let (host_a, host_b) = (a.device.host(), b.device.host());
    let (a0, b0) = (host_a.now(), host_b.now());
    a.settle(Wall::from_millis(7));
    assert_eq!(host_a.now() - a0, Wall::from_millis(7) + Wall::from_micros(1), "A moved by the wait on A");
    assert_eq!(host_b.now(), b0, "B stood still while the thread waited on A");
    let a1 = host_a.now();
    b.settle(Wall::from_millis(5));
    assert_eq!(host_b.now() - b0, Wall::from_millis(5) + Wall::from_micros(1), "B moved by the wait on B");
    assert_eq!(host_a.now(), a1, "A stood still while the thread waited on B");
}

fn direct_with_uhd_clock_asleep_inside_its_read() -> Direct {
    let direct = armed_direct(FakeConfig { faults: vec![FakeFault::Unreachable(common::rig::FAULT_AT)], ..FakeConfig::default() });
    let host = direct.device.host();
    common::rig::asleep_inside_its_read(
        direct,
        &host,
        |direct| Arc::strong_count(&direct.time),
        |direct, wall| {
            direct.settle(wall);
        },
    )
}

#[test]
fn direct_rig_dropped_joins_uhd_clock_even_asleep_inside_its_read() {
    let mut direct = direct_with_uhd_clock_asleep_inside_its_read();
    let seen = direct._authority.install_probe(&direct.time, direct.device.host());
    let before = direct.device.host().now();
    drop(direct);
    assert_joined(&seen, before, "dropped");
}

#[test]
fn direct_rig_unwound_joins_uhd_clock_even_asleep_inside_its_read() {
    let probe = Mutex::new(None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut direct = direct_with_uhd_clock_asleep_inside_its_read();
        let seen = direct._authority.install_probe(&direct.time, direct.device.host());
        *probe.lock().unwrap() = Some((seen, direct.device.host().now()));
        panic!("the test body fails");
    }));
    assert!(result.is_err());
    let (seen, before) = probe.into_inner().unwrap().unwrap();
    assert_joined(&seen, before, "unwound");
}

// ---------------------------------------------------------------- the Run wrapper (note 51 stage 2a)

/// The panic message of a caught panic.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<String>().cloned().or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_owned())).unwrap_or_default()
}

/// A transmit Session whose uhd-control panics at its next device check once the flag is set: its Run then ends by `DEVICE_LOST`, a Policy end
/// that nothing declared.
fn doomed_session() -> (Run, Arc<std::sync::atomic::AtomicBool>, TempDir) {
    let (device, armed) = panicking_control_device(FakeConfig::default());
    let (run, dir) = tx_session_on(device);
    (run, armed, dir)
}

/// `call` ends `run` with an end nobody declared: the call itself panics with `ended unexpectedly` (the caller owns the Run outside the closure,
/// so a later `Drop` cannot be what panics), and the Run still hands out its Manifest without a second panic. The test expects no bound and so
/// captures no marker: one that a bound writes anyway reaches the output, and the message assertion fails beside it.
fn assert_the_call_that_ends_an_undeclared_run_panics(mut run: Run, call: impl FnOnce(&mut Run)) {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(&mut run)));
    let message = panic_text(&*caught.expect_err("the call that returned the end panicked"));
    assert!(message.contains("ended unexpectedly"), "{message}");
    assert_the_loss_ended_it(&run.finish());
}

#[test]
fn run_a_policy_end_is_judged_by_submit() {
    let dir = TempDir::new();
    let profile = profile(&dir, json!({}), json!({}), true);
    let (assembly, armed) = panicking_book_assembly(&profile, fake(FakeConfig::default()));
    let mut run = session_on(&profile, &rx0_tx0(&profile), assembly);
    past_t0(&mut run, ms(1));
    past_tx_origin(&mut run);
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_the_call_that_ends_an_undeclared_run_panics(run, |run| {
        let _ = run.submit(SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None);
    });
}

#[test]
fn run_a_policy_end_is_judged_by_advance_to() {
    let (run, armed, _dir) = doomed_session();
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_the_call_that_ends_an_undeclared_run_panics(run, |run| {
        let _ = run.advance_to(after(run, ms(5_000)));
    });
}

#[test]
fn run_a_policy_end_is_judged_by_wait_for() {
    let (run, armed, _dir) = doomed_session();
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_the_call_that_ends_an_undeclared_run_panics(run, |run| {
        let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, after(run, ms(5_000)));
    });
}

#[test]
fn run_a_policy_end_is_judged_by_run_until_end() {
    let (run, armed, _dir) = doomed_session();
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_the_call_that_ends_an_undeclared_run_panics(run, |run| {
        let _ = run.run_until_end(after(run, ms(5_000)));
    });
}

/// A real-clock Run whose transmit thread is blocked in a device call (`TxBlocks`), and whose uhd-control panics once the flag is set: the loss is
/// the declared Policy end, and the Provider's join of the blocked thread is the bound that cuts. As `wedged`, it gives the transmit thread 300 ms
/// of real time after T0 to enter its first timed send.
fn wedged_run_that_loses_its_device(dir: &TempDir) -> Run {
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let inner = fake_real(FakeConfig { faults: vec![FakeFault::TxBlocks], ..FakeConfig::default() });
    let device = Arc::new(PanicsInControl { inner, armed: armed.clone() });
    let (bytes, waveform) = waveform_of(&tone(1_000));
    let spec = with_burst(with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &waveform, true, "send_asap_and_flag", 1_000);
    let mut run = spec_run(&spec, &profile(dir, json!({}), json!({}), false), device, BTreeMap::from([(waveform.hash.clone(), bytes)])).expecting(Expect::Policy("ezsdr.DEVICE_LOST"));
    past_t0(&mut run, ms(300));
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    run
}

/// `call` ends a declared Run whose cleanup meets a bound: exactly one marker, written by the call, and one panic naming the bound.
fn assert_the_call_that_meets_a_bound_marks_it(call: impl FnOnce(&mut Run)) {
    let dir = TempDir::new();
    let mut run = wedged_run_that_loses_its_device(&dir);
    let (caught, markers) = capture_markers(&["join"], || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(&mut run))));
    let message = panic_text(&*caught.expect_err("the call that returned the end panicked"));
    assert!(message.contains("met join"), "{message}");
    assert_eq!(markers.len(), 1, "{markers:?}");
    assert_the_loss_ended_it(&run.finish());
}

#[test]
fn run_a_bound_is_marked_by_advance_to() {
    assert_the_call_that_meets_a_bound_marks_it(|run| {
        let _ = run.advance_to(after(run, ms(5_000)));
    });
}

#[test]
fn run_a_bound_is_marked_by_wait_for() {
    assert_the_call_that_meets_a_bound_marks_it(|run| {
        let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, after(run, ms(5_000)));
    });
}

#[test]
fn run_a_bound_is_marked_by_run_until_end() {
    assert_the_call_that_meets_a_bound_marks_it(|run| {
        let _ = run.run_until_end(after(run, ms(5_000)));
    });
}

#[test]
fn run_a_declared_end_answers_after_the_end() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::Unlocked(Wall::from_millis(2_300))], ..FakeConfig::default() });
    let profile = profile(&dir, json!({ "clock_source": "external" }), json!({}), false);
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile, device.clone(), BTreeMap::new()).expecting(Expect::Policy("radio.CLOCK_LOST"));
    let Err(RunHandleError::Ended { termination }) = run.run_until_end(after(&run, ms(5_000))) else { panic!("the Run did not end") };
    assert_eq!(termination, Termination::Stopped { cause: StopCause::Policy { event: kind("radio.CLOCK_LOST") } });
    assert!(matches!(run.state(), RunState::CleanedUp { termination: ref ended } if *ended == termination), "{:?}", run.state());
    let horizon = after(&run, ms(1_000));
    assert!(matches!(run.submit(SessionAction::Stop { target: None }, None), Err(RunHandleError::Ended { termination: ref ended }) if *ended == termination));
    assert!(matches!(run.advance_to(horizon), Err(RunHandleError::Ended { termination: ref ended }) if *ended == termination));
    assert!(matches!(run.wait_for(&[kind("radio.CLOCK_LOST")], 0, horizon), Err(RunHandleError::Ended { termination: ref ended }) if *ended == termination));
    assert!(matches!(run.run_until_end(horizon), Err(RunHandleError::Ended { termination: ref ended }) if *ended == termination));
    let (delivered, skipped) = (run.events(0), run.events(1));
    let manifest = run.finish();
    assert_eq!(delivered, manifest.events.delivered);
    assert!(delivered.len() == 1 && skipped.is_empty(), "{delivered:?} {skipped:?}");
    assert_eq!(manifest.termination.reason, termination);
    assert_eq!(calls(&device, "close_streams").len(), 1, "the Run was finished once");
}

/// Keeps a `Gated` device open past the end of a test whose Run is dropped while the gate is closed.
struct Release(Arc<Gated>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// A transmit Session whose receive and transmit threads are held inside a device call: its Run cannot be stopped within the Provider's join.
fn held_session(device: &Arc<Gated>) -> (Run, TempDir) {
    let (mut run, dir) = tx_session_on(device.clone());
    let entry = send(&mut run, "start_repeat", None, &tone(500));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(40));
    device.hold();
    device.entered(&["rx_recv", "tx_"]);
    (run, dir)
}

#[test]
fn run_a_dropped_live_run_is_judged() {
    let device = Arc::new(Gated::new(FakeConfig::default()));
    let _release = Release(device.clone());
    let (caught, markers) = capture_markers(&["join"], || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (_run, _dir) = held_session(&device);
        }))
    });
    let message = panic_text(&*caught.expect_err("the dropped Run panicked"));
    assert!(message.contains("met join"), "{message}");
    assert_eq!(markers.len(), 1, "{markers:?}");
}

#[test]
fn run_unwinding_marks_a_bound_and_never_panics_again() {
    // A Run alive while the test panics is finished by its `Drop`: a healthy one ends quietly, one that meets a bound writes the marker, and
    // neither panics again (a second panic while unwinding aborts the whole test binary).
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (_run, _device, _dir) = tx_session(FakeConfig::default());
        panic!("boom");
    }));
    assert_eq!(panic_text(&*caught.expect_err("the test's own panic")), "boom");
    let device = Arc::new(Gated::new(FakeConfig::default()));
    let _release = Release(device.clone());
    let (caught, markers) = capture_markers(&["join"], || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (_run, _dir) = held_session(&device);
            panic!("boom");
        }))
    });
    assert_eq!(panic_text(&*caught.expect_err("the test's own panic")), "boom", "the message is not masked");
    assert_eq!(markers.len(), 1, "the bound is marked while unwinding: {markers:?}");
}

// `Expect::Any` is the bench loops' declaration: `perf_rx_rate_trial` and `b8_lead_trial` are the bodies of `hw_perf_rx_rates` and `hw_b8_leads`.

#[test]
fn run_any_a_perf_trial_reports_a_lost_device_and_the_next_trial_goes_on() {
    let lost = fake(FakeConfig { faults: vec![FakeFault::Lost(Wall::from_millis(2_200))], ..FakeConfig::default() });
    let line = perf_rx_rate_trial(lost, 1e6);
    assert!(line.contains("Err(Ended") && line.contains("DEVICE_LOST"), "{line}");
    let line = perf_rx_rate_trial(fake(FakeConfig::default()), 1e6);
    assert!(line.contains("Ok(())") && line.contains("Client"), "{line}");
}

#[test]
fn run_any_a_lead_trial_reports_a_lost_device_and_the_next_trial_goes_on() {
    let lost = fake(FakeConfig { faults: vec![FakeFault::Lost(Wall::from_millis(2_050))], ..FakeConfig::default() });
    let lines = b8_lead_trial(lost, 5_000);
    assert!(lines[0].contains("DEVICE_LOST"), "{lines:?}");
    let lines = b8_lead_trial(fake(FakeConfig::default()), 5_000);
    assert!(lines[0].contains("Client"), "{lines:?}");
}

#[test]
fn run_any_a_perf_trial_still_fails_on_a_bound() {
    let device = Arc::new(Gated::new(FakeConfig::default()));
    let _release = Release(device.clone());
    device.hold();
    let (caught, markers) = capture_markers(&["join"], || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| perf_rx_rate_trial(device.clone(), 1e6))));
    let message = panic_text(&*caught.expect_err("the trial panicked"));
    assert!(message.contains("met join"), "{message}");
    assert_eq!(markers.len(), 1, "{markers:?}");
}

// ---- the bench probes of 2026-11-15 (`common::bench1115`), once on the fake: the calls they make, their pure judgements are unit-tested there.

#[test]
fn bench1115_zeros_sweep_rehearses_on_the_fake() {
    // On the host clock because the probe waits in real time. The fake returns the transmitted tone only at equal rates and frequencies, so the
    // 1 MS/s restart cell is judged PASS (it models no fill); a retune moves the receive frequency, so its tone is a ramp there: INCONCLUSIVE.
    use common::bench1115::{Start, ZCell, cell_stats, cell_verdict, zeros_cells, zeros_report, zeros_sweep};
    use common::tone::Verdict;
    let device = fake_real(one_obx());
    device.set_time_zero(false).unwrap();
    let cell = |decim, start, gain_db| ZCell { decim, start, gain_db, trials: 2 };
    let cells =
        [cell(200, Start::Restart, 0.0), cell(200, Start::Fresh, 0.0), cell(200, Start::Retune, 0.0), cell(100, Start::AfterRate, 0.0), cell(6, Start::Restart, 0.0), cell(200, Start::Restart, 10.0)];
    let runs = zeros_sweep(&*device, &cells);
    assert_eq!(runs.len(), cells.len());
    for (cell, trials) in &runs {
        assert_eq!(trials.len(), 3, "{}: a warm-up and two trials", cell.name());
        assert!(trials[0].warmup && trials[1..].iter().all(|trial| !trial.warmup));
        let stats = cell_stats(trials);
        assert_eq!((stats.errors, stats.misplaced), (0, 0), "{}: {:?}", cell.name(), trials.iter().map(|t| t.error.clone()).collect::<Vec<_>>());
    }
    let verdict = |decim, start: Start, gain: f64| runs.iter().find(|(c, _)| c.decim == decim && c.start == start && c.gain_db == gain).map(|(c, t)| cell_verdict(c, &cell_stats(t))).unwrap();
    assert!(matches!(verdict(200, Start::Restart, 0.0), Verdict::Pass(_)), "{:?}", verdict(200, Start::Restart, 0.0));
    assert!(matches!(verdict(200, Start::Fresh, 0.0), Verdict::Pass(_)), "{:?}", verdict(200, Start::Fresh, 0.0));
    assert!(matches!(verdict(200, Start::Restart, 10.0), Verdict::Pass(_)));
    let verdicts = zeros_report(&runs);
    assert!(verdicts.iter().all(|(_, v)| !matches!(v, Verdict::Fail(_))), "{verdicts:?}");
    assert!(zeros_cells().len() > cells.len());
    let gain_now = |device: &FakeDevice| device.apply(Dir::Tx, 0, &ezsdr_radio_uhd::Settings::default()).unwrap().gain;
    assert_eq!(gain_now(&device), 0.0, "the tone source is back at 0 dB after a +10 dB cell");
}

#[test]
fn bench1115_zeros_sweep_refuses_a_gain_above_the_bench_maximum_and_restores_0_db() {
    use common::bench1115::{MAX_TONE_GAIN_DB, Start, ZCell, zeros_sweep};
    let device = fake_real(one_obx());
    device.set_time_zero(false).unwrap();
    let cells = [ZCell { decim: 200, start: Start::Restart, gain_db: 10.0, trials: 1 }, ZCell { decim: 200, start: Start::Restart, gain_db: MAX_TONE_GAIN_DB + 5.0, trials: 1 }];
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| zeros_sweep(&*device, &cells)));
    let message = panic_text(&*refused.expect_err("a gain above the maximum is refused"));
    assert!(message.contains("RF safety"), "{message}");
    assert_eq!(device.apply(Dir::Tx, 0, &ezsdr_radio_uhd::Settings::default()).unwrap().gain, 0.0, "the unwind put the tone source back at 0 dB");
}

#[test]
fn bench1115_pn_sweep_rehearses_on_the_fake() {
    // On the host clock because the probe waits in real time. The fake models a loopback delay and no DDC fill: the PN is found at the delay in a
    // continuous capture and in a restart, with no zeros, so the trial is INSEPARABLE and the cell passes.
    use common::bench1115::{PnCell, PnOutcome, pn_cell_verdict, pn_report, pn_stats, pn_sweep};
    use common::tone::Verdict;
    let device = fake_real(FakeConfig { loopback_delay_samples: 44, ..one_obx() });
    device.set_time_zero(false).unwrap();
    // 103 ms of 1 MS/s is not a whole number of the fake's 2 000-sample blocks, so a window starts and ends inside one.
    let cell = PnCell { decim: 200, trials: 2, settle_ms: 103 };
    let runs = pn_sweep(&*device, &[cell]);
    let (_, trials) = &runs[0];
    assert_eq!(trials.len(), 3, "a warm-up and two trials");
    assert!(trials[0].warmup && trials[1..].iter().all(|trial| !trial.warmup));
    for trial in trials {
        assert!(trial.error.is_none() && trial.incomplete.is_none() && !trial.misplaced, "{trial:?}");
        let measure = trial.measure.expect("measured");
        assert_eq!((measure.z, measure.cont.d, measure.restart.d), (0, Some(44), Some(44)), "{measure:?}");
        assert_eq!(trial.outcome(), Some(PnOutcome::Inseparable));
    }
    assert!(matches!(pn_cell_verdict(&cell, &pn_stats(trials)), Verdict::Pass(_)));
    let sends: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("tx_send 0")).collect();
    let starts: Vec<&String> = sends.iter().filter(|call| call.contains("sob=true")).collect();
    assert!(starts.len() == 3 && starts.iter().all(|call| !call.contains("at=-")), "one timed start a trial: {sends:?}");
    assert!(sends.iter().filter(|call| !call.contains("sob=true")).all(|call| call.contains("at=-")), "the continuation is untimed: {sends:?}");
    assert!(sends.iter().filter(|call| call.contains("eob=true")).count() == 3 && sends.len() > 6, "a burst of several sends, closed once a trial: {sends:?}");
    let verdicts = pn_report(&runs);
    assert!(verdicts.iter().all(|(_, v)| matches!(v, Verdict::Pass(_))), "{verdicts:?}");
}

#[test]
fn bench1115_a_failed_receive_call_ends_a_pn_window_with_its_error() {
    // The PN trial's `?` turns this error into the trial's `error` (an infrastructure FAIL), not an INCONCLUSIVE capture, and the read does not poll
    // the failure for its whole limit.
    use common::bench1115::read_window;
    use common::raw::{rx_start_ahead, timed};
    let device = fake_real(one_obx());
    device.set_time_zero(false).unwrap();
    let (closed, took) = timed(|| read_window(&*device, 2_000, 200, 0, 100, Wall::from_secs(4)));
    assert!(matches!(&closed, Err(text) if text.starts_with("rx_recv:") && text.contains("not open")), "{:?}", closed.as_ref().map(|w| w.samples.len()));
    assert!(took < Wall::from_secs(2), "the read stopped at the failure, not at its limit");
    device.apply(ezsdr_radio_uhd::Dir::Rx, 0, &ezsdr_radio_uhd::Settings { rate: Some(1e6), ..ezsdr_radio_uhd::Settings::default() }).unwrap();
    device.rx_open(0, &[0]).unwrap();
    let from = rx_start_ahead(&*device);
    let window = read_window(&*device, 2_000, 200, from, 100, Wall::from_secs(4)).expect("an open stream reads");
    assert_eq!(window.samples.len(), 100);
}

#[test]
fn bench1115_the_cleanup_drain_ends_at_a_failure_and_keeps_draining_past_an_overflow() {
    // A closed streamer fails at once ("UR-3: the receive streamer is not open" on hardware: about 184 000 rows in a trial's 300 ms): the drain keeps that row
    // and stops instead of polling it for its whole limit. Timeouts still end it only after two in a row; an overflow after a stop does not end it.
    use common::raw::{drain_without_clock, rx_run_then_stop, timed};
    let device = fake_real(one_obx());
    device.set_time_zero(false).unwrap();
    let (rows, took) = timed(|| drain_without_clock(&*device, Wall::from_millis(10), Wall::from_secs(5)));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["kind"], "other");
    assert!(rows[0]["result"].as_str().unwrap().contains("not open"), "{rows:?}");
    assert!(took < Wall::from_secs(2), "the drain stopped at the failure, not at its limit");
    device.rx_open(0, &[0]).unwrap();
    let rows = drain_without_clock(&*device, Wall::from_millis(10), Wall::from_secs(5));
    assert_eq!(rows.iter().map(|row| row["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["timeout", "timeout"], "an open, stopped stream drains in two timeouts");
    let device = fake_real(FakeConfig { faults: vec![FakeFault::Overflow(Wall::ZERO)], ..one_obx() });
    device.set_time_zero(false).unwrap();
    device.apply(ezsdr_radio_uhd::Dir::Rx, 0, &ezsdr_radio_uhd::Settings { rate: Some(1e6), ..ezsdr_radio_uhd::Settings::default() }).unwrap();
    device.rx_open(0, &[0]).unwrap();
    rx_run_then_stop(&*device, Wall::from_millis(30));
    let rows = drain_without_clock(&*device, Wall::from_millis(10), Wall::from_secs(5));
    let kinds: Vec<&str> = rows.iter().map(|row| row["kind"].as_str().unwrap()).collect();
    assert!(rows[0]["result"].as_str().unwrap().contains("Overflow"), "{rows:?}");
    assert!(kinds.len() >= 3 && kinds[1..].iter().all(|kind| *kind != "other") && kinds[kinds.len() - 2..] == ["timeout", "timeout"], "an overflow does not end the drain: {rows:?}");
}

#[test]
fn bench1115_t3_probes_rehearse_on_the_fake() {
    // (a) on the fake: every call succeeds, T3 is ACKed and B is ACKed at its end. The capture cannot show B: before a transmit the fake's receive samples
    // are a ramp above the tone (`device.rs` `sample_at`), so each trial is judged "not in the capture" (the onset is unit-tested on a synthetic capture).
    // (b) the fake's reports belong to their streamer, (c) its ACK is kept 150 ms.
    use common::bench1115::*;
    use common::tone::Verdict;
    let device = fake_real(one_obx());
    device.set_time_zero(false).unwrap();
    let skip = skip_probe(&*device, &[1e6, 200e6], 2);
    assert_eq!(skip.len(), 6);
    for (_, kind, trials) in &skip {
        assert_eq!(trials.len(), 2 + usize::from(*kind == SkipKind::OpenThenFlush), "{kind:?}: the controls bracket the skipping trials");
        for trial in trials {
            assert!(trial.error.is_none() && trial.reports.len() == 1, "{kind:?}: {:?}", trial.trace());
            assert_eq!(trial.t3_ack.is_some(), *kind == SkipKind::OpenThenFlush, "{kind:?}");
            assert_eq!(trial.reports[0].tick, Some(trial.b_end + 1));
        }
    }
    let verdicts = skip_report(&skip);
    println!("{}", skip_decision(&skip, &[1e6, 200e6], 2));
    assert_eq!(verdicts.len(), 2 * (3 + 2 + 2 + 1));
    let of = |kind: &str| verdicts.iter().filter(|(claim, _)| claim.contains(kind)).map(|(_, v)| v).collect::<Vec<_>>();
    assert!(of("n75 a open_then_flush").iter().all(|v| matches!(v, Verdict::Fail(text) if text == "burst B is not in the capture")), "{verdicts:?}");
    assert!(of("skip").iter().all(|v| matches!(v, Verdict::Inconclusive(text) if text.contains("no stable bracketing control"))), "{verdicts:?}");
    assert!(of("control stability").iter().all(|v| matches!(v, Verdict::Inconclusive(_))), "{verdicts:?}");
    assert!(skip_decision(&skip, &[1e6, 200e6], 2).contains("INCONCLUSIVE"));
    let rest = routing_and_late_probe(&*device, &[1e6], 2);
    assert_eq!(rest.len(), 6);
    assert!(rest.iter().all(|(_, v)| matches!(v, Verdict::Pass(_))), "{rest:?}");
}

#[test]
fn bench1115_skip_probe_records_a_receive_overflow_and_its_gap() {
    // The recording side of (a): an overrun on the first receive call of the first trial skips 50 ms, so the capture starts after B's window opened: the
    // trial keeps the outcome and is not clean, and the verdict is INCONCLUSIVE (never a FAIL about B), the later trials stay clean. (A gap inside a capture
    // needs a fault that fires after a sample block, which the fake times by the clock; `a_trial_records_the_outcomes...` in bench1115.rs covers it.)
    use common::bench1115::*;
    use common::tone::Verdict;
    let device = fake_real(FakeConfig { faults: vec![FakeFault::Overflow(Wall::ZERO)], ..one_obx() });
    device.set_time_zero(false).unwrap();
    let skip = skip_probe(&*device, &[1e6], 1);
    let all: Vec<&SkipTrial> = skip.iter().flat_map(|(_, _, trials)| trials.iter()).collect();
    let dirty: Vec<&&SkipTrial> = all.iter().filter(|trial| !trial.rx_other.is_empty()).collect();
    assert_eq!(dirty.len(), 1, "the overrun fires once: {:?}", all.iter().map(|trial| trial.trace()).collect::<Vec<_>>());
    assert!(dirty[0].rx_other[0].contains("Overflow") && !dirty[0].spans_b && !dirty[0].clean(), "{:?}", dirty[0].trace());
    assert!(all.iter().filter(|trial| trial.rx_other.is_empty()).all(|trial| trial.clean() && trial.contiguous && trial.spans_b));
    let verdicts = skip_report(&skip);
    assert!(verdicts.iter().any(|(_, v)| matches!(v, Verdict::Inconclusive(text) if text.contains("not clean") && text.contains("Overflow"))), "{verdicts:?}");
    assert!(!skip_decision(&skip, &[1e6], 1).contains("T3 is needed"));
}

#[test]
fn bench1115_probes_refuse_another_channel_before_any_device_call() {
    // `EZSDR_D9_CHANNEL` is read once per process, so each probe runs in a child with it set to 1; the refusal is the first thing it does.
    for name in [
        "bench1115_zeros_sweep_rehearses_on_the_fake",
        "bench1115_pn_sweep_rehearses_on_the_fake",
        "bench1115_t3_probes_rehearse_on_the_fake",
        "bench1115_routing_and_late_probe_rehearses_on_the_fake",
        "bench1115_open_flush_sweep_rehearses_on_the_fake_and_a_dropped_ack_fails",
        "bench1115_skip_probe_records_a_receive_overflow_and_its_gap",
    ] {
        let out = std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", name, "--test-threads=1", "--nocapture"]).env("EZSDR_D9_CHANNEL", "1").output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(!out.status.success() && stderr.contains("EZSDR_D9_CHANNEL=1: the 2026-11-15 probes transmit on channel 0"), "{name}: {stderr}");
    }
}

#[test]
fn bench1115_routing_and_late_probe_rehearses_on_the_fake() {
    // Alone, so that the channel refusal below reaches this probe's own entry.
    use common::bench1115::*;
    use common::tone::Verdict;
    let device = fake_real(one_obx());
    device.set_time_zero(false).unwrap();
    let rest = routing_and_late_probe(&*device, &[1e6], 1);
    assert_eq!(rest.len(), 3);
    assert!(rest.iter().all(|(_, v)| matches!(v, Verdict::Pass(_))), "{rest:?}");
}

#[test]
fn bench1115_open_flush_sweep_rehearses_on_the_fake_and_a_dropped_ack_fails() {
    use common::bench1115::*;
    use common::tone::Verdict;
    let two = fake_real(FakeConfig { front_ends: vec!["UBX", "UBX"], ..FakeConfig::default() });
    two.set_time_zero(false).unwrap();
    let verdicts = open_flush_sweep(&*two, &[1e6, 2e6], 2, true);
    assert_eq!(verdicts.len(), 8, "{verdicts:?}");
    assert!(verdicts.iter().all(|(claim, v)| matches!(v, Verdict::Pass(_)) || claim.contains("x 2")), "{verdicts:?}");
    assert!(verdicts.iter().all(|(_, v)| matches!(v, Verdict::Pass(_))), "{verdicts:?}");
    assert!(open_flush_decision(&verdicts, &[1e6, 2e6], 2, false).contains("enable (3)"));
    let unconfirmed = open_flush_sweep(&*two, &[1e6], 1, false);
    assert_eq!(unconfirmed.iter().map(|(_, v)| matches!(v, Verdict::Pass(_))).collect::<Vec<_>>(), [true, false], "{unconfirmed:?}");
    assert!(matches!(&unconfirmed[1].1, Verdict::Inconclusive(text) if text.contains("not runnable") && text.contains("EZSDR_SLOT_B_ATTENUATOR_CONFIRMED")), "{unconfirmed:?}");
    let one = fake_real(one_obx());
    one.set_time_zero(false).unwrap();
    let verdicts = open_flush_sweep(&*one, &[1e6], 2, true);
    assert_eq!(verdicts.iter().map(|(_, v)| matches!(v, Verdict::Pass(_))).collect::<Vec<_>>(), [true, true, false]);
    assert!(matches!(&verdicts[2].1, Verdict::Inconclusive(text) if text.contains("not runnable") && text.contains("empty slot")), "{verdicts:?}");
    assert!(open_flush_decision(&verdicts, &[1e6], 2, false).contains("NOT RUNNABLE"), "the gated two-channel cell keeps the decision open");
    assert!(open_flush_decision(&verdicts, &[1e6], 2, true).contains("enable (3)"), "unless the owner waives it");
    let dropped = fake_real(FakeConfig { faults: vec![FakeFault::DropBurstAck], ..one_obx() });
    dropped.set_time_zero(false).unwrap();
    let verdicts = open_flush_sweep(&*dropped, &[1e6], 1, true);
    assert!(matches!(&verdicts[0].1, Verdict::Fail(text) if text.contains("missing on positions [0]")), "{verdicts:?}");
    assert!(open_flush_decision(&verdicts, &[1e6], 1, false).contains("ask the owner again"));
}

// ---- the stage-4 bench tools of 2026-11-15 (`common::s4bench`), once on the fake: the record's own code with other plan values; their pure judgements are unit-tested there.

/// Three of `s2_cells` at 1 and 2 MS/s (the fake's own link would refuse 100 and 200), one trial each: the runner and its decision lines on the real `run_stage`.
#[test]
fn s4bench_s2_rehearses_on_the_fake_and_reads_a_scripted_overflow_and_underflow() {
    use common::s4bench::*;
    let faults = vec![FakeFault::Overflow(Wall::from_millis(3_000)), FakeFault::TxReport(Wall::from_millis(3_000), TxCode::Underflow)];
    let (device, description, _) = raw_group_fixture(FakeConfig { faults, ..one_obx() });
    let wire_bytes = u64::try_from(description.performance.wire_bytes_per_sample).unwrap();
    let plan = measurement::MeasurementPlan { throughput_seconds: 2, tx_waveform_samples: 2_000, ..measurement::MeasurementPlan::rehearsal() };
    common::iii::pin_to_slot_a(&*device).unwrap();
    let cells = s2_select(s2_cells((1_000_000, 2_000_000), 1, 1), Some("rx-1msps, tx-1msps, tx-2msps")).unwrap();
    let run = s2_run(&cells, |stage| measurement::run_stage(device.clone(), &plan, stage, wire_bytes), || None);
    assert_eq!(run.stopped, None);
    assert_eq!(run.cells.iter().map(|(_, trials)| trials.len()).collect::<Vec<_>>(), [1; 3]);
    let trial = |id: &str| &run.cells.iter().find(|(cell, _)| cell.id == id).unwrap().1[0];
    // Each scripted fault fires once, in the first Run that reaches its instant: the first receive cell and the first transmit cell.
    let rx = trial("rx-1msps");
    assert!(rx.rx_overflow_events >= 1 && rx.first_rx_overflow_ns.is_some_and(|ns| ns > 0 && ns < 2_000_000_000), "{rx:?}");
    let tx = trial("tx-1msps");
    assert!(tx.tx_underflow_events >= 1 && tx.first_tx_underflow_ns.is_some_and(|ns| ns > 0 && ns < 2_000_000_000), "{tx:?}");
    assert_eq!(tx.window_held(S2_TX_WINDOW_NS), Some(false), "an underflow inside the first 10 s");
    assert_eq!(trial("tx-2msps").window_held(S2_TX_WINDOW_NS), None, "a clean 2 s Run cannot show the 10 s window: its samples fall short");
    let lines = s2_decisions(&run, plan.throughput_seconds);
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|line| line.starts_with("S2 decision ")));
    assert!(lines.iter().any(|line| line.contains("rx-1msps") && line.contains("1 failed") && line.contains("RX_OVERFLOW in 1 of 1")), "{lines:?}");
    assert!(lines.iter().any(|line| line.contains("tx-1msps") && line.contains("underflow within it: 1")), "{lines:?}");
    assert!(lines.iter().any(|line| line.contains("tx-2msps") && line.contains("INCONCLUSIVE")), "the clean TX cell cannot show the window in 2 s: {lines:?}");
}

/// A `FakeDevice` whose underflow reports carry no device time, as an async message with no time spec does.
struct UntimedUnderflows {
    inner: Arc<FakeDevice>,
}

impl Device for UntimedUnderflows {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.front_end(dir, chan)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> {
        self.inner.rates(d, c)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_sources(c, t)
    }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.set_time_zero(p)
    }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> {
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        self.inner.apply(d, c, s)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_open(s, c)
    }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_start(s, at)
    }
    fn rx_packet_samples(&self, s: usize) -> usize {
        self.inner.rx_packet_samples(s)
    }
    fn rx_stop(&self, s: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.rx_stop(s)
    }
    fn rx_recv(&self, s: usize, n: usize, t: Wall, one_packet: bool) -> ezsdr_radio_uhd::RxRecv {
        self.inner.rx_recv(s, n, t, one_packet)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_open(s, c)
    }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        let report = self.inner.tx_async(s, t)?;
        Ok(report.map(|report| if matches!(report.code, TxCode::Underflow) { ezsdr_radio_uhd::TxReport { tick: None, ..report } } else { report }))
    }
    fn rx_close(&self, s: usize) {
        self.inner.rx_close(s)
    }
    fn tx_close(&self, s: usize) {
        self.inner.tx_close(s)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
}

#[test]
fn s4bench_s2_an_untimestamped_underflow_does_not_make_a_window_held() {
    use common::s4bench::*;
    let run_cell = |untimed: bool| {
        let (device, description, _) = raw_group_fixture(FakeConfig { faults: vec![FakeFault::TxReport(Wall::from_millis(3_000), TxCode::Underflow)], ..one_obx() });
        let wire_bytes = u64::try_from(description.performance.wire_bytes_per_sample).unwrap();
        let plan = measurement::MeasurementPlan { throughput_seconds: 2, tx_waveform_samples: 2_000, ..measurement::MeasurementPlan::rehearsal() };
        common::iii::pin_to_slot_a(&*device).unwrap();
        let stage_device: Arc<dyn Device> = if untimed { Arc::new(UntimedUnderflows { inner: device }) } else { device };
        let cells = s2_select(s2_cells((1_000_000, 2_000_000), 1, 1), Some("tx-1msps")).unwrap();
        let run = s2_run(&cells, |stage| measurement::run_stage(stage_device.clone(), &plan, stage, wire_bytes), || None);
        assert_eq!(run.stopped, None);
        (run.cells[0].1[0].clone(), s2_decisions(&run, plan.throughput_seconds).remove(0))
    };
    let (timed, _) = run_cell(false);
    assert_eq!((timed.tx_underflow_events, timed.tx_underflow_untimed), (1, 0), "{timed:?}");
    let at = timed.first_tx_underflow_ns.filter(|ns| *ns > 1).expect("a timed underflow has a time after T0");
    assert_eq!(timed.window_held(at - 1), Some(true), "a timed underflow just after the window leaves the window held");
    let (untimed, line) = run_cell(true);
    assert_eq!((untimed.tx_underflow_events, untimed.tx_underflow_untimed), (1, 1), "{untimed:?}");
    // The window ends just before this run's own read-time date of the report (two runs' dates differ by host timing, so not `at` of the timed run).
    let untimed_at = untimed.first_tx_underflow_ns.filter(|ns| *ns > 1).expect("the untimed underflow is dated at its read time");
    assert_eq!(untimed.window_held(untimed_at - 1), None, "the same report without a device time cannot be placed outside the window");
    assert!(line.contains("1 underflow reports had no device time"), "the decision line says so: {line}");
}

#[test]
fn s4bench_t200_runs_the_record_stage_alone_and_reads_a_scripted_underflow() {
    use common::s4bench::*;
    let (device, _, _) = raw_group_fixture(FakeConfig { faults: vec![FakeFault::TxReport(Wall::from_millis(3_000), TxCode::Underflow)], ..one_obx() });
    let device: Arc<dyn Device> = device;
    common::iii::pin_to_slot_a(&*device).unwrap();
    let plan = measurement::MeasurementPlan { throughput_seconds: 2, tx_throughput_rates_hz: vec![1_000_000], ..measurement::MeasurementPlan::rehearsal() };
    let (runs, stopped) = t200_runs(&device, &plan, 2, || None);
    assert_eq!(stopped, None);
    assert_eq!(runs.len(), 2);
    assert!(runs[0].underflow_events >= 1 && runs[0].first_underflow_ns.is_some_and(|ns| ns > 0 && ns < 2_000_000_000), "{:?}", runs[0]);
    let line = t200_decision(&runs, 2, &plan, stopped.as_deref());
    assert!(line.starts_with("T200 decision (record-suite shape, 1 MS/s, 2 s, waveform 1000 samples)"), "{line}");
    let two_rates = measurement::MeasurementPlan { tx_throughput_rates_hz: vec![1_000_000, 2_000_000], ..plan.clone() };
    assert!(t200_runs(&device, &two_rates, 1, || None).1.is_some_and(|reason| reason.contains("one transmit rate")));
    let (_, latched) = t200_runs(&device, &plan, 2, || Some("a control timeout".to_owned()));
    assert!(latched.is_some_and(|reason| reason.contains("run 0") && reason.contains("a control timeout")), "a latched handle stops the next run");
}

#[test]
fn s4bench_s5_lead_passes_run_the_records_groups_at_other_rates_and_channels() {
    use common::s4bench::*;
    let (fake_device, _, _) = raw_group_fixture(FakeConfig { front_ends: vec!["OBX", "OBX"], ..FakeConfig::default() });
    let device: Arc<dyn Device> = fake_device.clone();
    let plan = measurement::MeasurementPlan {
        timed_command_leads_ns: vec![D3_LEAD_NS],
        timed_command_tx_rates_hz: vec![1_000_000, 2_000_000],
        rx_start_leads_ns: vec![D4_LEAD_NS],
        rx_start_rates_hz: vec![1_000_000, 2_000_000],
        step_trials: 2,
        ..measurement::MeasurementPlan::rehearsal()
    };
    let calls_since = |before: usize| -> Vec<String> { fake_device.calls().into_iter().skip(before).collect() };
    let before = fake_device.calls().len();
    let (lines, error) = s5_command_lead(&device, &plan);
    assert_eq!(error, None);
    let calls = calls_since(before);
    assert!(calls.iter().any(|call| call.starts_with("apply tx 0 rate=2000000")), "the transmit stream ran at the plan's second rate: {calls:?}");
    assert_eq!(lines.iter().filter(|line| line.starts_with("S5 command-lead trial ")).count(), 2 * 2);
    let decision = lines.last().unwrap();
    assert!(decision.starts_with("S5 decision command-lead: ") && decision.contains("5.000 ms: every trial Success in 2 of 2 conditions") && decision.contains("tx-2msps"), "{decision}");
    // One channel at the record's 1 MS/s keeps the record's own condition ids; another rate or two channels name theirs.
    let before = fake_device.calls().len();
    let (lines, error) = s5_rx_start_lead(&device, &plan);
    assert_eq!(error, None);
    let calls = calls_since(before);
    assert!(calls.iter().any(|call| call.starts_with("apply rx 0 rate=2000000")) && calls.iter().any(|call| call.starts_with("apply rx 0 rate=1000000")), "both rates ran: {calls:?}");
    assert!(!calls.iter().any(|call| call.starts_with("apply rx 1")), "one channel touches no second channel: {calls:?}");
    let decision = lines.last().unwrap();
    assert!(decision.starts_with("S5 decision rx-start-lead-1ch: "), "{decision}");
    assert!(["restart-same:", "restart-retune:", "restart-same-rx-2msps-1ch:", "restart-retune-rx-2msps-1ch:"].iter().all(|condition| decision.contains(condition)), "{decision}");
    assert_eq!(lines.iter().filter(|line| line.starts_with("S5 rx-start-lead-1ch trial ")).count(), 2 * 2 * 2, "one rung, both conditions, two rates, two trials");
    let two = measurement::MeasurementPlan { rx_start_channels: 2, rx_start_rates_hz: vec![2_000_000], ..plan.clone() };
    let before = fake_device.calls().len();
    let (lines, error) = s5_rx_start_lead(&device, &two);
    let calls = calls_since(before);
    let applies = |channel: usize| -> Vec<String> { calls.iter().filter_map(|call| call.strip_prefix(&format!("apply rx {channel} ")).map(str::to_owned)).collect() };
    assert!(calls.contains(&"rx_open 0 [0, 1]".to_owned()), "{calls:?}");
    assert!(!applies(0).is_empty() && applies(0) == applies(1), "every receive apply reaches both channels with the same values: {calls:?}");
    assert!(
        applies(1).iter().any(|call| call.contains("freq=1000500000")) && applies(1).iter().all(|call| call.starts_with("rate=2000000 ")),
        "the retune crosses 1 GHz at the plan's rate: {calls:?}"
    );
    assert_eq!(error, None);
    let decision = lines.last().unwrap();
    assert!(decision.starts_with("S5 decision rx-start-lead-2ch: ") && decision.contains("restart-retune-rx-2msps-2ch") && !decision.contains("not runnable"), "{decision}");
    // A CBX in slot B cannot carry the 1 GHz retune: the two-channel pass reports it and runs nothing.
    let (cbx, _, _) = raw_group_fixture(obx_and_cbx());
    let cbx: Arc<dyn Device> = cbx;
    let (lines, error) = with_selector(two_channel_selector(), || s5_rx_start_lead(&cbx, &two));
    assert_eq!((error, lines.len()), (None, 1));
    assert!(lines[0].contains("INCONCLUSIVE: not runnable") && lines[0].contains("0:B:cbx:0"), "{lines:?}");
}

#[test]
fn s4bench_s4_runs_the_overflow_trials_and_prints_the_restart_after_resume() {
    use common::s4bench::*;
    let device: Arc<dyn Device> = fake(one_obx());
    let cells = vec![S4Cell { rate_hz: 1_000_000, stall_ms: 300, trials: 2, sweep: false }, S4Cell { rate_hz: 1_000_000, stall_ms: 400, trials: 2, sweep: true }];
    let run = s4_run(&device, &cells, 200, || None);
    assert_eq!(run.stopped, None);
    assert_eq!(run.cells.iter().map(|(_, trials)| trials.len()).collect::<Vec<_>>(), [2, 2]);
    let lines = s4_decisions(&run, 1_000_000);
    assert!(lines[0].starts_with("S4 decision 1 MS/s stall 300 ms: ") && lines[1].starts_with("S4 decision 1 MS/s stall 400 ms: "), "{lines:?}");
    assert!(lines.iter().any(|line| line.starts_with("S4 decision D5 bound: ")), "{lines:?}");
    assert!(lines.iter().any(|line| line.starts_with("S4 decision stall length at 1 MS/s: ") && line.contains("stall 300 ms") && line.contains("stall 400 ms")), "{lines:?}");
    let (_, stopped) = (0, s4_run(&device, &cells, 200, || Some("a control timeout".to_owned())).stopped);
    assert!(stopped.is_some_and(|reason| reason.contains("a control timeout")));
}
