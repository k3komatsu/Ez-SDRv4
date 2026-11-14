//! E1a (design note 49 §5.4.4 item 6; plan `2026-11-10-e1/test-plan.md`): bookings and Stops in one receipt-ordered list, against the UHD
//! owner on a single-threaded rig whose clock the test thread holds. Receipt and owner turn are called from the test thread in the order written; device time
//! moves only through `device_at`, so no verdict depends on when a thread runs. Probes run on the test thread with no lock held.
//!
//! Units: one sample is 1 us (the rig's 1 MS/s), 200 root ticks; `MS` is a millisecond in root ticks.

use super::*;
use crate::provider::tx::oracle as o;
use ezsdr_radio::staging::Read;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) const MS: i64 = 200_000;

pub(super) fn params() -> o::Params {
    o::Params { clock: o::Clock { origin: 0, n: 200 }, root_hz: 200_000_000, window: 10 * MS, lead: 2 * MS, allowance: 3 * MS }
}

pub(super) fn at(rig: &SwitchRig, ticks: i64) {
    device_at(&rig.core, &rig.time, &rig.device, ticks);
}

/// A finite booking of `rig.waves[wave]` at sample `k`.
pub(super) fn book(rig: &mut SwitchRig, id: u64, wave: usize, k: i64) {
    book_with(&mut rig.control, &rig.core, id, 0, rig.clock, k, &rig.waves[wave], false, LatePolicy::DropAndFlag, BTreeMap::new());
}

/// uhd-control's Stop of transmit stream 0, received and not read.
pub(super) fn stop_receipt(rig: &mut SwitchRig) {
    rig.control.book(Dispatched::new(ActionId(9), Action::Stop { target: Some(crate::provider::test_support::target_of(&rig.core.tx_id)) }, Some(rig.core.tx_id.clone())));
}

/// The `rejected` notes of cancelled bookings, in order.
pub(super) fn notes(core: &Core) -> Vec<String> {
    lock(&core.rec).rejected.iter().filter(|row| row["kind"] == "tx_burst").filter_map(|row| row["reason"].as_str().filter(|reason| reason.starts_with("UR-26")).map(str::to_owned)).collect()
}

pub(super) fn rejection_events(events: &EventCollector) -> Vec<ezsdr_kernel::event::Event> {
    rejected(events)
}

/// The bookings left unread on stream 0's list (halts skipped).
fn booked_entries(core: &Core) -> usize {
    let mut streams = lock(&core.streams);
    std::iter::from_fn(|| streams.stream_mut(stream_key(Dir::Tx, 0)).book.list.read()).filter(|read| matches!(read, Read::Booking(_))).count()
}

/// The end of what was handed to the device: the first timed send's start plus every sample sent.
pub(super) fn handed_end(rig: &SwitchRig) -> i64 {
    let calls = rig.device.calls();
    let sends: Vec<&String> = calls.iter().filter(|call| call.starts_with("tx_send ")).collect();
    let field = |call: &str, name: &str| call.split_whitespace().find_map(|word| word.strip_prefix(name)).map(str::to_owned);
    let first = sends.iter().find_map(|call| field(call, "at=").filter(|at| at != "-")).expect("a timed send");
    rig.clock.at_or_after(first.parse().unwrap()) + sends.iter().map(|call| field(call, "n=").unwrap().parse::<i64>().unwrap()).sum::<i64>()
}

pub(super) fn last_send_eob(rig: &SwitchRig) -> bool {
    sends(&rig.device).last().is_some_and(|send| send.2)
}

pub(super) fn rows(rig: &SwitchRig) -> Vec<serde_json::Value> {
    lock(&rig.core.rec).bursts.clone()
}

