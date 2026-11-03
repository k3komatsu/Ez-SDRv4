//! The thread runtime (stage E2a; test plan §2–§4): the brain, the settings executor `S`, one receive executor `R_k` per
//! stream, the clock reader `K` and (outside forcing) a timer thread, over the same `Brain`, cell procedures and
//! `rules` functions as the DST world. The executors reach the device only through `Arc<dyn Device>`. The sequencing
//! here is the thread twin of `World`'s programs; every decision is the shared code's.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use super::cell::{Cell, Outcome, Phase, Recv, Take, Terminal};
use super::core::{Change, Command, Exit, Input, Op, SetCmd, Timer};
use super::force::{Actor, Forced, Point, Sched, join_all, spawn_on};
use super::model_device::{CheckingDevice, LEvent, ModelDevice, PARTIAL};
use super::observed::{Observed, Raw};
use super::rules::{self, CallPlan, CallResult, DeviceOp, FlushEnd, Handle, Next, Round};
use super::shared::{ClockCmd, Queue, Quirk, RxState, Shared, Sink};
use super::world::{EnvInput, EnvSlot, Event, INITIAL_OCCUPANT, PUB_STEPS, PubStep, World, guard_step};
use crate::device::{Device, Dir, FakeDevice, Settings};

/// The outcome a settings call's `Result` stands for (Q2: the partial marker is `ModelDevice`'s test convention; any other
/// non-lost error is a refusal, as FakeDevice's DSP FIFO refusal is).
pub(crate) fn outcome_of(result: &Result<crate::device::Applied, crate::device::DeviceError>) -> Outcome {
    match result {
        Ok(_) => Outcome::Ok,
        Err(error) if error.lost => Outcome::Lost,
        Err(error) if error.message == PARTIAL => Outcome::Partial,
        Err(_) => Outcome::Refused,
    }
}

/// What a run reads from its device: the device itself, the `now` source and, for a `FakeDevice`, the fake.
struct Sources {
    /// The wall time of one device tick on the fake's clock (a timer waits for its deadline on that clock).
    tick: Duration,
    inner: Arc<dyn crate::device::Device>,
    now: Box<dyn Fn() -> i64 + Send + Sync>,
    fake: Option<Arc<FakeDevice>>,
    /// E2b's measurement hooks (counters and the receive work), absent in every other run.
    bench: Option<Arc<super::speed::Bench>>,
    /// Device ticks per receive sample (the decimation at the device).
    sample_ticks: i64,
}

/// A running system: the shared state, the model behind it, and the threads.
pub(crate) struct Runtime {
    pub(crate) sh: Arc<Shared>,
    pub(crate) model: Arc<ModelDevice>,
    pub(crate) checking: Arc<CheckingDevice>,
    pub(crate) env: Vec<EnvSlot>,
    /// The `FakeDevice` of a run started by [`Runtime::start_fake`] (then `model` is an unused stand-in): the observation reads its logs.
    pub(crate) fake: Option<Arc<FakeDevice>>,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

impl Runtime {
    /// Starts the threads over `world`'s initial state (cells, brain, queues, device model, handles).
    pub(crate) fn start(world: &World, forcing: bool, real_panic: Option<usize>) -> Runtime {
        Runtime::start_with(world, forcing, real_panic, Quirk::None)
    }

    pub(crate) fn start_with(world: &World, forcing: bool, real_panic: Option<usize>, quirk: Quirk) -> Runtime {
        let sched = Sched::new(forcing);
        let model = ModelDevice::new(sched.clone(), world.device.clone(), world.params.packet);
        let now_model = model.clone();
        let sources = Sources { tick: Duration::from_millis(1), inner: model.clone(), now: Box::new(move || now_model.now()), fake: None, bench: None, sample_ticks: 1 };
        Runtime::build(world, sched, model, real_panic, quirk, sources, None)
    }

    /// A targeted free run (each actor waits on its own condition, as on a real-clock fake) whose threads pass `hook` at each [`Gate`](super::shared::Gate)
    /// (a test holds them there).
    pub(crate) fn start_held(world: &World, hook: super::shared::GateHook) -> Runtime {
        let sched = Sched::new(false);
        sched.set_targeted();
        let model = ModelDevice::new(sched.clone(), world.device.clone(), world.params.packet);
        let now_model = model.clone();
        let sources = Sources { tick: Duration::from_millis(1), inner: model.clone(), now: Box::new(move || now_model.now()), fake: None, bench: None, sample_ticks: 1 };
        Runtime::build(world, sched, model, None, Quirk::None, sources, Some(hook))
    }

    /// A free run over a `FakeDevice` on its own clock (S6, S7): the cells, queues and handles come from `world`, the device
    /// from `fake`, which the caller has opened and started to match them. The `now` source is the fake's own clock.
    pub(crate) fn start_fake(world: &World, fake: Arc<FakeDevice>, sample_ticks: i64) -> Runtime {
        let sched = Sched::new(false);
        let model = ModelDevice::new(sched.clone(), world.device.clone(), world.params.packet);
        let now_fake = fake.clone();
        let sources = Sources {
            tick: Duration::from_nanos(1_000_000_000 / fake.master_clock_rate()),
            inner: fake.clone(),
            now: Box::new(move || now_fake.time_now().expect("the fake's clock")),
            fake: Some(fake),
            bench: None,
            sample_ticks,
        };
        Runtime::build(world, sched, model, None, Quirk::None, sources, None)
    }

    /// E2b's free run over a `FakeDevice` seen through `device` (a timing decorator over `fake`), with its own `now` source and the
    /// measurement hooks `bench`.
    pub(crate) fn start_measured(
        world: &World,
        fake: Arc<FakeDevice>,
        device: Arc<dyn Device>,
        now: Box<dyn Fn() -> i64 + Send + Sync>,
        bench: Arc<super::speed::Bench>,
        sample_ticks: i64,
    ) -> Runtime {
        let sched = Sched::new(false);
        let model = ModelDevice::new(sched.clone(), world.device.clone(), world.params.packet);
        let sources = Sources { tick: Duration::from_nanos(1_000_000_000 / fake.master_clock_rate()), inner: device, now, fake: Some(fake), bench: Some(bench), sample_ticks };
        Runtime::build(world, sched, model, None, Quirk::None, sources, None)
    }

