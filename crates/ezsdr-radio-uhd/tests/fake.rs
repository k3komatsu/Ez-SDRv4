//! Spec 18 §6: every UR rule through the real coordinator on `FakeDevice`, with an
//! Assembly built as the server's catalogue builds it (the capture Sink, the host
//! Link, `DeviceAuthority` and `UhdRadio` on one device), unless a test says "unit".
//! Timings are wall-clock; each assertion allows a loaded machine's jitter.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding};
use ezsdr_kernel::coordinator::{RunHandle, RunHandleError, start_spec_run};
use ezsdr_kernel::event::{Action, ActionId, Event, EventCollector, EventKind};
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
use ezsdr_kernel::stream::{BurstEnd, DataLinkDecl, GapCause};
use ezsdr_kernel::time::{ClockRegistry, Duration, RelativeBudget, TimePoint};
use ezsdr_radio::payloads::{RxOverflowCause, RxOverflowPayload, TimeErrorOutcome, TimeErrorPayload};
use ezsdr_radio_uhd::{Device, DeviceAuthority, FakeConfig, FakeDevice, FakeFault, TxCode, UhdRadio};
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
    assert_eq!(descriptor.impl_hash, Some(ContentHash::of_bytes(b"ezsdr.radio.uhd 0.1.0")));
    let mut old = ModuleRegistry::new();
    let mut vocabulary = ezsdr_radio::vocabulary();
    vocabulary.version = ezsdr_kernel::module_api::Version::new(1, 2, 0);
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
    assert!(UhdRadio::from_binding(&binding(ok.clone(), Some(x310_ubx())), device.clone()).is_ok());
    let mut other = binding(ok.clone(), Some(x310_ubx()));
    other.module.id = ezsdr_kernel::id::ModuleId::parse("ezsdr.radio.mock").unwrap();
    assert!(refuse(other, device.clone()).starts_with("UR-5:"));
    assert!(refuse(binding(ok.clone(), None), device.clone()).starts_with("UR-5:"));
    let x310_like = json!({ "name": "x310-like", "version": { "major": 1, "minor": 1, "patch": 0 } });
    assert!(refuse(binding(ok.clone(), Some(x310_like)), device.clone()).starts_with("UR-5:"));
    let mut fed = binding(ok.clone(), Some(x310_ubx()));
    fed.feed = serde_json::from_value(json!({ "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 4 })).unwrap();
    assert!(refuse(fed, device.clone()).starts_with("UR-5:"));
    for selector in [json!({ "args": ARGS, "fake": true }), json!({ "args": ARGS, "block_len": 0 }), json!({ "args": ARGS, "clock_source": "atomic" }), json!({})] {
        assert!(refuse(binding(selector.clone(), Some(x310_ubx())), device.clone()).starts_with("UR-5:"), "{selector}");
    }
    let slow = fake(FakeConfig { master_clock_rate: 184_320_000, ..FakeConfig::default() });
    assert_eq!(
        refuse(binding(ok, Some(x310_ubx())), slow),
        "UR-5: profile x310-ubx needs a 200 MHz master clock; the device runs at 184320000 Hz"
    );
}

fn x310_cbx() -> Json {
    json!({ "name": "x310-cbx", "version": { "major": 0, "minor": 1, "patch": 0 } })
}

fn x310_obx() -> Json {
    json!({ "name": "x310-obx", "version": { "major": 0, "minor": 1, "patch": 0 } })
}

#[test]
fn ur_09_x310_obx_is_one_channel_from_10_mhz_to_8_4_ghz_defaulting_to_1_ghz() {
    let radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x310_obx())), fake(one_obx())).unwrap();
    let capability = |name: &str| radio.instance().tree.capabilities[&Key::parse(name).unwrap()].clone();
    let range = |min, max| ezsdr_kernel::spec::CapabilityValue::Range { min, max };
    for dir in ["rx", "tx"] {
        assert_eq!(capability(&format!("radio.{dir}.frequency_hz")), range(Value::Num(1e7), Value::Num(8.4e9)));
        assert_eq!(capability(&format!("radio.{dir}.channels")), range(Value::Int(0), Value::Int(1)));
    }
    assert_eq!(capability("radio.phase_behavior_on_retune"), ezsdr_kernel::spec::CapabilityValue::One { value: Value::Str("random_unless_timed_tune".to_owned()) });
    assert_eq!(radio.instance().profile.as_ref().unwrap().name, "x310-obx");
    let dir = TempDir::new();
    let mut doc = profile(&dir, json!({}), json!({}), true);
    doc["bindings"]["radio"]["profile"] = x310_obx();
    let mut run = session(&doc, fake(one_obx()));
    past_t0(&mut run, ms(1));
    let manifest = run.finish();
    let applied = section(&manifest, "applied").as_array().unwrap().iter().find(|r| r["key"] == "radio.rx.frequency_hz").cloned().unwrap();
    assert_eq!(applied["claimed"], 1e9);
}

#[test]
fn ur_05_the_profile_must_be_the_device_s_front_ends() {
    let ok = json!({ "args": ARGS });
    let refuse = |profile: Json, config: FakeConfig| UhdRadio::from_binding(&binding(ok.clone(), Some(profile)), fake(config)).err().unwrap().message;
    assert!(UhdRadio::from_binding(&binding(ok.clone(), Some(x310_cbx())), fake(one_cbx())).is_ok());
    let boards = |front_ends: Vec<&'static str>| FakeConfig { front_ends, ..FakeConfig::default() };
    assert_eq!(refuse(x310_ubx(), one_cbx()), "UR-5: profile x310-ubx needs UBX front ends; rx channel 0 is `CBX-120 RX`");
    assert_eq!(refuse(x310_ubx(), boards(vec!["UBX", "CBX"])), "UR-5: profile x310-ubx needs UBX front ends; rx channel 1 is `CBX RX`");
    assert_eq!(refuse(x310_ubx(), boards(vec!["UBX"])), "UR-5: profile x310-ubx needs 2 rx channels; the device has 1");
    assert_eq!(refuse(x310_cbx(), FakeConfig::default()), "UR-5: profile x310-cbx needs CBX front ends; rx channel 0 is `UBX RX`");
    assert_eq!(refuse(x310_cbx(), boards(vec![])), "UR-5: profile x310-cbx needs 1 rx channels; the device has 0");
    assert!(UhdRadio::from_binding(&binding(ok.clone(), Some(x310_obx())), fake(one_obx())).is_ok());
    assert!(UhdRadio::from_binding(&binding(ok.clone(), Some(x310_obx())), fake(boards(vec!["OBX", "OBX"]))).is_ok());
    assert_eq!(refuse(x310_ubx(), one_obx()), "UR-5: profile x310-ubx needs UBX front ends; rx channel 0 is `OBX RX`");
    assert_eq!(refuse(x310_obx(), FakeConfig::default()), "UR-5: profile x310-obx needs OBX front ends; rx channel 0 is `UBX RX`");
    assert_eq!(refuse(x310_obx(), one_cbx()), "UR-5: profile x310-obx needs OBX front ends; rx channel 0 is `CBX-120 RX`");
}

#[test]
fn ur_09_x310_cbx_is_one_channel_from_1_2_ghz_defaulting_to_2_45_ghz() {
    let radio = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x310_cbx())), fake(one_cbx())).unwrap();
    let capability = |name: &str| radio.instance().tree.capabilities[&Key::parse(name).unwrap()].clone();
    let range = |min, max| ezsdr_kernel::spec::CapabilityValue::Range { min, max };
    for dir in ["rx", "tx"] {
        assert_eq!(capability(&format!("radio.{dir}.frequency_hz")), range(Value::Num(1.2e9), Value::Num(6e9)));
        assert_eq!(capability(&format!("radio.{dir}.channels")), range(Value::Int(0), Value::Int(1)));
    }
    // A Session that sets no frequency tunes to the default, inside the CBX's range.
    let dir = TempDir::new();
    let mut doc = profile(&dir, json!({}), json!({}), true);
    doc["bindings"]["radio"]["profile"] = x310_cbx();
    let mut run = session(&doc, fake(one_cbx()));
    past_t0(&mut run, ms(1));
    let manifest = run.finish();
    let applied = section(&manifest, "applied").as_array().unwrap().iter().find(|r| r["key"] == "radio.rx.frequency_hz").cloned().unwrap();
    assert_eq!(applied["claimed"], 2.45e9);
}

#[test]
fn ur_06_the_provider_refuses_another_root() {
    let dir = TempDir::new();
    let profile = profile(&dir, json!({ "authority_args": "addr=10.0.0.9" }), json!({}), false);
    let run = start_spec_run(&receive_spec(1, 1e6, 1e9, None), &profile, assembly(&profile, fake(FakeConfig::default()), BTreeMap::new(), |r| r)).unwrap();
    assert_eq!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Prepare } });
    let manifest = run.finish();
    assert!(failure(&manifest).contains("UR-6: the primary root is not this device's"), "{}", failure(&manifest));
}

// ---------------------------------------------------------------- the Authority (UR-7, UR-8)

fn authority(config: FakeConfig, time_source: &str) -> (DeviceAuthority, Arc<FakeDevice>, ClockDomainId) {
    let device = fake(config);
    let clocks = Arc::new(ClockRegistry::new());
    let authority = DeviceAuthority::new(device.clone(), clocks, "internal", time_source, ARGS).unwrap();
    let root = authority.root();
    (authority, device, root)
}

#[test]
fn ur_07_the_authority_paces_to_the_device() {
    let (authority, _, root) = authority(FakeConfig::default(), "internal");
    let time = authority.time();
    let at = TimePoint::new(root, time.now(root).unwrap().ticks + ms(30));
    time.schedule(at, Box::new(|_| {})).unwrap();
    let begun = Instant::now();
    assert_eq!(authority.next_wakeup(), Some(at));
    assert!(begun.elapsed() >= Wall::from_millis(29), "{:?}", begun.elapsed());
    assert!(time.now(root).unwrap().ticks >= at.ticks);
}

