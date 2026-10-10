//! VH-8's third layer (spec 22): uhd-rx carrying out the plans `ezsdr_radio::timeline` makes
//! of generated sequences, on synthetic packets and the `ManualTimeAuthority` rig, against
//! those plans and RM-16's floor. uhd-control does not run: the harness books each command
//! into the receive timeline uhd-rx shares, as uhd-control does, a seeded latency after its
//! receipt (maintenance note 23); uhd-rx books its refusals, the thread that finds the device
//! lost the loss, and `Provider::stop` the stream's end, at once. A scripted device stands for
//! the X3x0's receive side: a timed start queued, an untimed stop at once with what it
//! produced before still delivered, a queued start that a stop does not cancel (#56), one
//! packet a request, delivered at the instant after its last sample plus a latency, and the
//! sequence's overruns, sequence errors and loss. It moves the rig's time while a receive call
//! waits, booking on the way, so that a booking can come during a wait. Every field is gated.

use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use std::time::Duration as Wall;

use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::{Fidelity, StopMode};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, BlockRef, DataLink, Direction, DropCarry, PublishOutcome};
use ezsdr_kernel::time::{ManualTimeAuthority, Rational, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::timeline::{Config, Item, Kind, Line, Segment, Stream, plan};
use generator::{Change, Fault, Op, Sequence, Terms};

use super::{Rx, RxCmd, Turn};
use super::super::control::ColdConfig;
use super::super::core::{Core, lock};
use crate::device::{Applied, Device, DeviceError, Dir, FakeConfig, FakeDevice, FakeFault, Iq, RxRecv, Settings, TxReport};
use crate::profile::DELIVERY_ALLOWANCE_NS;

use super::super::generator;

/// T0, 2 s on the 200 MHz root.
const T0: i64 = 400_000_000;
/// Root ticks per millisecond.
const MS: i64 = 200_000;
/// What an overrun loses, as MockRadio's `x310-like` (layer 1).
const GAP: i64 = 50 * MS;
/// The arrival order of `Provider::stop`, after every command of a sequence.
const STOP_SEQ: u64 = 1_000;

/// SplitMix64 of `seed` and `salt`, below `n`: layer 3's own draws, so that the shared
/// generator's sequences stay those of layers 1 and 2.
fn draw(seed: u64, salt: u64, n: u64) -> u64 {
    let mut z = seed.wrapping_add(salt.wrapping_mul(0x9E37_79B9_7F4A_7C15)).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) % n
}

/// What layer 3 adds to a sequence: the packets' latency, uhd-control's latency from a
/// command's receipt to its booking, `block_len` (the scripted device's packet is the request), `Provider::stop`, a
/// rate the device refuses once (UR-12), and the horizon the rig runs to.
#[derive(Clone, Copy, Debug)]
struct Extra {
    latency: i64,
    lag: i64,
    block: i64,
    stop: Option<(i64, StopMode)>,
    refuse: Option<u64>,
    /// Past every origin and cut, so that every cut is delivered and every origin passed.
    horizon: i64,
}

fn extra(seed: u64, sequence: &Sequence) -> Extra {
    let latency = [0, MS, 3 * MS][draw(seed, 1, 3) as usize];
    let at = T0 + 20 * draw(seed, 3, 4_500_001) as i64;
    let stop = match draw(seed, 2, 4) {
        0 => Some((at, StopMode::Orderly)),
        1 => Some((at, StopMode::Abort)),
        _ => None,
    };
    let rate = sequence.ops().find_map(|(_, op)| match op {
        Op::Cold { direction: Direction::Rx, change: Change::Rate(rate), .. } => Some(*rate),
        _ => None,
    });
    let block = if draw(seed, 5, 3) == 0 { 65_536 } else { 2_000 };
    let lag = MS * draw(seed, 6, 3) as i64;
    // Past every origin and cut the sequence's commands plan, by two receive calls at the
    // lowest rate and the allowances.
    let segments = plan(&terms(block).stream, &sequence.items(&terms(block))).unwrap();
    let last = segments.iter().map(|s| s.cut.map_or(s.origin, |cut| s.instant(cut).unwrap())).max().unwrap_or(T0);
    let horizon = last.max(T0 + 90_000_000) + 20 * MS + 2 * block * 500;
    Extra { latency, lag, block, stop, refuse: rate.filter(|_| draw(seed, 4, 2) == 0), horizon }
}

/// This Module's timeline terms for the receive stream on the rig: the start lead, one
/// receive call of `block_len` (the scripted device's packet is the request), the delivery
/// allowance (UR-25, VH-4).
fn terms(block: i64) -> Terms {
    let description = crate::profile::Profile::X310Ubx.description(block as u32);
    let ticks = |ns: i64| (ns + 4) / 5;
    let config = Config { channels: 1, ratio: Rational::new(200, 1).unwrap() };
    let lead = ticks(description.timing.start_lead_ns);
    let stream = Stream { direction: Direction::Rx, origin: T0, config, lead, call: block, allowance: ticks(DELIVERY_ALLOWANCE_NS) };
    Terms { t0: T0, root_hz: 200_000_000, timed_lead: 0, stream }
}

