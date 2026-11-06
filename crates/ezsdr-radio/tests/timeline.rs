//! RM-25: `ezsdr_radio::timeline`, RM-16, RM-21 and RM-25 computed once (spec 22, VH-7),
//! and its invariants over VH-8's generated sequences.

mod generator;

use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::stream::Direction;
use ezsdr_kernel::time::Rational;
use ezsdr_radio::stream_target;
use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Line, Segment, Stream, plan};
use generator::{CLASSES, Terms};

/// Root ticks per millisecond on the 1 GHz root.
const MS: i64 = 1_000_000;

fn config(channels: u16, rate: u64) -> Config {
    Config { channels: Channels::first(u32::from(channels)), ratio: Rational::new(1_000_000_000, rate).unwrap() }
}

/// The set of the given channel numbers.
fn channels(numbers: &[u32]) -> Channels {
    numbers.iter().fold(Channels::first(0), |set, number| set.with(*number))
}

/// A configuration of `numbers` at `rate` samples per second.
fn set_config(numbers: &[u32], rate: u64) -> Config {
    Config { channels: channels(numbers), ratio: Rational::new(1_000_000_000, rate).unwrap() }
}

/// A stream at 1 MS/s from 0 with a 50 ms lead and no receive call (Appendix C's terms).
fn stream(direction: Direction, channels: u16) -> Stream {
    Stream { direction, origin: 0, config: config(channels, 1_000_000), lead: 50 * MS, call: 0, allowance: 0 }
}

/// A receive stream of `numbers` at `rate`, from 0, with a 50 ms lead and no call.
fn rx_stream(numbers: &[u32], rate: u64) -> Stream {
    Stream { direction: Direction::Rx, origin: 0, config: set_config(numbers, rate), lead: 50 * MS, call: 0, allowance: 0 }
}

fn item(e: i64, seq: u64, ready: i64, kind: Kind) -> Item {
    Item { e, seq, ready, delivered: None, refused: false, kind }
}

fn refused(item: Item) -> Item {
    Item { refused: true, ..item }
}

fn delivered(sample: i64, item: Item) -> Item {
    Item { delivered: Some(sample), ..item }
}

fn segments(stream: &Stream, items: &[Item]) -> Vec<(i64, Option<i64>)> {
    plan(stream, items).unwrap().iter().map(|s| (s.origin, s.cut.map(|k| s.instant(k).unwrap()))).collect()
}

/// A line's planned segments as `(origin, cut instant)`.
fn planned(line: &Line) -> Vec<(i64, Option<i64>)> {
    line.plan.iter().map(|segment| (segment.origin, segment.cut.map(|k| segment.instant(k).unwrap()))).collect()
}

/// The line booked with the items that take effect by `t`: what the public plan and `streaming()`
/// say at `t` (RM-25).
fn prefix(stream: &Stream, items: &[Item], t: i64) -> Line {
    let mut line = Line::new(*stream);
    for item in items.iter().filter(|item| item.e <= t) {
        line.book(*item).unwrap();
    }
    line
}

#[test]
fn rm_26_a_receive_stream_takes_the_receive_start_lead_and_a_transmit_line_has_none() {
    // RM-4, RM-25: a receive stream's new segment uses `radio.timing.rx_start_lead_ns`; a
    // transmit stream has no start lead (note 47 stage 4).
    let timing = ezsdr_radio::TimingEnvelope { min_timed_command_lead_ns: 0, startup_latency_ns: 0, overflow_restart_gap_ns: 0, rx_start_lead_ns: 50 * MS, tx_in_flight_ns: 0 };
    assert_eq!(ezsdr_radio::timeline::start_lead_ns(&timing), 50 * MS);

    // A `start_rx` after a `Stop` starts its lead after that cut, at 1 MS/s.
    let direction = Direction::Rx;
    let stream = Stream { direction, origin: 0, config: config(1, 1_000_000), lead: ezsdr_radio::timeline::start_lead_ns(&timing), call: 0, allowance: 0 };
    assert_eq!(segments(&stream, &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), item(1_000 * MS, 1, 1_000 * MS, Kind::Start)]), [(0, Some(1_000 * MS)), (1_050 * MS, None)]);
}

#[test]
fn rm_15_later_bound_combines_the_current_and_handed_front() {
    use ezsdr_radio::timeline::later_bound;

    assert_eq!(later_bound(10, None), 10);
    assert_eq!(later_bound(10, Some(9)), 10);
    assert_eq!(later_bound(10, Some(10)), 10);
    assert_eq!(later_bound(10, Some(11)), 11);
}

#[test]
fn rm_25_a_receive_successor_adds_one_call_and_allowance_before_lead() {
    let cut = 10 * MS;
    for (lead, expected) in [(3 * MS, cut + 26 * MS), (50 * MS, cut + 73 * MS)] {
        let stream = Stream { lead, call: 20_000, allowance: 3 * MS, ..rx_stream(&[0], 1_000_000) };
        assert_eq!(segments(&stream, &[item(cut, 1, cut, Kind::Stop), item(cut, 2, cut, Kind::Start)]), [(0, Some(cut)), (expected, None)], "lead {lead}");
    }
}

#[test]
fn rm_25_a_receive_origin_with_a_fractional_call_rounds_once_to_the_grid() {
    // Three MS/s: one sample is 1000/3 ns, so the call term is ceil(1000/3) = 334 ns, and the
    // lattice step is 1000 ns. The origin is ceil(16 000 334 / 1000) * 1000 = 16 001 000; a
    // dropped call term, a dropped allowance, a dropped lead and a floor each give another value.
    let cut = 10 * MS;
    let stream = Stream { lead: 3 * MS, call: 1, allowance: 3 * MS, ..rx_stream(&[0], 3_000_000) };
    assert_eq!(segments(&stream, &[item(cut, 1, cut, Kind::Stop), item(cut, 2, cut, Kind::Start)]), [(0, Some(cut)), (16_001_000, None)]);
}

#[test]
fn rm_15_preemption_front_applies_each_late_policy() {
    use ezsdr_kernel::id::ClockDomainId;
    use ezsdr_kernel::stream::{LateOutcome, LatePolicy};
    use ezsdr_kernel::time::Duration;
    use ezsdr_radio::timeline::late_policy_front_outcome;

    let late_by = Duration::new(ClockDomainId::HOST_MONOTONIC, 2);
    assert_eq!(late_policy_front_outcome(LatePolicy::RejectAtPlan, 9, 10, Some(11), |_| Ok::<_, ()>(late_by)).unwrap(), Some(LateOutcome::PlanViolation { late_by }));
    assert_eq!(late_policy_front_outcome(LatePolicy::SendAsapAndFlag, 9, 10, Some(11), |_| Ok::<_, ()>(late_by)).unwrap(), Some(LateOutcome::SendAsap { late_by }));
    assert_eq!(late_policy_front_outcome(LatePolicy::DropAndFlag, 9, 10, Some(11), |_| Ok::<_, ()>(late_by)).unwrap(), Some(LateOutcome::Drop { late_by }));
    assert_eq!(late_policy_front_outcome(LatePolicy::DropAndFlag, 11, 10, Some(11), |_| Ok::<_, ()>(late_by)).unwrap(), None, "equality with the front is on time");
    assert_eq!(late_policy_front_outcome(LatePolicy::DropAndFlag, 9, 11, Some(10), |_| Ok::<_, ()>(late_by)).unwrap(), None, "a front at or before the ordinary bound adds no lateness");
}

