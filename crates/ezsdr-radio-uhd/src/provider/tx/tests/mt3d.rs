//! Maintenance table 3, sub-step D1 (plan `2026-11-14-mt3/test-plan-v6.md` §9): a repeat whose device burst the device dropped late or starved resumes on its
//! lattice at `R`, the first lattice point at or after `max(busy + m, now + lead, front)` (UR-22), and says so (`resumes_at`, one `timing` row, split records).
//!
//! Units: one sample is 1 us (the rig's 1 MS/s), 200 root ticks; `MS` is a millisecond in root ticks; the repeat `W0` has a period of 1 000 samples and
//! starts at 20 000, so its lattice is `20 000 + 1 000 j`; `m` is 10 ms, 10 000 samples. Every expected `R` is worked out by hand in its test's comment from
//! the `r` (the owner's clock when a send returned), `x` (the send's first sample) and `n` (its samples) the test scripts: `busy' = max(busy, r, x) + n`.
//! Nothing here reads a wall clock.

use super::e1::{MS, at, book, rows};
use super::e2a::closing_sends;
use super::mt3::{go, report, unit};
use super::*;
use crate::provider::tx::oracle as o;

/// W0 (the ramp 1) and a finite 100-sample burst (the ramp 2).
fn repeat_rig() -> SwitchRig {
    switch_rig(vec![o::ramp(1, 1_000), o::ramp(2, 100)])
}

/// `(tag, resumes_at)` of every `late_at_device` and `TX_UNDERFLOW` event since the last call: `resumes_at` is the sample.
fn emitted(events: &EventCollector) -> Vec<(&'static str, Option<i64>)> {
    events
        .drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0))
        .into_iter()
        .filter_map(|event| {
            let tag = match event.kind.as_str() {
                kind if kind == ezsdr_radio::kinds::TX_UNDERFLOW => "underflow",
                kind if kind == ezsdr_radio::kinds::TIME_ERROR && event.payload["outcome"] == "late_at_device" => "late",
                _ => return None,
            };
            Some((tag, event.payload["resumes_at"]["ticks"].as_i64()))
        })
        .collect()
}

/// The `tx_resume` rows of the `timing` section.
fn resumes(core: &Core) -> Vec<serde_json::Value> {
    lock(&core.rec).timing.iter().filter(|row| row["what"] == "tx_resume").cloned().collect()
}

/// The fake's transmit calls as `(samples, first sample's tick, start of burst, end of burst)`.
fn tx_sends(device: &FakeDevice) -> Vec<(i64, Option<i64>, bool, bool)> {
    let field = |call: &str, name: &str| call.split_whitespace().find_map(|word| word.strip_prefix(name)).map(str::to_owned);
    device
        .calls()
        .iter()
        .filter(|call| call.starts_with("tx_send "))
        .filter_map(|call| Some((field(call, "n=")?.parse().ok()?, field(call, "at=")?.parse().ok(), field(call, "sob=")? == "true", field(call, "eob=")? == "true")))
        .collect()
}

/// The owner's steps until `n` sends were made (no report read between them).
fn hand(rig: &mut SwitchRig, n: usize) {
    for _ in 0..8 {
        if tx_sends(&rig.device).len() >= n {
            return;
        }
        drain_tx(&mut rig.tx);
        rig.tx.step();
    }
    panic!("{n} sends were not made: {:?}", rig.device.calls());
}

/// The notes that say a burst was not resumed.
fn ended_notes(core: &Core) -> usize {
    lock(&core.rec).rejected.iter().filter(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("not resumed"))).count()
}

/// W0 booked at 20 000, whose first send (1 000 samples, to the 21 ms window edge) reaches a device already at 21 ms and is dropped late. The owner is
/// brought to the device's time (§9.0) and no report is read: the fake queued its TimeError inside the send.
fn late_repeat() -> SwitchRig {
    late_repeat_with(Vec::new())
}

fn late_repeat_with(faults: Vec<FakeFault>) -> SwitchRig {
    let mut rig = switch_rig_with(vec![o::ramp(1, 1_000), o::ramp(2, 100)], faults);
    book_repeat(&mut rig, 1, 20_000, 0);
    at(&rig, 11 * MS);
    rig.device.set_time(21 * MS);
    go(&mut rig);
    at(&rig, 21 * MS);
    rig
}

/// `late_repeat`, its report read (the repeat resumes at 32 000) and its part at 32 000 handed, the owner and the device at 24 ms.
fn resumed_part() -> SwitchRig {
    let mut rig = late_repeat();
    rig.tx.reports(Wall::ZERO);
    assert_eq!(rig.tx.open_next(), Some(32_000), "paused until its resume point");
    for ms in [23, 24] {
        at(&rig, ms * MS);
        go(&mut rig);
    }
    assert!(tx_sends(&rig.device).iter().any(|send| send.1 == Some(rig.clock.instant(32_000)) && send.2), "the part starts at 32 000: {:?}", rig.device.calls());
    emitted(&rig.events);
    rig
}

#[test]
fn mt3d_a_late_started_repeat_resumes_on_its_lattice_after_busy_plus_m() {
    // V1, V8, V10 (UR-22): the device sees W0's first send (1 000 samples at 20 000) at 21 ms and drops it. The owner is at 21 ms when it reads the report.
    // busy: send 1 (r = 11 000, x = 20 000, n = 1 000) = max(0, 11 000, 20 000) + 1 000 = 21 000; the close (r = 21 000, x = H = 21 000, n = 2) = 21 002.
    // B = max(21 002 + 10 000, 21 000 + 2 000, front 21 017) = 31 002, so R = 32 000 (the first 20 000 + 1 000 j at or after it).
    let mut rig = late_repeat();
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events), [("late", Some(32_000))], "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(rig.tx.open_next(), Some(32_000), "the repeat is still the open booking, paused until R");
    assert_eq!(closing_sends(&rig.device), [format!("tx_send 0 n=2 at={} sob=false eob=true", rig.clock.instant(21_000))], "{:?}", rig.device.calls());
    let row = resumes(&rig.core);
    assert_eq!(row.len(), 1);
    assert_eq!((row[0]["cause"].as_str(), row[0]["from"].as_i64(), row[0]["resumes_at"].as_i64(), row[0]["stream"].as_i64()), (Some("late_at_device"), Some(21_000), Some(32_000), Some(0)), "{row:?}");
    assert_eq!((ended_notes(&rig.core), lock(&rig.core.rec).rejected.len()), (0, 0), "no note: {:?}", lock(&rig.core.rec).rejected);
    // V8: nothing is sent before R; the part starts there, on phase 0, with at least 32 samples.
    let before = tx_sends(&rig.device).len();
    at(&rig, 22 * MS);
    go(&mut rig);
    assert_eq!(tx_sends(&rig.device).len(), before, "R + 32 is not inside the window yet: {:?}", rig.device.calls());
    for ms in [23, 24, 33] {
        at(&rig, ms * MS);
        go(&mut rig);
    }
    let part = tx_sends(&rig.device).into_iter().find(|send| send.2 && send.1 == Some(rig.clock.instant(32_000))).expect("the part's start of burst at R");
    assert!(part.0 >= 32 && !part.3, "{part:?}");
    assert!(rig.device.transmitted(rig.clock.instant(20_000), 12_000).iter().all(Option::is_none), "silent from 20 000 to R: {:?}", rig.device.calls());
    assert_eq!(rig.device.transmitted(rig.clock.instant(32_000), 1_000), o::ramp(1, 1_000).into_iter().map(Some).collect::<Vec<_>>(), "W0 from its first sample at R");
    // Split records: the first ends in a discontinuity with what was handed; the second opens at R and is still open.
    let rows = rows(&rig);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0]["end"].as_str(), rows[0]["target"]["ticks"].as_i64(), rows[0]["samples"].as_u64()), (Some("discontinuity"), Some(20_000), Some(1_000)), "{rows:?}");
    assert!(emitted(&rig.events).is_empty());
    assert!(lock(&rig.core.rec).bursts.iter().all(|row| row["late_by"].is_null() || row["target"]["ticks"] == 20_000));
    assert_eq!(resumes(&rig.core).len(), 1, "one timing row per resume");
    // The next record opens with the part (SC-24): ending the repeat writes it.
    stop(&mut rig);
    let rows = super::e1::rows(&rig);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!((rows[1]["end"].as_str(), rows[1]["target"]["ticks"].as_i64()), (Some("stop"), Some(32_000)), "{rows:?}");
    assert!(rows[1]["late_by"].is_null() && rows[1]["requested_target"].is_null(), "the part carries nothing of the original start's late policy: {rows:?}");
}

#[test]
fn mt3d_an_underflow_resumes_a_repeat_and_names_the_span_with_busy_from_x() {
    // V2, RV-03b (UR-22): the owner at 18 000 hands send 1 (x = 20 000, n = 2 500) and send 2 (x = 22 500, n = 2 500), both with r = 18 000; it moves to 24 500
    // and an Underflow at tick 24 000 is reported; the close (x = H = 25 000, n = 2, r = 24 500). busy = max(0, 18 000, 20 000) + 2 500 = 22 500, then
    // max(22 500, 18 000, 22 500) + 2 500 = 25 000, then max(25 000, 24 500, 25 000) + 2 = 25 002. B = max(35 002, 24 500 + 2 000, 25 017) = 35 002: R = 36 000.
    // With `x` dropped: busy 20 500, 23 000, 24 502, B = 34 502, R = 35 000.
    let mut rig = repeat_rig();
    book_repeat(&mut rig, 1, 20_000, 0);
    at(&rig, 17 * MS);
    at(&rig, 18 * MS);
    hand(&mut rig, 2);
    assert_eq!(
        tx_sends(&rig.device).iter().map(|send| (send.0, send.1)).collect::<Vec<_>>(),
        [(2_500, Some(rig.clock.instant(20_000))), (2_500, Some(rig.clock.instant(22_500)))],
        "{:?}",
        rig.device.calls()
    );
    at(&rig, 24_500 * 200);
    report(&mut rig.tx, TxCode::Underflow, &rig.clock, 24_000, 0);
    assert_eq!(emitted(&rig.events), [("underflow", Some(36_000))], "{:?}", lock(&rig.core.rec).rejected);
    let row = resumes(&rig.core);
    assert_eq!((row.len(), row[0]["cause"].as_str(), row[0]["from"].as_i64(), row[0]["resumes_at"].as_i64()), (1, Some("underflow"), Some(25_000), Some(36_000)), "{row:?}");
    assert_eq!(closing_sends(&rig.device), [format!("tx_send 0 n=2 at={} sob=false eob=true", rig.clock.instant(25_000))]);
    for ms in [27, 36] {
        at(&rig, ms * MS);
        go(&mut rig);
    }
    assert!(rig.device.transmitted(rig.clock.instant(36_000), 1_000).iter().all(Option::is_some), "the part plays: {:?}", rig.device.calls());
}

