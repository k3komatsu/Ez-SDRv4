//! VH-8's first layer (spec 22): MockRadio against `ezsdr_radio::timeline` on generated
//! sequences. Step 1 gates only the classes where MockRadio already equals the timeline
//! and prints every other divergence, which lists today's issues mechanically; step 2
//! gates every class.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{Action, ActionId, EventCollector};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Endpoint, ExecutionClass, ModuleErrorKind, ModuleRef, Pacing,
    PrepareContext, ProfileRef, Provider, Requested, Role, UpdateClass, Version,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, BlockRef, DataLink, Direction, DropCarry, PublishOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, RelativeBudget, TimePoint};
use ezsdr_mock_radio::MockRadio;
use ezsdr_radio::timeline::{Config, Segment, Stream, plan};
use generator::{Change, Class, Entry, Event, Fault, Op, Sequence, Terms};

#[path = "../../ezsdr-radio/tests/generator/mod.rs"]
mod generator;

const ROOT: ClockDomainId = ClockDomainId::local(7);
const T0: i64 = 2_000_000_000;
/// The Run is stepped to T0 + 1 s; a sequence's rounds, `at`s and faults fall within T0 + 0.4 s.
const HORIZON: i64 = T0 + 1_000_000_000;
const MS: i64 = 1_000_000;

/// Divergences from the timeline still printed, by profile and class; every other field
/// of every class is asserted.
const DIVERGENT: &[(&str, Class, &[&str])] = &[];

fn divergent(profile: &str, class: Class) -> &'static [&'static str] {
    DIVERGENT.iter().find(|(p, c, _)| *p == profile && *c == class).map_or(&[], |(_, _, fields)| fields)
}

/// Fields printed as well for a sequence holding a 3 MS/s transmit change.
const FRACTIONAL_TX: &[&str] = &[];

fn fractional_tx(sequence: &Sequence) -> bool {
    sequence.ops().any(|(_, op)| matches!(op, Op::Cold { direction: Direction::Tx, change: Change::Rate(3_000_000), .. }))
}

/// What a Run leaves, as far as layer 1 sees it: the SampleClocks as `(origin, ended)`, the
/// receive blocks as `(clock, first, len, flags, lost)`, the `applied` rows of `cold` and of
/// `hardware_timed` updates as `(key, value, at)`, how many Actions were refused, the
/// `faults` rows as `(fault, applied, lost)`, and a step that failed other than by the loss.
#[derive(PartialEq, Debug, Default)]
struct Record {
    rx_clocks: Vec<(i64, Option<i64>)>,
    tx_clocks: Vec<(i64, Option<i64>)>,
    blocks: Vec<(usize, i64, u32, BlockFlags, Option<u64>)>,
    cold: Vec<(String, f64, i64)>,
    timed: Vec<(String, f64, i64)>,
    rejected: usize,
    faults: Vec<(String, bool, u64)>,
    error: Option<String>,
}

impl Record {
    /// Prints each differing field, a list from its first difference on.
    fn show(&self, observed: &Record) {
        fn from<T: std::fmt::Debug + PartialEq>(name: &str, a: &[T], b: &[T]) {
            if a != b {
                let first = a.iter().zip(b).position(|(a, b)| a != b).unwrap_or(a.len().min(b.len())).saturating_sub(1);
                eprintln!("{name} from {first} (lengths {}, {}):\n  expected {:?}\n  observed {:?}", a.len(), b.len(), &a[first.min(a.len())..(first + 4).min(a.len())], &b[first.min(b.len())..(first + 4).min(b.len())]);
            }
        }
        from("rx_clocks", &self.rx_clocks, &observed.rx_clocks);
        from("tx_clocks", &self.tx_clocks, &observed.tx_clocks);
        from("blocks", &self.blocks, &observed.blocks);
        from("cold", &self.cold, &observed.cold);
        from("timed", &self.timed, &observed.timed);
        from("faults", &self.faults, &observed.faults);
        eprintln!("rejected {} {}, error {:?} {:?}", self.rejected, observed.rejected, self.error, observed.error);
    }

    fn diverged(&self, other: &Record) -> Vec<&'static str> {
        [
            ("rx_clocks", self.rx_clocks == other.rx_clocks),
            ("tx_clocks", self.tx_clocks == other.tx_clocks),
            ("blocks", self.blocks == other.blocks),
            ("cold", self.cold == other.cold),
            ("timed", self.timed == other.timed),
            ("rejected", self.rejected == other.rejected),
            ("faults", self.faults == other.faults),
            ("error", self.error == other.error),
        ]
        .into_iter()
        .filter(|(_, same)| !same)
        .map(|(field, _)| field)
        .collect()
    }
}

