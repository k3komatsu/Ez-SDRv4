//! E3a-1 (design note 49 stage E3a; plan `2026-11-12-e3/test-plan.md` §2): today's send decisions of `Tx::send` pinned before they move into
//! `ezsdr_radio` as pure functions. Each test ends in ordered snapshots (the device's sends, the chain's front, cursor and held tail, the
//! open booking, the Kernel burst records and the stats) written out from the base run. The rigs are the single-threaded ones of
//! `txp_rig`, their clock held by the test thread: device time moves only through `device_at`, so no verdict depends on when a thread runs (note 51 §3), and nothing here
//! offers a margin or a repetition count as its guarantee.
//!
//! Units: one sample is 1 us at 1 MS/s (200 root ticks) unless a case says otherwise.

use super::*;

/// The snapshots a test expects, as `(case, lines)`; a case that differs, or that the table lacks, is reported in the form that is pasted
/// back, so that a snapshot is written from a base run and then only read.
pub(super) type Expected = &'static [(&'static str, &'static [&'static str])];

pub(super) struct Cases {
    expected: Expected,
    checked: Vec<String>,
    differs: Vec<String>,
}

impl Cases {
    pub(super) fn new(expected: Expected) -> Cases {
        Cases { expected, checked: Vec::new(), differs: Vec::new() }
    }

    pub(super) fn check(&mut self, case: impl Into<String>, actual: Vec<String>) {
        let case = case.into();
        let want = self.expected.iter().find(|(name, _)| *name == case).map(|(_, lines)| *lines);
        if want != Some(actual.iter().map(String::as_str).collect::<Vec<_>>().as_slice()) {
            let body: String = actual.iter().map(|line| format!("        \"{line}\",\n")).collect();
            self.differs.push(format!("    (\"{case}\", &[\n{body}    ]),\n"));
        }
        self.checked.push(case);
    }

    pub(super) fn finish(self) {
        let stale: Vec<&str> = self.expected.iter().map(|(name, _)| *name).filter(|name| !self.checked.iter().any(|case| case == name)).collect();
        assert!(self.differs.is_empty() && stale.is_empty(), "snapshots that differ from the written ones (stale cases: {stale:?}):\n{}", self.differs.concat());
    }
}

/// What a drain left: every send of the device in order, the chain's front, cursor and held tail, the open booking, the Kernel burst
/// records and the stats (the lock-wait maximum left out: it measures contention, not behaviour).
fn snap(core: &Core, device: &FakeDevice, tx: &Tx) -> Vec<String> {
    let mut lines: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("tx_send ")).collect();
    lines.push(format!("front {:?}", tx.chain().front().map(|(_, sample)| sample)));
    lines.push(format!("chain next {:?} tail {}", tx.chain().next(), tx.chain().tail().is_some()));
    lines.push(format!("open next {:?} first {:?}", tx.open_next(), tx.open_first()));
    let rec = lock(&core.rec);
    for row in &rec.bursts {
        lines.push(format!("record {} samples {} wraps {} blocks {} end {}", row["target"]["ticks"], row["samples"], row["wraps"], row["blocks"], row["end"].as_str().unwrap_or("?")));
    }
    lines.push(format!("rejected {}", rec.rejected.len()));
    let stats: Vec<String> = rec.stats.iter().filter(|(name, _)| **name != "tx_lock_wait_max_ns").map(|(name, n)| format!("{name}={n}")).collect();
    lines.push(format!("stats {}", stats.join(" ")));
    lines
}

// ------------------------------------------------------------------------------------------------------------ C1