#[test]
fn mt3d_the_expired_timeline_resumes_and_the_report_that_comes_later_carries_the_event() {
    // V3 (UR-22, UR-28): the owner finds its timeline expired before any report is read: the close and the resume happen, the event comes with the report.
    // busy: send 1 = 21 000; the close (r = 22 000, x = 21 000, n = 2) = 22 002; B = max(32 002, 22 000 + 2 000, 21 017), R = 33 000.
    let mut rig = late_repeat();
    at(&rig, 22 * MS);
    assert!(rig.tx.step());
    assert_eq!(rig.tx.open_next(), Some(33_000));
    assert!(emitted(&rig.events).is_empty(), "no report was read: the event comes with it");
    let row = resumes(&rig.core);
    assert_eq!((row.len(), row[0]["cause"].as_str(), row[0]["resumes_at"].as_i64()), (1, Some("expired"), Some(33_000)), "{row:?}");
    assert_eq!(ended_notes(&rig.core), 0, "{:?}", lock(&rig.core.rec).rejected);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events), [("late", Some(33_000))], "the event carries the plan the expired check made");
    assert_eq!(resumes(&rig.core).len(), 1, "the report that comes later resumes nothing again: {:?}", lock(&rig.core.rec).timing);
    assert_eq!(rig.tx.open_next(), Some(33_000));
}

#[test]
fn mt3d_a_second_stall_inside_the_send_moves_busy_and_so_the_resume_point() {
    // V4 (UR-22): units are samples. The repeat (N = 1 000, k0 = 20 000); send 1 (x1 = 20 000, n1 = 2 500) returns at r1 = 21 000; send 2 (x2 = 22 500,
    // n2 = 2 500) stalls 6 ms and returns at r2 = 27 000; the reports are read at 27 000 and the close (n = 2, at H = 25 000) returns at r3 = 27 000.
    // busy1 = max(0, 21 000, 20 000) + 2 500 = 23 500; busy2 = max(23 500, 27 000, 22 500) + 2 500 = 29 500; busy3 = max(29 500, 27 000, 25 000) + 2 = 29 502.
    // R = the first 20 000 + 1 000 j at or after max(39 502, 27 000 + 2 000, 25 017) = 40 000. Without `r`: busy 22 500, 25 000, 25 002, B = 35 002, R = 36 000.
    // Without `n`: busy3 = 27 000, R = 37 000.
    let mut rig = repeat_rig();
    book_repeat(&mut rig, 1, 20_000, 0);
    at(&rig, 17 * MS);
    let (core, time, device) = (rig.core.clone(), rig.time.clone(), rig.device.clone());
    let calls = std::sync::atomic::AtomicUsize::new(0);
    crate::provider::test_support::probe(&rig.core, "tx_send_planned", move |_| {
        let call = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(ms) = [21, 27].get(call) {
            device_at(&core, &time, &device, ms * MS);
        }
    });
    hand(&mut rig, 2);
    assert_eq!(
        tx_sends(&rig.device).iter().map(|send| (send.0, send.1)).collect::<Vec<_>>(),
        [(2_500, Some(rig.clock.instant(20_000))), (2_500, Some(rig.clock.instant(22_500)))],
        "{:?}",
        rig.device.calls()
    );
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events).first(), Some(&("late", Some(40_000))), "{:?}", lock(&rig.core.rec).timing);
    assert_eq!(resumes(&rig.core).len(), 1, "the second drop's report is the gap's own: {:?}", lock(&rig.core.rec).timing);
    assert_eq!(closing_sends(&rig.device), [format!("tx_send 0 n=2 at={} sob=false eob=true", rig.clock.instant(25_000))], "{:?}", rig.device.calls());
    for ms in [32, 41] {
        at(&rig, ms * MS);
        go(&mut rig);
    }
    assert!(rig.device.transmitted(rig.clock.instant(40_000), 1_000).iter().all(Option::is_some), "the part plays: {:?}", rig.device.calls());
}

#[test]
fn mt3d_a_part_that_plays_less_than_the_stall_part_ends_the_repeat() {
    // V11 (UR-22): the first gap's stall part is P = (busy + m) - H = (21 002 + 10 000) - 21 000 = 10 002 samples. The part started at 32 000: an Underflow after
    // played = P - 1 ends the repeat, after P (equality) or P + 1 resumes it again; a late drop that reaches the part (nothing of it played) ends it.
    for (code, played, ends) in [(TxCode::Underflow, 10_001, true), (TxCode::Underflow, 10_002, false), (TxCode::Underflow, 10_003, false), (TxCode::TimeError, 1, true)] {
        let mut rig = resumed_part();
        let resumes_before = resumes(&rig.core).len();
        report(&mut rig.tx, code, &rig.clock, 32_000 + played, 0);
        let events = emitted(&rig.events);
        assert_eq!(ended_notes(&rig.core), usize::from(ends), "{code:?} after {played}: {:?}", lock(&rig.core.rec).rejected);
        assert_eq!(rig.tx.open_next().is_none(), ends, "{code:?} after {played}");
        assert_eq!(resumes(&rig.core).len(), resumes_before + usize::from(!ends), "{code:?} after {played}");
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].1.is_some(), !ends, "{code:?} after {played}: an ended repeat has no `resumes_at`: {events:?}");
    }
}