/// A rig on a clock whose origin is `origin` ticks after the start: `booking_control_at` registers it.
fn rig_at(origin: i64, waves: Vec<Vec<Iq>>) -> SwitchRig {
    let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
    let mut inputs = BTreeMap::new();
    let mut refs = Vec::new();
    for (i, samples) in waves.iter().enumerate() {
        let (wave, bytes) = waveform_input(&format!("e1_{i}"), samples);
        inputs.insert(wave.hash.clone(), Arc::<[u8]>::from(bytes));
        refs.push(wave);
    }
    let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
    let guard = crate::provider::test_support::clock_guard(&device.host());
    device_at(&core, &time, &device, 0);
    device.tx_open(0, &[0]).unwrap();
    device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
    let (control, cmds) = booking_control_at(&core, origin);
    let clock = tx_clock(&core);
    let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
    SwitchRig { core, device, time, events, control, tx, clock, waves: refs, _guard: guard }
}

/// `Tx::shutdown`, its 100 ms acknowledgement window passing on the clock the rig's guard holds, which drops with it; returns the Core and the events.
pub(super) fn shutdown(rig: SwitchRig) -> (Arc<Core>, Arc<EventCollector>) {
    let SwitchRig { core, events, mut tx, _guard, .. } = rig;
    tx.shutdown();
    (core, events)
}

fn q1_slots_check(rig: &SwitchRig, slots: &o::Slots, from: i64, to: i64) -> Option<i64> {
    let seen = rig.device.transmitted(rig.clock.instant(from), (to - from) as usize);
    slots.check(from, to, &|k| seen[(k - from) as usize]).unwrap_or_else(|error| panic!("{error}: {:?}", rig.device.calls()))
}

#[test]
fn e1_q1_a_stop_is_read_between_the_booking_before_it_and_the_one_after_it() {
    // A (12 000 samples from 10 000) is cut by the Stop at 11 ms; H3 (40 000) was booked before the Stop; R (30 000) after it. All three
    // receipts are read together, in receipt order: H3 gets the Stop's note and R plays whole.
    let mut rig = switch_rig(vec![o::ramp(5, 12_000), o::ramp(6, 200), o::ramp(3, 300)]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 2, 40_000);
    run(&mut rig, 0, 10);
    at(&rig, 11 * MS);
    for _ in 0..8 {
        drain_tx(&mut rig.tx);
        if !rig.tx.step() {
            break;
        }
    }
    let (t_k, bound) = params().stop_bound(11 * MS);
    stop_receipt(&mut rig);
    book(&mut rig, 3, 1, 30_000);
    rig.tx.read_list();
    run(&mut rig, 12, 60);
    let slots = o::Slots { segs: vec![o::Seg::once(5, 10_000, 12_000).cuttable(), o::Seg::once(6, 30_000, 200)], cut: Some((t_k, bound)) };
    let cut = q1_slots_check(&rig, &slots, 9_990, 30_300).expect("A is cut");
    assert!(cut < 22_000);
    assert_eq!(notes(&rig.core), ["UR-26: cancelled by Stop"], "H3: {:?}", lock(&rig.core.rec).rejected);
    assert!(rejection_events(&rig.events).is_empty());
    let rows = rows(&rig);
    assert_eq!((rows[0]["samples"].as_i64(), rows[0]["end"].as_str()), (Some(cut - 10_000), Some("stop")), "{rows:?}");
    assert_eq!((rows[1]["target"]["ticks"].as_i64(), rows[1]["samples"].as_i64()), (Some(30_000), Some(200)));
}

/// A repeat playing on a rig, device time 3 ms, the owner's list read.
pub(super) fn playing() -> SwitchRig {
    let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000], vec![NEW; 200]]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 3);
    rig
}

#[test]
fn e1_q2_the_step_reads_a_stop_it_has_not_read_before_it_hands() {
    let mut rig = playing();
    let t = rig.core.now();
    crate::provider::test_support::end_streams(&rig.core, t, false, false);
    at(&rig, t + 2 * MS);
    assert!(rig.tx.step(), "the step reads the halt first");
    let (_, bound) = params().stop_bound(t);
    assert!(handed_end(&rig) <= bound, "nothing past the bound {bound} is handed: {:?}", rig.device.calls());
    assert!(last_send_eob(&rig), "the closing send has EOB: {:?}", rig.device.calls());
    assert_eq!(rows(&rig)[0]["end"], "stop");
}

