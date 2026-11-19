//! VH-8's third layer (spec 22): uhd-rx carrying out the plans `ezsdr_radio::timeline` makes
//! of generated sequences, on synthetic packets and the `ManualTimeAuthority` rig, against
//! those plans and RM-16's floor. uhd-control does not run: the harness books each command
//! into the receive timeline uhd-rx shares, as uhd-control does, a seeded latency after its
//! receipt (maintenance note 23); uhd-rx books its refusals, the thread that finds the device
//! lost the loss, and `Provider::stop` the stream's end, at once. A scripted device stands for
//! the X3x0's receive side: a timed start queued, an untimed stop at once with what it
//! produced before still delivered and a start still queued cancelled (#67), one
//! packet a request, delivered at the instant after its last sample plus a latency, and the
//! sequence's overruns, sequence errors and loss. It moves the rig's time while a receive call
//! waits, booking on the way, so that a booking can come during a wait. Every field is gated.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use std::time::Duration as Wall;

use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::{Fidelity, StopMode};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, BlockRef, DataLink, Direction, DropCarry, PublishOutcome};
use ezsdr_kernel::time::{ManualTimeAuthority, Rational, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Line, Segment, Stream, plan};
use generator::{Fault, Leaf, Op, Sequence, Terms};

use super::super::control::ColdConfig;
use super::super::core::{Core, booked_stream, lock};
use super::{Rx, RxCmd, Turn};
use crate::HostClock;
use crate::device::{Applied, Device, DeviceError, Dir, FakeConfig, FakeDevice, FakeFault, Iq, RxRecv, Settings, TxReport};
use crate::profile::DELIVERY_ALLOWANCE_NS;
use ezsdr_radio::staging::Staging;

use super::super::generator;

/// T0, 2 s on the 200 MHz root.
const T0: i64 = 400_000_000;
/// Root ticks per millisecond.
const MS: i64 = 200_000;
/// What an overrun loses, in root ticks: the profile's `overflow_restart_gap_ns`, shared with
/// MockRadio and the hardware.
fn gap() -> i64 {
    (ezsdr_radio::x3xx::timing().overflow_restart_gap_ns + 4) / 5
}
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
    /// A restart survives the first stop after it: the START UHD queued behind the stop, cleared only by a second stop.
    survivor: bool,
}

fn extra(seed: u64, sequence: &Sequence) -> Extra {
    let latency = [0, MS, 3 * MS][draw(seed, 1, 3) as usize];
    let at = T0 + 20 * draw(seed, 3, 4_500_001) as i64;
    let stop = match draw(seed, 2, 4) {
        0 => Some((at, StopMode::Orderly)),
        1 => Some((at, StopMode::Abort)),
        _ => None,
    };
    // A configure is made by a `start_rx` only (a Run's settings change only at its start), at the stream's 1 MS/s.
    let rate = sequence.ops().any(|(_, op)| matches!(op, Op::StartRx { .. })).then_some(1_000_000);
    let block = if draw(seed, 5, 3) == 0 { 65_536 } else { 2_000 };
    let lag = MS * draw(seed, 6, 3) as i64;
    // Past every origin and cut the sequence's commands plan, by two receive calls at the
    // lowest rate and the allowances; a gap-delayed block still falls inside it when the
    // profile's gap grows.
    let segments = plan(&terms(block).stream, &sequence.items(&terms(block))).unwrap();
    let last = segments.iter().map(|s| s.cut.map_or(s.origin, |cut| s.instant(cut).unwrap())).max().unwrap_or(T0);
    let horizon = last.max(T0 + 90_000_000 + (gap() - 50 * MS)) + 20 * MS + 2 * block * 500;
    Extra { latency, lag, block, stop, refuse: rate.filter(|_| draw(seed, 4, 2) == 0), horizon, survivor: draw(seed, 7, 2) == 1 }
}

/// This Module's timeline terms for the receive stream on the rig: the start lead, one
/// receive call of `block_len` (the scripted device's packet is the request), the delivery
/// allowance (UR-25, VH-4).
fn terms(block: i64) -> Terms {
    let description = super::super::test_support::default_description(block as u32);
    let ticks = |ns: i64| (ns + 4) / 5;
    let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
    let lead = ticks(ezsdr_radio::timeline::start_lead_ns(&description.timing));
    let stream = Stream { direction: Direction::Rx, origin: T0, config, lead, call: block, allowance: ticks(DELIVERY_ALLOWANCE_NS) };
    Terms { t0: T0, root_hz: 200_000_000, stream }
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
        let sequence = generator::sequence(seed);
        let extra = extra(seed, &sequence);
        let observed = run(&sequence, &extra);
        let (problems, reach) = compare(&sequence, &extra, &observed);
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
            ("a cut floored at what was delivered", reach.floored),
            ("a cut inside an overrun's gap", reach.cut_in_gap),
            ("a block after an overrun's gap", reach.block_after_gap),
            ("a survivor cleared by a second stop", reach.survivor_cleared),
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
    // S20: the shapes #108 needed are still exercised, so a change that stops producing them cannot pass quietly.
    assert!(seen["a cut inside an overrun's gap"] >= 3, "the rig no longer produces a cut inside an overrun's gap");
    assert!(seen["a block after an overrun's gap"] > 0, "the rig no longer produces a block after an overrun's gap");
    assert!(seen["a survivor cleared by a second stop"] >= 4, "the rig no longer produces a restart that survives a stop");
    assert!(failed.is_empty(), "{} sequences differ (EZSDR_TIMELINE_SEED=<seed> shows one), the first: {:#?}", failed.len(), &failed[..failed.len().min(20)]);
}

