//! #99: a finite burst the owner abandons ends its chain's join in the record, in the section that abandons it (RM-15, UR-22): a booking at its end is
//! refused at receipt, a piece accepted behind it before the abandon was known is a chain of its own, and an abandon that follows a Stop or a loss
//! changes nothing. Every test is single-threaded on the frozen rigs of `switch_rig_with` (1 MS/s, 200 root ticks a sample, a 10 000 sample window,
//! `g` = 15): device time moves only through `at` and `run`, so no verdict depends on when a thread runs. The record is read as a receipt reads it
//! (`receipt_runs`, `accepted`), never through its fields.

use super::e1::{at, book, stop_receipt};
use super::e3b::{accepted, at_nth_lock, dev_front, evs, receipt_runs};
use super::*;
use crate::provider::test_support::probe;
use crate::provider::tx::oracle as o;
use ezsdr_radio::staging::Overlap;

const UR_21_OVERLAP: &str = "radio.COMMAND_REJECTED \"UR-21: the burst overlaps an accepted booking\"";

fn underflow(rig: &mut SwitchRig) {
    rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(rig.clock.instant(10_200)), channel: 0 });
    drain_tx(&mut rig.tx);
}

fn seq_error(rig: &mut SwitchRig) {
    rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::SeqError, tick: None, channel: 0 });
    drain_tx(&mut rig.tx);
}

/// A 2 000 sample burst A at 10 000, a 10 sample B and a 4 sample C (the ramps 5, 6 and 7), the device faulting as `faults` says.
fn abc(faults: Vec<FakeFault>) -> SwitchRig {
    switch_rig_with(vec![o::ramp(5, 2_000), o::ramp(6, 10), o::ramp(7, 4)], faults)
}

#[test]
fn n99_u1_a_booking_at_the_end_of_an_abandoned_burst_is_refused() {
    // S1, the issue's sequence: A is abandoned by a short send. B at A's end is refused at receipt (before: accepted as a join of the burst the device
    // closed, and sent alone), so C, booked at B's predicted front, is a plain booking: sent, not dropped at dispatch.
    let mut rig = abc(vec![FakeFault::ShortSend(0)]);
    book(&mut rig, 1, 0, 10_000);
    run(&mut rig, 0, 1);
    assert_eq!(sends(&rig.device), [(500, true, false), (2, false, true)], "{:?}", rig.device.calls());
    assert_eq!(dev_front(&rig.tx), Some(10_517), "the abandon's front: the cursor plus two plus the gap");
    assert!(evs(&rig.events).is_empty(), "no event at the abandon");
    let clock = rig.clock;
    assert_eq!(receipt_runs(&rig.core, clock, 11_990, 12_100), "E11990..=12003 .12004..=12099", "A's footprint ends at its end plus four; no booking at its end joins it");
    book(&mut rig, 2, 1, 12_000);
    assert_eq!(evs(&rig.events), [UR_21_OVERLAP]);
    book(&mut rig, 3, 2, 12_014);
    assert!(evs(&rig.events).is_empty(), "C is accepted: B left no region");
    run(&mut rig, 1, 20);
    assert_eq!(sends(&rig.device)[2..], [(4, true, true)], "C is sent as its own closed burst: {:?}", rig.device.calls());
    assert!(evs(&rig.events).is_empty(), "C is not dropped at dispatch");
    assert_eq!(dev_front(&rig.tx), Some(12_033), "C's closed front: its padded end plus the gap");
}

/// What a case expects after A, 2 000 samples at 10 000, is abandoned: the device's sends and front, the events, and the record's verdicts at A's end.
struct Abandoned {
    case: &'static str,
    sends: Vec<(usize, bool, bool)>,
    front: Option<i64>,
    events: Vec<&'static str>,
}

