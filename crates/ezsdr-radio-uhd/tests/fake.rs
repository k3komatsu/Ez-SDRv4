//! Spec 18 §6: every UR rule through the real coordinator on `FakeDevice`, with an
//! Assembly built as the server's catalogue builds it (the capture Sink, the host
//! Link, `DeviceAuthority` and `UhdRadio` on one device), unless a test says "unit".
//! The device's clock is the manual one (plan 30 §4.4), so a test's waits and its
//! timing assertions are that clock's; see tests/README.md for the rules and for
//! timing-precondition diagnostics.

mod common;

use ezsdr_kernel::spec::Scalar;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding};
use ezsdr_kernel::coordinator::{RunHandle, RunHandleError, start_spec_run};
use ezsdr_kernel::event::{Action, ActionId, Event, EventCollector, EventKind, EventSource};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ResourceId, RunId};
use ezsdr_kernel::manifest::Manifest;
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
    fed.feed = serde_json::from_value(json!({ "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 4 })).unwrap();
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
    for dir in ["rx", "tx"] {
        assert_eq!(capability(&format!("radio.{dir}.frequency_hz")), range(Scalar::try_from(1e7).unwrap(), Scalar::try_from(8.4e9).unwrap()));
        assert_eq!(capability(&format!("radio.{dir}.channels")), range(Scalar::from(0), Scalar::from(1)));
    }
    assert_eq!(capability("radio.phase_behavior_on_retune"), ezsdr_kernel::spec::CapabilityValue::One { value: Scalar::from("random_unless_timed_tune".to_owned()) });
    assert_eq!(capability("radio.rx.coherent"), ezsdr_kernel::spec::CapabilityValue::One { value: Scalar::Bool(true) });
    assert_eq!(radio.instance().profile.as_ref().unwrap().name, "x3xx");
    let dir = TempDir::new();
    let mut doc = profile(&dir, selector, json!({}), true);
    doc["bindings"]["radio"]["profile"] = x3xx();
    let mut run = session(&doc, fake(one_obx()));
    past_t0(&mut run, ms(1));
    let manifest = run.finish();
    let applied = section(&manifest, "applied").as_array().unwrap().iter().find(|r| r["key"] == "radio.rx.frequency_hz").cloned().unwrap();
    assert_eq!(applied["claimed"], 1e9);
}

#[test]
fn ur_05_the_profile_must_be_the_device_s_front_ends() {
    let refuse = |selector: Json, config: FakeConfig| UhdRadio::from_binding(&binding(selector, Some(x3xx())), fake(config)).err().unwrap().message;
    assert!(UhdRadio::from_binding(&binding(one_board("cbx"), Some(x3xx())), fake(one_cbx())).is_ok());
    let boards = |front_ends: Vec<&'static str>| FakeConfig { front_ends, ..FakeConfig::default() };
    let two = |board: &str| json!({ "args": ARGS, "rx": [format!("0:A:{board}:0"), format!("0:B:{board}:0")], "tx": [format!("0:A:{board}:0"), format!("0:B:{board}:0")] });
    assert_eq!(refuse(one_board("ubx"), one_cbx()), "UR-5: profile x3xx needs UBX front ends; rx channel 0 is `CBX-120 RX`");
    assert_eq!(refuse(two("ubx"), boards(vec!["UBX", "CBX"])), "UR-5: profile x3xx needs UBX front ends; rx channel 1 is `CBX RX`");
    assert_eq!(refuse(two("ubx"), boards(vec!["UBX"])), "UR-5: profile x3xx needs 2 rx channels; the device has 1");
    assert_eq!(refuse(one_board("cbx"), FakeConfig::default()), "UR-5: profile x3xx needs CBX front ends; rx channel 0 is `UBX RX`");
    assert_eq!(refuse(one_board("cbx"), boards(vec![])), "UR-5: profile x3xx needs 1 rx channels; the device has 0");
    // The transmit side is checked the same way, and so is a device whose two directions
    // name boards of different kinds.
    assert!(UhdRadio::from_binding(&binding(one_board("obx"), Some(x3xx())), fake(one_obx())).is_ok());
    assert!(UhdRadio::from_binding(&binding(two("ubx"), Some(x3xx())), fake(boards(vec!["UBX", "UBX"]))).is_ok());
    assert_eq!(refuse(one_board("ubx"), one_obx()), "UR-5: profile x3xx needs UBX front ends; rx channel 0 is `OBX RX`");
    assert_eq!(refuse(one_board("obx"), FakeConfig::default()), "UR-5: profile x3xx needs OBX front ends; rx channel 0 is `UBX RX`");
    assert_eq!(refuse(one_board("obx"), one_cbx()), "UR-5: profile x3xx needs OBX front ends; rx channel 0 is `CBX-120 RX`");
    let mixed = json!({ "args": ARGS, "rx": ["0:A:ubx:0"], "tx": ["0:A:obx:0"] });
    assert!(refuse(mixed, FakeConfig::default()).contains("one device carries one kind"));
}

#[test]
fn ur_09_the_cbx_is_one_channel_from_1_2_ghz_defaulting_to_2_45_ghz() {
    let selector = one_board("cbx");
    let radio = UhdRadio::from_binding(&binding(selector.clone(), Some(x3xx())), fake(one_cbx())).unwrap();
    let capability = |name: &str| radio.instance().tree.capabilities[&Key::parse(name).unwrap()].clone();
    let range = |min, max| ezsdr_kernel::spec::CapabilityValue::Range { min, max };
    for dir in ["rx", "tx"] {
        assert_eq!(capability(&format!("radio.{dir}.frequency_hz")), range(Scalar::try_from(1.2e9).unwrap(), Scalar::try_from(6e9).unwrap()));
        assert_eq!(capability(&format!("radio.{dir}.channels")), range(Scalar::from(0), Scalar::from(1)));
    }
    // A CBX has no timed LO phase synchronisation: its phase value says so, and its
    // channels are aligned but not coherent (UR-9, RM-9, design 33 D13).
    assert_eq!(capability("radio.phase_behavior_on_retune"), ezsdr_kernel::spec::CapabilityValue::One { value: Scalar::from("random".to_owned()) });
    assert_eq!(capability("radio.rx.coherent"), ezsdr_kernel::spec::CapabilityValue::One { value: Scalar::Bool(false) });
    // A Session that sets no frequency tunes to the default, inside the CBX's range.
    let dir = TempDir::new();
    let mut doc = profile(&dir, selector, json!({}), true);
    doc["bindings"]["radio"]["profile"] = x3xx();
    let mut run = session(&doc, fake(one_cbx()));
    past_t0(&mut run, ms(1));
    let manifest = run.finish();
    let applied = section(&manifest, "applied").as_array().unwrap().iter().find(|r| r["key"] == "radio.rx.frequency_hz").cloned().unwrap();
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
    let (authority, device, root) = authority(FakeConfig { drift_ppm: 100.0, ..FakeConfig::default() }, "internal");
    let time = authority.time();
    // The device's own clock, so that `uhd-clock` re-anchors over the same 2 s of it (plan 30 §4.5).
    device.host().sleep(Wall::from_secs(2));
    let ours = time.now(root).unwrap().ticks_in(root).unwrap();
    let theirs = device.time_now().unwrap();
    assert!((ours - theirs).abs() < 20_000, "{} ticks apart", ours - theirs);
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
    assert_eq!(capability("radio.tx.repeat_max_samples"), one(Scalar::from(4_294_967_295)));
    assert_eq!(capability("radio.tx.repeat_align_samples"), one(Scalar::from(1)));
    assert_eq!(capability("radio.rx.block_len"), one(Scalar::from(2_000)));
    assert_eq!(capability("radio.perf.wire_bytes_per_sample"), one(Scalar::from(4)));
    assert_eq!(capability("radio.tx.path_delay_samples"), one(Scalar::from(45)));
    let envelope = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.envelope").unwrap()];
    assert_eq!(envelope["profile"]["name"], "x3xx");
    assert_eq!(envelope["timing"]["min_timed_command_lead_ns"], 5_000_000);
    assert_eq!(envelope["performance"]["rx_bytes_per_s"], 1_000_000_000);
    assert_eq!(instance.sections[&Namespace::parse("ezsdr.radio.uhd.device").unwrap()]["fake"], true);
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
    let report = radio.coerce(&request(&[("radio.rx.sample_rate_hz", Scalar::try_from(19.5e6).unwrap())])).unwrap();
    assert_eq!(report.applied[&Key::parse("radio.rx.sample_rate_hz").unwrap()], Value::num(20e6).unwrap());
    assert_eq!(report.coercions.len(), 1);
    assert!(!radio.coerce(&request(&[("radio.rx.frequency_hz", Scalar::try_from(7e9).unwrap())])).unwrap().rejected.is_empty());
    assert!(!radio.coerce(&request(&[("radio.rx.channels", Scalar::from(4))])).unwrap().rejected.is_empty());
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
    let pairs: Vec<_> = ["usrp", "usrp/rx", "usrp/tx"].iter().flat_map(|s| kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k))).collect();
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
    let decl = DataLinkDecl {
        id: ezsdr_kernel::id::DataLinkId::local(0),
        from: ezsdr_kernel::contract::PortRef { component: Ident::parse("radio").unwrap(), port: Ident::parse("rx").unwrap() },
        to: ezsdr_kernel::contract::PortRef { component: Ident::parse("rec").unwrap(), port: Ident::parse("in").unwrap() },
        contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        policy,
        capacity: 4,
    };
    let link = ezsdr_link_host::HostLinkModule::new().create(&decl).unwrap();
    AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse("rx").unwrap(), Endpoint::StreamOut(link))
}

/// A queue the test pushes Actions into, as the coordinator dispatches them.
#[derive(Default)]
struct Pushed(Mutex<std::collections::VecDeque<Action>>, std::sync::atomic::AtomicU64);
impl ActionReceiver for Pushed {
    // The coordinator's resolution (KC-23): every test Action is the device's; ids count
    // from 1, as the Kernel's do (KC-24).
    fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
        let id = || ActionId(self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1);
        self.0.lock().unwrap().pop_front().map(|action| ezsdr_kernel::module_api::Dispatched::new(id(), action, Some(ResourceId::parse("usrp").unwrap())))
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

