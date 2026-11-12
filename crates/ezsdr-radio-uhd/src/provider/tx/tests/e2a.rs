//! E2a-2a (design note 49 stage E2a; plan `2026-11-10-e2a/test-plan.md` §3.2): the open booking in the stream's book, each send planned
//! in one section under the streams lock, made with no lock held and completed in another section, and the test build's checks of
//! the line lock. E2a-2b (§3.3): the abandon reaches only its own repeat and the switch replacing it (#80, F3), a halt cancels an open
//! booking that issued nothing (#85), and a late underflow with no device burst open changes nothing (A5, F4). The frozen single-threaded rig of E1: device time moves only through `device_at`, and probes run on the test thread
//! with no lock held.
//!
//! Units: one sample is 1 us (the rig's 1 MS/s), 200 root ticks; `MS` is a millisecond in root ticks.

use super::e1::{MS, at, book, handed_end, last_send_eob, notes, params, playing, rejection_events, rows, shutdown, stop_receipt};
use super::*;
use crate::provider::tx::oracle as o;
use ezsdr_radio::staging::STOP_FRONT;
use ezsdr_radio::timeline::Kind;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The sends carrying an end-of-burst, as the fake recorded them.
fn closing_sends(device: &FakeDevice) -> Vec<String> {
    device.calls().into_iter().filter(|call| call.starts_with("tx_send ") && call.ends_with("eob=true")).collect()
}

#[test]
fn e2a_kl39_a_finite_burst_abandoned_by_a_short_send_keeps_its_whole_region() {
    // KL-39 (RM-15, UR-22): A (2 000 samples at 10 000) is cut short on its first send, the fake taking 500 of 1 000, so the owner
    // abandons it at 10 500. A finite burst keeps its whole booked region (to its held-tail front 12 004), not the cursor's footprint
    // 10 505 an abandoned repeat keeps: a booking there is refused at receipt, one at A's front is accepted.
    let mut rig = switch_rig_with(vec![o::ramp(5, 2_000), o::ramp(6, 200)], vec![FakeFault::ShortSend(0)]);
    book(&mut rig, 1, 0, 10_000);
    run(&mut rig, 0, 1);
    assert_eq!(closing_sends(&rig.device), [format!("tx_send 0 n=2 at={} sob=false eob=true", rig.clock.instant(10_500))], "abandoned at 10 500: {:?}", rig.device.calls());
    rejected(&rig.events);
    book(&mut rig, 2, 1, 10_500 + STOP_FRONT);
    book(&mut rig, 3, 1, 12_004);
    assert_eq!(refused_ids(&rig.events), [2], "{:?}", lock(&rig.core.rec).rejected);
}

#[test]
fn e2a_lock_reentry_of_one_mutex_panics() {
    // On its own thread, so that a missing check hangs that thread, not the test: the 10 s bound is reached only then.
    let (done, panicked) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mutex = std::sync::Mutex::new(0);
        let reentered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = lock(&mutex);
            let _again = lock(&mutex);
        }));
        let _ = done.send(reentered.err().and_then(|panic| panic.downcast_ref::<&str>().map(|text| text.to_string())));
    });
    // A missing check deadlocks the worker: only the bound sees it, so it is a declared bound kill (note 51 §6.3).
    let message = match panicked.recv_timeout(Wall::from_secs(10)) {
        Ok(message) => message,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            crate::provider::test_support::infra("the re-entry did not panic within 10 s");
            panic!("the re-entry did not panic: Timeout")
        }
        Err(disconnected) => panic!("the re-entry's thread ended without a report: {disconnected:?}"),
    };
    assert!(message.is_some_and(|text| text.contains("re-entered")), "the re-entry's panic names it");
}

#[test]
#[should_panic(expected = "emit was made under the line lock")]
fn e2a_emit_under_the_streams_lock_panics() {
    let rig = switch_rig(vec![vec![OLD; 100]]);
    let _streams = lock(&rig.core.streams);
    rig.core.emit(&rig.core.tx_id, ezsdr_radio::kinds::COMMAND_REJECTED, ezsdr_kernel::event::Severity::Error, serde_json::json!({}));
}

#[test]
fn e2a_a_send_records_the_longest_lock_wait() {
    let mut rig = switch_rig(vec![vec![OLD; 100]]);
    assert!(!lock(&rig.core.rec).stats.contains_key("tx_lock_wait_max_ns"), "the key is written by the first send");
    book(&mut rig, 1, 0, 20_000);
    run(&mut rig, 0, 30);
    let wait = lock(&rig.core.rec).stats.get("tx_lock_wait_max_ns").copied();
    assert!(wait.is_some_and(|ns| ns >= 0), "{wait:?}");
}

#[test]
fn core_stat_max_keeps_the_largest() {
    let rig = switch_rig(vec![vec![OLD; 100]]);
    for n in [3, 9, 5] {
        rig.core.stat_max("e2a_max", n);
    }
    assert_eq!(lock(&rig.core.rec).stats.get("e2a_max"), Some(&9));
}

