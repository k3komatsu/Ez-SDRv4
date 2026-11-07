//! MockRadio on the SimulationChannel (spec 09 as amended in Phase 3, MR-25, MR-31…MR-36).

use ezsdr_kernel::spec::Scalar;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{Action, ActionId, Event, EventCollector, EventSource, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Endpoint, ModuleError, ModuleRef, Pacing, PrepareContext, ProfileRef, Provider, Requested, RfFidelity, Role, UpdateClass, Version,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BackPressure, BlockRef, BurstEnd, BurstRecord, DataLink, DropCarry, LatePolicy, PublishOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, RelativeBudget, TimePoint};
use ezsdr_mock_radio::MockRadio;
use ezsdr_sim::SimRng;
use ezsdr_sim::channel::Medium;
use serde_json::json;

#[path = "../../ezsdr-acceptance/src/layout.rs"]
mod layout_support;

const ROOT: ClockDomainId = ClockDomainId::local(7);

fn rid(name: &str) -> ResourceId {
    ResourceId::parse(name).unwrap()
}

/// A resource target written as the node path the test resolves it to: the double plays
/// KC-23 with `matched` the identity, `<resource>[/<path>]` (spec 27).
fn tgt(path: &str) -> Target {
    let (resource, rest) = path.split_once('/').unwrap_or((path, ""));
    Target::Resource { resource: Ident::parse(resource).unwrap(), path: rest.to_owned() }
}

/// The node KC-23 resolves a test target to: the same path.
fn node_of(action: &Action) -> Option<ResourceId> {
    match action.target()? {
        Target::Resource { resource, path } if path.is_empty() => Some(ResourceId::parse(resource.as_str()).unwrap()),
        Target::Resource { resource, path } => Some(ResourceId::parse(&format!("{resource}/{path}")).unwrap()),
        _ => None,
    }
}
fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}
fn eq(value: Scalar) -> Constraint {
    Constraint::Eq { value }
}

fn module_ref() -> ModuleRef {
    ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(2, 0, 0) }
}

#[derive(Default)]
struct Queue(Mutex<VecDeque<Action>>);
impl ActionReceiver for Queue {
    fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
        self.0.lock().unwrap().pop_front().map(|action| {
            let node = node_of(&action);
            ezsdr_kernel::module_api::Dispatched::new(ActionId(1), action, node)
        })
    }
}

struct Accepting;
impl ActionSubmitter for Accepting {
    fn submit(&self, _action: Action) -> Result<ActionId, Vec<Violation>> {
        Ok(ActionId(1))
    }
}

#[derive(Default)]
struct Sink {
    queue: Mutex<VecDeque<BlockRef>>,
    drops: AtomicU64,
}
impl DataLink for Sink {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        self.queue.lock().unwrap().push_back(block);
        PublishOutcome::Accepted
    }
    fn receive(&self) -> Option<(BlockRef, DropCarry)> {
        self.queue.lock().unwrap().pop_front().map(|b| (b, DropCarry::default()))
    }
    fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }
    fn take_drop_carry(&self) -> DropCarry {
        DropCarry::default()
    }
    fn policy(&self) -> BackPressure {
        BackPressure::DropOldest
    }
}

/// One Run's shared time, clocks, inputs and medium.
struct World {
    clocks: Arc<ClockRegistry>,
    auth: Arc<ManualTimeAuthority>,
    medium: Arc<Medium>,
    inputs: Arc<Mutex<BTreeMap<ContentHash, Arc<[u8]>>>>,
    environment: Arc<BTreeMap<Namespace, serde_json::Value>>,
}

impl World {
    fn new(environment: serde_json::Value) -> World {
        let clocks = Arc::new(ClockRegistry::new());
        clocks.register(ClockDomain::root(ROOT, Rational::new(1_000_000_000, 1).unwrap(), EpochRef::Arbitrary { set_by: "test".to_owned() })).unwrap();
        let auth = Arc::new(ManualTimeAuthority::new(clocks.clone(), ROOT, &[], Pacing::FreeRunning).unwrap());
        let environment = environment.as_object().unwrap().iter().map(|(name, value)| (Namespace::parse(name).unwrap(), value.clone())).collect();
        World { clocks, auth, medium: Medium::new(), inputs: Arc::new(Mutex::new(BTreeMap::new())), environment: Arc::new(environment) }
    }

    /// Stores `samples` (one channel, interleaved) as an input and returns its reference.
    fn waveform(&self, samples: &[(f32, f32)]) -> ArtifactRef {
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| re.to_le_bytes().into_iter().chain(im.to_le_bytes())).collect();
        let reference = ezsdr_kernel::manifest::ingest_input(Ident::parse("waveform").unwrap(), Namespace::parse("ezsdr.input").unwrap(), "mem:test", &bytes);
        self.inputs.lock().unwrap().insert(reference.hash.clone(), bytes.into());
        reference
    }
}

struct Radio {
    mock: MockRadio,
    id: String,
    events: Arc<EventCollector>,
    actions: Arc<Queue>,
    link: Arc<Sink>,
}

struct Options<'a> {
    profile: &'a str,
    fragment: &'a str,
    id: &'a str,
    constraints: &'a [(&'a str, Constraint)],
    selector: &'a [(&'a str, Value)],
    medium: bool,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options { profile: "ideal", fragment: "radio", id: "mock", constraints: &[("radio.tx.c0.stream", Constraint::Eq { value: Scalar::Int(0) })], selector: &[], medium: true }
    }
}

fn prepare(world: &World, options: Options<'_>) -> Result<Radio, ModuleError> {
    let (mut selector, version) = if options.profile == "x3xx" {
        (layout_support::mock_x3xx_selector(options.id, &["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]), Version::new(0, 1, 0))
    } else {
        (layout_support::ideal_selector(options.id, 2, 2), Version::new(2, 0, 0))
    };
    for (name, value) in options.selector {
        selector.insert(Ident::parse(name).unwrap(), value.clone());
    }
    let binding = Binding { module: module_ref(), selector, profile: Some(ProfileRef { name: options.profile.to_owned(), version }), feed: None };
    let mut mock = MockRadio::from_binding(&binding).unwrap();
    if options.medium {
        mock = mock.with_medium(world.medium.clone());
    }
    let mut registry = ezsdr_kernel::module_api::ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::new();
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
    ezsdr_sim::register(&mut registry, &mut checks, &mut kinds).unwrap();
    let event_kinds = kinds.kinds();
    let id = options.id.to_owned();
    let pairs: Vec<_> = [rid(&id), rid(&format!("{id}/rx/s0")), rid(&format!("{id}/rx/s1")), rid(&format!("{id}/tx/s0")), rid(&format!("{id}/tx/s1"))]
        .into_iter()
        .map(|node| EventSource::Node { node })
        .flat_map(|source| event_kinds.iter().cloned().map(move |kind| (source.clone(), kind)))
        .collect();
    let events = Arc::new(EventCollector::new(&pairs, &event_kinds, 4096, &Policy::default()));
    let actions = Arc::new(Queue::default());
    let link = Arc::new(Sink::default());
    let ctx = {
        let mut ctx = PrepareContext::testing(EventSource::Node { node: rid(&id) }, world.auth.clone(), world.clocks.clone(), events.clone(), actions.clone(), Arc::new(Accepting));
        ctx.run = RunId::from_string("mock-channel".to_owned());
        ctx.environment = world.environment.clone();
        ctx.inputs = world.inputs.clone();
        ctx.links = vec![AttachedPort::new(Ident::parse(options.fragment).unwrap(), Ident::parse("rx0").unwrap(), Endpoint::StreamOut(link.clone()))];
        ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
        ctx
    };
    // RM-30: no channel is in a stream unless a request puts it there; these runs receive on c0.
    let mut constraints: Vec<_> = options.constraints.iter().map(|(name, constraint)| (key(name), constraint.clone())).collect();
    if !constraints.iter().any(|(name, _)| name.as_str() == "radio.rx.c0.stream") {
        constraints.push((key("radio.rx.c0.stream"), Constraint::Eq { value: Scalar::Int(0) }));
    }
    let requested = Requested { resource: rid(&id), constraints: constraints.into_iter().collect() };
    let fragment =
        Fragment { id: Ident::parse(options.fragment).unwrap(), instance: module_ref(), role: Role::Provider, content: json!({ "selector": {}, "requested": requested }), after: Vec::new() };
    mock.prepare(&fragment, ctx)?;
    Ok(Radio { mock, id, events, actions, link })
}

impl Radio {
    fn push(&self, action: Action) {
        self.actions.0.lock().unwrap().push_back(action);
    }

    fn tx_domain(&self, world: &World) -> ClockDomainId {
        world.clocks.sample_clock_records().iter().rev().find(|record| record.stream == rid(&format!("{}/tx/s0", self.id))).unwrap().domain
    }

    /// Every received sample of channel `channel`, by sample index, in arrival order.
    fn received(&self, channel: usize) -> Vec<(i64, f32, f32)> {
        self.received_all().into_iter().nth(channel).unwrap_or_default()
    }

    /// Every received sample of every channel, drained from the link in one pass.
    fn received_all(&self) -> Vec<Vec<(i64, f32, f32)>> {
        let mut out: Vec<Vec<(i64, f32, f32)>> = Vec::new();
        while let Some((block, _)) = self.link.receive() {
            let header = block.header().clone();
            let bytes = block.host_bytes().unwrap();
            let len = header.len as usize;
            let channels = bytes.len() / (len * 8);
            out.resize_with(out.len().max(channels), Vec::new);
            for (channel, samples) in out.iter_mut().enumerate().take(channels) {
                for index in 0..len {
                    let (re, im) = ezsdr_hostmem::read_cf32(bytes, len, channel, index);
                    samples.push((header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() + index as i64, re, im));
                }
            }
        }
        out
    }

    fn section(&self, name: &str) -> serde_json::Value {
        // MR-27: every section name carries this instance's own id, so two Mocks in one
        // Run do not overwrite each other's records.
        self.mock.instance().sections[&Namespace::parse(&format!("ezsdr.radio.mock.{name}")).unwrap()].clone()
    }

    fn drain_events(&self) -> Vec<Event> {
        self.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
    }
}

fn burst(radio: &Radio, world: &World, waveform: ArtifactRef, at: i64, repeat: bool) -> Action {
    Action::TxBurst {
        target: tgt(&format!("{}/tx/s0", radio.id)),
        waveform,
        repeat,
        at: AbsoluteDeadline::new(TimePoint::new(radio.tx_domain(world), at)),
        requested_at: None,
        late_policy: LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    }
}

fn step(world: &World, radios: &mut [&mut Radio], tick: i64) {
    world.auth.advance_to(TimePoint::new(ROOT, tick)).unwrap();
    for radio in radios.iter_mut() {
        radio.mock.step(TimePoint::new(ROOT, tick)).unwrap();
    }
}

fn loopback(noise: Option<f64>) -> serde_json::Value {
    let mut channel = json!({ "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }] });
    if let Some(noise) = noise {
        channel["noise_dbfs"] = json!({ "radio": noise });
    }
    json!({ "sim.channel": channel })
}

fn ramp(len: usize) -> Vec<(f32, f32)> {
    (0..len).map(|n| ((n + 1) as f32 / 4096.0, -((n + 1) as f32) / 8192.0)).collect()
}

/// Two started `ideal` Mocks, `a` (device `dev_a`) and `b` (device `dev_b`), at the given
/// sample rates, each with one transmit channel.
fn two(environment: serde_json::Value, a_rate: f64, b_rate: f64) -> (World, Radio, Radio) {
    let world = World::new(environment);
    let a_constraints =
        [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(a_rate).unwrap())), ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(a_rate).unwrap()))];
    let b_constraints =
        [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(b_rate).unwrap())), ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(b_rate).unwrap()))];
    let mut a = prepare(&world, Options { fragment: "a", id: "dev_a", constraints: &a_constraints, ..Options::default() }).unwrap();
    let mut b = prepare(&world, Options { fragment: "b", id: "dev_b", constraints: &b_constraints, ..Options::default() }).unwrap();
    for radio in [&mut a, &mut b] {
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    }
    (world, a, b)
}