#[test]
fn rm_26_the_coverage_shapes_come_from_the_observed_run() {
    let mut observed = Observed::default();
    assert_eq!(observed_shapes(&observed), (false, false), "an empty run shows neither shape");
    observed.blocks.push((0, (0, 2_000, BlockFlags::GAP_BEFORE, Some(10))));
    assert_eq!(observed_shapes(&observed), (false, false), "a block after a gap alone is not a restart");
    observed.blocks.push((0, (2_000, 2_000, BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, Some(10))));
    assert_eq!(observed_shapes(&observed), (false, true), "a published RESTARTED block is the block-after-gap shape");

    let cut = Observed { clocks: vec![(0, Some(100)), (200, None)], overruns: vec![(0, 50)], ..Observed::default() };
    assert_eq!(observed_shapes(&cut), (true, false), "an overrun tail on an ended clock with a successor is a cut inside the gap");
    let no_successor = Observed { clocks: vec![(0, Some(100))], ..cut.clone() };
    assert!(!observed_shapes(&no_successor).0, "the last clock has no successor to reach");
    let no_tail = Observed { overruns: Vec::new(), ..cut.clone() };
    assert!(!observed_shapes(&no_tail).0, "without a delivered overrun tail nothing says the cut was in a gap");
    let not_ended = Observed { clocks: vec![(0, None), (200, None)], ..cut.clone() };
    assert!(!observed_shapes(&not_ended).0, "a clock that did not end was not cut");
    let answered = Observed { blocks: vec![(0, (50, 2_000, BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, Some(10)))], ..cut.clone() };
    assert!(!observed_shapes(&answered).0, "a published block at the first lost sample answered the report");
}

/// The successor origin of a Stop and a Start at one cut: one receive call at the old rate, the
/// allowance and the lead after the cut. Both Providers book the origin through this `plan`
/// (`rm_26` compares each Provider's observed origins with it), so the formula is pinned here.
#[test]
fn n5_the_receive_successor_origin_is_old_rate_call_plus_allowance_plus_lead() {
    let block = 2_000;
    let stream = terms(block).stream;
    let cut = T0 + 10 * MS;
    let item = |seq, kind| Item { e: cut, seq, ready: cut, delivered: None, refused: false, kind };
    let segments = plan(&stream, &[item(1, Kind::Stop), item(2, Kind::Start)]).unwrap();
    assert_eq!(segments[1].origin, cut + block * 200 + stream.allowance + stream.lead);
}

/// What uhd-rx left: the receive clocks as `(origin, ended)`, its blocks by clock, the
/// device's receive calls as `(instant, call)`, its `rx_stop` rows as `(at, issued)`, the
/// refusals uhd-rx booked, and what the scripted device found wrong as it happened.
#[derive(Clone, Debug, Default)]
struct Observed {
    clocks: Vec<(i64, Option<i64>)>,
    blocks: Vec<(usize, Block)>,
    /// The overrun `RX_OVERFLOW` events the run delivered, as `(clock index, first lost sample)`.
    overruns: Vec<(usize, i64)>,
    calls: Vec<(i64, String)>,
    stops: Vec<(i64, i64)>,
    /// The origins of the reports a successor's origin gate dropped (UR-18), from its `rx_report_before_origin` rows.
    gated: Vec<i64>,
    refused: Vec<u64>,
    /// Configurations the device was asked for at the refused rate.
    refusable: usize,
    rejected: usize,
    late: usize,
    lost: bool,
    abort: bool,
    /// The stops that left a restart streaming, by their index in `calls`: the device's log of survivors.
    survived: Vec<usize>,
    /// The device still streams at the end, unstopped.
    streaming: bool,
    /// Timed starts of segments the plan dropped before their origin (#67).
    dropped: usize,
    /// A command was booked while a receive call waited for the packet it returned.
    straddled: bool,
    /// uhd-rx pruned the timeline's items (#63).
    pruned: bool,
    violations: Vec<String>,
    /// How many `Set`s the line refused (RM-21), and the values each start's configure applied, by the origin it started.
    set_refused: usize,
    start_values: Vec<(i64, Option<(f64, f64)>)>,
    /// The rig's device time at the end, in root ticks, and its host clock's reading then.
    end: i64,
    host: Wall,
}

/// What a sequence's run shows of the rig's reach (S20).
#[derive(Default)]
struct Reach {
    floored: bool,
    cut_in_gap: bool,
    block_after_gap: bool,
    /// A stop left a restart streaming that only a second stop cleared.
    survivor_cleared: bool,
}