#[test]
fn e2a_s6_a_short_send_with_a_stop_unread_closes_once_then_cancels_held() {
    // A (2 000 samples at 10 000) is cut short on its first send: the fake takes 500 of 1 000. H (40 000) was booked before the Stop,
    // which uhd-control receives after the step's guard; H' (60 000) after the Stop. The abandon's two-zero close at A's cursor is the
    // only end-of-burst; reading the Stop then finds no device burst open, cancels H, and keeps H' (UR-22, UR-26).
    let mut rig = switch_rig_with(vec![o::ramp(5, 2_000), o::ramp(6, 200)], vec![FakeFault::ShortSend(0)]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 40_000);
    rig.tx.read_list();
    at(&rig, MS);
    let SwitchRig { core, device, time, events, control, mut tx, clock, waves, _frozen } = rig;
    let control = Arc::new(Mutex::new(Some(control)));
    let fired = Arc::new(AtomicBool::new(false));
    // The probe holds the Core weakly, and the control is taken back after the step: a probe holding either keeps the rig from ever
    // dropping (UR-33's drop check).
    let (hook_core, hook_control, wave) = (Arc::downgrade(&core), control.clone(), waves[1].clone());
    crate::provider::test_support::probe(&core, "tx_step_guarded", move |_| {
        let Some(hook_core) = hook_core.upgrade() else { return };
        if !fired.swap(true, Ordering::SeqCst) {
            let mut control = hook_control.lock().unwrap();
            let control = control.as_mut().expect("the rig's control");
            control.book(Dispatched::new(ActionId(9), Action::Stop { target: Some(crate::provider::test_support::target_of(&hook_core.tx_id)) }, Some(hook_core.tx_id.clone())));
            book_with(control, &hook_core, 3, 0, clock, 60_000, &wave, false, LatePolicy::DropAndFlag, BTreeMap::new());
        }
    });
    assert!(tx.step());
    let _control = control.lock().unwrap().take();
    tx.read_list();
    let closes = closing_sends(&device);
    assert_eq!(closes, [format!("tx_send 0 n=2 at={} sob=false eob=true", clock.instant(10_500))], "one close, the abandon's: {:?}", device.calls());
    assert_eq!(notes(&core), ["UR-26: cancelled by Stop"], "H: {:?}", lock(&core.rec).rejected);
    turns(&core, &time, &device, &mut tx, 2, 70, 1);
    assert_eq!(device.transmitted(clock.instant(60_000), 200), o::ramp(6, 200).into_iter().map(Some).collect::<Vec<_>>(), "H' plays: {:?}", device.calls());
    assert!(rejection_events(&events).is_empty());
}

#[test]
fn e2a_a_stop_received_between_the_plan_and_the_device_call_changes_nothing_in_the_send() {
    // A repeat plays; the step plans its next chunk, and the Run's end is booked before the chunk reaches the device. The chunk goes
    // out whole, inside the end's bound, and the end's read closes the device burst after it (RM-16, UR-25).
    let mut rig = playing();
    let t = rig.core.now() + 3 * MS;
    at(&rig, t);
    let fired = Arc::new(AtomicBool::new(false));
    let core = Arc::downgrade(&rig.core);
    crate::provider::test_support::probe(&rig.core, "tx_send_planned", move |_| {
        let Some(core) = core.upgrade() else { return };
        if !fired.swap(true, Ordering::SeqCst) {
            crate::provider::test_support::end_streams(&core, t, false, false);
        }
    });
    let before = sends(&rig.device).len();
    assert!(rig.tx.step());
    let sent = sends(&rig.device);
    let chunk = (rig.core.tx_in_flight() / rig.clock.n / 4) as usize;
    assert_eq!(sent[before..], [(chunk, false, false)], "the planned chunk is handed whole: {:?}", rig.device.calls());
    rig.tx.read_list();
    let (_, bound) = params().stop_bound(t);
    assert!(handed_end(&rig) <= bound, "nothing past {bound} is handed: {:?}", rig.device.calls());
    assert!(last_send_eob(&rig), "{:?}", rig.device.calls());
    assert_eq!(rows(&rig)[0]["end"], "stop");
}

#[test]
fn e2a_loss_between_the_plan_and_the_device_call_completes_once_and_shutdown_notes_the_held() {
    // A (12 000 samples at 10 000) is open and H (40 000) held; the device is lost after a send is planned and before it is made.
    // The send fails once, as a loss: one DEVICE_LOST, no close after it; the shutdown cancels H with the Run end's note (UR-26, UR-29).
    let mut rig = switch_rig_with(vec![o::ramp(5, 12_000), o::ramp(6, 200)], vec![FakeFault::Lost(Wall::from_millis(20))]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 40_000);
    rig.tx.read_list();
    run(&mut rig, 0, 10);
    at(&rig, 13 * MS);
    let fired = Arc::new(AtomicBool::new(false));
    let (core, time, device) = (Arc::downgrade(&rig.core), rig.time.clone(), rig.device.clone());
    crate::provider::test_support::probe(&rig.core, "tx_send_planned", move |_| {
        let Some(core) = core.upgrade() else { return };
        if !fired.swap(true, Ordering::SeqCst) {
            time.advance_to(core.at(20 * MS)).unwrap();
            device.set_time(20 * MS);
        }
    });
    let before = sends(&rig.device).len();
    assert!(rig.tx.step());
    assert_eq!(sends(&rig.device).len(), before, "the lost send is not made, and no close follows it: {:?}", rig.device.calls());
    let lost = rig.events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert_eq!(lost.iter().filter(|event| event.kind.as_str() == EventKind::DEVICE_LOST).count(), 1, "{lost:?}");
    let (core, _) = shutdown(rig);
    assert_eq!(notes(&core), ["UR-26: cancelled by Provider::stop"], "H: {:?}", lock(&core.rec).rejected);
}

// ------------------------------------------------------------------------------------------------------------ E2a-2b

/// The owner's turn as the run loop makes it: read the list, step until there is nothing to do.
fn turn(rig: &mut SwitchRig) {
    for _ in 0..8 {
        drain_tx(&mut rig.tx);
        if !rig.tx.step() {
            break;
        }
    }
}

/// The ids of the Actions refused since the last call.
fn refused_ids(events: &EventCollector) -> Vec<u64> {
    rejected(events).iter().filter_map(|event| event.payload["action"].as_u64()).collect()
}