    fn build(world: &World, sched: Arc<Sched>, model: Arc<ModelDevice>, real_panic: Option<usize>, quirk: Quirk, sources: Sources, gate_hook: Option<super::shared::GateHook>) -> Runtime {
        let forcing = sched.forcing();
        let Sources { tick, inner, now, fake, bench, sample_ticks } = sources;
        // Every thread of a fake run on a manual clock is a participant of it (the receive executors as production's are, and the brain, S, K and
        // the timer, so none can fall behind simulated time).
        let host = fake.as_ref().map(|fake| fake.host()).filter(|host| host.is_manual());
        if let Some(host) = &host {
            sched.set_host(host.clone());
        }
        // A free run on a fake's real clock waits and wakes per actor (a manual clock's participants wait on that clock).
        if !forcing && fake.is_some() && host.is_none() {
            sched.set_targeted();
        }
        let failure = Arc::new(AtomicBool::new(world.failure));
        // A measurement run keeps no per-call history (the no-cell-held assertion stays).
        let checking = Arc::new(if bench.is_some() { CheckingDevice::unlogged(inner, failure.clone()) } else { CheckingDevice::new(inner, failure.clone()) });
        let streams = world.cells.len();
        let sh = Arc::new(Shared {
            sched: sched.clone(),
            cells: world.cells.iter().map(|cell| Mutex::new(cell.clone())).collect(),
            brain: Mutex::new(world.brain.clone()),
            failure,
            gate: Mutex::new(None),
            device: checking.clone(),
            inbox: Queue::new(Actor::Brain),
            s_fifo: Queue::new(Actor::Settings),
            s_terminate: AtomicBool::new(world.settings.terminate),
            r_fifo: (0..streams).map(|k| Queue::new(Actor::Rx(k))).collect(),
            rx: (0..streams)
                .map(|k| {
                    let carried = world
                        .device
                        .streamers
                        .get(&k)
                        .map_or(ezsdr_radio::timeline::Channels::first(0), |s| s.channels.iter().fold(ezsdr_radio::timeline::Channels::first(0), |c, &ch| c.with(ch as u32)));
                    Mutex::new(RxState { handle: world.rx[k].handle, handles: world.rx[k].handles, carried, consumed: world.rx[k].consumed.clone(), flush_end: world.rx[k].flush_end })
                })
                .collect(),
            k_fifo: Queue::new(Actor::Clock),
            timers: Mutex::new(world.timers.clone()),
            sink: Mutex::new(Sink { reservations: world.monitor.reserved.iter().map(|(&id, &(channel, segment, epoch))| (id, channel, segment, epoch)).collect(), ..Sink::default() }),
            params: world.params,
            now,
            tick,
            stopping: AtomicBool::new(false),
            halt_senders: AtomicBool::new(false),
            exit_model: world.teardown.is_some(),
            brain_exit: AtomicBool::new(false),
            real_panic,
            quirk,
            panic_in_op: Mutex::new(Vec::new()),
            ops_run: std::sync::atomic::AtomicUsize::new(0),
            before_write: Mutex::new(None),
            after_batch: Mutex::new(None),
            gate_hook: Mutex::new(gate_hook),
            bench,
            sample_ticks,
        });
        for input in &world.inbox {
            sh.push(&sh.inbox, input.clone());
        }
        for cmd in &world.settings.fifo {
            sh.push(&sh.s_fifo, cmd.clone());
        }
        for (k, r) in world.rx.iter().enumerate() {
            for op in &r.fifo {
                sh.push(&sh.r_fifo[k], *op);
            }
        }
        // Every thread that can send to the brain registers with its inbox, all of them before any thread starts (a zero count must never mean "not
        // registered yet"), and deregisters when it returns.
        let s_guard = (!world.settings.unspawned).then(|| sh.sender(Actor::Settings));
        let r_guards: Vec<_> = (0..streams).filter(|k| !world.rx[*k].unspawned).map(|k| (k, sh.sender(Actor::Rx(k)))).collect();
        let clock_guard = sh.sender(Actor::Clock);
        let timer_guard = (!forcing).then(|| sh.sender(Actor::Timer));
        let mut handles = Vec::new();
        let s = sh.clone();
        handles.push(spawn_on(&sched, host.as_ref(), Actor::Brain, move || run_brain(&s)));
        let s = sh.clone();
        handles.push(spawn_on(&sched, host.as_ref(), Actor::Settings, move || {
            run_settings(&s, s_guard);
            s.mark_exited(Actor::Settings);
        }));
        for (k, guard) in r_guards {
            let s = sh.clone();
            handles.push(spawn_on(&sched, host.as_ref(), Actor::Rx(k), move || {
                run_rx(&s, k, guard);
                s.mark_exited(Actor::Rx(k));
            }));
        }
        let s = sh.clone();
        handles.push(spawn_on(&sched, host.as_ref(), Actor::Clock, move || run_clock(&s, clock_guard)));
        if let Some(guard) = timer_guard {
            let s = sh.clone();
            handles.push(spawn_on(&sched, host.as_ref(), Actor::Timer, move || run_timer(&s, guard)));
        }
        Runtime { sh, model, checking, env: world.env.clone(), fake, handles: Mutex::new(handles) }
    }

    /// Replays `trace` event by event; an actor that is not where the trace says is an error naming both.
    pub(crate) fn replay(&self, trace: &[Event]) -> Result<(), String> {
        self.sh.sched.settle()?;
        for (index, event) in trace.iter().enumerate() {
            self.event(event).map_err(|why| format!("event {index} ({event:?}): {why}"))?;
        }
        Ok(())
    }