#[test]
fn ur_07_schedule_wakes_a_waiting_next_wakeup() {
    // Every nap is at most 20 ms (UR-7), so a missed wake only makes the waiter late by
    // up to one nap: the instant is 1 ms ahead, and the median lateness of 20 tries
    // tells a woken waiter (well under 1 ms) from one that sleeps out its nap (~10 ms).
    let (authority, _, root) = authority(FakeConfig::default(), "internal");
    let authority = Arc::new(authority);
    let time = authority.time();
    let mut late = Vec::new();
    for _ in 0..20 {
        let far = time.schedule(TimePoint::new(root, time.now(root).unwrap().ticks + ms(1_000)), Box::new(|_| {})).unwrap();
        let waiter = { let a = authority.clone(); std::thread::spawn(move || (a.next_wakeup(), Instant::now())) };
        std::thread::sleep(Wall::from_millis(7));
        let near = TimePoint::new(root, time.now(root).unwrap().ticks + ms(1));
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
    let (authority, _, root) = authority(FakeConfig { faults: vec![FakeFault::StepBack(Wall::from_millis(50), 200)], ..FakeConfig::default() }, "internal");
    let time = authority.time();
    let mut last = time.now(root).unwrap().ticks;
    let until = Instant::now() + Wall::from_millis(400);
    while Instant::now() < until {
        let now = time.now(root).unwrap().ticks;
        assert!(now >= last, "{now} < {last}");
        last = now;
    }
}

#[test]
fn ur_07_a_read_bracketed_wider_than_1_ms_is_discarded() {
    // UR-7: a device read that took 5 ms is not an anchor (Review L, L05).
    let (slow, _, _) = authority(FakeConfig { faults: vec![FakeFault::SlowTimeRead(Wall::ZERO)], ..FakeConfig::default() }, "internal");
    std::thread::sleep(Wall::from_millis(350));
    assert!(slow.discarded_reads() >= 2, "{}", slow.discarded_reads());
    let (quick, _, _) = authority(FakeConfig::default(), "internal");
    std::thread::sleep(Wall::from_millis(350));
    assert_eq!(quick.discarded_reads(), 0);
}

#[test]
fn ur_07_the_time_is_set_at_the_next_pps_with_an_external_source() {
    let (_, device, _) = authority(FakeConfig::default(), "external");
    let calls = device.calls();
    let sources = calls.iter().position(|c| c == "set_sources internal external").unwrap();
    let zero = calls.iter().position(|c| c == "set_time_zero pps").unwrap();
    assert!(sources < zero, "{calls:?}");
}

#[test]
fn ur_07_schedule_accepts_an_instant_already_passed() {
    let (authority, _, root) = authority(FakeConfig::default(), "internal");
    let time = authority.time();
    let fired = time.now(root).unwrap();
    time.schedule(fired, Box::new(|_| {})).unwrap();
    assert_eq!(authority.next_wakeup(), Some(fired));
    std::thread::sleep(Wall::from_millis(6));
    let passed = TimePoint::new(root, time.now(root).unwrap().ticks - ms(1));
    time.schedule(passed, Box::new(|_| {})).unwrap();
    let begun = Instant::now();
    assert_eq!(authority.next_wakeup(), Some(passed));
    assert!(begun.elapsed() < Wall::from_millis(10));
    assert!(matches!(
        time.schedule(TimePoint::new(root, fired.ticks - 1), Box::new(|_| {})),
        Err(ezsdr_kernel::time::TimeError::InPast { .. })
    ));
}

#[test]
fn ur_07_callbacks_run_outside_the_schedule_lock() {
    // MA-29 as KG-3 amends it (the exit review's UR-7 clause): a callback may schedule at
    // its own instant, fired in the same wakeup, and another thread may schedule while a
    // callback runs. `next_wakeup` runs on a thread so that a callback holding the lock
    // fails the test in seconds instead of hanging it.
    let (authority, _device, root) = authority(FakeConfig::default(), "internal");
    let time = authority.time();
    let t = TimePoint::new(root, time.now(root).unwrap().ticks + ms(20));
    let fired = Arc::new(Mutex::new(Vec::new()));
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (inner, log) = (time.clone(), fired.clone());
    time.schedule(t, Box::new(move |at| {
        log.lock().unwrap().push("first");
        let log2 = log.clone();
        inner.schedule(at, Box::new(move |_| log2.lock().unwrap().push("second"))).unwrap();
        started_tx.send(()).unwrap();
        std::thread::sleep(Wall::from_millis(200));
        log.lock().unwrap().push("first ends");
    })).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || done_tx.send(authority.next_wakeup()).unwrap());
    started_rx.recv_timeout(Wall::from_secs(3)).expect("the first callback ran");
    let begun = Instant::now();
    time.schedule(TimePoint::new(root, t.ticks + ms(1_000)), Box::new(|_| {})).unwrap();
    assert!(begun.elapsed() < Wall::from_millis(100), "schedule waited {:?} for the running callback", begun.elapsed());
    let woke = done_rx.recv_timeout(Wall::from_secs(3)).expect("next_wakeup returned");
    assert_eq!(woke, Some(t));
    assert_eq!(*fired.lock().unwrap(), vec!["first", "first ends", "second"]);
}

#[test]
fn ur_07_cancel_wakes_a_waiting_next_wakeup() {
    // As above: without the wake, the waiter returns only when its nap ends.
    let (authority, _, root) = authority(FakeConfig::default(), "internal");
    let authority = Arc::new(authority);
    let time = authority.time();
    let mut late = Vec::new();
    for _ in 0..20 {
        let handle = time.schedule(TimePoint::new(root, time.now(root).unwrap().ticks + ms(1_000)), Box::new(|_| {})).unwrap();
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
    std::thread::sleep(Wall::from_secs(2));
    let ours = time.now(root).unwrap().ticks;
    let theirs = device.time_now().unwrap();
    assert!((ours - theirs).abs() < 20_000, "{} ticks apart", ours - theirs);
}

#[test]
fn ur_08_the_relations_bracket_a_device_read() {
    let (authority, _, root) = authority(FakeConfig::default(), "internal");
    let relations = authority.relations();
    assert_eq!(relations.iter().map(|r| r.target).collect::<Vec<_>>(), [ClockDomainId::HOST_MONOTONIC, ClockDomainId::UTC]);
    for relation in &relations {
        assert_eq!(relation.source, root);
        assert!(relation.uncertainty.ticks >= 0 && relation.uncertainty.ticks <= 10_000_000);
    }
    assert_eq!(relations[0].method, "ezsdr.radio.uhd.host_bracket");
    assert_eq!(relations[1].method, "ezsdr.radio.uhd.host_utc_bracket");
}

// ---------------------------------------------------------------- the profile and the instance (UR-9…UR-11)

fn provider() -> UhdRadio {
    UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x310_ubx())), fake(FakeConfig::default())).unwrap()
}

#[test]
fn ur_09_profile_values_reach_the_capabilities_and_the_envelope_section() {
    let radio = provider();
    let instance = radio.instance();
    let capability = |name: &str| instance.tree.capabilities[&Key::parse(name).unwrap()].clone();
    let one = |v: Value| ezsdr_kernel::spec::CapabilityValue::One { value: v };
    assert_eq!(capability("radio.timing.min_timed_command_lead_ns"), one(Value::Int(5_000_000)));
    assert_eq!(capability("radio.timing.startup_latency_ns"), one(Value::Int(2_000_000_000)));
    assert_eq!(capability("radio.timing.command_queue_depth"), one(Value::Int(16)));
    assert_eq!(capability("radio.tx.repeat_max_samples"), one(Value::Int(4_294_967_295)));
    assert_eq!(capability("radio.tx.repeat_align_samples"), one(Value::Int(1)));
    assert_eq!(capability("radio.rx.block_len"), one(Value::Int(2_000)));
    assert_eq!(capability("radio.perf.wire_bytes_per_sample"), one(Value::Int(4)));
    assert_eq!(capability("radio.tx.path_delay_samples"), one(Value::Int(45)));
    let envelope = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.usrp.envelope").unwrap()];
    assert_eq!(envelope["profile"]["name"], "x310-ubx");
    assert_eq!(envelope["timing"]["min_timed_command_lead_ns"], 5_000_000);
    assert_eq!(envelope["performance"]["rx_bytes_per_s"], 1_000_000_000);
    assert_eq!(instance.sections[&Namespace::parse("ezsdr.radio.uhd.usrp.device").unwrap()]["fake"], true);
}

#[test]
fn ur_10_instance() {
    let radio = provider();
    let instance = radio.instance();
    assert!(!instance.driving.stepped);
    assert_eq!(instance.min_command_lead, Some(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000)));
    assert!(instance.arm_after.is_empty());
    assert_eq!(instance.profile.as_ref().unwrap().name, "x310-ubx");
    assert_eq!(instance.id, ResourceId::parse("usrp").unwrap());
}

fn request(pairs: &[(&str, Value)]) -> ezsdr_kernel::module_api::Requested {
    ezsdr_kernel::module_api::Requested {
        resource: ResourceId::parse("usrp").unwrap(),
        constraints: pairs.iter().map(|(k, v)| (Key::parse(k).unwrap(), Constraint::Eq { value: v.clone() })).collect(),
    }
}

#[test]
fn ur_11_coerce_is_the_descriptions() {
    let radio = provider();
    let report = radio.coerce(&request(&[("radio.rx.sample_rate_hz", Value::Num(19.5e6))])).unwrap();
    assert_eq!(report.applied[&Key::parse("radio.rx.sample_rate_hz").unwrap()], Value::Num(20e6));
    assert_eq!(report.coercions.len(), 1);
    assert!(!radio.coerce(&request(&[("radio.rx.frequency_hz", Value::Num(7e9))])).unwrap().rejected.is_empty());
    assert!(!radio.coerce(&request(&[("radio.rx.channels", Value::Int(4))])).unwrap().rejected.is_empty());
}

// ---------------------------------------------------------------- prepare, arm, start (UR-12…UR-16)

struct NoActions;
impl ActionReceiver for NoActions {
    fn recv(&self) -> Option<Action> {
        None
    }
}
struct NoSubmit;
impl ActionSubmitter for NoSubmit {
    fn submit(&self, _: Action) -> Result<ActionId, Vec<ezsdr_kernel::binding::Violation>> {
        Ok(ActionId(0))
    }
}

/// A unit `prepare` of the UHD Provider with a hand-built context.
fn direct_prepare(radio: &mut UhdRadio, device: Arc<FakeDevice>, class: ExecutionClass, links: Vec<AttachedPort>, constraints: &[(&str, Value)]) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
    let clocks = Arc::new(ClockRegistry::new());
    let authority = DeviceAuthority::new(device, clocks.clone(), "internal", "internal", ARGS).unwrap();
    let mut registry = ModuleRegistry::new();
    let (mut checks, mut kinds) = (AdmissionCheckRegistry::new(), EventKindRegistry::with_kernel_kinds());
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
    let pairs: Vec<_> = ["usrp", "usrp/rx", "usrp/tx"].iter().flat_map(|s| kinds.kinds().into_iter().map(move |k| (ResourceId::parse(s).unwrap(), k))).collect();
    let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 1024, &Policy::default()));
    let ctx = PrepareContext {
        run: RunId::from_string("unit".to_owned()),
        class,
        time: authority.time(),
        clocks,
        events,
        actions: Arc::new(NoActions),
        actions_out: Arc::new(NoSubmit),
        environment: Arc::new(BTreeMap::new()),
        inputs: Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new()),
        links,
        components: BTreeMap::new(),
        host_budget: RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap(),
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
    AttachedPort { component: Ident::parse("radio").unwrap(), port: Ident::parse("rx").unwrap(), endpoint: Endpoint::StreamOut(link) }
}

