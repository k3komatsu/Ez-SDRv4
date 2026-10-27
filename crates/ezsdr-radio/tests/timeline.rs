//! RM-25: `ezsdr_radio::timeline`, RM-16, RM-21 and RM-25 computed once (spec 22, VH-7),
//! and its invariants over VH-8's generated sequences.

mod generator;

use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::stream::Direction;
use ezsdr_kernel::time::Rational;
use ezsdr_radio::stream_target;
use ezsdr_radio::timeline::{
    Channels, Config, Item, Kind, Line, Lines, MoveError, Segment, Stream, StreamKey, NO_RELEASE, covering, plan,
};
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

/// A receive stream of `numbers` at `rate` from 0 with no lead: the source of the 1 Hz cases,
/// where a `Stop`'s cut is a whole second away from its instant.
fn rx_bare(numbers: &[u32], rate: u64) -> Stream {
    Stream { lead: 0, ..rx_stream(numbers, rate) }
}

fn item(e: i64, seq: u64, ready: i64, kind: Kind) -> Item {
    Item { e, seq, ready, release: NO_RELEASE, delivered: None, refused: false, kind }
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

/// The instance's source and destination streams (RM-29).
const FROM: StreamKey = (Direction::Rx, 0);
const TO: StreamKey = (Direction::Rx, 1);

/// One instance's lines: `from` in stream `FROM` and `to` in `TO`, each planned.
fn lines_of(from: Stream, to: Stream) -> Lines {
    let mut lines = Lines::new();
    lines.insert(FROM, Line::new(from));
    lines.insert(TO, Line::new(to));
    lines
}

/// One of an instance's streams, planned like [`planned`].
fn planned_at(lines: &Lines, stream: StreamKey) -> Vec<(i64, Option<i64>)> {
    planned(lines.line(stream).unwrap_or_else(|| panic!("no line for {stream:?}")))
}

/// One of an instance's streams, planned (RM-25), for an assertion's message.
fn lanes(lines: &Lines, stream: StreamKey) -> Vec<(i64, Option<i64>)> {
    planned_at(lines, stream)
}

/// RM-25: whether one of an instance's streams is halted at `t`.
fn line_halted(lines: &Lines, stream: StreamKey, t: i64) -> bool {
    lines.line(stream).unwrap_or_else(|| panic!("no line for {stream:?}")).halted(t)
}

/// The segment an instance's stream planned for the booking `seq` (RM-25's `by`).
fn by(lines: &Lines, stream: StreamKey, seq: u64) -> Segment {
    let line = lines.line(stream).unwrap_or_else(|| panic!("no line for {stream:?}"));
    *line.plan.iter().find(|segment| segment.by == Some(seq)).unwrap_or_else(|| panic!("no segment for {seq}: {:?}", line.plan))
}

/// RM-34: the instant one of an instance's streams releases `channel` for its move `seq`.
fn released(lines: &Lines, stream: StreamKey, channel: u32, seq: u64) -> Option<i64> {
    lines.line(stream).unwrap_or_else(|| panic!("no line for {stream:?}")).released(channel, seq)
}

#[test]
fn rm_26_each_stream_takes_its_own_directions_start_lead() {
    // RM-4, RM-25: the TimingEnvelope has one start lead per direction, and a stream's new
    // segment uses its own (design 33 D5).
    let timing = ezsdr_radio::TimingEnvelope {
        min_timed_command_lead_ns: 0,
        startup_latency_ns: 0,
        command_queue_depth: 0,
        release_window_ns: 0,
        overflow_restart_gap_ns: 0,
        rx_start_lead_ns: 50 * MS,
        tx_start_lead_ns: 20 * MS,
        tx_in_flight_ns: 0,
    };
    assert_eq!(ezsdr_radio::timeline::start_lead_ns(&timing, Direction::Rx), 50 * MS);
    assert_eq!(ezsdr_radio::timeline::start_lead_ns(&timing, Direction::Tx), 20 * MS);

    // A restart after a cut starts its own direction's lead after that cut, at 1 MS/s.
    for (direction, lead) in [(Direction::Rx, 50 * MS), (Direction::Tx, 20 * MS)] {
        let stream = Stream {
            direction,
            origin: 0,
            config: config(1, 1_000_000),
            lead: ezsdr_radio::timeline::start_lead_ns(&timing, direction),
            call: 0,
            allowance: 0,
        };
        assert_eq!(
            segments(&stream, &[item(1_000 * MS, 0, 1_000 * MS, Kind::Cold(config(1, 1_000_000)))]),
            [(0, Some(1_000 * MS)), (1_000 * MS + lead, None)],
            "{direction:?}",
        );
    }
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
fn rm_15_preemption_front_applies_each_late_policy() {
    use ezsdr_kernel::id::ClockDomainId;
    use ezsdr_kernel::stream::{LateOutcome, LatePolicy};
    use ezsdr_kernel::time::Duration;
    use ezsdr_radio::timeline::late_policy_front_outcome;

    let late_by = Duration::new(ClockDomainId::HOST_MONOTONIC, 2);
    assert_eq!(late_policy_front_outcome(LatePolicy::RejectAtPlan, 9, 10, Some(11), |_| Ok::<_, ()>(late_by)).unwrap(),
        Some(LateOutcome::PlanViolation { late_by }));
    assert_eq!(late_policy_front_outcome(LatePolicy::SendAsapAndFlag, 9, 10, Some(11), |_| Ok::<_, ()>(late_by)).unwrap(),
        Some(LateOutcome::SendAsap { late_by }));
    assert_eq!(late_policy_front_outcome(LatePolicy::DropAndFlag, 9, 10, Some(11), |_| Ok::<_, ()>(late_by)).unwrap(),
        Some(LateOutcome::Drop { late_by }));
    assert_eq!(late_policy_front_outcome(LatePolicy::DropAndFlag, 11, 10, Some(11), |_| Ok::<_, ()>(late_by)).unwrap(), None,
        "equality with the front is on time");
    assert_eq!(late_policy_front_outcome(LatePolicy::DropAndFlag, 9, 11, Some(10), |_| Ok::<_, ()>(late_by)).unwrap(), None,
        "a front at or before the ordinary bound adds no lateness");
}

#[test]
fn rm_26_the_timeline_cases() {
    let rx = stream(Direction::Rx, 1);
    let tx = stream(Direction::Tx, 1);
    let fast = Kind::Cold(config(1, 2_000_000));
    let case = |name: &str, stream: &Stream, items: &[Item], expected: &[(i64, Option<i64>)]| {
        assert_eq!(segments(stream, items), expected, "{name}");
    };
    // Appendix C's eleven cases, at 1 MS/s on a 1 GHz root with a 50 ms lead.
    case("a cold change after a Stop starts nothing (#46)", &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), item(2_000 * MS, 1, 2_000 * MS, fast)],
        &[(0, Some(1_000 * MS))]);
    case("a second Stop keeps the first end (VG-1)", &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), item(2_000 * MS, 1, 2_000 * MS, Kind::Stop)],
        &[(0, Some(1_000 * MS))]);
    case("a start that arrived first, at 2 s, restarts after a Stop at 1 s", &rx,
        &[item(2_000 * MS, 0, 500 * MS, Kind::Start), item(1_000 * MS, 1, 1_000 * MS, Kind::Stop)],
        &[(0, Some(1_000 * MS)), (2_000 * MS, None)]);
    case("stop(); start() restarts a lead after the cut", &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), item(1_000 * MS, 1, 1_000 * MS, Kind::Start)],
        &[(0, Some(1_000 * MS)), (1_050 * MS, None)]);
    case("start(); stop() does not restart", &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Start), item(1_000 * MS, 1, 1_000 * MS, Kind::Stop)],
        &[(0, Some(1_000 * MS))]);
    case("0 channels and back at once starts a lead after the cut (#49)", &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Cold(config(0, 1_000_000))),
          item(1_000 * MS, 1, 1_000 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(0, Some(1_000 * MS)), (1_050 * MS, None)]);
    case("a Stop before a pending enable from 0 starts nothing (#52)", &stream(Direction::Rx, 0),
        &[item(2_000 * MS, 0, 500 * MS, Kind::Cold(config(1, 1_000_000))), item(1_000 * MS, 1, 1_000 * MS, Kind::Stop)],
        &[]);
    case("two quick changes give one restart and no empty clock (#53)", &rx,
        &[item(1_000 * MS, 0, 900 * MS, fast), item(1_010 * MS, 1, 905 * MS, Kind::Cold(config(2, 2_000_000)))],
        &[(0, Some(1_000 * MS)), (1_100 * MS, None)]);
    case("a Stop between e1 and e2 leaves no clock (#53)", &rx,
        &[item(1_000 * MS, 0, 900 * MS, fast), item(1_020 * MS, 1, 1_020 * MS, Kind::Stop)],
        &[(0, Some(1_000 * MS))]);
    case("an enable 500 ms ahead starts at its instant (VF-6)", &stream(Direction::Rx, 0),
        &[item(1_500 * MS, 0, 1_000 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(1_500 * MS, None)]);
    case("an off-lattice Stop rounds up (#33)", &rx,
        &[item(1_000 * MS + 1, 0, 1_000 * MS + 1, Kind::Stop)],
        &[(0, Some(1_000 * MS + 1_000))]);
    case("a loss at a change's instant comes first, a fault being received first (#48)", &rx,
        &[item(1_000 * MS + 1, 1, 900 * MS, fast), item(1_000 * MS + 1, 0, 0, Kind::End { abort: false })],
        &[(0, Some(1_000 * MS + 1_000))]);
    case("a loss after a change's instant and before its origin leaves no new clock (#48)", &rx,
        &[item(1_000 * MS - 500, 1, 900 * MS, fast), item(1_000 * MS + 200, 0, 0, Kind::End { abort: false })],
        &[(0, Some(1_000 * MS))]);

    // A rate no whole number of root ticks: 3 MS/s is 1 000/3 ticks a sample, its lattice
    // 1 000 ticks (RM-25).
    let third = Stream { config: config(1, 3_000_000), lead: 0, ..rx };
    case("a cut on a fractional rate is its sample's instant, rounded up", &third,
        &[item(500, 0, 500, Kind::Stop)], &[(0, Some(667))]);
    // The cut is a sample index: the sample at 334 ticks is sample 2 (666.7), not sample 1,
    // whose instant rounds up to 334; read back from 667, it would be sample 3.
    let cut = plan(&third, &[item(334, 0, 334, Kind::Stop)]).unwrap()[0].cut;
    assert_eq!(cut, Some(2), "a fractional cut is its sample's index");
    case("a restart on a fractional rate starts on its lattice", &third,
        &[item(500, 0, 500, Kind::Cold(config(1, 3_000_000)))], &[(0, Some(667)), (1_000, None)]);

    // VH-4's ready term: one receive call at the old rate (2 000 samples at 400 kS/s, 5 ms)
    // and 3 ms after the cut, then the lead; an enable from no stream has no call.
    let x310 = Stream { config: config(1, 400_000), call: 2_000, allowance: 3 * MS, ..rx };
    case("a receive restart waits the call in progress at its cut", &x310,
        &[item(1_000 * MS, 0, 500 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(0, Some(1_000 * MS)), (1_058 * MS, None)]);
    case("a receive enable from no stream waits only the lead", &Stream { config: config(0, 400_000), ..x310 },
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(1_050 * MS, None)]);
    case("a transmit restart has no call term", &Stream { direction: Direction::Tx, ..x310 },
        &[item(1_000 * MS, 0, 500 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(0, Some(1_000 * MS)), (1_050 * MS, None)]);

    // The transmit side: a Stop ends bursts, not the clock; a change booked early still
    // waits the lead after its cut; an empty transmit clock is kept, ended at its origin.
    case("a transmit enable from no stream waits the lead after it is ready (VH-4)", &stream(Direction::Tx, 0),
        &[item(1_000 * MS, 0, 1_020 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(1_070 * MS, None)]);
    case("a transmit Stop ends no clock", &tx, &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop)], &[(0, None)]);
    case("transmit 0 channels and back, booked early, starts a lead after the cut", &tx,
        &[item(1_000 * MS, 0, 100 * MS, Kind::Cold(config(0, 1_000_000))),
          item(1_000 * MS, 1, 100 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(0, Some(1_000 * MS)), (1_050 * MS, None)]);
    case("a second transmit change before the first's origin cuts at that origin (#57)", &tx,
        &[item(1_000 * MS, 0, 500 * MS, fast), item(1_010 * MS, 1, 600 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(0, Some(1_000 * MS)), (1_050 * MS, Some(1_050 * MS)), (1_100 * MS, None)]);

    // VH-2: a refused change is removed and what follows replanned; it halts the stream
    // until the next cold change, and a transmit clock booked for it ends at its origin.
    case("a refused receive change halts the stream until the next change", &rx,
        &[refused(item(1_000 * MS, 0, 500 * MS, fast)), item(2_000 * MS, 1, 1_500 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(0, Some(1_000 * MS)), (2_000 * MS, None)]);
    case("a refused transmit change ends its clock at its origin", &tx,
        &[refused(item(1_000 * MS, 0, 500 * MS, fast)), item(2_000 * MS, 1, 1_500 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(0, Some(1_000 * MS)), (1_050 * MS, Some(1_050 * MS)), (2_000 * MS, None)]);
    case("a change within one lead of a refused one counts from the cut made, not the refused origin", &rx,
        &[refused(item(1_000 * MS, 0, 500 * MS, fast)), item(1_010 * MS, 1, 1_005 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(0, Some(1_000 * MS)), (1_055 * MS, None)]);
    case("start_rx resumes a stream a refused change halted", &rx,
        &[refused(item(1_000 * MS, 0, 500 * MS, fast)), item(1_500 * MS, 1, 1_500 * MS, Kind::Start)],
        &[(0, Some(1_000 * MS)), (1_550 * MS, None)]);
    case("a refused start_rx starts nothing; the next change resumes the stream", &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), refused(item(2_000 * MS, 1, 2_000 * MS, Kind::Start)),
          item(3_000 * MS, 2, 3_000 * MS, fast)],
        &[(0, Some(1_000 * MS)), (3_050 * MS, None)]);
    case("a refused start_rx starts nothing; the next start_rx does", &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), refused(item(2_000 * MS, 1, 2_000 * MS, Kind::Start)),
          item(3_000 * MS, 2, 3_000 * MS, Kind::Start)],
        &[(0, Some(1_000 * MS)), (3_050 * MS, None)]);
    case("a refused start_rx leaves the stream halted", &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::Stop), refused(item(2_000 * MS, 1, 2_000 * MS, Kind::Start))],
        &[(0, Some(1_000 * MS))]);

    // RM-16: every cut is at or after the first sample not yet delivered; an abort's is it.
    case("a Stop's cut is floored at what was delivered", &rx,
        &[delivered(1_002_000, item(1_000 * MS, 0, 1_000 * MS, Kind::Stop))], &[(0, Some(1_002 * MS))]);
    case("an abort cuts at the first sample not yet delivered", &rx,
        &[delivered(700_000, item(1_000 * MS, 0, 1_000 * MS, Kind::End { abort: true }))], &[(0, Some(700 * MS))]);
    case("an abort that knows nothing delivered cuts at its instant", &rx,
        &[item(1_000 * MS, 0, 1_000 * MS, Kind::End { abort: true }), item(1_500 * MS, 1, 1_500 * MS, Kind::Start)],
        &[(0, Some(1_000 * MS))]);

    // VH-2: a loss after a booked transmit change orphans its clock, which ends at its
    // origin; the receive side has no clock to end.
    let loss = |stream: &Stream| segments(stream, &[item(1_000 * MS, 1, 500 * MS, fast), item(1_020 * MS, 0, 0, Kind::End { abort: false })]);
    assert_eq!(loss(&tx), [(0, Some(1_000 * MS)), (1_050 * MS, Some(1_050 * MS))]);
    assert_eq!(loss(&rx), [(0, Some(1_000 * MS))]);
    case("a loss ends a running transmit stream's bursts, not its clock", &tx,
        &[item(1_000 * MS, 0, 0, Kind::End { abort: false }), item(1_500 * MS, 1, 1_500 * MS, fast)], &[(0, None)]);

    // VH-2: a `cold` change never takes effect before an earlier-arrived one of its stream,
    // its instant floored at that one's and, at it, after it, on both directions.
    let early = |e: i64, seq: u64, ready: i64, kind: Kind| item(e, seq, ready, kind);
    let late = [early(2_000 * MS, 0, 500 * MS, fast), early(1_000 * MS, 1, 600 * MS, Kind::Cold(config(1, 1_000_000)))];
    assert_eq!(ezsdr_radio::timeline::effective(&late, &late[1]), 2_000 * MS);
    assert_eq!(ezsdr_radio::timeline::effective(&late, &late[0]), 2_000 * MS);
    case("a receive change that arrives after a later one follows it", &rx, &late,
        &[(0, Some(2_000 * MS)), (2_100 * MS, None)]);
    case("a transmit change that arrives after a later one follows it", &tx, &late,
        &[(0, Some(2_000 * MS)), (2_050 * MS, Some(2_050 * MS)), (2_100 * MS, None)]);
    assert_eq!(plan(&rx, &late).unwrap()[1].config, config(1, 1_000_000), "the later-arrived change is in force");
    assert_eq!(ezsdr_radio::timeline::effective(&late, &item(1_000 * MS, 2, 700 * MS, Kind::Stop)), 1_000 * MS, "a Stop is not floored");

    // VH-2: a transmit clock is recorded at booking, so a loss before a change booked ahead of
    // it keeps that change's cut and ends its clock at its origin; receive ends at the loss.
    let booked = [item(1_000 * MS, 1, 500 * MS, fast), item(800 * MS, 0, 0, Kind::End { abort: false })];
    case("a loss before a booked transmit change keeps its cut", &tx, &booked,
        &[(0, Some(1_000 * MS)), (1_050 * MS, Some(1_050 * MS))]);
    case("a loss before a booked receive change ends the stream at the loss", &rx, &booked, &[(0, Some(800 * MS))]);
    // A refused transmit change whose clock a change booked before the refusal already cut
    // keeps that cut.
    case("a transmit change booked before a refusal keeps its cut of the refused clock", &tx,
        &[refused(item(1_000 * MS, 0, 500 * MS, fast)), item(1_100 * MS, 1, 600 * MS, Kind::Cold(config(1, 1_000_000)))],
        &[(0, Some(1_000 * MS)), (1_050 * MS, Some(1_100 * MS)), (1_150 * MS, None)]);

    // The configuration a segment runs is its change's.
    let planned = plan(&rx, &[item(1_000 * MS, 0, 900 * MS, fast)]).unwrap();
    assert_eq!(planned[1].config, config(1, 2_000_000));
    // Each segment names the command that began it; the first, none.
    assert_eq!(planned.iter().map(|segment| segment.by).collect::<Vec<_>>(), [None, Some(0)]);
}