    fn event(&self, event: &Event) -> Result<(), String> {
        let sched = &self.sh.sched;
        let rx = Actor::Rx;
        let (actor, point, forced) = match *event {
            Event::Tick => {
                self.model.advance(1);
                return sched.settle();
            }
            Event::Env(i) => {
                match &self.env[i].input {
                    EnvInput::Input(input) => self.sh.send(input.clone()),
                    EnvInput::Loss => self.model.set_lost(),
                    EnvInput::Late { .. } => return Err("a late slot is delivered by the clock reader".to_string()),
                }
                return sched.settle();
            }
            Event::Timer(i) => {
                let mut timers = self.sh.timers.lock().unwrap_or_else(|e| e.into_inner());
                if i >= timers.len() {
                    return Err("no such timer".to_string());
                }
                let (timer, at) = timers.remove(i);
                drop(timers);
                let now = self.model.now();
                if now < at {
                    return Err(format!("a timer due at {at} is delivered at {now}"));
                }
                self.sh.send(timer_input(timer, now));
                return sched.settle();
            }
            Event::BStep => (Actor::Brain, Point::BStep, Forced::None),
            Event::BOp => (Actor::Brain, Point::BOp, Forced::None),
            Event::ClockLock(i) => {
                let EnvInput::Late { stream } = self.env[i].input else { return Err("not a late slot".to_string()) };
                self.sh.push(&self.sh.k_fifo, ClockCmd::Serve { stream });
                (Actor::Clock, Point::Lock, Forced::None)
            }
            Event::ClockRead => (Actor::Clock, Point::Call, Forced::None),
            Event::ClockUnlock => (Actor::Clock, Point::Unlock, Forced::None),
            Event::ClockSend => (Actor::Clock, Point::ClockSend, Forced::None),
            Event::STake => (Actor::Settings, Point::STake, Forced::None),
            Event::SDiscard => (Actor::Settings, Point::SDiscard, Forced::None),
            Event::SCheck => (Actor::Settings, Point::SCheck, Forced::None),
            Event::SLock => (Actor::Settings, Point::Lock, Forced::None),
            Event::SEffect(outcome) => (Actor::Settings, Point::Call, Forced::Outcome(outcome)),
            Event::SUnlock => (Actor::Settings, Point::Unlock, Forced::None),
            Event::SResult => (Actor::Settings, Point::SResult, Forced::None),
            Event::SSend => (Actor::Settings, Point::SSend, Forced::None),
            Event::RCheck(k) => (rx(k), Point::RCheck, Forced::None),
            Event::RCmd(k) => (rx(k), Point::RCmd, Forced::None),
            Event::RSnap(k) => (rx(k), Point::RSnap, Forced::None),
            Event::RBegin(k) => (rx(k), Point::RBegin, Forced::None),
            Event::REnd(k) => (rx(k), Point::REnd, Forced::None),
            Event::RPublish(k) => (rx(k), Point::RPublish, Forced::None),
            Event::RLock(k) => (rx(k), Point::Lock, Forced::None),
            Event::RCall(k) => (rx(k), Point::Call, Forced::None),
            Event::RFault(k) => (rx(k), Point::Call, Forced::Fault),
            Event::RUnlock(k) => (rx(k), Point::Unlock, Forced::None),
            Event::RGate(k) => (rx(k), Point::RGate, Forced::None),
            Event::RUngate(k) => (rx(k), Point::RUngate, Forced::None),
            Event::RResult(k) => (rx(k), Point::RResult, Forced::None),
            Event::RSend(k) => (rx(k), Point::RSend, Forced::None),
            // The provider's stop, cleanup and start prefix are not threads of this runtime: dedicated tests drive them (the cases that model them are
            // not replayed).
            Event::BExit => {
                // The brain thread returns: after its guard it already has; otherwise it is parked waiting for an input, and is told there is none.
                if self.sh.sink().exited.contains(&Actor::Brain) {
                    return sched.settle();
                }
                self.sh.brain_exit.store(true, Ordering::SeqCst);
                (Actor::Brain, Point::BStep, Forced::None)
            }
            Event::Spawn(_) | Event::Join | Event::Detach | Event::CleanupDecide | Event::CleanupClose(_) | Event::Destroy(_) | Event::OwnerRelease => {
                return Err(format!("{event:?} is a provider event; the thread runtime does not replay it"));
            }
            Event::Pub(holder, step) => {
                let actor = match holder {
                    super::world::Holder::Settings => Actor::Settings,
                    super::world::Holder::Rx(k) => Actor::Rx(k),
                    other => return Err(format!("{other:?} has no publication events")),
                };
                (actor, Point::Pub(step), Forced::None)
            }
        };
        sched.step(actor, point, forced)
    }

    /// The provider's cleanup (note 45 §4.9): with a detached thread or the failure flag set it leaves the streamers alone; otherwise it closes
    /// every streamer, and each destructor reads the native mark. Returns the decision, as the world's `CleanupDecide` does.
    pub(crate) fn cleanup(&self, detached: bool) -> super::world::Cleanup {
        if detached || self.sh.failed() {
            return super::world::Cleanup::Retain;
        }
        self.sh.device.close_streams();
        super::world::Cleanup::Close
    }

    /// The stopper's join of `actor` with a limit in device ticks: time passes (a forced run's `Tick`) until the actor has ended or the limit has
    /// passed; returns whether it ended (false: the stopper detaches it, and the thread keeps running).
    pub(crate) fn join_within(&self, actor: Actor, limit: i64) -> Result<bool, String> {
        let start = self.model.now();
        loop {
            if self.sh.sink().exited.contains(&actor) {
                return Ok(true);
            }
            if self.model.now() >= start + limit {
                return Ok(false);
            }
            self.replay(&[Event::Tick])?;
        }
    }

    /// The final release of the native device's last owner (`UhdDevice::drop`, once no thread holds the device): the streamers still open are
    /// freed by the native destructors, which read the native mark and not `F`.
    pub(crate) fn release_owner(&self) {
        self.model.release_streamers();
    }

    /// The model device's destructor log: the streamers freed and those retained.
    pub(crate) fn destructors(&self) -> (Vec<usize>, Vec<usize>) {
        let st = self.model.state();
        (st.destroyed.clone(), st.retained.clone())
    }

    /// The observation of the run so far: the system must be quiet.
    pub(crate) fn raw(&self) -> Raw {
        let (model, mut calls, _) = self.model.snapshot();
        let mut applications: std::collections::BTreeMap<usize, Vec<(u64, i64)>> =
            model.dsp.iter().map(|(&channel, chain)| (channel, chain.log.iter().filter(|(_, id)| *id != INITIAL_OCCUPANT).map(|&(tick, id)| (id, tick)).collect())).collect();
        if let Some(fake) = &self.fake {
            // The fake's calls are its own lines (`FakeDevice::calls`); the applications are its DDC's timed changes, the value id in the frequency.
            calls.clear();
            applications = (0..2).map(|channel| (channel, fake.dsp_log(Dir::Rx, channel).iter().filter(|(_, _, timed)| *timed).map(|&(tick, freq, _)| (freq as u64, tick)).collect())).collect();
            applications.retain(|_, log| !log.is_empty());
        }
        let brain = self.sh.brain();
        let sink = self.sh.sink();
        let mut exited = sink.exited.clone();
        if brain.dead() {
            exited.insert(Actor::Brain);
        }
        Raw {
            dispositions: brain.dispositions.clone(),
            terminals: sink.terminals.clone(),
            facts: sink.facts.clone(),
            published: sink.published.clone(),
            reservations: sink.reservations.clone(),
            applications,
            calls,
            exited,
            brain_exit: brain.exit,
        }
    }

