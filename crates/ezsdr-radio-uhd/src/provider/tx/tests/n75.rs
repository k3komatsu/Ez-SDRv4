//! #75: a confirmed, bounded recovery flush (UR-22) seen from the Provider: what the retry's longer window does to bookings, to Stop and loss at its
//! boundaries, to an ordinary Stop and to the record's receipt-side reservations. Every test is single-threaded on a manual-clock rig whose clock the test thread holds:
//! device time moves through `device_at` and through the owner's own waits (the test thread is a participant of the clock through `clock_guard`,
//! so the owner's work, a probe that books through uhd-control included, runs on it), so no verdict depends on when a thread runs.
//!
//! Two clocks, never mixed: device time is `device.time_now()`, which a recovery's waits move; the Kernel's `core.now()` is the rig's
//! `ManualTimeAuthority`, which moves only when a test moves it (`device_at`).

use super::e1::{MS, notes};
use super::e3b::{accepted, evs, receipt_runs};
use super::*;
use crate::provider::test_support::{flush_attempt_rows, probe, t3_calls};
use crate::provider::tx::oracle as o;
use ezsdr_radio::staging::Overlap;
use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The experiment payloads (timed sends) on the fake's call log from call `base` on: the flush is untimed (`at=-`).
fn timed_sends(device: &FakeDevice, base: usize) -> Vec<(usize, String)> {
    device.calls().into_iter().enumerate().filter(|(index, call)| *index >= base && call.starts_with("tx_send") && !call.contains("at=- ")).collect()
}

fn last_flush(device: &FakeDevice) -> usize {
    device.calls().iter().rposition(|call| call.starts_with("tx_send") && call.contains("at=- ")).expect("a flush was sent")
}

#[test]
fn ur_75_a_retry_longer_window_leaves_held_bookings_to_their_late_policy() {
    // A burst held 60 ms ahead of the Kernel's clock is inside the recovery's 200 ms window; the retry's 100 ms of device time carries the device past it
    // while the Kernel's clock has not moved, so the owner has not judged it late: the reopen turn sends nothing and cancels nothing. Once the Kernel's
    // clock follows the device the late policy decides, as after any recovery: a `DropAndFlag` burst is dropped, a `SendAsapAndFlag` one is moved and
    // sent after the confirmed flush, each reported once. The same rig without the retry shows the burst on time (sent at its target, no report).
    for (policy, outcome) in [(LatePolicy::DropAndFlag, "drop"), (LatePolicy::SendAsapAndFlag, "send_asap")] {
        for retried in [true, false] {
            let case = format!("{outcome}, retried {retried}");
            let faults = if retried { vec![FakeFault::DropAckOfSend(0)] } else { Vec::new() };
            let (core, device, time, mut tx, clock, events, _guard) = retry_rig_without_burst(faults, true);
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            txp_book(&mut tx, &core, clock, txp_burst(&clock, target, 1, false, policy));
            tx.chain().mark_reopen_needed();
            let before = device.time_now().unwrap();
            let worked = tx.pass();
            let elapsed = device.time_now().unwrap() - before;
            assert!(worked, "{case}");
            assert_eq!(tx.chain().recovery(), Recovery::Clean, "{case}");
            assert!(timed_sends(&device, 0).is_empty(), "{case}: no experiment payload crossed the reopen turn: {:?}", device.calls());
            assert!(held_keys(&tx).contains(&target), "{case}: the burst is still held");
            assert!(evs(&events).is_empty(), "{case}: the Kernel's clock did not move, so nothing is late yet");
            let expired = device.time_now().unwrap() >= clock.instant(target);
            assert_eq!(expired, retried, "{case}: the device is past the target only after the retry");
            if retried {
                assert!(elapsed >= core.ticks(100_000_000) && elapsed < core.ticks(140_000_000), "{case}: one 100 ms attempt was lost");
            } else {
                assert!(elapsed < core.ticks(40_000_000), "{case}: no attempt was lost");
            }

            device_at(&core, &time, &device, device.time_now().unwrap());
            for _ in 0..4 {
                tx.pass();
            }
            let sent = timed_sends(&device, 0);
            if !retried {
                assert_eq!(sent.len(), 1, "{case}: on time, sent once: {:?}", device.calls());
                assert!(sent[0].1.contains(&format!("at={} sob=true", clock.instant(target))), "{case}: sent at its target: {sent:?}");
                assert!(evs(&events).is_empty(), "{case}");
            } else if outcome == "drop" {
                assert!(sent.is_empty(), "{case}: a dropped burst sends no payload: {sent:?}");
                assert_eq!(evs(&events), ["radio.TIME_ERROR \"drop\""], "{case}");
                assert!(held_keys(&tx).is_empty(), "{case}");
            } else {
                assert_eq!(sent.len(), 1, "{case}: {:?}", device.calls());
                assert!(sent[0].0 > last_flush(&device), "{case}: the moved burst follows the last flush: {:?}", device.calls());
                assert!(!sent[0].1.contains(&format!("at={} ", clock.instant(target))), "{case}: it was moved past its target: {sent:?}");
                assert_eq!(evs(&events), ["radio.TIME_ERROR \"send_asap\""], "{case}: the move is reported once");
            }
        }
    }
}