/// A queue the test pushes Actions into, as the coordinator dispatches them.
#[derive(Default)]
struct Pushed(Mutex<std::collections::VecDeque<Action>>);
impl ActionReceiver for Pushed {
    fn recv(&self) -> Option<Action> {
        self.0.lock().unwrap().pop_front()
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
    fn new(config: FakeConfig, constraints: &[(&str, Value)]) -> Direct {
        Direct::with_links(config, constraints, Vec::new())
    }

    fn with_links(config: FakeConfig, constraints: &[(&str, Value)], links: Vec<AttachedPort>) -> Direct {
        Direct::build(config, constraints, links, json!({}))
    }

    /// With selector keys beside `args` (UR-5).
    fn build(config: FakeConfig, constraints: &[(&str, Value)], links: Vec<AttachedPort>, selector: Json) -> Direct {
        let device = fake(config);
        let clocks = Arc::new(ClockRegistry::new());
        let authority = DeviceAuthority::new(device.clone(), clocks.clone(), "internal", "internal", ARGS).unwrap();
        let (time, root) = (authority.time(), authority.root());
        let mut registry = ModuleRegistry::new();
        let (mut checks, mut kinds) = (AdmissionCheckRegistry::new(), EventKindRegistry::with_kernel_kinds());
        ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
        let pairs: Vec<_> = ["usrp", "usrp/rx", "usrp/tx"].iter().flat_map(|s| kinds.kinds().into_iter().map(move |k| (ResourceId::parse(s).unwrap(), k))).collect();
        let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()));
        let queue = Arc::new(Pushed::default());
        let ctx = PrepareContext {
            run: RunId::from_string("direct".to_owned()),
            class: ExecutionClass::HardwareInLoop,
            time: time.clone(),
            clocks: clocks.clone(),
            events: events.clone(),
            actions: queue.clone(),
            actions_out: Arc::new(NoSubmit),
            environment: Arc::new(BTreeMap::new()),
            inputs: Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new()),
            links,
            components: BTreeMap::new(),
            host_budget: RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap(),
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
        let mut radio = UhdRadio::from_binding(&binding(keys, Some(x310_ubx())), device.clone()).unwrap();
        radio.prepare(&fragment, ctx).unwrap();
        radio.arm().unwrap();
        let t0 = time.now(root).unwrap().ticks + ms(2_000) + ms(1);
        radio.start(Some(TimePoint::new(root, t0))).unwrap();
        time.wait_until(TimePoint::new(root, t0 + ms(1))).unwrap();
        Direct { radio, device, queue, events, time, root, clocks, delivered: Vec::new(), _authority: authority }
    }

    fn now(&self) -> i64 {
        self.time.now(self.root).unwrap().ticks
    }

    fn push(&self, action: Action) {
        self.queue.0.lock().unwrap().push_back(action);
    }

    fn update(&self, key: &str, value: Value, class: ezsdr_kernel::module_api::UpdateClass, at: Option<i64>) {
        self.push(Action::UpdateParameter {
            target: ResourceId::parse("usrp").unwrap(),
            key: Key::parse(key).unwrap(),
            value,
            class,
            at: at.map(|t| ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(self.root, t))),
        });
    }

    fn settle(&mut self, wall: Wall) -> Vec<Event> {
        std::thread::sleep(wall);
        self.delivered.extend(self.events.drain());
        self.delivered.clone()
    }

    fn of(&mut self, name: &str) -> Vec<Event> {
        self.delivered.extend(self.events.drain());
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
    let three = direct_prepare(&mut radio, device, ExecutionClass::HardwareInLoop, Vec::new(), &[("radio.rx.channels", Value::Int(3))]).unwrap_err();
    assert_eq!(three.kind, ModuleErrorKind::Rejected);
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
    assert_eq!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Prepare } });
    let manifest = run.finish();
    assert!(failure(&manifest).contains("UR-12: the device applied 19900000 S/s for the claimed 20000000"), "{}", failure(&manifest));
}

#[test]
fn ur_12_every_advertised_rate_is_accepted() {
    // UR-9 advertises 200 MHz / N for N in 1…512; UR-12 accepts each (Review L, P0-2).
    let ezsdr_radio::device::Grid::Values(rates) = ezsdr_radio_uhd::profile::Profile::X310Ubx.description(2_000).rates else { panic!("a Values grid") };
    assert_eq!(rates.len(), 512);
    for rate in &rates {
        let n = ezsdr_radio_uhd::decimation(200_000_000, *rate).unwrap_or_else(|| panic!("{rate} refused"));
        assert_eq!(200_000_000.0 / n as f64, *rate);
    }
    // One that is not on the grid, and one past it.
    assert_eq!(ezsdr_radio_uhd::decimation(200_000_000, 19.5e6), None);
    assert_eq!(ezsdr_radio_uhd::decimation(200_000_000, 200e6 / 513.0), None);
    // 200 MHz / 3 prepares, captures, and is reached by a cold switch as well.
    let dir = TempDir::new();
    let manifest = captured(spec_run(&receive_spec(1, 200e6 / 3.0, 1e9, Some(1_000)), &profile(&dir, json!({}), json!({}), false), fake(FakeConfig::default()), BTreeMap::new()));
    assert_eq!(capture_of(&manifest, "rec").size_bytes, 8_000, "{}", failure(&manifest));
    assert_eq!(manifest.clocks.sample_clocks[0].root_ticks_per_tick.num(), 3);
    let dir = TempDir::new();
    let mut run = session(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()));
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::Num(200e6 / 7.0)), None).unwrap()));
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
    assert_eq!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm } });
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
    let entry = run.submit(set("radio.tx.channels", Value::Int(1)), None).unwrap();
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
    let begun = Instant::now();
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::Num(2e6)), None).unwrap()));
    assert!(admitted(&run.submit(set("radio.rx.gain_db", Value::Num(3.0)), None).unwrap()));
    assert!(begun.elapsed() < Wall::from_millis(30), "{:?}", begun.elapsed());
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
    assert_eq!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm } });
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
        assert_eq!(record.origin.ticks % 200, 0, "{record:?}");
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
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { kind: kind(EventKind::DEVICE_LOST) } });
    assert!(section(&manifest, "rejected").as_array().unwrap().iter().any(|r| r["thread"] == "uhd-rx"));
    assert_marked_lost_before_closed(&device);
}

fn wedged(dir: &TempDir) -> (Manifest, Arc<FakeDevice>, Wall) {
    let device = fake(FakeConfig { faults: vec![FakeFault::TxBlocks], ..FakeConfig::default() });
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
    assert_eq!(map.first.ticks, 0);
    assert!(map.gaps.is_empty(), "{:?}", map.gaps);
    let origin = manifest.clocks.sample_clocks[0].origin.ticks;
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
    let t0 = manifest.clocks.sample_clocks[0].origin.ticks;
    assert_eq!(payload.requested.ticks, t0);
    assert!(payload.applied.ticks - t0 >= ms(50) && payload.applied.ticks % 200 == 0, "{payload:?}");
    // TM-13b as KG-14 amends it: the origin stays T0, so the first sample the capture
    // holds is at the restart's index, after the first block's GAP_BEFORE.
    let map = &capture_of(&manifest, "rec").continuity[0];
    assert_eq!(map.first.ticks, (payload.applied.ticks - t0) / 200);
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

#[test]
fn ur_17_an_overlap_trimmed_to_nothing_is_dropped() {
    let (manifest, _dir) = receive_run(FakeConfig { faults: vec![FakeFault::Repeat(Wall::from_millis(2_050))], ..FakeConfig::default() }, 1, 200_000);
    assert!(section(&manifest, "stats")["rx_overlapping"].as_i64().unwrap() >= 1);
    assert!(capture_of(&manifest, "rec").continuity[0].gaps.is_empty());
}

fn gap_run(fault: FakeFault, channels: i64) -> (Manifest, TempDir) {
    receive_run(FakeConfig { faults: vec![fault], ..FakeConfig::default() }, channels, 300_000)
}

#[test]
fn ur_18_an_overrun_is_rm_17s_gap() {
    let (manifest, _dir) = gap_run(FakeFault::Overflow(Wall::from_millis(2_100)), 1);
    let map = &capture_of(&manifest, "rec").continuity[0];
    assert_eq!(map.gaps.len(), 1, "{:?}", map.gaps);
    assert_eq!(map.gaps[0].cause, GapCause::OverflowRestart {});
    assert_eq!(map.gaps[0].lost, Some(map.gaps[0].len));
    let events = events_of(&manifest, "radio.RX_OVERFLOW");
    assert_eq!(events.len(), 1);
    let payload = RxOverflowPayload::from_payload(&events[0].payload).unwrap();
    assert_eq!(payload.cause, RxOverflowCause::Overrun);
    assert_eq!(payload.lost, map.gaps[0].len);
    assert_eq!(payload.restart_gap_ns, payload.lost as i64 * 1_000);
}

#[test]
fn ur_18_a_sequence_error_is_rm_18s_gap() {
    let (manifest, _dir) = gap_run(FakeFault::SequenceError(Wall::from_millis(2_100)), 1);
    let map = &capture_of(&manifest, "rec").continuity[0];
    assert_eq!(map.gaps[0].cause, GapCause::SequenceError {});
    let payload = RxOverflowPayload::from_payload(&events_of(&manifest, "radio.RX_OVERFLOW")[0].payload).unwrap();
    assert_eq!((payload.cause, payload.restart_gap_ns), (RxOverflowCause::Sequence, 0));
}

#[test]
fn ur_18_an_unannounced_jump_is_a_seq_discontinuity() {
    let (manifest, _dir) = gap_run(FakeFault::Jump(Wall::from_millis(2_100)), 1);
    let map = &capture_of(&manifest, "rec").continuity[0];
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].cause, GapCause::SequenceError {});
    assert!(events_of(&manifest, "radio.RX_OVERFLOW").is_empty());
}

#[test]
fn ur_19_an_alignment_error_is_a_whole_stream_gap() {
    let (manifest, _dir) = gap_run(FakeFault::Alignment(Wall::from_millis(2_100)), 2);
    let map = &capture_of(&manifest, "rec").continuity[0];
    assert_eq!(map.gaps.len(), 1, "{:?}", map.gaps);
    assert!(map.channel_gaps.is_empty(), "no per-channel ALIGNMENT (UR-19)");
    let events = events_of(&manifest, "radio.ALIGNMENT_ERROR");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["lost"], map.gaps[0].len);
}

