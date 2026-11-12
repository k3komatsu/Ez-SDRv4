//! E3b-1 (design note 49 stage E3b; plan `2026-11-12-e3b/test-plan.md`): today's device-burst state (`TxChain`) and the record's front pinned
//! before the state moves into the book. Every test is single-threaded on the frozen rigs of `txp_rig`: device time moves only through
//! `device_at`, so no verdict depends on when a thread runs (note 51 §3). An operation that waits on the device's host clock (a recovery's
//! flush, a reopen, the shutdown's acknowledgement window) runs through `released`, which returns the owner and a new freeze before a
//! snapshot is read; the freeze is never dropped around an unregistered test thread (plan M1).
//!
//! The chain is read and written only through the `dev_*` and `poke_*` functions below (E3b-3 changed their bodies, which now reach the chain in the
//! stream's book through `Tx::chain()`, and `chain_line` for a hook that holds no `Tx`).
//!
//! Units: one sample is 1 us at 1 MS/s (200 root ticks) unless a case says otherwise; `g` is the gap of a closed short burst (15 samples at
//! 1 MS/s, 3 000 at 200 MS/s).

use super::e1::{book, params};
use super::e3::{Cases, Expected};
use super::*;
use crate::provider::test_support::{Frozen, on_next_lock, probe, record_locks, released, taken_locks};
use crate::provider::tx::oracle as o;
use ezsdr_kernel::module_api::StopMode;
use ezsdr_radio::staging::Overlap;

// ------------------------------------------------------------------------------------------------------------ the chain, as a test reads it

/// The device burst's first sample (root tick), its next sample and whether a tail is held, the front and the recovery flag, in one line.
fn dev_line(tx: &Tx) -> String {
    chain_line(&tx.core, tx.k)
}

/// The same line read by a hook, which holds no `Tx`: the chain is in the stream's book, read here with the mutex taken raw so that it counts no
/// section and fires no hook.
fn chain_line(core: &Core, k: usize) -> String {
    let streams = core.streams.lock().unwrap_or_else(|e| e.into_inner());
    let device = &streams.stream(stream_key(Dir::Tx, k)).book.device;
    format!(
        "burst {} first {:?} next {:?} tail {} front {:?} recovery {}",
        device.burst_is_open(),
        device.first(),
        device.next(),
        device.tail().is_some(),
        device.front().map(|(_, sample)| sample),
        device.needs_reopen()
    )
}

fn dev_open(tx: &Tx) -> bool {
    tx.chain().burst_is_open()
}

fn dev_next(tx: &Tx) -> Option<i64> {
    tx.chain().next()
}

fn dev_tail(tx: &Tx) -> bool {
    tx.chain().tail().is_some()
}

fn dev_front(tx: &Tx) -> Option<i64> {
    tx.chain().front().map(|(_, sample)| sample)
}

fn dev_recovering(tx: &Tx) -> bool {
    tx.chain().needs_reopen()
}

fn poke_open_device(tx: &mut Tx, first: i64, next: i64, tail: bool) {
    tx.chain().open_device_burst(first, next, tail.then_some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
}

fn poke_front(tx: &mut Tx, clock: Clock, sample: i64) {
    tx.chain().advance_front(clock.domain, sample);
}

fn poke_reopen_needed(tx: &mut Tx) {
    tx.chain().mark_reopen_needed();
}

// ------------------------------------------------------------------------------------------------------------ the record, as a receipt reads it

/// The record's verdict on a one-sample booking at each sample of `from..to`, asked of a clone of the record (nothing is booked), written as
/// runs: `.` accepted, `E` refused for overlapping a booking, `H` refused for starting before a chain the owner may have handed.
fn receipt_runs(core: &Core, clock: Clock, from: i64, to: i64) -> String {
    let (record, now, horizon) = {
        let streams = lock(&core.streams);
        (streams.stream(stream_key(Dir::Tx, 0)).book.record.clone(), core.now(), ezsdr_radio::staging::handover_horizon(core.now(), core.tx_in_flight(), |t| clock.at_or_after(t)))
    };
    let mut runs: Vec<(i64, i64, char)> = Vec::new();
    for k in from..to {
        let verdict = match record.clone().accept(k, Some(1), now, |e| core.join_by(clock, e), horizon) {
            Ok(()) => '.',
            Err(Overlap::Booking) => 'E',
            Err(Overlap::Handed) => 'H',
        };
        match runs.last_mut() {
            Some((_, last, c)) if *c == verdict => *last = k,
            _ => runs.push((k, k, verdict)),
        }
    }
    runs.iter().map(|(a, b, c)| format!("{c}{a}..={b}")).collect::<Vec<_>>().join(" ")
}

fn accepted(core: &Core, clock: Clock, at: i64, len: i64) -> Result<(), Overlap> {
    let (mut record, now, horizon) = {
        let streams = lock(&core.streams);
        (streams.stream(stream_key(Dir::Tx, 0)).book.record.clone(), core.now(), ezsdr_radio::staging::handover_horizon(core.now(), core.tx_in_flight(), |t| clock.at_or_after(t)))
    };
    record.accept(at, Some(len), now, |e| core.join_by(clock, e), horizon)
}

/// The device's sends, the chain and the record's runs over `from..to`.
fn front_lines(core: &Core, device: &FakeDevice, clock: Clock, tx: &Tx, from: i64, to: i64) -> Vec<String> {
    let sent: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("tx_send ")).collect();
    // A long repeat's identical middle sends are counted, not listed.
    let mut lines: Vec<String> = if sent.len() > 6 { [sent[..2].to_vec(), vec![format!("... {} sends in all", sent.len())], sent[sent.len() - 2..].to_vec()].concat() } else { sent };
    lines.push(format!("device front {:?} next {:?} tail {}", dev_front(tx), dev_next(tx), dev_tail(tx)));
    lines.push(format!("receipt {}", receipt_runs(core, clock, from, to)));
    lines
}

// ------------------------------------------------------------------------------------------------------------ C1