#[test]
fn ur_22_stop_and_loss_at_the_retry_boundary_release_the_claim() {
    // The publication is made between attempt 1 and attempt 2 (`t3_between`, outside `device_gate`): the owner ends its recovery there, with no close,
    // open or send after it, returns its claim and keeps the held burst for the Stop path, which settles it.
    for stop in [true, false] {
        let case = if stop { "Provider::stop" } else { "loss" };
        let (core, device, _time, mut tx, _clock, events, _guard, target) = retry_rig(vec![FakeFault::DropAckOfSend(0)], true);
        let at_gate = Arc::new(AtomicUsize::new(usize::MAX));
        {
            let (weak_core, weak_device, mark) = (Arc::downgrade(&core), Arc::downgrade(&device), at_gate.clone());
            probe(&core, "t3_between", move |_| {
                let (Some(core), Some(device)) = (weak_core.upgrade(), weak_device.upgrade()) else { return };
                mark.store(device.calls().len(), Ordering::SeqCst);
                if stop {
                    lock(&core.streams).stop = Some((core.now(), StopMode::Orderly));
                } else {
                    core.device_lost("test loss at the retry boundary");
                }
            });
        }
        let worked = tx.pass();
        assert!(worked, "{case}: the owner handled the interrupted recovery");
        assert_eq!(lock(&core.streams).holder(stream_key(Dir::Tx, 0)), Holder::Free, "{case}: the claim is returned");
        assert_eq!(tx.chain().recovery(), Recovery::Clean, "{case}: Stop and loss leave the chain Clean");
        assert!(held_keys(&tx).contains(&target), "{case}: the held burst is not refused by the recovery");
        assert!(lock(&core.rec).rejected.iter().all(|row| !row["reason"].as_str().is_some_and(|reason| reason.contains("could not be reopened"))), "{case}");
        let calls = device.calls();
        assert!(calls[at_gate.load(Ordering::SeqCst)..].iter().all(|call| !call.starts_with("tx_")), "{case}: no transmit call after the publication: {calls:?}");
        assert_eq!(t3_calls(&device).iter().filter(|call| **call == "send").count(), 1, "{case}: one T3 in all");
        assert_eq!(flush_attempt_rows(&core).len(), 1, "{case}: no row for the interrupted attempt");
        assert!(rejected_kinds(&events).is_empty(), "{case}: no rejection event");
        assert_eq!(core.is_lost(), !stop, "{case}");
        halt(&mut tx);
        assert!(held_keys(&tx).is_empty(), "{case}: the Stop path settles the held burst");
    }
}