fn rate_of(config: Config) -> f64 {
    200e6 * config.ratio.den() as f64 / config.ratio.num() as f64
}

// ---------------------------------------------------------------- the comparison

/// A block as layer 3 compares it: `(first, len, flags, lost)` on its segment.
type Block = (i64, u32, BlockFlags, Option<u64>);

#[test]
fn rm_26_the_timeline_against_uhd_rx() {
    // `EZSDR_TIMELINE_SEED=<seed>` prints that sequence's run.
    let shown: Option<u64> = std::env::var("EZSDR_TIMELINE_SEED").ok().and_then(|seed| seed.parse().ok());
    let mut failed = Vec::new();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for seed in 0..1_000 {
        let sequence = generator::sequence(seed, &generator::RATES);
        let extra = extra(seed, &sequence);
        let observed = run(&sequence, &extra);
        let (problems, floored) = compare(&sequence, &extra, &observed);
        if shown == Some(seed) {
            eprintln!("seed {seed}: {sequence:?}\n{extra:?}\nobserved {observed:#?}\nproblems {problems:#?}");
        }
        for (what, counted) in [
            ("stop before a queued start", observed.dropped > 0),
            ("refusal", !observed.refused.is_empty()),
            ("abort", observed.abort),
            ("orderly stop", extra.stop.is_some_and(|(_, mode)| mode == StopMode::Orderly) && !observed.lost),
            ("loss", observed.lost),
            ("fault", observed.blocks.iter().any(|(_, (_, _, flags, _))| *flags != BlockFlags::NONE)),
            ("booking during a wait", observed.straddled),
            ("a cut floored at what was delivered", floored),
            ("block_len 65 536", extra.block == 65_536),
            ("items pruned", observed.pruned),
        ] {
            *seen.entry(what).or_default() += usize::from(counted);
        }
        if !problems.is_empty() {
            failed.push(format!("seed {seed}, {:?}: {problems:?}", sequence.class));
        }
    }
    eprintln!("layer 3: 1000 sequences; cases {seen:?}");
    assert!(failed.is_empty(), "{} sequences differ (EZSDR_TIMELINE_SEED=<seed> shows one), the first: {:#?}", failed.len(), &failed[..failed.len().min(20)]);
}

/// What uhd-rx left: the receive clocks as `(origin, ended)`, its blocks by clock, the
/// device's receive calls as `(instant, call)`, its `rx_stop` rows as `(at, issued)`, the
/// refusals uhd-rx booked, and what the scripted device found wrong as it happened.
#[derive(Debug, Default)]
struct Observed {
    clocks: Vec<(i64, Option<i64>)>,
    blocks: Vec<(usize, Block)>,
    calls: Vec<(i64, String)>,
    stops: Vec<(i64, i64)>,
    refused: Vec<u64>,
    /// Configurations the device was asked for at the refused rate.
    refusable: usize,
    rejected: usize,
    late: usize,
    lost: bool,
    abort: bool,
    /// The device still streams at the end, unstopped.
    streaming: bool,
    /// Timed starts of segments the plan dropped before their origin (VH-6).
    dropped: usize,
    /// A command was booked while a receive call waited for the packet it returned.
    straddled: bool,
    /// uhd-rx pruned the timeline's items (#63).
    pruned: bool,
    violations: Vec<String>,
}

