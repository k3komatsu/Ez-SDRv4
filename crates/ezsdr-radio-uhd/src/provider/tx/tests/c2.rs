//! C2 (design note 49 stage C2; plan `2026-11-12-c2/test-plan.md`): a switch's outcome follows whether the sample at its boundary was sent
//! (RM-37, UR-38). The frozen single-threaded rig of E1: device time moves only through `device_at`, a probe runs on the test thread with
//! no lock held, so no verdict depends on when a thread runs (note 51 §3).
//!
//! The rig is O1's `failing_switch`: a repeat R (`waves[0]`, 1 000 samples) from 2 000 handed through 1 ms, a switch to `waves[1]` (400
//! samples) received at 2 ms, so its boundary `b` is 14 000. R's sends are the first five (`FailSend(k)` and its kin count every send
//! that carries samples), the send that carries `b` is the sixth, `k` = 5, and the owner plans it once device time passes 4 ms.
//!
//! Units: one sample is 1 us (the rig's 1 MS/s), 200 root ticks; `MS` is a millisecond in root ticks.

use super::e1::{self, MS, at, notes, rows, shutdown, stop_receipt};
use super::*;
use crate::device::TxReport;
use crate::provider::test_support::{end_streams, probe, target_of};
use crate::provider::tx::oracle as o;
use ezsdr_radio::staging::Booked;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const B: i64 = 14_000;

/// The rig with the switch received at 2 ms and read by the owner.
fn switched(faults: Vec<FakeFault>) -> SwitchRig {
    let mut rig = switch_rig_with(vec![o::ramp(17, 1_000), o::ramp(18, 400)], faults);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 1);
    at(&rig, 2 * MS);
    book_switch(&mut rig, 2, 2_000, 1);
    turn(&mut rig);
    assert_eq!(switch_row(&rig.core, 0)["boundary"], B);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending");
    rig
}

/// The owner's turn as the run loop makes it: read the list, step until there is nothing to do.
fn turn(rig: &mut SwitchRig) {
    for _ in 0..8 {
        drain_tx(&mut rig.tx);
        if !rig.tx.step() {
            break;
        }
    }
}

/// Whether the send being planned is the first of the switch's own booking: the one that carries the sample at `b`.
fn carries_the_boundary(core: &Core) -> bool {
    let streams = lock(&core.streams);
    streams.stream(stream_key(Dir::Tx, 0)).book.open.as_ref().is_some_and(|open| open.first && open.booking.id() == Some(ActionId(2)))
}

/// What `act` is given at the hook: the Core, uhd-control and the device clock.
struct Hook<'a> {
    core: &'a Core,
    control: &'a mut Control,
    time: &'a ezsdr_kernel::time::ManualTimeAuthority,
    device: &'a FakeDevice,
}

impl Hook<'_> {
    /// uhd-control's Stop of the stream, received now.
    fn stop(&mut self) {
        self.control.book(Dispatched::new(ActionId(9), Action::Stop { target: Some(target_of(&self.core.tx_id)) }, Some(self.core.tx_id.clone())));
    }

    /// The Provider's stop or a loss of the device at the tick `ticks`, as `Streams::end` books it.
    fn end(&self, ticks: i64, lost: bool) {
        end_streams(self.core, ticks, false, lost);
    }

    /// The device time moves to `ticks`.
    fn device_at(&self, ticks: i64) {
        self.time.advance_to(self.core.at(ticks)).unwrap();
        self.device.set_time(ticks);
    }

    /// The row of the switch as the Manifest would have it now.
    fn row(&self) -> String {
        switch_row(self.core, 0)["outcome"].as_str().unwrap().to_owned()
    }
}