#[test]
fn mt3d_a_stale_report_of_the_dropped_burst_does_not_drop_the_resumed_part() {
    // V12 (UR-22, M2): the part at 32 000 is handed and the trigger report read. A TimeError, then an Underflow, with ticks in the dropped burst's span
    // [20 000, 32 000): the TimeError makes no event (the dropped burst's correlation is gone and the part's starts at R); the Underflow keeps its own
    // event, with no `resumes_at`. Neither resumes or abandons anything.
    let mut rig = resumed_part();
    let before = (resumes(&rig.core).len(), rig.tx.open_next());
    report(&mut rig.tx, TxCode::TimeError, &rig.clock, 25_000, 0);
    assert!(emitted(&rig.events).is_empty());
    report(&mut rig.tx, TxCode::Underflow, &rig.clock, 26_000, 0);
    assert_eq!(emitted(&rig.events), [("underflow", None)]);
    assert_eq!((resumes(&rig.core).len(), ended_notes(&rig.core), rig.tx.open_next()), (before.0, 0, before.1), "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(rows(&rig).len(), 1, "the part's record is unchanged");
}

#[test]
fn mt3d_a_second_drop_found_in_the_gap_abandons_at_the_handed_cursor() {
    // V13 (UR-22, M3): the part at R1 = 32 000 is dropped. The owner's expired check finds it before the report is read and resumes again (H2, R2; nothing is
    // open). Then the part's TimeError reaches its correlation: nothing of it played, so the repeat ends by the abandon, at H2, not at R2: a booking 5 000
    // samples past H2 is on time and accepted (a footprint at R2 + 5 would refuse it), and no send follows at R2.
    let mut rig = resumed_part();
    let h2 = rig.tx.open_next().expect("the part's cursor");
    at(&rig, h2 / 1_000 * MS + MS);
    assert!(rig.tx.step());
    let paused_at = rig.tx.open_next().expect("paused again");
    assert!(paused_at > h2 + 5_000, "R2 {paused_at} is past H2 {h2}");
    assert_eq!(resumes(&rig.core).len(), 2);
    let sends = tx_sends(&rig.device).len();
    report(&mut rig.tx, TxCode::TimeError, &rig.clock, 33_000, 0);
    assert_eq!(ended_notes(&rig.core), 1, "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(rig.tx.open_next(), None);
    assert_eq!(tx_sends(&rig.device).len(), sends, "no close and no start at R2: {:?}", rig.device.calls());
    book(&mut rig, 7, 1, h2 + 5_000);
    assert!(super::e2a::refused_ids(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
}

#[test]
fn mt3d_what_still_ends_a_repeat() {
    // V14 (UR-22): a lost sequence ends a repeat as it did; a one-shot burst is ended by an Underflow and by a late drop, never resumed.
    let mut rig = repeat_rig();
    book_repeat(&mut rig, 1, 20_000, 0);
    at(&rig, 11 * MS);
    go(&mut rig);
    report(&mut rig.tx, TxCode::SeqError, &rig.clock, 0, 0);
    assert_eq!((ended_notes(&rig.core), resumes(&rig.core).len(), rig.tx.open_next()), (1, 0, None), "{:?}", lock(&rig.core.rec).rejected);
    for code in [TxCode::Underflow, TxCode::TimeError] {
        let mut rig = switch_rig(vec![o::ramp(1, 20_000)]);
        book(&mut rig, 1, 0, 20_000);
        at(&rig, 11 * MS);
        go(&mut rig);
        report(&mut rig.tx, code, &rig.clock, 20_500, 0);
        assert_eq!((ended_notes(&rig.core), resumes(&rig.core).len(), rig.tx.open_next()), (1, 0, None), "{code:?}: {:?}", lock(&rig.core.rec).rejected);
    }
}

#[test]
fn mt3d_a_stop_in_the_gap_sends_nothing_and_ends_the_record() {
    // V15, V16 (UR-22): the repeat is paused with no device burst open. A Stop, read at the next turn, closes nothing and sends nothing at or after R; the
    // record ends `stop` and the booking is gone.
    let mut rig = late_repeat();
    rig.tx.reports(Wall::ZERO);
    let sends = tx_sends(&rig.device).len();
    stop(&mut rig);
    assert_eq!(rig.tx.open_next(), None);
    for ms in [23, 24, 33] {
        at(&rig, ms * MS);
        go(&mut rig);
    }
    assert_eq!(tx_sends(&rig.device).len(), sends, "{:?}", rig.device.calls());
    assert_eq!(rows(&rig).iter().map(|row| row["end"].as_str()).collect::<Vec<_>>(), [Some("stop")]);
}

#[test]
fn mt3d_busy_survives_a_new_owner() {
    // V20 (n2): the stream's `busy` is the book's, which outlives the owner; a new owner on the same core finds it. After send 1 (r = 11 000, x = 20 000,
    // n = 1 000) busy is 21 000 samples.
    let rig = late_repeat();
    let busy = |core: &Core| lock(&core.streams).stream(stream_key(Dir::Tx, 0)).book.busy;
    assert_eq!(busy(&rig.core), 21_000 * 200);
    let (_to_tx, cmds) = std::sync::mpsc::channel();
    let _new = Tx::new(rig.core.clone(), 0, cmds, Some(rig.clock), Channels::first(1));
    assert_eq!(busy(&rig.core), 21_000 * 200);
}

#[test]
fn mt3d_a_switch_that_joins_a_resumed_part_is_named_with_it() {
    // V21 (UR-28, carry (b)): a late drop that reaches a resumed part names the part's start and the Kernel burst that joined it. After the part is the
    // open device burst, its correlation is the last, so the join lands on it.
    let mut rig = resumed_part();
    let (domain, part) = (rig.clock.domain, rig.tx.unacked.back().map(|pending| pending.target.ticks_in(rig.clock.domain).unwrap()));
    assert_eq!(part, Some(32_000), "the part's correlation starts at R");
    rig.tx.join_burst(TimePoint::new(domain, 40_000));
    report(&mut rig.tx, TxCode::TimeError, &rig.clock, 33_000, 0);
    let named: Vec<i64> = rig
        .events
        .drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0))
        .iter()
        .filter(|event| event.payload["outcome"] == "late_at_device")
        .map(|event| event.payload["target"]["ticks"].as_i64().unwrap())
        .collect();
    assert_eq!(named, [32_000, 40_000]);
    // V21b: with no correlation (a reopen cleared them) a join makes none and touches none.
    let (mut tx, _clock, _events, _to_tx, _core) = unit(1);
    tx.join_burst(TimePoint::new(domain, 5));
    assert!(tx.unacked.is_empty());
}

#[test]
fn mt3d_two_channels_resume_once_and_tell_each_underflow() {
    // V23 (UR-22, UR-28): a starvation reports one Underflow per channel. The first resumes the repeat and its event carries `resumes_at`; the second, read
    // after the resume with a tick before R, is a stale report: its `TX_UNDERFLOW` stays without `resumes_at`, and no second resume or `timing` row follows.
    let mut rig = repeat_rig();
    // The fake's streamer has one channel; the correlation is given two positions (reports of channel 1 are scripted), which is all the reports read.
    book_repeat(&mut rig, 1, 20_000, 0);
    at(&rig, 17 * MS);
    at(&rig, 18 * MS);
    hand(&mut rig, 2);
    rig.tx.unacked.back_mut().expect("the burst's correlation").channels = [0, 1].into();
    at(&rig, 24_500 * 200);
    for channel in [0, 1] {
        report(&mut rig.tx, TxCode::Underflow, &rig.clock, 24_000, channel);
    }
    assert_eq!(emitted(&rig.events), [("underflow", Some(36_000)), ("underflow", None)]);
    assert_eq!(resumes(&rig.core).len(), 1);
    assert!(rig.tx.unacked.is_empty(), "each channel's Underflow retired its own position");
}

#[test]
fn mt3d_two_channels_late_resume_once_in_either_order_and_the_part_ends_on_one_channel() {
    // V23 (UR-22, UR-28, RT22): a late drop on two channels. The report that comes first, from either channel, names the Kernel burst once with `resumes_at`
    // and makes the one resume and the one `timing` row; the other channel's report, which reaches the burst's remaining position, adds no event and no resume.
    // The part's first report to reach it decides the stutter rule from either channel.
    for first_channel in [0, 1] {
        let mut rig = late_repeat();
        // The fake's report (channel 0) waits in its queue; a scripted report of `first_channel` is read first.
        rig.tx.unacked.back_mut().expect("the dropped burst's correlation").channels = [0, 1].into();
        report(&mut rig.tx, TxCode::TimeError, &rig.clock, 21_000, first_channel);
        assert_eq!(emitted(&rig.events), [("late", Some(32_000))], "channel {first_channel} first");
        report(&mut rig.tx, TxCode::TimeError, &rig.clock, 21_000, 1 - first_channel);
        rig.tx.reports(Wall::ZERO);
        assert!(emitted(&rig.events).is_empty(), "the other channel and the fake's own reports add no event: channel {first_channel} first");
        assert_eq!((resumes(&rig.core).len(), rig.tx.open_next()), (1, Some(32_000)));
    }
    // The part's correlation holds both channels: a late report from channel 1 alone reaches it, nothing of it played, and the repeat ends.
    let mut rig = resumed_part();
    rig.tx.unacked.back_mut().expect("the part's correlation").channels = [0, 1].into();
    report(&mut rig.tx, TxCode::TimeError, &rig.clock, 33_000, 1);
    assert_eq!((ended_notes(&rig.core), rig.tx.open_next()), (1, None), "{:?}", lock(&rig.core.rec).rejected);
}

#[test]
fn mt3d_the_resume_point_function() {
    // V25 (UR-22): `R` is the first lattice point at or after the latest of the three bounds. Every row has `B >= origin`, so `0 <= R - B < N`; where `busy + m`
    // is the latest, `R - busy < m + N`.
    let (_tx, clock, _events, _to_tx, _core) = unit(1);
    let (origin, period, m) = (20_000 * 200, 1_000 * 200, 10_000 * 200);
    let point = |busy: i64, decided: i64, front: Option<i64>| super::super::resume_boundary(clock, 20_000, 1_000, busy + m, decided, front).map(|sample| (sample, clock.instant(sample)));
    for (busy, decided, front, expected) in [
        (21_002 * 200, 23_000 * 200, Some(21_017 * 200), 32_000),
        (0, 40_500 * 200, Some(21_017 * 200), 41_000),
        (0, 23_000 * 200, Some(45_001 * 200), 46_000),
        (22_000 * 200, 23_000 * 200, None, 32_000),
    ] {
        let bound = (busy + m).max(decided).max(front.unwrap_or(i64::MIN));
        let (sample, tick) = point(busy, decided, front).expect("fits");
        assert_eq!(sample, expected, "{busy} {decided} {front:?}");
        assert!((0..period).contains(&(tick - bound)) && tick >= origin, "{busy} {decided} {front:?}: {tick} from {bound}");
    }
    // A bound on a lattice point is that point; a bound before the origin is the origin; none past the clock.
    assert_eq!(point(31_000 * 200 - m, 0, None).map(|point| point.0), Some(31_000));
    assert_eq!(super::super::resume_boundary(clock, 20_000, 1_000, -1_000_000, 0, None), Some(20_000), "a bound before the origin is the origin");
    assert_eq!(super::super::resume_boundary(clock, 20_000, 1_000, i64::MAX, i64::MAX, None), None);
}

#[test]
fn mt3d_an_uncertain_close_runs_t3_first_and_resumes_after_the_flush() {
    // V7 (UR-22, #75): the close (the fake's second send) takes one of its two samples, so it is uncertain and does not count in `busy` (busy = 21 000 after send 1).
    // B = max(21 000 + 10 000, 21 000 + 2 000, front) = 31 000, a lattice point: R = 31 000. The owner reopens the streamer and runs T3 before anything else
    // is sent; the part follows at R.
    let mut rig = late_repeat_with(vec![FakeFault::ShortSend(1)]);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events), [("late", Some(31_000))], "{:?}", lock(&rig.core.rec).rejected);
    assert!(rig.tx.chain().needs_reopen());
    assert!(rig.tx.pass(), "the reopen and its flush run on this turn");
    assert!(!rig.tx.chain().needs_reopen());
    let front = rig.tx.chain().front().expect("the flush's front");
    assert!(front.1 < 31_000, "R is past the flush front {front:?}");
    for ms in [22, 23, 24] {
        at(&rig, ms * MS);
        go(&mut rig);
    }
    let calls = rig.device.calls();
    let position = |what: &dyn Fn(&String) -> bool| calls.iter().position(what).unwrap_or_else(|| panic!("{calls:?}"));
    let (close, flush, part) =
        (position(&|call| call == "tx_close 0"), position(&|call| call.contains("n=4 at=- sob=true eob=true")), position(&|call| call.contains(&format!("at={} sob=true", rig.clock.instant(31_000)))));
    assert!(close < flush && flush < part, "close, T3, then the part: {calls:?}");
}

#[test]
fn mt3d_a_late_wake_resumes_no_earlier_than_now_plus_the_lead() {
    // V6 (UR-22): the owner reads the report at 50 500, the close uncertain (so `busy` is 21 000 and busy + m = 31 000): B = max(31 000, 50 500 + 2 000, front)
    // = 52 500 and R = 53 000.
    let mut rig = late_repeat_with(vec![FakeFault::ShortSend(1)]);
    at(&rig, 50_500 * 200);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events), [("late", Some(53_000))], "{:?}", lock(&rig.core.rec).rejected);
}

#[test]
fn mt3d_a_resume_whose_event_never_comes_still_splits_the_record() {
    // V24 (UR-22): the correlation is cleared between the drop and its report (a generation change): the timing row and the split record exist, no event is
    // invented.
    let mut rig = late_repeat();
    at(&rig, 22 * MS);
    assert!(rig.tx.step());
    rig.tx.unacked.clear();
    rig.tx.reports(Wall::ZERO);
    assert!(emitted(&rig.events).is_empty());
    assert_eq!(resumes(&rig.core).len(), 1);
    for ms in [24, 25, 34] {
        at(&rig, ms * MS);
        go(&mut rig);
    }
    assert_eq!(rows(&rig).iter().map(|row| row["end"].as_str()).collect::<Vec<_>>(), [Some("discontinuity")]);
}

#[test]
fn mt3d_a_delayed_underflow_that_retires_two_entries_applies_the_stutter_rule() {
    // V3b (UR-22): the part at R1 = 33 000 (after an expired resume) is itself found expired and resumed again, before any report is read. One delayed
    // Underflow then reaches the dropped burst and the part together: one TX_UNDERFLOW, and the newest entry decides by the stall part before it,
    // P1 = (busy1 + m) - H1 = (22 002 + 10 000) - 21 000 = 11 002 samples: played = P1 - 1 ends (abandon at H2, no start at R2, no stray close), P1 and P1 + 1 keep
    // the second resume.
    // Two geometries for the second gap: the owner wakes `wake` samples past H2, so its stall part is P2 = busy2 + m - H2 = 10 002 + wake: g1 wake 1 500, P2 =
    // P1 + 500; g2 wake 500, P2 = P1 - 500. A rule that compared with P2 ends the P1 cases in g1 and resumes the P1 - 1 case in g2.
    for (wake, delta) in [(1_500, 500), (500, -500)] {
        for (played, ends) in [(11_001, true), (11_002, false), (11_003, false)] {
            let mut rig = late_repeat();
            at(&rig, 22 * MS);
            assert!(rig.tx.step());
            for ms in [24, 25] {
                at(&rig, ms * MS);
                go(&mut rig);
            }
            let h2 = rig.tx.open_next().expect("the part's cursor");
            at(&rig, (h2 + wake) * 200);
            assert!(rig.tx.step());
            assert_eq!(resumes(&rig.core).len(), 2);
            let r2 = rig.tx.open_next().expect("paused again");
            let stall = rig.tx.with_book(|book| book.open.as_ref().and_then(|open| open.paused).map(|paused| paused.stall));
            assert_eq!(stall, Some((11_002 + delta) * 200), "P2 of wake {wake}");
            while let Ok(Some(_)) = rig.device.tx_async(0, Wall::ZERO) {}
            let sends = tx_sends(&rig.device).len();
            report(&mut rig.tx, TxCode::Underflow, &rig.clock, 33_000 + played, 0);
            assert_eq!(emitted(&rig.events), [("underflow", (!ends).then_some(r2))], "wake {wake}, played {played}");
            assert!(rig.tx.unacked.is_empty());
            assert_eq!(ended_notes(&rig.core), usize::from(ends), "wake {wake}, played {played}");
            assert_eq!(resumes(&rig.core).len(), 2, "no third resume");
            assert_eq!(rig.tx.open_next(), (!ends).then_some(r2), "wake {wake}, played {played}");
            assert_eq!(tx_sends(&rig.device).len(), sends, "no close and no start at R2: {:?}", rig.device.calls());
        }
        // A late drop that reaches both entries: nothing of the part played, so the repeat ends whatever the gap; each Kernel burst is told, the dropped
        // burst with the plan it was given, the part with none.
        let mut rig = late_repeat();
        at(&rig, 22 * MS);
        assert!(rig.tx.step());
        for ms in [24, 25] {
            at(&rig, ms * MS);
            go(&mut rig);
        }
        let h2 = rig.tx.open_next().expect("the part's cursor");
        at(&rig, (h2 + wake) * 200);
        assert!(rig.tx.step());
        while let Ok(Some(_)) = rig.device.tx_async(0, Wall::ZERO) {}
        report(&mut rig.tx, TxCode::TimeError, &rig.clock, 33_001, 0);
        assert_eq!(emitted(&rig.events), [("late", Some(33_000)), ("late", None)], "wake {wake}");
        assert_eq!((ended_notes(&rig.core), resumes(&rig.core).len(), rig.tx.open_next()), (1, 2, None), "wake {wake}");
    }
}