/// The timeline's account of what uhd-rx does with a sequence's plans, compared with what it
/// did; the problems found, and whether RM-16's floor moved a cut.
fn compare(sequence: &Sequence, extra: &Extra, observed: &Observed) -> (Vec<String>, bool) {
    let terms = terms(extra.block);
    let mut problems = observed.violations.clone();
    let items = sequence.items(&terms);
    let loss = items.iter().find(|item| matches!(item.kind, Kind::End { .. })).map(|item| item.e);
    // What is booked: every command received before `Provider::stop`, and its end of the
    // stream, and booked before a loss, and the loss; a refused change marked as such (RM-25).
    let stop = extra.stop.filter(|(at, _)| loss.is_none_or(|loss| *at < loss));
    let mut booked: Vec<Item> = items.iter()
        .filter(|item| stop.is_none_or(|(at, _)| item.ready <= at && !matches!(item.kind, Kind::End { .. })))
        .filter(|item| matches!(item.kind, Kind::End { .. }) || loss.is_none_or(|loss| item.ready + extra.lag < loss))
        .map(|item| Item { refused: observed.refused.contains(&item.seq), ..*item })
        .collect();
    if let Some((at, mode)) = stop {
        booked.push(Item { e: at, seq: STOP_SEQ, ready: at, delivered: None, refused: false, kind: Kind::End { abort: mode == StopMode::Abort } });
    }
    booked.sort_by_key(|item| (item.ready, item.seq));
    // Every segment any plan had, in the order of their origins, as the device would run it
    // from its origin; its cut is settled below.
    let mut line = Line::new(terms.stream);
    let mut plans = vec![(i64::MIN, line.plan.clone())];
    for item in &booked {
        line.book(*item).unwrap();
        // uhd-rx follows the last of the plans booked at one instant; a loss and
        // `Provider::stop` are booked at their instants.
        let arrival = if matches!(item.kind, Kind::End { .. }) { item.ready } else { item.ready + extra.lag };
        if plans.last().is_some_and(|(t, _)| *t == arrival) {
            plans.pop();
        }
        plans.push((arrival, line.plan.clone()));
    }
    let key = |s: &Segment| (s.origin, s.config.channels, s.config.ratio.num(), s.config.ratio.den());
    let mut all: Vec<Segment> = Vec::new();
    for segment in plans.iter().flat_map(|(_, plan)| plan) {
        if !all.iter().any(|s| key(s) == key(segment)) {
            all.push(Segment { cut: None, ..*segment });
        }
    }
    all.sort_by_key(|s| s.origin);
    // Each overrun and sequence error on the segment the device runs when it fires: the one
    // begun last by then. One past that segment's cut leaves its blocks as they are.
    let mut hits: Vec<Vec<(i64, Fault)>> = vec![Vec::new(); all.len()];
    for (at, fault) in sequence.faults.iter().filter(|(_, fault)| *fault != Fault::Loss) {
        let f = terms.t0 + terms.ticks(*at);
        if let Some(index) = all.iter().rposition(|s| s.origin <= f) {
            hits[index].push((f, *fault));
        }
    }
    for hits in &mut hits {
        hits.sort_by_key(|(f, _)| *f);
    }
    // RM-16's floor: a command is booked `lag` after its receipt, and the cut uhd-rx takes is
    // the plan's — at its origin for a segment it has begun that the plan no longer has — but
    // never before the first sample it had delivered then; an end moves only earlier. When
    // uhd-rx begins a segment is its own affair: the device's log says (its timed start).
    let begun = |segment: &Segment, by: i64| observed.calls.iter().any(|(at, call)| *at < by && call.strip_prefix("rx_start ") == Some(segment.origin.to_string().as_str()));
    let delivered = |index: usize, by: i64| -> i64 {
        let bound = loss.map_or(by, |loss| loss.min(by));
        blocks(&all[index], &hits[index], extra).iter()
            .take_while(|(first, len, _, _)| all[index].instant(first + i64::from(*len)).unwrap() + extra.latency < bound)
            .last().map_or(0, |(first, len, _, _)| first + i64::from(*len))
    };
    let mut floor = false;
    let mut cuts: Vec<Option<i64>> = vec![None; all.len()];
    let mut seen = vec![false; all.len()];
    for (arrival, plan) in &plans {
        for (index, segment) in all.iter().enumerate() {
            let planned = plan.iter().find(|s| key(s) == key(segment)).map(|s| s.cut);
            let cut = match planned {
                Some(Some(cut)) => cut,
                Some(None) => {
                    seen[index] = true;
                    continue;
                }
                None if seen[index] && begun(segment, *arrival) => 0,
                None => {
                    // Replanned away before uhd-rx began it: it has no cut of its own.
                    cuts[index] = None;
                    continue;
                }
            };
            seen[index] = true;
            let floored = cut.max(delivered(index, *arrival));
            floor |= floored > cut;
            cuts[index] = Some(cuts[index].map_or(floored, |cut| cut.min(floored)));
        }
    }
    // A segment the last plan does not have, never begun, never ran.
    let last = &plans.last().expect("a plan").1;
    let kept: Vec<usize> = (0..all.len())
        .filter(|index| cuts[*index] != Some(0) && (cuts[*index].is_some() || last.iter().any(|s| key(s) == key(&all[*index]))))
        .collect();
    let segments: Vec<Segment> = kept.iter().map(|index| Segment { cut: cuts[*index], ..all[*index] }).collect();
    let hits: Vec<Vec<(i64, Fault)>> = kept.iter().map(|index| hits[*index].clone()).collect();
    for segment in &segments {
        if segment.origin >= extra.horizon - 10 * MS || segment.cut.is_some_and(|cut| segment.instant(cut).unwrap() >= extra.horizon - 10 * MS) {
            problems.push(format!("the sequence reaches past the horizon: {segment:?}"));
        }
    }
    // Every segment's blocks to its cut or the horizon; after a loss or an abort the segment
    // being delivered has a prefix of them and none after it has any.
    let abort_at = stop.filter(|(_, mode)| *mode == StopMode::Abort).map(|(at, _)| at);
    let cutoff = loss.is_some() || abort_at.is_some();
    // When the device was told to stop each stream, by its origin.
    let mut stopped: BTreeMap<i64, i64> = BTreeMap::new();
    let mut current = None;
    for (at, call) in &observed.calls {
        if let Some(origin) = call.strip_prefix("rx_start ") {
            current = origin.parse().ok();
        } else if let Some(origin) = current.take().filter(|_| call.starts_with("rx_stop ")) {
            stopped.insert(origin, *at);
        }
    }
    let mut clocks = Vec::new();
    let mut cut_off = false;
    for (index, segment) in segments.iter().enumerate() {
        let full = blocks(segment, &hits[index], extra);
        let got: Vec<Block> = observed.clocks.iter().position(|(origin, _)| *origin == segment.origin)
            .map(|clock| observed.blocks.iter().filter(|(c, _)| *c == clock).map(|(_, block)| *block).collect())
            .unwrap_or_default();
        let whole = !cut_off && got == full;
        if !whole && !(cutoff && (got.is_empty() && cut_off || !cut_off && full.starts_with(&got))) {
            let from = got.iter().zip(&full).position(|(a, b)| a != b).unwrap_or(got.len().min(full.len()));
            let near = |blocks: &[Block]| blocks[from.saturating_sub(1).min(blocks.len())..(from + 2).min(blocks.len())].to_vec();
            problems.push(format!("segment {index} at {}: {} blocks, the plan's {}, from {from}: {:?}, the plan's {:?}", segment.origin, got.len(), full.len(), near(&got), near(&full)));
        }
        cut_off |= !whole;
        if let Some(last) = got.last() {
            let mut end = segment.cut.map(|cut| segment.instant(cut).unwrap());
            if abort_at.is_some_and(|abort| stopped.get(&segment.origin).is_none_or(|at| *at >= abort)) {
                // RM-16: an abort ends the stream it finds at the first sample not yet delivered.
                end = Some(segment.instant(last.0 + i64::from(last.1)).unwrap());
            }
            clocks.push((segment.origin, end));
        }
    }
    if observed.clocks != clocks {
        problems.push(format!("clocks {:?}, the plan's {clocks:?}", observed.clocks));
    }
    // The device's stream commands: a timed start ahead of each origin, checked as it was
    // issued; untimed stops, one after each start, at or after its origin unless an abort
    // stopped it at once (VH-6); a streamer reopened only for another channel count.
    let (mut started, mut opened): (Option<i64>, usize) = (None, 0);
    for (at, call) in &observed.calls {
        let value = call.split(' ').nth(1).unwrap_or_default();
        if call.starts_with("rx_start ") {
            if started.is_some() {
                problems.push(format!("{call} at {at}: the stream started before is not stopped"));
            }
            started = value.parse().ok();
        } else if call.starts_with("rx_stop ") {
            match started.take() {
                Some(origin) if *at < origin && abort_at.is_none_or(|abort| *at < abort) => problems.push(format!("{call} at {at}: before the origin {origin} (VH-6)")),
                Some(_) => {}
                None => problems.push(format!("{call} at {at}: no stream to stop")),
            }
            if value != "now" {
                problems.push(format!("a timed stop: {call}"));
            }
        } else if call.starts_with("apply_refused ") {
            // UR-12's refusal: uhd-rx reopens the streamer for the next segment.
            opened = 0;
        } else if call.starts_with("rx_open ") {
            let channels: usize = value.parse().unwrap();
            if channels == opened {
                problems.push(format!("{call} at {at}: reopened for the same channel count"));
            }
            opened = channels;
        }
    }
    let stopped_calls = observed.calls.iter().filter(|(_, call)| call.starts_with("rx_stop ")).count();
    if observed.stops.len() != stopped_calls {
        problems.push(format!("{} rx_stop rows for {stopped_calls} stops", observed.stops.len()));
    }
    if let Some((at, issued)) = observed.stops.iter().find(|(at, issued)| issued < at && abort_at.is_none_or(|abort| *issued < abort)) {
        problems.push(format!("an rx_stop row issued at {issued}, before its cut {at}"));
    }
    let open = clocks.last().is_some_and(|(_, end)| end.is_none());
    if !observed.lost && abort_at.is_none() && observed.streaming != open {
        problems.push(format!("the device streams at the end: {}, the plan's last segment open: {open}", observed.streaming));
    }
    // RM-25, UR-12: the first configuration at the refused rate is refused, once.
    let refusals = usize::from(observed.refusable > 0);
    if observed.refused.len() != refusals || observed.rejected != refusals {
        problems.push(format!("{} refusals, {} rejected, for {refusals}", observed.refused.len(), observed.rejected));
    }
    if observed.late > 0 {
        problems.push(format!("{} late starts", observed.late));
    }
    (problems, floor)
}