/// `run(rig, from, until)` with `act` run once, at the `tx_send_planned` of the send that carries the switch's boundary sample: issued, not
/// made, no lock held (the probe holds the Core weakly, and the control is taken back after the run: a probe holding either keeps the rig
/// from ever dropping).
fn run_acting_at_the_boundary_send(rig: SwitchRig, from: i64, until: i64, act: impl Fn(&mut Hook) + Send + Sync + 'static) -> SwitchRig {
    let SwitchRig { core, device, time, events, control, mut tx, clock, waves, _guard } = rig;
    let control = Arc::new(Mutex::new(Some(control)));
    let fired = Arc::new(AtomicBool::new(false));
    let (hook_core, hook_control, hook_time, hook_device, hook_fired) = (Arc::downgrade(&core), control.clone(), Arc::downgrade(&time), Arc::downgrade(&device), fired.clone());
    probe(&core, "tx_send_planned", move |_| {
        let (Some(core), Some(time), Some(device)) = (hook_core.upgrade(), hook_time.upgrade(), hook_device.upgrade()) else { return };
        if !carries_the_boundary(&core) || hook_fired.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut control = hook_control.lock().unwrap();
        act(&mut Hook { core: &core, control: control.as_mut().expect("the rig's control"), time: &time, device: &device });
    });
    // The run stops at the turn the hook fired in: `act` may have moved device time, which a later turn must not set back.
    for ms in from..=until {
        device_at(&core, &time, &device, core.ticks(ms * 1_000_000));
        for _ in 0..8 {
            drain_tx(&mut tx);
            if !tx.step() {
                break;
            }
        }
        if fired.load(Ordering::SeqCst) {
            break;
        }
    }
    assert!(fired.load(Ordering::SeqCst), "the send that carries b was planned");
    let control = control.lock().unwrap().take().expect("the rig's control");
    SwitchRig { core, device, time, events, control, tx, clock, waves, _guard }
}

fn heard_new(rig: &SwitchRig) -> bool {
    rig.device.transmitted(rig.clock.instant(0), 30_000).iter().flatten().any(|s| o::ramp(18, 400).contains(s) && *s != [0.0, 0.0])
}

/// The first send that carries the sample at `b` is on the device.
fn boundary_send_is_on_the_device(rig: &SwitchRig) -> bool {
    let at = format!("at={}", rig.clock.instant(B));
    rig.device.calls().iter().any(|call| call.starts_with("tx_send 0 ") && call.contains(&at))
}

#[test]
fn c2_s4_a_stop_before_the_boundary_send_cancels_the_switch() {
    // KL-42: the Stop at 5.6 ms (bound 15 605, past b = 14 000) is received and read before the owner issued any of the new waveform:
    // the row says `cancelled`, as the note does (UR-26), and not a sample of it was sent. Through `stop_receipt` and through the
    // owner's read, as the two Stop paths of the run loop.
    for read_first in [true, false] {
        let mut rig = switched(Vec::new());
        run(&mut rig, 3, 3);
        at(&rig, 5 * MS + 3 * MS / 5);
        if read_first {
            rig.tx.read_list();
        }
        stop_receipt(&mut rig);
        rig.tx.read_list();
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "read first {read_first}");
        run(&mut rig, 6, 30);
        assert!(!boundary_send_is_on_the_device(&rig), "{:?}", rig.device.calls());
        assert!(!heard_new(&rig), "read first {read_first}: {:?}", rig.device.calls());
        assert_eq!(rows(&rig).len(), 1, "no successor record: {:?}", rows(&rig));
        assert_eq!(notes(&rig.core), ["UR-26: cancelled by Stop"], "{:?}", lock(&rig.core.rec).rejected);
    }
}

#[test]
fn c2_s3_a_stop_after_the_boundary_send_leaves_it_applied() {
    // The Stop comes right after the first send of the new waveform (H1: what is handed is not recalled): the new waveform sounds up to
    // the Stop's cut. The row stays pending until device time reaches b (a loss before it would still cancel: `c2_s24_…`), then applied.
    let mut rig = switched(Vec::new());
    let mut ms = 3;
    while !boundary_send_is_on_the_device(&rig) {
        run(&mut rig, ms, ms);
        ms += 1;
        assert!(ms < 12, "the owner sends the boundary sample once device time passes 4 ms: {:?}", rig.device.calls());
    }
    assert!(ms * 1_000 < B, "device time has not reached b: {ms} ms");
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "sent, device time short of b");
    stop_receipt(&mut rig);
    rig.tx.read_list();
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "the Stop is before b");
    run(&mut rig, ms + 1, 30);
    assert!(heard_new(&rig), "{:?}", rig.device.calls());
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied");
}

