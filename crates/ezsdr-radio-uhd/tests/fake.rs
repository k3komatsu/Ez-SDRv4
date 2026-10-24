//! Spec 18 §6: every UR rule through the real coordinator on `FakeDevice`, with an
//! Assembly built as the server's catalogue builds it (the capture Sink, the host
//! Link, `DeviceAuthority` and `UhdRadio` on one device), unless a test says "unit".
//! The device's clock is the manual one (plan 30 §4.4), so a test's waits and its
//! timing assertions are that clock's; see tests/README.md for the rules and for
//! timing-precondition diagnostics.

mod common;

use ezsdr_kernel::spec::Scalar;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding};
use ezsdr_kernel::coordinator::{RunHandle, RunHandleError, start_spec_run};
use ezsdr_kernel::event::{Action, ActionId, Event, EventCollector, EventKind, EventSource, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ResourceId, RunId};
use ezsdr_kernel::manifest::{ArtifactRef, Manifest};
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, Authority, AttachedPort, Endpoint, ExecutionClass, Link,
    ModuleErrorKind, ModuleRegistry, PrepareContext, Provider, Role,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::run::{RunState, Stage, StopCause, Termination};
use ezsdr_kernel::session::SessionAction;
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BurstEnd, DataLinkDecl};
use ezsdr_kernel::time::{ClockRegistry, Duration, EpochRef, RelativeBudget, TimePoint};
use ezsdr_radio::payloads::{TimeErrorOutcome, TimeErrorPayload};
use ezsdr_radio_uhd::{ClockMode, Device, DeviceAuthority, FakeConfig, FakeDevice, FakeFault, TxCode, UhdRadio};
use serde_json::{Value as Json, json};

use common::*;

// ---------------------------------------------------------------- identity (UR-1…UR-5)

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
    assert_eq!(
        refuse(binding(ok, Some(x3xx())), slow),
        "UR-5: profile x3xx needs a 200 MHz master clock; the device runs at 184320000 Hz"
    );
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
    assert_eq!(
        refuse(two("ubx"), boards(vec!["UBX"])),
        "UR-5: uhd_usrp_set_rx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev B:0"
    );
    assert_eq!(refuse(one_board("cbx"), FakeConfig::default()), "UR-5: the layout's rx channel 0 is `UBX RX`, not `cbx` front end 0");
    assert_eq!(
        refuse(one_board("cbx"), boards(vec![])),
        "UR-5: uhd_usrp_set_rx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev A:0"
    );
    // The transmit side is checked the same way, and so is a device whose two directions
    // name boards of different kinds.
    assert!(UhdRadio::from_binding(&binding(one_board("obx"), Some(x3xx())), fake(one_obx())).is_ok());
    assert!(UhdRadio::from_binding(&binding(two("ubx"), Some(x3xx())), fake(boards(vec!["UBX", "UBX"]))).is_ok());
    assert_eq!(refuse(one_board("ubx"), one_obx()), "UR-5: the layout's rx channel 0 is `OBX RX`, not `ubx` front end 0");
    assert_eq!(refuse(one_board("obx"), FakeConfig::default()), "UR-5: the layout's rx channel 0 is `UBX RX`, not `obx` front end 0");
    assert_eq!(refuse(one_board("obx"), one_cbx()), "UR-5: the layout's rx channel 0 is `CBX-120 RX`, not `obx` front end 0");
    let mixed = json!({ "args": ARGS, "rx": ["0:A:ubx:0"], "tx": ["0:A:obx:0"] });
    assert_eq!(refuse(mixed, FakeConfig::default()), "UR-5: the layout's `rx` element at position 0 (timekeeper 0, slot A, `ubx` front end 0) sits in a slot that also carries `obx`; a duplex board carries both directions and admits no other board there");
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
        let run = start_spec_run(&receive_spec(1, 1e6, 1e9, None), &profile, assembly(&profile, fake(FakeConfig::default()), BTreeMap::new(), |r| r)).unwrap();
        assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Prepare, .. } }), "{clock_source} {time_source}");
        let manifest = run.finish();
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
        let waiter = { let a = authority.clone(); std::thread::spawn(move || (a.next_wakeup(), Instant::now())) };
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
    assert!(matches!(
        time.schedule(TimePoint::new(root, fired.ticks_in(root).unwrap() - 1), Box::new(|_| {})),
        Err(ezsdr_kernel::time::TimeError::InPast { .. })
    ));
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
    time.schedule(t, Box::new(move |at| {
        log.lock().unwrap().push("first");
        let log2 = log.clone();
        inner.schedule(at, Box::new(move |_| log2.lock().unwrap().push("second"))).unwrap();
        started_tx.send(()).unwrap();
        let _ = proceed_rx.recv();
        log.lock().unwrap().push("first ends");
    })).unwrap();
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
    let own = b.time().schedule(TimePoint::new(b.root(), 0),
        Box::new(move |_| { *flag.lock().unwrap() = true; })).unwrap();
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
        let waiter = { let a = authority.clone(); std::thread::spawn(move || (a.next_wakeup(), Instant::now())) };
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
    assert_eq!(capability("radio.timing.command_queue_depth"), one(Scalar::from(16)));
    // Design 38 D1: the release window is the profile's value, x3xx's 3 ms.
    assert_eq!(capability("radio.timing.release_window_ns"), one(Scalar::from(3_000_000)));
    assert_eq!(capability("radio.tx.repeat_max_samples"), one(Scalar::from(4_294_967_295)));
    assert_eq!(capability("radio.tx.repeat_align_samples"), one(Scalar::from(1)));
    assert_eq!(capability("radio.rx.block_len"), one(Scalar::from(2_000)));
    assert_eq!(capability("radio.perf.wire_bytes_per_sample"), one(Scalar::from(4)));
    assert_eq!(capability("radio.tx.c0.path_delay_samples"), one(Scalar::from(45)));
    let envelope = &section_of(instance, "envelope");
    assert_eq!(envelope["profile"]["name"], "x3xx");
    assert_eq!(envelope["timing"]["min_timed_command_lead_ns"], 5_000_000);
    assert_eq!(envelope["timing"]["release_window_ns"], 3_000_000);
    assert_eq!(envelope["performance"]["rx_bytes_per_s"], 100_000_000);
    assert_eq!(section_of(instance, "device")["fake"], true);
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
    let pairs: Vec<_> = ["usrp", "usrp/rx/s0", "usrp/tx/s0", "usrp/rx/s1", "usrp/tx/s1", "usrp/rx/s2", "usrp/tx/s2", "usrp/rx/s3", "usrp/tx/s3"].iter().flat_map(|s| kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k))).collect();
    let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 1024, &Policy::default()));
    let ctx = {
        let mut ctx = PrepareContext::testing(
            EventSource::Node { node: ResourceId::parse("usrp").unwrap() },
            authority.time(),
            clocks,
            events,
            Arc::new(NoActions),
            Arc::new(NoSubmit),
        );
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

/// A queue the test pushes Actions into, as the coordinator dispatches them.
#[derive(Default)]
struct Pushed(Mutex<std::collections::VecDeque<Action>>, std::sync::atomic::AtomicU64);
impl ActionReceiver for Pushed {
    // The coordinator's resolution (KC-23): the test's target is the node it resolves to,
    // `<resource>[/<path>]`, so a test may name a stream's node; ids count from 1, as the
    // Kernel's do (KC-24).
    fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
        let id = || ActionId(self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1);
        self.0.lock().unwrap().pop_front().map(|action| {
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
    radio: UhdRadio,
    device: Arc<FakeDevice>,
    queue: Arc<Pushed>,
    events: Arc<EventCollector>,
    time: Arc<dyn ezsdr_kernel::time::TimeAuthority>,
    root: ClockDomainId,
    clocks: Arc<ClockRegistry>,
    delivered: Vec<Event>,
    _authority: DeviceAuthority,
}

impl Direct {
    fn new(config: FakeConfig, constraints: &[(&str, Scalar)]) -> Direct {
        Direct::with_links(config, constraints, Vec::new())
    }

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

    /// `build_with` on a device the test already has, which the Provider sees through
    /// `tested` (UR-13's rate reads): `inner` is what the test observes the calls on.
    fn build_on(inner: Arc<FakeDevice>, tested: Arc<dyn Device>, constraints: &[(&str, Scalar)], links: Vec<AttachedPort>, selector: Json, inputs: BTreeMap<ContentHash, Vec<u8>>) -> Direct {
        let mut direct = Direct::armed_on(inner, tested, constraints, links, selector, inputs);
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
        let authority = DeviceAuthority::new(tested.clone(), clocks.clone(), "internal", "internal", ARGS).unwrap();
        let (time, root) = (authority.time(), authority.root());
        let mut registry = ModuleRegistry::new();
        let (mut checks, mut kinds) = (AdmissionCheckRegistry::new(), EventKindRegistry::with_kernel_kinds());
        ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
        // The instance's own nodes under either key set (RM-29): a layout's events come
        // from its stream nodes.
        let pairs: Vec<_> = ["usrp", "usrp/rx/s0", "usrp/tx/s0", "usrp/rx/s1", "usrp/tx/s1", "usrp/rx/s2", "usrp/tx/s2", "usrp/rx/s3", "usrp/tx/s3"].iter().flat_map(|s| kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k))).collect();
        let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()));
        let queue = Arc::new(Pushed::default());
        let ctx = {
            let mut ctx = PrepareContext::testing(
                EventSource::Node { node: ResourceId::parse("usrp").unwrap() },
                time.clone(),
                clocks.clone(),
                events.clone(),
                queue.clone(),
                Arc::new(NoSubmit),
            );
            ctx.run = RunId::from_string("direct".to_owned());
            ctx.class = ExecutionClass::HardwareInLoop;
            ctx.environment = Arc::new(BTreeMap::new());
            ctx.inputs = Arc::new(inputs.into_iter().map(|(hash, bytes)| (hash, Arc::<[u8]>::from(bytes))).collect::<BTreeMap<_, _>>());
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
        let mut keys = json!({ "args": ARGS });
        keys.as_object_mut().unwrap().extend(selector.as_object().cloned().unwrap_or_default());
        let mut radio = UhdRadio::from_binding(&binding(keys, Some(x3xx())), tested).unwrap();
        radio.prepare(&fragment, ctx).unwrap();
        radio.arm().unwrap();
        Direct { radio, device, queue, events, time, root, clocks, delivered: Vec::new(), _authority: authority }
    }

    /// Starts the armed Provider at T0 and waits a millisecond past it (UR-15).
    fn start(&mut self) {
        let t0 = self.time.now(self.root).unwrap().ticks_in(self.root).unwrap() + ms(2_000) + ms(1);
        self.radio.start(Some(TimePoint::new(self.root, t0))).unwrap();
        self.time.wait_until(TimePoint::new(self.root, t0 + ms(1))).unwrap();
    }

    fn now(&self) -> i64 {
        self.time.now(self.root).unwrap().ticks_in(self.root).unwrap()
    }

    fn push(&self, action: Action) {
        self.queue.0.lock().unwrap().push_back(action);
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
        // The test thread is a non-participant; it waits on the device's host clock so that
        // the device's time advances while its threads are idle (plan 30 §4.5).
        self.device.host().sleep(wall);
        self.delivered.extend(self.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)));
        self.delivered.clone()
    }

    /// Advances the fake host clock until the observed state is reached; the real-time bound
    /// is only a watchdog, not the condition the test asserts.
    fn until(&mut self, what: &str, mut reached: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Wall::from_secs(10);
        while !reached(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}: {:?}", self.device.calls());
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

/// Fails one selected RX reopen after removing that stream's old streamer (D5).
struct RefuseRxOpenOnce {
    inner: Arc<FakeDevice>,
    stream: usize,
    armed: std::sync::atomic::AtomicBool,
    attempts: Mutex<Vec<(usize, Vec<usize>)>>,
}

impl RefuseRxOpenOnce {
    fn new(inner: Arc<FakeDevice>, stream: usize) -> RefuseRxOpenOnce {
        RefuseRxOpenOnce { inner, stream, armed: std::sync::atomic::AtomicBool::new(false), attempts: Mutex::new(Vec::new()) }
    }

    fn arm(&self) {
        self.armed.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn attempts(&self) -> Vec<(usize, Vec<usize>)> {
        self.attempts.lock().unwrap().clone()
    }
}

impl Device for RefuseRxOpenOnce {
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.front_end(dir, chan) }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, dir: ezsdr_radio_uhd::Dir, markup: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(dir, markup) }
    fn rates(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(dir, chan) }
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(clock, time) }
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(at_next_pps) }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
    fn apply(&self, dir: ezsdr_radio_uhd::Dir, chan: usize, settings: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> { self.inner.apply(dir, chan, settings, at) }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.attempts.lock().unwrap().push((stream, channels.to_vec()));
        if stream == self.stream && self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.inner.rx_close(stream);
            return Err(ezsdr_radio_uhd::DeviceError::failed("fake: scripted one-shot stream-open refusal"));
        }
        self.inner.rx_open(stream, channels)
    }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(stream, at) }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.inner.rx_packet_samples(stream) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(stream, at) }
    fn rx_recv(&self, stream: usize, n: usize, timeout: Wall) -> ezsdr_radio_uhd::RxRecv { self.inner.rx_recv(stream, n, timeout) }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(stream, channels) }
    fn tx_send(&self, stream: usize, samples: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.inner.tx_send(stream, samples, at, sob, eob, timeout) }
    fn tx_async(&self, stream: usize, timeout: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> { self.inner.tx_async(stream, timeout) }
    fn rx_close(&self, stream: usize) { self.inner.rx_close(stream) }
    fn tx_close(&self, stream: usize) { self.inner.tx_close(stream) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
}

fn move_open_refusal_recovery(failed_stream: usize) {
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let inner = fake(fake_mixed());
    let refusal = Arc::new(RefuseRxOpenOnce::new(inner.clone(), failed_stream));
    let tested: Arc<dyn Device> = refusal.clone();
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::armed_on(inner.clone(), tested, &rx_three(), vec![r0, r1], mixed_selector(), BTreeMap::new());
    direct.start();
    direct.until("both initial receive streams start", |direct| calls(&direct.device, "rx_start").len() == 2);
    refusal.arm();
    direct.update("radio.rx.c1.stream", Value::from(1), Cold, Some(direct.now() + ms(100)));
    let failed_channels = if failed_stream == 0 { vec![0] } else { vec![1, 2] };
    direct.until("the selected open refusal", |_| refusal.attempts().contains(&(failed_stream, failed_channels.clone())));
    direct.until("the move refusal event", |direct| !direct.of("radio.COMMAND_REJECTED").is_empty());
    assert_eq!(calls(&direct.device, "rx_start 1").len(), 1, "the refused destination segment never starts: {:?}", inner.calls());
    let log = inner.calls();
    assert!(!log.iter().any(|call| call.contains("Attempting to reconnect")), "{log:?}");

    direct.update("radio.rx.s0.sample_rate_hz", Value::num(4e6).unwrap(), Cold, Some(direct.now() + ms(50)));
    direct.update("radio.rx.s1.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(direct.now() + ms(50)));
    direct.until("both lines resume from their recorded memberships", |direct| calls(&direct.device, "rx_start 0").len() >= 2 && calls(&direct.device, "rx_start 1").len() >= 2);
    let log = inner.calls();
    assert!(!log.iter().any(|call| call.contains("Attempting to reconnect")), "{log:?}");
    assert_eq!(direct.of("radio.COMMAND_REJECTED").len(), 1, "{log:?}");
    let _ = direct.finish();
}

fn move_config_refusal_recovery(claimed: f64, wrong_nth: usize) {
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let fault = FakeFault::WrongRate { claimed, applied: claimed - 1_000.0, nth: wrong_nth };
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig { faults: vec![fault], ..fake_mixed() }, &rx_three(), vec![r0, r1], mixed_selector());
    direct.until("both initial receive streams start", |direct| calls(&direct.device, "rx_start").len() == 2);
    direct.update("radio.rx.c1.stream", Value::from(1), Cold, Some(direct.now() + ms(100)));
    direct.until("the move configuration refusal", |direct| !direct.of("radio.COMMAND_REJECTED").is_empty());
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert!(rejection_reason(&rejected[rejected.len() - 1..]).starts_with("UR-12:"), "{rejected:?}");
    let before = calls(&direct.device, "rx_start 1").len();
    assert_eq!(before, 1, "the refused move destination started before recovery: {:?}", direct.device.calls());
    let log = direct.device.calls();
    assert!(!log.iter().any(|call| call.contains("Attempting to reconnect")), "{log:?}");
    direct.update("radio.rx.s1.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(direct.now() + ms(50)));
    direct.until("the moved destination recovers from its recorded membership", |direct| calls(&direct.device, "rx_start 1").len() > before);
    assert!(direct.of("radio.COMMAND_REJECTED").len() == 1, "{:?}", direct.device.calls());
    assert!(!direct.device.calls().iter().any(|call| call.contains("Attempting to reconnect")), "{:?}", direct.device.calls());
    let _ = direct.finish();
}

/// An instance's Manifest section `suffix` (UR-30).
fn section_of(instance: &ezsdr_kernel::module_api::ProviderInstance, suffix: &str) -> Json {
    instance.sections[&Namespace::parse(&format!("ezsdr.radio.uhd.{suffix}")).unwrap()].clone()
}

/// The `timing` section's `plan` row (UR-30), each stream's segments in stream order.
fn plan_row(timing: &Json) -> Json {
    timing.as_array().unwrap().iter().find(|row| row["what"] == "plan").unwrap_or_else(|| panic!("no plan: {timing}")).clone()
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
    let run = spec_run(&receive_spec(1, 20e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Prepare, .. } }));
    let manifest = run.finish();
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
    let applied = fake_device.apply(ezsdr_radio_uhd::Dir::Rx, 0, &ezsdr_radio_uhd::Settings { rate: Some(200e6 / 257.0), ..ezsdr_radio_uhd::Settings::default() }, None).unwrap();
    assert_eq!(applied.rate, 200e6 / 256.0, "the fake clips like the device");
    let applied = fake_device.apply(ezsdr_radio_uhd::Dir::Rx, 0, &ezsdr_radio_uhd::Settings { rate: Some(200e6 / 2041.0), ..ezsdr_radio_uhd::Settings::default() }, None).unwrap();
    assert_eq!(applied.rate, 200e6 / 2040.0, "and to the top of the grid");
    // 200 MHz / 3 prepares, captures, and is reached by a cold switch as well.
    let dir = TempDir::new();
    let manifest = captured(spec_run(&receive_spec(1, 200e6 / 3.0, 1e9, Some(1_000)), &profile(&dir, json!({ "link_gbps": [10] }), json!({}), false), fake(FakeConfig::default()), BTreeMap::new()));
    assert_eq!(capture_of(&manifest, "rec").size_bytes, 8_000, "{}", failure(&manifest));
    assert_eq!(manifest.clocks.sample_clocks[0].root_ticks_per_tick.num(), 3);
    let dir = TempDir::new();
    let mut run = session(&profile(&dir, json!({ "link_gbps": [10] }), json!({}), true), fake(FakeConfig::default()));
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.rx.s0.sample_rate_hz", Value::num(200e6 / 7.0).unwrap()), None).unwrap()));
    wait(&mut run, ms(200));
    let manifest = run.finish();
    assert!(rejections(&manifest).is_empty(), "{:?}", rejections(&manifest));
    let new = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap()).nth(1).expect("the new receive clock");
    assert_eq!(new.root_ticks_per_tick.num(), 7);
}

#[test]
fn ur_13_arm_cases() {
    let dir = TempDir::new();
    let unlocked = fake(FakeConfig { faults: vec![FakeFault::Unlocked(Wall::ZERO)], ..FakeConfig::default() });
    let run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({ "clock_source": "external" }), json!({}), false), unlocked, BTreeMap::new());
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }));
    assert!(failure(&run.finish()).contains("UR-13: the external reference is not locked"));
    let run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({ "clock_source": "external" }), json!({}), false), fake(FakeConfig::default()), BTreeMap::new());
    running(&run);
    let _ = run.finish();
}

#[test]
fn ur_14_actions_are_finished_before_the_next_recv() {
    // KC-21a: an Action is finished when uhd-control takes the next, so its submit returns
    // only once it is booked. Enabling transmit configures the direction untimed, which
    // the fake's 20 ms `apply` makes slow (a timed update's `apply` does not wait), and
    // registers the clock: both are done when the submit returns (Review P, NB-2).
    let dir = TempDir::new();
    let device = fake(FakeConfig { apply_delay: Wall::from_millis(20), ..FakeConfig::default() });
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device.clone());
    past_t0(&mut run, ms(1));
    let entry = run.submit(set("radio.tx.c0.stream", Value::from(0)), None).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    assert!(!calls(&device, "apply tx").is_empty(), "{:?}", device.calls());
    let tx = ResourceId::parse("usrp/tx/s0").unwrap();
    assert!(run.sample_clocks().iter().any(|r| r.stream == tx && r.ended_at.is_none()), "{:?}", run.sample_clocks());
    let _ = run.finish();
}

#[test]
fn ur_14_the_control_thread_never_waits_for_a_device_instant() {
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device.clone());
    past_t0(&mut run, ms(1));
    // The switch is a restart lead (50 ms) away; booking it and the next Action takes
    // uhd-control a few milliseconds, and KC-21a returns each call when it has booked.
    assert!(admitted(&run.submit(set("radio.rx.s0.sample_rate_hz", Value::num(2e6).unwrap()), None).unwrap()));
    assert!(admitted(&run.submit(set("radio.rx.c0.gain_db", Value::num(3.0).unwrap()), None).unwrap()));
    // Command ordering is proved with a controlled clock by the library's
    // ur_14_booking_does_not_wait_for_a_future_cold_switch. A wall-time upper
    // bound here would also measure whether the host scheduled these threads.
    wait(&mut run, ms(200));
    let manifest = run.finish();
    let clocks: Vec<_> = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap()).collect();
    assert_eq!(clocks.len(), 2, "the rate change happened");
}

#[test]
fn ur_15_start_cases() {
    let dir = TempDir::new();
    let early = profile(&dir, json!({}), json!({ "ezsdr.time": { "class": "hardware_in_loop", "start_lead_ns": 1_000_000_000u64 } }), false);
    let run = spec_run(&receive_spec(1, 1e6, 1e9, None), &early, fake(FakeConfig::default()), BTreeMap::new());
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }));
    let manifest = run.finish();
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
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new());
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
    let mut run = spec_run(&spec, &profile(dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(waveform.hash.clone(), bytes)]));
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
        Err(_) => panic!("{what} did not finish within {bound:?}"),
    }
}

fn tx_session(config: FakeConfig) -> (RunHandle, Arc<FakeDevice>, TempDir) {
    let dir = TempDir::new();
    let device = fake(config);
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device.clone());
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.tx.c0.stream", Value::from(0)), None).unwrap()));
    past_tx_origin(&mut run);
    (run, device, dir)
}

/// The same transmit Session on any `Device` (a test wrapper, plan 30 §7).
fn tx_session_on(device: Arc<dyn Device>) -> (RunHandle, TempDir) {
    let dir = TempDir::new();
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device);
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.tx.c0.stream", Value::from(0)), None).unwrap()));
    past_tx_origin(&mut run);
    (run, dir)
}

/// A Spec Run advanced to `ticks` after T0, then finished. The Run's clock is the device's,
/// which may already be past a `now()`-relative horizon before T0 (plan 30 §4.4).
fn finish_spec(mut run: RunHandle, ticks: i64) -> Manifest {
    let t0 = run.start_instant().expect("a Spec Run starts");
    let _ = run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ticks));
    run.finish()
}