#[test]
fn e3b_c1_the_front_after_each_close() {
    // The owner's front (the actual one) beside the record's answer at a booking (the prediction), for each way a device burst closes, at 1 and at
    // 200 MS/s. The two agree for a short head's padded close and for a held-tail close; where they differ (the direct close of a long
    // burst's last send, an abandon, a flush) the difference is today's behaviour, pinned here as it is (plan §1.4, F1 to F6).
    let mut cases = Cases::new(C1);
    for (name, rate, n, gap) in [("1 MS/s", 1e6f64, 200i64, 15i64), ("200 MS/s", 200e6, 1, 3_000)] {
        let start = (5_000.0 * rate / 1e6).round() as i64;
        for len in [1usize, 3, 4, 32] {
            let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig_rate(None, 2_000, rate, n);
            txp_book(&mut tx, &core, clock, txp_burst(&clock, start, len, false, LatePolicy::DropAndFlag));
            txp_drain(&mut tx);
            cases.check(format!("{name}: short head {len}"), front_lines(&core, &device, clock, &tx, start, start + len.max(4) as i64 + gap + 8));
        }
        let start = (6_000.0 * rate / 1e6).round() as i64;
        for stop in [true, false] {
            let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig_rate(None, 2_000, rate, n);
            txp_book(&mut tx, &core, clock, txp_burst(&clock, start, 33, false, LatePolicy::DropAndFlag));
            txp_drain(&mut tx);
            if stop {
                halt(&mut tx);
            } else {
                device_at(&core, &time, &device, core.join_by(clock, start + 33) + 1);
                txp_drain(&mut tx);
            }
            cases.check(format!("{name}: long head 33, {}", if stop { "a Stop closes it" } else { "its own end closes it" }), front_lines(&core, &device, clock, &tx, start, start + 33 + 12));
        }
        // A joined one-sample piece closes the chain (A 40, B 1 at A's end).
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig_rate(None, 2_000, rate, n);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, start, 40, false, LatePolicy::DropAndFlag));
        txp_book(&mut tx, &core, clock, txp_burst(&clock, start + 40, 1, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, core.join_by(clock, start + 41) + 1);
        txp_drain(&mut tx);
        cases.check(format!("{name}: A 40 and a joined one-sample piece"), front_lines(&core, &device, clock, &tx, start, start + 41 + 12));
        // An open repeat stopped: no tail, the close starts at the cursor.
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig_rate(None, 2_000, rate, n);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, start, 500, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        halt(&mut tx);
        cases.check(format!("{name}: a repeat stopped at its cursor"), front_lines(&core, &device, clock, &tx, start, start + 5_000 + gap));
    }

    // The direct close of a long burst's last send (F1): 40 samples whose last 8 reach the device lead, so no tail is held.
    let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_100_000), 2_000);
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 40, false, LatePolicy::DropAndFlag));
    device_at(&core, &time, &device, txp_ticks(2_932));
    txp_drain(&mut tx);
    device_at(&core, &time, &device, txp_ticks(3_040));
    txp_drain(&mut tx);
    cases.check("1 MS/s: long 40 closed directly (F1)", front_lines(&core, &device, clock, &tx, 5_000, 5_060));

    // An abandon at the cursor (a first send the device takes half of): a repeat (F2) and a finite burst (F3).
    for (name, repeat, len) in [("a repeat", true, 2_000usize), ("a finite burst", false, 200)] {
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig_faults(None, 2_000, 1e6, 200, vec![FakeFault::ShortSend(0)]);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, len, repeat, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        cases.check(format!("1 MS/s: {name} abandoned by a short send"), front_lines(&core, &device, clock, &tx, 5_000, 5_000 + len as i64 + 40));
    }

    // A failed padded EOB and its recovery flush (F6), at 200 MS/s: the clock-waiting reopen runs as a participant.
    let (core, device, time, _events, clock, mut tx, frozen) = txp_high_rate_rig(1, vec![FakeFault::ShortSend(0)], BTreeMap::new());
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 500_000, 1, false, LatePolicy::DropAndFlag));
    assert!(tx.step(), "the one-sample payload reaches its padded EOB send");
    cases.check("200 MS/s: a failed padded EOB", front_lines(&core, &device, clock, &tx, 500_000, 500_004 + 3_000 + 8).into_iter().chain([format!("recovering {}", dev_recovering(&tx))]).collect());
    device_at(&core, &time, &device, 500_003);
    let ((tx, recovered), _frozen) = released(frozen, &device.host(), move || {
        let recovered = tx.pass();
        (tx, recovered)
    });
    assert!(recovered);
    cases.check(
        "200 MS/s: the acknowledged recovery flush",
        front_lines(&core, &device, clock, &tx, 500_000, 500_004 + 3_000 + 8).into_iter().chain([format!("recovering {}", dev_recovering(&tx))]).collect(),
    );
    cases.finish();
}

const C1: Expected = &[
    ("1 MS/s: short head 1", &["tx_send 0 n=4 at=1000000 sob=true eob=true", "device front Some(5019) next None tail false", "receipt E5000..=5018 .5019..=5026"]),
    ("1 MS/s: short head 3", &["tx_send 0 n=4 at=1000000 sob=true eob=true", "device front Some(5019) next None tail false", "receipt E5000..=5018 .5019..=5026"]),
    ("1 MS/s: short head 4", &["tx_send 0 n=4 at=1000000 sob=true eob=true", "device front Some(5019) next None tail false", "receipt E5000..=5018 .5019..=5026"]),
    ("1 MS/s: short head 32", &["tx_send 0 n=32 at=1000000 sob=true eob=true", "device front Some(5047) next None tail false", "receipt E5000..=5046 .5047..=5054"]),
    (
        "1 MS/s: long head 33, a Stop closes it",
        &[
            "tx_send 0 n=32 at=1200000 sob=true eob=false",
            "tx_send 0 n=2 at=1206400 sob=false eob=true",
            "device front Some(6037) next None tail false",
            "receipt E6000..=6032 .6033..=6033 E6034..=6036 .6037..=6044",
        ],
    ),
    (
        "1 MS/s: long head 33, its own end closes it",
        &["tx_send 0 n=32 at=1200000 sob=true eob=false", "tx_send 0 n=2 at=1206400 sob=false eob=true", "device front Some(6037) next None tail false", "receipt E6000..=6036 .6037..=6044"],
    ),
    (
        "1 MS/s: A 40 and a joined one-sample piece",
        &["tx_send 0 n=40 at=1200000 sob=true eob=false", "tx_send 0 n=2 at=1208000 sob=false eob=true", "device front Some(6045) next None tail false", "receipt E6000..=6044 .6045..=6052"],
    ),
    (
        "1 MS/s: a repeat stopped at its cursor",
        &["tx_send 0 n=2500 at=1200000 sob=true eob=false", "tx_send 0 n=2 at=1700000 sob=false eob=true", "device front Some(8505) next None tail false", "receipt E6000..=11014"],
    ),
    ("200 MS/s: short head 1", &["tx_send 0 n=4 at=1000000 sob=true eob=true", "device front Some(1003004) next None tail false", "receipt E1000000..=1003003 .1003004..=1003011"]),
    ("200 MS/s: short head 3", &["tx_send 0 n=4 at=1000000 sob=true eob=true", "device front Some(1003004) next None tail false", "receipt E1000000..=1003003 .1003004..=1003011"]),
    ("200 MS/s: short head 4", &["tx_send 0 n=4 at=1000000 sob=true eob=true", "device front Some(1003004) next None tail false", "receipt E1000000..=1003003 .1003004..=1003011"]),
    ("200 MS/s: short head 32", &["tx_send 0 n=32 at=1000000 sob=true eob=true", "device front Some(1003032) next None tail false", "receipt E1000000..=1003031 .1003032..=1003039"]),
    (
        "200 MS/s: long head 33, a Stop closes it",
        &[
            "tx_send 0 n=32 at=1200000 sob=true eob=false",
            "tx_send 0 n=2 at=1200032 sob=false eob=true",
            "device front Some(1200037) next None tail false",
            "receipt E1200000..=1200032 .1200033..=1200033 E1200034..=1200036 .1200037..=1200044",
        ],
    ),
    (
        "200 MS/s: long head 33, its own end closes it",
        &[
            "tx_send 0 n=32 at=1200000 sob=true eob=false",
            "tx_send 0 n=2 at=1200032 sob=false eob=true",
            "device front Some(1200037) next None tail false",
            "receipt E1200000..=1200036 .1200037..=1200044",
        ],
    ),
    (
        "200 MS/s: A 40 and a joined one-sample piece",
        &[
            "tx_send 0 n=40 at=1200000 sob=true eob=false",
            "tx_send 0 n=2 at=1200040 sob=false eob=true",
            "device front Some(1200045) next None tail false",
            "receipt E1200000..=1200044 .1200045..=1200052",
        ],
    ),
    (
        "200 MS/s: a repeat stopped at its cursor",
        &[
            "tx_send 0 n=20000 at=1200000 sob=true eob=false",
            "tx_send 0 n=20000 at=1220000 sob=false eob=false",
            "... 41 sends in all",
            "tx_send 0 n=20000 at=1980000 sob=false eob=false",
            "tx_send 0 n=2 at=2000000 sob=false eob=true",
            "device front Some(2000005) next None tail false",
            "receipt E1200000..=1207999",
        ],
    ),
    (
        "1 MS/s: long 40 closed directly (F1)",
        &["tx_send 0 n=32 at=1000000 sob=true eob=false", "tx_send 0 n=8 at=1006400 sob=false eob=true", "device front Some(5043) next None tail false", "receipt E5000..=5043 .5044..=5059"],
    ),
    (
        "1 MS/s: a repeat abandoned by a short send",
        &["tx_send 0 n=1250 at=1000000 sob=true eob=false", "tx_send 0 n=2 at=1250000 sob=false eob=true", "device front Some(6267) next None tail false", "receipt E5000..=6254 .6255..=7039"],
    ),
    (
        "1 MS/s: a finite burst abandoned by a short send",
        &[
            "tx_send 0 n=99 at=1000000 sob=true eob=false",
            "tx_send 0 n=2 at=1019800 sob=false eob=true",
            "device front Some(5116) next None tail false",
            "receipt E5000..=5199 .5200..=5200 E5201..=5203 .5204..=5239",
        ],
    ),
    ("200 MS/s: a failed padded EOB", &["tx_send 0 n=2 at=500000 sob=true eob=false", "device front None next None tail false", "receipt E500000..=503003 .503004..=503011", "recovering true"]),
    (
        "200 MS/s: the acknowledged recovery flush",
        &[
            "tx_send 0 n=2 at=500000 sob=true eob=false",
            "tx_send 0 n=4 at=- sob=true eob=true",
            "device front Some(503007) next None tail false",
            "receipt E500000..=503003 .503004..=503011",
            "recovering false",
        ],
    ),
];