#[test]
fn c2_s5_a_stop_in_flight_is_decided_by_the_result() {
    // The Stop is received between the plan and the device call of the send that carries b: the row stays `pending` until the send's
    // result, then follows it (whole: applied, once device time reaches b or the Manifest is written, as after any Stop below b; short with
    // a sample taken: applied too, once device time reaches b: the owner's abandon halts at device time; none taken: cancelled; an exception: uncertain). The note is the
    // Stop's own, written only for a payload it cancels.
    for (fault, expected) in [(None, "applied"), (Some(FakeFault::ShortSend(5)), "applied"), (Some(FakeFault::StalledSend(5)), "cancelled"), (Some(FakeFault::FailSend(5)), "uncertain")] {
        for provider_stop in [false, true] {
            let rig = switched(fault.iter().cloned().collect());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let log = seen.clone();
            let rig = run_acting_at_the_boundary_send(rig, 3, 8, move |hook| {
                let now = hook.core.now();
                if provider_stop {
                    hook.end(now, false);
                } else {
                    hook.stop();
                }
                log.lock().unwrap().push(hook.row());
            });
            let case = format!("{fault:?}, provider stop {provider_stop}");
            assert_eq!(*seen.lock().unwrap(), ["pending"], "{case}: pending between the Stop and the result");
            let waits = matches!(fault, None | Some(FakeFault::ShortSend(_)));
            assert_eq!(switch_row(&rig.core, 0)["outcome"], if waits { "pending" } else { expected }, "{case}");
            rig.core.settle_unresolved_switches();
            assert_eq!(switch_row(&rig.core, 0)["outcome"], expected, "{case}: at the Manifest write");
            if expected == "cancelled" {
                assert!(!heard_new(&rig), "{case}");
            }
        }
    }
}

#[test]
fn c2_s8_a_short_boundary_send_is_applied_when_it_took_a_sample_and_cancelled_when_it_took_none() {
    // UR-22, RM-37: `ShortSend` takes half of the send that carries b (1 250 samples: applied; half of it sounds), `StalledSend` none of it
    // (cancelled: the owner's cursor is b, which has not handed the sample at b), `FailSend` leaves it uncertain. The owner's abandon halts
    // at device time (8 ms, short of b), so the row of a short send that took a sample waits for device time to reach b, as after a Stop.
    for (fault, expected, sounds) in [(FakeFault::ShortSend(5), "applied", true), (FakeFault::StalledSend(5), "cancelled", false), (FakeFault::FailSend(5), "uncertain", true)] {
        let mut rig = switched(vec![fault.clone()]);
        run(&mut rig, 3, 8);
        assert_eq!(switch_row(&rig.core, 0)["outcome"], if expected == "applied" { "pending" } else { expected }, "{fault:?}: abandoned at device time 8 ms");
        run(&mut rig, 9, 30);
        assert_eq!(switch_row(&rig.core, 0)["outcome"], expected, "{fault:?}");
        assert_eq!(heard_new(&rig), sounds, "{fault:?}: {:?}", rig.device.calls());
    }
}

/// The send that carries b is whole; `code` is reported before device time reaches b; the row waits like a Stop's and is applied once device time reaches b.
/// Returns the `tx_resume` rows written.
fn s10(code: crate::device::TxCode, tick: Option<i64>) -> usize {
    let mut rig = switched(Vec::new());
    let mut ms = 3;
    while !boundary_send_is_on_the_device(&rig) {
        run(&mut rig, ms, ms);
        ms += 1;
        assert!(ms < 12, "{:?}", rig.device.calls());
    }
    rig.tx.report(TxReport { code, tick: tick.map(|sample| rig.clock.instant(sample)), channel: 0 });
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "device time is short of b");
    run(&mut rig, ms, 30);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied");
    assert!(!notes(&rig.core).iter().any(|note| note.contains("Stop")), "{:?}", lock(&rig.core.rec).rejected);
    let timing = lock(&rig.core.rec).timing.clone();
    timing.iter().filter(|row| row["what"] == "tx_resume").count()
}

#[test]
fn c2_s10_an_underflow_after_the_boundary_send_does_not_unsend_it() {
    // The send that carries b is whole; an underflow whose tick is b is reported before device time reaches b. The repeat resumes (UR-22) instead of being
    // abandoned, and the sample at b was sent either way: the row waits like a Stop's and is applied once device time reaches b, and no note is written for the
    // new waveform. (The abandon's own footprint body is the sequence-error twin below; the switch across the gap is D2's.)
    assert_eq!(s10(crate::device::TxCode::Underflow, Some(B)), 1, "the underflow resumed the repeat");
}

#[test]
fn c2_s10_a_sequence_error_after_the_boundary_send_does_not_unsend_it() {
    // As above with a lost sequence, which ends a repeat by the abandon at its cursor, past b, at device time short of b: no resume.
    assert_eq!(s10(crate::device::TxCode::SeqError, None), 0);
}