/// The transmit sample `lead` root ticks from now, on the running transmit clock.
fn tx_at(run: &RunHandle, lead: i64) -> TimePoint {
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx/s0").unwrap() && r.ended_at.is_none()).unwrap();
    let n = clock.root_ticks_per_tick.num() as i64;
    TimePoint::new(clock.domain, (run.now().ticks_in(clock.root).unwrap() + lead - clock.origin.ticks_in(clock.root).unwrap() + n - 1).div_euclid(n))
}

fn send_at(run: &mut RunHandle, verb_name: &str, at: Option<TimePoint>, samples: &[(f32, f32)]) -> ezsdr_kernel::session::LogEntry {
    let (bytes, _) = waveform_of(samples);
    run.submit(verb(verb_name, resource("radio/tx/s0"), at, &[]), Some(&bytes)).unwrap()
}

fn send(run: &mut RunHandle, verb_name: &str, lead: Option<i64>, samples: &[(f32, f32)]) -> ezsdr_kernel::session::LogEntry {
    let at = lead.map(|lead| tx_at(run, lead));
    send_at(run, verb_name, at, samples)
}

/// A waveform's content hash, for the inputs map of a Spec that names it twice.
fn hash_of(bytes: &[u8]) -> ContentHash {
    ContentHash::of_bytes(bytes)
}

/// A second Spec input under its own name, for a Spec whose schedule names two waveforms
/// (KC-9).
fn second_waveform(samples: &[(f32, f32)]) -> (Vec<u8>, ArtifactRef) {
    let (bytes, _) = waveform_of(samples);
    let reference = ezsdr_kernel::manifest::ingest_input(
        Ident::parse("waveform2").unwrap(),
        Namespace::parse("ezsdr.input").unwrap(),
        format!("mem:{}", ContentHash::of_bytes(&bytes)),
        &bytes,
    );
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

/// Every `COMMAND_REJECTED` reason in `events`, in order (RM-22); one per event, so a count
/// of reasons is a count of refusals.
fn rejection_reasons(events: &[Event]) -> Vec<String> {
    events.iter().map(|event| event.payload["reason"].as_str().unwrap_or_else(|| panic!("a rejection without a reason: {event:?}")).to_owned()).collect()
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
    assert!(reasons.iter().any(|r| r.starts_with("UR-21: a held burst already has this start")), "{reasons:?}");
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
    assert!(errors[2..].iter().all(|p| p.outcome == TimeErrorOutcome::SendAsap && (p.target == second_at || p.target == third_at)),
        "{errors:#?}");
    // The third was booked and played: the burst is on the device with the request kept when
    // the booking moved it, at or after the request.
    let records = bursts(&manifest);
    assert_eq!(records.len(), 2, "{records:?}");
    let third = &records[1];
    assert!(third.target.ticks_in(third.target.domain()).unwrap() >= third_at.ticks_in(third_at.domain()).unwrap(), "{third:?} requested {third_at:?}");
    assert!(third.requested_target.is_none() || third.requested_target == Some(third_at), "{third:?} requested {third_at:?}");
}

#[test]
fn ur_21_an_untimed_send_is_on_time_after_its_delivery() {
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    for _ in 0..20 {
        let _ = send(&mut run, "send", None, &tone(100));
        wait(&mut run, ms(5));
    }
    wait(&mut run, ms(30));
    let manifest = run.finish();
    assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
    assert_eq!(bursts(&manifest).len(), 20);
}

#[test]
fn ur_21_an_untimed_burst_after_a_cold_change_is_on_time_at_its_clock_s_origin() {
    // RS-19: with no `at`, the burst is admitted at the new transmit clock's origin, which
    // uhd-control registered before the Kernel's KC-21a wait returned (UR-25), so it is on time.
    let (mut run, _device, _dir) = tx_session(FakeConfig::default());
    assert!(admitted(&run.submit(set("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap()), None).unwrap()));
    let entry = send(&mut run, "start_repeat", None, &tone(200));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(300));
    let manifest = run.finish();
    assert_eq!(time_errors(&manifest), []);
    // At the origin, tick 0, unless host load let more than the start lead less the device
    // lead pass before admission (#60's class); either way not before it and not late.
    let record = &bursts(&manifest)[0];
    assert!(record.target.ticks_in(record.target.domain()).unwrap() >= 0 && record.late_by.is_none(), "{record:?}");
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
    let sends = calls(&device, "tx_send");
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
    let failure = std::panic::catch_unwind(|| require_stream_timing(&manifest, "diagnostic regression", true))
        .expect_err("a missed prerequisite must fail, not skip or pass the trial");
    let message = failure.downcast_ref::<String>().unwrap();
    assert!(message.contains("TIMING_PRECONDITION_UNMET (diagnostic regression)"));
    assert!(message.contains("radio.TIME_ERROR") && message.contains("drop"));
}

#[test]
fn ur_23_a_burst_ends_at_the_next_bursts_start() {
    // Both bursts are booked in the Spec, before the Run starts (rule (1)): a repeat at
    // 10 ms, cut by a burst at 20 ms.
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
        { "at": { "clock": "radio", "offset_ticks": 20_000 }, "action": { "kind": "tx_burst", "target": target,
            "waveform": b, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } }
    ]);
    let manifest = finish_spec(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(ha, one), (hb, two)])), ms(60));
    let records = bursts(&manifest);
    assert_eq!(records.len(), 2, "{records:?}");
    // The repeat ends at the second's start with end-of-burst, not as a stopped burst: the
    // block that ends it carries END_OF_BURST (`ends_at_held`, SC-24/SC-28).
    assert_eq!(records[0].end, BurstEnd::Eob, "{records:?}");
    assert_eq!(records[0].target.ticks_in(records[0].target.domain()).unwrap() + records[0].samples as i64, records[1].target.ticks_in(records[1].target.domain()).unwrap());
    assert!(records[1].late_by.is_none());
    assert!(time_errors(&manifest).is_empty());
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError"), "{:?}", device.calls());
    // Two Kernel bursts, one device burst: the second continues the first without
    // end-of-burst or a timed start at the tick the first ended (design-notes §11 F3).
    let sends = calls(&device, "tx_send");
    assert_eq!(sends.iter().filter(|c| c.contains("sob=true")).count(), 1, "{sends:?}");
    assert_eq!(sends.iter().filter(|c| c.ends_with("eob=true")).count(), 1, "{sends:?}");
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
    let sends = calls(&device, "tx_send");
    assert_eq!(sends.iter().filter(|call| call.ends_with("eob=true")).count(), 1, "{sends:?}");
    assert_eq!(device.unended_bursts(), 0, "a burst started inside one without end-of-burst: {:?}", device.calls());
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "Underflow"), "{:?}", device.calls());
}

// ---------------------------------------------------------------- updates (UR-24…UR-26)

fn late_commands(manifest: &Manifest) -> usize {
    events_of(manifest, "radio.LATE_COMMAND").len()
}

#[test]
fn ur_24_an_untimed_retune_is_on_time_after_its_delivery() {
    let dir = TempDir::new();
    let mut run = session(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()));
    past_t0(&mut run, ms(1));
    for i in 0..10 {
        assert!(admitted(&run.submit(set("radio.rx.c0.frequency_hz", Value::num(1e9 + f64::from(i)).unwrap()), None).unwrap()));
        wait(&mut run, ms(5));
    }
    assert_eq!(late_commands(&run.finish()), 0);
}

// `ur_24_the_device_queue_order_is_the_effective_order` is a unit test in
// `src/provider/control.rs`, on the controlled clock: its case, a command earlier than one
// already released, was left to host timing here, which the manual clock no longer provides.

// `ur_25_the_switch_applies_the_configuration_in_effect_at_e2` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
#[test]
fn ur_24_a_released_command_is_not_recalled_by_stop() {
    // UR-24, UR-26, UR-30, RM-16 (the exit review's clause): a timed command already
    // released to the device (inside the 3 ms release window) when a `Stop` comes is not
    // recalled: it is applied at its `e`, and recorded in `applied` as issued, not cancelled.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let radio = serde_json::to_value(resource("radio")).unwrap();
    let mut spec = receive_spec(1, 1e6, 1e9, None);
    spec["schedule"] = json!([{
        "at": { "clock": "radio", "offset_ticks": 20_000 },
        "action": { "kind": "update_parameter", "target": radio, "key": "radio.rx.c0.frequency_hz", "value": 2.3e9 }
    }, {
        "at": { "clock": "radio", "offset_ticks": 19_500 },
        "action": { "kind": "stop", "target": radio }
    }]);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new());
    let t0 = run.start_instant().unwrap();
    run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ms(100))).unwrap();
    let manifest = run.finish();
    let applied = calls(&device, "apply rx 0 rate=- freq=2300000000");
    assert_eq!(applied.len(), 1, "{:?}", device.calls());
    assert!(!applied[0].contains("at=-"), "a timed apply: {applied:?}");
    let rows: Vec<_> = section(&manifest, "applied").as_array().unwrap().iter().filter(|r| r["key"] == "radio.rx.c0.frequency_hz" && r["claimed"] == 2.3e9).cloned().collect();
    assert!(rows.iter().any(|r| r["issued"] == true), "{rows:?}");
    assert!(rows.iter().all(|r| r.get("cancelled").is_none()), "{rows:?}");
}

#[test]
fn ur_24_stop_cancels_held_commands() {
    // RM-16, UR-26 (spec 22, VH-6): a `Stop` of the device cancels no held timed command —
    // the one for 5 ms after it is issued —, and `Provider::stop` cancels those still held.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let mut spec = receive_spec(1, 1e6, 1e9, None);
    let update = |offset: i64, value: f64| json!({
        "at": { "clock": "radio", "offset_ticks": offset },
        "action": { "kind": "update_parameter", "target": serde_json::to_value(resource("radio")).unwrap(),
                    "key": "radio.rx.c0.frequency_hz", "value": value }
    });
    spec["schedule"] = json!([update(1_000_000, 2.3e9), update(10_000_000_000, 2.2e9), {
        "at": { "clock": "radio", "offset_ticks": 10_000 },
        "action": { "kind": "stop", "target": serde_json::to_value(resource("radio")).unwrap() }
    }]);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new());
    let t0 = run.start_instant().unwrap();
    run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ms(1_100))).unwrap();
    let manifest = run.finish();
    assert_eq!(calls(&device, "apply rx 0 rate=- freq=2300000000").len(), 1, "{:?}", device.calls());
    assert!(calls(&device, "apply rx 0 rate=- freq=2200000000").is_empty(), "{:?}", device.calls());
    let applied = section(&manifest, "applied").as_array().unwrap().clone();
    assert!(applied.iter().all(|r| r["cancelled"] != "Stop"), "{applied:?}");
    assert!(applied.iter().any(|r| r["cancelled"] == "Provider::stop" && r["claimed"] == 2.2e9), "{applied:?}");
}




#[test]
fn ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time() {
    for _ in 0..5 {
        let (mut run, device, _dir) = tx_session(FakeConfig::default());
        let entry = send(&mut run, "start_repeat", None, &tone(500));
        assert!(admitted(&entry), "{entry:?}");
        wait(&mut run, ms(40));
        let manifest = run.finish();
        assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
        let calls = device.calls();
        let configured = calls.iter().position(|c| c.starts_with("apply tx 0 rate=1000000")).unwrap();
        let sent = calls.iter().position(|c| c.starts_with("tx_send") && c.contains("sob=true")).unwrap();
        assert!(configured < sent);
    }
}

#[test]
fn ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration() {
    // UR-25, RM-25 (spec 20, VF-6): a receive stream enabled from 0 channels starts at the
    // first lattice instant at or after both `e` and `c`, the end of its configuration, plus
    // the profile's start lead. The `enabled` row records `e` and `c` (UR-30), so the origin
    // is checked exactly, whatever the load: one untimed `apply` takes 80 ms here.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let config = FakeConfig { apply_delay: Wall::from_millis(80), ..FakeConfig::default() };
    let mut direct = Direct::with_links(config, &[("radio.rx.c0.stream", Scalar::from(-1))], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    direct.update("radio.rx.c0.stream", Value::from(0), Cold, None);
    direct.settle(Wall::from_millis(300));
    let instance = direct.finish();
    let envelope = &section_of(&instance, "envelope");
    let lead = envelope["timing"]["rx_start_lead_ns"].as_i64().unwrap() * MCR / 1_000_000_000;
    let timing = &section_of(&instance, "timing");
    let row = timing.as_array().unwrap().iter().find(|r| r["what"] == "cold_change").cloned().unwrap_or_else(|| panic!("{timing}"));
    let segment = timing.as_array().unwrap().iter().find(|r| r["what"] == "rx_segment").cloned().unwrap_or_else(|| panic!("{timing}"));
    let (e, configured, origin) = (row["e"].as_i64().unwrap(), row["ready"].as_i64().unwrap(), segment["origin"].as_i64().unwrap());
    // `configured` is the configuration's own end (`ready`); the exact relation is the origin's
    // and is checked here, and the end's own recording and the lead from it are pinned by the
    // unit test `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`
    // in `src/provider/control.rs`, whose controlled clock records a known end.
    assert_eq!(origin, (e.max(configured + lead) + 199) / 200 * 200, "{row}, lead {lead}");
}


#[test]
fn ur_25_a_cold_change_the_envelope_refuses_changes_nothing() {
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::new(FakeConfig::default(), &[("radio.rx.c0.stream", Scalar::from(0)), ("radio.rx.c1.stream", Scalar::from(0))]);
    let before = direct.device.calls().len();
    // Two channels at 200 Msps need 1.6 GB/s on a 100 MB/s 1 GbE link (RM-32).
    direct.update("radio.rx.s0.sample_rate_hz", Value::num(200e6).unwrap(), Cold, None);
    direct.settle(Wall::from_millis(30));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1);
    assert!(rejection_reason(&rejected).starts_with("RM-32: "), "{rejected:?}");
    // MA-14: the refusal names the Action it answers.
    assert_eq!(rejected[0].payload["action"], 1, "{rejected:?}");
    assert!(direct.device.calls()[before..].iter().all(|c| !c.starts_with("apply") && !c.starts_with("rx_stop")));
    let _ = direct.finish();
}

#[test]
fn ur_14_a_device_error_while_booking_names_its_action() {
    // UR-14, RM-22 (spec 27 §3): a receive enable from no stream whose configuration
    // fails without the device being lost (the device applying another rate, UR-12) is
    // refused at booking, and the refusal names the Action.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let config = FakeConfig { faults: vec![FakeFault::WrongRate { claimed: 1e6, applied: 0.5e6, nth: 0 }], ..FakeConfig::default() };
    let mut direct = Direct::with_links(config, &[("radio.rx.c0.stream", Scalar::from(-1))], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    direct.update("radio.rx.c0.stream", Value::from(0), Cold, None);
    direct.settle(Wall::from_millis(100));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?} {:?}", direct.device.calls());
    assert!(rejection_reason(&rejected).starts_with("UR-12: the device applied"), "{rejected:?}");
    assert_eq!(rejected[0].payload["action"], 1, "{rejected:?}");
    let _ = direct.finish();
}

#[test]
fn ur_25_enabling_tx_applies_the_configuration() {
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let mut spec_profile = profile(&dir, json!({}), json!({}), true);
    spec_profile["environment"]["radio.rf_envelope"] = json!({ "allowed_bands": [{ "lo_hz": 1e8, "hi_hz": 5e9 }] });
    let mut run = session(&spec_profile, device.clone());
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.tx.c0.frequency_hz", Value::num(2.4e9).unwrap()), None).unwrap()));
    assert!(admitted(&run.submit(set("radio.tx.c0.gain_db", Value::num(5.0).unwrap()), None).unwrap()));
    assert!(admitted(&run.submit(set("radio.rx.c0.frequency_hz", Value::num(2.4e9).unwrap()), None).unwrap()));
    wait(&mut run, ms(20));
    assert!(admitted(&run.submit(set("radio.tx.c0.stream", Value::from(0)), None).unwrap()));
    let calls = device.calls();
    assert!(calls.iter().any(|c| c.starts_with("apply tx 0 rate=1000000 freq=2400000000 gain=5 antenna=TX/RX at=-")), "{calls:?}");
    past_tx_origin(&mut run);
    let wave = tone(1_000);
    let _ = send(&mut run, "start_repeat", Some(ms(20)), &wave);
    wait(&mut run, ms(40));
    let capture_at = after(&run, ms(5));
    assert!(admitted(&run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", Value::from(2_000))]), None).unwrap()));
    let horizon = after(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let manifest = run.finish();
    let samples = read_capture(&capture_of(&manifest, "rec"), 1).remove(0);
    let first = samples.iter().position(|s| wave.contains(s)).unwrap_or_else(|| {
        panic!("the loopback came back: async {} TIME_ERROR {:?}", section(&manifest, "async"), time_errors(&manifest))
    });
    assert!(samples[first..].iter().zip(wave.iter().cycle().skip(wave.iter().position(|w| *w == samples[first]).unwrap())).all(|(a, b)| a == b));
}

#[test]
fn ur_25_a_rate_change_admits_a_burst_on_the_new_clock() {
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    assert!(admitted(&run.submit(set("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap()), None).unwrap()));
    let entry = send(&mut run, "start_repeat", None, &tone(100));
    assert!(admitted(&entry), "{entry:?}");
    // Booked before the switch (50 ms after receipt), transmitted after it on the new clock.
    wait(&mut run, ms(150));
    let manifest = run.finish();
    assert!(events_of(&manifest, "radio.COMMAND_REJECTED").is_empty(), "{:?}", events_of(&manifest, "radio.COMMAND_REJECTED"));
    let new = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/tx/s0").unwrap()).nth(1).expect("the new transmit clock");
    assert!(bursts(&manifest).iter().any(|b| b.target.domain() == new.domain), "{:?}", bursts(&manifest));
}

#[test]
fn ur_25_a_cold_transmit_change_cancels_the_held_bursts() {
    // UR-23: a cold change still reports each held burst it cancels on the old clock. Both
    // at one fixed offset, the burst first, so that both are booked before the Run starts and
    // in that order (rule (1)): the change cuts the old clock at the burst's own start.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let (bytes, waveform) = waveform_of(&tone(100));
    let target = serde_json::to_value(resource("radio/tx/s0")).unwrap();
    let mut spec = with_tx(receive_spec(1, 1e6, 1e9, None), 1e6);
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 300_000 }, "action": { "kind": "tx_burst", "target": target,
            "waveform": waveform, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 300_000 }, "action": { "kind": "update_parameter", "target": serde_json::to_value(resource("radio")).unwrap(),
            "key": "radio.tx.s0.sample_rate_hz", "value": 2e6 } }
    ]);
    let manifest = finish_spec(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(hash_of(&bytes), bytes)])), ms(400));
    let events: Vec<_> = events_of(&manifest, "radio.COMMAND_REJECTED").into_iter().filter(|event| event.payload["kind"] == "tx_burst").collect();
    let rejected: Vec<_> = section(&manifest, "rejected").as_array().unwrap().iter().filter(|row| row["kind"] == "tx_burst").collect();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["reason"], "cancelled by a cold change");
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert_eq!(rejected[0]["reason"], "cancelled by a cold change");
    assert!(bursts(&manifest).is_empty(), "the cancelled waveform was not sent");
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
    assert_eq!(rejection_reason(&rejected), "UR-26: radio.rx.antenna is not updated at runtime by this Provider");
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
    let origin = direct.clocks.sample_clock_records().iter().find(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap()).unwrap().origin.ticks_in(direct.clocks.sample_clock_records().iter().find(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap()).unwrap().root).unwrap();
    let end = origin + end.expect("blocks were published") * 200;
    assert!(end < stop, "published up to {end}, stopped at {stop}");
    // One stop, recorded once: not again when the tail reaches the cut (Review N, N4).
    let calls = direct.device.calls();
    assert_eq!(calls.iter().filter(|c| c.starts_with("rx_stop")).count(), 1, "{calls:?}");
    let applied = &section_of(&instance, "applied");
    assert_eq!(applied.as_array().unwrap().iter().filter(|r| r["key"] == "rx_stop").count(), 1, "{applied}");
}


#[test]
fn ur_25_a_receive_session_follows_its_recorded_plan() {
    // Spec 22, VH-8: the receive path end to end — uhd-control's booking, uhd-rx's thread, the
    // streamer and the device's commands — carries out the plan uhd-control records: after two
    // `cold` rate changes, a `Stop`, `start_rx` and the Run's stop, the plan has four segments,
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
    assert!(admitted(&run.submit(set("radio.rx.s0.sample_rate_hz", Value::num(2e6).unwrap()), None).unwrap()));
    wait(&mut run, ms(100));
    assert!(admitted(&run.submit(SessionAction::Stop { target: Some(resource("radio/rx/s0")) }, None).unwrap()));
    wait(&mut run, ms(20));
    assert!(admitted(&run.submit(verb("start_rx", resource("radio/rx/s0"), None, &[]), None).unwrap()));
    wait(&mut run, ms(100));
    assert!(admitted(&run.submit(set("radio.rx.s0.sample_rate_hz", Value::num(1e6).unwrap()), None).unwrap()));
    wait(&mut run, ms(150));
    let manifest = run.finish();
    let timing = section(&manifest, "timing").clone();
    let plan = plan_row(&timing);
    let segments: Vec<(i64, Option<i64>)> = plan["rx"].as_array().unwrap().iter().map(|s| (s["origin"].as_i64().unwrap(), s["end"].as_i64())).collect();
    assert_eq!(segments.len(), 4, "{plan}");
    for pair in segments.windows(2) {
        assert!(pair[0].1.is_some_and(|end| end <= pair[1].0), "segments out of order or overlapping: {plan}");
    }
    assert!(segments[3].1.is_some(), "the Run's stop ends the last: {plan}");
    let clocks: Vec<(i64, Option<i64>)> = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/rx/s0").unwrap())
        .map(|r| (r.origin.ticks_in(r.root).unwrap(), r.ended_at.map(|t| t.ticks_in(r.root).unwrap()))).collect();
    let stops: Vec<i64> = section(&manifest, "applied").as_array().unwrap().iter().filter(|r| r["key"] == "rx_stop").filter_map(|r| r["at"]["ticks"].as_i64()).collect();
    assert!(!clocks.is_empty(), "{timing:?}");
    let mut next = 0;
    for (origin, end) in &clocks {
        let index = segments[next..].iter().position(|(o, _)| o == origin).map(|i| next + i)
            .unwrap_or_else(|| panic!("a clock at {origin} that is no segment of the plan, or out of order: {timing:?}"));
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
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({ "clock_source": "external" }), json!({}), false), device, BTreeMap::new());
    let result = run.run_until_end(after(&run, ms(5_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind("radio.CLOCK_LOST") } });
    assert_eq!(events_of(&manifest, "radio.CLOCK_LOST")[0].payload, json!({ "reference": "frequency" }));
    assert_eq!(events_of(&manifest, "radio.CLOCK_LOST")[0].source, usrp(), "RM-11's source is the device node");
}