    /// With selector keys beside `args` (UR-5).
    fn build(config: FakeConfig, constraints: &[(&str, Scalar)], links: Vec<AttachedPort>, selector: Json) -> Direct {
        let device = fake(config);
        let clocks = Arc::new(ClockRegistry::new());
        let authority = DeviceAuthority::new(device.clone(), clocks.clone(), "internal", "internal", ARGS).unwrap();
        let (time, root) = (authority.time(), authority.root());
        let mut registry = ModuleRegistry::new();
        let (mut checks, mut kinds) = (AdmissionCheckRegistry::new(), EventKindRegistry::with_kernel_kinds());
        ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
        let pairs: Vec<_> = ["usrp", "usrp/rx", "usrp/tx"].iter().flat_map(|s| kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k))).collect();
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
        let mut keys = json!({ "args": ARGS });
        keys.as_object_mut().unwrap().extend(selector.as_object().cloned().unwrap_or_default());
        let mut radio = UhdRadio::from_binding(&binding(keys, Some(x3xx())), device.clone()).unwrap();
        radio.prepare(&fragment, ctx).unwrap();
        radio.arm().unwrap();
        let t0 = time.now(root).unwrap().ticks_in(root).unwrap() + ms(2_000) + ms(1);
        radio.start(Some(TimePoint::new(root, t0))).unwrap();
        time.wait_until(TimePoint::new(root, t0 + ms(1))).unwrap();
        Direct { radio, device, queue, events, time, root, clocks, delivered: Vec::new(), _authority: authority }
    }

    fn now(&self) -> i64 {
        self.time.now(self.root).unwrap().ticks_in(self.root).unwrap()
    }

    fn push(&self, action: Action) {
        self.queue.0.lock().unwrap().push_back(action);
    }

    fn update(&self, key: &str, value: Value, class: ezsdr_kernel::module_api::UpdateClass, at: Option<i64>) {
        self.push(Action::UpdateParameter {
            target: resource("radio"),
            key: Key::parse(key).unwrap(),
            value,
            class,
            at: at.map(|t| ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(self.root, t))),
        });
    }

    fn settle(&mut self, wall: Wall) -> Vec<Event> {
        // The test thread is a non-participant; it waits on the device's host clock so that
        // the device's time advances while its threads are idle (plan 30 §4.5).
        self.device.host().sleep(wall);
        self.delivered.extend(self.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)));
        self.delivered.clone()
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
    let mut radio = provider();
    let three = direct_prepare(&mut radio, device, ExecutionClass::HardwareInLoop, Vec::new(), &[("radio.rx.channels", Scalar::from(3))]).unwrap_err();
    assert_eq!(three.kind, ModuleErrorKind::Rejected);
}

#[test]
fn ur_12_a_tx_only_layout_declares_no_receive_clock() {
    // UR-12, design 33 D5: the Module configures a direction, and declares its stream's
    // SampleClock, only when its channel count is > 0. A `tx`-only layout leaves RM-5's
    // receive channel default 0 (the description's), so no receive channel is configured and
    // `usrp/rx` is never declared.
    let direct = Direct::build(FakeConfig::default(), &[("radio.tx.channels", Scalar::from(1))], Vec::new(), json!({ "rx": [], "tx": ["0:A:ubx:0"] }));
    let clocks: Vec<String> = direct.clocks.sample_clock_records().iter().map(|record| record.stream.path().to_owned()).collect();
    assert_eq!(clocks, ["usrp/tx"], "a layout with no receive front end declares no receive clock");
    let instance = direct.finish();
    let applied = instance.sections[&Namespace::parse("ezsdr.radio.uhd.applied").unwrap()].clone();
    assert!(applied.as_array().unwrap().iter().all(|row| !row["key"].as_str().unwrap_or("").starts_with("radio.rx")), "no receive channel is configured: {applied}");
}

#[test]
fn ur_12_the_applied_values_are_recorded() {
    let dir = TempDir::new();
    let mut spec = receive_spec(1, 1e6, 2.4e9, Some(2_000));
    spec["resources"]["radio"]["requires"]["radio.rx.gain_db"] = json!({ "kind": "eq", "value": 10.3 });
    let manifest = captured(spec_run(&spec, &profile(&dir, json!({}), json!({}), false), fake(FakeConfig::default()), BTreeMap::new()));
    let applied = section(&manifest, "applied").as_array().unwrap().clone();
    let row = |key: &str| applied.iter().find(|r| r["key"] == key).unwrap_or_else(|| panic!("{key}: {applied:?}")).clone();
    assert_eq!(row("radio.rx.frequency_hz")["claimed"], 2.4e9);
    assert_eq!(row("radio.rx.frequency_hz")["read_back"], 2.4e9);
    assert_eq!(row("radio.rx.gain_db")["claimed"], 10.5);
    assert_eq!(row("radio.rx.gain_db")["read_back"], 10.5);
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
    let selection = ezsdr_radio::x3xx::Selection::of(
        ezsdr_radio::x3xx::Board::Ubx,
        &[ezsdr_radio::x3xx::Slot::A, ezsdr_radio::x3xx::Slot::B],
        &[ezsdr_radio::x3xx::Slot::A, ezsdr_radio::x3xx::Slot::B],
    );
    let description = ezsdr_radio_uhd::profile::description(&selection, 2_000);
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
    let manifest = captured(spec_run(&receive_spec(1, 200e6 / 3.0, 1e9, Some(1_000)), &profile(&dir, json!({}), json!({}), false), fake(FakeConfig::default()), BTreeMap::new()));
    assert_eq!(capture_of(&manifest, "rec").size_bytes, 8_000, "{}", failure(&manifest));
    assert_eq!(manifest.clocks.sample_clocks[0].root_ticks_per_tick.num(), 3);
    let dir = TempDir::new();
    let mut run = session(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()));
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::num(200e6 / 7.0).unwrap()), None).unwrap()));
    wait(&mut run, ms(200));
    let manifest = run.finish();
    assert!(rejections(&manifest).is_empty(), "{:?}", rejections(&manifest));
    let new = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).nth(1).expect("the new receive clock");
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
    let entry = run.submit(set("radio.tx.channels", Value::from(1)), None).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    assert!(!calls(&device, "apply tx").is_empty(), "{:?}", device.calls());
    let tx = ResourceId::parse("usrp/tx").unwrap();
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
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::num(2e6).unwrap()), None).unwrap()));
    assert!(admitted(&run.submit(set("radio.rx.gain_db", Value::num(3.0).unwrap()), None).unwrap()));
    // Command ordering is proved with a controlled clock by the library's
    // ur_14_booking_does_not_wait_for_a_future_cold_switch. A wall-time upper
    // bound here would also measure whether the host scheduled these threads.
    wait(&mut run, ms(200));
    let manifest = run.finish();
    let clocks: Vec<_> = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).collect();
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
    assert!(section(&manifest, "rejected").as_array().unwrap().iter().any(|r| r["thread"] == "uhd-rx"));
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
    assert!(section(&manifest, "rejected").as_array().unwrap().iter().any(|r| r["thread"] == "uhd-tx"), "{:?}", section(&manifest, "rejected"));
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
    assert!(admitted(&run.submit(set("radio.tx.channels", Value::from(1)), None).unwrap()));
    past_tx_origin(&mut run);
    (run, device, dir)
}

/// The same transmit Session on any `Device` (a test wrapper, plan 30 §7).
fn tx_session_on(device: Arc<dyn Device>) -> (RunHandle, TempDir) {
    let dir = TempDir::new();
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device);
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.tx.channels", Value::from(1)), None).unwrap()));
    past_tx_origin(&mut run);
    (run, dir)
}

/// The transmit sample `lead` root ticks from now, on the running transmit clock.
fn tx_at(run: &RunHandle, lead: i64) -> TimePoint {
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx").unwrap() && r.ended_at.is_none()).unwrap();
    let n = clock.root_ticks_per_tick.num() as i64;
    TimePoint::new(clock.domain, (run.now().ticks_in(clock.root).unwrap() + lead - clock.origin.ticks_in(clock.root).unwrap() + n - 1).div_euclid(n))
}

fn send_at(run: &mut RunHandle, verb_name: &str, at: Option<TimePoint>, samples: &[(f32, f32)]) -> ezsdr_kernel::session::LogEntry {
    let (bytes, _) = waveform_of(samples);
    run.submit(verb(verb_name, resource("radio/tx"), at, &[]), Some(&bytes)).unwrap()
}

fn send(run: &mut RunHandle, verb_name: &str, lead: Option<i64>, samples: &[(f32, f32)]) -> ezsdr_kernel::session::LogEntry {
    let at = lead.map(|lead| tx_at(run, lead));
    send_at(run, verb_name, at, samples)
}

fn time_errors(manifest: &Manifest) -> Vec<TimeErrorPayload> {
    events_of(manifest, "radio.TIME_ERROR").iter().map(|e| serde_json::from_value(e.payload.clone()).unwrap()).collect()
}

fn rejections(manifest: &Manifest) -> Vec<String> {
    section(manifest, "rejected").as_array().unwrap().iter().filter_map(|r| r["reason"].as_str().map(str::to_owned)).collect()
}

#[test]
fn ur_21_txburst_refusals() {
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    // Not a multiple of 8 · channels.
    let odd = vec![0u8; 12];
    let _ = run.submit(verb("send", resource("radio/tx"), None, &[]), Some(&odd)).unwrap();
    // A non-finite component.
    let _ = send(&mut run, "send", Some(ms(20)), &[(f32::NAN, 0.0)]);
    // A start equal to a held burst's.
    let at = tx_at(&run, ms(500));
    let _ = send_at(&mut run, "send", Some(at), &tone(10));
    let _ = send_at(&mut run, "send", Some(at), &tone(10));
    wait(&mut run, ms(20));
    let manifest = run.finish();
    let reasons = rejections(&manifest);
    for want in ["UR-21: the waveform size", "UR-21: a waveform sample is not finite", "UR-21: a held burst already has this start"] {
        assert!(reasons.iter().any(|r| r.starts_with(want)), "{want}: {reasons:?}");
    }
    let rejected = events_of(&manifest, "radio.COMMAND_REJECTED");
    assert!(rejected.len() >= 3);
    // MA-14: a burst refused at receipt names the Action it answers.
    assert!(rejected.iter().all(|e| e.payload["action"].is_u64()), "{rejected:?}");
}

#[test]
fn ur_21_a_burst_before_start_up_is_refused() {
    let dir = TempDir::new();
    let mut run = session(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()));
    assert!(admitted(&run.submit(set("radio.tx.channels", Value::from(1)), None).unwrap()));
    // On the new clock, after its origin, the start lead after the change (RM-25), and
    // before the device's start-up.
    let _ = send(&mut run, "send", Some(ms(60)), &tone(10));
    let manifest = run.finish();
    assert!(rejections(&manifest).iter().any(|r| r.starts_with("UR-21: the burst begins before the device's start-up")), "{:?}", rejections(&manifest));
}