#[test]
fn e1_q2b_now_is_read_under_the_lock_that_checked_the_list() {
    let mut rig = playing();
    let t = rig.core.now();
    let fired = Arc::new(AtomicBool::new(false));
    // The probe holds the Core weakly: `core.probes` holding it would keep the rig from ever dropping (UR-33's drop check).
    let (core, time, device) = (Arc::downgrade(&rig.core), rig.time.clone(), rig.device.clone());
    crate::provider::test_support::probe(&rig.core, "tx_step_guarded", move |_| {
        let Some(core) = core.upgrade() else { return };
        if !fired.swap(true, Ordering::SeqCst) {
            crate::provider::test_support::end_streams(&core, t, false, false);
            device_at(&core, &time, &device, t + 2 * MS);
        }
    });
    rig.tx.step();
    let (_, bound) = params().stop_bound(t);
    assert!(handed_end(&rig) <= bound, "the turn that raced the Stop hands nothing past {bound}: {:?}", rig.device.calls());
    rig.tx.read_list();
    assert!(last_send_eob(&rig), "{:?}", rig.device.calls());
    assert_eq!(rows(&rig)[0]["end"], "stop");
}

#[test]
fn e1_q3_a_pre_origin_provider_stop_cancels_a_booking_with_its_own_note() {
    for order in ["nothing read before the origin", "the Stop and the booking read, then the end", "shutdown before the origin turn"] {
        let origin = 1_000 * MS;
        let mut rig = rig_at(origin, vec![vec![OLD; 100]]);
        stop_receipt(&mut rig);
        book(&mut rig, 1, 0, 20_000);
        if order.starts_with("the Stop") {
            rig.tx.read_list();
        }
        let t2 = rig.core.now() + 5 * MS;
        crate::provider::test_support::end_streams(&rig.core, t2, false, false);
        at(&rig, origin);
        let (core, events) = if order.starts_with("shutdown") {
            shutdown(rig)
        } else {
            rig.tx.pass();
            rig.tx.pass();
            assert!(rig.tx.clock.is_none() && rig.tx.cut.is_none(), "{order}: the clock ends at its origin");
            assert!(sends(&rig.device).is_empty(), "{order}: nothing was sent");
            (rig.core.clone(), rig.events.clone())
        };
        assert_eq!(notes(&core), ["UR-26: cancelled by Provider::stop"], "{order}: {:?}", lock(&core.rec).rejected);
        assert!(rejection_events(&events).is_empty(), "{order}");
    }
}

#[test]
fn e1_q4b_a_switch_read_after_the_stop_that_took_it_is_cancelled_with_it() {
    // The switch (boundary 15 000) is taken at receipt; the Stop received after it bounds the stream at 13 005, before the boundary, so
    // the record cancels it. The owner reads the switch's payload, then the Stop, which cancels the payload with one note: no refusal.
    let mut rig = playing();
    book_switch(&mut rig, 2, 3_000, 1);
    stop_receipt(&mut rig);
    rig.tx.read_list();
    assert!(rejection_events(&rig.events).is_empty(), "nothing is refused");
    assert_eq!(notes(&rig.core), ["UR-26: cancelled by Stop"], "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled");
    assert!(last_send_eob(&rig));
    assert_eq!(rows(&rig)[0]["end"], "stop");
    run(&mut rig, 4, 30);
    assert!(!rig.device.transmitted(rig.clock.instant(0), 30_000).contains(&Some(NEW)), "{:?}", rig.device.calls());
}

