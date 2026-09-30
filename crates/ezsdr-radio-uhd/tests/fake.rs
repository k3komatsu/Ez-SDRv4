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
    let dir = TempDir::new();
    let device = fake(FakeConfig { apply_delay: Wall::from_millis(20), ..FakeConfig::default() });
    let mut run = session(&profile(&dir, json!({}), json!({}), true), device.clone());
    past_t0(&mut run, ms(1));
    let entry = run.submit(set("radio.rx.gain_db", Value::Num(5.0)), None).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    assert!(calls(&device, "apply rx 0 rate=- freq=- gain=5").iter().any(|c| c.contains("effective=")), "{:?}", device.calls());
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

#[test]
fn ur_16_a_panicking_thread_is_a_lost_device() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::RxPanic(Wall::from_millis(2_200))], ..FakeConfig::default() });
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    let result = run.run_until_end(after(&run, ms(5_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { kind: kind(EventKind::DEVICE_LOST) } });
    assert!(section(&manifest, "rejected").as_array().unwrap().iter().any(|r| r["thread"] == "uhd-rx"));
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
    assert_eq!(outcomes, [TimeErrorOutcome::Drop, TimeErrorOutcome::SendAsap], "{outcomes:?}");
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
fn ur_25_a_device_that_ignores_the_timed_stop_is_stopped_at_e1() {
    // UR-25's fallback: uhd-rx stops the stream untimed when its samples reach e₁, and
    // does not reopen a streamer whose channel count did not change.
    for faults in [vec![], vec![FakeFault::IgnoresTimedStop]] {
        let ignoring = !faults.is_empty();
        let dir = TempDir::new();
        let device = fake(FakeConfig { faults, ..FakeConfig::default() });
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
        assert_eq!(calls.iter().any(|c| c == "rx_stop now"), ignoring, "{calls:?}");
        let capture = capture_of(&manifest, "rec");
        assert!(capture.continuity.iter().all(|m| m.gaps.is_empty()), "{:?}", capture.continuity);
        assert_eq!(capture.size_bytes, 160_000);
    }
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
fn ur_25_a_timed_receive_stop_is_released_a_restart_lead_ahead() {
    // UR-24: a stream command far ahead would hold every later timed command behind it
    // in the device's queue; the stop at e₁ goes to the device only 50 ms before it.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let mut direct = Direct::with_links(FakeConfig::default(), &[], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)]);
    let at = direct.now() + ms(400);
    direct.update("radio.rx.sample_rate_hz", Value::Num(2e6), Cold, Some(at));
    direct.settle(Wall::from_millis(150));
    let stops = |d: &Direct| d.device.calls().into_iter().filter(|c| c.starts_with("rx_stop ")).collect::<Vec<_>>();
    assert!(stops(&direct).is_empty(), "issued early: {:?}", stops(&direct));
    direct.settle(Wall::from_millis(400));
    let issued = stops(&direct);
    assert!(!issued.is_empty(), "never issued");
    let e1: i64 = issued[0].trim_start_matches("rx_stop ").parse().unwrap();
    assert!(e1 >= at && e1 < at + 200, "stop at {e1}, asked {at}");
    let instance = direct.finish();
    let applied = &instance.sections[&Namespace::parse("ezsdr.radio.uhd.usrp.applied").unwrap()];
    assert!(applied.as_array().unwrap().iter().any(|r| r["key"] == "rx_stop" && r["at"]["ticks"] == e1), "{applied}");
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
    let first = samples.iter().position(|s| wave.contains(s)).expect("the loopback came back");
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
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
    let result = run.run_until_end(after(&run, ms(5_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { kind: kind(EventKind::DEVICE_LOST) } });
    assert_eq!(events_of(&manifest, EventKind::DEVICE_LOST)[0].source, ResourceId::parse("usrp").unwrap());
}

#[test]
fn ur_29_a_silent_stream_is_a_lost_device() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::Silence(Wall::from_millis(2_100))], ..FakeConfig::default() });
    let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, BTreeMap::new());
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
}

#[test]
fn ur_29_a_lost_device_is_found_while_idle() {
    let dir = TempDir::new();
    let device = fake(FakeConfig { faults: vec![FakeFault::Lost(Wall::from_millis(2_500))], ..FakeConfig::default() });
    let mut doc = profile(&dir, json!({}), json!({}), true);
    doc["bindings"]["rec"].as_object_mut().unwrap().remove("feed");
    doc.as_object_mut().unwrap().remove("placements");
    doc["bindings"].as_object_mut().unwrap().remove("rec");
    let mut run = session(&doc, device);
    past_t0(&mut run, ms(1));
    let result = run.run_until_end(after(&run, ms(5_000)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = run.finish();
    assert_eq!(events_of(&manifest, EventKind::DEVICE_LOST).len(), 1);
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