    pub(crate) fn observed(&self) -> Result<Observed, String> {
        Observed::extract(self.raw())
    }

    pub(crate) fn l_log(&self) -> Vec<LEvent> {
        self.model.snapshot().2
    }

    /// Ends the run: abort every parked or blocked actor, join within `bound`; returns the panics that were not aborts.
    pub(crate) fn abort(&self, bound: Duration) -> Result<Vec<String>, String> {
        self.sh.sched.abort();
        let handles = std::mem::take(&mut *self.handles.lock().unwrap_or_else(|e| e.into_inner()));
        join_all(handles, bound)?;
        Ok(self.sh.sched.panics())
    }

    /// Ends a free run: every wait ends when its queue is empty, and the threads join within `bound`.
    pub(crate) fn stop(&self, bound: Duration) -> Result<Vec<String>, String> {
        self.sh.stopping.store(true, Ordering::SeqCst);
        self.sh.sched.notify();
        let handles = std::mem::take(&mut *self.handles.lock().unwrap_or_else(|e| e.into_inner()));
        let started = std::time::Instant::now();
        // Time keeps passing while the threads wind down, so a recv that waits for a packet or its timeout ends (a fake run's own
        // clock runs by itself; the advance below then touches only the unused stand-in model).
        while !handles.iter().all(|handle| handle.is_finished()) {
            if started.elapsed() > bound {
                return Err("a thread did not finish within the bound".to_string());
            }
            self.model.advance(1);
            std::thread::sleep(Duration::from_millis(1));
        }
        join_all(handles, bound)?;
        Ok(self.sh.sched.panics())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.sh.sched.abort();
    }
}

// ------------------------------------------------------------------------------------------------ the brain

pub(super) fn route(sh: &Arc<Shared>, command: Command) {
    sh.sink().routed.push(command.clone());
    match command {
        Command::Settings(cmd) => {
            if let Some(bench) = &sh.bench {
                bench.stage(super::speed::Stage::Pushed, cmd.id);
            }
            sh.push(&sh.s_fifo, cmd)
        }
        Command::Rx { stream, op } => sh.push(&sh.r_fifo[stream], op),
        Command::Timer { stream, at } => {
            sh.timers.lock().unwrap_or_else(|e| e.into_inner()).push((Timer::Drain(stream), at));
            sh.sched.wake(Actor::Timer);
        }
        Command::CapacityTimer { at } => {
            sh.timers.lock().unwrap_or_else(|e| e.into_inner()).push((Timer::Capacity, at));
            sh.sched.wake(Actor::Timer);
        }
        Command::SettingsTerminate => {
            sh.s_terminate.store(true, Ordering::SeqCst);
            sh.sched.wake(Actor::Settings);
        }
    }
}

/// One brain operation: its event, a guard's publication step (the world does it before the operation), the operation on
/// every cell, the reservation record, the dead brain's closing senders and the routing of the outbox.
fn brain_op(sh: &Arc<Shared>, op: Op) {
    let a = Actor::Brain;
    if let Op::Guard(guard) = op {
        sh.publish_step(a, guard_step(guard), Point::BOp);
    } else {
        sh.sched.hook(a, Point::BOp);
    }
    let reserving = rules::reserving(&sh.brain(), &op);
    let number = sh.ops_run.fetch_add(1, Ordering::SeqCst);
    let mut attempt = None;
    let mut in_flight = None;
    let mut loads = (0, 0);
    sh.with_cells(|cells| {
        assert!(!sh.panic_in_op.lock().unwrap_or_else(|e| e.into_inner()).contains(&number), "injected panic in a brain operation");
        let mut brain = sh.brain();
        brain.op(cells);
        let reserved = reserving.and_then(|r| rules::reserved(&brain, cells, r));
        // A rejected attempt's reason is read for the measurement only (a refusal leaves the cells as they were).
        let reason = if sh.bench.is_some() && reserved.is_none() { reserving.and_then(|(id, _, _)| brain.reservation_refusal(id, cells)) } else { None };
        attempt = reserving.map(|(id, _, _)| (id, reserved.is_some(), reason));
        // The diagnostic's occupancy and retirement observation (stream 0 only), read where the operation ran.
        in_flight = sh.bench.as_ref().filter(|bench| bench.stamps.is_some()).map(|_| cells[0].in_flight.iter().map(|flight| flight.entry.order.1).collect::<Vec<u64>>());
        // K's load and D's occupancy after the operation (the measurement's, stream 0 being the one chain of the experiment).
        if sh.bench.is_some() {
            loads = (super::core::device_load(cells, brain.now), cells[0].in_flight.len());
        }
        if let Some(reserved) = reserved {
            let (id, channel, segment, epoch) = reserved;
            sh.sink().reservations.push((id, channel, segment, epoch));
        }
    });
    if let (Some(bench), Some((id, reserved, reason))) = (&sh.bench, attempt) {
        bench.reserve_attempt(id, reserved, reason, loads.0, loads.1);
    }
    if let (Some(bench), Some(ids)) = (&sh.bench, &in_flight) {
        bench.observe_in_flight(ids, loads.0);
    }
    let (dead, commands, unspawned, control_unspawned) = {
        let mut brain = sh.brain();
        (brain.dead(), std::mem::take(&mut brain.outbox), brain.unspawned.clone(), brain.control_unspawned)
    };
    if dead {
        sh.timers.lock().unwrap_or_else(|e| e.into_inner()).clear();
        for command in rules::dead_commands(sh.cells.len(), &unspawned, control_unspawned) {
            route(sh, command);
        }
    }
    for command in commands {
        route(sh, command);
    }
    sh.sched.changed();
}

/// Every queued operation of the brain; true when the brain is dead after one of them.
fn run_ops(sh: &Arc<Shared>) -> bool {
    loop {
        let front = sh.brain().ops.front().cloned();
        let Some(op) = front else { return false };
        brain_op(sh, op);
        if sh.brain().dead() {
            return true;
        }
    }
}

/// The guard of a brain that panicked, one straight-line routine run from the unwind handler (note 45 §4.5 procedure 4). It is not
/// driven by the brain's operations, so nothing injected into an operation can reach it. `G` is held from before `F` is published
/// until after it, so no lifecycle call is admitted between the failure and its flag; the steps already done by the panicked guard
/// (a gate it holds, `F`, the mark) are not repeated. It passes the DST guard's points in the DST guard's order (gate, flag, ungate,
/// mark, each a brain operation event), then ends the brain and routes what a dead brain's senders do.
fn guard_routine(sh: &Arc<Shared>) {
    let a = Actor::Brain;
    let holds = *sh.gate.lock().unwrap_or_else(|e| e.into_inner()) == Some(a);
    if !sh.failed() {
        if !holds {
            sh.publish_step(a, PubStep::Gate, Point::BOp);
        }
        sh.publish_step(a, PubStep::Set, Point::BOp);
        sh.publish_step(a, PubStep::Ungate, Point::BOp);
    } else if holds {
        sh.publish_step(a, PubStep::Ungate, Point::BOp);
    }
    if !sh.sink().order.contains(&"mark") {
        sh.publish_step(a, PubStep::Mark, Point::BOp);
    }
    {
        let mut brain = sh.brain();
        brain.ops.clear();
        brain.exit = Exit::Dead;
    }
    sh.timers.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let (unspawned, control_unspawned) = {
        let brain = sh.brain();
        (brain.unspawned.clone(), brain.control_unspawned)
    };
    for command in rules::dead_commands(sh.cells.len(), &unspawned, control_unspawned) {
        route(sh, command);
    }
    sh.sched.notify();
}

/// The unwind payload of an aborted run is passed on; any other payload is a panic of the brain.
fn is_abort(payload: &(dyn std::any::Any + Send)) -> bool {
    payload.downcast_ref::<super::force::Aborted>().is_some()
}

fn run_brain(sh: &Arc<Shared>) {
    let a = Actor::Brain;
    let mut at = 0usize;
    loop {
        sh.gate(super::shared::Gate::BrainTop);
        if sh.exit_model && !sh.sched.forcing() && sh.brain().exit == Exit::Terminated && !sh.halt_senders.swap(true, Ordering::SeqCst) {
            // The clock reader and the timer are told to stop; each finishes the send it has begun and deregisters.
            sh.sched.notify();
        }
        let pred = sh.pred(|s| s.inbox.nonempty() || s.stopping.load(Ordering::SeqCst) || s.brain_exit.load(Ordering::SeqCst) || s.brain_may_exit());
        let (_, input) = sh.sched.acquire(a, Point::BStep, pred, || sh.inbox.pop());
        if let (Some(bench), Some(Input::Release(held))) = (&sh.bench, &input) {
            bench.stage(super::speed::Stage::BrainTake, held.entry.order.1);
        }
        let Some(mut input) = input else {
            sh.mark_exited(a);
            sh.sched.notify();
            return;
        };
        if rules::panics_at(at, sh.params.panic_step) {
            input = Input::Panic;
        }
        let injected = sh.real_panic == Some(at);
        at += 1;
        // One iteration, the step and every operation it queued, runs under the panic guard.
        let iteration = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert!(!injected, "injected brain panic");
            sh.brain().step(input);
            run_ops(sh)
        }));
        let dead = match iteration {
            Ok(dead) => dead,
            Err(payload) if is_abort(&*payload) => std::panic::resume_unwind(payload),
            Err(_) => {
                guard_routine(sh);
                true
            }
        };
        // The thread returns after its guard, and (with the stop model, free running) once the brain has terminated and nothing is queued for it;
        // under forcing the trace's `BExit` event ends it (`Runtime::event`), because an input may still arrive between the two.
        sh.gate(super::shared::Gate::ExitTest);
        let finished = dead || sh.brain_may_exit();
        if finished {
            sh.mark_exited(a);
            sh.sched.notify();
            return;
        }
    }
}