/// VH-8 on the timeline alone, `start_rx` and refusals included: on every generated
/// sequence, both streams' plans keep RM-16's and RM-25's invariants and do not depend on
/// the order the items are given in.
#[test]
fn rm_26_generated_sequences_keep_the_timeline_invariants() {
    let (mut seen, mut fractional) = (std::collections::BTreeSet::new(), 0);
    for seed in 0..1_000 {
        let sequence = generator::sequence(seed, &generator::FRACTIONAL_RATES);
        seen.insert(sequence.class);
        for direction in [Direction::Rx, Direction::Tx] {
            let stream = Stream { origin: 2_000 * MS, call: 2_000, allowance: 3 * MS, ..stream(direction, 1) };
            let terms = Terms { t0: 2_000 * MS, root_hz: 1_000_000_000, timed_lead: 2 * MS, release_window: 3 * MS, stream };
            let mut items = sequence.items(&terms);
            // Every fifth change or start refused at its instant (VH-2).
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
        let sequence = generator::sequence(seed, &generator::FRACTIONAL_RATES);
        for direction in [Direction::Rx, Direction::Tx] {
            let stream = Stream { origin: 2_000 * MS, call: 2_000, allowance: 3 * MS, ..stream(direction, 1) };
            let terms = Terms { t0: 2_000 * MS, root_hz: 1_000_000_000, timed_lead: 2 * MS, release_window: 3 * MS, stream };
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
                if item.seq % 5 == 4 && matches!(item.kind, Kind::Cold(_) | Kind::Start) {
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

/// #63: pruning folds items in the order of their effective instants: a `cold` change received
/// after another but requested before it takes effect at the earlier one's instant (RM-25).
#[test]
fn rm_26_pruning_folds_a_cold_change_floored_at_an_earlier_one() {
    for direction in [Direction::Rx, Direction::Tx] {
        let (mut full, mut pruned) = (Line::new(stream(direction, 1)), Line::new(stream(direction, 1)));
        for booked in [item(500 * MS, 1, 0, Kind::Cold(config(1, 2_000_000))), item(300 * MS, 2, 100 * MS, Kind::Cold(config(1, 500_000)))] {
            full.book(booked).unwrap();
            pruned.book(booked).unwrap();
        }
        pruned.prune(900 * MS);
        let last = item(1_000 * MS, 3, 900 * MS, Kind::Cold(config(1, 1_000_000)));
        full.book(last).unwrap();
        pruned.book(last).unwrap();
        assert_eq!(pruned.plan, full.plan, "{direction:?}");
    }
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
    for direction in [Direction::Rx, Direction::Tx] {
        let mut line = Line::new(stream(direction, 1));
        for n in 0..10_000_i64 {
            let now = n * 300 * MS;
            line.prune(now);
            let rate = if n % 2 == 0 { 2_000_000 } else { 1_000_000 };
            line.book(item(now + 100 * MS, n as u64, now, Kind::Cold(config(1, rate)))).unwrap();
            assert!(line.items.len() <= 2, "{direction:?}: {} items after {n} bookings", line.items.len());
        }
        assert_eq!(line.plan.len(), 10_001, "{direction:?}: every segment stays in the plan");
    }
}

/// RM-21, RM-34: a stream's configuration is its set of channels, so a `cold` change to
/// another set of the same size is another configuration: it starts a new segment, and a
/// plan's segment is found by its own set, not by its channel count.
#[test]
fn rm_21_a_cold_change_to_another_set_of_the_same_size_starts_a_segment() {
    let stream = rx_stream(&[0, 1], 1_000_000);
    let mut line = Line::new(stream);
    line.book(item(1_000 * MS, 0, 1_000 * MS, Kind::Cold(set_config(&[0, 2], 1_000_000)))).unwrap();
    assert_eq!(line.plan.len(), 2, "a new segment: {:?}", line.plan);
    assert_ne!(line.plan[0].config, line.plan[1].config, "the two sets differ");
    assert_eq!(line.plan[1].config.channels.iter().collect::<Vec<_>>(), [0, 2]);
    assert_eq!(line.plan[1].config.channels.count(), 2);
    assert!(line.plan[1].config.channels.contains(2) && !line.plan[1].config.channels.contains(1));
    // Each segment is found by its own configuration: the first set is not the second
    // segment's, although both name two channels.
    assert!(line.segment(line.plan[1].origin, line.plan[0].config).is_none(), "{:?}", line.plan);
    assert_eq!(line.segment(line.plan[1].origin, line.plan[1].config).map(|segment| segment.config), Some(line.plan[1].config));
}

/// RM-21: the set, not the count, says whether a stream runs. Losing the last channel ends
/// the segment and the first channel entering starts the next one; on transmit the clock
/// that holds no channel is kept, ended at its origin (RM-25).
#[test]
fn rm_21_a_stream_that_loses_its_last_channel_starts_again_when_one_enters() {
    let mut line = Line::new(rx_stream(&[0], 1_000_000));
    line.book(item(1_000 * MS, 0, 1_000 * MS, Kind::Cold(set_config(&[], 1_000_000)))).unwrap();
    assert_eq!(planned(&line), [(0, Some(1_000 * MS))], "no running segment with no channel");
    line.book(item(2_000 * MS, 1, 2_000 * MS, Kind::Cold(set_config(&[3], 1_000_000)))).unwrap();
    assert_eq!(planned(&line), [(0, Some(1_000 * MS)), (2_050 * MS, None)]);
    assert_eq!(line.plan[1].config.channels.iter().collect::<Vec<_>>(), [3]);

    let mut tx = Line::new(Stream { direction: Direction::Tx, ..rx_stream(&[0], 1_000_000) });
    tx.book(item(1_000 * MS, 0, 100 * MS, Kind::Cold(set_config(&[], 1_000_000)))).unwrap();
    assert_eq!(planned(&tx), [(0, Some(1_000 * MS))]);
    tx.book(item(1_000 * MS, 1, 100 * MS, Kind::Cold(set_config(&[1], 1_000_000)))).unwrap();
    assert_eq!(planned(&tx), [(0, Some(1_000 * MS)), (1_050 * MS, None)]);
}

/// RM-34: one channel moves from one stream to another, both lines booking a `cold` change
/// at one instant: the source's set without the channel, the destination's with it, each
/// keeping its own rate, and both starting a new segment.
#[test]
fn rm_34_a_move_books_both_lines_at_one_instant() {
    let from = rx_stream(&[0, 1], 1_000_000);
    let to = rx_stream(&[2], 2_000_000);
    let mut lines = lines_of(from, to);
    let (e, late) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 500 * MS, 0, 500 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    assert!(!late, "asked for an instant that is still ahead");
    assert_eq!(by(&lines, FROM, 0).config.channels.iter().collect::<Vec<_>>(), [0]);
    assert_eq!(by(&lines, FROM, 0).config.ratio, from.config.ratio, "the source keeps its rate");
    assert_eq!(by(&lines, TO, 0).config.channels.iter().collect::<Vec<_>>(), [1, 2]);
    assert_eq!(by(&lines, TO, 0).config.ratio, to.config.ratio, "the destination keeps its rate");
    assert_eq!(by(&lines, FROM, 0).origin, by(&lines, TO, 0).origin, "both segments at one instant");
    assert_eq!(by(&lines, FROM, 0).origin, 1_050 * MS);
    // The move's instant is on both lines as one item with one arrival order.
    assert_eq!(by(&lines, FROM, 0).by, by(&lines, TO, 0).by);
}

/// RM-34: a `cold` change either line received first floors the move's one instant, so the
/// move takes effect on both lines at or after it and is reported late (UC-2).
#[test]
fn rm_34_a_move_is_floored_at_a_cold_change_of_either_line() {
    for earlier_on_to in [false, true] {
        let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
        // The earlier change keeps its own line's set and changes its rate.
        let earlier = |numbers: &[u32]| item(3_000 * MS, 0, 500 * MS, Kind::Cold(set_config(numbers, 2_000_000)));
        if earlier_on_to {
            lines.book(TO, earlier(&[2])).unwrap();
        } else {
            lines.book(FROM, earlier(&[0, 1])).unwrap();
        }
        let (e, late) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 500 * MS, 1, 500 * MS).unwrap();
        assert_eq!(e, 3_000 * MS, "{earlier_on_to}");
        assert!(late, "later than asked: {earlier_on_to}");
        assert!(by(&lines, FROM, 1).origin >= 3_000 * MS + 50 * MS, "{:?}", lanes(&lines, FROM));
        assert!(by(&lines, TO, 1).origin >= 3_000 * MS + 50 * MS, "{:?}", lanes(&lines, TO));
        assert_eq!(by(&lines, FROM, 1).config.channels.iter().collect::<Vec<_>>(), [0]);
        assert_eq!(by(&lines, TO, 1).config.channels.iter().collect::<Vec<_>>(), [1, 2]);
    }
}

/// RM-34: a `cold` change received after the move takes effect at or after the move's
/// instant, on either line (RM-25's flooring).
#[test]
fn rm_34_a_later_cold_change_is_floored_at_the_move() {
    for later_on_to in [false, true] {
        let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
        let (e, _) = lines.book_move(FROM, TO, 1, Some(3_000 * MS), 500 * MS, 0, 500 * MS).unwrap();
        assert_eq!(e, 3_000 * MS);
        let later = item(1_000 * MS, 1, 500 * MS, Kind::Cold(set_config(&[0, 2], 1_000_000)));
        let stream = if later_on_to { TO } else { FROM };
        lines.book(stream, later).unwrap();
        let line = lines.line(stream).unwrap();
        let last = line.items.last().copied().expect("the change is booked");
        assert_eq!(ezsdr_radio::timeline::effective(&line.items, &last), 3_000 * MS, "{later_on_to}");
        let segment = by(&lines, stream, 1);
        // The change takes effect at the move's instant, before the segment the move began
        // a lead later, so it cuts that segment at its origin and starts a lead after it.
        assert_eq!(planned_at(&lines, stream).last().copied(), Some((3_100 * MS, None)), "{:?}", line.plan);
        assert_eq!(segment.config.channels.iter().collect::<Vec<_>>(), [0, 2]);
    }
}

/// RM-34, RM-16: the destination's new segment waits a lead after the instant of the
/// source's cut, which is the source's old clock's sample, so a source whose samples fall
/// further apart can hold the destination back past its own cut.
#[test]
fn rm_34_the_destination_waits_the_source_s_cut_and_its_own_lead() {
    let from = rx_stream(&[0, 1], 3_000_000);
    let to = rx_stream(&[2], 100_000_000);
    let mut lines = lines_of(from, to);
    // An instant that is not a sample instant on the source's 3 MS/s clock.
    let e = 1_000_000_001;
    let (instant, _) = lines.book_move(FROM, TO, 1, Some(e), 500 * MS, 0, 500 * MS).unwrap();
    assert_eq!(instant, e);
    let source = lines.line(FROM).unwrap().plan[0];
    let source_cut = source.instant(source.sample_at_or_after(e).unwrap()).unwrap();
    assert!(source_cut > e, "the source's cut rounds up: {source_cut}");
    let destination = lines.line(TO).unwrap().plan[0];
    let destination_cut = destination.instant(destination.sample_at_or_after(e).unwrap()).unwrap();
    assert!(source_cut > destination_cut, "the source's release is the later term: {source_cut}");
    assert!(by(&lines, FROM, 0).origin >= source_cut + 50 * MS, "{:?}", lanes(&lines, FROM));
    assert!(by(&lines, TO, 0).origin >= source_cut + 50 * MS, "{:?}", lanes(&lines, TO));
    assert!(by(&lines, TO, 0).origin > destination_cut + 50 * MS, "the destination's own cut is not the binding term");
}

/// RM-34, RM-25: a refused move halts both lines and leaves the moved channel's set as each
/// line's configuration of record; the next `cold` change or `start_rx` starts each again.
/// A `seq` one line only has is no move.
#[test]
fn rm_34_a_refused_move_halts_both_lines_until_a_change_or_a_start() {
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
    let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 500 * MS, 0, 500 * MS).unwrap();
    assert!(!lines.refuse_move(9).unwrap(), "no such move");
    // An item on one line only, with this `seq`: no move, so no refusal (it takes effect on
    // a stream that is already on, before the move's instant).
    lines.book(FROM, item(500 * MS, 1, 500 * MS, Kind::Start)).unwrap();
    let before = lines.clone();
    assert!(!lines.refuse_move(1).unwrap(), "one line only");
    assert_eq!(lines, before, "a partial refusal changes nothing");

    assert!(lines.refuse_move(0).unwrap());
    assert!(line_halted(&lines, FROM, e) && line_halted(&lines, TO, e), "both halt");
    assert!(!lines.line(FROM).unwrap().plan.iter().any(|segment| segment.cut.is_none()));
    assert!(!lines.line(TO).unwrap().plan.iter().any(|segment| segment.cut.is_none()));
    let moved = |stream: StreamKey| *lines.line(stream).unwrap().items.iter().find(|item| item.seq == 0).expect("the move is booked");
    assert_eq!(moved(FROM).kind, Kind::Cold(set_config(&[0], 1_000_000)));
    assert_eq!(moved(TO).kind, Kind::Cold(set_config(&[1, 2], 1_000_000)));
    assert_eq!((moved(FROM).refused, moved(TO).refused), (true, true));

    lines.book(FROM, item(3_000 * MS, 2, 3_000 * MS, Kind::Cold(set_config(&[0, 1], 1_000_000)))).unwrap();
    assert!(!line_halted(&lines, FROM, 3_000 * MS), "a successful change resumes the source");
    assert_eq!(by(&lines, FROM, 2).config.channels.iter().collect::<Vec<_>>(), [0, 1]);
    lines.book(TO, item(3_000 * MS, 1, 3_000 * MS, Kind::Start)).unwrap();
    assert!(!line_halted(&lines, TO, 3_000 * MS), "start_rx resumes the destination");
    assert_eq!(by(&lines, TO, 1).config.channels.iter().collect::<Vec<_>>(), [1, 2]);
}

/// RM-34, RM-21: a `Stop`ped receive stream does not resume because a channel moved into it;
/// `start_rx` starts it, with the channel it holds.
#[test]
fn rm_34_a_move_into_a_stopped_stream_does_not_start_it() {
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
    lines.book(TO, item(500 * MS, 0, 500 * MS, Kind::Stop)).unwrap();
    let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 1_000 * MS, 1, 1_000 * MS).unwrap();
    assert_eq!(planned_at(&lines, TO), [(0, Some(500 * MS))], "off: no segment for the moved channel: {e}");
    lines.book(TO, item(2_000 * MS, 2, 2_000 * MS, Kind::Start)).unwrap();
    assert_eq!(by(&lines, TO, 2).config.channels.iter().collect::<Vec<_>>(), [1, 2]);
    assert!(by(&lines, TO, 2).origin >= 2_000 * MS, "{:?}", lanes(&lines, TO));
}

/// RM-34, RM-25: a move asked for before a receive line's start takes effect at that start,
/// the later of the two, as a `cold` change of one line's own does.
#[test]
fn rm_34_a_move_is_floored_at_a_receive_line_s_start() {
    let from = Stream { origin: 2_000 * MS, ..rx_stream(&[0, 1], 1_000_000) };
    let to = Stream { origin: 3_000 * MS, ..rx_stream(&[2], 1_000_000) };
    let mut lines = lines_of(from, to);
    let asked = 500 * MS;
    let (e, late) = lines.book_move(FROM, TO, 1, Some(asked), asked, 0, asked).unwrap();
    assert_eq!(e, to.origin, "the later receive line's start floors the one instant");
    assert!(!late, "a line's start is not lateness, as it is not for a `cold` change");
    assert!(by(&lines, FROM, 0).origin >= from.origin + 50 * MS, "{:?}", lanes(&lines, FROM));
    assert_eq!(by(&lines, FROM, 0).config.channels.iter().collect::<Vec<_>>(), [0]);
    assert_eq!(by(&lines, TO, 0).config.channels.iter().collect::<Vec<_>>(), [1, 2]);
}

/// RM-34, RM-25, VH-4: a stream with no channel has no segment, and the first channel
/// moving into it starts the next one with only the start lead after the move's instant.
#[test]
fn rm_34_a_move_into_a_stream_with_no_channel_starts_it() {
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[], 1_000_000));
    assert!(lines.line(TO).unwrap().plan.is_empty(), "no channel, no segment");
    let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 1_000 * MS, 0, 1_000 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    assert_eq!(by(&lines, TO, 0).origin, 1_050 * MS, "{:?}", lanes(&lines, TO));
    assert_eq!(by(&lines, TO, 0).config.channels.iter().collect::<Vec<_>>(), [1]);
}

