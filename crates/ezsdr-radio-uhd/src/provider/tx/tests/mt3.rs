//! Maintenance table 3, sub-step B (plan `2026-11-14-mt3/test-plan-v3.md` §3.2): a TimeError retires and names the device bursts it reaches, an Underflow
//! retires them silently, an abandon retires nothing, and a late report that reaches the open one-shot device burst abandons it (UR-22, UR-28).
//!
//! Units: one sample is 1 us (the rig's 1 MS/s), 200 root ticks; `MS` is a millisecond in root ticks.

use super::e1::{MS, at, book, rows};
use super::e2a::{closing_sends, late_report_after_a_closed_burst, refused_ids};
use super::*;
use crate::provider::tx::oracle as o;

/// A transmit owner with no device traffic, on a 1 MS/s clock, and the events it emits.
pub(super) fn unit(channels: u32) -> (Tx, Clock, Arc<EventCollector>, std::sync::mpsc::Sender<TxCmd>, Arc<Core>) {
    let (core, _, _, events) = super::super::super::test_support::rig();
    let clock = core.register(Dir::Tx, 200, 0).unwrap();
    let (to_tx, cmds) = std::sync::mpsc::channel();
    let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(channels));
    (tx, clock, events, to_tx, core)
}

pub(super) fn report(tx: &mut Tx, code: TxCode, clock: &Clock, sample: i64, channel: usize) {
    tx.report(TxReport { code, tick: Some(clock.instant(sample)), channel });
}

/// The `(target tick, late_by_ns)` of every `late_at_device` event since the last call.
fn told(events: &EventCollector) -> Vec<(i64, i64)> {
    time_errors(events)
        .iter()
        .filter(|event| event.payload["outcome"] == "late_at_device")
        .map(|event| (event.payload["target"]["ticks"].as_i64().unwrap(), event.payload["late_by_ns"].as_i64().unwrap()))
        .collect()
}

pub(super) fn targets(tx: &Tx) -> Vec<i64> {
    let domain = tx.clock.expect("the owner has a clock").domain;
    tx.unacked.iter().map(|pending| pending.target.ticks_in(domain).unwrap()).collect()
}

fn dropped_notes(core: &Core) -> usize {
    lock(&core.rec).rejected.iter().filter(|row| row["reason"] == "UR-22: the device dropped the burst late; burst not resumed").count()
}

#[test]
fn mt3_a_time_error_retires_every_pending_burst_it_reaches() {
    // U1 (UR-28): one report at 25 000 reaches A (10 000) and B (20 000) and not C: each gets its event, from its own start, oldest first.
    let (mut tx, clock, events, _to_tx, _core) = unit(1);
    for k in [10_000, 20_000, 1_000_000] {
        tx.remember_burst(TimePoint::new(clock.domain, k));
    }
    report(&mut tx, TxCode::TimeError, &clock, 25_000, 0);
    assert_eq!(told(&events), [(10_000, 15_000_000), (20_000, 5_000_000)]);
    assert_eq!(targets(&tx), [1_000_000], "A and B are retired, C stands");
    // The reach is at or before the tick: a tick at B's first sample retires B, one before it does not.
    for (tick, expect) in [(20_000, vec![10_000, 20_000]), (19_999, vec![10_000])] {
        let (mut tx, clock, events, _to_tx, _core) = unit(1);
        for k in [10_000, 20_000] {
            tx.remember_burst(TimePoint::new(clock.domain, k));
        }
        report(&mut tx, TxCode::TimeError, &clock, tick, 0);
        assert_eq!(told(&events).iter().map(|(k, _)| *k).collect::<Vec<_>>(), expect, "tick {tick}");
    }
}