/// A profile's timeline terms; `x310-like` with VH-4's ready term (one receive call of
/// 2 000 samples, 3 ms) and the timed lead of MR-3.
fn terms(profile: &str, direction: Direction) -> Terms {
    let (lead, call, allowance, timed_lead) = if profile == "ideal" { (0, 0, 0, 0) } else { (50 * MS, 2_000, 3 * MS, 2 * MS) };
    // The transmit clock starts at `arm`, at 0 (RM-25); the receive clock at T0.
    let origin = if direction == Direction::Rx { T0 } else { 0 };
    let config = Config { channels: 1, ratio: Rational::new(1_000_000_000, 1_000_000).unwrap() };
    Terms { t0: T0, root_hz: 1_000_000_000, timed_lead, stream: Stream { direction, origin, config, lead, call, allowance } }
}

/// Per field, how many sequences show it and the first seed that does.
type Divergences = BTreeMap<&'static str, (usize, u64)>;

#[test]
fn rm_26_the_timeline_against_mockradio() {
    // `EZSDR_TIMELINE_SEED=<seed>` prints that sequence's two records side by side.
    let shown: Option<u64> = std::env::var("EZSDR_TIMELINE_SEED").ok().and_then(|seed| seed.parse().ok());
    let mut summary: BTreeMap<(&str, Class), (usize, Divergences)> = BTreeMap::new();
    let mut failed = Vec::new();
    for seed in 0..1_000 {
        // 3 MS/s, 1 000/3 ticks a sample, is an `ideal` rate only.
        let (profile, rates) = if seed % 2 == 0 { ("ideal", &generator::FRACTIONAL_RATES[..]) } else { ("x310-like", &generator::RATES[..]) };
        let sequence = generator::sequence(seed, rates);
        let expected = expect(profile, &sequence);
        let observed = run(profile, &sequence);
        let diverged = observed.diverged(&expected);
        if shown == Some(seed) {
            eprintln!("seed {seed}, {profile}: {sequence:?}");
            expected.show(&observed);
        }
        let printed = |field: &&str| divergent(profile, sequence.class).contains(field)
            || (fractional_tx(&sequence) && FRACTIONAL_TX.contains(field));
        let gated: Vec<_> = diverged.iter().filter(|field| !printed(field)).collect();
        if !gated.is_empty() {
            failed.push(format!("seed {seed}, {profile}, {:?}: {gated:?}", sequence.class));
        }
        let (count, fields) = summary.entry((profile, sequence.class)).or_default();
        *count += 1;
        for field in diverged {
            fields.entry(field).or_insert((0, seed)).0 += 1;
        }
    }
    // VH-8: today's divergences per class, each field with how many sequences show it and
    // the first seed that does; a listed field no sequence shows any more is named.
    for ((profile, class), (count, fields)) in &summary {
        let equal: Vec<_> = divergent(profile, *class).iter().filter(|field| !fields.contains_key(*field)).collect();
        eprintln!("layer 1, {profile}, {class:?}: {count} sequences; printed {fields:?}; listed but equal {equal:?}");
    }
    assert!(failed.is_empty(), "{} sequences differ (EZSDR_TIMELINE_SEED=<seed> shows one), the first: {:#?}", failed.len(), &failed[..failed.len().min(20)]);
}