#[test]
fn c2_s11_a_host_late_switch_is_pending_until_the_turn_then_cancelled() {
    // Device time passes b while the owner has issued none of the new waveform: nothing of it was sent, so the row is not applied at b.
    // The owner's next turn finds its timeline expired and abandons the repeat short of b: cancelled, and no sample of the new waveform.
    let mut rig = switched(Vec::new());
    at(&rig, 15 * MS);
    rig.tx.read_list();
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "device time is past b, nothing sent");
    // A lost sequence ends the repeat as the expired timeline did before a repeat resumed (UR-22).
    rig.tx.report(TxReport { code: crate::device::TxCode::SeqError, tick: None, channel: 0 });
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled");
    run(&mut rig, 16, 30);
    assert!(!heard_new(&rig), "{:?}", rig.device.calls());
}

#[test]
fn c2_s14_provider_stop_is_decided_like_a_stop() {
    // `Provider::stop` (`Streams::end`, not lost): not issued → cancelled (S4); issued and taken → applied (S3), when device time reaches b
    // or at the Manifest write.
    let mut rig = switched(Vec::new());
    run(&mut rig, 3, 3);
    end_streams(&rig.core, 5 * MS + 3 * MS / 5, false, false);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "S4");
    let mut rig = switched(Vec::new());
    let mut ms = 3;
    while !boundary_send_is_on_the_device(&rig) {
        run(&mut rig, ms, ms);
        ms += 1;
        assert!(ms < 12, "{:?}", rig.device.calls());
    }
    end_streams(&rig.core, ms * MS, false, false);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "S3: the Stop is before b");
    rig.core.settle_unresolved_switches();
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied", "S3");
}

#[test]
fn c2_s13_a_loss_before_the_boundary_is_decided_by_device_time() {
    // Q1: a loss settles a switch by device time at the loss: the sample at b was played only when `b < t_k`. A loss at 3 ms (before b, send
    // not issued) is cancelled; a loss after the boundary send, with device time short of b, is cancelled too; one with device time
    // past b is applied. (A Stop at the same instants would say applied for the sent one: `c2_s3_…`.)
    let mut rig = switched(Vec::new());
    run(&mut rig, 3, 3);
    end_streams(&rig.core, 3 * MS, false, true);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "not issued");

    let mut rig = switched(Vec::new());
    let mut ms = 3;
    while !boundary_send_is_on_the_device(&rig) {
        run(&mut rig, ms, ms);
        ms += 1;
        assert!(ms < 12, "{:?}", rig.device.calls());
    }
    end_streams(&rig.core, ms * MS, false, true);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "S20: sent, the loss before b");
}

#[test]
fn c2_s19_a_loss_while_the_boundary_send_is_in_flight_is_decided_by_the_result_and_the_loss_sample() {
    // M1 (Sol's plan review): a loss at the sample `t_k` before b while the send that carries b is in flight: the row stays pending, and
    // then follows the result and the loss sample, never the Stop's `applied`: cancelled for a send taken whole, short or not at all,
    // uncertain for an exception. S19b: with device time past b at the loss, the whole send is applied.
    for (fault, expected) in [(None, "cancelled"), (Some(FakeFault::ShortSend(5)), "cancelled"), (Some(FakeFault::StalledSend(5)), "cancelled"), (Some(FakeFault::FailSend(5)), "uncertain")] {
        let rig = switched(fault.iter().cloned().collect());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let rig = run_acting_at_the_boundary_send(rig, 3, 8, move |hook| {
            hook.end(hook.core.now(), true);
            log.lock().unwrap().push(hook.row());
        });
        assert_eq!(*seen.lock().unwrap(), ["pending"], "{fault:?}");
        assert_eq!(switch_row(&rig.core, 0)["outcome"], expected, "{fault:?}");
    }
    // (The fake drops a late continuation whatever fault is set, so an exception cannot be arranged after device time has passed b.)
    for (fault, expected) in [(None, "applied"), (Some(FakeFault::StalledSend(5)), "cancelled")] {
        let rig = switched(fault.iter().cloned().collect());
        let rig = run_acting_at_the_boundary_send(rig, 3, 8, move |hook| {
            hook.device_at(15 * MS);
            hook.end(15 * MS, true);
        });
        assert_eq!(switch_row(&rig.core, 0)["outcome"], expected, "S19b: the loss after b, {fault:?}");
    }
}

#[test]
fn c2_s20_a_loss_after_the_boundary_send_before_device_time_reaches_b() {
    // The send that carries b is whole and the book has not run `advance`: a loss at or before b cancels, one after b is applied (what
    // `advance` would have done), whatever device time the owner had read.
    for (ticks, expected) in [(5 * MS, "cancelled"), (B * 200, "cancelled"), ((B + 1) * 200, "applied")] {
        let mut rig = switched(Vec::new());
        let mut ms = 3;
        while !boundary_send_is_on_the_device(&rig) {
            run(&mut rig, ms, ms);
            ms += 1;
            assert!(ms < 12, "{:?}", rig.device.calls());
        }
        at(&rig, ticks.max(ms * MS));
        end_streams(&rig.core, ticks.max(ms * MS), false, true);
        assert_eq!(switch_row(&rig.core, 0)["outcome"], expected, "loss at {ticks}");
    }
}