/// S20's two coverage shapes, from what the run showed: a published `RESTARTED` block, and an overrun
/// tail (an `RX_OVERFLOW` with no published block at or after its first lost sample) on a receive clock
/// that ended with a successor.
fn observed_shapes(observed: &Observed) -> (bool, bool) {
    let cut_in_gap = observed.overruns.iter().any(|(clock, tick)| {
        observed.clocks.get(*clock).is_some_and(|(_, ended)| ended.is_some())
            && observed.clocks.get(clock + 1).is_some()
            && observed.blocks.iter().filter(|(block_clock, _)| block_clock == clock).all(|(_, (first, _, _, _))| first < tick)
    });
    let block_after_gap = observed.blocks.iter().any(|(_, (_, _, flags, _))| flags.contains(BlockFlags::RESTARTED));
    (cut_in_gap, block_after_gap)
}

/// The timeline's account of what uhd-rx does with a sequence's plans, compared with what it
/// did; the problems found, and which of the rig's shapes (S20) the run shows.
fn compare(sequence: &Sequence, extra: &Extra, observed: &Observed) -> (Vec<String>, Reach) {
    let terms = terms(extra.block);
    let mut problems = observed.violations.clone();
    let items = sequence.items(&terms);
    let loss = items.iter().find(|item| matches!(item.kind, Kind::End { .. })).map(|item| item.e);
    // What is booked: every command received before `Provider::stop`, and its end of the
    // stream, and booked before a loss, and the loss; a refused change marked as such (RM-25).
    let stop = extra.stop.filter(|(at, _)| loss.is_none_or(|loss| *at < loss));
    let mut booked: Vec<Item> = items
        .iter()
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
    // The cut a segment carries in the last plan that has it: none when only the first plan has
    // it, where no stop was booked and its stream runs on.
    let planned_cut = |segment: &Segment| -> Option<i64> {
        let last = plans.iter().enumerate().filter_map(|(index, (_, plan))| plan.iter().find(|s| key(s) == key(segment)).map(|s| (index, s.cut))).next_back();
        match last {
            Some((index, cut)) if index > 0 => cut,
            _ => None,
        }
    };
    // Each overrun and sequence error on the segment the scripted device runs when it fires: the
    // one begun last by then. The device fires a fault whose sample an earlier overrun's skip has
    // passed at the skipping stream's next call, even past its cut, so it stays on that segment;
    // that is the device's behaviour, not UHD's. A hit placed by the skip fires; a fallback hit
    // fires iff its sample is before the planned cut (at the cut itself the stream stops before a
    // call could fire it). Limitation: the planned cut, not the floored cut RM-16 may take, decides.
    let mut hits: Vec<Vec<(i64, Fault)>> = vec![Vec::new(); all.len()];
    let mut fired: BTreeSet<(usize, i64)> = BTreeSet::new();
    let mut faults: Vec<(i64, Fault)> = sequence.faults.iter().filter(|(_, fault)| *fault != Fault::Loss).map(|(at, fault)| (terms.t0 + terms.ticks(*at), *fault)).collect();
    faults.sort_by_key(|(f, _)| *f);
    for (f, fault) in faults {
        let skipped = (0..all.len()).find(|index| hits[*index].iter().any(|(f1, kind)| *kind == Fault::Overrun && fired.contains(&(*index, *f1)) && *f1 <= f && f < *f1 + gap()));
        if let Some(index) = skipped.or_else(|| all.iter().rposition(|s| s.origin <= f)) {
            if fault == Fault::Overrun && (skipped == Some(index) || planned_cut(&all[index]).is_none_or(|cut| all[index].sample_at_or_after(f).unwrap() < cut)) {
                fired.insert((index, f));
            }
            hits[index].push((f, fault));
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
        blocks(&all[index], &hits[index], extra)
            .iter()
            .take_while(|(first, len, _, _)| all[index].instant(first + i64::from(*len)).unwrap() + extra.latency < bound)
            .last()
            .map_or(0, |(first, len, _, _)| first + i64::from(*len))
    };
    let mut floor = false;
    let mut cuts: Vec<Option<i64>> = vec![None; all.len()];
    // The arrival of the first plan that takes a segment uhd-rx has seen away or cuts it at its first sample: no stop of it can be earlier (#67).
    let mut cancelled: Vec<Option<i64>> = vec![None; all.len()];
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
                    if seen[index] {
                        cancelled[index].get_or_insert(*arrival);
                    }
                    continue;
                }
            };
            seen[index] = true;
            let floored = cut.max(delivered(index, *arrival));
            if floored == 0 {
                cancelled[index].get_or_insert(*arrival);
            }
            floor |= floored > cut;
            cuts[index] = Some(cuts[index].map_or(floored, |cut| cut.min(floored)));
        }
    }
    // A segment the last plan does not have, never begun, never ran.
    let last = &plans.last().expect("a plan").1;
    let kept: Vec<usize> = (0..all.len()).filter(|index| cuts[*index] != Some(0) && (cuts[*index].is_some() || last.iter().any(|s| key(s) == key(&all[*index])))).collect();
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
        let got: Vec<Block> = observed
            .clocks
            .iter()
            .position(|(origin, _)| *origin == segment.origin)
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
    // issued; untimed stops, one after each start, before its origin only when the plan had
    // taken the segment away or cut it at its first sample by then (#67); a streamer reopened
    // only for another channel count.
    let early = |origin: i64, at: i64| {
        let index = all.iter().position(|s| s.origin == origin);
        index.is_some_and(|index| early_stop_legal(kept.contains(&index), cancelled[index], at))
    };
    // The first sample of each block uhd-rx published, by its stream's origin: a block after an overrun begins at or
    // after the restart, so any block at or after `kf0` answered that overrun's report (UR-17, UR-18).
    let published: BTreeMap<i64, Vec<i64>> =
        observed.clocks.iter().enumerate().map(|(index, (origin, _))| (*origin, observed.blocks.iter().filter(|(clock, _)| *clock == index).map(|(_, (first, _, _, _))| *first).collect())).collect();
    // The last overrun hit of a kept segment and still unanswered by a published block: a second stop may follow
    // there, and it clears the START UHD queued behind the first. A report the device returns needs no
    // prediction by the rig, so no `fired` condition: the oracle permits a second stop wherever an overrun hit of
    // the segment has no published block at or after it, and the device's own log is what requires one.
    let unanswered = |origin: i64| {
        let Some(index) = segments.iter().position(|segment| segment.origin == origin) else { return false };
        let Some((f, _)) = hits[index].iter().rfind(|(_, fault)| *fault == Fault::Overrun) else { return false };
        let kf0 = segments[index].sample_at_or_after(*f).unwrap();
        published.get(&origin).is_none_or(|firsts| firsts.iter().all(|first| *first < kf0))
    };
    let mut stop_origins = Vec::new();
    let mut second_stops = 0;
    let mut ended: Option<i64> = None;
    // A report the successor's gate dropped (UR-18) armed that stream's end, so its stop may need the second one too;
    // a published block would have cleared the arming, and a block is on a clock at the stream's origin.
    let gated = |origin: i64| observed.gated.contains(&origin) && observed.clocks.iter().all(|(clock, _)| *clock != origin);
    let (mut started, mut opened): (Option<i64>, usize) = (None, 0);
    for (at, call) in &observed.calls {
        let value = call.split(' ').nth(1).unwrap_or_default();
        if call.starts_with("rx_start ") {
            if started.is_some() {
                problems.push(format!("{call} at {at}: the stream started before is not stopped"));
            }
            started = value.parse().ok();
            ended = None;
        } else if call.starts_with("rx_stop ") {
            match started.take() {
                Some(origin) => {
                    stop_origins.push(origin);
                    ended = Some(origin);
                    if *at < origin && !early(origin, *at) {
                        problems.push(format!("{call} at {at}: before the origin {origin} of a segment the plan keeps, or before the plan took it away"));
                    }
                }
                None if ended.take().is_some_and(|origin| unanswered(origin) || gated(origin)) => second_stops += 1,
                None => problems.push(format!("{call} at {at}: no stream to stop")),
            }
            if value != "now" {
                problems.push(format!("a timed stop: {call}"));
            }
        } else if call.starts_with("rx_open ") {
            let channels: usize = value.parse().unwrap();
            if channels == opened {
                problems.push(format!("{call} at {at}: reopened for the same channel count"));
            }
            opened = channels;
        }
    }
    let stopped_calls = observed.calls.iter().filter(|(_, call)| call.starts_with("rx_stop ")).count();
    if observed.stops.len() + second_stops != stopped_calls {
        problems.push(format!("{} rx_stop rows and {second_stops} second stops for {stopped_calls} stops", observed.stops.len()));
    }
    // A survivor the device's log shows must be cleared by the next stop: the second stop (UR-17, UR-26).
    if !observed.lost {
        for i in &observed.survived {
            if observed.calls.get(i + 1).is_none_or(|(_, call)| !call.starts_with("rx_stop ")) {
                problems.push(format!("a restart survived the stop at call {i} and no second stop cleared it"));
            }
        }
    }
    if let Some((at, issued)) =
        observed.stops.iter().zip(&stop_origins).find(|((at, issued), origin)| issued < at && abort_at.is_none_or(|abort| *issued < abort) && !early(**origin, *issued)).map(|(row, _)| row)
    {
        problems.push(format!("an rx_stop row issued at {issued}, before its cut {at}"));
    }
    let open = clocks.last().is_some_and(|(_, end)| end.is_none());
    if !observed.lost && abort_at.is_none() && observed.streaming != open {
        problems.push(format!("the device streams at the end: {}, the plan's last segment open: {open}", observed.streaming));
    }
    // RM-21: each `Set` the line took while streaming was refused, as the generator's oracle (written apart) says; and each segment a start began was
    // configured with the values that start took at its booking, whatever came after (a refused configure has no start to compare).
    let ops_booked = |index: usize| {
        let entry = sequence.schedule(&terms).into_iter().find(|entry| entry.event == generator::Event::Op(index)).expect("every op is scheduled");
        loss.is_none_or(|loss| entry.receipt + extra.lag < loss) && stop.is_none_or(|(at, _)| entry.receipt <= at)
    };
    let staged = sequence.staged(&terms, ops_booked);
    if staged.refused.len() != observed.set_refused {
        problems.push(format!("{} values refused, the oracle's {}", observed.set_refused, staged.refused.len()));
    }
    let defaults = super::super::test_support::default_description(extra.block as u32).defaults();
    let number = |name: &str| match defaults[&super::super::core::key(name)] {
        ezsdr_kernel::spec::Value::Scalar(ezsdr_kernel::spec::Scalar::Num(value)) => value.get(),
        ref other => panic!("{other:?}"),
    };
    let initial = (number("radio.rx.c0.frequency_hz"), number("radio.rx.c0.gain_db"));
    if observed.refused.is_empty() && observed.refusable == 0 {
        for (origin, applied) in &observed.start_values {
            let by = segments.iter().rev().find(|s| s.origin == *origin).and_then(|s| s.by);
            let (Some(by), Some(applied)) = (by, applied) else { continue };
            let Some((_, snapshot)) = staged.starts.iter().find(|(index, _)| (sequence.faults.len() + index) as u64 == by) else { continue };
            let expected = (snapshot.0.map_or(initial.0, |v| Leaf::Frequency.value(v)), snapshot.1.map_or(initial.1, |v| Leaf::Gain.value(v)));
            if *applied != expected {
                problems.push(format!("the start at {origin} (command {by}) was configured with {applied:?}, its snapshot is {expected:?}"));
            }
        }
    }
    // RM-25, UR-12: the first configuration at the refused rate is refused, once.
    let refusals = usize::from(observed.refusable > 0);
    if observed.refused.len() != refusals || observed.rejected != refusals {
        problems.push(format!("{} refusals, {} rejected, for {refusals}", observed.refused.len(), observed.rejected));
    }
    if observed.late > 0 {
        problems.push(format!("{} late starts", observed.late));
    }
    // S20: the shapes the rig is here to reach, so that a run which stops producing them fails instead of passing quietly.
    let (cut_in_gap, block_after_gap) = observed_shapes(observed);
    let reach = Reach { floored: floor, cut_in_gap, block_after_gap, survivor_cleared: !observed.survived.is_empty() };
    (problems, reach)
}

