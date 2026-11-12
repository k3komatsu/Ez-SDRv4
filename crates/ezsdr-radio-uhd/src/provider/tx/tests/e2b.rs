//! E2b-1 (design note 49 stage E2b; plan `2026-11-12-e2b/test-plan.md` §3.1, §3.3): today's configure and reopen turns pinned before
//! `Configure` and `Reopen` become operations of the book. Each characterization ends with one ordered snapshot of what the turn left
//! (device calls, the records and the events, §3.1 "Snapshots") taken from the base run. The configure tests run on the test thread with
//! the device's clock frozen and device time moved only by `device_at`; a test that reopens the streamer runs the owner's work as a clock
//! participant (`released`), so that the flush's wait runs at clock speed (note 51 §3). Probes and `on_next_lock` hooks run on the owner's
//! thread with no lock held.
//!
//! Units: one sample is 1 us (the rigs' 1 MS/s), 200 root ticks; `MS` is a millisecond in root ticks.

use super::e1::{MS, at, notes, stop_receipt};
use super::*;
use crate::provider::test_support::{handed_over, next_lock_armed, on_next_lock, probe, released};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const T3: &str = "tx_send 0 n=4 at=- sob=true eob=true";

fn key0() -> ezsdr_radio::timeline::StreamKey {
    stream_key(Dir::Tx, 0)
}

fn mark(core: &Core) -> Option<u64> {
    lock(&core.streams).stream(key0()).configured
}

/// The events emitted since the last read, as `{kind, payload}` in emission order, appended to `log` for the snapshot.
fn read_events(events: &EventCollector, log: &mut Vec<Value>) -> Vec<Value> {
    let read: Vec<Value> =
        events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().map(|event| json!({ "kind": event.kind.as_str(), "payload": event.payload })).collect();
    log.extend(read.iter().cloned());
    read
}

/// The `(action, reason)` of each `COMMAND_REJECTED` among `read`.
fn refusals_in(read: &[Value]) -> Vec<(Value, Value)> {
    read.iter().filter(|event| event["kind"] == ezsdr_radio::kinds::COMMAND_REJECTED).map(|event| (event["payload"]["action"].clone(), event["payload"]["reason"].clone())).collect()
}

/// Everything the turns left, in one value: the device's calls, the Provider's records (the lock-wait maxima left out: they measure
/// contention, not behaviour, plan §3.1) and every event, the ones `log` already read first.
fn snapshot(core: &Core, device: &FakeDevice, events: &EventCollector, mut log: Vec<Value>) -> Value {
    read_events(events, &mut log);
    let rec = lock(&core.rec);
    let stats: BTreeMap<&str, i64> = rec.stats.iter().filter(|(name, _)| **name != "tx_lock_wait_max_ns").map(|(name, n)| (*name, *n)).collect();
    json!({
        "calls": device.calls(),
        "applied": rec.applied,
        "bursts": rec.bursts,
        "rejected": rec.rejected,
        "switches": rec.switches,
        "timing": rec.timing,
        "device_async": rec.device_async,
        "stats": stats,
        "events": log,
    })
}

/// A snapshot as the expectations are written: one top-level field a line, an array's items one a line.
fn render(snapshot: &Value) -> String {
    let fields = snapshot.as_object().expect("a snapshot");
    let mut out = String::from("{\n");
    for (i, (name, value)) in fields.iter().enumerate() {
        let comma = if i + 1 < fields.len() { "," } else { "" };
        match value {
            Value::Array(items) if !items.is_empty() => {
                out.push_str(&format!("\"{name}\": [\n"));
                for (j, item) in items.iter().enumerate() {
                    out.push_str(&format!("  {item}{}\n", if j + 1 < items.len() { "," } else { "" }));
                }
                out.push_str(&format!("]{comma}\n"));
            }
            _ => out.push_str(&format!("\"{name}\": {value}{comma}\n")),
        }
    }
    out.push('}');
    out
}

/// The snapshot equals today's, written out in `expected`.
fn check(what: &str, actual: Value, expected: &str) {
    let expected: Value = serde_json::from_str(expected).expect("the expected snapshot is JSON");
    assert!(actual == expected, "{what}: the snapshot differs from today's:\n{}", render(&actual));
}

/// `Tx::shutdown` with the device's clock released for its 100 ms acknowledgement window, as `e1::shutdown` makes it.
fn shutdown_owner(core: &Core, tx: &mut Tx, frozen: Frozen) {
    let _driver = handed_over(frozen, &core.device.host());
    tx.shutdown();
}

/// The owner's turns at the current device time, as the run loop makes them.
fn turn(tx: &mut Tx) {
    for _ in 0..8 {
        drain_tx(tx);
        if !tx.step() {
            break;
        }
    }
}

/// Transmit stream 0 with uhd-control on its record, built as `ur_21_a_refused_burst_stays_counted_until_its_booked_end` builds it
/// (`radio.tx.c0.stream = 0`, the connect values F1 on the device, generation 0 published), with `rig.waves[i]` for each of `waves`.
fn control_rig(waves: Vec<Vec<Iq>>, faults: Vec<FakeFault>) -> SwitchRig {
    control_rig_with(waves, faults, |device| device)
}

/// `control_rig` with the Core on `wrap` of the fake (the rig's `device` stays the fake, whose calls the snapshot reads).
fn control_rig_with(waves: Vec<Vec<Iq>>, faults: Vec<FakeFault>, wrap: impl FnOnce(Arc<FakeDevice>) -> Arc<dyn Device>) -> SwitchRig {
    let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, faults, ..FakeConfig::default() }));
    let mut inputs = BTreeMap::new();
    let mut refs = Vec::new();
    for (i, samples) in waves.iter().enumerate() {
        let (wave, bytes) = waveform_input(&format!("e2b_{i}"), samples);
        inputs.insert(wave.hash.clone(), Arc::<[u8]>::from(bytes));
        refs.push(wave);
    }
    let (core, time, events) = crate::provider::test_support::rig_on_inputs(wrap(device.clone()), Vec::new(), 2_000, inputs);
    let frozen = Frozen::of(&device);
    device_at(&core, &time, &device, 0);
    device.tx_open(0, &[0]).unwrap();
    device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), freq: Some(F1), ..Settings::default() }).unwrap();
    let clock = core.register(Dir::Tx, 200, 0).unwrap();
    lock(&core.streams).record_configured(key0(), Some(0));
    let mut config = core.description.defaults();
    config.insert(crate::provider::core::key("radio.tx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
    let mut start = crate::provider::control::Start { t0: clock.origin, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] };
    start.tx[0] = Some((clock, Channels::first(1)));
    let control = Control::new(core.clone(), Arc::new(NoActions), config, false, start);
    let (_to_tx, cmds) = std::sync::mpsc::channel();
    let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
    SwitchRig { core, device, time, events, control, tx, clock, waves: refs, _frozen: frozen }
}

