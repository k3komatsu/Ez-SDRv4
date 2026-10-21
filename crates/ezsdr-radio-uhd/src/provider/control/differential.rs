//! VH-8's second layer (spec 22): uhd-control's booking against `ezsdr_radio::timeline` on
//! generated sequences, on the `ManualTimeAuthority` rig. uhd-rx and uhd-tx do not run: an
//! idealized owner carries out each plan at its instants, and layer 3 runs the real ones.
//! Every class is gated.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::channel;

use ezsdr_kernel::event::Action;
use ezsdr_kernel::module_api::{ActionReceiver, UpdateClass};
use ezsdr_kernel::spec::{Ident, Value};
use ezsdr_kernel::stream::{BackPressure, BlockRef, DataLink, Direction, DropCarry, PublishOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, Rational};
use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Stream, NO_RELEASE, plan};
use super::super::test_support::target_of;
use generator::{Change, Class, Entry, Event, Fault, Op, Sequence, Terms};

use ezsdr_kernel::id::ClockDomainId;

use super::Control;
use super::super::core::{booked_stream, key, lock};
use crate::device::{Device, Dir, FakeConfig};
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS, RELEASE_WINDOW_NS};

use super::super::generator;

/// T0, 2 s on the 200 MHz root.
const T0: i64 = 400_000_000;
/// The rig runs to T0 + 1 s; a sequence's rounds, `at`s and faults fall within T0 + 0.4 s.
const HORIZON: i64 = T0 + 200_000_000;
/// Root ticks per millisecond.
const MS: i64 = 200_000;

/// Divergences from the timeline still printed, by class; every other field of every
/// class is asserted.
const DIVERGENT: &[(Class, &[&str])] = &[];

fn divergent(class: Class) -> &'static [&'static str] {
    DIVERGENT.iter().find(|(c, _)| *c == class).map_or(&[], |(_, fields)| fields)
}

/// What the booking leaves, as far as layer 2 sees it: the SampleClocks as `(origin,
/// ended)`, the issued `hardware_timed` updates as `(key, value, at)` in the order of their
/// instants (uhd-control records one when it issues it, or books it for a stream with no
/// channel), and how many Actions were refused.
#[derive(PartialEq, Debug, Default)]
struct Record {
    rx_clocks: Vec<(i64, Option<i64>)>,
    tx_clocks: Vec<(i64, Option<i64>)>,
    timed: Vec<(String, f64, i64)>,
    rejected: usize,
}

impl Record {
    fn diverged(&self, other: &Record) -> Vec<&'static str> {
        [
            ("rx_clocks", self.rx_clocks == other.rx_clocks),
            ("tx_clocks", self.tx_clocks == other.tx_clocks),
            ("timed", self.timed == other.timed),
            ("rejected", self.rejected == other.rejected),
        ]
        .into_iter()
        .filter(|(_, same)| !same)
        .map(|(field, _)| field)
        .collect()
    }
}

/// This Module's timeline terms on `x3xx` (UR-25, VH-4): the start lead, one receive
/// call of `block_len` samples (the FakeDevice's packet is the whole request), the delivery
/// allowance, and the device lead of a `hardware_timed` update.
fn terms(direction: Direction) -> Terms {
    let ticks = |ns: i64| ns / 5;
    let origin = if direction == Direction::Rx { T0 } else { 0 };
    let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
    let stream = Stream { direction, origin, config, lead: 50 * MS, call: 2_000, allowance: ticks(DELIVERY_ALLOWANCE_NS) };
    Terms { t0: T0, root_hz: 200_000_000, timed_lead: ticks(DEVICE_LEAD_NS), stream }
}

