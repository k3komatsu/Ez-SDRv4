//! Tests of the thread runtime on a device (stage E2a-2; test plan §7, §13–§15, §17): the free runs on `FakeDevice` (S6, S7),
//! the forced cleanup path (S6b), the native lock's contention (S8) and the conformance of `ModelDevice` to `FakeDevice` (S9).

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::cases;
use super::cell::{Phase, Terminal};
use super::core::{Disposition, Input};
use super::dst::Model;
use super::fixtures::{cell, held, seg, value};
use super::thread_tests::JOIN;
use super::threads::Runtime;
use super::world::{Fact, World};
use crate::device::{Device, Dir, FakeConfig, FakeDevice, Settings};
use crate::host::ClockMode;

// ------------------------------------------------------------------------------------ free runs on FakeDevice

/// A fake at ratio one (1 tick = 1 sample = 1 ms), packets of four samples, two channels, on a manual clock: the clock stands while
/// the test's participant thread works and runs only while it sleeps, so every instant below is a tick the test chose (#76: no
/// verdict depends on how fast the runtime's threads are scheduled).
fn fake() -> Arc<FakeDevice> {
    Arc::new(FakeDevice::new(FakeConfig { master_clock_rate: 1000, rx_packet: Some(4), command_queue: 16, clock: ClockMode::Manual, ..FakeConfig::default() }))
}

/// The cells of `streams` streams, each streaming under a segment that begins at `origin`, as the open fake is.
fn streaming_cells(streams: usize, origin: i64) -> World {
    let params = super::thread_tests::params();
    let mut w = World::new((0..streams).map(|k| cell(Phase::Streaming(seg(origin)), k, None, 0)).collect(), Vec::new(), params);
    for k in 0..streams {
        cases::running(&mut w, k, vec![k]);
    }
    w
}

/// A free run of `streams` streams on a fake, with the test body on a participant of its clock: time zero is set, every stream is
/// opened at 1 kS/s and started 20 ticks ahead, the runtime started, and `body` run. Returns when the body does; its panic is rethrown.
/// A run that does not finish within 60 s of wall time fails this test alone (a bounded result channel, as `conformance::on_fake`):
/// participants that wait on each other would hang in a clock sleep that no poll bound can interrupt, and the diagnosis names their
/// states. Terminating the process belongs to the external runner.
fn run_fake(streams: usize, body: impl FnOnce(&Lock) + Send + 'static) {
    let fake = fake();
    let device = fake.clone();
    let slot: Arc<std::sync::Mutex<Option<Arc<super::shared::Shared>>>> = Arc::new(std::sync::Mutex::new(None));
    let published = slot.clone();
    let (done, result) = std::sync::mpsc::channel();
    let handle = fake
        .host()
        .spawn("test", move || {
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                device.set_time_zero(false).unwrap();
                let origin = device.time_now().unwrap() + 20;
                for k in 0..streams {
                    device.apply(Dir::Rx, k, &Settings { rate: Some(1e3), ..Settings::default() }, None).unwrap();
                    device.rx_open(k, &[k]).unwrap();
                    device.rx_start(k, origin).unwrap();
                }
                let rt = Runtime::start_fake(&streaming_cells(streams, origin), device.clone());
                *published.lock().unwrap() = Some(rt.sh.clone());
                body(&Lock { fake: device, rt, streams, origin });
            }));
            let _ = done.send(run);
        })
        .unwrap();
    match result.recv_timeout(Duration::from_secs(60)) {
        Ok(Ok(())) => handle.join().unwrap(),
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(_) => {
            use super::force::Actor::*;
            let states = slot.lock().unwrap().as_ref().map(|sh| ([Brain, Settings, Rx(0), Rx(1), Clock, Timer].map(|a| (a, sh.sched.state_of(a))), sh.sink().exited.clone()));
            panic!("a fake-clock run did not finish within 60 s: its participants wait on each other? clock waits {}, states and exits {states:?}, calls {:?}", fake.host().blocked(), fake.calls());
        }
    }
}