// ------------------------------------------------------------------------------------------ the settings executor

fn run_settings(sh: &Arc<Shared>, _sender: Option<super::shared::SenderGuard>) {
    let a = Actor::Settings;
    loop {
        let pred = sh.pred(|s| s.s_fifo.nonempty() || s.s_terminate.load(Ordering::SeqCst) || s.stopping.load(Ordering::SeqCst));
        let (_, cmd) = sh.sched.acquire(a, Point::STake, pred, || sh.s_fifo.pop());
        let Some(cmd) = cmd else { break };
        if let Some(bench) = &sh.bench {
            bench.stage(super::speed::Stage::STake, cmd.id);
        }
        if !settle_one(sh, &cmd) {
            break;
        }
    }
    // An exited `S` discards what is routed to it afterwards (I1) until the brain's terminate, the last thing it routes to `S`, has arrived and
    // the queue is empty: `S` then returns and deregisters.
    loop {
        let pred = sh.pred(|s| s.s_fifo.nonempty() || s.stopping.load(Ordering::SeqCst) || s.s_terminate.load(Ordering::SeqCst));
        // A wait that is not an event (under forcing the thread returns on its own); only a queued command is an event, the discard.
        sh.sched.wait_until(a, pred.clone());
        if !sh.s_fifo.nonempty() {
            return;
        }
        let (_, cmd) = sh.sched.acquire(a, Point::SDiscard, pred, || sh.s_fifo.pop());
        let Some(cmd) = cmd else { return };
        sh.abandon(cmd.id);
    }
}