/// R (a repeat of `waves[0]`, 1 000 samples, from 2 000) handed through 1 ms; at 2.5 ms the owner's fourth send, [9 500, 12 000), fails
/// (`FailSend(3)`). Between the step's guard and that send, uhd-control receives a Stop and then B (`waves[1]`, a repeat when `b_repeat`)
/// at the Stop's bound. Returns the rig, the abandon's cursor (the failed send's attempted end) and B's start.
fn stop_then_b_then_a_failed_send(b_repeat: bool) -> (SwitchRig, i64, i64) {
    let mut rig = switch_rig_with(vec![o::ramp(17, 1_000), o::ramp(18, 200), o::ramp(19, 200)], vec![FakeFault::FailSend(3)]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 1);
    let t = 5 * MS / 2;
    at(&rig, t);
    let (_, b_start) = params().stop_bound(t);
    let SwitchRig { core, device, time, events, control, mut tx, clock, waves, _frozen } = rig;
    let control = Arc::new(Mutex::new(Some(control)));
    let fired = Arc::new(AtomicBool::new(false));
    let (hook_core, hook_control, wave) = (Arc::downgrade(&core), control.clone(), waves[1].clone());
    crate::provider::test_support::probe(&core, "tx_step_guarded", move |_| {
        let Some(hook_core) = hook_core.upgrade() else { return };
        if !fired.swap(true, Ordering::SeqCst) {
            let mut control = hook_control.lock().unwrap();
            let control = control.as_mut().expect("the rig's control");
            control.book(Dispatched::new(ActionId(9), Action::Stop { target: Some(crate::provider::test_support::target_of(&hook_core.tx_id)) }, Some(hook_core.tx_id.clone())));
            book_with(control, &hook_core, 3, 0, clock, b_start, &wave, b_repeat, LatePolicy::DropAndFlag, BTreeMap::new());
        }
    });
    assert!(tx.step());
    let control = control.lock().unwrap().take().expect("the rig's control");
    let rig = SwitchRig { core, device, time, events, control, tx, clock, waves, _frozen };
    let failed = rig.device.calls().into_iter().filter(|call| call.starts_with("tx_send 0 ")).nth(3).expect("the fourth send");
    let field = |name: &str| failed.split_whitespace().find_map(|word| word.strip_prefix(name)).map(str::to_owned).expect("a field");
    let first = rig.clock.at_or_after(field("at=").parse().unwrap());
    assert_eq!(first, 9_500, "the failing send is the intended one: {:?}", rig.device.calls());
    // `calls()` logs the accepted half of a `FailSend`; the cursor is the attempted end.
    let cursor = first + 2 * field("n=").parse::<i64>().unwrap();
    assert!(cursor + STOP_FRONT <= b_start, "B starts at or after the abandoned repeat's footprint");
    (rig, cursor, b_start)
}

#[test]
fn e2a_80_a_failed_send_after_a_stop_leaves_the_booking_accepted_after_it() {
    // #80 (RM-15, RM-21, UR-22): R's send fails after a Stop and B, accepted after it, are received: the abandon ends R at its cursor and
    // keeps B. A booking overlapping B is refused at receipt, B plays whole, the abandon's close is the one end-of-burst before B, and the
    // stream stays busy until B's end.
    let (mut rig, cursor, b_start) = stop_then_b_then_a_failed_send(false);
    rejected(&rig.events);
    book(&mut rig, 5, 2, b_start + 100);
    assert_eq!(refused_ids(&rig.events), [5], "a booking overlapping B is refused at receipt: {:?}", lock(&rig.core.rec).rejected);
    // R's region ends at its footprint, the cursor plus STOP_FRONT.
    book(&mut rig, 6, 2, cursor + STOP_FRONT - 1);
    book(&mut rig, 7, 2, cursor + STOP_FRONT);
    assert_eq!(refused_ids(&rig.events), [6], "R holds [2 000, c + STOP_FRONT): {:?}", lock(&rig.core.rec).rejected);
    let busy = lock(&rig.core.streams).stream(stream_key(Dir::Tx, 0)).book.record.stageable(rig.clock.instant(b_start + 200) - 1).err();
    assert_eq!(busy, Some(ezsdr_radio::staging::Refusal::TxBusy { until: ezsdr_radio::staging::Until::At(rig.clock.instant(b_start + 200)) }), "busy until B's end");
    run(&mut rig, 3, 20);
    assert_eq!(rig.device.transmitted(rig.clock.instant(b_start), 200), o::ramp(18, 200).into_iter().map(Some).collect::<Vec<_>>(), "B plays: {:?}", rig.device.calls());
    let before_b: Vec<String> = closing_sends(&rig.device).into_iter().filter(|call| !call.contains(&format!("at={}", rig.clock.instant(b_start)))).collect();
    assert_eq!(before_b.len(), 2, "the abandon's close and B's own end: {:?}", rig.device.calls());
    assert_eq!(before_b[0], format!("tx_send 0 n=2 at={} sob=false eob=true", rig.clock.instant(cursor)), "{:?}", rig.device.calls());
    assert!(notes(&rig.core).is_empty(), "B is not cancelled: {:?}", lock(&rig.core.rec).rejected);
}

#[test]
fn e2a_80_the_booking_accepted_after_the_stop_is_the_playing_repeat() {
    // #80 (RM-37): as above with B a repeat. The abandon of R leaves B the playing repeat, so a switch on B, once B sounds, is accepted.
    let (mut rig, _, b_start) = stop_then_b_then_a_failed_send(true);
    drain_tx(&mut rig.tx);
    assert_eq!(rig.tx.with_book(|book| book.playing.map(|playing| playing.id)), Some(ActionId(3)), "B is the playing repeat");
    run(&mut rig, 3, 13);
    assert!(rig.device.transmitted(rig.clock.instant(b_start), 200).iter().all(Option::is_some), "B sounds: {:?}", rig.device.calls());
    rejected(&rig.events);
    book_switch(&mut rig, 4, 13_000, 2);
    assert!(refused_ids(&rig.events).is_empty(), "the switch on B is accepted: {:?}", lock(&rig.core.rec).rejected);
    run(&mut rig, 13, 40);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied");
    let time_errors = rig.events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert!(time_errors.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{time_errors:?}");
}

/// A SendAsapAndFlag and a DropAndFlag booking of waves[2] at `k`, each judged at receipt: the ids refused.
fn book_both_at(rig: &mut SwitchRig, k: i64) -> Vec<u64> {
    rejected(&rig.events);
    book_with(&mut rig.control, &rig.core, 5, 0, rig.clock, k, &rig.waves[2], false, LatePolicy::SendAsapAndFlag, BTreeMap::new());
    book_with(&mut rig.control, &rig.core, 6, 0, rig.clock, k, &rig.waves[2], false, LatePolicy::DropAndFlag, BTreeMap::new());
    refused_ids(&rig.events)
}

#[test]
fn e2a_f3_the_pending_twin_answers_the_same() {
    // F3 (RM-15's footprint): S is pending (taken at receipt, its first send unissued) and R's cursor is at b = 15 000: an underflow
    // cancels S and R's front is b + 5, so a SendAsap and a DropAndFlag booking at b + 1 are both refused at receipt. S cannot be
    // applied with its boundary at the cursor: it applies only once its boundary send was taken (C2).
    let b = 15_000;
    let mut rig = switch_rig(vec![o::ramp(8, 1_000), o::ramp(9, 1_000), o::ramp(10, 100)]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 3);
    book_switch(&mut rig, 2, 3_000, 1);
    drain_tx(&mut rig.tx);
    rig.tx.with_book(|book| book.open.as_mut().expect("the repeat is handed").next = b);
    rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(rig.clock.instant(b)), channel: 0 });
    assert_eq!(book_both_at(&mut rig, b + 1), [5, 6], "both refused at receipt: {:?}", lock(&rig.core.rec).rejected);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled");
}