#[test]
fn ur_75_a_stop_with_a_pending_recovery_then_three_unconfirmed_t3s() {
    // UR-26: a Stop whose close failed keeps the recovery pending, and the next burst triggers it. With a flush that is confirmed it recovers as before
    // (the payload follows the flush); with none confirmed in three attempts it ends as any failed recovery does and no payload follows a flush.
    for (faults, recovers) in [(Vec::new(), true), (vec![FakeFault::DropBurstAck], false)] {
        let case = format!("recovers {recovers}");
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, faults, ..FakeConfig::default() }));
        let _guard = crate::provider::test_support::clock_guard(&device.host());
        let (core, _time, mut tx, clock, events) = recovery_rig(device.clone());
        tx.chain().open_device_burst(0, 100, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
        *device.eob_error.lock().unwrap() = Some(crate::DeviceError::failed("controlled Stop EOB failure"));
        halt(&mut tx);
        assert!(tx.chain().needs_reopen(), "{case}: the Stop did not clear the recovery requirement");
        *device.eob_error.lock().unwrap() = None;
        assert_eq!(rejected_kinds(&events), ["tx_close"], "{case}: the Stop's failed close is the only rejection so far");
        let base = device.calls().len();
        let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
        give(&mut tx, held(&clock, target, 1, LatePolicy::DropAndFlag));
        let worked = tx.pass();
        assert!(worked, "{case}");
        assert!(!tx.chain().needs_reopen(), "{case}");
        assert!(timed_sends(&device, base).is_empty(), "{case}: the reopen turn sent no payload: {:?}", device.calls());
        if recovers {
            assert!(tx.clock.is_some() && held_keys(&tx).contains(&target), "{case}");
            assert!(flush_attempt_rows(&core).is_empty(), "{case}");
            tx.pass();
            let sent = timed_sends(&device, base);
            assert!(sent.len() == 1 && sent[0].1.contains(&format!("at={} sob=true", clock.instant(target))) && sent[0].0 > last_flush(&device), "{case}: {:?}", device.calls());
        } else {
            assert!(tx.clock.is_none() && held_keys(&tx).is_empty(), "{case}: the failed recovery ended the stream and cancelled the burst");
            assert_eq!(flush_attempt_rows(&core).len(), 3, "{case}");
            assert_eq!(t3_calls(&device).last(), Some(&"close"), "{case}: the last streamer is closed");
            assert_eq!(rejected_kinds(&events), ["tx_burst", "tx_reopen"], "{case}");
        }
    }
}

#[test]
fn ur_75_a_report_an_unconfirmed_attempt_stashed_makes_its_event_after_the_recovery() {
    // UR-28: attempt 1 reads a report the flush does not consume and gets no ack; attempt 2 confirms. The owner reads attempt 1's report like any other
    // and makes its event: a lost sequence is a `TX_UNDERFLOW` of cause `lost`, a late underflow in a packet one of cause `starved`. It is attributed
    // to the old generation (no burst of the replacement is abandoned for it) and recorded raw once.
    for (code, cause) in [(TxCode::SeqError, "lost"), (TxCode::SeqErrorInBurst, "lost"), (TxCode::UnderflowInPacket, "starved")] {
        let case = format!("{code:?}");
        let (core, _device, _time, mut tx, _clock, events, _guard, target) = retry_rig(vec![FakeFault::DropAckOfSend(0), FakeFault::TxReport(Wall::ZERO, code)], true);
        let worked = tx.pass();
        assert!(worked, "{case}");
        assert_eq!(flush_attempt_rows(&core).len(), 1, "{case}: attempt 1 was unconfirmed");
        assert_eq!(tx.chain().recovery(), Recovery::Clean, "{case}");
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != format!("{code:?}")), "{case}: nothing is recorded before the owner reads the report");
        tx.pass();
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let underflows: Vec<_> =
            events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TX_UNDERFLOW).map(|event| event.payload["cause"].as_str().unwrap_or_default().to_owned()).collect();
        assert_eq!(underflows, [cause], "{case}: the report made its one event: {events:?}");
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED), "{case}: and rejected nothing: {events:?}");
        let raw = lock(&core.rec).device_async.iter().filter(|row| row["code"] == format!("{code:?}")).cloned().collect::<Vec<_>>();
        assert_eq!(raw.len(), 1, "{case}: {raw:?}");
        assert!(raw[0]["tx_flush"].is_null() && raw[0]["stream"] == 0, "{case}: {raw:?}");
        assert!(held_keys(&tx).contains(&target), "{case}: the held burst is not abandoned for the old generation's report");
        assert!(lock(&core.rec).rejected.iter().all(|row| row["kind"] != "tx_burst"), "{case}");
    }
}