#[test]
fn e3_c1_a_lone_finite_burst_by_length() {
    // A finite burst alone at 5 000 (the record holds it, so that a later booking can still join it): the padding of 1 to 3 samples, the
    // short first send of at most 32, the held last sample of a longer one and the close at `J(e)`, written out for each length. The first
    // snapshot is the drain at device time 0 (everything the window takes), the second the drain past the join deadline.
    let mut cases = Cases::new(C1);
    for len in [1usize, 2, 3, 4, 5, 31, 32, 33, 34, 40, 100] {
        let (core, device, time, _events, clock, mut tx, _guard) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, len, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        cases.check(format!("len {len}, drained at device time 0"), snap(&core, &device, &tx));
        device_at(&core, &time, &device, core.join_by(clock, 5_000 + len as i64) + 1);
        txp_drain(&mut tx);
        cases.check(format!("len {len}, past J(e)"), snap(&core, &device, &tx));
    }
    cases.finish();
}

// ------------------------------------------------------------------------------------------------------------ C2

/// A signature of what the owner has done: the sends so far, where the open booking stands and whether a sample is held.
fn signature(device: &FakeDevice, tx: &Tx) -> String {
    format!("sends {} open next {:?} chain next {:?} tail {}", sends(device).len(), tx.open_next(), tx.chain().next(), tx.chain().tail().is_some())
}

/// The owner at each device time `from..=to` (samples, one a step), one line for the time a signature first appears: where a send goes
/// is where the window first takes what the rule needs, so a threshold off by one moves a line.
fn first_times(core: &Core, time: &Arc<ezsdr_kernel::time::ManualTimeAuthority>, device: &FakeDevice, tx: &mut Tx, from: i64, to: i64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut last = String::new();
    for t in from..=to {
        device_at(core, time, device, txp_ticks(t));
        txp_drain(tx);
        let now = signature(device, tx);
        if now != last {
            lines.push(format!("t={t}: {now}"));
            last = now;
        }
    }
    lines.push(format!("calls {:?}", sends(device)));
    lines
}

#[test]
fn e3_c2_the_first_send_window_edge() {
    // The window is 2.1 ms and the booking starts at 3 000, so the window holds `t - 900` samples of it at device time `t`: a first send goes
    // the first time it holds what the rule needs (the burst's own length up to 32, 32 beyond), a continuation the first time it holds
    // all it wants. A tiled repeat's first send is 32 and its continuation a whole 525-sample chunk.
    let mut cases = Cases::new(C2);
    for len in [10usize, 31, 33] {
        let (core, device, time, _events, clock, mut tx, _guard) = txp_rig(Some(2_100_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, len, false, LatePolicy::DropAndFlag));
        let lines = first_times(&core, &time, &device, &mut tx, 900, 960);
        cases.check(format!("finite {len}"), lines);
    }
    for len in [1usize, 5, 31] {
        let (core, device, time, _events, clock, mut tx, _guard) = txp_rig(Some(2_100_000), 2_000);
        let mut burst = txp_burst(&clock, 3_000, len, true, LatePolicy::DropAndFlag);
        burst.tile_repeat();
        txp_give(&mut tx, burst);
        let lines = first_times(&core, &time, &device, &mut tx, 900, 1_500);
        cases.check(format!("tiled repeat of {len}"), lines);
    }
    // A continuation of 2 500 on the default 10 ms window: two chunks go at once, the third when the window holds all of it.
    let (core, device, time, _events, clock, mut tx, _guard) = txp_rig(None, 2_000);
    txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, false, LatePolicy::DropAndFlag));
    cases.check("finite 30 000, a continuation of 2 500", first_times(&core, &time, &device, &mut tx, 0, 600));
    cases.finish();
}

// ------------------------------------------------------------------------------------------------------------ C3