#[test]
fn e2a_80_an_abandon_at_the_repeat_s_own_start_keeps_today_s_answers() {
    // Rule 3, c == r: R (timed 9 ms ahead) is abandoned at its own start, its first send taking nothing (`StalledSend(0)`), so nothing of R
    // reached the device: its chain goes and the Stop region [r, r + 5) stays. A finite burst before r is accepted (not refused as handed),
    // and a SendAsap booking inside [r, r + 5) is moved past it, not refused.
    let mut rig = switch_rig_with(vec![o::ramp(17, 1_000), o::ramp(18, 1_000), o::ramp(19, 100)], vec![FakeFault::StalledSend(0)]);
    let r = 9_000;
    book_repeat(&mut rig, 1, r, 0);
    turn(&mut rig);
    assert!(rig.device.calls().iter().any(|call| call.starts_with("tx_send n=0 stalled")), "R's first send took nothing: {:?}", rig.device.calls());
    assert_eq!(rig.tx.open_next(), None, "R is abandoned");
    rejected(&rig.events);
    book(&mut rig, 2, 1, 3_000);
    book_with(&mut rig.control, &rig.core, 3, 0, rig.clock, r + 1, &rig.waves[2], false, LatePolicy::SendAsapAndFlag, BTreeMap::new());
    assert!(refused_ids(&rig.events).is_empty(), "both accepted: {:?}", lock(&rig.core.rec).rejected);
    run(&mut rig, 0, 20);
    assert_eq!(rig.device.transmitted(rig.clock.instant(3_000), 1_000), o::ramp(18, 1_000).into_iter().map(Some).collect::<Vec<_>>(), "{:?}", rig.device.calls());
    let moved: Vec<i64> = rows(&rig).iter().filter(|row| row["samples"] == 100).map(|row| row["target"]["ticks"].as_i64().unwrap()).collect();
    assert_eq!(moved, [r + STOP_FRONT], "the SendAsap booking is moved to the region's front: {:?}", rows(&rig));
}

#[test]
fn e2a_85_provider_stop_cancels_an_unsent_open_booking_with_a_note() {
    // #85 (RM-16, UR-26): H (100 samples at 10 000) is open with its first send waiting for 32 samples of window space (the window ends at
    // 10 016). `Provider::stop` cancels it with the Run end's note; no sample and no burst record.
    let mut rig = switch_rig(vec![vec![OLD; 100]]);
    book(&mut rig, 1, 0, 10_000);
    at(&rig, 16 * 200);
    turn(&mut rig);
    assert_eq!((rig.tx.open_next(), rig.tx.open_first()), (Some(10_000), Some(true)), "H is open with nothing issued");
    let device = rig.device.clone();
    let (core, _) = shutdown(rig);
    assert_eq!(notes(&core), ["UR-26: cancelled by Provider::stop"], "{:?}", lock(&core.rec).rejected);
    assert!(sends(&device).is_empty() && lock(&core.rec).bursts.is_empty(), "{:?}", device.calls());
}

#[test]
fn e2a_f4_a_late_underflow_with_nothing_issued_sends_nothing_and_keeps_the_booking() {
    // A5, F4 (UR-22): an open first send waits for window space with no device burst open (`ur_23_a_first_send_waits_for_32_window_samples`).
    // A late underflow report finds no device burst to end: no send (today a two-zero end-of-burst with no start), the booking stays open
    // with no note, and it plays whole once its window opens.
    let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_100_000), 2_000);
    let burst = txp_burst(&clock, 3_000, 100, false, LatePolicy::DropAndFlag);
    let samples = burst.samples[0].clone();
    txp_give(&mut tx, burst);
    device_at(&core, &time, &device, txp_ticks(928));
    txp_drain(&mut tx);
    assert_eq!((tx.open_next(), tx.open_first()), (Some(3_000), Some(true)));
    tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(clock.instant(900)), channel: 0 });
    assert!(sends(&device).is_empty(), "nothing is sent: {:?}", device.calls());
    assert_eq!((tx.open_next(), tx.open_first()), (Some(3_000), Some(true)), "the booking is still open");
    assert!(lock(&core.rec).rejected.iter().all(|row| row["kind"] != "tx_burst"), "{:?}", lock(&core.rec).rejected);
    device_at(&core, &time, &device, txp_ticks(1_000));
    txp_drain(&mut tx);
    assert_eq!(device.transmitted(clock.instant(3_000), 100), samples.iter().copied().map(Some).collect::<Vec<_>>(), "{:?}", device.calls());
}

// ------------------------------------------------------------------------------------------------------------ E2a-3

/// The notes of switches cancelled with the repeat they replace (UR-22).
fn switch_notes(core: &Core) -> usize {
    lock(&core.rec).rejected.iter().filter(|row| row["reason"] == "UR-22: the repeat switch was cancelled with the repeat it replaces").count()
}