#[test]
fn mt3d_a_stalled_start_of_the_part_ends_the_repeat_with_no_stray_close() {
    // V14 (UR-22): the part's start of burst (the fake's third send) takes nothing. No device burst was open, so the abandon sends no close of its own; the
    // repeat ends there (a failed or short send ends a repeat as before).
    let mut rig = late_repeat_with(vec![FakeFault::StalledSend(2)]);
    rig.tx.reports(Wall::ZERO);
    let closes = closing_sends(&rig.device).len();
    assert_eq!(closes, 1, "the resume's close: {:?}", rig.device.calls());
    for ms in [23, 24, 33] {
        at(&rig, ms * MS);
        go(&mut rig);
    }
    assert_eq!(rig.tx.open_next(), None, "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(closing_sends(&rig.device).len(), closes, "no stray close: {:?}", rig.device.calls());
}

#[test]
fn mt3d_the_margin_is_ten_milliseconds_and_the_lattice_rounds_up() {
    // V5 (UR-22): a repeat whose first send is a whole 2 500-sample chunk (x = k0, r = 17 000) reaches a device at 21 000 and is dropped; the drain of its
    // samples (2 500 from 21 000, plus the fake's pad) ends near 23 560. busy1 = max(0, 17 000, k0) + 2 500 = k0 + 2 500; the close (H = k0 + 2 500, r = 21 000,
    // n = 2): busy = k0 + 2 502, so busy + m = k0 + 12 502. The decision instant plus the lead is 23 000.
    // Case A (period 1 000, k0 = 20 030): R = the first 20 030 + 1 000 j at or after 32 532 = 33 030. Without `m` the bound is 23 000 and R = 23 030, inside the
    // drain, where the fake is expected to refuse it and the stutter rule to end the repeat (not run: the row that drops `m`, MT3D-02, is killed by the exact
    // `R`); the part plays at 33 030.
    // Case B (period 12 502, k0 = 20 030): busy + m = 32 532 is the lattice point itself, so R = 32 532 and R - busy = m exactly.
    for (wave, period, expected) in [(0, 1_000, 33_030), (2, 12_502, 32_532)] {
        let mut rig = switch_rig(vec![o::ramp(1, 1_000), o::ramp(2, 100), o::ramp(3, 12_502)]);
        book_repeat(&mut rig, 1, 20_030, wave);
        at(&rig, 17 * MS);
        rig.device.set_time(21 * MS);
        hand(&mut rig, 1);
        assert_eq!(tx_sends(&rig.device).iter().map(|send| send.0).collect::<Vec<_>>(), [2_500], "{:?}", rig.device.calls());
        at(&rig, 21 * MS);
        rig.tx.reports(Wall::ZERO);
        assert_eq!(emitted(&rig.events), [("late", Some(expected))], "period {period}: {:?}", lock(&rig.core.rec).rejected);
        assert_eq!(lock(&rig.core.rec).timing.iter().filter(|row| row["what"] == "tx_resume").count(), 1);
        for ms in [expected / 1_000 - 9, expected / 1_000 + 1] {
            at(&rig, ms * MS);
            go(&mut rig);
        }
        assert!(rig.device.transmitted(rig.clock.instant(expected), 1_000).iter().all(Option::is_some), "the part plays at {expected}: {:?}", rig.device.calls());
        assert_eq!(ended_notes(&rig.core), 0, "{:?}", lock(&rig.core.rec).rejected);
        assert!(rig.tx.open_next().is_some());
    }
}

#[test]
fn mt3d_a_recovery_that_fails_ends_the_repeat_and_the_clock() {
    // V7 (UR-22, #75): the close is uncertain, so the owner reopens and runs T3 before anything else is sent. Three unconfirmed flushes (the fake's sends 2, 3 and
    // 4 take nothing), or a reopen the device refuses, end the transmit stream's clock: the repeat ends, its record ends `stop`, nothing is sent at R. (A flush
    // front past R is a missed R, which D2 moves.)
    for faults in [vec![FakeFault::ShortSend(1), FakeFault::StalledSend(2), FakeFault::StalledSend(3), FakeFault::StalledSend(4)], vec![FakeFault::ShortSend(1), FakeFault::TxOpenFails(1)]] {
        let mut rig = late_repeat_with(faults.clone());
        rig.tx.reports(Wall::ZERO);
        assert_eq!(emitted(&rig.events), [("late", Some(31_000))], "{faults:?}");
        assert!(rig.tx.pass(), "the recovery runs on this turn: {faults:?}");
        assert!(rig.tx.clock.is_none() && rig.tx.open_next().is_none(), "{faults:?}: {:?}", lock(&rig.core.rec).rejected);
        let sends = tx_sends(&rig.device).len();
        for ms in [22, 30, 31] {
            at(&rig, ms * MS);
            go(&mut rig);
        }
        assert_eq!(tx_sends(&rig.device).len(), sends, "nothing at R: {faults:?}: {:?}", rig.device.calls());
        assert_eq!(rows(&rig).iter().map(|row| row["end"].as_str()).collect::<Vec<_>>(), [Some("stop")], "{faults:?}");
    }
}

#[test]
fn mt3d_more_that_ends_a_repeat_a_resume_point_past_the_clock_and_a_failed_continuation() {
    // V14 (UR-22): when `R` does not fit the clock (the stream's `busy` is at the end of the representable ticks) the repeat ends by the abandon at H: a note, no
    // timing row, an event without `resumes_at`. A failed or short send of the part after its start of burst (the fake's fourth send) ends it too, and so
    // does a failed start of burst.
    let mut rig = late_repeat();
    rig.tx.with_book(|book| book.busy = i64::MAX);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events), [("late", None)]);
    assert_eq!((ended_notes(&rig.core), resumes(&rig.core).len(), rig.tx.open_next()), (1, 0, None), "{:?}", lock(&rig.core.rec).rejected);
    for fault in [FakeFault::FailSend(2), FakeFault::ShortSend(3), FakeFault::StalledSend(3)] {
        let mut rig = late_repeat_with(vec![fault.clone()]);
        rig.tx.reports(Wall::ZERO);
        for ms in [23, 24, 25, 26, 33, 34] {
            at(&rig, ms * MS);
            go(&mut rig);
        }
        assert_eq!(rig.tx.open_next(), None, "{fault:?}: {:?}", rig.device.calls());
        // The part's device burst was open or may have been (the failed start may have gone): the abandon closes it, once.
        let closes = 2;
        assert_eq!(closing_sends(&rig.device).len(), closes, "{fault:?}: {:?}", rig.device.calls());
    }
}

#[test]
fn mt3d_the_expired_timeline_resumes_and_a_later_underflow_carries_the_plan() {
    // V3 (UR-22, UR-28) with an Underflow: the owner at 26 ms finds the timeline of the repeat's two handed sends (to 25 000) expired before any report is read
    // and resumes it. busy: 25 000 after the sends (as in `an_underflow_resumes_a_repeat_and_names_the_span_with_busy_from_x`), the close (r = 26 000, x = 25 000) =
    // 26 002; B = max(36 002, 28 000) so R = 37 000. The Underflow read later carries that plan, once.
    let mut rig = repeat_rig();
    book_repeat(&mut rig, 1, 20_000, 0);
    at(&rig, 17 * MS);
    at(&rig, 18 * MS);
    hand(&mut rig, 2);
    at(&rig, 26 * MS);
    assert!(rig.tx.step());
    assert_eq!(rig.tx.open_next(), Some(37_000));
    assert!(emitted(&rig.events).is_empty());
    report(&mut rig.tx, TxCode::Underflow, &rig.clock, 24_000, 0);
    assert_eq!(emitted(&rig.events), [("underflow", Some(37_000))]);
    assert_eq!(resumes(&rig.core).len(), 1);
}

#[test]
fn mt3d_an_expired_timeline_whose_resume_point_is_past_the_clock_ends_the_repeat_with_a_note() {
    // V14 (UR-22): the expired check of a resumable repeat whose `R` does not fit the clock ends it by the abandon at H, with the note, no timing row, and no
    // close of its own beyond the resume's.
    let mut rig = late_repeat();
    rig.tx.with_book(|book| book.busy = i64::MAX);
    at(&rig, 22 * MS);
    assert!(rig.tx.step());
    assert_eq!((ended_notes(&rig.core), resumes(&rig.core).len(), rig.tx.open_next()), (1, 0, None), "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(closing_sends(&rig.device).len(), 1, "the resume's close only: {:?}", rig.device.calls());
}

#[test]
fn mt3d_provider_stop_and_a_loss_in_the_gap_send_nothing_and_tell_nothing() {
    // V16 (UR-22): the repeat is paused with no device burst open. A device loss sends nothing at or after R; `Provider::stop` (also after the loss) ends the
    // booking with no close and no send, leaves no booking, makes no event, and the record ends `stop`.
    for lost in [false, true] {
        let mut rig = late_repeat();
        rig.tx.reports(Wall::ZERO);
        emitted(&rig.events);
        let sends = tx_sends(&rig.device).len();
        if lost {
            rig.core.device_lost("scripted loss in the gap");
            for ms in [23, 24, 33] {
                at(&rig, ms * MS);
                assert!(!rig.tx.step(), "a lost device has nothing to do");
            }
        }
        let device = rig.device.clone();
        let (core, events) = super::e1::shutdown(rig);
        assert_eq!(tx_sends(&device).len(), sends, "lost {lost}: {:?}", device.calls());
        assert!(emitted(&events).is_empty(), "lost {lost}");
        let ends: Vec<_> = lock(&core.rec).bursts.iter().map(|row| row["end"].as_str().map(str::to_owned)).collect();
        assert_eq!(ends, [Some("stop".to_owned())], "lost {lost}");
    }
}

#[test]
fn mt3d_a_second_resume_gives_a_third_record_and_no_discontinuity_event() {
    // V8 (UR-22): the part at 32 000 is itself found expired and resumed again, and its part is sent: three records (the first two end in a discontinuity with
    // what each handed, the third is open until the Stop), and no `TX_DISCONTINUITY` or other event beyond the drops' own.
    let mut rig = resumed_part();
    let h2 = rig.tx.open_next().expect("the part's cursor");
    at(&rig, (h2 + 1_500) * 200);
    assert!(rig.tx.step());
    let r2 = rig.tx.open_next().expect("paused again");
    for ms in [r2 / 1_000 - 9, r2 / 1_000 + 1] {
        at(&rig, ms * MS);
        go(&mut rig);
    }
    stop(&mut rig);
    let records = rows(&rig);
    assert_eq!(records.iter().map(|row| row["end"].as_str()).collect::<Vec<_>>(), [Some("discontinuity"), Some("discontinuity"), Some("stop")], "{records:?}");
    assert_eq!(records.iter().map(|row| row["target"]["ticks"].as_i64()).collect::<Vec<_>>(), [Some(20_000), Some(32_000), Some(r2)]);
    let kinds: Vec<String> = rig.events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).iter().map(|event| event.kind.as_str().to_owned()).collect();
    assert!(kinds.iter().all(|kind| !kind.contains("DISCONTINUITY")), "{kinds:?}");
}

