//! The recovery flush (T3) as a confirmed, bounded retry (UR-22, #75): the strict attempt, the loop that runs it on fresh streamers, and the
//! single attempt the open at `arm` keeps while `OPEN_FLUSH_CONFIRMED` is off. The device's clock is the fake's manual one, held by the test
//! thread as a participant (`clock_guard`): it stands still while the test reads it and runs while the Core waits on it, so every instant is exact.

use super::*;
use crate::ClockMode;
use crate::device::{FakeConfig, FakeDevice, FakeFault, TxCode, TxReport};
use crate::provider::test_support::{LockSpy, ReportFeed, clock_guard, flush_attempt_rows, probe, rig_on, rig_with, t3_calls};
use ezsdr_kernel::event::EventCollector;
use std::sync::atomic::AtomicUsize;

fn config(faults: Vec<FakeFault>) -> FakeConfig {
    FakeConfig { clock: ClockMode::Manual, faults, ..FakeConfig::default() }
}

fn ms(core: &Core, n: i64) -> i64 {
    core.ticks(n * 1_000_000)
}

fn zeros() -> [Iq; 4] {
    [Iq::default(); 4]
}

/// The events' `COMMAND_REJECTED` kinds, in order.
fn rejected_kinds(core: &Core, events: &EventCollector) -> Vec<String> {
    events
        .drain(core.at(core.now()))
        .into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED)
        .map(|event| event.payload["kind"].as_str().unwrap_or_default().to_owned())
        .collect()
}

fn flush_acks(core: &Core) -> Vec<(Option<u64>, Option<i64>)> {
    lock(&core.rec).device_async.iter().filter(|row| row["tx_flush"] == true && row["code"] == "BurstAck").map(|row| (row["channel"].as_u64(), row["tick"].as_i64())).collect()
}

/// A manual-clock rig held still: the Core, the fake, the clock's hold and the events.
fn held_rig(faults: Vec<FakeFault>) -> (Arc<Core>, Arc<FakeDevice>, crate::host::Driver, Arc<EventCollector>) {
    // Held before the Core is built: the manual clock runs while nothing holds it, and the faults' instants count from the device's start.
    let device = Arc::new(FakeDevice::new(config(faults)));
    let guard = clock_guard(&device.host());
    let (core, _time, events) = rig_on(device.clone(), Vec::new(), 2_000);
    (core, device, guard, events)
}

/// Device time a clock run on a loaded host may add to an exact wait of the attempts (a sleep ends when its thread runs again): well under
/// one attempt's 100 ms.
const SLACK_MS: i64 = 40;

/// The T3 calls' shape of `attempts` attempts on fresh streamers, `lead` calls before them.
fn shape(lead: &[&'static str], attempts: usize, last_closed: bool) -> Vec<&'static str> {
    let mut calls = lead.to_vec();
    for attempt in 0..attempts {
        if attempt > 0 {
            calls.push("close");
        }
        calls.extend(["open", "send"]);
    }
    if last_closed {
        calls.push("close");
    }
    calls
}

/// A strict attempt over a feed: the result, the Core and the device time `p` it read before its send (the clock is held at it).
struct Strict {
    result: TxFlushResult,
    core: Arc<Core>,
    p: i64,
    elapsed: i64,
}

/// One strict attempt on `nch` channels with `faults`; `script` gives `BurstAck`s `(channel, tick − p)` (`None`: tickless) before it reads, and
/// `not_before` is relative to `p`.
fn strict(faults: Vec<FakeFault>, nch: usize, script: &[(usize, Option<i64>)], not_before: Option<i64>) -> Strict {
    let inner = Arc::new(FakeDevice::new(config(faults)));
    let feed = Arc::new(ReportFeed::new(inner.clone()));
    let (core, _time, _events) = rig_on(feed.clone(), Vec::new(), 2_000);
    let _guard = clock_guard(&inner.host());
    let channels: Vec<usize> = (0..nch).collect();
    feed.tx_open(0, &channels).unwrap();
    let p = inner.time_now().unwrap();
    for (channel, tick) in script {
        feed.give(TxReport { code: TxCode::BurstAck, tick: tick.map(|delta| p + delta), channel: *channel });
    }
    let attempt = Attempt { number: 1, not_before: not_before.map_or(i64::MIN, |delta| p + delta) };
    let result = core.tx_flush_attempt(0, &channels, Some(attempt));
    let elapsed = inner.time_now().unwrap() - p;
    Strict { result, core, p, elapsed }
}

#[test]
fn ur_75_t3_ends_on_the_first_confirmed_attempt_on_a_fresh_streamer() {
    for reopen in [false, true] {
        for (k, faults) in [(1, vec![]), (2, vec![FakeFault::DropAckOfSend(0)]), (3, vec![FakeFault::DropAckOfSend(0), FakeFault::DropAckOfSend(1)])] {
            let (core, device, _guard, events) = held_rig(faults.clone());
            if reopen {
                device.tx_open(0, &[0]).unwrap();
            }
            let start = device.time_now().unwrap();
            let confirmed = if reopen { core.reopen_tx_streamer(0, Channels::first(1), i64::MIN) } else { core.open_tx_confirmed(0, Channels::first(1), i64::MIN) };
            let confirmed = confirmed.unwrap_or_else(|error| panic!("{faults:?}: {error:?}"));
            let lead: &[&str] = if reopen { &["open", "close"] } else { &[] };
            assert_eq!(t3_calls(&device), shape(lead, k, false), "{faults:?} (reopen {reopen})");
            let waited = ms(&core, 100) * (k as i64 - 1);
            assert!(confirmed.ack_tick >= start + waited, "the ack is attempt {k}'s, not one carried from attempt 1: {} vs {}", confirmed.ack_tick, start + waited);
            let elapsed = device.time_now().unwrap() - start;
            assert!(elapsed >= waited && elapsed < waited + ms(&core, SLACK_MS), "{faults:?}: {elapsed}");
            let rows = flush_attempt_rows(&core);
            assert_eq!(rows.iter().map(|row| row["attempt"].as_u64().unwrap()).collect::<Vec<_>>(), (1..k as u64).collect::<Vec<_>>(), "{rows:?}");
            assert_eq!(flush_acks(&core).len(), 1, "one flush ack row: {:?}", lock(&core.rec).device_async);
            assert!(events.drain(core.at(core.now())).is_empty(), "a recovered flush emits nothing");
            assert_eq!(device.tx_send(0, &[&zeros()[..]], None, true, true, Wall::from_millis(1)).unwrap(), 4, "one streamer is left open ({faults:?})");
            if reopen {
                assert_eq!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).generation, 1, "the record moves on once for the operation");
            }
        }
    }
}

