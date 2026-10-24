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
use ezsdr_kernel::time::{AbsoluteDeadline, Rational, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Stream, NO_RELEASE, plan};
use super::super::test_support::target_of;
use generator::{Change, Class, Entry, Event, Fault, Op, Sequence, Terms};

use ezsdr_kernel::id::ClockDomainId;

use super::Control;
use super::super::core::{booked_stream, key, lock};
use crate::device::{Device, Dir, FakeConfig};
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS};

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
/// ended)`, the `hardware_timed` updates as `(key, value, at)` in the order of their
/// instants, the `LATE_COMMAND`s, the normalized `applied` rows, and how many Actions were
/// refused.
#[derive(PartialEq, Debug, Default)]
struct Record {
    rx_clocks: Vec<(i64, Option<i64>)>,
    tx_clocks: Vec<(i64, Option<i64>)>,
    timed: Vec<(String, f64, i64)>,
    late: Vec<(Option<String>, i64, i64)>,
    applied: Vec<(String, f64, i64, bool)>,
    rejected: usize,
}

impl Record {
    fn diverged(&self, other: &Record) -> Vec<&'static str> {
        [
            ("rx_clocks", self.rx_clocks == other.rx_clocks),
            ("tx_clocks", self.tx_clocks == other.tx_clocks),
            ("timed", self.timed == other.timed),
            ("late", self.late == other.late),
            ("applied", self.applied == other.applied),
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
    let release_window = ticks(super::super::test_support::default_description(2_000).timing.release_window_ns);
    Terms { t0: T0, root_hz: 200_000_000, timed_lead: ticks(DEVICE_LEAD_NS), release_window, stream }
}

/// The generated operations' effective instants with the layout's instance-wide cold floor
/// and the timeds' floor (RM-21, design 38 D2 (1)); `floored` says, by op index, each timed
/// update's instant and whether its requested one held.
fn layout_schedule(sequence: &Sequence, terms: &Terms) -> (Vec<Entry>, BTreeMap<usize, (i64, bool)>) {
    let mut entries: Vec<Entry> = sequence.faults.iter().enumerate().map(|(index, (at, _))| {
        let e = terms.t0 + terms.ticks(*at);
        Entry { e, seq: index as u64, receipt: e, event: Event::Fault(index) }
    }).collect();
    let mut cold = Vec::new();
    let mut timed: Vec<i64> = Vec::new();
    let mut floored = BTreeMap::new();
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
            Op::Timed { .. } => {
                let (e, late) = ezsdr_radio::timeline::timed_instant(requested, receipt, terms.timed_lead, terms.release_window, timed.iter().copied());
                timed.push(e);
                floored.insert(index, (e, late));
                e
            }
        };
        entries.push(Entry { e, seq: (sequence.faults.len() + index) as u64, receipt, event: Event::Op(index) });
    }
    entries.sort_by_key(|entry| (entry.e, entry.seq));
    (entries, floored)
}