/// The test's view of a run in lock step: time moves one tick at a time, and after each tick the receive executors have taken every
/// packet that is complete by it.
struct Lock {
    fake: Arc<FakeDevice>,
    rt: Runtime,
    streams: usize,
    origin: i64,
}

impl Lock {
    fn now(&self) -> i64 {
        self.fake.time_now().unwrap()
    }

    /// Time moves one tick at a time, by this participant's own sleep. The receive executors are participants of the fake's clock, as
    /// production's are (rules M1-M7): the clock cannot pass a runnable executor, and a wake whose deadline falls inside the tick runs
    /// to its next wait before this sleep returns. So on return every packet complete by the previous tick has been taken (an
    /// executor's relative wait ends at its tick plus a fraction of one tick): the poll only waits for that state, it never moves time.
    fn advance_to(&self, t: i64) {
        while self.now() < t {
            self.fake.host().sleep(Duration::from_millis(1));
            let now = self.now();
            let covered = if now >= self.origin + 5 { self.origin + (now - 1 - self.origin) / 4 * 4 } else { 0 };
            poll("the executors have taken every packet complete by the previous tick", || (0..self.streams).all(|k| self.rt.sh.with_cell(k, |cell| cell.progress >= covered)));
        }
    }

    /// Time moves, a tick at a time, until `done` holds (bounded by ticks, not by wall time).
    fn advance_until(&self, what: &str, done: impl Fn() -> bool) {
        for _ in 0..400 {
            if done() {
                return;
            }
            self.advance_to(self.now() + 1);
        }
        assert!(done(), "gave up waiting for: {what}");
    }

    fn published(&self) -> bool {
        let sink = self.rt.sh.sink();
        (0..self.streams).all(|k| sink.published.iter().any(|p| p.stream == k && !p.ticks.is_empty()))
    }

    fn consumed(&self, id: u64) -> bool {
        self.rt.sh.brain().dispositions.iter().any(|d| d.0 == id && d.1 == Disposition::Consumed)
    }

    /// Releases value `id` on stream 0 for instant `e` and waits (the clock standing) until `S` has made its call and recorded the acceptance.
    fn release(&self, id: u64, e: i64) {
        self.rt.sh.send(Input::Release(held(0, value(0, e, id), 1)));
        poll("S accepts the value", || self.rt.sh.sink().acceptances.iter().any(|a| a.0 == id));
    }

    /// Shutdown; time moves until every executor has exited (a receive in progress needs the clock to return), then the threads join.
    fn shut_down(&self) {
        self.rt.sh.send(Input::Shutdown);
        let all_exited = || {
            let sink = self.rt.sh.sink();
            sink.exited.contains(&super::force::Actor::Settings) && (0..self.streams).all(|k| sink.exited.contains(&super::force::Actor::Rx(k)))
        };
        // A receive in progress needs the clock to return; a sleep of this participant returns once every runnable executor has run.
        for _ in 0..64 {
            if all_exited() {
                break;
            }
            self.fake.host().sleep(Duration::from_millis(1));
        }
        assert!(all_exited(), "every executor exited");
        let panics = self.rt.stop(Duration::from_secs(1)).expect("every thread joins");
        assert!(panics.is_empty(), "{panics:?}");
    }
}