/// The generated operations' effective instants with the layout's instance-wide cold floor.
fn layout_schedule(sequence: &Sequence, terms: &Terms) -> Vec<Entry> {
    let mut entries: Vec<Entry> = sequence.faults.iter().enumerate().map(|(index, (at, _))| {
        let e = terms.t0 + terms.ticks(*at);
        Entry { e, seq: index as u64, receipt: e, event: Event::Fault(index) }
    }).collect();
    let mut cold = Vec::new();
    for (index, (round, op)) in sequence.ops().enumerate() {
        let receipt = terms.t0 + terms.ticks(round);
        let at_ns = match *op {
            Op::Cold { at_ns, .. } | Op::Timed { at_ns, .. } => at_ns,
            _ => None,
        };
        let requested = at_ns.map(|ns| terms.t0 + terms.ticks(ns));
        let e = match *op {
            Op::Stop { .. } | Op::StartRx => receipt,
            Op::Cold { direction, .. } => {
                let start = (direction == Direction::Rx).then_some(terms.t0);
                let (e, _) = ezsdr_radio::timeline::layout_instant(requested, receipt, start, cold.iter().copied());
                cold.push(e);
                e
            }
            Op::Timed { .. } => requested.unwrap_or(i64::MIN).max(receipt + terms.timed_lead),
        };
        entries.push(Entry { e, seq: (sequence.faults.len() + index) as u64, receipt, event: Event::Op(index) });
    }
    entries.sort_by_key(|entry| (entry.e, entry.seq));
    entries
}

/// A generated channel count is expanded as c0/c1 membership changes at the same instant.
fn layout_items(sequence: &Sequence, terms: &Terms) -> Vec<Item> {
    let receive = terms.stream.direction == Direction::Rx;
    let ops: Vec<&Op> = sequence.ops().map(|(_, op)| op).collect();
    let mut config = terms.stream.config;
    let mut items = Vec::new();
    for entry in layout_schedule(sequence, terms) {
        if let Event::Op(index) = entry.event {
            if let Op::Cold { direction, change: Change::Channels(count), .. } = *ops[index] {
                if direction != terms.stream.direction { continue; }
                let desired = Channels::first(u32::from(count));
                let mut sub = 0;
                for channel in 0..2 {
                    let wanted = desired.contains(channel);
                    if config.channels.contains(channel) == wanted { continue; }
                    config.channels = if wanted { config.channels.with(channel) } else { config.channels.without(channel) };
                    items.push(Item { e: entry.e, seq: entry.seq * 2 + sub, ready: entry.receipt, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(config) });
                    sub += 1;
                }
                continue;
            }
        }
        let kind = match entry.event {
            Event::Fault(index) if sequence.faults[index].1 == Fault::Loss => Kind::End { abort: false },
            Event::Fault(_) => continue,
            Event::Op(index) => match *ops[index] {
                Op::Stop { .. } if receive => Kind::Stop,
                Op::StartRx if receive => Kind::Start,
                Op::Cold { direction, change, .. } if direction == terms.stream.direction => {
                    match change {
                        Change::Channels(_) => unreachable!("channel changes are expanded above"),
                        Change::Rate(rate) => {
                            config.ratio = terms.ratio(rate);
                            if config.channels.is_empty() { continue; }
                        }
                    }
                    Kind::Cold(config)
                }
                _ => continue,
            },
        };
        items.push(Item { e: entry.e, seq: entry.seq * 2, ready: entry.receipt, release: NO_RELEASE, delivered: None, refused: false, kind });
    }
    items
}

/// Per field, how many sequences show it and the first seed that does.
type Divergences = BTreeMap<&'static str, (usize, u64)>;