#[test]
fn ur_28_underflows_are_tx_underflow() {
    let config = FakeConfig {
        faults: vec![FakeFault::TxReport(Wall::from_millis(2_100), TxCode::Underflow), FakeFault::TxReport(Wall::from_millis(2_100), TxCode::SeqError)],
        ..FakeConfig::default()
    };
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
    let (mut run, _, _dir) = tx_session(FakeConfig { faults: vec![FakeFault::TxAsyncFails(Wall::from_millis(2_100), true)], ..FakeConfig::default() });
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
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new());
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
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new());
    // The property is about device time: the Run must end long before its 8 s horizon, not
    // after 7 s of wall time, which a lagging manual clock would break (plan 30 §4.4). The
    // device's own clock measures it; the wall bound is only a watchdog.
    let begun = device.host().now();
    let wall = Instant::now();
    let result = run.run_until_end(after(&run, ms(8_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    assert!(device.host().now() - begun < Wall::from_secs(7), "the Run used {:?} of device time", device.host().now() - begun);
    assert!(wall.elapsed() < Wall::from_secs(30), "the Run took {:?} of wall time", wall.elapsed());
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
    let mut run = session(&doc, device.clone());
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
    let mut run = session(&doc, device.clone());
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
    for suffix in ["device", "envelope", "applied", "bursts", "async", "rejected", "timing", "stats"] {
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
    device.apply(dir, 1, &set, None).unwrap();
    device.set_subdev_spec(dir, "A:1").unwrap();
    assert_eq!(device.apply(dir, 0, &ezsdr_radio_uhd::Settings::default(), None).unwrap().freq, 2e9, "A:1's setting stayed with it");
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
    assert_eq!(
        refuse("A:1"),
        "uhd_usrp_set_rx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev A:1",
        "a pair the table has not",
    );
    assert_eq!(
        refuse("0"),
        "uhd_usrp_set_rx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev :0",
        "a bare front end names no slot",
    );
    assert_eq!(
        refuse("A:0 A:0"),
        "uhd_usrp_set_rx_subdev_spec: UHD error 43: ValueError: duplicate subdev A:0",
        "a repeated pair",
    );
    assert_eq!(
        refuse(""),
        "uhd_usrp_set_rx_subdev_spec: UHD error 43: ValueError: empty subdev-spec markup string",
        "an empty spec",
    );
    assert_eq!(
        refuse("A:0:1"),
        "uhd_subdev_spec_make: UHD error 43: ValueError: invalid subdev-spec markup string: A:0:1",
        "a malformed markup",
    );
    assert_eq!(
        refuse("A:0\tB:0"),
        "uhd_subdev_spec_make: UHD error 43: ValueError: invalid subdev-spec markup string: A:0\tB:0",
        "a tab is no separator: UHD splits on spaces alone",
    );
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
    assert!(matches!(device.rx_recv(0, 1, Wall::from_millis(1)), ezsdr_radio_uhd::RxRecv::Failed(_)), "a refused reopen leaves the stream with none");
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
    assert!(matches!(device.rx_recv(0, 1, Wall::from_millis(1)), ezsdr_radio_uhd::RxRecv::Failed(_)), "the receive streamer went with the map");
    let out = [[0.0f32, 0.0]; 4];
    assert!(device.tx_send(0, &[&out], None, false, false, Wall::from_millis(1)).is_ok(), "the transmit streamer stays");
    // The transmit side the same: its streamer goes with its map, the receive one stays.
    let device = fake(FakeConfig::default());
    device.rx_open(0, &[0]).unwrap();
    device.tx_open(0, &[0]).unwrap();
    device.set_subdev_spec(ezsdr_radio_uhd::Dir::Tx, "A:0").unwrap();
    assert!(device.tx_send(0, &[&out], None, false, false, Wall::from_millis(1)).is_err(), "the transmit streamer went with its map");
    assert!(matches!(device.rx_recv(0, 1, Wall::from_millis(1)), ezsdr_radio_uhd::RxRecv::Timeout), "the receive streamer stays: its call waits, not fails");
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

/// The receive link as the bench measured it (INFERRED from its stops: 2–4 ms), with the
/// X300's 1 996-sample packets.
fn bench_link(latency_ms: u64) -> FakeConfig {
    FakeConfig { rx_latency: Wall::from_millis(latency_ms), rx_packet: Some(1_996), ..FakeConfig::default() }
}

/// Captures across `changes` `cold` receive rate changes in one Session on `config`,
/// alternating `rates`, and for each the old clock's end, the capture's maps and whether
/// every sample of each map is its own clock's ramp (the fake without a transmitter).
fn captures_across_cold_changes(config: FakeConfig, selector: Json, rates: [f64; 2], changes: usize) -> (Manifest, Vec<String>) {
    let dir = TempDir::new();
    let device = fake(config);
    // Every capture and every change is booked before the Run starts (rule (1)): each is an
    // offset of the Spec's schedule, counted in the receive stream's clock at the arm's rate.
    // Each capture begins 64 ms before its change and spans its samples across both rates, so
    // it holds both clocks' samples; the next begins after the last one ends. The Run's tail
    // is long enough for the last capture to be whole — a run end would make it `partial`.
    let tick = |ms: i64| (rates[0] * ms as f64 / 1_000.0) as i64;
    let mut schedule = Vec::new();
    let mut planned = Vec::new();
    let mut last = 0;
    for i in 0..changes {
        let n = (rates[i % 2] * 0.12) as i64 + (rates[(i + 1) % 2] * 0.08) as i64;
        planned.push(n);
        let start = 200 + i as i64 * 320;
        schedule.push(json!({ "at": { "clock": "radio", "offset_ticks": tick(start) }, "action": {
            "kind": "update_parameter", "target": { "kind": "output", "output": "rec" }, "key": "sink.capture_samples", "value": n } }));
        schedule.push(json!({ "at": { "clock": "radio", "offset_ticks": tick(start + 64) }, "action": {
            "kind": "update_parameter", "target": serde_json::to_value(resource("radio")).unwrap(),
            "key": "radio.rx.s0.sample_rate_hz", "value": rates[(i + 1) % 2] } }));
        last = start;
    }
    let mut spec = receive_spec(1, rates[0], 1e9, None);
    spec["schedule"] = json!(schedule);
    let manifest = finish_spec(spec_run(&spec, &profile(&dir, selector, json!({}), false), device.clone(), BTreeMap::new()), ms(last + 2_000));
    let clocks = &manifest.clocks.sample_clocks;
    let mut problems = Vec::new();
    // The bounded wait never goes under 1 ms (UHD truncates the timeout to whole ms).
    if device.min_recv_timeout() < Wall::from_millis(1) {
        problems.push(format!("a receive wait of {:?}", device.min_recv_timeout()));
    }
    let captures: Vec<_> = manifest.artifacts.iter().filter(|(output, _)| output.as_str() == "rec").flat_map(|(_, list)| list).collect();
    // Every planned capture is written, whole: an empty or missing capture would otherwise
    // skip the clock, cut and ramp checks below and still leave `problems` empty.
    if captures.len() != changes {
        problems.push(format!("{} captures for {changes} planned: {:?}", captures.len(), captures.iter().map(|a| a.id.as_str().to_owned()).collect::<Vec<_>>()));
    }
    for (index, artifact) in captures.iter().enumerate() {
        // Each capture begins before its change, so it holds both clocks' samples; all of
        // its planned samples are recorded, or the Run cut it short.
        if artifact.continuity.len() != 2 {
            problems.push(format!("{}: the samples of {} clocks, not two: {:?}", artifact.id, artifact.continuity.len(), artifact.continuity));
        }
        if artifact.partial {
            problems.push(format!("{}: the capture is partial", artifact.id));
        }
        let data = read_capture(artifact, 1).remove(0);
        if data.is_empty() {
            problems.push(format!("{}: no samples", artifact.id));
            continue;
        }
        if planned.get(index).is_some_and(|n| data.len() as i64 != *n) {
            problems.push(format!("{}: {} samples for {} planned", artifact.id, data.len(), planned[index]));
        }
        let mut at = 0usize;
        for (m, map) in artifact.continuity.iter().enumerate() {
            let clock = clocks.iter().find(|c| c.domain == map.domain).unwrap();
            let n = clock.root_ticks_per_tick.num() as i64;
            let valid: u64 = map.valid.iter().flatten().map(|segment| segment.len).sum();
            if valid == 0 {
                problems.push(format!("{}: map {m} has no valid sample of its own clock", artifact.id));
            }
            if m == 0 && artifact.continuity.len() > 1 {
                let e1 = (clock.ended_at.unwrap().ticks_in(clock.root).unwrap() - clock.origin.ticks_in(clock.root).unwrap()) / n;
                if map.end.ticks_in(map.domain).unwrap() != e1 {
                    problems.push(format!("{}: the old clock's samples end at {} for e₁ {e1}", artifact.id, map.end.ticks_in(map.domain).unwrap()));
                }
            }
            // A missed start's gap leads the map (SC-13); the samples begin after it.
            let first = map.valid[0].first().map_or(map.end.ticks_in(map.domain).unwrap(), |s| s.start.ticks_in(map.domain).unwrap());
            let len = (map.end.ticks_in(map.domain).unwrap() - first) as usize;
            for i in 0..len {
                let tick = clock.origin.ticks_in(clock.root).unwrap() + (first + i as i64) * n;
                let ramp = ((tick / n).rem_euclid(65_536)) as f32 / 65_536.0;
                if data[at + i].0 != ramp {
                    problems.push(format!("{}: map {m} sample {} is {}, its clock's ramp {ramp}", artifact.id, first + i as i64, data[at + i].0));
                    break;
                }
            }
            at += len;
        }
    }
    for event in events_of(&manifest, "radio.LATE_COMMAND") {
        problems.push(format!("LATE_COMMAND {}", event.payload));
    }
    (manifest, problems)
}

#[test]
fn ur_25_a_long_block_at_a_low_rate_stops_the_stream_at_e1() {
    // Review N, B1: at 390 625 S/s a block of 65 536 is 168 ms, longer than the restart
    // lead. The stop must still come at e₁, the switch before e₂, and no old-clock sample
    // may be published on the new clock.
    let (_, problems) = captures_across_cold_changes(bench_link(2), json!({ "block_len": 65_536 }), [390_625.0, 2e6], 2);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn ur_25_a_slow_link_at_a_low_rate_delivers_the_old_clock_to_e1() {
    // Review N, B2: packets of 5 ms at 390 625 S/s delivered 3.5 ms late; the old stream
    // must not be ended by a timeout before its samples up to e₁ arrive.
    let config = FakeConfig { rx_latency: Wall::from_micros(3_500), ..bench_link(0) };
    let (_, problems) = captures_across_cold_changes(config, json!({}), [390_625.0, 400_000.0], 5);
    assert!(problems.is_empty(), "{problems:#?}");
}



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
    assert!(codes.contains(&"BurstAck".to_owned()) && !codes.contains(&"Underflow".to_owned()), "{codes:?} {:?}", calls(&device, "tx_send"));
}

#[test]
fn ur_25_a_late_switch_drops_the_old_stream_s_tail() {
    // Review N, B1's second guard: a link so slow (60 ms) that the stop at e₁ comes after
    // e₂; the switch is late (LATE_COMMAND, UR-17's restart), but the old stream's tail,
    // still in flight after its stop and past the new origin, is not published on the new
    // clock.
    let config = FakeConfig { rx_latency: Wall::from_millis(60), ..bench_link(0) };
    let (_, problems) = captures_across_cold_changes(config, json!({}), [1e6, 2e6], 1);
    let problems: Vec<_> = problems.into_iter().filter(|p| !p.starts_with("LATE_COMMAND")).collect();
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn ur_29_a_stream_silent_before_a_far_cut_is_a_lost_device() {
    // Review N, N7: a `cold` change booked far ahead does not switch UR-29's silence check
    // off until its e₁.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::with_links(
        FakeConfig { faults: vec![FakeFault::Silence(Wall::from_millis(2_300))], ..FakeConfig::default() },
        &[],
        vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)],
    );
    let now = direct.now();
    direct.update("radio.rx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(now + ms(5_000)));
    direct.settle(Wall::from_millis(2_000));
    let lost = direct.of(EventKind::DEVICE_LOST);
    let _ = direct.finish();
    assert_eq!(lost.len(), 1, "{lost:?}");
}

#[test]
fn ur_23_a_burst_one_sample_after_a_burst_is_played() {
    // Review O, O-B1 (`hw_b8_raw_empty_eob_gap`): the device burst of a burst ending at c
    // must end at c, not a padded sample later, so that a burst at c + 1 is played. Both are
    // booked in the Spec, before the Run starts (rule (4); the booking window is a unit test,
    // `ur_23_a_burst_one_sample_after_a_burst_is_played` in `src/provider/tx.rs`). A's
    // device burst is 30 000 samples and B's 1 000, each with its own start.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let (one, a) = waveform_of(&tone(30_000));
    let (two, b) = second_waveform(&vec![(-0.25f32, 0.125); 1_000]);
    let (ha, hb) = (a.hash.clone(), b.hash.clone());
    let target = serde_json::to_value(resource("radio/tx/s0")).unwrap();
    let mut spec = with_tx(receive_spec(1, 1e6, 1e9, None), 1e6);
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 10_000 }, "action": { "kind": "tx_burst", "target": target.clone(),
            "waveform": a, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 40_001 }, "action": { "kind": "tx_burst", "target": target,
            "waveform": b, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } }
    ]);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(ha, one), (hb, two)]));
    let t0 = run.start_instant().expect("a Spec Run starts");
    let root = t0.ticks_in(t0.domain()).unwrap();
    let _ = run.advance_to(TimePoint::new(t0.domain(), root + ms(100)));
    let manifest = run.finish();
    assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
    // Each device burst is its Kernel burst's samples and no more: an empty end-of-burst
    // would add a zero sample, ending A at c + 1, B's start.
    let sends = tx_sends(&device);
    let device_bursts: Vec<&[Sent]> = sends.split_inclusive(|s| s.eob).collect();
    assert_eq!(device_bursts.len(), 2, "{sends:?}");
    for (burst, at, len) in [(device_bursts[0], root + ms(10), 30_000), (device_bursts[1], root + ms(40) + 200, 1_000)] {
        assert!(burst[0].sob && burst[0].at == Some(at), "{sends:?}");
        assert!(burst[1..].iter().all(|s| !s.sob && s.at.is_none()), "{sends:?}");
        assert_eq!(burst.iter().map(|s| s.n).sum::<usize>(), len, "{sends:?}");
    }
    assert_eq!(device.unended_bursts(), 0);
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError" && r["code"] != "Underflow"), "{sends:?}");
    assert_eq!(bursts(&manifest).len(), 2, "{:?}", bursts(&manifest));
}

/// A `tx_send` the fake saw, as its calls record it: `n` counts an empty end-of-burst's
/// zero sample.
#[derive(Debug)]
struct Sent {
    n: usize,
    at: Option<i64>,
    sob: bool,
    eob: bool,
}

fn tx_sends(device: &FakeDevice) -> Vec<Sent> {
    calls(device, "tx_send").iter().map(|c| {
        let field = |key: &str| c.split_whitespace().find_map(|f| f.strip_prefix(key));
        Sent {
            n: field("n=").unwrap().parse().unwrap(),
            at: field("at=").and_then(|t| t.parse().ok()),
            sob: field("sob=") == Some("true"),
            eob: field("eob=") == Some("true"),
        }
    }).collect()
}

#[test]
fn ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time() {
    // Review O, O-B2 (R-1): with a 168 ms block at 390 625 S/s, a `cold` change booked at
    // any phase of the receive call in progress still switches before e₂; and with a
    // 100-sample block, where the call in progress waits for a packet (Review P, TG-3; the
    // packet term itself is a margin here: design-notes §14).
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let cases = [(65_536, 1_996, 28), (100, 1_996, 1)];
    for (block_len, packet, step_ms, phase) in cases.into_iter().flat_map(|(b, k, s)| (0..6).map(move |p| (b, k, s, p))) {
        let port = attached(ezsdr_kernel::stream::BackPressure::DropOldest);
        let config = FakeConfig { rx_packet: Some(packet), ..bench_link(2) };
        let mut direct = Direct::build(config, &[("radio.rx.s0.sample_rate_hz", Scalar::try_from(390_625.0).unwrap())], vec![port], json!({ "block_len": block_len }));
        direct.settle(Wall::from_millis(200 + phase * step_ms));
        direct.update("radio.rx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, None);
        direct.settle(Wall::from_millis(600));
        let late = direct.of("radio.LATE_COMMAND");
        let instance = direct.finish();
        let timing = &section_of(&instance, "timing");
        let switch = timing.as_array().unwrap().iter().find(|r| r["what"] == "rx_segment").cloned().unwrap_or_else(|| panic!("no switch: {timing}"));
        // UR-25's ready instant (spec 22, VH-4): the new origin is at least the cut, at the
        // booking, plus the longer of a block and a packet at the old rate, 3 ms and the
        // start lead (Review Q, TG-Q3).
        let booked = timing.as_array().unwrap().iter().find(|r| r["what"] == "cold_change").cloned().unwrap();
        let floor = ms(50) + block_len.max(packet) as i64 * 512 + ms(3);
        let origin = switch["origin"].as_i64().unwrap();
        assert!(origin - booked["booked_at"].as_i64().unwrap() >= floor, "block {block_len} packet {packet}: {booked} {switch}");
        assert!(switch["at"].as_i64().unwrap() < origin, "block {block_len} packet {packet} phase {phase}: switched after e₂: {switch}");
        assert!(late.is_empty(), "block {block_len} packet {packet} phase {phase}: {late:?}");
    }
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
    let b = ezsdr_kernel::manifest::ingest_input(
        Ident::parse("waveform2").unwrap(),
        Namespace::parse("ezsdr.input").unwrap(),
        format!("mem:{}", ContentHash::of_bytes(&two)),
        &two,
    );
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
    let sends = calls(&device, "tx_send");
    assert_eq!(sends.iter().filter(|c| c.contains("sob=true")).count(), 1, "{sends:?}");
    assert_eq!(sends.iter().filter(|c| c.ends_with("eob=true")).count(), 1, "{sends:?}");
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError" && r["code"] != "Underflow"), "{sends:?}");
}

#[test]
fn ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time() {
    // A one-sample burst's only buffer carries its start and time spec, so it is not held
    // back: it is played at its time, not untimed at the deadline (Review P, NB-3, TG-2).
    // Booked in the Spec at 40 ms, before the Run starts (rule (1)).
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let (bytes, waveform) = waveform_of(&[(0.5, -0.25)]);
    let spec = with_burst(with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &waveform, false, "drop_and_flag", 40_000);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(waveform.hash.clone(), bytes)]));
    let t0 = run.start_instant().expect("a Spec Run starts");
    let a_root = t0.ticks_in(t0.domain()).unwrap() + ms(40);
    let _ = run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ms(60)));
    let manifest = run.finish();
    assert_eq!(device.transmitted(a_root, 1), vec![Some([0.5, -0.25])], "{:?}", calls(&device, "tx_send"));
    assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
}

#[test]
fn ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took() {
    // Review P, NB-1: a send that comes up short abandons the burst. The device burst is
    // closed with an empty end-of-burst (one zero sample) straight after the samples the
    // device took; the burst's held last sample is not sent there. The final buffer cut
    // short is the burst's first (1 000 samples) or its second (3 000; 2 000 a buffer).
    // A send that fails after taking half is ended the same way (Review Q, NB-Q1).
    let cases = [(FakeFault::ShortSend(0), 1_000usize), (FakeFault::ShortSend(1), 3_000), (FakeFault::FailSend(0), 1_000)];
    for (fault, len) in cases {
        let nth = format!("{fault:?}");
        // Booked in the Spec at 40 ms, before the Run starts (rule (1)).
        let dir = TempDir::new();
        let device = fake(FakeConfig { faults: vec![fault], ..FakeConfig::default() });
        let wave = tone(len);
        let (bytes, waveform) = waveform_of(&wave);
        let spec = with_burst(with_tx(receive_spec(1, 1e6, 1e9, None), 1e6), &waveform, false, "drop_and_flag", 40_000);
        let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::from([(waveform.hash.clone(), bytes)]));
        let t0 = run.start_instant().expect("a Spec Run starts");
        let a_root = t0.ticks_in(t0.domain()).unwrap() + ms(40);
        let _ = run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + ms(60)));
        let manifest = run.finish();
        let sends = calls(&device, "tx_send");
        assert!(rejections(&manifest).iter().any(|r| r.contains("UR-22: a send did not complete") || r.contains("fake: the send failed")), "{nth}: {:?} {sends:?}", rejections(&manifest));
        let sent = device.transmitted(a_root, len + 1);
        let took = sent.iter().zip(&wave).take_while(|(s, w)| **s == Some([w.0, w.1])).count();
        assert!(took < len - 1, "nth {nth}: {took} of {len} {sends:?}");
        assert_eq!(sent[took], Some([0.0, 0.0]), "nth {nth}: after {took} {sends:?}");
        assert!(sent[took + 1..].iter().all(Option::is_none), "nth {nth}: {sends:?}");
        assert_eq!(sends.iter().filter(|c| c.ends_with("eob=true")).count(), 1, "nth {nth}: {sends:?}");
    }
}




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
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.front_end(dir, chan) }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(c, t) }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(p) }
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
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> { self.inner.apply(d, c, s, at) }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(d, m) }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(d, c) }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(s, c) }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(s, at) }
    fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
    fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(s, at) }
    fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv { self.inner.rx_recv(s, n, t) }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(s, c) }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.inner.tx_send(s, b, at, sob, eob, t) }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> { self.inner.tx_async(s, t) }
    fn rx_close(&self, s: usize) { self.inner.rx_close(s) }
    fn tx_close(&self, s: usize) { self.inner.tx_close(s) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
}

fn watched() -> (Arc<Watched>, Arc<Mutex<Option<String>>>) {
    let on = Arc::new(Mutex::new(None));
    let device = Watched {
        inner: FakeDevice::new(FakeConfig::default()),
        slow: Default::default(),
        after: Default::default(),
        reads_after: Default::default(),
        dropped_on: on.clone(),
    };
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
    /// Hold the `skip`th `tx_send` carrying samples busy (0-based), then send.
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
    /// Every `apply`, in order: `(timed, the device clock at entry, at return)` (§4.7's test).
    applies: Mutex<Vec<(bool, Wall, Wall)>>,
    /// The last `rx_recv` returned samples, so the next one is the stall's own position.
    after_block: std::sync::atomic::AtomicBool,
}

impl Descheduled {
    fn new(inner: FakeDevice) -> Descheduled {
        Descheduled { inner, plan: Mutex::new(None), entered: Mutex::new(None), applies: Mutex::new(Vec::new()), after_block: Default::default() }
    }