/// One taken command, through its result and its send. False when the executor exits (the failure flag was set).
fn settle_one(sh: &Arc<Shared>, cmd: &SetCmd) -> bool {
    let a = Actor::Settings;
    let early = (sh.quirk == Quirk::CallBeforeTake).then(|| call_settings(sh, cmd));
    let take = sh.with_cells(|cells| rules::take_command(cells, cmd));
    if take != Take::Call {
        if let Some(bench) = &sh.bench {
            bench.count(&bench.take_dropped);
        }
        sh.sink().terminals.push((cmd.id, Terminal::Dropped));
        sh.send(Input::Done { id: cmd.id, terminal: Terminal::Dropped, now: (sh.now)() });
        return true;
    }
    sh.sched.hook(a, Point::SCheck);
    // After a set flag the commands to abandon leave the live FIFO in one step; a command routed later goes to the discarder.
    let failure = sh.failed();
    let batch = if failure { sh.s_fifo.take_all() } else { Vec::new() };
    let mut queued: VecDeque<u64> = batch.iter().map(|c| c.id).collect();
    let hook = sh.after_batch.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(hook) = hook {
        hook(sh);
    }
    if rules::settings_check(failure, Some(cmd.id), &mut queued, |id| sh.abandon(id)) == rules::Check::Exit {
        return false;
    }
    sh.with_cells(|cells| rules::start_calls(cells, cmd.id));
    let outcome = early.unwrap_or_else(|| call_settings(sh, cmd));
    // The acceptance is recorded where the call returns, at the time source read there (the world does the same at `S.unlock`); the device's
    // own application instant is the device's fact and stays apart.
    if matches!(cmd.change, Change::Timed { .. }) && matches!(outcome, Outcome::Ok | Outcome::Partial) {
        let at = (sh.now)();
        sh.with_cells(|cells| rules::accept_all(cells, cmd.id, at));
        sh.sink().acceptances.push((cmd.id, at));
    }
    if outcome == Outcome::Lost {
        for step in PUB_STEPS {
            sh.publish_step(a, step, Point::Pub(step));
        }
    }
    let terminal = Terminal::Call(outcome);
    let write = |sh: &Arc<Shared>| {
        sh.sched.hook(a, Point::SResult);
        sh.with_cells(|cells| rules::settle_call(cells, cmd, terminal));
        sh.sink().terminals.push((cmd.id, terminal));
        sh.sched.changed();
    };
    if sh.quirk == Quirk::SendBeforeResult {
        sh.sched.hook(a, Point::SSend);
        sh.send(Input::Done { id: cmd.id, terminal, now: (sh.now)() });
        write(sh);
        return true;
    }
    write(sh);
    sh.sched.hook(a, Point::SSend);
    sh.send(Input::Done { id: cmd.id, terminal, now: (sh.now)() });
    true
}

/// The device call of a settings command, as its outcome.
fn call_settings(sh: &Arc<Shared>, cmd: &SetCmd) -> Outcome {
    match rules::settings_call(&cmd.change) {
        Some((channel, freq, at)) => {
            let settings = Settings { freq: freq.map(|f| f as f64), ..Settings::default() };
            // The measurement's decorator records which request this call carries (the owner's own identity of the booking).
            if let Some(bench) = &sh.bench {
                bench.clock.tag(cmd.id);
            }
            outcome_of(&sh.device.apply(Dir::Rx, channel, &settings, at))
        }
        // A move has no settings call: it takes the native lock through a null call, whose own result is the outcome (a lost device
        // errs at the call point, as the world's effect does); no second device read follows it.
        None => match sh.device.time_now() {
            Ok(_) => Outcome::Ok,
            Err(error) if error.lost => Outcome::Lost,
            Err(_) => Outcome::Refused,
        },
    }
}

// -------------------------------------------------------------------------------------------- a receive executor

fn run_rx(sh: &Arc<Shared>, k: usize, _sender: super::shared::SenderGuard) {
    let a = Actor::Rx(k);
    // The loop's one decision per iteration, with a view of the cell when the iteration receives (a free run: the first of the block's two cell
    // sections; forcing takes its snapshot and view at their own events).
    let decide: Arc<dyn Fn() -> Result<Looked, Option<Duration>> + Send + Sync> = {
        let s = sh.clone();
        Arc::new(move || {
            let looked = loop_next(&s, k, !s.sched.forcing());
            if s.stopping.load(Ordering::SeqCst) || looked.next != Next::Idle { Ok(looked) } else { Err(looked.timed.then_some(Duration::from_millis(2))) }
        })
    };
    loop {
        let looked = sh.sched.acquire_with(a, Point::RCheck, decide.clone());
        let (next, view) = looked.map_or((Next::Idle, None), |l| (l.next, l.view));
        if sh.stopping.load(Ordering::SeqCst) {
            return;
        }
        match next {
            Next::Exit => {
                sh.r_fifo[k].clear();
                sh.sched.hook(a, Point::RSend);
                sh.gate(super::shared::Gate::FinalSend(a));
                sh.send(Input::Boundary(super::core::Boundary::Lost { stream: k }));
                return;
            }
            Next::Cmd => {
                sh.sched.hook(a, Point::RCmd);
                let op = sh.r_fifo[k].pop().expect("a command is queued");
                match rules::call_of(op) {
                    None => {
                        sh.r_fifo[k].clear();
                        return;
                    }
                    Some((call, lifecycle, quiesce)) => run_call(sh, k, call, lifecycle, quiesce),
                }
            }
            Next::FlushStop => run_call(sh, k, rules::RxCall::FlushStop, false, false),
            Next::Recv => recv_cycle(sh, k, view),
            Next::Idle => {}
        }
    }
}

/// The receive executor's loop-top decision, with what it read.
struct Looked {
    next: Next,
    /// A copy of the cell, taken in the same section when the decision is to receive and a view was asked for.
    view: Option<Cell>,
    /// An idle executor whose decision depends on the time (a flush that ends at its drain window): it wakes by itself.
    timed: bool,
}

/// What the receive executor does at its loop top (the shared decision, read from the shared state). `carried` (the channels the
/// streamer was opened on) is the definition `World::carried` reads from the device model: it changes only at open and close.
fn loop_next(sh: &Shared, k: usize, want_view: bool) -> Looked {
    let failure = sh.failed();
    let fifo = sh.r_fifo[k].nonempty();
    // The executor's own state, copied out so no lock is held across the cell's (`consumed` holds only values still in the cell: see `recv_cycle`).
    let (handle_open, flush_end, consumed) = {
        let st = sh.rx_state(k);
        (st.handle.is_some(), st.flush_end, st.consumed.clone())
    };
    // The time is read only where the decision uses it: a flush has an end only while it runs.
    let now = if flush_end.is_some() { (sh.now)() } else { 0 };
    let at_end = sh.at_end();
    let (next, view) = view_of(sh, k, |cell| {
        let over = rules::flush_over(cell, &consumed, flush_end, now, at_end);
        let receiving = rules::receiving(cell, handle_open);
        let next = rules::next_action(failure, fifo, over, receiving, at_end);
        (next, (want_view && next == Next::Recv).then(|| cell.clone()))
    });
    Looked { next, view, timed: flush_end.is_some() }
}