#[test]
fn rm_26_the_timeline_cases() {
    let rx = stream(Direction::Rx, 1);
    let tx = stream(Direction::Tx, 1);
    let case = |name: &str, stream: &Stream, items: &[Item], expected: &[(i64, Option<i64>)]| {
        assert_eq!(segments(stream, items), expected, "{name}");
    };
    // Appendix C's cases that survive the removal of Run-time changes, at 1 MS/s on a 1 GHz root
    // with a 50 ms lead.
    case("a second Stop keeps the first end (VG-1)", &rx, &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), item(2_000 * MS, 1, 2_000 * MS, Kind::Stop)], &[(0, Some(1_000 * MS))]);
    case(
        "a start that arrived first, at 2 s, restarts after a Stop at 1 s",
        &rx,
        &[item(2_000 * MS, 0, 500 * MS, Kind::Start), item(1_000 * MS, 1, 1_000 * MS, Kind::Stop)],
        &[(0, Some(1_000 * MS)), (2_000 * MS, None)],
    );
    case(
        "stop(); start() restarts a lead after the cut",
        &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), item(1_000 * MS, 1, 1_000 * MS, Kind::Start)],
        &[(0, Some(1_000 * MS)), (1_050 * MS, None)],
    );
    case("start(); stop() does not restart", &rx, &[item(1_000 * MS, 0, 1_000 * MS, Kind::Start), item(1_000 * MS, 1, 1_000 * MS, Kind::Stop)], &[(0, Some(1_000 * MS))]);
    case("an off-lattice Stop rounds up (#33)", &rx, &[item(1_000 * MS + 1, 0, 1_000 * MS + 1, Kind::Stop)], &[(0, Some(1_000 * MS + 1_000))]);

    // RM-16, RM-25: a cut is never before sample 0. A Stop that reaches a restart before it begins
    // cuts it at its origin with no sample: the segment is left out, and the next start's lead
    // follows that origin (1 050 ms), not the Stop's instant.
    case("a Stop at the Run's start leaves a receive stream no segment", &rx, &[item(0, 0, 0, Kind::Stop)], &[]);
    case(
        "a Stop before a restart begins leaves it out, and the next start follows its origin",
        &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), item(1_000 * MS, 1, 1_000 * MS, Kind::Start), item(1_020 * MS, 2, 1_020 * MS, Kind::Stop), item(1_030 * MS, 3, 1_030 * MS, Kind::Start)],
        &[(0, Some(1_000 * MS)), (1_100 * MS, None)],
    );

    // A rate no whole number of root ticks: 3 MS/s is 1 000/3 ticks a sample, its lattice
    // 1 000 ticks (RM-25).
    let third = Stream { config: config(1, 3_000_000), lead: 0, ..rx };
    case("a cut on a fractional rate is its sample's instant, rounded up", &third, &[item(500, 0, 500, Kind::Stop)], &[(0, Some(667))]);
    // The cut is a sample index: the sample at 334 ticks is sample 2 (666.7), not sample 1,
    // whose instant rounds up to 334; read back from 667, it would be sample 3.
    let cut = plan(&third, &[item(334, 0, 334, Kind::Stop)]).unwrap()[0].cut;
    assert_eq!(cut, Some(2), "a fractional cut is its sample's index");
    case("a restart on a fractional rate starts on its lattice", &third, &[item(500, 0, 500, Kind::Stop), item(500, 1, 500, Kind::Start)], &[(0, Some(667)), (1_000, None)]);

    // VH-4's ready term: one receive call at the old rate (2 000 samples at 400 kS/s, 5 ms)
    // and 3 ms after the cut, then the lead.
    let x310 = Stream { config: config(1, 400_000), call: 2_000, allowance: 3 * MS, ..rx };
    case(
        "a receive restart waits the call in progress at its cut",
        &x310,
        &[item(1_000 * MS, 0, 500 * MS, Kind::Stop), item(1_000 * MS, 1, 500 * MS, Kind::Start)],
        &[(0, Some(1_000 * MS)), (1_058 * MS, None)],
    );

    // The transmit side: a Stop ends bursts, not the clock.
    case("a transmit Stop ends no clock", &tx, &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop)], &[(0, None)]);
    // m10 (N47S4-36): a transmit End ends its clock at the origin whether or not it aborts.
    case("a transmit abort before the origin ends the clock at it", &Stream { origin: 1_000 * MS, ..tx }, &[item(500 * MS, 0, 500 * MS, Kind::End { abort: true })], &[(1_000 * MS, Some(1_000 * MS))]);

    // VH-2, RM-25: a refused `start_rx` starts nothing and halts the stream until a Stop and the next start (a Provider refuses a start on a halted stream).
    case(
        "a refused start_rx starts nothing; a Stop and the next start_rx do",
        &rx,
        &[
            item(1_000 * MS, 0, 1_000 * MS, Kind::Stop),
            refused(item(2_000 * MS, 1, 2_000 * MS, Kind::Start)),
            item(3_000 * MS, 2, 3_000 * MS, Kind::Stop),
            item(3_000 * MS, 3, 3_000 * MS, Kind::Start),
        ],
        &[(0, Some(1_000 * MS)), (3_050 * MS, None)],
    );
    case("a start_rx on a running stream does nothing", &rx, &[item(1_500 * MS, 0, 1_500 * MS, Kind::Start)], &[(0, None)]);
    case("a refused start_rx leaves the stream halted", &rx, &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), refused(item(2_000 * MS, 1, 2_000 * MS, Kind::Start))], &[(0, Some(1_000 * MS))]);

    // RM-16: every cut is at or after the first sample not yet delivered; an abort's is it.
    case("a Stop's cut is floored at what was delivered", &rx, &[delivered(1_002_000, item(1_000 * MS, 0, 1_000 * MS, Kind::Stop))], &[(0, Some(1_002 * MS))]);
    case("an abort cuts at the first sample not yet delivered", &rx, &[delivered(700_000, item(1_000 * MS, 0, 1_000 * MS, Kind::End { abort: true }))], &[(0, Some(700 * MS))]);
    case(
        "an abort that knows nothing delivered cuts at its instant",
        &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::End { abort: true }), item(1_500 * MS, 1, 1_500 * MS, Kind::Start)],
        &[(0, Some(1_000 * MS))],
    );

    // VH-2: a loss ends the receive stream at the loss and a running transmit stream's bursts,
    // not its clock; a transmit clock that begins after the loss ends at its origin.
    case("a loss ends the receive stream at the loss", &rx, &[item(800 * MS, 0, 0, Kind::End { abort: false }), item(1_500 * MS, 1, 1_500 * MS, Kind::Start)], &[(0, Some(800 * MS))]);
    case("a loss ends a running transmit stream's bursts, not its clock", &tx, &[item(1_000 * MS, 0, 0, Kind::End { abort: false })], &[(0, None)]);
    let late_clock = Stream { origin: 2_000 * MS, ..tx };
    case("a loss before a transmit clock's origin ends it at its origin", &late_clock, &[item(1_000 * MS, 0, 0, Kind::End { abort: false })], &[(2_000 * MS, Some(2_000 * MS))]);

    // Each segment names the command that began it; the first, none.
    let restarted = plan(&rx, &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), item(1_000 * MS, 1, 1_000 * MS, Kind::Start)]).unwrap();
    assert_eq!(restarted.iter().map(|segment| segment.by).collect::<Vec<_>>(), [None, Some(1)]);
}