#[test]
fn ur_21_late_policies_on_the_device_lead() {
    // The manual clock (plan 30 §4.4): the booking's verdicts are deterministic device-time
    // facts — the 1 ms `send` (`drop_and_flag`) is dropped and the 0.5 ms `start_repeat`
    // (`send_asap_and_flag`) is moved, both at the receipt site (`control.rs`'s `book_burst`
    // applies the 2 ms device lead). Whether the moved repeat is moved *again* at uhd-tx's
    // first hand-over depends on real time passing between the booking and the hand-over
    // (see the unit test below), which the manual clock does not promise — on a busy
    // Provider it is frozen away, so the verdict can be absent. That re-decision is pinned
    // deterministically by `ur_21_first_dispatch_rechecks_all_late_policies` (the re-spelled
    // row U15) and is not asserted here.
    let (mut run, _device, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "send", Some(ms(1)), &tone(10));
    let _ = send(&mut run, "start_repeat", Some(ms(1) / 2), &tone(10));
    wait(&mut run, ms(50));
    let _ = run.submit(SessionAction::Stop { target: Some(resource("radio/tx")) }, None);
    wait(&mut run, ms(20));
    // On time with a margin that survives the test binary's parallel load (Review M,
    // P1-B); the device lead's own boundary is the bench's to measure (B8).
    let _ = send(&mut run, "send", Some(ms(20)), &tone(10));
    wait(&mut run, ms(60));
    let manifest = run.finish();
    let errors = time_errors(&manifest);
    let outcomes: Vec<_> = errors.iter().map(|p| p.outcome).collect();
    // The booking's own verdicts: Drop for the 1 ms burst, SendAsap for the moved repeat.
    assert_eq!(&outcomes[..2], [TimeErrorOutcome::Drop, TimeErrorOutcome::SendAsap], "{errors:#?}");
    // Only the repeat's optional hand-over re-decision may follow, and the 20 ms burst is on
    // time: `send` is `drop_and_flag`, so a late third burst would show here as a `Drop`.
    assert!(outcomes[2..].iter().all(|o| *o == TimeErrorOutcome::SendAsap), "{errors:#?}");
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
fn ur_21_a_burst_inside_the_in_flight_window_is_late() {
    for _ in 0..3 {
        let (mut run, device, _dir) = tx_session(FakeConfig::default());
        let _ = send(&mut run, "start_repeat", Some(ms(20)), &tone(1_000));
        wait(&mut run, ms(60));
        let _ = send(&mut run, "start_repeat", Some(ms(5)), &tone(100));
        wait(&mut run, ms(30));
        let _ = send(&mut run, "send", Some(ms(5)), &tone(100));
        wait(&mut run, ms(30));
        let manifest = run.finish();
        let outcomes: Vec<_> = time_errors(&manifest).iter().map(|p| p.outcome).collect();
        assert_eq!(outcomes, [TimeErrorOutcome::SendAsap, TimeErrorOutcome::Drop], "{outcomes:?}");
        assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError"), "{:?}", device.calls());
        assert_eq!(device.unended_bursts(), 0, "a burst started inside one without end-of-burst: {:?}", device.calls());
    }
}

#[test]
fn ur_21_a_burst_before_its_clock_s_origin_is_late() {
    // An explicit `at` 5 ms ahead lies before the new transmit clock's origin, a start lead
    // after the cold change (RM-14, RM-15).
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    assert!(admitted(&run.submit(set("radio.tx.sample_rate_hz", Value::num(2e6).unwrap()), None).unwrap()));
    let entry = send(&mut run, "start_repeat", Some(ms(5)), &tone(200));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(300));
    let manifest = run.finish();
    let errors = time_errors(&manifest);
    assert_eq!(errors.iter().map(|p| p.outcome).collect::<Vec<_>>(), [TimeErrorOutcome::SendAsap]);
    let e2 = tx_clock_origin(&manifest, 1);
    let record = &bursts(&manifest)[0];
    assert_eq!(record.target.ticks_in(record.target.domain()).unwrap(), 0, "moved to the new clock's origin {e2}");
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError"), "{:?}", device.calls());
}

#[test]
fn ur_21_an_untimed_burst_after_a_cold_change_is_on_time_at_its_clock_s_origin() {
    // RS-19: with no `at`, the burst is admitted at the new transmit clock's origin, which
    // uhd-control registered before the Kernel's KC-21a wait returned (UR-25), so it is on time.
    let (mut run, _device, _dir) = tx_session(FakeConfig::default());
    assert!(admitted(&run.submit(set("radio.tx.sample_rate_hz", Value::num(2e6).unwrap()), None).unwrap()));
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
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "send", Some(ms(20)), &tone(3_000));
    wait(&mut run, ms(60));
    let manifest = run.finish();
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
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "start_repeat", Some(ms(10)), &tone(1_000));
    wait(&mut run, ms(40));
    let _ = send(&mut run, "send", Some(ms(20)), &tone(500));
    wait(&mut run, ms(60));
    let manifest = run.finish();
    let records = bursts(&manifest);
    assert_eq!(records.len(), 2, "{records:?}");
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
fn ur_23_bursts_go_out_in_target_order() {
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "send", Some(ms(80)), &tone(100));
    let _ = send(&mut run, "send", Some(ms(40)), &tone(100));
    wait(&mut run, ms(120));
    let _ = run.finish();
    let starts: Vec<i64> = calls(&device, "tx_send").iter().filter(|c| c.contains("sob=true")).map(|c| c.split("at=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap()).collect();
    assert_eq!(starts.len(), 2);
    assert!(starts[0] < starts[1], "{starts:?}");
}

#[test]
fn ur_23_a_stop_ends_the_burst_within_the_in_flight_window() {
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "start_repeat", Some(ms(10)), &tone(1_000));
    wait(&mut run, ms(60));
    let stop_at = run.now().ticks_in(run.now().domain()).unwrap();
    let _ = run.submit(SessionAction::Stop { target: Some(resource("radio/tx")) }, None);
    wait(&mut run, ms(40));
    let manifest = run.finish();
    let record = &bursts(&manifest)[0];
    assert_eq!(record.end, BurstEnd::Stop);
    let origin = tx_clock_origin(&manifest, 0);
    let end = origin + (record.target.ticks_in(record.target.domain()).unwrap() + record.samples as i64) * 200;
    assert!(end <= stop_at + ms(10) + ms(20), "ends {} ms after the stop", (end - stop_at) / ms(1));
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
        assert!(admitted(&run.submit(set("radio.rx.frequency_hz", Value::num(1e9 + f64::from(i)).unwrap()), None).unwrap()));
        wait(&mut run, ms(5));
    }
    assert_eq!(late_commands(&run.finish()), 0);
}