/// One segment's blocks: a packet of `block_len` samples a request, the last before the cut
/// ending there, one cut short at a fault's sample, and after it the gap MockRadio's
/// `x310-like` has (layer 1); up to the horizon.
fn blocks(segment: &Segment, hits: &[(i64, Fault)], extra: &Extra) -> Vec<Block> {
    let k = |t: i64| segment.sample_at_or_after(t).unwrap();
    let end = segment.cut.unwrap_or(i64::MAX);
    let mut hits = hits.iter().peekable();
    let (mut next, mut flags, mut lost) = (0, BlockFlags::NONE, None::<u64>);
    let mut out = Vec::new();
    loop {
        let fault_k = hits.peek().map(|(f, _)| k(*f));
        let stop = (next + extra.block).min(end).min(fault_k.unwrap_or(i64::MAX));
        if stop > next {
            if segment.instant(stop).unwrap() + extra.latency > extra.horizon {
                return out;
            }
            out.push((next, (stop - next) as u32, flags, lost));
            (next, flags, lost) = (stop, BlockFlags::NONE, None);
            continue;
        }
        let Some((f, fault)) = hits.next() else { return out };
        let kf = k(*f).max(next);
        let (kg, flag) = match fault {
            Fault::Overrun => (k(f + GAP).max(kf), BlockFlags::RESTARTED),
            _ => (kf + extra.block, BlockFlags::SEQ_DISCONTINUITY),
        };
        flags = flags | BlockFlags::GAP_BEFORE | flag;
        lost = Some(lost.unwrap_or(0) + (kg - kf) as u64);
        next = kg.min(end);
    }
}