#[test]
fn ur_75_the_owner_s_recovery_waits_out_failing_time_reads_and_ends_failed_only_at_the_cap() {
    // The reads fail from 50 ms: attempt 1 (at 0) sends and loses its ack; attempt 2's reads start inside the window. A window of 500 ms is waited out
    // (a read that succeeds starts the attempt's own 100 ms): the stream recovers and nothing is rejected. A window past the 2 s cap ends the reopen
    // `Failed` with the cause: one failed recovery's rejection, its held burst cancelled, the streamer closed, the device not lost.
    for (window_ms, recovers) in [(500, true), (10_000, false)] {
        let case = format!("reads fail for {window_ms} ms");
        let faults = vec![FakeFault::DropAckOfSend(0), FakeFault::TimeReadFailsFor(Wall::from_millis(50), Wall::from_millis(window_ms))];
        let (core, device, _time, mut tx, _clock, events, _guard, target) = retry_rig(faults, true);
        let before = device.host().now();
        let worked = tx.pass();
        let elapsed = device.host().now() - before;
        assert!(worked, "{case}");
        assert!(!core.is_lost(), "{case}");
        assert!(!tx.chain().needs_reopen(), "{case}");
        let reopen_rows = lock(&core.rec).rejected.iter().filter(|row| row["kind"] == "tx_reopen").cloned().collect::<Vec<_>>();
        if recovers {
            assert_eq!(tx.chain().recovery(), Recovery::Clean, "{case}");
            assert!(tx.clock.is_some() && held_keys(&tx).contains(&target), "{case}: the stream recovered and holds its burst");
            assert!(rejected_kinds(&events).is_empty(), "{case}: no tx_reopen rejection");
            assert!(reopen_rows.is_empty(), "{case}: {reopen_rows:?}");
            assert_eq!(t3_calls(&device), ["open", "close", "open", "send", "close", "open", "send"], "{case}: attempt 2 sent after the reads and confirmed");
            assert!(elapsed >= Wall::from_millis(550), "{case}: the reads were waited out: {elapsed:?}");
        } else {
            assert!(tx.clock.is_none() && held_keys(&tx).is_empty(), "{case}: the failed recovery ended the stream and cancelled the burst");
            assert_eq!(rejected_kinds(&events), ["tx_burst", "tx_reopen"], "{case}: one rejection for the failed recovery");
            assert!(reopen_rows.len() == 1 && reopen_rows[0]["reason"].as_str().unwrap().contains("flush time read failed: "), "{case}: {reopen_rows:?}");
            assert_eq!(t3_calls(&device), ["open", "close", "open", "send", "close", "open", "close"], "{case}: no send after the reads and no third attempt");
            assert!(elapsed >= Wall::from_millis(2_100), "{case}: the cap was waited out: {elapsed:?}");
        }
        halt(&mut tx);
    }
}

#[test]
fn ur_75_a_loss_or_a_stop_while_the_time_reads_fail_is_not_a_failed_recovery() {
    // Reads fail for good from 50 ms. The device is lost at 600 ms, or a Stop is published at the third failing read: the recovery is interrupted at
    // once (no further read after a Stop, no send, no third attempt) and nothing is rejected as a failed recovery.
    for stop in [false, true] {
        let case = if stop { "Provider::stop" } else { "loss" };
        let mut faults = vec![FakeFault::DropAckOfSend(0), FakeFault::TimeReadFailsFor(Wall::from_millis(50), Wall::from_secs(10))];
        if !stop {
            faults.push(FakeFault::Lost(Wall::from_millis(600)));
        }
        let (core, device, _time, mut tx, _clock, events, _guard, target) = retry_rig(faults, true);
        let reads = Arc::new(AtomicUsize::new(0));
        {
            let (weak_core, seen) = (Arc::downgrade(&core), reads.clone());
            probe(&core, "t3_time_read", move |_| {
                let Some(core) = weak_core.upgrade() else { return };
                if seen.fetch_add(1, Ordering::SeqCst) + 1 == 3 && stop {
                    lock(&core.streams).stop = Some((core.now(), StopMode::Orderly));
                }
            });
        }
        let worked = tx.pass();
        assert!(worked, "{case}");
        assert_eq!(t3_calls(&device), ["open", "close", "open", "send", "close", "open"], "{case}: no send, no close and no third attempt");
        if stop {
            assert_eq!(reads.load(Ordering::SeqCst), 3, "{case}: no read after the Stop");
        }
        assert_eq!(flush_attempt_rows(&core).len(), 1, "{case}: no row for the failing reads");
        assert_eq!(lock(&core.streams).holder(stream_key(Dir::Tx, 0)), Holder::Free, "{case}: the claim is returned");
        assert_eq!(tx.chain().recovery(), Recovery::Clean, "{case}");
        assert!(held_keys(&tx).contains(&target), "{case}: the held burst is left to the Stop path");
        assert!(rejected_kinds(&events).is_empty(), "{case}: no rejection event");
        assert_eq!(core.is_lost(), !stop, "{case}");
        halt(&mut tx);
    }
}