    /// Arms the planned hold; the receiver fires when the hold begins.
    fn arm(&self, plan: Deschedule) -> std::sync::mpsc::Receiver<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.entered.lock().unwrap() = Some(tx);
        *self.plan.lock().unwrap() = Some(plan);
        rx
    }

    fn applies(&self) -> Vec<(bool, Wall, Wall)> {
        self.applies.lock().unwrap().clone()
    }

    /// The inner fake's transmit sends, as `common::calls` reads them (UR-33).
    fn sends(&self) -> Vec<String> {
        self.inner.calls().into_iter().filter(|c| c.starts_with("tx_send")).collect()
    }

    /// Runs the planned busy hold when `step` is the armed one, signalling first. `condition`
    /// carries what a plan keyed on more than the call itself needs: the shutdown's non-zero
    /// report timeout, or that the last `rx_recv` returned a block.
    fn hold(&self, step: Step, condition: bool) {
        let busy = {
            let mut plan = self.plan.lock().unwrap();
            let armed = match plan.as_mut() {
                Some(p) if p.step() == step => match p {
                    Deschedule::ShutdownAck(_) | Deschedule::RxRecvAfterBlock(_) => condition,
                    Deschedule::TxSend(n, _) => {
                        if *n == 0 {
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
                return;
            }
            plan.take().expect("armed above").busy()
        };
        if let Some(tx) = self.entered.lock().unwrap().take() {
            let _ = tx.send(());
        }
        std::thread::sleep(busy);
    }
}

impl Device for Descheduled {
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.front_end(dir, chan) }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(c, t) }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(p) }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        let begun = self.inner.host().now();
        let applied = self.inner.apply(d, c, s, at);
        self.applies.lock().unwrap().push((at.is_some(), begun, self.inner.host().now()));
        applied
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(d, m) }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(d, c) }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(s, c) }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(s, at) }
    fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
    fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(s, at) }
    fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv {
        use std::sync::atomic::Ordering::SeqCst;
        let after_block = self.after_block.swap(false, SeqCst);
        self.hold(Step::RxRecv, after_block);
        let got = self.inner.rx_recv(s, n, t);
        if matches!(got, ezsdr_radio_uhd::RxRecv::Samples { .. }) {
            self.after_block.store(true, SeqCst);
        }
        got
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(s, c) }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> {
        self.hold(Step::TxSend, false);
        self.inner.tx_send(s, b, at, sob, eob, t)
    }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.hold(Step::TxAsync, !t.is_zero());
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) { self.inner.rx_close(s) }
    fn tx_close(&self, s: usize) { self.inner.tx_close(s) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
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
        entered.recv_timeout(Wall::from_secs(5)).unwrap_or_else(|_| panic!("{mode:?}: uhd-tx was not descheduled inside tx_send"));
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
    let manifest = bounded("the transmit shutdown", Wall::from_secs(30), move || {
        wait(&mut run, ms(15));
        // Armed only now: the run loop's own report reads use a zero timeout and must not consume it.
        let _entered = runner.arm(Deschedule::ShutdownAck(Wall::from_millis(200)));
        run.finish()
    });
    let codes: Vec<String> = section(&manifest, "async").as_array().unwrap().iter().map(|r| r["code"].as_str().unwrap().to_owned()).collect();
    assert!(codes.contains(&"BurstAck".to_owned()), "the last BURST_ACK was lost: {codes:?} {:?}", device.sends());
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
    let begun = Instant::now();
    let manifest = bounded("the transmit shutdown", Wall::from_secs(30), move || {
        wait(&mut run, ms(15));
        run.finish()
    });
    assert!(begun.elapsed() < Wall::from_secs(10), "the shutdown did not finish in 10 s: {:?}", begun.elapsed());
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
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.call("front_end"); self.inner.front_end(dir, chan) }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.call("set_sources"); self.inner.set_sources(c, t) }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.call("set_time_zero"); self.inner.set_time_zero(p) }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.call("time_now"); self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.call("ref_locked"); self.inner.ref_locked() }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> { self.call("apply"); self.inner.apply(d, c, s, at) }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(d, m) }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(d, c) }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.call("rx_open"); self.inner.rx_open(s, c) }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.call("rx_start"); self.inner.rx_start(s, at) }
    fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
    fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.call("rx_stop"); self.inner.rx_stop(s, at) }
    fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv { self.call("rx_recv"); self.inner.rx_recv(s, n, t) }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.call("tx_open"); self.inner.tx_open(s, c) }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.call("tx_send"); self.inner.tx_send(s, b, at, sob, eob, t) }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> {
        self.call("tx_async");
        if !t.is_zero() && !self.lost.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(ezsdr_radio_uhd::DeviceError { lost: true, message: "fake: the link went down during the shutdown".to_owned() });
        }
        self.inner.tx_async(s, t)
    }
    fn rx_close(&self, s: usize) { self.call("rx_close"); self.inner.rx_close(s) }
    fn tx_close(&self, s: usize) { self.call("tx_close"); self.inner.tx_close(s) }
    fn close_streams(&self) { self.call("close_streams"); self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
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
        after: Mutex::new(Vec::new()),
    });
    let (run, _dir) = tx_session_on(device.clone());
    let manifest = bounded("the transmit shutdown", Wall::from_secs(30), move || run.finish());
    assert!(device.lost.load(std::sync::atomic::Ordering::SeqCst), "the shutdown never read a report with a timeout");
    let rejections = rejections(&manifest);
    assert!(!rejections.iter().any(|r| r.contains("did not join")), "uhd-tx did not end after the loss: {rejections:?}");
    assert_eq!(*device.after.lock().unwrap(), Vec::<&str>::new(), "device calls after the loss");
}

/// R01, §4.7: a cold change whose configuration holds the projection's clock lock across its
/// device call (a 300 ms `apply_delay`) is overlapped with a `hardware_timed` update to the
/// same projection, released by uhd-control, which takes the same lock. On the manual clock
/// the waiter is idle, so the clock advances and both finish; a plain mutex would leave the
/// waiter busy, freeze the clock behind it, and stall the Run.
#[test]
fn ur_25_a_configure_overlapped_with_a_timed_update_to_the_same_projection_finishes() {
    let dir = TempDir::new();
    let device = Arc::new(Descheduled::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, apply_delay: Wall::from_millis(300), ..FakeConfig::default() })));
    let mut spec = receive_spec(1, 1e6, bench_hz(&*device), None);
    let target = serde_json::to_value(resource("radio")).unwrap();
    // `offset_ticks` counts the stream clock's samples at 1 MS/s (KC-16), so these are 50 ms
    // and 150 ms after T0: the cold change's configuration runs for 300 ms while uhd-control
    // releases the timed update (UR-24).
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 50_000 }, "action": { "kind": "update_parameter", "target": target, "key": "radio.rx.s0.sample_rate_hz", "value": 2e6 } },
        { "at": { "clock": "radio", "offset_ticks": 150_000 }, "action": { "kind": "update_parameter", "target": target, "key": "radio.rx.c0.gain_db", "value": 3.0 } }
    ]);
    let doc = profile(&dir, json!({}), json!({}), false);
    let runner = device.clone();
    let begun = Instant::now();
    let manifest = bounded("the overlapped configuration", Wall::from_secs(30), move || {
        let mut run = spec_run(&spec, &doc, runner, BTreeMap::new());
        // Past the Run's 2 s start-up and its scheduled instants (plan 30 §7's two bars).
        let _ = run.advance_to(after(&run, ms(2_000) + ms(800)));
        run.finish()
    });
    assert!(begun.elapsed() < Wall::from_secs(10), "the overlapped configuration did not finish in 10 s: {:?}", begun.elapsed());
    let rejections = rejections(&manifest);
    assert!(!rejections.iter().any(|r| r.contains("did not join") || r.contains("timed out")), "{rejections:?}");
    // The timed update waited for the cold configuration's lock: its device call began only
    // after that configuration's call returned (a plain mutex would leave no timed call).
    let applies = device.applies();
    let cold = applies.iter().rfind(|(timed, _, _)| !timed).unwrap_or_else(|| panic!("no untimed apply: {applies:?}"));
    let timed = applies.iter().find(|(timed, begun, _)| *timed && *begun >= cold.2).unwrap_or_else(|| panic!("the timed update did not wait for the configuration: {applies:?}"));
    assert!(timed.1 <= cold.2 + Wall::from_millis(200), "the hand-off took {:?}: {applies:?}", timed.1 - cold.2);
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
        let mut run = start_spec_run(&spec, &doc, assembly(&doc, runner, BTreeMap::new(), |r| r.with_rx_stall(Wall::from_millis(500), Wall::from_millis(300)))).unwrap();
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
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new());
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
    let calls = direct.device.calls();
    assert_eq!(calls.iter().filter(|c| c.starts_with("rx_stop")).count(), 1, "{calls:?}");
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
        Gated { inner: Arc::new(FakeDevice::new(config)), held: Mutex::new(false), open: std::sync::Condvar::new(), entered: Mutex::new(Some(entered)), arrivals: Mutex::new(arrivals), only: None, ends: Mutex::new(None) }
    }

    /// The device the test observes the calls on.
    fn inner(&self) -> Arc<FakeDevice> { self.inner.clone() }

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
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.front_end(dir, chan) }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(d, m) }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(d, c) }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(c, t) }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(p) }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> { self.inner.apply(d, c, s, at) }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(s, c) }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(s, at) }
    fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
    fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(s, at) }
    fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv {
        self.hold_call(s, "rx_recv");
        self.inner.rx_recv(s, n, t)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(s, c) }
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
    fn rx_close(&self, s: usize) { self.inner.rx_close(s) }
    fn tx_close(&self, s: usize) { self.inner.tx_close(s) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
}

#[test]
fn ur_26_the_sending_threads_join_before_the_receiving_ones() {
    // RM-16, UR-16 (design 33 D15): `Provider::stop` joins the transmit streams' threads
    // before the receive streams' — a `stop` with both held inside a device call records the
    // transmit thread first. The real clock keeps uhd-control running while the two held
    // threads would otherwise freeze the manual one (UR-2, plan 30 §4.4).
    let device = Arc::new(Gated::new(FakeConfig::default()));
    let (mut run, _dir) = tx_session_on(device.clone());
    let entry = send(&mut run, "start_repeat", None, &tone(500));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(40));
    device.hold();
    device.entered(&["rx_recv", "tx_"]);
    let runner = device.clone();
    let manifest = bounded("the transmit join order", Wall::from_secs(30), move || run.finish());
    device.release();
    let threads: Vec<String> = section(&manifest, "rejected").as_array().unwrap().iter()
        .filter(|r| r["reason"].as_str().is_some_and(|reason| reason.contains("did not join")))
        .map(|r| r["thread"].as_str().unwrap().to_owned()).collect();
    assert_eq!(threads, ["uhd-tx-s0", "uhd-rx-s0"], "{:?}", section(&manifest, "rejected"));
    assert!(!runner.inner.calls().iter().any(|c| c == "close_streams"), "a detached thread's streamer was closed (UR-16)");
}

#[test]
fn ur_25_tx_channels_to_zero_ends_the_clock_and_a_re_enable_starts_a_new_one() {
    // RM-25, UR-26 (design 33 D15): a transmit `cold` change to 0 channels cuts the running
    // segment and ends its clock without opening an empty streamer; a later enable starts a
    // new segment on a new clock, a start lead later, and its bursts go out on it.
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    let entry = send(&mut run, "start_repeat", None, &tone(500));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(40));
    assert!(admitted(&run.submit(set("radio.tx.c0.stream", Value::from(-1)), None).unwrap()));
    wait(&mut run, ms(20));
    // A burst on the ended clock is refused at admission (SC-23, UR-26).
    let refused = send(&mut run, "start_repeat", None, &tone(100));
    assert!(!admitted(&refused), "{refused:?}");
    assert!(matches!(&refused.outcome, ezsdr_kernel::session::Outcome::Rejected { violations }
        if violations.iter().any(|v| v.reason.starts_with("SC-23: ") && v.reason.ends_with("has no running transmit SampleClock"))), "{refused:?}");
    assert!(admitted(&run.submit(set("radio.tx.c0.stream", Value::from(0)), None).unwrap()));
    wait(&mut run, ms(60));
    let entry = send(&mut run, "start_repeat", None, &tone(100));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(40));
    let calls = device.calls();
    let manifest = run.finish();
    let clocks: Vec<(i64, Option<i64>, ezsdr_kernel::id::ClockDomainId)> = manifest.clocks.sample_clocks.iter()
        .filter(|r| r.stream == ResourceId::parse("usrp/tx/s0").unwrap())
        .map(|r| (r.origin.ticks_in(r.root).unwrap(), r.ended_at.map(|t| t.ticks_in(r.root).unwrap()), r.domain)).collect();
    assert_eq!(clocks.len(), 2, "the disable ends one clock and the enable registers one: {clocks:?} {calls:?}");
    let end = clocks[0].1.unwrap_or_else(|| panic!("the first transmit clock was not ended: {clocks:?}"));
    let timing = section(&manifest, "timing").clone();
    let plan = plan_row(&timing);
    let segments: Vec<(i64, Option<i64>)> = plan["tx"].as_array().unwrap().iter().map(|s| (s["origin"].as_i64().unwrap(), s["end"].as_i64())).collect();
    assert_eq!(segments.first().and_then(|(_, end)| *end), Some(end), "the clock's end is not its segment's: {segments:?}");
    let envelope = section(&manifest, "envelope");
    let lead = envelope["timing"]["tx_start_lead_ns"].as_i64().unwrap() * MCR / 1_000_000_000;
    assert!(clocks[1].0 >= end + lead, "the new segment is not a start lead past the cut: {clocks:?}");
    assert!(bursts(&manifest).iter().any(|b| b.target.domain() == clocks[1].2), "{:?}", bursts(&manifest));
    assert!(!calls.iter().any(|c| c.starts_with("tx_open 0 []")), "an empty streamer was opened: {calls:?}");
    assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
    assert!(events_of(&manifest, "radio.COMMAND_REJECTED").is_empty(), "{:?}", events_of(&manifest, "radio.COMMAND_REJECTED"));
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
    assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
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
}

impl Recording {
    fn new(config: FakeConfig) -> Recording {
        Recording { inner: FakeDevice::new(config), log: Mutex::new(Vec::new()) }
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
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize {
        self.record(|| format!("channels {}", dir.name()));
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.record(|| format!("front_end {} {chan}", dir.name()));
        self.inner.front_end(dir, chan)
    }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(c, t) }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(p) }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> { self.inner.apply(d, c, s, at) }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        self.record(|| "serial".to_owned());
        self.inner.serial()
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.record(|| format!("set_subdev_spec {} {m}", d.name()));
        self.inner.set_subdev_spec(d, m)
    }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(d, c) }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(s, c) }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(s, at) }
    fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
    fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(s, at) }
    fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv { self.inner.rx_recv(s, n, t) }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(s, c) }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.inner.tx_send(s, b, at, sob, eob, t) }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> { self.inner.tx_async(s, t) }
    fn rx_close(&self, s: usize) { self.inner.rx_close(s) }
    fn tx_close(&self, s: usize) { self.inner.tx_close(s) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
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
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        let name = self.inner.front_end(dir, chan)?;
        let basic = ["BasicRX", "LFRX", "BasicTX", "LFTX"].iter().any(|base| name.starts_with(base));
        if basic {
            let base = name.split_once(" (").map_or(name.as_str(), |(base, _)| base);
            return Ok(format!("{base} (0)"));
        }
        Ok(name)
    }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(d, m) }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(d, c) }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(c, t) }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(p) }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> { self.inner.apply(d, c, s, at) }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(s, c) }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(s, at) }
    fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
    fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(s, at) }
    fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv { self.inner.rx_recv(s, n, t) }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(s, c) }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.inner.tx_send(s, b, at, sob, eob, t) }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> { self.inner.tx_async(s, t) }
    fn rx_close(&self, s: usize) { self.inner.rx_close(s) }
    fn tx_close(&self, s: usize) { self.inner.tx_close(s) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
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
    let device: Arc<dyn Device> = Arc::new(Mislabelled { inner: FakeDevice::new(FakeConfig { front_ends: vec!["BasicRX", "BasicRX"], serial: "ur05-fe-mislabelled".to_owned(), ..FakeConfig::default() }) });
    let error = UhdRadio::from_binding(&binding(selector, Some(x3xx())), device).err().unwrap();
    assert_eq!(error.kind, ModuleErrorKind::Rejected);
    assert!(error.message.contains("rx channel 0") && error.message.contains("BasicRX (0)" ) && error.message.contains("basic_rx"), "{}", error.message);
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
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.front_end(dir, chan) }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> {
        Err(ezsdr_radio_uhd::DeviceError::failed("fake: the serial read failed"))
    }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(d, m) }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(d, c) }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(c, t) }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(p) }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> { self.inner.apply(d, c, s, at) }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(s, c) }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(s, at) }
    fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
    fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(s, at) }
    fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv { self.inner.rx_recv(s, n, t) }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(s, c) }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.inner.tx_send(s, b, at, sob, eob, t) }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> { self.inner.tx_async(s, t) }
    fn rx_close(&self, s: usize) { self.inner.rx_close(s) }
    fn tx_close(&self, s: usize) { self.inner.tx_close(s) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
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
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: ezsdr_radio_uhd::Dir, chan: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.front_end(dir, chan) }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(d, m) }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(c, t) }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(p) }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> { self.inner.apply(d, c, s, at) }
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
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(s, c) }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(s, at) }
    fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
    fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(s, at) }
    fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv { self.inner.rx_recv(s, n, t) }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(s, c) }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.inner.tx_send(s, b, at, sob, eob, t) }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> { self.inner.tx_async(s, t) }
    fn rx_close(&self, s: usize) { self.inner.rx_close(s) }
    fn tx_close(&self, s: usize) { self.inner.tx_close(s) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
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
        let run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
        assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }), "tx={tx}: {:?}", run.state());
        let manifest = run.finish();
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
    let run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }), "{:?}", run.state());
    let manifest = run.finish();
    assert!(failure(&manifest).contains("UR-13"), "{}", failure(&manifest));
    let log = inner.calls();
    assert_eq!(calls(&inner, "rx_open 0 [0]").len(), 1, "the refusal is after the open: {log:?}");
    let closed = log.iter().position(|call| call == "rx_close 0").expect("the rate refusal closed the streamer");
    let all_closed = log.iter().position(|call| call == "close_streams").expect("run cleanup");
    assert!(closed < all_closed, "the failed rate check closes before cleanup: {log:?}");
}

#[test]
fn ur_34_a_rate_check_refusal_closes_the_streamer_before_a_later_move() {
    // A3: a successful from-none open whose first UR-13 rate comparison fails is closed in
    // that control turn, so Unknown matches the physical device before the channel moves.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let inner = fake(FakeConfig::default());
    let tested: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: Some(off_grid()), tx: None, failing: None });
    let mut constraints = rx_two();
    constraints[0] = ("radio.rx.c0.stream", Scalar::from(-1));
    constraints[1] = ("radio.rx.c1.stream", Scalar::from(-1));
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::armed_on(inner.clone(), tested, &constraints, vec![r0, r1], layout_selector(), BTreeMap::new());
    direct.start();
    direct.update("radio.rx.c0.stream", Value::from(0), Cold, Some(direct.now() + ms(40)));
    direct.until("the refused open is closed", |direct| calls(&direct.device, "rx_close 0").len() == 1);
    direct.update("radio.rx.c0.stream", Value::from(1), Cold, Some(direct.now() + ms(40)));
    direct.until("the moved channel's destination open is closed", |direct| calls(&direct.device, "rx_close 1").len() == 1);
    let log = direct.device.calls();
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert!(log.iter().position(|call| call == "rx_close 0").unwrap() < log.iter().position(|call| call == "rx_open 1 [0]").unwrap(), "{log:?}");
    assert!(!log.iter().any(|call| call.contains("Attempting to reconnect")), "{log:?}");
    assert!(rejected.iter().all(|event| !rejection_reason(std::slice::from_ref(event)).contains("reconnect")), "{rejected:?}");
    let _ = direct.finish();
}

#[test]
fn ur_34_a_move_destination_open_refusal_halts_both_lines_and_recovers() {
    // S6: refusal to open the destination must refuse the move on both lines; later cold
    // updates reopen the streams according to the membership the move left as record.
    move_open_refusal_recovery(1);
}

#[test]
fn ur_34_a_move_source_reopen_refusal_halts_both_lines_and_recovers() {
    // S6: the source's remaining set is reopened first; its refusal also stops the destination.
    move_open_refusal_recovery(0);
}

#[test]
fn ur_34_a_move_destination_configure_refusal_halts_both_lines_and_recovers() {
    // S6/A5: the first destination apply is refused after its open; both plans halt and a
    // later cold configuration resumes from the move's recorded membership.
    move_config_refusal_recovery(1e6, 1);
}

#[test]
fn ur_34_a_move_source_configure_refusal_halts_both_lines_and_recovers() {
    // S6/A5: the source's remaining channel configuration fails after its reopen.
    move_config_refusal_recovery(2e6, 2);
}

#[test]
fn ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer() {
    // UR-25 (S12): the streamers `arm` opened and checked are the ones the first segments run
    // on, and the next segment of the same channel set is configured on them too: each
    // direction's streamer is opened once, and the owner adopts the segment without opening.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::with_links(
        FakeConfig::default(),
        &[("radio.rx.c0.stream", Scalar::from(0)), ("radio.tx.c0.stream", Scalar::from(0))],
        vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)],
    );
    let now = direct.now();
    direct.update("radio.rx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(now + ms(50)));
    direct.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(now + ms(50)));
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_open 0 [0]").len(), 1, "{log:?}");
    assert_eq!(calls(&direct.device, "tx_open 0 [0]").len(), 1, "{log:?}");
    assert!(log.iter().any(|c| c.starts_with("rx_start")), "the first segment starts: {log:?}");
    assert!(log.iter().any(|c| c.starts_with("apply rx 0 ") && c.contains("rate=2000000") && c.ends_with("at=-")),
        "the next segment configures without a reopen: {log:?}");
    assert!(log.iter().any(|c| c.starts_with("apply tx 0 ") && c.contains("rate=2000000") && c.ends_with("at=-")),
        "the next segment configures without a reopen: {log:?}");
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

// ------------------------------------------------ 1b-4c: stream 0 of the layout's keys

/// The selector of a two-channel-UBX layout, both directions (RM-28).
fn layout_selector() -> Json {
    json!({  "rx": ["0:A:ubx:0", "0:B:ubx:0"], "tx": ["0:A:ubx:0", "0:B:ubx:0"] })
}

/// One UBX and one BasicRX board: three receive channels, two streams, and two RX links.
fn mixed_selector() -> Json {
    json!({  "rx": ["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"], "tx": ["0:A:ubx:0"] })
}

fn fake_mixed() -> FakeConfig {
    FakeConfig { front_ends: vec!["UBX", "BasicRX"], ..FakeConfig::default() }
}