#[test]
fn e2a_f1_an_underflow_cancels_a_switch_whose_payload_is_unread() {
    // F1, A1 (RM-37, UR-22): R plays; a switch S to NEW (boundary 15 000) is received and not yet read when an underflow abandons R at
    // its cursor, before the boundary. The abandon cancels S and its unread payload with it: the row reads cancelled, one note says so,
    // and no sample of NEW sounds (a payload read after the abandon would start a burst of its own at 15 000). The record holds nothing
    // of S, so a burst over [15 000, 16 000) is accepted and plays.
    let other: Vec<Iq> = vec![[0.5, 0.5]; 200];
    let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000], other.clone()]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 3);
    book_switch(&mut rig, 2, 3_000, 1);
    let cursor = rig.tx.open_next().expect("R is handed");
    assert!(cursor < 15_000, "the cursor {cursor} is before the boundary");
    rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(rig.clock.instant(cursor)), channel: 0 });
    rig.tx.read_list();
    assert!(!held_keys(&rig.tx).contains(&15_000), "S's payload went with S: {:?}", held_keys(&rig.tx));
    assert_eq!(switch_notes(&rig.core), 1, "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled");
    book(&mut rig, 3, 2, 15_500);
    assert!(rejected(&rig.events).is_empty(), "nothing refused: {:?}", lock(&rig.core.rec).rejected);
    run(&mut rig, 3, 30);
    assert!(!rig.device.transmitted(rig.clock.instant(15_000), 5_000).contains(&Some(NEW)), "{:?}", rig.device.calls());
    assert_eq!(rig.device.transmitted(rig.clock.instant(15_500), 200), other.into_iter().map(Some).collect::<Vec<_>>(), "{:?}", rig.device.calls());
    assert_eq!(switch_notes(&rig.core), 1, "{:?}", lock(&rig.core.rec).rejected);
}

#[test]
fn e2a_f1_a_failed_send_with_a_switch_received_in_flight_cancels_it() {
    // F1, A1 (RM-37, UR-22): R plays and its send [9 500, 12 000) is in flight (issued: R has started, so the switch is accepted) when
    // uhd-control receives a switch S (boundary 15 000). The send fails, so the abandon at its cursor 12 000 cancels S and its unread
    // payload: the row reads cancelled, one note, and no sample of S's waveform sounds.
    let mut rig = switch_rig_with(vec![o::ramp(17, 1_000), o::ramp(18, 1_000)], vec![FakeFault::FailSend(3)]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 1);
    at(&rig, 5 * MS / 2);
    let SwitchRig { core, device, time, events, control, mut tx, clock, waves, _frozen } = rig;
    let control = Arc::new(Mutex::new(Some(control)));
    let fired = Arc::new(AtomicBool::new(false));
    let (hook_core, hook_control, wave) = (Arc::downgrade(&core), control.clone(), waves[1].clone());
    crate::provider::test_support::probe(&core, "tx_send_planned", move |_| {
        let Some(hook_core) = hook_core.upgrade() else { return };
        if !fired.swap(true, Ordering::SeqCst) {
            let mut control = hook_control.lock().unwrap();
            let control = control.as_mut().expect("the rig's control");
            book_with(control, &hook_core, 2, 0, clock, 3_000, &wave, true, LatePolicy::SendAsapAndFlag, switch_mark());
        }
    });
    assert!(tx.step());
    let control = control.lock().unwrap().take().expect("the rig's control");
    let mut rig = SwitchRig { core, device, time, events, control, tx, clock, waves, _frozen };
    let failed = rig.device.calls().into_iter().filter(|call| call.starts_with("tx_send 0 ")).nth(3).expect("the fourth send");
    assert!(failed.contains(&format!("at={}", rig.clock.instant(9_500))), "the failing send is the intended one: {:?}", rig.device.calls());
    assert!(!refused_ids(&rig.events).contains(&2), "S was accepted: {:?}", lock(&rig.core.rec).rejected);
    assert_eq!(switch_row(&rig.core, 0)["boundary"], 15_000);
    run(&mut rig, 3, 30);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled");
    assert_eq!(switch_notes(&rig.core), 1, "{:?}", lock(&rig.core.rec).rejected);
    assert!(!rig.device.transmitted(rig.clock.instant(2_000), 28_000).iter().flatten().any(|s| o::ramp(18, 1_000).contains(s)), "{:?}", rig.device.calls());
}

#[test]
fn e2a_84_a_second_switch_on_a_switch_the_host_has_not_issued_is_refused() {
    // D3 (UR-38): a switch is the playing repeat from receipt and starts only when its own payload is issued. S1 (boundary 15 000) is
    // received at 3 ms; device time reaches 16 ms before the owner steps again, so the owner has issued none of it and the record still
    // holds S1 pending (C2: it is applied only once the sample at b was sent): a second switch S2 is refused at receipt, naming S2, and
    // nothing changes. Once S1's payload is issued by normal running, a switch on it is accepted and names S1's period as the old one.
    let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 500], vec![NEW; 300]]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 3);
    book_switch(&mut rig, 2, 3_000, 1);
    drain_tx(&mut rig.tx);
    at(&rig, 16 * MS);
    rejected(&rig.events);
    book_switch(&mut rig, 3, 16_000, 2);
    assert_eq!(refusals(&rig.events), ["UR-21: a repeat switch needs a playing repeat and no pending switch"], "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(lock(&rig.core.rec).switches.len(), 1, "a refused switch writes no row");
    assert_eq!(rig.tx.with_book(|book| book.playing.map(|playing| (playing.id, playing.started))), Some((ActionId(2), false)));

    let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 500], vec![NEW; 300]]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 3);
    book_switch(&mut rig, 2, 3_000, 1);
    run(&mut rig, 3, 16);
    assert_eq!(rig.tx.with_book(|book| book.playing.map(|playing| (playing.id, playing.started))), Some((ActionId(2), true)), "S1's payload is issued");
    book_switch(&mut rig, 3, 16_000, 2);
    assert!(refusals(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!((switch_row(&rig.core, 1)["old_period"].as_i64(), switch_row(&rig.core, 1)["predecessor"].as_u64()), (Some(500), Some(2)));
}

#[test]
fn e2a_b1_a_switch_behind_an_unstarted_finite_burst_is_refused() {
    // F5, UR-38 (KL-41): F (500 samples at 2 000) and R (a repeat at 3 000, behind F's front) are booked; device time passes R's start
    // before the owner issues anything. R plays in the record but has not started on the device, so a switch on it is refused at receipt
    // with no row (MockRadio accepts it: `mr_42_a_switch_in_a_step_that_jumped_over_the_repeat_s_start_is_accepted`).
    let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000], vec![[0.5, 0.5]; 500]]);
    book(&mut rig, 1, 2, 2_000);
    book_repeat(&mut rig, 2, 3_000, 0);
    at(&rig, 7 * MS / 2);
    book_switch(&mut rig, 3, 3_500, 1);
    assert_eq!(refusals(&rig.events), ["UR-38: the repeat the switch replaces has not started on the device"], "{:?}", lock(&rig.core.rec).rejected);
    assert!(lock(&rig.core.rec).switches.is_empty());
    assert_eq!(rig.tx.with_book(|book| book.playing.map(|playing| playing.id)), Some(ActionId(2)), "R is still the playing repeat");
}