#[test]
fn ur_20_rx_overflow_is_emitted_on_the_hot_path() {
    let (manifest, _dir) = gap_run(FakeFault::Overflow(Wall::from_millis(2_100)), 1);
    let event = &events_of(&manifest, "radio.RX_OVERFLOW")[0];
    assert!(event.payload.is_array(), "the hot path's byte array: {}", event.payload);
    assert_eq!(event.source, ResourceId::parse("usrp/rx").unwrap());
    assert!(RxOverflowPayload::from_payload(&event.payload).is_ok());
}

// ---------------------------------------------------------------- transmit (UR-21…UR-23)

/// A Session with one transmit channel enabled and T0 passed.
fn tx_session(config: FakeConfig) -> (RunHandle, Arc<FakeDevice>, TempDir) {
    let dir = TempDir::new();
    let device = fake(config);
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device.clone());
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.tx.channels", Value::Int(1)), None).unwrap()));
    (run, device, dir)
}

/// The transmit sample `lead` root ticks from now, on the running transmit clock.
fn tx_at(run: &RunHandle, lead: i64) -> TimePoint {
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream == ResourceId::parse("usrp/tx").unwrap() && r.ended_at.is_none()).unwrap();
    let n = clock.root_ticks_per_tick.num() as i64;
    TimePoint::new(clock.domain, (run.now().ticks + lead - clock.origin.ticks + n - 1).div_euclid(n))
}

fn send_at(run: &mut RunHandle, verb_name: &str, at: Option<TimePoint>, samples: &[(f32, f32)]) -> ezsdr_kernel::session::LogEntry {
    let (bytes, _) = waveform_of(samples);
    run.submit(verb(verb_name, "radio/tx", at, &[]), Some(&bytes)).unwrap()
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
    let _ = run.submit(verb("send", "radio/tx", None, &[]), Some(&odd)).unwrap();
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
    assert!(events_of(&manifest, "radio.COMMAND_REJECTED").len() >= 3);
}

#[test]
fn ur_21_a_burst_before_start_up_is_refused() {
    let dir = TempDir::new();
    let mut run = session(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()));
    assert!(admitted(&run.submit(set("radio.tx.channels", Value::Int(1)), None).unwrap()));
    let _ = send(&mut run, "send", None, &tone(10));
    let manifest = run.finish();
    assert!(rejections(&manifest).iter().any(|r| r.starts_with("UR-21: the burst begins before the device's start-up")), "{:?}", rejections(&manifest));
}

#[test]
fn ur_21_late_policies_on_the_device_lead() {
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "send", Some(ms(1)), &tone(10));
    let _ = send(&mut run, "start_repeat", Some(ms(1) / 2), &tone(10));
    wait(&mut run, ms(50));
    let _ = run.submit(SessionAction::Stop { target: Some(ResourceId::parse("radio/tx").unwrap()) }, None);
    wait(&mut run, ms(20));
    // On time with a margin that survives the test binary's parallel load (Review M,
    // P1-B); the device lead's own boundary is the bench's to measure (B8).
    let _ = send(&mut run, "send", Some(ms(20)), &tone(10));
    wait(&mut run, ms(60));
    let manifest = run.finish();
    let outcomes: Vec<_> = time_errors(&manifest).iter().map(|p| p.outcome).collect();
    // Booking moved the repeat to receipt + 2 ms; ownership transfer consumes
    // some of that lead, so first dispatch moves it again instead of lying OnTime.
    assert_eq!(outcomes, [TimeErrorOutcome::Drop, TimeErrorOutcome::SendAsap,
        TimeErrorOutcome::SendAsap], "{outcomes:?}");
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
    let (mut run, device, _dir) = tx_session(FakeConfig::default());
    assert!(admitted(&run.submit(set("radio.tx.sample_rate_hz", Value::Num(2e6)), None).unwrap()));
    let entry = send(&mut run, "start_repeat", None, &tone(200));
    assert!(admitted(&entry), "{entry:?}");
    wait(&mut run, ms(300));
    let manifest = run.finish();
    let errors = time_errors(&manifest);
    assert_eq!(errors.iter().map(|p| p.outcome).collect::<Vec<_>>(), [TimeErrorOutcome::SendAsap]);
    let e2 = tx_clock_origin(&manifest, 1);
    let record = &bursts(&manifest)[0];
    assert_eq!(record.target.ticks, 0, "moved to the new clock's origin {e2}");
    assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError"), "{:?}", device.calls());
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
            "action": { "kind": "update_parameter", "target": serde_json::to_value(ResourceId::parse("sink/rec").unwrap()).unwrap(),
                        "key": "sink.capture_samples", "value": 4_000, "class": "block_boundary" }
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
    assert!(calls(&device, "tx_send").last().unwrap().ends_with("eob=true"), "{:?}", calls(&device, "tx_send"));
    assert_eq!(bursts(&manifest)[0].end, BurstEnd::Eob);
    assert_eq!(bursts(&manifest)[0].samples, 3_000);
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
    assert_eq!(records[0].target.ticks + records[0].samples as i64, records[1].target.ticks);
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
    let stop_at = run.now().ticks;
    let _ = run.submit(SessionAction::Stop { target: Some(ResourceId::parse("radio/tx").unwrap()) }, None);
    wait(&mut run, ms(40));
    let manifest = run.finish();
    let record = &bursts(&manifest)[0];
    assert_eq!(record.end, BurstEnd::Stop);
    let origin = tx_clock_origin(&manifest, 0);
    let end = origin + (record.target.ticks + record.samples as i64) * 200;
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
        assert!(admitted(&run.submit(set("radio.rx.frequency_hz", Value::Num(1e9 + f64::from(i))), None).unwrap()));
        wait(&mut run, ms(5));
    }
    assert_eq!(late_commands(&run.finish()), 0);
}

#[test]
fn ur_24_hardware_timed_updates() {
    use ezsdr_kernel::module_api::UpdateClass::HardwareTimed;
    let mut direct = Direct::new(FakeConfig { command_queue: 64, ..FakeConfig::default() }, &[("radio.tx.channels", Value::Int(1))]);
    let now = direct.now();
    // 1 ms ahead is inside the 2 ms device lead: late (UC-6 as KG-8 amends it).
    direct.update("radio.rx.frequency_hz", Value::Num(2.1e9), HardwareTimed, Some(now + ms(1)));
    direct.update("radio.tx.gain_db", Value::Num(3.0), HardwareTimed, Some(now + ms(40)));
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
        direct.update("radio.rx.gain_db", Value::Num(f64::from(i % 30)), HardwareTimed, Some(far + i64::from(i)));
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
    direct.update("radio.rx.frequency_hz", Value::Num(2.1e9), HardwareTimed, Some(now + ms(500)));
    direct.update("radio.rx.frequency_hz", Value::Num(2.2e9), HardwareTimed, Some(now + ms(20)));
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

#[test]
fn ur_24_the_device_queue_order_is_the_effective_order() {
    // UC-2 for the device's in-order queue: a command whose instant precedes one already
    // released is applied at the released one's, with LATE_COMMAND, so the instants the
    // device is handed never decrease (Review L, L07). Three commands inside the release
    // window, latest first.
    use ezsdr_kernel::module_api::UpdateClass::HardwareTimed;
    let mut late = 0;
    for _ in 0..10 {
        let mut direct = Direct::new(FakeConfig::default(), &[]);
        let now = direct.now();
        for (i, us) in [2_900i64, 2_600, 2_300].into_iter().enumerate() {
            direct.update("radio.rx.frequency_hz", Value::Num(2.0e9 + i as f64 * 1e6), HardwareTimed, Some(now + us * 200));
        }
        direct.settle(Wall::from_millis(30));
        let ats: Vec<i64> = direct
            .device
            .calls()
            .iter()
            .filter(|c| c.starts_with("apply rx 0 rate=- freq=") && !c.contains("at=-"))
            .map(|c| c.split("at=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap())
            .collect();
        assert_eq!(ats.len(), 3, "{ats:?}");
        assert!(ats.windows(2).all(|w| w[0] <= w[1]), "handed to the device out of order: {ats:?}");
        late += direct.of("radio.LATE_COMMAND").len();
        let _ = direct.finish();
    }
    assert!(late > 0, "the late case was never reached");
}

#[test]
fn ur_25_the_switch_applies_the_configuration_in_effect_at_e2() {
    // UR-25 (Review K, N-P2-9): a retune held for an instant before e₂ is part of the
    // configuration the switch applies (Review L, L09).
    use ezsdr_kernel::module_api::UpdateClass::{Cold, HardwareTimed};
    let mut direct = Direct::with_links(FakeConfig::default(), &[], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    let now = direct.now();
    direct.update("radio.rx.frequency_hz", Value::Num(2.1e9), HardwareTimed, Some(now + ms(320)));
    direct.update("radio.rx.sample_rate_hz", Value::Num(2e6), Cold, Some(now + ms(300)));
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
    let radio = serde_json::to_value(ResourceId::parse("radio").unwrap()).unwrap();
    let mut spec = receive_spec(1, 1e6, 1e9, None);
    spec["schedule"] = json!([{
        "at": { "clock": "radio", "offset_ticks": 20_000 },
        "action": { "kind": "update_parameter", "target": radio, "key": "radio.rx.frequency_hz", "value": 2.3e9, "class": "hardware_timed" }
    }, {
        "at": { "clock": "radio", "offset_ticks": 19_500 },
        "action": { "kind": "stop", "target": radio }
    }]);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new());
    let t0 = run.start_instant().unwrap();
    run.advance_to(TimePoint::new(t0.domain, t0.ticks + ms(100))).unwrap();
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
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let mut spec = receive_spec(1, 1e6, 1e9, None);
    spec["schedule"] = json!([{
        "at": { "clock": "radio", "offset_ticks": 1_000_000 },
        "action": { "kind": "update_parameter", "target": serde_json::to_value(ResourceId::parse("radio").unwrap()).unwrap(),
                    "key": "radio.rx.frequency_hz", "value": 2.3e9, "class": "hardware_timed" }
    }, {
        "at": { "clock": "radio", "offset_ticks": 10_000 },
        "action": { "kind": "stop", "target": serde_json::to_value(ResourceId::parse("radio").unwrap()).unwrap() }
    }]);
    let mut run = spec_run(&spec, &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new());
    let t0 = run.start_instant().unwrap();
    run.advance_to(TimePoint::new(t0.domain, t0.ticks + ms(1_100))).unwrap();
    let manifest = run.finish();
    assert!(calls(&device, "apply rx 0 rate=- freq=2300000000").is_empty(), "{:?}", device.calls());
    assert!(section(&manifest, "applied").as_array().unwrap().iter().any(|r| r["cancelled"] == "Stop"));
}

#[test]
fn ur_25_a_cold_rate_change_starts_a_new_clock_on_its_lattice() {
    let dir = TempDir::new();
    let mut run = session(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()));
    past_t0(&mut run, ms(1));
    let capture_at = after(&run, ms(10));
    assert!(admitted(&run.submit(verb("capture", "sink/rec", Some(capture_at), &[("sink.capture_samples", Value::Int(200_000))]), None).unwrap()));
    wait(&mut run, ms(20));
    let entry = run.submit(set("radio.rx.sample_rate_hz", Value::Num(19.5e6)), None).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    let horizon = after(&run, ms(5_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let manifest = run.finish();
    let clocks: Vec<_> = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).collect();
    assert_eq!(clocks.len(), 2);
    let e1 = clocks[0].ended_at.unwrap().ticks;
    let e2 = clocks[1].origin.ticks;
    assert_eq!(e1 % 200, 0);
    assert_eq!(e2 % 10, 0, "20 Msps is 10 root ticks per sample");
    assert!(e2 - e1 >= ms(50));
    let capture = capture_of(&manifest, "rec");
    assert_eq!(capture.continuity.len(), 2, "{:?}", capture.continuity);
    assert!(capture.continuity.iter().all(|m| m.gaps.is_empty()));
}

#[test]
fn ur_25_the_old_stream_is_stopped_untimed_at_e1() {
    // UR-25: the X3x0 ignores a stop's time (design-notes §11 F1), so uhd-rx stops the
    // stream untimed when its samples reach e₁, never timed, and does not reopen a
    // streamer whose channel count did not change.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device.clone());
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::Num(2e6)), None).unwrap()));
    let capture_at = after(&run, ms(100));
    assert!(admitted(&run.submit(verb("capture", "sink/rec", Some(capture_at), &[("sink.capture_samples", Value::Int(20_000))]), None).unwrap()));
    let horizon = after(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    // Before the Run's own stop, which stops the stream untimed as well (Review M, P1-A).
    let calls = device.calls();
    let manifest = run.finish();
    assert_eq!(calls.iter().filter(|c| c.starts_with("rx_open")).count(), 1, "one streamer: {calls:?}");
    assert!(calls.iter().any(|c| c == "rx_stop now"), "{calls:?}");
    assert!(calls.iter().all(|c| !c.starts_with("rx_stop ") || c == "rx_stop now"), "a timed receive stop: {calls:?}");
    let capture = capture_of(&manifest, "rec");
    assert!(capture.continuity.iter().all(|m| m.gaps.is_empty()), "{:?}", capture.continuity);
    assert_eq!(capture.size_bytes, 160_000);
}