#[test]
fn rm_26_the_timeline_against_uhd_control() {
    let shown: Option<u64> = std::env::var("EZSDR_TIMELINE_SEED").ok().and_then(|seed| seed.parse().ok());
    let mut summary: BTreeMap<Class, (usize, Divergences)> = BTreeMap::new();
    let (mut failed, mut uncompared) = (Vec::new(), 0);
    let (mut became_two, mut reduced_from_two) = (0, 0);
    for seed in 0..1_000 {
        let sequence = generator::sequence(seed, &generator::RATES);
        let mut memberships = [1u16; 2];
        for (_, op) in sequence.ops() {
            if let Op::Cold { direction, change: Change::Channels(next), .. } = *op {
                let index = usize::from(direction == Direction::Tx);
                let before = memberships[index];
                if before < 2 && next == 2 { became_two += 1; }
                if before == 2 && next < 2 { reduced_from_two += 1; }
                memberships[index] = next;
            }
        }
        let expected = expect(&sequence);
        let observed = run(&sequence);
        let diverged = observed.diverged(&expected);
        if shown == Some(seed) {
            eprintln!("seed {seed}: {sequence:?}\nexpected {expected:?}\nobserved {observed:?}");
        }
        let window = release_window_matters(&sequence);
        uncompared += usize::from(window);
        let printed = |field: &&str| divergent(sequence.class).contains(field) || (*field == "timed" && window);
        let gated: Vec<_> = diverged.iter().filter(|field| !printed(field)).collect();
        if !gated.is_empty() {
            failed.push(format!("seed {seed}, {:?}: {gated:?}", sequence.class));
        }
        let (count, fields) = summary.entry(sequence.class).or_default();
        *count += 1;
        for field in diverged {
            fields.entry(field).or_insert((0, seed)).0 += 1;
        }
    }
    eprintln!("layer 2: two-channel transitions in 1,000 seeds: became two {became_two}, reduced from two {reduced_from_two}");
    assert!(became_two > 0 && reduced_from_two > 0, "the generated sequences must cover both two-channel membership transitions");
    for (class, (count, fields)) in &summary {
        let equal: Vec<_> = divergent(*class).iter().filter(|field| !fields.contains_key(*field)).collect();
        eprintln!("layer 2, {class:?}: {count} sequences; printed {fields:?}; listed but equal {equal:?}");
    }
    eprintln!("layer 2: `timed` not compared in {uncompared} sequences, the release window's (#62)");
    assert!(failed.is_empty(), "{} sequences differ (EZSDR_TIMELINE_SEED=<seed> shows one), the first: {:#?}", failed.len(), &failed[..failed.len().min(20)]);
}

/// Issue #62, the one case `timed` is not compared: the UHD Module releases a held timed
/// update to the device up to the release window (3 ms) ahead of its instant, and the
/// device queue is in order (UR-24), so an update booked once a later-instant one may have
/// been released is applied late behind it, and one released before a loss its instant
/// follows is issued; MockRadio and the timeline apply each at its instant. Whether
/// MockRadio models the release window is the owner's decision (#62). Detected from the
/// sequence alone, with the rig's 1 ms poll as margin.
fn release_window_matters(sequence: &Sequence) -> bool {
    let schedule = layout_schedule(sequence, &terms(Direction::Rx));
    let ops: Vec<&Op> = sequence.ops().map(|(_, op)| op).collect();
    let margin = (RELEASE_WINDOW_NS + 1_000_000) / 5;
    let timed: Vec<&Entry> = schedule.iter().filter(|entry| matches!(entry.event, Event::Op(op) if matches!(ops[op], Op::Timed { .. }))).collect();
    let loss = sequence.faults.iter().filter(|(_, fault)| *fault == Fault::Loss).map(|(ns, _)| T0 + ns / 5).min();
    timed.iter().any(|b| timed.iter().any(|a| a.seq < b.seq && b.e < a.e && b.receipt + margin >= a.e))
        || loss.is_some_and(|loss| timed.iter().any(|entry| entry.e >= loss && entry.e < loss + margin && entry.receipt < loss))
}