#[test]
fn ur_24_hardware_timed_updates() {
    use ezsdr_kernel::module_api::UpdateClass::HardwareTimed;
    let mut direct = Direct::new(FakeConfig { command_queue: 64, ..FakeConfig::default() }, &[("radio.tx.channels", Scalar::from(1))]);
    let now = direct.now();
    // 1 ms ahead is inside the 2 ms device lead: late (UC-6 as KG-8 amends it).
    direct.update("radio.rx.frequency_hz", Value::num(2.1e9).unwrap(), HardwareTimed, Some(now + ms(1)));
    direct.update("radio.tx.gain_db", Value::num(3.0).unwrap(), HardwareTimed, Some(now + ms(40)));
    direct.settle(Wall::from_millis(80));
    let late = direct.of("radio.LATE_COMMAND");
    assert_eq!(late.len(), 1, "{late:?}");
    assert_eq!(late[0].payload["key"], "radio.rx.frequency_hz");
    let applied = direct.device.calls();
    let tx = applied.iter().find(|c| c.starts_with("apply tx 0 rate=- freq=- gain=3")).unwrap();
    assert!(tx.contains(&format!("at={}", now + ms(40))), "{tx}");
    // The late one is applied where LATE_COMMAND says, a device lead after its receipt.
    let rx = applied.iter().find(|c| c.starts_with("apply rx 0") && c.contains("at=") && !c.contains("at=-")).unwrap();
    let at: i64 = rx.split("at=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
    assert_eq!(Some(at), late[0].payload["applied"]["ticks"].as_i64(), "{rx}");
    assert!(at >= now + ms(2), "{rx}");
    // 17 pending: the 17th is refused (UR-24's depth 16).
    let far = direct.now() + ms(5_000);
    for i in 0..17 {
        direct.update("radio.rx.gain_db", Value::num(f64::from(i % 30)).unwrap(), HardwareTimed, Some(far + i64::from(i)));
    }
    direct.settle(Wall::from_millis(30));
    let full = direct.of("radio.COMMAND_QUEUE_FULL");
    assert_eq!(full.len(), 1, "{full:?}");
    assert_eq!(full[0].payload, json!({ "key": "radio.rx.gain_db", "depth": 16 }));
    let _ = direct.finish();
}

#[test]
fn ur_24_a_far_future_update_does_not_delay_a_nearer_one() {
    // The far one reaches uhd-control first; the device's queue is in order (the fake
    // keeps it so), so handing it over at receipt would hold the near one behind it.
    use ezsdr_kernel::module_api::UpdateClass::HardwareTimed;
    let mut direct = Direct::new(FakeConfig::default(), &[]);
    let now = direct.now();
    direct.update("radio.rx.frequency_hz", Value::num(2.1e9).unwrap(), HardwareTimed, Some(now + ms(500)));
    direct.update("radio.rx.frequency_hz", Value::num(2.2e9).unwrap(), HardwareTimed, Some(now + ms(20)));
    direct.settle(Wall::from_millis(80));
    let applied: Vec<(i64, i64)> = direct
        .device
        .calls()
        .iter()
        .filter(|c| c.starts_with("apply rx 0 rate=- freq=") && !c.contains("at=-"))
        .map(|c| {
            let at: i64 = c.split("at=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
            let effective: i64 = c.split("effective=").nth(1).unwrap().parse().unwrap();
            (at, effective)
        })
        .collect();
    assert_eq!(applied, vec![(now + ms(20), now + ms(20))], "only the near one released, at its instant");
    assert!(direct.of("radio.LATE_COMMAND").is_empty());
    let _ = direct.finish();
}

// `ur_24_the_device_queue_order_is_the_effective_order` is a unit test in
// `src/provider/control.rs`, on the controlled clock: its case, a command earlier than one
// already released, was left to host timing here, which the manual clock no longer provides.

#[test]
fn ur_25_the_switch_applies_the_configuration_in_effect_at_e2() {
    // UR-25 (Review K, N-P2-9): a retune held for an instant before e₂ is part of the
    // configuration the switch applies (Review L, L09).
    use ezsdr_kernel::module_api::UpdateClass::{Cold, HardwareTimed};
    let mut direct = Direct::with_links(FakeConfig::default(), &[], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    let now = direct.now();
    direct.update("radio.rx.frequency_hz", Value::num(2.1e9).unwrap(), HardwareTimed, Some(now + ms(320)));
    direct.update("radio.rx.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(now + ms(300)));
    direct.settle(Wall::from_millis(450));
    let calls = direct.device.calls();
    assert!(calls.iter().any(|c| c.starts_with("apply rx 0 rate=2000000 freq=2100000000") && c.ends_with("at=-")), "{calls:?}");
    let _ = direct.finish();
}

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
        "action": { "kind": "update_parameter", "target": radio, "key": "radio.rx.frequency_hz", "value": 2.3e9 }
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
    let rows: Vec<_> = section(&manifest, "applied").as_array().unwrap().iter().filter(|r| r["key"] == "radio.rx.frequency_hz" && r["claimed"] == 2.3e9).cloned().collect();
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
                    "key": "radio.rx.frequency_hz", "value": value }
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
fn ur_25_a_stream_silent_before_e1_switches_before_e2() {
    // design-notes §11 F2: a stream that stops yielding before e₁ is ended within a block
    // of e₁, not at uhd-rx's next 100 ms receive timeout, so that the timed start at e₂
    // is on time. The fake falls silent 130 ms before e₁: 100 ms timeouts would come about
    // 30 ms before the cut and 70 ms after it, past e₂ (the cut, one receive call, the
    // allowance and the start lead: about 55 ms).
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    // Direct starts its Run ~2 s after the fake's time zero (the start-up lead).
    let silent = ms(2_300);
    let mut direct = Direct::with_links(
        FakeConfig { faults: vec![FakeFault::Silence(Wall::from_millis(2_300))], ..FakeConfig::default() },
        &[],
        vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)],
    );
    assert!(direct.now() < silent - ms(150), "the Run started late: {}", direct.now());
    direct.update("radio.rx.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(silent + ms(130)));
    let until = silent + ms(300) - direct.now();
    direct.settle(Wall::from_nanos((until * 5) as u64));
    let late = direct.of("radio.LATE_COMMAND");
    let direct_calls = direct.device.calls();
    // The bounded wait never goes under 1 ms (UHD truncates the timeout to whole ms).
    assert!(direct.device.min_recv_timeout() >= Wall::from_millis(1), "{:?}", direct.device.min_recv_timeout());
    let instance = direct.finish();
    let timing = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.timing").unwrap()];
    let switch = timing.as_array().unwrap().iter().find(|r| r["what"] == "rx_segment").cloned().unwrap_or_else(|| panic!("no switch: {timing}"));
    let e2 = switch["origin"].as_i64().unwrap();
    assert!(switch["at"].as_i64().unwrap() < e2, "switched after e₂: {switch}");
    // Stopped untimed and recorded, before the new stream's start.
    let calls = direct_calls;
    let stop = calls.iter().position(|c| c == "rx_stop 0 now").unwrap_or_else(|| panic!("{calls:?}"));
    let start = calls.iter().position(|c| *c == format!("rx_start 0 {e2}")).unwrap_or_else(|| panic!("{calls:?}"));
    assert!(stop < start, "{calls:?}");
    let applied = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.applied").unwrap()];
    assert!(applied.as_array().unwrap().iter().any(|r| r["key"] == "rx_stop"), "{applied}");
    assert!(late.is_empty(), "{late:?}");
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
fn ur_25_rx_channels_from_zero_starts_at_its_instant() {
    // RM-25: a stream enabled from 0 channels starts at or after its effective instant.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::with_links(FakeConfig::default(), &[("radio.rx.channels", Scalar::from(0))], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    let at = direct.now() + ms(300);
    direct.update("radio.rx.channels", Value::from(1), Cold, Some(at));
    direct.settle(Wall::from_millis(30));
    // The timed start is issued once configured; the clock is registered at the first block.
    let start = direct.device.calls().into_iter().rev().find(|c| c.starts_with("rx_start ")).unwrap_or_else(|| panic!("{:?}", direct.device.calls()));
    let origin: i64 = start["rx_start 0 ".len()..].parse().unwrap();
    assert!(origin >= at && origin < at + 200, "origin {origin}, asked {at}");
    direct.settle(Wall::from_millis(400));
    let records: Vec<_> = direct.clocks.sample_clock_records().into_iter().filter(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).collect();
    assert_eq!(records.iter().map(|r| r.origin.ticks_in(r.root).unwrap()).collect::<Vec<_>>(), [origin], "{records:?}");
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty());
    let _ = direct.finish();
}

#[test]
fn ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration() {
    // UR-25, RM-25 (spec 20, VF-6): a receive stream enabled from 0 channels starts at the
    // first lattice instant at or after both `e` and `c`, the end of its configuration, plus
    // the profile's start lead. The `enabled` row records `e` and `c` (UR-30), so the origin
    // is checked exactly, whatever the load: one untimed `apply` takes 80 ms here.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let config = FakeConfig { apply_delay: Wall::from_millis(80), ..FakeConfig::default() };
    let mut direct = Direct::with_links(config, &[("radio.rx.channels", Scalar::from(0))], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    direct.update("radio.rx.channels", Value::from(1), Cold, None);
    direct.settle(Wall::from_millis(300));
    let instance = direct.finish();
    let envelope = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.envelope").unwrap()];
    let lead = envelope["timing"]["rx_start_lead_ns"].as_i64().unwrap() * MCR / 1_000_000_000;
    let timing = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.timing").unwrap()];
    let row = timing.as_array().unwrap().iter().find(|r| r["what"] == "cold_change").cloned().unwrap_or_else(|| panic!("{timing}"));
    let segment = timing.as_array().unwrap().iter().find(|r| r["what"] == "rx_segment").cloned().unwrap_or_else(|| panic!("{timing}"));
    let (e, configured, origin) = (row["e"].as_i64().unwrap(), row["ready"].as_i64().unwrap(), segment["origin"].as_i64().unwrap());
    assert!(configured - e >= ms(80), "{row}");
    // 200 root ticks per sample: 1 MS/s on the 200 MHz root.
    assert_eq!(origin, (e.max(configured + lead) + 199) / 200 * 200, "{row}, lead {lead}");
}


#[test]
fn ur_25_a_cold_change_the_envelope_refuses_changes_nothing() {
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::new(FakeConfig::default(), &[("radio.rx.channels", Scalar::from(2))]);
    let before = direct.device.calls().len();
    // Two channels at 200 Msps need 1.6 GB/s on a 1 GB/s link (RM-7).
    direct.update("radio.rx.sample_rate_hz", Value::num(200e6).unwrap(), Cold, None);
    direct.settle(Wall::from_millis(30));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1);
    assert!(rejected[0].payload["reason"].as_str().unwrap().starts_with("RM-7: "), "{rejected:?}");
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
    let mut direct = Direct::with_links(config, &[("radio.rx.channels", Scalar::from(0))], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    direct.update("radio.rx.channels", Value::from(1), Cold, None);
    direct.settle(Wall::from_millis(100));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1, "{rejected:?} {:?}", direct.device.calls());
    assert!(rejected[0].payload["reason"].as_str().unwrap().starts_with("UR-12: the device applied"), "{rejected:?}");
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
    assert!(admitted(&run.submit(set("radio.tx.frequency_hz", Value::num(2.4e9).unwrap()), None).unwrap()));
    assert!(admitted(&run.submit(set("radio.tx.gain_db", Value::num(5.0).unwrap()), None).unwrap()));
    assert!(admitted(&run.submit(set("radio.rx.frequency_hz", Value::num(2.4e9).unwrap()), None).unwrap()));
    wait(&mut run, ms(20));
    assert!(admitted(&run.submit(set("radio.tx.channels", Value::from(1)), None).unwrap()));
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
    assert!(admitted(&run.submit(set("radio.tx.sample_rate_hz", Value::num(2e6).unwrap()), None).unwrap()));
    let entry = send(&mut run, "start_repeat", None, &tone(100));
    assert!(admitted(&entry), "{entry:?}");
    // Booked before the switch (50 ms after receipt), transmitted after it on the new clock.
    wait(&mut run, ms(150));
    let manifest = run.finish();
    assert!(events_of(&manifest, "radio.COMMAND_REJECTED").is_empty(), "{:?}", events_of(&manifest, "radio.COMMAND_REJECTED"));
    let new = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/tx").unwrap()).nth(1).expect("the new transmit clock");
    assert!(bursts(&manifest).iter().any(|b| b.target.domain() == new.domain), "{:?}", bursts(&manifest));
}

#[test]
fn ur_25_a_cold_transmit_change_cancels_the_held_bursts() {
    // UR-25: bursts held on the old clock are refused with COMMAND_REJECTED (Review L, L11).
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "send", Some(ms(300)), &tone(100));
    assert!(admitted(&run.submit(set("radio.tx.sample_rate_hz", Value::num(2e6).unwrap()), None).unwrap()));
    wait(&mut run, ms(150));
    let manifest = run.finish();
    let reasons: Vec<_> = events_of(&manifest, "radio.COMMAND_REJECTED").iter().map(|e| e.payload["reason"].clone()).collect();
    assert!(reasons.iter().any(|r| r == "cancelled by a cold change"), "{reasons:?}");
}


#[test]
fn ur_26_stop_actions() {
    // UR-26: a `Stop` of the transmit stream ends its burst, of the receive stream cuts it at
    // its booking, of the device does both, and of another target is refused.
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "start_repeat", Some(ms(10)), &tone(100));
    wait(&mut run, ms(30));
    for target in ["radio/tx", "radio/rx", "radio", "radio/gpio"] {
        let entry = run.submit(SessionAction::Stop { target: Some(resource(target)) }, None).unwrap();
        assert!(admitted(&entry), "{target}: {entry:?}");
        wait(&mut run, ms(5));
    }
    // A Command this Provider does not serve (`start_rx` of the transmit stream) is refused
    // under the kind `command`.
    let _ = run.submit(verb("start_rx", resource("radio/tx"), None, &[]), None).unwrap();
    wait(&mut run, ms(20));
    let manifest = run.finish();
    assert!(section(&manifest, "rejected").as_array().unwrap().iter().any(|r| r["kind"] == "command"));
    assert_eq!(bursts(&manifest)[0].end, BurstEnd::Stop);
    assert!(section(&manifest, "applied").as_array().unwrap().iter().any(|r| r["key"] == "rx_stop"));
    assert!(rejections(&manifest).iter().any(|r| r.starts_with("UR-26: the Stop target is not this device")), "{:?}", rejections(&manifest));
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
    let timing = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.timing").unwrap()];
    let stop = timing.as_array().unwrap().iter().find(|r| r["what"] == "stop").unwrap()["at"].as_i64().unwrap();
    let origin = direct.clocks.sample_clock_records().iter().find(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).unwrap().origin.ticks_in(direct.clocks.sample_clock_records().iter().find(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).unwrap().root).unwrap();
    let end = origin + end.expect("blocks were published") * 200;
    assert!(end < stop, "published up to {end}, stopped at {stop}");
    // One stop, recorded once: not again when the tail reaches the cut (Review N, N4).
    let calls = direct.device.calls();
    assert_eq!(calls.iter().filter(|c| c.starts_with("rx_stop")).count(), 1, "{calls:?}");
    let applied = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.applied").unwrap()];
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
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::num(2e6).unwrap()), None).unwrap()));
    wait(&mut run, ms(100));
    assert!(admitted(&run.submit(SessionAction::Stop { target: Some(resource("radio/rx")) }, None).unwrap()));
    wait(&mut run, ms(20));
    assert!(admitted(&run.submit(verb("start_rx", resource("radio/rx"), None, &[]), None).unwrap()));
    wait(&mut run, ms(100));
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::num(1e6).unwrap()), None).unwrap()));
    wait(&mut run, ms(150));
    let manifest = run.finish();
    let timing = section(&manifest, "timing").as_array().unwrap().clone();
    let plan = timing.iter().find(|r| r["what"] == "plan").unwrap_or_else(|| panic!("no plan: {timing:?}"));
    let segments: Vec<(i64, Option<i64>)> = plan["rx"].as_array().unwrap().iter().map(|s| (s["origin"].as_i64().unwrap(), s["end"].as_i64())).collect();
    assert_eq!(segments.len(), 4, "{plan}");
    for pair in segments.windows(2) {
        assert!(pair[0].1.is_some_and(|end| end <= pair[1].0), "segments out of order or overlapping: {plan}");
    }
    assert!(segments[3].1.is_some(), "the Run's stop ends the last: {plan}");
    let clocks: Vec<(i64, Option<i64>)> = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/rx").unwrap())
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
fn ur_24_the_queue_counts_a_command_per_channel() {
    // UR-24's 16 device commands, one per channel: with two receive channels the 9th
    // update is refused (Review M, N-7).
    use ezsdr_kernel::module_api::UpdateClass::HardwareTimed;
    let mut direct = Direct::new(FakeConfig { command_queue: 64, ..FakeConfig::default() }, &[("radio.rx.channels", Scalar::from(2))]);
    let far = direct.now() + ms(5_000);
    for i in 0..9 {
        direct.update("radio.rx.gain_db", Value::num(f64::from(i)).unwrap(), HardwareTimed, Some(far + i64::from(i)));
    }
    direct.settle(Wall::from_millis(30));
    assert_eq!(direct.of("radio.COMMAND_QUEUE_FULL").len(), 1);
    let _ = direct.finish();
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
    let (mut run, _, _dir) = tx_session(FakeConfig { faults: vec![FakeFault::SlowSend(Wall::from_millis(20))], ..FakeConfig::default() });
    let at = tx_at(&run, ms(8));
    let _ = send_at(&mut run, "send", Some(at), &tone(100));
    wait(&mut run, ms(100));
    let manifest = run.finish();
    let late: Vec<_> = time_errors(&manifest).into_iter().filter(|p| p.outcome == TimeErrorOutcome::LateAtDevice).collect();
    assert_eq!(late.len(), 1, "{:?}", time_errors(&manifest));
    assert_eq!(late[0].target, at);
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
fn ur_33_the_fake_device_loops_back_what_it_transmits() {
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.rx_open(0, &[0]).unwrap();
    device.tx_open(0, &[0]).unwrap();
    let start = device.time_now().unwrap() + ms(20);
    let wave: Vec<[f32; 2]> = (0..500).map(|i| [i as f32 / 1_000.0, 0.5]).collect();
    device.tx_send(0, &[&wave], Some(start), true, true, Wall::from_secs(1)).unwrap();
    device.rx_start(0, start).unwrap();
    let mut got = Vec::new();
    while got.len() < 500 {
        if let ezsdr_radio_uhd::RxRecv::Samples { samples, first_tick } = device.rx_recv(0, 100, Wall::from_millis(100)) {
            assert_eq!(first_tick, start + got.len() as i64 * 200);
            got.extend(samples[0].clone());
        }
    }
    assert_eq!(got, wave);
}

#[test]
fn ur_33_scripted_faults_fire() {
    let device = fake(FakeConfig {
        faults: vec![FakeFault::Overflow(Wall::from_millis(20)), FakeFault::TxReport(Wall::from_millis(5), TxCode::Underflow), FakeFault::Lost(Wall::from_millis(200))],
        ..FakeConfig::default()
    });
    device.set_time_zero(false).unwrap();
    device.rx_open(0, &[0]).unwrap();
    device.rx_start(0, device.time_now().unwrap() + ms(1)).unwrap();
    // Reports are per streamer (UR-33): the scripted report needs an open transmit stream.
    device.tx_open(0, &[0]).unwrap();
    let mut saw_overflow = false;
    while !saw_overflow {
        saw_overflow = matches!(device.rx_recv(0, 1_000, Wall::from_millis(100)), ezsdr_radio_uhd::RxRecv::Overflow { out_of_sequence: false });
    }
    device.host().sleep(Wall::from_millis(10));
    assert_eq!(device.tx_async(0, Wall::ZERO).unwrap().map(|r| r.code), Some(TxCode::Underflow));
    device.host().sleep(Wall::from_millis(200));
    assert!(device.time_now().unwrap_err().lost);
}

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
fn ur_33_two_receive_streamers_keep_their_own_channels() {
    // UR-33: each receive stream has its own channel list, start, stop and packets, and the
    // loopback returns transmit channel `c` on receive channel `c`. Stopping one stream ends
    // only its own: the other goes on delivering samples produced after the stop's instant,
    // and the stopped one delivers nothing of its own past its tail — a stop that took the
    // other stream (P216) can pass neither check, whatever the host's load.
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.tx_open(0, &[0, 1]).unwrap();
    device.rx_open(0, &[0]).unwrap();
    device.rx_open(1, &[1]).unwrap();
    let start = device.time_now().unwrap() + ms(20);
    let a: Vec<[f32; 2]> = (0..600).map(|i| [i as f32 / 100.0, 0.25]).collect();
    let b: Vec<[f32; 2]> = (1..=600).map(|i| [-i as f32 / 100.0, -0.25]).collect();
    device.tx_send(0, &[&a, &b], Some(start), true, true, Wall::from_secs(1)).unwrap();
    device.rx_start(0, start).unwrap();
    device.rx_start(1, start).unwrap();
    let read = |stream: usize, n: usize| {
        let mut got: Vec<[f32; 2]> = Vec::new();
        let mut timeouts = 0;
        while got.len() < n && timeouts < 3 {
            match device.rx_recv(stream, 100, Wall::from_millis(100)) {
                ezsdr_radio_uhd::RxRecv::Samples { samples, .. } => {
                    timeouts = 0;
                    assert_eq!(samples.len(), 1, "stream {stream} carries one channel");
                    got.extend(samples[0].clone());
                }
                ezsdr_radio_uhd::RxRecv::Timeout => timeouts += 1,
                other => panic!("{other:?}"),
            }
        }
        got
    };
    assert_eq!(read(0, 300), a[..300], "stream 0 returns its own channel's signal");
    assert_eq!(read(1, 300), b[..300], "stream 1 returns its own channel's signal");
    // Each streamer carries its own stop: stopping stream 1 leaves stream 0 running.
    device.rx_stop(1, None).unwrap();
    // The instant is read after the stop, so it is at or past the stop the device took: a
    // stop that took stream 0 (P216) delivers no sample at or after that instant, ever.
    let stopped = device.time_now().unwrap();
    assert_eq!(read(0, 300), a[300..600], "the other stream keeps running");
    // Stream 0 goes on, well past the stop's instant, delivering samples produced after it.
    let past = stopped + ms(1);
    let mut reached = stopped;
    let mut blocks = 0usize;
    while reached < past && blocks < 2_000 {
        blocks += 1;
        match device.rx_recv(0, 100, Wall::from_millis(100)) {
            ezsdr_radio_uhd::RxRecv::Samples { first_tick, samples } => reached = first_tick + samples[0].len() as i64 * 200,
            ezsdr_radio_uhd::RxRecv::Overflow { .. } => {}
            other => panic!("stream 0 after the stop: {other:?}"),
        }
    }
    assert!(reached >= past, "stream 0 must keep delivering past the stop's instant {stopped}: reached {reached}");
    // Stream 1's tail holds only samples produced before its stop; once delivered, its calls
    // time out. A stop that took stream 0 instead leaves stream 1 streaming, and no call of
    // its would ever time out, however far the reader falls behind.
    let mut timed_out = false;
    let mut calls = 0usize;
    while calls < 2_000 && !timed_out {
        calls += 1;
        match device.rx_recv(1, 100, Wall::from_millis(2)) {
            ezsdr_radio_uhd::RxRecv::Samples { first_tick, .. } => {
                assert!(first_tick < stopped, "stream 1 delivered a sample produced after its stop at {stopped}: tick {first_tick}");
            }
            ezsdr_radio_uhd::RxRecv::Overflow { .. } => {}
            ezsdr_radio_uhd::RxRecv::Timeout => timed_out = true,
            other => panic!("stream 1 after its stop: {other:?}"),
        }
    }
    assert!(timed_out, "a stopped stream's calls time out once its tail is delivered");
}

#[test]
fn ur_33_two_transmit_streamers_keep_their_own_reports() {
    // UR-33: UHD's asynchronous reports are per streamer (`uhd_tx_streamer_recv_async_msg`):
    // a burst's acknowledgement reaches only the stream that sent it.
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.tx_open(0, &[0]).unwrap();
    device.tx_open(1, &[1]).unwrap();
    let wave = vec![[0.1f32, 0.0]; 100];
    let t0 = (device.time_now().unwrap() / 200 + 20_000) * 200;
    device.tx_send(0, &[&wave], Some(t0), true, true, Wall::from_secs(1)).unwrap();
    let mut reports = Vec::new();
    while device.time_now().unwrap() <= t0 + 100 * 200 {
        reports.extend(device.tx_async(0, Wall::from_millis(1)).unwrap());
    }
    reports.extend(std::iter::from_fn(|| device.tx_async(0, Wall::ZERO).unwrap()));
    assert_eq!(reports.iter().map(|r| r.code).collect::<Vec<_>>(), [TxCode::BurstAck], "the sending stream's own report");
    assert!(device.tx_async(1, Wall::ZERO).unwrap().is_none(), "the other stream has none");
    let t1 = (device.time_now().unwrap() / 200 + 20_000) * 200;
    device.tx_send(1, &[&wave], Some(t1), true, true, Wall::from_secs(1)).unwrap();
    let mut reports = Vec::new();
    while device.time_now().unwrap() <= t1 + 100 * 200 {
        reports.extend(device.tx_async(1, Wall::from_millis(1)).unwrap());
    }
    reports.extend(std::iter::from_fn(|| device.tx_async(1, Wall::ZERO).unwrap()));
    assert_eq!(reports.iter().map(|r| r.code).collect::<Vec<_>>(), [TxCode::BurstAck], "the second stream's own report");
    assert!(device.tx_async(0, Wall::ZERO).unwrap().is_none());
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
    // Every streamer is freed: a channel one of them held can be opened again.
    device.close_streams();
    device.rx_open(2, &[0, 1]).unwrap();
    device.tx_open(2, &[0, 1]).unwrap();
}

#[test]
fn ur_33_an_empty_channel_list_is_channel_zero() {
    // UR-33: UHD reads an empty channel list as channel 0 (`sanitize_stream_args`), and
    // `UhdDevice` sends it as channel 0, so a streamer opened with none is channel 0's —
    // one receive vector, one channel of transmit buffers — and holds channel 0 like any
    // other streamer.
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.rx_open(0, &[]).unwrap();
    device.rx_start(0, device.time_now().unwrap() + ms(1)).unwrap();
    match device.rx_recv(0, 100, Wall::from_millis(100)) {
        ezsdr_radio_uhd::RxRecv::Samples { samples, .. } => assert_eq!(samples.len(), 1, "the streamer's one channel is channel 0"),
        other => panic!("{other:?}"),
    }
    let error = device.rx_open(1, &[]).expect_err("channel 0 is stream 0's").message;
    assert!(error.contains("Attempting to reconnect output port!"), "{error}");
    let device = fake(FakeConfig::default());
    device.tx_open(0, &[]).unwrap();
    let wave = vec![[0.1f32, 0.0]; 8];
    assert_eq!(device.tx_send(0, &[&wave], None, false, false, Wall::from_secs(1)).unwrap(), 8, "one buffer is its channel 0's");
    let error = device.tx_open(1, &[]).expect_err("channel 0 is stream 0's").message;
    assert!(error.contains("Attempting to reconnect input port!"), "{error}");
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
fn ur_33_a_timed_command_follows_its_front_end_across_a_spec() {
    // UR-33: a timed command's destination is the front end (the radio block) it was booked
    // on: a spec change that keeps that front end applies it to its new channel, one that
    // drops it applies nothing — never another front end, and never out of range.
    let dir = ezsdr_radio_uhd::Dir::Rx;
    let set = |freq: f64| ezsdr_radio_uhd::Settings { freq: Some(freq), ..ezsdr_radio_uhd::Settings::default() };
    let settle = |device: &FakeDevice| {
        // Past the queued command's instant, a no-op timed change settles the queue first.
        device.host().sleep(Wall::from_millis(10));
        device.apply(dir, 0, &ezsdr_radio_uhd::Settings::default(), Some(device.time_now().unwrap() + ms(50))).unwrap();
    };
    // Dropped: slot B's command outlives the spec "A:0" and touches nothing.
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.apply(dir, 1, &set(2e9), Some(device.time_now().unwrap() + ms(5))).unwrap();
    device.set_subdev_spec(dir, "A:0").unwrap();
    settle(&device);
    assert_eq!(device.apply(dir, 0, &ezsdr_radio_uhd::Settings::default(), None).unwrap().freq, 1e9, "the dropped front end's command touched channel 0");
    // Moved: the command for slot B applies to B's channel after the spec reorders it.
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.apply(dir, 1, &set(2e9), Some(device.time_now().unwrap() + ms(5))).unwrap();
    device.set_subdev_spec(dir, "B:0 A:0").unwrap();
    settle(&device);
    assert_eq!(device.apply(dir, 0, &ezsdr_radio_uhd::Settings::default(), None).unwrap().freq, 2e9, "the command followed its front end to channel 0");
    assert_eq!(device.apply(dir, 1, &ezsdr_radio_uhd::Settings::default(), None).unwrap().freq, 1e9, "and left the channel it was booked with");
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
    // bindings of one device apart by it.
    assert_eq!(fake(FakeConfig::default()).serial().unwrap(), "FAKE0001");
    assert_eq!(fake(FakeConfig { serial: "F00BA2", ..FakeConfig::default() }).serial().unwrap(), "F00BA2");
}

#[test]
fn ur_33_the_antenna_is_read_back_after_an_untimed_change() {
    // UR-33, design 33 D16: `Applied.antenna` is what the device reports back after a
    // change that named one, and a timed change, which is not read back, names none.
    let dir = ezsdr_radio_uhd::Dir::Rx;
    let device = fake(FakeConfig::default());
    let set = |name: &str| ezsdr_radio_uhd::Settings { antenna: Some(name.to_owned()), ..ezsdr_radio_uhd::Settings::default() };
    assert_eq!(device.apply(dir, 0, &set("TX/RX"), None).unwrap().antenna.as_deref(), Some("TX/RX"));
    assert_eq!(device.apply(dir, 0, &ezsdr_radio_uhd::Settings::default(), None).unwrap().antenna, None, "a change that names none");
    let at = device.time_now().unwrap() + ms(5);
    assert_eq!(device.apply(dir, 0, &set("RX2"), Some(at)).unwrap().antenna, None, "a timed change is not read back");
}

#[test]
fn ur_33_the_fake_device_keeps_the_devices_queues() {
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.tx_open(0, &[0]).unwrap();
    let wave = vec![[0.1f32, 0.0]; 100];
    let now = device.time_now().unwrap();
    device.tx_send(0, &[&wave], Some(now + ms(10)), true, false, Wall::from_secs(1)).unwrap();
    // A timed start before the queued samples' end is late and dropped.
    device.tx_send(0, &[&wave], Some(now + ms(10) + 1), true, true, Wall::from_secs(1)).unwrap();
    let mut codes = Vec::new();
    while let Some(report) = device.tx_async(0, Wall::ZERO).unwrap() {
        codes.push(report.code);
    }
    assert!(codes.contains(&TxCode::TimeError), "{codes:?}");
    // In-order commands: one before a queued one takes effect after it.
    let settings = ezsdr_radio_uhd::Settings { freq: Some(2e9), ..Default::default() };
    device.apply(ezsdr_radio_uhd::Dir::Rx, 0, &settings, Some(now + ms(100))).unwrap();
    device.apply(ezsdr_radio_uhd::Dir::Rx, 0, &settings, Some(now + ms(50))).unwrap();
    let last = device.calls().into_iter().rev().find(|c| c.starts_with("apply rx")).unwrap();
    assert!(last.ends_with(&format!("effective={}", now + ms(100))), "{last}");
    // 17 commands not yet in effect: the 17th blocks.
    let full = fake(FakeConfig::default());
    full.set_time_zero(false).unwrap();
    for i in 0..16 {
        full.apply(ezsdr_radio_uhd::Dir::Rx, 0, &settings, Some(ms(5_000) + i)).unwrap();
    }
    let (sent, done) = std::sync::mpsc::channel();
    let blocked = full.clone();
    std::thread::spawn(move || {
        let _ = blocked.apply(ezsdr_radio_uhd::Dir::Rx, 0, &ezsdr_radio_uhd::Settings::default(), Some(ms(5_100)));
        let _ = sent.send(());
    });
    assert!(done.recv_timeout(Wall::from_millis(200)).is_err(), "the 17th apply blocks");
    let _ = Mutex::new(());
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

#[test]
fn rehearsal_b5_overflow() {
    let _ = rehearse_overflow(fake(FakeConfig::default()), Wall::from_millis(500), Wall::from_millis(300));
}

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
    let frequency = section(&manifest, "applied").as_array().unwrap().iter().find(|r| r["key"] == "radio.rx.frequency_hz").cloned().unwrap();
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
// The fake's ways of being as strict as the X300 (UR-33), each pinned on the device itself
// (Review N, T2), and the bench link the receive tests of B1 and B2 use. The raw receive
// cases — a timed stop, a stopped stream's tail and its whole timeout, a request cut short by
// a stop — are unit tests in `src/device.rs`, where every thread of a scenario is a
// participant of the fake's clock (plan 30 D2).

/// The receive link as the bench measured it (INFERRED from its stops: 2–4 ms), with the
/// X300's 1 996-sample packets.
fn bench_link(latency_ms: u64) -> FakeConfig {
    FakeConfig { rx_latency: Wall::from_millis(latency_ms), rx_packet: Some(1_996), ..FakeConfig::default() }
}

/// The device's transmit reports until its time passes `tick` (a burst's acknowledgement
/// comes once its end has played, and a later report behind it).
fn reports_until(device: &FakeDevice, tick: i64) -> Vec<ezsdr_radio_uhd::TxReport> {
    let mut reports = Vec::new();
    while device.time_now().unwrap() <= tick + 200_000 {
        reports.extend(device.tx_async(0, Wall::from_millis(1)).unwrap());
    }
    reports.extend(std::iter::from_fn(|| device.tx_async(0, Wall::ZERO).unwrap()));
    reports
}

#[test]
fn ur_33_an_empty_end_of_burst_is_one_zero_sample() {
    // UHD sends an empty end-of-burst as one zero sample (`tx_streamer_impl.hpp:266–276`):
    // the burst ends a sample later, and a timed start one sample after its data is late
    // (`hw_b8_raw_empty_eob_gap`: 4 of 4), while after end-of-burst on the data it is not.
    for (empty, late) in [(true, true), (false, false)] {
        let device = fake(FakeConfig::default());
        device.set_time_zero(false).unwrap();
        device.tx_open(0, &[0]).unwrap();
        let wave = vec![[0.1f32, 0.0]; 100];
        let t0 = (device.time_now().unwrap() / 200 + 20_000) * 200;
        device.tx_send(0, &[&wave], Some(t0), true, !empty, Wall::from_secs(1)).unwrap();
        if empty {
            device.tx_send(0, &[&[][..]], None, false, true, Wall::from_secs(1)).unwrap();
        }
        device.tx_send(0, &[&wave], Some(t0 + 101 * 200), true, true, Wall::from_secs(1)).unwrap();
        let codes: Vec<_> = reports_until(&device, t0 + 201 * 200).into_iter().map(|r| r.code).collect();
        assert_eq!(codes.contains(&TxCode::TimeError), late, "empty end-of-burst {empty}: {codes:?}");
    }
}

#[test]
fn ur_33_a_timed_start_at_the_previous_burst_s_end_is_late() {
    // design-notes §11 F3 (`hw_b8_raw_burst_gap`): at gap 0 the second burst is late and
    // dropped, at one sample it is played. The late report's tick is at or after its time.
    for (gap, late) in [(0i64, true), (1, false)] {
        let device = fake(FakeConfig::default());
        device.set_time_zero(false).unwrap();
        device.tx_open(0, &[0]).unwrap();
        let wave = vec![[0.1f32, 0.0]; 100];
        let t0 = (device.time_now().unwrap() / 200 + 20_000) * 200;
        device.tx_send(0, &[&wave], Some(t0), true, true, Wall::from_secs(1)).unwrap();
        let t1 = t0 + (100 + gap) * 200;
        device.tx_send(0, &[&wave], Some(t1), true, true, Wall::from_secs(1)).unwrap();
        let reports = reports_until(&device, t1 + 100 * 200);
        let error = reports.iter().find(|r| r.code == TxCode::TimeError);
        assert_eq!(error.is_some(), late, "gap {gap}: {reports:?}");
        if let Some(error) = error {
            assert!(error.tick.unwrap() >= t1, "{error:?}");
        }
    }
}

#[test]
fn ur_33_a_burst_that_runs_dry_underflows() {
    // The X300 reports an underflow when an open burst runs out of samples before its
    // end-of-burst.
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.tx_open(0, &[0]).unwrap();
    let wave = vec![[0.1f32, 0.0]; 1_000];
    let t0 = (device.time_now().unwrap() / 200 + 5_000) * 200;
    device.tx_send(0, &[&wave], Some(t0), true, false, Wall::from_secs(1)).unwrap();
    device.host().sleep(Wall::from_millis(30));
    let mut codes = Vec::new();
    while let Some(report) = device.tx_async(0, Wall::ZERO).unwrap() {
        codes.push(report.code);
    }
    assert!(codes.contains(&TxCode::Underflow), "{codes:?}");
}

#[test]
fn ur_33_a_burst_resumed_after_an_underflow_plays_late() {
    // UHD 4.10's `radio_tx_core.v`: a burst that runs dry between packets goes idle
    // (`:359–370`), its next untimed packet plays at once (`:341`, `:347`), and a timed one
    // whose time has passed is late (`:348–355`). So the rest of the burst is late by the
    // gap, and a timed start at its scheduled end + 1 sample, before its actual end, is
    // late. The ticks are the fake's. A host stall only widens the gap.
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.tx_open(0, &[0]).unwrap();
    let wave = vec![[0.1f32, 0.0]; 1_000];
    let rest = vec![[0.2f32, 0.0]; 20_000];
    let t0 = (device.time_now().unwrap() / 200 + 100_000) * 200;
    let dry = t0 + 1_000 * 200;
    device.tx_send(0, &[&wave], Some(t0), true, false, Wall::from_secs(1)).unwrap();
    while device.time_now().unwrap() < dry + ms(5) {
        device.host().sleep(Wall::from_millis(1));
    }
    device.tx_send(0, &[&rest], None, false, true, Wall::from_secs(1)).unwrap();
    let resumed = device.time_now().unwrap();
    let scheduled_end = dry + 20_000 * 200;
    let b = scheduled_end + 200;
    device.tx_send(0, &[&wave], Some(b), true, true, Wall::from_secs(1)).unwrap();
    let reports = reports_until(&device, resumed + ms(60));
    let codes: Vec<_> = reports.iter().map(|r| r.code).collect();
    assert_eq!(codes, [TxCode::Underflow, TxCode::BurstAck, TxCode::TimeError], "{reports:?}");
    assert_eq!(reports[0].tick, Some(dry), "it ran dry at the end of what it had");
    assert!(reports[1].tick.unwrap() >= scheduled_end + ms(5), "the rest played late: {reports:?}");
    assert!(reports[2].tick.unwrap() >= b, "{reports:?}");
}

/// Captures across `changes` `cold` receive rate changes in one Session on `config`,
/// alternating `rates`, and for each the old clock's end, the capture's maps and whether
/// every sample of each map is its own clock's ramp (the fake without a transmitter).
fn captures_across_cold_changes(config: FakeConfig, selector: Json, rates: [f64; 2], changes: usize) -> (Manifest, Vec<String>) {
    let dir = TempDir::new();
    let device = fake(config);
    let mut run = session(&profile(&dir, selector, json!({}), true), device.clone());
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::num(rates[0]).unwrap()), None).unwrap()));
    wait(&mut run, ms(200));
    for i in 0..changes {
        let capture_at = after(&run, ms(10));
        let n = (rates[i % 2] * 0.12) as i64 + (rates[(i + 1) % 2] * 0.08) as i64;
        assert!(admitted(&run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", Value::from(n))]), None).unwrap()));
        wait(&mut run, ms(60));
        assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::num(rates[(i + 1) % 2]).unwrap()), None).unwrap()));
        let horizon = after(&run, ms(3_000));
        let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], i, horizon);
    }
    let manifest = run.finish();
    let clocks = &manifest.clocks.sample_clocks;
    let mut problems = Vec::new();
    // The bounded wait never goes under 1 ms (UHD truncates the timeout to whole ms).
    if device.min_recv_timeout() < Wall::from_millis(1) {
        problems.push(format!("a receive wait of {:?}", device.min_recv_timeout()));
    }
    for artifact in manifest.artifacts.iter().filter(|(output, _)| output.as_str() == "rec").flat_map(|(_, list)| list) {
        // Each capture begins before its change, so it holds both clocks' samples.
        if artifact.continuity.len() < 2 {
            problems.push(format!("{}: the samples of one clock only: {:?}", artifact.id, artifact.continuity));
        }
        let data = read_capture(artifact, 1).remove(0);
        let mut at = 0usize;
        for (m, map) in artifact.continuity.iter().enumerate() {
            let clock = clocks.iter().find(|c| c.domain == map.domain).unwrap();
            let n = clock.root_ticks_per_tick.num() as i64;
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
fn ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it() {
    // Review N, B3: a burst booked at the end of one whose last buffer already went out
    // (5–10 ms before its end: inside the in-flight window, outside the 5 ms lead) is
    // played, not dropped by the device at the tick the first ended.
    let mut played = 0;
    for _ in 0..3 {
        let (mut run, device, _dir) = tx_session(FakeConfig::default());
        wait(&mut run, ms(1));
        let a = tx_at(&run, ms(40));
        let _ = send_at(&mut run, "send", Some(a), &tone(30_000));
        let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx").unwrap() && r.ended_at.is_none()).unwrap();
        let a_end = clock.origin.ticks_in(clock.root).unwrap() + (a.ticks_in(clock.domain).unwrap() + 30_000) * clock.root_ticks_per_tick.num() as i64;
        while run.now().ticks_in(clock.root).unwrap() < a_end - ms(8) {
            wait(&mut run, ms(1) / 4);
        }
        let b = TimePoint::new(a.domain(), a.ticks_in(clock.domain).unwrap() + 30_000);
        let second: Vec<(f32, f32)> = (0..1_000).map(|i| (-0.25 - i as f32 / 4_000.0, 0.125)).collect();
        let entry = send_at(&mut run, "send", Some(b), &second);
        wait(&mut run, ms(40));
        let manifest = run.finish();
        assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError" && r["code"] != "Underflow"), "{:?}", calls(&device, "tx_send"));
        assert_eq!(device.unended_bursts(), 0, "{:?}", calls(&device, "tx_send"));
        if admitted(&entry) && time_errors(&manifest).is_empty() && bursts(&manifest).len() == 2 {
            played += 1;
            // Every sample handed over once: the first's held last sample too (Review O).
            let handed: usize = calls(&device, "tx_send").iter().map(|c| c.split("n=").nth(1).unwrap().split(' ').next().unwrap().parse::<usize>().unwrap()).sum();
            assert_eq!(handed, 31_000, "{:?}", calls(&device, "tx_send"));
            // And in order: the first's held last sample ahead of the second's first, the
            // two waveforms end to end on the device (Review P, TG-1).
            let a_root = clock.origin.ticks_in(clock.root).unwrap() + a.ticks_in(clock.domain).unwrap() * clock.root_ticks_per_tick.num() as i64;
            let whole: Vec<Option<[f32; 2]>> = tone(30_000).iter().chain(&second).map(|&(i, q)| Some([i, q])).collect();
            assert!(device.transmitted(a_root, 31_000) == whole, "{:?}", calls(&device, "tx_send"));
        }
    }
    assert!(played > 0, "the case was never reached");
}

#[test]
fn ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it() {
    // Review N, T3: a burst whose device burst is held open for a burst that may be
    // booked at its end, then `Stop`: one end-of-burst closes it, before it runs dry.
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    // `now()` is the Run's last instant: refresh it, and leave room for the submission.
    wait(&mut run, ms(1));
    let a = tx_at(&run, ms(40));
    let entry = send_at(&mut run, "send", Some(a), &tone(30_000));
    assert!(admitted(&entry), "{entry:?}");
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx").unwrap() && r.ended_at.is_none()).unwrap();
    let a_end = clock.origin.ticks_in(clock.root).unwrap() + (a.ticks_in(clock.domain).unwrap() + 30_000) * clock.root_ticks_per_tick.num() as i64;
    while run.now().ticks_in(clock.root).unwrap() < a_end - ms(7) {
        wait(&mut run, ms(1) / 4);
    }
    let _ = run.submit(SessionAction::Stop { target: Some(resource("radio/tx")) }, None);
    wait(&mut run, ms(40));
    let manifest = run.finish();
    assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
    let sends = calls(&device, "tx_send");
    assert_eq!(sends.iter().filter(|c| c.ends_with("eob=true")).count(), 1, "{sends:?}");
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "Underflow"), "{sends:?}");
    assert_eq!(device.unended_bursts(), 0);
}