// ---------------------------------------------------------------- the run

/// Keeps every block's header.
#[derive(Default)]
struct Headers(Mutex<Vec<BlockHeader>>);

impl DataLink for Headers {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        lock(&self.0).push(block.header().clone());
        PublishOutcome::Accepted
    }
    fn receive(&self) -> Option<(BlockRef, DropCarry)> { None }
    fn drops(&self) -> u64 { 0 }
    fn take_drop_carry(&self) -> DropCarry { DropCarry::default() }
    fn policy(&self) -> BackPressure { BackPressure::DropOldest }
}

/// What happens at an instant, besides the device's packets.
enum Ev {
    /// uhd-control books a command received `lag` earlier.
    Book(Item),
    /// The device is lost.
    Loss,
    /// `Provider::stop`: the stream's end booked (UR-26).
    Stop(StopMode),
    /// `Provider::stop`'s shutdown reaches uhd-rx, `lag` later.
    Shutdown(StopMode),
}

/// A stream the scripted device runs: started at `origin`, `n` root ticks a sample, its next
/// sample's tick, and once stopped the stop's instant, before which it still delivers.
#[derive(Debug)]
struct Run {
    origin: i64,
    n: i64,
    next: i64,
    end: Option<i64>,
}

/// The rig's time, what uhd-control does, and the scripted device's receive side.
struct World {
    time: Option<Arc<ManualTimeAuthority>>,
    root: ClockDomainId,
    now: i64,
    events: BTreeMap<(i64, u64), Ev>,
    order: u64,
    to_rx: Sender<RxCmd>,
    core: Option<Arc<Core>>,
    /// `Provider::stop`'s instant, once it has begun: commands received after it are not booked.
    stopped: Option<i64>,
    /// How many commands were booked.
    booked: usize,
    /// The commands whose segments uhd-rx booked refused.
    refused: Vec<u64>,
    abort: bool,
    rate: f64,
    runs: VecDeque<Run>,
    lost: bool,
    faults: Vec<(i64, Fault, bool)>,
    latency: i64,
    lag: i64,
    block: i64,
    horizon: i64,
    done: bool,
    calls: Vec<(i64, String)>,
    straddled: bool,
    violations: Vec<String>,
}

impl World {
    fn at(&mut self, t: i64, ev: Ev) {
        self.order += 1;
        self.events.insert((t, self.order), ev);
    }

    fn set_now(&mut self, t: i64) {
        if t > self.now {
            self.now = t;
            self.time.as_ref().expect("the rig's time").advance_to(TimePoint::new(self.root, t)).unwrap();
        }
    }

    /// Moves time to `to`, carrying out what happens on the way; false at a loss, where time
    /// stops.
    fn advance(&mut self, to: i64) -> bool {
        while let Some(entry) = self.events.first_entry() {
            let (t, _) = *entry.key();
            if t > to {
                break;
            }
            let ev = entry.remove();
            self.set_now(t);
            self.fire(ev);
            if self.lost {
                return false;
            }
        }
        self.set_now(to);
        true
    }