#[test]
fn ur_75_a_send_that_is_not_whole_ends_its_attempt_at_once() {
    for (fault, text) in
        [(FakeFault::StalledSend(0), "flush send accepted 0 of 4 samples"), (FakeFault::ShortSend(0), "flush send accepted 2 of 4 samples"), (FakeFault::FailSend(0), "flush send failed")]
    {
        let spy = Arc::new(LockSpy::new(config(vec![fault.clone()])));
        let (core, _time, events) = rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        let _guard = clock_guard(&spy.host());
        let start = spy.time_now().unwrap();
        let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
        assert!(result.is_ok(), "{fault:?}: attempt 2 confirms: {result:?}");
        let order = spy.order();
        let send = order.iter().position(|call| *call == "tx_send").unwrap();
        let close = order.iter().position(|call| *call == "tx_close").unwrap();
        assert!(order[send..close].iter().all(|call| *call != "tx_async"), "{fault:?}: no wait for an ack that cannot come: {order:?}");
        assert!(spy.time_now().unwrap() - start < ms(&core, SLACK_MS), "{fault:?}");
        let rows = flush_attempt_rows(&core);
        assert_eq!(rows.len(), 1, "{rows:?}");
        let error = rows[0]["error"].as_str().unwrap();
        assert!(error.contains(text) && !error.contains("BurstAck missing"), "{fault:?}: {error}");
        assert!(rejected_kinds(&core, &events).is_empty(), "{fault:?}: a failed send is a row, not an event (F2)");
    }
}

#[test]
fn ur_75_three_unconfirmed_attempts_fail_and_close_their_streamer() {
    let whole = [FakeFault::DropBurstAck];
    let cases: Vec<(Vec<FakeFault>, i64, &str)> = vec![
        (whole.to_vec(), 300, "BurstAck missing"),
        (vec![FakeFault::StalledSend(0), FakeFault::StalledSend(1), FakeFault::StalledSend(2)], 0, "accepted 0 of 4"),
        (vec![FakeFault::ShortSend(0), FakeFault::ShortSend(1), FakeFault::ShortSend(2)], 0, "accepted 2 of 4"),
        (vec![FakeFault::FailSend(0), FakeFault::FailSend(1), FakeFault::FailSend(2)], 0, "flush send failed"),
        (vec![FakeFault::DropAckOfSend(0), FakeFault::StalledSend(1), FakeFault::ShortSend(2)], 100, "accepted 2 of 4"),
    ];
    for reopen in [false, true] {
        for (faults, waited, last) in &cases {
            let (core, device, _guard, events) = held_rig(faults.clone());
            if reopen {
                device.tx_open(0, &[0]).unwrap();
            }
            let start = device.time_now().unwrap();
            let result = if reopen { core.reopen_tx_streamer(0, Channels::first(1), i64::MIN).map(|_| ()) } else { core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ()) };
            let error = result.expect_err("three unconfirmed attempts end the loop");
            assert!(!error.lost && error.message.contains("after 3 attempts") && error.message.contains(last), "{faults:?}: {}", error.message);
            let lead: &[&str] = if reopen { &["open", "close"] } else { &[] };
            assert_eq!(t3_calls(&device), shape(lead, 3, true), "{faults:?}");
            let rows = flush_attempt_rows(&core);
            assert_eq!(rows.iter().map(|row| row["attempt"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2, 3], "{rows:?}");
            let elapsed = device.time_now().unwrap() - start;
            assert!(elapsed >= ms(&core, *waited) && elapsed < ms(&core, *waited + SLACK_MS), "{faults:?}: {} ms", core.ns(elapsed) / 1_000_000);
            assert!(device.tx_send(0, &[&zeros()[..]], None, true, true, Wall::from_millis(1)).is_err(), "nothing is left open ({faults:?})");
            assert!(!core.is_lost());
            assert!(events.drain(core.at(core.now())).is_empty(), "the loop emits nothing itself: the Provider reports the failure ({faults:?})");
            if reopen {
                let streams = lock(&core.streams);
                let state = streams.stream(stream_key(Dir::Tx, 0));
                assert_eq!((state.set, state.generation), (DeviceSet::Unknown, 0), "{faults:?}");
            }
        }
    }
}