/// A Stop as uhd-control books it (`control.rs:317,322`): the record's stop at `now`, then the halt on the list; the bound's sample, none
/// when the record has no clock (the owner rigs register theirs with the Core alone).
fn full_stop(core: &Core) -> Option<i64> {
    let mut streams = lock(&core.streams);
    let now = core.now();
    let clock = streams.stream(key0()).clock;
    let bound = streams.stream_mut(key0()).stop_at(now, core.tx_in_flight());
    streams.stream_mut(key0()).book.list.halt(ezsdr_radio::CancelReason::Stop);
    core.release_streams(streams);
    clock.zip(bound).map(|(clock, bound)| clock.at_or_after(bound))
}

/// uhd-control shared with a probe or a hook, and taken back before the test ends: one left in a probe would keep the Core from ever
/// dropping (UR-33's drop check).
type Shared = Arc<Mutex<Option<Control>>>;

fn sent(device: &FakeDevice) -> bool {
    device.calls().iter().any(|call| call.starts_with("tx_send ") && call != T3)
}

// ------------------------------------------------------------------------------------------------------------ the seam

#[test]
fn on_next_lock_runs_once_before_the_next_lock_of_its_mutex() {
    let (a, b) = (Arc::new(Mutex::new(0)), Mutex::new(0));
    let ran = Arc::new(AtomicUsize::new(0));
    let (hook_a, hook_ran) = (a.clone(), ran.clone());
    on_next_lock(&a, move || {
        // Before the call's acquisition: the mutex is free.
        assert!(hook_a.try_lock().is_ok(), "the hook runs before the lock is taken");
        hook_ran.fetch_add(1, Ordering::SeqCst);
        let again = hook_ran.clone();
        on_next_lock(&hook_a, move || {
            again.fetch_add(10, Ordering::SeqCst);
        });
    });
    drop(lock(&b));
    assert_eq!((ran.load(Ordering::SeqCst), next_lock_armed()), (0, true), "another mutex's lock does not fire it");
    drop(lock(&a));
    assert_eq!((ran.load(Ordering::SeqCst), next_lock_armed()), (1, true), "it fired once, and armed the next");
    drop(lock(&a));
    assert_eq!((ran.load(Ordering::SeqCst), next_lock_armed()), (11, false), "the hook armed inside fires on the call after");
    drop(lock(&a));
    assert_eq!(ran.load(Ordering::SeqCst), 11, "a hook runs once");
}

// ------------------------------------------------------------------------------------------------------------ the configure turn

#[test]
fn e2b_c0_a_configure_records_its_applied_rows_before_the_burst_s_first_send() {
    let (core, device, time, mut tx, _frozen, events, _to_tx) = owner(1);
    let clock = tx.clock.expect("a clock");
    let at_mark = Arc::new(Mutex::new(None));
    let (seen, weak, probe_device) = (at_mark.clone(), Arc::downgrade(&core), device.clone());
    probe(&core, "tx_configure_before_mark", move |_| {
        let Some(core) = weak.upgrade() else { return };
        *seen.lock().unwrap() = Some((probe_device.calls(), lock(&core.rec).applied.clone()));
    });
    give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 1));
    turns(&core, &time, &device, &mut tx, 1, 30, 1);
    let (calls, applied) = at_mark.lock().unwrap().take().expect("the configure ran");
    assert!(!calls.iter().any(|call| call.starts_with("tx_send ")), "no send before the configure's result: {calls:?}");
    assert!(applied.iter().any(|row| row["key"].as_str().is_some_and(|key| key.ends_with("frequency_hz")) && row["claimed"].as_f64() == Some(F2)), "{applied:?}");
    let traffic = traffic(&device, 1);
    assert_eq!(traffic.iter().filter(|call| call.starts_with("apply tx")).count(), 1, "{traffic:?}");
    let first = traffic.iter().find(|call| call.starts_with("tx_send ")).expect("X is sent");
    assert!(first.contains(&format!("at={} sob=true", clock.instant(20_000))), "{traffic:?}");
    check("C0", snapshot(&core, &device, &events, Vec::new()), C0);
}

#[test]
fn e2b_c1_a_configure_waits_for_the_stream_claim() {
    let (core, device, time, mut tx, _frozen, events, _to_tx) = owner(1);
    let clock = tx.clock.expect("a clock");
    give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 1));
    device_at(&core, &time, &device, 15 * MS);
    let claim = lock(&core.streams).claim_owner(key0()).expect("the test holds the stream");
    assert!(!tx.step(), "a refused claim makes no configure turn");
    assert!(!traffic(&device, 1).iter().any(|call| call.starts_with("apply tx")), "{:?}", device.calls());
    assert_eq!(held_keys(&tx), [20_000], "X stays held");
    assert_eq!(mark(&core), Some(0));
    lock(&core.streams).give_back(claim);
    turns(&core, &time, &device, &mut tx, 15, 30, 1);
    let traffic = traffic(&device, 1);
    let apply = traffic.iter().position(|call| call.starts_with("apply tx")).expect("the configure runs once the claim is back");
    assert!(traffic[apply..].iter().any(|call| call.contains(&format!("at={} sob=true", clock.instant(20_000)))), "X is sent after it: {traffic:?}");
    check("C1", snapshot(&core, &device, &events, Vec::new()), C1);
}