/// The timeline's account of a sequence on `profile`, with MR-18's `applied` rows, MR-20's
/// refusals after a loss, MR-21's and MR-22's blocks and VH-5's fault rows.
fn expect(profile: &str, sequence: &Sequence) -> Record {
    let (rx, tx) = (terms(profile, Direction::Rx), terms(profile, Direction::Tx));
    let schedule = sequence.schedule(&rx);
    let lost = schedule.iter().position(|entry| matches!(entry.event, Event::Fault(i) if sequence.faults[i].1 == Fault::Loss));
    let rx_items = sequence.items(&rx);
    let segments = plan(&rx.stream, &rx_items).unwrap();
    let mut record = Record {
        tx_clocks: clocks(&plan(&tx.stream, &sequence.items(&tx)).unwrap()),
        ..Record::default()
    };
    let ops: Vec<&Op> = sequence.ops().map(|(_, op)| op).collect();
    for (index, entry) in schedule.iter().enumerate() {
        let dropped = lost.is_some_and(|lost| index > lost);
        match entry.event {
            Event::Op(_) if dropped => record.rejected += 1,
            Event::Op(op) => match *ops[op] {
                Op::Cold { direction, change, .. } => {
                    let (name, value) = match change {
                        Change::Channels(n) => ("channels", f64::from(n)),
                        Change::Rate(rate) => ("sample_rate_hz", rate as f64),
                    };
                    record.cold.push((format!("radio.{}.{name}", side(direction)), value, entry.e));
                }
                Op::Timed { direction, gain_db, .. } => record.timed.push((format!("radio.{}.gain_db", side(direction)), gain_db, entry.e)),
                Op::Stop { .. } | Op::StartRx => {}
            },
            Event::Fault(_) => {}
        }
    }
    // Each fault in the order it fires, on the segment running then (RM-21: the plan of
    // what came before it has a segment open, begun by its instant).
    let gap = if profile == "ideal" { 0 } else { 50 * MS };
    let mut hits: Vec<Vec<(i64, u64, Fault)>> = vec![Vec::new(); segments.len()];
    let mut rows = Vec::new();
    for (index, entry) in schedule.iter().enumerate() {
        let Event::Fault(fault) = entry.event else { continue };
        let kind = sequence.faults[fault].1;
        let before: Vec<_> = rx_items.iter().filter(|item| (item.e, item.seq) < (entry.e, entry.seq)).copied().collect();
        let open = plan(&rx.stream, &before).unwrap().last().copied().filter(|s| s.cut.is_none() && s.origin <= entry.e);
        // A loss stops the device whether or not the receive stream runs (MR-20).
        let fired = !lost.is_some_and(|lost| index > lost);
        let running = fired && (kind == Fault::Loss || open.is_some());
        if let (true, Some(open)) = (fired, open) {
            hits[segments.iter().position(|s| s.origin == open.origin).expect("the segment")].push((entry.e, entry.seq, kind));
        }
        rows.push((entry, kind, running));
    }
    // RM-25: a receive segment's SampleClock is registered at its first block, so one that
    // publishes none has none.
    let mut injected = BTreeMap::new();
    for (index, segment) in segments.iter().enumerate() {
        let mut blocks = Vec::new();
        receive(record.rx_clocks.len(), segment, &hits[index], gap, &mut blocks, &mut injected);
        if !blocks.is_empty() {
            record.rx_clocks.extend(clocks(std::slice::from_ref(segment)));
            record.blocks.extend(blocks);
        }
    }
    record.faults = rows.into_iter().map(|(entry, kind, running): (&Entry, Fault, bool)| {
        let lost = if running { injected.get(&entry.seq).copied().unwrap_or(0) } else { 0 };
        (name(kind).to_owned(), running, lost)
    }).collect();
    record
}

/// MR-12's blocks of one receive segment up to the horizon, cut by its faults as MR-20,
/// MR-21 and MR-22 say; records each fault's `k_g − k_f` by its arrival order.
fn receive(index: usize, segment: &Segment, faults: &[(i64, u64, Fault)], gap: i64, blocks: &mut Vec<(usize, i64, u32, BlockFlags, Option<u64>)>, injected: &mut BTreeMap<u64, u64>) {
    const BLOCK: i64 = 2_000;
    let k = |t: i64| segment.sample_at_or_after(t).unwrap();
    let end = segment.cut.unwrap_or(i64::MAX);
    // MR-14: a block is published once its last sample's exact instant is in the past.
    let (num, den) = (segment.config.ratio.num() as i64, segment.config.ratio.den() as i64);
    let published = |stop: i64| segment.origin + (stop - 1) * num / den < HORIZON;
    let mut faults = faults.iter().peekable();
    let (mut next, mut flags, mut lost) = (0, BlockFlags::NONE, None::<u64>);
    loop {
        let fault_k = faults.peek().map(|(f, _, _)| k(*f));
        let stop = (next + BLOCK).min(end).min(fault_k.unwrap_or(i64::MAX));
        if stop > next {
            if !published(stop) {
                return;
            }
            blocks.push((index, next, (stop - next) as u32, flags, lost));
            (next, flags, lost) = (stop, BlockFlags::NONE, None);
            continue;
        }
        let Some((f, seq, fault)) = faults.next() else { return };
        let kf = k(*f).max(next);
        let kg = match fault {
            Fault::Overrun => k(f + gap).max(kf),
            Fault::Sequence => kf + BLOCK,
            Fault::Loss => return,
        };
        injected.insert(*seq, (kg - kf) as u64);
        if kg > kf {
            flags = flags | BlockFlags::GAP_BEFORE | if *fault == Fault::Overrun { BlockFlags::RESTARTED } else { BlockFlags::SEQ_DISCONTINUITY };
            lost = Some(lost.unwrap_or(0) + (kg - kf) as u64);
        }
        next = kg.min(end);
    }
}

