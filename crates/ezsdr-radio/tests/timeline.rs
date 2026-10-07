//! RM-26: `ezsdr_radio::timeline`, RM-16, RM-21 and RM-25 computed once (spec 22, VH-7),
//! and its invariants over VH-8's generated sequences.

mod generator;

use ezsdr_kernel::stream::Direction;
use ezsdr_kernel::time::Rational;
use ezsdr_radio::timeline::{Config, Item, Kind, Segment, Stream, plan};
use generator::{CLASSES, Terms};

/// Root ticks per millisecond on the 1 GHz root.
const MS: i64 = 1_000_000;

fn config(channels: u16, rate: u64) -> Config {
    Config { channels, ratio: Rational::new(1_000_000_000, rate).unwrap() }
}

/// A stream at 1 MS/s from 0 with a 50 ms lead and no receive call (Appendix C's terms).
fn stream(direction: Direction, channels: u16) -> Stream {
    Stream { direction, origin: 0, config: config(channels, 1_000_000), lead: 50 * MS, call: 0, allowance: 0 }
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
            let terms = Terms { t0: 2_000 * MS, root_hz: 1_000_000_000, timed_lead: 2 * MS, stream };
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

fn check(stream: &Stream, planned: &[Segment], seed: u64) {
    let receive = stream.direction == Direction::Rx;
    for (index, segment) in planned.iter().enumerate() {
        let at = format!("seed {seed}, {:?} segment {index}: {planned:?}", stream.direction);
        // RM-25: on the lattice, the first at the stream's origin.
        if index > 0 || segment.origin != stream.origin {
            assert_eq!(segment.origin % segment.config.ratio.num() as i64, 0, "{at}");
        }
        assert!(segment.config.channels > 0, "{at}");
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