/// RM-34: when either line's booking fails neither line is changed, including the case
/// where the source books and the destination cannot be planned.
#[test]
fn rm_34_a_move_whose_booking_fails_leaves_both_lines() {
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
    lines.book(FROM, item(1_000 * MS, 0, 1_000 * MS, Kind::Stop)).unwrap();
    let before = lines.clone();
    // The destination's new segment's instant cannot be represented: the source books and
    // is rolled back.
    assert_eq!(lines.book_move(FROM, TO, 1, Some(i64::MAX), i64::MAX, 1, i64::MAX), Err(MoveError::Time(ezsdr_kernel::time::TimeError::Overflow)));
    assert_eq!(lines, before, "neither line is changed");
}

/// RM-34, #63: a line that pruned its items keeps the configuration they left, and a move
/// booked on it starts from that one, not from the Run's.
#[test]
fn rm_34_a_move_reads_the_configuration_a_pruned_line_keeps() {
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
    lines.book(FROM, item(500 * MS, 0, 500 * MS, Kind::Cold(set_config(&[5], 1_000_000)))).unwrap();
    lines.book(FROM, item(600 * MS, 1, 600 * MS, Kind::Cold(set_config(&[5, 6], 1_000_000)))).unwrap();
    lines.prune(1_000 * MS);
    assert!(lines.line(FROM).unwrap().items.is_empty(), "the items are folded: {:?}", lines.line(FROM).unwrap().items);
    // A `seq` no pruned item used, so the plan's segment for it is the move's.
    let (e, _) = lines.book_move(FROM, TO, 6, Some(2_000 * MS), 2_000 * MS, 5, 2_000 * MS).unwrap();
    assert_eq!(e, 2_000 * MS);
    assert_eq!(by(&lines, FROM, 5).config.channels.iter().collect::<Vec<_>>(), [5], "the pruned set without the moved channel");
    assert_eq!(by(&lines, TO, 5).config.channels.iter().collect::<Vec<_>>(), [2, 6]);
}