#[test]
fn e2b_c2_a_full_stop_during_the_configure_settles_x_and_keeps_y() {
    // At the configure's probe, uhd-control receives a Stop at 15 ms (its record removes X's chain: X starts after the Stop) and then Y at
    // the Stop's bound; X's result comes after both. Y carries X's generation: the refusal is a one-shot one (`ApplyFails`), so that Y's
    // own configure succeeds and Y plays.
    for refused in [false, true] {
        let what = if refused { "C2 refused" } else { "C2 configured" };
        let wrap = move |device: Arc<FakeDevice>| -> Arc<dyn Device> { if refused { Arc::new(crate::provider::test_support::ApplyFails::new(device, Dir::Tx, Some(0))) } else { device } };
        let mut rig = control_rig_with(vec![vec![OLD; 4], vec![NEW; 200]], Vec::new(), wrap);
        book_frequency(&mut rig.control, &rig.core, 1, F2);
        book_burst_on(&mut rig.control, &rig.core, 2, 0, rig.clock, 20_000, &rig.waves[0], false, LatePolicy::DropAndFlag);
        at(&rig, 15 * MS);
        let SwitchRig { core, device, time, events, control, mut tx, clock, waves, _frozen } = rig;
        let control: Shared = Arc::new(Mutex::new(Some(control)));
        let bound = Arc::new(Mutex::new(None));
        let (stop_bound, hook_control, weak, wave, fired) = (bound.clone(), control.clone(), Arc::downgrade(&core), waves[1].clone(), AtomicBool::new(false));
        probe(&core, "tx_configure_before_mark", move |_| {
            if let (false, Some(core)) = (fired.swap(true, Ordering::SeqCst), weak.upgrade()) {
                let bound = full_stop(&core).expect("the record has the stream's clock");
                book_burst_on(hook_control.lock().unwrap().as_mut().expect("the rig's control"), &core, 4, 0, clock, bound, &wave, false, LatePolicy::DropAndFlag);
                *stop_bound.lock().unwrap() = Some(bound);
            }
        });
        drain_tx(&mut tx);
        assert!(tx.step(), "{what}: the configure turn");
        assert_eq!(mark(&core), if refused { None } else { Some(1) }, "{what}");
        let bound = bound.lock().unwrap().expect("the Stop came during the configure");
        assert!(bound > 20_004, "{what}: Y is after X");
        turn(&mut tx);
        let mut log = Vec::new();
        let read = read_events(&events, &mut log);
        if refused {
            assert_eq!(refusals_in(&read).iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(), [json!(2)], "{what}: {read:?}");
            assert!(notes(&core).is_empty(), "{what}: the refused X is not also cancelled: {:?}", lock(&core.rec).rejected);
        } else {
            assert!(refusals_in(&read).is_empty(), "{what}: {read:?}");
            assert_eq!(notes(&core), ["UR-26: cancelled by Stop"], "{what}");
        }
        book_burst_on(control.lock().unwrap().as_mut().expect("the rig's control"), &core, 5, 0, clock, bound + 100, &waves[1], false, LatePolicy::DropAndFlag);
        let read = read_events(&events, &mut log);
        assert_eq!(refusals_in(&read), [(json!(5), json!("UR-21: the burst overlaps an accepted booking"))], "{what}: Y's chain survived X's result: {read:?}");
        turns(&core, &time, &device, &mut tx, 16, 45, 1);
        assert!(device.transmitted(clock.instant(20_000), 4).iter().all(Option::is_none), "{what}: nothing of X: {:?}", device.calls());
        assert!(device.transmitted(clock.instant(bound), 200).iter().all(|sample| *sample == Some(NEW)), "{what}: Y plays whole: {:?}", device.calls());
        control.lock().unwrap().take();
        check(what, snapshot(&core, &device, &events, log), if refused { C2_REFUSED } else { C2_CONFIGURED });
    }
}

#[test]
fn e2b_c3_a_stop_after_the_configure_result_cancels_the_burst_at_the_next_turn() {
    let mut rig = control_rig(vec![vec![OLD; 4]], Vec::new());
    book_frequency(&mut rig.control, &rig.core, 1, F2);
    book_burst_on(&mut rig.control, &rig.core, 2, 0, rig.clock, 20_000, &rig.waves[0], false, LatePolicy::DropAndFlag);
    at(&rig, 15 * MS);
    drain_tx(&mut rig.tx);
    assert!(rig.tx.step(), "the configure turn");
    assert_eq!((mark(&rig.core), held_keys(&rig.tx)), (Some(1), vec![20_000]), "configured, and X held");
    stop_receipt(&mut rig);
    assert!(rig.tx.step(), "the next turn reads the halt first");
    assert_eq!(notes(&rig.core), ["UR-26: cancelled by Stop"]);
    assert!(held_keys(&rig.tx).is_empty());
    run(&mut rig, 16, 30);
    assert!(!sent(&rig.device), "{:?}", rig.device.calls());
    check("C3", snapshot(&rig.core, &rig.device, &rig.events, Vec::new()), C3);
}

#[test]
fn e2b_c4_a_loss_around_the_configure() {
    // (i) the loss lands at the mark, (ii) between two channels (the configure fails as lost), (iii) after the turn returned.
    for case in ["C4(i)", "C4(ii)", "C4(iii)"] {
        let channels = if case == "C4(ii)" { 2 } else { 1 };
        let (core, device, time, mut tx, frozen, events, _to_tx) = owner(channels);
        let clock = tx.clock.expect("a clock");
        let weak = Arc::downgrade(&core);
        if case == "C4(i)" {
            probe(&core, "tx_configure_before_mark", move |_| {
                if let Some(core) = weak.upgrade() {
                    core.device_lost("test: lost before the configure's mark");
                }
            });
        } else if case == "C4(ii)" {
            let applies = AtomicUsize::new(0);
            probe(&core, "apply", move |_| {
                if let (1, Some(core)) = (applies.fetch_add(1, Ordering::SeqCst), weak.upgrade()) {
                    core.device_lost("test: lost between the configure's channels");
                }
            });
        }
        give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, channels));
        if case == "C4(iii)" {
            device_at(&core, &time, &device, 15 * MS);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: the configure turn");
            assert_eq!(mark(&core), Some(1), "{case}");
            core.device_lost("test: lost after the configure's result");
            assert!(!tx.step(), "{case}: the step finds the loss");
        } else {
            turns(&core, &time, &device, &mut tx, 1, 15, 1);
        }
        assert!(core.is_lost(), "{case}");
        assert_eq!(mark(&core), None, "{case}");
        assert!(!sent(&device), "{case}: {:?}", device.calls());
        let mut log = Vec::new();
        let read = read_events(&events, &mut log);
        assert!(refusals_in(&read).is_empty(), "{case}: {read:?}");
        assert_eq!(read.iter().filter(|event| event["kind"] == EventKind::DEVICE_LOST).count(), 1, "{case}: {read:?}");
        shutdown_owner(&core, &mut tx, frozen);
        let expected_notes: &[&str] = if case == "C4(ii)" { &[] } else { &["UR-26: cancelled by Provider::stop"] };
        assert_eq!(notes(&core), expected_notes, "{case}");
        let expected = match case {
            "C4(i)" => C4_I,
            "C4(ii)" => C4_II,
            _ => C4_III,
        };
        check(case, snapshot(&core, &device, &events, log), expected);
    }
}