/// Each segment's SampleClock as `(origin, ended)`: it ends at its cut's instant.
fn clocks(segments: &[Segment]) -> Vec<(i64, Option<i64>)> {
    segments.iter().map(|s| (s.origin, s.cut.map(|k| s.instant(k).unwrap()))).collect()
}

fn side(direction: Direction) -> &'static str {
    if direction == Direction::Rx { "rx" } else { "tx" }
}

fn name(fault: Fault) -> &'static str {
    match fault {
        Fault::Overrun => "rx_overflow",
        Fault::Sequence => "rx_sequence_error",
        Fault::Loss => "device_lost",
    }
}

// ---------------------------------------------------------------- the Run

#[derive(Default)]
struct Queue(Mutex<VecDeque<Action>>);

impl ActionReceiver for Queue {
    fn recv(&self) -> Option<Action> {
        self.0.lock().unwrap().pop_front()
    }
}

struct NoSubmitter;
impl ActionSubmitter for NoSubmitter {
    fn submit(&self, _action: Action) -> Result<ActionId, Vec<Violation>> {
        Ok(ActionId(1))
    }
}

/// Keeps every block's header and nothing else.
#[derive(Default)]
struct Headers(Mutex<Vec<BlockHeader>>);

impl DataLink for Headers {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        self.0.lock().unwrap().push(block.header().clone());
        PublishOutcome::Accepted
    }
    fn receive(&self) -> Option<(BlockRef, DropCarry)> { None }
    fn drops(&self) -> u64 { 0 }
    fn take_drop_carry(&self) -> DropCarry { DropCarry::default() }
    fn policy(&self) -> BackPressure { BackPressure::DropOldest }
}

fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}

fn rid(name: &str) -> ResourceId {
    ResourceId::parse(name).unwrap()
}