/// One device call of the executor with its steps: the gate around a lifecycle call (rechecking the flag inside it), the
/// call under `L`, the cell's result, the boundary, the loss publication or the gate's release.
fn run_call(sh: &Arc<Shared>, k: usize, call: rules::RxCall, lifecycle: bool, quiesce: bool) {
    let a = Actor::Rx(k);
    if lifecycle {
        sh.gate_acquire(a, Point::RGate);
        if sh.failed() {
            sh.gate_drop();
            return;
        }
    }
    let open = sh.rx_state(k).handle.is_some();
    let result = match rules::plan_call(call, open) {
        CallPlan::Do(op) => do_op(sh, k, op),
        CallPlan::Skip(skipped) => {
            let read = sh.device.time_now();
            if sh.device.serial().is_err() {
                CallResult::Lost
            } else if read.is_err() {
                CallResult::Failed
            } else {
                skipped
            }
        }
    };
    sh.sched.hook(a, Point::RResult);
    let now = (sh.now)();
    let after = sh.with_cell(k, |cell| rules::after_call(cell, k, call, result, now, sh.params.drain_window));
    {
        let mut st = sh.rx_state(k);
        match after.handle {
            Handle::Opened => {
                st.handles += 1;
                st.handle = Some(st.handles);
            }
            Handle::Closed => st.handle = None,
            Handle::Keep => {}
        }
        match after.flush_end {
            FlushEnd::Set(end) => st.flush_end = Some(end),
            FlushEnd::Clear => st.flush_end = None,
            FlushEnd::Keep => {}
        }
    }
    sh.sched.notify();
    if let Some(input) = after.send {
        sh.sched.hook(a, Point::RSend);
        sh.send(input);
    }
    if result == CallResult::Lost {
        for step in PUB_STEPS {
            if lifecycle && step == PubStep::Gate {
                continue;
            }
            sh.publish_step(a, step, Point::Pub(step));
        }
        return;
    }
    if quiesce {
        sh.sched.hook(a, Point::RSend);
        sh.sink().events.push("quiesce");
        sh.send(Input::Boundary(super::core::Boundary::Quiesced { stream: k }));
    }
    if lifecycle {
        sh.gate_release(a, Point::RUngate);
    }
}

fn do_op(sh: &Arc<Shared>, k: usize, op: DeviceOp) -> CallResult {
    let dev = &sh.device;
    match op {
        DeviceOp::Start { at } => CallResult::of(&dev.rx_start(k, at)),
        DeviceOp::StartNow => CallResult::of(&dev.rx_start_now(k)),
        DeviceOp::Stop => CallResult::of(&dev.rx_stop(k, None)),
        DeviceOp::Open => {
            let channels: Vec<usize> = sh.with_cell(k, |cell| cell.channels.iter().map(|c| c as usize).collect());
            let result = dev.rx_open(k, &channels);
            if result.is_ok() {
                sh.rx_state(k).carried = channels.iter().fold(ezsdr_radio::timeline::Channels::first(0), |c, &ch| c.with(ch as u32));
            }
            CallResult::of(&result)
        }
        DeviceOp::Close => {
            dev.rx_close(k);
            // The trait's close is infallible: a streamer that is still open after it did not close (a failure; a lost
            // device is told by the identity read).
            let still_open = dev.rx_packet_samples(k) != 0;
            rules::close_outcome(still_open, still_open && dev.serial().is_err())
        }
    }
}

/// One recv cycle: snapshot, the recv, the publish (recomputing from a fresh snapshot while it is stale), the boundaries.
/// `R`'s one acquisition of its cell (the loop top's decision and view, a later view, or the write of a block). In a diagnostic run its wait and hold are recorded here, after the
/// lock is released, so the recording is not inside the time it measures.
fn view_of<T>(sh: &Shared, k: usize, f: impl FnOnce(&mut Cell) -> T) -> T {
    if !super::speed::instrument_on() {
        return sh.with_cell(k, f);
    }
    let asked = std::time::Instant::now();
    let (value, got, released) = sh.with_cell(k, |cell| {
        let got = std::time::Instant::now();
        let value = f(cell);
        (value, got, std::time::Instant::now())
    });
    super::speed::R_CELL_WAIT.record(got - asked);
    super::speed::R_CELL_HOLD.record(released - got);
    value
}