#[test]
fn n99_u2_every_way_a_finite_burst_is_abandoned_ends_its_chain() {
    // S2: the send's three arms (short, failed, stalled), the sequence report and the underflow report, and the open burst's expired timeline: each one reaches
    // the record's one section, `abandon`, and each leaves A's footprint and no join at its end.
    let cases = vec![
        (Abandoned { case: "a short send", sends: vec![(500, true, false), (2, false, true)], front: Some(10_517), events: vec![] }, 0),
        (
            Abandoned {
                case: "a failed send",
                sends: vec![(500, true, false), (2, false, true)],
                front: Some(11_017),
                events: vec!["radio.COMMAND_REJECTED \"fake: the send failed after half its samples\""],
            },
            1,
        ),
        (Abandoned { case: "a stalled send", sends: vec![(0, true, false)], front: None, events: vec![] }, 2),
        (Abandoned { case: "an underflow report", sends: vec![(1000, true, false), (2, false, true)], front: Some(11_017), events: vec!["radio.TX_UNDERFLOW "] }, 3),
        (Abandoned { case: "a sequence error report", sends: vec![(1000, true, false), (2, false, true)], front: Some(11_017), events: vec!["radio.TX_UNDERFLOW "] }, 4),
        (Abandoned { case: "the open burst's timeline expired", sends: vec![(1000, true, false), (999, false, false), (2, false, true)], front: Some(12_004), events: vec![] }, 5),
    ];
    for (expected, how) in cases {
        let faults = match how {
            0 => vec![FakeFault::ShortSend(0)],
            1 => vec![FakeFault::FailSend(0)],
            2 => vec![FakeFault::StalledSend(0)],
            _ => vec![],
        };
        let mut rig = abc(faults);
        book(&mut rig, 1, 0, 10_000);
        run(&mut rig, 0, 1);
        match how {
            3 => underflow(&mut rig),
            4 => seq_error(&mut rig),
            5 => run(&mut rig, 1, 12),
            _ => {}
        }
        let case = expected.case;
        assert_eq!(sends(&rig.device), expected.sends, "{case}: {:?}", rig.device.calls());
        assert_eq!(dev_front(&rig.tx), expected.front, "{case}");
        assert_eq!(evs(&rig.events), expected.events, "{case}");
        assert_eq!(accepted(&rig.core, rig.clock, 12_000, 10), Err(Overlap::Booking), "{case}: a booking at A's end does not join it");
        assert_eq!(accepted(&rig.core, rig.clock, 12_003, 1), Err(Overlap::Booking), "{case}: the footprint");
        assert_eq!(accepted(&rig.core, rig.clock, 12_004, 1), Ok(()), "{case}: A's footprint ends at its end plus four");
    }
}

#[test]
fn n99_u3_a_piece_accepted_before_the_abandon_is_a_chain_of_its_own() {
    // S4: B is booked at A's end while A can still be continued, A is then abandoned: B stays accepted and is sent alone, with its own front (its padded
    // end plus the gap), so a booking C at B's joined front is refused (before: accepted, then dropped at dispatch).
    let mut rig = abc(vec![FakeFault::ShortSend(0)]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 12_000);
    assert!(evs(&rig.events).is_empty(), "B joins A at receipt");
    run(&mut rig, 0, 1);
    assert_eq!(sends(&rig.device), [(500, true, false), (2, false, true)], "{:?}", rig.device.calls());
    assert!(evs(&rig.events).is_empty());
    assert_eq!(receipt_runs(&rig.core, rig.clock, 11_990, 12_100), "E11990..=12024 .12025..=12099", "B's region [12000, 12025) follows A's footprint");
    book(&mut rig, 3, 2, 12_014);
    assert_eq!(evs(&rig.events), [UR_21_OVERLAP], "C is refused at receipt, not dropped later");
    run(&mut rig, 1, 20);
    assert_eq!(sends(&rig.device)[2..], [(10, true, true)], "B is sent as its own closed burst: {:?}", rig.device.calls());
    assert_eq!(dev_front(&rig.tx), Some(12_025));
    assert!(evs(&rig.events).is_empty());

    // S4b: C is accepted before the abandon too (the receipt is never revoked): it is judged by its late policy at dispatch.
    let mut rig = abc(vec![FakeFault::ShortSend(0)]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 12_000);
    book(&mut rig, 3, 2, 12_014);
    assert!(evs(&rig.events).is_empty());
    run(&mut rig, 0, 20);
    assert_eq!(sends(&rig.device)[2..], [(10, true, true)], "{:?}", rig.device.calls());
    assert_eq!(evs(&rig.events), ["radio.TIME_ERROR \"drop\""], "the residual: C was accepted at B's predicted front");
    assert_eq!(dev_front(&rig.tx), Some(12_025));
}