#[test]
fn e1_q5_a_booking_waiting_when_the_repeat_is_abandoned_still_plays() {
    let mut rig = playing();
    stop_receipt(&mut rig);
    book(&mut rig, 3, 2, 30_000);
    rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::SeqError, tick: None, channel: 0 });
    // #80: the abandon keeps B's chain, so a booking overlapping B is refused at receipt; R, the abandoned repeat, is no longer playing.
    rejected(&rig.events);
    book(&mut rig, 4, 2, 30_100);
    assert_eq!(rejected(&rig.events).iter().filter_map(|event| event.payload["action"].as_u64()).collect::<Vec<_>>(), [4], "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(rig.tx.with_book(|book| book.playing), None);
    rig.tx.read_list();
    run(&mut rig, 4, 40);
    assert!(notes(&rig.core).is_empty(), "B is not cancelled: {:?}", lock(&rig.core.rec).rejected);
    assert!(rig.device.transmitted(rig.clock.instant(30_000), 200).iter().all(|s| *s == Some(NEW)), "B plays: {:?}", rig.device.calls());
}

#[test]
fn e1_q6_a_stop_a_booking_and_the_run_end_all_unread() {
    let mut rig = switch_rig(vec![vec![OLD; 100]]);
    book(&mut rig, 1, 0, 20_000);
    stop_receipt(&mut rig);
    book(&mut rig, 2, 0, 30_000);
    let t = rig.core.now();
    crate::provider::test_support::end_streams(&rig.core, t, false, false);
    rig.tx.read_list();
    assert_eq!(notes(&rig.core), ["UR-26: cancelled by Stop", "UR-26: cancelled by Provider::stop"]);
    assert!(held_keys(&rig.tx).is_empty());
    let (core, events) = shutdown(rig);
    assert_eq!(notes(&core).len(), 2, "the shutdown finds nothing left");
    assert!(rejection_events(&events).is_empty());
}

#[test]
fn e1_q6b_shutdown_reads_the_list_before_it_cancels() {
    let mut rig = switch_rig(vec![vec![OLD; 100]]);
    book(&mut rig, 1, 0, 20_000);
    stop_receipt(&mut rig);
    let (core, _) = shutdown(rig);
    assert_eq!(notes(&core), ["UR-26: cancelled by Stop"]);
}

#[test]
fn e1_q7_a_stop_received_while_a_configure_refuses_its_burst() {
    let (core, device, time, mut tx, _guard, events, _to_tx) = owner_with(1, vec![FakeFault::TxApplyRefused(0, F2)]);
    let clock = tx.clock.expect("a clock");
    let stopper = Arc::downgrade(&core);
    crate::provider::test_support::probe(&core, "tx_configure_before_mark", move |_| {
        if let Some(stopper) = stopper.upgrade() {
            lock(&stopper.streams).stream_mut(stream_key(Dir::Tx, 0)).book.list.halt(ezsdr_radio::CancelReason::Stop);
        }
    });
    give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 1));
    turns(&core, &time, &device, &mut tx, 1, 30, 1);
    let refused = rejected(&events);
    assert_eq!(refused.iter().map(|e| e.payload["action"].clone()).collect::<Vec<_>>(), [serde_json::json!(1)], "{refused:?}");
    assert!(notes(&core).is_empty(), "the burst was in the owner's hand, not held: {:?}", lock(&core.rec).rejected);
    assert!(!traffic(&device, 1).iter().any(|call| call.starts_with("tx_send ")));
    assert!(held_keys(&tx).is_empty());
}

#[test]
fn e1_q8_provider_stop_closes_the_open_burst_at_the_next_turn() {
    // A (12 000 samples from 10 000) is open and X (40 000) is held when the Run ends; the owner's next pass closes A at its handed cursor
    // and X gets the note, with no Shutdown.
    let mut rig = switch_rig(vec![o::ramp(5, 12_000), o::ramp(6, 200)]);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 40_000);
    run(&mut rig, 0, 10);
    at(&rig, 11 * MS);
    for _ in 0..8 {
        drain_tx(&mut rig.tx);
        if !rig.tx.step() {
            break;
        }
    }
    let t = rig.core.now();
    crate::provider::test_support::end_streams(&rig.core, t, false, false);
    at(&rig, t + MS);
    rig.tx.pass();
    let (_, bound) = params().stop_bound(t);
    assert!(handed_end(&rig) <= bound, "{:?}", rig.device.calls());
    assert!(last_send_eob(&rig), "{:?}", rig.device.calls());
    assert_eq!(rows(&rig)[0]["end"], "stop");
    assert_eq!(notes(&rig.core), ["UR-26: cancelled by Provider::stop"]);
}