/// VH-8 on the timeline alone, `start_rx` and refusals included: on every generated
/// sequence, both directions' plans, at 1 and 3 MS/s, keep RM-16's and RM-25's invariants and do not depend on
/// the order the items are given in.
#[test]
fn rm_26_generated_sequences_keep_the_timeline_invariants() {
    let (mut seen, mut fractional) = (std::collections::BTreeSet::new(), 0);
    for seed in 0..1_000 {
        let sequence = generator::sequence(seed);
        seen.insert(sequence.class);
        for (direction, rate) in [(Direction::Rx, 1_000_000), (Direction::Tx, 1_000_000), (Direction::Rx, 3_000_000), (Direction::Tx, 3_000_000)] {
            let stream = Stream { origin: 2_000 * MS, call: 2_000, allowance: 3 * MS, config: config(1, rate), ..stream(direction, 1) };
            let terms = Terms { t0: 2_000 * MS, root_hz: 1_000_000_000, stream };
            let mut items = sequence.items(&terms);
            // Every fifth start refused at its instant (VH-2).
            for item in items.iter_mut().filter(|item| item.seq % 5 == 4) {
                item.refused = true;
            }
            let planned = plan(&stream, &items).unwrap();
            items.reverse();
            assert_eq!(plan(&stream, &items).unwrap(), planned, "seed {seed}: the plan depends on the items' order");
            check(&stream, &planned, seed);
            fractional += planned.iter().filter(|segment| segment.config.ratio.den() > 1).count();
        }
    }
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), CLASSES, "every class is generated");
    assert!(fractional > 0, "a generated segment runs at a fractional ratio");
}

/// #63: a line that prunes the items before each booking's receipt plans every generated
/// sequence as one that keeps them all, after every booking and every refusal.
#[test]
fn rm_26_a_pruned_line_plans_as_one_that_keeps_every_item() {
    let mut pruned_any = 0;
    for seed in 0..1_000 {
        let sequence = generator::sequence(seed);
        for direction in [Direction::Rx, Direction::Tx] {
            let stream = Stream { origin: 2_000 * MS, call: 2_000, allowance: 3 * MS, ..stream(direction, 1) };
            let terms = Terms { t0: 2_000 * MS, root_hz: 1_000_000_000, stream };
            let mut items = sequence.items(&terms);
            // As a Provider books them: in the order received.
            items.sort_by_key(|item| (item.ready, item.seq));
            let (mut full, mut pruned) = (Line::new(stream), Line::new(stream));
            for item in items {
                // Everything booked from now on takes effect at or after its receipt.
                let before = pruned.items.len();
                pruned.prune(item.ready);
                pruned_any += before - pruned.items.len();
                full.book(item).unwrap();
                pruned.book(item).unwrap();
                // Every fifth change or start refused once booked (VH-2).
                if item.seq % 5 == 4 && item.kind == Kind::Start {
                    assert!(full.refuse(item.seq).unwrap() && pruned.refuse(item.seq).unwrap());
                }
                assert_eq!(pruned.plan, full.plan, "seed {seed}, {direction:?}, after item {}", item.seq);
            }
        }
    }
    assert!(pruned_any > 0, "some item is pruned");
}

/// #63: an item that takes effect at the prune instant is kept, so that one booked later at
/// that instant, received earlier, still takes effect before it: a start received first and
/// booked after a `Stop` at the same instant does not restart the stream.
#[test]
fn rm_26_a_line_pruned_at_an_item_s_instant_keeps_it() {
    let rx = stream(Direction::Rx, 1);
    let (mut full, mut pruned) = (Line::new(rx), Line::new(rx));
    let stop = item(1_000 * MS, 1, 500 * MS, Kind::Stop);
    let start = item(1_000 * MS, 0, 1_000 * MS, Kind::Start);
    full.book(stop).unwrap();
    pruned.book(stop).unwrap();
    pruned.prune(1_000 * MS);
    full.book(start).unwrap();
    pruned.book(start).unwrap();
    assert_eq!(pruned.plan, full.plan);
    assert_eq!(full.plan.len(), 1, "no restart: {:?}", full.plan);
}

/// RM-25: a booking whose plan cannot be represented leaves the line as it was.
#[test]
fn rm_26_a_refused_booking_leaves_the_line() {
    let mut line = Line::new(stream(Direction::Rx, 1));
    line.book(item(1_000 * MS, 1, 1_000 * MS, Kind::Stop)).unwrap();
    let before = line.clone();
    assert!(line.book(item(i64::MAX - 1, 2, 1_000 * MS, Kind::Start)).is_err());
    assert_eq!(line, before);
}

/// #63: over a long Run the items a line keeps stay few, whatever it has booked.
#[test]
fn rm_26_a_pruned_line_keeps_few_items() {
    let mut line = Line::new(stream(Direction::Rx, 1));
    for n in 0..10_000_i64 {
        let now = n * 300 * MS;
        line.prune(now);
        line.book(item(now + 100 * MS, 2 * n as u64, now, Kind::Stop)).unwrap();
        line.book(item(now + 100 * MS, 2 * n as u64 + 1, now, Kind::Start)).unwrap();
        assert!(line.items.len() <= 2, "{} items after {n} bookings", line.items.len());
    }
    assert_eq!(line.plan.len(), 10_001, "every segment stays in the plan");
}

/// RM-12, RM-13, RM-16: the one place a stream node's id is read: `<device>/rx/s<k>` and
/// `<device>/tx/s<k>` name stream `k` of that direction, and nothing else does.
#[test]
fn rm_12_a_stream_node_names_its_direction_and_number() {
    let device = ResourceId::parse("radio").unwrap();
    let at = |path: &str| ResourceId::parse(path).unwrap();
    assert_eq!(stream_target(&device, &at("radio/rx/s0")), Some((Direction::Rx, 0)));
    assert_eq!(stream_target(&device, &at("radio/tx/s12")), Some((Direction::Tx, 12)));
    assert_eq!(stream_target(&device, &at("radio/rx/s999999999")), Some((Direction::Rx, 999_999_999)));
    for rejected in ["radio/rx", "radio/tx", "radio/rx/s", "radio/rx/s01", "radio/rx/s0x", "radio/rx/c0", "radio/rx/s1234567890", "radio/s0", "radio/rx/s0/deeper", "radio/other/s0"] {
        assert_eq!(stream_target(&device, &at(rejected)), None, "{rejected}");
    }
    assert_eq!(stream_target(&at("other"), &at("radio/rx/s0")), None, "another device");
    assert_eq!(stream_target(&at("radio/rx"), &at("radio/rx/s0")), None, "the stream node is no device");
}

