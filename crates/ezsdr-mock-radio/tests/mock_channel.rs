//! MockRadio on the SimulationChannel (spec 09 as amended in Phase 3, MR-25, MR-31…MR-36).

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{Action, ActionId, Event, EventCollector};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Endpoint, ExecutionClass, ModuleError, ModuleRef,
    Pacing, PrepareContext, ProfileRef, Provider, Requested, RfFidelity, Role, UpdateClass, Version,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BackPressure, BlockRef, BurstEnd, BurstRecord, DataLink, DropCarry, LatePolicy, PublishOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, RelativeBudget, TimePoint};
use ezsdr_mock_radio::MockRadio;
use ezsdr_sim::channel::Medium;
use ezsdr_sim::SimRng;
use serde_json::json;

const ROOT: ClockDomainId = ClockDomainId::local(7);

fn rid(name: &str) -> ResourceId { ResourceId::parse(name).unwrap() }
fn key(name: &str) -> Key { Key::parse(name).unwrap() }
fn eq(value: Value) -> Constraint { Constraint::Eq { value } }

fn module_ref() -> ModuleRef {
    ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(1, 4, 0) }
}

#[derive(Default)]
struct Queue(Mutex<VecDeque<Action>>);
impl ActionReceiver for Queue {
    fn recv(&self) -> Option<Action> { self.0.lock().unwrap().pop_front() }
}

struct Accepting;
impl ActionSubmitter for Accepting {
    fn submit(&self, _action: Action) -> Result<ActionId, Vec<Violation>> { Ok(ActionId(1)) }
}

#[derive(Default)]
struct Sink {
    queue: Mutex<VecDeque<BlockRef>>,
    drops: AtomicU64,
}
impl DataLink for Sink {
    fn publish(&self, block: BlockRef) -> PublishOutcome { self.queue.lock().unwrap().push_back(block); PublishOutcome::Accepted }
    fn receive(&self) -> Option<BlockRef> { self.queue.lock().unwrap().pop_front() }
    fn drops(&self) -> u64 { self.drops.load(Ordering::Relaxed) }
    fn take_drop_carry(&self) -> DropCarry { DropCarry::default() }
    fn policy(&self) -> BackPressure { BackPressure::DropOldest }
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
        Options { profile: "ideal", fragment: "radio", id: "mock", constraints: &[("radio.tx.channels", Constraint::Eq { value: Value::Int(1) })], selector: &[], medium: true }
    }
}

fn prepare(world: &World, options: Options<'_>) -> Result<Radio, ModuleError> {
    let mut selector = BTreeMap::from([(Ident::parse("id").unwrap(), Value::Str(options.id.to_owned()))]);
    for (name, value) in options.selector {
        selector.insert(Ident::parse(name).unwrap(), value.clone());
    }
    let binding = Binding { module: module_ref(), selector, profile: Some(ProfileRef { name: options.profile.to_owned(), version: Version::new(1, 1, 0) }), feed: None };
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
    let pairs: Vec<_> = [rid(&id), rid(&format!("{id}/rx")), rid(&format!("{id}/tx"))].into_iter()
        .flat_map(|source| event_kinds.iter().cloned().map(move |kind| (source.clone(), kind)))
        .collect();
    let events = Arc::new(EventCollector::new(&pairs, &event_kinds, 4096, &Policy::default()));
    let actions = Arc::new(Queue::default());
    let link = Arc::new(Sink::default());
    let ctx = PrepareContext {
        run: RunId::from_string("mock-channel".to_owned()),
        class: ExecutionClass::Simulation,
        time: world.auth.clone(),
        clocks: world.clocks.clone(),
        events: events.clone(),
        actions: actions.clone(),
        actions_out: Arc::new(Accepting),
        environment: world.environment.clone(),
        inputs: world.inputs.clone(),
        links: vec![AttachedPort { component: Ident::parse(options.fragment).unwrap(), port: Ident::parse("rx").unwrap(), endpoint: Endpoint::StreamOut(link.clone()) }],
        components: BTreeMap::new(),
        host_budget: RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap(),
    };
    let requested = Requested {
        resource: rid(&id),
        constraints: options.constraints.iter().map(|(name, constraint)| (key(name), constraint.clone())).collect(),
    };
    let fragment = Fragment {
        id: Ident::parse(options.fragment).unwrap(),
        instance: module_ref(),
        role: Role::Provider,
        content: json!({ "selector": {}, "requested": requested }),
        after: Vec::new(),
    };
    mock.prepare(&fragment, ctx)?;
    Ok(Radio { mock, id, events, actions, link })
}

impl Radio {
    fn push(&self, action: Action) { self.actions.0.lock().unwrap().push_back(action); }

    fn tx_domain(&self, world: &World) -> ClockDomainId {
        world.clocks.sample_clock_records().iter().rev().find(|record| record.stream == rid(&format!("{}/tx", self.id))).unwrap().domain
    }

    /// Every received sample of channel `channel`, by sample index, in arrival order.
    fn received(&self, channel: usize) -> Vec<(i64, f32, f32)> {
        let mut out = Vec::new();
        while let Some(block) = self.link.receive() {
            let header = block.header().clone();
            let bytes = block.host_bytes().unwrap();
            for index in 0..header.len as usize {
                let (re, im) = ezsdr_hostmem::read_cf32(bytes, header.len as usize, channel, index);
                out.push((header.first_sample_time.ticks + index as i64, re, im));
            }
        }
        out
    }