#[test]
fn ur_23_a_burst_alone_is_ended_before_it_runs_dry() {
    // Review N, B3's deferral: with no burst booked at its end, the device burst is
    // closed a device lead before its end, not left to underflow.
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "send", Some(ms(20)), &tone(5_000));
    wait(&mut run, ms(80));
    let manifest = run.finish();
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
    direct.update("radio.rx.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(now + ms(5_000)));
    direct.settle(Wall::from_millis(2_000));
    let lost = direct.of(EventKind::DEVICE_LOST);
    let _ = direct.finish();
    assert_eq!(lost.len(), 1, "{lost:?}");
}

#[test]
fn ur_23_a_burst_one_sample_after_a_burst_is_played() {
    // Review O, O-B1 (`hw_b8_raw_empty_eob_gap`): the device burst of a burst ending at c
    // must end at c, not a padded sample later, so that a burst at c + 1 is played —
    // booked before the first's last buffer went out, and after. What uhd-tx hands the
    // device is checked first: a slow host cannot fail it, though it can keep uhd-tx from
    // holding a last sample back, the path where an empty end-of-burst would show. Then what
    // the device did with it, which needs the host to keep up. A trial in which a burst was
    // dropped at booking (booked within the 2 ms device lead) did not set the case up and
    // is run again, three trials at most; an underflow or a late report at the device
    // fails it.
    for late_booking in [false, true] {
        let mut not_booked = Vec::new();
        loop {
            let (mut run, device, _dir) = tx_session(FakeConfig::default());
            wait(&mut run, ms(1));
            let a = tx_at(&run, ms(40));
            let entry = send_at(&mut run, "send", Some(a), &tone(30_000));
            assert!(admitted(&entry), "{entry:?}");
            let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx").unwrap() && r.ended_at.is_none()).unwrap();
            let n = clock.root_ticks_per_tick.num() as i64;
            let a_root = clock.origin.ticks_in(clock.root).unwrap() + a.ticks_in(clock.domain).unwrap() * n;
            let a_end = a_root + 30_000 * n;
            if late_booking {
                while run.now().ticks_in(clock.root).unwrap() < a_end - ms(8) {
                    wait(&mut run, ms(1) / 4);
                }
            }
            let b = TimePoint::new(a.domain(), a.ticks_in(clock.domain).unwrap() + 30_001);
            let entry = send_at(&mut run, "send", Some(b), &tone(1_000));
            assert!(admitted(&entry), "{entry:?}");
            wait(&mut run, ms(80));
            // A host that fell behind may not have handed B over yet: `finish` would cancel it.
            // The watch is the device's own clock (plan 30 §4.5).
            let horizon = device.host().now() + Wall::from_secs(2);
            while tx_sends(&device).iter().filter(|s| s.eob).count() < 2 && device.host().now() < horizon {
                wait(&mut run, ms(1));
            }
            let manifest = run.finish();
            let dropped: Vec<_> = time_errors(&manifest).into_iter().filter(|p| p.outcome == TimeErrorOutcome::Drop).collect();
            if !dropped.is_empty() {
                not_booked.push(dropped);
                assert!(not_booked.len() < 3, "booked late {late_booking}: dropped at booking in every trial: {not_booked:?}");
                continue;
            }
            // Each device burst is its Kernel burst's samples and no more: an empty
            // end-of-burst would add a zero sample, ending A at c + 1, B's start.
            let sends = tx_sends(&device);
            let device_bursts: Vec<&[Sent]> = sends.split_inclusive(|s| s.eob).collect();
            assert_eq!(device_bursts.len(), 2, "booked late {late_booking}: {sends:?}");
            for (burst, at, len) in [(device_bursts[0], a_root, 30_000), (device_bursts[1], a_end + n, 1_000)] {
                assert!(burst[0].sob && burst[0].at == Some(at), "booked late {late_booking}: {sends:?}");
                assert!(burst[1..].iter().all(|s| !s.sob && s.at.is_none()), "booked late {late_booking}: {sends:?}");
                assert_eq!(burst.iter().map(|s| s.n).sum::<usize>(), len, "booked late {late_booking}: {sends:?}");
            }
            assert_eq!(device.unended_bursts(), 0);
            // The device's reports, each at its tick from A's end: an underflow a sample before
            // A's or B's end (-0.001 ms, +1.000 ms) is a held last sample sent too late, one
            // earlier the host not keeping the burst fed.
            let reports: Vec<String> = section(&manifest, "async").as_array().unwrap().iter().map(|r| {
                let at = r["tick"].as_i64().map_or("-".to_owned(), |t| format!("{:+.3} ms", (t - a_end) as f64 / ms(1) as f64));
                format!("{} {at}", r["code"].as_str().unwrap())
            }).collect();
            assert!(!reports.iter().any(|r| r.starts_with("TimeError") || r.starts_with("Underflow")), "booked late {late_booking}: {reports:?} {sends:?}");
            assert!(time_errors(&manifest).is_empty(), "{:?}", time_errors(&manifest));
            assert_eq!(bursts(&manifest).len(), 2, "{:?}", bursts(&manifest));
            break;
        }
    }
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
        let mut direct = Direct::build(config, &[("radio.rx.sample_rate_hz", Scalar::try_from(390_625.0).unwrap())], vec![port], json!({ "block_len": block_len }));
        direct.settle(Wall::from_millis(200 + phase * step_ms));
        direct.update("radio.rx.sample_rate_hz", Value::num(2e6).unwrap(), Cold, None);
        direct.settle(Wall::from_millis(600));
        let late = direct.of("radio.LATE_COMMAND");
        let instance = direct.finish();
        let timing = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.timing").unwrap()];
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
    let target = serde_json::to_value(resource("radio/tx")).unwrap();
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
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    wait(&mut run, ms(1));
    let a = tx_at(&run, ms(40));
    let entry = send_at(&mut run, "send", Some(a), &[(0.5, -0.25)]);
    assert!(admitted(&entry), "{entry:?}");
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx").unwrap() && r.ended_at.is_none()).unwrap();
    let a_root = clock.origin.ticks_in(clock.root).unwrap() + a.ticks_in(clock.domain).unwrap() * clock.root_ticks_per_tick.num() as i64;
    wait(&mut run, ms(60));
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
        let (mut run, device, _dir) = tx_session(FakeConfig { faults: vec![fault], ..FakeConfig::default() });
        wait(&mut run, ms(1));
        let a = tx_at(&run, ms(40));
        let wave = tone(len);
        let entry = send_at(&mut run, "send", Some(a), &wave);
        assert!(admitted(&entry), "{entry:?}");
        let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx").unwrap() && r.ended_at.is_none()).unwrap();
        let a_root = clock.origin.ticks_in(clock.root).unwrap() + a.ticks_in(clock.domain).unwrap() * clock.root_ticks_per_tick.num() as i64;
        wait(&mut run, ms(60));
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

#[test]
fn ur_33_a_burst_s_acknowledgement_comes_once_its_end_has_played() {
    // The device sends BURST_ACK when a burst's end plays, so a later burst's report waits
    // behind it (Review Q, TG-Q1).
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.tx_open(0, &[0]).unwrap();
    let wave = vec![[0.1f32, 0.0]; 100];
    let t0 = (device.time_now().unwrap() / 200 + 20_000) * 200;
    device.tx_send(0, &[&wave], Some(t0), true, true, Wall::from_secs(1)).unwrap();
    device.tx_send(0, &[&wave], Some(t0 + 100 * 200), true, true, Wall::from_secs(1)).unwrap();
    assert!(device.tx_async(0, Wall::ZERO).unwrap().is_none(), "an acknowledgement before the burst played");
    let codes: Vec<_> = reports_until(&device, t0 + 100 * 200).into_iter().map(|r| r.code).collect();
    assert_eq!(codes.first(), Some(&TxCode::BurstAck), "{codes:?}");
    assert!(codes.contains(&TxCode::TimeError), "the second burst's report behind the first's: {codes:?}");
}

#[test]
fn ur_22_a_first_send_cut_short_keeps_the_reports_paired() {
    // Review Q, TG-Q1: a burst whose first send came up short, or failed after taking half
    // (NB-Q1), is still counted for its report, so the late report of the burst after it
    // names that burst. B starts where A's device burst ended (499 samples taken, then the
    // empty end-of-burst's zero sample), booked once A was abandoned: the device drops it.
    for fault in [FakeFault::ShortSend(0), FakeFault::FailSend(0)] {
        let (mut run, device, _dir) = tx_session(FakeConfig { faults: vec![fault], ..FakeConfig::default() });
        wait(&mut run, ms(1));
        let a = tx_at(&run, ms(40));
        let entry = send_at(&mut run, "send", Some(a), &tone(1_000));
        assert!(admitted(&entry), "{entry:?}");
        let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx").unwrap() && r.ended_at.is_none()).unwrap();
        let a_root = clock.origin.ticks_in(clock.root).unwrap() + a.ticks_in(clock.domain).unwrap() * clock.root_ticks_per_tick.num() as i64;
        while run.now().ticks_in(clock.root).unwrap() < a_root - ms(8) {
            wait(&mut run, ms(1) / 4);
        }
        let b = TimePoint::new(a.domain(), a.ticks_in(clock.domain).unwrap() + 500);
        let entry = send_at(&mut run, "send", Some(b), &tone(1_000));
        assert!(admitted(&entry), "{entry:?}");
        wait(&mut run, ms(60));
        let manifest = run.finish();
        let late: Vec<_> = time_errors(&manifest).into_iter().filter(|p| p.outcome == TimeErrorOutcome::LateAtDevice).collect();
        assert_eq!(late.len(), 1, "{:?} {:?}", time_errors(&manifest), calls(&device, "tx_send"));
        assert_eq!(late[0].target, b);
    }
}

#[test]
fn ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned() {
    // Review Q, NB-Q2: the send of A's held sample ahead of B, B continuing A's device
    // burst, takes nothing or fails (Review R, TG-R1). B would play a sample early: it is
    // abandoned, and the device burst ended with an empty end-of-burst after A's 29 999
    // samples taken.
    let cases = [(FakeFault::StalledSend(15), "UR-22: a send did not complete"), (FakeFault::FailSend(15), "fake: the send failed")];
    for (fault, reason) in cases {
        let what = format!("{fault:?}");
        let (mut run, device, _dir) = tx_session(FakeConfig { faults: vec![fault], ..FakeConfig::default() });
        wait(&mut run, ms(1));
        let a = tx_at(&run, ms(40));
        let entry = send_at(&mut run, "send", Some(a), &tone(30_000));
        assert!(admitted(&entry), "{entry:?}");
        let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx").unwrap() && r.ended_at.is_none()).unwrap();
        let a_root = clock.origin.ticks_in(clock.root).unwrap() + a.ticks_in(clock.domain).unwrap() * clock.root_ticks_per_tick.num() as i64;
        let a_end = a_root + 30_000 * clock.root_ticks_per_tick.num() as i64;
        while run.now().ticks_in(clock.root).unwrap() < a_end - ms(8) {
            wait(&mut run, ms(1) / 4);
        }
        let entry = send_at(&mut run, "send", Some(TimePoint::new(a.domain(), a.ticks_in(clock.domain).unwrap() + 30_000)), &[(-0.5, 0.5); 1_000]);
        assert!(admitted(&entry), "{entry:?}");
        wait(&mut run, ms(40));
        let manifest = run.finish();
        let sends = calls(&device, "tx_send");
        assert!(sends.iter().any(|c| c.contains("stalled") || c.contains("failed")), "{what}: the case was not reached: {sends:?}");
        let sent = device.transmitted(a_root, 30_001);
        let whole: Vec<Option<[f32; 2]>> = tone(30_000).iter().map(|&(i, q)| Some([i, q])).collect();
        assert!(sent[..29_999] == whole[..29_999], "{sends:?}");
        assert_eq!(sent[29_999], Some([0.0, 0.0]), "B played in A's last sample's place: {sends:?}");
        assert_eq!(sent[30_000], None, "{sends:?}");
        assert_eq!(sends.iter().filter(|c| c.ends_with("eob=true")).count(), 1, "{sends:?}");
        assert!(rejections(&manifest).iter().any(|r| r.contains(reason)), "{what}: {:?} {sends:?}", rejections(&manifest));
    }
}

#[test]
fn ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst() {
    // Review Q, TG-Q2: B continues A's device burst (booked after A's last buffer went out,
    // A's held sample sent ahead of B); B's first send takes nothing: the device burst is
    // still open, and is ended with an empty end-of-burst after A's last sample.
    let (mut run, device, _dir) = tx_session(FakeConfig { faults: vec![FakeFault::StalledSend(16)], ..FakeConfig::default() });
    wait(&mut run, ms(1));
    let a = tx_at(&run, ms(40));
    let entry = send_at(&mut run, "send", Some(a), &tone(30_000));
    assert!(admitted(&entry), "{entry:?}");
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx").unwrap() && r.ended_at.is_none()).unwrap();
    let a_root = clock.origin.ticks_in(clock.root).unwrap() + a.ticks_in(clock.domain).unwrap() * clock.root_ticks_per_tick.num() as i64;
    let a_end = a_root + 30_000 * clock.root_ticks_per_tick.num() as i64;
    while run.now().ticks_in(clock.root).unwrap() < a_end - ms(8) {
        wait(&mut run, ms(1) / 4);
    }
    let entry = send_at(&mut run, "send", Some(TimePoint::new(a.domain(), a.ticks_in(clock.domain).unwrap() + 30_000)), &tone(1_000));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(40));
    let manifest = run.finish();
    let sends = calls(&device, "tx_send");
    assert!(sends.iter().any(|c| c.contains("stalled")), "the case was not reached: {sends:?}");
    assert_eq!(sends.iter().filter(|c| c.ends_with("eob=true")).count(), 1, "{sends:?}");
    assert_eq!(device.unended_bursts(), 0, "{sends:?}");
    let sent = device.transmitted(a_root, 30_002);
    let whole: Vec<Option<[f32; 2]>> = tone(30_000).iter().map(|&(i, q)| Some([i, q])).collect();
    assert!(sent[..30_000] == whole[..], "{sends:?}");
    assert_eq!(sent[30_000], Some([0.0, 0.0]), "{sends:?}");
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "Underflow"), "{sends:?}");
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
        let entry = send_at(&mut run, "send", Some(at), &tone(10_000));
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
    let entry = send_at(&mut run, "send", Some(at), &tone(30_000));
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
        { "at": { "clock": "radio", "offset_ticks": 50_000 }, "action": { "kind": "update_parameter", "target": target, "key": "radio.rx.sample_rate_hz", "value": 2e6 } },
        { "at": { "clock": "radio", "offset_ticks": 150_000 }, "action": { "kind": "update_parameter", "target": target, "key": "radio.rx.gain_db", "value": 3.0 } }
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
    let clock = manifest.clocks.sample_clocks.iter().find(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).expect("the receive clock");
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