// ---------------------------------------------------------------------------------------------------------------- bookings and a Stop made inside the retry

thread_local! {
    /// uhd-control, parked for a probe that books from the owner's thread (the test thread, a participant of the clock through `clock_guard`).
    static CONTROL: RefCell<Option<Control>> = const { RefCell::new(None) };
}

/// A 2 000 sample burst X (ramp 5), three 10 sample ones (ramps 6, 7 and 8), the device faulting as `faults` says.
fn rig(faults: Vec<FakeFault>) -> SwitchRig {
    switch_rig_with(vec![o::ramp(5, 2_000), o::ramp(6, 10), o::ramp(7, 10), o::ramp(8, 10)], faults)
}

/// A finite booking of `waves[wave]` at sample `k`, received through the parked control.
fn book_at(core: &Core, clock: Clock, waves: &[ezsdr_kernel::manifest::ArtifactRef], id: u64, wave: usize, k: i64) {
    CONTROL.with(|slot| book_with(slot.borrow_mut().as_mut().expect("the control is parked"), core, id, 0, clock, k, &waves[wave], false, LatePolicy::DropAndFlag, BTreeMap::new()));
}

/// uhd-control's ordinary Stop of transmit stream 0, received now through the parked control.
fn stop_by(core: &Core) {
    CONTROL.with(|slot| {
        slot.borrow_mut().as_mut().expect("the control is parked").book(Dispatched::new(
            ActionId(9),
            Action::Stop { target: Some(crate::provider::test_support::target_of(&core.tx_id)) },
            Some(core.tx_id.clone()),
        ))
    });
}

/// X is booked at 10 000 (and, with `behind`, a 10 sample burst at its end, 12 000, which the abandon frames on its own) and abandoned by its short
/// send (a front at 10 517), device time is moved past that front and a recovery is pending, the test thread driving the clock. Returns what a case
/// needs.
#[allow(clippy::type_complexity)]
fn abandoned(
    faults: Vec<FakeFault>,
    behind: bool,
) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Tx, Clock, Vec<ezsdr_kernel::manifest::ArtifactRef>, crate::host::Driver) {
    let SwitchRig { core, device, time, events, control, mut tx, clock, waves, _guard } = rig(faults);
    CONTROL.with(|slot| *slot.borrow_mut() = Some(control));
    book_at(&core, clock, &waves, 1, 0, 10_000);
    if behind {
        book_at(&core, clock, &waves, 6, 1, 12_000);
    }
    for ms in 0..=1 {
        device_at(&core, &time, &device, core.ticks(ms * 1_000_000));
        for _ in 0..8 {
            drain_tx(&mut tx);
            if !tx.step() {
                break;
            }
        }
    }
    assert_eq!(sends(&device), [(500, true, false), (2, false, true)], "X is abandoned by its short send: {:?}", device.calls());
    device_at(&core, &time, &device, 11 * MS);
    tx.chain().mark_reopen_needed();
    (core, device, time, events, tx, clock, waves, _guard)
}

/// The turns of the owner, the way the run loop makes them, until it has nothing to do.
fn turns_until_idle(tx: &mut Tx) {
    for _ in 0..8 {
        if !tx.pass() {
            break;
        }
    }
}