// ---------------------------------------------------------------------------------------------------------------------------------------------------
// Sub-step D2 (plan §9, V17-V19, V22): a switch across a gap, a missed resume point, a report between a switch's join and its open.
// ---------------------------------------------------------------------------------------------------------------------------------------------------

/// The events since the last call.
fn drained(events: &EventCollector) -> Vec<ezsdr_kernel::event::Event> {
    events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0))
}

/// `(target, late_by_ns)` of every `TIME_ERROR { send_asap }` in `events`.
fn send_asaps(events: &[ezsdr_kernel::event::Event]) -> Vec<(i64, i64)> {
    events
        .iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR && event.payload["outcome"] == "send_asap")
        .map(|event| (event.payload["target"]["ticks"].as_i64().unwrap(), event.payload["late_by_ns"].as_i64().unwrap()))
        .collect()
}

/// The owner's clock alone moves to `ticks` (the device's stays), as a host that is scheduled late.
fn owner_at(rig: &SwitchRig, ticks: i64) {
    rig.time.advance_to(rig.core.at(ticks)).unwrap();
}

/// The first timed start-of-burst sample the fake was handed at or after `from`, as a sample index.
fn part_start(rig: &SwitchRig) -> Option<i64> {
    tx_sends(&rig.device).into_iter().filter(|send| send.2).filter_map(|send| send.1).map(|tick| rig.clock.at_or_after(tick)).max()
}

/// The owner and the device run from `from` to `to` milliseconds, a millisecond at a time.
fn sweep(rig: &mut SwitchRig, from: i64, to: i64) {
    for ms in from..=to {
        at(rig, ms * MS);
        go(rig);
    }
}

/// W0 (period `N`, from 2 000) handed late: the owner at 0 hands 2 000..9 500 in three sends (x = 2 000, 4 500, 7 000; n = 2 500 each;
/// r = 0), and the device, at 3 ms, drops them whole; the reports are unread. `H = 9 500` and `busy = 9 500`. A report read at an owner time up to 18 000 samples
/// (`now + lead` below `busy + m`) closes at `H` (busy 9 502) and resumes at the first W0 lattice point at or after 19 502. The second wave is NEW (100 samples), the
/// third a later switch's (300).
fn gap_rig(old: Vec<Iq>) -> SwitchRig {
    let mut rig = switch_rig(vec![old, o::ramp(2, 100), o::ramp(3, 300)]);
    book_repeat(&mut rig, 1, 2_000, 0);
    rig.device.set_time(3 * MS);
    go(&mut rig);
    assert_eq!(tx_sends(&rig.device).len(), 3, "{:?}", rig.device.calls());
    rig
}

#[test]
fn mt3d_a_switch_held_across_a_late_first_send_moves_to_the_resume_point_and_plays_from_there() {
    // V17 (b), D1 review M1 (UR-22, RM-37): the repeat's sends (2 000..11 000) reach a device at 12 ms and are dropped; the switch to NEW (100 samples) is booked at the
    // owner's 3 ms, so its boundary is the first W0 lattice point at or after 3 000 + 12 000 = 15 000: 15 000, inside the dropped span [H, R) = [9 500, 20 000). The
    // report is read at 3 ms (busy 9 502, R = the first 2 000 + 1 000 j at or after 19 502 = 20 000): the switch takes effect at R, with NEW, flagged late by
    // R - b = 5 000 samples. (Before D2 this was the `stale` branch: a debug panic, and in release a refused switch whose row stayed pending.)
    let mut rig = gap_rig(o::ramp(1, 1_000));
    owner_at(&rig, 3 * MS);
    book_switch(&mut rig, 2, 15_000, 2);
    drain_tx(&mut rig.tx);
    assert_eq!(held_keys(&rig.tx), [15_000], "the switch waits at its boundary, in the gap: {:?} {:?}", refusals(&rig.events), lock(&rig.core.rec).rejected);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events), [("late", Some(20_000))], "{:?}", lock(&rig.core.rec).rejected);
    assert!(rig.tx.step(), "the gap's turn settles the switch against R");
    assert!(held_keys(&rig.tx).is_empty(), "the switch is the booking that resumes at R");
    assert_eq!(rig.tx.open_next(), Some(20_000));
    assert_eq!(ended_notes(&rig.core), 0, "{:?}", lock(&rig.core.rec).rejected);
    sweep(&mut rig, 13, 23);
    assert_eq!(part_start(&rig), Some(20_000), "{:?}", rig.device.calls());
    assert!(rig.device.transmitted(rig.clock.instant(9_500), 10_500).iter().all(Option::is_none), "silent in the gap");
    assert_eq!(rig.device.transmitted(rig.clock.instant(20_000), 300), o::ramp(3, 300).into_iter().map(Some).collect::<Vec<_>>(), "NEW from R, from its first sample: its lattice starts at R");
    // The switch is told late once, for its own target, by its distance from R; the record's row follows it.
    let events = drained(&rig.events);
    assert_eq!(send_asaps(&events), [(15_000, 5_000_000)], "{events:#?}");
    let row = switch_row(&rig.core, 0);
    assert_eq!((row["outcome"].as_str(), row["boundary"].as_i64()), (Some("applied"), Some(20_000)), "{row:?}");
    assert_eq!(resumes(&rig.core).len(), 1, "{:?} {:?}", lock(&rig.core.rec).timing, rig.device.calls());
    // A later switch is accepted (the record holds nothing pending), on NEW's lattice from the moved boundary: at 23.5 ms its bound is 23 500 + 12 000 = 35 500, so
    // the first 20 000 + 300 j at or after it, 35 600 (the lattice from the boundary the switch was received at, 15 000, would give 35 700).
    at(&rig, 23_500 * 200);
    book_switch(&mut rig, 3, 24_000, 1);
    assert!(refusals(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(switch_row(&rig.core, 1)["boundary"].as_i64(), Some(35_600));
    // The part's burst record carries the same late start as the event (RM-14): `send_asap` by the same distance, and the switch's own `at` as the requested one (here the receipt recorded none: its `at` is its boundary).
    stop(&mut rig);
    let records = rows(&rig);
    let part = records.iter().find(|row| row["target"]["ticks"] == 20_000).expect("the part's record");
    assert_eq!((part["late_by"]["ticks"].as_i64(), part["requested_target"]["ticks"].as_i64()), (Some(5_000_000), Some(15_000)), "{records:#?}");
}

#[test]
fn mt3d_a_switch_received_in_the_gap_is_accepted_and_moves_late() {
    // V17 (b), arriving: the same gap with the report already read (R = 20 000); the switch is booked at the owner's 3 ms, boundary 15 000 < R. The record accepts it
    // (there is no refusal in a gap: EA-16 is unchanged), and it replaces the repeat at R.
    let mut rig = gap_rig(o::ramp(1, 1_000));
    owner_at(&rig, 3 * MS);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events), [("late", Some(20_000))]);
    book_switch(&mut rig, 2, 4_000, 1);
    assert!(refusals(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
    sweep(&mut rig, 13, 23);
    assert_eq!(part_start(&rig), Some(20_000), "{:?}", rig.device.calls());
    assert_eq!(rig.device.transmitted(rig.clock.instant(20_000), 100), o::ramp(2, 100).into_iter().map(Some).collect::<Vec<_>>());
    assert_eq!(send_asaps(&drained(&rig.events)), [(4_000, 5_000_000)]);
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied");
}

#[test]
fn mt3d_a_switch_at_the_resume_point_replaces_the_repeat_there_and_is_not_late() {
    // V17 (UR-22): the owner reads the report at 3 ms (R = 20 000), then moves to 8 ms: a switch booked then has the bound 8 000 + 12 000 = 20 000 and the boundary
    // 20 000 = R. It replaces the repeat at R as it would on time: told nothing, its row's boundary R, its waveform from R. (A boundary one past R would be moved.)
    let mut rig = gap_rig(o::ramp(1, 1_000));
    owner_at(&rig, 3 * MS);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events), [("late", Some(20_000))]);
    owner_at(&rig, 8 * MS);
    book_switch(&mut rig, 2, 9_000, 1);
    sweep(&mut rig, 13, 23);
    assert_eq!(part_start(&rig), Some(20_000), "{:?}", rig.device.calls());
    assert_eq!(rig.device.transmitted(rig.clock.instant(20_000), 100), o::ramp(2, 100).into_iter().map(Some).collect::<Vec<_>>(), "NEW from R: {:?}", rig.device.calls());
    assert!(send_asaps(&drained(&rig.events)).is_empty());
    let row = switch_row(&rig.core, 0);
    assert_eq!((row["outcome"].as_str(), row["boundary"].as_i64()), (Some("applied"), Some(20_000)), "{row:?}");
}