/// RM-34, RM-16: the source's release is the cut the move makes on the plan as it stands, and
/// a segment the move cuts before its own origin is cut at that origin, a lead after the
/// move's instant, so the destination waits it rather than its own instant.
#[test]
fn rm_34_a_move_waits_a_source_segment_that_starts_after_its_instant() {
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[], 1_000_000));
    // A `cold` change received first whose new segment starts a lead after the move's own
    // instant, so the move cuts it at its origin.
    lines.book(FROM, item(1_000 * MS, 0, 1_000 * MS, Kind::Cold(set_config(&[0, 1], 2_000_000)))).unwrap();
    let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 500 * MS, 1, 500 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    let cut = released(&lines, FROM, 1, 1).expect("the move cuts the source's segment");
    assert_eq!(cut, 1_050 * MS, "the segment's origin is the cut");
    assert_eq!(by(&lines, FROM, 1).origin, cut + 50 * MS, "{:?}", lanes(&lines, FROM));
    assert_eq!(by(&lines, TO, 1).origin, cut + 50 * MS, "the destination waits the source's release: {:?}", lanes(&lines, TO));
}

/// RM-34: a move's record is not a snapshot of its booking: a later booking that changes the
/// source's plan moves the destination's segment with it.
#[test]
fn rm_34_a_later_booking_moves_the_destination_with_the_source() {
    let mut lines = lines_of(rx_stream(&[0, 1], 3_000_000), rx_stream(&[], 100_000_000));
    // The source is stopped, so nothing runs when the move is booked: the move begins no
    // segment there, and the channel's release is the `Stop`'s cut, before the destination's
    // own instant.
    lines.book(FROM, item(500 * MS, 0, 500 * MS, Kind::Stop)).unwrap();
    let e = 1_000_000_001;
    let (instant, _) = lines.book_move(FROM, TO, 1, Some(e), e, 1, 500 * MS).unwrap();
    assert_eq!(instant, e);
    assert_eq!(released(&lines, FROM, 1, 1), Some(500 * MS), "off: the Stop already released the channel");
    assert!(lines.line(FROM).unwrap().plan.iter().all(|segment| segment.by != Some(1)), "off: the move begins no segment");
    assert_eq!(by(&lines, TO, 1).origin, 1_000_000_010, "the destination starts at its own instant: {:?}", lanes(&lines, TO));
    // A `start_rx` booked after the move runs the source up to the move's instant, whose cut
    // is the next source sample; the destination waits that instead.
    lines.book(FROM, item(900 * MS, 2, 900 * MS, Kind::Start)).unwrap();
    let cut = released(&lines, FROM, 1, 1).expect("the source now runs to the move");
    assert_eq!(cut, 1_000_000_334, "the source's sample at or after the move's instant");
    assert!(by(&lines, TO, 1).origin >= cut + 50 * MS, "{:?}", lanes(&lines, TO));
}

/// RM-34, VH-4: the release floors every item of the destination at or after the move, not the
/// move's item alone: a `start_rx` booked into the destination afterwards waits the source's
/// cut too, and follows it when a later booking of the source's stream moves that cut.
#[test]
fn rm_34_a_move_floors_a_later_start_of_the_destination() {
    let from = rx_stream(&[0, 1], 3_000_000);
    let mut lines = lines_of(from, rx_stream(&[], 100_000_000));
    // Both receives are off, so neither runs when the move is booked.
    lines.book(FROM, item(500 * MS, 0, 500 * MS, Kind::Stop)).unwrap();
    lines.book(TO, item(500 * MS, 1, 500 * MS, Kind::Stop)).unwrap();
    let e = 1_000_000_001;
    let (instant, _) = lines.book_move(FROM, TO, 1, Some(e), e, 2, e).unwrap();
    assert_eq!(instant, e);
    assert!(lines.line(FROM).unwrap().plan.iter().all(|segment| segment.by != Some(2)), "off: the move begins no segment");
    assert_eq!(released(&lines, FROM, 1, 2), Some(500 * MS), "off: the Stop already released the channel");
    // The destination is started after the move, while the source is still off: the source has
    // released nothing since the Stop, so it starts at its own instant and lead.
    lines.book(TO, item(e, 3, e, Kind::Start)).unwrap();
    assert_eq!(by(&lines, TO, 3).origin, 1_050_000_010, "the destination's own instant: {:?}", lanes(&lines, TO));
    // A `start_rx` booked into the source afterwards runs it up to the move's instant, whose
    // cut is the next source sample; the destination's start waits that instead.
    lines.book(FROM, item(900 * MS, 4, 900 * MS, Kind::Start)).unwrap();
    let cut = released(&lines, FROM, 1, 2).expect("the source now runs to the move");
    assert_eq!(cut, 1_000_000_334, "the source's sample at or after the move's instant");
    assert!(
        by(&lines, TO, 3).origin >= cut + 50 * MS,
        "the destination's start waits the source's release: {:?}",
        lanes(&lines, TO),
    );
}

/// RM-34, VH-2: the release is held apart from the ready, so a transmit change booked before
/// its stream's end keeps its clock even though the move made the destination wait a source cut
/// after that end.
#[test]
fn rm_34_a_move_keeps_a_transmit_clock_booked_before_an_end() {
    let mut lines = lines_of(
        Stream { direction: Direction::Tx, ..rx_stream(&[0, 1], 1_000_000) },
        Stream { direction: Direction::Tx, ..rx_stream(&[], 1_000_000) },
    );
    // A loss at 800 ms, booked before the move: a change booked before it keeps its clock.
    for stream in [FROM, TO] {
        lines.book(stream, item(800 * MS, 0, 0, Kind::End { abort: false })).unwrap();
    }
    let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 500 * MS, 1, 500 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    let cut = released(&lines, FROM, 1, 1).expect("the source runs to the move");
    assert_eq!(cut, 1_000 * MS);
    // VH-2: the destination's clock was booked at 500 ms, before the loss, so it stands, and
    // it waits the source's release.
    let destination = by(&lines, TO, 1);
    assert_eq!(destination.config.channels.iter().collect::<Vec<_>>(), [1]);
    assert_eq!(destination.origin, cut + 50 * MS, "the destination waits the source's release: {:?}", lanes(&lines, TO));
    // The release is not the ready: the item's ready is still its booking, which is what told
    // the replay it was booked before the loss.
    let moved = |stream: StreamKey| {
        *lines.line(stream).unwrap().items.iter().find(|item| item.seq == 1).expect("the move is booked on both lines")
    };
    assert_eq!(moved(TO).ready, 500 * MS, "the release is held apart from the ready");
    assert_eq!(moved(TO).release, cut, "the lines hold the release");
    assert_eq!(moved(FROM).release, NO_RELEASE, "no move makes the source release anything");
}

/// RM-34, #63: a pruning that folds a move's two items away keeps the source's cut as the
/// destination's release floor, which every item booked into it afterwards waits.
#[test]
fn rm_34_a_pruned_move_keeps_its_release_floor() {
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[], 1_000_000));
    // A `cold` change received first whose new segment starts a lead after the move's own
    // instant, so the source's cut is later than the pruning instant.
    lines.book(FROM, item(1_000 * MS, 0, 1_000 * MS, Kind::Cold(set_config(&[0, 1], 2_000_000)))).unwrap();
    lines.book(TO, item(500 * MS, 1, 500 * MS, Kind::Stop)).unwrap();
    let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 500 * MS, 2, 500 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    let cut = released(&lines, FROM, 1, 2).expect("the move cuts the source's segment");
    assert_eq!(cut, 1_050 * MS, "the source's segment starts a lead after the move's instant");
    assert!(lines.line(TO).unwrap().plan.is_empty(), "off: the move starts nothing");
    // The pruning at 1 020 ms, between the move's instant and that cut, folds both items away
    // and drops the move's record with them.
    lines.prune(1_020 * MS);
    assert!(lines.line(FROM).unwrap().items.is_empty() && lines.line(TO).unwrap().items.is_empty(), "the items are folded");
    // A `start_rx` booked into the destination then waits the release the pruning kept.
    lines.book(TO, item(1_030 * MS, 3, 1_030 * MS, Kind::Start)).unwrap();
    let started = by(&lines, TO, 3);
    assert!(started.origin >= cut + 50 * MS, "the destination waits the pruned move's release: {:?}", lanes(&lines, TO));
    assert_eq!(started.config.channels.iter().collect::<Vec<_>>(), [1], "the folded configuration, with the moved channel");
}

