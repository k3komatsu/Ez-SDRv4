//! VH-8's second layer (spec 22): uhd-control's booking against `ezsdr_radio::timeline` on
//! generated sequences, on the `ManualTimeAuthority` rig. uhd-rx and uhd-tx do not run: an
//! idealized owner carries out each plan at its instants, and layer 3 runs the real ones.
//! The generator's `Stop`, `start_rx` and loss are booked; a Run's settings change only at its
//! start (note 47).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::channel;

use super::super::test_support::target_of;
use ezsdr_kernel::event::Action;
use ezsdr_kernel::module_api::ActionReceiver;
use ezsdr_kernel::spec::{Ident, Value};
use ezsdr_kernel::stream::{BackPressure, BlockRef, DataLink, Direction, DropCarry, PublishOutcome};
use ezsdr_kernel::time::Rational;
use ezsdr_radio::timeline::{Channels, Config, Stream, plan};
use generator::{Class, Fault, Leaf, Op, Sequence, Terms};

use ezsdr_kernel::id::ClockDomainId;

use super::super::core::{booked_stream, key, lock};
use super::{ColdConfig, Control};
use crate::device::{Device, Dir, FakeConfig};
use crate::profile::DELIVERY_ALLOWANCE_NS;

use super::super::generator;

/// T0, 2 s on the 200 MHz root.
const T0: i64 = 400_000_000;
/// The rig runs to T0 + 1 s; a sequence's rounds, `at`s and faults fall within T0 + 0.4 s.
const HORIZON: i64 = T0 + 200_000_000;
/// Root ticks per millisecond.
const MS: i64 = 200_000;

/// What the booking leaves, as far as layer 2 sees it: the SampleClocks as `(origin,
/// ended)`, how many Actions were refused, and the `(frequency, gain)` of channel 0 each receive segment's start took (RM-21).
#[derive(PartialEq, Debug, Default)]
struct Record {
    rx_clocks: Vec<(i64, Option<i64>)>,
    tx_clocks: Vec<(i64, Option<i64>)>,
    rejected: usize,
    starts: Vec<(i64, (f64, f64))>,
}

impl Record {
    fn diverged(&self, other: &Record) -> Vec<&'static str> {
        [("rx_clocks", self.rx_clocks == other.rx_clocks), ("tx_clocks", self.tx_clocks == other.tx_clocks), ("rejected", self.rejected == other.rejected), ("starts", self.starts == other.starts)]
            .into_iter()
            .filter(|(_, same)| !same)
            .map(|(field, _)| field)
            .collect()
    }
}

/// This Module's timeline terms on `x3xx` (UR-25, VH-4): the start lead, one receive
/// call of `block_len` samples (the FakeDevice's packet is the whole request) and the delivery
/// allowance.
fn terms(direction: Direction) -> Terms {
    let ticks = |ns: i64| ns / 5;
    let origin = if direction == Direction::Rx { T0 } else { 0 };
    let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
    let release_window = ticks(super::super::test_support::default_description(2_000).timing.release_window_ns);
    let stream = Stream { direction, origin, config, lead: 50 * MS, call: 2_000, release_window: if direction == Direction::Rx { release_window } else { 0 }, allowance: ticks(DELIVERY_ALLOWANCE_NS) };
    Terms { t0: T0, root_hz: 200_000_000, stream }
}

/// Per field, how many sequences show it and the first seed that does.
type Divergences = BTreeMap<&'static str, (usize, u64)>;

#[test]
fn rm_26_the_timeline_against_uhd_control() {
    let shown: Option<u64> = std::env::var("EZSDR_TIMELINE_SEED").ok().and_then(|seed| seed.parse().ok());
    let mut summary: BTreeMap<Class, (usize, Divergences)> = BTreeMap::new();
    let mut failed = Vec::new();
    for seed in 0..1_000 {
        let sequence = generator::sequence(seed);
        let expected = expect(&sequence);
        let observed = run(&sequence);
        let diverged = observed.diverged(&expected);
        if shown == Some(seed) {
            eprintln!("seed {seed}: {sequence:?}\nexpected {expected:?}\nobserved {observed:?}");
        }
        if !diverged.is_empty() {
            failed.push(format!("seed {seed}, {:?}: {diverged:?}", sequence.class));
        }
        let (count, fields) = summary.entry(sequence.class).or_default();
        *count += 1;
        for field in diverged {
            fields.entry(field).or_insert((0, seed)).0 += 1;
        }
    }
    for (class, (count, fields)) in &summary {
        eprintln!("layer 2, {class:?}: {count} sequences; diverged {fields:?}");
    }
    assert!(failed.is_empty(), "{} sequences differ (EZSDR_TIMELINE_SEED=<seed> shows one), the first: {:#?}", failed.len(), &failed[..failed.len().min(20)]);
}