// ------------------------------------------------------------------------------------------------------------ C2

/// Where an untimed 10-sample booking asking for `at` is placed by the record (RM-15): past a chain it may have handed, else at `at`.
fn placed(core: &Core, clock: Clock, at: i64) -> Option<i64> {
    let (record, now, horizon) = {
        let streams = lock(&core.streams);
        (streams.stream(stream_key(Dir::Tx, 0)).book.record.clone(), core.now(), ezsdr_radio::staging::handover_horizon(core.now(), core.tx_in_flight(), |t| clock.at_or_after(t)))
    };
    record.place_untimed(at, at, Some(10), now, |e| core.join_by(clock, e), horizon)
}

#[test]
fn e3b_c2_may_be_handed_is_the_window_and_not_the_owner() {
    // RM-15, KL-37: a booking that starts before an accepted chain is refused when the chain starts below the hand-over horizon (10 000 at device
    // time 0), whatever the owner has done with it. The owner hands a first send of a chain only when 32 of its samples (all of a shorter one)
    // fit the window, so for a chain starting in the last 31 samples below the horizon the record's answer is `Handed` while the owner has sent
    // nothing (F5). The receipt and the untimed placement are read three times: before the owner reads the chain, once it holds it, and once
    // it has stepped; the three must agree.
    let mut cases = Cases::new(C2);
    for d in [-5_000i64, -40, -33, -32, -31, -1, 0] {
        let s = 10_000 + d;
        let mut lines = Vec::new();
        let mut answers = Vec::new();
        for state in ["not read", "held", "stepped"] {
            let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
            let c = txp_burst(&clock, s, 100, false, LatePolicy::DropAndFlag);
            txp_record(&core, clock, &c);
            if state != "not read" {
                txp_give(&mut tx, c);
            }
            if state == "stepped" {
                txp_drain(&mut tx);
            }
            let answer = (accepted(&core, clock, s - 1_000, 10), placed(&core, clock, s - 1_000));
            lines.push(format!("{state}: B at {} {:?}, untimed placed {:?}, owner sends {:?}", s - 1_000, answer.0, answer.1, sends(&device)));
            answers.push(answer);
        }
        assert!(answers.windows(2).all(|pair| pair[0] == pair[1]), "d {d}: the receipt reads the record alone, not what the owner did: {answers:?}");
        cases.check(format!("C at the horizon {d:+}"), lines);
    }

    // A chain behind an open join is not yet handed (the owner holds the device burst at the join's end until `J(e)`): A (40 at 5 000) ends at
    // 5 040, and C (100 at 9 900) starts below the horizon. A booking before C is accepted while A can still be joined and refused after.
    let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 40, false, LatePolicy::DropAndFlag));
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 9_900, 100, false, LatePolicy::DropAndFlag));
    let mut lines = vec![format!("device time 0: B at 8 900 {:?}, placed {:?}", accepted(&core, clock, 8_900, 10), placed(&core, clock, 8_900))];
    txp_drain(&mut tx);
    lines.push(format!("owner stepped, sends {:?}", sends(&device)));
    device_at(&core, &time, &device, core.join_by(clock, 5_040) + 1);
    lines.push(format!("past J(5 040): B at 8 900 {:?}, placed {:?}", accepted(&core, clock, 8_900, 10), placed(&core, clock, 8_900)));
    cases.check("C behind an open join", lines);
    cases.finish();
}

const C2: Expected = &[
    (
        "C at the horizon -5000",
        &[
            "not read: B at 4000 Err(Handed), untimed placed Some(5104), owner sends []",
            "held: B at 4000 Err(Handed), untimed placed Some(5104), owner sends []",
            "stepped: B at 4000 Err(Handed), untimed placed Some(5104), owner sends [(99, true, false)]",
        ],
    ),
    (
        "C at the horizon -40",
        &[
            "not read: B at 8960 Err(Handed), untimed placed Some(10064), owner sends []",
            "held: B at 8960 Err(Handed), untimed placed Some(10064), owner sends []",
            "stepped: B at 8960 Err(Handed), untimed placed Some(10064), owner sends [(40, true, false)]",
        ],
    ),
    (
        "C at the horizon -33",
        &[
            "not read: B at 8967 Err(Handed), untimed placed Some(10071), owner sends []",
            "held: B at 8967 Err(Handed), untimed placed Some(10071), owner sends []",
            "stepped: B at 8967 Err(Handed), untimed placed Some(10071), owner sends [(33, true, false)]",
        ],
    ),
    (
        "C at the horizon -32",
        &[
            "not read: B at 8968 Err(Handed), untimed placed Some(10072), owner sends []",
            "held: B at 8968 Err(Handed), untimed placed Some(10072), owner sends []",
            "stepped: B at 8968 Err(Handed), untimed placed Some(10072), owner sends [(32, true, false)]",
        ],
    ),
    (
        "C at the horizon -31",
        &[
            "not read: B at 8969 Err(Handed), untimed placed Some(10073), owner sends []",
            "held: B at 8969 Err(Handed), untimed placed Some(10073), owner sends []",
            "stepped: B at 8969 Err(Handed), untimed placed Some(10073), owner sends []",
        ],
    ),
    (
        "C at the horizon -1",
        &[
            "not read: B at 8999 Err(Handed), untimed placed Some(10103), owner sends []",
            "held: B at 8999 Err(Handed), untimed placed Some(10103), owner sends []",
            "stepped: B at 8999 Err(Handed), untimed placed Some(10103), owner sends []",
        ],
    ),
    (
        "C at the horizon +0",
        &[
            "not read: B at 9000 Ok(()), untimed placed Some(9000), owner sends []",
            "held: B at 9000 Ok(()), untimed placed Some(9000), owner sends []",
            "stepped: B at 9000 Ok(()), untimed placed Some(9000), owner sends []",
        ],
    ),
    ("C behind an open join", &["device time 0: B at 8 900 Ok(()), placed Some(8900)", "owner stepped, sends [(39, true, false)]", "past J(5 040): B at 8 900 Err(Handed), placed Some(10004)"]),
];