fn check(stream: &Stream, planned: &[Segment], seed: u64) {
    let receive = stream.direction == Direction::Rx;
    for (index, segment) in planned.iter().enumerate() {
        let at = format!("seed {seed}, {:?} segment {index}: {planned:?}", stream.direction);
        // RM-25: on the lattice, the first at the stream's origin.
        if index > 0 || segment.origin != stream.origin {
            assert_eq!(segment.origin % segment.config.ratio.num() as i64, 0, "{at}");
        }
        assert!(!segment.config.channels.is_empty(), "{at}");
        match segment.cut {
            None => assert_eq!(index, planned.len() - 1, "only the last segment runs on: {at}"),
            Some(cut) => {
                // RM-16: never before the origin; a receive clock has a sample.
                assert!(cut >= 0 && (!receive || cut > 0), "{at}");
            }
        }
        if let Some(previous) = index.checked_sub(1).map(|i| planned[i]) {
            // RM-25's start rule, against the last cut kept.
            let cut = previous.cut.expect("a cut before the next segment");
            let mut earliest = previous.instant(cut).unwrap() + stream.lead;
            if receive {
                let call = Segment { origin: 0, cut: None, config: previous.config, by: None }.instant(stream.call).unwrap();
                earliest += call + stream.allowance;
            }
            assert!(segment.origin >= earliest, "{at}");
        }
    }
}

#[test]
fn mr_29_a_receive_link_floors_a_command_at_its_t0() {
    // MR-29 (RM-25): a `start_rx` takes effect at its `at`, or at its receipt when that is
    // later, and a receive link never before T0. Both here lie before T0; a transmit link
    // has no such floor.
    let t0 = 10 * MS;
    let receive = Line::new(Stream { origin: t0, ..stream(Direction::Rx, 1) });
    assert_eq!(ezsdr_radio::timeline::command_instant(Some(&receive), Some(t0 - 2 * MS), t0 - 3 * MS), t0);
    let transmit = Line::new(Stream { origin: t0, ..stream(Direction::Tx, 1) });
    assert_eq!(ezsdr_radio::timeline::command_instant(Some(&transmit), Some(t0 - 2 * MS), t0 - 3 * MS), t0 - 2 * MS, "a transmit link takes `max(at, now)`");
}

/// RM-25 (design 43): the plan of a receive stream whose `start_rx` is refused when the stale
/// delivered index of an item says nothing of the segment the refusal left out: a virtual segment
/// is cut at the item's instant, whatever `delivered` holds.
#[test]
fn rm_25_a_virtual_segment_ignores_a_stale_delivered_index() {
    let rx = rx_stream(&[0, 1], 1_000_000);
    let stopped = item(500 * MS, 0, 500 * MS, Kind::Stop);
    let refused_start = refused(item(1_000 * MS, 1, 1_000 * MS, Kind::Start));
    for stale in [None, Some(700_000), Some(2_000_000)] {
        // An aborting End: it stops the stream for good, so no Start after it begins a segment.
        let end = Item { delivered: stale, ..item(1_500 * MS, 3, 1_500 * MS, Kind::End { abort: true }) };
        let mut line = Line::new(rx);
        for booked in [stopped, refused_start, end] {
            line.book(booked).unwrap();
        }
        let before = planned(&line);
        assert_eq!(before, [(0, Some(500 * MS))], "{stale:?}: the refused segment is no segment of the plan");
        line.book(item(1_600 * MS, 4, 1_600 * MS, Kind::Start)).unwrap();
        assert_eq!(planned(&line), before, "{stale:?}: an End stops the stream for good");

        // A Stop: the stream begins again a lead after the Stop's cut.
        let stop = Item { delivered: stale, ..item(1_500 * MS, 3, 1_500 * MS, Kind::Stop) };
        let mut line = Line::new(rx);
        for booked in [stopped, refused_start, stop, item(1_600 * MS, 4, 1_600 * MS, Kind::Start)] {
            line.book(booked).unwrap();
        }
        assert_eq!(planned(&line), [(0, Some(500 * MS)), (1_650 * MS, None)], "{stale:?}");
    }
}

/// The items in the order they take effect (RM-25).
fn in_order(items: &[Item]) -> Vec<Item> {
    let mut order = items.to_vec();
    order.sort_by_key(|item| (item.e, item.seq));
    order
}

/// Whether the next item to take effect after the item `seq` is a `start_rx`: it resumes a
/// stream the refusal of `seq` halted and does nothing to one it did not (the one way a refusal
/// changes a later segment besides the refused one). Any other item ends the question.
fn start_follows(items: &[Item], seq: u64) -> bool {
    let order = in_order(items);
    let at = order.iter().position(|item| item.seq == seq).expect("the item is booked");
    order.get(at + 1).is_some_and(|item| item.kind == Kind::Start)
}

/// The streams the generated refusal tests run: the 50 ms lead of the others, and a 5 ms one,
/// so that a refusal overlaps a lead.
fn refusal_streams(direction: Direction) -> [Stream; 2] {
    let base = Stream { origin: 2_000 * MS, call: 2_000, allowance: 3 * MS, ..stream(direction, 1) };
    [base, Stream { lead: 5 * MS, ..base }]
}

/// N6 (note 48 S2b-ii, G5): the halt rule through the public plan and `streaming()`. A refused
/// `start_rx` leaves the stream streaming and plans no segment for itself; a later Stop stops it,
/// and the next `start_rx` starts it again.
#[test]
fn halt_rule_is_streaming_and_refusal_exclusion() {
    let rx = stream(Direction::Rx, 1);
    let mut line = Line::new(rx);
    line.book(item(1_000 * MS, 0, 1_000 * MS, Kind::Stop)).unwrap();
    line.book(refused(item(2_000 * MS, 1, 2_000 * MS, Kind::Start))).unwrap();
    assert!(line.streaming(), "a refused start leaves the stream on");
    assert_eq!(planned(&line), [(0, Some(1_000 * MS))], "the refused segment is in no plan");
    line.book(item(3_000 * MS, 2, 3_000 * MS, Kind::Stop)).unwrap();
    assert!(!line.streaming(), "a Stop after the refusal stops the stream");
    line.book(item(3_000 * MS, 3, 3_000 * MS, Kind::Start)).unwrap();
    assert!(line.streaming(), "the next start_rx starts it");
    assert_eq!(planned(&line), [(0, Some(1_000 * MS)), (3_050 * MS, None)]);
}