#[test]
fn mt3_an_underflowed_closed_burst_is_never_told_late() {
    // U7 (UR-28, A'): A is closed (no device burst open) and pending; an Underflow at 10 500 retires it silently, so the TimeError that follows names
    // nothing. Without the Underflow the same TimeError names it.
    let (mut tx, clock, events, _to_tx, _core) = unit(1);
    tx.remember_burst(TimePoint::new(clock.domain, 10_000));
    report(&mut tx, TxCode::Underflow, &clock, 10_500, 0);
    assert!(tx.unacked.is_empty(), "the Underflow retires the burst it reaches");
    report(&mut tx, TxCode::TimeError, &clock, 10_600, 0);
    assert_eq!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TX_UNDERFLOW).count(), 1);
    assert!(told(&events).is_empty(), "an underflowed burst is not late");
    let (mut tx, clock, events, _to_tx, _core) = unit(1);
    tx.remember_burst(TimePoint::new(clock.domain, 10_000));
    report(&mut tx, TxCode::TimeError, &clock, 10_600, 0);
    assert_eq!(told(&events), [(10_000, 600_000)]);
}

#[test]
fn mt3_a_two_channel_burst_gets_one_event_and_one_channel_alone_retires_it() {
    // U9 (UR-28, m1): the burst's event comes with the first channel's TimeError and the second adds none; a burst acknowledged on channel 0 and late
    // on channel 1 alone is told once.
    let (mut tx, clock, events, _to_tx, _core) = unit(2);
    tx.remember_burst(TimePoint::new(clock.domain, 10_000));
    report(&mut tx, TxCode::TimeError, &clock, 10_500, 0);
    assert_eq!(told(&events).len(), 1);
    assert_eq!(tx.unacked.len(), 1, "channel 1 is still pending");
    report(&mut tx, TxCode::TimeError, &clock, 10_500, 1);
    assert!(told(&events).is_empty(), "the second channel's report adds no event");
    assert!(tx.unacked.is_empty());
    let (mut tx, clock, events, _to_tx, _core) = unit(2);
    tx.remember_burst(TimePoint::new(clock.domain, 10_000));
    report(&mut tx, TxCode::BurstAck, &clock, 10_100, 0);
    assert!(told(&events).is_empty());
    report(&mut tx, TxCode::TimeError, &clock, 10_500, 1);
    assert_eq!(told(&events).len(), 1, "channel 1 alone was late");
    assert!(tx.unacked.is_empty());
    // O4: an Underflow on channel 0 retires and marks the burst, so channel 1's TimeError alone names nothing.
    let (mut tx, clock, events, _to_tx, _core) = unit(2);
    tx.remember_burst(TimePoint::new(clock.domain, 10_000));
    report(&mut tx, TxCode::Underflow, &clock, 10_500, 0);
    report(&mut tx, TxCode::TimeError, &clock, 10_600, 1);
    assert!(told(&events).is_empty(), "a burst that underflowed on one channel is never named late");
    assert!(tx.unacked.is_empty());
}

#[test]
fn mt3_a_played_burst_whose_ack_was_lost_is_named_late_by_a_later_time_error() {
    // U14 (UR-28, m2; KL-45): A played but its ack was lost, so it is still pending when B's TimeError reaches both: A is named too. An ack that came
    // retires A first, and then only B is named.
    let (mut tx, clock, events, _to_tx, _core) = unit(1);
    for k in [10_000, 20_000] {
        tx.remember_burst(TimePoint::new(clock.domain, k));
    }
    report(&mut tx, TxCode::TimeError, &clock, 25_000, 0);
    assert_eq!(told(&events).iter().map(|(k, _)| *k).collect::<Vec<_>>(), [10_000, 20_000]);
    let (mut tx, clock, events, _to_tx, _core) = unit(1);
    for k in [10_000, 20_000] {
        tx.remember_burst(TimePoint::new(clock.domain, k));
    }
    report(&mut tx, TxCode::BurstAck, &clock, 15_000, 0);
    report(&mut tx, TxCode::TimeError, &clock, 25_000, 0);
    assert_eq!(told(&events).iter().map(|(k, _)| *k).collect::<Vec<_>>(), [20_000]);
}