    fn fire(&mut self, ev: Ev) {
        let core = self.core.clone().expect("the rig's core");
        match ev {
            // A loss stops time, so nothing is booked after it.
            Ev::Book(item) if self.stopped.is_none_or(|at| item.ready <= at) => {
                let mut streams = lock(&core.streams);
                if matches!(item.kind, Kind::Cold(_) | Kind::Start) {
                    streams.configs[Dir::Rx as usize].insert(item.seq, (Arc::new(ColdConfig::new(Settings::default())), false));
                }
                assert!(streams.book(&core, Dir::Rx, item), "booked");
                self.booked += 1;
            }
            Ev::Loss => self.lost = true,
            Ev::Stop(mode) if self.stopped.is_none() => {
                self.stopped = Some(self.now);
                self.abort = mode == StopMode::Abort;
                // As `Provider::stop` books the end, with `STOP_SEQ` its arrival order.
                let mut streams = lock(&core.streams);
                streams.seq = STOP_SEQ - 1;
                streams.stop = Some((self.now, mode));
                streams.end(&core, self.now, self.abort, false);
                drop(streams);
                if self.lag == 0 {
                    self.fire(Ev::Shutdown(mode));
                } else {
                    let at = self.now + self.lag;
                    self.at(at, Ev::Shutdown(mode));
                }
            }
            Ev::Shutdown(mode) => {
                let _ = self.to_rx.send(RxCmd::Shutdown(mode));
            }
            Ev::Book(_) | Ev::Stop(_) => {}
        }
    }

    fn call(&mut self, call: String) {
        self.calls.push((self.now, call));
    }
}

/// The X3x0's receive side as uhd-rx sees it, everything else the FakeDevice's.
struct Scripted {
    inner: FakeDevice,
    world: Arc<Mutex<World>>,
}