/// The instant of the sequence's loss, if it has one.
fn loss_of(sequence: &Sequence) -> Option<i64> {
    sequence.faults.iter().filter(|(_, fault)| *fault == Fault::Loss).map(|(ns, _)| T0 + ns / 5).min()
}

/// The timeline's account of a sequence; after a loss nothing more is booked, and a `Set` or a `start_rx` while streaming is refused (the generator's
/// oracle, RM-21).
fn expect(sequence: &Sequence) -> Record {
    let rx = terms(Direction::Rx);
    let clocks = |terms: &Terms| plan(&terms.stream, &sequence.items(terms)).unwrap().iter().map(|s| (s.origin, s.cut.map(|k| s.instant(k).unwrap()))).collect();
    let loss = loss_of(sequence);
    let receipts = sequence.schedule(&rx);
    let booked = |index: usize| receipts.iter().find(|entry| entry.event == generator::Event::Op(index)).is_some_and(|entry| loss.is_none_or(|loss| entry.receipt < loss));
    let staged = sequence.staged(&rx, booked);
    let snapshots: BTreeMap<u64, generator::Snapshot> = staged.starts.iter().map(|(index, snapshot)| ((sequence.faults.len() + index) as u64, *snapshot)).collect();
    let initial = initial_values();
    let starts = plan(&rx.stream, &sequence.items(&rx))
        .unwrap()
        .iter()
        .map(|segment| {
            let snapshot = segment.by.and_then(|by| snapshots.get(&by).copied()).unwrap_or((None, None));
            (segment.origin, (snapshot.0.map_or(initial.0, |v| Leaf::Frequency.value(v)), snapshot.1.map_or(initial.1, |v| Leaf::Gain.value(v))))
        })
        .collect();
    Record { rx_clocks: clocks(&rx), tx_clocks: clocks(&terms(Direction::Tx)), rejected: staged.refused.len() + staged.refused_starts.len(), starts }
}

/// The Run's own `(frequency, gain)` of receive channel 0.
fn initial_values() -> (f64, f64) {
    let defaults = super::super::test_support::default_description(2_000).defaults();
    let number = |name: &str| match defaults[&key(name)] {
        Value::Scalar(ezsdr_kernel::spec::Scalar::Num(value)) => value.get(),
        ref other => panic!("{other:?}"),
    };
    (number("radio.rx.c0.frequency_hz"), number("radio.rx.c0.gain_db"))
}

struct NoActions;
impl ActionReceiver for NoActions {
    fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
        None
    }
}

/// A receive link that takes nothing: uhd-rx does not run.
struct NoLink;
impl DataLink for NoLink {
    fn publish(&self, _block: BlockRef) -> PublishOutcome {
        PublishOutcome::Accepted
    }
    fn receive(&self) -> Option<(BlockRef, DropCarry)> {
        None
    }
    fn drops(&self) -> u64 {
        0
    }
    fn take_drop_carry(&self) -> DropCarry {
        DropCarry::default()
    }
    fn policy(&self) -> BackPressure {
        BackPressure::DropOldest
    }
}