#[test]
fn ur_75_a_stray_ack_never_confirms_an_attempt_whose_send_put_nothing_out() {
    let far = Some(i64::MAX / 2);
    let stray = || TxReport { code: TxCode::BurstAck, tick: far, channel: 0 };
    for fault in [FakeFault::StalledSend(0), FakeFault::ShortSend(0)] {
        // (a) and (c): one strict attempt.
        let inner = Arc::new(FakeDevice::new(config(vec![fault.clone()])));
        let feed = Arc::new(ReportFeed::new(inner));
        let (core, _time, _events) = rig_on(feed.clone(), Vec::new(), 2_000);
        feed.tx_open(0, &[0]).unwrap();
        feed.give(stray());
        let result = core.tx_flush_attempt(0, &[0], Some(Attempt { number: 1, not_before: i64::MIN }));
        assert!(!result.acknowledged, "{fault:?}");
        assert_eq!(feed.reads(0), 0, "{fault:?}: no read after a send that put nothing out");
        assert!(feed.tx_async(0, Wall::ZERO).unwrap().is_some(), "{fault:?}: the stray ack is still queued");
        assert!(flush_acks(&core).is_empty(), "{fault:?}");
    }
    // (b) the loop.
    let inner = Arc::new(FakeDevice::new(config(vec![FakeFault::StalledSend(0), FakeFault::StalledSend(1), FakeFault::StalledSend(2)])));
    let feed = Arc::new(ReportFeed::new(inner));
    let (core, _time, _events) = rig_on(feed.clone(), Vec::new(), 2_000);
    for _ in 0..3 {
        feed.give(stray());
    }
    assert!(core.open_tx_confirmed(0, Channels::first(1), i64::MIN).is_err());
    assert_eq!(feed.reads(0), 0);
    assert!(flush_acks(&core).is_empty());
}

#[test]
fn ur_75_a_whole_send_is_confirmed_only_by_every_channel_position() {
    // Attempt 1: channel 0's ack only; attempt 2 collects both itself, and attempt 1's ack is not carried.
    let (core, _device, _guard, _events) = held_rig(vec![FakeFault::DropAckOfSend(0), FakeFault::TxReport(Wall::ZERO, TxCode::BurstAck)]);
    let confirmed = core.open_tx_confirmed(0, Channels::first(2), i64::MIN).map(|_| ());
    confirmed.unwrap();
    assert_eq!(flush_acks(&core).iter().map(|row| row.0).collect::<Vec<_>>(), [Some(0), Some(0), Some(1)]);
    let rows = flush_attempt_rows(&core);
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["error"].as_str().unwrap().contains("channel positions [1]"), "{rows:?}");

    let (core, _device, _guard, _events) = held_rig(vec![FakeFault::DropBurstAck, FakeFault::TxReport(Wall::ZERO, TxCode::BurstAck)]);
    let result = core.open_tx_confirmed(0, Channels::first(2), i64::MIN).map(|_| ());
    assert!(result.is_err(), "a partial ack never confirms");
    assert_eq!(flush_acks(&core).iter().map(|row| row.0).collect::<Vec<_>>(), [Some(0)]);
    assert_eq!(flush_attempt_rows(&core).len(), 3);
}

#[test]
fn ur_75_the_confirmed_tick_is_the_largest_ack_tick_across_positions() {
    for (first, second) in [(1_000, 2_000), (2_000, 1_000)] {
        let inner = Arc::new(FakeDevice::new(config(vec![FakeFault::DropAckOfSend(0)])));
        let feed = Arc::new(ReportFeed::new(inner.clone()));
        let (core, _time, _events) = rig_on(feed.clone(), Vec::new(), 2_000);
        feed.tx_open(0, &[0, 1]).unwrap();
        let _guard = clock_guard(&inner.host());
        let p = inner.time_now().unwrap();
        feed.give(TxReport { code: TxCode::BurstAck, tick: Some(p + first), channel: 0 });
        feed.give(TxReport { code: TxCode::BurstAck, tick: Some(p + second), channel: 1 });
        let confirmed = core.reopen_tx_streamer(0, Channels::first(2), i64::MIN).map(|confirmed| confirmed.ack_tick);
        assert_eq!(confirmed.unwrap(), p + 2_000, "({first}, {second})");
    }
}

#[test]
fn ur_75_an_ack_older_than_the_floor_is_not_this_attempts() {
    let drop = || vec![FakeFault::DropAckOfSend(0)];
    // A positive control: with nothing scripted and no fault the fake's own ack confirms, at a tick at or after `p`.
    let control = strict(Vec::new(), 1, &[], None);
    assert!(control.result.acknowledged && control.result.ack_tick.unwrap() >= control.p);
    // (a) stale first, no genuine one: unconfirmed after 100 ms, the stale ack kept.
    let a = strict(drop(), 1, &[(0, Some(-1))], None);
    assert!(!a.result.acknowledged);
    assert!(flush_acks(&a.core).is_empty());
    assert!(a.elapsed >= ms(&a.core, 100));
    assert_eq!(a.core.take_stashed_tx_report(0).map(|report| report.tick), Some(Some(a.p - 1)), "the stale ack is stashed, not lost");
    // (b) the boundary on both sides.
    for (delta, confirmed) in [(-1, false), (0, true), (1, true)] {
        let b = strict(drop(), 1, &[(0, Some(delta))], None);
        assert_eq!((b.result.acknowledged, b.result.ack_tick), if confirmed { (true, Some(b.p + delta)) } else { (false, None) }, "tick p{delta:+}");
    }
    // (c) stale, then genuine, on the same position: the genuine one is the flush's.
    let c = strict(drop(), 1, &[(0, Some(-5)), (0, Some(5))], None);
    assert_eq!((c.result.acknowledged, c.result.ack_tick), (true, Some(c.p + 5)));
    assert_eq!(flush_acks(&c.core), [(Some(0), Some(c.p + 5))]);
    assert_eq!(c.core.take_stashed_tx_report(0).map(|report| report.tick), Some(Some(c.p - 5)));
    // (d) tickless never confirms, and a genuine one after it does.
    let d = strict(drop(), 1, &[(0, None)], None);
    assert!(!d.result.acknowledged);
    assert!(d.result.problem.as_deref().is_some_and(|text| text.contains("BurstAck missing")));
    assert!(d.core.take_stashed_tx_report(0).is_some_and(|report| report.tick.is_none()));
    let d2 = strict(drop(), 1, &[(0, None), (0, Some(3))], None);
    assert_eq!((d2.result.acknowledged, d2.result.ack_tick), (true, Some(d2.p + 3)));
    // (e) mixed positions.
    let e = strict(drop(), 2, &[(0, Some(2)), (1, Some(-2))], None);
    assert!(!e.result.acknowledged && e.result.problem.as_deref().is_some_and(|text| text.contains("channel positions [1]")), "{:?}", e.result.problem);
    let e = strict(drop(), 2, &[(0, Some(-2)), (1, Some(2))], None);
    assert!(!e.result.acknowledged && e.result.problem.as_deref().is_some_and(|text| text.contains("channel positions [0]")), "{:?}", e.result.problem);
    let e = strict(drop(), 2, &[(0, Some(2)), (1, Some(4))], None);
    assert_eq!((e.result.acknowledged, e.result.ack_tick), (true, Some(e.p + 4)));
    // (g) the old burst's padded end raises the floor, by `max`.
    let g = strict(drop(), 1, &[(0, Some(999))], Some(1_000));
    assert!(!g.result.acknowledged, "an ack at or above p but below the old burst's bound is the old burst's");
    assert!(g.core.take_stashed_tx_report(0).is_some());
    let g = strict(drop(), 1, &[(0, Some(1_000))], Some(1_000));
    assert_eq!((g.result.acknowledged, g.result.ack_tick), (true, Some(g.p + 1_000)));
    let g = strict(drop(), 1, &[(0, Some(0))], Some(-500));
    assert!(g.result.acknowledged, "a bound below p leaves the floor at p");
    let g = strict(Vec::new(), 1, &[], Some(1_000_000));
    assert!(!g.result.acknowledged, "the fake's own genuine ack is below a bound far ahead: the cost of the rule is a retry");
    assert!(strict(Vec::new(), 1, &[], Some(0)).result.acknowledged);
}