#[test]
fn ur_12_a_layout_s_stream_zero_declares_its_clock_and_opens_its_channels() {
    // UR-12, RM-29: `prepare` and `arm` declare stream 0's clock at `usrp/rx/s0`, read its
    // channel set from the membership, and open — and configure — exactly that set, each
    // channel with its own numbered settings; the record waits for the first block.
    let direct = Direct::armed(
        FakeConfig::default(),
        &[
            ("radio.rx.c0.stream", Scalar::from(-1)),
            ("radio.rx.c1.stream", Scalar::from(0)),
            ("radio.rx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap()),
        ],
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
    assert!(log.iter().any(|call| call == "apply rx 1 rate=2000000 freq=1000000000 gain=0 antenna=RX2 at=-"), "{log:?}");
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
        [
            "apply rx 0 rate=2000000 freq=1000000000 gain=5 antenna=RX2 at=-",
            "apply rx 1 rate=2000000 freq=2400000000 gain=7.5 antenna=TX/RX at=-",
        ],
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
    assert!(calls(&direct.device, "apply tx").contains(&"apply tx 1 rate=2000000 freq=2400000000 gain=7.5 antenna=TX/RX at=-".to_owned()), "{:?}", direct.device.calls());
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
    let device = fake(FakeConfig {
        faults: vec![FakeFault::WrongRate { claimed: 2e6, applied: 1.9e6, nth: 0 }],
        serial: "ur12-layout-wrong-rate".to_owned(),
        ..FakeConfig::default()
    });
    let mut radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).unwrap();
    let constraints = [
        ("radio.rx.c0.stream", Scalar::from(-1)),
        ("radio.rx.c1.stream", Scalar::from(0)),
        ("radio.rx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap()),
    ];
    let error = direct_prepare(&mut radio, device.clone(), ExecutionClass::HardwareInLoop, Vec::new(), &constraints).unwrap_err();
    assert!(error.message.contains("UR-12: the device applied") && error.message.contains("1900000") && error.message.contains("2000000"), "{}", error.message);
    assert!(calls(&device, "rx_open").is_empty(), "{:?}", device.calls());
}

#[test]
fn ur_12_a_layout_an_antenna_read_back_that_differs_is_refused() {
    // UR-12, D16: the antenna is read back after an untimed change and compared; a front
    // end whose getter disagrees halts the configuration (the record keeps both values).
    let device = fake(FakeConfig {
        faults: vec![FakeFault::WrongAntenna { claimed: "TX/RX", applied: "RX2" }],
        serial: "ur12-layout-antenna".to_owned(),
        ..FakeConfig::default()
    });
    let mut radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).unwrap();
    let constraints = [
        ("radio.rx.c0.stream", Scalar::from(-1)),
        ("radio.rx.c1.stream", Scalar::from(0)),
        ("radio.rx.c1.antenna", Scalar::from("TX/RX")),
    ];
    let error = direct_prepare(&mut radio, device.clone(), ExecutionClass::HardwareInLoop, Vec::new(), &constraints).unwrap_err();
    assert!(error.message.contains("UR-12") && error.message.contains("TX/RX") && error.message.contains("RX2"), "{}", error.message);
    assert!(error.message.contains("radio.rx.c1"), "{}", error.message);
    assert!(calls(&device, "rx_open").is_empty(), "{:?}", device.calls());
}

// `ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
#[test]
fn ur_24_a_timed_update_for_a_channel_out_of_its_stream_waits_for_its_segment() {
    // UR-24, item 7: a timed update for a channel of no running stream is recorded, not
    // handed over; the segment that later takes the channel applies it untimed.
    use ezsdr_kernel::module_api::UpdateClass::{Cold, HardwareTimed};
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)], layout_selector());
    let at = direct.now() + ms(40);
    direct.update("radio.rx.c1.frequency_hz", Value::num(2.1e9).unwrap(), HardwareTimed, Some(at));
    direct.settle(Wall::from_millis(80));
    let log = direct.device.calls();
    assert_eq!(log.iter().filter(|call| call.starts_with("apply rx 1 ")).count(), 0, "a channel in no stream is not handed over: {log:?}");
    assert_eq!(direct.of("radio.COMMAND_REJECTED").len(), 0);
    assert_eq!(calls(&direct.device, "rx_open").len(), 1, "the recorded update cuts nothing");
    // The channel joins: the new segment's configuration carries the recorded value,
    // untimed, and nothing hands it over as timed.
    direct.update("radio.rx.c1.stream", Value::from(0), Cold, None);
    direct.settle(Wall::from_millis(200));
    let log = direct.device.calls();
    assert!(log.iter().any(|call| call == "rx_open 0 [0, 1]"), "{log:?}");
    let untimed = log.iter().filter(|call| call.starts_with("apply rx 1 ") && call.contains("freq=2100000000") && call.ends_with("at=-")).count();
    assert_eq!(untimed, 1, "{log:?}");
    assert_eq!(log.iter().filter(|call| call.starts_with("apply rx 1 ") && call.contains("freq=2100000000") && !call.ends_with("at=-")).count(), 0, "{log:?}");
    assert_eq!(log.iter().filter(|call| call.starts_with("apply rx 0 ") && call.contains("freq=2100000000")).count(), 0, "the value leaked to c0: {log:?}");
    assert_eq!(direct.of("radio.COMMAND_REJECTED").len(), 0);
    let instance = direct.finish();
    let applied = &section_of(&instance, "applied");
    assert_eq!(applied.as_array().unwrap().iter().filter(|row| row["key"] == "radio.rx.c1.frequency_hz" && row["note"] == "no channel").count(), 1, "{applied}");
    assert!(applied.as_array().unwrap().iter().any(|row| row["key"] == "radio.rx.c1.frequency_hz" && row["claimed"] == 2.1e9 && row["read_back"] == 2.1e9), "{applied}");
}

// `ur_24_a_channel_keyed_command_counts_one_queue_slot` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
#[test]
fn ur_25_a_channel_s_antenna_change_cuts_its_stream() {
    // UR-25, D16: a channel's antenna is a `cold` change: the stream is cut, the new
    // segment applies antenna untimed and its read-back is recorded; the ready instant is
    // the cut plus the receive call, the allowance and the lead.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)], layout_selector());
    direct.update("radio.rx.c0.antenna", Value::from("TX/RX"), Cold, None);
    direct.settle(Wall::from_millis(150));
    let log = direct.device.calls();
    let stop = log.iter().position(|call| call == "rx_stop 0 now").expect("the cut stops the streamer");
    let apply = log.iter().position(|call| call == "apply rx 0 rate=1000000 freq=1000000000 gain=0 antenna=TX/RX at=-").unwrap_or_else(|| panic!("the new segment's configuration: {log:?}"));
    assert!(stop < apply, "{log:?}");
    assert_eq!(log.iter().filter(|call| call.starts_with("apply rx 1 ")).count(), 0, "{log:?}");
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.len(), 2, "{records:?}");
    assert!(records[0].ended_at.is_some(), "{records:?}");
    let (end, origin) = (records[0].ended_at.unwrap().ticks_in(records[0].root).unwrap(), records[1].origin.ticks_in(records[1].root).unwrap());
    assert!(origin >= end + ms(55), "the receive call, the allowance and the lead (RM-25): {origin} vs {end}");
    assert!(origin - (end + ms(55)) < 200, "the origin is the next lattice instant (RM-25): {origin} vs {end}");
    assert_eq!(records[1].stream.path(), "usrp/rx/s0");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty());
    let instance = direct.finish();
    let applied = &section_of(&instance, "applied");
    assert!(applied.as_array().unwrap().iter().any(|row|
        row["key"] == "radio.rx.c0.antenna" && row["channel"] == 0 && row["claimed"] == "TX/RX" && row["read_back"] == "TX/RX"
    ), "{applied}");
}

#[test]
fn ur_25_an_antenna_change_of_a_channel_in_no_stream_only_records() {
    // UR-25, item 7's rule on the cold side: a channel of no running stream takes its
    // value; no cut, no device call, and the record reads no antenna back.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)], layout_selector());
    direct.update("radio.rx.c1.antenna", Value::from("TX/RX"), Cold, None);
    direct.settle(Wall::from_millis(120));
    let log = direct.device.calls();
    assert_eq!(log.iter().filter(|call| call.starts_with("apply rx 1 ")).count(), 0, "{log:?}");
    assert_eq!(direct.clocks.sample_clock_records().len(), 1, "the running stream is not cut");
    assert!(direct.of("radio.LATE_COMMAND").is_empty());
    let instance = direct.finish();
    let applied = &section_of(&instance, "applied");
    let row = applied.as_array().unwrap().iter().find(|row| row["key"] == "radio.rx.c1.antenna").unwrap_or_else(|| panic!("{applied}"));
    assert_eq!(row["claimed"], "TX/RX");
    assert!(row["read_back"].is_null(), "the device was not asked: {row}");
}

#[test]
fn ur_25_a_refused_antenna_read_back_halts_the_stream() {
    // UR-25, UR-12, D16: a runtime antenna whose read-back disagrees is refused, the
    // segment does not start, and the refusal is recorded with both names.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::build(
        FakeConfig { faults: vec![FakeFault::WrongAntenna { claimed: "TX/RX", applied: "RX2" }], ..FakeConfig::default() },
        &[],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    direct.update("radio.rx.c0.antenna", Value::from("TX/RX"), Cold, None);
    direct.settle(Wall::from_millis(150));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?} {:?}", direct.device.calls());
    let reason = rejection_reason(&rejected);
    assert!(reason.starts_with("UR-12:") && reason.contains("TX/RX") && reason.contains("RX2") && reason.contains("radio.rx.c0"), "{reason}");
    assert_eq!(calls(&direct.device, "rx_start").len(), 1, "the refused segment never starts: {:?}", direct.device.calls());
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert!(records[0].ended_at.is_some(), "the cut ended the old clock");
    let instance = direct.finish();
    let applied = &section_of(&instance, "applied");
    assert!(applied.as_array().unwrap().iter().any(|row| row["key"] == "radio.rx.c0.antenna" && row["claimed"] == "TX/RX" && row["read_back"] == "RX2"), "{applied}");
    let rejected = &section_of(&instance, "rejected");
    assert!(rejected.as_array().unwrap().iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.starts_with("UR-12:"))), "{rejected}");
}

// ------------------------------------------------ 1b-4c: the cut, the floor and the targets

#[test]
fn ur_25_a_layout_rate_change_cuts_its_stream_and_configures_every_channel() {
    // UR-25, item 8: a stream's rate change cuts it and the new segment configures every
    // channel of its set — each channel's own values, the timed updates due by the origin
    // included — and the new clock starts on its lattice; the other direction is untouched.
    // Both book before the Run starts (rule (1)): the 300 ms cut and the 320 ms retune are
    // the Spec's own offsets, so no delay of the test thread can move them.
    let dir = TempDir::new();
    let device = fake(FakeConfig { front_ends: vec!["UBX", "BasicRX"], serial: "ur25-layout-rate".to_owned(), ..FakeConfig::default() });
    let selector = json!({  "rx": ["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"], "tx": ["0:A:ubx:0"] });
    let mut spec = with_tx(receive_spec(1, 1e6, 1e9, None), 1e6);
    spec["requirements"]["vocabularies"] = json!([{ "id": "radio", "major": 2 }, { "id": "sink", "major": 1 }]);
    let requires = &mut spec["resources"]["radio"]["requires"];
    requires["radio.rx.c1.stream"] = json!({ "kind": "eq", "value": 0 });
    requires["radio.rx.c0.frequency_hz"] = json!({ "kind": "eq", "value": 2.4e9 });
    requires["radio.rx.c0.gain_db"] = json!({ "kind": "eq", "value": 20 });
    requires["radio.rx.c1.frequency_hz"] = json!({ "kind": "eq", "value": 25e6 });
    // The timed update first, 20 ms behind the switch's instant: the switch's configuration
    // carries the values due by its origin.
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 320_000 }, "action": { "kind": "update_parameter",
            "target": serde_json::to_value(resource("radio")).unwrap(), "key": "radio.rx.c1.frequency_hz", "value": 30e6 } },
        { "at": { "clock": "radio", "offset_ticks": 300_000 }, "action": { "kind": "update_parameter",
            "target": serde_json::to_value(resource("radio")).unwrap(), "key": "radio.rx.s0.sample_rate_hz", "value": 2e6 } }
    ]);
    let manifest = finish_spec(spec_run(&spec, &profile(&dir, selector, json!({}), false), device.clone(), BTreeMap::new()), ms(400));
    let log = device.calls();
    let stop = log.iter().position(|call| call == "rx_stop 0 now").expect("the cut: {log:?}");
    let start = log.iter().enumerate().position(|(index, call)| index > stop && call.starts_with("rx_start 0")).expect("the new segment starts: {log:?}");
    assert!(stop < start, "{log:?}");
    let between: Vec<&String> = log[stop + 1..start].iter().filter(|call| call.starts_with("apply rx") && call.ends_with("at=-")).collect();
    let wanted = ["apply rx 0 rate=2000000 freq=2400000000 gain=20 antenna=RX2 at=-", "apply rx 1 rate=2000000 freq=30000000 gain=0 antenna=AB at=-"];
    assert_eq!(between.len(), 2, "{log:?}");
    for call in wanted {
        assert!(between.iter().any(|got| got.as_str() == call), "{call} missing in {between:?}");
    }
    assert!(!log.iter().any(|call| call.starts_with("apply rx 2 ") && call.contains("rate=2000000")), "c2 is in no stream: {log:?}");
    assert_eq!(calls(&device, "rx_open 0 [0, 1]").len(), 1, "the set changed once, so one reopen: {log:?}");
    let records = &manifest.clocks.sample_clocks;
    let rx: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/rx/s0").collect();
    assert_eq!(rx.len(), 2, "{records:?}");
    assert_eq!(rx[0].root_ticks_per_tick, ezsdr_kernel::time::Rational::new(200, 1).unwrap());
    assert!(rx[0].ended_at.is_some());
    assert_eq!(rx[1].root_ticks_per_tick, ezsdr_kernel::time::Rational::new(100, 1).unwrap());
    let (end, origin) = (rx[0].ended_at.unwrap().ticks_in(rx[0].root).unwrap(), rx[1].origin.ticks_in(rx[1].root).unwrap());
    assert!(origin >= end + ms(55), "{origin} vs {end}");
    // The transmit side is untouched.
    let tx: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/tx/s0").collect();
    assert_eq!(tx.len(), 1, "{records:?}");
    assert!(tx[0].ended_at.is_none(), "the transmit clock is not cut");
    assert!(!log.iter().any(|call| call.starts_with("apply tx") && call.contains("rate=2000000")), "the transmit side is configured again: {log:?}");
    let timing = section(&manifest, "timing");
    let row = plan_row(timing);
    let rx_segments = row["rx"].as_array().unwrap();
    assert_eq!(rx_segments.len(), 2, "{row}");
    assert_eq!(rx_segments[1]["stream"], 0, "{row}");
    assert!(rx_segments[1]["sequence"].is_u64(), "the second segment names the change that began it: {row}");
    assert_eq!(rx_segments[1]["channels"], json!([0, 1]), "{row}");
}

#[test]
fn ur_25_a_layout_rate_change_of_a_channel_less_stream_only_takes_its_value() {
    // RM-21, item 8: a change to a stream no channel is in only applies its value — no cut,
    // no clock, no device call.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::build(
        FakeConfig { front_ends: vec!["UBX", "BasicRX"], serial: "ur25-layout-empty".to_owned(), ..FakeConfig::default() },
        &[("radio.rx.c1.stream", Scalar::from(0))],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        json!({  "rx": ["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"], "tx": ["0:A:ubx:0"] }),
    );
    direct.settle(Wall::from_millis(30));
    let before = direct.device.calls().len();
    let clocks_before = direct.clocks.sample_clock_records().len();
    direct.update("radio.rx.s1.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(direct.now() + ms(100)));
    direct.settle(Wall::from_millis(200));
    let log = direct.device.calls();
    assert_eq!(log.len(), before, "a change to an empty stream wrote to the device: {log:?}");
    assert_eq!(direct.clocks.sample_clock_records().len(), clocks_before);
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty());
    let instance = direct.finish();
    let timing = section_of(&instance, "timing");
    assert!(timing.as_array().unwrap().iter().all(|row| row["what"] != "rx_segment"), "no segment begins: {timing}");
    let plan = plan_row(&timing);
    assert_eq!(plan["rx"].as_array().unwrap().len(), 1, "{plan}");
}

#[test]
fn ur_25_a_layout_transmit_rate_change_configures_the_new_segment() {
    // Item 8, the transmit sibling: the switch's new segment configures each channel of its
    // set with its own values untimed, and the transmit clock is replaced on its lattice.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let front = FakeConfig { front_ends: vec!["UBX", "BasicTX"], serial: "ur25-layout-tx-rate".to_owned(), ..FakeConfig::default() };
    let mut direct = Direct::build(
        front,
        &[
            ("radio.tx.c0.stream", Scalar::from(0)),
            ("radio.tx.c1.stream", Scalar::from(0)),
            ("radio.tx.c0.frequency_hz", Scalar::try_from(2.4e9).unwrap()),
            ("radio.tx.c1.frequency_hz", Scalar::try_from(25e6).unwrap()),
        ],
        Vec::new(),
        json!({  "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0", "0:B:basic_tx:0"] }),
    );
    direct.settle(Wall::from_millis(30));
    direct.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(direct.now() + ms(100)));
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert!(log.iter().any(|call| call == "apply tx 0 rate=2000000 freq=2400000000 gain=0 antenna=TX/RX at=-"), "{log:?}");
    assert!(log.iter().any(|call| call.starts_with("apply tx 1 ") && call.contains("rate=2000000") && call.contains("freq=25000000") && call.ends_with("at=-")), "{log:?}");
    let records = direct.clocks.sample_clock_records();
    let tx: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/tx/s0").collect();
    assert_eq!(tx.len(), 2, "{records:?}");
    assert_eq!((tx[0].root_ticks_per_tick, tx[1].root_ticks_per_tick), (ezsdr_kernel::time::Rational::new(200, 1).unwrap(), ezsdr_kernel::time::Rational::new(100, 1).unwrap()));
    assert!(tx[0].ended_at.is_some() && tx[1].ended_at.is_none(), "{records:?}");
    assert_eq!(calls(&direct.device, "tx_open 0 [0, 1]").len(), 1, "{log:?}");
    assert!(!log.iter().any(|call| call.starts_with("apply rx") && call.contains("rate=2000000")), "the receive side is untouched: {log:?}");
    let instance = direct.finish();
    let timing = &section_of(&instance, "timing");
    assert!(timing.as_array().unwrap().iter().any(|row| row["what"] == "tx_switch"), "{timing}");
}

// `ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// `ur_25_a_same_size_membership_swap_reopens_the_streamer` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
#[test]
fn ur_25_a_receive_membership_join_opens_the_streamer_with_the_new_set() {
    // RM-25, D14, item 10: a channel joining stream 0 cuts the stream, reopens the streamer
    // with the new set on the lattice, and the new segment configures every channel.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::build(
        FakeConfig::default(),
        &[("radio.rx.c0.frequency_hz", Scalar::try_from(2.0e9).unwrap()), ("radio.rx.c1.frequency_hz", Scalar::try_from(1.1e9).unwrap())],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(30));
    let at = direct.now() + ms(300);
    direct.update("radio.rx.c1.stream", Value::from(0), Cold, Some(at));
    direct.settle(Wall::from_millis(700));
    let log = direct.device.calls();
    let stop = log.iter().position(|call| call == "rx_stop 0 now").expect("the cut: {log:?}");
    let open = log.iter().position(|call| call == "rx_open 0 [0, 1]").expect("the reopen with the new set: {log:?}");
    let start = log.iter().enumerate().position(|(index, call)| index > open && call.starts_with("rx_start 0")).expect("the start: {log:?}");
    assert!(stop < open && open < start, "{log:?}");
    assert_eq!(log.iter().filter(|call| call.starts_with("set_subdev_spec")).count(), 2, "a membership change touches no spec: {log:?}");
    assert!(!log.iter().any(|call| call == "rx_open 0 []"), "{log:?}");
    assert!(log.iter().any(|call| call == "apply rx 1 rate=1000000 freq=1100000000 gain=0 antenna=RX2 at=-"), "{log:?}");
    assert_eq!(log.iter().filter(|call| call == &"apply rx 0 rate=1000000 freq=2000000000 gain=0 antenna=RX2 at=-").count(), 2, "prepare's configuration and the new segment's: {log:?}");
    let records = direct.clocks.sample_clock_records();
    let rx: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/rx/s0").collect();
    assert_eq!(rx.len(), 2, "{records:?}");
    let (end, origin) = (rx[0].ended_at.unwrap().ticks_in(rx[0].root).unwrap(), rx[1].origin.ticks_in(rx[1].root).unwrap());
    assert!(origin >= end + ms(55) && origin - (end + ms(55)) < 200, "{origin} vs {end}");
    let instance = direct.finish();
    let timing = section_of(&instance, "timing");
    let plan = plan_row(&timing);
    assert_eq!(plan["rx"].as_array().unwrap().len(), 2, "{plan}");
    assert_eq!(timing.as_array().unwrap().iter().filter(|row| row["what"] == "rx_segment").count(), 1, "the joined segment: {timing}");
}

#[test]
fn ur_25_a_receive_membership_leave_opens_the_streamer_for_the_remaining_set() {
    // RM-25, item 10: a channel leaving reopens the streamer for the rest — `[1]`, neither
    // `[0]` nor `[0, 1]` — and the new segment configures that rest alone.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::build(
        FakeConfig::default(),
        &[("radio.rx.c1.stream", Scalar::from(0))],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(30));
    direct.update("radio.rx.c0.stream", Value::from(-1), Cold, Some(direct.now() + ms(300)));
    direct.settle(Wall::from_millis(700));
    let log = direct.device.calls();
    assert_eq!(log.iter().filter(|call| call.as_str() == "rx_open 0 [1]").count(), 1, "{log:?}");
    assert!(!log.iter().any(|call| call.as_str() == "rx_open 0 [0]"), "{log:?}");
    let applies: Vec<&String> = log.iter().filter(|call| call.starts_with("apply rx 0 ") && call.ends_with("at=-")).collect();
    assert_eq!(applies.len(), 1, "prepare's own configuration only: {log:?}");
    let instance = direct.finish();
    let plan = plan_row(&section_of(&instance, "timing"));
    assert_eq!(plan["rx"].as_array().unwrap()[1]["channels"], json!([1]), "{plan}");
}

#[test]
fn ur_25_the_last_receive_channel_leaves_no_clock_and_no_block() {
    // RM-25, item 10: the last channel leaving ends the running segment and plans no new
    // one: no clock, no block, and the streamer is not opened empty.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)], layout_selector());
    direct.settle(Wall::from_millis(30));
    let opens = calls(&direct.device, "rx_open").len();
    let starts = calls(&direct.device, "rx_start").len();
    direct.update("radio.rx.c0.stream", Value::from(-1), Cold, None);
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(log.iter().filter(|call| call.as_str() == "rx_stop 0 now").count(), 1, "{log:?}");
    assert_eq!(calls(&direct.device, "rx_open").len(), opens, "{log:?}");
    assert_eq!(calls(&direct.device, "rx_start").len(), starts, "{log:?}");
    assert!(!log.iter().any(|call| call.as_str() == "rx_open 0 []"), "{log:?}");
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert!(records[0].ended_at.is_some(), "{records:?}");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty());
    let instance = direct.finish();
    let plan = plan_row(&section_of(&instance, "timing"));
    assert_eq!(plan["rx"].as_array().unwrap().len(), 1, "{plan}");
}

#[test]
fn ur_34_a_join_after_a_leave_closes_the_source_s_streamer_first() {
    // S0a (H3): a channel detached from s1 must release that streamer's graph edge before s0
    // reopens with the channel. `until` observes the owner's cut, not a fixed delay.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &rx_two(), vec![r0, r1], layout_selector());
    direct.until("both receive streams start", |direct| calls(&direct.device, "rx_start").len() == 2);
    direct.update("radio.rx.c1.stream", Value::from(-1), Cold, Some(direct.now() + ms(40)));
    direct.until("s1's leave cut", |direct| calls(&direct.device, "rx_stop 1 now").len() == 1);
    direct.update("radio.rx.c1.stream", Value::from(0), Cold, Some(direct.now() + ms(40)));
    direct.until("s0's join open attempt", |direct| calls(&direct.device, "rx_open 0 [0, 1]").len() == 1);
    direct.until("s0's joined receive start", |direct| calls(&direct.device, "rx_start 0").len() == 2);
    direct.until("s0's joined receive clock", |direct| direct.clocks.sample_clock_records().iter().filter(|clock| clock.stream.path() == "usrp/rx/s0").count() == 2);
    let log = direct.device.calls();
    let rejected = direct.of("radio.COMMAND_REJECTED");
    let clocks = direct.clocks.sample_clock_records();
    let _ = direct.finish();
    let close = log.iter().position(|call| call == "rx_close 1").expect("the old streamer is closed: {log:?}");
    let open = log.iter().position(|call| call == "rx_open 0 [0, 1]").unwrap();
    assert!(close < open, "source close precedes destination open: {log:?}");
    assert!(rejected.is_empty(), "{rejected:?}");
    let s0: Vec<_> = clocks.iter().filter(|clock| clock.stream.path() == "usrp/rx/s0").collect();
    assert!(s0.len() >= 2 && s0.last().is_some_and(|clock| clock.ended_at.is_none()), "s0's join did not halt: {clocks:?}");
}