// ------------------------------------------------------------------------------------------------------------ C3

/// `Tx::pass` run as a participant of the device's host clock (the flush and the reopen wait on it), the owner and a new freeze given back.
fn pass_released(tx: Tx, frozen: Frozen, device: &FakeDevice) -> (Tx, bool, Frozen) {
    let ((tx, worked), frozen) = released(frozen, &device.host(), move || {
        let mut tx = tx;
        let worked = tx.pass();
        (tx, worked)
    });
    (tx, worked, frozen)
}

/// What the owner has done to the chain, one line: the chain, the open booking's next sample and the device's sends.
fn state_line(tx: &Tx, device: &FakeDevice) -> String {
    format!("{}; open next {:?}; sends {:?}", dev_line(tx), tx.open_next(), sends(device))
}

fn stop_streams(core: &Core) {
    lock(&core.streams).stop = Some((core.now(), StopMode::Orderly));
}

#[test]
fn e3b_c3_the_chain_through_its_lifecycles() {
    // Today's chain transitions as the owner makes them, one ordered line after each operation: an open device burst under an underflow report
    // (E2a-4: a tick before the burst's first sample is the burst before's), the failed close and every way its recovery ends (UR-22, UR-33),
    // `Provider::stop` and a loss. The sends that wait on the device's host clock run through `released`.
    let mut cases = Cases::new(C3);
    let first = 200 * 5_000;
    for (name, code, tick) in [
        ("underflow before the first sample", TxCode::Underflow, Some(first - 1)),
        ("underflow at the first sample", TxCode::Underflow, Some(first)),
        ("underflow without a tick", TxCode::Underflow, None),
        ("sequence error before the first sample", TxCode::SeqError, Some(first - 1)),
    ] {
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 2_000, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        let mut lines = vec![format!("sent: {}", state_line(&tx, &device))];
        tx.report(TxReport { code, tick, channel: 0 });
        lines.push(format!("reported: {}", state_line(&tx, &device)));
        cases.check(name, lines);
    }

    // The failed padded EOB (a one-sample burst at 200 MS/s whose two-sample send the device cuts) and how its recovery ends.
    for (name, faults) in [
        ("failed EOB, acknowledged flush", vec![FakeFault::ShortSend(0)]),
        ("failed EOB, flush not acknowledged", vec![FakeFault::ShortSend(0), FakeFault::DropBurstAck]),
        ("failed EOB, reopen refused", vec![FakeFault::ShortSend(0), FakeFault::TxOpenFails(1)]),
    ] {
        let (core, device, time, _events, clock, mut tx, frozen) = txp_high_rate_rig(1, faults, BTreeMap::new());
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 500_000, 1, false, LatePolicy::DropAndFlag));
        assert!(tx.step());
        let mut lines = vec![format!("failed: {}", state_line(&tx, &device))];
        device_at(&core, &time, &device, 500_003);
        let (tx, worked, _frozen) = pass_released(tx, frozen, &device);
        lines.push(format!("recovered ({worked}): {}; clock {}", state_line(&tx, &device), tx.clock.is_some()));
        cases.check(name, lines);
    }

    // A recovery pending behind an open device burst with a held tail: the burst is kept by a reopen that succeeds and by `Provider::stop`,
    // and ended by a reopen that fails and by a loss.
    for (name, how) in [("reopen succeeds", 0), ("Stop before the reopen", 1), ("loss before the reopen", 2), ("reopen refused, Stop in flight", 3), ("reopen refused, loss in flight", 4)] {
        let faults = if how >= 3 { vec![FakeFault::TxOpenFails(1)] } else { Vec::new() };
        let (core, device, time, _events, clock, mut tx, frozen) = txp_rig_faults(None, 2_000, 1e6, 200, faults);
        device_at(&core, &time, &device, 10);
        poke_open_device(&mut tx, 1_000_000, 5_100, true);
        poke_front(&mut tx, clock, 5_000);
        poke_reopen_needed(&mut tx);
        let mut lines = vec![format!("pending: {}", state_line(&tx, &device))];
        match how {
            1 => stop_streams(&core),
            2 => core.device_lost("test loss before the reopen"),
            3 | 4 => {
                let owner = core.clone();
                probe(&core, "tx_reopen_done", move |_| if how == 3 { stop_streams(&owner) } else { owner.device_lost("test loss in flight") });
            }
            _ => {}
        }
        let (tx, worked, _frozen) = pass_released(tx, frozen, &device);
        lines.push(format!("passed ({worked}): {}", state_line(&tx, &device)));
        cases.check(name, lines);
    }

    // `Provider::stop` (the shutdown): the open burst is closed and the recovery cleared; and a loss found by a step.
    let (core, device, time, _events, clock, mut tx, frozen) = txp_rig(None, 2_000);
    device_at(&core, &time, &device, 10);
    poke_open_device(&mut tx, 1_000_000, 5_100, true);
    poke_front(&mut tx, clock, 5_000);
    poke_reopen_needed(&mut tx);
    let mut lines = vec![format!("running: {}", state_line(&tx, &device))];
    let ((tx, ()), _frozen) = released(frozen, &device.host(), move || {
        let mut tx = tx;
        tx.shutdown();
        (tx, ())
    });
    lines.push(format!("shut down: {}", state_line(&tx, &device)));
    cases.check("Provider::stop", lines);

    let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    poke_open_device(&mut tx, 1_000_000, 5_100, true);
    poke_front(&mut tx, clock, 5_000);
    let mut lines = vec![format!("running: {}", state_line(&tx, &device))];
    core.device_lost("test loss");
    assert!(!tx.step());
    lines.push(format!("lost: {}", state_line(&tx, &device)));
    cases.check("a loss found by a step", lines);
    cases.finish();
}