#[test]
fn mt3d_a_switch_within_32_samples_after_the_resume_point_moves_to_the_next_safe_boundary() {
    // V17 (b), within 32: W0 has a period of 20, so its lattice is 2 000 + 20 j and R = the first point at or after 19 502 = 19 520. The owner reads the report at 3 ms,
    // then moves to 7.521 ms: a switch booked then has the bound 7 521 + 12 000 = 19 521 and the boundary 19 540, which is 20 samples after R, inside the first send's
    // 32 samples. It moves to the first lattice point at or after R + 32 = 19 552: 19 560. The repeat plays from R, the switch joins it there, flagged late by 20 samples.
    let mut rig = gap_rig(o::ramp(1, 20));
    owner_at(&rig, 3 * MS);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events), [("late", Some(19_520))]);
    owner_at(&rig, 7_521 * 200);
    book_switch(&mut rig, 2, 12_000, 1);
    assert!(refusals(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
    drain_tx(&mut rig.tx);
    assert!(rig.tx.step());
    assert_eq!(held_keys(&rig.tx), [19_560], "the switch waits at the first safe boundary");
    sweep(&mut rig, 12, 22);
    assert_eq!(part_start(&rig), Some(19_520), "{:?}", rig.device.calls());
    assert_eq!(
        rig.device.transmitted(rig.clock.instant(19_558), 4),
        vec![Some(o::ramp(1, 20)[18]), Some(o::ramp(1, 20)[19]), Some(o::ramp(2, 100)[0]), Some(o::ramp(2, 100)[1])],
        "W0 to 19 559, NEW from 19 560"
    );
    assert_eq!(send_asaps(&drained(&rig.events)), [(12_000, 20_000)], "told once, from its own boundary");
    let row = switch_row(&rig.core, 0);
    assert_eq!((row["outcome"].as_str(), row["boundary"].as_i64()), (Some("applied"), Some(19_560)), "{row:?}");
}

#[test]
fn mt3d_a_switch_at_or_past_32_samples_after_the_resume_point_is_kept() {
    // V17 (c): after the report (R = 20 000) a switch booked at the owner's 11 ms has the boundary 23 000, 1 000 after R: kept as it is. W0 plays from R, the switch joins
    // it at its boundary, nothing is told late and nothing is cancelled.
    let mut rig = gap_rig(o::ramp(1, 1_000));
    owner_at(&rig, 3 * MS);
    rig.tx.reports(Wall::ZERO);
    emitted(&rig.events);
    owner_at(&rig, 11 * MS);
    book_switch(&mut rig, 2, 12_000, 1);
    drain_tx(&mut rig.tx);
    assert_eq!(held_keys(&rig.tx), [23_000]);
    sweep(&mut rig, 13, 24);
    assert_eq!(part_start(&rig), Some(20_000), "{:?}", rig.device.calls());
    assert_eq!(rig.device.transmitted(rig.clock.instant(22_999), 2), vec![Some(o::ramp(1, 1_000)[999]), Some(o::ramp(2, 100)[0])]);
    assert!(send_asaps(&drained(&rig.events)).is_empty());
    let row = switch_row(&rig.core, 0);
    assert_eq!((row["outcome"].as_str(), row["boundary"].as_i64()), (Some("applied"), Some(23_000)), "{row:?}");
}

#[test]
fn mt3d_a_switch_whose_boundary_send_was_handed_before_the_drop_is_applied_and_resumes_with_its_waveform() {
    // V17 (a): the switch's boundary send (15 000..) is handed before the Underflow, so the repeat that resumes is NEW and the switch is applied at its own boundary
    // once device time reaches it: never moved, never told late. The repeat is booked at 2 000, the switch at the owner's 3 ms (boundary 15 000); the owner runs on, and
    // the Underflow (tick 15 500, inside the boundary send) is read when the owner is at 9 ms.
    let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 100]]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 3);
    book_switch(&mut rig, 2, 3_000, 1);
    run(&mut rig, 3, 9);
    let boundary_send = tx_sends(&rig.device).into_iter().find(|send| send.1 == Some(rig.clock.instant(15_000)));
    assert!(boundary_send.is_some(), "the switch's boundary send was handed: {:?}", rig.device.calls());
    report(&mut rig.tx, TxCode::Underflow, &rig.clock, 15_500, 0);
    let events = drained(&rig.events);
    let r =
        events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TX_UNDERFLOW).and_then(|event| event.payload["resumes_at"]["ticks"].as_i64()).expect("the Underflow resumed the repeat");
    assert_eq!((r - 15_000) % 100, 0, "R is on NEW's lattice from its boundary: {r}");
    assert!(r >= 20_000, "{r}");
    run(&mut rig, 9, 40);
    assert_eq!(rig.device.transmitted(rig.clock.instant(r), 100), vec![Some(NEW); 100], "NEW plays again at R: {:?}", rig.device.calls());
    assert!(send_asaps(&drained(&rig.events)).is_empty());
    let row = switch_row(&rig.core, 0);
    assert_eq!((row["outcome"].as_str(), row["boundary"].as_i64()), (Some("applied"), Some(15_000)), "{row:?}");
    assert_eq!(ended_notes(&rig.core), 0, "{:?}", lock(&rig.core.rec).rejected);
}

#[test]
fn mt3d_a_missed_resume_point_moves_to_the_next_on_time_boundary_and_is_told_once() {
    // V18 (UR-22): R = 32 000 (the first test's numbers), instant 6 400 000 ticks, the device lead 400 000. A wake with `now + lead` equal to R's instant is on time:
    // the part starts at R and nothing is told. One tick later R is missed: R moves to the first W0 lattice point at or after `now + lead` = 6 400 001, which is sample
    // 32 001, so 33 000, and the part that starts there is told with `send_asap` for the repeat's own target. A second miss before the part is sent moves it again
    // (now = 40 ms: `now + lead` = 42 000, a lattice point) and tells nothing more: one event per gap, late by the whole move, 42 000 - 32 000 = 10 ms. The stall part of
    // the gap stays 10 002 samples, so an Underflow after exactly that many samples of the part resumes it again (equality), where counting the wait would end it.
    let on_time = {
        let mut rig = late_repeat();
        rig.tx.reports(Wall::ZERO);
        emitted(&rig.events);
        at(&rig, 6_000_000);
        go(&mut rig);
        assert_eq!(part_start(&rig), Some(32_000), "{:?}", rig.device.calls());
        send_asaps(&drained(&rig.events))
    };
    assert!(on_time.is_empty(), "{on_time:?}");
    let mut rig = late_repeat();
    rig.tx.reports(Wall::ZERO);
    emitted(&rig.events);
    at(&rig, 6_000_001);
    assert!(rig.tx.step());
    assert_eq!(rig.tx.open_next(), Some(33_000), "moved, nothing sent yet: {:?}", rig.device.calls());
    assert!(send_asaps(&drained(&rig.events)).is_empty(), "the start is told when the part is sent");
    at(&rig, 40 * MS);
    assert!(rig.tx.step());
    assert_eq!(rig.tx.open_next(), Some(42_000));
    go(&mut rig);
    assert_eq!(part_start(&rig), Some(42_000), "{:?}", rig.device.calls());
    assert_eq!(send_asaps(&drained(&rig.events)), [(20_000, 10_000_000)]);
    assert_eq!(resumes(&rig.core).len(), 1, "a missed resume point is not a new resume");
    // The part's burst record carries the same late start as the event (RM-14): `send_asap` by the same distance, the repeat's own target as the requested one.
    report(&mut rig.tx, TxCode::Underflow, &rig.clock, 42_000 + 10_002, 0);
    assert_eq!((ended_notes(&rig.core), resumes(&rig.core).len()), (0, 2), "equality resumes: {:?}", lock(&rig.core.rec).rejected);
    stop(&mut rig);
    let records = rows(&rig);
    let part = records.iter().find(|row| row["target"]["ticks"] == 42_000).expect("the part's record");
    assert_eq!((part["late_by"]["ticks"].as_i64(), part["requested_target"]["ticks"].as_i64()), (Some(10_000_000), Some(20_000)), "{records:#?}");
}

#[test]
fn mt3d_a_resume_point_the_device_front_has_passed_is_missed() {
    // V18, V7 (UR-22): the front of the device after an uncertain close's flush can lie past R; that is a missed R as much as a late wake. The first test's repeat with its
    // close short (the fake takes one of its two samples): the reopen's flush leaves the front at `flush`, below R = 31 000; a front moved 5 000 samples on (a longer
    // flush) is past it, and the next turn moves R to the lattice point at or after the front.
    let mut rig = late_repeat_with(vec![FakeFault::ShortSend(1)]);
    rig.tx.reports(Wall::ZERO);
    assert_eq!(emitted(&rig.events), [("late", Some(31_000))]);
    assert!(rig.tx.pass(), "the reopen and its flush");
    let front = rig.tx.chain().front().expect("the flush's front");
    rig.tx.with_book(|book| book.device.advance_front(rig.clock.domain, 31_000 + 5_000));
    at(&rig, 22 * MS);
    assert!(rig.tx.step());
    assert_eq!(rig.tx.open_next(), Some(36_000), "the first lattice point at or after 36 000 (flush front {front:?})");
}