/// An untimed switch to `wave` stamped `k` (RS-19: no requested instant).
fn book_untimed_switch(rig: &mut SwitchRig, id: u64, k: i64, wave: usize) {
    let node = rig.core.description.stream_node(&rig.core.id, Dir::Tx, 0);
    rig.control.book(Dispatched::new_untimed(
        ActionId(id),
        Action::TxBurst {
            target: crate::provider::test_support::target_of(&node),
            waveform: rig.waves[wave].clone(),
            repeat: true,
            at: AbsoluteDeadline::new(TimePoint::new(rig.clock.domain, k)),
            requested_at: None,
            late_policy: LatePolicy::SendAsapAndFlag,
            metadata: switch_mark(),
        },
        Some(node),
    ));
}

#[test]
fn e2a_86_a_timed_switch_pushed_past_its_boundary_records_its_at() {
    // #86 (RM-37, SC-29a): R (period 1 000 from 2 000) and a switch received at 3 ms, whose bound is 15 000. A timed switch for 3 000 is
    // pushed to 15 000 and its new repeat's record carries 3 000 as its requested target; one for 16 000 is on its own grid point and
    // carries none; an untimed one carries none. None is late, and the row has no `moved_to` (M3 is the Mock's twin).
    let p = params();
    for (case, at_k, untimed) in [("pushed", 3_000, false), ("on the rule", 16_000, false), ("untimed", 3_000, true)] {
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        if untimed {
            book_untimed_switch(&mut rig, 2, at_k, 1);
        } else {
            book_switch(&mut rig, 2, at_k, 1);
        }
        run(&mut rig, 3, 20);
        stop(&mut rig);
        run(&mut rig, 21, 35);
        let b = o::switch_boundary(&p, 2_000, 1_000, 3 * MS, Some(at_k)).unwrap();
        let row = switch_row(&rig.core, 0);
        assert_eq!((row["outcome"].as_str(), row["boundary"].as_i64(), row.get("moved_to")), (Some("applied"), Some(b), None), "{case}: {row}");
        let successor: Vec<serde_json::Value> = rows(&rig).into_iter().filter(|row| row["target"]["ticks"] == b).collect();
        assert_eq!(successor.len(), 1, "{case}: {:?}", rows(&rig));
        let want = o::switch_requested_target(&p, 2_000, 1_000, 3 * MS, at_k, untimed);
        assert_eq!((successor[0]["requested_target"]["ticks"].as_i64(), successor[0]["late_by"].is_null()), (want, true), "{case}: {:?}", successor[0]);
        assert_eq!(want.is_some(), case == "pushed", "{case}");
    }
}