fn pruned_move_order(source_first: bool) {
    let mut unpruned = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[], 1_000_000));
    unpruned.book(FROM, item(1_000 * MS, 0, 1_000 * MS, Kind::Stop)).unwrap();
    unpruned.book(FROM, item(1_000 * MS, 1, 1_000 * MS, Kind::Start)).unwrap();
    unpruned.book_move(FROM, TO, 1, Some(1_000 * MS), 1_000 * MS, 2, 1_000 * MS).unwrap();
    let release_before = released(&unpruned, FROM, 1, 2).expect("the start's segment releases the channel");
    assert!(release_before > 1_000 * MS, "the source Start segment is cut at its origin");
    let destination_before = by(&unpruned, TO, 2).origin;
    assert_eq!(destination_before, release_before + 50 * MS, "the destination waits the source release");
    let mut pruned = unpruned.clone();
    let first = if source_first { FROM } else { TO };
    let second = if source_first { TO } else { FROM };
    pruned.prune_stream(first, 1_001 * MS);
    assert!(pruned.line(FROM).unwrap().items.iter().any(|item| item.seq == 2), "the first line keeps its move item");
    assert!(pruned.line(TO).unwrap().items.iter().any(|item| item.seq == 2), "the peer keeps its move item");
    assert_eq!(planned_at(&pruned, FROM), planned_at(&unpruned, FROM));
    assert_eq!(planned_at(&pruned, TO), planned_at(&unpruned, TO));

    // The Start at the move's instant puts the source's segment after the move; its refusal
    // moves the release back to the Stop's cut, even after the source requested pruning.
    assert!(pruned.refuse(FROM, 1).unwrap());
    assert!(unpruned.refuse(FROM, 1).unwrap());
    let release_after = released(&pruned, FROM, 1, 2).expect("the refused move still has a source release");
    assert!(release_after < release_before, "refusing the Start changes the release");
    assert_eq!(release_after, released(&unpruned, FROM, 1, 2).unwrap());
    let destination_after = by(&pruned, TO, 2).origin;
    assert_eq!(destination_after, by(&unpruned, TO, 2).origin, "the refused Start updates the destination release");
    assert!(destination_after < destination_before, "the destination's planned move segment follows the changed release");
    assert_eq!(planned_at(&pruned, FROM), planned_at(&unpruned, FROM));
    assert_eq!(planned_at(&pruned, TO), planned_at(&unpruned, TO));

    // A move refusal still reaches both item records after a one-sided prune.
    assert!(pruned.refuse(FROM, 2).unwrap());
    assert!(unpruned.refuse(FROM, 2).unwrap());
    assert!(line_halted(&pruned, FROM, 1_000 * MS) && line_halted(&pruned, TO, 1_000 * MS), "both lines halt");
    assert_eq!(planned_at(&pruned, FROM), planned_at(&unpruned, FROM));
    assert_eq!(planned_at(&pruned, TO), planned_at(&unpruned, TO));

    // The second stream's request completes the pair. Its source release remains the
    // destination floor after both move items and their record have been folded away.
    pruned.prune_stream(second, 1_001 * MS);
    assert!(pruned.line(FROM).unwrap().items.is_empty() && pruned.line(TO).unwrap().items.is_empty(), "both requests fold the move");
    assert_eq!(planned_at(&pruned, FROM), planned_at(&unpruned, FROM));
    assert_eq!(planned_at(&pruned, TO), planned_at(&unpruned, TO));
    let next = item(900 * MS, 3, 900 * MS, Kind::Cold(set_config(&[1], 1_000_000)));
    pruned.book(TO, next).unwrap();
    unpruned.book(TO, next).unwrap();
    assert_eq!(by(&pruned, TO, 3).origin, by(&unpruned, TO, 3).origin, "the release floor matches the unpruned dependency");
    assert!(by(&pruned, TO, 3).origin >= release_after + 50 * MS, "the move's release survives the completed prune");
}

#[test]
fn rm_34_a_source_first_prune_keeps_move_dependencies_until_both_lines_pass() {
    // RM-34: a source-only prune retains both move items, later release changes still reach
    // the destination, refusal halts both lines, and the completed prune preserves the floor.
    pruned_move_order(true);
}

#[test]
fn rm_34_a_destination_first_prune_keeps_move_dependencies_until_both_lines_pass() {
    // The destination-first order has the same move dependency and floor guarantees.
    pruned_move_order(false);
}

fn chained_move_prune_order(prune_b_first: bool) {
    let (a, b, c) = ((Direction::Rx, 0), (Direction::Rx, 1), (Direction::Rx, 2));
    let mut unpruned = Lines::new();
    unpruned.insert(a, Line::new(rx_stream(&[0], 1_000_000)));
    unpruned.insert(b, Line::new(rx_stream(&[], 1_000_000)));
    unpruned.insert(c, Line::new(rx_stream(&[], 1_000_000)));
    unpruned.book_move(a, b, 0, Some(10 * MS), 10 * MS, 1, 10 * MS).unwrap();
    unpruned.book_move(b, c, 0, Some(20 * MS), 20 * MS, 2, 20 * MS).unwrap();

    let mut pruned = unpruned.clone();
    let (first, second) = if prune_b_first { (b, c) } else { (c, b) };
    pruned.prune_stream(first, 30 * MS);
    for stream in [a, b, c] {
        assert_eq!(planned_at(&pruned, stream), planned_at(&unpruned, stream), "after pruning {first:?}");
    }
    pruned.prune_stream(second, 30 * MS);
    for stream in [a, b, c] {
        assert_eq!(planned_at(&pruned, stream), planned_at(&unpruned, stream), "after pruning {second:?}");
    }
    assert_eq!(pruned.move_lines(2), Some((b, c)), "the second move remains recorded");
    for stream in [b, c] {
        assert!(pruned.line(stream).unwrap().items.iter().any(|item| item.seq == 2), "{stream:?} keeps the second move item");
    }
    assert_eq!(released(&pruned, b, 0, 2), released(&unpruned, b, 0, 2), "release reads the retained source item");

    assert!(pruned.refuse_move(2).unwrap());
    assert!(unpruned.refuse_move(2).unwrap());
    assert!(line_halted(&pruned, c, 20 * MS), "refusing B→C halts C");
    for stream in [a, b, c] {
        assert_eq!(planned_at(&pruned, stream), planned_at(&unpruned, stream), "after refusing B→C on {stream:?}");
    }
}

#[test]
fn rm_34_a_chain_prune_reaches_a_fixed_point_from_b_then_c() {
    // A→B at 10 and B→C at 20: only B and C request a prune at 30. A's missing
    // boundary clamps B to 10, which in turn clamps C to 20.
    chained_move_prune_order(true);
}

#[test]
fn rm_34_a_chain_prune_reaches_a_fixed_point_from_c_then_b() {
    chained_move_prune_order(false);
}