#[test]
fn ur_75_a_stale_ack_stays_for_the_report_owner_across_the_loop() {
    // Attempt 1 reads a stale ack (and the fake's own is dropped): unconfirmed; attempt 2 confirms on its own ack. The stale ack is kept for the report
    // owner, which records it raw, before attempt 2's open clears the stash; the loop writes it nowhere itself.
    let inner = Arc::new(FakeDevice::new(config(vec![FakeFault::DropAckOfSend(0)])));
    let feed = Arc::new(ReportFeed::new(inner.clone()));
    let (core, _time, _events) = rig_on(feed.clone(), Vec::new(), 2_000);
    let _guard = clock_guard(&inner.host());
    let p = inner.time_now().unwrap();
    feed.give(TxReport { code: TxCode::BurstAck, tick: Some(p - 1), channel: 0 });
    let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
    result.unwrap();
    assert_eq!(core.take_stashed_tx_report(0).map(|report| (report.code, report.tick)), Some((TxCode::BurstAck, Some(p - 1))));
    assert!(core.take_stashed_tx_report(0).is_none());
    let rec = lock(&core.rec);
    assert!(rec.device_async.iter().all(|row| row["tx_flush"] == true), "the loop records only the flush's own ack: {:?}", rec.device_async);
    assert_eq!(rec.device_async.len(), 1, "attempt 2's own ack");
}