#[test]
fn e3_c3_joined_pieces() {
    // Bookings back to back: each piece continues the device burst of the one before, so only the first is a first send; a piece's end is the
    // device burst's only where nothing follows. A(40)+B(10)+C(4)+D(1), A(100)+B(1) and A(33)+B(33), drained at device time 0 and past `J` of
    // the last end. The one-sample piece at the end is padded to two, not to four.
    let mut cases = Cases::new(C3);
    for (name, pieces) in [("A40 B10 C4 D1", vec![40usize, 10, 4, 1]), ("A100 B1", vec![100, 1]), ("A33 B33", vec![33, 33])] {
        let (core, device, time, _events, clock, mut tx, _guard) = txp_rig(None, 2_000);
        let mut start = 5_000;
        for len in &pieces {
            txp_book(&mut tx, &core, clock, txp_burst(&clock, start, *len, false, LatePolicy::DropAndFlag));
            start += *len as i64;
        }
        txp_drain(&mut tx);
        cases.check(format!("{name}, drained at device time 0"), snap(&core, &device, &tx));
        device_at(&core, &time, &device, core.join_by(clock, start) + 1);
        txp_drain(&mut tx);
        cases.check(format!("{name}, past J(e)"), snap(&core, &device, &tx));
    }
    cases.finish();
}

// ------------------------------------------------------------------------------------------------------------ C4

#[test]
fn e3_c4_the_cut_at_a_held_start() {
    // The send is cut at the next held burst's start: the Kernel burst ends there (`eob` in the record), the device burst goes on unless the
    // cut send is a short first send. The bursts are given without the record, so a booking inside a repeat's span is possible: the shapes a
    // switch boundary makes (a repeat cut at a wrap, mid-period, before 32 samples of it were sent) and a finite burst cut by a booking at its
    // end, inside it, and a repeat whose wrap is the cut.
    let mut cases = Cases::new(C4);
    // (name, first burst (length, repeat), held burst's start offset from the first's start and its length)
    for (name, first, repeat, held_at, held_len) in [
        ("repeat 1000, held at +1500 (mid-period)", 1_000usize, true, 1_500i64, 300usize),
        ("finite 100, held at +60 (inside)", 100, false, 60, 50),
        ("F3a: repeat 1000, held at +20 (before 32 samples)", 1_000, true, 20, 300),
        ("finite 100, held at +100 (its end)", 100, false, 100, 50),
        ("repeat 1000, held at +1000 (its wrap)", 1_000, true, 1_000, 300),
    ] {
        let (core, device, time, _events, clock, mut tx, _guard) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 5_000, first, repeat, LatePolicy::DropAndFlag));
        txp_give(&mut tx, txp_burst(&clock, 5_000 + held_at, held_len, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        cases.check(format!("{name}, drained at device time 0"), snap(&core, &device, &tx));
        device_at(&core, &time, &device, core.join_by(clock, 5_000 + held_at + held_len as i64) + 1);
        txp_drain(&mut tx);
        cases.check(format!("{name}, past J(e)"), snap(&core, &device, &tx));
    }
    cases.finish();
}

// ------------------------------------------------------------------------------------------------------------ C5

#[test]
fn e3_c5_the_hold_deadline() {
    // `open_ended = instant(next + count) - now > device lead` (strict): a burst of 100 at 5 000 whose first 32 samples go at 2 932 (window 2.1
    // ms) and whose last 68 go when the owner's clock is `lead + 1 tick`, `lead` and `lead - 1 tick` before the burst's end. Past the
    // deadline the last send carries the end of burst itself; before it the last sample is held back and the close that follows in the same
    // drain (the join deadline of the burst's end is long past) sends it with the end of burst.
    let mut cases = Cases::new(C5);
    for (name, delta) in [("lead + 1 tick before the end", 1i64), ("exactly the lead before the end", 0), ("lead - 1 tick before the end", -1)] {
        let (core, device, time, _events, clock, mut tx, _guard) = txp_rig(Some(2_100_000), 2_000);
        let lead = core.device_lead();
        assert_eq!(lead, 400_000, "the device lead is 2 ms of root ticks");
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 100, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, txp_ticks(2_932));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(32, true, false)], "{name}: the first send");
        let now = clock.instant(5_100) - lead - delta;
        device_at(&core, &time, &device, now);
        txp_drain(&mut tx);
        cases.check(format!("{name}, the last send and the close after it"), snap(&core, &device, &tx));
    }
    cases.finish();
}