#[test]
fn ur_34_a_transmit_join_after_a_leave_closes_the_source_s_streamer_first() {
    // S0b (H3): the transmitter has the same channel-ownership conflict.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut constraints = tx_two();
    constraints.push(("radio.rx.c0.stream", Scalar::from(0)));
    let (rx, _link) = attached_reading("rx0");
    let mut direct = Direct::build(FakeConfig::default(), &constraints, vec![rx], layout_selector());
    direct.until("both transmit clocks are registered", |direct| direct.clocks.sample_clock_records().iter().filter(|clock| clock.stream.path().starts_with("usrp/tx/")).count() == 2);
    direct.update("radio.tx.c1.stream", Value::from(-1), Cold, Some(direct.now() + ms(40)));
    direct.until("s1's transmit clock ends", |direct| direct.clocks.sample_clock_records().iter().any(|clock| clock.stream.path() == "usrp/tx/s1" && clock.ended_at.is_some()));
    direct.update("radio.tx.c1.stream", Value::from(0), Cold, Some(direct.now() + ms(40)));
    direct.until("s0's join open attempt", |direct| calls(&direct.device, "tx_open 0 [0, 1]").len() == 1);
    direct.until("s0's joined transmit clock", |direct| direct.clocks.sample_clock_records().iter()
        .filter(|clock| clock.stream.path() == "usrp/tx/s0").count() == 2
            && direct.clocks.sample_clock_records().iter().any(|clock| clock.stream.path() == "usrp/tx/s0" && clock.ended_at.is_none()));
    let log = direct.device.calls();
    let rejected = direct.of("radio.COMMAND_REJECTED");
    let clocks = direct.clocks.sample_clock_records();
    let _ = direct.finish();
    let close = log.iter().position(|call| call == "tx_close 1").expect("the old streamer is closed: {log:?}");
    let open = log.iter().position(|call| call == "tx_open 0 [0, 1]").unwrap();
    assert!(close < open, "source close precedes destination open: {log:?}");
    assert!(rejected.is_empty(), "{rejected:?}");
    let s0: Vec<_> = clocks.iter().filter(|clock| clock.stream.path() == "usrp/tx/s0").collect();
    assert!(s0.len() >= 2 && s0.last().is_some_and(|clock| clock.ended_at.is_none()), "s0's join did not halt: {clocks:?}");
}

#[test]
fn ur_34_an_idle_destination_leaves_its_join_to_the_owner_when_another_streamer_holds_the_channel() {
    // S0c (H5): a stopped s0 has no streamer yet, but uhd-control must leave the open to its
    // owner while s1 still holds c1.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &rx_two(), vec![r0, r1], layout_selector());
    direct.until("both receive streams start", |direct| calls(&direct.device, "rx_start").len() == 2);
    direct.stop_at("usrp/rx/s0");
    direct.until("s0's stop", |direct| calls(&direct.device, "rx_stop 0 now").len() == 1);
    direct.update("radio.rx.c1.stream", Value::from(-1), Cold, Some(direct.now() + ms(40)));
    direct.until("s1's leave cut", |direct| calls(&direct.device, "rx_stop 1 now").len() == 1);
    direct.update("radio.rx.c1.stream", Value::from(0), Cold, Some(direct.now() + ms(40)));
    direct.start_rx_at("usrp/rx/s0", Some(direct.now() + ms(40)));
    direct.until("s1's streamer release for the restarted destination", |direct| calls(&direct.device, "rx_close 1").len() == 1);
    direct.until("s0's join open attempt", |direct| calls(&direct.device, "rx_open 0 [0, 1]").len() == 1);
    direct.until("s0's joined receive start", |direct| calls(&direct.device, "rx_start 0").len() == 2);
    let log = direct.device.calls();
    let rejected = direct.of("radio.COMMAND_REJECTED");
    let _ = direct.finish();
    let close = log.iter().position(|call| call == "rx_close 1").expect("s1's streamer is closed: {log:?}");
    let open = log.iter().position(|call| call == "rx_open 0 [0, 1]").unwrap();
    assert!(close < open, "reservation did not open before release: {log:?}");
    assert!(rejected.is_empty(), "{rejected:?}");
}

#[test]
fn ur_34_a_membership_change_to_its_own_stream_changes_nothing() {
    // S9: the same membership is a no-op, not a move; a following timed update proves the
    // control thread consumed both Actions without needing a wall-clock delay.
    use ezsdr_kernel::module_api::UpdateClass::{Cold, HardwareTimed};
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &rx_two(), vec![r0, r1], layout_selector());
    direct.until("both receive streams start", |direct| calls(&direct.device, "rx_start").len() == 2);
    direct.until("both receive clocks register", |direct| direct.clocks.sample_clock_records().len() == 2);
    let clock_count = direct.clocks.sample_clock_records().len();
    let before = direct.device.calls();
    direct.update("radio.rx.c1.stream", Value::from(1), Cold, None);
    direct.update("radio.rx.c1.gain_db", Value::num(3.0).unwrap(), HardwareTimed, Some(direct.now() + ms(40)));
    direct.until("the following timed update", |direct| direct.device.calls().iter().any(|call| call.starts_with("apply rx 1 ") && call.contains("gain=3") && !call.ends_with("at=-")));
    let log = direct.device.calls();
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(direct.clocks.sample_clock_records().len(), clock_count, "{log:?}");
    for prefix in ["rx_open", "rx_close", "rx_stop"] {
        assert_eq!(log.iter().filter(|call| call.starts_with(prefix)).count(), before.iter().filter(|call| call.starts_with(prefix)).count(), "{prefix}: {log:?}");
    }
    assert!(rejected.is_empty(), "{rejected:?}");
    let _ = direct.finish();
}

#[test]
fn ur_34_a_move_cuts_both_streams_and_the_source_reopens_before_the_destination_opens() {
    // S1/A7: compare per-owner transitions and the release-dependent plan; 2,000 samples at
    // 2 Msps is a 1 ms call, not 2 ms.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let constraints = rx_three();
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(fake_mixed(), &constraints, vec![r0, r1], mixed_selector());
    direct.until("both non-empty receive streams start", |direct| calls(&direct.device, "rx_start").len() == 2);
    direct.update("radio.rx.c1.stream", Value::from(1), Cold, Some(direct.now() + ms(100)));
    direct.until("both moved receive streams restart", |direct| calls(&direct.device, "rx_start 0").len() == 2 && calls(&direct.device, "rx_start 1").len() == 2);
    direct.until("both moved receive clocks register", |direct| direct.clocks.sample_clock_records().iter().filter(|clock| clock.stream.path() == "usrp/rx/s0").count() == 2
        && direct.clocks.sample_clock_records().iter().filter(|clock| clock.stream.path() == "usrp/rx/s1").count() == 2);
    let log = direct.device.calls();
    let stops = [
        log.iter().position(|call| call == "rx_stop 0 now").expect("s0 cut: {log:?}"),
        log.iter().position(|call| call == "rx_stop 1 now").expect("s1 cut: {log:?}"),
    ];
    let opens = [
        log.iter().position(|call| call == "rx_open 0 [0]").expect("source reopens without c1: {log:?}"),
        log.iter().position(|call| call == "rx_open 1 [1, 2]").expect("destination reopens with c1: {log:?}"),
    ];
    assert!(stops[0] < opens[0] && stops[1] < opens[1], "each owner stops before its own open: {log:?}");
    assert!(opens[0] < opens[1], "the source releases c1 before the destination opens it: {log:?}");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty(), "{log:?}");
    let clocks = direct.clocks.sample_clock_records();
    assert_eq!(clocks.iter().filter(|clock| clock.stream.path() == "usrp/rx/s0").count(), 2, "{clocks:?}");
    assert_eq!(clocks.iter().filter(|clock| clock.stream.path() == "usrp/rx/s1").count(), 2, "{clocks:?}");
    let instance = direct.finish();
    let plan = plan_row(&section_of(&instance, "timing"));
    let source = plan_stream(&plan, "rx", 0);
    let destination = plan_stream(&plan, "rx", 1);
    assert_eq!(source.last().unwrap()["channels"], json!([0]), "{plan}");
    assert_eq!(destination.last().unwrap()["channels"], json!([1, 2]), "{plan}");
    assert_eq!(source.last().unwrap()["sequence"], destination.last().unwrap()["sequence"], "{plan}");
}

#[test]
fn ur_34_a_move_of_the_source_s_only_channel_closes_its_streamer() {
    // S2/A7: the destination is s1 and therefore has rx1, not rx2.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut constraints = rx_three();
    constraints[0] = ("radio.rx.c0.stream", Scalar::from(-1));
    constraints[1] = ("radio.rx.c1.stream", Scalar::from(0));
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(fake_mixed(), &constraints, vec![r0, r1], mixed_selector());
    direct.until("both non-empty receive streams start", |direct| calls(&direct.device, "rx_start").len() == 2);
    direct.update("radio.rx.c1.stream", Value::from(1), Cold, Some(direct.now() + ms(100)));
    direct.until("destination restarts after taking the only source channel", |direct| calls(&direct.device, "rx_start 1").len() == 2);
    let log = direct.device.calls();
    let stop = log.iter().position(|call| call == "rx_stop 0 now").expect("source cut: {log:?}");
    let close = log.iter().position(|call| call == "rx_close 0").expect("empty source closes: {log:?}");
    let open = log.iter().position(|call| call == "rx_open 1 [1, 2]").expect("destination opens: {log:?}");
    assert!(stop < close && close < open, "{log:?}");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty(), "{log:?}");
    assert!(direct.of("radio.LATE_COMMAND").is_empty(), "an untimed change is never late: {log:?}");
    let instance = direct.finish();
    let plan = plan_row(&section_of(&instance, "timing"));
    assert_eq!(plan_stream(&plan, "rx", 0).len(), 1, "the source runs no later channel-less segment: {plan}");
}

#[test]
fn ur_34_a_move_into_an_empty_stream_opens_it_after_the_source_s_release() {
    // S3: the destination has a receive link and line but no channel, clock or streamer.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut constraints = rx_three();
    constraints[1] = ("radio.rx.c1.stream", Scalar::from(1));
    constraints[2] = ("radio.rx.c2.stream", Scalar::from(-1));
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let (r2, _l2) = attached_reading("rx2");
    let mut direct = Direct::build(fake_mixed(), &constraints, vec![r0, r1, r2], mixed_selector());
    direct.until("both non-empty receive streams start", |direct| calls(&direct.device, "rx_start").len() == 2);
    let e = direct.now() + ms(100);
    direct.update("radio.rx.c1.stream", Value::from(2), Cold, Some(e));
    direct.until("the channel enters empty s2", |direct| calls(&direct.device, "rx_start 2").len() == 1);
    direct.until("s2's receive clock registers", |direct| direct.clocks.sample_clock_records().iter().any(|clock| clock.stream.path() == "usrp/rx/s2"));
    let log = direct.device.calls();
    let close = log.iter().position(|call| call == "rx_close 1").expect("source streamer close");
    let open = log.iter().position(|call| call == "rx_open 2 [1]").expect("empty destination open");
    assert!(close < open, "reservation did not open s2 while s1 held c1: {log:?}");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty(), "{log:?}");
    let instance = direct.finish();
    let timing = section_of(&instance, "timing");
    // The move's instant is the requested one or, once that has passed, the booking's — the
    // floor's own rule, compared with the recorded booking rather than with a second read.
    let move_row = timing.as_array().unwrap().iter().find(|row| row["what"] == "move").expect("the move timing row: {timing}");
    assert_eq!(move_row["e"].as_i64(), Some(e.max(move_row["booked_at"].as_i64().unwrap())), "the move's effective instant: {move_row}");
    assert_eq!(move_row["ready"], move_row["booked_at"], "the destination is ready when its move is booked: {move_row}");
    let plan = plan_row(&section_of(&instance, "timing"));
    assert_eq!(plan_stream(&plan, "rx", 2).last().unwrap()["channels"], json!([1]), "{plan}");
}

#[test]
fn ur_34_a_move_from_a_stopped_stream_closes_its_streamer_for_the_destination() {
    // S4: an idle owner still releases a streamer when a later destination segment needs it.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut constraints = rx_three();
    constraints[0] = ("radio.rx.c0.stream", Scalar::from(-1));
    constraints[1] = ("radio.rx.c1.stream", Scalar::from(0));
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(fake_mixed(), &constraints, vec![r0, r1], mixed_selector());
    direct.until("the source receive stream starts", |direct| calls(&direct.device, "rx_start 0").len() == 1);
    direct.stop_at("usrp/rx/s0");
    direct.until("the source is stopped", |direct| calls(&direct.device, "rx_stop 0 now").len() == 1);
    direct.update("radio.rx.c1.stream", Value::from(1), Cold, Some(direct.now() + ms(100)));
    direct.until("the destination receives the moved channel", |direct| calls(&direct.device, "rx_start 1").len() == 2);
    let log = direct.device.calls();
    let close = log.iter().position(|call| call == "rx_close 0").expect("the stopped source releases its streamer: {log:?}");
    let open = log.iter().position(|call| call == "rx_open 1 [1, 2]").expect("the destination opens: {log:?}");
    assert!(close < open, "{log:?}");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty(), "{log:?}");
    let _ = direct.finish();
}

// `ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// `ur_34_a_cold_change_before_the_move_s_release_is_floored_at_the_move` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// `ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// `ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// `ur_34_a_transmit_move_ends_the_old_burst_and_sends_the_new_burst_on_its_stream` is a unit test in `src/provider/tx.rs` on the controlled clock (design note 37).
// `ur_34_a_transmit_move_carries_a_prebooked_burst_across_the_repeat_cut` is a unit test in `src/provider/tx.rs` on the controlled clock (design note 37).
// `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// ------------------------------------------------ 1b-4c: destinations, judgement, manifest

#[test]
fn ur_26_d14_a_layout_instance_names_its_streams() {
    // RM-12, RM-29, D14: under the layout the instance's node and its streams' nodes
    // (`<id>/rx/s<k>`) are the targets; the key decides what changes; a stream node's `Stop`
    // stops that stream alone.
    use ezsdr_kernel::module_api::UpdateClass::HardwareTimed;
    let (bytes, waveform) = waveform_of(&tone(2_000));
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes)]);
    let mut direct = Direct::build_with(
        FakeConfig::default(),
        &[("radio.tx.c0.stream", Scalar::from(0))],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
        inputs,
    );
    direct.settle(Wall::from_millis(30));
    // 1. The instance's node takes an update.
    direct.update("radio.rx.c0.frequency_hz", Value::num(2.1e9).unwrap(), HardwareTimed, Some(direct.now() + ms(40)));
    // 2. A stream's node takes one too.
    let at = direct.now() + ms(40);
    direct.update_at("usrp/rx/s0", "radio.rx.c0.gain_db", Value::num(3.0).unwrap(), HardwareTimed, Some(at));
    // 3. The key decides the direction: a transmit key on a receive stream's node acts on
    //    transmit.
    direct.update_at("usrp/rx/s0", "radio.tx.c0.gain_db", Value::num(1.5).unwrap(), HardwareTimed, Some(at));
    direct.settle(Wall::from_millis(80));
    let log = direct.device.calls();
    assert_eq!(log.iter().filter(|call| call.starts_with("apply rx 0 ") && call.contains("freq=2100000000") && !call.ends_with("at=-")).count(), 1, "{log:?}");
    assert_eq!(log.iter().filter(|call| call.starts_with("apply rx 0 ") && call.contains("gain=3") && !call.ends_with("at=-")).count(), 1, "{log:?}");
    assert_eq!(log.iter().filter(|call| call.starts_with("apply tx 0 ") && call.contains("gain=1.5") && !call.ends_with("at=-")).count(), 1, "{log:?}");
    // 4. A burst on the transmit stream's node is taken and plays.
    let record = direct.clocks.sample_clock_records().into_iter().find(|record| record.stream.path() == "usrp/tx/s0").expect("the transmit clock");
    let k = (direct.now() + ms(30) - record.origin.ticks_in(record.root).unwrap()) / record.root_ticks_per_tick.num() as i64;
    direct.burst_at("usrp/tx/s0", &waveform, false, TimePoint::new(record.domain, k));
    direct.settle(Wall::from_millis(60));
    assert!(!calls(&direct.device, "tx_send").is_empty(), "the burst plays: {:?}", direct.device.calls());
    // 5. `Stop` of a stream's node stops that stream alone: the receive side is cut, and
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
    assert_eq!(log.iter().filter(|call| call.as_str() == "rx_stop 0 now").count(), 1, "{log:?}");
    assert_eq!(direct.clocks.sample_clock_records().len(), records, "{log:?}");
    let cancelled = rejection_reasons(&direct.of("radio.COMMAND_REJECTED"));
    assert!(cancelled.iter().all(|reason| !reason.contains("cancelled by Stop")), "a receive stream's Stop cancels no transmit burst: {cancelled:?}");
    assert!(tx_sends(&direct.device).len() > before_held, "the held burst still plays: {log:?}");
    let stopped = direct.clocks.sample_clock_records();
    let rx: Vec<_> = stopped.iter().filter(|record| record.stream.path() == "usrp/rx/s0").collect();
    assert!(rx[0].ended_at.is_some(), "{stopped:?}");
    // 6. `start_rx` of the same node starts a new segment: a second receive clock.
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
    assert!(!log.iter().any(|call| call.starts_with("tx_send ")), "the cancelled burst never reached the device: {log:?}");
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
fn ur_26_d14_the_old_addresses_and_foreign_nodes_are_refused() {
    // UR-26, D14: under the layout the old `<id>/rx` and `<id>/tx` are no targets, and
    // neither is a node outside the instance's streams.
    use ezsdr_kernel::module_api::UpdateClass::HardwareTimed;
    let mut direct = Direct::build(
        FakeConfig::default(),
        &[("radio.tx.c0.stream", Scalar::from(0))],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(30));
    let before = (direct.device.calls().len(), direct.clocks.sample_clock_records().len());
    let rejections = |direct: &mut Direct| -> Vec<Json> {
        direct.of("radio.COMMAND_REJECTED").into_iter().map(|event| event.payload).collect()
    };
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
    let mut direct = Direct::build_with(
        FakeConfig::default(),
        &[("radio.tx.c0.stream", Scalar::from(0))],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
        inputs,
    );
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
    assert_eq!(calls(&direct.device, "tx_send").len(), 0, "the burst is not sent on stream 0: {:?}", direct.device.calls());
    let _ = direct.finish();
}

#[test]
fn ur_25_a_layout_instance_s_full_duplex_rule_is_judged_at_runtime() {
    // RM-31, item 13: the update's effective instant's whole configuration is judged, the
    // same `coerce` `prepare` uses; a violation is a COMMAND_REJECTED and nothing is booked.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::build(
        FakeConfig::default(),
        &[("radio.tx.c0.stream", Scalar::from(0))],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        json!({  "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"] }),
    );
    direct.settle(Wall::from_millis(30));
    let before = direct.device.calls().len();
    direct.update("radio.rx.c0.antenna", Value::from("TX/RX"), Cold, None);
    direct.settle(Wall::from_millis(200));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    let reason = rejection_reason(&rejected);
    assert!(reason.starts_with("RM-31:"), "{reason}");
    assert!(reason.contains("receive channel c0") && reason.contains("transmit channel c0"), "{reason}");
    let log = direct.device.calls();
    assert!(!log[before..].iter().any(|call| call.contains("antenna=TX/RX")), "the refused change reached the device: {log:?}");
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.len(), 2, "{records:?}");
    assert!(records[0].ended_at.is_none() && records[1].ended_at.is_none(), "the refusal cuts nothing: {records:?}");
    let instance = direct.finish();
    let timing = &section_of(&instance, "timing");
    assert!(timing.as_array().unwrap().iter().all(|row| !(row["what"] == "cold_change" && row["key"] == "radio.rx.c0.antenna")), "{timing}");
    // `prepare` refuses the same settings with the same reason (UR-12).
    let device = fake(FakeConfig { serial: "ur25-rm31-prepare".to_owned(), ..FakeConfig::default() });
    let mut radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).unwrap();
    let constraints = [("radio.tx.c0.stream", Scalar::from(0)), ("radio.rx.c0.antenna", Scalar::from("TX/RX"))];
    let error = direct_prepare(&mut radio, device.clone(), ExecutionClass::HardwareInLoop, Vec::new(), &constraints).unwrap_err();
    assert_eq!(error.kind, ModuleErrorKind::Rejected);
    assert_eq!(error.message, format!("UR-12: {reason}"));
}

#[test]
fn ur_25_a_layout_instance_s_throughput_rule_is_judged_at_runtime() {
    // RM-32, item 13: the throughput of the projected configuration; the same reason in
    // `prepare`.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::build(
        FakeConfig::default(),
        &[("radio.rx.s0.sample_rate_hz", Scalar::try_from(20e6).unwrap())],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(30));
    direct.update("radio.rx.c1.stream", Value::from(0), Cold, None);
    direct.settle(Wall::from_millis(200));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?} {:?}", direct.device.calls());
    let reason = rejection_reason(&rejected);
    assert!(reason.starts_with("RM-32:"), "{reason}");
    assert!(reason.contains("160000000") && reason.contains("rx"), "{reason}");
    let log = direct.device.calls();
    assert!(!log.iter().any(|call| call == "rx_open 0 [0, 1]"), "{log:?}");
    assert!(!log.iter().any(|call| call.starts_with("apply rx 1 ")), "{log:?}");
    let instance = direct.finish();
    let timing = &section_of(&instance, "timing");
    assert!(timing.as_array().unwrap().iter().all(|row| !(row["what"] == "cold_change" && row["key"] == "radio.rx.c1.stream")), "{timing}");
    let device = fake(FakeConfig { serial: "ur25-rm32-prepare".to_owned(), ..FakeConfig::default() });
    let mut radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), device.clone()).unwrap();
    let constraints = [("radio.rx.s0.sample_rate_hz", Scalar::try_from(20e6).unwrap()), ("radio.rx.c1.stream", Scalar::from(0))];
    let error = direct_prepare(&mut radio, device.clone(), ExecutionClass::HardwareInLoop, Vec::new(), &constraints).unwrap_err();
    assert_eq!(error.message, format!("UR-12: {reason}"));
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
    let radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS, "rx": ["0:A:ubx:0"], "tx": [] }), Some(x3xx())), fake(FakeConfig { serial: "ur30-spec-c".to_owned(), ..FakeConfig::default() })).unwrap();
    let device = device_section(&radio);
    assert_eq!(device["subdev_spec"]["rx"], "A:0");
    assert!(device["subdev_spec"]["tx"].is_null(), "{device}");
    let radio = UhdRadio::from_binding(
        &binding(json!({ "args": ARGS,  "rx": ["0:A:ubx:0"], "tx": [] }), Some(x3xx())),
        fake(FakeConfig { serial: "ur30-spec-d".to_owned(), ..FakeConfig::default() }),
    ).unwrap();
    let device = device_section(&radio);
    assert_eq!(device["subdev_spec"]["rx"], "A:0");
    assert!(device["subdev_spec"]["tx"].is_null(), "keys: {device}");
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
fn ur_25_a_no_op_membership_change_takes_only_its_value() {
    // RM-25, item 10: a `stream` update whose value is the one already in effect books
    // nothing — no cut, no reopen, no device call — and takes its value alone, as
    // MockRadio's `was == is` does.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)], layout_selector());
    direct.settle(Wall::from_millis(30));
    let before = (direct.device.calls().len(), direct.clocks.sample_clock_records().len());
    // Untimed: the instant is the booking's own, so no window of the test thread's stands
    // between the value's instant and its row. The bracket is the two reads around the call.
    let booked_at = direct.now();
    direct.update("radio.rx.c1.stream", Value::from(-1), Cold, None);
    let booked_by = direct.now();
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(log.len(), before.0, "the device was not asked: {log:?}");
    assert_eq!(direct.clocks.sample_clock_records().len(), before.1, "the segment was not cut: {log:?}");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty(), "{log:?}");
    assert!(direct.of("radio.LATE_COMMAND").is_empty(), "a no-op update is late: {:?}", direct.of("radio.LATE_COMMAND"));
    let instance = direct.finish();
    let applied = &section_of(&instance, "applied");
    let rows: Vec<&Json> = applied.as_array().unwrap().iter().filter(|row| row["key"] == "radio.rx.c1.stream" && row["read_back"].is_null()).collect();
    assert_eq!(rows.len(), 1, "one value-only row: {applied}");
    let at = rows[0]["at"]["ticks"].as_i64().unwrap();
    // Untimed, the value's instant is the booking's or the receive start T0's, whichever is
    // later (RM-25's floor); that it is not before the booking is what the test thread could
    // move, and it holds either way.
    assert!(booked_at <= booked_by, "the clock did not move: {booked_at} then {booked_by}");
    assert!(at >= booked_at, "the value's instant {at} precedes its booking {booked_at}");
}