/// The timeline's account of a sequence, with UR-24's issued updates; nothing is refused,
/// and after a loss nothing more is booked.
fn expect(sequence: &Sequence) -> Record {
    let (rx, tx) = (terms(Direction::Rx), terms(Direction::Tx));
    let clocks = |terms: &Terms| plan(&terms.stream, &layout_items(sequence, terms)).unwrap().iter()
        .map(|s| (s.origin, s.cut.map(|k| s.instant(k).unwrap()))).collect();
    let schedule = layout_schedule(sequence, &rx);
    let lost = schedule.iter().position(|entry| matches!(entry.event, Event::Fault(i) if sequence.faults[i].1 == Fault::Loss));
    let ops: Vec<&Op> = sequence.ops().map(|(_, op)| op).collect();
    let timed = schedule.iter().enumerate()
        .filter(|(index, _)| !lost.is_some_and(|lost| *index > lost))
        .filter_map(|(_, entry)| match entry.event {
            Event::Op(op) => match *ops[op] {
                Op::Timed { direction, gain_db, .. } => Some((format!("radio.{}.c0.gain_db", side(direction)), gain_db, entry.e)),
                _ => None,
            },
            Event::Fault(_) => None,
        })
        .collect();
    Record { rx_clocks: clocks(&rx), tx_clocks: clocks(&tx), timed: by_instant(timed), rejected: 0 }
}

/// The rows by instant and key, each key's in the order written: two directions' updates
/// at one instant have no order between them (UC-2).
fn by_instant(mut rows: Vec<(String, f64, i64)>) -> Vec<(String, f64, i64)> {
    rows.sort_by(|a, b| (a.2, &a.0).cmp(&(b.2, &b.0)));
    rows
}

fn side(direction: Direction) -> &'static str {
    if direction == Direction::Rx { "rx" } else { "tx" }
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
    fn publish(&self, _block: BlockRef) -> PublishOutcome { PublishOutcome::Accepted }
    fn receive(&self) -> Option<(BlockRef, DropCarry)> { None }
    fn drops(&self) -> u64 { 0 }
    fn take_drop_carry(&self) -> DropCarry { DropCarry::default() }
    fn policy(&self) -> BackPressure { BackPressure::DropOldest }
}