#[test]
fn c2_s21_a_stop_and_a_loss_in_flight_either_order() {
    // The Stop's bound is past b and the loss is at the same instant, before b: cancelled in either order, the row pending until the
    // result; an exception is uncertain.
    for (fault, expected) in [(None, "cancelled"), (Some(FakeFault::FailSend(5)), "uncertain")] {
        for stop_first in [true, false] {
            let rig = switched(fault.iter().cloned().collect());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let log = seen.clone();
            let rig = run_acting_at_the_boundary_send(rig, 3, 8, move |hook| {
                let now = hook.core.now();
                if stop_first {
                    hook.stop();
                    hook.end(now, true);
                } else {
                    hook.end(now, true);
                    hook.stop();
                }
                log.lock().unwrap().push(hook.row());
            });
            assert_eq!(*seen.lock().unwrap(), ["pending"], "{fault:?}, stop first {stop_first}");
            assert_eq!(switch_row(&rig.core, 0)["outcome"], expected, "{fault:?}, stop first {stop_first}");
        }
    }
}

#[test]
fn c2_s22_a_loss_after_provider_stop_in_flight_keeps_the_loss_sample() {
    // Sol's code review, major 1: `Provider::stop` ends the streams with the boundary send in flight (bound past b: the row waits for the
    // result), and the device is then lost at the sample `t_k` before b. The loss is a halt too: the row follows the result and the loss
    // sample, so a send that completes whole is cancelled (`t_k` before b), an exception is uncertain, and with device time past b at the
    // loss the whole send is applied. The reverse order (the loss first) is the same row.
    for (fault, past_b, expected) in [(None, false, "cancelled"), (Some(FakeFault::FailSend(5)), false, "uncertain"), (None, true, "applied")] {
        for loss_first in [false, true] {
            let rig = switched(fault.iter().cloned().collect());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let log = seen.clone();
            let rig = run_acting_at_the_boundary_send(rig, 3, 8, move |hook| {
                let now = if past_b {
                    hook.device_at(15 * MS);
                    15 * MS
                } else {
                    hook.core.now()
                };
                if loss_first {
                    hook.end(now, true);
                    hook.end(now, false);
                } else {
                    hook.end(now, false);
                    hook.end(now, true);
                }
                log.lock().unwrap().push(hook.row());
            });
            let case = format!("{fault:?}, device time past b {past_b}, loss first {loss_first}");
            assert_eq!(*seen.lock().unwrap(), ["pending"], "{case}: pending between the halts and the result");
            assert_eq!(switch_row(&rig.core, 0)["outcome"], expected, "{case}");
        }
    }
}

#[test]
fn c2_s24_a_loss_after_a_stop_below_b_overturns_a_whole_boundary_send_s_applied() {
    // Opus re-check of fix round 1 (Q1: a loss settles by device time): the boundary send is whole and device time (ms) is short of b = 14 000;
    // a `Provider::stop` or uhd-control's Stop with its bound past b comes. The row waits (`pending`): a loss at a sample at or before b
    // cancels it, as it would have before the Stop; a loss past b, device time reaching b and the Manifest write apply it. Every cell is
    // the oracle's (`switch_outcome_after_loss`, `switch_outcome_after_stop`).
    let p = e1::params();
    for provider_stop in [true, false] {
        for (cell, loss) in [("loss at the Stop", Some(0)), ("loss past b", Some(15 * MS)), ("device time reaches b", None), ("the Manifest write", None)] {
            let mut rig = switched(Vec::new());
            let mut ms = 3;
            while !boundary_send_is_on_the_device(&rig) {
                run(&mut rig, ms, ms);
                ms += 1;
                assert!(ms < 12, "{:?}", rig.device.calls());
            }
            let t = ms * MS;
            let (_, bound) = p.stop_bound(t);
            if provider_stop {
                end_streams(&rig.core, t, false, false);
            } else {
                stop_receipt(&mut rig);
                rig.tx.read_list();
            }
            let case = format!("provider stop {provider_stop}, {cell}");
            assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "{case}: the Stop is before b");
            let loss = loss.map(|after| t.max(after));
            if let Some(ticks) = loss {
                at(&rig, ticks);
                end_streams(&rig.core, ticks, false, true);
            } else if cell == "device time reaches b" {
                run(&mut rig, ms + 1, 30);
            }
            let expected = match loss {
                Some(ticks) => o::switch_outcome_after_loss(B, bound, p.clock.at_or_after(ticks), o::Sent::Whole),
                None => o::switch_outcome_after_stop(B, bound, o::Sent::Whole),
            };
            if loss == Some(t) || cell == "device time reaches b" {
                assert_eq!(switch_row(&rig.core, 0)["outcome"], expected.as_str(), "{case}: settled before the Manifest write");
            }
            rig.core.settle_unresolved_switches();
            assert_eq!(switch_row(&rig.core, 0)["outcome"], expected.as_str(), "{case}");
        }
    }
}