#[test]
fn ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap() {
    let reads = |core: &Arc<Core>| {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        probe(core, "t3_time_read", move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        count
    };
    // The device's clock starts at an instant of the test's own, set once the clock is held: the faults' windows are relative to it, and a delay
    // before the hold cannot move them past the attempts.
    let origin = |after_ms: u64| Wall::from_secs(10) + Wall::from_millis(after_ms);
    let run = |faults: Vec<FakeFault>| {
        let (core, device, _guard, events) = held_rig(faults);
        device.set_time(core.ticks(10_000_000_000));
        (core, device, _guard, events)
    };
    // (a) transient: attempt 2's reads at 100, 101 and 102 ms fail; the one at 103 ms sets the floor, and the attempt sends and confirms.
    let (core, device, _guard, _events) = run(vec![FakeFault::DropAckOfSend(0), FakeFault::TimeReadFailsFor(origin(100), Wall::from_millis(3))]);
    let count = reads(&core);
    let start = device.time_now().unwrap();
    let confirmed = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|confirmed| confirmed.ack_tick);
    assert!(confirmed.unwrap() >= start + ms(&core, 103), "the floor is the later read, not the first failed instant");
    assert_eq!(t3_calls(&device), shape(&[], 2, false));
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert_eq!(flush_attempt_rows(&core).len(), 1);
    assert!(device.time_now().unwrap() - start < ms(&core, 103 + SLACK_MS));
    // (b) a failing read is not the attempt's: it fails for 500 ms, then succeeds and the attempt's own 100 ms starts: it sends and confirms, and the
    // recovery ends with no rejection (attempt 3 is never needed).
    let (core, device, _guard, _events) = run(vec![FakeFault::DropAckOfSend(0), FakeFault::TimeReadFailsFor(origin(100), Wall::from_millis(500))]);
    let count = reads(&core);
    let start = device.time_now().unwrap();
    let confirmed = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|confirmed| confirmed.ack_tick);
    assert!(confirmed.unwrap() >= start + ms(&core, 600), "the floor is the first read that succeeded, after the 500 ms of failures");
    assert_eq!(t3_calls(&device), shape(&[], 2, false));
    assert!(count.load(Ordering::SeqCst) >= 400, "a read every 1 ms, not one per attempt");
    assert_eq!(flush_attempt_rows(&core).len(), 1, "no row for the failing reads");
    assert!(device.time_now().unwrap() - start < ms(&core, 600 + SLACK_MS));
    // (b2) the failing reads' time does not come out of the attempt's window: the send after them has its whole 100 ms for the ack.
    let (core, device, _guard, _events) = run(vec![FakeFault::DropAckOfSend(0), FakeFault::TimeReadFailsFor(origin(100), Wall::from_millis(500)), FakeFault::DropAckOfSend(1)]);
    let start = device.time_now().unwrap();
    let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
    result.unwrap();
    assert_eq!(t3_calls(&device), shape(&[], 3, false), "attempt 2 sent and waited its 100 ms for an ack that never came; attempt 3 confirmed");
    assert!(device.time_now().unwrap() - start >= ms(&core, 100 + 500 + 100));
    // (c) the cap: reads that fail for more than 2 s end the recovery `Failed` (not lost) with the cause, no third attempt, and the streamer closed.
    let (core, device, _guard, _events) = run(vec![FakeFault::DropAckOfSend(0), FakeFault::TimeReadFailsFor(origin(100), Wall::from_secs(10))]);
    let start = device.host().now();
    let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
    let error = result.expect_err("the reads never succeed");
    assert!(!error.lost && error.message.contains("flush time read failed: ") && !error.message.contains("Provider::stop"), "{}", error.message);
    assert_eq!(t3_calls(&device), ["open", "send", "close", "open", "close"]);
    let rows = flush_attempt_rows(&core);
    assert!(rows[0]["error"].as_str().unwrap().contains("BurstAck missing"));
    assert!(rows.len() == 2 && rows[1]["error"].as_str().unwrap().starts_with("flush time read failed") && rows[1]["attempt"] == 2, "{rows:?}");
    let elapsed = device.host().now() - start;
    assert!(elapsed >= Wall::from_millis(2_100) && elapsed < Wall::from_millis(2_100 + SLACK_MS as u64), "{elapsed:?}");
    // (c2) a read that fails for 1.9 s and then succeeds is within the cap: the recovery goes on.
    let (core, device, _guard, _events) = run(vec![FakeFault::DropAckOfSend(0), FakeFault::TimeReadFailsFor(origin(100), Wall::from_millis(1_900))]);
    let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
    result.unwrap();
    assert_eq!(t3_calls(&device), shape(&[], 2, false));
    // (c3) the cap counts from the first failure of one run of reads, and a read that succeeds ends the run: failures apart add up to nothing.
    let (core, device, _guard, _events) = run(vec![
        FakeFault::DropAckOfSend(0),
        FakeFault::DropAckOfSend(1),
        FakeFault::TimeReadFailsFor(origin(100), Wall::from_millis(1_500)),
        FakeFault::TimeReadFailsFor(origin(1_650), Wall::from_millis(1_500)),
    ]);
    let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
    result.unwrap();
    assert_eq!(t3_calls(&device), shape(&[], 3, false));
    // (c) a Stop or a loss between the retries ends the attempt at once.
    for stop in [true, false] {
        let (core, device, _guard, events) = run(vec![FakeFault::DropAckOfSend(0), FakeFault::TimeReadFailsFor(origin(100), Wall::from_secs(1))]);
        let count = Arc::new(AtomicUsize::new(0));
        let (seen, owner) = (count.clone(), core.clone());
        probe(&core, "t3_time_read", move |_| {
            if seen.fetch_add(1, Ordering::SeqCst) + 1 == 3 {
                if stop {
                    lock(&owner.streams).stop = Some((owner.now(), StopMode::Orderly));
                } else {
                    owner.device_lost("test loss between time reads");
                }
            }
        });
        let start = device.host().now();
        let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
        let error = result.expect_err("interrupted");
        assert_eq!(error.lost, !stop, "{}", error.message);
        assert_eq!(error.message.contains("Provider::stop"), stop, "{}", error.message);
        assert!(device.host().now() - start < Wall::from_millis(100 + SLACK_MS as u64), "the attempt ended at once, not after its 100 ms");
        assert_eq!(t3_calls(&device), ["open", "send", "close", "open"]);
        assert_eq!(count.load(Ordering::SeqCst), 3, "no read after the publication: not after the back-off either");
        assert_eq!(flush_attempt_rows(&core).len(), 1, "no row for the interrupted attempt");
        let lost_events = events.drain(core.at(core.now())).iter().filter(|event| event.kind.as_str() == EventKind::DEVICE_LOST).count();
        assert_eq!(lost_events, usize::from(!stop));
    }
    // (d) lost at the read: attempt 2's read sleeps past the instant the device is lost.
    let (core, device, _guard, events) = run(vec![FakeFault::DropAckOfSend(0), FakeFault::Lost(origin(250))]);
    let slow = device.clone();
    probe(&core, "t3_between", move |_| *slow.slow_next_read.lock().unwrap() = Some(Wall::from_millis(200)));
    let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
    assert!(result.expect_err("lost at the read").lost);
    assert_eq!(t3_calls(&device), ["open", "send", "close", "open"], "no send after the lost read");
    assert_eq!(events.drain(core.at(core.now())).iter().filter(|event| event.kind.as_str() == EventKind::DEVICE_LOST).count(), 1);
    // (d2) a loss while the reads fail ends the loop as a loss, not as a failed recovery: reads fail from 100 ms, the device is lost at 600 ms.
    let (core, device, _guard, events) = run(vec![FakeFault::DropAckOfSend(0), FakeFault::TimeReadFailsFor(origin(100), Wall::from_secs(10)), FakeFault::Lost(origin(600))]);
    let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
    let error = result.expect_err("lost while the reads fail");
    assert!(error.lost, "{}", error.message);
    assert_eq!(t3_calls(&device), ["open", "send", "close", "open"], "no send, no close and no third attempt");
    assert_eq!(flush_attempt_rows(&core).len(), 1, "no row for the reads");
    assert_eq!(events.drain(core.at(core.now())).iter().filter(|event| event.kind.as_str() == EventKind::DEVICE_LOST).count(), 1);
    // (e) the read succeeds, but only after 100 ms: that time is not the attempt's, which sends and has its whole window.
    let (core, device, _guard, _events) = run(vec![FakeFault::DropAckOfSend(0)]);
    let slow = device.clone();
    let fired = Arc::new(AtomicUsize::new(0));
    let seen = fired.clone();
    probe(&core, "t3_between", move |_| {
        if seen.fetch_add(1, Ordering::SeqCst) == 0 {
            *slow.slow_next_read.lock().unwrap() = Some(Wall::from_millis(100));
        }
    });
    let start = device.time_now().unwrap();
    let confirmed = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|confirmed| confirmed.ack_tick);
    assert!(confirmed.unwrap() >= start + ms(&core, 200), "the floor is the read that returned at 200 ms");
    let rows = flush_attempt_rows(&core);
    assert_eq!(rows.iter().map(|row| row["attempt"].as_u64().unwrap()).collect::<Vec<_>>(), [1], "{rows:?}");
    assert_eq!(t3_calls(&device), shape(&[], 2, false));
}