#[test]
fn n99_u4_the_abandoned_middle_piece_leaves_the_pieces_before_and_behind_it() {
    // S5: A (2 000), B (2 000) and C (10) are joined; a short send abandons B. A and B stay one chain that takes no join at 14 000; C is a chain of its own
    // and the record fronts it at 14 025 (its padded end plus the gap), where it was 14 014 (before: C's end plus four, and a booking there was dropped).
    let mut rig = switch_rig_with(vec![o::ramp(5, 2_000), o::ramp(6, 2_000), o::ramp(7, 10), o::ramp(8, 4), o::ramp(9, 4)], vec![FakeFault::ShortSend(2)]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 12_000);
    book(&mut rig, 3, 2, 14_000);
    assert!(evs(&rig.events).is_empty());
    run(&mut rig, 0, 4);
    assert_eq!(sends(&rig.device), [(1000, true, false), (1000, false, false), (1000, false, false), (2, false, true)], "{:?}", rig.device.calls());
    assert_eq!(dev_front(&rig.tx), Some(13_017));
    assert_eq!(receipt_runs(&rig.core, rig.clock, 13_990, 14_100), "E13990..=14024 .14025..=14099");
    assert_eq!(accepted(&rig.core, rig.clock, 14_010, 4), Err(Overlap::Booking), "C is not continued (before: accepted)");
    book(&mut rig, 4, 3, 14_014);
    assert_eq!(evs(&rig.events), [UR_21_OVERLAP], "D at C's joined front is refused");
    run(&mut rig, 4, 30);
    assert_eq!(sends(&rig.device)[4..], [(10, true, true)], "C is sent as its own closed burst");
    assert_eq!(dev_front(&rig.tx), Some(14_025));
    assert!(evs(&rig.events).is_empty());
}

#[test]
fn n99_u5_a_tail_that_comes_up_short_abandons_the_piece_that_waits_for_it() {
    // S6: A (2 000) is sent with its held tail, B (2 000) is booked at its end and its first send is the tail's: the tail send is short, B is abandoned unsent.
    // A and B stay one chain, which takes no join at 14 000.
    let mut rig = switch_rig_with(vec![o::ramp(5, 2_000), o::ramp(6, 2_000), o::ramp(7, 10)], vec![FakeFault::ShortSend(2)]);
    book(&mut rig, 1, 0, 10_000);
    run(&mut rig, 0, 3);
    assert_eq!(sends(&rig.device), [(1000, true, false), (999, false, false)], "{:?}", rig.device.calls());
    book(&mut rig, 2, 1, 12_000);
    assert!(evs(&rig.events).is_empty(), "B joins A at receipt");
    run(&mut rig, 3, 4);
    assert_eq!(sends(&rig.device), [(1000, true, false), (999, false, false), (0, false, false), (2, false, true)], "{:?}", rig.device.calls());
    assert_eq!(dev_front(&rig.tx), Some(12_017));
    assert_eq!(accepted(&rig.core, rig.clock, 14_000, 10), Err(Overlap::Booking), "B's end: no join");
    assert_eq!(accepted(&rig.core, rig.clock, 14_004, 10), Ok(()));
}

#[test]
fn n99_u6_the_waiting_device_burst_that_expires_re_frames_the_piece_held_behind_it() {
    // S7: A is sent whole and the device burst waits for a continuation; B at A's end is held; device time passes the burst's timeline (no open
    // booking: the abandon's end is the burst's cursor): A's chain ends, B is a chain of its own.
    let mut rig = abc(vec![]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 12_000);
    run(&mut rig, 0, 2);
    assert_eq!(sends(&rig.device), [(1000, true, false), (1000, false, false)], "{:?}", rig.device.calls());
    assert_eq!(receipt_runs(&rig.core, rig.clock, 11_990, 12_100), "E11990..=12009 .12010..=12010 E12011..=12013 .12014..=12099", "A and B are one chain, joinable at B's end");
    at(&rig, rig.core.ticks(12_500_000));
    drain_tx(&mut rig.tx);
    rig.tx.step();
    assert_eq!(sends(&rig.device), [(1000, true, false), (1000, false, false), (2, false, true)], "{:?}", rig.device.calls());
    assert_eq!(dev_front(&rig.tx), Some(12_017));
    assert_eq!(receipt_runs(&rig.core, rig.clock, 11_990, 12_100), "E11990..=12024 .12025..=12099", "A's chain ends at 12 000 with its footprint; B's front is its own");
}