const C3: Expected = &[
    (
        "underflow before the first sample",
        &[
            "sent: burst true first Some(1000000) next Some(7000) tail true front None recovery false; open next None; sends [(1999, true, false)]",
            "reported: burst true first Some(1000000) next Some(7000) tail true front None recovery false; open next None; sends [(1999, true, false)]",
        ],
    ),
    (
        "underflow at the first sample",
        &[
            "sent: burst true first Some(1000000) next Some(7000) tail true front None recovery false; open next None; sends [(1999, true, false)]",
            "reported: burst false first None next None tail false front Some(7017) recovery false; open next None; sends [(1999, true, false), (2, false, true)]",
        ],
    ),
    (
        "underflow without a tick",
        &[
            "sent: burst true first Some(1000000) next Some(7000) tail true front None recovery false; open next None; sends [(1999, true, false)]",
            "reported: burst false first None next None tail false front Some(7017) recovery false; open next None; sends [(1999, true, false), (2, false, true)]",
        ],
    ),
    (
        "sequence error before the first sample",
        &[
            "sent: burst true first Some(1000000) next Some(7000) tail true front None recovery false; open next None; sends [(1999, true, false)]",
            "reported: burst false first None next None tail false front Some(7017) recovery false; open next None; sends [(1999, true, false), (2, false, true)]",
        ],
    ),
    (
        "failed EOB, acknowledged flush",
        &[
            "failed: burst false first None next None tail false front None recovery true; open next None; sends [(2, true, false)]",
            "recovered (true): burst false first None next None tail false front Some(503007) recovery false; open next None; sends [(2, true, false), (4, true, true)]; clock true",
        ],
    ),
    (
        "failed EOB, flush not acknowledged",
        &[
            "failed: burst false first None next None tail false front None recovery true; open next None; sends [(2, true, false)]",
            "recovered (true): burst false first None next None tail false front None recovery false; open next None; sends [(2, true, false), (4, true, true)]; clock true",
        ],
    ),
    (
        "failed EOB, reopen refused",
        &[
            "failed: burst false first None next None tail false front None recovery true; open next None; sends [(2, true, false)]",
            "recovered (true): burst false first None next None tail false front None recovery false; open next None; sends [(2, true, false)]; clock false",
        ],
    ),
    (
        "reopen succeeds",
        &[
            "pending: burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery true; open next None; sends []",
            "passed (true): burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery false; open next None; sends [(4, true, true)]",
        ],
    ),
    (
        "Stop before the reopen",
        &[
            "pending: burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery true; open next None; sends []",
            "passed (false): burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery false; open next None; sends []",
        ],
    ),
    (
        "loss before the reopen",
        &[
            "pending: burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery true; open next None; sends []",
            "passed (false): burst false first None next None tail false front Some(5000) recovery false; open next None; sends []",
        ],
    ),
    (
        "reopen refused, Stop in flight",
        &[
            "pending: burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery true; open next None; sends []",
            "passed (true): burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery false; open next None; sends []",
        ],
    ),
    (
        "reopen refused, loss in flight",
        &[
            "pending: burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery true; open next None; sends []",
            "passed (true): burst false first None next None tail false front Some(5000) recovery false; open next None; sends []",
        ],
    ),
    (
        "Provider::stop",
        &[
            "running: burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery true; open next None; sends []",
            "shut down: burst false first None next None tail false front Some(5104) recovery false; open next None; sends [(2, false, true)]",
        ],
    ),
    (
        "a loss found by a step",
        &[
            "running: burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery false; open next None; sends []",
            "lost: burst false first None next None tail false front Some(5000) recovery false; open next None; sends []",
        ],
    ),
];

// ------------------------------------------------------------------------------------------------------------ C4

/// How many times `f` took the streams lock on this thread.
fn streams_sections(core: &Core, f: impl FnOnce()) -> usize {
    let address = std::ptr::from_ref(&core.streams) as usize;
    record_locks();
    f();
    taken_locks().into_iter().filter(|taken| *taken == address).count()
}

/// `step` called until it reports nothing to do, with the streams sections each call took: `(did work, sections)`.
fn step_sections(core: &Core, tx: &mut Tx) -> Vec<(bool, usize)> {
    let mut out = Vec::new();
    for _ in 0..100 {
        let mut worked = false;
        let sections = streams_sections(core, || worked = tx.step());
        out.push((worked, sections));
        if !worked {
            return out;
        }
    }
    panic!("the owner still reports work after 100 steps");
}

#[test]
fn e3b_c4_the_streams_lock_sections() {
    // The number of critical sections of the streams lock each owner operation takes (UR-25, note 49 §12). E3b-3 may add a section only as a named
    // change to this table; E3b-4 folds the chain's effect into the book's own sections and brings the counts back to these.
    let mut cases = Cases::new(C4);

    let (core, _device, _time, _events, _clock, mut tx, _frozen) = txp_rig(None, 2_000);
    cases.check("an idle step", vec![format!("{:?}", step_sections(&core, &mut tx))]);

    let (core, _device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 100, false, LatePolicy::DropAndFlag));
    cases.check("a first send of a finite burst, held tail", vec![format!("{:?}", step_sections(&core, &mut tx))]);

    let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 100, false, LatePolicy::DropAndFlag));
    let sent = step_sections(&core, &mut tx);
    device_at(&core, &time, &device, core.join_by(clock, 5_100) + 1);
    cases.check("close_unjoined: the owner's close at the join deadline", vec![format!("{sent:?}"), format!("{:?}", step_sections(&core, &mut tx))]);

    let (core, _device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, false, LatePolicy::DropAndFlag));
    cases.check("a first send and its continuations", vec![format!("{:?}", step_sections(&core, &mut tx))]);

    let (core, _device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 40, false, LatePolicy::DropAndFlag));
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_040, 10, false, LatePolicy::DropAndFlag));
    cases.check("a joined piece", vec![format!("{:?}", step_sections(&core, &mut tx))]);

    let (core, _device, _time, _events, clock, mut tx, _frozen) = txp_rig_faults(None, 2_000, 1e6, 200, vec![FakeFault::ShortSend(0)]);
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 200, false, LatePolicy::DropAndFlag));
    cases.check("abandon: a short send", vec![format!("{:?}", step_sections(&core, &mut tx))]);

    for (name, repeat, len) in [("end_open: a Stop with a tail", false, 33usize), ("end_open: a Stop of a repeat", true, 500)] {
        let (core, _device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, len, repeat, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        lock(&core.streams).stream_mut(stream_key(Dir::Tx, 0)).book.list.halt(ezsdr_radio::CancelReason::Stop);
        cases.check(name, vec![format!("{}", streams_sections(&core, || tx.read_list()))]);
    }

    let (core, device, time, _events, clock, mut tx, frozen) = txp_high_rate_rig(1, vec![FakeFault::ShortSend(0)], BTreeMap::new());
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 500_000, 1, false, LatePolicy::DropAndFlag));
    cases.check("a failed padded EOB", vec![format!("{:?}", step_sections(&core, &mut tx))]);
    device_at(&core, &time, &device, 500_003);
    let ((tx, counted), _frozen) = released(frozen, &device.host(), move || {
        let mut tx = tx;
        let mut worked = false;
        let sections = streams_sections(&tx.core.clone(), || worked = tx.pass());
        (tx, (worked, sections))
    });
    cases.check("reopen_streamer: a recovery", vec![format!("{counted:?}")]);
    drop(tx);
    cases.finish();
}

/// The chain's reads and writes are inside the sections the operation already had (E3b-4), so the counts are those of E3b-1, before the chain
/// moved into the book (E3b-3 had made each its own section).
const C4: Expected = &[
    ("an idle step", &["[(false, 3)]"]),
    ("a first send of a finite burst, held tail", &["[(true, 10), (false, 4)]"]),
    ("close_unjoined: the owner's close at the join deadline", &["[(true, 10), (false, 4)]", "[(true, 6), (false, 3)]"]),
    ("a first send and its continuations", &["[(true, 10), (true, 7), (false, 6)]"]),
    ("a joined piece", &["[(true, 10), (true, 12), (false, 4)]"]),
    ("abandon: a short send", &["[(true, 13), (false, 3)]"]),
    ("end_open: a Stop with a tail", &["4"]),
    ("end_open: a Stop of a repeat", &["4"]),
    ("a failed padded EOB", &["[(true, 13), (false, 3)]"]),
    ("reopen_streamer: a recovery", &["(true, 7)"]),
];