/// How a boundary's publication is made: `Provider::stop` or the device's loss.
fn publish(core: &Core, stop: bool) {
    if stop {
        lock(&core.streams).stop = Some((core.now(), StopMode::Orderly));
    } else {
        core.device_lost("test loss at a boundary of the retry");
    }
}

#[test]
fn ur_75_stop_and_loss_at_each_boundary_of_the_retry() {
    // G1: before the close (`t3_between`); G2: inside the second open, before its device call; G3: after the second open returned.
    for gate in ["t3_between", "open", "open_returned"] {
        for stop in [true, false] {
            let spy = Arc::new(LockSpy::new(config(vec![FakeFault::DropAckOfSend(0)])));
            let (core, _time, events) = rig_on(spy.clone(), Vec::new(), 2_000);
            spy.watch(&core);
            let at = Arc::new(AtomicUsize::new(usize::MAX));
            let fired = Arc::new(AtomicUsize::new(0));
            let (owner, seen, mark, order) = (core.clone(), fired.clone(), at.clone(), spy.clone());
            probe(&core, gate, move |_| {
                let boundary = if gate == "t3_between" { 1 } else { 2 };
                if seen.fetch_add(1, Ordering::SeqCst) + 1 == boundary {
                    mark.store(order.order().len(), Ordering::SeqCst);
                    publish(&owner, stop);
                }
            });
            let reads = Arc::new(AtomicUsize::new(0));
            let counted = reads.clone();
            probe(&core, "t3_time_read_returned", move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
            });
            let _guard = clock_guard(&spy.host());
            let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
            let error = result.expect_err("interrupted");
            let what = format!("{gate} stop {stop}");
            assert_eq!(reads.load(Ordering::SeqCst), 1, "{what}: attempt 1's pre-send read only: no device call of attempt 2 follows the publication");
            assert_eq!(error.lost, !stop, "{what}: {}", error.message);
            assert_eq!(error.message.contains("Provider::stop"), stop, "{what}: {}", error.message);
            assert_eq!(core.is_lost(), !stop, "{what}");
            assert_eq!(flush_attempt_rows(&core).len(), 1, "{what}: no row for the interrupted attempt");
            let order = spy.order();
            let after: Vec<_> = order[at.load(Ordering::SeqCst)..].iter().filter(|call| matches!(**call, "tx_close" | "tx_open" | "tx_send" | "tx_async")).copied().collect();
            let want: &[&str] = match (gate, stop) {
                ("t3_between", _) | ("open", false) | ("open_returned", _) => &[],
                _ => &["tx_open"],
            };
            assert_eq!(after, want, "{what}: {order:?}");
            assert_eq!(order.iter().filter(|call| **call == "tx_send").count(), 1, "{what}: one T3 in all");
            let lost_events = events.drain(core.at(core.now())).iter().filter(|event| event.kind.as_str() == EventKind::DEVICE_LOST).count();
            assert_eq!(lost_events, usize::from(!stop), "{what}");
        }
    }
}

#[test]
fn ur_75_a_stop_or_loss_published_around_a_successful_pre_send_time_read_sends_no_t3() {
    // The read returns its time, but the publication was made while it was in flight (the probe runs after it returned, before the attempt looks at
    // the result): the attempt must not send, in attempt 1 and in attempt 2 alike.
    for (nth, stop) in [(1, true), (1, false), (2, true), (2, false)] {
        let (core, device, _guard, events) = held_rig(vec![FakeFault::DropAckOfSend(0)]);
        let fired = Arc::new(AtomicUsize::new(0));
        let (owner, seen) = (core.clone(), fired.clone());
        probe(&core, "t3_time_read_returned", move |_| {
            if seen.fetch_add(1, Ordering::SeqCst) + 1 == nth {
                publish(&owner, stop);
            }
        });
        let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
        let what = format!("read {nth}, stop {stop}");
        let error = result.expect_err("interrupted");
        assert_eq!(error.lost, !stop, "{what}: {}", error.message);
        assert_eq!(error.message.contains("Provider::stop"), stop, "{what}: {}", error.message);
        let want: &[&str] = if nth == 1 { &["open"] } else { &["open", "send", "close", "open"] };
        assert_eq!(t3_calls(&device), want, "{what}: no send after the publication");
        assert_eq!(fired.load(Ordering::SeqCst), nth, "{what}: no read after the publication");
        assert_eq!(flush_attempt_rows(&core).len(), nth - 1, "{what}: no row for the interrupted attempt");
        let lost_events = events.drain(core.at(core.now())).iter().filter(|event| event.kind.as_str() == EventKind::DEVICE_LOST).count();
        assert_eq!(lost_events, usize::from(!stop), "{what}");
    }
}