/// RM-34: the destination waits the greatest cut of the moves into it, not the last booked: a
/// later move whose source releases earlier does not shorten what an earlier one released.
#[test]
fn rm_34_a_move_takes_the_greatest_cut_of_the_moves_into_a_line() {
    let (a, b, c) = ((Direction::Rx, 0), (Direction::Rx, 1), (Direction::Rx, 2));
    let mut lines = Lines::new();
    lines.insert(a, Line::new(rx_stream(&[0, 1], 1_000_000)));
    lines.insert(b, Line::new(rx_stream(&[2], 1_000_000)));
    lines.insert(c, Line::new(rx_stream(&[], 1_000_000)));
    // C is stopped, so neither move starts it.
    lines.book(c, item(500 * MS, 0, 500 * MS, Kind::Stop)).unwrap();
    // A change received first whose new segment starts a lead after the move's instant, so A
    // releases the channel at 1 050 ms.
    lines.book(a, item(1_000 * MS, 1, 1_000 * MS, Kind::Cold(set_config(&[0, 1], 1_000_000)))).unwrap();
    let (e, _) = lines.book_move(a, c, 0, Some(1_000 * MS), 1_000 * MS, 2, 1_000 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    assert_eq!(released(&lines, a, 0, 2), Some(1_050 * MS));
    // A second move into C, received later, whose source is running and releases at its own
    // instant: its cut is the smaller of the two.
    let (e, _) = lines.book_move(b, c, 2, Some(1_010 * MS), 1_010 * MS, 3, 1_010 * MS).unwrap();
    assert_eq!(e, 1_010 * MS);
    assert_eq!(released(&lines, b, 2, 3), Some(1_010 * MS));
    // C's start waits the greatest of the two releases, so it begins a lead after 1 050 ms.
    lines.book(c, item(1_030 * MS, 4, 1_030 * MS, Kind::Start)).unwrap();
    assert_eq!(by(&lines, c, 4).origin, 1_100 * MS, "{:?}", lanes(&lines, c));
}

/// RM-34, RM-25: at one instant the order is by arrival, so an item of the destination at the
/// move's instant that arrived before it is not floored: the move cuts its segment and waits
/// that cut, not the source's release.
#[test]
fn rm_34_a_move_does_not_floor_an_item_received_before_it_at_its_instant() {
    let mut lines = lines_of(rx_stream(&[0], 1_000_000), rx_stream(&[1], 1_000_000));
    // The destination's own change at the move's instant, received first.
    lines.book(TO, item(1_000 * MS, 0, 1_000 * MS, Kind::Cold(set_config(&[1, 2], 1_000_000)))).unwrap();
    // The source's change starts its segment a lead after the same instant, so the move cuts
    // it at its origin: the release is 1 050 ms.
    lines.book(FROM, item(1_000 * MS, 1, 1_000 * MS, Kind::Cold(set_config(&[0], 1_000_000)))).unwrap();
    let (e, _) = lines.book_move(FROM, TO, 0, Some(1_000 * MS), 1_000 * MS, 2, 1_000 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    assert_eq!(released(&lines, FROM, 0, 2), Some(1_050 * MS), "the source's segment starts a lead after the move's instant");
    // The destination's change is cut at its own origin, 1 050 ms, and the move's segment
    // begins a lead after that, not a lead after the release of a floored change.
    assert_eq!(by(&lines, TO, 2).origin, 1_100 * MS, "{:?}", lanes(&lines, TO));
    assert!(planned_at(&lines, TO).iter().all(|(_, cut)| *cut != Some(1_050 * MS)), "the change's empty segment is not planned: {:?}", lanes(&lines, TO));
}

/// RM-34, D3's4: refusing a move's `seq` on one of its lines refuses it on both — the call goes
/// to `refuse_move` — so the two halt together whichever of them names it.
#[test]
fn rm_34_a_move_is_refused_on_both_lines_whichever_line_names_it() {
    for stream in [FROM, TO] {
        let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
        // A third stream of the instance, which the move does not touch.
        lines.insert((Direction::Rx, 2), Line::new(rx_stream(&[3], 1_000_000)));
        let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 500 * MS, 0, 500 * MS).unwrap();
        // A stream the move does not touch holds no such command, so nothing is refused.
        let before = lines.clone();
        assert!(!lines.refuse((Direction::Rx, 2), 0).unwrap(), "{stream:?}: no such command");
        assert_eq!(lines, before, "{stream:?}");
        assert!(lines.refuse(stream, 0).unwrap(), "{stream:?} names the move");
        assert!(line_halted(&lines, FROM, e) && line_halted(&lines, TO, e), "{stream:?}: both halt");
        for line in [FROM, TO] {
            let item = *lines.line(line).unwrap().items.iter().find(|item| item.seq == 0).expect("the move is booked on both");
            assert!(item.refused, "{stream:?}, {line:?}");
        }
    }
}

/// RM-34, RM-25, UC-2: `book_move` takes effect at `at`, or at its receipt when `at` is absent
/// or past, and reports the lateness of that choice.
#[test]
fn rm_34_a_move_without_a_timed_instant_takes_effect_at_its_receipt() {
    // No `at`: the move takes effect at its receipt and is not late.
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
    let (e, late) = lines.book_move(FROM, TO, 1, None, 1_000 * MS, 0, 1_000 * MS).unwrap();
    assert_eq!((e, late), (1_000 * MS, false));
    // An `at` before the receipt: the receipt is the one that counts, and the move is late.
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
    let (e, late) = lines.book_move(FROM, TO, 1, Some(500 * MS), 1_000 * MS, 0, 1_000 * MS).unwrap();
    assert_eq!((e, late), (1_000 * MS, true));
    assert_eq!(by(&lines, FROM, 0).origin, 1_050 * MS, "{:?}", lanes(&lines, FROM));
}

/// RM-34, #63: a pruning that folds several moves into one line keeps the greatest of their
/// releases as that line's floor.
#[test]
fn rm_34_a_pruned_move_takes_the_greatest_floor_of_its_line() {
    let (a, b, c) = ((Direction::Rx, 0), (Direction::Rx, 1), (Direction::Rx, 2));
    let mut lines = Lines::new();
    lines.insert(a, Line::new(rx_stream(&[0, 1], 1_000_000)));
    lines.insert(b, Line::new(rx_stream(&[2], 1_000_000)));
    lines.insert(c, Line::new(rx_stream(&[], 1_000_000)));
    lines.book(c, item(500 * MS, 0, 500 * MS, Kind::Stop)).unwrap();
    // The first move's source releases at 1 050 ms: a change received first starts its segment
    // a lead after the move's instant.
    lines.book(a, item(1_000 * MS, 1, 1_000 * MS, Kind::Cold(set_config(&[0, 1], 1_000_000)))).unwrap();
    let (e, _) = lines.book_move(a, c, 0, Some(1_000 * MS), 1_000 * MS, 2, 1_000 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    // The second, received later, releases at its own instant, 1 010 ms.
    let (e, _) = lines.book_move(b, c, 2, Some(1_010 * MS), 1_010 * MS, 3, 1_010 * MS).unwrap();
    assert_eq!(e, 1_010 * MS);
    // The pruning at 1 020 ms folds both moves into the destination's state.
    lines.prune(1_020 * MS);
    assert!(lines.line(c).unwrap().items.is_empty(), "the items are folded");
    // Its start waits the greatest of the two releases, so it begins at 1 100 ms.
    lines.book(c, item(1_030 * MS, 4, 1_030 * MS, Kind::Start)).unwrap();
    assert_eq!(by(&lines, c, 4).origin, 1_100 * MS, "{:?}", lanes(&lines, c));
}

/// RM-34: the moves' dependence is held over the lines, not once at each booking: a booking
/// that changes A's plan moves B's item and, through B's cut, C's.
#[test]
fn rm_34_a_moves_dependence_propagates_along_a_chain() {
    let (a, b, c) = ((Direction::Rx, 0), (Direction::Rx, 1), (Direction::Rx, 2));
    let mut lines = Lines::new();
    lines.insert(a, Line::new(rx_stream(&[0, 1], 1_000_000)));
    lines.insert(b, Line::new(rx_stream(&[1], 100_000_000)));
    lines.insert(c, Line::new(rx_stream(&[3], 1_000_000)));
    // A is stopped, so its move cuts nothing when it is booked.
    lines.book(a, item(500 * MS, 0, 500 * MS, Kind::Stop)).unwrap();
    let (e, _) = lines.book_move(a, b, 0, Some(4_000 * MS), 4_000 * MS, 1, 500 * MS).unwrap();
    assert_eq!(e, 4_000 * MS);
    // B's channel then moves to C, after the instant B's own segment for the first move
    // starts, so the second move cuts that segment at its origin.
    let (e, _) = lines.book_move(b, c, 1, Some(4_080 * MS), 4_080 * MS, 2, 500 * MS).unwrap();
    assert_eq!(e, 4_080 * MS);
    assert_eq!(released(&lines, b, 1, 2), Some(4_080 * MS), "B's first segment runs then");
    let before = by(&lines, c, 2).origin;
    assert_eq!(before, 4_130 * MS);
    // A `start_rx` booked after both moves and ready at 4 s: A's segment for the first move
    // starts a lead later, so that cut and B's, and C's segment, move with it.
    lines.book(a, item(3_900 * MS, 3, 4_000 * MS, Kind::Start)).unwrap();
    assert_eq!(released(&lines, a, 0, 1), Some(4_050 * MS), "A's release moved");
    assert_eq!(released(&lines, b, 1, 2), Some(4_100 * MS), "B's segment starts after the second move's instant");
    assert_eq!(by(&lines, c, 2).origin, 4_150 * MS, "C waits B's cut: {:?}", lanes(&lines, c));
    assert!(by(&lines, c, 2).origin > before, "C's segment moved");
}

/// RM-34, RM-21: a halted stream resumes on a successful move, as on any `cold` change, and
/// its set is the refused change's configuration of record with the moved channel.
#[test]
fn rm_34_a_move_resumes_a_halted_stream() {
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
    // A refused change halts the destination, with its refused set as the configuration of
    // record it starts from (RM-25).
    lines.book(TO, refused(item(500 * MS, 0, 500 * MS, Kind::Cold(set_config(&[2, 3], 1_000_000))))).unwrap();
    assert!(line_halted(&lines, TO, 500 * MS), "a refused change halts the stream");
    assert_eq!(planned_at(&lines, TO), [(0, Some(500 * MS))], "nothing runs while it is halted");
    let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 1_000 * MS, 1, 1_000 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    assert!(!line_halted(&lines, TO, e), "a successful move resumes it");
    assert_eq!(by(&lines, TO, 1).config.channels.iter().collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(by(&lines, TO, 1).origin, 1_050 * MS, "{:?}", lanes(&lines, TO));
}

/// RM-34: the dependence is held after a refusal too: refusing the change the source's
/// release came from removes that release, and the destination's segment moves back to what
/// the source's plan still releases then.
#[test]
fn rm_34_a_refused_source_change_moves_the_destination() {
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[], 1_000_000));
    lines.book(FROM, item(1_000 * MS, 0, 1_000 * MS, Kind::Cold(set_config(&[0, 1], 2_000_000)))).unwrap();
    let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 500 * MS, 1, 500 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    assert_eq!(released(&lines, FROM, 1, 1), Some(1_050 * MS));
    assert_eq!(by(&lines, TO, 1).origin, 1_100 * MS, "the destination waits the source's release: {:?}", lanes(&lines, TO));
    assert!(lines.refuse(FROM, 0).unwrap());
    // The refused change begins no segment, so the source's release is the cut the refusal
    // still made of the segment that ran before it: 1 000 ms.
    assert_eq!(released(&lines, FROM, 1, 1), Some(1_000 * MS), "the refused change already cut the source at its instant");
    assert_eq!(by(&lines, TO, 1).origin, 1_050 * MS, "the destination moves back to the release left: {:?}", lanes(&lines, TO));
}

/// RM-34: the source may release the moved channel before the move's item takes effect — a
/// `Stop` cuts its 1 Hz segment at the next sample, 1 s, while the move is booked for 200 ms —
/// and the destination still waits that cut rather than its own instant.
#[test]
fn rm_34_the_destination_waits_a_stopped_source_s_cut() {
    let mut lines = lines_of(rx_bare(&[0], 1), rx_bare(&[], 1_000_000));
    lines.book(FROM, item(100 * MS, 0, 100 * MS, Kind::Stop)).unwrap();
    assert_eq!(planned_at(&lines, FROM), [(0, Some(1_000 * MS))], "the Stop cuts the 1 Hz segment at its next sample");
    let (e, _) = lines.book_move(FROM, TO, 0, Some(200 * MS), 200 * MS, 1, 200 * MS).unwrap();
    assert_eq!(e, 200 * MS);
    assert!(lines.line(FROM).unwrap().plan.iter().all(|segment| segment.by != Some(1)), "off: the move begins no segment");
    assert_eq!(released(&lines, FROM, 0, 1), Some(1_000 * MS), "the source released the channel at the Stop's cut");
    assert_eq!(by(&lines, TO, 1).origin, 1_000 * MS, "the destination waits the source's release: {:?}", lanes(&lines, TO));
}

/// RM-34: the same on the transmit side, with a loss: its clock runs to the move's instant and
/// is cut at its 1 Hz sample, 1 s, which the destination waits too.
#[test]
fn rm_34_the_destination_waits_an_ended_source_s_cut() {
    let mut lines = lines_of(
        Stream { direction: Direction::Tx, ..rx_bare(&[0], 1) },
        Stream { direction: Direction::Tx, ..rx_bare(&[], 1_000_000) },
    );
    // The loss is received at the Run's start and takes effect at 100 ms; the move is booked at
    // 50 ms, before it, and takes effect at 200 ms, so it is what cuts the clock there.
    lines.book(FROM, item(100 * MS, 0, 0, Kind::End { abort: false })).unwrap();
    let (e, _) = lines.book_move(FROM, TO, 0, Some(200 * MS), 50 * MS, 1, 50 * MS).unwrap();
    assert_eq!(e, 200 * MS);
    assert_eq!(released(&lines, FROM, 0, 1), Some(1_000 * MS), "the 1 Hz clock is cut at its next sample");
    assert_eq!(by(&lines, TO, 1).origin, 1_000 * MS, "the destination waits the source's release: {:?}", lanes(&lines, TO));
}

/// RM-34, #63: a pruning between the move and the instant it releases the channel keeps that
/// release for the items the destination's line books afterwards: they still wait it.
#[test]
fn rm_34_a_pruned_stopped_source_keeps_its_release() {
    let mut lines = lines_of(rx_bare(&[0], 1), rx_bare(&[], 1_000_000));
    // The destination is stopped, so the move into it starts nothing.
    lines.book(TO, item(50 * MS, 0, 50 * MS, Kind::Stop)).unwrap();
    lines.book(FROM, item(100 * MS, 1, 100 * MS, Kind::Stop)).unwrap();
    let (e, _) = lines.book_move(FROM, TO, 0, Some(200 * MS), 200 * MS, 2, 200 * MS).unwrap();
    assert_eq!(e, 200 * MS);
    assert!(planned_at(&lines, TO).is_empty(), "off: the move starts nothing");
    // The pruning at 500 ms, between the move and the release at 1 s, folds the Stop and the
    // move's two items away.
    lines.prune(500 * MS);
    assert!(lines.line(FROM).unwrap().items.is_empty() && lines.line(TO).unwrap().items.is_empty(), "the items are folded");
    lines.book(TO, item(600 * MS, 3, 600 * MS, Kind::Start)).unwrap();
    let started = by(&lines, TO, 3);
    assert_eq!(started.origin, 1_000 * MS, "the destination waits the pruned move's release: {:?}", lanes(&lines, TO));
    assert_eq!(started.config.channels.iter().collect::<Vec<_>>(), [0], "the folded configuration, with the moved channel");
}

/// RM-34: only the segments that hold the moved channel say when the source releases it. A
/// source that holds it as the configuration of record of a change it refused never ran it, so
/// a segment that runs another channel alone does not floor the destination.
#[test]
fn rm_34_a_move_does_not_wait_a_segment_of_another_channel() {
    let mut lines = lines_of(rx_stream(&[1], 1_000_000), rx_stream(&[], 1_000_000));
    // The refused change leaves channel 0 as the source's configuration of record, without ever
    // running it; the move of channel 0 is booked from that one.
    lines.book(FROM, refused(item(100 * MS, 0, 100 * MS, Kind::Cold(set_config(&[0, 1], 1_000_000))))).unwrap();
    let (e, _) = lines.book_move(FROM, TO, 0, Some(100 * MS), 50 * MS, 1, 50 * MS).unwrap();
    assert_eq!(e, 100 * MS);
    assert_eq!(released(&lines, FROM, 0, 1), None, "the source never ran channel 0, so nothing released it");
    assert_eq!(by(&lines, TO, 1).origin, 100 * MS, "the segment that runs channel 1 alone does not floor it: {:?}", lanes(&lines, TO));
}

/// RM-34: the release is the source's plan, not the arrival order of its items. A `start_rx`
/// booked after the move still runs the channel before it, and a later `Stop` ends that segment
/// after the move's instant, so the source held the channel until there.
#[test]
fn rm_34_a_move_reads_the_source_s_plan_not_its_arrival_order() {
    // A 100 Hz source: its samples, and so the cut of a `Stop`, are 10 ms apart.
    let mut lines = lines_of(rx_bare(&[0], 100), rx_bare(&[], 1_000_000));
    lines.book(FROM, item(5 * MS, 0, 5 * MS, Kind::Stop)).unwrap();
    assert_eq!(planned_at(&lines, FROM), [(0, Some(10 * MS))], "the Stop cuts the first segment at its next sample");
    let (e, _) = lines.book_move(FROM, TO, 0, Some(26 * MS), 6 * MS, 1, 6 * MS).unwrap();
    assert_eq!(e, 26 * MS);
    assert_eq!(released(&lines, FROM, 0, 1), Some(10 * MS), "the Stop's cut is what the plan says then");
    assert_eq!(by(&lines, TO, 1).origin, 26 * MS, "before the destination's own instant: {:?}", lanes(&lines, TO));
    // The source is off when the move takes effect, so the move cuts nothing: the segments its
    // plan holds alone say when the channel was released.
    lines.book(FROM, item(15 * MS, 2, 15 * MS, Kind::Start)).unwrap();
    lines.book(FROM, item(25 * MS, 3, 25 * MS, Kind::Stop)).unwrap();
    assert!(lines.line(FROM).unwrap().plan.iter().all(|segment| segment.by != Some(1)), "off: the move begins no segment");
    assert_eq!(
        planned_at(&lines, FROM),
        [(0, Some(10 * MS)), (20 * MS, Some(30 * MS))],
        "the segment the later start ran ends at its next 100 Hz sample",
    );
    assert_eq!(released(&lines, FROM, 0, 1), Some(30 * MS), "that segment released the channel, though it began after the move was booked");
    assert_eq!(by(&lines, TO, 1).origin, 30 * MS, "the destination waits it: {:?}", lanes(&lines, TO));
}

/// RM-34, VH-2: a loss leaves a transmit clock running, so a move of its channel booked after
/// the loss cuts nothing; the source gave the channel up when the loss took effect, and the
/// destination starts at its own instant instead of waiting a clock that never ends.
#[test]
fn rm_34_a_move_reads_a_transmit_clock_the_loss_left_running() {
    let mut lines = lines_of(
        Stream { direction: Direction::Tx, ..rx_bare(&[0], 1_000_000) },
        Stream { direction: Direction::Tx, ..rx_bare(&[], 1_000_000) },
    );
    lines.book(FROM, item(100 * MS, 0, 0, Kind::End { abort: false })).unwrap();
    assert_eq!(planned_at(&lines, FROM), [(0, None)], "a loss cuts no transmit clock");
    // The move is booked after the loss, so the source's item takes effect after it and cuts
    // nothing, while the destination's item stands.
    let (e, _) = lines.book_move(FROM, TO, 0, Some(200 * MS), 200 * MS, 1, 200 * MS).unwrap();
    assert_eq!(e, 200 * MS);
    let moved = *lines.line(TO).unwrap().items.iter().find(|item| item.seq == 1).expect("the move is booked on the destination");
    assert_eq!(moved.release, 100 * MS, "the source gave the channel up at the loss");
    assert_eq!(by(&lines, TO, 1).origin, 200 * MS, "the destination starts at the move's own instant: {:?}", lanes(&lines, TO));
}

#[test]
fn rm_34_a_move_waits_a_segment_the_plan_leaves_out() {
    let mut lines = lines_of(rx_bare(&[0], 1), rx_bare(&[], 1_000_000));
    lines.book(FROM, item(100 * MS, 0, 100 * MS, Kind::Stop)).unwrap();
    lines.book(FROM, item(1_500 * MS, 2, 1_500 * MS, Kind::Start)).unwrap();
    lines.book(FROM, item(1_700 * MS, 3, 1_700 * MS, Kind::Stop)).unwrap();
    // The 1 Hz start's segment begins at 2 s and the later Stop cuts that empty segment at its
    // origin, so the plan holds the first segment alone.
    assert_eq!(planned_at(&lines, FROM), [(0, Some(1_000 * MS))], "the segment with no sample is left out");
    let (e, _) = lines.book_move(FROM, TO, 0, Some(1_900 * MS), 1_800 * MS, 1, 1_800 * MS).unwrap();
    assert_eq!(e, 1_900 * MS);
    assert!(lines.line(FROM).unwrap().plan.iter().all(|segment| segment.by != Some(1)), "off: the move begins no segment");
    assert_eq!(released(&lines, FROM, 0, 1), Some(2_000 * MS), "the source's clock for it is cut at 2 s");
    assert_eq!(by(&lines, TO, 1).origin, 2_000 * MS, "the destination waits it: {:?}", lanes(&lines, TO));
}

/// RM-34: a channel the source gains again after the move does not floor the destination: the
/// release is the end of the last segment that held the channel before the move.
#[test]
fn rm_34_a_move_does_not_wait_a_channel_the_source_gains_after_it() {
    let mut lines = lines_of(rx_bare(&[0], 1_000_000), rx_bare(&[], 1_000_000));
    let (e, _) = lines.book_move(FROM, TO, 0, Some(200 * MS), 200 * MS, 0, 200 * MS).unwrap();
    assert_eq!(e, 200 * MS);
    assert_eq!(by(&lines, TO, 0).origin, 200 * MS, "the destination waits the move's own cut: {:?}", lanes(&lines, TO));
    // The source gains channel 0 again at 500 ms and loses it at 700 ms: that segment is after
    // the move, so it says nothing about the release the destination waits.
    lines.book(FROM, item(500 * MS, 1, 500 * MS, Kind::Cold(set_config(&[0], 1_000_000)))).unwrap();
    lines.book(FROM, item(700 * MS, 2, 700 * MS, Kind::Cold(set_config(&[], 1_000_000)))).unwrap();
    assert_eq!(released(&lines, FROM, 0, 0), Some(200 * MS), "the segment gained afterwards is no release of the move");
    assert_eq!(by(&lines, TO, 0).origin, 200 * MS, "the destination is not floored by it: {:?}", lanes(&lines, TO));
}

/// RM-34: the release is the greatest end of the segments the source ran before the move, not
/// the last one read: a segment RM-25 leaves out of the plan ends before a later one, and the
/// destination still waits the greater.
#[test]
fn rm_34_a_move_waits_the_greatest_end_not_the_last_one_read() {
    let mut lines = lines_of(rx_bare(&[0], 1_000_000), rx_stream(&[], 1_000_000));
    // A `Stop` at 300 ms, then a `start_rx` and a `Stop` of the same instant whose segment is
    // cut at its origin (so RM-25 leaves it out of the plan), then a `start_rx` at 400 ms that
    // the move cuts at 500 ms.
    for (e, seq, kind) in [(300, 0, Kind::Stop), (300, 1, Kind::Start), (300, 2, Kind::Stop), (400, 3, Kind::Start), (900, 4, Kind::Stop)] {
        lines.book(FROM, item(e * MS, seq, e * MS, kind)).unwrap();
    }
    let (e, _) = lines.book_move(FROM, TO, 0, Some(500 * MS), 500 * MS, 5, 0).unwrap();
    assert_eq!(e, 500 * MS);
    assert_eq!(
        planned_at(&lines, FROM),
        [(0, Some(300 * MS)), (400 * MS, Some(500 * MS))],
        "the segment with no sample is left out, and the move cuts the one from 400 ms",
    );
    assert_eq!(released(&lines, FROM, 0, 5), Some(500 * MS), "the greater end, not the omitted segment's 300 ms");
    assert_eq!(by(&lines, TO, 5).origin, 550 * MS, "the destination waits the greater end and its own lead: {:?}", lanes(&lines, TO));
}

/// RM-34: a segment RM-25 leaves out of the plan that began after the move is no release
/// either: the source gains the channel again there, and the destination waits the move's own
/// cut.
#[test]
fn rm_34_a_move_does_not_wait_an_omitted_segment_the_source_gains_after_it() {
    let mut lines = lines_of(rx_bare(&[0], 1_000_000), rx_bare(&[], 1_000_000));
    let (e, _) = lines.book_move(FROM, TO, 0, Some(200 * MS), 200 * MS, 0, 200 * MS).unwrap();
    assert_eq!(e, 200 * MS);
    // The source gains channel 0 again at 650 ms, and a `Stop` at that instant cuts the segment
    // at its origin, so RM-25 leaves it out of the plan.
    lines.book(FROM, item(650 * MS, 1, 650 * MS, Kind::Cold(set_config(&[0], 1_000_000)))).unwrap();
    lines.book(FROM, item(650 * MS, 2, 650 * MS, Kind::Stop)).unwrap();
    assert_eq!(planned_at(&lines, FROM), [(0, Some(200 * MS))], "the segment gained afterwards is left out of the plan");
    assert_eq!(released(&lines, FROM, 0, 0), Some(200 * MS), "it began after the move, so it is no release of it");
    assert_eq!(by(&lines, TO, 0).origin, 200 * MS, "the destination is not floored by it: {:?}", lanes(&lines, TO));
}

/// RM-34, RM-25: a move between transmit streams books both lines as a `cold` change does,
/// and a refused move ends each clock at its origin (VH-2).
#[test]
fn rm_34_a_move_books_both_transmit_clocks() {
    let mut lines = lines_of(
        Stream { direction: Direction::Tx, ..rx_stream(&[0, 1], 1_000_000) },
        Stream { direction: Direction::Tx, ..rx_stream(&[2], 2_000_000) },
    );
    let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 500 * MS, 0, 500 * MS).unwrap();
    assert_eq!(e, 1_000 * MS);
    assert_eq!(by(&lines, FROM, 0).origin, by(&lines, TO, 0).origin, "both clocks at one instant");
    assert_eq!(by(&lines, FROM, 0).origin, 1_050 * MS);
    assert_eq!(by(&lines, FROM, 0).config.channels.iter().collect::<Vec<_>>(), [0]);
    assert_eq!(by(&lines, TO, 0).config.channels.iter().collect::<Vec<_>>(), [1, 2]);
    assert!(lines.refuse_move(0).unwrap());
    for stream in [FROM, TO] {
        assert_eq!(planned_at(&lines, stream).last().copied(), Some((1_050 * MS, Some(1_050 * MS))), "{stream:?}");
    }
}