fn recv_cycle(sh: &Arc<Shared>, k: usize, mut first_view: Option<Cell>) {
    let a = Actor::Rx(k);
    sh.sched.hook(a, Point::RSnap);
    // A free run's snapshot is the loop top's view: the recv began under it, and the first write rechecks against that same copy.
    let mut snap = match &first_view {
        Some(view) => view.snapshot(),
        None => sh.with_cell(k, |cell| cell.snapshot()),
    };
    sh.sched.hook(a, Point::RBegin);
    if sh.at_end() {
        return;
    }
    let handle = sh.rx_state(k).handle.expect("an open handle");
    let result = sh.device.rx_recv(k, sh.params.packet, Duration::from_millis(sh.params.recv_timeout as u64), true);
    let now = (sh.now)();
    let ended = rules::end_of_recv(rules::received_of(&result, sh.sample_ticks), k, now);
    if ended.lost {
        for step in PUB_STEPS {
            sh.publish_step(a, step, Point::Pub(step));
        }
        return;
    }
    let recv = Recv { epoch: snap.epoch, handle, samples: ended.samples };
    loop {
        sh.sched.hook(a, Point::RPublish);
        if sh.quirk == Quirk::PublishWithoutRecheck {
            snap = sh.with_cell(k, |cell| cell.snapshot());
        }
        let (carried, seen) = {
            let st = sh.rx_state(k);
            (st.carried, st.consumed.clone())
        };
        // Compute outside the cell from a view of it; inside, recheck that nothing the computation read has changed, and write
        // (note 45 §4.5). A cell that moved on in between sends the round back to the view.
        let (round, facts, view) = loop {
            let view = first_view.take().unwrap_or_else(|| view_of(sh, k, |cell| cell.clone()));
            let computed = rules::publish_compute(&view, &recv, carried, &seen, sh.bench.is_none());
            let facts: Vec<super::world::Fact> = computed
                .consumed
                .iter()
                .map(|&(id, sample)| {
                    let channel = view.in_flight.iter().find(|f| f.entry.order.1 == id).map_or(usize::MAX, |f| f.entry.channel);
                    super::world::Fact { leftover: view.is_left(id), value: id, stream: k, channel, sample }
                })
                .collect();
            let hook = sh.before_write.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(hook) = hook {
                hook(sh);
            }
            let written = view_of(sh, k, |cell| cell.unchanged_since(&view).then(|| rules::publish_write(cell, k, snap, &recv, computed)));
            if let Some(round) = written {
                break (round, facts, view);
            }
            if let Some(bench) = &sh.bench {
                bench.count(&bench.publish_retries);
            }
        };
        match (round, facts) {
            (Round::Stale, _) => {
                if let Some(bench) = &sh.bench {
                    bench.count(&bench.stale_rounds);
                }
                sh.sched.hook(a, Point::RSnap);
                snap = sh.with_cell(k, |cell| cell.snapshot());
            }
            (Round::Published { sends, consumed, published, discarded }, facts) => {
                {
                    let mut st = sh.rx_state(k);
                    st.consumed.extend(consumed.iter().map(|&(id, _)| id));
                    // Only values still in the cell are ever read again (a value's id is never reserved twice): the history stays as small as the cell.
                    st.consumed.retain(|id| view.in_flight.iter().any(|f| f.entry.order.1 == *id));
                }
                if let Some(bench) = &sh.bench {
                    // A measurement run keeps counters only: the per-sample tick lists of every block would grow without bound. The sink gets
                    // what the cell accepted and nothing else, after the acceptance.
                    bench.count(&bench.publishes);
                    if view.phase != Phase::Flushing {
                        let (origin, bound) = (view.origin_of(recv.epoch), view.bound(recv.epoch));
                        let range = rules::published_range(origin, bound, &recv.samples);
                        let before = recv.samples.index_from(origin) as u64;
                        bench.add(&bench.discarded_before_origin, before);
                        bench.add(&bench.discarded_after_cut, discarded.saturating_sub(before));
                        if !range.is_empty() {
                            (bench.sink)(&result, range);
                        }
                    }
                } else {
                    let mut sink = sh.sink();
                    sink.events.push("publish");
                    sink.published.push(super::world::Published { stream: k, epoch: recv.epoch, ticks: published, discarded });
                    sink.facts.extend(facts);
                }
                sh.sched.changed();
                for input in rules::recv_inputs(sends, ended.late_send.clone()) {
                    sh.sched.hook(a, Point::RSend);
                    sh.send(input);
                }
                break;
            }
        }
    }
}

// ----------------------------------------------------------------------------------------- the clock and timers

fn run_clock(sh: &Arc<Shared>, _sender: super::shared::SenderGuard) {
    let a = Actor::Clock;
    loop {
        let pred = sh.pred(|s| s.k_fifo.nonempty() || s.stopping.load(Ordering::SeqCst) || s.halt_senders.load(Ordering::SeqCst));
        sh.sched.wait_until(a, pred);
        if sh.halt_senders.load(Ordering::SeqCst) {
            return;
        }
        let Some(ClockCmd::Serve { stream }) = sh.k_fifo.pop() else {
            if sh.stopping.load(Ordering::SeqCst) {
                return;
            }
            continue;
        };
        let now = sh.device.time_now().unwrap_or_else(|_| (sh.now)());
        sh.sched.hook(a, Point::ClockSend);
        sh.gate(super::shared::Gate::FinalSend(a));
        sh.send(Input::LateCommand { stream, now });
    }
}

/// The input a delivered timer is: the drain deadline of a stream or the capacity timer, with the device time read at delivery.
fn timer_input(timer: Timer, now: i64) -> Input {
    match timer {
        Timer::Drain(stream) => Input::Timer { stream, now },
        Timer::Capacity => Input::CapacityTimer { now },
    }
}

/// Outside forcing: delivers a brain-booked deadline once the time source has passed it (never early). On a fake's manual clock the
/// timer is a participant and waits for the earliest deadline on that clock (a wait that ends at the deadline, or at a notify), so the
/// clock stops there until the delivery is made; otherwise it polls the time source.
fn run_timer(sh: &Arc<Shared>, _sender: super::shared::SenderGuard) {
    let a = Actor::Timer;
    if let Some(host) = sh.sched.host() {
        loop {
            let seen = host.generation();
            if sh.stopping.load(Ordering::SeqCst) || sh.halt_senders.load(Ordering::SeqCst) {
                return;
            }
            let now = (sh.now)();
            let due = {
                let mut timers = sh.timers.lock().unwrap_or_else(|e| e.into_inner());
                timers.iter().position(|&(_, at)| now >= at).map(|i| timers.remove(i))
            };
            if let Some((timer, _)) = due {
                sh.gate(super::shared::Gate::FinalSend(a));
                sh.send(timer_input(timer, now));
                continue;
            }
            let next = sh.timers.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|&(_, at)| at).min();
            let deadline = next.map(|at| host.now() + sh.tick * (at - now).clamp(0, 1 << 20) as u32);
            host.wait(deadline, Some(std::time::Instant::now() + Duration::from_millis(50)), seen);
        }
    }
    loop {
        // One evaluation under the scheduler's lock: a due deadline is delivered; otherwise the timer sleeps until the earliest one, or until a booking
        // or the stop wakes it.
        let due = sh.sched.wait_for(a, || {
            let now = (sh.now)();
            if sh.halt_senders.load(Ordering::SeqCst) {
                return Ok(None);
            }
            let mut timers = sh.timers.lock().unwrap_or_else(|e| e.into_inner());
            match timers.iter().position(|&(_, at)| now >= at) {
                Some(i) => Ok(Some(timers.remove(i))),
                None if sh.stopping.load(Ordering::SeqCst) => Ok(None),
                None => Err(timers.iter().map(|&(_, at)| at).min().map(|at| sh.tick * (at - now).clamp(1, 1 << 20) as u32)),
            }
        });
        match due {
            Some((timer, _)) => {
                sh.gate(super::shared::Gate::FinalSend(a));
                sh.send(timer_input(timer, (sh.now)()))
            }
            None => return,
        }
    }
}