#[test]
fn ur_75_a_stop_between_the_close_and_the_open_opens_nothing() {
    // The Stop arrives while attempt 2's close is held in the device (it holds `device_gate`, which a loss would wait for: only a Stop is
    // injectable here); the loop meets it before the open. Twenty runs: the held close must make the order the same every time.
    for _ in 0..20 {
        let spy = Arc::new(LockSpy::new(config(vec![FakeFault::DropAckOfSend(0)])));
        let (core, _time, _events) = rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        let gate = spy.stop_after("tx_close");
        let opener = core
            .device
            .host()
            .spawn("t3-stop-after-close", {
                let core = core.clone();
                move || {
                    let _ = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ()).expect_err("stopped");
                }
            })
            .unwrap();
        gate.wait();
        lock(&core.streams).stop = Some((core.now(), StopMode::Orderly));
        gate.release();
        opener.join().unwrap();
        let order = spy.order();
        let close = order.iter().position(|call| *call == "tx_close").unwrap();
        assert!(order[close..].iter().all(|call| !matches!(*call, "tx_open" | "tx_send" | "tx_async")), "{order:?}");
        assert!(!core.is_lost());
    }
}

#[test]
fn ur_75_an_open_that_fails_between_attempts_ends_the_loop() {
    // The rig's own open is #0, attempt 1's #1, attempt 2's #2.
    let (core, device, _guard, _events) = held_rig(vec![FakeFault::DropAckOfSend(0), FakeFault::TxOpenFails(2)]);
    device.tx_open(0, &[0]).unwrap();
    let result = core.reopen_tx_streamer(0, Channels::first(1), i64::MIN).map(|_| ());
    let error = result.expect_err("the open is refused");
    assert!(error.message.contains("scripted refusal of transmit open") && !error.message.contains("after 3 attempts"), "{}", error.message);
    assert_eq!(t3_calls(&device).iter().filter(|call| **call == "send").count(), 1, "no T3 after the refused open");
    assert_eq!(flush_attempt_rows(&core).len(), 1);
    assert_eq!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).set, DeviceSet::Unknown);
    // The rate check runs once, on the confirmed streamer.
    let (core, device, _guard, _events) = held_rig(vec![FakeFault::DropAckOfSend(0), FakeFault::RatesMismatch(0)]);
    device.tx_open(0, &[0]).unwrap();
    let result = core.reopen_tx_streamer(0, Channels::first(1), i64::MIN).map(|_| ());
    assert!(result.expect_err("the rates differ").message.contains("UR-13"));
    assert_eq!(t3_calls(&device).iter().filter(|call| **call == "send").count(), 2, "the check came after the confirmed attempt 2");
}

#[test]
fn ur_75_reports_stashed_in_an_attempt_survive_the_next_open_for_the_report_owner() {
    // The owner's normal report handling (UR-28) makes their events, so the loop keeps them, in order and ahead of the last attempt's own, rather
    // than writing them raw or letting the next open clear them.
    let (core, _device, _guard, _events) = held_rig(vec![FakeFault::DropAckOfSend(0), FakeFault::TxReport(Wall::ZERO, TxCode::Other(77)), FakeFault::TxReport(Wall::ZERO, TxCode::SeqError)]);
    let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
    result.unwrap();
    let kept: Vec<_> = std::iter::from_fn(|| core.take_stashed_tx_report(0)).map(|report| report.code).collect();
    assert_eq!(kept, [TxCode::Other(77), TxCode::SeqError], "{kept:?}");
    let rec = lock(&core.rec);
    assert!(rec.device_async.iter().all(|row| row["tx_flush"] == true), "{:?}", rec.device_async);
}

#[test]
fn ur_75_a_report_read_after_attempt_1_s_deadline_stays_for_the_report_owner() {
    // The read of attempt 1 returns an Underflow after its deadline (held 120 ms of real time): stashed as the Run's, and still there after attempt 2.
    let faults = vec![FakeFault::DropAckOfSend(0), FakeFault::TxReport(Wall::ZERO, TxCode::Underflow)];
    let spy = Arc::new(LockSpy::new(FakeConfig { clock: ClockMode::Real, faults, ..FakeConfig::default() }));
    let (core, _time, _events) = rig_on(spy.clone(), Vec::new(), 2_000);
    spy.watch(&core);
    let gate = spy.stop_after("tx_async");
    let (done, finished) = std::sync::mpsc::channel();
    let opener = core
        .device
        .host()
        .spawn("t3-late-underflow", {
            let core = core.clone();
            move || {
                let _ = done.send(core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ()));
            }
        })
        .unwrap();
    gate.wait();
    std::thread::sleep(Wall::from_millis(120));
    gate.release();
    opener.join().unwrap();
    finished.recv().unwrap().unwrap();
    assert_eq!(core.take_stashed_tx_report(0).map(|report| report.code), Some(TxCode::Underflow));
    assert!(core.take_stashed_tx_report(0).is_none());
    assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "Underflow"), "a late report is not the flush's");
}

/// All three attempts fail: attempt 1's stash and the later ones' are all the Run's, in order, none lost to an open.
#[test]
fn ur_75_after_three_failed_attempts_every_stash_stays_the_runs() {
    let (core, _device, _guard, _events) = held_rig(vec![FakeFault::DropBurstAck, FakeFault::TxReport(Wall::ZERO, TxCode::Other(77))]);
    let result = core.open_tx_confirmed(0, Channels::first(1), i64::MIN).map(|_| ());
    assert!(result.is_err());
    assert_eq!(core.take_stashed_tx_report(0).map(|report| report.code), Some(TxCode::Other(77)), "attempt 1's report survived the later opens");
    assert!(core.take_stashed_tx_report(0).is_none());
}