/// Books a sequence on uhd-control as its loop does (MA-14b): each round's Actions at its
/// instant, each followed by a release, and a release every millisecond; a loss marks the
/// device lost at its instant, before a round at that instant, and nothing is booked after.
/// The owners are idealized: uhd-rx registers a receive clock once its origin has passed and
/// ends it at its cut once that has passed too, as UR-17 says; layer 3 runs the real ones.
fn run(sequence: &Sequence) -> Record {
    // The FakeDevice's queue drains on the host's clock, which this rig does not advance.
    let fake = FakeConfig { command_queue: usize::MAX, ..FakeConfig::default() };
    let (core, device, time, _) = super::super::test_support::rig_with(fake, vec![Arc::new(NoLink)]);
    let tx = core.register(Dir::Tx, 200, 0).unwrap();
    device.rx_open(0, &[0, 1]).unwrap();
    device.tx_open(0, &[0, 1]).unwrap();
    let (to_tx, from_tx) = channel();
    let mut config = core.description.defaults();
    config.insert(key("radio.tx.c0.stream"), Value::from(0));
    let start = super::Start {
        t0: T0,
        rx: vec![Some((ezsdr_radio::timeline::Channels::first(1), 200))],
        tx: vec![Some((tx, ezsdr_radio::timeline::Channels::first(1)))],
    };
    let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start);
    let ticks = |ns: i64| ns / 5;
    let at = |ns: Option<i64>| ns.map(|ns| AbsoluteDeadline::new(core.at(T0 + ticks(ns))));
    let loss = sequence.faults.iter().filter(|(_, fault)| *fault == Fault::Loss).map(|(ns, _)| T0 + ticks(*ns)).min();
    let mut rounds = sequence.rounds.iter().peekable();
    let mut memberships = [[true, false], [true, false]];
    // The idealized uhd-rx: each planned segment's clock, once registered, and whether ended.
    let mut receive: Vec<(i64, ezsdr_radio::timeline::Config, ClockDomainId, bool)> = Vec::new();
    let mut now = T0;
    loop {
        time.advance_to(core.at(now)).unwrap();
        if loss.is_some_and(|loss| loss <= now) {
            core.device_lost("VH-8: the sequence's loss");
        }
        while let Some((_, ops)) = rounds.next_if(|(round, _)| T0 + ticks(*round) <= now) {
            for op in ops {
                if let Op::Cold { direction, change: Change::Channels(count), at_ns } = *op {
                    if !core.is_lost() {
                        let side = side(direction);
                        let index = usize::from(direction == Direction::Tx);
                        let mut changed = false;
                        for (channel, member) in memberships[index].iter_mut().enumerate() {
                            let selected = channel < usize::from(count);
                            if *member == selected { continue; }
                            changed = true;
                            *member = selected;
                            control.book(ezsdr_kernel::module_api::Dispatched::new(ezsdr_kernel::event::ActionId(1), Action::UpdateParameter {
                                target: target_of(&core.id),
                                key: key(&format!("radio.{side}.c{channel}.stream")),
                                value: Value::from(if selected { 0 } else { -1 }),
                                class: UpdateClass::Cold,
                                at: at(at_ns),
                            }, Some(core.id.clone())));
                        }
                        if !changed {
                            control.book(ezsdr_kernel::module_api::Dispatched::new(ezsdr_kernel::event::ActionId(1), Action::UpdateParameter {
                                target: target_of(&core.id),
                                key: key(&format!("radio.{side}.c0.stream")),
                                value: Value::from(if count == 0 { -1 } else { 0 }),
                                class: UpdateClass::Cold,
                                at: at(at_ns),
                            }, Some(core.id.clone())));
                        }
                    }
                    continue;
                }
                if !core.is_lost() {
                    // The node each Action is resolved to (KC-23); its target only names it.
                    let (action, node) = match *op {
                        Op::Stop { device } => {
                            let node = if device { core.id.clone() } else { core.rx_id.clone() };
                            (Action::Stop { target: Some(target_of(&node)) }, node)
                        }
                        Op::StartRx => (Action::Command { target: target_of(&core.rx_id), verb: Ident::parse("start_rx").unwrap(), params: BTreeMap::new(), at: None }, core.rx_id.clone()),
                        Op::Cold { direction, change, at_ns } => {
                            let (name, value) = match change {
                                Change::Channels(n) => ("c0.stream", Value::from(if n == 0 { -1 } else { 0 })),
                                Change::Rate(rate) => ("s0.sample_rate_hz", Value::num(rate as f64).unwrap()),
                            };
                            (Action::UpdateParameter { target: target_of(&core.id), key: key(&format!("radio.{}.{name}", side(direction))), value, class: UpdateClass::Cold, at: at(at_ns) }, core.id.clone())
                        }
                        Op::Timed { direction, gain_db, at_ns } => (Action::UpdateParameter {
                            target: target_of(&core.id), key: key(&format!("radio.{}.c0.gain_db", side(direction))), value: Value::num(gain_db).unwrap(), class: UpdateClass::HardwareTimed, at: at(at_ns),
                        }, core.id.clone()),
                    };
                    control.book(ezsdr_kernel::module_api::Dispatched::new(ezsdr_kernel::event::ActionId(1), action, Some(node)));
                }
                control.release();
            }
        }
        control.release();
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
    let records = core.clocks.sample_clock_records();
    let clocks_of = |stream| records.iter().filter(|r| r.stream == stream).map(|r| (r.origin.ticks_in(r.root).unwrap(), r.ended_at.map(|t| t.ticks_in(r.root).unwrap()))).collect();
    let rec = lock(&core.rec);
    Record {
        rx_clocks: clocks_of(core.rx_id.clone()),
        tx_clocks: clocks_of(core.tx_id.clone()),
        timed: by_instant(rec.applied.iter()
            .filter(|r| r["issued"] == true || r["note"] == "no channel")
            .map(|r| (r["key"].as_str().unwrap().to_owned(), r["claimed"].as_f64().unwrap(), r["at"]["ticks"].as_i64().unwrap()))
            .collect()),
        rejected: rec.rejected.len(),
    }
}