#[test]
fn mt3d_a_missed_resume_point_reruns_the_switch_rule() {
    // V19, M5 (UR-22): after R = 20 000 a switch booked at the owner's 11 ms has the boundary 23 000, 1 000 past R: kept at first. Then R is missed and moves past it:
    // with the owner at 39.5 ms `now + lead` is 41 500, so R' = 42 000 on W0's lattice (NEW's lattice from 23 000 would give 41 500). The switch that was kept is now before
    // R', so it replaces the repeat at R' (no `stale` branch, no refusal, no row left pending), late by R' - b = 19 000 samples from its own boundary. A second miss
    // before its part is sent (the owner at 50.5 ms: `now + lead` = 52 500) moves it again on W0's lattice, 53 000 (the replaced repeat's lattice, not the switch's own
    // from 42 000, which would give 52 500), with the boundary of the switch and its lattice following; still told once, now late by 30 000 samples.
    let mut rig = gap_rig(o::ramp(1, 1_000));
    owner_at(&rig, 3 * MS);
    rig.tx.reports(Wall::ZERO);
    emitted(&rig.events);
    owner_at(&rig, 11 * MS);
    book_switch(&mut rig, 2, 12_000, 2);
    drain_tx(&mut rig.tx);
    assert_eq!(held_keys(&rig.tx), [23_000], "kept at first");
    at(&rig, 39_500 * 200);
    assert!(rig.tx.step(), "R = 20 000 is missed");
    assert_eq!(rig.tx.open_next(), Some(42_000), "now + lead = 41 500: W0's lattice gives 42 000, not NEW's 41 500");
    assert_eq!(held_keys(&rig.tx), [23_000]);
    assert!(rig.tx.step(), "the switch rule runs against the moved resume point");
    assert!(held_keys(&rig.tx).is_empty());
    assert_eq!(rig.tx.open_next(), Some(42_000));
    at(&rig, 50_500 * 200);
    assert!(rig.tx.step(), "R' = 42 000 is missed too");
    assert_eq!(rig.tx.open_next(), Some(53_000), "the replaced repeat's lattice: 52 500 would be the switch's own");
    go(&mut rig);
    assert_eq!(part_start(&rig), Some(53_000), "{:?}", rig.device.calls());
    assert_eq!(rig.device.transmitted(rig.clock.instant(53_000), 300), o::ramp(3, 300).into_iter().map(Some).collect::<Vec<_>>(), "NEW (300 samples) from its first sample at R''");
    assert_eq!(send_asaps(&drained(&rig.events)), [(12_000, 30_000_000)], "late by R'' - b, once");
    assert!(refusals(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
    sweep(&mut rig, 54, 55);
    let row = switch_row(&rig.core, 0);
    assert_eq!((row["outcome"].as_str(), row["boundary"].as_i64()), (Some("applied"), Some(53_000)), "{row:?}");
    book_switch(&mut rig, 3, 56_000, 1);
    assert!(refusals(&rig.events).is_empty(), "a later switch is accepted, on the moved boundary's lattice: {:?}", lock(&rig.core.rec).rejected);
    assert_eq!(switch_row(&rig.core, 1)["boundary"].as_i64(), Some(67_100), "55 000 + 12 000 = 67 000: the first 53 000 + 300 j at or after it (42 000 + 300 j would give 67 200)");
}

#[test]
fn mt3d_a_switch_moved_within_32_is_told_from_its_own_boundary_when_the_resume_point_then_moves_past_it() {
    // V17, V19 (UR-22): the 20-sample repeat's switch at 19 540 moves to 19 560 (the test above). The owner is then not scheduled until 40.01 ms: R = 19 520 is missed and
    // moves to the first 2 000 + 20 j at or after `now + lead` = 42 010: 42 020. The switch (now at 21 060, before R') replaces the repeat at R'; it is told once, late by
    // R' - 19 540 = 22 480 samples, measured from the boundary it had at receipt, not the one the gap moved it to (19 560).
    let mut rig = gap_rig(o::ramp(1, 20));
    owner_at(&rig, 3 * MS);
    rig.tx.reports(Wall::ZERO);
    emitted(&rig.events);
    owner_at(&rig, 7_521 * 200);
    book_switch(&mut rig, 2, 12_000, 1);
    drain_tx(&mut rig.tx);
    assert!(rig.tx.step());
    assert_eq!(held_keys(&rig.tx), [19_560]);
    at(&rig, 40_010 * 200);
    for _ in 0..2 {
        assert!(rig.tx.step(), "{:?}", rig.device.calls());
    }
    assert_eq!(rig.tx.open_next(), Some(42_020));
    assert!(held_keys(&rig.tx).is_empty(), "the switch replaced the repeat at R'");
    go(&mut rig);
    assert_eq!(part_start(&rig), Some(42_020), "{:?}", rig.device.calls());
    assert_eq!(send_asaps(&drained(&rig.events)), [(12_000, 22_480_000)]);
}

#[test]
fn mt3d_a_report_between_a_switch_s_join_and_its_open_resumes_the_repeat() {
    // V22, carry (c) (UR-22, UR-23): the switch joins the repeat's device burst at its boundary 15 000, so no booking is open when an Underflow reaches the device
    // burst; the switch is held at the burst's cursor. The repeat resumes (not abandoned, the switch not cancelled): the switch becomes the booking that resumes,
    // and plays NEW (300 samples) from R. The replaced repeat is no longer in the book, so R is taken on the switch's own lattice (15 000 + 300 j): the owner at 3 ms
    // handed sends to 13 000 (busy 13 000 + ...); the close (x = 15 000, n = 2) gives busy 15 002, B = max(25 002, 5 000, front 15 017): R = 25 200.
    let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 300]]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 3);
    book_switch(&mut rig, 2, 3_000, 1);
    drain_tx(&mut rig.tx);
    rig.tx.with_book(|book| book.open.as_mut().expect("the repeat is handed").next = 15_000);
    assert!(rig.tx.step(), "the held switch joins the open burst at its boundary");
    assert!(rig.tx.open_next().is_none());
    rig.tx.report(crate::device::TxReport { code: TxCode::Underflow, tick: Some(rig.clock.instant(15_000)), channel: 0 });
    assert_eq!(ended_notes(&rig.core), 0, "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(resumes(&rig.core).len(), 1);
    assert!(held_keys(&rig.tx).is_empty(), "the switch is the open booking now");
    assert_eq!(rig.tx.open_next(), Some(25_200));
    run(&mut rig, 3, 30);
    assert_eq!(rig.device.transmitted(rig.clock.instant(25_200), 300), vec![Some(NEW); 300], "{:?}", rig.device.calls());
    assert_eq!(send_asaps(&drained(&rig.events)), [(3_000, 10_200_000)], "told from its own boundary 15 000");
    let row = switch_row(&rig.core, 0);
    assert_eq!((row["outcome"].as_str(), row["boundary"].as_i64()), (Some("applied"), Some(25_200)), "{row:?}");
    // A SeqError in the same state still ends the repeat and cancels the switch (the twin: `ur_21_s4_a_report_between_the_join_and_the_switch_s_open_cancels_it`).
}

#[test]
fn mt3d_a_switch_joined_then_found_expired_is_moved_past_the_gap() {
    // V22 with the expired timeline (UR-22): the owner finds the device burst's cursor 15 000 passed before any report is read. The repeat resumes (it is no longer
    // abandoned there), the held switch becoming the booking that resumes at R. (Its Underflow twin is the test above.)
    let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 300]]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 3);
    book_switch(&mut rig, 2, 3_000, 1);
    drain_tx(&mut rig.tx);
    rig.tx.with_book(|book| book.open.as_mut().expect("the repeat is handed").next = 15_000);
    assert!(rig.tx.step(), "the held switch joins the open burst at its boundary");
    device_at(&rig.core, &rig.time, &rig.device, rig.core.ticks(16_000_000));
    assert!(rig.tx.step());
    assert_eq!(ended_notes(&rig.core), 0, "{:?}", lock(&rig.core.rec).rejected);
    let row = resumes(&rig.core);
    assert_eq!((row.len(), row[0]["cause"].as_str(), row[0]["from"].as_i64()), (1, Some("expired"), Some(15_000)), "{row:?}");
    assert!(held_keys(&rig.tx).is_empty());
    let r = rig.tx.open_next().expect("paused until R");
    assert_eq!((r - 15_000) % 300, 0, "{r}");
    run(&mut rig, 16, 40);
    assert_eq!(rig.device.transmitted(rig.clock.instant(r), 300), vec![Some(NEW); 300], "{:?}", rig.device.calls());
    assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied");
}

#[test]
fn mt3d_a_halt_in_the_gap_after_a_switch_replaced_the_repeat_sends_nothing_and_cancels_the_switch() {
    // V15, V17 (UR-22, RM-37): the switch replaced the paused repeat (nothing of it sent); a Stop, `Provider::stop` or a loss before R sends nothing, the record of the
    // repeat ends `stop`, and the switch is cancelled (its row, and for a Stop its Action's rejection).
    for halt in ["stop", "provider_stop", "loss"] {
        let mut rig = gap_rig(o::ramp(1, 1_000));
        owner_at(&rig, 3 * MS);
        book_switch(&mut rig, 2, 4_000, 1);
        drain_tx(&mut rig.tx);
        rig.tx.reports(Wall::ZERO);
        assert!(rig.tx.step());
        assert!(held_keys(&rig.tx).is_empty() && rig.tx.open_next() == Some(20_000), "{halt}: the switch replaced the repeat");
        let sends = tx_sends(&rig.device).len();
        let core = rig.core.clone();
        match halt {
            "stop" => stop(&mut rig),
            "provider_stop" => {
                let t = rig.core.now();
                crate::provider::test_support::end_streams(&rig.core, t, false, false);
                at(&rig, t + MS);
                rig.tx.pass();
            }
            _ => core.device_lost("scripted loss in the gap"),
        }
        sweep(&mut rig, 13, 23);
        assert_eq!(tx_sends(&rig.device).len(), sends, "{halt}: {:?}", rig.device.calls());
        if halt != "loss" {
            assert_eq!(rows(&rig).iter().map(|row| row["end"].as_str()).collect::<Vec<_>>(), [Some("stop")], "{halt}");
            assert_eq!(super::e1::notes(&core).len(), 1, "{halt}: the switch's Action is told: {:?}", lock(&core.rec).rejected);
        }
        assert_eq!(switch_row(&core, 0)["outcome"], "cancelled", "{halt}");
    }
}

#[test]
fn mt3d_a_switch_part_keeps_the_stutter_bound_of_the_gap_it_replaced() {
    // V11, V17 (UR-22, b'): the part that carries a switch across a gap resumed the repeat, so it is judged like any part: after the switch replaced the repeat at R =
    // 20 000 (P = busy + m - H = 9 502 + 10 000 - 9 500 = 10 002 samples), an Underflow after P - 1 samples of it ends the repeat; after P it resumes again.
    for (played, ends) in [(10_001, true), (10_002, false)] {
        let mut rig = gap_rig(o::ramp(1, 1_000));
        owner_at(&rig, 3 * MS);
        book_switch(&mut rig, 2, 4_000, 1);
        drain_tx(&mut rig.tx);
        rig.tx.reports(Wall::ZERO);
        sweep(&mut rig, 13, 23);
        assert_eq!(part_start(&rig), Some(20_000), "{:?}", rig.device.calls());
        assert!(held_keys(&rig.tx).is_empty());
        emitted(&rig.events);
        report(&mut rig.tx, TxCode::Underflow, &rig.clock, 20_000 + played, 0);
        assert_eq!(ended_notes(&rig.core), usize::from(ends), "after {played}: {:?}", lock(&rig.core.rec).rejected);
        assert_eq!(rig.tx.open_next().is_none(), ends, "after {played}");
    }
}

#[test]
fn mt3d_a_report_after_a_switch_s_open_and_before_its_first_send_resumes_the_repeat() {
    // V22, carry (c) (UR-22, UR-23): the switch joined the repeat's device burst at 15 000 and was taken (it is the open booking, nothing of it sent), but the window
    // reaches only one sample past its start (the owner at 5.001 ms: 5 001 + 10 000 = 15 001), so a continuation of it does not fit yet and the turn sends nothing. An
    // Underflow in that state resumes the repeat as one in the join window does: the switch becomes the paused booking, R = 25 200 on its own lattice (15 000 + 300 j,
    // the replaced repeat's lattice no longer being kept; busy 15 002 as in the join test), and it plays NEW from there.
    let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 300]]);
    book_repeat(&mut rig, 1, 2_000, 0);
    run(&mut rig, 0, 3);
    book_switch(&mut rig, 2, 3_000, 1);
    drain_tx(&mut rig.tx);
    rig.tx.with_book(|book| book.open.as_mut().expect("the repeat is handed").next = 15_000);
    assert!(rig.tx.step(), "the held switch joins the open burst at its boundary");
    at(&rig, 5_001 * 200);
    assert!(!rig.tx.step(), "the switch is taken, and its first send does not fit the window yet");
    assert_eq!(rig.tx.open_next(), Some(15_000));
    assert!(rig.tx.with_book(|book| book.open.as_ref().is_some_and(|open| open.first && open.paused.is_none())), "the switch is open with nothing sent");
    rig.tx.report(crate::device::TxReport { code: TxCode::Underflow, tick: Some(rig.clock.instant(15_000)), channel: 0 });
    assert_eq!(ended_notes(&rig.core), 0, "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!(resumes(&rig.core).len(), 1);
    assert_eq!(rig.tx.open_next(), Some(25_200));
    run(&mut rig, 6, 40);
    assert_eq!(rig.device.transmitted(rig.clock.instant(25_200), 300), vec![Some(NEW); 300], "{:?}", rig.device.calls());
    let row = switch_row(&rig.core, 0);
    assert_eq!((row["outcome"].as_str(), row["boundary"].as_i64()), (Some("applied"), Some(25_200)), "{row:?}");
}