#[test]
fn e2b_c5_a_switch_during_its_repeat_s_configure_is_refused_until_the_repeat_sends() {
    // KL-41: R (a SendAsap repeat at 20 ms with a new value) is configured at 12 ms, and the configure runs until 21 ms, past R's start, so
    // that the record's chain of R has begun when the switch is received (before it, a switch is refused for no playing repeat). Nothing
    // of R is issued until its first send: a switch is refused as not started until then.
    let mut rig = control_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]], Vec::new());
    book_frequency(&mut rig.control, &rig.core, 1, F2);
    book_with(&mut rig.control, &rig.core, 2, 0, rig.clock, 20_000, &rig.waves[0], true, LatePolicy::SendAsapAndFlag, BTreeMap::new());
    at(&rig, 12 * MS);
    let SwitchRig { core, device, time, events, control, mut tx, clock, waves, _frozen } = rig;
    let control: Shared = Arc::new(Mutex::new(Some(control)));
    let at_configure = Arc::new(Mutex::new(None));
    let (hook_control, seen, weak, wave, slow_time, slow_device) = (control.clone(), at_configure.clone(), Arc::downgrade(&core), waves[1].clone(), time.clone(), device.clone());
    let fired = AtomicBool::new(false);
    probe(&core, "tx_configure_before_mark", move |_| {
        let Some(core) = weak.upgrade() else { return };
        if !fired.swap(true, Ordering::SeqCst) {
            device_at(&core, &slow_time, &slow_device, 21 * MS);
            let started = lock(&core.streams).stream(key0()).book.playing.map(|playing| playing.started);
            book_with(hook_control.lock().unwrap().as_mut().expect("the rig's control"), &core, 3, 0, clock, 30_000, &wave, true, LatePolicy::SendAsapAndFlag, switch_mark());
            *seen.lock().unwrap() = Some(started);
        }
    });
    drain_tx(&mut tx);
    assert!(tx.step(), "R's configure turn");
    let mut log = Vec::new();
    let not_started = json!("UR-38: the repeat the switch replaces has not started on the device");
    assert_eq!(*at_configure.lock().unwrap(), Some(Some(false)), "R plays, not started, during its configure");
    assert_eq!(refusals_in(&read_events(&events, &mut log)), [(json!(3), not_started.clone())], "the switch received during the configure");
    book_with(control.lock().unwrap().as_mut().expect("the rig's control"), &core, 4, 0, clock, 30_000, &waves[1], true, LatePolicy::SendAsapAndFlag, switch_mark());
    assert_eq!(refusals_in(&read_events(&events, &mut log)), [(json!(4), not_started)], "the switch received after the configure, before R's first send");
    assert!(lock(&core.rec).switches.is_empty());
    turn(&mut tx);
    assert!(sent(&device), "R's first send: {:?}", device.calls());
    // R was moved to its first send's start (SendAsap); a switch needs that start reached as well.
    turns(&core, &time, &device, &mut tx, 22, 24, 1);
    book_with(control.lock().unwrap().as_mut().expect("the rig's control"), &core, 5, 0, clock, 30_000, &waves[1], true, LatePolicy::SendAsapAndFlag, switch_mark());
    let read = read_events(&events, &mut log);
    assert!(refusals_in(&read).is_empty(), "the switch after R's first send is accepted: {read:?} {:?}", device.calls());
    assert_eq!(lock(&core.rec).switches.iter().map(|row| row["outcome"].clone()).collect::<Vec<_>>(), [json!("pending")]);
    control.lock().unwrap().take();
    check("C5", snapshot(&core, &device, &events, log), C5);
}

#[test]
fn e2b_c6_provider_stop_during_a_refused_configure_reports_the_refusal_once() {
    let (core, device, time, mut tx, frozen, events, to_tx) = owner_with(1, vec![FakeFault::TxApplyRefused(0, F2)]);
    let clock = tx.clock.expect("a clock");
    let (weak, fired) = (Arc::downgrade(&core), AtomicBool::new(false));
    probe(&core, "apply", move |_| {
        let Some(stopper) = weak.upgrade() else { return };
        if !fired.swap(true, Ordering::SeqCst) {
            let at = stopper.now();
            let mut streams = lock(&stopper.streams);
            streams.stop = Some((at, StopMode::Orderly));
            streams.end(&stopper, at, false, false);
            stopper.release_streams(streams);
            to_tx.send(TxCmd::Shutdown).unwrap();
        }
    });
    give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 1));
    device_at(&core, &time, &device, 15 * MS);
    assert!(tx.step(), "the configure turn");
    assert!(matches!(tx.cmds.try_recv(), Ok(TxCmd::Shutdown)), "Provider::stop came during the configure");
    assert_eq!(lock(&core.streams).holder(key0()), Holder::Free, "the configure's claim is back");
    shutdown_owner(&core, &mut tx, frozen);
    let mut log = Vec::new();
    let read = read_events(&events, &mut log);
    assert_eq!(refusals_in(&read).iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(), [json!(1)], "X refused once: {read:?}");
    assert!(notes(&core).is_empty(), "and not also cancelled: {:?}", lock(&core.rec).rejected);
    assert!(!sent(&device), "{:?}", device.calls());
    check("C6", snapshot(&core, &device, &events, log), C6);
}