    fn section(&self, name: &str) -> serde_json::Value {
        // MR-27: every section name carries this instance's own id, so two Mocks in one
        // Run do not overwrite each other's records.
        self.mock.instance().sections[&Namespace::parse(&format!("ezsdr.radio.mock.{}.{name}", self.id)).unwrap()].clone()
    }

    fn drain_events(&self) -> Vec<Event> { self.events.drain() }
}

fn burst(radio: &Radio, world: &World, waveform: ArtifactRef, at: i64, repeat: bool) -> Action {
    Action::TxBurst {
        target: rid(&format!("{}/tx", radio.id)),
        waveform,
        repeat,
        at: AbsoluteDeadline::new(TimePoint::new(radio.tx_domain(world), at)),
        requested_at: None,
        late_policy: LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    }
}

fn update(radio: &Radio, name: &str, value: Value, at: i64) -> Action {
    Action::UpdateParameter { target: rid(&radio.id), key: key(name), value, class: UpdateClass::HardwareTimed, at: Some(AbsoluteDeadline::new(TimePoint::new(ROOT, at))) }
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
    let a_constraints = [("radio.tx.channels", eq(Value::Int(1))), ("radio.tx.sample_rate_hz", eq(Value::Num(a_rate))), ("radio.rx.sample_rate_hz", eq(Value::Num(a_rate)))];
    let b_constraints = [("radio.tx.channels", eq(Value::Int(1))), ("radio.tx.sample_rate_hz", eq(Value::Num(b_rate))), ("radio.rx.sample_rate_hz", eq(Value::Num(b_rate)))];
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
        if due.ticks > until { break; }
        step(world, radios, due.ticks);
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
    let ramp_pattern = [("rx_test_pattern", Value::Str("ramp".to_owned()))];
    assert!(refusal(prepare(&world, Options { selector: &ramp_pattern, ..Options::default() })).starts_with("MR-31: a channel-coupled Mock takes no receive test pattern"));

    let wide = World::new(json!({ "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 2, "rx": "radio", "rx_channel": 0, "gain_db": 0 }] } }));
    assert!(refusal(prepare(&wide, Options { profile: "x310-like", ..Options::default() })).contains("names a channel beyond this profile's 2"));
    let deep = World::new(json!({ "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 2, "gain_db": 0 }] } }));
    assert!(refusal(prepare(&deep, Options { profile: "x310-like", ..Options::default() })).contains("names a channel beyond this profile's 2"));

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
    assert_eq!(reasons, [
        "MR-32: the waveform's bytes are not an input of this Run",
        "MR-32: the waveform's bytes do not have its size_bytes",
        "RM-13: a waveform sample is not finite",
    ]);
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
        let absent = ezsdr_kernel::manifest::ingest_input(
            Ident::parse("waveform").unwrap(),
            Namespace::parse("ezsdr.input").unwrap(),
            "mem:none",
            &[0u8; 80],
        );
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
        // `x310-like` aligns a repeated waveform to two samples, so eleven samples
        // break the repeat constraint. `ideal` aligns to one and would not.
        let t0 = 2_000_000_000;
        let world = World::new(loopback(None));
        let mut radio = prepare(&world, Options { profile: "x310-like", ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
        step(&world, &mut [&mut radio], t0);
        let absent = ezsdr_kernel::manifest::ingest_input(
            Ident::parse("waveform").unwrap(),
            Namespace::parse("ezsdr.input").unwrap(),
            "mem:none",
            &[0u8; 88],
        );
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
fn mr_32_b_the_transmitted_timelines_are_read_at_the_samples_own_instant() {
    // MR-32 and MR-33: the gain, the phase and the frequency are the ones in force at the
    // sample's own instant, not at the antenna instant. `x310-like` advances the transmit
    // side by 45 samples, so the two instants differ by 45 000 root ticks at 1 Msps. A
    // timed tune inside that window would move the transmitted value 45 samples early if
    // the timelines were read at the antenna instant, leaving a 45-sample window in which
    // the transmitter and the receiver disagree: the frequency gate goes silent and the
    // LO phase is the wrong one. Correct behaviour is that no sample is lost or rotated
    // early, because the receiver asks at the very instant the transmitter read.
    let t0 = 2_000_000_000;
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options { profile: "x310-like", ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
    // two samples, because `x310-like` aligns a repeated waveform to two and a
    // one-sample waveform would be refused by MR-16's repeat constraints
    let waveform = world.waveform(&[(0.5, 0.0), (0.5, 0.0)]);
    // MR-9 registers the transmit clock at the arm instant, so transmit sample 2 002 000
    // is the first at or after `x310-like`'s synchronisation end. Receive sample `k` is
    // at t0 + 1000k and the loopback path has no delay, so it hears transmit sample
    // 2 000 000 + k, less the profile's 45-sample advance: heard from k = 2 045.
    radio.push(burst(&radio, &world, waveform, 2_002_000, true));
    let switch = 2_002_500_000;
    radio.push(update(&radio, "radio.tx.frequency_hz", Value::Num(1.0e9), switch));
    radio.push(update(&radio, "radio.rx.frequency_hz", Value::Num(1.0e9), switch));
    // handle the burst in the round at t0, where its target is inside the 2 ms lead; a
    // later round would find it late and MR-17 would move it
    step(&world, &mut [&mut radio], t0);
    // the receive block holding k = 2 545 is published at T0 + 3 999 001
    run_to(&world, &mut [&mut radio], 2_004_000_000);

    let Phases { rx, tx, rx_timed, tx_timed } = phases(0, "mock");
    assert!((tx[0] - tx_timed[0]).abs() > 1e-3, "the timed tune must move the transmit phase, or this test proves nothing");
    assert!((rx[0] - rx_timed[0]).abs() > 1e-3, "the timed tune must move the receive phase, or this test proves nothing");
    // Every sample from k = 2 045 on must be heard. The frequency updates here are to the
    // value already in force, so only the phase changes; `mr_32_c_…` moves the frequency.
    let heard: Vec<_> = radio.received(0).into_iter().filter(|(k, _, _)| *k >= 2_045).collect();
    assert!(heard.len() > 600, "the burst must be heard well past the tune, {} samples", heard.len());
    assert!(heard.iter().any(|(k, _, _)| *k < 2_500) && heard.iter().any(|(k, _, _)| *k >= 2_545), "both sides of both tunes must be heard");
    for (k, re, im) in &heard {
        assert!(*re != 0.0 || *im != 0.0, "sample {k} is silent");
        // The receive phase is read at the receive sample's own instant, so the receive
        // tune reaches k = 2 500. The transmit phase is read at the *transmitted*
        // sample's own instant, which is 45 samples earlier than the instant it is
        // radiated at, so the transmit tune reaches k = 2 545. Reading it at the antenna
        // instant instead would move that boundary to k = 2 500 and lose the middle
        // region. What is heard is the transmit phase minus the receive phase
        // (MR-32 and MR-35).
        let phase = match k {
            k if *k < 2_500 => tx[0] - rx[0],
            k if *k < 2_545 => tx[0] - rx_timed[0],
            _ => tx_timed[0] - rx_timed[0],
        };
        let (want_re, want_im) = (0.5 * phase.cos(), 0.5 * phase.sin());
        assert!(
            (f64::from(*re) - want_re).abs() < 1e-6 && (f64::from(*im) - want_im).abs() < 1e-6,
            "sample {k}: ({re}, {im}) vs ({want_re}, {want_im})"
        );
    }
}

#[test]
fn mr_32_c_the_transmitted_frequency_is_read_at_the_samples_own_instant() {
    // MR-33 and CH-4: a sample takes the frequency in force at its own instant, and CH-4's
    // gate is exact equality. A transmitted sample is stamped 45 samples before the instant
    // it is radiated at (RM-23's `n_tx`), so a simultaneous retune of both directions
    // leaves the transmitter on the new frequency 45 samples before the receiver asks for
    // it, and CH-4's gate silences those samples. This is what MR-32 with UC-6 and C4
    // imply; whether a real X310's LO acts before or after the 45-sample advance is a
    // hardware question Phase 8 measures, so the window is pinned as specified rather than
    // argued for. Reading the transmit frequency at the antenna instant would close it.
    let t0 = 2_000_000_000;
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options { profile: "x310-like", ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
    let waveform = world.waveform(&[(0.5, 0.0), (0.5, 0.0)]);
    radio.push(burst(&radio, &world, waveform, 2_002_000, true));
    let switch = 2_002_500_000;
    radio.push(update(&radio, "radio.tx.frequency_hz", Value::Num(2.0e9), switch));
    radio.push(update(&radio, "radio.rx.frequency_hz", Value::Num(2.0e9), switch));
    step(&world, &mut [&mut radio], t0);
    run_to(&world, &mut [&mut radio], 2_004_000_000);

    let heard: Vec<_> = radio.received(0).into_iter().filter(|(k, _, _)| *k >= 2_045).collect();
    let silent: Vec<i64> = heard.iter().filter(|(_, re, im)| *re == 0.0 && *im == 0.0).map(|(k, _, _)| *k).collect();
    assert_eq!(
        (silent.first().copied(), silent.last().copied(), silent.len()),
        (Some(2_500), Some(2_544), 45),
        "the window in which the two sides of the gate disagree is exactly 2 500..=2 544"
    );
    assert!(heard.iter().any(|(k, _, _)| *k >= 2_545), "the burst must be heard again after the window");
}

#[test]
fn mr_34_a_a_timed_tune_reaches_every_transmit_channel() {
    // MR-34: after a timed tune every channel of that direction takes the same timed-tune
    // constant, not only the first. `mr_34_a_timed_tune_…` runs one transmit channel, so
    // a loop that stopped after it would pass there.
    let t0 = 2_000_000_000;
    let environment = json!({
        "sim.seed": 1,
        "sim.channel": { "couplings": [
            { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 },
            { "tx": "radio", "tx_channel": 1, "rx": "radio", "rx_channel": 1, "gain_db": 0.0 }
        ] }
    });
    let world = World::new(environment);
    let constraints = [("radio.tx.channels", eq(Value::Int(2))), ("radio.rx.channels", eq(Value::Int(2)))];
    let mut radio = prepare(&world, Options { profile: "x310-like", constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
    // Two samples on each of the two transmit channels: 32 bytes, which is a length of 2
    // samples per channel and satisfies `x310-like`'s alignment of 2. One sample per
    // channel would be a length of 1 and refused by MR-16's repeat constraints.
    let waveform = world.waveform(&[(0.5, 0.0), (0.25, 0.0), (0.5, 0.0), (0.25, 0.0)]);
    radio.push(burst(&radio, &world, waveform, 2_002_000, true));
    radio.push(update(&radio, "radio.tx.frequency_hz", Value::Num(1.0e9), t0 + 4_000_000));
    // handle the burst in the round at t0, where its target is inside the 2 ms lead
    step(&world, &mut [&mut radio], t0);
    run_to(&world, &mut [&mut radio], t0 + 10_000_001);

    let Phases { rx, tx, tx_timed, .. } = phases(1, "mock");
    // receive sample k is at t0 + 1000k and hears transmit sample 2 000 000 + k − 45, so
    // the burst is heard from k = 2 045 and the timed tune at t0 + 4 ms, which is transmit
    // sample 2 004 000, reaches the transmitted sample at k = 4 045. Only the transmit
    // direction is retuned, so the receive phase stays at rx[0] throughout.
    let (before, after) = (3_000, 5_000);
    let blocks: Vec<_> = std::iter::from_fn(|| radio.link.receive()).collect();
    let read = |k: i64, channel: usize| {
        let block = blocks.iter().find(|b| {
            b.header().first_sample_time.ticks <= k && k < b.header().first_sample_time.ticks + i64::from(b.header().len)
        }).unwrap();
        let index = (k - block.header().first_sample_time.ticks) as usize;
        ezsdr_hostmem::read_cf32(block.host_bytes().unwrap(), block.header().len as usize, channel, index)
    };
    for channel in 0..2 {
        assert!(
            (tx[channel] - tx_timed[channel]).abs() > 1e-3,
            "the timed tune must move transmit channel {channel}'s phase, or this test proves nothing"
        );
        let amplitude = if channel == 0 { 0.5 } else { 0.25 };
        for (k, phase) in [(before, tx[channel] - rx[channel]), (after, tx_timed[channel] - rx[channel])] {
            let (re, im) = read(k, channel);
            let (want_re, want_im) = (amplitude * phase.cos(), amplitude * phase.sin());
            assert!(
                (f64::from(re) - want_re).abs() < 1e-6 && (f64::from(im) - want_im).abs() < 1e-6,
                "transmit channel {channel} at k {k}: ({re}, {im}) vs ({want_re}, {want_im})"
            );
        }
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
        radio.push(Action::Stop { target: Some(rid("mock/tx")) });
        step(&world, &mut [&mut radio], 2_500_500);
        step(&world, &mut [&mut radio], 6_000_001);
        let records: Vec<BurstRecord> = serde_json::from_value(radio.section("bursts")).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].end, BurstEnd::Stop);
        assert_eq!(records[0].samples, 2_501, "the samples before the stop instant 2 500 500 are 0 … 2 500");
        let rejected = radio.section("rejected");
        assert_eq!(rejected.as_array().unwrap().len(), 1, "the held burst is cancelled by the stop");
        assert!(rejected[0]["reason"].as_str().unwrap().contains("cancelled by stop"));
        if channelled {
            for (k, re, im) in radio.received(0) {
                let expected = if k <= 2_500 { samples[(k % 1_000) as usize] } else { (0.0, 0.0) };
                assert_eq!((re, im), expected, "sample {k}");
            }
        }
    }
}

#[test]
fn mr_33_gain_scales_the_samples_from_its_instant() {
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options::default()).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let waveform = world.waveform(&[(0.5, 0.25)]);
    radio.push(burst(&radio, &world, waveform, 0, true));
    radio.push(update(&radio, "radio.tx.gain_db", Value::Num(-6.0), 1_000_500));
    radio.push(update(&radio, "radio.rx.gain_db", Value::Num(6.0), 3_000_000));
    radio.push(update(&radio, "radio.tx.gain_db", Value::Num(-12.0), 4_000_000));
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 6_000_001);
    let down = 10f64.powf(-6.0 / 20.0);
    let up = 10f64.powf(6.0 / 20.0);
    let further = 10f64.powf(-12.0 / 20.0);
    for (k, re, im) in radio.received(0) {
        let scale = if k <= 1_000 { 1.0 } else if k < 3_000 { down } else if k < 4_000 { down * up } else { further * up };
        assert!((f64::from(re) - 0.5 * scale).abs() < 1e-6 && (f64::from(im) - 0.25 * scale).abs() < 1e-6, "sample {k}: ({re}, {im})");
    }
}

#[test]
fn mr_33_a_receiver_hears_only_its_own_frequency() {
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options::default()).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let waveform = world.waveform(&[(0.5, 0.25)]);
    radio.push(burst(&radio, &world, waveform, 0, true));
    radio.push(update(&radio, "radio.tx.frequency_hz", Value::Num(2.0e9), 1_000_000));
    radio.push(update(&radio, "radio.rx.frequency_hz", Value::Num(2.0e9), 3_000_000));
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 6_000_001);
    for (k, re, im) in radio.received(0) {
        let heard = !(1_000..3_000).contains(&k);
        assert_eq!((re, im), if heard { (0.5, 0.25) } else { (0.0, 0.0) }, "sample {k}");
    }
}

/// The draws of MR-34 for a two-channel `x310-like` device: the receive and transmit
/// phases after the untimed tune, then after every timed tune.
struct Phases { rx: Vec<f64>, tx: Vec<f64>, rx_timed: Vec<f64>, tx_timed: Vec<f64> }

fn phases(seed: u64, device: &str) -> Phases {
    let mut rng = SimRng::new(seed, &format!("{device}/lo"));
    let mut draw = || 2.0 * std::f64::consts::PI * ((rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64);
    let mut pair = || vec![draw(), draw()];
    Phases { rx: pair(), tx: pair(), rx_timed: pair(), tx_timed: pair() }
}

#[test]
fn mr_34_a_timed_tune_replaces_the_random_phase_with_its_constant() {
    let t0 = 2_000_000_000;
    let environment = json!({
        "sim.seed": 1,
        "sim.channel": { "couplings": [
            { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 },
            { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 1, "gain_db": 0.0 }
        ] }
    });
    let world = World::new(environment);
    let constraints = [("radio.tx.channels", eq(Value::Int(1))), ("radio.rx.channels", eq(Value::Int(2)))];
    let mut radio = prepare(&world, Options { profile: "x310-like", constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
    step(&world, &mut [&mut radio], t0);
    let waveform = world.waveform(&[(0.5, 0.0), (0.5, 0.0)]);
    radio.push(burst(&radio, &world, waveform, 2_002_000, true));
    radio.push(update(&radio, "radio.rx.frequency_hz", Value::Num(1.0e9), t0 + 6_000_000));
    radio.push(update(&radio, "radio.tx.frequency_hz", Value::Num(1.0e9), t0 + 8_000_000));
    radio.push(update(&radio, "radio.rx.frequency_hz", Value::Num(1.0e9), t0 + 9_500_000));
    step(&world, &mut [&mut radio], t0);
    step(&world, &mut [&mut radio], t0 + 10_000_001);
    let Phases { rx, tx, rx_timed, tx_timed } = phases(1, "mock");
    let blocks: Vec<_> = std::iter::from_fn(|| radio.link.receive()).collect();
    let check = |k: i64, channel: usize, expected: f64| {
        let block = blocks.iter().find(|b| b.header().first_sample_time.ticks <= k && k < b.header().first_sample_time.ticks + i64::from(b.header().len)).unwrap();
        let index = (k - block.header().first_sample_time.ticks) as usize;
        let (re, im) = ezsdr_hostmem::read_cf32(block.host_bytes().unwrap(), block.header().len as usize, channel, index);
        let (want_re, want_im) = (0.5 * expected.cos(), 0.5 * expected.sin());
        assert!((f64::from(re) - want_re).abs() < 1e-6 && (f64::from(im) - want_im).abs() < 1e-6, "k {k} channel {channel}: ({re}, {im}) vs ({want_re}, {want_im})");
    };
    check(3_000, 0, tx[0] - rx[0]);
    check(3_000, 1, tx[0] - rx[1]);
    check(7_000, 0, tx[0] - rx_timed[0]);
    check(7_000, 1, tx[0] - rx_timed[1]);
    check(9_000, 0, tx_timed[0] - rx_timed[0]);
    check(9_000, 1, tx_timed[0] - rx_timed[1]);
    check(9_900, 0, tx_timed[0] - rx_timed[0]);
    check(9_900, 1, tx_timed[0] - rx_timed[1]);
    assert!((rx[0] - rx[1]).abs() > 1e-3, "seed 1 gives the two receive channels different phases");
    assert!((rx[0] - rx_timed[0]).abs() > 1e-3, "a timed tune moves the phase to its own constant");
}

#[test]
fn mr_34_the_deterministic_profile_keeps_every_phase_zero() {
    let environment = json!({ "sim.seed": 1, "sim.channel": { "couplings": [
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 1, "gain_db": 0.0 }
    ] } });
    let world = World::new(environment);
    let constraints = [("radio.tx.channels", eq(Value::Int(1))), ("radio.rx.channels", eq(Value::Int(2)))];
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
    let mut radio = prepare(&world, Options { profile: "x310-like", ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, t0))).unwrap();
    step(&world, &mut [&mut radio], t0);
    let samples = ramp(1_000);
    let waveform = world.waveform(&samples);
    radio.push(burst(&radio, &world, waveform, 2_002_000, false));
    radio.push(update(&radio, "radio.tx.gain_db", Value::Num(6.0), t0 + 2_500_000));
    step(&world, &mut [&mut radio], t0);
    step(&world, &mut [&mut radio], t0 + 4_000_001);
    let Phases { rx, tx, .. } = phases(3, "mock");
    let (sin, cos) = (tx[0] - rx[0]).sin_cos();
    for (k, re, im) in radio.received(0) {
        let n = k - 2_045;
        let (want_re, want_im) = if (0..1_000).contains(&n) {
            let (wr, wi) = samples[n as usize];
            // the gain change applies to transmit samples stamped from T0 + 2.5 ms, which reach
            // the antenna 45 samples later: receive samples from 2 545 on, not from 2 500 on
            let gain = if k >= 2_545 { 10f64.powf(6.0 / 20.0) } else { 1.0 };
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
    let mut radio = prepare(&world, Options::default()).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let waveform = world.waveform(&[(2.0, -3.0), (0.05, 0.05), (0.05, 1.5), (0.02, 0.05)]);
    radio.push(burst(&radio, &world, waveform, 0, true));
    radio.push(update(&radio, "radio.rx.gain_db", Value::Num(20.0), 1_000_000));
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 2_000_001);
    let received = radio.received(0);
    assert_eq!(received.len(), 2_000);
    for (k, re, im) in &received {
        let before = *k < 1_000;
        let expected = match (k % 4, before) {
            (0, _) => (1.0, -1.0),
            (1, true) => (0.05, 0.05),
            (1, false) => (0.5, 0.5),
            (2, true) => (0.05, 1.0),
            (2, false) => (0.5, 1.0),
            (_, true) => (0.02, 0.05),
            (_, false) => (0.2, 0.5),
        };
        assert!((re - expected.0).abs() < 1e-6 && (im - expected.1).abs() < 1e-6, "sample {k}: ({re}, {im})");
    }
    let stats = radio.section("stats");
    assert_eq!(stats["tx_clipped"], 2, "two waveform samples have a component beyond full scale, one of them only its imaginary part");
    assert_eq!(stats["rx_clipped"], 500, "from sample 1 000 on, two samples in four have a component beyond full scale, one of them only its imaginary part");
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
        a.push(Action::Stop { target: Some(rid("dev_a/tx")) });
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
    if let Action::TxBurst { late_policy, .. } = &mut late { *late_policy = LatePolicy::DropAndFlag; }
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
fn mr_32_a_cold_transmit_change_ends_the_old_radiation() {
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options::default()).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let before = world.waveform(&[(0.5, 0.0)]);
    radio.push(burst(&radio, &world, before, 0, true));
    step(&world, &mut [&mut radio], 0);
    radio.push(Action::UpdateParameter { target: rid("mock"), key: key("radio.tx.sample_rate_hz"), value: Value::Num(2_000_000.0), class: UpdateClass::Cold, at: None });
    step(&world, &mut [&mut radio], 2_000_500);
    let after = world.waveform(&[(0.0, 0.5)]);
    // RM-25: the new clock starts at 2 001 000, the lattice instant at or after the
    // change, so its sample 2 is the instant 2 002 000 (Phase 7, VE-4).
    step(&world, &mut [&mut radio], 2_001_000);
    radio.push(burst(&radio, &world, after, 2, true));
    step(&world, &mut [&mut radio], 2_001_000);
    step(&world, &mut [&mut radio], 6_000_001);
    for (k, re, im) in radio.received(0) {
        let expected = match k {
            ..=2_000 => (0.5, 0.0),
            2_001 => (0.0, 0.0),
            _ => (0.0, 0.5),
        };
        assert_eq!((re, im), expected, "sample {k}");
    }
}

#[test]
fn mr_35_a_cold_receive_change_samples_the_field_on_the_new_clock() {
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options::default()).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let samples = ramp(10_000);
    let waveform = world.waveform(&samples);
    radio.push(burst(&radio, &world, waveform, 0, false));
    step(&world, &mut [&mut radio], 0);
    radio.push(Action::UpdateParameter { target: rid("mock"), key: key("radio.rx.sample_rate_hz"), value: Value::Num(2_000_000.0), class: UpdateClass::Cold, at: None });
    step(&world, &mut [&mut radio], 1_000_000);
    step(&world, &mut [&mut radio], 4_000_001);
    let first_domain = world.clocks.sample_clock_records().iter().find(|record| record.stream == rid("mock/rx")).unwrap().domain;
    let mut checked = (0, 0);
    while let Some(block) = radio.link.receive() {
        let header = block.header().clone();
        let bytes = block.host_bytes().unwrap();
        for index in 0..header.len as usize {
            let k = header.first_sample_time.ticks + index as i64;
            let (re, im) = ezsdr_hostmem::read_cf32(bytes, header.len as usize, 0, index);
            let j = if header.first_sample_time.domain == first_domain { checked.0 += 1; k } else { checked.1 += 1; 1_000 + k / 2 };
            assert_eq!((re, im), samples[j as usize], "clock {:?} sample {k}", header.first_sample_time.domain);
        }
    }
    assert_eq!(checked, (1_000, 6_000));
}

#[test]
fn mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter() {
    enum End { StopBefore, StopInTheRound, ColdChange }
    for end in [End::StopBefore, End::StopInTheRound, End::ColdChange] {
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
                radio.push(Action::Stop { target: Some(rid("mock/tx")) });
                (3_999_500, "cancelled by stop")
            }
            End::StopInTheRound => {
                run_to(&world, &mut [&mut radio], 3_999_999);
                radio.push(Action::Stop { target: Some(rid("mock/tx")) });
                (4_000_000, "cancelled by stop")
            }
            End::ColdChange => {
                radio.push(Action::UpdateParameter { target: rid("mock"), key: key("radio.tx.sample_rate_hz"), value: Value::Num(2_000_000.0), class: UpdateClass::Cold, at: Some(AbsoluteDeadline::new(TimePoint::new(ROOT, 3_999_500))) });
                run_to(&world, &mut [&mut radio], 3_999_499);
                (3_999_500, "cancelled by a cold change")
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
        assert_eq!((records[0].samples, records[0].end), (4_000, BurstEnd::Eob), "the first burst ended at the held burst's start");
        let rejected = radio.section("rejected");
        assert_eq!(rejected.as_array().unwrap().len(), 1, "the held burst never started");
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
fn ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order() {
    let run = |a_first: bool| {
        let (world, mut a, mut b) = two(a_into_b(), 3_000_000.0, 3_000_000.0);
        let first = world.waveform(&ramp(999));
        let second = world.waveform(&[(0.0, 0.5)]);
        a.push(burst(&a, &world, first, 0, true));
        a.push(burst(&a, &world, second, 1_999, true));
        ordered(&world, &mut a, &mut b, 0, a_first);
        while let Some(due) = world.auth.next_due() {
            if due.ticks >= 666_334 { break; }
            ordered(&world, &mut a, &mut b, due.ticks, a_first);
        }
        a.push(Action::Stop { target: Some(rid("dev_a/tx")) });
        ordered(&world, &mut a, &mut b, 666_334, a_first);
        while let Some(due) = world.auth.next_due() {
            if due.ticks > 2_000_001 { break; }
            ordered(&world, &mut a, &mut b, due.ticks, a_first);
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
    assert_eq!((records_a_first[0].samples, records_a_first[0].end), (1_999, BurstEnd::Eob));
    assert_eq!((records_a_first[1].samples, records_a_first[1].end), (1, BurstEnd::Stop), "the second burst transmitted its one sample before the stop");
}

#[test]
fn mr_32_a_two_channel_waveform_is_channel_interleaved() {
    let world = World::new(json!({ "sim.channel": { "couplings": [
        { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 },
        { "tx": "radio", "tx_channel": 1, "rx": "radio", "rx_channel": 1, "gain_db": 0.0 }
    ] } }));
    let constraints = [("radio.tx.channels", eq(Value::Int(2))), ("radio.rx.channels", eq(Value::Int(2)))];
    let mut radio = prepare(&world, Options { constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    let interleaved: Vec<(f32, f32)> = (0..2_000).flat_map(|n| [((n + 1) as f32 / 4096.0, 0.0), (0.0, (n + 1) as f32 / 4096.0)]).collect();
    let waveform = world.waveform(&interleaved);
    radio.push(Action::TxBurst {
        target: rid("mock/tx"),
        waveform,
        repeat: false,
        at: AbsoluteDeadline::new(TimePoint::new(radio.tx_domain(&world), 0)),
        requested_at: None,
        late_policy: LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    });
    step(&world, &mut [&mut radio], 0);
    step(&world, &mut [&mut radio], 2_000_001);
    let blocks: Vec<_> = std::iter::from_fn(|| radio.link.receive()).collect();
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
    radio.push(Action::Stop { target: Some(rid("mock/tx")) });
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
    let constraints = [("radio.rx.gain_db", eq(Value::Num(6.0)))];
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
fn mr_32_a_stop_after_a_cold_change_keeps_the_old_clock_s_radiation() {
    // a 1 ms path, so that `b` evaluates instants of the old transmit clock after the stop on the new one
    let environment = json!({ "sim.channel": { "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0.0, "delay_ns": 1_000_000 }] } });
    let (world, mut a, mut b) = two(environment, 1_000_000.0, 1_000_000.0);
    let first = world.waveform(&[(0.5, 0.0)]);
    let second = world.waveform(&[(0.0, 0.5)]);
    a.push(burst(&a, &world, first, 0, true));
    a.push(burst(&a, &world, second, 1_500, true));
    step(&world, &mut [&mut a, &mut b], 0);
    a.push(Action::UpdateParameter { target: rid("dev_a"), key: key("radio.tx.sample_rate_hz"), value: Value::Num(2_000_000.0), class: UpdateClass::Cold, at: Some(AbsoluteDeadline::new(TimePoint::new(ROOT, 2_000_500))) });
    run_to(&world, &mut [&mut a, &mut b], 2_000_500);
    let samples = ramp(100);
    let third = world.waveform(&samples);
    // RM-25: the new clock starts at 2 001 000, so its sample 2 is 2 002 000 (VE-4).
    run_to(&world, &mut [&mut a, &mut b], 2_001_000);
    a.push(burst(&a, &world, third, 2, false));
    step(&world, &mut [&mut a, &mut b], 2_001_000);
    run_to(&world, &mut [&mut a, &mut b], 2_002_249);
    a.push(Action::Stop { target: Some(rid("dev_a/tx")) });
    step(&world, &mut [&mut a, &mut b], 2_002_250);
    run_to(&world, &mut [&mut a, &mut b], 6_000_001);
    step(&world, &mut [&mut a, &mut b], 6_000_001);
    let summary: Vec<_> = records(&a).iter().map(|record| (record.samples, record.end)).collect();
    assert_eq!(summary, vec![(1_500, BurstEnd::Eob), (501, BurstEnd::Stop), (1, BurstEnd::Stop)], "the cold change at 2 000.5 us cuts the old clock at sample 2 001; the stop at 2 002.25 us cuts the new one, whose origin is 2 001 us, at sample 3");
    let received = b.received(0);
    assert_eq!(received.len(), 6_000);
    for (k, re, im) in received {
        let want = match k {
            1_000..=2_499 => (0.5, 0.0),
            2_500..=3_000 => (0.0, 0.5),
            3_002 => samples[0],
            _ => (0.0, 0.0),
        };
        assert_eq!((re, im), want, "sample {k}");
    }
}

#[test]
fn mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused() {
    let refused = "MR-16: burst begins at or before the open burst's next sample";
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
        radio.push(Action::Stop { target: Some(rid("mock/tx")) });
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
    radio.push(Action::Stop { target: Some(rid("mock/tx")) });
    step(&world, &mut [&mut radio], 1_500_500);
    run_to(&world, &mut [&mut radio], 3_000_001);
    assert_eq!(radio.section("rejected").as_array().unwrap().len(), 1);
    assert_eq!(radio.section("rejected")[0]["reason"], refused);
    assert_eq!(summary(&radio), vec![(2, BurstEnd::Eob), (499, BurstEnd::Stop)]);
    for (k, re, im) in radio.received(0) {
        let want = match k { 1_000..=1_001 => (0.5, 0.0), 1_002..=1_500 => (0.25, 0.25), _ => (0.0, 0.0) };
        assert_eq!((re, im), want, "sample {k}");
    }

    // a late burst for sample 500, judged where MR-17's SendAsap moves it (no lead on `ideal`):
    // onto the open burst's start in the round at 1 000 000 ns, refused with TIME_ERROR { refused };
    // one sample past it in the round at 1 000 500 ns, admitted, and the open burst ends there
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
    radio.push(Action::Stop { target: Some(rid("mock/tx")) });
    step(&world, &mut [&mut radio], 3_000_500);
    step(&world, &mut [&mut radio], 6_000_001);
    assert_eq!(radio.section("rejected"), json!([]));
    assert_eq!(summary(&radio), vec![(1, BurstEnd::Eob), (2_000, BurstEnd::Stop)]);
    for (k, re, im) in radio.received(0) {
        let want = match k { 1_000 => samples[0], 1_001..=3_000 => (0.0, 0.5), _ => (0.0, 0.0) };
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
        radio.push(Action::Stop { target: Some(rid("mock/tx")) });
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

        // a burst at sample 999, the last of a burst that has not ended: it takes over there, and
        // no two records hold one sample
        let (world, mut radio) = first_pass(environment, medium, &samples, false);
        let second = world.waveform(&[(0.0, 0.5)]);
        radio.push(burst(&radio, &world, second, 999, true));
        step(&world, &mut [&mut radio], 999_000);
        run_to(&world, &mut [&mut radio], 1_500_000);
        radio.push(Action::Stop { target: Some(rid("mock/tx")) });
        step(&world, &mut [&mut radio], 1_500_500);
        run_to(&world, &mut [&mut radio], 3_000_001);
        step(&world, &mut [&mut radio], 3_000_001);
        assert_eq!(radio.section("rejected"), json!([]));
        assert_eq!(summary(&radio), vec![(999, BurstEnd::Eob), (502, BurstEnd::Stop)]);
        if medium {
            for (k, re, im) in radio.received(0) {
                let want = match k { 0..=998 => samples[k as usize], 999..=1_500 => (0.0, 0.5), _ => (0.0, 0.0) };
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
        a.push(burst(&a, &world, first, 0, true));
        a.push(burst(&a, &world, second, 1_500, true));
        a.push(burst(&a, &world, third, 2_500, false));
        a.push(burst(&a, &world, fourth, 3_000, true));
        ordered(&world, &mut a, &mut b, 0, a_first);
        ordered(&world, &mut a, &mut b, 1_000_000, a_first);
        a.push(Action::Stop { target: Some(rid("dev_a/tx")) });
        ordered(&world, &mut a, &mut b, 3_200_500, a_first);
        ordered(&world, &mut a, &mut b, 8_000_001, a_first);
        let summary: Vec<_> = records(&a).iter().map(|record| (record.samples, record.end)).collect();
        (b.received(0), summary, a.section("rejected"))
    };
    let (received, summary, rejected) = run(true);
    assert_eq!((received.clone(), summary.clone(), rejected.clone()), run(false));
    assert_eq!(summary, vec![(1_500, BurstEnd::Eob), (1_000, BurstEnd::Eob), (100, BurstEnd::Eob), (201, BurstEnd::Stop)]);
    assert_eq!(rejected, json!([]));
    let (first, third) = (ramp(1_000), ramp(100));
    for (k, re, im) in received {
        let want = match k {
            0..=1_499 => first[(k % 1_000) as usize],
            1_500..=2_499 => (0.0, 0.5),
            2_500..=2_599 => third[(k - 2_500) as usize],
            3_000..=3_200 => (0.25, 0.25),
            _ => (0.0, 0.0),
        };
        assert_eq!((re, im), want, "sample {k}");
    }
}