thread_local! {
    static CONTROL: std::cell::RefCell<Option<Control>> = const { std::cell::RefCell::new(None) };
}

/// uhd-control's Stop of the stream, from inside the owner's send: the probe's closure is `Send + Sync`, so the control is parked in a thread-local.
fn stop_by(core: &Core) {
    CONTROL.with(|slot| {
        slot.borrow_mut().as_mut().expect("the control is parked").book(Dispatched::new(
            ActionId(9),
            Action::Stop { target: Some(crate::provider::test_support::target_of(&core.tx_id)) },
            Some(core.tx_id.clone()),
        ))
    });
}

#[test]
fn n99_u7_a_stop_or_a_loss_that_comes_during_the_send_is_not_undone_by_its_abandon() {
    // F1: A (2 000 at 10 000) with B (10) joined behind it; the first send takes 0.5 s of device time (a short or stalled send waits out SEND_TIMEOUT, 1 s),
    // a Stop or a loss comes in it, and the send then comes up short or fails. The record the halt left (A's chain ended, its front at B's end plus four: a
    // booking at 12 014 is accepted) is the record after the abandon: without the guard the abandon split B off with its own front 12 025.
    for (case, fault, loss) in
        [("a Stop, a short send", FakeFault::ShortSend(0), false), ("a Stop, a failed send", FakeFault::FailSend(0), false), ("a device loss, a short send", FakeFault::ShortSend(0), true)]
    {
        let rig = abc(vec![fault]);
        let SwitchRig { core, device, time, mut control, mut tx, clock, waves, _frozen, .. } = rig;
        for (id, wave, k) in [(1, 0, 10_000), (2, 1, 12_000)] {
            book_with(&mut control, &core, id, 0, clock, k, &waves[wave], false, LatePolicy::DropAndFlag, BTreeMap::new());
        }
        CONTROL.with(|slot| *slot.borrow_mut() = Some(control));
        {
            let (c, t, d) = (core.clone(), time.clone(), device.clone());
            probe(&core, "tx_send_planned", move |_| {
                device_at(&c, &t, &d, c.ticks(500_000_000));
                if loss { c.device_lost("n99") } else { stop_by(&c) }
            });
        }
        turns(&core, &time, &device, &mut tx, 0, 1, 1);
        let closed: &[(usize, bool, bool)] = if loss { &[(500, true, false)] } else { &[(500, true, false), (2, false, true)] };
        assert_eq!(sends(&device), closed, "{case}: {:?}", device.calls());
        assert_eq!(receipt_runs(&core, clock, 11_990, 12_100), "E11990..=12013 .12014..=12099", "{case}: the halt's record stands");
    }
}

#[test]
fn n99_u8_a_halt_the_owner_has_not_read_is_not_undone_by_a_report_driven_abandon() {
    // F1: A is open at 11 000 with B booked behind it; device time passes 11.5 ms before the owner runs again; a Stop (unread) or a loss comes, and an
    // underflow report abandons A before the owner reads the Stop (a pass reads the reports first). With no halt the same abandon re-frames B.
    for (case, halt, expected) in [("a Stop", 0, "E11990..=12013 .12014..=12099"), ("a device loss", 1, "E11990..=12013 .12014..=12099"), ("no halt", 2, "E11990..=12024 .12025..=12099")] {
        let mut rig = abc(vec![]);
        book(&mut rig, 1, 0, 10_000);
        run(&mut rig, 0, 1);
        book(&mut rig, 2, 1, 12_000);
        at(&rig, rig.core.ticks(11_500_000));
        match halt {
            0 => stop_receipt(&mut rig),
            1 => rig.core.device_lost("n99"),
            _ => {}
        }
        underflow(&mut rig);
        assert_eq!(receipt_runs(&rig.core, rig.clock, 11_990, 12_100), expected, "{case}");
    }
}