#[test]
fn ur_25_a_cold_receive_change_delivers_every_sample_before_e1() {
    // design-notes §11 F1 (the bench's hw_b8_cold_change_capture): a capture across a
    // `cold` receive change holds the old clock's samples up to e₁ and the new clock's
    // from e₂, with no LATE_COMMAND between them.
    let dir = TempDir::new();
    let device = fake(FakeConfig::default());
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device.clone());
    past_t0(&mut run, ms(1));
    let capture_at = after(&run, ms(10));
    assert!(admitted(&run.submit(verb("capture", "sink/rec", Some(capture_at), &[("sink.capture_samples", Value::Int(200_000))]), None).unwrap()));
    wait(&mut run, ms(60));
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::Num(2e6)), None).unwrap()));
    let horizon = after(&run, ms(5_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let manifest = run.finish();
    let clocks: Vec<_> = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).collect();
    assert_eq!(clocks.len(), 2, "{clocks:?}");
    let (old, new) = (clocks[0], clocks[1]);
    let e1_k = (old.ended_at.unwrap().ticks - old.origin.ticks) / old.root_ticks_per_tick.num() as i64;
    let capture = capture_of(&manifest, "rec");
    assert_eq!(capture.continuity.len(), 2, "{:?}", capture.continuity);
    assert_eq!(capture.continuity[0].end.ticks, e1_k, "the old clock's samples end at e₁: {:?}", capture.continuity[0]);
    assert_eq!(capture.continuity[1].first.ticks, 0, "the new clock's start at e₂ ({:?}): {:?}", new.origin, capture.continuity[1]);
    assert!(capture.continuity.iter().all(|m| m.gaps.is_empty()), "{:?}", capture.continuity);
    assert!(events_of(&manifest, "radio.LATE_COMMAND").is_empty(), "{:?}", events_of(&manifest, "radio.LATE_COMMAND"));
}

#[test]
fn ur_25_a_stream_silent_before_e1_switches_before_e2() {
    // design-notes §11 F2: a stream that stops yielding before e₁ is ended within a block
    // of e₁, not at uhd-rx's next 100 ms receive timeout, so that the timed start at e₂
    // is on time. The fake falls silent 105 ms before e₁: its timeouts come 100 ms apart.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    // Direct starts its Run ~2 s after the fake's time zero (the start-up lead).
    let silent = ms(2_300);
    let mut direct = Direct::with_links(
        FakeConfig { faults: vec![FakeFault::Silence(Wall::from_millis(2_300))], ..FakeConfig::default() },
        &[],
        vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)],
    );
    assert!(direct.now() < silent - ms(150), "the Run started late: {}", direct.now());
    direct.update("radio.rx.sample_rate_hz", Value::Num(2e6), Cold, Some(silent + ms(105)));
    let until = silent + ms(300) - direct.now();
    direct.settle(Wall::from_nanos((until * 5) as u64));
    let late = direct.of("radio.LATE_COMMAND");
    let direct_calls = direct.device.calls();
    // The bounded wait never goes under 1 ms (UHD truncates the timeout to whole ms).
    assert!(direct.device.min_recv_timeout() >= Wall::from_millis(1), "{:?}", direct.device.min_recv_timeout());
    let instance = direct.finish();
    let timing = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.usrp.timing").unwrap()];
    let switch = timing.as_array().unwrap().iter().find(|r| r["what"] == "rx_switch").cloned().unwrap_or_else(|| panic!("no switch: {timing}"));
    let e2 = switch["e2"].as_i64().unwrap();
    assert!(switch["at"].as_i64().unwrap() < e2, "switched after e₂: {switch}");
    // Stopped untimed and recorded, before the new stream's start.
    let calls = direct_calls;
    let stop = calls.iter().position(|c| c == "rx_stop now").unwrap_or_else(|| panic!("{calls:?}"));
    let start = calls.iter().position(|c| *c == format!("rx_start {e2}")).unwrap_or_else(|| panic!("{calls:?}"));
    assert!(stop < start, "{calls:?}");
    let applied = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.usrp.applied").unwrap()];
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
    let mut direct = Direct::with_links(FakeConfig::default(), &[("radio.rx.channels", Value::Int(0))], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    let at = direct.now() + ms(300);
    direct.update("radio.rx.channels", Value::Int(1), Cold, Some(at));
    direct.settle(Wall::from_millis(30));
    let records: Vec<_> = direct.clocks.sample_clock_records().into_iter().filter(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).collect();
    assert_eq!(records.len(), 1, "{records:?}");
    let origin = records[0].origin.ticks;
    assert!(origin >= at && origin < at + 200, "origin {origin}, asked {at}");
    assert!(direct.device.calls().iter().any(|c| *c == format!("rx_start {origin}")), "{:?}", direct.device.calls());
    assert!(direct.of("radio.COMMAND_REJECTED").is_empty());
    let _ = direct.finish();
}

#[test]
fn ur_25_the_receive_stop_is_recorded_when_it_is_issued() {
    // UR-24, UR-25: no stop reaches the device before e₁ (the X3x0 would stop at once);
    // the untimed stop issued when the samples reach e₁ is recorded in `applied`.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::with_links(FakeConfig::default(), &[], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    let at = direct.now() + ms(400);
    direct.update("radio.rx.sample_rate_hz", Value::Num(2e6), Cold, Some(at));
    direct.settle(Wall::from_millis(300));
    let stops = |d: &Direct| d.device.calls().into_iter().filter(|c| c.starts_with("rx_stop ")).collect::<Vec<_>>();
    assert!(stops(&direct).is_empty(), "issued early: {:?}", stops(&direct));
    direct.settle(Wall::from_millis(300));
    assert_eq!(stops(&direct).first().map(String::as_str), Some("rx_stop now"), "{:?}", stops(&direct));
    let instance = direct.finish();
    let applied = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.usrp.applied").unwrap()];
    let row = applied.as_array().unwrap().iter().find(|r| r["key"] == "rx_stop").cloned().unwrap_or_else(|| panic!("{applied}"));
    let e1 = row["at"]["ticks"].as_i64().unwrap();
    assert!(e1 >= at && e1 < at + 200, "cut at {e1}, asked {at}");
    assert!(row["issued"]["ticks"].as_i64().unwrap() >= e1, "{row}");
}

#[test]
fn ur_25_a_cold_change_the_envelope_refuses_changes_nothing() {
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::new(FakeConfig::default(), &[("radio.rx.channels", Value::Int(2))]);
    let before = direct.device.calls().len();
    // Two channels at 200 Msps need 1.6 GB/s on a 1 GB/s link (RM-7).
    direct.update("radio.rx.sample_rate_hz", Value::Num(200e6), Cold, None);
    direct.settle(Wall::from_millis(30));
    let rejected = direct.of("radio.COMMAND_REJECTED");
    assert_eq!(rejected.len(), 1);
    assert!(rejected[0].payload["reason"].as_str().unwrap().starts_with("RM-7: "), "{rejected:?}");
    assert!(direct.device.calls()[before..].iter().all(|c| !c.starts_with("apply") && !c.starts_with("rx_stop")));
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
    assert!(admitted(&run.submit(set("radio.tx.frequency_hz", Value::Num(2.4e9)), None).unwrap()));
    assert!(admitted(&run.submit(set("radio.tx.gain_db", Value::Num(5.0)), None).unwrap()));
    assert!(admitted(&run.submit(set("radio.rx.frequency_hz", Value::Num(2.4e9)), None).unwrap()));
    wait(&mut run, ms(20));
    assert!(admitted(&run.submit(set("radio.tx.channels", Value::Int(1)), None).unwrap()));
    let calls = device.calls();
    assert!(calls.iter().any(|c| c.starts_with("apply tx 0 rate=1000000 freq=2400000000 gain=5 antenna=TX/RX at=-")), "{calls:?}");
    let wave = tone(1_000);
    let _ = send(&mut run, "start_repeat", Some(ms(20)), &wave);
    wait(&mut run, ms(40));
    let capture_at = after(&run, ms(5));
    assert!(admitted(&run.submit(verb("capture", "sink/rec", Some(capture_at), &[("sink.capture_samples", Value::Int(2_000))]), None).unwrap()));
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
    assert!(admitted(&run.submit(set("radio.tx.sample_rate_hz", Value::Num(2e6)), None).unwrap()));
    let entry = send(&mut run, "start_repeat", None, &tone(100));
    assert!(admitted(&entry), "{entry:?}");
    // Booked before the switch (50 ms after receipt), transmitted after it on the new clock.
    wait(&mut run, ms(150));
    let manifest = run.finish();
    assert!(events_of(&manifest, "radio.COMMAND_REJECTED").is_empty(), "{:?}", events_of(&manifest, "radio.COMMAND_REJECTED"));
    let new = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/tx").unwrap()).nth(1).expect("the new transmit clock");
    assert!(bursts(&manifest).iter().any(|b| b.target.domain == new.domain), "{:?}", bursts(&manifest));
}