/// The rig device's problem for a timed start issued at or after its origin.
const LATE_START: &str = "not ahead of its origin";

/// A stop before its start's origin is legal only for a segment the plan does not keep, once
/// the plan that took it away or cut it at its first sample (`cancelled`, its arrival) had
/// arrived (#67).
fn early_stop_legal(kept: bool, cancelled: Option<i64>, at: i64) -> bool {
    !kept && cancelled.is_some_and(|arrival| at >= arrival)
}

#[test]
fn n67_an_early_stop_is_legal_only_for_a_cancelled_segment_after_its_plan() {
    assert!(early_stop_legal(false, Some(10), 10));
    assert!(early_stop_legal(false, Some(10), 11));
    assert!(!early_stop_legal(false, Some(10), 9), "before the plan that cancelled it arrived");
    assert!(!early_stop_legal(false, None, 11), "no plan cancelled it");
    assert!(!early_stop_legal(true, Some(10), 11), "the plan keeps the segment: its samples are lost");
}

#[test]
fn rm_26_the_rig_s_host_clock_moves_with_its_device_time() {
    let device = FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() });
    let mut host = RigHost::new(device.host());
    let step = 4_000_000;
    // A bracket, not a bound: the manual clock can creep by real time between the reads, so an
    // over-advance by a further step fails whatever the host's speed.
    let real_before = std::time::Instant::now();
    let before = host.clock.now();
    host.advance(step);
    let after = host.clock.now();
    let real = real_before.elapsed();
    let delta = Wall::from_millis(20);
    assert!(after - before >= delta, "the clock moved {:?}, less than the device time's {delta:?}", after - before);
    assert!(after - before <= delta + real, "the clock moved {:?}, further than {delta:?} and the {real:?} it could creep", after - before);
    assert_eq!(host.given, delta);
}