#[test]
fn c2_s25_a_loss_after_the_result_of_a_send_in_flight_at_a_stop_overturns_applied() {
    // The same history as `c2_s24_…` with the Stop received while the boundary send is in flight: the result (whole) arrives, the row is
    // still `pending` (device time is short of b), and a loss at that device time cancels it, as the oracle says.
    let p = e1::params();
    for provider_stop in [true, false] {
        let rig = run_acting_at_the_boundary_send(switched(Vec::new()), 3, 8, move |hook| {
            if provider_stop {
                hook.end(hook.core.now(), false);
            } else {
                hook.stop();
            }
        });
        let case = format!("provider stop {provider_stop}");
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "{case}: the result is in, device time is short of b");
        let t = rig.core.now();
        let (_, bound) = p.stop_bound(t);
        end_streams(&rig.core, t, false, true);
        assert_eq!(switch_row(&rig.core, 0)["outcome"], o::switch_outcome_after_loss(B, bound, p.clock.at_or_after(t), o::Sent::Whole).as_str(), "{case}");
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "{case}");
    }
}

#[test]
fn c2_s26_an_abandon_after_a_whole_boundary_send_halts_at_device_time() {
    // Opus re-check of fix round 2: the boundary send is whole and device time is short of b = 14 000; the next send (the seventh: 1 250
    // samples at 16 500) fails after half of them, which is not a loss, so the owner abandons the repeat at its cursor, past b. `Halted.at`
    // is device time, not the cursor: the row waits like a Stop's, a loss at a sample at or before b cancels it, a loss past b, device time
    // reaching b and the Manifest write apply it. Every cell is the oracle's (`switch_outcome_after_abandon_and_loss`).
    let p = e1::params();
    for (cell, loss) in [("loss at the abandon", Some(0)), ("loss past b", Some(15 * MS)), ("device time reaches b", None), ("the Manifest write", None)] {
        let mut rig = switched(vec![FakeFault::FailSend(6)]);
        let mut ms = 3;
        while lock(&rig.core.rec).rejected.is_empty() {
            run(&mut rig, ms, ms);
            ms += 1;
            assert!(ms < 12, "the seventh send fails before b: {:?}", rig.device.calls());
        }
        assert!(boundary_send_is_on_the_device(&rig) && (ms - 1) * 1_000 < B, "the abandon comes after the boundary send, device time {ms} ms");
        let case = cell;
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "{case}: abandoned at a cursor past b, device time short of it");
        let t = (ms - 1) * MS;
        let loss = loss.map(|after| t.max(after));
        if let Some(ticks) = loss {
            at(&rig, ticks);
            end_streams(&rig.core, ticks, false, true);
        } else if cell == "device time reaches b" {
            run(&mut rig, ms, 30);
        }
        let expected = match loss {
            Some(ticks) => o::switch_outcome_after_abandon_and_loss(B, 16_500, p.clock.at_or_after(ticks), o::Sent::Whole),
            None => o::switch_outcome_after_abandon(B, 16_500, o::Sent::Whole),
        };
        if loss.is_some() || cell == "device time reaches b" {
            assert_eq!(switch_row(&rig.core, 0)["outcome"], expected.as_str(), "{case}: settled before the Manifest write");
        }
        rig.core.settle_unresolved_switches();
        assert_eq!(switch_row(&rig.core, 0)["outcome"], expected.as_str(), "{case}");
    }
}