#[test]
fn n99_u9_a_receipt_between_the_sends_and_the_abandon_does_not_hide_the_abandoned_chain() {
    // F2: A (2 000), B (2 000) and C (10) are joined; device time passes B's start before the owner runs again, and a receipt (D, far ahead) prunes the
    // record at 12.5 ms: B's join point is gone from the chain. The report that abandons A still ends A's chain and frames B and C behind it.
    let mut rig = switch_rig_with(vec![o::ramp(5, 2_000), o::ramp(6, 2_000), o::ramp(7, 10), o::ramp(8, 4)], vec![]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 12_000);
    book(&mut rig, 3, 2, 14_000);
    run(&mut rig, 0, 1);
    at(&rig, rig.core.ticks(12_500_000));
    book(&mut rig, 4, 3, 30_000);
    assert!(evs(&rig.events).is_empty());
    underflow(&mut rig);
    assert_eq!(receipt_runs(&rig.core, rig.clock, 11_990, 14_100), "E11990..=14024 .14025..=14099");
    assert_eq!(accepted(&rig.core, rig.clock, 14_010, 4), Err(Overlap::Booking), "C is not continued");
}

#[test]
fn n99_u10_a_receipt_at_the_seam_is_decided_by_the_record_it_finds() {
    // S8: B is received between the short send's completion section and the abandon's record section (the owner's third streams lock after the plan) and
    // is accepted as a join: the abandon then frames it. A B received after the abandon's section (the fourth lock) is refused.
    for (nth, accepted_at_seam) in [(3, true), (4, false)] {
        let rig = abc(vec![FakeFault::ShortSend(0)]);
        let SwitchRig { core, device, time, events, mut control, mut tx, clock, waves, _frozen } = rig;
        book_with(&mut control, &core, 1, 0, clock, 10_000, &waves[0], false, LatePolicy::DropAndFlag, BTreeMap::new());
        CONTROL.with(|slot| *slot.borrow_mut() = Some(control));
        let wave = waves[1].clone();
        {
            let (c, c2) = (core.clone(), core.clone());
            probe(&core, "tx_send_planned", move |_| {
                let c = c.clone();
                let wave = wave.clone();
                at_nth_lock(
                    &c2,
                    |core| &core.streams,
                    nth,
                    Box::new(move || CONTROL.with(|slot| book_with(slot.borrow_mut().as_mut().unwrap(), &c, 2, 0, clock, 12_000, &wave, false, LatePolicy::DropAndFlag, BTreeMap::new()))),
                );
            });
        }
        turns(&core, &time, &device, &mut tx, 0, 1, 1);
        assert_eq!(evs(&events).is_empty(), accepted_at_seam, "B at the seam (lock {nth}): {:?}", evs(&events));
        assert_eq!(sends(&device), [(500, true, false), (2, false, true)], "{:?}", device.calls());
        assert_eq!(receipt_runs(&core, clock, 11_990, 12_100), if accepted_at_seam { "E11990..=12024 .12025..=12099" } else { "E11990..=12003 .12004..=12099" }, "lock {nth}");
    }
}

#[test]
fn n99_u11_what_the_abandon_leaves_to_a_later_booking_and_a_later_stop() {
    // S3: a long B at A's end is refused like a short one, and a long booking at A's footprint end is a plain booking.
    let mut rig = switch_rig_with(vec![o::ramp(5, 2_000), o::ramp(6, 200), o::ramp(7, 200)], vec![FakeFault::ShortSend(0)]);
    book(&mut rig, 1, 0, 10_000);
    run(&mut rig, 0, 1);
    book(&mut rig, 2, 1, 12_000);
    assert_eq!(evs(&rig.events), [UR_21_OVERLAP]);
    book(&mut rig, 3, 2, 12_004);
    assert!(evs(&rig.events).is_empty());
    // S9 (RM-16): a Stop after the abandon cancels what B was, a piece the abandon framed on its own: it leaves the region [t, B) like any booking that
    // starts at or after t (before: B was a joined piece of A's chain, and the Stop left none).
    let mut rig = abc(vec![FakeFault::ShortSend(0)]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 12_000);
    run(&mut rig, 0, 1);
    at(&rig, rig.core.ticks(11_000_000));
    stop_receipt(&mut rig);
    assert_eq!(receipt_runs(&rig.core, rig.clock, 11_900, 25_100), "E11900..=21004 .21005..=25099", "the Stop's region [11 000, 21 005)");
}
