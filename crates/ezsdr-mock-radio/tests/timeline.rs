//! VH-8's first layer (spec 22): MockRadio against `ezsdr_radio::timeline` on generated
//! sequences. Step 1 gates only the classes where MockRadio already equals the timeline
//! and prints every other divergence, which lists today's issues mechanically; step 2
//! gates every class.

use ezsdr_kernel::spec::Scalar;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{Action, ActionId, EventCollector, EventSource, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Endpoint, ModuleErrorKind, ModuleRef, Pacing, PrepareContext, ProfileRef, Provider, Requested, Role, UpdateClass, Version,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, BlockRef, DataLink, Direction, DropCarry, PublishOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, RelativeBudget, TimePoint};
use ezsdr_mock_radio::MockRadio;
use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Line, Lines, NO_RELEASE, Segment, Stream, StreamKey};
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

#[derive(Clone, Copy, PartialEq, Debug)]
enum LayoutChange {
    Rate { stream: u32, rate: u64 },
    Membership { channel: u32, stream: i64 },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum LayoutOp {
    Stop { device: bool, stream: Option<u32> },
    StartRx { stream: u32 },
    Cold { direction: Direction, change: LayoutChange, at_ns: Option<i64> },
    Timed { direction: Direction, channel: u32, gain_db: f64, at_ns: Option<i64> },
}

#[derive(Clone, PartialEq, Debug)]
struct LayoutSequence {
    class: Class,
    rounds: Vec<(i64, Vec<LayoutOp>)>,
    faults: Vec<(i64, Fault)>,
}

impl LayoutSequence {
    fn ops(&self) -> impl Iterator<Item = (i64, &LayoutOp)> {
        self.rounds.iter().flat_map(|(at, ops)| ops.iter().map(move |op| (*at, op)))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TransitionCounts {
    leave: usize,
    return_to_occupied: usize,
    move_between_streams: usize,
    join_empty_stream: usize,
}

fn direction_index(direction: Direction) -> usize {
    usize::from(direction == Direction::Tx)
}

fn add_membership(round: &mut Vec<LayoutOp>, state: &mut [[Option<u32>; 2]; 2], counts: &mut TransitionCounts, direction: Direction, channel: u32, target: i64, at_ns: Option<i64>) {
    let direction_index = direction_index(direction);
    let channel_index = channel as usize;
    let before = state[direction_index][channel_index];
    let after = u32::try_from(target).ok();
    if before == after {
        return;
    }
    match (before, after) {
        (Some(_), None) => counts.leave += 1,
        (Some(_), Some(_)) => counts.move_between_streams += 1,
        (None, Some(stream)) => {
            if state[direction_index].iter().enumerate().any(|(index, held)| index != channel_index && *held == Some(stream)) {
                counts.return_to_occupied += 1;
            } else {
                counts.join_empty_stream += 1;
            }
        }
        (None, None) => unreachable!(),
    }
    state[direction_index][channel_index] = after;
    round.push(LayoutOp::Cold { direction, change: LayoutChange::Membership { channel, stream: target }, at_ns });
}

/// Expands the old 0/1/2 channel-count input into the equivalent c<n>.stream changes,
/// then guarantees one generated example of each membership transition in every seed.
fn layout_sequence(seed: u64, sequence: &Sequence) -> (LayoutSequence, TransitionCounts) {
    let mut state = [[Some(0), None], [Some(0), None]];
    let mut counts = TransitionCounts::default();
    let mut rounds = Vec::new();
    for (at, ops) in &sequence.rounds {
        let mut layout_ops = Vec::new();
        for op in ops {
            match *op {
                Op::Stop { device: true } => layout_ops.push(LayoutOp::Stop { device: true, stream: None }),
                Op::Stop { device: false } => {
                    for stream in 0..2 {
                        layout_ops.push(LayoutOp::Stop { device: false, stream: Some(stream) });
                    }
                }
                Op::StartRx => {
                    for stream in 0..2 {
                        layout_ops.push(LayoutOp::StartRx { stream });
                    }
                }
                Op::Cold { direction, change: Change::Channels(channels), at_ns } => {
                    for channel in 0..2 {
                        add_membership(&mut layout_ops, &mut state, &mut counts, direction, channel, if channel < u32::from(channels) { 0 } else { -1 }, at_ns);
                    }
                }
                Op::Cold { direction, change: Change::Rate(rate), at_ns } => {
                    for stream in 0..2 {
                        layout_ops.push(LayoutOp::Cold { direction, change: LayoutChange::Rate { stream, rate }, at_ns });
                    }
                }
                Op::Timed { direction, gain_db, at_ns } => {
                    layout_ops.push(LayoutOp::Timed { direction, channel: 0, gain_db, at_ns });
                }
            }
        }
        rounds.push((*at, layout_ops));
    }

    let latest = sequence
        .ops()
        .map(|(round, op)| {
            let at = match op {
                Op::Cold { at_ns, .. } | Op::Timed { at_ns, .. } => at_ns.unwrap_or(0),
                _ => 0,
            };
            round.max(at)
        })
        .max()
        .unwrap_or(0)
        .max(0);
    let tail = latest + 10 * MS;
    let direction = if seed % 2 == 0 { Direction::Rx } else { Direction::Tx };
    for (offset, channel, stream) in [(0, 0, 0), (1, 1, 1), (10, 0, -1), (20, 0, 1), (30, 1, 0), (40, 0, 0), (50, 1, -1), (60, 1, 1)] {
        let mut ops = Vec::new();
        add_membership(&mut ops, &mut state, &mut counts, direction, channel, stream, None);
        rounds.push((tail + offset * MS, ops));
    }
    (LayoutSequence { class: sequence.class, rounds, faults: sequence.faults.clone() }, counts)
}

fn fractional_tx(sequence: &LayoutSequence) -> bool {
    sequence.ops().any(|(_, op)| matches!(op, LayoutOp::Cold { direction: Direction::Tx, change: LayoutChange::Rate { rate: 3_000_000, .. }, .. }))
}

/// RM-25's layout floor is instance-wide, unlike the old generator's per-direction floor.
fn layout_schedule(profile: &str, sequence: &LayoutSequence) -> Vec<Entry> {
    let terms = terms(profile, Direction::Rx, Channels::first(0));
    let mut entries: Vec<_> = sequence
        .faults
        .iter()
        .enumerate()
        .map(|(index, (at, _))| {
            let e = terms.t0 + terms.ticks(*at);
            Entry { e, seq: index as u64, receipt: e, event: Event::Fault(index) }
        })
        .collect();
    let mut floor = i64::MIN;
    let mut timed: Vec<i64> = Vec::new();
    for (index, (round, op)) in sequence.ops().enumerate() {
        let receipt = terms.t0 + terms.ticks(round);
        let at = |at_ns: &Option<i64>| at_ns.map(|ns| terms.t0 + terms.ticks(ns));
        let e = match op {
            LayoutOp::Stop { .. } | LayoutOp::StartRx { .. } => receipt,
            LayoutOp::Cold { at_ns, .. } => {
                floor = at(at_ns).unwrap_or(receipt).max(receipt).max(floor);
                floor
            }
            LayoutOp::Timed { at_ns, .. } => {
                let (e, _) = ezsdr_radio::timeline::timed_instant(at(at_ns), receipt, terms.timed_lead, terms.release_window, timed.iter().copied());
                timed.push(e);
                e
            }
        };
        entries.push(Entry { e, seq: (sequence.faults.len() + index) as u64, receipt, event: Event::Op(index) });
    }
    entries.sort_by_key(|entry| (entry.e, entry.seq));
    entries
}

/// What layer 1 observes: per-stream SampleClocks, blocks with stream/clock/channel count,
/// applied `cold` and `hardware_timed` rows, refused Actions, fault rows, and non-loss errors.
type BlockRecord = (u32, usize, i64, u32, u16, BlockFlags, Option<u64>);

#[derive(PartialEq, Debug, Default)]
struct Record {
    rx_clocks: Vec<(u32, i64, Option<i64>)>,
    tx_clocks: Vec<(u32, i64, Option<i64>)>,
    blocks: Vec<BlockRecord>,
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
                eprintln!(
                    "{name} from {first} (lengths {}, {}):\n  expected {:?}\n  observed {:?}",
                    a.len(),
                    b.len(),
                    &a[first.min(a.len())..(first + 4).min(a.len())],
                    &b[first.min(b.len())..(first + 4).min(b.len())]
                );
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

/// A profile's timeline terms; `x3xx` with VH-4's ready term (one receive call of
/// 2 000 samples, 3 ms) and the timed lead of MR-3.
fn terms(profile: &str, direction: Direction, channels: Channels) -> Terms {
    let (lead, call, allowance, timed_lead, release_window) = if profile == "ideal" { (0, 0, 0, 0, 0) } else { (50 * MS, 2_000, 3 * MS, 2 * MS, 3 * MS) };
    // The transmit clock starts at `arm`, at 0 (RM-25); the receive clock at T0.
    let origin = if direction == Direction::Rx { T0 } else { 0 };
    let config = Config { channels, ratio: Rational::new(1_000_000_000, 1_000_000).unwrap() };
    Terms {
        t0: T0,
        root_hz: 1_000_000_000,
        timed_lead,
        release_window,
        stream: Stream { direction, origin, config, lead, call, release_window: if direction == Direction::Rx { release_window } else { 0 }, allowance },
    }
}

fn line_key(direction: Direction, stream: u32) -> StreamKey {
    (direction, stream)
}

fn initial_lines(profile: &str) -> Lines {
    let initial = [[Some(0), None], [Some(0), None]];
    let mut lines = Lines::new();
    for direction in [Direction::Rx, Direction::Tx] {
        for stream in 0..2 {
            let members = initial[direction_index(direction)];
            let channels = members.iter().enumerate().fold(Channels::first(0), |set, (channel, member)| if *member == Some(stream) { set.with(channel as u32) } else { set });
            lines.insert(line_key(direction, stream), Line::new(terms(profile, direction, channels).stream));
        }
    }
    lines
}

/// Per field, how many sequences show it and the first seed that does.
type Divergences = BTreeMap<&'static str, (usize, u64)>;

#[test]
fn rm_26_the_timeline_against_mockradio() {
    // `EZSDR_TIMELINE_SEED=<seed>` prints that sequence's two records side by side.
    let shown: Option<u64> = std::env::var("EZSDR_TIMELINE_SEED").ok().and_then(|seed| seed.parse().ok());
    let mut summary: BTreeMap<(&str, Class), (usize, Divergences)> = BTreeMap::new();
    let mut failed = Vec::new();
    let mut transitions = TransitionCounts::default();
    for seed in 0..1_000 {
        // 3 MS/s, 1 000/3 ticks a sample, is an `ideal` rate only.
        let (profile, rates) = if seed % 2 == 0 { ("ideal", &generator::FRACTIONAL_RATES[..]) } else { ("x3xx", &generator::RATES[..]) };
        let generated = generator::sequence(seed, rates);
        let (sequence, seen) = layout_sequence(seed, &generated);
        transitions.leave += seen.leave;
        transitions.return_to_occupied += seen.return_to_occupied;
        transitions.move_between_streams += seen.move_between_streams;
        transitions.join_empty_stream += seen.join_empty_stream;
        let expected = expect(profile, &sequence);
        let observed = run(profile, &sequence);
        let diverged = observed.diverged(&expected);
        if shown == Some(seed) {
            eprintln!("seed {seed}, {profile}: {sequence:?}");
            expected.show(&observed);
        }
        let printed = |field: &&str| divergent(profile, sequence.class).contains(field) || (fractional_tx(&sequence) && FRACTIONAL_TX.contains(field));
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
    eprintln!(
        "layout membership transitions in 1,000 seeds: leave={}, return={}, move={}, empty-stream join={}",
        transitions.leave, transitions.return_to_occupied, transitions.move_between_streams, transitions.join_empty_stream
    );
    assert!(
        transitions.leave > 0 && transitions.return_to_occupied > 0 && transitions.move_between_streams > 0 && transitions.join_empty_stream > 0,
        "each layout membership transition must be generated: {transitions:?}"
    );
    assert!(failed.is_empty(), "{} sequences differ (EZSDR_TIMELINE_SEED=<seed> shows one), the first: {:#?}", failed.len(), &failed[..failed.len().min(20)]);
}

fn book(lines: &mut Lines, stream: StreamKey, entry: &Entry, kind: Kind) {
    lines
        .book(stream, Item { e: entry.e, seq: entry.seq, ready: entry.receipt, release: NO_RELEASE, delivered: None, refused: false, kind })
        .unwrap_or_else(|error| panic!("timeline refused {stream:?} at {}: {error:?}", entry.e));
}

fn active_rx(lines: &Lines, entry: &Entry) -> Vec<(u32, Option<u64>)> {
    (0..2)
        .filter_map(|stream| {
            let line = lines.line(line_key(Direction::Rx, stream))?;
            line.plan.iter().find_map(|planned| {
                line.made(planned.origin, planned.config, entry.e, entry.seq)
                    .unwrap_or_else(|error| panic!("timeline fault query failed for {stream}: {error:?}"))
                    .filter(|segment| segment.cut.is_none() && segment.origin <= entry.e && !segment.config.channels.is_empty())
                    .map(|segment| (stream, segment.by))
            })
        })
        .collect()
}

fn book_operation(profile: &str, lines: &mut Lines, entry: &Entry, op: LayoutOp) {
    match op {
        LayoutOp::Stop { device: true, .. } => {
            for direction in [Direction::Rx, Direction::Tx] {
                for stream in 0..2 {
                    book(lines, line_key(direction, stream), entry, Kind::Stop);
                }
            }
        }
        LayoutOp::Stop { device: false, stream: Some(stream) } => book(lines, line_key(Direction::Rx, stream), entry, Kind::Stop),
        LayoutOp::Stop { device: false, stream: None } => unreachable!(),
        LayoutOp::StartRx { stream } => book(lines, line_key(Direction::Rx, stream), entry, Kind::Start),
        LayoutOp::Cold { direction, change: LayoutChange::Rate { stream, rate }, .. } => {
            let key = line_key(direction, stream);
            let mut config = lines.config_at(key, entry.e);
            config.ratio = terms(profile, direction, config.channels).ratio(rate);
            book(lines, key, entry, Kind::Cold(config));
        }
        LayoutOp::Cold { direction, change: LayoutChange::Membership { channel, stream }, .. } => {
            let from = (0..2).find(|index| lines.config_at(line_key(direction, *index), entry.e).channels.contains(channel));
            match (from, u32::try_from(stream).ok()) {
                (Some(from), Some(to)) if from != to => {
                    let result = lines
                        .book_move(line_key(direction, from), line_key(direction, to), channel, Some(entry.e), entry.receipt, entry.seq, entry.receipt)
                        .unwrap_or_else(|error| panic!("timeline refused move c{channel} s{from}->s{to} at {}: {error:?}", entry.e));
                    assert_eq!(result.0, entry.e, "layout move instant");
                }
                (Some(from), None) => {
                    let key = line_key(direction, from);
                    let mut config = lines.config_at(key, entry.e);
                    config.channels = config.channels.without(channel);
                    book(lines, key, entry, Kind::Cold(config));
                }
                (None, Some(to)) => {
                    let key = line_key(direction, to);
                    let mut config = lines.config_at(key, entry.e);
                    config.channels = config.channels.with(channel);
                    book(lines, key, entry, Kind::Cold(config));
                }
                _ => unreachable!("layout generator emits only membership changes"),
            }
        }
        LayoutOp::Timed { .. } => {}
    }
}

/// The timeline's account of each layout stream, with applied rows, refusals, blocks and faults.
fn expect(profile: &str, sequence: &LayoutSequence) -> Record {
    let schedule = layout_schedule(profile, sequence);
    let ops: Vec<_> = sequence.ops().map(|(_, op)| *op).collect();
    let mut lines = initial_lines(profile);
    let mut record = Record::default();
    let mut fault_rows = Vec::new();
    let loss_at = schedule.iter().find(|entry| matches!(entry.event, Event::Fault(index) if sequence.faults[index].1 == Fault::Loss)).map(|entry| entry.e);

    // Actions received before a loss have already booked their lines, even when their later
    // effective instant is cancelled by that loss. Actions received at or after it do not.
    let op_entries: Vec<_> = (0..ops.len()).map(|index| schedule.iter().find(|entry| entry.event == Event::Op(index)).copied().unwrap()).collect();
    for (index, (_, op)) in sequence.ops().enumerate() {
        let entry = op_entries[index];
        if loss_at.is_some_and(|loss| entry.receipt >= loss) {
            continue;
        }
        book_operation(profile, &mut lines, &entry, *op);
    }

    let mut lost_device = false;
    for entry in &schedule {
        match entry.event {
            Event::Fault(fault_index) => {
                let kind = sequence.faults[fault_index].1;
                if lost_device {
                    fault_rows.push((entry.e, entry.seq, kind, false, None, Vec::new()));
                    continue;
                }
                let active = active_rx(&lines, entry);
                if kind == Fault::Loss {
                    fault_rows.push((entry.e, entry.seq, kind, true, None, active));
                    for direction in [Direction::Rx, Direction::Tx] {
                        for stream in 0..2 {
                            book(&mut lines, line_key(direction, stream), entry, Kind::End { abort: false });
                        }
                    }
                    lost_device = true;
                } else {
                    let selected = active.first().map(|(stream, _)| *stream);
                    fault_rows.push((entry.e, entry.seq, kind, selected.is_some(), selected, active));
                }
            }
            Event::Op(index) => {
                let op = ops[index];
                if loss_at.is_some_and(|loss| entry.e >= loss) {
                    record.rejected += 1;
                    continue;
                }
                match op {
                    LayoutOp::Cold { direction, change: LayoutChange::Rate { stream, rate }, .. } => {
                        record.cold.push((format!("radio.{}.s{stream}.sample_rate_hz", side(direction)), rate as f64, entry.e))
                    }
                    LayoutOp::Cold { direction, change: LayoutChange::Membership { channel, stream }, .. } => {
                        record.cold.push((format!("radio.{}.c{channel}.stream", side(direction)), stream as f64, entry.e))
                    }
                    LayoutOp::Timed { direction, channel, gain_db, .. } => record.timed.push((format!("radio.{}.c{channel}.gain_db", side(direction)), gain_db, entry.e)),
                    LayoutOp::Stop { .. } | LayoutOp::StartRx { .. } => {}
                }
            }
        }
    }

    let gap = if profile == "ideal" { 0 } else { 50 * MS };
    let mut injected = BTreeMap::new();
    for stream in 0..2 {
        let segments = &lines.line(line_key(Direction::Rx, stream)).unwrap().plan;
        for segment in segments.iter().filter(|segment| !segment.config.channels.is_empty()) {
            let clock_index = record.rx_clocks.iter().filter(|(number, _, _)| *number == stream).count();
            let faults: Vec<_> = fault_rows
                .iter()
                .filter_map(|(at, seq, kind, applied, selected, active)| {
                    if *applied && active.contains(&(stream, segment.by)) { Some((*at, *seq, if *kind != Fault::Loss && *selected == Some(stream) { Some(*kind) } else { None })) } else { None }
                })
                .collect();
            let mut blocks = Vec::new();
            receive(stream, clock_index, segment, &faults, gap, &mut blocks, &mut injected);
            if !blocks.is_empty() {
                record.rx_clocks.extend(clocks(stream, std::slice::from_ref(segment)));
                record.blocks.extend(blocks);
            }
        }
    }
    for stream in 0..2 {
        let segments: Vec<_> = lines.line(line_key(Direction::Tx, stream)).unwrap().plan.iter().filter(|segment| !segment.config.channels.is_empty()).copied().collect();
        record.tx_clocks.extend(clocks(stream, &segments));
    }
    record.faults = fault_rows
        .into_iter()
        .map(|(_, seq, kind, applied, _, _)| {
            let lost = if applied { injected.get(&seq).copied().unwrap_or(0) } else { 0 };
            (name(kind).to_owned(), applied, lost)
        })
        .collect();
    record
}

/// MR-12's blocks of one receive segment up to the horizon, cut by its faults as MR-20,
/// MR-21 and MR-22 say; records each fault's `k_g − k_f` by its arrival order.
fn receive(stream: u32, clock: usize, segment: &Segment, faults: &[(i64, u64, Option<Fault>)], gap: i64, blocks: &mut Vec<BlockRecord>, injected: &mut BTreeMap<u64, u64>) {
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
            blocks.push((stream, clock, next, (stop - next) as u32, segment.config.channels.count() as u16, flags, lost));
            (next, flags, lost) = (stop, BlockFlags::NONE, None);
            continue;
        }
        let Some((f, seq, fault)) = faults.next() else { return };
        let kf = k(*f).max(next);
        let Some(fault) = fault else {
            next = kf;
            continue;
        };
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

/// Each stream's SampleClocks as `(stream, origin, ended)`.
fn clocks(stream: u32, segments: &[Segment]) -> Vec<(u32, i64, Option<i64>)> {
    segments.iter().map(|s| (stream, s.origin, s.cut.map(|k| s.instant(k).unwrap()))).collect()
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
    fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
        self.0.lock().unwrap().pop_front().map(|action| {
            let node = node_of(&action);
            ezsdr_kernel::module_api::Dispatched::new(ActionId(1), action, node)
        })
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

fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}

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

fn node_src(path: &str) -> EventSource {
    EventSource::Node { node: ResourceId::parse(path).unwrap() }
}

/// Runs the layout operations at their round instants, then steps to the horizon.
fn run(profile: &str, sequence: &LayoutSequence) -> Record {
    let module = ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(2, 0, 0) };
    let (version, rx, tx) = if profile == "x3xx" {
        (Version::new(0, 1, 0), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")]), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")]))
    } else {
        (Version::new(2, 0, 0), Value::from(2), Value::from(2))
    };
    let selector = BTreeMap::from([(Ident::parse("id").unwrap(), Value::from("mock".to_owned())), (Ident::parse("rx").unwrap(), rx), (Ident::parse("tx").unwrap(), tx)]);
    let binding = Binding { module: module.clone(), selector, profile: Some(ProfileRef { name: profile.to_owned(), version }), feed: None };
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
    let pairs: Vec<_> = ["mock", "mock/rx/s0", "mock/rx/s1", "mock/tx/s0", "mock/tx/s1"].iter().flat_map(|s| all.iter().map(move |k| (node_src(s), k.clone()))).collect();
    let events = Arc::new(EventCollector::new(&pairs, &all, 4096, &Policy::default()));
    let actions = Arc::new(Queue::default());
    let links = [Arc::new(Headers::default()), Arc::new(Headers::default())];
    let faults: Vec<_> = sequence.faults.iter().map(|(at, fault)| serde_json::json!({ "at_ns": at, "fault": name(*fault), "target": "radio" })).collect();
    let constraints = [("radio.rx.c0.stream", Scalar::from(0)), ("radio.tx.c0.stream", Scalar::from(0))].map(|(name, value)| (key(name), Constraint::Eq { value }));
    let ctx = {
        let mut ctx = PrepareContext::testing(EventSource::Node { node: rid("mock") }, auth.clone(), clocks.clone(), events, actions.clone(), Arc::new(NoSubmitter));
        ctx.run = RunId::from_string("timeline".to_owned());
        ctx.environment = Arc::new(BTreeMap::from([(Namespace::parse("sim.faults").unwrap(), serde_json::json!(faults))]));
        ctx.inputs = Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new());
        ctx.links = (0..2).map(|stream| AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse(&format!("rx{stream}")).unwrap(), Endpoint::StreamOut(links[stream].clone()))).collect();
        ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
        ctx
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
                LayoutOp::Stop { device: true, .. } => Action::Stop { target: Some(tgt("mock")) },
                LayoutOp::Stop { device: false, stream: Some(stream) } => Action::Stop { target: Some(tgt(&format!("mock/rx/s{stream}"))) },
                LayoutOp::Stop { device: false, stream: None } => unreachable!(),
                LayoutOp::StartRx { stream } => Action::Command { target: tgt(&format!("mock/rx/s{stream}")), verb: Ident::parse("start_rx").unwrap(), params: BTreeMap::new(), at: None },
                LayoutOp::Cold { direction, change: LayoutChange::Rate { stream, rate }, at_ns } => Action::UpdateParameter {
                    target: tgt("mock"),
                    key: key(&format!("radio.{}.s{stream}.sample_rate_hz", side(direction))),
                    value: Value::num(rate as f64).unwrap(),
                    class: UpdateClass::Cold,
                    at: at(at_ns),
                },
                LayoutOp::Cold { direction, change: LayoutChange::Membership { channel, stream }, at_ns } => Action::UpdateParameter {
                    target: tgt("mock"),
                    key: key(&format!("radio.{}.c{channel}.stream", side(direction))),
                    value: Value::from(stream),
                    class: UpdateClass::Cold,
                    at: at(at_ns),
                },
                LayoutOp::Timed { direction, channel, gain_db, at_ns } => Action::UpdateParameter {
                    target: tgt("mock"),
                    key: key(&format!("radio.{}.c{channel}.gain_db", side(direction))),
                    value: Value::num(gain_db).unwrap(),
                    class: UpdateClass::HardwareTimed,
                    at: at(at_ns),
                },
            });
        }
        step(&mut mock, T0 + round);
    }
    step(&mut mock, HORIZON);
    let records = clocks.sample_clock_records();
    let section = |name: &str| mock.instance().sections[&Namespace::parse(&format!("ezsdr.radio.mock.{name}")).unwrap()].clone();
    let rows = |name: &str| section(name).as_array().unwrap().clone();
    let applied: Vec<_> = rows("applied").iter().map(|r| (r["key"].as_str().unwrap().to_owned(), r["value"].as_f64().unwrap(), r["at"]["ticks"].as_i64().unwrap())).collect();
    let clocks_of = |direction: &str, stream: u32| -> Vec<(u32, i64, Option<i64>)> {
        let name = format!("mock/{direction}/s{stream}");
        records.iter().filter(|r| r.stream == rid(&name)).map(|r| (stream, r.origin.ticks_in(ROOT).unwrap(), r.ended_at.map(|t| t.ticks_in(ROOT).unwrap()))).collect()
    };
    let mut rx_clocks = Vec::new();
    let mut tx_clocks = Vec::new();
    let mut blocks = Vec::new();
    for stream in 0..2 {
        let name = format!("mock/rx/s{stream}");
        let domains: Vec<_> = records.iter().filter(|r| r.stream == rid(&name)).map(|r| r.domain).collect();
        rx_clocks.extend(clocks_of("rx", stream));
        blocks.extend(links[stream as usize].0.lock().unwrap().iter().map(|h| {
            let clock = domains.iter().position(|d| *d == h.first_sample_time.domain()).unwrap();
            (stream, clock, h.first_sample_time.ticks_in(h.first_sample_time.domain()).unwrap(), h.len, h.channels, h.flags, h.lost)
        }));
        tx_clocks.extend(clocks_of("tx", stream));
    }
    Record {
        rx_clocks,
        tx_clocks,
        blocks,
        cold: applied.iter().filter(|r| !r.0.ends_with("gain_db")).cloned().collect(),
        timed: applied.into_iter().filter(|r| r.0.ends_with("gain_db")).collect(),
        rejected: rows("rejected").len(),
        faults: rows("faults").iter().map(|r| (r["fault"].as_str().unwrap().to_owned(), r["applied"].as_bool().unwrap(), r["lost"].as_u64().unwrap())).collect(),
        error,
    }
}