#[test]
fn e2b_c8_a_receipt_between_a_refused_configure_s_mark_and_its_drop_is_refused() {
    let mut rig = control_rig(vec![vec![[0.25, 0.0]; 4], vec![[0.5, 0.0]; 1]], vec![FakeFault::TxApplyRefused(0, F2)]);
    book_frequency(&mut rig.control, &rig.core, 1, F2);
    book_burst_on(&mut rig.control, &rig.core, 2, 0, rig.clock, 20_000, &rig.waves[0], false, LatePolicy::SendAsapAndFlag);
    let SwitchRig { core, device, time, events, control, mut tx, clock, waves, _frozen } = rig;
    // The test thread takes the control only outside the owner's turns: a hook left armed fails the count below instead of waiting on it.
    let control: Shared = Arc::new(Mutex::new(Some(control)));
    let hooks = Arc::new(AtomicUsize::new(0));
    let in_gap = Arc::new(Mutex::new(None));
    let (hook_control, hook_count, seen, weak, wave) = (control.clone(), hooks.clone(), in_gap.clone(), Arc::downgrade(&core), waves[1].clone());
    let fired = AtomicBool::new(false);
    probe(&core, "tx_configure_before_mark", move |_| {
        let Some(core) = weak.upgrade() else { return };
        if fired.swap(true, Ordering::SeqCst) {
            return;
        }
        let (control, count, seen, weak, wave) = (hook_control.clone(), hook_count.clone(), seen.clone(), weak.clone(), wave.clone());
        // The first hook fires before the mark's section; the one it arms, before the owner's next section, the refused burst's drop.
        on_next_lock(&core.streams, move || {
            count.fetch_add(1, Ordering::SeqCst);
            let Some(core) = weak.upgrade() else { return };
            on_next_lock(&core.streams, move || {
                count.fetch_add(1, Ordering::SeqCst);
                let Some(core) = weak.upgrade() else { return };
                let state = {
                    let streams = lock(&core.streams);
                    (streams.stream(key0()).configured, streams.holder(key0()))
                };
                *seen.lock().unwrap() = Some(state);
                book_burst_on(control.lock().unwrap().as_mut().expect("the rig's control"), &core, 5, 0, clock, 20_002, &wave, false, LatePolicy::DropAndFlag);
            });
        });
    });
    turns(&core, &time, &device, &mut tx, 1, 15, 1);
    assert_eq!((hooks.load(Ordering::SeqCst), next_lock_armed()), (2, false), "both hooks ran, and none is left");
    assert_eq!(*in_gap.lock().unwrap(), Some((None, Holder::Free)), "in the gap the mark is cleared and the claim is back");
    let mut log = Vec::new();
    let read = read_events(&events, &mut log);
    let refusals = refusals_in(&read);
    assert_eq!(refusals.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(), [json!(5), json!(2)], "Z refused in the gap, then X: {read:?}");
    assert_eq!(refusals[0].1, "UR-21: the burst overlaps an accepted booking");
    book_burst_on(control.lock().unwrap().as_mut().expect("the rig's control"), &core, 6, 0, clock, 20_002, &waves[1], false, LatePolicy::DropAndFlag);
    assert!(refusals_in(&read_events(&events, &mut log)).is_empty(), "after the turn the refused burst's span is free");
    control.lock().unwrap().take();
    check("C8", snapshot(&core, &device, &events, log), C8);
}

#[test]
fn e2b_c9b_a_loss_just_before_the_configure_s_mark_section_leaves_the_mark_cleared() {
    // The hook's loss lands between the configure's device call and its mark's section.
    let (core, device, time, mut tx, frozen, events, _to_tx) = owner(1);
    let clock = tx.clock.expect("a clock");
    let ran = arm_loss_at(&core, "tx_configure_before_mark");
    give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 1));
    turns(&core, &time, &device, &mut tx, 1, 15, 1);
    assert_eq!((ran.load(Ordering::SeqCst), next_lock_armed()), (1, false), "(b): the hook ran");
    assert_eq!(mark(&core), None, "(b): the loss's clear stands");
    assert!(!sent(&device), "{:?}", device.calls());
    shutdown_owner(&core, &mut tx, frozen);
    assert_eq!(notes(&core), ["UR-26: cancelled by Provider::stop"], "(b)");
    check("C9(b)", snapshot(&core, &device, &events, Vec::new()), C9_B);
}

/// Arms, at the probe `name` on the owner's thread, a hook that loses the device just before the owner's next streams lock; the count
/// of its runs.
fn arm_loss_at(core: &Arc<Core>, name: &'static str) -> Arc<AtomicUsize> {
    let ran = Arc::new(AtomicUsize::new(0));
    let (count, weak, fired) = (ran.clone(), Arc::downgrade(core), AtomicBool::new(false));
    probe(core, name, move |_| {
        let Some(core) = weak.upgrade() else { return };
        if fired.swap(true, Ordering::SeqCst) {
            return;
        }
        let (count, weak) = (count.clone(), weak.clone());
        on_next_lock(&core.streams, move || {
            count.fetch_add(1, Ordering::SeqCst);
            if let Some(core) = weak.upgrade() {
                core.device_lost("test: lost before the result's section");
            }
        });
    });
    ran
}

// ------------------------------------------------------------------------------------------------------------ the reopen turn

/// The owner rig with a pending recovery and B (one sample, 60 ms ahead) held, as `ur_22_stop_and_loss_during_owner_recovery_release_the_claim`
/// holds it; B's sample.
fn recovering(faults: Vec<FakeFault>) -> (OwnerRig, Clock, i64) {
    let rig = owner_with(1, faults);
    let clock = rig.3.clock.expect("a clock");
    let target = clock.at_or_after(rig.0.now() + rig.0.ticks(60_000_000));
    hold(&rig.3, held(&clock, target, 1, LatePolicy::DropAndFlag));
    lock(&rig.0.streams).record_configured(key0(), Some(1));
    rig.3.chain().mark_reopen_needed();
    (rig, clock, target)
}

#[test]
fn e2b_c7_a_stop_during_a_reopen_is_read_after_its_result() {
    for fails in [false, true] {
        let what = if fails { "C7 failed" } else { "C7 reopened" };
        let faults = if fails { vec![FakeFault::TxOpenFails(1)] } else { Vec::new() };
        let ((core, device, _time, tx, frozen, events, _to_tx), clock, b) = recovering(faults);
        let (weak, fired) = (Arc::downgrade(&core), AtomicBool::new(false));
        probe(&core, "tx_reopen_claimed", move |_| {
            if let (false, Some(core)) = (fired.swap(true, Ordering::SeqCst), weak.upgrade()) {
                full_stop(&core);
            }
        });
        let (tx, _frozen) = released(frozen, &device.host(), move || {
            let mut tx = tx;
            assert!(tx.pass(), "the reopen turn");
            tx.pass();
            tx
        });
        let b_at = format!("at={} sob=true", clock.instant(b));
        assert!(!device.calls().iter().any(|call| call.contains(&b_at)), "{what}: B is not sent: {:?}", device.calls());
        let mut log = Vec::new();
        let read = read_events(&events, &mut log);
        let cancelled = json!("UR-22: cancelled because the TX streamer could not be reopened");
        if fails {
            assert!(tx.clock.is_none(), "{what}: the clock ends");
            assert!(refusals_in(&read).iter().any(|(_, reason)| *reason == cancelled), "{what}: B refused, ClockEnded: {read:?}");
            assert!(notes(&core).is_empty(), "{what}: and no Stop note: {:?}", lock(&core.rec).rejected);
        } else {
            assert!(device.calls().iter().any(|call| call == T3), "{what}: T3 was sent: {:?}", device.calls());
            assert!(refusals_in(&read).is_empty(), "{what}: {read:?}");
            assert_eq!(notes(&core), ["UR-26: cancelled by Stop"], "{what}: the halt read after the result cancels B");
        }
        assert!(held_keys(&tx).is_empty(), "{what}");
        check(what, snapshot(&core, &device, &events, log), if fails { C7_FAILED } else { C7_REOPENED });
    }
}