#[test]
fn rm_26_the_rig_s_host_clock_follows_set_now() {
    let sequence = generator::sequence(1);
    let extra = extra(1, &sequence);
    let observed = run(&sequence, &extra);
    assert!(observed.end > T0, "the rig's device time ended at {}", observed.end);
    // A host clock left on real time stays far below the device time it should have followed.
    assert!(observed.host >= Wall::from_nanos(observed.end as u64 * 5), "the host clock reads {:?}, before the device time {} in it", observed.host, observed.end);
}

/// One segment's blocks: a packet of `block_len` samples a request, the last before the cut
/// ending there, one cut short at a fault's sample, and after it the gap MockRadio's
/// `x3xx` has (layer 1); up to the horizon.
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
            Fault::Overrun => (k(f + gap()).max(kf), BlockFlags::RESTARTED),
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

/// What happens at an instant, besides the device's packets.
enum Ev {
    /// uhd-control books a command received `lag` earlier.
    Book(Item),
    /// uhd-control books a receive value (RM-21): the stream's line decides whether it is staged.
    Set(Leaf, u8, i64),
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

/// The rig's host clock: the FakeDevice's own manual clock (the one uhd-rx reads), moved with the rig's device time.
struct RigHost {
    clock: HostClock,
    given: Wall,
}

impl RigHost {
    fn new(clock: HostClock) -> RigHost {
        RigHost { clock, given: Wall::ZERO }
    }