#[test]
fn c2_s23_a_worker_that_panics_in_the_boundary_send_leaves_the_row_uncertain_at_the_close() {
    // Sol's code review, major 2: the owner panics after it issued the send that carries b, so no result comes, and the panic handler
    // publishes the loss (`device_lost`). The row stays `pending` as long as the Run goes on, and the Manifest write settles it as a send
    // whose result is unknown: uncertain, as `FailSend` would have it, unless a Stop with a bound at or before b (the one at 3 ms) came first: cancelled.
    for stopped_before_b in [false, true] {
        let rig = switched(Vec::new());
        let core = rig.core.clone();
        let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| {
            run_acting_at_the_boundary_send(rig, 3, 8, move |hook| {
                if stopped_before_b {
                    hook.end(3 * MS, false);
                }
                panic!("test: the owner panics in the boundary send");
            })
        }));
        assert!(panicked.is_err());
        core.device_lost("a Provider thread panicked");
        assert_eq!(switch_row(&core, 0)["outcome"], if stopped_before_b { "cancelled" } else { "pending" }, "stopped before b {stopped_before_b}");
        core.settle_unresolved_switches();
        assert_eq!(switch_row(&core, 0)["outcome"], o::switch_outcome_at_close(B, stopped_before_b.then_some(B)).as_str(), "stopped before b {stopped_before_b}");
    }
}

#[test]
fn c2_no_row_is_left_pending_when_the_run_ends() {
    // The cells above, each followed by the Run's end (`Streams::end`) and the shutdown: a row is settled or the Run's end settles it. (A row stuck pending would make
    // the Manifest's `switches` say nothing about a switch the Run ended on.)
    let mut cells: Vec<(&str, SwitchRig)> = Vec::new();
    cells.push(("host late", {
        let mut rig = switched(Vec::new());
        at(&rig, 15 * MS);
        rig.tx.read_list();
        rig.tx.step();
        rig
    }));
    cells.push(("issued, device time short of b", {
        let mut rig = switched(Vec::new());
        let mut ms = 3;
        while !boundary_send_is_on_the_device(&rig) {
            run(&mut rig, ms, ms);
            ms += 1;
        }
        rig
    }));
    cells.push(("not issued", {
        let mut rig = switched(Vec::new());
        run(&mut rig, 3, 3);
        rig
    }));
    for (case, rig) in cells {
        end_streams(&rig.core, 15 * MS, false, false);
        let (core, _) = shutdown(rig);
        let row = switch_row(&core, 0)["outcome"].as_str().unwrap().to_owned();
        assert_ne!(row, "pending", "{case}");
    }
}

#[test]
fn c2_s15_a_failed_reopen_cancels_even_a_sent_switch() {
    // UR-22 (F8): the reopen that ends the repeat ends the clock, and with it the switch, whatever became of its boundary send, also one
    // that a Stop below b is holding for device time (UR-38: it is `cancelled`, as a loss would have made it).
    for stopped in [false, true] {
        let mut rig = switched(vec![FakeFault::TxOpenFails(1)]);
        let mut ms = 3;
        while !boundary_send_is_on_the_device(&rig) {
            run(&mut rig, ms, ms);
            ms += 1;
            assert!(ms < 12, "{:?}", rig.device.calls());
        }
        if stopped {
            stop_receipt(&mut rig);
            rig.tx.read_list();
            assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "the Stop is before b");
        }
        rig.tx.chain().mark_reopen_needed();
        assert!(rig.tx.pass(), "the reopen runs");
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "Stop {stopped}");
    }
}

/// The owner's turn as the run loop makes it, on a rig whose clock the test thread holds as a participant (so `turn` cannot take the rig whole).
fn owner_turn(tx: &mut Tx) {
    for _ in 0..8 {
        drain_tx(tx);
        if !tx.step() {
            break;
        }
    }
}