// ------------------------------------------------------------------------------------------------------------ C3b: the seams between sections

/// Runs `f` on this thread immediately before its `n`th next `core::lock` of the mutex `field` selects (the first when `n` is 1), with no guard
/// of that call held. A hook that runs while the owner holds the streams guard must not lock it.
fn at_nth_lock<T: 'static>(core: &Arc<Core>, field: fn(&Core) -> &std::sync::Mutex<T>, n: usize, f: Box<dyn FnOnce()>) {
    if n <= 1 {
        on_next_lock(field(core), f);
    } else {
        let again = core.clone();
        on_next_lock(field(core), move || at_nth_lock(&again, field, n - 1, f));
    }
}

/// Runs each of `hooks` before the next streams lock of this thread, one per lock in order, with no guard held.
fn on_each_streams_lock(core: &Arc<Core>, mut hooks: Vec<Box<dyn FnOnce()>>) {
    if hooks.is_empty() {
        return;
    }
    let first = hooks.remove(0);
    let again = core.clone();
    on_next_lock(&core.streams, move || {
        first();
        on_each_streams_lock(&again, hooks);
    });
}

/// What a receipt and a reader of the book see at a seam: the open booking's next sample and first flag, the device's sends so far and the
/// record's runs. The chain, which E3b-3 put in the book, is read beside it by `chain_line` and asserted in `C3B_CHAIN` (the permitted
/// differences at E3b-4 are in the plan's amendment m1).
fn seam_line(core: &Core, device: &FakeDevice, clock: Clock, from: i64, to: i64) -> String {
    let open = lock(&core.streams).stream(stream_key(Dir::Tx, 0)).book.open.as_ref().map(|open| (open.next, open.first));
    format!("open {open:?}; sends {}; receipt {}", sends(device).len(), receipt_runs(core, clock, from, to))
}

type Seams = std::rc::Rc<std::cell::RefCell<Vec<String>>>;

#[test]
fn e3b_c3b_the_seams_between_the_sections() {
    // Where an operation's effects sit relative to its streams sections (plan m1): at three places a receipt could land between two of an
    // operation's writes, the hook records what the book, the device and the record show, and each line is asserted to equal the line after the
    // operation: the receipt reads the prediction, not the chain, so a write moved from one section to the next changes no answer.
    let mut cases = Cases::new(C3B);
    let mut chain_cases = Cases::new(C3B_CHAIN);

    // (1) A send's booking completion and the chain's effect: the owner's third `rec` lock after the plan is the `tx_bursts` count, after
    // `complete_send` and before the chain is written.
    let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 100, false, LatePolicy::DropAndFlag));
    let seams: Seams = Default::default();
    let chains: Seams = Default::default();
    let (hook_core, hook_device, hook_seams, hook_chains) = (core.clone(), device.clone(), seams.clone(), chains.clone());
    at_nth_lock(
        &core,
        |core| &core.rec,
        3,
        Box::new(move || {
            hook_seams.borrow_mut().push(seam_line(&hook_core, &hook_device, clock, 5_000, 5_130));
            hook_chains.borrow_mut().push(chain_line(&hook_core, 0));
        }),
    );
    assert!(tx.step());
    let after = seam_line(&core, &device, clock, 5_000, 5_130);
    let seen = seams.borrow().clone();
    assert_eq!(seen.len(), 1, "the seam after the booking's completion fired once");
    chain_cases.check("(1) the chain at the seam", vec![chains.borrow()[0].clone()]);
    cases.check("(1) booking completed, chain not yet written", vec![seen[0].clone(), format!("after: {after}"), format!("chain: {}", dev_line(&tx))]);
    assert_eq!(seen[0].split("; receipt ").nth(1), after.split("; receipt ").nth(1), "the receipt reads the same record at the seam and after the send");

    // (1b) The same seam of a continuation: the open booking's next sample has advanced and the chain's has not.
    let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, false, LatePolicy::DropAndFlag));
    assert!(tx.step());
    let before = format!("first send: {}; chain {}", seam_line(&core, &device, clock, 3_000, 3_050), dev_line(&tx));
    let seams: Seams = Default::default();
    let chains: Seams = Default::default();
    let (hook_core, hook_device, hook_seams, hook_chains) = (core.clone(), device.clone(), seams.clone(), chains.clone());
    at_nth_lock(
        &core,
        |core| &core.rec,
        3,
        Box::new(move || {
            hook_seams.borrow_mut().push(seam_line(&hook_core, &hook_device, clock, 3_000, 3_050));
            hook_chains.borrow_mut().push(chain_line(&hook_core, 0));
        }),
    );
    assert!(tx.step());
    let after = seam_line(&core, &device, clock, 3_000, 3_050);
    let seen = seams.borrow().clone();
    assert_eq!(seen.len(), 1, "the seam after the continuation's completion fired once");
    chain_cases.check("(1b) the chain at the seam of a continuation", vec![chains.borrow()[0].clone()]);
    cases.check("(1b) a continuation completed, chain not yet written", vec![before, format!("seam: {}", seen[0]), format!("after: {after}"), format!("chain: {}", dev_line(&tx))]);

    // (2) `end_open`: the first streams section reads the open booking and the chain before the device is called; the second ends the booking
    // and applies the close's result to the chain, so a hook before it sees the chain as the close found it.
    for (name, repeat, len) in [("(2) end_open of a repeat", true, 500usize), ("(2) end_open with a held tail", false, 33)] {
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, len, repeat, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        let before = format!("before: {}; chain {}", seam_line(&core, &device, clock, 5_000, 5_050), dev_line(&tx));
        let seams: Seams = Default::default();
        let chains: Seams = Default::default();
        let hooks: Vec<Box<dyn FnOnce()>> = (0..2)
            .map(|_| {
                let (hook_core, hook_device, hook_seams, hook_chains) = (core.clone(), device.clone(), seams.clone(), chains.clone());
                Box::new(move || {
                    hook_seams.borrow_mut().push(seam_line(&hook_core, &hook_device, clock, 5_000, 5_050));
                    hook_chains.borrow_mut().push(chain_line(&hook_core, 0));
                }) as Box<dyn FnOnce()>
            })
            .collect();
        on_each_streams_lock(&core, hooks);
        tx.end_open(true);
        let after = seam_line(&core, &device, clock, 5_000, 5_050);
        let seen = seams.borrow().clone();
        assert_eq!(seen.len(), 2, "both seams of end_open fired");
        chain_cases.check(format!("{name}: the chain at its two seams"), chains.borrow().clone());
        cases.check(
            name,
            vec![
                before,
                format!("section 1 (before the chain is closed): {}", seen[0]),
                format!("section 2 (after the close): {}", seen[1]),
                format!("after: {after}"),
                format!("chain: {}", dev_line(&tx)),
            ],
        );
        let runs = |line: &str| line.split("; receipt ").nth(1).map(str::to_owned);
        assert!(runs(&seen[0]) == runs(&seen[1]) && runs(&seen[1]) == runs(&after), "{name}: the receipt reads the same record before, between and after the close");
    }

    // (3) The reopen: between the flush's front and the reopen's result section the owner holds the claim; the book shows the open booking
    // unchanged and the record is unmoved.
    let (core, device, time, _events, clock, mut tx, frozen) = txp_high_rate_rig(1, vec![FakeFault::ShortSend(0)], BTreeMap::new());
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 500_000, 1, false, LatePolicy::DropAndFlag));
    assert!(tx.step());
    device_at(&core, &time, &device, 500_003);
    let before = seam_line(&core, &device, clock, 500_000, 503_020);
    // The probe runs on the owner's thread right after the flush returns; its hook fires before the next streams section, the result's.
    let sent_core = core.clone();
    let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let seen_chains: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let (hook_seen, hook_chains) = (seen.clone(), seen_chains.clone());
    let probe_device = device.clone();
    probe(&core, "tx_reopen_done", move |_| {
        let (hook_core, hook_device, hook_seen, hook_chains) = (sent_core.clone(), probe_device.clone(), hook_seen.clone(), hook_chains.clone());
        on_next_lock(&sent_core.streams, move || {
            hook_seen.lock().unwrap().push(seam_line(&hook_core, &hook_device, clock, 500_000, 503_020));
            hook_chains.lock().unwrap().push(chain_line(&hook_core, 0));
        });
    });
    let (tx, worked, _frozen) = pass_released(tx, frozen, &device);
    assert!(worked);
    let after = seam_line(&core, &device, clock, 500_000, 503_020);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "the seam before the reopen's result section fired once");
    chain_cases.check("(3) the chain at the seam", seen_chains.lock().unwrap().clone());
    cases.check("(3) flush returned, reopen not yet completed", vec![format!("before: {before}"), format!("seam: {}", seen[0]), format!("after: {after}"), format!("chain: {}", dev_line(&tx))]);
    let runs = |line: &str| line.split("; receipt ").nth(1).map(str::to_owned);
    assert!(runs(&before) == runs(&seen[0]) && runs(&seen[0]) == runs(&after), "the receipt reads the same record before, between and after the reopen");
    cases.finish();
    chain_cases.finish();
}