/// Books a sequence's `Stop`s and `start_rx`s on uhd-control as its loop does (MA-14b): each
/// round's Actions at its instant; a loss marks the device lost at its instant, before a round at that instant,
/// and nothing is booked after. The owners are idealized: uhd-rx registers a receive clock
/// once its origin has passed and ends it at its cut once that has passed too, as UR-17 says;
/// layer 3 runs the real ones.
fn run(sequence: &Sequence) -> Record {
    let (core, device, time, _events) = super::super::test_support::rig_with(FakeConfig::default(), vec![Arc::new(NoLink)]);
    let tx = core.register(Dir::Tx, 200, 0).unwrap();
    device.rx_open(0, &[0, 1]).unwrap();
    device.tx_open(0, &[0, 1]).unwrap();
    let (to_tx, from_tx) = channel();
    let mut config = core.description.defaults();
    config.insert(key("radio.rx.c0.stream"), Value::from(0));
    config.insert(key("radio.tx.c0.stream"), Value::from(0));
    let start = super::Start { t0: T0, rx: vec![Some((ezsdr_radio::timeline::Channels::first(1), 200))], tx: vec![Some((tx, ezsdr_radio::timeline::Channels::first(1)))] };
    let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start);
    let ticks = |ns: i64| ns / 5;
    let loss = loss_of(sequence);
    let mut rounds = sequence.rounds.iter().peekable();
    // The idealized uhd-rx: each planned segment's clock, once registered, and whether ended.
    let mut receive: Vec<(i64, ezsdr_radio::timeline::Config, ClockDomainId, bool)> = Vec::new();
    let mut now = T0;
    loop {
        time.advance_to(core.at(now)).unwrap();
        if loss.is_some_and(|loss| loss <= now) {
            core.device_lost("VH-8: the sequence's loss");
        }
        while let Some((round, ops)) = rounds.next_if(|(round, _)| T0 + ticks(*round) <= now) {
            for op in ops {
                if !core.is_lost() {
                    // The node each Action is resolved to (KC-23); its target only names it.
                    let (action, node) = match *op {
                        Op::Stop { device } => {
                            let node = if device { core.id.clone() } else { core.rx_id.clone() };
                            (Action::Stop { target: Some(target_of(&node)) }, node)
                        }
                        Op::StartRx { at } => {
                            let at = at.map(|ns| ezsdr_kernel::time::AbsoluteDeadline::new(core.at(T0 + ticks(*round) + ticks(ns))));
                            (Action::Command { target: target_of(&core.rx_id), verb: Ident::parse("start_rx").unwrap(), params: BTreeMap::new(), at }, core.rx_id.clone())
                        }
                        Op::Set { leaf, v } => {
                            let name = match leaf {
                                Leaf::Frequency => "radio.rx.c0.frequency_hz",
                                Leaf::Gain => "radio.rx.c0.gain_db",
                            };
                            let action = Action::UpdateParameter {
                                target: target_of(&core.id),
                                key: key(name),
                                value: Value::num(leaf.value(v)).unwrap(),
                                class: ezsdr_kernel::module_api::UpdateClass::Cold,
                                at: None,
                            };
                            (action, core.id.clone())
                        }
                    };
                    control.book(ezsdr_kernel::module_api::Dispatched::new(ezsdr_kernel::event::ActionId(1), action, Some(node)));
                }
            }
        }
        while from_tx.try_recv().is_ok() {}
        let plan = lock(&core.streams).line(booked_stream(Dir::Rx)).map(|line| line.plan.clone()).unwrap_or_default();
        for segment in plan.into_iter().filter(|segment| segment.origin < now) {
            let index = match receive.iter().position(|(origin, config, _, _)| *origin == segment.origin && *config == segment.config) {
                Some(index) => index,
                None => {
                    let clock = core.register(Dir::Rx, segment.config.ratio.num() as i64, segment.origin).unwrap();
                    receive.push((segment.origin, segment.config, clock.domain, false));
                    receive.len() - 1
                }
            };
            let (_, _, domain, ended) = &mut receive[index];
            if let Some(cut) = segment.cut.map(|cut| segment.instant(cut).unwrap()).filter(|cut| *cut <= now && !*ended) {
                core.clocks.end(*domain, core.at(cut)).unwrap();
                *ended = true;
            }
        }
        if now >= HORIZON {
            break;
        }
        let next_round = rounds.peek().map(|(round, _)| T0 + ticks(*round));
        now = [Some(now + MS), next_round, loss.filter(|loss| *loss > now)].into_iter().flatten().min().unwrap();
    }
    // The end at the loss's instant records the plan.
    if core.is_lost() {
        control.finish();
    }
    let records = core.clocks.sample_clock_records();
    let clocks_of = |stream| records.iter().filter(|r| r.stream == stream).map(|r| (r.origin.ticks_in(r.root).unwrap(), r.ended_at.map(|t| t.ticks_in(r.root).unwrap()))).collect();
    let rejected = lock(&core.rec).rejected.len();
    // Each receive segment's start took a copy of the staged values at its booking (RM-21): `configs`, by the command's arrival order.
    let streams = lock(&core.streams);
    let initial = initial_values();
    let held = |config: &ColdConfig, name: &str, default: f64| match config.base.get(&key(name)) {
        Some(Value::Scalar(ezsdr_kernel::spec::Scalar::Num(value))) => value.get(),
        _ => default,
    };
    let starts = streams
        .line(booked_stream(Dir::Rx))
        .map(|line| {
            line.plan
                .iter()
                .map(|segment| match segment.by.and_then(|by| streams.stream(booked_stream(Dir::Rx)).configs.get(&by)) {
                    Some(config) => (segment.origin, (held(config, "radio.rx.c0.frequency_hz", initial.0), held(config, "radio.rx.c0.gain_db", initial.1))),
                    None => (segment.origin, initial),
                })
                .collect()
        })
        .unwrap_or_default();
    drop(streams);
    Record { rx_clocks: clocks_of(core.rx_id.clone()), tx_clocks: clocks_of(core.tx_id.clone()), rejected, starts }
}