#[test]
fn mt3d_a_repeat_joined_behind_a_finite_burst_is_not_resumed_by_a_report_in_the_join_window() {
    // V22 (UR-22): carry (c) follows a *switch* held at the device burst's cursor, as `abandon` does, not any held repeat. A finite burst of 1 000 samples (6 000..7 000)
    // is booked first and a plain repeat is held at its end, 7 000, where the owner joins it. A report between the finite burst's end and the repeat's open ends the device burst as it did:
    // a note, no resume, no `resumes_at` on the event (the repeat has not played, so nothing of it is paused).
    let mut rig = switch_rig(vec![o::ramp(1, 1_000), o::ramp(2, 100)]);
    book(&mut rig, 1, 0, 6_000);
    // The record would refuse the repeat's booking (it meets the finite burst's front); the owner's state is what is tested, so the repeat is held directly.
    let mut repeat = super::held(&rig.clock, 7_000, 100, LatePolicy::DropAndFlag);
    repeat.repeat = true;
    super::hold(&rig.tx, repeat);
    let mut joined = false;
    for ms in 1..=8 {
        at(&rig, ms * MS);
        for _ in 0..8 {
            drain_tx(&mut rig.tx);
            let worked = rig.tx.step();
            if rig.tx.open_next().is_none() && held_keys(&rig.tx) == [7_000] {
                joined = true;
                break;
            }
            if !worked {
                break;
            }
        }
        if joined {
            break;
        }
    }
    assert!(joined, "the repeat joined the finite burst's device burst: {:?} {:?} {:?}", rig.device.calls(), refusals(&rig.events), lock(&rig.core.rec).rejected);
    report(&mut rig.tx, TxCode::Underflow, &rig.clock, 6_500, 0);
    assert_eq!(emitted(&rig.events), [("underflow", None)], "{:?}", lock(&rig.core.rec).rejected);
    assert_eq!((resumes(&rig.core).len(), ended_notes(&rig.core)), (0, 1), "{:?}", lock(&rig.core.rec).rejected);
}

#[test]
fn mt3d_a_resume_point_or_a_switch_boundary_the_clock_cannot_hold_ends_the_repeat_with_a_note() {
    // V14 (UR-22): the owner's gap turn ends the repeat by the abandon at H, with a note, when the point it must move to does not fit the clock. (1) a missed resume
    // point: the device front is moved to the end of the clock's samples, so R is missed and its next point on the lattice does not exist. (2) a switch within 32 samples
    // after a resume point at the end of the clock: its first safe boundary does not exist.
    let top = i64::MAX / 200 - 1;
    let mut rig = late_repeat();
    rig.tx.reports(Wall::ZERO);
    rig.tx.with_book(|book| book.device.advance_front(rig.clock.domain, top));
    assert!(rig.tx.step());
    assert_eq!((ended_notes(&rig.core), rig.tx.open_next()), (1, None), "{:?}", lock(&rig.core.rec).rejected);
    assert!(lock(&rig.core.rec).rejected.iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("resume point does not fit the clock"))), "{:?}", lock(&rig.core.rec).rejected);
    let mut rig = late_repeat();
    rig.tx.reports(Wall::ZERO);
    rig.tx.with_book(|book| book.open.as_mut().expect("paused").next = top);
    let mut held = super::held(&rig.clock, top + 10, 100, LatePolicy::SendAsapAndFlag);
    (held.repeat, held.switch) = (true, true);
    super::hold(&rig.tx, held);
    assert!(rig.tx.step());
    assert_eq!((ended_notes(&rig.core), rig.tx.open_next()), (1, None), "{:?}", lock(&rig.core.rec).rejected);
    assert!(
        lock(&rig.core.rec).rejected.iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("switch's boundary does not fit the clock"))),
        "{:?}",
        lock(&rig.core.rec).rejected
    );
}

#[test]
fn mt3d_a_switch_the_gap_moved_and_a_report_then_resumed_is_told_from_its_receipt_boundary() {
    // V17, V22 (UR-22): the 20-sample repeat's switch at 19 540 moves to 19 560 (R + 40, joining the part sent at R = 19 520). When the part's first send is done the
    // switch joins its device burst, and an Underflow between the join and the switch's open resumes the repeat with the switch as the paused booking. It is told once,
    // late by its new start less the boundary it was received at (19 540), not the 19 560 the first gap moved it to.
    let mut rig = gap_rig(o::ramp(1, 20));
    owner_at(&rig, 3 * MS);
    rig.tx.reports(Wall::ZERO);
    emitted(&rig.events);
    owner_at(&rig, 7_521 * 200);
    book_switch(&mut rig, 2, 12_000, 1);
    drain_tx(&mut rig.tx);
    assert!(rig.tx.step());
    assert_eq!(held_keys(&rig.tx), [19_560]);
    at(&rig, 12 * MS);
    let mut joined = false;
    for _ in 0..12 {
        drain_tx(&mut rig.tx);
        rig.tx.step();
        if rig.tx.open_next().is_none() && held_keys(&rig.tx) == [19_560] {
            joined = true;
            break;
        }
    }
    assert!(joined, "the switch joined the part's device burst: {:?}", rig.device.calls());
    assert_eq!(part_start(&rig), Some(19_520));
    // The part started at 19 520 played at least the stall part of its gap, P = 10 002 samples (the scripted tick 30 000), so the repeat resumes again.
    report(&mut rig.tx, TxCode::Underflow, &rig.clock, 30_000, 0);
    assert_eq!((ended_notes(&rig.core), resumes(&rig.core).len()), (0, 2), "{:?}", lock(&rig.core.rec).rejected);
    let r2 = rig.tx.open_next().expect("paused until R2");
    sweep(&mut rig, 13, r2 / 1_000 + 1);
    assert_eq!(part_start(&rig), Some(r2), "{:?}", rig.device.calls());
    assert_eq!(send_asaps(&drained(&rig.events)), [(12_000, (r2 - 19_540) * 1_000)], "late from the boundary at receipt, once");
}

// ---------------------------------------------------------------------------------------------------------------------------------------------------
// Sol's code review of the step (sol-code.md): a lost sequence in the gap, and a delayed report of a stopped repeat.
// ---------------------------------------------------------------------------------------------------------------------------------------------------

#[test]
fn mt3d_a_lost_sequence_while_the_repeat_is_paused_ends_it() {
    // V14 (UR-22, sol-code major 1): a sequence loss ends a repeat, running or waiting to resume. The repeat is paused until R = 32 000 (no device burst open) when a
    // `SeqError` or `SeqErrorInBurst` is read: the repeat ends by the abandon at H, with the note, no send at or after R, and its record ends; a booking 5 000 samples past H is
    // on time and accepted (the footprint is H + 5, as when the repeat was running).
    for code in [TxCode::SeqError, TxCode::SeqErrorInBurst] {
        let mut rig = late_repeat();
        rig.tx.reports(Wall::ZERO);
        assert_eq!(rig.tx.open_next(), Some(32_000), "paused");
        let (sends, before) = (tx_sends(&rig.device).len(), emitted(&rig.events));
        assert_eq!(before, [("late", Some(32_000))]);
        rig.tx.report(TxReport { code, tick: None, channel: 0 });
        assert_eq!((ended_notes(&rig.core), rig.tx.open_next()), (1, None), "{code:?}: {:?}", lock(&rig.core.rec).rejected);
        for ms in [23, 24, 33] {
            at(&rig, ms * MS);
            go(&mut rig);
        }
        assert_eq!(tx_sends(&rig.device).len(), sends, "{code:?}: no send at R: {:?}", rig.device.calls());
        book(&mut rig, 7, 1, 21_000 + 5_000);
        assert!(super::e2a::refused_ids(&rig.events).is_empty(), "{code:?}: {:?}", lock(&rig.core.rec).rejected);
        assert_eq!(rows(&rig).iter().map(|row| row["end"].as_str()).collect::<Vec<_>>(), [Some("stop")], "{code:?}");
    }
}

#[test]
fn mt3d_a_stopped_repeat_s_delayed_report_does_not_end_a_newer_repeat_that_waits_to_resume() {
    // V3b, V12 (UR-22, sol-code major 2): A resumes at R = 32 000 and its part starts there. The part's Underflow after P - 1 = 10 001 samples is not read yet when A is stopped
    // (its correlation stays pending) and the repeat B, booked after the Stop's region at 60 000, is handed ahead and found expired: it pauses until RB. The delayed
    // Underflow (tick 42 001, before B's start) is then read: it tells A's part and retires nothing of B, whose pause is none of its business, so B resumes untouched.
    let mut rig = resumed_part();
    report_nothing_yet(&mut rig);
    stop(&mut rig);
    book_repeat(&mut rig, 3, 60_000, 0);
    at(&rig, 52 * MS);
    go(&mut rig);
    assert!(tx_sends(&rig.device).iter().any(|send| send.2 && send.1 == Some(rig.clock.instant(60_000))), "B started: {:?}", rig.device.calls());
    at(&rig, 70 * MS);
    assert!(rig.tx.step(), "B's timeline expired: it pauses");
    let rb = rig.tx.open_next().expect("B is paused until RB");
    assert!(rb > 70_000, "{rb}");
    let resumes_before = resumes(&rig.core).len();
    emitted(&rig.events);
    report(&mut rig.tx, TxCode::Underflow, &rig.clock, 42_001, 0);
    assert_eq!(emitted(&rig.events), [("underflow", None)], "A's part is told, with no plan");
    assert_eq!((ended_notes(&rig.core), rig.tx.open_next(), resumes(&rig.core).len()), (0, Some(rb), resumes_before), "B waits for RB untouched: {:?}", lock(&rig.core.rec).rejected);
    at(&rig, (rb / 1_000 - 9) * MS);
    go(&mut rig);
    at(&rig, (rb / 1_000 + 1) * MS);
    go(&mut rig);
    assert_eq!(part_start(&rig), Some(rb), "B's part starts at RB: {:?}", rig.device.calls());
}

/// The unread report of `resumed_part`'s part is simply not read: nothing to do, but the test says it.
fn report_nothing_yet(rig: &mut SwitchRig) {
    assert!(drained(&rig.events).is_empty(), "no report was read since the part was sent");
}