#[test]
fn e2a_88_a_switch_after_a_repeat_moved_at_dispatch_takes_a_boundary_on_the_moved_grid() {
    // #88 (RM-14, RM-37): R (period 700, SendAsapAndFlag) is booked for 3 000; the owner dispatches it at 5 ms, late, and moves it to m.
    // Once R sounds, a switch S takes its boundary on R's moved grid m + 700 j, not on the grid R was booked on (3 000 + 700 j, off phase
    // by (m - 3 000) mod 700), and R plays whole periods from m up to it.
    let mut rig = switch_rig(vec![o::ramp(21, 700), o::ramp(22, 400)]);
    book_with(&mut rig.control, &rig.core, 1, 0, rig.clock, 3_000, &rig.waves[0], true, LatePolicy::SendAsapAndFlag, BTreeMap::new());
    run(&mut rig, 5, 7);
    let first = rig.device.calls().into_iter().find_map(|call| call.strip_prefix("tx_send 0 ").and_then(|rest| rest.split_whitespace().find_map(|w| w.strip_prefix("at=")).map(str::to_owned)));
    let m = rig.clock.at_or_after(first.expect("R is sent").parse().unwrap());
    assert!((m - 3_000) % 700 != 0, "the moved grid is off phase: m = {m}");
    book_switch(&mut rig, 2, 7_000, 1);
    assert!(refusals(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
    let b = switch_row(&rig.core, 0)["boundary"].as_i64().unwrap();
    assert_eq!((b - m) % 700, 0, "b = {b} is on the moved grid from {m}");
    run(&mut rig, 7, 25);
    let seen = rig.device.transmitted(rig.clock.instant(m - 10), (b + 1_200 - m + 10) as usize);
    let slots = o::Slots { segs: vec![o::Seg::repeating(21, m, 700, b), o::Seg::repeating(22, b, 400, i64::MAX)], cut: None };
    slots.check(m - 10, b + 1_200, &|k| seen[(k - m + 10) as usize]).unwrap_or_else(|error| panic!("{error}: {:?}", rig.device.calls()));
}

#[test]
fn e2a_s11_a_stop_after_a_switch_whose_boundary_is_before_the_bound_cancels_it_and_its_payload() {
    // RM-37, RM-16 (note 49 §5.4.5, KL-42): a switch (boundary 15 000) and then a Stop at 5.6 ms (bound 15 605) are received before the
    // owner reads either. The owner reads the payload, then the Stop, which cancels the payload with one note: the device burst closes
    // before the boundary, no sample of the new waveform was sent, and the row is `cancelled` (C2).
    let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 3);
    book_switch(&mut rig, 2, 3_000, 1);
    at(&rig, 5 * MS + 3 * MS / 5);
    stop_receipt(&mut rig);
    rig.tx.read_list();
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled");
    assert_eq!(notes(&rig.core), ["UR-26: cancelled by Stop"], "{:?}", lock(&rig.core.rec).rejected);
    assert!(rejection_events(&rig.events).is_empty());
    run(&mut rig, 6, 30);
    assert!(last_send_eob(&rig));
    assert!(!rig.device.transmitted(rig.clock.instant(0), 30_000).contains(&Some(NEW)), "{:?}", rig.device.calls());
}

// ------------------------------------------------------------------------------------------------------------ E2a-4

/// #93 (UR-22): A (500 samples at 10 000) ends with its own end-of-burst; B (20 000 samples at 11 000) is a new device burst whose first
/// sends are issued by 7 ms, so it is open on the device when a `code` report whose tick is sample `k`'s is read. Returns the rig after the
/// report.
fn late_report_after_a_closed_burst(code: crate::device::TxCode, k: i64) -> SwitchRig {
    let mut rig = switch_rig(vec![o::ramp(31, 500), o::ramp(32, 20_000)]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 11_000);
    run(&mut rig, 0, 7);
    let starts: Vec<String> = rig.device.calls().into_iter().filter(|call| call.starts_with("tx_send ") && call.contains(" sob=true")).collect();
    let b_start = format!("at={} sob=true", rig.clock.instant(11_000));
    assert!(starts.len() == 2 && starts[1].contains(&b_start), "A and then B started: {:?}", rig.device.calls());
    assert_eq!(closing_sends(&rig.device).len(), 1, "A closed, B open: {:?}", rig.device.calls());
    assert_eq!(rig.tx.open_next().map(|next| next > 11_000), Some(true), "B is handed and open: {:?}", rig.device.calls());
    rig.tx.report(crate::device::TxReport { code, tick: Some(rig.clock.instant(k)), channel: 0 });
    rig
}

fn underflow_events(events: &EventCollector) -> usize {
    events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TX_UNDERFLOW).count()
}

fn underflow_notes(core: &Core) -> usize {
    lock(&core.rec).rejected.iter().filter(|row| row["reason"] == "UR-22: the device burst underflowed; burst not resumed").count()
}