#[test]
fn e2b_c9a_a_loss_just_before_the_reopen_s_result_section_leaves_the_mark_cleared() {
    let ((core, device, _time, tx, frozen, events, _to_tx), _clock, b) = recovering(Vec::new());
    let ran = arm_loss_at(&core, "tx_reopen_done");
    let ((tx, armed), _frozen) = released(frozen, &device.host(), move || {
        let mut tx = tx;
        tx.pass();
        (tx, next_lock_armed())
    });
    assert_eq!((ran.load(Ordering::SeqCst), armed), (1, false), "the hook ran");
    assert_eq!(device.calls().iter().filter(|call| call.starts_with("tx_open")).count(), 2, "the reopen ran: {:?}", device.calls());
    assert!(core.is_lost());
    assert_eq!(mark(&core), None, "the loss's clear stands");
    assert_eq!(held_keys(&tx), [b]);
    check("C9(a)", snapshot(&core, &device, &events, Vec::new()), C9_A);
}

#[test]
fn e2b_c9c_a_loss_just_before_a_failed_reopen_s_result_section_ends_no_clock() {
    let ((core, device, _time, tx, frozen, events, _to_tx), _clock, b) = recovering(vec![FakeFault::TxOpenFails(1)]);
    let ran = arm_loss_at(&core, "tx_reopen_done");
    let ((tx, armed), _frozen) = released(frozen, &device.host(), move || {
        let mut tx = tx;
        tx.pass();
        (tx, next_lock_armed())
    });
    assert_eq!((ran.load(Ordering::SeqCst), armed), (1, false), "the hook ran");
    assert!(core.is_lost());
    assert!(tx.clock.is_some(), "a loss ends no clock at the reopen");
    assert_eq!(held_keys(&tx), [b], "B is not refused");
    assert!(lock(&core.rec).rejected.iter().all(|row| !row["reason"].as_str().is_some_and(|reason| reason.contains("could not be reopened"))), "{:?}", lock(&core.rec).rejected);
    let mut log = Vec::new();
    let read = read_events(&events, &mut log);
    assert!(refusals_in(&read).is_empty(), "{read:?}");
    assert_eq!(read.iter().filter(|event| event["kind"] == EventKind::DEVICE_LOST).count(), 1, "{read:?}");
    check("C9(c)", snapshot(&core, &device, &events, log), C9_C);
}

// ------------------------------------------------------------------------------------------------------------ R2: no line lock across these calls

#[test]
fn e2b_r2a_the_line_lock_is_free_inside_the_configure_s_apply() {
    let spy = Arc::new(LockSpy::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
    let (core, time, _) = crate::provider::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
    spy.watch(&core);
    spy.tx_open(0, &[0]).unwrap();
    let clock = core.register(Dir::Tx, 200, 0).unwrap();
    lock(&core.streams).record_configured(key0(), Some(0));
    let (_to_tx, cmds) = std::sync::mpsc::channel();
    let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
    give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 1));
    time.advance_to(core.at(15 * MS)).unwrap();
    let before = spy.seen("apply");
    assert!(tx.step(), "the configure turn");
    assert_eq!(spy.seen("apply"), before + 1, "the spy saw the configure's apply");
    assert_eq!(mark(&core), Some(1));
}