/// RM-34: a move the lines cannot make is refused and neither line is changed: a number that
/// is no channel, a channel the source does not hold, one the destination already holds, a
/// move into the channel's own stream, and one between two directions.
#[test]
fn rm_34_an_impossible_move_is_refused_without_a_booking() {
    let mut lines = lines_of(rx_stream(&[0, 1, 2], 1_000_000), rx_stream(&[2], 1_000_000));
    // A transmit stream of the instance: a channel number names a channel of its own direction
    // alone (RM-27), so a move across the two is no move.
    lines.insert((Direction::Tx, 0), Line::new(Stream { direction: Direction::Tx, ..rx_stream(&[], 1_000_000) }));
    let before = lines.clone();
    for (channel, from, to, error) in [
        (64, FROM, TO, MoveError::NoSuchChannel),
        (100, FROM, TO, MoveError::NoSuchChannel),
        (5, FROM, TO, MoveError::NotInSource),
        (3, TO, FROM, MoveError::NotInSource),
        (2, FROM, TO, MoveError::AlreadyInDestination),
        (0, FROM, FROM, MoveError::SameStream),
        (0, FROM, (Direction::Tx, 0), MoveError::DifferentDirections),
        (0, (Direction::Tx, 0), FROM, MoveError::DifferentDirections),
    ] {
        assert_eq!(lines.book_move(from, to, channel, Some(1_000 * MS), 500 * MS, 0, 500 * MS), Err(error), "channel {channel}");
        assert_eq!(lines, before, "channel {channel} changes no line");
    }
}