#[test]
fn ur_25_a_cold_transmit_change_cancels_the_held_bursts() {
    // UR-25: bursts held on the old clock are refused with COMMAND_REJECTED (Review L, L11).
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "send", Some(ms(300)), &tone(100));
    assert!(admitted(&run.submit(set("radio.tx.sample_rate_hz", Value::Num(2e6)), None).unwrap()));
    wait(&mut run, ms(150));
    let manifest = run.finish();
    let reasons: Vec<_> = events_of(&manifest, "radio.COMMAND_REJECTED").iter().map(|e| e.payload["reason"].clone()).collect();
    assert!(reasons.iter().any(|r| r == "cancelled by a cold change"), "{reasons:?}");
}

#[test]
fn ur_25_a_rate_the_device_does_not_apply_at_the_switch_is_rejected() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::WrongRate { claimed: 20e6, applied: 19.9e6, nth: 0 }], ..FakeConfig::default() });
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device);
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::Num(20e6)), None).unwrap()));
    wait(&mut run, ms(300));
    let manifest = run.finish();
    assert!(rejections(&manifest).iter().any(|r| r.starts_with("UR-12: the device applied 19900000 S/s")), "{:?}", rejections(&manifest));
    let new = manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).nth(1).unwrap();
    assert_eq!(new.ended_at, Some(new.origin));
}

#[test]
fn ur_26_stop_actions() {
    let (mut run, _, _dir) = tx_session(FakeConfig::default());
    let _ = send(&mut run, "start_repeat", Some(ms(10)), &tone(100));
    wait(&mut run, ms(30));
    for target in ["radio/tx", "radio/rx", "radio", "radio/gpio"] {
        let entry = run.submit(SessionAction::Stop { target: Some(ResourceId::parse(target).unwrap()) }, None).unwrap();
        assert!(admitted(&entry), "{target}: {entry:?}");
        wait(&mut run, ms(5));
    }
    wait(&mut run, ms(20));
    let manifest = run.finish();
    assert_eq!(bursts(&manifest)[0].end, BurstEnd::Stop);
    assert!(section(&manifest, "timing").as_array().unwrap().iter().any(|r| r["what"] == "rx_stop"));
    assert!(rejections(&manifest).iter().any(|r| r.starts_with("UR-26: the Stop target is not this device")), "{:?}", rejections(&manifest));
}

/// The root instant just past the capture's last sample, and the Provider's stop.
fn capture_end_and_stop(manifest: &Manifest) -> (i64, i64) {
    let map = &capture_of(manifest, "rec").continuity[0];
    let origin = manifest.clocks.sample_clocks[0].origin.ticks;
    let end = origin + map.end.ticks * 200;
    let stop = section(manifest, "timing").as_array().unwrap().iter().find(|r| r["what"] == "stop").unwrap()["at"].as_i64().unwrap();
    (end, stop)
}

#[test]
fn ur_26_orderly_stop_delivers_the_tail_abort_does_not() {
    // Orderly: samples up to the stop instant plus stop_tail_ns reach the capture.
    let dir = TempDir::new();
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, Some(10_000_000)), &profile(&dir, json!({}), json!({}), false), fake(FakeConfig::default()), BTreeMap::new());
    past_t0(&mut run, ms(100));
    let manifest = run.finish();
    let (end, stop) = capture_end_and_stop(&manifest);
    assert!(end >= stop, "the tail after the stop was delivered: end {end}, stop {stop}");
    // uhd-rx's own stop instant: the capture runs on for stop_tail_ns (1 ms) after it.
    let rx_stop = section(&manifest, "timing").as_array().unwrap().iter().find(|r| r["what"] == "rx_stop").unwrap()["at"].as_i64().unwrap();
    assert!(end + 200 >= rx_stop + ms(1), "the 1 ms tail: end {end}, uhd-rx stopped at {rx_stop}");
    // …counted from the stop instant, not from when uhd-rx got to it (Review M, N-6).
    assert!(end <= stop + ms(1) + 200, "the tail ends 1 ms after the stop instant: end {end}, stop {stop}");
    // Abort (CLOCK_LOST's default): nothing after the stop.
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::Unlocked(Wall::from_millis(2_200))], ..FakeConfig::default() });
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, Some(10_000_000)), &profile(&dir, json!({ "clock_source": "external" }), json!({}), false), device, BTreeMap::new());
    let _ = run.run_until_end(after(&run, ms(5_000)));
    let manifest = run.finish();
    let (end, stop) = capture_end_and_stop(&manifest);
    assert!(end <= stop + ms(1), "an abort discards at once: end {end}, stop {stop}");
}

#[test]
fn ur_26_an_abort_publishes_nothing_after_the_stop_instant() {
    // RM-16: under abort the receive stream ends at the stop instant, with no tail
    // (Review L, L12). The link keeps the newest blocks (drop-oldest). Blocks of 20 ms,
    // so that uhd-rx is almost surely inside a receive wait when the stop begins, and
    // the block it then receives straddles the stop instant.
    let port = attached(ezsdr_kernel::stream::BackPressure::DropOldest);
    let Endpoint::StreamOut(link) = &port.endpoint else { unreachable!() };
    let link = link.clone();
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![port], json!({ "block_len": 20_000 }));
    direct.settle(Wall::from_millis(50));
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Abort).unwrap();
    let mut end = None;
    while let Some(block) = link.receive() {
        let header = block.header();
        end = Some(header.first_sample_time.ticks + i64::from(header.len));
    }
    direct.radio.cleanup();
    let instance = direct.radio.instance().clone();
    let timing = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.usrp.timing").unwrap()];
    let stop = timing.as_array().unwrap().iter().find(|r| r["what"] == "stop").unwrap()["at"].as_i64().unwrap();
    let origin = direct.clocks.sample_clock_records().iter().find(|r| r.stream == ResourceId::parse("usrp/rx").unwrap()).unwrap().origin.ticks;
    let end = origin + end.expect("blocks were published") * 200;
    assert!(end <= stop + 200, "published up to {end}, stopped at {stop}");
    // One stop, recorded once: not again when the tail reaches the cut (Review N, N4).
    let calls = direct.device.calls();
    assert_eq!(calls.iter().filter(|c| c.starts_with("rx_stop")).count(), 1, "{calls:?}");
    let applied = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.usrp.applied").unwrap()];
    assert_eq!(applied.as_array().unwrap().iter().filter(|r| r["key"] == "rx_stop").count(), 1, "{applied}");
}

#[test]
fn ur_26_an_orderly_stop_does_not_restart_the_stream() {
    // RM-16's tail on 20 ms blocks: uhd-rx reaches the stop only after a whole block, too
    // late for a timed stop, so it stops untimed, and a late stop never restarts the stream
    // as a missed start would (Review M, P1-A).
    let port = attached(ezsdr_kernel::stream::BackPressure::DropOldest);
    let mut direct = Direct::build(FakeConfig::default(), &[], vec![port], json!({ "block_len": 20_000 }));
    direct.settle(Wall::from_millis(50));
    let device = direct.device.clone();
    let starts = |calls: &[String]| calls.iter().filter(|c| c.starts_with("rx_start")).count();
    let before = starts(&device.calls());
    direct.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    std::thread::sleep(Wall::from_millis(100));
    assert_eq!(starts(&device.calls()), before, "{:?}", device.calls());
    assert!(direct.of("radio.LATE_COMMAND").is_empty(), "{:?}", direct.of("radio.LATE_COMMAND"));
    direct.radio.cleanup();
    // No timed stop was handed over inside the device lead.
    let instance = direct.radio.instance().clone();
    let timing = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.usrp.timing").unwrap()];
    assert!(timing.as_array().unwrap().iter().all(|r| r["what"] != "rx_stop_late"), "{timing}");
}

#[test]
fn ur_25_enabling_a_draining_stream_is_refused() {
    // 1 → 0 → 1 receive channels inside the restart lead: the old stream still runs to its
    // e₁, so the enable is refused rather than opening a second stream (Review M, N-4).
    let dir = TempDir::new();
    let mut run = session(&profile(&dir, json!({}), json!({}), true), fake(FakeConfig::default()));
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.rx.channels", Value::Int(0)), None).unwrap()));
    assert!(admitted(&run.submit(set("radio.rx.channels", Value::Int(1)), None).unwrap()));
    wait(&mut run, ms(100));
    let manifest = run.finish();
    assert!(rejections(&manifest).iter().any(|r| r.starts_with("UR-25: the stream changed to 0 channels is still draining")), "{:?}", rejections(&manifest));
}

#[test]
fn ur_24_the_queue_counts_a_command_per_channel() {
    // UR-24's 16 device commands, one per channel: with two receive channels the 9th
    // update is refused (Review M, N-7).
    use ezsdr_kernel::module_api::UpdateClass::HardwareTimed;
    let mut direct = Direct::new(FakeConfig { command_queue: 64, ..FakeConfig::default() }, &[("radio.rx.channels", Value::Int(2))]);
    let far = direct.now() + ms(5_000);
    for i in 0..9 {
        direct.update("radio.rx.gain_db", Value::Num(f64::from(i)), HardwareTimed, Some(far + i64::from(i)));
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
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { kind: kind("radio.CLOCK_LOST") } });
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
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { kind: kind(EventKind::DEVICE_LOST) } });
    assert_eq!(events_of(&manifest, EventKind::DEVICE_LOST)[0].source, ResourceId::parse("usrp").unwrap());
    assert_marked_lost_before_closed(&device);
}