#[test]
fn ur_75_an_ordinary_tx_stop_during_the_recovery() {
    // A Stop receipt (uhd-control's `Action::Stop`, not `Provider::stop`) interrupts nothing: the recovery runs to its end and the list is read afterwards, in
    // receipt order. H was read before the reopen began; the probe at the retry boundary books A, then the Stop, then B.
    const B: i64 = 300_000;
    for recovers in [true, false] {
        let case = if recovers { "recovered" } else { "failed" };
        // Sends: X's short one (#0), its close (#1), then the flush attempts from #2 on.
        let faults = vec![FakeFault::ShortSend(0), if recovers { FakeFault::DropAckOfSend(2) } else { FakeFault::DropBurstAck }];
        let (core, device, time, events, mut tx, clock, waves, _driver) = abandoned(faults, false);
        let _ = evs(&events);
        // H, booked and read before the recovery begins.
        book_at(&core, clock, &waves, 2, 1, 40_000);
        tx.read_list();
        let base = device.calls().len();
        assert!(held_keys(&tx).contains(&40_000), "{case}: H is held");
        let fired = Arc::new(AtomicBool::new(false));
        {
            let (weak, waves, fired) = (Arc::downgrade(&core), waves.clone(), fired.clone());
            probe(&core, "t3_between", move |_| {
                let Some(core) = weak.upgrade() else { return };
                if fired.swap(true, Ordering::SeqCst) {
                    return;
                }
                book_at(&core, clock, &waves, 3, 2, 50_000);
                stop_by(&core);
                book_at(&core, clock, &waves, 4, 3, B);
            });
        }
        let key = stream_key(Dir::Tx, 0);
        let rows_before = lock(&core.rec).rejected.len();
        assert!(tx.pass(), "{case}: the reopen runs");
        assert!(fired.load(Ordering::SeqCst), "{case}: the probe ran at a retry boundary");
        assert_eq!(lock(&core.streams).holder(key), Holder::Free, "{case}: the claim was returned");
        assert!(!tx.chain().needs_reopen(), "{case}");
        assert!(device.time_now().unwrap() >= 11 * MS + core.ticks(100_000_000), "{case}: at least the first attempt was lost");
        let reopen_calls = device.calls().len();
        if recovers {
            assert_eq!(tx.chain().recovery(), Recovery::Clean, "{case}");
            assert_eq!(
                device.calls().iter().rfind(|call| call.starts_with("tx_")).map(String::as_str),
                Some("tx_send 0 n=4 at=- sob=true eob=true"),
                "{case}: the recovery ends at attempt 2's flush: {:?}",
                device.calls()
            );
            assert_eq!(t3_calls(&device)[t3_calls(&device).len() - 5..], ["open", "send", "close", "open", "send"], "{case}");
            assert!(evs(&events).is_empty(), "{case}: a recovered stream is not rejected");
            tx.read_list();
            assert_eq!(notes(&core), ["UR-26: cancelled by Stop", "UR-26: cancelled by Stop"], "{case}: H and A are cancelled by the Stop");
            assert_eq!(held_keys(&tx), [B], "{case}: B, received after the Stop, is held");
            assert!(evs(&events).is_empty(), "{case}: the Stop's cancellations are not events, and B is not refused");
            device_at(&core, &time, &device, clock.instant(B) - 5 * MS);
            turns_until_idle(&mut tx);
            let sent = timed_sends(&device, base);
            assert!(sent.len() == 1 && sent[0].1.contains(&format!("at={} sob=true", clock.instant(B))), "{case}: B is sent: {:?}", device.calls());
            assert!(sent[0].0 > last_flush(&device), "{case}: B follows the last flush");
            assert_eq!(t3_calls(&device)[..].iter().filter(|call| **call == "send").count(), 2 + 2 + 1, "{case}: X's short send and its close, two flushes, B's payload");
        } else {
            assert!(tx.clock.is_none(), "{case}: three unconfirmed flushes end the stream");
            assert_eq!(t3_calls(&device).last(), Some(&"close"), "{case}: the calls end with attempt 3's close");
            assert_eq!(flush_attempt_rows(&core).len(), 3, "{case}");
            tx.read_list();
            assert_eq!(rejected_kinds(&events), ["tx_burst", "tx_reopen", "tx_burst", "tx_burst"], "{case}: H cancelled by the failure, then the failure, then A and B met by the ended clock");
            assert!(notes(&core).is_empty(), "{case}: the ended clock came first: nobody is cancelled by the Stop");
            let reasons: Vec<String> = lock(&core.rec).rejected.iter().skip(rows_before).filter(|row| row["kind"] == "tx_burst").map(|row| row["reason"].as_str().unwrap().to_owned()).collect();
            assert_eq!(
                reasons,
                [
                    "UR-22: cancelled because the TX streamer could not be reopened",
                    "UR-25: its transmit clock ended before the burst could start",
                    "UR-25: its transmit clock ended before the burst could start"
                ],
                "{case}"
            );
            assert!(timed_sends(&device, base).is_empty(), "{case}: no payload");
            assert_eq!(device.calls().len(), reopen_calls, "{case}: no later attempt after the failure");
        }
        assert_eq!(lock(&core.streams).holder(key), Holder::Free, "{case}: no later attempt holds the claim");
    }
}

// ---------------------------------------------------------------------------------------------------------------- the record

/// The record's verdicts as a receipt reads them: a wide range, across X's footprint and the region a Stop adds.
fn runs(core: &Core, clock: Clock) -> String {
    receipt_runs(core, clock, 11_900, 25_100)
}