/// RM-25 (design 43), O-eq: on a generated sequence, refusing one `start_rx` changes
/// no segment that a later item begins (origin and cut), and leaves the refused one out of the
/// plan; the refusal booked with the item and recorded after it plan the same. A `start_rx`
/// that follows the refused item before any change resumes the halted stream, which is the one
/// segment a refusal does add, so those sequences are not compared.
#[test]
fn rm_25_a_refusal_does_not_move_a_later_origin() {
    let (mut compared, mut skipped) = (0, 0);
    for seed in 0..1_000 {
        let sequence = generator::sequence(seed);
        for stream in refusal_streams(Direction::Rx) {
            let terms = Terms { t0: 2_000 * MS, root_hz: 1_000_000_000, stream };
            let mut items = sequence.items(&terms);
            items.sort_by_key(|item| (item.ready, item.seq));
            let candidates: Vec<u64> = items.iter().filter(|item| item.kind == Kind::Start).map(|item| item.seq).collect();
            if candidates.is_empty() {
                continue;
            }
            let k = candidates[seed as usize % candidates.len()];
            if start_follows(&items, k) {
                skipped += 1;
                continue;
            }
            let plain = plan(&stream, &items).unwrap();
            let marked: Vec<Item> = items.iter().map(|item| Item { refused: item.refused || item.seq == k, ..*item }).collect();
            let booked = plan(&stream, &marked).unwrap();
            // The refused one is not in the plan, and every other segment is where it was.
            assert!(booked.iter().all(|segment| segment.by != Some(k)), "seed {seed}: the refused segment is planned: {booked:?}");
            let others: Vec<Segment> = plain.iter().filter(|segment| segment.by != Some(k)).copied().collect();
            assert_eq!(booked, others, "seed {seed}: refusing {k} moves a later segment");
            check(&stream, &booked, seed);
            // Recorded after the booking, the refusal plans the same, and at every instant the plan
            // and `streaming()` agree.
            let (mut full, mut after) = (Line::new(stream), Line::new(stream));
            let mut direct = Line::new(stream);
            for item in &items {
                full.book(*item).unwrap();
                after.book(*item).unwrap();
                direct.book(Item { refused: item.seq == k, ..*item }).unwrap();
            }
            assert!(after.refuse(k).unwrap());
            assert_eq!(after.plan, direct.plan, "seed {seed}: a refusal recorded after the booking plans differently");
            for item in &in_order(&items) {
                let (after_t, direct_t) = (prefix(&stream, &after.items, item.e), prefix(&stream, &direct.items, item.e));
                assert_eq!(planned(&after_t), planned(&direct_t), "seed {seed}, t {}", item.e);
                assert_eq!(after_t.streaming(), direct_t.streaming(), "seed {seed}, t {}", item.e);
            }
            compared += 1;
        }
    }
    assert!(compared > 500, "{compared} compared, {skipped} skipped");
}

/// RM-25 (design 43): a refused `start_rx` is no segment of the plan, however the line learned of
/// the refusal.
#[test]
fn rm_25_a_refused_receive_segment_is_not_in_the_plan() {
    let rx = stream(Direction::Rx, 1);
    for items in [
        vec![item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), refused(item(2_000 * MS, 1, 2_000 * MS, Kind::Start))],
        vec![item(500 * MS, 0, 500 * MS, Kind::Stop), refused(item(1_000 * MS, 1, 1_000 * MS, Kind::Start)), item(2_000 * MS, 2, 2_000 * MS, Kind::Start)],
        // The refused segment is cut by a later Stop (its cut falls after its origin) and the next start follows.
        vec![item(500 * MS, 0, 500 * MS, Kind::Stop), refused(item(1_000 * MS, 1, 1_000 * MS, Kind::Start)), item(3_000 * MS, 2, 3_000 * MS, Kind::Stop), item(4_000 * MS, 3, 4_000 * MS, Kind::Start)],
    ] {
        let booked = {
            let mut line = Line::new(rx);
            for item in &items {
                line.book(*item).unwrap();
            }
            line
        };
        let after = {
            let mut line = Line::new(rx);
            for item in &items {
                line.book(Item { refused: false, ..*item }).unwrap();
            }
            for item in items.iter().filter(|item| item.refused) {
                assert!(line.refuse(item.seq).unwrap());
            }
            line
        };
        for line in [&booked, &after] {
            for item in items.iter().filter(|item| item.refused) {
                assert!(line.plan.iter().all(|segment| segment.by != Some(item.seq)), "{:?}", line.plan);
            }
            assert!(line.plan.iter().all(|segment| segment.cut != Some(0)), "{:?}", line.plan);
        }
        assert_eq!(booked.plan, after.plan);
        for item in &items {
            let (booked_t, after_t) = (prefix(&rx, &booked.items, item.e), prefix(&rx, &after.items, item.e));
            assert_eq!(planned(&booked_t), planned(&after_t), "at {}", item.e);
            assert_eq!(booked_t.streaming(), after_t.streaming(), "at {}", item.e);
        }
    }
}

/// RM-25 (design 43), the exact values: a `Stop`, a refused `start_rx`, the next `start_rx`, and
/// the rest of the refusal's overlaps. Lead 50 ms, no receive call.
#[test]
fn rm_25_a_refused_segment_runs_for_planning_until_the_next_start_cuts_it() {
    let rx = stream(Direction::Rx, 1);
    let stop = item(500 * MS, 0, 500 * MS, Kind::Stop);
    let case = |name: &str, items: &[Item], expected: &[(i64, Option<i64>)]| {
        let mut line = Line::new(rx);
        for item in items {
            line.book(*item).unwrap();
        }
        assert_eq!(planned(&line), expected, "{name}");
        assert_eq!(segments(&rx, items), expected, "{name}");
        line
    };
    // A Stop cuts the refused virtual segment at its own instant (cut 2 000 ms), and the start after
    // it begins a lead later.
    let items = [stop, refused(item(1_000 * MS, 1, 1_000 * MS, Kind::Start)), item(2_000 * MS, 2, 2_000 * MS, Kind::Stop), item(2_000 * MS, 3, 2_000 * MS, Kind::Start)];
    case("Stop, S1 refused, Stop, S2", &items, &[(0, Some(500 * MS)), (2_050 * MS, None)]);
    // The refusal holds until S2 takes effect: by 1 999 ms the plan has the stop's segment alone.
    assert_eq!(planned(&prefix(&rx, &items, 1_999 * MS)), [(0, Some(500 * MS))], "halted until S2 takes effect");
    assert!(prefix(&rx, &items, 1_999 * MS).streaming(), "a refused start leaves the stream on");
    assert_eq!(planned(&prefix(&rx, &items, 2_000 * MS)), [(0, Some(500 * MS)), (2_050 * MS, None)], "S2 begins the segment a lead later");
    case(
        "S2 inside S1's lead",
        &[stop, refused(item(1_000 * MS, 1, 1_000 * MS, Kind::Start)), item(1_010 * MS, 2, 1_010 * MS, Kind::Stop), item(1_010 * MS, 3, 1_010 * MS, Kind::Start)],
        &[(0, Some(500 * MS)), (1_100 * MS, None)],
    );
    case(
        "two refusals",
        &[
            stop,
            refused(item(1_000 * MS, 1, 1_000 * MS, Kind::Start)),
            item(2_000 * MS, 2, 2_000 * MS, Kind::Stop),
            refused(item(2_000 * MS, 3, 2_000 * MS, Kind::Start)),
            item(3_000 * MS, 4, 3_000 * MS, Kind::Stop),
            item(3_000 * MS, 5, 3_000 * MS, Kind::Start),
        ],
        &[(0, Some(500 * MS)), (3_050 * MS, None)],
    );
    case(
        "S2 at S1's instant",
        &[stop, refused(item(1_000 * MS, 1, 1_000 * MS, Kind::Start)), item(1_000 * MS, 2, 1_000 * MS, Kind::Stop), item(1_000 * MS, 3, 1_000 * MS, Kind::Start)],
        &[(0, Some(500 * MS)), (1_100 * MS, None)],
    );
}