// ------------------------------------------------------------------------------------------------------------ C6

#[test]
fn e3_c6_the_send_cap_by_window_and_rate() {
    // The cap is a quarter of the in-flight window in samples, clamped to 32 .. 20 000: a long repeat booked 3 ms ahead of device time 0 (2.05 ms
    // ahead for the 2.1 ms window, whose first send is clipped to the 50 samples the window holds) goes out in chunks of it, written out for each
    // window and rate (root ticks a sample `n`). A window of no more than the device lead plus 32
    // samples can hand nothing over before its booking is late.
    let mut cases = Cases::new(C6);
    for (window_ns, rate, n, start_us) in [
        (10_000_000i64, 1e6, 200i64, 3_000),
        (10_000_000, 200e6, 1, 3_000),
        (4_000_000, 1e6, 200, 3_000),
        (2_100_000, 1e6, 200, 2_050),
        (10_000_000, 25e3, 8_000, 3_000),
        (10_000_000, 12.5e3, 16_000, 3_000),
    ] {
        let (_core, device, _time, _events, clock, mut tx, _guard) = txp_rig_rate(Some(window_ns), 2_000, rate, n);
        let start = (start_us as f64 * 1e-6 * rate).ceil() as i64;
        txp_give(&mut tx, txp_burst(&clock, start, 100_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        let counts: Vec<usize> = sends(&device).iter().map(|send| send.0).collect();
        cases.check(format!("window {window_ns} ns, {rate} S/s"), vec![format!("first sends {:?} of {}", &counts[..counts.len().min(3)], counts.len()), format!("open next {:?}", tx.open_next())]);
    }
    for window_ns in [1_000_000i64, 100_000] {
        let (core, device, time, events, clock, mut tx, _guard) = txp_rig(Some(window_ns), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 100_000, true, LatePolicy::DropAndFlag));
        for t in 0..=3_200 {
            device_at(&core, &time, &device, txp_ticks(t));
            txp_drain(&mut tx);
        }
        let late = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).count();
        cases.check(format!("window {window_ns} ns, device time 0 .. 3 200"), vec![format!("sends {} time errors {late} open next {:?}", sends(&device).len(), tx.open_next())]);
    }
    cases.finish();
}

// ------------------------------------------------------------------------------------------------------------ today's snapshots