/// RM-34, #63: `config_at` is the configuration a line leaves in effect at an instant, a
/// move's and the pruned state included, which a Provider builds a later `cold` change from.
#[test]
fn rm_34_config_at_reads_the_move_and_the_pruned_state() {
    let mut lines = lines_of(rx_stream(&[0, 1], 1_000_000), rx_stream(&[2], 1_000_000));
    let (e, _) = lines.book_move(FROM, TO, 1, Some(1_000 * MS), 500 * MS, 0, 500 * MS).unwrap();
    assert_eq!(lines.config_at(FROM, e - 1).channels.iter().collect::<Vec<_>>(), [0, 1], "before the move");
    assert_eq!(lines.config_at(FROM, e).channels.iter().collect::<Vec<_>>(), [0], "the source's set without the channel");
    assert_eq!(lines.config_at(TO, e).channels.iter().collect::<Vec<_>>(), [1, 2], "the destination's with it");
    assert_eq!(lines.config_at(TO, e).ratio, lines.config_at(TO, e - 1).ratio, "each keeps its rate");
    // Pruning folds the items into the state the line is planned from, which `config_at`
    // reads from then on.
    lines.prune(2_000 * MS);
    assert!(lines.line(FROM).unwrap().items.is_empty(), "the items are folded");
    assert_eq!(lines.config_at(FROM, e - 1).channels.iter().collect::<Vec<_>>(), [0], "the pruned state, not the Run's");
    assert_eq!(lines.config_at(FROM, 3_000 * MS).channels.iter().collect::<Vec<_>>(), [0]);
    // The pruning folds the move's two items away with its record, so that a later booking
    // plans the lines as the pruning left them.
    let to = planned_at(&lines, TO);
    lines.book(FROM, item(2_500 * MS, 1, 2_500 * MS, Kind::Cold(set_config(&[0], 1_000_000)))).unwrap();
    assert_eq!(planned_at(&lines, TO), to, "the pruned move leaves the destination's plan as it was");
}

/// VH-8 and RM-34: with a move attempted among a generated sequence's items, both lines'
/// plans keep RM-16's and RM-25's invariants, the destination keeps waiting the source's cut,
/// and a channel the source does not hold is refused with nothing changed.
#[test]
fn rm_34_generated_moves_keep_both_lines_invariants() {
    let (mut moved, mut refused) = (0, 0);
    let mut destination_segments = 0;
    for seed in 0..500 {
        let sequence = generator::sequence(seed, &generator::FRACTIONAL_RATES);
        let from = Stream { origin: 2_000 * MS, call: 2_000, allowance: 3 * MS, ..rx_stream(&[0, 1], 1_000_000) };
        let to = Stream { origin: 2_000 * MS, call: 2_000, allowance: 3 * MS, ..rx_stream(&[2], 2_000_000) };
        let terms = Terms { t0: 2_000 * MS, root_hz: 1_000_000_000, timed_lead: 2 * MS, release_window: 3 * MS, stream: from };
        let items = sequence.items(&terms);
        let mut lines = Lines::new();
        lines.insert(FROM, Line::new(from));
        lines.insert(TO, Line::new(to));
        let half = items.len() / 2;
        for booked in &items[..half] {
            lines.book(FROM, *booked).unwrap();
        }
        let at = 1_000 * MS + 100 * MS * (seed % 5) as i64;
        let seq = 1_000_000 + seed;
        let before = lines.clone();
        match lines.book_move(FROM, TO, 1, Some(at), at, seq, at) {
            Ok(_) => {
                moved += 1;
                // The items after the move are booked after it, so they take the arrival orders
                // after its own, on the destination as well: its plan then holds segments after
                // the move whose origin the release floors too; only both plans' invariants are
                // checked, not the items' order.
                for booked in &items[half..] {
                    lines.book(FROM, Item { seq: booked.seq + 2_000_000, ..*booked }).unwrap();
                    lines.book(TO, Item { seq: booked.seq + 3_000_000, ..*booked }).unwrap();
                }
                check(&from, &lines.line(FROM).unwrap().plan, seed);
                check(&to, &lines.line(TO).unwrap().plan, seed);
                destination_segments += check_move(&lines, FROM, TO, 1, seq, seed);
            }
            Err(error) => {
                // A source the sequence left holding no channel has nothing to move.
                assert_eq!(error, MoveError::NotInSource, "seed {seed}");
                assert_eq!(lines, before, "seed {seed}: a refused move changes no line");
                refused += 1;
            }
        }
    }
    assert_eq!(moved + refused, 500, "every generated sequence moves or is refused");
    assert!(moved > 0 && refused > 0, "both a move and its refusal are generated: {moved}, {refused}");
    assert!(destination_segments > 0, "the destination's segments at or after the move are checked");
}

/// RM-34 across two lines of one instance: every segment of the destination at or after the
/// move `seq` starts a lead after what the source's plan releases the moved channel at
/// (RM-16), and the claim is empty alone when the source releases nothing. Returns how many
/// such segments the plan held, so that the caller can see the check was exercised.
///
/// RM-34 floors the items that take effect at or after the move, RM-25's order; the generated
/// sequence's items take effect from T0's second, and the move's instant is floored at the
/// lines' starts, so every destination segment this check selects is one the floor covers.
fn check_move(lines: &Lines, from: StreamKey, to: StreamKey, channel: u32, seq: u64, seed: u64) -> u64 {
    let lead = lines.line(to).expect("the instance holds the destination").stream.lead;
    let release = released(lines, from, channel, seq);
    let destination = lines.line(to).expect("the instance holds the destination");
    let mut checked = 0;
    for segment in &destination.plan {
        if segment.by.is_some_and(|by| by >= seq) {
            checked += 1;
            if let Some(release) = release {
                assert!(
                    segment.origin >= release + lead,
                    "seed {seed}: a destination segment starts before the source's release: {segment:?}, {release}",
                );
            }
        }
    }
    checked
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
    for rejected in [
        "radio/rx", "radio/tx", "radio/rx/s", "radio/rx/s01", "radio/rx/s0x",
        "radio/rx/c0", "radio/rx/s1234567890", "radio/s0", "radio/rx/s0/deeper", "radio/other/s0",
    ] {
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
fn rm_25_the_layout_instant_is_floored_at_every_earlier_cold_change() {
    // RM-25 (design 33 D5, item 11): the one place the layout instance's floor lives. The
    // `e` is what the change takes effect at, and `late` says whether the requested instant
    // held; `earlier` is the effective instants of the instance's still-pending `cold`
    // changes, whichever direction, stream or line.
    type Case = (Option<i64>, i64, Option<i64>, &'static [i64], i64, bool);
    let cases: [Case; 8] = [
        (Some(10), 0, None, &[], 10, false),
        (Some(10), 0, None, &[50], 50, true),
        (None, 5, None, &[50], 50, true),
        (Some(10), 20, None, &[], 20, true),
        (Some(10), 0, Some(30), &[], 30, true),
        (Some(30), 0, Some(30), &[], 30, false),
        (Some(10), 0, None, &[50, 20], 50, true),
        (Some(50), 0, None, &[10], 50, false),
    ];
    for (at, now, start, earlier, e, late) in cases {
        assert_eq!(
            ezsdr_radio::timeline::layout_instant(at, now, start, earlier.iter().copied()),
            (e, late),
            "at={at:?} now={now} start={start:?} earlier={earlier:?}"
        );
    }
}

#[test]
fn rm_25_the_timed_instant_is_floored_at_every_booked_command_inside_the_window() {
    // RM-21 (design 38 D2 (1)): the one place a timed update's floor lives. `e` is what it
    // takes effect at, and `late` whether the requested instant held; `earlier` is the
    // instants of the timed updates booked before it, whether given, waiting or refused.
    // window 30: an instant beyond now + 30 is outside it and floors nothing.
    type Case = (Option<i64>, i64, i64, i64, &'static [i64], i64, bool);
    let cases: [Case; 11] = [
        (Some(100), 0, 20, 30, &[], 100, false),
        (Some(25), 0, 20, 30, &[30], 30, true),
        (Some(45), 0, 20, 30, &[50], 45, false),
        (Some(25), 0, 20, 30, &[25], 25, false),
        (Some(25), 0, 20, 30, &[26], 26, true),
        (None, 0, 20, 30, &[25], 25, false),
        (Some(35), 25, 20, 30, &[50], 50, true),
        (Some(80), 10, 20, 30, &[50], 80, false),
        (Some(10), 40, 20, 30, &[], 60, true),
        (Some(5), 10, 0, 0, &[10], 10, true),
        (Some(10), 10, 0, 0, &[10], 10, false),
    ];
    for (at, now, lead, window, earlier, e, late) in cases {
        assert_eq!(
            ezsdr_radio::timeline::timed_instant(at, now, lead, window, earlier.iter().copied()),
            (e, late),
            "at={at:?} now={now} lead={lead} window={window} earlier={earlier:?}"
        );
    }
}

#[test]
fn mr_29_a_receive_link_floors_a_command_at_its_t0() {
    // MR-29 (RM-25): a `start_rx` takes effect at its `at`, or at its receipt when that is
    // later, and a receive link never before T0. Both here lie before T0; a transmit link
    // has no such floor.
    let t0 = 10 * MS;
    let receive = Line::new(Stream { origin: t0, ..stream(Direction::Rx, 1) });
    assert_eq!(ezsdr_radio::timeline::command_instant(Some(&receive), Some(t0 - 2 * MS), t0 - 3 * MS, 0, false), (t0, false));
    let transmit = Line::new(Stream { origin: t0, ..stream(Direction::Tx, 1) });
    assert_eq!(
        ezsdr_radio::timeline::command_instant(Some(&transmit), Some(t0 - 2 * MS), t0 - 3 * MS, 0, false),
        (t0 - 2 * MS, false),
        "a transmit link takes `max(at, now)`",
    );
}

#[test]
fn rm_36_covering_boundaries() {
    // RM-36: a segment covers `t` from its origin and, if cut, until the instant of its cut
    // sample; at the 3/2 and 1/3 ratios that instant is rounded up (RM-16).
    let at = |origin: i64, cut: Option<i64>, numbers: &[u32], rate: u64| Segment { origin, cut, config: set_config(numbers, rate), by: None };
    let open = at(10 * MS, None, &[0], 1_000_000);
    assert_eq!(covering([&open], 0, 10 * MS - 1), None);
    assert_eq!(covering([&open], 0, 10 * MS), Some(&open));
    assert_eq!(covering([&open], 0, 10 * MS + 1), Some(&open));
    assert_eq!(covering([&open], 0, i64::MAX), Some(&open));
    assert_eq!(covering([&open], 1, 20 * MS), None, "another channel");
    assert_eq!(covering(&[] as &[Segment], 0, 20 * MS), None);
    // Cut at sample 5 of 1 MS/s: instant 10 ms + 5 us.
    let cut = at(10 * MS, Some(5), &[0, 1], 1_000_000);
    let end = cut.instant(5).unwrap();
    assert_eq!(end, 10 * MS + 5_000);
    assert_eq!(covering([&cut], 1, end - 1), Some(&cut));
    assert_eq!(covering([&cut], 1, end), None);
    assert_eq!(covering([&cut], 1, end + 1), None);
    // A cut at sample 0 ends at the origin: nothing passes.
    let empty = at(10 * MS, Some(0), &[0], 1_000_000);
    assert_eq!(covering([&empty], 0, 10 * MS), None);
    // Ratios whose sample instants are rounded up.
    for (num, den, k, expected) in [(3, 2, 3, 10 * MS + 5), (1, 3, 7, 10 * MS + 3)] {
        let odd = Segment { origin: 10 * MS, cut: Some(k), config: Config { channels: channels(&[0]), ratio: Rational::new(num, den).unwrap() }, by: None };
        let end = odd.instant(k).unwrap();
        assert_eq!(end, expected, "ratio {num}/{den}");
        assert_eq!(covering([&odd], 0, end - 1), Some(&odd), "ratio {num}/{den}");
        assert_eq!(covering([&odd], 0, end), None, "ratio {num}/{den}");
    }
    // An instant that cannot be computed covers nothing.
    let overflow = Segment { origin: i64::MAX - 1, cut: Some(i64::MAX), config: set_config(&[0], 1), by: None };
    assert!(overflow.instant(i64::MAX).is_err());
    assert_eq!(covering([&overflow], 0, i64::MAX), None);
    // Of several segments the one that passes at `t` is returned, not the first listed.
    let first = at(0, Some(5), &[0], 1_000_000);
    let second = at(30 * MS, None, &[0], 1_000_000);
    assert_eq!(covering([&first, &second], 0, 40 * MS), Some(&second));
    assert_eq!(covering([&first, &second], 0, 2_000), Some(&first));
    assert_eq!(covering([&first, &second], 0, 20 * MS), None);
}