/// Runs a sequence on MockRadio 1.5.0: each round's Actions are stepped at its instant,
/// then the Run is stepped to the horizon.
fn run(profile: &str, sequence: &Sequence) -> Record {
    let module = ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(2, 0, 0) };
    let binding = Binding {
        module: module.clone(),
        selector: BTreeMap::from([(Ident::parse("id").unwrap(), Value::Str("mock".to_owned()))]),
        profile: Some(ProfileRef { name: profile.to_owned(), version: Version::new(2, 0, 0) }),
        feed: None,
    };
    let mut mock = MockRadio::from_binding(&binding).unwrap();
    let clocks = Arc::new(ClockRegistry::new());
    clocks.register(ClockDomain::root(ROOT, Rational::new(1_000_000_000, 1).unwrap(), EpochRef::Arbitrary { set_by: "test".to_owned() })).unwrap();
    let auth = Arc::new(ManualTimeAuthority::new(clocks.clone(), ROOT, &[], Pacing::FreeRunning).unwrap());
    let mut kinds = EventKindRegistry::new();
    let mut registry = ezsdr_kernel::module_api::ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
    ezsdr_sim::register(&mut registry, &mut checks, &mut kinds).unwrap();
    let all = kinds.kinds();
    let pairs: Vec<_> = ["mock", "mock/rx", "mock/tx"].iter().flat_map(|s| all.iter().map(move |k| (rid(s), k.clone()))).collect();
    let events = Arc::new(EventCollector::new(&pairs, &all, 4096, &Policy::default()));
    let actions = Arc::new(Queue::default());
    let link = Arc::new(Headers::default());
    let faults: Vec<_> = sequence.faults.iter().map(|(at, fault)| serde_json::json!({ "at_ns": at, "fault": name(*fault), "target": "radio" })).collect();
    let constraints = [("radio.rx.channels", Value::Int(1)), ("radio.tx.channels", Value::Int(1))]
        .map(|(name, value)| (key(name), Constraint::Eq { value }));
    let ctx = PrepareContext {
        run: RunId::from_string("timeline".to_owned()),
        class: ExecutionClass::Simulation,
        time: auth.clone(),
        clocks: clocks.clone(),
        events,
        actions: actions.clone(),
        actions_out: Arc::new(NoSubmitter),
        environment: Arc::new(BTreeMap::from([(Namespace::parse("sim.faults").unwrap(), serde_json::json!(faults))])),
        inputs: Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new()),
        links: vec![AttachedPort { component: Ident::parse("radio").unwrap(), port: Ident::parse("rx").unwrap(), endpoint: Endpoint::StreamOut(link.clone()) }],
        components: BTreeMap::new(),
        host_budget: RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap(),
    };
    let fragment = Fragment {
        id: Ident::parse("radio").unwrap(),
        instance: module,
        role: Role::Provider,
        content: serde_json::json!({ "selector": {}, "requested": Requested { resource: rid("mock"), constraints: constraints.into_iter().collect() } }),
        after: Vec::new(),
    };
    mock.prepare(&fragment, ctx).unwrap();
    mock.arm().unwrap();
    mock.start(Some(TimePoint::new(ROOT, T0))).unwrap();
    let mut error = None;
    let mut step = |mock: &mut MockRadio, tick: i64| {
        auth.advance_to(TimePoint::new(ROOT, tick)).unwrap();
        match mock.step(TimePoint::new(ROOT, tick)) {
            Err(e) if e.kind != ModuleErrorKind::DeviceLost => error = error.take().or(Some(e.message)),
            _ => {}
        }
    };
    let at = |ns: Option<i64>| ns.map(|ns| AbsoluteDeadline::new(TimePoint::new(ROOT, T0 + ns)));
    for (round, ops) in &sequence.rounds {
        for op in ops {
            let mut queue = actions.0.lock().unwrap();
            queue.push_back(match *op {
                Op::Stop { device } => Action::Stop { target: Some(rid(if device { "mock" } else { "mock/rx" })) },
                Op::StartRx => Action::Command { target: rid("mock/rx"), verb: Ident::parse("start_rx").unwrap(), params: BTreeMap::new(), at: None },
                Op::Cold { direction, change, at_ns } => {
                    let (name, value) = match change {
                        Change::Channels(n) => ("channels", Value::Int(i64::from(n))),
                        Change::Rate(rate) => ("sample_rate_hz", Value::Num(rate as f64)),
                    };
                    Action::UpdateParameter { target: rid("mock"), key: key(&format!("radio.{}.{name}", side(direction))), value, class: UpdateClass::Cold, at: at(at_ns) }
                }
                Op::Timed { direction, gain_db, at_ns } => Action::UpdateParameter {
                    target: rid("mock"), key: key(&format!("radio.{}.gain_db", side(direction))), value: Value::Num(gain_db), class: UpdateClass::HardwareTimed, at: at(at_ns),
                },
            });
        }
        step(&mut mock, T0 + round);
    }
    step(&mut mock, HORIZON);
    let records = clocks.sample_clock_records();
    let clocks_of = |stream: &str| records.iter().filter(|r| r.stream == rid(stream)).map(|r| (r.origin.ticks_in(ROOT).unwrap(), r.ended_at.map(|t| t.ticks_in(ROOT).unwrap()))).collect();
    let domains: Vec<_> = records.iter().filter(|r| r.stream == rid("mock/rx")).map(|r| r.domain).collect();
    let section = |name: &str| mock.instance().sections[&Namespace::parse(&format!("ezsdr.radio.mock.mock.{name}")).unwrap()].clone();
    let rows = |name: &str| section(name).as_array().unwrap().clone();
    let applied: Vec<_> = rows("applied").iter().map(|r| (r["key"].as_str().unwrap().to_owned(), r["value"].as_f64().unwrap(), r["at"]["ticks"].as_i64().unwrap())).collect();
    Record {
        rx_clocks: clocks_of("mock/rx"),
        tx_clocks: clocks_of("mock/tx"),
        blocks: link.0.lock().unwrap().iter().map(|h| {
            let clock = domains.iter().position(|d| *d == h.first_sample_time.domain()).unwrap();
            (clock, h.first_sample_time.ticks_in(h.first_sample_time.domain()).unwrap(), h.len, h.flags, h.lost)
        }).collect(),
        cold: applied.iter().filter(|r| !r.0.ends_with("gain_db")).cloned().collect(),
        timed: applied.into_iter().filter(|r| r.0.ends_with("gain_db")).collect(),
        rejected: rows("rejected").len(),
        faults: rows("faults").iter().map(|r| (r["fault"].as_str().unwrap().to_owned(), r["applied"].as_bool().unwrap(), r["lost"].as_u64().unwrap())).collect(),
        error,
    }
}