/// RM-25 (design 43), a Stop and `start_rx` after a refusal: the refused segment is cut where the
/// Stop is, and the new one begins a lead (and, with a receive call, the call) after that cut.
#[test]
fn rm_25_a_start_after_a_refusal_begins_from_the_refused_segment_s_cut() {
    let rx = stream(Direction::Rx, 1);
    let stop = item(500 * MS, 0, 500 * MS, Kind::Stop);
    let case = |stream: &Stream, items: &[Item], expected: &[(i64, Option<i64>)]| {
        assert_eq!(segments(stream, items), expected);
        let mut line = Line::new(*stream);
        for item in items {
            line.book(*item).unwrap();
        }
        assert_eq!(planned(&line), expected);
    };
    let refused_start = refused(item(1_000 * MS, 1, 1_000 * MS, Kind::Start));
    let after = |first: &[Item], e: i64| [first, &[item(e, 2, e, Kind::Stop), item(e, 3, e, Kind::Start)]].concat();
    case(&rx, &after(&[stop, refused_start], 1_500 * MS), &[(0, Some(500 * MS)), (1_550 * MS, None)]);
    // A Stop before the refused segment's origin cuts it with no sample, at that origin.
    case(&rx, &after(&[stop, refused_start], 1_010 * MS), &[(0, Some(500 * MS)), (1_100 * MS, None)]);
    // A refused start_rx is cut by the next Stop.
    case(&rx, &after(&[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), refused(item(2_000 * MS, 1, 2_000 * MS, Kind::Start))], 2_500 * MS), &[(0, Some(1_000 * MS)), (2_550 * MS, None)]);
    // With a receive call of 2 000 samples (2 ms) and 3 ms: the refused segment is cut at 1 500, and
    // the call it had in progress is 2 ms and the allowance 3: the new segment begins at 1 555 ms.
    let call = Stream { call: 2_000, allowance: 3 * MS, ..rx };
    case(&call, &after(&[stop, refused_start], 1_500 * MS), &[(0, Some(500 * MS)), (1_555 * MS, None)]);
}

/// RM-25 (design 43): a booking at `b` adds no coverage before `b` plus its stream's start lead,
/// whether or not some of the items are refused: it begins no segment that early and moves no
/// origin earlier. The boundaries tested are every origin and cut instant of the plan before and
/// after, and the ones around `b` and `b + lead`.
#[test]
fn rm_25_a_booking_adds_no_coverage_before_its_start_lead() {
    // RM-36: a segment covers `t` from its origin and, if cut, until the instant of its cut sample.
    let covers = |plan: &[Segment], channel: u32, t: i64| {
        plan.iter().any(|segment| segment.config.channels.contains(channel) && segment.origin <= t && segment.cut.is_none_or(|cut| segment.instant(cut).is_ok_and(|end| end > t)))
    };
    let mut booked_total = 0;
    for seed in 0..400 {
        let sequence = generator::sequence(seed);
        let from = Stream { origin: 2_000 * MS, call: 2_000, allowance: 3 * MS, ..rx_stream(&[0, 1], 1_000_000) };
        let terms = Terms { t0: 2_000 * MS, root_hz: 1_000_000_000, stream: from };
        let mut items = sequence.items(&terms);
        items.sort_by_key(|item| (item.ready, item.seq));
        let mut line = Line::new(from);
        for booked in &items {
            let b = booked.ready;
            let before = line.clone();
            line.book(Item { refused: booked.seq % 5 == 4, ..*booked }).unwrap();
            booked_total += 1;
            let mut boundaries: Vec<i64> = Vec::new();
            for line in [&before, &line] {
                for segment in &line.plan {
                    boundaries.push(segment.origin);
                    if let Some(cut) = segment.cut {
                        boundaries.push(segment.instant(cut).unwrap());
                    }
                }
            }
            let mut instants: Vec<i64> = boundaries.iter().flat_map(|x| [x - 1, *x, x + 1]).collect();
            instants.extend([b - 1, b, b + from.lead - 1]);
            for t in instants.into_iter().filter(|t| *t < b + from.lead) {
                for channel in 0..3 {
                    assert!(
                        !covers(&line.plan, channel, t) || covers(&before.plan, channel, t),
                        "seed {seed}: booking {booked:?} at {b} added coverage of channel {channel} at {t}: {:?} -> {:?}",
                        before.plan,
                        line.plan,
                    );
                }
            }
        }
    }
    assert!(booked_total > 1_000, "{booked_total} bookings");
}

/// RM-16, RM-25 (design 43): a real segment keeps its delivered floor; a virtual one has none.
#[test]
fn rm_16_a_real_segment_keeps_its_delivered_floor() {
    let rx = stream(Direction::Rx, 1);
    // A restart runs from 1 050 ms; an abort at 1 500 ms that knows 600 000 samples delivered
    // cuts there, not at 450 000; a Stop's cut is floored at what was delivered.
    let run = |end: Item| segments(&rx, &[item(1_000 * MS, 0, 500 * MS, Kind::Stop), item(1_000 * MS, 1, 500 * MS, Kind::Start), end]);
    assert_eq!(run(delivered(600_000, item(1_500 * MS, 2, 1_500 * MS, Kind::End { abort: true }))), [(0, Some(1_000 * MS)), (1_050 * MS, Some(1_650 * MS))]);
    assert_eq!(run(delivered(600_000, item(1_500 * MS, 2, 1_500 * MS, Kind::Stop))), [(0, Some(1_000 * MS)), (1_050 * MS, Some(1_650 * MS))]);
    assert_eq!(run(delivered(100_000, item(1_500 * MS, 2, 1_500 * MS, Kind::Stop))), [(0, Some(1_000 * MS)), (1_050 * MS, Some(1_500 * MS))]);
}