/// One history of X's recovery with `faults`; at the probe named, the record's verdicts, and with `stop` an ordinary Stop received there (once). Returns
/// every reading the probe made, then the verdicts after the recovery.
fn observed(faults: Vec<FakeFault>, probe_name: &'static str, stop: bool) -> (Vec<String>, String) {
    let (core, _device, _time, _events, mut tx, clock, _waves, _driver) = abandoned(faults, true);
    assert_eq!(accepted(&core, clock, 12_000, 1), Err(Overlap::Booking), "a booking at X's end is refused");
    assert_eq!(accepted(&core, clock, 12_025, 1), Ok(()), "X's footprint and the burst the abandon framed behind it end at 12 025");
    let seen = Arc::new(Mutex::new(vec![runs(&core, clock)]));
    {
        let (weak, seen, stopped) = (Arc::downgrade(&core), seen.clone(), Arc::new(AtomicBool::new(false)));
        probe(&core, probe_name, move |_| {
            let Some(core) = weak.upgrade() else { return };
            let mut seen = seen.lock().unwrap();
            seen.push(runs(&core, clock));
            if stop && !stopped.swap(true, Ordering::SeqCst) {
                stop_by(&core);
                seen.push(runs(&core, clock));
            }
        });
    }
    assert!(tx.pass(), "the reopen runs");
    let after = runs(&core, clock);
    let seen = seen.lock().unwrap().clone();
    (seen, after)
}

#[test]
fn ur_75_the_record_across_a_retried_recovery_is_the_one_attempt_record() {
    // The abandoned finite burst's reservation (a booking at its end is refused; its footprint ends at its end plus four) and a Stop received inside the
    // window are receipt-side: the longer path of a retried recovery, recovered or failed, leaves the record as the no-retry controls do. The controls are
    // a recovery that confirms at once and one whose open fails, observed at the reopen's start.
    let retried = |fault: FakeFault| vec![FakeFault::ShortSend(0), fault];
    for stop in [false, true] {
        let case = format!("stop {stop}");
        let (control, control_after) = observed(vec![FakeFault::ShortSend(0)], "tx_reopen_claimed", stop);
        let (refused_open, refused_after) = observed(vec![FakeFault::ShortSend(0), FakeFault::TxOpenFails(1)], "tx_reopen_claimed", stop);
        let (recovers, recovers_after) = observed(retried(FakeFault::DropAckOfSend(2)), "t3_between", stop);
        let (fails, fails_after) = observed(retried(FakeFault::DropBurstAck), "t3_between", stop);
        let before = control[0].clone();
        assert_eq!(recovers_after, control_after, "{case}: a retried recovery that confirms leaves the control's record");
        assert_eq!(fails_after, refused_after, "{case}: a recovery that fails after three attempts leaves the record a failed open does");
        for (name, seen) in [("control", &control), ("refused open", &refused_open), ("recovers", &recovers), ("fails", &fails)] {
            assert_eq!(seen[0], before, "{case}: {name}: the record at the start");
            assert!(seen.len() >= 2, "{case}: {name}: the probe ran {seen:?}");
        }
        if stop {
            // [the start, the reading before the Stop, the one after it] and, at the failed recovery's second gate, one more.
            let stopped = control[2].clone();
            assert_ne!(stopped, before, "the Stop changed the record (else the case proves nothing)");
            for (name, seen) in [("control", &control), ("refused open", &refused_open), ("recovers", &recovers), ("fails", &fails)] {
                assert_eq!(seen[1], before, "{case}: {name}: the reading at the gate before the Stop");
                assert!(seen[2..].iter().all(|reading| *reading == stopped), "{case}: {name}: every reading after the Stop is the control's: {seen:?}");
            }
            assert_eq!(control_after, stopped, "{case}: the control's record after the recovery is the Stop's");
            assert_eq!(fails.len(), 4, "{case}: the failed recovery's gates: before attempt 2 (before and after the Stop) and before attempt 3: {fails:?}");
        } else {
            for (name, seen) in [("control", &control), ("refused open", &refused_open), ("recovers", &recovers), ("fails", &fails)] {
                assert!(seen.iter().all(|reading| *reading == before), "{case}: {name}: every reading is the start's: {seen:?}");
            }
            assert_eq!(control_after, before, "{case}: a recovery that confirms at once leaves the record as it was");
            assert_eq!(fails.len(), 3, "{case}: the start and the failed recovery's two gates: {fails:?}");
        }
    }
}