    /// Moves the clock by `ticks` of device time (5 ns each on the 200 MHz root) and records the step given.
    fn advance(&mut self, ticks: i64) {
        let step = Wall::from_nanos(ticks as u64 * 5);
        self.given += step;
        self.clock.advance_for_test(step);
    }
}

/// The rig's time, what uhd-control does, and the scripted device's receive side.
struct World {
    time: Option<Arc<ManualTimeAuthority>>,
    root: ClockDomainId,
    now: i64,
    /// The device's host clock, which uhd-rx's silence monitor and waits read.
    host: RigHost,
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
    /// The knob: a restart may survive the first stop after it (UHD's queued START).
    survive: bool,
    /// UHD has queued an overrun's restart START and no sample of it has been returned yet.
    restart: bool,
    /// The index in `calls` of each stop that left a survivor.
    survived: Vec<usize>,
    latency: i64,
    lag: i64,
    block: i64,
    horizon: i64,
    done: bool,
    calls: Vec<(i64, String)>,
    straddled: bool,
    violations: Vec<String>,
    /// The receive values of record, as uhd-control's `Staging` holds them (RM-21).
    staging: Staging,
    /// How many `Set`s the line refused as streaming.
    set_refused: usize,
    /// The `(frequency, gain)` of the last configure of channel 0 before a start, and what each start was configured with, by origin.
    last_apply: Option<(f64, f64)>,
    start_values: Vec<(i64, Option<(f64, f64)>)>,
}

impl World {
    fn at(&mut self, t: i64, ev: Ev) {
        self.order += 1;
        self.events.insert((t, self.order), ev);
    }