/// RM-25 (design 43): a pruned line plans and halts as one that keeps every item, with the refused
/// segment pending (before its origin) or running (after it) at the prune.
#[test]
fn rm_25_a_pruned_line_with_a_refused_segment_plans_as_one_that_keeps_every_item() {
    for prune_at in [1_020 * MS, 1_200 * MS] {
        for step in 0..3 {
            let mut full = Line::new(rx_stream(&[0, 1], 1_000_000));
            full.book(item(500 * MS, 0, 500 * MS, Kind::Stop)).unwrap();
            full.book(refused(item(1_000 * MS, 1, 1_000 * MS, Kind::Start))).unwrap();
            let mut pruned = full.clone();
            pruned.prune(prune_at);
            assert!(pruned.items.is_empty(), "the refused item is folded away at {prune_at}");
            let next = match step {
                0 => item(1_500 * MS, 2, 1_500 * MS, Kind::Stop),
                1 => item(1_500 * MS, 2, 1_500 * MS, Kind::Start),
                _ => item(1_500 * MS, 2, 1_500 * MS, Kind::Stop),
            };
            for line in [&mut full, &mut pruned] {
                line.book(next).unwrap();
                if step == 2 {
                    line.book(item(1_600 * MS, 3, 1_600 * MS, Kind::Start)).unwrap();
                }
            }
            // The pruned line, booked by `t`: the items before the prune, the prune, then the rest.
            let pruned_by = |t: i64| {
                let (early, late): (Vec<Item>, Vec<Item>) = full.items.iter().partition(|item| item.e < prune_at);
                let mut line = Line::new(rx_stream(&[0, 1], 1_000_000));
                for item in early.iter().filter(|item| item.e <= t) {
                    line.book(*item).unwrap();
                }
                line.prune(prune_at);
                for item in late.iter().filter(|item| item.e <= t) {
                    line.book(*item).unwrap();
                }
                line
            };
            for t in [1_000 * MS, 1_050 * MS, 1_400 * MS, 1_500 * MS, 1_700 * MS, 2_000 * MS] {
                let full_t = prefix(&rx_stream(&[0, 1], 1_000_000), &full.items, t);
                let pruned_t = pruned_by(t);
                assert_eq!(planned(&pruned_t), planned(&full_t), "prune {prune_at}, step {step}, t {t}");
                assert_eq!(pruned_t.streaming(), full_t.streaming(), "prune {prune_at}, step {step}, t {t}");
            }
            assert_eq!(planned(&pruned), planned(&full), "prune {prune_at}, step {step}");
        }
    }
}

// ---------------------------------------------------------------- note 47 stage 3

#[test]
fn rm_21_streaming_is_the_final_state_of_the_replayed_line() {
    // RM-21: a receive stream is streaming when its line, replayed in the order its commands take effect, ends on: a booked `start_rx` makes
    // it so at its effective instant, and a `Stop` booked later that takes effect before that instant does not cancel it.
    let stream = rx_stream(&[0], 1_000_000);
    let line_of = |items: &[Item]| {
        let mut line = Line::new(stream);
        for item in items {
            line.book(*item).unwrap();
        }
        line
    };
    let stop = |e, seq| item(e, seq, e, Kind::Stop);
    let start = |e, seq, ready| item(e, seq, ready, Kind::Start);
    assert!(Line::new(stream).streaming(), "the Run starts streaming");
    assert!(!line_of(&[stop(100 * MS, 1)]).streaming());
    assert!(line_of(&[stop(100 * MS, 1), start(100 * MS, 2, 100 * MS)]).streaming());
    // A `Stop` booked after a start that takes effect later does not cancel it.
    assert!(line_of(&[stop(100 * MS, 1), start(1_100 * MS, 2, 100 * MS), stop(200 * MS, 3)]).streaming());
    // A `Stop` that takes effect after the start does.
    assert!(!line_of(&[stop(100 * MS, 1), start(1_100 * MS, 2, 100 * MS), stop(1_200 * MS, 3)]).streaming());
    // A start the device refused leaves the stream streaming (RM-25's halt).
    assert!(line_of(&[stop(100 * MS, 1), refused(start(100 * MS, 2, 100 * MS))]).streaming());
    // A stream that holds no channel streams nothing: a start of it is accepted and carried out by no one.
    assert!(!Line::new(rx_stream(&[], 1_000_000)).streaming());
    // A pruned line answers as one that keeps every item.
    let items = [stop(100 * MS, 1), start(1_100 * MS, 2, 100 * MS), stop(200 * MS, 3)];
    let mut pruned = line_of(&items);
    pruned.prune(150 * MS);
    assert_eq!(pruned.streaming(), line_of(&items).streaming());
}

#[test]
fn rm_25_lead_left_reports_a_configure_that_left_less_than_the_declared_lead() {
    use ezsdr_radio::timeline::lead_left;
    let min_lead = 5 * MS;
    let origin = 100 * MS;
    // The boundary: exactly the declared lead is whole.
    assert_eq!(lead_left(origin, origin - min_lead, min_lead), None);
    assert_eq!(lead_left(origin, origin - min_lead + 1, min_lead), Some(min_lead - 1));
    assert_eq!(lead_left(origin, origin - 1, min_lead), Some(1));
    // At or after the origin the start is already missed, and the device's own late report owns it.
    assert_eq!(lead_left(origin, origin, min_lead), None);
    assert_eq!(lead_left(origin, origin + 7 * MS, min_lead), None);
    assert_eq!(lead_left(origin, 0, min_lead), None);
    // A configure that ended 25 ms before the origin leaves the declared 5 ms lead whole, whatever the 50 ms start lead: the threshold is
    // the declared command lead, not the start lead.
    assert_eq!(lead_left(origin, origin - 25 * MS, min_lead), None);
}

/// The terms the generated sequences' oracle is read with: the instants only (`Terms::ticks`).
fn oracle_terms() -> Terms {
    Terms { t0: 2_000 * MS, root_hz: 1_000_000_000, stream: Stream { origin: 2_000 * MS, call: 2_000, allowance: 3 * MS, ..stream(Direction::Rx, 1) } }
}