#[test]
fn ur_75_with_the_switch_off_the_open_flush_is_todays() {
    let off = std::hint::black_box(OPEN_FLUSH_CONFIRMED);
    assert!(!off, "this test pins the switch-off path: at the flip it is rewritten for the loop");
    for fault in [FakeFault::DropBurstAck, FakeFault::StalledSend(0), FakeFault::ShortSend(0), FakeFault::FailSend(0)] {
        let spy = Arc::new(LockSpy::new(config(vec![fault.clone()])));
        let (core, _time, events) = rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        let _guard = clock_guard(&spy.host());
        let start = spy.time_now().unwrap();
        let result = core.open_streamer(Dir::Tx, 0, Channels::first(1));
        assert!(result.is_ok(), "{fault:?}: the open flush never refuses: {result:?}");
        let order = spy.order();
        let send = order.iter().position(|call| *call == "tx_send").unwrap();
        assert!(order[send..].contains(&"tx_async"), "{fault:?}: today's 100 ms read runs even after a send that put nothing out: {order:?}");
        assert!(!order.contains(&"tx_close"), "{fault:?}: the streamer stays open");
        assert!(spy.time_now().unwrap() - start >= ms(&core, 100), "{fault:?}");
        let rows: Vec<_> = lock(&core.rec).rejected.iter().filter(|row| row["call"] == "tx_flush").cloned().collect();
        assert_eq!(rows.len(), 1, "{fault:?}: {rows:?}");
        assert!(rows[0].get("attempt").is_none(), "{fault:?}: the single attempt writes no attempt number");
        let kinds = rejected_kinds(&core, &events);
        assert_eq!(kinds, if matches!(fault, FakeFault::FailSend(_)) { vec!["tx_flush".to_owned()] } else { Vec::new() }, "{fault:?}: only a failed send keeps its event");
        assert_eq!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).set, DeviceSet::Open(Channels::first(1)), "{fault:?}");
        assert!(spy.tx_send(0, &[&zeros()[..]], None, true, true, Wall::from_millis(1)).is_ok(), "{fault:?}: a usable send follows");
    }
    // (d) the same faults, failing every attempt, end the strict recovery in an error: the flush the open at `arm` leaves unconfirmed is
    // a refusal there.
    for faults in [
        vec![FakeFault::DropBurstAck],
        vec![FakeFault::StalledSend(0), FakeFault::StalledSend(1), FakeFault::StalledSend(2)],
        vec![FakeFault::ShortSend(0), FakeFault::ShortSend(1), FakeFault::ShortSend(2)],
        vec![FakeFault::FailSend(0), FakeFault::FailSend(1), FakeFault::FailSend(2)],
    ] {
        let (core, device, _guard, _events) = held_rig(faults.clone());
        device.tx_open(0, &[0]).unwrap();
        let result = core.reopen_tx_streamer(0, Channels::first(1), i64::MIN).map(|_| ());
        assert!(result.is_err(), "{faults:?}");
    }
}

#[test]
fn ur_75_the_open_flush_at_arm_keeps_todays_ack_rule() {
    // A stale and a tickless ack are accepted at the open with the switch off, and refused by the strict function.
    for tick in [Some(-1), None] {
        let inner = Arc::new(FakeDevice::new(config(vec![FakeFault::DropAckOfSend(0)])));
        let feed = Arc::new(ReportFeed::new(inner));
        let (core, _time, _events) = rig_on(feed.clone(), Vec::new(), 2_000);
        feed.give(TxReport { code: TxCode::BurstAck, tick, channel: 0 });
        core.open_streamer(Dir::Tx, 0, Channels::first(1)).unwrap();
        assert_eq!(flush_acks(&core), [(Some(0), tick)], "{tick:?}: the legacy flush counts any ack of an expected position");
        assert!(lock(&core.rec).rejected.iter().all(|row| row["call"] != "tx_flush"), "{tick:?}");
        let offset = tick.map(|_| -1);
        let strict = strict(vec![FakeFault::DropAckOfSend(0)], 1, &[(0, offset)], None);
        assert!(!strict.result.acknowledged, "{tick:?}: the same report is refused by the strict flush");
    }
}

#[test]
fn ur_75_the_owner_s_mid_run_open_keeps_todays_flush_while_the_switch_is_off() {
    // The owner's open of a transmit stream has no production caller today (`owner_configure` has only receive callers): a unit plan stands in.
    use super::super::control::ColdConfig;
    use ezsdr_kernel::time::Rational;
    use ezsdr_radio::timeline::Segment;
    let (core, device, _time, events) = rig_with(config(vec![FakeFault::DropBurstAck]), Vec::new());
    let key = stream_key(Dir::Tx, 0);
    let planned = {
        let mut streams = lock(&core.streams);
        streams.stream_mut(key).configs.insert(1, Arc::new(ColdConfig::new(BTreeMap::new())));
        let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
        let mut line = Line::new(timeline::Stream { direction: Direction::Tx, origin: 0, config, lead: 0, call: 2_000, allowance: 0 });
        line.plan = vec![Segment { origin: 0, cut: None, config, by: Some(1) }];
        streams.lines.insert((Direction::Tx, 0), line);
        streams.planned(key)
    };
    assert!(core.owner_configure(Dir::Tx, 0, &planned[0]).unwrap(), "an unconfirmed open flush does not refuse the owner's open");
    assert_eq!(flush_attempt_rows(&core).len(), 0);
    assert_eq!(lock(&core.rec).rejected.iter().filter(|row| row["call"] == "tx_flush").count(), 1, "the single attempt's row");
    assert_eq!(t3_calls(&device), ["open", "send"], "no retry, no close");
    assert!(rejected_kinds(&core, &events).is_empty());
    assert_eq!(device.tx_send(0, &[&zeros()[..]], None, true, true, Wall::from_millis(1)).unwrap(), 4, "the streamer is usable");
}