#[test]
fn e1_q12_a_booking_racing_a_provider_stop_cut_is_refused_at_receipt() {
    let mut rig = switch_rig(vec![vec![OLD; 100]]);
    let fired = Arc::new(AtomicBool::new(false));
    let core = Arc::downgrade(&rig.core);
    crate::provider::test_support::probe(&rig.core, "tx_book_cut_checked", move |_| {
        let Some(core) = core.upgrade() else { return };
        if !fired.swap(true, Ordering::SeqCst) {
            let now = core.now();
            crate::provider::test_support::end_streams(&core, now, false, false);
        }
    });
    book(&mut rig, 1, 0, 20_000);
    let events = rejection_events(&rig.events);
    assert_eq!(events.iter().map(|e| e.payload["action"].clone()).collect::<Vec<_>>(), [serde_json::json!(1)], "{events:?}");
    assert_eq!(events[0].payload["reason"], "UR-21: no transmit channel is configured");
    assert_eq!(booked_entries(&rig.core), 0);
    assert!(notes(&rig.core).is_empty());
}

#[test]
fn e1_q13_a_refused_booking_is_not_on_the_list() {
    let mut rig = switch_rig(vec![vec![OLD; 1_000]]);
    book(&mut rig, 1, 0, 20_000);
    book(&mut rig, 2, 0, 20_500);
    assert_eq!(rejection_events(&rig.events).len(), 1);
    at(&rig, 30 * MS);
    book(&mut rig, 3, 0, 5_000);
    assert_eq!(booked_entries(&rig.core), 1, "only the accepted booking is on the list");
}

#[test]
fn e1_q14_a_continuation_accepted_after_the_unread_check_keeps_the_burst_open() {
    let (core, device, time, events, clock, mut tx, _guard) = txp_rig(None, 2_000);
    txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 40, false, LatePolicy::DropAndFlag));
    device_at(&core, &time, &device, core.ticks(12_000_000));
    txp_drain(&mut tx);
    let deadline = core.join_by(clock, 20_040);
    device_at(&core, &time, &device, deadline - 1_000);
    let fired = Arc::new(AtomicBool::new(false));
    let (hook_core, hook_time, hook_device) = (Arc::downgrade(&core), time.clone(), device.clone());
    crate::provider::test_support::probe(&core, "tx_step_guarded", move |_| {
        let Some(hook_core) = hook_core.upgrade() else { return };
        if !fired.swap(true, Ordering::SeqCst) {
            device_at(&hook_core, &hook_time, &hook_device, deadline);
            let b = txp_burst(&clock, 20_040, 10, false, LatePolicy::DropAndFlag);
            txp_record(&hook_core, clock, &b);
            lock(&hook_core.streams).stream_mut(stream_key(Dir::Tx, 0)).book.list.book(b);
            device_at(&hook_core, &hook_time, &hook_device, deadline + 1);
        }
    });
    tx.step();
    assert!(sends(&device).iter().all(|send| !send.2), "no end-of-burst before B: {:?}", device.calls());
    tx.read_list();
    txp_drain(&mut tx);
    device_at(&core, &time, &device, core.ticks(19_000_000));
    txp_drain(&mut tx);
    let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{events:?}");
    assert_eq!(sends(&device).iter().filter(|send| send.1).count(), 1, "one device burst: {:?}", device.calls());
    assert_eq!(device.transmitted(clock.instant(20_000), 50), [played(20_000, 40), played(20_040, 10)].concat());
    assert_eq!(past_front(&core), 0);
}