#[test]
fn e2a_93_a_late_underflow_of_the_burst_before_does_not_cut_the_next_device_burst() {
    // #93 (UR-22, note 39 §6.6): A starved before its end, and its report is read only once B's first sends are issued. The report's tick
    // (inside A) is before B's first tick, so it belongs to A's device burst: the event is emitted, and B plays whole with no note and no
    // two-sample close before its own end.
    let mut rig = late_report_after_a_closed_burst(crate::device::TxCode::Underflow, 10_400);
    assert_eq!(underflow_events(&rig.events), 1);
    run(&mut rig, 7, 40);
    assert_eq!(underflow_notes(&rig.core), 0, "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(closing_sends(&rig.device).len(), 2, "A's close and B's own: {:?}", rig.device.calls());
    assert_eq!(rig.device.transmitted(rig.clock.instant(11_000), 20_000), o::ramp(32, 20_000).into_iter().map(Some).collect::<Vec<_>>(), "{:?}", rig.device.calls());
}

#[test]
fn e2a_93_an_underflow_at_the_open_burst_s_first_tick_abandons_it() {
    // #93's twin (UR-22): a report whose tick is B's first sample's is B's own, and abandons B at its cursor as before.
    let mut rig = late_report_after_a_closed_burst(crate::device::TxCode::Underflow, 11_000);
    assert_eq!(underflow_events(&rig.events), 1);
    assert_eq!(underflow_notes(&rig.core), 1, "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(rig.tx.open_next(), None, "B is abandoned");
    run(&mut rig, 7, 40);
    assert_eq!(closing_sends(&rig.device).len(), 2, "A's close and the abandon's: {:?}", rig.device.calls());
    assert!(rig.device.transmitted(rig.clock.instant(11_000), 20_000).last() == Some(&None), "B's end never sounds: {:?}", rig.device.calls());
}

#[test]
fn e2a_93_a_sequence_error_before_the_open_burst_s_first_tick_still_abandons_it() {
    // #93 m1 (UR-22): only an underflow's tick is compared. A sequence error may be reported when a packet reaches the radio, up to the
    // in-flight window ahead of its play time, so one whose tick is before B's first sample still abandons B.
    let mut rig = late_report_after_a_closed_burst(crate::device::TxCode::SeqError, 10_400);
    assert_eq!(underflow_events(&rig.events), 1);
    assert_eq!(underflow_notes(&rig.core), 1, "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(rig.tx.open_next(), None, "B is abandoned");
    run(&mut rig, 7, 40);
    assert!(rig.device.transmitted(rig.clock.instant(11_000), 20_000).last() == Some(&None), "B's end never sounds: {:?}", rig.device.calls());
}

/// The refusals reported since the last call, as `(Action id, reason)`.
fn refusals_by_id(events: &EventCollector) -> Vec<(u64, String)> {
    rejected(events).iter().map(|event| (event.payload["action"].as_u64().expect("a refusal names its Action"), event.payload["reason"].as_str().unwrap().to_owned())).collect()
}

#[test]
fn ur_21_a_burst_time_that_does_not_fit_the_clock_is_refused_with_its_id_and_the_next_booking_is_served() {
    // RM-22, UR-21 (#95): a burst at 2^62 samples (its instant passes i64::MAX), a repeat there, a target whose lead overflows and a
    // burst whose end passes i64::MAX are each refused at receipt with their ids and book nothing; a finite burst received after them is accepted and plays.
    let mut rig = switch_rig(vec![o::ramp(5, 200)]);
    book(&mut rig, 1, 0, 1 << 62);
    book_with(&mut rig.control, &rig.core, 2, 0, rig.clock, 1 << 62, &rig.waves[0], true, LatePolicy::SendAsapAndFlag, BTreeMap::new());
    book(&mut rig, 3, 0, i64::MIN);
    // The last sample whose instant fits: a burst of 200 starts there and its end does not.
    let last = (i64::MAX - rig.clock.origin) / rig.clock.n;
    assert!(rig.clock.try_instant(last).is_some() && rig.clock.try_instant(last + 1).is_none());
    book(&mut rig, 4, 0, last);
    let refused = refusals_by_id(&rig.events);
    assert_eq!(refused.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [1, 2, 3, 4], "{refused:?}");
    for index in [0, 1, 3] {
        assert_eq!(refused[index].1, "UR-21: the burst time overflows the transmit clock", "{refused:?}");
    }
    assert!(refused[2].1.starts_with("UR-21: "), "{refused:?}");
    book(&mut rig, 5, 0, 10_000);
    assert!(refusals_by_id(&rig.events).is_empty(), "the next booking is accepted: {:?}", lock(&rig.core.rec).rejected);
    run(&mut rig, 0, 20);
    assert_eq!(rig.device.transmitted(rig.clock.instant(10_000), 200), o::ramp(5, 200).into_iter().map(Some).collect::<Vec<_>>(), "{:?}", rig.device.calls());
    assert_eq!(rows(&rig).len(), 1, "only the served burst is recorded: {:?}", rows(&rig));
}

#[test]
fn ur_38_a_switch_at_the_end_of_the_range_is_refused_with_its_id_and_the_next_one_is_taken() {
    // RM-37, UR-38 (#94): the repeat plays; a switch at i64::MAX is refused at receipt with its id and no `switches` row, the repeat
    // plays on, and a switch received next is taken at its boundary.
    let mut rig = playing();
    book_switch(&mut rig, 2, i64::MAX, 1);
    assert_eq!(refusals_by_id(&rig.events), [(2, "UR-21: the burst time overflows the transmit clock".to_owned())]);
    assert!(lock(&rig.core.rec).switches.is_empty(), "a refused switch leaves no row");
    book_switch(&mut rig, 3, 3_000, 1);
    assert!(refusals_by_id(&rig.events).is_empty(), "the next switch is accepted: {:?}", lock(&rig.core.rec).rejected);
    assert_eq!(lock(&rig.core.rec).switches.len(), 1);
    assert_eq!(switch_row(&rig.core, 0)["action_id"], 3);
}

#[test]
fn ur_26_a_stop_whose_bound_does_not_fit_the_clock_is_refused_with_its_id_and_changes_nothing() {
    // RM-16, UR-26 (#95): a Stop whose bound's instant passes i64::MAX is refused at receipt with its id: no Stop is booked on the line and
    // none reaches the owner. The rig's clock cannot hold a device time past i64::MAX / 5 (its host rescale overflows), so the horizon
    // `at + tx_in_flight` cannot pass i64::MAX here; the stream's clock is replaced by one of 2^61 root ticks a sample (the registry caps a
    // real one lower), whose bound's instant does. uhd-control goes on: a second Stop is refused alike.
    let mut rig = switch_rig(vec![o::ramp(5, 200)]);
    lock(&rig.core.streams).stream_mut(stream_key(Dir::Tx, 0)).clock = Some(Clock { n: 1 << 61, ..rig.clock });
    at(&rig, 3 * MS);
    stop_receipt(&mut rig);
    stop_receipt(&mut rig);
    let reason = "UR-26: the Stop's bound overflows the transmit clock".to_owned();
    assert_eq!(refusals_by_id(&rig.events), [(9, reason.clone()), (9, reason)]);
    assert!(lock(&rig.core.streams).line(stream_key(Dir::Tx, 0)).unwrap().items.iter().all(|item| item.kind != Kind::Stop), "no Stop is booked on the line");
    assert!(lock(&rig.core.streams).stream_mut(stream_key(Dir::Tx, 0)).book.list.read().is_none(), "the Stop reached no one");
    // `Provider::stop` is no Action, so it is not refused: its bound saturates, where an overflow would panic.
    lock(&rig.core.streams).end(&rig.core, 3 * MS, false, false);
    assert!(lock(&rig.core.streams).stream_mut(stream_key(Dir::Tx, 0)).book.list.read().is_some(), "the Provider's stop halts the stream");
}

#[test]
fn ur_38_a_switch_whose_boundary_does_not_fit_a_clock_of_one_tick_a_sample_is_refused_by_the_book() {
    // RM-37, UR-38 (#94): at 200 MS/s one sample is one root tick, so a switch at i64::MAX passes the instant checks (its instant fits). The
    // repeat from 2 000 000 with period 1 000 has no grid point at or after i64::MAX that fits, so the book refuses it with its id,
    // writes no row, and the repeat plays on.
    let mut rig = switch_rig_on(vec![vec![OLD; 1_000], vec![NEW; 200]], Vec::new(), 1);
    book_repeat(&mut rig, 1, 2_000_000, 0);
    run(&mut rig, 0, 12);
    assert!(rig.clock.try_instant(i64::MAX).is_some(), "the instant fits, so the instant checks do not refuse");
    book_switch(&mut rig, 2, i64::MAX, 1);
    assert_eq!(refusals_by_id(&rig.events), [(2, "UR-21: the burst time overflows the transmit clock".to_owned())]);
    assert!(lock(&rig.core.rec).switches.is_empty(), "a refused switch leaves no row");
    assert_eq!(lock(&rig.core.streams).stream(stream_key(Dir::Tx, 0)).book.playing.map(|playing| playing.id), Some(ActionId(1)), "the repeat plays on");
}