// `ur_25_a_layout_cold_change_of_a_linkless_receive_side_is_floored_at_t0` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
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

#[test]
fn ur_13_the_reopen_enforces_the_direction_s_rate_comparison() {
    // UR-13 (Sol 3): a direction opened first by an enable from no stream is compared
    // there; when the comparison refuses, the direction's recorded outcome is re-enforced
    // at every later open — the first open the owner makes included —, so the segment is
    // refused instead of run on a device the profile cannot describe.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    for tx in [false, true] {
        let dir = if tx { "tx" } else { "rx" };
        let inner = fake(FakeConfig { serial: format!("ur13-reopen-{dir}"), ..FakeConfig::default() });
        let tested: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: Some(off_grid()), tx: Some(off_grid()), failing: None });
        let mut direct = Direct::build_on(
            inner.clone(),
            tested,
            &[("radio.rx.c0.stream", Scalar::from(-1)), ("radio.tx.c0.stream", Scalar::from(-1))],
            vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
            layout_selector(),
            BTreeMap::new(),
        );
        direct.settle(Wall::from_millis(30));
        assert!(calls(&direct.device, &format!("{dir}_open")).is_empty(), "{dir}: the arm opened no streamer: {:?}", direct.device.calls());
        // The direction's first open: uhd-control's own enable from no stream, compared there
        // and refused. The change's values stay the configuration of record and the stream
        // is halted (RM-25).
        direct.update(&format!("radio.{dir}.c0.stream"), Value::from(0), Cold, Some(direct.now() + ms(40)));
        direct.settle(Wall::from_millis(200));
        let first: Vec<String> = rejection_reasons(&direct.of("radio.COMMAND_REJECTED"));
        assert_eq!(first.len(), 1, "{dir}: {first:?} {:?}", direct.device.calls());
        assert!(first[0].starts_with("UR-13:") && first[0].contains(dir), "{dir}: {}", first[0]);
        // A second open: a `start_rx` is the receive owner's own reopen, which re-enforces the
        // refused outcome; a second channel joining the transmit stream is an enable from no
        // stream again, which uhd-control compares there. Either way: a second
        // `COMMAND_REJECTED`, one more device open, and the segment neither configured nor run.
        // The transmit owner's reopen is pinned by its unit test (tx.rs).
        let opens = calls(&direct.device, &format!("{dir}_open")).len();
        let applies = calls(&direct.device, "apply").len();
        if tx {
            direct.update("radio.tx.c1.stream", Value::from(0), Cold, Some(direct.now() + ms(60)));
            direct.settle(Wall::from_millis(300));
        } else {
            direct.start_rx_at("usrp/rx/s0", Some(direct.now() + ms(60)));
            direct.settle(Wall::from_millis(300));
        }
        let rejected: Vec<String> = rejection_reasons(&direct.of("radio.COMMAND_REJECTED"));
        assert_eq!(rejected.len(), 2, "{dir}: the owner's open is refused too: {rejected:?} {:?}", direct.device.calls());
        assert!(rejected[1].starts_with("UR-13:") && rejected[1].contains(dir), "{dir}: {}", rejected[1]);
        assert_eq!(calls(&direct.device, &format!("{dir}_open")).len(), opens + 1, "{dir}: the owner opened the streamer once: {:?}", direct.device.calls());
        assert_eq!(calls(&direct.device, "apply").len(), applies, "{dir}: the refused segment is not configured: {:?}", direct.device.calls());
        let records = direct.clocks.sample_clock_records();
        assert!(records.iter().filter(|record| record.stream.path().starts_with(&format!("usrp/{dir}"))).all(|record| record.ended_at.is_some()), "{dir}: no segment ran: {records:?}");
        let instance = direct.finish();
        assert_eq!(section_of(&instance, "device")["rates_checked"][dir], true, "{dir}: the comparison is recorded");
    }
}

#[test]
fn ur_13_the_receive_owner_s_first_open_is_compared_there() {
    // UR-13: the receive owner's first open is the direction's only comparison when
    // uhd-control's enable from no stream was refused at its open, before it compared. The
    // owner's `start_rx` then opens the streamer, and the device's rates refuse it there.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let inner = fake(FakeConfig { faults: vec![FakeFault::RxOpenFails], serial: "ur13-owner-first-rx".to_owned(), ..FakeConfig::default() });
    let tested: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: Some(off_grid()), tx: None, failing: None });
    let mut direct = Direct::build_on(
        inner.clone(),
        tested,
        &[("radio.rx.c0.stream", Scalar::from(-1)), ("radio.tx.c0.stream", Scalar::from(-1))],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
        BTreeMap::new(),
    );
    direct.settle(Wall::from_millis(30));
    assert!(calls(&direct.device, "rx_open").is_empty(), "`arm` opened no receive streamer: {:?}", direct.device.calls());
    // The enable from no stream: its open is refused before any comparison, so nothing is
    // compared before the open, and the stream is halted (RM-25).
    direct.update("radio.rx.c0.stream", Value::from(0), Cold, Some(direct.now() + ms(40)));
    direct.settle(Wall::from_millis(200));
    let first: Vec<String> = rejection_reasons(&direct.of("radio.COMMAND_REJECTED"));
    assert_eq!(first.len(), 1, "{first:?} {:?}", direct.device.calls());
    assert!(!first[0].starts_with("UR-13:"), "the first refusal is the open's: {}", first[0]);
    // The owner has nothing planned and settles; its `start_rx` opens the streamer, and the
    // owner's comparison, the direction's first, refuses it.
    let applies = calls(&direct.device, "apply").len();
    direct.start_rx_at("usrp/rx/s0", Some(direct.now() + ms(60)));
    direct.settle(Wall::from_millis(300));
    let rejected: Vec<String> = rejection_reasons(&direct.of("radio.COMMAND_REJECTED"));
    assert_eq!(rejected.len(), 2, "the owner's open is refused by its comparison: {rejected:?} {:?}", direct.device.calls());
    assert!(rejected[1].starts_with("UR-13:") && rejected[1].contains("rx"), "{}", rejected[1]);
    assert_eq!(calls(&direct.device, "rx_open").len(), 2, "{:?}", direct.device.calls());
    assert_eq!(calls(&direct.device, "apply").len(), applies, "the refused segment is not configured: {:?}", direct.device.calls());
    let instance = direct.finish();
    assert_eq!(section_of(&instance, "device")["rates_checked"]["rx"], true, "the owner's comparison is recorded");
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

// `ur_25_a_floor_raised_layout_join_is_judged_at_its_instant` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
// `ur_24_the_layout_destination_follows_the_effective_membership` is a unit test in `src/provider/control.rs` on the controlled clock (design note 37).
#[test]
fn ur_13_a_runtime_join_compares_the_rate_set() {
    // UR-13, major 5: the comparison is not `arm`'s alone — a direction whose streamer
    // opens first at a run-time join is compared there, an off-grid device is refused, and
    // the segment is halted (RM-25).
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let inner = fake(FakeConfig { serial: "ur13-runtime-join".to_owned(), ..FakeConfig::default() });
    let tested: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: None, tx: Some(off_grid()), failing: None });
    let mut direct = Direct::build_on(
        inner.clone(),
        tested,
        &[("radio.tx.c0.stream", Scalar::from(-1))],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
        BTreeMap::new(),
    );
    direct.settle(Wall::from_millis(30));
    assert!(calls(&direct.device, "tx_open").is_empty(), "`arm` opened no transmit streamer: {:?}", direct.device.calls());
    direct.update("radio.tx.c0.stream", Value::from(0), Cold, Some(direct.now() + ms(60)));
    direct.settle(Wall::from_millis(400));
    let log = direct.device.calls();
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?} {log:?}");
    let reason = rejection_reason(&rejected);
    assert!(reason.starts_with("UR-13:") && reason.contains("tx"), "{reason}");
    assert_eq!(calls(&direct.device, "tx_open 0 [0]").len(), 1, "the join opened the streamer and was compared there: {log:?}");
    assert_eq!(calls(&direct.device, "rates tx 0").len(), 1, "{log:?}");
    assert!(!log.iter().any(|call| call.starts_with("apply tx 0 ") && call.ends_with("at=-")), "the refused segment is not configured: {log:?}");
    let instance = direct.finish();
    assert_eq!(section_of(&instance, "device")["rates_checked"]["tx"], true, "the comparison is recorded");
}

#[test]
fn ur_13_a_rate_read_that_fails_refuses() {
    // UR-13, major 5: a `rates` read that fails is a refusal with `UR-13:` — never
    // "no rates", which would let a device the profile cannot describe run.
    let dir = TempDir::new();
    let inner = fake(FakeConfig { serial: "ur13-read-fails".to_owned(), ..FakeConfig::default() });
    let device: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: None, tx: None, failing: Some(ezsdr_radio_uhd::Dir::Rx) });
    let run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }), "{:?}", run.state());
    let manifest = run.finish();
    let reason = failure(&manifest);
    assert!(reason.contains("UR-13:") && reason.contains("rates read failed"), "{reason}");
    let log = inner.calls();
    let opened = log.iter().position(|call| call.starts_with("rx_open")).expect("the streamer opened");
    assert!(log.iter().position(|call| call.starts_with("close_streams")).is_some_and(|closed| opened < closed), "{log:?}");
}


#[test]
fn ur_13_an_enable_from_no_stream_compares_the_rate_set() {
    // UR-13, major 5: the receive side's first open can be uhd-control's enable from no
    // stream; it is compared there — the receive reopen only enforces the direction's
    // recorded outcome — so an off-grid device refuses the enable.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let inner = fake(FakeConfig { serial: "ur13-from-none".to_owned(), ..FakeConfig::default() });
    let tested: Arc<dyn Device> = Arc::new(RatesMisreported { inner: inner.clone(), rx: Some(off_grid()), tx: None, failing: None });
    let mut direct = Direct::build_on(
        inner.clone(),
        tested,
        &[("radio.rx.c0.stream", Scalar::from(-1))],
        vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)],
        json!({}),
        BTreeMap::new(),
    );
    direct.settle(Wall::from_millis(30));
    assert!(calls(&direct.device, "rx_open").is_empty(), "`arm` opened no receive streamer: {:?}", direct.device.calls());
    direct.update("radio.rx.c0.stream", Value::from(0), Cold, Some(direct.now() + ms(50)));
    direct.settle(Wall::from_millis(200));
    let log = direct.device.calls();
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?} {log:?}");
    let reason = rejection_reason(&rejected);
    assert!(reason.starts_with("UR-13:") && reason.contains("rx"), "{reason}");
    assert_eq!(calls(&direct.device, "rx_open 0 [0]").len(), 1, "the enable opened the streamer and was compared there: {log:?}");
    assert!(!log.iter().any(|call| call.starts_with("apply rx 0 ") && call.ends_with("at=-")), "the refused enable is not configured: {log:?}");
    let _ = direct.finish();
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
fn ur_25_a_refused_enable_from_no_stream_halts_and_keeps_its_values() {
    // UR-25, RM-25 (Sol 2): a configuration uhd-control applies itself for an enable from
    // no stream, which the device refuses, is booked refused as its owner books a refusal
    // it finds: the change's values stay the configuration of record, the stream is
    // halted, and the next enable applies them.
    use ezsdr_kernel::module_api::UpdateClass::{Cold, HardwareTimed};
    let mut direct = Direct::build(
        FakeConfig {
            faults: vec![FakeFault::WrongAntenna { claimed: "TX/RX", applied: "RX2" }],
            serial: "ur25-from-none-refused".to_owned(),
            ..FakeConfig::default()
        },
        &[
            ("radio.rx.c0.stream", Scalar::from(-1)),
            ("radio.rx.c1.stream", Scalar::from(-1)),
            ("radio.rx.c1.antenna", Scalar::from("TX/RX")),
        ],
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(30));
    assert!(calls(&direct.device, "rx_open").is_empty(), "`arm` opened no receive streamer: {:?}", direct.device.calls());
    // The channel joins with no stream to join: uhd-control opens the streamer and applies
    // the configuration itself, and the antenna's read-back refuses it.
    let join_at = direct.now() + ms(50);
    direct.update("radio.rx.c1.stream", Value::from(0), Cold, Some(join_at));
    direct.settle(Wall::from_millis(200));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?} {:?}", direct.device.calls());
    let reason = rejection_reason(&rejected);
    assert!(reason.starts_with("UR-12:") && reason.contains("reads back RX2"), "{reason}");
    assert!(rejected[0].payload["action"].is_u64(), "the enable from no stream names its Action: {:?}", rejected[0].payload);
    assert!(direct.clocks.sample_clock_records().is_empty(), "the refused enable ran no segment");
    // A timed update for the channel: the halted stream has no channel for it, so it is
    // recorded at its instant, and the next enable applies it (UR-24, RM-25).
    let gain_at = direct.now() + ms(50);
    direct.update("radio.rx.c1.gain_db", Value::num(3.0).unwrap(), HardwareTimed, Some(gain_at));
    direct.settle(Wall::from_millis(200));
    // The successful change restarts the stream from the refused change's configuration:
    // the joined channel, its recorded gain and the antenna that now reads back.
    direct.update("radio.rx.c1.antenna", Value::from("RX2"), Cold, Some(direct.now() + ms(50)));
    direct.settle(Wall::from_millis(400));
    let log = direct.device.calls();
    assert_eq!(log.iter().filter(|call| call.as_str() == "rx_open 0 [1]").count(), 2, "the refused enable's open and the restart's: {log:?}");
    let applied: Vec<&String> = log.iter().filter(|call| call.starts_with("apply rx 1 ") && call.ends_with("at=-")).collect();
    assert!(applied.iter().any(|call| call.contains("gain=3") && call.contains("antenna=RX2")), "the restart applies the recorded gain and the refused membership: {log:?}");
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.iter().filter(|record| record.stream.path() == "usrp/rx/s0").count(), 1, "{records:?}");
    assert!(records.iter().any(|record| record.stream.path() == "usrp/rx/s0" && record.ended_at.is_none()), "{records:?}");
    assert!(direct.of("radio.COMMAND_REJECTED").len() == 1, "the restart alone succeeds: {:?}", direct.device.calls());
    let instance = direct.finish();
    let rows = &section_of(&instance, "applied");
    assert_eq!(
        rows.as_array().unwrap().iter().filter(|row| row["key"] == "radio.rx.c1.gain_db" && row["note"] == "no channel" && row["at"]["ticks"] == gain_at).count(),
        1,
        "the timed update is recorded at its own instant while the stream is halted: {rows}"
    );
    let timing = &section_of(&instance, "timing");
    let changes: Vec<&Json> = timing.as_array().unwrap().iter().filter(|row| row["what"] == "cold_change" && row["key"] == "radio.rx.c1.stream").collect();
    assert_eq!(changes.len(), 1, "the refused change is booked at its instant: {timing}");
    // The requested instant or, once that has passed, the booking's: the floor's own rule,
    // compared with the recorded booking rather than with a second read.
    assert_eq!(changes[0]["e"].as_i64(), Some(join_at.max(changes[0]["booked_at"].as_i64().unwrap())), "{timing}");
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