const C1: Expected = &[
    (
        "len 1, drained at device time 0",
        &[
            "tx_send 0 n=4 at=1000000 sob=true eob=true",
            "front Some(5019)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 1 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=1",
        ],
    ),
    (
        "len 1, past J(e)",
        &[
            "tx_send 0 n=4 at=1000000 sob=true eob=true",
            "front Some(5019)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 1 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=1",
        ],
    ),
    (
        "len 2, drained at device time 0",
        &[
            "tx_send 0 n=4 at=1000000 sob=true eob=true",
            "front Some(5019)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 2 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=2",
        ],
    ),
    (
        "len 2, past J(e)",
        &[
            "tx_send 0 n=4 at=1000000 sob=true eob=true",
            "front Some(5019)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 2 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=2",
        ],
    ),
    (
        "len 3, drained at device time 0",
        &[
            "tx_send 0 n=4 at=1000000 sob=true eob=true",
            "front Some(5019)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 3 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=3",
        ],
    ),
    (
        "len 3, past J(e)",
        &[
            "tx_send 0 n=4 at=1000000 sob=true eob=true",
            "front Some(5019)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 3 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=3",
        ],
    ),
    (
        "len 4, drained at device time 0",
        &[
            "tx_send 0 n=4 at=1000000 sob=true eob=true",
            "front Some(5019)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 4 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=4",
        ],
    ),
    (
        "len 4, past J(e)",
        &[
            "tx_send 0 n=4 at=1000000 sob=true eob=true",
            "front Some(5019)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 4 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=4",
        ],
    ),
    (
        "len 5, drained at device time 0",
        &[
            "tx_send 0 n=5 at=1000000 sob=true eob=true",
            "front Some(5020)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 5 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=5",
        ],
    ),
    (
        "len 5, past J(e)",
        &[
            "tx_send 0 n=5 at=1000000 sob=true eob=true",
            "front Some(5020)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 5 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=5",
        ],
    ),
    (
        "len 31, drained at device time 0",
        &[
            "tx_send 0 n=31 at=1000000 sob=true eob=true",
            "front Some(5046)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 31 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=31",
        ],
    ),
    (
        "len 31, past J(e)",
        &[
            "tx_send 0 n=31 at=1000000 sob=true eob=true",
            "front Some(5046)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 31 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=31",
        ],
    ),
    (
        "len 32, drained at device time 0",
        &[
            "tx_send 0 n=32 at=1000000 sob=true eob=true",
            "front Some(5047)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 32 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=32",
        ],
    ),
    (
        "len 32, past J(e)",
        &[
            "tx_send 0 n=32 at=1000000 sob=true eob=true",
            "front Some(5047)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 32 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=32",
        ],
    ),
    (
        "len 33, drained at device time 0",
        &[
            "tx_send 0 n=32 at=1000000 sob=true eob=false",
            "front None",
            "chain next Some(5033) tail true",
            "open next None first None",
            "record 5000 samples 33 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=33",
        ],
    ),
    (
        "len 33, past J(e)",
        &[
            "tx_send 0 n=32 at=1000000 sob=true eob=false",
            "tx_send 0 n=2 at=1006400 sob=false eob=true",
            "front Some(5037)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 33 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=33",
        ],
    ),
    (
        "len 34, drained at device time 0",
        &[
            "tx_send 0 n=33 at=1000000 sob=true eob=false",
            "front None",
            "chain next Some(5034) tail true",
            "open next None first None",
            "record 5000 samples 34 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=34",
        ],
    ),
    (
        "len 34, past J(e)",
        &[
            "tx_send 0 n=33 at=1000000 sob=true eob=false",
            "tx_send 0 n=2 at=1006600 sob=false eob=true",
            "front Some(5038)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 34 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=34",
        ],
    ),
    (
        "len 40, drained at device time 0",
        &[
            "tx_send 0 n=39 at=1000000 sob=true eob=false",
            "front None",
            "chain next Some(5040) tail true",
            "open next None first None",
            "record 5000 samples 40 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=40",
        ],
    ),
    (
        "len 40, past J(e)",
        &[
            "tx_send 0 n=39 at=1000000 sob=true eob=false",
            "tx_send 0 n=2 at=1007800 sob=false eob=true",
            "front Some(5044)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 40 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=40",
        ],
    ),
    (
        "len 100, drained at device time 0",
        &[
            "tx_send 0 n=99 at=1000000 sob=true eob=false",
            "front None",
            "chain next Some(5100) tail true",
            "open next None first None",
            "record 5000 samples 100 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=100",
        ],
    ),
    (
        "len 100, past J(e)",
        &[
            "tx_send 0 n=99 at=1000000 sob=true eob=false",
            "tx_send 0 n=2 at=1019800 sob=false eob=true",
            "front Some(5104)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 100 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=100",
        ],
    ),
];
const C2: Expected = &[
    (
        "finite 10",
        &[
            "t=900: sends 0 open next None chain next None tail false",
            "t=901: sends 0 open next Some(3000) chain next None tail false",
            "t=910: sends 1 open next None chain next None tail false",
            "calls [(10, true, true)]",
        ],
    ),
    (
        "finite 31",
        &[
            "t=900: sends 0 open next None chain next None tail false",
            "t=901: sends 0 open next Some(3000) chain next None tail false",
            "t=931: sends 1 open next None chain next None tail false",
            "calls [(31, true, true)]",
        ],
    ),
    (
        "finite 33",
        &[
            "t=900: sends 0 open next None chain next None tail false",
            "t=901: sends 0 open next Some(3000) chain next None tail false",
            "t=932: sends 1 open next Some(3032) chain next Some(3032) tail false",
            "t=933: sends 2 open next None chain next None tail false",
            "calls [(32, true, false), (2, false, true)]",
        ],
    ),
    (
        "tiled repeat of 1",
        &[
            "t=900: sends 0 open next None chain next None tail false",
            "t=901: sends 0 open next Some(3000) chain next None tail false",
            "t=932: sends 1 open next Some(3032) chain next Some(3032) tail false",
            "t=1457: sends 2 open next Some(3557) chain next Some(3557) tail false",
            "calls [(32, true, false), (525, false, false)]",
        ],
    ),
    (
        "tiled repeat of 5",
        &[
            "t=900: sends 0 open next None chain next None tail false",
            "t=901: sends 0 open next Some(3000) chain next None tail false",
            "t=932: sends 1 open next Some(3032) chain next Some(3032) tail false",
            "t=1457: sends 2 open next Some(3557) chain next Some(3557) tail false",
            "calls [(32, true, false), (525, false, false)]",
        ],
    ),
    (
        "tiled repeat of 31",
        &[
            "t=900: sends 0 open next None chain next None tail false",
            "t=901: sends 0 open next Some(3000) chain next None tail false",
            "t=932: sends 1 open next Some(3032) chain next Some(3032) tail false",
            "t=1457: sends 2 open next Some(3557) chain next Some(3557) tail false",
            "calls [(32, true, false), (525, false, false)]",
        ],
    ),
    (
        "finite 30 000, a continuation of 2 500",
        &[
            "t=0: sends 2 open next Some(8000) chain next Some(8000) tail false",
            "t=500: sends 3 open next Some(10500) chain next Some(10500) tail false",
            "calls [(2500, true, false), (2500, false, false), (2500, false, false)]",
        ],
    ),
];
const C3: Expected = &[
    (
        "A40 B10 C4 D1, drained at device time 0",
        &[
            "tx_send 0 n=40 at=1000000 sob=true eob=false",
            "tx_send 0 n=10 at=1008000 sob=false eob=false",
            "tx_send 0 n=4 at=1010000 sob=false eob=false",
            "front None",
            "chain next Some(5055) tail true",
            "open next None first None",
            "record 5000 samples 40 wraps 1 blocks 1 end eob",
            "record 5040 samples 10 wraps 1 blocks 1 end eob",
            "record 5050 samples 4 wraps 1 blocks 1 end eob",
            "record 5054 samples 1 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=4 tx_errors=0 tx_samples=55",
        ],
    ),
    (
        "A40 B10 C4 D1, past J(e)",
        &[
            "tx_send 0 n=40 at=1000000 sob=true eob=false",
            "tx_send 0 n=10 at=1008000 sob=false eob=false",
            "tx_send 0 n=4 at=1010000 sob=false eob=false",
            "tx_send 0 n=2 at=1010800 sob=false eob=true",
            "front Some(5059)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 40 wraps 1 blocks 1 end eob",
            "record 5040 samples 10 wraps 1 blocks 1 end eob",
            "record 5050 samples 4 wraps 1 blocks 1 end eob",
            "record 5054 samples 1 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=4 tx_errors=0 tx_samples=55",
        ],
    ),
    (
        "A100 B1, drained at device time 0",
        &[
            "tx_send 0 n=100 at=1000000 sob=true eob=false",
            "front None",
            "chain next Some(5101) tail true",
            "open next None first None",
            "record 5000 samples 100 wraps 1 blocks 1 end eob",
            "record 5100 samples 1 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=101",
        ],
    ),
    (
        "A100 B1, past J(e)",
        &[
            "tx_send 0 n=100 at=1000000 sob=true eob=false",
            "tx_send 0 n=2 at=1020000 sob=false eob=true",
            "front Some(5105)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 100 wraps 1 blocks 1 end eob",
            "record 5100 samples 1 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=101",
        ],
    ),
    (
        "A33 B33, drained at device time 0",
        &[
            "tx_send 0 n=33 at=1000000 sob=true eob=false",
            "tx_send 0 n=32 at=1006600 sob=false eob=false",
            "front None",
            "chain next Some(5066) tail true",
            "open next None first None",
            "record 5000 samples 33 wraps 1 blocks 1 end eob",
            "record 5033 samples 33 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=66",
        ],
    ),
    (
        "A33 B33, past J(e)",
        &[
            "tx_send 0 n=33 at=1000000 sob=true eob=false",
            "tx_send 0 n=32 at=1006600 sob=false eob=false",
            "tx_send 0 n=2 at=1013000 sob=false eob=true",
            "front Some(5070)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 33 wraps 1 blocks 1 end eob",
            "record 5033 samples 33 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=66",
        ],
    ),
];
const C4: Expected = &[
    (
        "repeat 1000, held at +1500 (mid-period), drained at device time 0",
        &[
            "tx_send 0 n=1500 at=1000000 sob=true eob=false",
            "tx_send 0 n=299 at=1300000 sob=false eob=false",
            "tx_send 0 n=2 at=1359800 sob=false eob=true",
            "front Some(6804)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 1500 wraps 1 blocks 1 end eob",
            "record 6500 samples 300 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=1800",
        ],
    ),
    (
        "repeat 1000, held at +1500 (mid-period), past J(e)",
        &[
            "tx_send 0 n=1500 at=1000000 sob=true eob=false",
            "tx_send 0 n=299 at=1300000 sob=false eob=false",
            "tx_send 0 n=2 at=1359800 sob=false eob=true",
            "front Some(6804)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 1500 wraps 1 blocks 1 end eob",
            "record 6500 samples 300 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=1800",
        ],
    ),
    (
        "finite 100, held at +60 (inside), drained at device time 0",
        &[
            "tx_send 0 n=60 at=1000000 sob=true eob=false",
            "tx_send 0 n=49 at=1012000 sob=false eob=false",
            "tx_send 0 n=2 at=1021800 sob=false eob=true",
            "front Some(5114)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 60 wraps 0 blocks 1 end eob",
            "record 5060 samples 50 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=110",
        ],
    ),
    (
        "finite 100, held at +60 (inside), past J(e)",
        &[
            "tx_send 0 n=60 at=1000000 sob=true eob=false",
            "tx_send 0 n=49 at=1012000 sob=false eob=false",
            "tx_send 0 n=2 at=1021800 sob=false eob=true",
            "front Some(5114)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 60 wraps 0 blocks 1 end eob",
            "record 5060 samples 50 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=110",
        ],
    ),
    (
        "F3a: repeat 1000, held at +20 (before 32 samples), drained at device time 0",
        &[
            "tx_send 0 n=20 at=1000000 sob=true eob=true",
            "front Some(5035)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 20 wraps 0 blocks 1 end eob",
            "rejected 1",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=20",
        ],
    ),
    (
        "F3a: repeat 1000, held at +20 (before 32 samples), past J(e)",
        &[
            "tx_send 0 n=20 at=1000000 sob=true eob=true",
            "front Some(5035)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 20 wraps 0 blocks 1 end eob",
            "rejected 1",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=20",
        ],
    ),
    (
        "finite 100, held at +100 (its end), drained at device time 0",
        &[
            "tx_send 0 n=100 at=1000000 sob=true eob=false",
            "tx_send 0 n=49 at=1020000 sob=false eob=false",
            "tx_send 0 n=2 at=1029800 sob=false eob=true",
            "front Some(5154)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 100 wraps 1 blocks 1 end eob",
            "record 5100 samples 50 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=150",
        ],
    ),
    (
        "finite 100, held at +100 (its end), past J(e)",
        &[
            "tx_send 0 n=100 at=1000000 sob=true eob=false",
            "tx_send 0 n=49 at=1020000 sob=false eob=false",
            "tx_send 0 n=2 at=1029800 sob=false eob=true",
            "front Some(5154)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 100 wraps 1 blocks 1 end eob",
            "record 5100 samples 50 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=150",
        ],
    ),
    (
        "repeat 1000, held at +1000 (its wrap), drained at device time 0",
        &[
            "tx_send 0 n=1000 at=1000000 sob=true eob=false",
            "tx_send 0 n=299 at=1200000 sob=false eob=false",
            "tx_send 0 n=2 at=1259800 sob=false eob=true",
            "front Some(6304)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 1000 wraps 1 blocks 1 end eob",
            "record 6000 samples 300 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=1300",
        ],
    ),
    (
        "repeat 1000, held at +1000 (its wrap), past J(e)",
        &[
            "tx_send 0 n=1000 at=1000000 sob=true eob=false",
            "tx_send 0 n=299 at=1200000 sob=false eob=false",
            "tx_send 0 n=2 at=1259800 sob=false eob=true",
            "front Some(6304)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 1000 wraps 1 blocks 1 end eob",
            "record 6000 samples 300 wraps 1 blocks 1 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=2 tx_errors=0 tx_samples=1300",
        ],
    ),
];
const C5: Expected = &[
    (
        "lead + 1 tick before the end, the last send and the close after it",
        &[
            "tx_send 0 n=32 at=1000000 sob=true eob=false",
            "tx_send 0 n=67 at=1006400 sob=false eob=false",
            "tx_send 0 n=2 at=1019800 sob=false eob=true",
            "front Some(5104)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 100 wraps 1 blocks 2 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=100",
        ],
    ),
    (
        "exactly the lead before the end, the last send and the close after it",
        &[
            "tx_send 0 n=32 at=1000000 sob=true eob=false",
            "tx_send 0 n=68 at=1006400 sob=false eob=true",
            "front Some(5103)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 100 wraps 1 blocks 2 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=100",
        ],
    ),
    (
        "lead - 1 tick before the end, the last send and the close after it",
        &[
            "tx_send 0 n=32 at=1000000 sob=true eob=false",
            "tx_send 0 n=68 at=1006400 sob=false eob=true",
            "front Some(5103)",
            "chain next None tail false",
            "open next None first None",
            "record 5000 samples 100 wraps 1 blocks 2 end eob",
            "rejected 0",
            "stats link_drops_seen=0 rx_before_origin=0 rx_blocks=0 rx_errors=0 rx_off_lattice=0 rx_overflows=0 rx_overlapping=0 rx_samples=0 tx_bursts=1 tx_errors=0 tx_samples=100",
        ],
    ),
];
const C6: Expected = &[
    ("window 10000000 ns, 1000000 S/s", &["first sends [2500, 2500] of 2", "open next Some(8000)"]),
    ("window 10000000 ns, 200000000 S/s", &["first sends [20000, 20000, 20000] of 70", "open next Some(2000000)"]),
    ("window 4000000 ns, 1000000 S/s", &["first sends [1000] of 1", "open next Some(4000)"]),
    ("window 2100000 ns, 1000000 S/s", &["first sends [50] of 1", "open next Some(2100)"]),
    ("window 10000000 ns, 25000 S/s", &["first sends [62, 62] of 2", "open next Some(199)"]),
    ("window 10000000 ns, 12500 S/s", &["first sends [32, 32] of 2", "open next Some(102)"]),
    ("window 1000000 ns, device time 0 .. 3 200", &["sends 0 time errors 1 open next None"]),
    ("window 100000 ns, device time 0 .. 3 200", &["sends 0 time errors 1 open next None"]),
];