#[test]
fn mt3_a_sequence_error_retires_nothing() {
    // UR-28: a sequence error's tick is unmeasured, so it retires and names nothing.
    let (mut tx, clock, events, _to_tx, _core) = unit(1);
    tx.remember_burst(TimePoint::new(clock.domain, 10_000));
    report(&mut tx, TxCode::SeqError, &clock, 20_000, 0);
    assert_eq!(targets(&tx), [10_000]);
    assert!(told(&events).is_empty());
}

/// A (2 000 samples at 10 000) and B (1 000 at 12 000, which continues A's device burst, RM-15) handed whole by 7 ms: one device burst, one start.
fn joined_rig(faults: Vec<FakeFault>) -> SwitchRig {
    let mut rig = switch_rig_with(vec![o::ramp(5, 2_000), o::ramp(6, 1_000)], faults);
    book(&mut rig, 1, 0, 10_000);
    book(&mut rig, 2, 1, 12_000);
    run(&mut rig, 0, 7);
    assert_eq!(rig.device.calls().iter().filter(|call| call.contains("sob=true")).count(), 1, "one device burst: {:?}", rig.device.calls());
    rig
}

#[test]
fn mt3_a_late_device_burst_names_every_kernel_burst_joined_into_it() {
    // U3 (UR-28, S4): a TimeError at 10 500 reaches the device burst A and B share: both are named, each from the device burst's start (not B's), and the
    // report abandons the device burst (UR-22).
    let mut rig = joined_rig(Vec::new());
    assert_eq!(rig.tx.unacked.iter().map(|pending| pending.joined.len()).collect::<Vec<_>>(), [1], "B joined A's correlation");
    report(&mut rig.tx, TxCode::TimeError, &rig.clock, 10_500, 0);
    assert_eq!(told(&rig.events), [(10_000, 500_000), (12_000, 500_000)]);
    assert!(rig.tx.unacked.is_empty());
    assert_eq!(dropped_notes(&rig.core), 1, "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(closing_sends(&rig.device).len(), 1, "the abandon closes the device burst: {:?}", rig.device.calls());
}

#[test]
fn mt3_a_joined_burst_whose_first_send_failed_is_still_named() {
    // U4 (Q6): B's first send (the third) fails, or comes up short having taken half, and may have gone: B is still named with A.
    for fault in [FakeFault::FailSend(2), FakeFault::ShortSend(2)] {
        let mut rig = joined_rig(vec![fault.clone()]);
        report(&mut rig.tx, TxCode::TimeError, &rig.clock, 10_500, 0);
        assert_eq!(told(&rig.events).iter().map(|(k, _)| *k).collect::<Vec<_>>(), [10_000, 12_000], "{fault:?}: {:?}", rig.device.calls());
    }
}

/// A 20 000-sample finite burst booked at 20 000 whose first send reaches a device 1 ms past its start: the owner's manual clock stays at 11 ms, the device
/// moves to 21 ms (the pattern of `ur_22_a_first_send_cut_short_keeps_the_reports_paired`). Returns the rig after the owner's steps, no report read.
fn late_burst() -> SwitchRig {
    let mut rig = switch_rig(vec![o::ramp(1, 20_000)]);
    book(&mut rig, 1, 0, 20_000);
    at(&rig, 11 * MS);
    rig.device.set_time(21 * MS);
    for _ in 0..8 {
        drain_tx(&mut rig.tx);
        if !rig.tx.step() {
            break;
        }
    }
    rig
}

fn payload_sends(device: &FakeDevice) -> usize {
    sends(device).iter().filter(|send| send.0 > 2).count()
}

fn async_codes(core: &Core, code: &str) -> usize {
    lock(&core.rec).device_async.iter().filter(|row| row["code"] == code).count()
}

#[test]
fn mt3_a_finite_burst_handed_late_transmits_nothing_and_is_told_once() {
    // U5 (UR-22, UR-28; S1): the device sees the first send 1 ms after its start. The fake drops the burst whole; the Module's report read names it once,
    // ends it at the handed cursor and feeds nothing more.
    let mut rig = late_burst();
    assert_eq!(rig.tx.unacked.len(), 1, "the start is correlated before any report");
    rig.tx.reports(Wall::ZERO);
    assert_eq!(told(&rig.events), [(20_000, 1_000_000)]);
    assert!(rig.tx.unacked.is_empty());
    assert!(rig.device.transmitted(rig.clock.instant(20_000), 21_000).iter().all(Option::is_none), "{:?}", rig.device.calls());
    assert_eq!(async_codes(&rig.core, "BurstAck"), 0);
    assert_eq!(rows(&rig)[0]["end"], "stop");
    assert_eq!(dropped_notes(&rig.core), 1, "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(rig.tx.open_next(), None);
    let (payload, errors) = (payload_sends(&rig.device), async_codes(&rig.core, "TimeError"));
    run(&mut rig, 12, 120);
    rig.tx.reports(Wall::ZERO);
    assert_eq!((payload_sends(&rig.device), async_codes(&rig.core, "TimeError")), (payload, errors), "nothing more is fed: {:?}", rig.device.calls());
    assert!(told(&rig.events).is_empty(), "told once");
}

#[test]
fn mt3_the_expired_timeline_abandon_runs_before_the_first_time_error_and_the_burst_is_still_named() {
    // U6 (Fable M1): the owner's own expiry check abandons the late burst before its first report is read; the abandon retires no correlation, so the
    // report read after it still names the burst, and no second note is made.
    let mut rig = late_burst();
    rig.time.advance_to(rig.core.at(22 * MS)).unwrap();
    assert!(rig.tx.step());
    assert_eq!(rig.tx.open_next(), None, "the expiry check abandoned the burst");
    assert_eq!(rig.tx.unacked.len(), 1, "an abandon never touches the correlation");
    rig.tx.reports(Wall::ZERO);
    assert_eq!(told(&rig.events), [(20_000, 1_000_000)]);
    assert!(rig.tx.unacked.is_empty());
    let notes: Vec<_> = lock(&rig.core.rec).rejected.iter().filter_map(|row| row["reason"].as_str().map(str::to_owned)).collect();
    assert_eq!(notes, ["UR-22: the transmit timeline expired; burst not resumed"], "no second note: the report reaches a closed burst");
}

#[test]
fn mt3_a_time_error_abandons_the_open_device_burst_only_when_its_tick_reaches_it() {
    // U10 (UR-22, UR-28): A (500 samples at 10 000) is closed and pending, B (from 11 000) is the open device burst. A tick before B's first sample is A's
    // (as for an underflow, #93): A is named, B plays on. A tick at B's first sample is B's: both are named and B is abandoned at its cursor.
    let rig = late_report_after_a_closed_burst(TxCode::TimeError, 10_999);
    assert_eq!(told(&rig.events).iter().map(|(k, _)| *k).collect::<Vec<_>>(), [10_000]);
    assert_eq!((dropped_notes(&rig.core), closing_sends(&rig.device).len()), (0, 1), "{:?}", rig.device.calls());
    assert!(rig.tx.open_next().is_some(), "B is still being handed");
    assert_eq!(targets(&rig.tx), [11_000]);
    let rig = late_report_after_a_closed_burst(TxCode::TimeError, 11_000);
    assert_eq!(told(&rig.events).iter().map(|(k, _)| *k).collect::<Vec<_>>(), [10_000, 11_000]);
    assert_eq!((dropped_notes(&rig.core), closing_sends(&rig.device).len()), (1, 2), "{:?}", rig.device.calls());
    assert_eq!(rig.tx.open_next(), None);
    // A burst a Stop closed is not abandoned by its late report: it is named, and no second close is sent.
    let mut rig = switch_rig(vec![o::ramp(7, 20_000)]);
    book(&mut rig, 1, 0, 10_000);
    run(&mut rig, 0, 7);
    stop(&mut rig);
    let closes = closing_sends(&rig.device);
    assert_eq!(closes.len(), 1, "the Stop closed the device burst: {:?}", rig.device.calls());
    report(&mut rig.tx, TxCode::TimeError, &rig.clock, 10_500, 0);
    assert_eq!(told(&rig.events).len(), 1);
    assert_eq!((dropped_notes(&rig.core), closing_sends(&rig.device)), (0, closes), "no abandon, no stray close");
    // With nothing open, or nothing pending, a stale report only appears in the raw rows.
    let (mut tx, clock, events, _to_tx, core) = unit(1);
    report(&mut tx, TxCode::TimeError, &clock, 10_500, 0);
    assert_eq!((told(&events).len(), dropped_notes(&core)), (0, 0));
}

#[test]
fn mt3_a_stop_after_a_late_abandon_sends_nothing_and_the_region_stays() {
    // U12 (UR-22, KL-39): the late burst's abandon is the end of its device burst. The finite burst keeps its whole booked region to its front (40 004),
    // as after any abandon; a Stop after it sends nothing, tells nothing and makes no second record.
    let mut rig = switch_rig(vec![o::ramp(1, 20_000), o::ramp(2, 100)]);
    book(&mut rig, 1, 0, 20_000);
    at(&rig, 11 * MS);
    rig.device.set_time(21 * MS);
    for _ in 0..8 {
        drain_tx(&mut rig.tx);
        if !rig.tx.step() {
            break;
        }
    }
    rig.tx.reports(Wall::ZERO);
    assert_eq!(told(&rig.events).len(), 1);
    book(&mut rig, 2, 1, 40_003);
    assert_eq!(refused_ids(&rig.events), [2], "{:?}", lock(&rig.core.rec).rejected);
    book(&mut rig, 3, 1, 40_004);
    assert!(refused_ids(&rig.events).is_empty());
    let before = rig.device.calls().len();
    stop(&mut rig);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(rig.device.calls().len(), before, "{:?}", rig.device.calls());
    assert!(told(&rig.events).is_empty());
    assert_eq!(rows(&rig).len(), 1);
}

/// The owner's steps until none is left to take (commands read first).
pub(super) fn go(rig: &mut SwitchRig) {
    for _ in 0..8 {
        drain_tx(&mut rig.tx);
        if !rig.tx.step() {
            break;
        }
    }
}

#[test]
fn mt3_a_burst_booked_inside_a_late_burst_s_drain_is_told_and_one_past_it_plays() {
    // U11 (UR-22, UR-28, UR-33; N1): A (500 samples at 20 000) reaches a device that is 100 us past its start, so the fake drops it whole and drains the
    // samples handed. B, booked after A's front (20 520), starts inside that drain: the fake drops it too, B gets its own event from its own start and is
    // abandoned. C, a millisecond and more past the drain, plays whole and is acknowledged with no event. The fake's drain is stricter than the X310's
    // (UR-33): nothing here claims the device's boundary.
    let mut rig = switch_rig(vec![o::ramp(1, 500), o::ramp(2, 100), o::ramp(3, 200)]);
    book(&mut rig, 1, 0, 20_000);
    at(&rig, 11 * MS);
    rig.device.set_time(20_100 * 200);
    go(&mut rig);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(told(&rig.events), [(20_000, 100_000)]);
    book(&mut rig, 2, 1, 20_520);
    book(&mut rig, 3, 2, 22_000);
    go(&mut rig);
    rig.tx.reports(Wall::ZERO);
    let b = told(&rig.events);
    assert_eq!(b.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [20_520], "B is told, from its own start: {b:?}");
    assert!(rig.device.transmitted(rig.clock.instant(20_520), 100).iter().all(Option::is_none), "B is dropped whole: {:?}", rig.device.calls());
    rig.time.advance_to(rig.core.at(13 * MS)).unwrap();
    go(&mut rig);
    for ms in [21, 25, 30] {
        at(&rig, ms * MS);
        go(&mut rig);
    }
    rig.tx.reports(Wall::ZERO);
    assert!(told(&rig.events).is_empty(), "C is not late");
    assert!(rig.device.transmitted(rig.clock.instant(22_000), 200).iter().all(Option::is_some), "C plays whole: {:?}", rig.device.calls());
    assert_eq!(async_codes(&rig.core, "BurstAck"), 1, "C is acknowledged");
    assert!(rig.tx.unacked.is_empty());
}

#[test]
fn mt3_a_late_abandon_whose_close_fails_has_its_event_and_recovers() {
    // U13 (UR-22, UR-28, #75): the late report's abandon sends the two-sample close, and the device takes one. The event is already made; the chain needs
    // the reopen, whose T3 flush ack is a recovery's own (`tx_flush`) and names nothing. The new streamer starts with no correlation, and the next burst
    // plays whole with no stale TimeError naming it.
    let mut rig = switch_rig_with(vec![o::ramp(1, 20_000), o::ramp(2, 100)], vec![FakeFault::ShortSend(1)]);
    book(&mut rig, 1, 0, 20_000);
    at(&rig, 11 * MS);
    rig.device.set_time(21 * MS);
    go(&mut rig);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(told(&rig.events), [(20_000, 1_000_000)]);
    assert!(rig.tx.with_device(|device| device.needs_reopen()), "{:?}", lock(&rig.core.rec).rejected);
    assert!(rig.tx.pass(), "the recovery runs on this turn");
    assert!(!rig.tx.with_device(|device| device.needs_reopen()));
    assert!(rig.tx.unacked.is_empty(), "the new streamer starts empty");
    assert!(lock(&rig.core.rec).device_async.iter().any(|row| row["tx_flush"] == true && row["code"] == "BurstAck"), "{:?}", lock(&rig.core.rec).device_async);
    assert!(told(&rig.events).is_empty(), "the flush's reports name no burst");
    book(&mut rig, 2, 1, 41_000);
    for ms in [32, 38, 41, 45, 50] {
        at(&rig, ms * MS);
        go(&mut rig);
        rig.tx.reports(Wall::ZERO);
    }
    assert!(rig.device.transmitted(rig.clock.instant(41_000), 100).iter().all(Option::is_some), "{:?}", rig.device.calls());
    assert!(told(&rig.events).is_empty(), "no stale report names the next burst");
    assert!(rig.tx.unacked.is_empty(), "the next burst is acknowledged");
}

#[test]
fn mt3_the_fake_drops_a_late_joined_device_burst_whole_and_both_bursts_are_named() {
    // S4 on the fake itself (the Session case `ur_28_two_joined_bursts_a_late_report_reaches_are_both_told` has to script its report: its SlowSend
    // expires the timeline before B can join): A (2 000 samples at 20 000) and B (1 000 at 22 000) are handed as one device burst while the device is a
    // millisecond past A's start and the owner's clock is not. Nothing of either plays, and the one report read names both.
    let mut rig = switch_rig(vec![o::ramp(1, 2_000), o::ramp(2, 1_000)]);
    book(&mut rig, 1, 0, 20_000);
    book(&mut rig, 2, 1, 22_000);
    at(&rig, 11 * MS);
    rig.device.set_time(21 * MS);
    for ms in [11, 12, 13] {
        rig.time.advance_to(rig.core.at(ms * MS)).unwrap();
        go(&mut rig);
    }
    assert_eq!(rig.device.calls().iter().filter(|call| call.contains("sob=true")).count(), 1, "one device burst: {:?}", rig.device.calls());
    assert_eq!(rig.tx.unacked.iter().map(|pending| pending.joined.len()).collect::<Vec<_>>(), [1], "B joined before any report was read");
    rig.tx.reports(Wall::ZERO);
    assert_eq!(told(&rig.events), [(20_000, 1_000_000), (22_000, 1_000_000)]);
    assert!(rig.device.transmitted(rig.clock.instant(20_000), 3_000).iter().all(Option::is_none), "{:?}", rig.device.calls());
    assert_eq!(async_codes(&rig.core, "BurstAck"), 0);
}