/// Three receive channels: c0 and c1 in s0 at 2 Msps, c2 in s1 at 1 Msps.
fn rx_three() -> Vec<(&'static str, Scalar)> {
    vec![
        ("radio.rx.c0.stream", Scalar::from(0)),
        ("radio.rx.c1.stream", Scalar::from(0)),
        ("radio.rx.c2.stream", Scalar::from(1)),
        ("radio.rx.s0.sample_rate_hz", Scalar::try_from(2e6).unwrap()),
        ("radio.rx.s1.sample_rate_hz", Scalar::try_from(1e6).unwrap()),
        ("radio.rx.s2.sample_rate_hz", Scalar::try_from(1e6).unwrap()),
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
fn ur_25_a_channel_enabled_from_no_stream_in_a_later_stream_opens_and_starts_it() {
    // UR-25, RM-29 (S3): a channel joining stream 1 from no stream opens that stream's
    // streamer, configures it and starts it; stream 0 is not cut.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut constraints = rx_two();
    constraints[1] = ("radio.rx.c1.stream", Scalar::from(-1));
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &constraints, vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    let join = direct.now() + ms(40);
    direct.update("radio.rx.c1.stream", Value::from(1), Cold, Some(join));
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    let open = log.iter().position(|call| call == "rx_open 1 [1]").expect("the open: {log:?}");
    let apply = log.iter().position(|call| call.starts_with("apply rx 1 ") && call.ends_with("at=-")).expect("the configure: {log:?}");
    let start = log.iter().enumerate().position(|(index, call)| index > apply && call.starts_with("rx_start 1 ")).expect("the start: {log:?}");
    assert!(open < apply && apply < start, "{log:?}");
    assert_eq!((calls(&direct.device, "rx_open 1 [1]").len(), log.iter().filter(|call| call.starts_with("apply rx 1 ") && call.ends_with("at=-")).count(), calls(&direct.device, "rx_start 1").len()), (1, 1, 1), "{log:?}");
    assert_eq!(calls(&direct.device, "rx_stop 0").len(), 0, "{log:?}");
    let first_start = log.iter().position(|call| call.starts_with("rx_start 0 ")).unwrap();
    assert!(!log[first_start..].iter().any(|call| call.starts_with("apply rx 0 ")), "stream 0 is not configured for the join: {log:?}");
    let records = direct.clocks.sample_clock_records();
    let s0 = records.iter().find(|record| record.stream.path() == "usrp/rx/s0").expect("stream 0's clock");
    assert!(s0.ended_at.is_none(), "{records:?}");
    let origin = log.iter().find(|call| call.starts_with("rx_start 1 ")).unwrap().rsplit(' ').next().unwrap().parse::<i64>().unwrap();
    assert!(origin >= join, "the segment begins at or after the change: {origin} vs {join}");
    let _ = direct.finish();
}

#[test]
fn ur_25_a_channel_leaving_a_later_stream_cuts_its_stream_alone() {
    // UR-25, RM-29 (S5): a channel leaving stream 1 cuts that stream alone; stream 0 goes
    // on. Joining it again starts a new segment without reopening the streamer or touching
    // stream 0.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &rx_two(), vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.update("radio.rx.c1.stream", Value::from(-1), Cold, Some(direct.now() + ms(40)));
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_stop 1 now").len(), 1, "{log:?}");
    assert_eq!(calls(&direct.device, "rx_stop 0").len(), 0, "{log:?}");
    let records = direct.clocks.sample_clock_records();
    assert!(records.iter().find(|record| record.stream.path() == "usrp/rx/s1").unwrap().ended_at.is_some(), "{records:?}");
    assert!(records.iter().find(|record| record.stream.path() == "usrp/rx/s0").unwrap().ended_at.is_none(), "{records:?}");
    let starts = calls(&direct.device, "rx_start 1").len();
    direct.settle(Wall::from_millis(100));
    assert_eq!(calls(&direct.device, "rx_start 1").len(), starts, "a cut starts no segment: {:?}", direct.device.calls());
    direct.update("radio.rx.c1.stream", Value::from(1), Cold, None);
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_start 1").len(), starts + 1, "{log:?}");
    assert_eq!(calls(&direct.device, "rx_open 1 [1]").len(), 2, "the enable from no stream opens the streamer: {log:?}");
    assert_eq!(calls(&direct.device, "rx_open 0").len(), 1, "stream 0 is not reopened: {log:?}");
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").count(), 2, "{records:?}");
    let _ = direct.finish();
}

#[test]
fn ur_25_a_rate_change_of_a_later_stream_cuts_that_stream_alone() {
    // UR-25, RM-29 (S6): stream 1's rate change cuts and configures stream 1 alone; stream
    // 0's segment and clock stand.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &rx_two(), vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.update("radio.rx.s1.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(direct.now() + ms(40)));
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_stop 1 now").len(), 1, "{log:?}");
    assert_eq!(log.iter().filter(|call| call.starts_with("apply rx 1 ") && call.contains("rate=2000000") && call.ends_with("at=-")).count(), 1, "{log:?}");
    assert_eq!(calls(&direct.device, "rx_open 1").len(), 1, "the same set is not reopened: {log:?}");
    assert_eq!(calls(&direct.device, "rx_stop 0").len(), 0, "{log:?}");
    let first_start = log.iter().position(|call| call.starts_with("rx_start 0 ")).unwrap();
    assert!(!log[first_start..].iter().any(|call| call.starts_with("apply rx 0 ")), "{log:?}");
    let records = direct.clocks.sample_clock_records();
    assert!(records.iter().find(|record| record.stream.path() == "usrp/rx/s0").unwrap().ended_at.is_none(), "{records:?}");
    let s1: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").collect();
    assert_eq!(s1.len(), 2, "{records:?}");
    assert!(s1[0].ended_at.is_some() && s1[1].ended_at.is_none(), "{records:?}");
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

#[test]
fn ur_25_a_refused_change_of_a_later_stream_halts_that_stream_alone() {
    // UR-25 (S8): a configure the device refuses halts its own stream; the other stream
    // keeps delivering.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let (r0, link0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(
        FakeConfig { faults: vec![FakeFault::WrongRate { claimed: 4e6, applied: 3.8e6, nth: 0 }], ..FakeConfig::default() },
        &rx_two(),
        vec![r0, r1],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(30));
    let drain = |link: &Arc<dyn ezsdr_kernel::stream::DataLink>| { while link.receive().is_some() {} };
    drain(&link0);
    direct.update("radio.rx.s1.sample_rate_hz", Value::num(4e6).unwrap(), Cold, Some(direct.now() + ms(40)));
    direct.settle(Wall::from_millis(300));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert!(rejection_reason(&rejected).contains("UR-12: the device applied 3800000 S/s"), "{rejected:?}");
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_stop 1 now").len(), 1, "{log:?}");
    let records = direct.clocks.sample_clock_records();
    let s1: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").collect();
    assert_eq!(s1.len(), 1, "the refused segment starts nothing: {records:?}");
    assert!(s1[0].ended_at.is_some(), "{records:?}");
    assert_eq!(calls(&direct.device, "rx_start 1").len(), 1, "{log:?}");
    assert!(records.iter().find(|record| record.stream.path() == "usrp/rx/s0").unwrap().ended_at.is_none(), "{records:?}");
    assert!(link0.receive().is_some(), "stream 0 still delivers after stream 1's refusal");
    let instance = direct.finish();
    let plan = plan_row(&section_of(&instance, "timing"));
    assert!(!plan["rx"].as_array().unwrap().iter().any(|row| row["ticks_per_sample"][0] == 50), "the refused 4 MS/s segment is not planned: {plan}");
}

#[test]
fn ur_24_a_timed_update_acts_on_the_stream_that_holds_its_channel() {
    // UR-24, RM-29 (S9): a timed update's decision line is the stream that holds its
    // channel at the instant; a halt of stream 0 does not stop channel 1's update.
    use ezsdr_kernel::module_api::UpdateClass::{Cold, HardwareTimed};
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(
        FakeConfig { faults: vec![FakeFault::WrongRate { claimed: 4e6, applied: 3.8e6, nth: 0 }], ..FakeConfig::default() },
        &rx_two(),
        vec![r0, r1],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(30));
    direct.update("radio.rx.s0.sample_rate_hz", Value::num(4e6).unwrap(), Cold, Some(direct.now() + ms(40)));
    direct.settle(Wall::from_millis(200));
    // Untimed ("as soon as possible"): its decision line is the stream that holds the channel
    // at its released instant, which a delayed test thread cannot move past a scene.
    direct.update_at("usrp/rx/s0", "radio.rx.c1.gain_db", Value::num(3.0).unwrap(), HardwareTimed, None);
    direct.settle(Wall::from_millis(100));
    let log = direct.device.calls();
    assert_eq!(log.iter().filter(|call| call.starts_with("apply rx 1 ") && call.contains("gain=3") && !call.ends_with("at=-")).count(), 1, "{log:?}");
    assert!(!log.iter().any(|call| call.starts_with("apply rx 0 ") && call.contains("gain=3")), "{log:?}");
    assert!(direct.of("radio.COMMAND_QUEUE_FULL").is_empty(), "{log:?}");
    assert!(direct.of("radio.LATE_COMMAND").is_empty(), "{log:?}");
    let instance = direct.finish();
    let rows = &section_of(&instance, "applied");
    let row = rows.as_array().unwrap().iter().find(|row| row["key"] == "radio.rx.c1.gain_db" && row["issued"] == true).expect("the update's issued row");
    assert_eq!(row["issued"], true, "{rows}");
    assert!(row.get("note").is_none(), "{rows}");
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
    let first = calls(&direct.device, "tx_send 0").len();
    let none = calls(&direct.device, "tx_send 1").len();
    assert!(first > 0 && none == 0, "only stream 0 sends: {:?}", direct.device.calls());
    burst_in(&direct, "usrp/tx/s1", &s1, &waveform, false, ms(40));
    direct.settle(Wall::from_millis(80));
    assert_eq!(calls(&direct.device, "tx_send 0").len(), first, "stream 0 is not sent stream 1's burst: {:?}", direct.device.calls());
    assert!(calls(&direct.device, "tx_send 1").len() > none, "stream 1 sends its own: {:?}", direct.device.calls());
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
    assert_eq!(calls(&direct.device, "tx_send").len(), 0, "{:?}", direct.device.calls());
    let _ = direct.finish();
}

#[test]
fn ur_26_two_receive_streams_end_at_their_own_stops() {
    // UR-26, RM-16 (S12a): a Stop of one receive stream ends it at its own stop, and the
    // other still runs to the Orderly stop later; the per-stream delivered floor is S12b/c's.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut constraints = rx_two();
    constraints[1] = ("radio.rx.c1.stream", Scalar::from(-1));
    let (r0, link0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig { rx_latency: Wall::from_millis(50), ..FakeConfig::default() }, &constraints, vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.update("radio.rx.c1.stream", Value::from(1), Cold, Some(direct.now() + ms(50)));
    direct.settle(Wall::from_millis(400));
    direct.stop_at("usrp/rx/s1");
    // Wait for stream 1's owner to finish, but not for stream 0's next (50 ms late)
    // packet: the abort must see the delivered floor stream 1's finish did or did not clear.
    // The packet between them is pinned by `ur_26_a_stream_s_delivered_is_cleared_alone`,
    // `ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts` and
    // `ur_26_an_end_takes_delivered_from_each_segment_s_own_stream` (design note 37).
    // A watchdog, not the verdict's bound: the state is what is asserted below.
    let deadline = Instant::now() + Wall::from_secs(10);
    let ended_before_abort = loop {
        if direct.clocks.sample_clock_records().iter().any(|record| record.stream.path() == "usrp/rx/s1" && record.ended_at.is_some()) {
            break true;
        }
        assert!(Instant::now() < deadline, "stream 1's own Stop did not end it within 10 s: {:?}", direct.clocks.sample_clock_records());
        direct.device.host().sleep(Wall::from_millis(1));
    };
    // The state, not a time bound: stream 1's own Stop ended it before the abort, so its end
    // is not the abort's cut.
    assert!(ended_before_abort, "stream 1's own Stop did not end it before the abort: {:?}", direct.clocks.sample_clock_records());
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
    let ended = |path: &str| records.iter().filter(|record| record.stream.path() == path)
        .filter_map(|record| record.ended_at.map(|at| at.ticks_in(record.root).unwrap())).max();
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
    impl Drop for Release<'_> { fn drop(&mut self) { self.0.release(); } }
    gated.hold();
    let release = Release(&gated);
    gated.entered_n(&[("rx_recv", 2), ("tx_", 2)]);
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    drop(release);
    ended(&ends, &["uhd-tx-s0", "uhd-tx-s1", "uhd-rx-s0", "uhd-rx-s1"]);
    direct.radio.cleanup();
    let rejected = &section_of(direct.radio.instance(), "rejected");
    let threads: Vec<String> = rejected.as_array().unwrap().iter()
        .filter(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("did not join")))
        .map(|row| row["thread"].as_str().unwrap().to_owned()).collect();
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
    impl Drop for Release<'_> { fn drop(&mut self) { self.0.release(); } }
    gated.hold();
    let release = Release(&gated);
    gated.entered(&["rx_recv"]);
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    drop(release);
    ended(&ends, &["uhd-rx-s1"]);
    direct.radio.cleanup();
    let rejected = &section_of(direct.radio.instance(), "rejected");
    let threads: Vec<String> = rejected.as_array().unwrap().iter()
        .filter(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("did not join")))
        .map(|row| row["thread"].as_str().unwrap().to_owned()).collect();
    assert_eq!(threads, ["uhd-rx-s1"], "{rejected}");
    assert_eq!(calls(&gated.inner, "close_streams").len(), 0, "{:?}", gated.inner.calls());
    assert_eq!(rejected.as_array().unwrap().iter().filter(|row| row["leaked"] == "streamers").count(), 1, "{rejected}");
}

#[test]
fn ur_16_a_panicking_later_stream_ends_the_run() {
    // UR-16, RM-29 (S15): a panic in stream 1's owner is that thread's, once a
    // DEVICE_LOST, and the other stream ends at the loss.
    struct PanicOnOne { inner: Arc<FakeDevice> }
    impl ezsdr_radio_uhd::Device for PanicOnOne {
        fn describe(&self) -> Json { self.inner.describe() }
        fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
        fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
        fn channels(&self, d: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(d) }
        fn front_end(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.front_end(d, c) }
        fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.serial() }
        fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(d, m) }
        fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(d, c) }
        fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(c, t) }
        fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(p) }
        fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.inner.time_now() }
        fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
        fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> { self.inner.apply(d, c, s, at) }
        fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(s, c) }
        fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(s, at) }
        fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
        fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(s, at) }
        fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv {
            assert_ne!(s, 1, "fake: rx_recv panics");
            self.inner.rx_recv(s, n, t)
        }
        fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(s, c) }
        fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.inner.tx_send(s, b, at, sob, eob, t) }
        fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> { self.inner.tx_async(s, t) }
        fn rx_close(&self, s: usize) { self.inner.rx_close(s) }
        fn tx_close(&self, s: usize) { self.inner.tx_close(s) }
        fn close_streams(&self) { self.inner.close_streams() }
        fn mark_lost(&self) { self.inner.mark_lost() }
        fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
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
    reached: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl HoldAfterApply {
    fn new(inner: Arc<FakeDevice>) -> HoldAfterApply {
        HoldAfterApply { inner, pending: Mutex::new(None), reached: Mutex::new(None), release: Mutex::new(None) }
    }

    fn hold_channel(&self, chan: usize) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (reached, reached_rx) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *self.pending.lock().unwrap() = Some(chan);
        *self.reached.lock().unwrap() = Some(reached);
        *self.release.lock().unwrap() = Some(release_rx);
        (reached_rx, release)
    }
}

impl ezsdr_radio_uhd::Device for HoldAfterApply {
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, d: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(d) }
    fn front_end(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.front_end(d, c) }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(d, m) }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(d, c) }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(c, t) }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(p) }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> {
        let applied = self.inner.apply(d, c, s, at)?;
        let hold = at.is_none() && d == ezsdr_radio_uhd::Dir::Rx && *self.pending.lock().unwrap() == Some(c);
        if hold {
            *self.pending.lock().unwrap() = None;
            if let Some(reached) = self.reached.lock().unwrap().take() {
                let _ = reached.send(());
            }
            if let Some(release) = self.release.lock().unwrap().take() {
                let _ = release.recv_timeout(Wall::from_secs(10));
            }
        }
        Ok(applied)
    }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(s, c) }
    fn rx_start(&self, s: usize, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(s, at) }
    fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
    fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(s, at) }
    fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv { self.inner.rx_recv(s, n, t) }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(s, c) }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.inner.tx_send(s, b, at, sob, eob, t) }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> { self.inner.tx_async(s, t) }
    fn rx_close(&self, s: usize) { self.inner.rx_close(s) }
    fn tx_close(&self, s: usize) { self.inner.tx_close(s) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
}

#[test]
fn ur_26_a_receive_start_queued_by_the_stop_is_issued_and_ends_at_its_origin() {
    // UR-26 (S16a): the owner does not re-read `Provider::stop` after its configuration;
    // the start it queued is issued, and the plan's end cuts the segment at its origin: no
    // block, and an untimed stop after the origin (VH-6).
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let inner = fake(FakeConfig::default());
    let spy = Arc::new(HoldAfterApply::new(inner.clone()));
    let (r0, _l0) = attached_reading("rx0");
    let (r1, link1) = attached_reading("rx1");
    let mut direct = Direct::armed_on(inner.clone(), spy.clone(), &rx_two(), vec![r0, r1], layout_selector(), BTreeMap::new());
    direct.start();
    direct.settle(Wall::from_millis(50));
    while link1.receive().is_some() {}
    let (hold, release) = spy.hold_channel(1);
    direct.update("radio.rx.s1.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(direct.now() + ms(50)));
    hold.recv_timeout(Wall::from_secs(10)).expect("the owner's configure");
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    while link1.receive().is_some() {}
    let _ = release.send(());
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_start 1").len(), 2, "the owner still issues its timed start: {log:?}");
    assert!(link1.receive().is_none(), "the end at the origin publishes nothing: {log:?}");
    assert_eq!(calls(&direct.device, "rx_stop 1 now").len(), 2, "the old segment's cut and the queued start's origin: {log:?}");
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").count(), 1, "no clock is registered without a block: {records:?}");
    assert!(log.iter().any(|call| call.starts_with("apply rx 1 ") && call.contains("rate=2000000") && call.ends_with("at=-")), "{log:?}");
    direct.radio.cleanup();
}

#[test]
fn ur_25_a_receive_cut_counts_its_own_stream_s_packet_size() {
    // UR-25 (S18): the ready gap after a cut counts the cut's stream's packet size, not
    // stream 0's (which has no streamer and answers 0).
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut constraints = rx_two();
    constraints[2] = ("radio.rx.s0.sample_rate_hz", Scalar::try_from(1e6).unwrap());
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig { rx_packet: Some(4_000), ..FakeConfig::default() }, &constraints, vec![r1], layout_selector());
    direct.settle(Wall::from_millis(100));
    assert_eq!((direct.device.rx_packet_samples(0), direct.device.rx_packet_samples(1)), (0, 4_000), "stream 0 has no streamer, stream 1's packet is 4000");
    direct.update("radio.rx.s1.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(direct.now() + ms(300)));
    direct.settle(Wall::from_millis(700));
    let instance = direct.finish();
    let plan = plan_row(&section_of(&instance, "timing"));
    let rx = plan["rx"].as_array().unwrap();
    assert_eq!(rx.len(), 2, "{plan}");
    let gap = rx[1]["origin"].as_i64().unwrap() - rx[0]["end"].as_i64().unwrap();
    assert!((11_400_000..11_400_400).contains(&gap), "4000 samples at 200 ticks + 3 ms + 50 ms, rounded to the lattice: {gap} in {plan}");
    let rows = &section_of(&instance, "applied");
    assert_eq!(rows.as_array().unwrap().iter().filter(|row| row["key"] == "radio.rx.s1.sample_rate_hz").count(), 2, "{rows}");
}

#[test]
fn ur_26_a_start_rx_of_a_channel_less_linked_stream_starts_nothing() {
    // UR-26 (R21): `start_rx` of a linked stream that holds no channel is accepted and
    // carries out nothing; the channel joining later starts it.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
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
    direct.update("radio.rx.c1.stream", Value::from(1), Cold, None);
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_open 1 [1]").len(), 1, "{log:?}");
    assert_eq!(calls(&direct.device, "rx_start 1").len(), 1, "{log:?}");
    let _ = direct.finish();
}

#[test]
fn ur_25_a_transmit_rate_change_of_a_later_stream_cuts_that_stream_alone() {
    // UR-25 (T1): a cold change of stream 1's transmit rate cuts and re-clocks stream 1
    // alone; stream 0's burst and clock go on.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
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
    let sent0 = calls(&direct.device, "tx_send 0").len();
    let opens0 = calls(&direct.device, "tx_open 0").len();
    direct.update("radio.tx.s1.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(direct.now() + ms(50)));
    direct.settle(Wall::from_millis(300));
    let records = direct.clocks.sample_clock_records();
    let s0_now: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/tx/s0").collect();
    assert_eq!(s0_now.len(), 1, "stream 0 keeps its one clock: {records:?}");
    assert!(s0_now[0].ended_at.is_none(), "{records:?}");
    let s1_now: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/tx/s1").collect();
    assert_eq!(s1_now.len(), 2, "stream 1's clock is replaced: {records:?}");
    assert!(s1_now[0].ended_at.is_some() && s1_now[1].ended_at.is_none(), "{records:?}");
    assert!(calls(&direct.device, "tx_send 0").len() > sent0, "stream 0's repeat goes on: {:?}", direct.device.calls());
    assert_eq!(calls(&direct.device, "tx_open 0").len(), opens0, "stream 0's streamer is untouched: {:?}", direct.device.calls());
    assert_eq!(calls(&direct.device, "tx_open 1").len(), 1, "the same set is configured without a reopen: {:?}", direct.device.calls());
    assert!(direct.device.calls().iter().any(|call| call.starts_with("apply tx 1 ") && call.contains("rate=2000000") && call.ends_with("at=-")), "{:?}", direct.device.calls());
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
    let stopped = calls(&direct.device, "tx_send 1").len();
    let sent0 = calls(&direct.device, "tx_send 0").len();
    direct.settle(Wall::from_millis(100));
    assert_eq!(calls(&direct.device, "tx_send 1").len(), stopped, "stream 1's burst ended: {:?}", direct.device.calls());
    assert!(calls(&direct.device, "tx_send 0").len() > sent0, "stream 0's burst goes on: {:?}", direct.device.calls());
    let s1_now = clock_of(&direct, "usrp/tx/s1");
    assert!(s1_now.ended_at.is_none(), "the clock stays: {s1_now:?}");
    burst_in(&direct, "usrp/tx/s1", &s1_now, &waveform, false, ms(40));
    direct.settle(Wall::from_millis(100));
    assert!(calls(&direct.device, "tx_send 1").len() > stopped, "a later burst plays on the kept clock: {:?}", direct.device.calls());
    let _ = direct.finish();
}

#[test]
fn ur_26_a_stopped_later_stream_waits_for_start_rx() {
    // UR-26 (O1): a `Stop`ped stream is off: a membership change does not start it; only
    // `start_rx` does.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(FakeConfig::default(), &rx_two(), vec![r0, r1], layout_selector());
    direct.settle(Wall::from_millis(30));
    direct.stop_at("usrp/rx/s1");
    direct.settle(Wall::from_millis(120));
    let starts = calls(&direct.device, "rx_start 1").len();
    direct.update("radio.rx.s1.sample_rate_hz", Value::num(2e6).unwrap(), Cold, None);
    direct.settle(Wall::from_millis(300));
    assert_eq!(calls(&direct.device, "rx_start 1").len(), starts, "an off stream is not started by a cold change: {:?}", direct.device.calls());
    direct.start_rx_at("usrp/rx/s1", None);
    direct.settle(Wall::from_millis(200));
    assert_eq!(calls(&direct.device, "rx_start 1").len(), starts + 1, "`start_rx` starts it: {:?}", direct.device.calls());
    let records = direct.clocks.sample_clock_records();
    assert_eq!(records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").count(), 2, "{records:?}");
    let _ = direct.finish();
}

#[test]
fn ur_25_a_successful_change_clears_a_later_stream_s_halt() {
    // UR-25 (O2): a refusal halts stream 1 alone; a later change that succeeds starts it
    // again, as the timeline's halted mark says.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let (r0, _l0) = attached_reading("rx0");
    let (r1, _l1) = attached_reading("rx1");
    let mut direct = Direct::build(
        FakeConfig { faults: vec![FakeFault::WrongRate { claimed: 4e6, applied: 3.8e6, nth: 0 }], ..FakeConfig::default() },
        &rx_two(),
        vec![r0, r1],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(30));
    direct.update("radio.rx.s1.sample_rate_hz", Value::num(4e6).unwrap(), Cold, None);
    direct.settle(Wall::from_millis(300));
    assert_eq!(direct.of("radio.COMMAND_REJECTED").len(), 1, "{:?}", direct.device.calls());
    let starts = calls(&direct.device, "rx_start 1").len();
    direct.update("radio.rx.s1.sample_rate_hz", Value::num(1e6).unwrap(), Cold, None);
    direct.settle(Wall::from_millis(300));
    let log = direct.device.calls();
    assert_eq!(calls(&direct.device, "rx_start 1").len(), starts + 1, "the halt is cleared: {log:?}");
    let records = direct.clocks.sample_clock_records();
    let s1: Vec<_> = records.iter().filter(|record| record.stream.path() == "usrp/rx/s1").collect();
    assert_eq!(s1.len(), 2, "{records:?}");
    assert!(s1.iter().any(|record| record.ended_at.is_some()) && s1.iter().any(|record| record.ended_at.is_none()), "{records:?}");
    use ezsdr_kernel::module_api::UpdateClass::HardwareTimed;
    direct.update("radio.rx.c1.gain_db", Value::num(3.0).unwrap(), HardwareTimed, Some(direct.now() + ms(20)));
    direct.settle(Wall::from_millis(60));
    let instance = direct.finish();
    let rows = &section_of(&instance, "applied");
    assert!(rows.as_array().unwrap().iter().any(|row| row["key"] == "radio.rx.c1.gain_db" && row["issued"] == true && row["note"].is_null()), "the cleared stream is live again: {rows}");
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
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, d: ezsdr_radio_uhd::Dir) -> usize { self.inner.channels(d) }
    fn front_end(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.front_end(d, c) }
    fn serial(&self) -> Result<String, ezsdr_radio_uhd::DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, d: ezsdr_radio_uhd::Dir, m: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_subdev_spec(d, m) }
    fn rates(&self, d: ezsdr_radio_uhd::Dir, c: usize) -> Result<Vec<(f64, f64, f64)>, ezsdr_radio_uhd::DeviceError> { self.inner.rates(d, c) }
    fn set_sources(&self, c: &str, t: &str) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_sources(c, t) }
    fn set_time_zero(&self, p: bool) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.set_time_zero(p) }
    fn time_now(&self) -> Result<i64, ezsdr_radio_uhd::DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, ezsdr_radio_uhd::DeviceError> { self.inner.ref_locked() }
    fn apply(&self, d: ezsdr_radio_uhd::Dir, c: usize, s: &ezsdr_radio_uhd::Settings, at: Option<i64>) -> Result<ezsdr_radio_uhd::Applied, ezsdr_radio_uhd::DeviceError> { self.inner.apply(d, c, s, at) }
    fn rx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(s, c) }
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
    fn rx_packet_samples(&self, s: usize) -> usize { self.inner.rx_packet_samples(s) }
    fn rx_stop(&self, s: usize, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(s, at) }
    fn rx_recv(&self, s: usize, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv {
        if s == 0 {
            self.recv0.store(true, std::sync::atomic::Ordering::SeqCst);
            self.recv0_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        self.inner.rx_recv(s, n, t)
    }
    fn tx_open(&self, s: usize, c: &[usize]) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(s, c) }
    fn tx_send(&self, s: usize, b: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.inner.tx_send(s, b, at, sob, eob, t) }
    fn tx_async(&self, s: usize, t: Wall) -> Result<Option<ezsdr_radio_uhd::TxReport>, ezsdr_radio_uhd::DeviceError> { self.inner.tx_async(s, t) }
    fn rx_close(&self, s: usize) { self.inner.rx_close(s) }
    fn tx_close(&self, s: usize) { self.inner.tx_close(s) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn host(&self) -> ezsdr_radio_uhd::HostClock { self.inner.host() }
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
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut constraints = rx_two();
    constraints.extend(tx_two());
    let mut direct = Direct::build(
        FakeConfig::default(),
        &constraints,
        vec![attached_on("rx0", ezsdr_kernel::stream::BackPressure::DropOldest), attached_on("rx1", ezsdr_kernel::stream::BackPressure::DropOldest)],
        layout_selector(),
    );
    direct.settle(Wall::from_millis(50));
    // Stream 0's second segment in each direction, so that a row's `sequence` is a command's.
    direct.update_at("usrp/rx/s0", "radio.rx.s0.sample_rate_hz", Value::num(1e6).unwrap(), Cold, Some(direct.now() + ms(20)));
    direct.update_at("usrp/tx/s0", "radio.tx.s0.sample_rate_hz", Value::num(1e6).unwrap(), Cold, Some(direct.now() + ms(20)));
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
    assert_eq!(rx0.len(), 2, "the change's segment and the first: {plan}");
    assert!(rx0[1]["sequence"].is_u64(), "the change's segment names it: {plan}");
    let tx0 = plan_stream(&plan, "tx", 0);
    assert_eq!(tx0.len(), 2, "the change's segment and the first: {plan}");
    assert!(tx0[1]["sequence"].is_u64(), "the change's segment names it: {plan}");
    let records = clocks.sample_clock_records();
    for k in [0, 1] {
        let path = format!("usrp/tx/s{k}");
        let open: Vec<_> = records.iter().filter(|record| record.stream.path() == path.as_str()).collect();
        assert_eq!(open.len(), if k == 0 { 2 } else { 1 }, "stream {k}'s clocks: {records:?}");
        assert!(open.last().unwrap().ended_at.is_none(), "a stop keeps stream {k}'s transmit clock: {records:?}");
    }
}