impl Device for Scripted {
    fn describe(&self) -> serde_json::Value { self.inner.describe() }
    fn fidelity(&self) -> Fidelity { self.inner.fidelity() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> { self.inner.front_end(dir, chan) }
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> { self.inner.set_sources(clock, time) }
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> { self.inner.set_time_zero(at_next_pps) }
    fn time_now(&self) -> Result<i64, DeviceError> { Ok(lock(&self.world).now) }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> { self.inner.ref_locked() }

    /// The FakeDevice's, which refuses a rate once when asked to (`WrongRate`); the receive
    /// rate it applied starts the next stream.
    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> {
        let applied = self.inner.apply(dir, chan, s, at)?;
        if dir == Dir::Rx && chan == 0 && at.is_none() && s.rate.is_some() {
            let mut world = lock(&self.world);
            world.rate = applied.rate;
            if s.rate != Some(applied.rate) {
                world.call(format!("apply_refused {}", s.rate.unwrap_or_default()));
            }
        }
        Ok(applied)
    }

    fn rx_open(&self, channels: usize) -> Result<(), DeviceError> {
        let mut world = lock(&self.world);
        world.call(format!("rx_open {channels}"));
        // A new streamer: nothing the old one had in flight is delivered.
        world.runs.retain(|run| run.end.is_none());
        Ok(())
    }

    fn rx_start(&self, at: i64) -> Result<(), DeviceError> {
        let mut world = lock(&self.world);
        if world.lost {
            return Err(DeviceError { lost: true, message: "scripted: the device is gone".to_owned() });
        }
        world.call(format!("rx_start {at}"));
        let now = world.now;
        let core = world.core.clone().expect("the rig's core");
        let planned = lock(&core.streams).lines[Dir::Rx as usize].as_ref().and_then(|line| line.plan.iter().find(|segment| segment.origin == at).copied());
        let mut problems = Vec::new();
        if at <= now {
            problems.push(format!("rx_start {at} at {now}: not ahead of its origin"));
        }
        match planned {
            Some(segment) if (rate_of(segment.config) - world.rate).abs() > 0.5 => problems.push(format!("rx_start {at}: the device runs {} S/s, the plan {}", world.rate, rate_of(segment.config))),
            Some(_) => {}
            None => problems.push(format!("rx_start {at}: no segment of the plan uhd-rx has begins there")),
        }
        world.violations.extend(problems);
        let n = (200e6 / world.rate).round() as i64;
        world.runs.push_back(Run { origin: at, n, next: at, end: None });
        Ok(())
    }

    fn rx_packet_samples(&self) -> usize {
        0
    }

    /// The X3x0 stops a running stream at once, whatever the stop's time (design-notes §11
    /// F1); a start still queued is not cancelled (#56).
    fn rx_stop(&self, at: Option<i64>) -> Result<(), DeviceError> {
        let mut world = lock(&self.world);
        if world.lost {
            return Err(DeviceError { lost: true, message: "scripted: the device is gone".to_owned() });
        }
        world.call(format!("rx_stop {}", at.map_or("now".to_owned(), |t| t.to_string())));
        let now = world.now;
        for run in world.runs.iter_mut().filter(|run| run.end.is_none() && run.origin <= now) {
            run.end = Some(now);
        }
        Ok(())
    }

    fn rx_recv(&self, n: usize, timeout: Wall) -> RxRecv {
        let mut world = lock(&self.world);
        let gone = || RxRecv::Failed(DeviceError { lost: true, message: "scripted: the device is gone".to_owned() });
        if world.lost {
            return gone();
        }
        let wait = (timeout.as_nanos() as i64 + 4) / 5;
        while world.runs.front().is_some_and(|run| run.end.is_some_and(|end| run.next >= end)) {
            world.runs.pop_front();
        }
        let Some(run) = world.runs.front() else {
            let to = world.now + wait;
            return if to > world.horizon {
                let horizon = world.horizon;
                world.advance(horizon);
                world.done = true;
                RxRecv::Timeout
            } else if world.advance(to) {
                RxRecv::Timeout
            } else {
                gone()
            };
        };
        let (origin, step) = (run.origin, run.n);
        let k = (run.next - origin) / step;
        let mut stop = k + n as i64;
        if let Some(end) = run.end {
            stop = stop.min((end - origin + step - 1) / step);
        }
        // The first fault on this stream not yet reported: at its sample, or cutting the
        // packet short there.
        let end = run.end;
        let fault = world.faults.iter().enumerate()
            .filter(|(_, (f, _, fired))| !fired && *f >= origin && end.is_none_or(|end| *f < end))
            .map(|(index, (f, _, _))| (((f - origin + step - 1) / step).max(k), index))
            .min()
            .map(|(kf, index)| (kf, index, world.faults[index].0, world.faults[index].1));
        if let Some((kf, index, f, fault)) = fault {
            if kf == k {
                world.faults[index].2 = true;
                let kg = match fault {
                    Fault::Overrun => ((f + GAP - origin + step - 1) / step).max(kf),
                    _ => kf + world.block,
                };
                world.runs[0].next = origin + kg * step;
                return RxRecv::Overflow { out_of_sequence: fault != Fault::Overrun };
            }
            stop = stop.min(kf);
        }
        let ready = origin + stop * step + world.latency;
        if ready > world.now + wait {
            let to = world.now + wait;
            if to > world.horizon {
                let horizon = world.horizon;
                world.advance(horizon);
                world.done = true;
                return RxRecv::Timeout;
            }
            return if world.advance(to) { RxRecv::Timeout } else { gone() };
        }
        if ready > world.horizon {
            let horizon = world.horizon;
                world.advance(horizon);
            world.done = true;
            return RxRecv::Timeout;
        }
        let booked = world.booked;
        if !world.advance(ready) {
            return gone();
        }
        world.straddled |= world.booked > booked;
        world.runs[0].next = origin + stop * step;
        let channels = world.calls.iter().rev().find_map(|(_, call)| call.strip_prefix("rx_open ")).map_or(1, |c| c.parse().unwrap());
        let samples: Vec<Vec<Iq>> = vec![vec![[0.0, 0.0]; (stop - k) as usize]; channels];
        RxRecv::Samples { first_tick: origin + k * step, samples }
    }

    fn tx_open(&self, channels: usize) -> Result<(), DeviceError> { self.inner.tx_open(channels) }
    fn tx_send(&self, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Wall) -> Result<usize, DeviceError> {
        self.inner.tx_send(samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, timeout: Wall) -> Result<Option<TxReport>, DeviceError> { self.inner.tx_async(timeout) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
}

/// Runs a sequence's receive plans on uhd-rx: each command booked `lag` after its receipt, the
/// loss booked by whichever finds the device lost, a refusal by uhd-rx, `Provider::stop` at
/// `extra`'s instant.
fn run(sequence: &Sequence, extra: &Extra) -> Observed {
    let terms = terms(extra.block);
    let link = Arc::new(Headers::default());
    let refuse = extra.refuse.map(|rate| FakeFault::WrongRate { claimed: rate as f64, applied: rate as f64 - 1_000.0, nth: 0 });
    let inner = FakeDevice::new(FakeConfig { faults: refuse.into_iter().collect(), ..FakeConfig::default() });
    let (to_rx, cmds) = channel();
    let world = Arc::new(Mutex::new(World {
        time: None,
        root: ClockDomainId::local(0),
        now: 0,
        events: BTreeMap::new(),
        order: 0,
        to_rx,
        core: None,
        stopped: None,
        booked: 0,
        refused: Vec::new(),
        abort: false,
        rate: 1e6,
        runs: VecDeque::new(),
        lost: false,
        faults: sequence.faults.iter().map(|(at, fault)| (terms.t0 + terms.ticks(*at), *fault, false)).collect(),
        latency: extra.latency,
        lag: extra.lag,
        block: extra.block,
        horizon: extra.horizon,
        done: false,
        calls: Vec::new(),
        straddled: false,
        violations: Vec::new(),
    }));
    let device = Arc::new(Scripted { inner, world: world.clone() });
    let (core, time, events) = super::super::test_support::rig_on(device.clone(), vec![link.clone()], extra.block as u32);
    let items = sequence.items(&terms);
    let loss = items.iter().find(|item| matches!(item.kind, Kind::End { .. })).copied();
    // `start` shares the stream's timeline from T0 (UR-15).
    lock(&core.streams).lines[Dir::Rx as usize] = Some(Line::new(terms.stream));
    {
        let mut w = lock(&world);
        w.time = Some(time);
        w.root = core.root;
        w.core = Some(core.clone());
        w.set_now(T0 - MS);
        // At one instant a loss comes first, then the commands in their order, then
        // `Provider::stop`.
        if let Some(loss) = loss {
            w.at(loss.e, Ev::Loss);
        }
        let mut booked: Vec<Item> = items.iter().filter(|item| !matches!(item.kind, Kind::End { .. })).copied().collect();
        booked.sort_by_key(|item| (item.ready, item.seq));
        let lag = w.lag;
        for item in booked {
            w.at(item.ready + lag, Ev::Book(item));
        }
        if let Some((at, mode)) = extra.stop {
            w.at(at, Ev::Stop(mode));
        }
    }
    // `prepare` opened the streamer for one channel, `start` started it at T0 (UR-15).
    device.rx_open(1).unwrap();
    device.rx_start(T0).unwrap();
    let handle = core.clocks.declare_sample_clock(core.id.child("rx").unwrap(), core.root, Rational::new(200, 1).unwrap()).unwrap();
    let mut rx = Rx::new(core.clone(), cmds, Some((handle, 1, T0)), None);
    for turn in 0.. {
        assert!(turn < 1_000_000, "uhd-rx makes no progress");
        let next = rx.turn();
        // uhd-control's part between uhd-rx's turns: a loss it finds on its own.
        let mut w = lock(&world);
        if w.lost && !core.is_lost() {
            drop(w);
            core.device_lost("VH-8: the sequence's loss");
            w = lock(&world);
        }
        {
            let streams = lock(&core.streams);
            let line = streams.lines[Dir::Rx as usize].as_ref().expect("the receive timeline");
            let refused: Vec<u64> = line.items.iter().filter(|item| item.refused && !w.refused.contains(&item.seq)).map(|item| item.seq).collect();
            w.refused.extend(refused);
        }
        if w.done || next == Turn::Exit {
            break;
        }
        if next == Turn::Idle {
            // Nothing changes for uhd-rx until the next event.
            match w.events.first_key_value().map(|(&(t, _), _)| t) {
                Some(t) if t <= w.horizon => {
                    w.advance(t);
                }
                _ => {
                    let horizon = w.horizon;
                    w.set_now(horizon);
                    w.done = true;
                }
            }
        }
    }
    let w = lock(&world);
    let records = core.clocks.sample_clock_records();
    let rx_clocks: Vec<_> = records.iter().filter(|r| r.stream == core.rx_id).collect();
    let headers = lock(&link.0);
    let rec = lock(&core.rec);
    let final_plan = lock(&core.streams).lines[Dir::Rx as usize].as_ref().map(|line| line.plan.clone()).unwrap_or_default();
    let lost_at = rec.timing.iter().find(|row| row["what"] == "device_lost").and_then(|row| row["at"].as_i64());
    let mut violations = w.violations.clone();
    if lost_at != loss.filter(|_| core.is_lost()).map(|loss| loss.e) {
        violations.push(format!("lost at {lost_at:?}, not at the sequence's loss"));
    }
    Observed {
        clocks: rx_clocks.iter().map(|r| (r.origin.ticks_in(r.root).unwrap(), r.ended_at.map(|t| t.ticks_in(r.root).unwrap()))).collect(),
        blocks: headers.iter().map(|h| {
            let clock = rx_clocks.iter().position(|r| r.domain == h.first_sample_time.domain()).expect("a block on a receive clock");
            (clock, (h.first_sample_time.ticks_in(rx_clocks[clock].domain).unwrap(), h.len, h.flags, h.lost))
        }).collect(),
        stops: rec.applied.iter().filter(|r| r["key"] == "rx_stop").map(|r| (r["at"]["ticks"].as_i64().unwrap(), r["issued"]["ticks"].as_i64().unwrap())).collect(),
        refused: w.refused.clone(),
        refusable: extra.refuse.map_or(0, |rate| device.inner.calls().iter().filter(|c| c.starts_with(&format!("apply rx 0 rate={rate} "))).count()),
        rejected: rec.rejected.len(),
        late: events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|e| e.kind.as_str() == kinds::LATE_COMMAND).count(),
        lost: core.is_lost(),
        abort: w.abort,
        streaming: !w.lost && w.runs.iter().any(|run| run.end.is_none()),
        dropped: w.calls.iter().filter_map(|(_, call)| call.strip_prefix("rx_start ")).filter(|at| !final_plan.iter().any(|s| s.origin.to_string() == *at)).count(),
        straddled: w.straddled,
        pruned: lock(&core.streams).lines[Dir::Rx as usize].as_ref().is_some_and(|line| line.items.len() < w.booked),
        calls: w.calls.clone(),
        violations,
    }
}