/// Polls `f` for at most `JOIN` of wall time: the bound only fails a hung run.
fn poll(what: &str, f: impl Fn() -> bool) {
    let started = Instant::now();
    while !f() {
        assert!(started.elapsed() < JOIN, "gave up waiting for: {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The calls of the fake that concern stream `k` (its lifecycle lines, by name and number).
fn stream_calls(fake: &FakeDevice, k: usize) -> Vec<String> {
    fake.calls().into_iter().filter(|c| c.starts_with("rx_") && c.split(' ').nth(1) == Some(&k.to_string())).collect()
}

/// S6, the happy path on the fake's manual clock: both streams start at a future origin and produce samples before the value is
/// released; one timed value is applied and consumed; Shutdown stops each stream once and every executor exits. Each value and
/// each command has one disposition and one terminal (P2: `observed` fails on a duplicate).
#[test]
fn s6_fakedevice_two_streams_shutdown_order() {
    run_fake(2, |run| {
        let (fake, rt, origin) = (&run.fake, &run.rt, run.origin);
        run.advance_to(origin + 8);
        assert!(run.published(), "both streams produced samples before the value is released");
        let e = run.now() + 40;
        run.release(1, e);
        run.advance_until("the value is consumed", || run.consumed(1));
        run.shut_down();
        let observed = rt.observed().expect("one disposition and one terminal per id");
        assert_eq!(observed.dispositions, vec![(1, Disposition::Consumed)]);
        assert_eq!(observed.terminals, vec![(1, Terminal::Call(super::cell::Outcome::Ok))]);
        for k in 0..2 {
            assert!(observed.exited.contains(&super::force::Actor::Rx(k)), "R{k} exited");
        }
        assert!(observed.exited.contains(&super::force::Actor::Settings));
        // Per stream: opened, started at the origin, stopped once, and nothing after the stop (no flush, no restore: nothing is owed).
        let tail = |k: usize| vec![format!("rx_open {k} [{k}]"), format!("rx_start {k} {origin}")];
        assert_eq!(stream_calls(fake, 1), [tail(1), vec!["rx_stop 1 now".to_string()]].concat(), "{:?}", fake.calls());
        assert_eq!(stream_calls(fake, 0), [tail(0), vec!["rx_stop 0 now".to_string()]].concat(), "{:?}", fake.calls());
        let calls = fake.calls();
        let apply = calls.iter().position(|c| c.starts_with("apply rx 0 ") && c.contains(&format!("at={e}"))).expect("the timed apply");
        let start = calls.iter().position(|c| *c == format!("rx_start 0 {origin}")).unwrap();
        let stop = calls.iter().position(|c| c == "rx_stop 0 now").unwrap();
        assert!(start < apply && apply < stop, "the value is applied while the stream runs: {calls:?}");
        assert_eq!(calls.iter().filter(|c| c.starts_with("rx_start_now")).count(), 0, "nothing was left to flush: {calls:?}");
        assert_eq!(calls.iter().filter(|c| c.starts_with("apply rx") && c.ends_with("at=-")).count(), 2, "no restore: only the two rate writes of the setup are untimed: {calls:?}");
    });
}

/// S7, a timed DDC value on the manual clock at ratio one: admission (the call enters before `e`, so its effective instant is
/// `e`), acceptance (the time read after the call returned, before `e`), application (the DDC's tick) and evidence (the sample
/// that consumed it) are observed separately, and the last two are both `e`.
#[test]
fn s7_fakedevice_value_applied_before_instant_consumed_at_it() {
    run_fake(1, |run| {
        let (fake, rt) = (&run.fake, &run.rt);
        run.advance_to(run.origin + 8);
        assert!(run.published());
        let e = run.now() + 40;
        run.release(1, e);
        run.advance_until("the value is consumed", || run.consumed(1));
        run.shut_down();
        let observed = rt.observed().unwrap();
        let calls = fake.calls();
        let apply = calls.iter().find(|c| c.starts_with("apply rx 0 ") && !c.ends_with("at=-")).expect("one timed apply");
        assert!(apply.ends_with(&format!("at={e} effective={e}")), "admitted before the instant: the device's effective instant is e: {apply}");
        let accepted = rt.sh.sink().acceptances.clone();
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].0, 1);
        assert!(accepted[0].1 < e, "accepted before the instant: {accepted:?} e {e}");
        assert_eq!(fake.dsp_log(Dir::Rx, 0).into_iter().filter(|(_, _, timed)| *timed).collect::<Vec<_>>(), vec![(e, 1.0, true)], "the DDC applied it at its instant");
        assert_eq!(observed.applications.get(&0), Some(&vec![(1, e)]));
        assert_eq!(observed.evidence, vec![Fact { leftover: false, value: 1, stream: 0, channel: 0, sample: e }], "consumed by the sample the DDC applied it at");
        assert!(observed.leftovers.is_empty());
    });
}

/// The probe's verdict at the instant an actor is held (no wall-time inference): `held` is a registered participant of the clock and
/// busy, every other participant but the `observers` is idle or waits for a manual deadline the clock has not reached (a participant
/// that wakes for an instant to poll is seen again at the next snapshot), and the clock's reading is returned from the same snapshot.
fn held_alone(host: &crate::host::HostClock, held: &str, observers: &[&str]) -> Duration {
    let (_, now) = host.participants();
    assert!(now.iter().any(|p| p.name == held), "{held} is not a registered participant of the clock: {now:?}");
    let started = Instant::now();
    loop {
        let (at, all) = host.participants();
        let alone = all.iter().filter(|p| p.name != held && !observers.contains(&p.name.as_str())).all(|p| !p.busy && p.deadline.is_none_or(|d| d > at));
        if alone && all.iter().any(|p| p.name == held && p.busy) {
            return at;
        }
        assert!(started.elapsed() < JOIN, "{held} never showed busy alone: {all:?} at {at:?}");
        let seen = host.generation();
        host.wait(None, Some(Instant::now() + Duration::from_millis(1)), seen);
    }
}

/// N1's controlled pause: every thread of the run is a participant of the fake's clock, so time cannot advance past a runnable one.
/// R0 is held inside its publish by a test hook (busy, not waiting on the clock) while a second participant sleeps three ticks across the
/// first packet's completion. Handshake, no timing inference: the body waits (idle on the clock, so it does not hold it) until R0 is
/// confirmed held, then asserts from the clock's probe (`held_alone`) that R0 is a registered, busy participant, every other one is idle
/// or on an undue deadline, and the clock has not moved; then it releases R0 and observes the sleeper's return. An executor that is
/// not a participant is not in the probe, and the first assertion fails.
#[test]
fn s6c_time_cannot_pass_a_runnable_executor() {
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    run_fake(1, |run| {
        let host = run.fake.host();
        let idle = |real: Duration| {
            let seen = host.generation();
            host.wait(None, Some(Instant::now() + real), seen);
        };
        run.advance_to(run.origin + 3);
        let (entered, release, returned) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (e, r) = (entered.clone(), release.clone());
        *run.rt.sh.before_write.lock().unwrap() = Some(Arc::new(move |_| {
            e.store(true, SeqCst);
            while !r.load(SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
        }));
        let (sleeper_host, done) = (host.clone(), returned.clone());
        let sleeper = host
            .spawn("sleeper", move || {
                sleeper_host.sleep(Duration::from_millis(3));
                done.store(true, SeqCst);
            })
            .unwrap();
        let started = Instant::now();
        while !entered.load(SeqCst) {
            assert!(started.elapsed() < JOIN, "R0 never reached its publish");
            idle(Duration::from_millis(1));
        }
        let held_at = host.now();
        let at = held_alone(&host, "Rx(0)", &["test", "sleeper"]);
        assert_eq!(at, held_at, "the clock moved while R0 was runnable");
        assert!(!returned.load(SeqCst), "a sleep returned while R0 was runnable");
        release.store(true, SeqCst);
        // Wait idle on the clock (a busy join would hold it): the sleeper's sleep ends once R0 has run on.
        let started = Instant::now();
        while !returned.load(SeqCst) {
            assert!(started.elapsed() < JOIN, "the sleep did not return after R0 was released");
            idle(Duration::from_millis(1));
        }
        sleeper.join().unwrap();
        run.shut_down();
    });
}

/// Sol's counterexample for the brain: a brain that has taken its next input and is runnable (here blocked on a std mutex a plain thread
/// holds) must stop the clock too, or the receive executors would run on and the test's `advance_until` would pass 400 ticks over a
/// message the brain has not processed. The body confirms the brain is mid-step (`Running` in the scheduler) and asserts from the
/// clock's probe that the brain is a registered, busy participant, the others idle or on undue deadlines, and the clock unmoved;
/// then the lock is released and the clock moves again.
#[test]
fn s6d_time_cannot_pass_a_runnable_brain() {
    use super::force::Actor;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    run_fake(1, |run| {
        let host = run.fake.host();
        let idle = |real: Duration| {
            let seen = host.generation();
            host.wait(None, Some(Instant::now() + real), seen);
        };
        run.advance_to(run.origin + 8);
        let (locked, release) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (sh, l, r) = (run.rt.sh.clone(), locked.clone(), release.clone());
        let holder = std::thread::spawn(move || {
            let _brain = sh.brain();
            l.store(true, SeqCst);
            while !r.load(SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let started = Instant::now();
        while !locked.load(SeqCst) {
            assert!(started.elapsed() < JOIN);
            idle(Duration::from_millis(1));
        }
        run.rt.sh.send(Input::Release(held(0, value(0, run.now() + 40, 1), 1)));
        while run.rt.sh.sched.state_of(Actor::Brain) != "Some(Running)" {
            assert!(started.elapsed() < JOIN, "the brain never took the input");
            idle(Duration::from_millis(1));
        }
        let held_at = host.now();
        let at = held_alone(&host, "Brain", &["test"]);
        assert_eq!(at, held_at, "the clock moved while the brain was runnable");
        release.store(true, SeqCst);
        holder.join().unwrap();
        run.advance_until("the value is consumed", || run.consumed(1));
        run.shut_down();
    });
}

/// Every thread of a fake run is a registered participant of its clock (the probe names them): the brain, S, K, the timer and each R_k.
#[test]
fn every_runtime_thread_of_a_fake_run_is_a_participant() {
    run_fake(2, |run| {
        let (_, all) = run.fake.host().participants();
        let names: std::collections::BTreeSet<String> = all.into_iter().map(|p| p.name).collect();
        for expected in ["Brain", "Settings", "Clock", "Timer", "Rx(0)", "Rx(1)"] {
            assert!(names.contains(expected), "{expected} is not registered: {names:?}");
        }
        run.shut_down();
    });
}

// ------------------------------------------------------------------------------------------------------ S6b

/// S6b, the cleanup path that S6 does not reach: a stop leaves an accepted value that no sample has read, so its flush runs and
/// the Restore the brain owes goes after it. The fixture is the world's `harness_stop_debt_is_flushed_before_restore` (value 1
/// accepted at 8 on channel 0, the device at T = 12 with stream 0 open and not started, R0's FIFO holding a Stop, the Restore
/// enqueued explicitly: a Stop invents none), its trace recorded from the world with the canonical continuation, then replayed
/// on threads over `ModelDevice`. No cut is booked: the Stop is the whole trigger (test plan §15 F2).
#[test]
fn s6b_stop_debt_flush_then_restore() {
    use super::core::Reason;
    use super::rules::DevCall;
    use super::thread_tests::{JOIN, Script, expected, params};

    let w = super::fixtures::stop_debt_world(params());
    assert_eq!((w.device.t, w.cells[0].progress, w.cells[0].cut, w.device.streamers[&0].buffered.len()), (12, 0, None, 0), "the initial time, progress, cut and buffering");
    let mut script = Script::new(w);
    script.rest();
    let flushed = std::cell::Cell::new(false);
    {
        let mut replay = script.w_initial();
        for event in &script.trace {
            replay.apply(event);
            flushed.set(flushed.get() || replay.cells[0].phase == Phase::Flushing);
        }
    }
    assert!(flushed.get(), "the world reaches the flush: {:?}", script.trace);

    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let observed = rt.observed().unwrap();
    // The calls, in device order: the Stop, the flush's start and stop (with its receives between), then the Restore.
    let kinds: Vec<&str> = observed
        .calls
        .iter()
        .filter(|call| !matches!(call, DevCall::Recv { .. }))
        .map(|call| match call {
            DevCall::Stop { stream: 0 } => "stop",
            DevCall::StartNow { stream: 0 } => "start_now",
            DevCall::Apply { channel: 0, freq: Some(-1), at: None } => "restore",
            other => panic!("an unexpected call {other:?} in {:?}", observed.calls),
        })
        .collect();
    assert_eq!(kinds, ["stop", "start_now", "stop", "restore"], "{:?}", observed.calls);
    let first_recv = observed.calls.iter().position(|call| matches!(call, DevCall::Recv { .. })).expect("the flush reads");
    let flush_start = observed.calls.iter().position(|call| *call == DevCall::StartNow { stream: 0 }).unwrap();
    assert!(flush_start < first_recv, "the flush starts before it reads: {:?}", observed.calls);
    assert_eq!(observed.leftovers, vec![Fact { leftover: true, value: 1, stream: 0, channel: 0, sample: 12 }], "the flush reads value 1 at its first sample");
    assert!(observed.evidence.is_empty(), "no consumption qualifies as evidence: {:?}", observed.evidence);
    assert_eq!(observed.dispositions, vec![(1, Disposition::Reported(Reason::Left))]);
    rt.sh.with_cell(0, |cell| {
        assert_eq!(cell.phase, Phase::Off);
        assert!(cell.in_flight.is_empty());
    });
    assert!(rt.sh.brain().deferred.is_empty(), "the Restore went");
    assert_eq!(observed, expected(&script.origin, &script.trace).unwrap(), "the thread run equals the world's");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

// ------------------------------------------------------------------------------------------------------ S8

/// Case 5's shape on stream 0: the cut is 12 and the drain window 8, so the drain deadline is 20; the radio queue (capacity 1)
/// holds an earlier command due at 40, so the settings command for value 1 (instant 8) waits inside `apply` holding `L`; R0
/// holds ticks 8..11 in its stream, which reach the cut.
fn blocked_apply_world() -> World {
    use super::cell::ValueState;
    use super::core::{Change, SetCmd};
    use super::fixtures::push;
    use super::thread_tests::streaming;

    let mut w = streaming(Some(12));
    push(&mut w.cells[0], value(0, 8, 1), ValueState::Queued);
    w.settings.fifo.push_back(SetCmd { id: 1, change: Change::Timed { channel: 0, freq: 1.0, at: 8, generation: 1 } });
    w.monitor.reserved.insert(1, (0, seg(0), 1));
    w.device.queue.push_back(40);
    w.device.t = 12;
    w.cells[0].progress = 8;
    let streamer = w.device.streamers.get_mut(&0).expect("stream 0 is open");
    streamer.next = Some(0);
    streamer.buffered = std::collections::VecDeque::from([8, 9, 10, 11]);
    w
}

/// S8, the stop that waits for the native lock. S holds `L` inside `apply` (the queue is full until T = 40) while the drain
/// deadline passes. The brain's step that processes the timer ends the drain at once (I16: it does not wait for the pin), and it
/// sends the Stop; R0 takes the Stop and its lock is a pending attempt behind S (P1): the device sees no stop. When the queue's
/// head leaves at T = 40, S's call returns and releases `L`, and only then does R0 acquire it and stop; the value, pinned until S's
/// result, is left. The actual order is read from the model's log of `L`.
#[test]
fn s8_blocked_apply_drain_deadline_stop_waits_for_the_lock() {
    use super::cell::Outcome;
    use super::core::Reason;
    use super::force::Actor;
    use super::model_device::LEvent;
    use super::rules::DevCall;
    use super::thread_tests::{JOIN, Script, expected};
    use super::world::Event::*;

    let mut script = Script::new(blocked_apply_world());
    // The cut is reached while S is blocked in its call: the drain is booked for the deadline.
    script.ev(&[STake, SCheck, SLock, RCheck(0), RSnap(0), RBegin(0), REnd(0), RPublish(0), RSend(0), BStep]).ops();
    let booked = script.trace.len();
    script.ev(&[const { Tick }; 8]);
    // The deadline passes: the brain's step on the timer sends the Stop; R0 takes it.
    script.ev(&[Timer(0), BStep]).ops();
    let ended = script.trace.len();
    script.ev(&[RCheck(0), RCmd(0)]);
    let taken = script.trace.len();
    script.ev(&[const { Tick }; 20]);
    script.ev(&[SEffect(Outcome::Ok), SUnlock]);
    let released = script.trace.len();
    script.ev(&[RLock(0), RCall(0), RUnlock(0)]);
    let stopped = script.trace.len();
    script.rest();

    let rt = Runtime::start(&script.w_initial(), true, None);
    let calls_of = |rt: &Runtime| rt.observed().unwrap().calls;
    let stops = |calls: &[DevCall]| calls.iter().filter(|c| matches!(c, DevCall::Stop { .. })).count();
    rt.replay(&script.trace[..booked]).unwrap();
    rt.sh.with_cell(0, |cell| assert!(cell.pinned() && cell.drain == super::cell::Drain::Draining, "S's call pins the value; the drain is booked"));
    assert!(rt.sh.sched.is_blocked(Actor::Settings), "S waits inside apply: {}", rt.sh.sched.state_of(Actor::Settings));
    assert_eq!(rt.l_log(), vec![LEvent::Acquire(Actor::Settings)], "S holds L");
    // I16: the step that processed the deadline ended the drain, with S still blocked and the stop not yet issued.
    rt.replay(&script.trace[booked..ended]).unwrap();
    rt.sh.with_cell(0, |cell| assert!(cell.drain == super::cell::Drain::Ended && cell.pinned(), "the drain ended at the deadline although the value is pinned"));
    assert!(rt.sh.sched.is_blocked(Actor::Settings));
    assert_eq!(stops(&calls_of(&rt)), 0);
    // P1: R0 holds the Stop and waits for L: a pending attempt, not an event, and the device has no stop.
    rt.replay(&script.trace[ended..taken]).unwrap();
    assert!(rt.sh.sched.is_blocked(Actor::Rx(0)), "R0's stop waits for L: {}", rt.sh.sched.state_of(Actor::Rx(0)));
    assert!(rt.replay(&[RLock(0)]).unwrap_err().contains("expected Rx(0) at Lock"));
    rt.replay(&script.trace[taken..taken + 20]).unwrap();
    assert!(rt.sh.sched.is_blocked(Actor::Rx(0)), "still waiting at T = 40 while S has not released L");
    assert_eq!(rt.l_log(), vec![LEvent::Acquire(Actor::Settings)]);
    assert_eq!(stops(&calls_of(&rt)), 0, "no stop while S holds L");
    // S returns and releases; then R0 acquires and stops.
    rt.replay(&script.trace[taken + 20..released]).unwrap();
    assert_eq!(rt.l_log(), vec![LEvent::Acquire(Actor::Settings), LEvent::Release(Actor::Settings)]);
    assert_eq!(stops(&calls_of(&rt)), 0, "the stop waits for R0's own grant");
    rt.replay(&script.trace[released..stopped]).unwrap();
    assert_eq!(rt.l_log(), vec![LEvent::Acquire(Actor::Settings), LEvent::Release(Actor::Settings), LEvent::Acquire(Actor::Rx(0)), LEvent::Release(Actor::Rx(0))]);
    rt.replay(&script.trace[stopped..]).unwrap();
    let observed = rt.observed().unwrap();
    let apply = observed.calls.iter().position(|c| matches!(c, DevCall::Apply { .. })).unwrap();
    let stop = observed.calls.iter().position(|c| matches!(c, DevCall::Stop { .. })).unwrap();
    assert!(apply < stop, "the stop follows S's call: {:?}", observed.calls);
    assert_eq!(observed.terminals, vec![(1, Terminal::Call(Outcome::Ok))]);
    assert_eq!(observed.dispositions, vec![(1, Disposition::Reported(Reason::Left))], "pinned until the result, then left");
    assert_eq!(observed, expected(&script.origin, &script.trace).unwrap());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// S8, the free run (20x): while the clock reader waits for `L` behind S's blocked `apply`, the timer is still delivered (it
/// reads the time source without `L`), so the brain ends the drain at the deadline; nothing else moves until S releases `L`, and
/// then the clock's read follows S's release. Model time is advanced by the test.
#[test]
fn s8c_timer_delivered_while_clock_waits_for_l() {
    use super::cell::Drain;
    use super::force::Actor;
    use super::model_device::LEvent;
    use super::shared::ClockCmd;

    // As the forced world, but at T = 8 with nothing buffered: S must hold the pin before the stream reaches the cut.
    let mut world = blocked_apply_world();
    world.device.t = 8;
    world.device.streamers.get_mut(&0).expect("stream 0 is open").buffered.clear();
    let rt = Runtime::start(&world, false, None);
    poll("S holds L inside apply", || rt.sh.sched.is_blocked(Actor::Settings));
    rt.model.advance(4);
    poll("the cut is reached and the drain is booked", || !rt.sh.timers.lock().unwrap().is_empty());
    rt.sh.push(&rt.sh.k_fifo, ClockCmd::Serve { stream: 0 });
    poll("the clock reader waits for L", || rt.sh.sched.is_blocked(Actor::Clock));
    rt.model.advance(8);
    poll("the drain ends at the deadline", || rt.sh.with_cell(0, |cell| cell.drain == Drain::Ended));
    assert!(rt.sh.sched.is_blocked(Actor::Clock) && rt.sh.sched.is_blocked(Actor::Settings), "S still holds L and the clock still waits");
    assert_eq!(rt.l_log(), vec![LEvent::Acquire(Actor::Settings)]);
    rt.model.advance(20);
    // Behind S both R0 (its Stop) and the clock wait: each takes and releases L in turn, in either order.
    poll("both waiters have had L", || [Actor::Clock, Actor::Rx(0)].iter().all(|w| rt.l_log().contains(&LEvent::Release(*w))));
    let log = rt.l_log();
    assert_eq!(&log[..2], [LEvent::Acquire(Actor::Settings), LEvent::Release(Actor::Settings)], "{log:?}");
    for waiter in [Actor::Clock, Actor::Rx(0)] {
        let at = log.iter().position(|e| *e == LEvent::Acquire(waiter)).expect("the waiter acquired L");
        assert_eq!(log[at + 1], LEvent::Release(waiter), "L is exclusive: {log:?}");
        assert!(at >= 2, "after S's release: {log:?}");
    }
    let panics = rt.stop(Duration::from_secs(1)).expect("every thread joins");
    assert!(panics.is_empty(), "{panics:?}");
}

// ------------------------------------------------------------------------------------------------------ S9j

/// S9j (Sol's N2): the receive's outcome is fixed at its deadline, not at its completion. FakeDevice commits at the call's entry
/// (a packet not ready within the timeout is a `Timeout`, `device.rs:1763-1767`), so the model must too when the completion is
/// processed late. Forced, on the world and on threads over `ModelDevice`: start at 0, `recv` with the timeout under test, time
/// passes, the completion (`REnd`) is deferred, then done.
/// (a) timeout 3: the packet 0..3 is complete at T = 4, after the deadline; the completion at T = 4 still gives a timeout (nothing
/// published); the next recv returns the packet whole. (b) timeout 8: complete at T = 4, in time; the completion at T = 9, after the
/// deadline, returns it.
#[test]
fn s9j_the_deadline_decides_the_recv_not_its_completion() {
    use super::thread_tests::{Script, check, params};
    use super::world::Event::*;
    for (timeout, ticks, first_publication) in [(3, 4, Vec::<i64>::new()), (8, 9, vec![0, 1, 2, 3])] {
        let mut w = cases::single(None, Vec::new(), super::world::Params { recv_timeout: timeout, ..params() });
        w.cells[0].progress = 0;
        let mut script = Script::new(w);
        script.ev(&[RCheck(0), RSnap(0), RBegin(0)]);
        for _ in 0..ticks {
            script.ev(&[Tick]);
        }
        script.ev(&[REnd(0), RPublish(0)]);
        script.rest();
        check(&script.w_initial(), &script.trace).unwrap_or_else(|why| panic!("timeout {timeout}: {why}"));
        let observed = super::thread_tests::expected(&script.w_initial(), &script.trace).unwrap();
        assert_eq!(observed.published[0].ticks, first_publication, "timeout {timeout}: the first recv's publication");
        if timeout == 3 {
            assert_eq!(observed.published[1].ticks, vec![0, 1, 2, 3], "the next recv returns the packet whole");
        }
    }
}