/// The chain at each seam (plan m1), as E3b-4 left it: a hook fires before its section, so it sees the chain as the sections before it wrote it.
/// (1) A whole send's effect is applied by `complete`, the section before the seam: the chain is already open at the send's end (E3b-3: still
/// closed). (2) A close's result, and the close of the chain, are applied by the section that ends the booking, so both hooks see the open chain
/// (E3b-3: the second saw it closed with its front). (3) The flush's front is applied by the reopen's result section, which the hook precedes
/// (E3b-3: it saw the front).
const C3B_CHAIN: Expected = &[
    ("(1) the chain at the seam", &["burst true first Some(1000000) next Some(5100) tail true front None recovery false"]),
    ("(1b) the chain at the seam of a continuation", &["burst true first Some(600000) next Some(8000) tail false front None recovery false"]),
    (
        "(2) end_open of a repeat: the chain at its two seams",
        &["burst true first Some(1000000) next Some(10000) tail false front None recovery false", "burst true first Some(1000000) next Some(10000) tail false front None recovery false"],
    ),
    (
        "(2) end_open with a held tail: the chain at its two seams",
        &["burst true first Some(1000000) next Some(5033) tail true front None recovery false", "burst true first Some(1000000) next Some(5033) tail true front None recovery false"],
    ),
    ("(3) the chain at the seam", &["burst false first None next None tail false front None recovery true"]),
];

const C3B: Expected = &[
    (
        "(1) booking completed, chain not yet written",
        &[
            "open None; sends 1; receipt E5000..=5099 .5100..=5100 E5101..=5103 .5104..=5129",
            "after: open None; sends 1; receipt E5000..=5099 .5100..=5100 E5101..=5103 .5104..=5129",
            "chain: burst true first Some(1000000) next Some(5100) tail true front None recovery false",
        ],
    ),
    (
        "(1b) a continuation completed, chain not yet written",
        &[
            "first send: open Some((5500, false)); sends 1; receipt .3000..=3049; chain burst true first Some(600000) next Some(5500) tail false front None recovery false",
            "seam: open Some((8000, false)); sends 2; receipt .3000..=3049",
            "after: open Some((8000, false)); sends 2; receipt .3000..=3049",
            "chain: burst true first Some(600000) next Some(8000) tail false front None recovery false",
        ],
    ),
    (
        "(2) end_open of a repeat",
        &[
            "before: open Some((10000, false)); sends 2; receipt E5000..=5049; chain burst true first Some(1000000) next Some(10000) tail false front None recovery false",
            "section 1 (before the chain is closed): open Some((10000, false)); sends 2; receipt E5000..=5049",
            "section 2 (after the close): open Some((10000, false)); sends 3; receipt E5000..=5049",
            "after: open None; sends 3; receipt E5000..=5049",
            "chain: burst false first None next None tail false front Some(10005) recovery false",
        ],
    ),
    (
        "(2) end_open with a held tail",
        &[
            "before: open None; sends 1; receipt E5000..=5032 .5033..=5033 E5034..=5036 .5037..=5049; chain burst true first Some(1000000) next Some(5033) tail true front None recovery false",
            "section 1 (before the chain is closed): open None; sends 1; receipt E5000..=5032 .5033..=5033 E5034..=5036 .5037..=5049",
            "section 2 (after the close): open None; sends 2; receipt E5000..=5032 .5033..=5033 E5034..=5036 .5037..=5049",
            "after: open None; sends 2; receipt E5000..=5032 .5033..=5033 E5034..=5036 .5037..=5049",
            "chain: burst false first None next None tail false front Some(5037) recovery false",
        ],
    ),
    (
        "(3) flush returned, reopen not yet completed",
        &[
            "before: open None; sends 1; receipt E500000..=503003 .503004..=503019",
            "seam: open None; sends 2; receipt E500000..=503003 .503004..=503019",
            "after: open None; sends 2; receipt E500000..=503003 .503004..=503019",
            "chain: burst false first None next None tail false front Some(503007) recovery false",
        ],
    ),
];

// ------------------------------------------------------------------------------------------------------------ C5

#[test]
fn e3b_c5_receipts_by_provider_uhd_column() {
    // Table C5 (`o::FRONT_RECEIPTS`, the literal MockRadio's `e3b_c5_…` reads too): a booking received after the row's bookings (and Stop) is
    // accepted or refused as the table says, as the independent oracle says and as uhd-control's receipt says. Each probe runs on a fresh rig,
    // at device time 0, so no verdict depends on a thread.
    let p = params();
    for row in o::FRONT_RECEIPTS {
        for &(offset, len, want) in row.probes {
            let mut waves: Vec<Vec<Iq>> = row.bookings.iter().enumerate().map(|(i, &(_, len))| o::ramp(i as u8 + 1, len.unwrap_or(100) as usize)).collect();
            waves.push(o::ramp(60, len as usize));
            let mut rig = switch_rig(waves);
            for (i, &(at, booked)) in row.bookings.iter().enumerate() {
                let k = row.start(0, at);
                match booked {
                    Some(_) => book(&mut rig, i as u64 + 1, i, k),
                    None => book_repeat(&mut rig, i as u64 + 1, k, i),
                }
            }
            assert!(refusals(&rig.events).is_empty(), "{}: the row's own bookings are accepted: {:?}", row.label, lock(&rig.core.rec).rejected);
            if row.stop {
                stop(&mut rig);
                refusals(&rig.events);
            }
            let probe = row.bookings.len();
            book(&mut rig, 100, probe, row.start(0, offset));
            let reasons = refusals(&rig.events);
            let got = if reasons.is_empty() { o::Verdict::Accept } else { o::Verdict::Refuse };
            assert!(reasons.iter().all(|reason| reason.starts_with("UR-21: the burst overlaps an accepted booking")), "{}: a refusal is an overlap: {reasons:?}", row.label);
            assert_eq!(row.verdict(&p, 0, 0, (offset, len)), want, "{}: probe {offset}: the oracle reads the literal", row.label);
            assert_eq!(got, want, "{}: probe {offset} of {len}: uhd-control's receipt", row.label);
        }
    }
}