fn a_into_b() -> serde_json::Value {
    json!({ "sim.seed": 5, "sim.channel": { "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0.0 }] } })
}

fn ordered(world: &World, a: &mut Radio, b: &mut Radio, tick: i64, a_first: bool) {
    if a_first { step(world, &mut [a, b], tick) } else { step(world, &mut [b, a], tick) }
}

/// Steps the radios at every due wakeup up to and including `until`, as MA-30's loop does.
fn run_to(world: &World, radios: &mut [&mut Radio], until: i64) {
    while let Some(due) = world.auth.next_due() {
        if due.ticks_in(ROOT).unwrap() > until {
            break;
        }
        step(world, radios, due.ticks_in(ROOT).unwrap());
    }
}

fn records(radio: &Radio) -> Vec<BurstRecord> {
    serde_json::from_value(radio.section("bursts")).unwrap()
}

#[test]
fn mr_31_channel_mode_needs_a_medium_a_zero_pattern_and_valid_channels() {
    let world = World::new(loopback(None));
    let refusal = |result: Result<Radio, ModuleError>| result.err().expect("prepare must be refused").message;
    assert!(refusal(prepare(&world, Options { medium: false, ..Options::default() })).starts_with("MR-31: the environment declares sim.channel"));
    let ramp_pattern = [("rx_test_pattern", Value::from("ramp".to_owned()))];
    assert!(refusal(prepare(&world, Options { selector: &ramp_pattern, ..Options::default() })).starts_with("MR-31: a channel-coupled Mock takes no receive test pattern"));

    let wide = World::new(json!({ "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 2, "rx": "radio", "rx_channel": 0, "gain_db": 0 }] } }));
    assert!(refusal(prepare(&wide, Options { profile: "x3xx", ..Options::default() })).contains("names a channel beyond this profile's 2"));
    let deep = World::new(json!({ "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 2, "gain_db": 0 }] } }));
    assert!(refusal(prepare(&deep, Options { profile: "x3xx", ..Options::default() })).contains("names a channel beyond this profile's 2"));

    let first = prepare(&world, Options::default()).unwrap();
    assert_eq!(first.mock.instance().fidelity.rf, RfFidelity::ImpairmentModel);
    assert!(refusal(prepare(&world, Options { id: "other", ..Options::default() })).starts_with("MR-31: CH-6: radio has already joined"));
    let malformed = World::new(json!({ "sim.channel": [] }));
    assert!(refusal(prepare(&malformed, Options::default())).starts_with("MR-7: CH-1: "));

    let ghost = World::new(json!({ "sim.channel": { "couplings": [{ "tx": "ghost", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0 }] } }));
    let mut lonely = prepare(&ghost, Options::default()).unwrap();
    assert!(lonely.mock.arm().unwrap_err().message.starts_with("MR-31: the channel names ghost, which did not join its medium"));

    let plain = World::new(json!({}));
    let unchanneled = prepare(&plain, Options { medium: false, ..Options::default() }).unwrap();
    assert_eq!(unchanneled.mock.instance().fidelity.rf, RfFidelity::None);
}

#[test]
fn mr_32_a_loopback_receives_what_it_transmits() {
    let world = World::new(json!({ "sim.channel": { "couplings": [
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 },
        { "tx": "radio", "tx_channel": 1, "rx": "radio", "rx_channel": 0, "gain_db": 20.0 }
    ] } }));
    let mut radio = prepare(&world, Options::default()).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let samples = ramp(3_000);
    let waveform = world.waveform(&samples);
    radio.push(burst(&radio, &world, waveform, 1_000, false));
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 6_000_001);
    let received = radio.received(0);
    assert_eq!(received.len(), 6_000);
    for (k, re, im) in received {
        let expected = if (1_000..4_000).contains(&k) { samples[(k - 1_000) as usize] } else { (0.0, 0.0) };
        assert_eq!((re, im), expected, "sample {k}");
    }
    let stats = radio.section("stats");
    assert_eq!((stats["rx_clipped"].as_u64(), stats["tx_clipped"].as_u64()), (Some(0), Some(0)));
}

#[test]
fn mr_32_waveform_refusals() {
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options::default()).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let absent = ezsdr_kernel::manifest::ingest_input(Ident::parse("waveform").unwrap(), Namespace::parse("ezsdr.input").unwrap(), "mem:none", &[0u8; 80]);
    let mut short = world.waveform(&ramp(10));
    short.size_bytes = 72;
    let not_finite = world.waveform(&[(f32::NAN, 0.0), (0.5, 0.5)]);
    for (waveform, at) in [(absent, 100), (short, 200), (not_finite, 300)] {
        radio.push(burst(&radio, &world, waveform, at, false));
    }
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 2_000_001);
    let rejected = radio.section("rejected");
    let reasons: Vec<&str> = rejected.as_array().unwrap().iter().map(|row| row["reason"].as_str().unwrap()).collect();
    assert_eq!(reasons, ["MR-32: the waveform's bytes are not an input of this Run", "MR-32: the waveform's bytes do not have its size_bytes", "RM-13: a waveform sample is not finite",]);
    assert_eq!(radio.drain_events().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 3);
    assert!(radio.received(0).iter().all(|(_, re, im)| (*re, *im) == (0.0, 0.0)));
}

#[test]
fn mr_32_a_the_waveform_refusals_hold_their_place_in_handle_tx_burst() {
    // MR-16 as VB-7 amended: the three MR-32 waveform refusals follow the repeat
    // constraints and precede MR-17's decision. A refusal moved after `decide` would
    // answer a late burst with a TIME_ERROR; one moved before the repeat check would
    // report the absent bytes where the repeat reason belongs. `mr_32_waveform_refusals`
    // cannot see either, because its three bursts are on time and not repeated.
    // `ideal` has no lead, so a target in the past is late and MR-17 would drop it.
    // MR-32's refusals are channel-mode rules, so only a channelled Mock reads a
    // waveform at all; on a pattern-mode Mock there is nothing to place.
    {
        let world = World::new(loopback(None));
        let mut radio = prepare(&world, Options::default()).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
        step(&world, &mut [&mut radio], 0);
        let absent = ezsdr_kernel::manifest::ingest_input(Ident::parse("waveform").unwrap(), Namespace::parse("ezsdr.input").unwrap(), "mem:none", &[0u8; 80]);
        let mut late = burst(&radio, &world, absent, 1, false);
        if let Action::TxBurst { late_policy, .. } = &mut late {
            *late_policy = LatePolicy::DropAndFlag;
        }
        radio.push(late);
        step(&world, &mut [&mut radio], 1_500);
        assert!(
            radio.drain_events().iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
            "the MR-32 refusal precedes MR-17's decision, so a late burst with absent bytes is not a TIME_ERROR"
        );
        assert_eq!(radio.section("rejected")[0]["reason"], "MR-32: the waveform's bytes are not an input of this Run");
    }

    {
        // `x3xx` aligns a repeated waveform to two samples, so eleven samples
        // break the repeat constraint. `ideal` aligns to one and would not.
        let t0 = 2_000_000_000;
        let world = World::new(loopback(None));
        let mut radio = prepare(&world, Options { profile: "x3xx", ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
        step(&world, &mut [&mut radio], t0);
        let absent = ezsdr_kernel::manifest::ingest_input(Ident::parse("waveform").unwrap(), Namespace::parse("ezsdr.input").unwrap(), "mem:none", &[0u8; 88]);
        radio.push(burst(&radio, &world, absent, 1_000, true));
        step(&world, &mut [&mut radio], t0 + 2_000_001);
        assert_eq!(
            radio.section("rejected")[0]["reason"],
            "MR-16: repeated waveform violates RM-13's length or alignment limits",
            "the MR-32 refusal follows the repeat constraints, so the repeat reason wins"
        );
    }
}

#[test]
fn mr_17_a_a_target_whose_instant_has_passed_is_late_in_channel_mode_too() {
    // MR-17 as VB-5 amended: the rounded-up instant reaches `LatePolicy::decide` in every
    // mode. `mr_17_a_target_whose_instant_has_passed_is_late_even_on_the_floor_sample`
    // runs with `medium: false`, so a floor used only in channel mode would pass it.
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options::default()).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    step(&world, &mut [&mut radio], 0);
    let waveform = world.waveform(&ramp(10));
    let mut late = burst(&radio, &world, waveform, 1, false);
    if let Action::TxBurst { late_policy, .. } = &mut late {
        *late_policy = LatePolicy::DropAndFlag;
    }
    radio.push(late);
    step(&world, &mut [&mut radio], 1_500);
    let events = radio.drain_events();
    let time_errors: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
    assert_eq!(time_errors.len(), 1, "sample 1 is at 1 000 ns, before the round at 1 500 ns");
    assert_eq!(time_errors[0].payload["outcome"], "drop");
    assert_eq!(time_errors[0].payload["late_by_ns"], 1_000, "measured from the first transmit sample at or after the round");
    assert_eq!(time_errors[0].time, TimePoint::new(radio.tx_domain(&world), 1), "the event's time is the round's instant in the transmit clock, floored");
    assert_eq!(radio.section("rejected")[0]["reason"], "MR-17: the late policy dropped the burst");
}

#[test]
fn mr_25_a_stopped_burst_transmits_every_sample_before_the_stop() {
    for environment in [loopback(None), json!({})] {
        let channelled = environment.get("sim.channel").is_some();
        let world = World::new(environment);
        let mut radio = prepare(&world, Options { medium: channelled, ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
        let samples = ramp(1_000);
        let waveform = world.waveform(&samples);
        radio.push(burst(&radio, &world, waveform, 0, true));
        step(&world, &mut [&mut radio], 0);
        step(&world, &mut [&mut radio], 2_000_001);
        let held = world.waveform(&[(0.75, 0.75)]);
        radio.push(burst(&radio, &world, held, 4_000, true));
        step(&world, &mut [&mut radio], 2_000_001);
        radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
        step(&world, &mut [&mut radio], 2_500_500);
        step(&world, &mut [&mut radio], 6_000_001);
        let records: Vec<BurstRecord> = serde_json::from_value(radio.section("bursts")).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].end, BurstEnd::Stop);
        assert_eq!(records[0].samples, 2_501, "the samples before the stop instant 2 500 500 are 0 … 2 500");
        let rejected = radio.section("rejected");
        assert_eq!(rejected.as_array().unwrap().len(), 1, "the held burst starts inside the open repeat: refused at receipt");
        assert!(rejected[0]["reason"].as_str().unwrap().contains("overlaps an accepted booking"));
        if channelled {
            for (k, re, im) in radio.received(0) {
                let expected = if k <= 2_500 { samples[(k % 1_000) as usize] } else { (0.0, 0.0) };
                assert_eq!((re, im), expected, "sample {k}");
            }
        }
    }
}

#[test]
fn mr_25_stop_hands_a_future_repeat_through_the_in_flight_window() {
    let t0 = 2_000_000_000;
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options { profile: "x3xx", ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
    let samples = vec![(0.5, 0.0); 2_000];
    let waveform = world.waveform(&samples);
    radio.push(burst(&radio, &world, waveform, 2_004_000, true));
    radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
    step(&world, &mut [&mut radio], t0);
    let records: Vec<BurstRecord> = serde_json::from_value(radio.section("bursts")).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!((records[0].samples, records[0].end), (6_000, BurstEnd::Stop));

    step(&world, &mut [&mut radio], t0 + 12_001_000);
    let received = radio.received(0);
    let sample = |k: i64| {
        received
            .iter()
            .find(|(at, _, _)| *at == k)
            .map(|(_, re, im)| (*re, *im))
            .unwrap_or_else(|| panic!("no loopback sample {k}; range={:?}..{:?}; bursts={:?}", received.first(), received.last(), radio.section("bursts")))
    };
    let first = sample(4_045);
    let last = sample(10_044);
    assert!((f32::hypot(first.0, first.1) - 0.5).abs() < 1e-6, "the future burst radiates: {first:?}");
    assert!((f32::hypot(last.0, last.1) - 0.5).abs() < 1e-6, "the last sample through Stop's handed frontier radiates: {last:?}");
    assert_eq!(sample(10_045), (0.0, 0.0), "radiation ends at the delayed cut");
}

#[test]
fn mr_25_stop_does_not_radiate_unhanded_window_edge_payload() {
    let t0 = 2_000_000_000;
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options { profile: "x3xx", ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
    let domain = radio.tx_domain(&world);
    radio.push(burst(&radio, &world, world.waveform(&[(0.0, 0.5)]), 2_010_005, false));
    radio.push(burst(&radio, &world, world.waveform(&[(0.5, 0.0)]), 2_009_999, false));
    radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
    step(&world, &mut [&mut radio], t0);

    let initial_records = records(&radio);
    assert_eq!(initial_records.len(), 1, "only the one-sample A was handed over: {initial_records:?}");
    assert_eq!((initial_records[0].target, initial_records[0].samples), (TimePoint::new(domain, 2_009_999), 1));
    let rejected = radio.section("rejected");
    assert_eq!(rejected.as_array().unwrap().len(), 1, "the booking after the Stop's cut was cancelled");
    assert!(rejected[0]["reason"].as_str().unwrap().contains("cancelled by stop"), "{rejected:?}");

    // C at H+1 starts inside A's window: refused at receipt, not late (RM-15). Its policy is DropAndFlag: a SendAsap one
    // meeting only the Stop's region is moved past it (F36), which is tested in mock_radio.rs.
    let mut c = burst(&radio, &world, world.waveform(&[(0.0, 0.5)]), 2_010_001, false);
    if let Action::TxBurst { late_policy, .. } = &mut c {
        *late_policy = LatePolicy::DropAndFlag;
    }
    radio.push(c);
    step(&world, &mut [&mut radio], t0);
    let rejected = radio.section("rejected");
    assert_eq!(rejected.as_array().unwrap().len(), 2);
    assert!(rejected[1]["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "{rejected:?}");
    assert!(radio.drain_events().iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "a refused C is not late");

    // D at H+4 is the first sample after A's window: on time.
    radio.push(burst(&radio, &world, world.waveform(&[(0.0, 0.5)]), 2_010_004, false));
    step(&world, &mut [&mut radio], t0);
    assert!(radio.drain_events().iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "D at H+4 is on time");

    step(&world, &mut [&mut radio], t0 + 12_006_001);
    let final_records = records(&radio);
    assert_eq!(final_records.len(), 2, "only A and on-time D have records: {final_records:?}");
    assert_eq!((final_records[1].target, final_records[1].samples), (TimePoint::new(domain, 2_010_004), 1));
    let received = radio.received(0);
    let sample = |k: i64| received.iter().find(|(at, _, _)| *at == k).map(|(_, re, im)| (*re, *im)).unwrap_or((0.0, 0.0));
    let handed = sample(10_044);
    assert!((f32::hypot(handed.0, handed.1) - 0.5).abs() < 1e-6, "A's payload through its exclusive end is radiated: {handed:?}");
    assert_eq!(sample(10_045), (0.0, 0.0), "the safe restart gap is not payload handed at Stop");
    assert_eq!(sample(10_046), (0.0, 0.0), "refused C radiated nothing");
    let d_sample = sample(10_049);
    assert!((f32::hypot(d_sample.0, d_sample.1) - 0.5).abs() < 1e-6, "on-time D at H+4 radiates: {d_sample:?}");
    assert_eq!(sample(10_050), (0.0, 0.0), "radiation ends after D");
}

#[test]
fn mr_25_a_first_send_that_does_not_fit_the_window_is_not_handed_over() {
    // UR-22 (S3): a first send of 32 samples must fit the window; 31 samples of a 40-sample burst do not, 32 do.
    let t0 = 2_000_000_000;
    for (start, handed) in [(2_009_969, false), (2_009_968, true)] {
        let world = World::new(loopback(None));
        let mut radio = prepare(&world, Options { profile: "x3xx", ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
        radio.push(burst(&radio, &world, world.waveform(&[(0.5, 0.0); 40]), start, false));
        radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
        step(&world, &mut [&mut radio], t0);
        assert_eq!(!records(&radio).is_empty(), handed, "start {start}: {:?}", records(&radio));
    }
}

#[test]
fn mr_25_a_repeat_first_send_is_judged_at_32_samples() {
    // UR-22 (S3): a repeat's first send tiles its waveform to 32 samples, so a repeat of two needs 32 samples of window space.
    let t0 = 2_000_000_000;
    for (start, handed) in [(2_009_969, false), (2_009_968, true)] {
        let world = World::new(loopback(None));
        let mut radio = prepare(&world, Options { profile: "x3xx", ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
        radio.push(burst(&radio, &world, world.waveform(&[(0.5, 0.0); 2]), start, true));
        radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
        step(&world, &mut [&mut radio], t0);
        assert_eq!(!records(&radio).is_empty(), handed, "start {start}: {:?}", records(&radio));
    }
}

#[test]
fn mr_17_a_one_sample_burst_uses_the_safe_sample() {
    // A one-sample burst holds its own sample and the next four (RM-15): a booking at 4003 or 4004 is refused at receipt, 4005 is the first free sample.
    let t0 = 2_000_000_000;
    for (target, refused) in [(2_004_003, true), (2_004_004, true), (2_004_005, false)] {
        let case = format!("B at sample {target}");
        let world = World::new(loopback(None));
        let mut radio = prepare(&world, Options { profile: "x3xx", ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
        let domain = radio.tx_domain(&world);
        let a = world.waveform(&[(0.5, 0.0)]);
        let b = world.waveform(&[(0.0, 0.5)]);
        radio.push(burst(&radio, &world, a, 2_004_000, false));
        radio.push(burst(&radio, &world, b, target, false));
        step(&world, &mut [&mut radio], t0);

        let rejected = radio.mock.instance().sections.get(&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()).map_or(0, |rows| rows.as_array().unwrap().len());
        assert_eq!(rejected, usize::from(refused), "{case}");
        if refused {
            assert!(radio.section("rejected")[0]["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "{case}");
        }
        let errors: Vec<_> = radio.drain_events().into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert!(errors.is_empty(), "{case}: {errors:?}");

        step(&world, &mut [&mut radio], t0 + 6_100_001);
        let records = records(&radio);
        assert_eq!(records.len(), if refused { 1 } else { 2 }, "{case}: {records:?}");
        assert_eq!(records[0].target, TimePoint::new(domain, 2_004_000));
        assert_eq!(records[0].samples, 1);
        if !refused {
            assert_eq!(records[1].target, TimePoint::new(domain, target), "{case}: actual B start");
            assert_eq!(records[1].samples, 1);
        }
        let received = radio.received(0);
        let sample = |k: i64| received.iter().find(|(at, _, _)| *at == k).map(|(_, re, im)| (*re, *im)).unwrap_or((0.0, 0.0));
        assert!((f32::hypot(sample(4_045).0, sample(4_045).1) - 0.5).abs() < 1e-6, "{case}: A's sample; records={records:?}");
        let b_sample = sample(target - 2_004_000 + 4_045);
        if refused {
            assert_eq!(b_sample, (0.0, 0.0), "{case}: refused B radiated");
        } else {
            assert!((f32::hypot(b_sample.0, b_sample.1) - 0.5).abs() < 1e-6, "{case}: B's actual loopback start {b_sample:?}");
        }
    }
}

#[test]
fn mr_25_repeated_stop_preserves_handed_samples() {
    let t0 = 2_000_000_000;
    for (running, second_delay) in [(false, 0), (false, 1_000_000), (true, 0), (true, 1_000_000)] {
        let case = format!("running={running}, second Stop +{second_delay} ns");
        let world = World::new(loopback(None));
        let mut radio = prepare(&world, Options { profile: "x3xx", ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
        let start = if running { 2_002_000 } else { 2_004_000 };
        let handover = if running { 2_014_000 } else { 2_010_000 };
        let waveform = world.waveform(&vec![(0.5, 0.0); 2_000]);
        radio.push(burst(&radio, &world, waveform, start, true));
        step(&world, &mut [&mut radio], t0);
        let first_stop_at = if running { t0 + 4_000_000 } else { t0 };
        if running {
            step(&world, &mut [&mut radio], first_stop_at);
        }
        radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
        step(&world, &mut [&mut radio], first_stop_at);
        radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
        step(&world, &mut [&mut radio], first_stop_at + second_delay);

        let records = records(&radio);
        assert_eq!(records.len(), 1, "{case}: {records:?}");
        assert_eq!((records[0].samples, records[0].end), ((handover - start) as u64, BurstEnd::Stop), "{case}");
        step(&world, &mut [&mut radio], t0 + 20_001_000);
        let received = radio.received(0);
        let sample = |k: i64| received.iter().find(|(at, _, _)| *at == k).map(|(_, re, im)| (*re, *im)).unwrap_or((0.0, 0.0));
        for k in [start - 2_000_000 + 45, handover - 1 - 2_000_000 + 45] {
            let got = sample(k);
            assert!(
                (f32::hypot(got.0, got.1) - 0.5).abs() < 1e-6,
                "{case}: handed sample {k} was recalled: {got:?}; records={records:?}; tail={:?}",
                received.iter().filter(|(at, _, _)| *at >= 14_035).collect::<Vec<_>>()
            );
        }
        assert_eq!(sample(handover - 2_000_000 + 45), (0.0, 0.0), "{case}: radiation continued past the first Stop's handed front");
    }
}

#[test]
fn mr_33_gain_scales_the_samples() {
    let world = World::new(loopback(None));
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.gain_db", eq(Scalar::try_from(-6.0).unwrap())), ("radio.rx.c0.gain_db", eq(Scalar::try_from(12.0).unwrap()))];
    let mut radio = prepare(&world, Options { constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let waveform = world.waveform(&[(0.5, 0.25)]);
    radio.push(burst(&radio, &world, waveform, 0, true));
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 6_000_001);
    // The gains asked at the start scale every sample: -6 dB on transmit, +12 dB on receive.
    let scale = 10f64.powf(6.0 / 20.0);
    for (k, re, im) in radio.received(0) {
        assert!((f64::from(re) - 0.5 * scale).abs() < 1e-6 && (f64::from(im) - 0.25 * scale).abs() < 1e-6, "sample {k}: ({re}, {im})");
    }
}

#[test]
fn mr_33_a_receiver_hears_only_its_own_frequency() {
    // CH-4's gate is exact equality of the two frequencies asked at the start.
    for (tx, rx, heard) in [(2.0e9, 2.0e9, true), (2.0e9, 1.0e9, false)] {
        let world = World::new(loopback(None));
        let constraints =
            [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.frequency_hz", eq(Scalar::try_from(tx).unwrap())), ("radio.rx.c0.frequency_hz", eq(Scalar::try_from(rx).unwrap()))];
        let mut radio = prepare(&world, Options { constraints: &constraints, ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
        let waveform = world.waveform(&[(0.5, 0.25)]);
        radio.push(burst(&radio, &world, waveform, 0, true));
        step(&world, &mut [&mut radio], 0);
        step(&world, &mut [&mut radio], 6_000_001);
        for (k, re, im) in radio.received(0) {
            assert_eq!((re, im), if heard { (0.5, 0.25) } else { (0.0, 0.0) }, "tx {tx} rx {rx} sample {k}");
        }
    }
}

/// The root tick of sample `k` of the transmit clock `domain`.
fn tx_root(world: &World, domain: ClockDomainId, k: i64) -> i64 {
    match world.clocks.convert(TimePoint::new(domain, k), ROOT).unwrap() {
        ezsdr_kernel::time::Converted::Exact { point } => point.ticks_in(ROOT).unwrap(),
        ezsdr_kernel::time::Converted::Inexact { .. } => panic!("the transmit clock is on the root lattice"),
    }
}

#[test]
fn mr_33_a_transmit_value_takes_effect_at_the_next_burst_s_start() {
    // RM-21, MR-33, S1: burst A radiates at f1; after A's exclusive end the value becomes f2; burst B radiates at f2.
    // Receive channel 0 is tuned to f1 and channel 1 to f2, so each burst is heard on the channel of its frequency.
    let world = World::new(json!({ "sim.channel": { "couplings": [
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 },
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 1, "gain_db": 0.0 }
    ] } }));
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.frequency_hz", eq(Scalar::try_from(2.0e9).unwrap())),
        ("radio.rx.c0.frequency_hz", eq(Scalar::try_from(2.0e9).unwrap())),
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.frequency_hz", eq(Scalar::try_from(1.0e9).unwrap())),
    ];
    let mut radio = prepare(&world, Options { constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let domain = radio.tx_domain(&world);
    let (a, b) = (ramp(40), ramp(40));
    radio.push(burst(&radio, &world, world.waveform(&a), 0, false));
    step(&world, &mut [&mut radio], 0);
    let end_a = tx_root(&world, domain, 40);
    run_to(&world, &mut [&mut radio], end_a - 1);
    radio.push(Action::UpdateParameter { target: tgt("mock"), key: key("radio.tx.c0.frequency_hz"), value: Value::num(1.0e9).unwrap(), class: UpdateClass::Cold, at: None });
    step(&world, &mut [&mut radio], end_a);
    radio.push(burst(&radio, &world, world.waveform(&b), 1000, false));
    step(&world, &mut [&mut radio], end_a);
    run_to(&world, &mut [&mut radio], tx_root(&world, domain, 1040) + 6_000_000);
    let mut channels = radio.received_all().into_iter();
    let (heard_f1, heard_f2) = (channels.next().unwrap(), channels.next().unwrap());
    assert!(!heard_f1.is_empty() && !heard_f2.is_empty());
    for (k, re, im) in heard_f1 {
        let expected = if (0..40).contains(&k) { a[k as usize] } else { (0.0, 0.0) };
        assert_eq!((re, im), expected, "channel at f1, sample {k}");
    }
    for (k, re, im) in heard_f2 {
        let expected = if (1000..1040).contains(&k) { b[(k - 1000) as usize] } else { (0.0, 0.0) };
        assert_eq!((re, im), expected, "channel at f2, sample {k}");
    }
}

#[test]
fn mr_33_a_transmit_value_takes_effect_at_a_fractional_burst_start() {
    // RM-21, MR-33, S1: at 3 MS/s on the 1 GHz root, sample 1999 falls at root tick 666333⅓. The value staged for an empty stream
    // is in force at that exact instant, so a receiver tuned to f2 hears the burst's first sample and one tuned to f1 does not.
    let world = World::new(json!({ "sim.channel": { "couplings": [
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 },
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 1, "gain_db": 0.0 }
    ] } }));
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(3.0e6).unwrap())),
        ("radio.tx.c0.frequency_hz", eq(Scalar::try_from(1.0e9).unwrap())),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(3.0e6).unwrap())),
        ("radio.rx.c0.frequency_hz", eq(Scalar::try_from(1.0e9).unwrap())),
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.frequency_hz", eq(Scalar::try_from(2.0e9).unwrap())),
    ];
    let mut radio = prepare(&world, Options { constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let burst_samples = ramp(10);
    radio.push(Action::UpdateParameter { target: tgt("mock"), key: key("radio.tx.c0.frequency_hz"), value: Value::num(2.0e9).unwrap(), class: UpdateClass::Cold, at: None });
    radio.push(burst(&radio, &world, world.waveform(&burst_samples), 1999, false));
    step(&world, &mut [&mut radio], 0);
    run_to(&world, &mut [&mut radio], 7_000_000);
    let mut channels = radio.received_all().into_iter();
    let (heard_f1, heard_f2) = (channels.next().unwrap(), channels.next().unwrap());
    assert!(!heard_f2.is_empty());
    for (k, re, im) in heard_f1 {
        assert_eq!((re, im), (0.0, 0.0), "channel at f1, sample {k}");
    }
    for (k, re, im) in heard_f2 {
        let expected = if (1999..2009).contains(&k) { burst_samples[(k - 1999) as usize] } else { (0.0, 0.0) };
        assert_eq!((re, im), expected, "channel at f2, sample {k}");
    }
}

/// The draws of MR-34 for a two-channel `x3xx` device: the receive and transmit phases.
struct Phases {
    rx: Vec<f64>,
    tx: Vec<f64>,
}

fn phases(seed: u64, device: &str) -> Phases {
    let mut rng = SimRng::new(seed, &format!("{device}/lo"));
    let mut draw = || 2.0 * std::f64::consts::PI * ((rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64);
    let mut pair = || vec![draw(), draw()];
    Phases { rx: pair(), tx: pair() }
}

#[test]
fn mr_34_the_deterministic_profile_keeps_every_phase_zero() {
    let environment = json!({ "sim.seed": 1, "sim.channel": { "couplings": [
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 1, "gain_db": 0.0 }
    ] } });
    let world = World::new(environment);
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.rx.c1.stream", eq(Scalar::from(0)))];
    let mut radio = prepare(&world, Options { constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let waveform = world.waveform(&[(0.5, 0.0)]);
    radio.push(burst(&radio, &world, waveform, 0, true));
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 2_000_001);
    assert!(radio.received(1).iter().all(|(_, re, im)| (*re, *im) == (0.5, 0.0)));
}

#[test]
fn mr_32_the_transmit_path_delay_shifts_a_loopback() {
    let t0 = 2_000_000_000;
    let world = World::new(json!({ "sim.seed": 3, "sim.channel": { "couplings": [
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }
    ] } }));
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.gain_db", eq(Scalar::try_from(6.0).unwrap()))];
    let mut radio = prepare(&world, Options { profile: "x3xx", constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
    step(&world, &mut [&mut radio], t0);
    let samples = ramp(1_000);
    let waveform = world.waveform(&samples);
    radio.push(burst(&radio, &world, waveform, 2_002_000, false));
    step(&world, &mut [&mut radio], t0);
    step(&world, &mut [&mut radio], t0 + 4_000_001);
    let Phases { rx, tx, .. } = phases(3, "mock");
    let (sin, cos) = (tx[0] - rx[0]).sin_cos();
    for (k, re, im) in radio.received(0) {
        let n = k - 2_045;
        let (want_re, want_im) = if (0..1_000).contains(&n) {
            let (wr, wi) = samples[n as usize];
            // the transmit gain asked at the start scales every sample, which reaches the antenna 45 samples after its stamp
            let gain = 10f64.powf(6.0 / 20.0);
            (gain * (f64::from(wr) * cos - f64::from(wi) * sin), gain * (f64::from(wr) * sin + f64::from(wi) * cos))
        } else {
            (0.0, 0.0)
        };
        assert!((f64::from(re) - want_re).abs() < 1e-6 && (f64::from(im) - want_im).abs() < 1e-6, "sample {k}");
    }
}

#[test]
fn mr_36_clipping_at_full_scale_is_counted() {
    let world = World::new(loopback(None));
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.rx.c0.gain_db", eq(Scalar::try_from(20.0).unwrap()))];
    let mut radio = prepare(&world, Options { constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let waveform = world.waveform(&[(2.0, -3.0), (0.05, 0.05), (0.05, 1.5), (0.02, 0.05)]);
    radio.push(burst(&radio, &world, waveform, 0, true));
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 2_000_001);
    let received = radio.received(0);
    assert_eq!(received.len(), 2_000);
    for (k, re, im) in &received {
        // The receive gain of 20 dB is asked at the start: every sample is scaled by it and then clipped.
        let expected = match k % 4 {
            0 => (1.0, -1.0),
            1 => (0.5, 0.5),
            2 => (0.5, 1.0),
            _ => (0.2, 0.5),
        };
        assert!((re - expected.0).abs() < 1e-6 && (im - expected.1).abs() < 1e-6, "sample {k}: ({re}, {im})");
    }
    let stats = radio.section("stats");
    assert_eq!(stats["tx_clipped"], 2, "two waveform samples have a component beyond full scale, one of them only its imaginary part");
    assert_eq!(stats["rx_clipped"], 1_000, "two samples in four have a component beyond full scale, one of them only its imaginary part");
}

#[test]
fn ch_09_the_receive_output_does_not_depend_on_the_stepping_order() {
    let run = |a_first: bool| {
        let world = World::new(json!({ "sim.seed": 5, "sim.channel": {
            "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0.0 }],
            "noise_dbfs": { "b": -60.0 }
        } }));
        let mut a = prepare(&world, Options { fragment: "a", id: "dev_a", ..Options::default() }).unwrap();
        let mut b = prepare(&world, Options { fragment: "b", id: "dev_b", ..Options::default() }).unwrap();
        for radio in [&mut a, &mut b] {
            radio.mock.arm().unwrap();
            radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
        }
        let samples = vec![(0.5f32, 0.25f32); 2_000];
        let waveform = world.waveform(&samples);
        let order = |world: &World, a: &mut Radio, b: &mut Radio, tick: i64| {
            if a_first { step(world, &mut [a, b], tick) } else { step(world, &mut [b, a], tick) }
        };
        order(&world, &mut a, &mut b, 0);
        a.push(burst(&a, &world, waveform, 1_999, false));
        order(&world, &mut a, &mut b, 1_999_000);
        order(&world, &mut a, &mut b, 1_999_001);
        a.push(Action::Stop { target: Some(tgt("dev_a/tx/s0")) });
        order(&world, &mut a, &mut b, 2_000_500);
        order(&world, &mut a, &mut b, 6_000_001);
        (b.received(0), samples)
    };
    let (first, samples) = run(true);
    let (second, _) = run(false);
    assert_eq!(first, second);
    let at = |k: i64| first.iter().find(|(index, _, _)| *index == k).map(|(_, re, im)| (*re, *im)).unwrap();
    let near = |(re, im): (f32, f32), (want_re, want_im): (f32, f32)| (re - want_re).abs() < 0.01 && (im - want_im).abs() < 0.01;
    assert!(near(at(1_999), samples[0]));
    assert!(near(at(2_000), samples[1]));
    assert!(near(at(2_001), (0.0, 0.0)));
}

#[test]
fn mr_17_a_target_whose_instant_has_passed_is_late_even_on_the_floor_sample() {
    let world = World::new(json!({}));
    let mut radio = prepare(&world, Options { medium: false, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    step(&world, &mut [&mut radio], 0);
    let waveform = world.waveform(&ramp(10));
    let mut late = burst(&radio, &world, waveform, 1, false);
    if let Action::TxBurst { late_policy, .. } = &mut late {
        *late_policy = LatePolicy::DropAndFlag;
    }
    radio.push(late);
    step(&world, &mut [&mut radio], 1_500);
    let events = radio.drain_events();
    let time_errors: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
    assert_eq!(time_errors.len(), 1, "sample 1 is at 1 000 ns, before the round at 1 500 ns");
    assert_eq!(time_errors[0].payload["outcome"], "drop");
    assert_eq!(time_errors[0].payload["late_by_ns"], 1_000, "measured from the first transmit sample at or after the round");
    assert_eq!(time_errors[0].time, TimePoint::new(radio.tx_domain(&world), 1), "the event's time is the round's instant in the transmit clock, floored");
    assert_eq!(radio.section("rejected")[0]["reason"], "MR-17: the late policy dropped the burst");
}

#[test]
fn mr_25_a_stop_before_the_burst_with_a_new_value_cancels_it_and_keeps_the_value_of_record() {
    // S10 (R4, RM-21): a value taken on an empty stream, then a burst booked after it; a Stop before the burst's start cancels
    // it. The value stays the value of record: a later burst radiates at that frequency, and the cancelled one radiates nothing.
    let world = World::new(json!({ "sim.channel": { "couplings": [
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 },
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 1, "gain_db": 0.0 }
    ] } }));
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.frequency_hz", eq(Scalar::try_from(2.0e9).unwrap())),
        ("radio.rx.c0.frequency_hz", eq(Scalar::try_from(2.0e9).unwrap())),
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.frequency_hz", eq(Scalar::try_from(2.4e9).unwrap())),
    ];
    let mut radio = prepare(&world, Options { constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let domain = radio.tx_domain(&world);
    let (cancelled, later) = (ramp(40), ramp(40));
    radio.push(Action::UpdateParameter { target: tgt("mock"), key: key("radio.tx.c0.frequency_hz"), value: Value::num(2.4e9).unwrap(), class: UpdateClass::Cold, at: None });
    step(&world, &mut [&mut radio], 0);
    radio.push(burst(&radio, &world, world.waveform(&cancelled), 1000, false));
    step(&world, &mut [&mut radio], 0);
    let before_start = tx_root(&world, domain, 1000) - 1;
    run_to(&world, &mut [&mut radio], before_start);
    radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
    step(&world, &mut [&mut radio], before_start);
    radio.push(burst(&radio, &world, world.waveform(&later), 20_000, false));
    step(&world, &mut [&mut radio], before_start);
    run_to(&world, &mut [&mut radio], tx_root(&world, domain, 20_040) + 6_000_000);
    let mut channels = radio.received_all().into_iter();
    let (heard_connect, heard_value) = (channels.next().unwrap(), channels.next().unwrap());
    assert!(heard_connect.iter().all(|(_, re, im)| (*re, *im) == (0.0, 0.0)), "nothing radiates at the connect frequency");
    for (k, re, im) in heard_value {
        let expected = if (20_000..20_040).contains(&k) { later[(k - 20_000) as usize] } else { (0.0, 0.0) };
        assert_eq!((re, im), expected, "the cancelled burst is not sent, and the later one uses the value of record, sample {k}");
    }
    let rejected = radio.section("rejected");
    assert!(rejected.as_array().unwrap().iter().any(|row| row["reason"].as_str().unwrap().contains("cancelled by stop")), "{rejected}");
}

#[test]
fn mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter() {
    enum End {
        StopBefore,
        StopInTheRound,
    }
    for end in [End::StopBefore, End::StopInTheRound] {
        let world = World::new(loopback(None));
        let mut radio = prepare(&world, Options::default()).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
        let first = world.waveform(&ramp(1_000));
        let second = world.waveform(&[(0.0, 0.5)]);
        radio.push(burst(&radio, &world, first, 0, true));
        radio.push(burst(&radio, &world, second, 4_000, true));
        step(&world, &mut [&mut radio], 0);
        let (tick, reason) = match end {
            End::StopBefore => {
                run_to(&world, &mut [&mut radio], 3_999_499);
                radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
                (3_999_500, "overlaps an accepted booking")
            }
            End::StopInTheRound => {
                run_to(&world, &mut [&mut radio], 3_999_999);
                radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
                (4_000_000, "overlaps an accepted booking")
            }
        };
        step(&world, &mut [&mut radio], tick);
        run_to(&world, &mut [&mut radio], 9_000_001);
        step(&world, &mut [&mut radio], 9_000_001);
        let heard: Vec<_> = radio.received(0).into_iter().filter(|(_, re, im)| (*re, *im) != (0.0, 0.0)).map(|(k, _, _)| k).collect();
        assert_eq!(heard.len(), 4_000, "the first burst's 4 000 samples and nothing after the held start");
        assert_eq!(heard.last(), Some(&3_999));
        let records = records(&radio);
        assert_eq!(records.len(), 1);
        assert_eq!((records[0].samples, records[0].end), (4_000, BurstEnd::Stop), "the repeat is cut by the Stop at the held burst's start");
        let rejected = radio.section("rejected");
        assert_eq!(rejected.as_array().unwrap().len(), 1, "the held burst starts inside the open repeat: refused at receipt");
        assert!(rejected[0]["reason"].as_str().unwrap().contains(reason));
    }

    let (world, mut a, mut b) = two(a_into_b(), 1_000_000.0, 1_000_000.0);
    let first = world.waveform(&[(0.5, 0.0)]);
    let second = world.waveform(&[(0.0, 0.5)]);
    a.push(burst(&a, &world, first, 0, true));
    a.push(burst(&a, &world, second, 3_000, true));
    step(&world, &mut [&mut a, &mut b], 0);
    run_to(&world, &mut [&mut a, &mut b], 2_999_499);
    world.auth.advance_to(TimePoint::new(ROOT, 2_999_500)).unwrap();
    a.mock.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    run_to(&world, &mut [&mut a, &mut b], 8_000_001);
    step(&world, &mut [&mut a, &mut b], 8_000_001);
    let late: Vec<_> = b.received(0).into_iter().filter(|(k, re, im)| *k >= 3_000 && (*re, *im) != (0.0, 0.0)).collect();
    assert!(late.is_empty(), "Provider::stop before a held start leaves the transmitter silent, not {} samples", late.len());
}

#[test]
fn ch_09_a_faulted_round_does_not_depend_on_the_stepping_order() {
    // CH-9, MR-20: a `device_lost` in the round in which the receiver publishes its
    // first block must not change what that block holds, whichever radio the loop
    // steps first — the loss cuts the transmitter at the round's instant, never into
    // the samples before it.
    let run = |a_first: bool| {
        let world = World::new(json!({ "sim.seed": 5,
            "sim.channel": { "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0.0 }] },
            "sim.faults": [{ "at_ns": 1_999_001, "fault": "device_lost", "target": "a" }] }));
        let mut a = prepare(&world, Options { fragment: "a", id: "dev_a", ..Options::default() }).unwrap();
        let mut b = prepare(&world, Options { fragment: "b", id: "dev_b", ..Options::default() }).unwrap();
        for radio in [&mut a, &mut b] {
            radio.mock.arm().unwrap();
            radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
        }
        let samples = vec![(0.5f32, 0.25f32); 2_000];
        let waveform = world.waveform(&samples);
        // MR-20: the step that applies the loss fails (KC-30); MA-30's round goes on
        // over the other radios, and the harness does the same.
        let mut lost = false;
        let mut lossy = |world: &World, a: &mut Radio, b: &mut Radio, tick: i64| {
            world.auth.advance_to(TimePoint::new(ROOT, tick)).unwrap();
            for radio in if a_first { [a, b] } else { [b, a] } {
                if let Err(error) = radio.mock.step(TimePoint::new(ROOT, tick)) {
                    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost, "{}", error.message);
                    lost = true;
                }
            }
        };
        ordered(&world, &mut a, &mut b, 0, a_first);
        a.push(burst(&a, &world, waveform, 1_999, false));
        ordered(&world, &mut a, &mut b, 1_999_000, a_first);
        while let Some(due) = world.auth.next_due() {
            if due.ticks_in(ROOT).unwrap() > 1_999_001 {
                break;
            }
            lossy(&world, &mut a, &mut b, due.ticks_in(ROOT).unwrap());
        }
        assert!(lost, "a lost device was stepped in the faulted round");
        (b.received(0), samples)
    };
    let (first, samples) = run(true);
    let (second, _) = run(false);
    assert_eq!(first, second, "CH-9: the faulted round's received samples do not depend on the stepping order");
    let at = |k: i64| first.iter().find(|(index, _, _)| *index == k).map(|(_, re, im)| (*re, *im)).unwrap();
    let near = |(re, im): (f32, f32), (want_re, want_im): (f32, f32)| (re - want_re).abs() < 0.01 && (im - want_im).abs() < 0.01;
    assert!(near(at(1_998), (0.0, 0.0)), "the burst's first sample is at 1 999: {:?}", at(1_998));
    assert!(near(at(1_999), samples[0]), "the block published in the faulted round still hears the burst: {:?}", at(1_999));
}

#[test]
fn ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order() {
    let run = |a_first: bool| {
        let (world, mut a, mut b) = two(a_into_b(), 3_000_000.0, 3_000_000.0);
        let first = world.waveform(&ramp(999));
        let second = world.waveform(&[(0.0, 0.5)]);
        a.push(burst(&a, &world, first, 0, false));
        a.push(burst(&a, &world, second, 1_999, true));
        ordered(&world, &mut a, &mut b, 0, a_first);
        while let Some(due) = world.auth.next_due() {
            if due.ticks_in(ROOT).unwrap() >= 666_334 {
                break;
            }
            ordered(&world, &mut a, &mut b, due.ticks_in(ROOT).unwrap(), a_first);
        }
        a.push(Action::Stop { target: Some(tgt("dev_a/tx/s0")) });
        ordered(&world, &mut a, &mut b, 666_334, a_first);
        while let Some(due) = world.auth.next_due() {
            if due.ticks_in(ROOT).unwrap() > 2_000_001 {
                break;
            }
            ordered(&world, &mut a, &mut b, due.ticks_in(ROOT).unwrap(), a_first);
        }
        ordered(&world, &mut a, &mut b, 2_000_001, a_first);
        (b.received(0), records(&a))
    };
    let (first, records_a_first) = run(true);
    let (second, records_b_first) = run(false);
    assert_eq!(first, second, "CH-9: the receiver's samples do not depend on the stepping order");
    assert_eq!(records_a_first, records_b_first);
    let at = |k: i64| first.iter().find(|(index, _, _)| *index == k).map(|(_, re, im)| (*re, *im)).unwrap();
    assert_eq!(at(1_999), (0.0, 0.5), "sample 1 999 (666 333⅓ ns) is before the stop's round and belongs to the second burst");
    assert_eq!(at(2_000), (0.0, 0.0));
    assert_eq!(records_a_first.len(), 2);
    assert_eq!((records_a_first[0].samples, records_a_first[0].end), (999, BurstEnd::Eob));
    assert_eq!((records_a_first[1].samples, records_a_first[1].end), (1, BurstEnd::Stop), "the second burst transmitted its one sample before the stop");
}

#[test]
fn mr_32_a_two_channel_waveform_is_channel_interleaved() {
    let world = World::new(json!({ "sim.channel": { "couplings": [
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 },
        { "tx": "radio", "tx_channel": 1, "rx": "radio", "rx_channel": 1, "gain_db": 0.0 }
    ] } }));
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c1.stream", eq(Scalar::from(0))), ("radio.rx.c1.stream", eq(Scalar::from(0)))];
    let mut radio = prepare(&world, Options { constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let interleaved: Vec<(f32, f32)> = (0..2_000).flat_map(|n| [((n + 1) as f32 / 4096.0, 0.0), (0.0, (n + 1) as f32 / 4096.0)]).collect();
    let waveform = world.waveform(&interleaved);
    radio.push(Action::TxBurst {
        target: tgt("mock/tx/s0"),
        waveform,
        repeat: false,
        at: AbsoluteDeadline::new(TimePoint::new(radio.tx_domain(&world), 0)),
        requested_at: None,
        late_policy: LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    });
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 2_000_001);
    let blocks: Vec<_> = std::iter::from_fn(|| radio.link.receive().map(|(b, _)| b)).collect();
    assert_eq!(blocks.len(), 1);
    let block = &blocks[0];
    for index in 0..2_000usize {
        let expected = (index + 1) as f32 / 4096.0;
        assert_eq!(ezsdr_hostmem::read_cf32(block.host_bytes().unwrap(), 2_000, 0, index), (expected, 0.0), "channel 0 sample {index}");
        assert_eq!(ezsdr_hostmem::read_cf32(block.host_bytes().unwrap(), 2_000, 1, index), (0.0, expected), "channel 1 sample {index}");
    }
}

#[test]
fn mr_25_a_stop_on_a_sample_instant_does_not_transmit_that_sample() {
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options::default()).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let waveform = world.waveform(&[(0.5, 0.25)]);
    radio.push(burst(&radio, &world, waveform, 0, true));
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 2_000_001);
    radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
    step(&world, &mut [&mut radio], 2_500_000);
    step(&world, &mut [&mut radio], 4_000_001);
    assert_eq!(records(&radio)[0].samples, 2_500, "sample 2 500 is at the stop instant 2 500 000 and is not transmitted");
    let last_heard = radio.received(0).into_iter().filter(|(_, re, im)| (*re, *im) != (0.0, 0.0)).map(|(k, _, _)| k).max();
    assert_eq!(last_heard, Some(2_499));
}

#[test]
fn mr_31_the_medium_takes_the_run_s_seed() {
    let sigma = (10f64.powf(-20.0 / 10.0) / 2.0).sqrt();
    for seed in [7u64, 8] {
        let world = World::new(json!({ "sim.seed": seed, "sim.channel": { "couplings": [], "noise_dbfs": { "radio": -20.0 } } }));
        let mut radio = prepare(&world, Options::default()).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
        step(&world, &mut [&mut radio], 0);
        step(&world, &mut [&mut radio], 2_000_001);
        let received = radio.received(0);
        let (g1, g2) = ezsdr_sim::channel::gaussian_pair(&mut SimRng::new(seed, "sim.channel/radio/0"));
        let (k, re, im) = received[0];
        assert_eq!(k, 0);
        assert!((f64::from(re) - sigma * g1).abs() < 1e-6 && (f64::from(im) - sigma * g2).abs() < 1e-6, "seed {seed}: ({re}, {im})");
    }
}

#[test]
fn mr_35_the_receive_gain_scales_the_noise_with_the_signal() {
    // MR-35: the receive gain scales "the field, noise included". A receive gain that
    // scaled only the signal would leave the noise at its input-referred power, so a
    // 0 dB test cannot tell the two apart — the noise is the only thing here.
    let sigma = (10f64.powf(-20.0 / 10.0) / 2.0).sqrt();
    let seed = 7u64;
    let world = World::new(json!({ "sim.seed": seed, "sim.channel": { "couplings": [], "noise_dbfs": { "radio": -20.0 } } }));
    let constraints = [("radio.rx.c0.gain_db", eq(Scalar::try_from(6.0).unwrap()))];
    let mut radio = prepare(&world, Options { constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 2_000_001);
    let (g1, g2) = ezsdr_sim::channel::gaussian_pair(&mut SimRng::new(seed, "sim.channel/radio/0"));
    let up = 10f64.powf(6.0 / 20.0);
    let (k, re, im) = radio.received(0)[0];
    assert_eq!(k, 0);
    assert!(
        (f64::from(re) - up * sigma * g1).abs() < 1e-6 && (f64::from(im) - up * sigma * g2).abs() < 1e-6,
        "the receive gain must scale the noise: ({re}, {im}) vs ({}, {})",
        up * sigma * g1,
        up * sigma * g2
    );
    assert!((up * sigma * g1 - sigma * g1).abs() > 1e-6, "a +6 dB gain must move the noise, or this proves nothing");
}

#[test]
fn mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused() {
    let refused = "MR-16: burst overlaps an accepted booking";
    let started = |environment: serde_json::Value, medium: bool, waveform: &[(f32, f32)]| {
        let world = World::new(environment);
        let mut radio = prepare(&world, Options { medium, ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
        let first = world.waveform(waveform);
        radio.push(burst(&radio, &world, first, 1_000, true));
        step(&world, &mut [&mut radio], 0);
        (world, radio)
    };
    let summary = |radio: &Radio| records(radio).iter().map(|record| (record.samples, record.end)).collect::<Vec<_>>();
    let samples = ramp(1_000);

    // at the start of a burst the round at sample 1 000 opened, which has transmitted nothing;
    // with and without a channel (Z11)
    for (environment, medium) in [(loopback(None), true), (json!({}), false)] {
        let (world, mut radio) = started(environment, medium, &samples);
        step(&world, &mut [&mut radio], 1_000_000);
        let second = world.waveform(&[(0.0, 0.5)]);
        radio.push(burst(&radio, &world, second, 1_000, true));
        step(&world, &mut [&mut radio], 1_000_000);
        radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
        step(&world, &mut [&mut radio], 3_000_500);
        step(&world, &mut [&mut radio], 6_000_001);
        assert_eq!(radio.section("rejected")[0]["reason"], refused);
        assert_eq!(summary(&radio), vec![(2_001, BurstEnd::Stop)]);
        if medium {
            for (k, re, im) in radio.received(0) {
                let want = if (1_000..=3_000).contains(&k) { samples[((k - 1_000) % 1_000) as usize] } else { (0.0, 0.0) };
                assert_eq!((re, im), want, "sample {k}");
            }
        }
    }

    // at the next sample of a burst whose block ending there has been emitted without END_OF_BURST:
    // a one-sample waveform repeats in one-sample blocks, so the round at 1 000 001 ns emits
    // [1 000, 1 001) and the open burst's next sample is 1 001; a burst one sample later is admitted
    let (world, mut radio) = started(loopback(None), true, &[(0.5, 0.0)]);
    run_to(&world, &mut [&mut radio], 1_000_001);
    let second = world.waveform(&[(0.0, 0.5)]);
    let third = world.waveform(&[(0.25, 0.25)]);
    radio.push(burst(&radio, &world, second, 1_001, true));
    radio.push(burst(&radio, &world, third, 1_002, true));
    step(&world, &mut [&mut radio], 1_000_001);
    run_to(&world, &mut [&mut radio], 1_500_000);
    radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
    step(&world, &mut [&mut radio], 1_500_500);
    run_to(&world, &mut [&mut radio], 3_000_001);
    // Both bookings start inside the open repeat's span, which holds every sample until its Stop (RM-15): both are refused at receipt.
    assert_eq!(radio.section("rejected").as_array().unwrap().len(), 2);
    assert!(radio.section("rejected").as_array().unwrap().iter().all(|row| row["reason"] == refused));
    assert_eq!(summary(&radio), vec![(501, BurstEnd::Stop)]);
    for (k, re, im) in radio.received(0) {
        let want = match k {
            1_000..=1_500 => (0.5, 0.0),
            _ => (0.0, 0.0),
        };
        assert_eq!((re, im), want, "sample {k}");
    }

    // a late burst for sample 500, judged where MR-17's SendAsap moves it (no lead on `ideal`):
    // onto the open burst's start in the round at 1 000 000 ns, refused with TIME_ERROR { refused };
    // in the round at 1 000 500 ns, refused at receipt too: the open repeat holds its span until its Stop
    let (world, mut radio) = started(loopback(None), true, &samples);
    step(&world, &mut [&mut radio], 1_000_000);
    radio.drain_events();
    let second = world.waveform(&[(0.0, 0.5)]);
    radio.push(burst(&radio, &world, second, 500, true));
    step(&world, &mut [&mut radio], 1_000_000);
    let events = radio.drain_events();
    assert_eq!(radio.section("rejected")[0]["reason"], refused);
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR && event.payload["outcome"] == "refused").count(), 1);
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 1);

    let (world, mut radio) = started(loopback(None), true, &samples);
    step(&world, &mut [&mut radio], 1_000_000);
    let second = world.waveform(&[(0.0, 0.5)]);
    radio.push(burst(&radio, &world, second, 500, true));
    step(&world, &mut [&mut radio], 1_000_500);
    radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
    step(&world, &mut [&mut radio], 3_000_500);
    step(&world, &mut [&mut radio], 6_000_001);
    assert_eq!(radio.section("rejected").as_array().unwrap().len(), 1);
    assert_eq!(radio.section("rejected")[0]["reason"], refused);
    assert_eq!(summary(&radio), vec![(2_001, BurstEnd::Stop)]);
    for (k, re, im) in radio.received(0) {
        let want = match k {
            1_000..=3_000 => samples[((k - 1_000) % 1_000) as usize],
            _ => (0.0, 0.0),
        };
        assert_eq!((re, im), want, "sample {k}");
    }
}

#[test]
fn mr_15_a_transmit_block_is_emitted_only_after_its_last_sample() {
    // a round at the instant of sample 999, the last of the first block, is stepped twice, and an
    // Action arrives for its second pass: sample 999 has not been transmitted in either pass;
    // with a loopback channel and without one (Z11)
    let first_pass = |environment: serde_json::Value, medium: bool, waveform: &[(f32, f32)], repeat: bool| {
        let world = World::new(environment);
        let mut radio = prepare(&world, Options { medium, ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
        let first = world.waveform(waveform);
        radio.push(burst(&radio, &world, first, 0, repeat));
        step(&world, &mut [&mut radio], 0);
        run_to(&world, &mut [&mut radio], 999_000);
        step(&world, &mut [&mut radio], 999_000);
        (world, radio)
    };
    let summary = |radio: &Radio| records(radio).iter().map(|record| (record.samples, record.end)).collect::<Vec<_>>();
    let samples = ramp(1_000);

    for (environment, medium) in [(loopback(None), true), (json!({}), false)] {
        // a stop: the burst transmits samples 0 … 998, is recorded with 999 and radiates 999
        let (world, mut radio) = first_pass(environment.clone(), medium, &samples, true);
        radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
        step(&world, &mut [&mut radio], 999_000);
        run_to(&world, &mut [&mut radio], 3_000_001);
        step(&world, &mut [&mut radio], 3_000_001);
        assert_eq!(summary(&radio), vec![(999, BurstEnd::Stop)]);
        if medium {
            for (k, re, im) in radio.received(0) {
                let want = if k < 999 { samples[k as usize] } else { (0.0, 0.0) };
                assert_eq!((re, im), want, "sample {k}");
            }
        }

        // a burst at sample 999, inside a finite burst that has not ended (its span runs to 1 001): it is refused at
        // receipt, so the first burst keeps sample 999 and no two records hold one sample
        let (world, mut radio) = first_pass(environment, medium, &samples, false);
        let second = world.waveform(&[(0.0, 0.5)]);
        radio.push(burst(&radio, &world, second, 999, true));
        step(&world, &mut [&mut radio], 999_000);
        run_to(&world, &mut [&mut radio], 1_500_000);
        radio.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
        step(&world, &mut [&mut radio], 1_500_500);
        run_to(&world, &mut [&mut radio], 3_000_001);
        step(&world, &mut [&mut radio], 3_000_001);
        assert_eq!(radio.section("rejected").as_array().unwrap().len(), 1);
        assert!(radio.section("rejected")[0]["reason"].as_str().unwrap().contains("overlaps an accepted booking"));
        assert_eq!(summary(&radio), vec![(1_000, BurstEnd::Eob)]);
        if medium {
            for (k, re, im) in radio.received(0) {
                let want = match k {
                    0..=999 => samples[k as usize],
                    _ => (0.0, 0.0),
                };
                assert_eq!((re, im), want, "sample {k}");
            }
        }
    }
}

#[test]
fn mr_25_a_stop_transmits_every_held_burst_before_it() {
    // the Mocks are stepped at 0, 1 ms and the stop only, so three held bursts start before the stop's round
    let run = |a_first: bool| {
        let (world, mut a, mut b) = two(a_into_b(), 1_000_000.0, 1_000_000.0);
        let first = world.waveform(&ramp(1_000));
        let second = world.waveform(&[(0.0, 0.5)]);
        let third = world.waveform(&ramp(100));
        let fourth = world.waveform(&[(0.25, 0.25)]);
        a.push(burst(&a, &world, first, 0, false));
        a.push(burst(&a, &world, second, 1_500, false));
        a.push(burst(&a, &world, third, 2_500, false));
        a.push(burst(&a, &world, fourth, 3_000, true));
        ordered(&world, &mut a, &mut b, 0, a_first);
        ordered(&world, &mut a, &mut b, 1_000_000, a_first);
        a.push(Action::Stop { target: Some(tgt("dev_a/tx/s0")) });
        ordered(&world, &mut a, &mut b, 3_200_500, a_first);
        ordered(&world, &mut a, &mut b, 8_000_001, a_first);
        let summary: Vec<_> = records(&a).iter().map(|record| (record.samples, record.end)).collect();
        (b.received(0), summary, a.section("rejected"))
    };
    let (received, summary, rejected) = run(true);
    assert_eq!((received.clone(), summary.clone(), rejected.clone()), run(false));
    assert_eq!(summary, vec![(1_000, BurstEnd::Eob), (1, BurstEnd::Eob), (100, BurstEnd::Eob), (201, BurstEnd::Stop)]);
    assert_eq!(rejected, json!([]));
    let (first, third) = (ramp(1_000), ramp(100));
    for (k, re, im) in received {
        let want = match k {
            0..=999 => first[k as usize],
            1_500 => (0.0, 0.5),
            2_500..=2_599 => third[(k - 2_500) as usize],
            3_000..=3_200 => (0.25, 0.25),
            _ => (0.0, 0.0),
        };
        assert_eq!((re, im), want, "sample {k}");
    }
}