/// A generated channel count is expanded as c0/c1 membership changes at the same instant.
fn layout_items(sequence: &Sequence, terms: &Terms) -> Vec<Item> {
    let receive = terms.stream.direction == Direction::Rx;
    let ops: Vec<&Op> = sequence.ops().map(|(_, op)| op).collect();
    let mut config = terms.stream.config;
    let mut items = Vec::new();
    let (schedule, _) = layout_schedule(sequence, terms);
    for entry in schedule {
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
    let (mut failed, mut ceiling_rows) = (Vec::new(), 0);
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
        let mut expected = expect(&sequence);
        let mut observed = run(&sequence);
        // Design 38 D2 (4): a command due before the loss that uhd-control's release did not
        // hand over before the end is the Ceiling, and only its row is left out.
        let ceiling = ceilings(&sequence);
        ceiling_rows += ceiling.len();
        for row in &ceiling {
            drop_row(&mut expected.applied, row);
            drop_row(&mut observed.applied, row);
        }
        let diverged = observed.diverged(&expected);
        if shown == Some(seed) {
            eprintln!("seed {seed}: {sequence:?}\nexpected {expected:?}\nobserved {observed:?}\nceiling {ceiling:?}");
        }
        let printed = |field: &&str| divergent(sequence.class).contains(field);
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
    // Design 38 D2 (4), S2: one sequence with a channel-less head, a channel-less command behind
    // it and a live one behind both at one instant, due before a loss no release ran before — the
    // Ceiling. Only the live command's row is left out; the head's and the channel-less
    // successor's are compared, so a Module that records either as cancelled fails here as it
    // fails the series' own test.
    let stalled = Sequence {
        class: Class::Loss,
        rounds: vec![(0, vec![
            Op::Cold { direction: Direction::Rx, change: Change::Channels(0), at_ns: None },
            Op::Timed { direction: Direction::Rx, gain_db: 1.0, at_ns: Some(10_400_000) },
            Op::Timed { direction: Direction::Rx, gain_db: 2.0, at_ns: Some(10_400_000) },
            Op::Timed { direction: Direction::Tx, gain_db: 3.0, at_ns: Some(10_400_000) },
        ])],
        faults: vec![(10_900_000, Fault::Loss)],
    };
    let mut expected = expect(&stalled);
    let mut observed = run(&stalled);
    let ceiling = ceilings(&stalled);
    assert_eq!(ceiling.len(), 1, "the explicit sequence's Ceiling: {ceiling:?}");
    for row in &ceiling {
        drop_row(&mut expected.applied, row);
        drop_row(&mut observed.applied, row);
    }
    let diverged = observed.diverged(&expected);
    assert!(diverged.is_empty(), "the Ceiling's sequence: {diverged:?}\nexpected {expected:?}\nobserved {observed:?}");
    eprintln!("layer 2: `timed`, `late` and the normalized `applied` rows compared in every sequence; {ceiling_rows} rows of the 1,000 seeds are design 38 D2 (4)'s Ceiling");
    assert!(failed.is_empty(), "{} sequences differ (EZSDR_TIMELINE_SEED=<seed> shows one), the first: {:#?}", failed.len(), &failed[..failed.len().min(20)]);
}

/// Leaves out the first row `(key, claimed, at)` one of the Ceiling commands wrote; the two
/// Modules' rows are otherwise compared, so a misclassification cannot hide behind it.
fn drop_row(rows: &mut Vec<(String, f64, i64, bool)>, row: &(String, f64, i64)) {
    if let Some(index) = rows.iter().position(|candidate| candidate.0 == row.0 && candidate.1 == row.1 && candidate.2 == row.2) {
        rows.remove(index);
    }
}

/// The instant of the sequence's loss, if it has one.
fn loss_of(sequence: &Sequence) -> Option<i64> {
    sequence.faults.iter().filter(|(_, fault)| *fault == Fault::Loss).map(|(ns, _)| T0 + ns / 5).min()
}

/// The instants uhd-control's loop runs a release at, as `run` walks: every round's and every
/// millisecond's `now` below the loss. The loss is published before that instant's release, so
/// none runs there (design 38 D2 (4), S2).
fn release_instants(sequence: &Sequence) -> Vec<i64> {
    let ticks = |ns: i64| ns / 5;
    let loss = loss_of(sequence);
    let mut rounds = sequence.rounds.iter().peekable();
    let mut now = T0;
    let mut instants = Vec::new();
    loop {
        let lost = loss.is_some_and(|loss| loss <= now);
        while rounds.next_if(|(round, _)| T0 + ticks(*round) <= now).is_some() {}
        if !lost {
            instants.push(now);
        }
        if now >= HORIZON {
            break;
        }
        let next_round = rounds.peek().map(|(round, _)| T0 + ticks(*round));
        now = [Some(now + MS), next_round, loss.filter(|loss| *loss > now)].into_iter().flatten().min().unwrap();
    }
    instants
}

/// Whether the expected plan gives `channel` a segment at or after `t`: one covering `t`, or
/// one beginning later (UR-24's `targets`).
fn reaches(plan: &[ezsdr_radio::timeline::Segment], channel: u32, t: i64) -> bool {
    plan.iter().any(|segment| {
        segment.config.channels.contains(channel) && (segment.origin >= t || (segment.origin <= t && segment.cut.is_none_or(|cut| segment.instant(cut).unwrap() > t)))
    })
}

/// The commands whose row the comparison leaves out, decided from the sequence alone (design
/// 38 D2 (4), S2): a live command B due before the loss, behind an earlier one A whose channel
/// the expected plan holds neither at A's instant nor in a later segment, when no release ran at
/// or after A's instant and before the loss. uhd-control did not hand B over then — its release
/// lag, which the timeline does not model — so its row alone differs, and only its row is left
/// out; a channel-less command keeps its row wherever it stands, as both Modules record it.
fn ceilings(sequence: &Sequence) -> Vec<(String, f64, i64)> {
    let Some(loss) = loss_of(sequence) else { return Vec::new() };
    let releases = release_instants(sequence);
    let ops: Vec<(i64, &Op)> = sequence.ops().collect();
    let (_, floored) = layout_schedule(sequence, &terms(Direction::Rx));
    let plans = |direction: Direction| plan(&terms(direction).stream, &layout_items(sequence, &terms(direction))).unwrap();
    let (rx, tx) = (plans(Direction::Rx), plans(Direction::Tx));
    let mut timeds: Vec<(i64, usize)> = floored.iter().map(|(&index, entry)| (entry.0, index)).collect();
    timeds.sort_unstable();
    let mut blocked: Vec<i64> = Vec::new();
    let mut rows = Vec::new();
    for (e, index) in timeds {
        let (round, op) = ops[index];
        let Op::Timed { direction, gain_db, .. } = *op else { continue };
        // A command received at or after the loss was never booked (design 38 D2 (4)).
        if T0 + round / 5 >= loss {
            continue;
        }
        let channel_less = !reaches(if direction == Direction::Rx { &rx } else { &tx }, 0, e);
        let stalled = blocked.iter().any(|earlier| releases.iter().all(|t| !(*t >= *earlier && *t < loss)));
        // Only a command with a channel here can be the Ceiling: a channel-less one is
        // `no channel` at either Module (design 38 D2 (4), S2).
        if !channel_less && stalled && e < loss {
            rows.push((format!("radio.{}.c0.gain_db", side(direction)), gain_db, e));
        }
        // Its own channel-less instant blocks the ones behind it only without a release then.
        if channel_less {
            blocked.push(e);
        }
    }
    rows
}

/// The timeline's account of a sequence, with UR-24's rows and late commands; nothing is
/// refused, and after a loss nothing more is booked, every pending command settling at it.
fn expect(sequence: &Sequence) -> Record {
    let (rx, tx) = (terms(Direction::Rx), terms(Direction::Tx));
    let clocks = |terms: &Terms| plan(&terms.stream, &layout_items(sequence, terms)).unwrap().iter()
        .map(|s| (s.origin, s.cut.map(|k| s.instant(k).unwrap()))).collect();
    let loss = loss_of(sequence);
    let (_, floored) = layout_schedule(sequence, &rx);
    let mut timed = Vec::new();
    let mut late = Vec::new();
    let mut applied = Vec::new();
    for (index, (round, op)) in sequence.ops().enumerate() {
        let receipt = T0 + round / 5;
        let Op::Timed { direction, gain_db, at_ns } = *op else { continue };
        // A command received at or after the loss is never booked (design 38 D2 (4)).
        if loss.is_some_and(|loss| receipt >= loss) {
            continue;
        }
        let (e, was_late) = floored[&index];
        let key = format!("radio.{}.c0.gain_db", side(direction));
        timed.push((key.clone(), gain_db, e));
        if was_late {
            late.push((Some(key.clone()), T0 + at_ns.expect("a late command's requested instant") / 5, e));
        }
        // One due before the loss takes effect at its instant; one due at or after it is
        // cancelled then (UR-26, design 38 D2 (4)).
        applied.push((key, gain_db, e, loss.is_some_and(|loss| e >= loss)));
    }
    let rows = |mut rows: Vec<(String, f64, i64, bool)>| {
        rows.sort_by(|a, b| (a.2, &a.0).cmp(&(b.2, &b.0)));
        rows
    };
    Record { rx_clocks: clocks(&rx), tx_clocks: clocks(&tx), timed: by_instant(timed), late, applied: rows(applied), rejected: 0 }
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
    let (core, device, time, events) = super::super::test_support::rig_with(fake, vec![Arc::new(NoLink)]);
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
    // Design 38 D2 (4): the end at the loss's instant settles every command still pending,
    // and writes the rows of the given ones whose instant came.
    if core.is_lost() {
        control.finish();
    }
    let records = core.clocks.sample_clock_records();
    let clocks_of = |stream| records.iter().filter(|r| r.stream == stream).map(|r| (r.origin.ticks_in(r.root).unwrap(), r.ended_at.map(|t| t.ticks_in(r.root).unwrap()))).collect();
    let late = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
        .filter(|event| event.kind.as_str() == kinds::LATE_COMMAND)
        .filter_map(|event| {
            let key = event.payload["key"].as_str()?.to_owned();
            (key.ends_with(".gain_db") || key.ends_with(".frequency_hz")).then(|| {
                (Some(key), event.payload["requested"]["ticks"].as_i64().unwrap(), event.payload["applied"]["ticks"].as_i64().unwrap())
            })
        })
        .collect();
    let rec = lock(&core.rec);
    let rows: Vec<(String, f64, i64, bool)> = rec.applied.iter()
        .filter(|r| r["read_back"].is_null())
        .filter_map(|r| {
            let key = r["key"].as_str()?;
            (key.ends_with(".gain_db") || key.ends_with(".frequency_hz")).then(|| {
                (key.to_owned(), r["claimed"].as_f64().unwrap(), r["at"]["ticks"].as_i64().unwrap(), r["cancelled"].is_string())
            })
        })
        .collect();
    let mut applied = rows.clone();
    applied.sort_by(|a, b| (a.2, &a.0).cmp(&(b.2, &b.0)));
    Record {
        rx_clocks: clocks_of(core.rx_id.clone()),
        tx_clocks: clocks_of(core.tx_id.clone()),
        timed: by_instant(rows.into_iter().map(|(key, value, at, _)| (key, value, at)).collect()),
        late,
        applied,
        rejected: rec.rejected.len(),
    }
}

#[test]
fn rm_26_the_ceiling_is_decided_from_the_sequence() {
    // Design 38 D2 (4), S2: a live command due before the loss behind a channel-less head is the
    // Ceiling when no release ran between that head's instant and the loss — the three commands
    // due at 10.4 ms here, the channel-less pair at 10.4 ms and the loss at 10.9 ms. The pair
    // keeps its `no channel` rows, so only the live one is left out. When a release does run then
    // (the walk's 11 ms before a loss at 11.9 ms), both Modules hand it over, and its row is
    // compared.
    let sequence = |loss: i64| Sequence {
        class: Class::Loss,
        rounds: vec![(0, vec![
            Op::Cold { direction: Direction::Rx, change: Change::Channels(0), at_ns: None },
            Op::Timed { direction: Direction::Rx, gain_db: 1.0, at_ns: Some(10_400_000) },
            Op::Timed { direction: Direction::Rx, gain_db: 2.0, at_ns: Some(10_400_000) },
            Op::Timed { direction: Direction::Tx, gain_db: 3.0, at_ns: Some(10_400_000) },
        ])],
        faults: vec![(loss, Fault::Loss)],
    };
    let live = ("radio.tx.c0.gain_db".to_owned(), 3.0, T0 + 10_400_000 / 5);
    assert_eq!(ceilings(&sequence(10_900_000)), [live], "the stalled live command is not the Ceiling alone");
    assert!(ceilings(&sequence(11_900_000)).is_empty(), "a released head's successor was left out");
    // The channel-less head and the channel-less command behind it keep their rows.
    assert!(ceilings(&sequence(10_900_000)).iter().all(|row| row.0 != "radio.rx.c0.gain_db"));
}