    fn set_now(&mut self, t: i64) {
        if t > self.now {
            self.host.advance(t - self.now);
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
                let stream = booked_stream(Dir::Rx);
                if item.kind == Kind::Start {
                    // RM-21: the generator's items hold no start the stream refuses as streaming, so the shared decision accepts each one booked. After a
                    // loss the line is over and the oracle takes no command: nothing is decided.
                    let streaming = streams.line(stream).is_some_and(Line::streaming);
                    let snapshot = if self.lost { self.staging.snapshot() } else { self.staging.start(0, streaming).expect("a start the oracle accepted") };
                    streams.stream_mut(stream).configs.insert(item.seq, Arc::new(ColdConfig::new(snapshot)));
                }
                assert!(streams.book(&core, stream, item, None), "booked");
                self.booked += 1;
            }
            Ev::Set(leaf, v, ready) if !self.lost && self.stopped.is_none_or(|at| ready <= at) => {
                let streams = lock(&core.streams);
                let line = streams.line(booked_stream(Dir::Rx)).expect("the receive timeline");
                let name = match leaf {
                    Leaf::Frequency => "radio.rx.c0.frequency_hz",
                    Leaf::Gain => "radio.rx.c0.gain_db",
                };
                let value = ezsdr_kernel::spec::Value::num(leaf.value(v)).unwrap();
                if self.staging.set(&super::super::core::key(name), value, false, 0, |_| line.streaming(), |_| ezsdr_radio::staging::Bookings::unarmed()).is_err() {
                    self.set_refused += 1;
                }
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
            Ev::Book(_) | Ev::Set(..) | Ev::Stop(_) => {}
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
    fn describe(&self) -> serde_json::Value {
        self.inner.describe()
    }
    fn fidelity(&self) -> Fidelity {
        self.inner.fidelity()
    }
    /// Sol R07 (plan 30 §4.2): a test wrapper forwards the inner device's host clock, or it
    /// would inherit the real one and the Authority's and the fake's clocks would disagree.
    fn host(&self) -> crate::HostClock {
        self.inner.host()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> {
        self.inner.front_end(dir, chan)
    }
    fn serial(&self) -> Result<String, DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError> {
        self.inner.set_subdev_spec(dir, markup)
    }
    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> {
        self.inner.rates(dir, chan)
    }
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> {
        self.inner.set_sources(clock, time)
    }
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> {
        self.inner.set_time_zero(at_next_pps)
    }
    fn time_now(&self) -> Result<i64, DeviceError> {
        Ok(lock(&self.world).now)
    }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> {
        self.inner.ref_locked()
    }

    /// The FakeDevice's, which refuses a rate once when asked to (`WrongRate`); the receive
    /// rate it applied starts the next stream.
    fn apply(&self, dir: Dir, chan: usize, s: &Settings) -> Result<Applied, DeviceError> {
        let applied = self.inner.apply(dir, chan, s)?;
        if dir == Dir::Rx && chan == 0 && s.rate.is_some() {
            let mut world = lock(&self.world);
            world.last_apply = s.freq.zip(s.gain);
            world.rate = applied.rate;
            if s.rate != Some(applied.rate) {
                world.call(format!("apply_refused {}", s.rate.unwrap_or_default()));
            }
        }
        Ok(applied)
    }

    /// The stream number is not modelled: this rig runs the single stream uhd-rx opens
    /// today (UR-2, design 33 D5's streams arrive in 1b-4b).
    fn rx_open(&self, _stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        let mut world = lock(&self.world);
        world.call(format!("rx_open {}", channels.len()));
        // A new streamer: nothing the old one had in flight is delivered.
        world.runs.retain(|run| run.end.is_none());
        Ok(())
    }

    fn rx_start(&self, _stream: usize, at: i64) -> Result<(), DeviceError> {
        let mut world = lock(&self.world);
        if world.lost {
            return Err(DeviceError { lost: true, message: "scripted: the device is gone".to_owned() });
        }
        world.call(format!("rx_start {at}"));
        let applied = world.last_apply.take();
        world.start_values.push((at, applied));
        let now = world.now;
        let core = world.core.clone().expect("the rig's core");
        let planned = lock(&core.streams).line(booked_stream(Dir::Rx)).and_then(|line| line.plan.iter().find(|segment| segment.origin == at).copied());
        let mut problems = Vec::new();
        if at <= now {
            problems.push(format!("rx_start {at} at {now}: {LATE_START}"));
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

    fn rx_packet_samples(&self, _stream: usize) -> usize {
        0
    }

    /// The X3x0 stops a running stream at once (design-notes §11 F1) and cancels a start still
    /// queued (#67).
    fn rx_stop(&self, _stream: usize) -> Result<(), DeviceError> {
        let mut world = lock(&self.world);
        if world.lost {
            return Err(DeviceError { lost: true, message: "scripted: the device is gone".to_owned() });
        }
        world.call("rx_stop now".to_owned());
        // UHD's START behind the stop: the first stop after an overrun, before a sample at or after its restart came back,
        // leaves the run streaming until a second stop (graph.cpp:573–622, INFERRED as the model of the race).
        if world.survive && std::mem::take(&mut world.restart) {
            world.survive = false;
            let call = world.calls.len() - 1;
            world.survived.push(call);
            return Ok(());
        }
        world.restart = false;
        let now = world.now;
        for run in world.runs.iter_mut().filter(|run| run.end.is_none() && run.origin <= now) {
            run.end = Some(now);
        }
        world.runs.retain(|run| run.end.is_some());
        Ok(())
    }

    fn rx_recv(&self, _stream: usize, n: usize, timeout: Wall, _one_packet: bool) -> RxRecv {
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
        let fault = world
            .faults
            .iter()
            .enumerate()
            .filter(|(_, (f, _, fired))| !fired && *f >= origin && end.is_none_or(|end| *f < end))
            .map(|(index, (f, _, _))| (((f - origin + step - 1) / step).max(k), index))
            .min()
            .map(|(kf, index)| (kf, index, world.faults[index].0, world.faults[index].1));
        if let Some((kf, index, f, fault)) = fault {
            if kf == k {
                world.faults[index].2 = true;
                // UHD restarts only after an overrun: its START sits queued until a sample of it is returned.
                if fault == Fault::Overrun {
                    world.restart = true;
                }
                let kg = match fault {
                    Fault::Overrun => ((f + gap() - origin + step - 1) / step).max(kf),
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
        // A sample of the restart has been returned: after a report the run's next sample is its `kg`, so any later samples are at or after it.
        world.restart = false;
        RxRecv::Samples { first_tick: origin + k * step, samples }
    }

    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        self.inner.tx_open(stream, channels)
    }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Wall) -> Result<usize, DeviceError> {
        self.inner.tx_send(stream, samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, stream: usize, timeout: Wall) -> Result<Option<TxReport>, DeviceError> {
        self.inner.tx_async(stream, timeout)
    }
    fn rx_close(&self, stream: usize) {
        self.inner.rx_close(stream)
    }
    fn tx_close(&self, stream: usize) {
        self.inner.tx_close(stream)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), crate::DeviceError> {
        self.inner.raise_thread_priority()
    }
}

/// Runs a sequence's receive plans on uhd-rx: each command booked `lag` after its receipt, the
/// loss booked by whichever finds the device lost, a refusal by uhd-rx, `Provider::stop` at
/// `extra`'s instant.
fn run(sequence: &Sequence, extra: &Extra) -> Observed {
    let terms = terms(extra.block);
    let link = Arc::new(Headers::default());
    let refuse = extra.refuse.map(|rate| FakeFault::WrongRate { claimed: rate as f64, applied: rate as f64 - 1_000.0, nth: 0 });
    let inner = FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, faults: refuse.into_iter().collect(), ..FakeConfig::default() });
    // The device's own clock, the one `Scripted::host` hands uhd-rx: a clock of its own would let the tests pass and fix nothing.
    let host = inner.host();
    let (to_rx, cmds) = channel();
    let world = Arc::new(Mutex::new(World {
        time: None,
        root: ClockDomainId::local(0),
        now: 0,
        host: RigHost::new(host),
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
        survive: extra.survivor,
        restart: false,
        survived: Vec::new(),
        latency: extra.latency,
        lag: extra.lag,
        block: extra.block,
        horizon: extra.horizon,
        done: false,
        calls: Vec::new(),
        straddled: false,
        violations: Vec::new(),
        staging: {
            // The Run's configuration: receive channel 0 in stream 0.
            let mut config = super::super::test_support::default_description(extra.block as u32).defaults();
            config.insert(super::super::core::key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
            Staging::new(config)
        },
        set_refused: 0,
        last_apply: None,
        start_values: Vec::new(),
    }));
    let device = Arc::new(Scripted { inner, world: world.clone() });
    let (core, time, events) = super::super::test_support::rig_on(device.clone(), vec![link.clone()], extra.block as u32);
    let items = sequence.items(&terms);
    let loss = items.iter().find(|item| matches!(item.kind, Kind::End { .. })).copied();
    // `start` shares the stream's timeline from T0 (UR-15).
    lock(&core.streams).lines.insert(booked_stream(Dir::Rx), Line::new(terms.stream));
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
        // The commands and the values, in their arrival order, each booked `lag` after its receipt.
        let lag = w.lag;
        let ops: Vec<&Op> = sequence.ops().map(|(_, op)| op).collect();
        let mut arrivals = sequence.schedule(&terms);
        arrivals.sort_by_key(|entry| (entry.receipt, entry.seq));
        for entry in arrivals {
            let generator::Event::Op(index) = entry.event else { continue };
            match ops[index] {
                Op::Set { leaf, v } => w.at(entry.receipt + lag, Ev::Set(*leaf, *v, entry.receipt)),
                // RM-21: a start the stream refused as streaming (the oracle's) is no item and books nothing.
                Op::StartRx { .. } if !items.iter().any(|item| item.seq == entry.seq) => {}
                _ => {
                    let item = items.iter().find(|item| item.seq == entry.seq).copied().expect("a command's item");
                    w.at(item.ready + lag, Ev::Book(item));
                }
            }
        }
        if let Some((at, mode)) = extra.stop {
            w.at(at, Ev::Stop(mode));
        }
    }
    // `prepare` opened the streamer for one channel, `start` started it at T0 (UR-15).
    device.rx_open(0, &[0]).unwrap();
    lock(&core.streams).record_open(crate::provider::core::stream_key(Dir::Rx, 0), Some(Channels::first(1)));
    device.rx_start(0, T0).unwrap();
    let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 0), core.root, Rational::new(200, 1).unwrap()).unwrap();
    let mut rx = Rx::new(core.clone(), 0, cmds, Some((handle, Channels::first(1), T0)), None);
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
            let line = streams.line(booked_stream(Dir::Rx)).expect("the receive timeline");
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
    let final_plan = lock(&core.streams).line(booked_stream(Dir::Rx)).map(|line| line.plan.clone()).unwrap_or_default();
    let lost_at = rec.timing.iter().find(|row| row["what"] == "device_lost").and_then(|row| row["at"].as_i64());
    let mut violations = w.violations.clone();
    if lost_at != loss.filter(|_| core.is_lost()).map(|loss| loss.e) {
        violations.push(format!("lost at {lost_at:?}, not at the sequence's loss"));
    }
    let delivered = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
    let overruns = delivered
        .iter()
        .filter(|event| event.kind.as_str() == kinds::RX_OVERFLOW)
        .filter(|event| ezsdr_radio::payloads::RxOverflowPayload::from_payload(&event.payload).is_ok_and(|payload| payload.cause == ezsdr_radio::payloads::RxOverflowCause::Overrun))
        .filter_map(|event| {
            let clock = rx_clocks.iter().position(|clock| clock.domain == event.time.domain())?;
            Some((clock, event.time.ticks_in(rx_clocks[clock].domain).ok()?))
        })
        .collect();
    Observed {
        clocks: rx_clocks.iter().map(|r| (r.origin.ticks_in(r.root).unwrap(), r.ended_at.map(|t| t.ticks_in(r.root).unwrap()))).collect(),
        blocks: headers
            .iter()
            .map(|h| {
                let clock = rx_clocks.iter().position(|r| r.domain == h.first_sample_time.domain()).expect("a block on a receive clock");
                (clock, (h.first_sample_time.ticks_in(rx_clocks[clock].domain).unwrap(), h.len, h.flags, h.lost))
            })
            .collect(),
        overruns,
        stops: rec.applied.iter().filter(|r| r["key"] == "rx_stop").map(|r| (r["at"]["ticks"].as_i64().unwrap(), r["issued"]["ticks"].as_i64().unwrap())).collect(),
        gated: rec.timing.iter().filter(|row| row["what"] == "rx_report_before_origin").filter_map(|row| row["origin"].as_i64()).collect(),
        refused: w.refused.clone(),
        refusable: extra.refuse.map_or(0, |rate| device.inner.calls().iter().filter(|c| c.starts_with(&format!("apply rx 0 rate={rate} "))).count()),
        rejected: rec.rejected.len(),
        late: delivered.iter().filter(|e| e.kind.as_str() == kinds::LATE_COMMAND).count(),
        lost: core.is_lost(),
        abort: w.abort,
        survived: w.survived.clone(),
        streaming: !w.lost && w.runs.iter().any(|run| run.end.is_none()),
        dropped: w.calls.iter().filter_map(|(_, call)| call.strip_prefix("rx_start ")).filter(|at| !final_plan.iter().any(|s| s.origin.to_string() == *at)).count(),
        straddled: w.straddled,
        pruned: lock(&core.streams).line(booked_stream(Dir::Rx)).is_some_and(|line| line.items.len() < w.booked),
        calls: w.calls.clone(),
        violations,
        set_refused: w.set_refused,
        start_values: w.start_values.clone(),
        end: w.now,
        host: w.host.clock.now(),
    }
}