#[test]
fn ur_29_a_silent_stream_is_a_lost_device() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::Silence(Wall::from_millis(2_100))], ..FakeConfig::default() });
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device.clone(), BTreeMap::new());
    let begun = Instant::now();
    let result = run.run_until_end(after(&run, ms(8_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    assert!(begun.elapsed() < Wall::from_secs(7), "{:?}", begun.elapsed());
    let manifest = run.finish();
    let lost = &events_of(&manifest, EventKind::DEVICE_LOST)[0];
    assert!(lost.payload["message"].as_str().unwrap().contains("UR-29"));
    // Within 1 to 2 s of the silence, which begins 2.1 s after the time was set (L14).
    let after_silence = lost.time.ticks - ms(2_100);
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
    let after_death = lost[0].time.ticks - ms(2_500);
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
        assert!(manifest.sections.contains_key(&Namespace::parse(&format!("ezsdr.radio.uhd.usrp.{suffix}")).unwrap()), "{suffix}");
    }
    let stats = section(&manifest, "stats");
    for key in ["rx_blocks", "rx_samples", "rx_overflows", "rx_errors", "rx_off_lattice", "rx_overlapping", "link_drops_seen", "tx_bursts", "tx_samples"] {
        assert!(stats[key].is_i64(), "{key}");
    }
    assert!(stats["rx_samples"].as_i64().unwrap() >= 1_000);
}

#[test]
fn ur_31_fidelity_follows_the_device() {
    let (manifest, _dir) = receive_run(FakeConfig::default(), 1, 1_000);
    assert_eq!(manifest.run.fidelity, ezsdr_kernel::module_api::Fidelity::NONE);
    assert_eq!(manifest.run.execution_class, ExecutionClass::HardwareInLoop);
}

// ---------------------------------------------------------------- the fake (UR-33)

#[test]
fn ur_33_the_fake_device_loops_back_what_it_transmits() {
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.rx_open(1).unwrap();
    device.tx_open(1).unwrap();
    let start = device.time_now().unwrap() + ms(20);
    let wave: Vec<[f32; 2]> = (0..500).map(|i| [i as f32 / 1_000.0, 0.5]).collect();
    device.tx_send(&[&wave], Some(start), true, true, Wall::from_secs(1)).unwrap();
    device.rx_start(start).unwrap();
    let mut got = Vec::new();
    while got.len() < 500 {
        if let ezsdr_radio_uhd::RxRecv::Samples { samples, first_tick } = device.rx_recv(100, Wall::from_millis(100)) {
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
    device.rx_open(1).unwrap();
    device.rx_start(device.time_now().unwrap() + ms(1)).unwrap();
    let mut saw_overflow = false;
    while !saw_overflow {
        saw_overflow = matches!(device.rx_recv(1_000, Wall::from_millis(100)), ezsdr_radio_uhd::RxRecv::Overflow { out_of_sequence: false });
    }
    std::thread::sleep(Wall::from_millis(10));
    assert_eq!(device.tx_async(Wall::ZERO).map(|r| r.code), Some(TxCode::Underflow));
    std::thread::sleep(Wall::from_millis(200));
    assert!(device.time_now().unwrap_err().lost);
}

#[test]
fn ur_33_no_document_selects_the_fake_device() {
    for args in ["fake", "type=fake"] {
        assert!(ezsdr_radio_uhd::open(args).is_err(), "{args}");
    }
    let refused = UhdRadio::from_binding(&binding(json!({ "args": ARGS, "fake": true }), Some(x310_ubx())), fake(FakeConfig::default()));
    assert!(refused.err().unwrap().message.contains("unsupported selector key `fake`"));
}

#[test]
fn ur_33_the_fake_device_keeps_the_devices_queues() {
    let device = fake(FakeConfig::default());
    device.set_time_zero(false).unwrap();
    device.tx_open(1).unwrap();
    let wave = vec![[0.1f32, 0.0]; 100];
    let now = device.time_now().unwrap();
    device.tx_send(&[&wave], Some(now + ms(10)), true, false, Wall::from_secs(1)).unwrap();
    // A timed start before the queued samples' end is late and dropped.
    device.tx_send(&[&wave], Some(now + ms(10) + 1), true, true, Wall::from_secs(1)).unwrap();
    let mut codes = Vec::new();
    while let Some(report) = device.tx_async(Wall::ZERO) {
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

// ---------------------------------------------------------------- the rehearsal (UR-34)
// Each step on the fake's two UBX and on the bench's one OBX (`x310-obx`); B3 on one CBX.

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
    assert_eq!(frequency["claimed"], 2.45e9, "the bench frequency is x310-cbx's default");
}

#[test]
fn rehearsal_b3_receive_at_t0_on_one_obx() {
    let manifest = rehearse_receive_at_t0(fake(one_obx()));
    assert_eq!(section(&manifest, "envelope")["profile"]["name"], "x310-obx");
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
// The fake's ways of being as strict as the X300 (UR-33), each pinned on the device
// itself (Review N, T2), and the bench link the receive tests of B1 and B2 use.

/// The receive link as the bench measured it (INFERRED from its stops: 2–4 ms), with the
/// X300's 1 996-sample packets.
fn bench_link(latency_ms: u64) -> FakeConfig {
    FakeConfig { rx_latency: Wall::from_millis(latency_ms), rx_packet: Some(1_996), ..FakeConfig::default() }
}

fn raw_rx(config: FakeConfig, rate: f64) -> (Arc<FakeDevice>, i64) {
    let device = fake(config);
    device.set_time_zero(false).unwrap();
    let settings = ezsdr_radio_uhd::Settings { rate: Some(rate), ..Default::default() };
    device.apply(ezsdr_radio_uhd::Dir::Rx, 0, &settings, None).unwrap();
    device.rx_open(1).unwrap();
    let start = (device.time_now().unwrap() / 200 + 5_000) * 200;
    device.rx_start(start).unwrap();
    (device, start)
}

/// Every block's (first tick, length) until `until` or two timeouts in a row.
fn blocks_until(device: &FakeDevice, until: impl Fn(&[(i64, usize)]) -> bool) -> Vec<(i64, usize)> {
    let mut blocks = Vec::new();
    let mut timeouts = 0;
    let deadline = Instant::now() + Wall::from_secs(5);
    while timeouts < 2 && !until(&blocks) {
        assert!(Instant::now() < deadline, "no end in 5 s: {} blocks, the last {:?}", blocks.len(), blocks.last());
        match device.rx_recv(2_000, Wall::from_millis(50)) {
            ezsdr_radio_uhd::RxRecv::Samples { first_tick, samples } => {
                timeouts = 0;
                blocks.push((first_tick, samples[0].len()));
            }
            ezsdr_radio_uhd::RxRecv::Timeout => timeouts += 1,
            other => panic!("{other:?}"),
        }
    }
    blocks
}

#[test]
fn ur_33_a_timed_receive_stop_stops_the_stream_at_once() {
    // design-notes §11 F1: the X3x0 ignores the stop's time. What was produced before
    // the stop is still delivered; nothing after it.
    let (device, start) = raw_rx(FakeConfig::default(), 1e6);
    let _ = blocks_until(&device, |b| b.last().is_some_and(|(t, n)| t + *n as i64 * 200 >= start + ms(20)));
    let issued = device.time_now().unwrap();
    device.rx_stop(Some(issued + ms(100))).unwrap();
    let tail = blocks_until(&device, |_| false);
    let end = tail.last().map_or(issued, |(t, n)| t + *n as i64 * 200);
    assert!(end <= issued + ms(1), "samples after the stop's issue: up to {end}, issued at {issued}");
}

#[test]
fn ur_33_a_stopped_stream_s_tail_comes_before_the_next_stream() {
    // The bench: 1–2 blocks after an untimed stop. The stop's tail, then the next
    // stream from its start.
    let (device, start) = raw_rx(bench_link(3), 1e6);
    let _ = blocks_until(&device, |b| b.last().is_some_and(|(t, n)| t + *n as i64 * 200 >= start + ms(20)));
    let stopped = device.time_now().unwrap();
    device.rx_stop(None).unwrap();
    let next = (stopped / 200 + 30_000) * 200;
    device.rx_start(next).unwrap();
    let blocks = blocks_until(&device, |b| b.iter().any(|(t, _)| *t >= next));
    let tail: Vec<_> = blocks.iter().filter(|(t, _)| *t < stopped).collect();
    assert!(!tail.is_empty(), "no tail: {blocks:?}");
    let first_new = blocks.iter().position(|(t, _)| *t >= next).expect("the next stream");
    assert!(blocks[..first_new].iter().all(|(t, _)| *t < stopped), "{blocks:?}");
    assert_eq!(blocks[first_new].0, next);
}

#[test]
fn ur_33_a_stopped_stream_s_recv_waits_its_whole_timeout() {
    let (device, _) = raw_rx(FakeConfig::default(), 1e6);
    device.rx_stop(None).unwrap();
    let _ = blocks_until(&device, |_| false);
    let begun = Instant::now();
    assert_eq!(device.rx_recv(2_000, Wall::from_millis(60)), ezsdr_radio_uhd::RxRecv::Timeout);
    assert!(begun.elapsed() >= Wall::from_millis(55), "{:?}", begun.elapsed());
}

#[test]
fn ur_33_a_recv_cut_short_by_a_packet_s_timeout_returns_what_it_has() {
    // UHD's `recv` waits for each packet up to its timeout and returns what it has; it
    // caches no timeout, so the next call waits as usual (`rx_streamer_impl.hpp:25–47`;
    // Review O, N-1). A stream stopped in the middle of a 20 ms request.
    let (device, start) = raw_rx(bench_link(1), 1e6);
    let _ = blocks_until(&device, |b| b.last().is_some_and(|(t, n)| t + *n as i64 * 200 >= start + ms(10)));
    let stopper = device.clone();
    let stop = std::thread::spawn(move || {
        std::thread::sleep(Wall::from_millis(8));
        stopper.rx_stop(None).unwrap();
    });
    let got = device.rx_recv(20_000, Wall::from_millis(5));
    stop.join().unwrap();
    let ezsdr_radio_uhd::RxRecv::Samples { samples, .. } = got else { panic!("{got:?}") };
    assert!(samples[0].len() < 20_000, "{}", samples[0].len());
    let begun = Instant::now();
    assert_eq!(device.rx_recv(2_000, Wall::from_millis(50)), ezsdr_radio_uhd::RxRecv::Timeout);
    assert!(begun.elapsed() >= Wall::from_millis(45), "the next call waited only {:?}", begun.elapsed());
}

/// The device's transmit reports until its time passes `tick` (a burst's acknowledgement
/// comes once its end has played, and a later report behind it).
fn reports_until(device: &FakeDevice, tick: i64) -> Vec<ezsdr_radio_uhd::TxReport> {
    let mut reports = Vec::new();
    while device.time_now().unwrap() <= tick + 200_000 {
        reports.extend(device.tx_async(Wall::from_millis(1)));
    }
    reports.extend(std::iter::from_fn(|| device.tx_async(Wall::ZERO)));
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
        device.tx_open(1).unwrap();
        let wave = vec![[0.1f32, 0.0]; 100];
        let t0 = (device.time_now().unwrap() / 200 + 20_000) * 200;
        device.tx_send(&[&wave], Some(t0), true, !empty, Wall::from_secs(1)).unwrap();
        if empty {
            device.tx_send(&[&[][..]], None, false, true, Wall::from_secs(1)).unwrap();
        }
        device.tx_send(&[&wave], Some(t0 + 101 * 200), true, true, Wall::from_secs(1)).unwrap();
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
        device.tx_open(1).unwrap();
        let wave = vec![[0.1f32, 0.0]; 100];
        let t0 = (device.time_now().unwrap() / 200 + 20_000) * 200;
        device.tx_send(&[&wave], Some(t0), true, true, Wall::from_secs(1)).unwrap();
        let t1 = t0 + (100 + gap) * 200;
        device.tx_send(&[&wave], Some(t1), true, true, Wall::from_secs(1)).unwrap();
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
    device.tx_open(1).unwrap();
    let wave = vec![[0.1f32, 0.0]; 1_000];
    let t0 = (device.time_now().unwrap() / 200 + 5_000) * 200;
    device.tx_send(&[&wave], Some(t0), true, false, Wall::from_secs(1)).unwrap();
    std::thread::sleep(Wall::from_millis(30));
    let mut codes = Vec::new();
    while let Some(report) = device.tx_async(Wall::ZERO) {
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
    device.tx_open(1).unwrap();
    let wave = vec![[0.1f32, 0.0]; 1_000];
    let rest = vec![[0.2f32, 0.0]; 20_000];
    let t0 = (device.time_now().unwrap() / 200 + 100_000) * 200;
    let dry = t0 + 1_000 * 200;
    device.tx_send(&[&wave], Some(t0), true, false, Wall::from_secs(1)).unwrap();
    while device.time_now().unwrap() < dry + ms(5) {
        std::thread::sleep(Wall::from_millis(1));
    }
    device.tx_send(&[&rest], None, false, true, Wall::from_secs(1)).unwrap();
    let resumed = device.time_now().unwrap();
    let scheduled_end = dry + 20_000 * 200;
    let b = scheduled_end + 200;
    device.tx_send(&[&wave], Some(b), true, true, Wall::from_secs(1)).unwrap();
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
    assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::Num(rates[0])), None).unwrap()));
    wait(&mut run, ms(200));
    for i in 0..changes {
        let capture_at = after(&run, ms(10));
        let n = (rates[i % 2] * 0.12) as i64 + (rates[(i + 1) % 2] * 0.08) as i64;
        assert!(admitted(&run.submit(verb("capture", "sink/rec", Some(capture_at), &[("sink.capture_samples", Value::Int(n))]), None).unwrap()));
        wait(&mut run, ms(60));
        assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", Value::Num(rates[(i + 1) % 2])), None).unwrap()));
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
    for artifact in manifest.artifacts.iter().filter(|a| a.id.as_str().starts_with("rec")) {
        let data = read_capture(artifact, 1).remove(0);
        let mut at = 0usize;
        for (m, map) in artifact.continuity.iter().enumerate() {
            let clock = clocks.iter().find(|c| c.domain == map.domain).unwrap();
            let n = clock.root_ticks_per_tick.num() as i64;
            if m == 0 && artifact.continuity.len() > 1 {
                let e1 = (clock.ended_at.unwrap().ticks - clock.origin.ticks) / n;
                if map.end.ticks != e1 {
                    problems.push(format!("{}: the old clock's samples end at {} for e₁ {e1}", artifact.id, map.end.ticks));
                }
            }
            let len = (map.end.ticks - map.first.ticks) as usize;
            for i in 0..len {
                let tick = clock.origin.ticks + (map.first.ticks + i as i64) * n;
                let ramp = ((tick / n).rem_euclid(65_536)) as f32 / 65_536.0;
                if data[at + i].0 != ramp {
                    problems.push(format!("{}: map {m} sample {} is {}, its clock's ramp {ramp}", artifact.id, map.first.ticks + i as i64, data[at + i].0));
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
        let a_end = clock.origin.ticks + (a.ticks + 30_000) * clock.root_ticks_per_tick.num() as i64;
        while run.now().ticks < a_end - ms(8) {
            wait(&mut run, ms(1) / 4);
        }
        let b = TimePoint::new(a.domain, a.ticks + 30_000);
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
            let a_root = clock.origin.ticks + a.ticks * clock.root_ticks_per_tick.num() as i64;
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
    let a_end = clock.origin.ticks + (a.ticks + 30_000) * clock.root_ticks_per_tick.num() as i64;
    while run.now().ticks < a_end - ms(7) {
        wait(&mut run, ms(1) / 4);
    }
    let _ = run.submit(SessionAction::Stop { target: Some(ResourceId::parse("radio/tx").unwrap()) }, None);
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
    direct.update("radio.rx.sample_rate_hz", Value::Num(2e6), Cold, Some(now + ms(5_000)));
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
            let a_root = clock.origin.ticks + a.ticks * n;
            let a_end = a_root + 30_000 * n;
            if late_booking {
                while run.now().ticks < a_end - ms(8) {
                    wait(&mut run, ms(1) / 4);
                }
            }
            let b = TimePoint::new(a.domain, a.ticks + 30_001);
            let entry = send_at(&mut run, "send", Some(b), &tone(1_000));
            assert!(admitted(&entry), "{entry:?}");
            wait(&mut run, ms(80));
            // A host that fell behind may not have handed B over yet: `finish` would cancel it.
            let horizon = Instant::now() + Wall::from_secs(2);
            while tx_sends(&device).iter().filter(|s| s.eob).count() < 2 && Instant::now() < horizon {
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
        let mut direct = Direct::build(config, &[("radio.rx.sample_rate_hz", Value::Num(390_625.0))], vec![port], json!({ "block_len": block_len }));
        direct.settle(Wall::from_millis(200 + phase * step_ms));
        direct.update("radio.rx.sample_rate_hz", Value::Num(2e6), Cold, None);
        direct.settle(Wall::from_millis(600));
        let late = direct.of("radio.LATE_COMMAND");
        let instance = direct.finish();
        let timing = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.usrp.timing").unwrap()];
        let switch = timing.as_array().unwrap().iter().find(|r| r["what"] == "rx_switch").cloned().unwrap_or_else(|| panic!("no switch: {timing}"));
        // UR-25's floor: 50 ms + the longer of a block and a packet + 3 ms (Review Q, TG-Q3).
        let booked = timing.as_array().unwrap().iter().find(|r| r["what"] == "cold_change").cloned().unwrap();
        let floor = ms(50) + block_len.max(packet) as i64 * 512 + ms(3);
        assert!(booked["e1"].as_i64().unwrap() - booked["booked_at"].as_i64().unwrap() >= floor, "block {block_len} packet {packet}: {booked}");
        assert!(switch["at"].as_i64().unwrap() < switch["e2"].as_i64().unwrap(), "block {block_len} packet {packet} phase {phase}: switched after e₂: {switch}");
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
    let target = serde_json::to_value(ResourceId::parse("radio/tx").unwrap()).unwrap();
    spec["schedule"] = json!([
        { "at": { "clock": "radio", "offset_ticks": 10_000 }, "action": { "kind": "tx_burst", "target": target, "waveform": a, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 11_000 }, "action": { "kind": "tx_burst", "target": target, "waveform": b, "repeat": false, "late_policy": "drop_and_flag", "metadata": {} } },
        { "at": { "clock": "radio", "offset_ticks": 10_000 }, "action": { "kind": "update_parameter", "target": serde_json::to_value(ResourceId::parse("sink/rec").unwrap()).unwrap(),
                    "key": "sink.capture_samples", "value": 1_500, "class": "block_boundary" } }
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
    let a_root = clock.origin.ticks + a.ticks * clock.root_ticks_per_tick.num() as i64;
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
        let a_root = clock.origin.ticks + a.ticks * clock.root_ticks_per_tick.num() as i64;
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
    device.tx_open(1).unwrap();
    let wave = vec![[0.1f32, 0.0]; 100];
    let t0 = (device.time_now().unwrap() / 200 + 20_000) * 200;
    device.tx_send(&[&wave], Some(t0), true, true, Wall::from_secs(1)).unwrap();
    device.tx_send(&[&wave], Some(t0 + 100 * 200), true, true, Wall::from_secs(1)).unwrap();
    assert!(device.tx_async(Wall::ZERO).is_none(), "an acknowledgement before the burst played");
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
        let a_root = clock.origin.ticks + a.ticks * clock.root_ticks_per_tick.num() as i64;
        while run.now().ticks < a_root - ms(8) {
            wait(&mut run, ms(1) / 4);
        }
        let b = TimePoint::new(a.domain, a.ticks + 500);
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
        let a_root = clock.origin.ticks + a.ticks * clock.root_ticks_per_tick.num() as i64;
        let a_end = a_root + 30_000 * clock.root_ticks_per_tick.num() as i64;
        while run.now().ticks < a_end - ms(8) {
            wait(&mut run, ms(1) / 4);
        }
        let entry = send_at(&mut run, "send", Some(TimePoint::new(a.domain, a.ticks + 30_000)), &[(-0.5, 0.5); 1_000]);
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
    let a_root = clock.origin.ticks + a.ticks * clock.root_ticks_per_tick.num() as i64;
    let a_end = a_root + 30_000 * clock.root_ticks_per_tick.num() as i64;
    while run.now().ticks < a_end - ms(8) {
        wait(&mut run, ms(1) / 4);
    }
    let entry = send_at(&mut run, "send", Some(TimePoint::new(a.domain, a.ticks + 30_000)), &tone(1_000));
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
    fn rx_open(&self, c: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_open(c) }
    fn rx_start(&self, at: i64) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_start(at) }
    fn rx_packet_samples(&self) -> usize { self.inner.rx_packet_samples() }
    fn rx_stop(&self, at: Option<i64>) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.rx_stop(at) }
    fn rx_recv(&self, n: usize, t: Wall) -> ezsdr_radio_uhd::RxRecv { self.inner.rx_recv(n, t) }
    fn tx_open(&self, c: usize) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.tx_open(c) }
    fn tx_send(&self, s: &[&[ezsdr_radio_uhd::Iq]], at: Option<i64>, sob: bool, eob: bool, t: Wall) -> Result<usize, ezsdr_radio_uhd::DeviceError> { self.inner.tx_send(s, at, sob, eob, t) }
    fn tx_async(&self, t: Wall) -> Option<ezsdr_radio_uhd::TxReport> { self.inner.tx_async(t) }
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
    // Provider's), uhd-clock makes no device read once `drop` has returned.
    use std::sync::atomic::Ordering::SeqCst;
    let (device, _) = watched();
    let authority = DeviceAuthority::new(device.clone(), Arc::new(ClockRegistry::new()), "internal", "internal", "fake").unwrap();
    let time = authority.time();
    std::thread::sleep(Wall::from_millis(50));
    let begun = Instant::now();
    drop(authority);
    assert!(begun.elapsed() < Wall::from_millis(50), "drop waited {:?}", begun.elapsed());
    device.after.store(true, SeqCst);
    std::thread::sleep(Wall::from_millis(350));
    assert_eq!(device.reads_after.load(SeqCst), 0);
    drop(time);
}