// ------------------------------------------------------------------------------------------------------------ C6

#[test]
fn e3b_c6_f1_and_f4_as_they_are() {
    // F1 (plan §1.4): a long burst whose last send closes the device burst directly leaves the device front at its padded end plus three, and
    // the record's front at its end plus four: a booking at 5 043 is refused at receipt although the device would take it (UR-22's measured
    // minimum is three samples). Pinned as it is: the receipt answers from the record alone (K1 = S; issue #99 covers the same family).
    let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_100_000), 2_000);
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 40, false, LatePolicy::DropAndFlag));
    device_at(&core, &time, &device, txp_ticks(2_932));
    txp_drain(&mut tx);
    device_at(&core, &time, &device, txp_ticks(3_040));
    txp_drain(&mut tx);
    assert_eq!(sends(&device), [(32, true, false), (8, false, true)], "{:?}", device.calls());
    assert_eq!(dev_front(&tx), Some(5_043), "the device front: the padded end plus three");
    assert_eq!(accepted(&core, clock, 5_042, 1), Err(Overlap::Booking));
    assert_eq!(accepted(&core, clock, 5_043, 1), Err(Overlap::Booking), "the receipt refuses what the device front allows");
    assert_eq!(accepted(&core, clock, 5_044, 1), Ok(()), "the record's front is the end plus four");
    assert!(!dev_open(&tx), "the device burst is closed");

    // F4 (issue #99, mech:planned-state-read-as-actual): a finite burst abandoned by a short send leaves the record's chain joinable at its
    // end `e`. A 10-sample piece booked there is accepted as a joined piece (front end + 4) and is then sent as a fresh closed short burst
    // (the device burst is closed), whose front is the gap (15): a 4-sample `DropAndFlag` booking accepted at the joined front is dropped at
    // dispatch.
    let mut rig = switch_rig_with(vec![o::ramp(5, 2_000), o::ramp(6, 10), o::ramp(7, 4)], vec![FakeFault::ShortSend(0)]);
    book(&mut rig, 1, 0, 10_000);
    run(&mut rig, 0, 1);
    assert_eq!(sends(&rig.device), [(500, true, false), (2, false, true)], "{:?}", rig.device.calls());
    assert_eq!(dev_front(&rig.tx), Some(10_517), "the abandon's front: the cursor plus two plus the gap");
    assert!(!dev_open(&rig.tx));
    assert!(evs(&rig.events).is_empty(), "no event at the abandon");
    book(&mut rig, 2, 1, 12_000);
    assert!(evs(&rig.events).is_empty(), "the piece at e is accepted as a join of the abandoned burst");
    let clock = rig.clock;
    assert_eq!(accepted(&rig.core, clock, 12_013, 1), Err(Overlap::Booking));
    assert_eq!(accepted(&rig.core, clock, 12_014, 1), Ok(()), "the record's front of the joined piece: its end plus four");
    book(&mut rig, 3, 2, 12_014);
    assert!(evs(&rig.events).is_empty(), "C is accepted at receipt");
    let before = sends(&rig.device).len();
    run(&mut rig, 1, 20);
    assert_eq!(sends(&rig.device)[before..], [(10, true, true)], "B is a fresh standalone closed burst: {:?}", rig.device.calls());
    assert_eq!(evs(&rig.events), ["radio.TIME_ERROR \"drop\""], "C is dropped at dispatch instead of refused at receipt");
    assert_eq!(dev_front(&rig.tx), Some(12_025), "the standalone burst's front: its padded end plus the gap");
}

/// The events since the last call, as `kind outcome-or-reason`.
fn evs(events: &EventCollector) -> Vec<String> {
    events
        .drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
        .into_iter()
        .map(|e| format!("{} {}", e.kind.as_str(), e.payload.get("outcome").map(|v| v.to_string()).or_else(|| e.payload.get("reason").map(|v| v.to_string())).unwrap_or_default()))
        .collect()
}

// ------------------------------------------------------------------------------------------------------------ B8

#[test]
fn e3b_b8_a_new_owner_starts_with_a_closed_chain() {
    // The chain lives in the stream's book, which outlives an owner; a new owner starts with nothing open, no front and nothing to recover (K5), as
    // a `Tx` built once per start did before the chain moved.
    let (core, _device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    poke_open_device(&mut tx, 1_000, 20, true);
    poke_front(&mut tx, clock, 50);
    poke_reopen_needed(&mut tx);
    assert_eq!(dev_line(&tx), "burst true first Some(1000) next Some(20) tail true front Some(50) recovery true");
    let (_to_tx, cmds) = std::sync::mpsc::channel();
    let fresh = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
    assert_eq!(dev_line(&fresh), "burst false first None next None tail false front None recovery false");
    assert_eq!(dev_line(&tx), dev_line(&fresh), "one book: the old owner's handle reads the same chain");
}

// ------------------------------------------------------------------------------------------------------------ the ends the fold applies

#[test]
fn e3b_c3c_a_loss_found_while_the_open_burst_ends() {
    // `end_open(true)` on a lost device sends no close: the device burst ends and a pending recovery is cleared, as every loss clears it, and the
    // front stays (E3b-4: the section that ends the booking applies the loss).
    let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    poke_open_device(&mut tx, 1_000_000, 5_100, true);
    poke_front(&mut tx, clock, 5_000);
    poke_reopen_needed(&mut tx);
    assert_eq!(dev_line(&tx), "burst true first Some(1000000) next Some(5100) tail true front Some(5000) recovery true");
    core.device_lost("test loss");
    tx.end_open(true);
    assert_eq!(dev_line(&tx), "burst false first None next None tail false front Some(5000) recovery false");
    assert!(sends(&device).is_empty(), "a lost device is sent no close");
}

#[test]
fn e3b_c4c_the_stat_reads_the_front_the_close_leaves() {
    // The owner's close with no held tail fronts at the cursor + 2 + 3, one past the record's front for a chain that ends there (held-tail close:
    // `end + 4`); the stat compares the device front the close left, not the one before it (E3b-4: `end_open` returns it from its last section).
    let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 40, false, LatePolicy::DropAndFlag));
    device_at(&core, &time, &device, core.ticks(12_000_000));
    txp_drain(&mut tx);
    device_at(&core, &time, &device, core.ticks(15_500_000));
    tx.chain().continue_device_burst(20_040, None);
    assert_eq!(dev_front(&tx), None, "no device front before the close");
    assert!(tx.close_unjoined(clock, 20_040));
    assert_eq!(dev_front(&tx), Some(20_045));
    assert_eq!(lock(&core.rec).stats.get("tx_close_past_record_front").copied(), Some(1), "the close's front 20 045 is past the record's 20 044");
}