#[test]
fn rm_26_every_class_generates_its_operations() {
    // No class generates empty sequences, and the value class generates both outcomes of a `Set` and the orders that tell the oracle's rule from an
    // arrival-order flag (RM-21).
    use generator::{Class, Op};
    let terms = oracle_terms();
    let mut counts: std::collections::BTreeMap<Class, usize> = std::collections::BTreeMap::new();
    let (mut accepted, mut refused, mut snapshots, mut overwritten, mut later_stop, mut future_start_then_stop) = (0, 0, 0, 0, 0, 0);
    let (mut starts_refused, mut earlier_after_booked_ahead) = (0, 0);
    for seed in 0..1_000 {
        let sequence = generator::sequence(seed);
        assert!(sequence.ops().count() > 0 || !sequence.faults.is_empty(), "seed {seed}: an empty sequence");
        *counts.entry(sequence.class).or_default() += 1;
        let staged = sequence.staged(&terms, |_| true);
        let ops: Vec<&Op> = sequence.ops().map(|(_, op)| op).collect();
        let sets = ops.iter().filter(|op| matches!(op, Op::Set { .. })).count();
        refused += staged.refused.len();
        starts_refused += staged.refused_starts.len();
        accepted += sets - staged.refused.len();
        snapshots += staged.starts.iter().filter(|(_, snapshot)| *snapshot != (None, None)).count();
        let accepted_frequencies = ops.iter().enumerate().filter(|(i, op)| matches!(op, Op::Set { leaf: generator::Leaf::Frequency, .. }) && !staged.refused.contains(i)).count();
        overwritten += usize::from(accepted_frequencies >= 2 && staged.starts.iter().any(|(_, snapshot)| snapshot.0.is_some()));
        // A `Set` accepted after a second `Stop` that follows a `Stop`.
        later_stop +=
            usize::from(ops.iter().enumerate().any(|(i, op)| matches!(op, Op::Set { .. }) && !staged.refused.contains(&i) && ops[..i].iter().filter(|o| matches!(o, Op::Stop { .. })).count() >= 2));
        // The orders an arrival-order flag gets wrong: a `Stop` booked after a `start_rx` that takes effect later.
        let schedule = sequence.schedule(&terms);
        let effective = |index: usize| schedule.iter().find(|entry| entry.event == generator::Event::Op(index)).map(|entry| (entry.e, entry.seq)).unwrap();
        // A refused `start_rx` that would have taken effect before a start accepted earlier and not begun yet: the line would have kept the first.
        earlier_after_booked_ahead += usize::from(staged.refused_starts.iter().any(|b| staged.starts.iter().any(|(a, _)| a < b && effective(*a).0 > effective(*b).0)));
        future_start_then_stop +=
            usize::from((0..ops.len()).any(|a| matches!(ops[a], Op::StartRx { .. }) && (a + 1..ops.len()).any(|b| matches!(ops[b], Op::Stop { .. }) && effective(b) < effective(a))));
    }
    for class in CLASSES {
        assert!(counts.get(&class).copied().unwrap_or(0) >= 100, "{class:?}: {counts:?}");
    }
    assert!(accepted >= 100 && refused >= 100, "accepted {accepted}, refused {refused}");
    assert!(snapshots >= 50, "starts that take a value other than the Run's: {snapshots}");
    assert!(starts_refused >= 100, "start_rx commands the stream refuses as streaming: {starts_refused}");
    assert!(earlier_after_booked_ahead >= 20, "a refused start that would have begun before an accepted, not yet begun one: {earlier_after_booked_ahead}");
    assert!(overwritten >= 20 && later_stop >= 20, "two values before one start: {overwritten}, a value after a second Stop: {later_stop}");
    assert!(future_start_then_stop >= 50, "a `Stop` takes effect before an earlier-booked start: {future_start_then_stop}");
}

#[test]
fn rm_26_the_staging_oracle_agrees_with_staging() {
    // The oracle (generator, written apart) and the pure types (`Staging` over `Line::streaming`) decide each `Set` of 1 000 sequences alike and
    // give each start the same values.
    use ezsdr_kernel::spec::{Key, Value};
    use ezsdr_radio::staging::Staging;
    use generator::{Leaf, Op};
    let terms = oracle_terms();
    let key = |name: &str| Key::parse(name).unwrap();
    let mut disagreements = Vec::new();
    for seed in 0..1_000 {
        let sequence = generator::sequence(seed);
        let oracle = sequence.staged(&terms, |_| true);
        let schedule = sequence.schedule(&terms);
        let ops: Vec<&Op> = sequence.ops().map(|(_, op)| op).collect();
        let config =
            [(key("radio.rx.c0.stream"), Value::from(0)), (key("radio.rx.c0.frequency_hz"), Value::num(1.0e9).unwrap()), (key("radio.rx.c0.gain_db"), Value::num(0.0).unwrap())].into_iter().collect();
        let mut staging = Staging::new(config);
        let mut line = Line::new(terms.stream);
        let (mut refused, mut starts, mut refused_starts) = (Vec::new(), Vec::new(), Vec::new());
        for (index, op) in ops.iter().enumerate() {
            let entry = schedule.iter().find(|entry| entry.event == generator::Event::Op(index)).unwrap();
            match op {
                Op::Stop { .. } => {
                    line.book(item(entry.e, entry.seq, entry.receipt, Kind::Stop)).unwrap();
                }
                Op::StartRx { .. } => {
                    // RM-21: the shared decision refuses a start on a stream that is streaming, and the line is not changed by it.
                    let Ok(snapshot) = staging.start(0, line.streaming()) else {
                        refused_starts.push(index);
                        continue;
                    };
                    line.book(item(entry.e, entry.seq, entry.receipt, Kind::Start)).unwrap();
                    let value = |name: &str, initial: f64, leaf: Leaf| -> Option<u8> {
                        let held = match snapshot[&key(name)] {
                            Value::Scalar(ezsdr_kernel::spec::Scalar::Num(value)) => value.get(),
                            ref other => panic!("{other:?}"),
                        };
                        (held != initial).then(|| (0..8u8).find(|v| leaf.value(*v) == held).expect("a generated value"))
                    };
                    // `v = 0` of a gain is the Run's own value, which the oracle calls "the value 0": compare through the number.
                    starts.push((index, (value("radio.rx.c0.frequency_hz", 1.0e9, Leaf::Frequency), value("radio.rx.c0.gain_db", 0.0, Leaf::Gain))));
                    continue;
                }
                Op::Set { leaf, v } => {
                    let name = match leaf {
                        Leaf::Frequency => "radio.rx.c0.frequency_hz",
                        Leaf::Gain => "radio.rx.c0.gain_db",
                    };
                    if staging.set(&key(name), Value::num(leaf.value(*v)).unwrap(), false, 0, |_| line.streaming(), |_| ezsdr_radio::staging::Busy::default()).is_err() {
                        refused.push(index);
                    }
                }
            }
        }
        // The Run's own frequency is 1 GHz, the value 0 of `Leaf::Frequency`, and its gain 0 dB, the value 0 of `Leaf::Gain`: both read as `None`.
        let normal = |snapshot: generator::Snapshot| (snapshot.0.filter(|v| *v != 0), snapshot.1.filter(|v| *v != 0));
        let oracle_starts: Vec<_> = oracle.starts.iter().map(|(index, snapshot)| (*index, normal(*snapshot))).collect();
        if oracle.refused != refused || oracle.refused_starts != refused_starts || oracle_starts != starts {
            disagreements.push(format!("seed {seed}: oracle {oracle:?}, staging refused {refused:?} refused starts {refused_starts:?} starts {starts:?}"));
        }
    }
    assert!(disagreements.is_empty(), "{} sequences differ, the first: {:#?}", disagreements.len(), &disagreements[..disagreements.len().min(5)]);
}