#[test]
fn c2_s15b_a_retried_recovery_settles_a_pending_switch_after_the_window_and_a_failed_one_cancels_it() {
    // UR-22 (F8) with the retried flush: the recovery now takes 100 ms of device time per lost attempt, so device time passes b while it runs. At each
    // retry boundary (the Kernel's clock moved to the device's, as the run loop's reads would) the switch is still `pending`: the recovery settles
    // nothing. A recovery that confirms leaves it to the owner's next turn, which applies it (device time is past b and the boundary send was whole), as
    // the no-retry control does; one that fails after three attempts cancels it, although the boundary sample was on the device and device time passed b.
    let flush_ordinal = {
        let mut rig = switched(Vec::new());
        let mut ms = 3;
        while !boundary_send_is_on_the_device(&rig) {
            run(&mut rig, ms, ms);
            ms += 1;
            assert!(ms < 12, "{:?}", rig.device.calls());
        }
        // The reopen's first flush is the next send that carries samples.
        rig.device.calls().iter().filter(|call| call.starts_with("tx_send 0 ") && !call.contains(" n=0 ")).count()
    };
    let control_row = {
        let mut rig = switched(Vec::new());
        let mut ms = 3;
        while !boundary_send_is_on_the_device(&rig) {
            run(&mut rig, ms, ms);
            ms += 1;
        }
        rig.tx.chain().mark_reopen_needed();
        assert!(rig.tx.pass(), "the reopen runs");
        assert_ne!(switch_row(&rig.core, 0)["outcome"], "cancelled", "control: a confirmed recovery cancels nothing");
        device_at(&rig.core, &rig.time, &rig.device, 20 * MS);
        owner_turn(&mut rig.tx);
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied", "control");
        switch_row(&rig.core, 0)
    };
    let drops = |from: usize, count: usize| (from..from + count).map(FakeFault::DropAckOfSend).collect::<Vec<_>>();
    // The faults drop the acknowledgements of the first flushes; a retry boundary lies before every attempt but the first.
    for (case, faults, boundaries, recovers) in
        [("attempt 2 confirms", drops(flush_ordinal, 1), 1, true), ("attempt 3 confirms", drops(flush_ordinal, 2), 2, true), ("three unconfirmed", drops(flush_ordinal, 3), 2, false)]
    {
        let mut rig = switched(faults);
        let mut ms = 3;
        while !boundary_send_is_on_the_device(&rig) {
            run(&mut rig, ms, ms);
            ms += 1;
            assert!(ms < 12, "{case}: {:?}", rig.device.calls());
        }
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "{case}: sent, device time short of b");
        let gates = Arc::new(Mutex::new(Vec::<(i64, String)>::new()));
        {
            let (core, time, device, gates) = (Arc::downgrade(&rig.core), Arc::downgrade(&rig.time), Arc::downgrade(&rig.device), gates.clone());
            probe(&rig.core, "t3_between", move |_| {
                let (Some(core), Some(time), Some(device)) = (core.upgrade(), time.upgrade(), device.upgrade()) else { return };
                let now = device.time_now().unwrap();
                device_at(&core, &time, &device, now);
                gates.lock().unwrap().push((now, switch_row(&core, 0)["outcome"].as_str().unwrap().to_owned()));
            });
        }
        rig.tx.chain().mark_reopen_needed();
        assert!(rig.tx.pass(), "{case}: the reopen runs");
        let gates = gates.lock().unwrap().clone();
        assert_eq!(gates.len(), boundaries, "{case}: {gates:?}");
        for (device_time, outcome) in &gates {
            assert!(*device_time >= rig.clock.instant(B), "{case}: device time {device_time} is past b at the gate");
            assert_eq!(outcome, "pending", "{case}: the recovery has not settled the switch");
        }
        if recovers {
            assert_ne!(switch_row(&rig.core, 0)["outcome"], "cancelled", "{case}");
            owner_turn(&mut rig.tx);
            assert_eq!(switch_row(&rig.core, 0), control_row, "{case}: the same row as the no-retry control");
        } else {
            assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "{case}: the failed recovery cancels it");
            assert!(rig.device.time_now().unwrap() >= rig.clock.instant(B), "{case}: although device time passed b");
        }
    }
}

#[test]
fn c2_s17b_two_accepted_switches_settle_each_on_its_own_boundary_send() {
    // m2: S1 is taken and device time passes b1: applied. S2 is accepted behind it; S1's next send (its repeat's continuation) completes
    // while S2 is pending and marks nothing of S2's; S2's own boundary send fails: [applied, uncertain], each with its action id, and
    // S2's predecessor is S1.
    // R's and S1's sends are the first ten (S1's boundary send is the sixth); the send that carries S2's boundary 28 000 is the twelfth.
    let mut rig = switch_rig_with(vec![o::ramp(17, 1_000), o::ramp(18, 400), o::ramp(19, 300)], vec![FakeFault::FailSend(11)]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 1);
    at(&rig, 2 * MS);
    book_switch(&mut rig, 2, 2_000, 1);
    turn(&mut rig);
    run(&mut rig, 3, 16);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied");
    book_switch(&mut rig, 3, 16_000, 2);
    assert!(refusals(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
    run(&mut rig, 17, 40);
    let (first, second) = (switch_row(&rig.core, 0), switch_row(&rig.core, 1));
    assert_eq!((first["action_id"].as_u64(), first["outcome"].as_str()), (Some(2), Some("applied")), "{first} {second}");
    assert_eq!((second["action_id"].as_u64(), second["predecessor"].as_u64(), second["outcome"].as_str()), (Some(3), Some(2), Some("uncertain")), "{first} {second}");
}