#[test]
fn e2b_r2b_the_line_lock_is_free_inside_the_reopen() {
    let spy = Arc::new(LockSpy::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
    let (core, _time, _) = crate::provider::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
    spy.watch(&core);
    spy.tx_open(0, &[0]).unwrap();
    let clock = core.register(Dir::Tx, 200, 0).unwrap();
    let (_to_tx, cmds) = std::sync::mpsc::channel();
    let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
    tx.chain().mark_reopen_needed();
    let host = core.device.host();
    let (worked, _frozen) = released(Frozen::on(host.clone()), &host, move || tx.pass());
    assert!(worked, "the reopen turn");
    assert_eq!(spy.seen("tx_open"), 2, "the test's open and the reopen's");
    assert!(spy.seen("tx_send") >= 1, "T3");
}

/// A transmit owner whose device holds generation 0 (`mark` `Some(0)`), with a booking of generation 1 open at its first sample.
fn open_with_unconfigured_generation() -> (Tx, Arc<Core>, Clock) {
    let (core, device, _time, _) = crate::provider::test_support::rig();
    device.tx_open(0, &[0]).unwrap();
    let clock = core.register(Dir::Tx, 200, 0).unwrap();
    lock(&core.streams).record_configured(key0(), Some(0));
    let (_to_tx, cmds) = std::sync::mpsc::channel();
    let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
    tx.poke_open(valued(&clock, 20_000, 4, 1, 1, F2, 1), 20_000, true);
    (tx, core, clock)
}

#[test]
#[cfg(debug_assertions)] // the check is a `debug_assert!`
#[should_panic(expected = "a device burst starts only with the values of its generation configured")]
fn e2b_r1_a_first_send_of_a_generation_the_device_does_not_hold_panics_in_the_test_build() {
    let (mut tx, _core, _clock) = open_with_unconfigured_generation();
    tx.step();
}

#[test]
fn e2b_r1b_a_first_send_on_a_lost_device_does_not_need_the_generation() {
    let (mut tx, core, clock) = open_with_unconfigured_generation();
    core.device_lost("test");
    assert!(!tx.send(clock, None, 0), "nothing is handed over: the window is closed");
}

// ------------------------------------------------------------------------------------------------------------ today's snapshots

const C0: &str = r#"{
"applied": [
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.tx.s0.sample_rate_hz","read_back":1000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":2000000000.0,"difference":0.0,"key":"radio.tx.c0.frequency_hz","read_back":2000000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":null,"difference":null,"key":"radio.tx.c0.gain_db","read_back":0.0}
],
"bursts": [
  {"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":4,"target":{"domain":{"local":3,"node":0},"ticks":20000},"wraps":0}
],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=- antenna=-",
  "tx_send 0 n=4 at=4000000 sob=true eob=true"
],
"device_async": [],
"events": [],
"rejected": [],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":1,"tx_errors":0,"tx_samples":4},
"switches": [],
"timing": []
}"#;
const C1: &str = r#"{
"applied": [
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.tx.s0.sample_rate_hz","read_back":1000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":2000000000.0,"difference":0.0,"key":"radio.tx.c0.frequency_hz","read_back":2000000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":null,"difference":null,"key":"radio.tx.c0.gain_db","read_back":0.0}
],
"bursts": [
  {"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":4,"target":{"domain":{"local":3,"node":0},"ticks":20000},"wraps":0}
],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=- antenna=-",
  "tx_send 0 n=4 at=4000000 sob=true eob=true"
],
"device_async": [],
"events": [],
"rejected": [],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":1,"tx_errors":0,"tx_samples":4},
"switches": [],
"timing": []
}"#;
const C2_CONFIGURED: &str = r#"{
"applied": [
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.tx.s0.sample_rate_hz","read_back":1000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":2000000000.0,"difference":0.0,"key":"radio.tx.c0.frequency_hz","read_back":2000000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":0.0,"difference":0.0,"key":"radio.tx.c0.gain_db","read_back":0.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":"TX/RX","key":"radio.tx.c0.antenna","read_back":"TX/RX"}
],
"bursts": [
  {"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":200,"target":{"domain":{"local":3,"node":0},"ticks":25005},"wraps":1}
],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=0 antenna=TX/RX",
  "tx_send 0 n=199 at=5001000 sob=true eob=false",
  "tx_send 0 n=2 at=5040800 sob=false eob=true"
],
"device_async": [],
"events": [
  {"kind":"radio.COMMAND_REJECTED","payload":{"action":5,"kind":"tx_burst","reason":"UR-21: the burst overlaps an accepted booking"}}
],
"rejected": [
  {"kind":"tx_burst","reason":"UR-26: cancelled by Stop"},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"kind":"tx_burst","reason":"UR-21: the burst overlaps an accepted booking"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":1,"tx_errors":0,"tx_samples":200},
"switches": [],
"timing": []
}"#;
const C2_REFUSED: &str = r#"{
"applied": [
  {"at":{"domain":{"local":2,"node":0},"ticks":3200000},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.tx.s0.sample_rate_hz","read_back":1000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3200000},"channel":0,"claimed":2000000000.0,"difference":0.0,"key":"radio.tx.c0.frequency_hz","read_back":2000000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3200000},"channel":0,"claimed":0.0,"difference":0.0,"key":"radio.tx.c0.gain_db","read_back":0.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3200000},"channel":0,"claimed":"TX/RX","key":"radio.tx.c0.antenna","read_back":"TX/RX"}
],
"bursts": [
  {"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":200,"target":{"domain":{"local":3,"node":0},"ticks":25005},"wraps":1}
],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=0 antenna=TX/RX",
  "tx_send 0 n=199 at=5001000 sob=true eob=false",
  "tx_send 0 n=2 at=5040800 sob=false eob=true"
],
"device_async": [],
"events": [
  {"kind":"radio.COMMAND_REJECTED","payload":{"action":2,"kind":"tx_burst","reason":"test: scripted refusal of an apply"}},
  {"kind":"radio.COMMAND_REJECTED","payload":{"action":5,"kind":"tx_burst","reason":"UR-21: the burst overlaps an accepted booking"}}
],
"rejected": [
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"kind":"tx_burst","reason":"test: scripted refusal of an apply"},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"kind":"tx_burst","reason":"UR-21: the burst overlaps an accepted booking"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":1,"tx_errors":0,"tx_samples":200},
"switches": [],
"timing": []
}"#;
const C3: &str = r#"{
"applied": [
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.tx.s0.sample_rate_hz","read_back":1000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":2000000000.0,"difference":0.0,"key":"radio.tx.c0.frequency_hz","read_back":2000000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":0.0,"difference":0.0,"key":"radio.tx.c0.gain_db","read_back":0.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":"TX/RX","key":"radio.tx.c0.antenna","read_back":"TX/RX"}
],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=0 antenna=TX/RX"
],
"device_async": [],
"events": [],
"rejected": [
  {"kind":"tx_burst","reason":"UR-26: cancelled by Stop"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": [
  {"at":3000000,"bound":5001000,"stream":0,"what":"tx_stop_applied"}
]
}"#;
const C4_I: &str = r#"{
"applied": [
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.tx.s0.sample_rate_hz","read_back":1000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":2000000000.0,"difference":0.0,"key":"radio.tx.c0.frequency_hz","read_back":2000000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":null,"difference":null,"key":"radio.tx.c0.gain_db","read_back":0.0}
],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=- antenna=-",
  "mark_lost"
],
"device_async": [],
"events": [
  {"kind":"ezsdr.DEVICE_LOST","payload":{"message":"test: lost before the configure's mark"}}
],
"rejected": [
  {"kind":"tx_burst","reason":"UR-26: cancelled by Provider::stop"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": [
  {"at":2200000,"message":"test: lost before the configure's mark","what":"device_lost"}
]
}"#;
const C4_II: &str = r#"{
"applied": [
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.tx.s0.sample_rate_hz","read_back":1000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":2000000000.0,"difference":0.0,"key":"radio.tx.c0.frequency_hz","read_back":2000000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":null,"difference":null,"key":"radio.tx.c0.gain_db","read_back":0.0}
],
"bursts": [],
"calls": [
  "tx_open 0 [0, 1]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 1 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=- antenna=-",
  "mark_lost"
],
"device_async": [],
"events": [
  {"kind":"ezsdr.DEVICE_LOST","payload":{"message":"test: lost between the configure's channels"}}
],
"rejected": [],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": [
  {"at":2200000,"message":"test: lost between the configure's channels","what":"device_lost"}
]
}"#;
const C4_III: &str = r#"{
"applied": [
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.tx.s0.sample_rate_hz","read_back":1000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":2000000000.0,"difference":0.0,"key":"radio.tx.c0.frequency_hz","read_back":2000000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"channel":0,"claimed":null,"difference":null,"key":"radio.tx.c0.gain_db","read_back":0.0}
],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=- antenna=-",
  "mark_lost"
],
"device_async": [],
"events": [
  {"kind":"ezsdr.DEVICE_LOST","payload":{"message":"test: lost after the configure's result"}}
],
"rejected": [
  {"kind":"tx_burst","reason":"UR-26: cancelled by Provider::stop"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": [
  {"at":3000000,"message":"test: lost after the configure's result","what":"device_lost"}
]
}"#;
const C5: &str = r#"{
"applied": [
  {"at":{"domain":{"local":2,"node":0},"ticks":2400000},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.tx.s0.sample_rate_hz","read_back":1000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2400000},"channel":0,"claimed":2000000000.0,"difference":0.0,"key":"radio.tx.c0.frequency_hz","read_back":2000000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2400000},"channel":0,"claimed":0.0,"difference":0.0,"key":"radio.tx.c0.gain_db","read_back":0.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2400000},"channel":0,"claimed":"TX/RX","key":"radio.tx.c0.antenna","read_back":"TX/RX"}
],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=0 antenna=TX/RX",
  "tx_send 0 n=2500 at=4600000 sob=true eob=false",
  "tx_send 0 n=2500 at=5100000 sob=false eob=false",
  "tx_send 0 n=2500 at=5600000 sob=false eob=false",
  "tx_send 0 n=2500 at=6100000 sob=false eob=false"
],
"device_async": [],
"events": [
  {"kind":"radio.COMMAND_REJECTED","payload":{"action":3,"kind":"tx_burst","reason":"UR-38: the repeat the switch replaces has not started on the device"}},
  {"kind":"radio.COMMAND_REJECTED","payload":{"action":4,"kind":"tx_burst","reason":"UR-38: the repeat the switch replaces has not started on the device"}},
  {"kind":"radio.TIME_ERROR","payload":{"cause":"late","late_by_ns":3000000,"outcome":"send_asap","target":{"domain":{"local":3,"node":0},"ticks":20000}}}
],
"rejected": [
  {"at":{"domain":{"local":2,"node":0},"ticks":4200000},"kind":"tx_burst","reason":"UR-38: the repeat the switch replaces has not started on the device"},
  {"at":{"domain":{"local":2,"node":0},"ticks":4200000},"kind":"tx_burst","reason":"UR-38: the repeat the switch replaces has not started on the device"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":1,"tx_errors":0,"tx_samples":10000},
"switches": [
  {"action_id":5,"boundary":36000,"new_period":1000,"old_period":1000,"outcome":"pending","predecessor":2,"selected_at":{"domain":{"local":2,"node":0},"ticks":4800000}}
],
"timing": []
}"#;
const C6: &str = r#"{
"applied": [],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=- antenna=-"
],
"device_async": [],
"events": [
  {"kind":"radio.COMMAND_REJECTED","payload":{"action":1,"kind":"tx_burst","reason":"fake: scripted refusal of a transmit apply"}}
],
"rejected": [
  {"at":{"domain":{"local":2,"node":0},"ticks":3000000},"kind":"tx_burst","reason":"fake: scripted refusal of a transmit apply"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": []
}"#;
const C7_REOPENED: &str = r#"{
"applied": [],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "tx_close 0",
  "tx_open 0 [0]",
  "tx_send 0 n=4 at=- sob=true eob=true",
  "rates tx 0"
],
"device_async": [
  {"channel":0,"code":"BurstAck","stream":0,"tick":801,"tx_flush":true}
],
"events": [],
"rejected": [
  {"kind":"tx_burst","reason":"UR-26: cancelled by Stop"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": []
}"#;
const C7_FAILED: &str = r#"{
"applied": [],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "tx_close 0",
  "tx_open 0 [0]"
],
"device_async": [],
"events": [
  {"kind":"radio.COMMAND_REJECTED","payload":{"kind":"tx_burst","reason":"UR-22: cancelled because the TX streamer could not be reopened"}},
  {"kind":"radio.COMMAND_REJECTED","payload":{"kind":"tx_reopen","reason":"fake: scripted refusal of transmit open"}}
],
"rejected": [
  {"at":{"domain":{"local":2,"node":0},"ticks":0},"kind":"tx_burst","reason":"UR-22: cancelled because the TX streamer could not be reopened"},
  {"at":{"domain":{"local":2,"node":0},"ticks":0},"kind":"tx_reopen","reason":"fake: scripted refusal of transmit open"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": []
}"#;
const C8: &str = r#"{
"applied": [],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=0 antenna=TX/RX"
],
"device_async": [],
"events": [
  {"kind":"radio.COMMAND_REJECTED","payload":{"action":5,"kind":"tx_burst","reason":"UR-21: the burst overlaps an accepted booking"}},
  {"kind":"radio.COMMAND_REJECTED","payload":{"action":2,"kind":"tx_burst","reason":"fake: scripted refusal of a transmit apply"}}
],
"rejected": [
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"kind":"tx_burst","reason":"UR-21: the burst overlaps an accepted booking"},
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"kind":"tx_burst","reason":"fake: scripted refusal of a transmit apply"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": []
}"#;
const C9_A: &str = r#"{
"applied": [],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "tx_close 0",
  "tx_open 0 [0]",
  "tx_send 0 n=4 at=- sob=true eob=true",
  "rates tx 0",
  "mark_lost"
],
"device_async": [
  {"channel":0,"code":"BurstAck","stream":0,"tick":801,"tx_flush":true}
],
"events": [
  {"kind":"ezsdr.DEVICE_LOST","payload":{"message":"test: lost before the result's section"}}
],
"rejected": [],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": [
  {"at":0,"message":"test: lost before the result's section","what":"device_lost"}
]
}"#;
const C9_B: &str = r#"{
"applied": [
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.tx.s0.sample_rate_hz","read_back":1000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":2000000000.0,"difference":0.0,"key":"radio.tx.c0.frequency_hz","read_back":2000000000.0},
  {"at":{"domain":{"local":2,"node":0},"ticks":2200000},"channel":0,"claimed":null,"difference":null,"key":"radio.tx.c0.gain_db","read_back":0.0}
],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "apply tx 0 rate=- freq=2000000000 gain=- antenna=-",
  "mark_lost"
],
"device_async": [],
"events": [
  {"kind":"ezsdr.DEVICE_LOST","payload":{"message":"test: lost before the result's section"}}
],
"rejected": [
  {"kind":"tx_burst","reason":"UR-26: cancelled by Provider::stop"}
],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": [
  {"at":2200000,"message":"test: lost before the result's section","what":"device_lost"}
]
}"#;
const C9_C: &str = r#"{
"applied": [],
"bursts": [],
"calls": [
  "tx_open 0 [0]",
  "apply tx 0 rate=1000000 freq=1000000000 gain=- antenna=-",
  "tx_close 0",
  "tx_open 0 [0]",
  "mark_lost"
],
"device_async": [],
"events": [
  {"kind":"ezsdr.DEVICE_LOST","payload":{"message":"test: lost before the result's section"}}
],
"rejected": [],
"stats": {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":0,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":0,"tx_bursts":0,"tx_errors":0,"tx_samples":0},
"switches": [],
"timing": [
  {"at":0,"message":"test: lost before the result's section","what":"device_lost"}
]
}"#;
