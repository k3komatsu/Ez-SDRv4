//! Tests of the thread runtime (stage E2a-1): forced replays of DST traces, the observation, the controller's
//! self-tests, the I10 oracle and the sequences S2–S5 and S10 (test plan §7, §13–§15).

use std::time::Duration;

use super::cases::{self, Variant};
use super::cell::{Outcome, Phase, Terminal, ValueState};
use super::core::{Change, Command, Input, RxOp, SetCmd};
use super::dst::{self, Model};
use super::fixtures::{cell, held, push, seg, value};
use super::force::Actor;
use super::model_device::LEvent;
use super::observed::Observed;
use super::rules::DevCall;
use super::shared::{Quirk, held_cells};
use super::threads::Runtime;
use super::world::{EnvInput, EnvSlot, Event, Params, World};
use Event::*;

pub(super) const JOIN: Duration = Duration::from_secs(5);

/// The observation of `world` after `trace`, as the DST world makes it.
pub(super) fn expected(world: &World, trace: &[Event]) -> Result<Observed, String> {
    let mut w = world.clone();
    for event in trace {
        w.apply(event);
    }
    Observed::of_world(&w)
}

/// Replays `trace` on threads over `world`'s initial state and returns the observation, tearing the run down by abort.
fn on_threads(world: &World, trace: &[Event], quirk: Quirk) -> Result<Observed, String> {
    let rt = Runtime::start_with(world, true, None, quirk);
    let result = rt.replay(trace).and_then(|()| rt.observed());
    let panics = rt.abort(JOIN)?;
    if let Some(panic) = panics.first() {
        return Err(format!("a thread panicked: {panic}"));
    }
    result
}

/// `Ok` when the thread run equals the world's observation at the trace's end; `Err` names the divergence.
pub(super) fn check(world: &World, trace: &[Event]) -> Result<(), String> {
    check_with(world, trace, Quirk::None)
}

fn check_with(world: &World, trace: &[Event], quirk: Quirk) -> Result<(), String> {
    let want = expected(world, trace)?;
    let got = on_threads(world, trace, quirk)?;
    if want == got {
        return Ok(());
    }
    let first = want.differing(&got);
    Err(format!("the thread run differs from the world in {first:?}:\n world   {want:?}\n threads {got:?}"))
}

/// A divergence, shrunk: the shortest failing prefix, then each event dropped while the world still enables the rest and the
/// run still fails.
fn shrink(world: &World, trace: &[Event]) -> Vec<Event> {
    let fails = |t: &[Event]| check(world, t).is_err();
    let mut best: Vec<Event> = trace.to_vec();
    for end in 1..=trace.len() {
        if fails(&trace[..end]) {
            best = trace[..end].to_vec();
            break;
        }
    }
    let mut i = 0;
    while i < best.len() {
        let mut candidate = best.clone();
        candidate.remove(i);
        let mut w = world.clone();
        let legal = candidate.iter().all(|event| {
            let enabled = w.enabled().contains(event);
            if enabled {
                w.apply(event);
            }
            enabled
        });
        if legal && fails(&candidate) {
            best = candidate;
        } else {
            i += 1;
        }
    }
    best
}

/// Replays `trace`; on a divergence panics with the shrunk trace.
fn assert_replays(name: &str, world: &World, trace: &[Event]) {
    if let Err(why) = check(world, trace) {
        let small = shrink(world, trace);
        panic!("{name}: {why}\n shrunk to {} events: {small:?}", small.len());
    }
}

type WitnessTraces = (std::collections::BTreeMap<&'static str, (u64, Vec<Event>)>, Vec<&'static str>);

/// The first trace that reaches each declared witness of a case over `seeds`, with the seed.
fn witness_traces(build: &dyn Fn(u64) -> cases::Case, seeds: std::ops::RangeInclusive<u64>) -> WitnessTraces {
    let mut traces = std::collections::BTreeMap::new();
    let mut declared: Vec<&'static str> = Vec::new();
    for seed in seeds {
        let case = build(seed);
        // Case 6 declares different witnesses per panic position: the declarations of every seed count.
        for witness in case.config.witnesses {
            if !declared.contains(witness) {
                declared.push(witness);
            }
        }
        let walked = dst::walk(&case.world, seed, cases::WALK_LIMIT);
        assert!(walked.failure.is_none(), "the world's own walk fails at seed {seed}: {:?}", walked.failure);
        for (witness, length) in cases::witness_prefixes(&case.world, &walked.trace) {
            traces.entry(witness).or_insert_with(|| (seed, walked.trace[..length].to_vec()));
        }
    }
    (traces, declared)
}

/// S1(b): every declared witness of every case over `seeds`; `require_all` makes a missing witness a failure. Returns
/// `(replayed, missing)`.
fn s1b(seeds_for: impl Fn(&str) -> std::ops::RangeInclusive<u64>, require_all: bool) {
    s1b_over(cases::full_builders(), seeds_for, require_all);
}

fn s1b_over(builders: Vec<(&'static str, cases::Builder)>, seeds_for: impl Fn(&str) -> std::ops::RangeInclusive<u64>, require_all: bool) {
    let mut replayed = 0;
    let mut missing: Vec<String> = Vec::new();
    let mut divergences: Vec<String> = Vec::new();
    for (name, build) in builders {
        let (traces, declared) = witness_traces(&*build, seeds_for(name));
        // Case 6 declares a different set per k: judge each trace against the case that produced it.
        for (witness, (seed, trace)) in &traces {
            let case = build(*seed);
            if !case.config.witnesses.contains(witness) {
                continue;
            }
            replayed += 1;
            if let Err(why) = check(&case.world, trace) {
                let small = shrink(&case.world, trace);
                divergences.push(format!("{name} witness {witness} seed {seed}: {why}\n shrunk to {} events: {small:?}", small.len()));
            }
        }
        for witness in declared {
            if !traces.contains_key(witness) {
                missing.push(format!("{name}: {witness}"));
            }
        }
    }
    println!("s1b: {replayed} witness traces replayed, missing witnesses: {missing:?}");
    assert!(divergences.is_empty(), "{} divergences:\n{}", divergences.len(), divergences.join("\n"));
    assert!(!require_all || missing.is_empty(), "declared witnesses with no retained trace: {missing:?}");
}

/// Sol E2a-1 P5: a declared witness that no seed reaches fails the full check, whatever the other seeds declare (case 6 declares
/// different witnesses per panic position, so the declarations of every seed count, not the last seed's).
#[test]
#[should_panic(expected = "declared witnesses with no retained trace")]
fn s1b_fails_when_a_declared_witness_is_missing() {
    let builders: Vec<(&'static str, cases::Builder)> = vec![(
        "a case whose first seed declares a witness nothing reaches",
        Box::new(|seed| {
            let mut case = cases::case1(Variant::Full);
            case.config.witnesses = if seed == 1 { &["W1a", "W-never-reached"] } else { &["W1a"] };
            case
        }),
    )];
    s1b_over(builders, |_| 1..=20, true);
}

/// A smoke subset: seeds 1–300 (seeded configuration 1–50); a witness no seed reaches is printed, not failed. It does not
/// satisfy S1(b): `s1b_full` does.
#[test]
fn s1b_smoke() {
    s1b(|name| if name == "seeded configuration" { 1..=50 } else { 1..=300 }, false);
}

/// S1(b) in full: the seeds of the wide sweep (`EZSDR_SEEDS`, default 1–20000), every declared witness required.
#[test]
#[ignore = "S1(b) in full: run in release on sim02/sim03"]
fn s1b_full() {
    let (first, last) = std::env::var("EZSDR_SEEDS").ok().and_then(|spec| spec.split_once('-').map(|(a, b)| (a.trim().parse().unwrap(), b.trim().parse().unwrap()))).unwrap_or((1, 20_000));
    s1b(|_| first..=last, true);
}

// ------------------------------------------------------------------------------------------------ the script helper

/// A hand-written trace, built against the world so that every event is legal where it stands.
pub(super) struct Script {
    pub(super) origin: World,
    pub(super) w: World,
    pub(super) trace: Vec<Event>,
}

impl Script {
    pub(super) fn new(w: World) -> Script {
        Script { origin: w.clone(), w, trace: Vec::new() }
    }
    pub(super) fn w_initial(&self) -> World {
        self.origin.clone()
    }
    pub(super) fn ev(&mut self, events: &[Event]) -> &mut Script {
        for event in events {
            assert!(self.w.enabled().contains(event), "{event:?} is not enabled after {:?}", self.trace);
            self.w.apply(event);
            self.trace.push(event.clone());
        }
        self
    }
    /// Every operation the brain's step queued.
    pub(super) fn ops(&mut self) -> &mut Script {
        while !self.w.brain.ops.is_empty() {
            self.ev(&[BOp]);
        }
        self
    }
    /// The canonical continuation: the first enabled event that is not a tick, a tick when nothing else is.
    pub(super) fn rest(&mut self) -> &mut Script {
        for _ in 0..20_000 {
            let enabled = self.w.enabled();
            let Some(event) = enabled.iter().find(|e| **e != Tick).or_else(|| enabled.first()).cloned() else { return self };
            self.ev(&[event]);
        }
        panic!("no terminal state");
    }
}

pub(super) fn params() -> Params {
    Params {
        horizon: 48,
        epilogue: 32,
        recv_timeout: 8,
        queue_cap: 1,
        fifo_cap: 32,
        drain_window: 8,
        packet: crate::brain::world::PACKET,
        call_faults: false,
        panic_step: None,
        k: crate::brain::core::DEVICE_BOUND,
        d: crate::brain::core::CHAIN_BOUND,
    }
}

/// Stream 0 streaming with `cut`, started at 0.
pub(super) fn streaming(cut: Option<i64>) -> World {
    cases::single(cut, Vec::new(), params())
}

/// A streaming world with the value 1 (instant 8) queued in the cell and its settings command in S's FIFO, and `ops` in R0's FIFO.
pub(super) fn with_command(ops: &[RxOp]) -> World {
    let mut w = streaming(Some(16));
    push(&mut w.cells[0], value(0, 8, 1), ValueState::Queued);
    w.settings.fifo.push_back(SetCmd { id: 1, change: Change::Timed { channel: 0, freq: 1.0, at: 8, generation: 1 } });
    w.rx[0].fifo.extend(ops.iter().copied());
    w.monitor.reserved.insert(1, (0, seg(0), 1));
    w
}

fn stop() -> RxOp {
    RxOp::Stop { at: None, epoch: 1 }
}

// --------------------------------------------------------------------------------------------------------- S1(a)

#[test]
fn s1a_counterexamples_replay_on_threads() {
    // The scripted trace of the only scripted counterexample (case 5 micro).
    let world = cases::case5(Variant::Micro).world;
    assert_replays("ce_shutdown_during_start", &world, &cases::counterexamples::shutdown_during_start_trace());
    // The other counterexample has no scripted trace (its test is its seed's walk): its walk trace is replayed.
    let case = cases::case7b(Variant::Full);
    let walked = dst::walk(&case.world, 43409, cases::WALK_LIMIT);
    assert!(walked.failure.is_none());
    assert_replays("ce_restore_issued_streaming (seed 43409)", &case.world, &walked.trace);
}

// ------------------------------------------------------------------------------------- acquisition (F1, N1)

/// `L` is free with S after its check and R0 at its command: either may acquire first, and the `L` log follows the trace.
#[test]
fn s1_l_free_either_acquisition_order() {
    let prefix = [STake, SCheck, RCheck(0), RCmd(0)];
    let s_first = [SLock, SEffect(Outcome::Ok), SUnlock, RLock(0), RCall(0), RUnlock(0)];
    let r_first = [RLock(0), RCall(0), RUnlock(0), SLock, SEffect(Outcome::Ok), SUnlock];
    for (name, tail, first) in [("S first", s_first, Actor::Settings), ("R first", r_first, Actor::Rx(0))] {
        let mut script = Script::new(with_command(&[stop()]));
        script.ev(&prefix).ev(&tail).rest();
        let rt = Runtime::start(&script.w_initial(), true, None);
        rt.replay(&script.trace).unwrap_or_else(|why| panic!("{name}: {why}"));
        let log = rt.l_log();
        assert_eq!(log[0], super::model_device::LEvent::Acquire(first), "{name}: the first acquisition");
        assert_eq!(rt.observed().unwrap(), Observed::of_world(&script.w).unwrap(), "{name}");
        assert!(rt.abort(JOIN).unwrap().is_empty());
    }
}

/// While S holds `L`, R0's lock is a pending attempt: R0 is blocked (not parked at a permit), and the trace cannot select it.
#[test]
fn s1_l_held_makes_the_other_attempt_pending() {
    let mut script = Script::new(with_command(&[stop()]));
    script.ev(&[STake, SCheck, RCheck(0), RCmd(0), SLock]);
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    assert!(rt.sh.sched.is_blocked(Actor::Rx(0)), "R0 waits for L: {}", rt.sh.sched.state_of(Actor::Rx(0)));
    let err = rt.replay(&[RLock(0)]).unwrap_err();
    assert!(err.contains("expected Rx(0) at Lock"), "{err}");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// The brain's guard and R0's lifecycle gate: `G` is free and either may take it first.
#[test]
fn s1_g_free_either_acquisition_order() {
    let world = || {
        let mut w = cases::single(None, Vec::new(), params());
        w.cells[0] = cell(Phase::Off, 0, None, 0);
        w.rx[0].fifo.push_back(RxOp::Close);
        w.inbox.push_back(Input::Panic);
        w
    };
    // R first: R0 takes G, the brain's gate waits until R0 releases it.
    let mut r_first = Script::new(world());
    r_first.ev(&[RCheck(0), RCmd(0), BStep, RGate(0)]);
    let rt = Runtime::start(&r_first.w_initial(), true, None);
    rt.replay(&r_first.trace).unwrap();
    assert!(rt.sh.sched.is_blocked(Actor::Brain), "the brain's gate is pending: {}", rt.sh.sched.state_of(Actor::Brain));
    assert!(rt.replay(&[BOp]).unwrap_err().contains("expected Brain at BOp"));
    assert!(rt.abort(JOIN).unwrap().is_empty());
    r_first.ev(&[RLock(0), RCall(0), RUnlock(0), RResult(0), RSend(0), RUngate(0)]).ops().rest();
    check(&r_first.w_initial(), &r_first.trace).unwrap();
    // Brain first: R0's gate waits until the guard releases it (S4a continues from here).
    let mut b_first = Script::new(world());
    b_first.ev(&[RCheck(0), RCmd(0), BStep, BOp]);
    let rt = Runtime::start(&b_first.w_initial(), true, None);
    rt.replay(&b_first.trace).unwrap();
    assert!(rt.sh.sched.is_blocked(Actor::Rx(0)), "R0's gate is pending: {}", rt.sh.sched.state_of(Actor::Rx(0)));
    assert!(rt.replay(&[RGate(0)]).unwrap_err().contains("expected Rx(0) at RGate"));
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

// ------------------------------------------------------------------------------------------------------ S3, S4

/// S3: `F` is set (by the brain's guard, through its `BOp`s) while a command waits in S's FIFO; S takes it, its check sees `F`,
/// and it is abandoned: no `Device::apply`, the terminal is `Abandoned`, `Done{Abandoned}` is sent, S exits (I1).
#[test]
fn s3_failure_set_then_queued_command_is_abandoned() {
    let mut w = streaming(Some(16));
    w.params.panic_step = Some(1);
    w.inbox.push_back(Input::Release(held(0, value(0, 8, 1), 1)));
    w.inbox.push_back(Input::Stop { stream: 0, requested: 10 });
    let mut script = Script::new(w);
    script.ev(&[BStep]).ops().ev(&[BStep]).ops().ev(&[STake, SCheck]).rest();
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let observed = rt.observed().unwrap();
    assert!(!observed.calls.iter().any(|c| matches!(c, DevCall::Apply { .. })), "no Device::apply after F: {:?}", observed.calls);
    assert_eq!(observed.terminals, vec![(1, Terminal::Abandoned)]);
    assert!(observed.exited.contains(&Actor::Settings));
    assert!(rt.sh.failed());
    assert_eq!(observed, Observed::of_world(&script.w).unwrap());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// S4a: `F` is published (the brain's guard) before R0 takes the lifecycle gate: the recheck inside `G` refuses, no `Close` is
/// called, the handle stays open, and no lock is taken.
#[test]
fn s4a_failure_before_the_gate_refuses_the_close() {
    let mut w = cases::single(None, Vec::new(), params());
    w.cells[0] = cell(Phase::Off, 0, None, 0);
    w.rx[0].fifo.push_back(RxOp::Close);
    w.inbox.push_back(Input::Panic);
    let mut script = Script::new(w);
    script.ev(&[RCheck(0), RCmd(0), BStep, BOp, BOp, BOp, RGate(0)]).ops().rest();
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let observed = rt.observed().unwrap();
    assert!(!observed.calls.iter().any(|c| matches!(c, DevCall::Close { .. })), "no close after F: {:?}", observed.calls);
    assert!(rt.model.snapshot().0.streamers.contains_key(&0), "the handle is retained");
    assert!(rt.l_log().is_empty(), "the refused call took no lock");
    assert!(rt.sh.gate.lock().unwrap().is_none(), "the gate is released");
    assert_eq!(observed, Observed::of_world(&script.w).unwrap());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// S4b: the device is lost while R0 holds `G` for its close: the close returns lost, the loss is published inside the held gate
/// (no second acquisition), `G` is released, nothing is closed and nothing deadlocks.
#[test]
fn s4b_loss_inside_the_gate_publishes_without_retaking_it() {
    let mut w = cases::single(None, vec![EnvSlot { input: EnvInput::Loss, earliest: 0, latest: 10, delivered: false, after: None }], params());
    w.cells[0] = cell(Phase::Off, 0, None, 0);
    w.rx[0].fifo.push_back(RxOp::Close);
    let mut script = Script::new(w);
    script.ev(&[RCheck(0), RCmd(0), RGate(0), Env(0), RLock(0), RCall(0), RUnlock(0), RResult(0)]).rest();
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let observed = rt.observed().unwrap();
    assert!(!observed.calls.iter().any(|c| matches!(c, DevCall::Close { .. })), "{:?}", observed.calls);
    assert!(rt.sh.failed());
    assert!(rt.sh.gate.lock().unwrap().is_none());
    assert_eq!(observed, Observed::of_world(&script.w).unwrap());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// An ordinary call whose check came before `F` may still be made (I1's window): S checks, then `F` is set, then S calls.
#[test]
fn s3b_check_before_failure_allows_the_call() {
    let mut w = with_command(&[]);
    w.inbox.push_back(Input::Panic);
    let mut script = Script::new(w);
    script.ev(&[STake, SCheck, BStep]).ops().ev(&[SLock, SEffect(Outcome::Ok), SUnlock]).rest();
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let observed = rt.observed().unwrap();
    assert_eq!(observed.calls.iter().filter(|c| matches!(c, DevCall::Apply { .. })).count(), 1, "the call whose check came before F is made");
    assert_eq!(observed, Observed::of_world(&script.w).unwrap());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

// ------------------------------------------------------------------------------------------------------- S10

/// S10: Stop shortens the cut to 14, then a plan change retires the epoch while R0 is inside a recv that holds ticks 12..15. The
/// old epoch publishes only 12 and 13, 14 and 15 are discarded and counted, the quiesce comes after the publication, and the new
/// epoch's start is sent only after it (I6, I12).
pub(super) fn plan_change_world() -> World {
    let mut w = cases::single(Some(16), Vec::new(), params());
    w.cells[0].progress = 12;
    w.device.t = 16;
    let streamer = w.device.streamers.get_mut(&0).expect("stream 0 is open");
    streamer.next = Some(0);
    streamer.buffered = std::collections::VecDeque::from([12, 13, 14, 15]);
    w.inbox.push_back(Input::Stop { stream: 0, requested: 14 });
    w.inbox.push_back(Input::PlanCancel { stream: 0, origin: 20, epoch: 2, plan_cut: Some(40) });
    w
}

fn plan_change_script() -> Script {
    let mut script = Script::new(plan_change_world());
    script.ev(&[RCheck(0), RSnap(0), RBegin(0), BStep]).ops().ev(&[BStep]).ops().ev(&[REnd(0), RPublish(0), RSnap(0), RPublish(0)]).rest();
    script
}

#[test]
fn s10_plan_change_in_recv_publishes_to_the_retiring_cut_then_quiesces() {
    let script = plan_change_script();
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let observed = rt.observed().unwrap();
    let old: Vec<_> = observed.published.iter().filter(|p| p.epoch == 1).collect();
    assert_eq!(old.len(), 1, "{:?}", observed.published);
    assert_eq!(old[0].ticks, vec![12, 13], "published only up to the retiring cut");
    assert_eq!(old[0].discarded, 2, "the rest is discarded and counted");
    let stop = observed.calls.iter().position(|c| *c == DevCall::Stop { stream: 0 }).expect("the plan's stop");
    let start = observed.calls.iter().position(|c| matches!(c, DevCall::Start { at: 20, .. })).expect("the new epoch's start");
    assert!(stop < start, "no start before the quiesce ack: {:?}", observed.calls);
    let events = rt.sh.sink().events.clone();
    let publish = events.iter().position(|e| *e == "publish").expect("the old epoch publishes");
    let quiesce = events.iter().position(|e| *e == "quiesce").expect("the quiesce is acknowledged");
    assert!(publish < quiesce, "the quiesce ack follows the publication: {events:?}");
    assert_eq!(observed, Observed::of_world(&script.w).unwrap());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

// ------------------------------------------------------------------------------------- controller self-tests

#[test]
fn controller_rejects_wrong_actor() {
    let mut w = streaming(Some(16));
    w.inbox.push_back(Input::Release(held(0, value(0, 8, 1), 1)));
    let rt = Runtime::start(&w, true, None);
    let err = rt.replay(&[SCheck]).unwrap_err();
    assert!(err.contains("event 0") && err.contains("expected Settings at SCheck, found Some(Blocked"), "{err}");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

#[test]
fn controller_rejects_swapped_events() {
    let mut script = Script::new(with_command(&[]));
    script.ev(&[STake, SCheck]);
    let rt = Runtime::start(&script.w_initial(), true, None);
    let err = rt.replay(&[SCheck, STake]).unwrap_err();
    assert!(err.contains("expected Settings at SCheck, found Some(Parked(STake"), "{err}");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

#[test]
fn controller_rejects_an_event_the_world_disables() {
    let mut script = Script::new(with_command(&[stop()]));
    script.ev(&[STake, SCheck, RCheck(0), RCmd(0), SLock]);
    assert!(!script.w.enabled().contains(&RLock(0)));
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let err = rt.replay(&[RLock(0)]).unwrap_err();
    assert!(err.contains("expected Rx(0) at Lock, found Blocked: the resource is held"), "{err}");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// A valid trace on a runtime that sends `Done` before it writes the result is rejected at the result.
#[test]
fn controller_rejects_a_park_elsewhere() {
    let mut script = Script::new(with_command(&[]));
    script.ev(&[STake, SCheck, SLock, SEffect(Outcome::Ok), SUnlock, SResult, SSend]);
    let rt = Runtime::start_with(&script.w_initial(), true, None, Quirk::SendBeforeResult);
    let err = rt.replay(&script.trace).unwrap_err();
    assert!(err.contains("event 5") && err.contains("expected Settings at SResult, found Some(Parked(SSend"), "{err}");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

// ------------------------------------------------------------------ a wrapper that reorders the protocol fails S1

#[test]
fn s1_wrapper_calling_before_take_fails() {
    let mut script = Script::new(with_command(&[]));
    script.ev(&[STake, SCheck, SLock]);
    let rt = Runtime::start_with(&script.w_initial(), true, None, Quirk::CallBeforeTake);
    let err = rt.replay(&script.trace).unwrap_err();
    assert!(err.contains("expected Settings at SCheck, found Some(Parked(Lock"), "{err}");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

#[test]
fn s1_wrapper_publishing_without_the_recheck_fails() {
    let script = plan_change_script();
    let rt = Runtime::start_with(&script.w_initial(), true, None, Quirk::PublishWithoutRecheck);
    assert!(rt.replay(&script.trace).is_err(), "a publish from a fresh snapshot skips the stale branch the trace takes");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

// ----------------------------------------------------------------------------------------------------- S2

/// I10: a device call with a cell guard held panics; without it the call returns.
#[test]
fn s2_device_call_with_cell_guard_panics() {
    let world = streaming(None);
    let rt = Runtime::start(&world, false, None);
    assert_eq!(held_cells(), 0);
    assert!(rt.sh.device.serial().is_ok());
    let guard = rt.sh.lock_cell(0);
    assert_eq!(held_cells(), 1);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = rt.sh.device.serial();
    }));
    let message = caught.expect_err("the device call must panic");
    let message = message.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("I10"), "{message}");
    drop(guard);
    assert_eq!(held_cells(), 0, "the guard's drop decrements the count");
    assert!(rt.sh.device.serial().is_ok());
    rt.sh.with_cells(|cells| assert_eq!(held_cells(), cells.len(), "every cell of a section counts"));
    assert_eq!(held_cells(), 0);
    rt.stop(JOIN).unwrap();
}

/// A panic inside a section restores the cells (unwind-safe scratch swap).
#[test]
fn with_cells_restores_the_cells_on_a_panic() {
    let rt = Runtime::start(&streaming(Some(16)), false, None);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.sh.with_cells(|cells| panic!("{}", cells.len())))).is_err();
    assert!(caught);
    rt.sh.with_cell(0, |cell| assert_eq!(cell.cut, Some(16), "the cell is back"));
    rt.stop(JOIN).unwrap();
}

// ----------------------------------------------------------------------------------------------------- S5

/// S5: a real panic in the brain thread at step `k` (free run, model clock driven by the test): the guard sets `F` before it
/// calls `mark_lost`, no handle is closed after `F`, and every thread joins within a second.
/// Waits (advancing model time) until every thread of `actors` has really ended, at most a second.
fn wait_exited(rt: &Runtime, actors: &[Actor]) {
    let started = std::time::Instant::now();
    while !actors.iter().all(|a| rt.sh.sched.is_exited(*a)) && started.elapsed() < Duration::from_secs(1) {
        rt.model.advance(1);
        std::thread::sleep(Duration::from_millis(1));
    }
    let states: Vec<_> = actors.iter().map(|a| (*a, rt.sh.sched.state_of(*a))).collect();
    assert!(actors.iter().all(|a| rt.sh.sched.is_exited(*a)), "every thread ended on its own: {states:?}");
}

fn brain_panic_at(k: usize) {
    let world = streaming(Some(16));
    let rt = Runtime::start(&world, false, Some(k));
    rt.sh.send(Input::Release(held(0, value(0, 8, 1), 1)));
    rt.sh.send(Input::Stop { stream: 0, requested: 10 });
    let started = std::time::Instant::now();
    while !rt.sh.brain().dead() && started.elapsed() < Duration::from_secs(5) {
        rt.model.advance(1);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(rt.sh.brain().dead(), "the guard ran");
    let order = rt.sh.sink().order.clone();
    assert_eq!(order, vec!["F", "mark"], "F is set before mark_lost");
    let calls = rt.checking.calls();
    let mark = calls.iter().position(|(name, _)| *name == "mark_lost").expect("mark_lost was called");
    assert!(calls[mark].1, "F was set when mark_lost was called: {calls:?}");
    assert!(!calls.iter().any(|(name, failed)| *failed && matches!(*name, "rx_close" | "rx_open")), "no handle closed or opened after F: {calls:?}");
    // The brain is dead: the brain thread, S and every R_k end on their own (their terminate, the closed sender), within a
    // second and before any `Runtime::stop` helps; only the clock reader waits for the stop.
    wait_exited(&rt, &[Actor::Brain, Actor::Settings, Actor::Rx(0)]);
    let panics = rt.stop(Duration::from_secs(1)).expect("every thread joins within 1 s");
    assert!(panics.iter().all(|p| p.contains("injected")), "{panics:?}");
}

/// Sol E2a-1 P2: a panic inside a brain operation (inside its cell section) restores the cells and runs the guard: `F`, then
/// `mark_lost`; the brain is dead and the cell is as it was.
#[test]
fn s5b_panic_inside_a_brain_operation_runs_the_guard_and_restores_the_cells() {
    let rt = Runtime::start(&streaming(Some(16)), false, None);
    *rt.sh.panic_in_op.lock().unwrap() = vec![0];
    rt.sh.send(Input::Release(held(0, value(0, 8, 1), 1)));
    let started = std::time::Instant::now();
    while !rt.sh.brain().dead() && started.elapsed() < Duration::from_secs(3) {
        rt.model.advance(1);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(rt.sh.brain().dead(), "the guard ran after the unwind inside the operation");
    assert_eq!(rt.sh.sink().order, vec!["F", "mark"]);
    assert!(rt.sh.failed());
    rt.sh.with_cell(0, |cell| {
        assert_eq!(cell.cut, Some(16), "the scratch swap restored the cell");
        // The running receive executor may have moved the phase on since: the identity fields show the cell is not the vacant one.
        assert_eq!((cell.generation, cell.epoch), (1, 1));
        assert!(cell.channels.contains(0));
    });
    let calls = rt.checking.calls();
    let mark = calls.iter().position(|(name, _)| *name == "mark_lost").expect("mark_lost was called");
    assert!(calls[mark].1, "F was set when mark_lost was called: {calls:?}");
    wait_exited(&rt, &[Actor::Brain, Actor::Settings, Actor::Rx(0)]);
    let panics = rt.stop(Duration::from_secs(1)).expect("every thread joins");
    assert!(panics.iter().all(|p| p.contains("injected")), "{panics:?}");
}

#[test]
fn s5_brain_panic_at_the_release() {
    brain_panic_at(0);
}

#[test]
fn s5_brain_panic_at_the_stop() {
    brain_panic_at(1);
}

/// How many divergences of a whole-walk sweep are shrunk and printed in full.
const SHRINK_CAP: usize = 10;

/// The whole walk of each case for each seed (not only the prefixes that reach a witness), replayed on threads. A divergence is
/// shrunk and reported with the case and seed.
fn s1c(seeds: std::ops::RangeInclusive<u64>, include_seeded: bool) {
    let mut divergences = Vec::new();
    let mut replayed = 0;
    for (name, build) in cases::full_builders() {
        if !include_seeded && name == "seeded configuration" {
            continue;
        }
        for seed in seeds.clone() {
            let case = build(seed);
            let walked = dst::walk(&case.world, seed, cases::WALK_LIMIT);
            assert!(walked.failure.is_none(), "the world's own walk fails: {name} seed {seed}");
            replayed += 1;
            if let Err(why) = check(&case.world, &walked.trace) {
                // Only the first SHRINK_CAP divergences are shrunk (a shrink replays the run once per event); the others say so.
                let small = (divergences.len() < SHRINK_CAP).then(|| shrink(&case.world, &walked.trace));
                let shrunk = small.map_or_else(|| format!("not shrunk (only the first {SHRINK_CAP} divergences are)"), |small| format!("shrunk to {} events: {small:?}", small.len()));
                divergences.push(format!("{name} seed {seed}: {why}\n {shrunk}"));
            }
        }
    }
    println!("s1c: {replayed} whole walks replayed, {} divergences", divergences.len());
    for d in &divergences {
        println!("{}", d.lines().next().unwrap_or(""));
    }
    assert!(divergences.is_empty(), "{} divergences:\n{}", divergences.len(), divergences.join("\n"));
}

#[test]
fn s1c_whole_walks_smoke() {
    s1c(1..=2, true);
}

#[test]
#[ignore = "whole-walk replays: EZSDR_SEEDS=a-b (default 1-300), run in release"]
fn s1c_whole_walks_wide() {
    let (first, last) = std::env::var("EZSDR_SEEDS").ok().and_then(|spec| spec.split_once('-').map(|(a, b)| (a.trim().parse().unwrap(), b.trim().parse().unwrap()))).unwrap_or((1, 300));
    s1c(first..=last, true);
}

/// Both wrappers record a timed value's acceptance where the call returns, with the same timestamp: the time source read at `S.unlock` (the
/// device's own application instant is the device's fact and stays apart). Time passes between the effect and the unlock; the value's
/// `accepted_at` is the tick of the unlock on both, and the thread run agrees with the world on the whole walk.
#[test]
fn acceptance_is_recorded_at_the_calls_return_in_both_wrappers() {
    use super::world::Event::{SCheck, SEffect, SLock, SResult, SSend, STake, SUnlock, Tick};
    let mut script = Script::new(with_command(&[]));
    script.ev(&[STake, SCheck, SLock, SEffect(Outcome::Ok), Tick, Tick, Tick]);
    assert_eq!(script.w.cells[0].in_flight[0].entry.accepted_at, None, "not yet: the call has not returned");
    script.ev(&[SUnlock]);
    assert_eq!(script.w.cells[0].in_flight[0].entry.accepted_at, Some(3), "the tick of the return, not the device's effective instant (8)");
    script.ev(&[SResult, SSend]);
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let on_threads = rt.sh.with_cell(0, |cell| cell.in_flight[0].entry.accepted_at);
    assert_eq!(on_threads, Some(3));
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

// ------------------------------------------------------------------------- the native lock (S8's building blocks)

/// The clock reader takes `L` for its time read: lock, read, unlock, then it sends the late command, which the brain takes.
#[test]
fn s8a_clock_read_takes_the_native_lock() {
    let mut w = cases::single(None, vec![EnvSlot { input: EnvInput::Late { stream: 0 }, earliest: 0, latest: 10, delivered: false, after: None }], params());
    w.cells[0] = cell(Phase::Streaming(seg(0)), 0, None, 0);
    let mut script = Script::new(w);
    script.ev(&[ClockLock(0), ClockRead, ClockUnlock, ClockSend, BStep]).ops().rest();
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    assert_eq!(rt.l_log()[..2], [LEvent::Acquire(Actor::Clock), LEvent::Release(Actor::Clock)], "the read is one acquisition and one release");
    assert_eq!(rt.observed().unwrap(), Observed::of_world(&script.w).unwrap());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// A timed write finds the radio queue full: S holds `L` and waits inside the device (blocked, not parked); the clock's read is
/// pending behind it; once the head leaves the queue the call returns and the read follows (acquisition order from the log).
#[test]
fn s8b_full_queue_blocks_apply_holding_l_and_the_clock_waits() {
    let mut w = with_command(&[]);
    w.env.push(EnvSlot { input: EnvInput::Late { stream: 0 }, earliest: 0, latest: 60, delivered: false, after: None });
    w.device.apply_timed(0, super::world::INITIAL_OCCUPANT, 40);
    let mut script = Script::new(w);
    script.ev(&[STake, SCheck, SLock]);
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    assert!(rt.sh.sched.is_blocked(Actor::Settings), "S waits for room in the queue: {}", rt.sh.sched.state_of(Actor::Settings));
    assert_eq!(rt.l_log(), vec![LEvent::Acquire(Actor::Settings)], "S holds L while it waits");
    let early = rt.replay(&[SEffect(Outcome::Ok)]).unwrap_err();
    assert!(early.contains("expected Settings at Call"), "{early}");
    // The clock's lock is pending behind S: it is blocked, not parked at a permit, so the trace cannot select it (E2-23's contention).
    let pending = Runtime::start(&script.w_initial(), true, None);
    pending.replay(&script.trace).unwrap();
    let err = pending.replay(&[ClockLock(0)]).unwrap_err();
    assert!(err.contains("expected Clock at Lock, found Some(Blocked"), "{err}");
    assert!(pending.sh.sched.is_blocked(Actor::Clock));
    assert_eq!(pending.l_log(), vec![LEvent::Acquire(Actor::Settings)], "the clock took no lock");
    assert!(pending.abort(JOIN).unwrap().is_empty());
    for _ in 0..40 {
        script.ev(&[Tick]);
    }
    script.ev(&[SEffect(Outcome::Ok), SUnlock, ClockLock(0), ClockRead]);
    let read = script.trace.len();
    // Time passes between the read and the send: the late command must carry the earlier instant.
    script.ev(&[const { Tick }; 3]);
    script.ev(&[ClockUnlock, ClockSend]);
    let sent = script.trace.len();
    script.rest();
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace[..read]).unwrap();
    let t = rt.model.now();
    rt.replay(&script.trace[read..sent]).unwrap();
    assert_ne!(rt.model.now(), t, "time passed between the read and the send");
    assert_eq!(rt.sh.inbox.items().last(), Some(&Input::LateCommand { stream: 0, now: t }), "the late command carries the time the clock read under L (T = {t})");
    rt.replay(&script.trace[sent..]).unwrap();
    let log = rt.l_log();
    assert_eq!(&log[..4], [LEvent::Acquire(Actor::Settings), LEvent::Release(Actor::Settings), LEvent::Acquire(Actor::Clock), LEvent::Release(Actor::Clock)], "{log:?}");
    assert_eq!(rt.observed().unwrap(), Observed::of_world(&script.w).unwrap());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

#[test]
fn outcome_of_decodes_the_model_convention() {
    use super::model_device::{PARTIAL, REFUSED};
    use crate::device::{Applied, DeviceError};
    let ok: Result<Applied, DeviceError> = Ok(Applied { rate: 0.0, freq: 0.0, gain: 0.0, antenna: None });
    assert_eq!(super::threads::outcome_of(&ok), Outcome::Ok);
    assert_eq!(super::threads::outcome_of(&Err(DeviceError { lost: true, message: String::new() })), Outcome::Lost);
    assert_eq!(super::threads::outcome_of(&Err(DeviceError::failed(PARTIAL))), Outcome::Partial);
    assert_eq!(super::threads::outcome_of(&Err(DeviceError::failed(REFUSED))), Outcome::Refused);
    assert_eq!(super::threads::outcome_of(&Err(DeviceError::failed("fake: control operation timed out"))), Outcome::Refused, "an unmarked error is a refusal");
}

/// S2b: the settings executor makes its device call with no cell guard held: a complete settings command on the checking device
/// (every thread panic, the I10 oracle's included, fails the replay).
#[test]
fn s2b_s_executor_never_holds_a_cell_over_apply() {
    let mut script = Script::new(with_command(&[]));
    script.ev(&[STake, SCheck, SLock, SEffect(Outcome::Ok), SUnlock, SResult, SSend]);
    check(&script.w_initial(), &script.trace).expect("no cell guard is held over a device call");
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    assert!(rt.checking.calls().iter().any(|(name, _)| *name == "apply"), "the call was made on the checking device");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// S4c: a forced fault on a close is a failed call, not an ok one: the streamer stays open and the call is not logged; the
/// brain-side outcome equals the world's (`RFault` gives `Failed`).
#[test]
fn s4c_fault_on_close_is_a_failed_call() {
    let mut w = cases::single(None, Vec::new(), params());
    w.params.call_faults = true;
    w.cells[0] = cell(Phase::Off, 0, None, 0);
    w.rx[0].fifo.push_back(RxOp::Close);
    let mut script = Script::new(w);
    script.ev(&[RCheck(0), RCmd(0), RGate(0), RLock(0), RFault(0), RUnlock(0), RResult(0)]).rest();
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let observed = rt.observed().unwrap();
    assert!(!observed.calls.iter().any(|c| matches!(c, DevCall::Close { .. })), "a faulted close is not made: {:?}", observed.calls);
    assert!(rt.model.snapshot().0.streamers.contains_key(&0), "the streamer stays open");
    assert_eq!(observed, Observed::of_world(&script.w).unwrap());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// Sol E2a-1 P1: a command routed while `S` is cleaning up after `F` is not lost. `S` abandons what it took out of its FIFO in
/// one step; a command routed after that goes to the discarder and ends `Abandoned` with its `Done`.
#[test]
fn s3c_command_routed_during_failure_cleanup_is_abandoned_not_lost() {
    let mut script = Script::new(with_command(&[]));
    script.ev(&[STake]);
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    rt.sh.failure.store(true, std::sync::atomic::Ordering::SeqCst);
    rt.sh.sched.notify();
    *rt.sh.after_batch.lock().unwrap() = Some(std::sync::Arc::new(|sh: &super::shared::Shared| {
        sh.push(&sh.s_fifo, SetCmd { id: 2, change: Change::Restore { channel: 0 } });
    }));
    rt.replay(&[SCheck, SDiscard]).expect("the command routed during the cleanup reaches the discarder");
    let terminals = rt.sh.sink().terminals.clone();
    assert_eq!(terminals, vec![(1, Terminal::Abandoned), (2, Terminal::Abandoned)]);
    let dones: Vec<_> = rt.sh.inbox.items().into_iter().filter_map(|i| if let Input::Done { id, terminal, .. } = i { Some((id, terminal)) } else { None }).collect();
    assert_eq!(dones, vec![(1, Terminal::Abandoned), (2, Terminal::Abandoned)], "each abandoned command sends its Done");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// Sol E2a-1 P4: the brain issues the new epoch's Start (and any Open or Close) only after it has processed the quiesce ack. The
/// events are a literal list (recorded once from the unmutated world), so a mutant of the brain changes only what the threaded
/// runtime does with them: the brain's operations after a step are drained however many it queued, and the assertions read what the
/// brain routed (its sends), not what the device did. The first assertion is before any acknowledgement exists.
fn drain_brain(rt: &Runtime) {
    while !rt.sh.brain().ops.is_empty() {
        rt.replay(&[BOp]).unwrap();
    }
}

#[test]
fn s10b_no_new_epoch_command_is_routed_before_the_quiesce_ack() {
    let world = plan_change_world();
    let rt = Runtime::start(&world, true, None);
    let new_epoch = |routed: &[Command]| routed.iter().any(|c| matches!(c, Command::Rx { stream: 0, op: RxOp::Start { epoch: 2, .. } | RxOp::Open | RxOp::Close }));
    // R is inside its recv; Stop is processed (three operations); then the plan change's step.
    rt.replay(&[RCheck(0), RSnap(0), RBegin(0), BStep, BOp, BOp, BOp, BStep]).unwrap();
    drain_brain(&rt);
    assert!(!new_epoch(&rt.sh.sink().routed), "nothing for the new epoch right after the plan change, before any ack: {:?}", rt.sh.sink().routed);
    // R publishes the old epoch's buffer, takes the Plan command, stops, and sends `Finished` and `Quiesced`.
    rt.replay(&[REnd(0), RPublish(0), RSnap(0), RPublish(0), RCheck(0), RCmd(0), RLock(0), RCall(0), RUnlock(0), RResult(0), RSend(0), RSend(0)]).unwrap();
    assert!(!new_epoch(&rt.sh.sink().routed), "nothing for the new epoch before the ack is taken: {:?}", rt.sh.sink().routed);
    rt.replay(&[BStep]).unwrap();
    drain_brain(&rt);
    assert!(!new_epoch(&rt.sh.sink().routed), "the old epoch's Finished alone does not release the new epoch: {:?}", rt.sh.sink().routed);
    rt.replay(&[BStep]).unwrap();
    drain_brain(&rt);
    assert!(new_epoch(&rt.sh.sink().routed), "after the ack the brain routes the new epoch's Start: {:?}", rt.sh.sink().routed);
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

// ----------------------------------------------------------------------- the panic guard itself (Sol E2a-1 re-review A, D)

/// A forced run of the brain's panic guard: `Input::Panic` is delivered, and brain operations numbered in `panics` panic inside their
/// cell section. `events` are the brain's steps and operations the trace grants.
fn guard_run(panics: &[usize], events: &[Event]) -> Runtime {
    let mut w = streaming(Some(16));
    w.inbox.push_back(Input::Panic);
    let rt = Runtime::start(&w, true, None);
    *rt.sh.panic_in_op.lock().unwrap() = panics.to_vec();
    rt.replay(events).unwrap_or_else(|why| panic!("panics at {panics:?}: {why}"));
    rt
}

fn assert_guard_ran(rt: &Runtime, what: &str) {
    assert!(rt.sh.brain().dead(), "{what}: the brain is dead");
    assert!(rt.sh.failed(), "{what}: F is set");
    assert!(rt.sh.gate.lock().unwrap().is_none(), "{what}: the brain released the gate it held");
    assert_eq!(rt.sh.sink().order, vec!["F", "mark"], "{what}: F before mark_lost, each once");
    let calls = rt.checking.calls();
    let mark = calls.iter().position(|(name, _)| *name == "mark_lost").expect("mark_lost was called");
    assert!(calls[mark].1, "{what}: F was set when mark_lost was called");
    assert!(rt.sh.sink().routed.contains(&Command::SettingsTerminate), "{what}: the executors' termination is routed");
    assert!(rt.sh.sched.is_exited(Actor::Brain), "{what}: the brain thread ended");
}

/// A panic at each brain operation, the guard's own included: the guard routine completes. Operation `n` of the panic's own
/// operations (gate 0, flag 1, ungate 2, mark 3) panics after its publication step, so the routine finds the gate held (n = 0, 1), `F`
/// set (n >= 1) or the mark made (n = 3), and does only what is left. `after` counts the brain events the routine then grants.
#[test]
fn s5c_a_panic_at_each_brain_operation_still_runs_the_guard() {
    for (n, after) in [(0usize, 3usize), (1, 2), (2, 1), (3, 0)] {
        let mut events = vec![BStep];
        events.extend(std::iter::repeat_n(BOp, n + 1 + after));
        let rt = guard_run(&[n], &events);
        assert_guard_ran(&rt, &format!("panic at the guard's operation {n}"));
        assert!(rt.abort(JOIN).unwrap().iter().all(|p| p.contains("injected")));
    }
    // A panic in an ordinary operation, before the guard has started: the routine does all four steps.
    let mut w = streaming(Some(16));
    w.inbox.push_back(Input::Release(held(0, value(0, 8, 1), 1)));
    let rt = Runtime::start(&w, true, None);
    *rt.sh.panic_in_op.lock().unwrap() = vec![0];
    let mut events = vec![BStep];
    events.extend(std::iter::repeat_n(BOp, 1 + 4));
    rt.replay(&events).unwrap();
    assert_guard_ran(&rt, "panic in the reservation, before the guard starts");
    assert!(rt.abort(JOIN).unwrap().iter().all(|p| p.contains("injected")));
}

/// The guard is not driven by the brain's operations: panics injected into every operation number cannot reach it.
#[test]
fn s5d_injected_panics_cannot_reach_the_guard() {
    let everywhere: Vec<usize> = (0..50).collect();
    let rt = guard_run(&everywhere, &[BStep, BOp, BOp, BOp, BOp]);
    assert_guard_ran(&rt, "a panic at every operation number");
    assert!(rt.abort(JOIN).unwrap().iter().all(|p| p.contains("injected")));
}

/// I14 with the guard running (Sol E2a-1 re-review 2): `G` is held from before `F` is set until after it. R wants the gate for a
/// close while the guard runs: it waits (pending, not an event), and when it gets the gate it sees `F` and makes no close.
#[test]
fn s5e_a_lifecycle_call_waiting_for_the_gate_sees_the_failure() {
    let mut w = cases::single(None, Vec::new(), params());
    w.cells[0] = cell(Phase::Off, 0, None, 0);
    w.rx[0].fifo.push_back(RxOp::Close);
    w.inbox.push_back(Input::Panic);
    let rt = Runtime::start(&w, true, None);
    *rt.sh.panic_in_op.lock().unwrap() = vec![0];
    // R is at its gate; the guard's first operation takes `G` and then panics.
    rt.replay(&[RCheck(0), RCmd(0), BStep, BOp]).unwrap();
    assert!(rt.sh.sched.is_blocked(Actor::Rx(0)), "R waits for the gate the guard holds");
    rt.replay(&[BOp]).unwrap();
    assert!(rt.sh.failed(), "F is set");
    assert_eq!(*rt.sh.gate.lock().unwrap(), Some(Actor::Brain), "G is still held when F is published");
    assert!(rt.sh.sched.is_blocked(Actor::Rx(0)), "R still waits");
    rt.replay(&[BOp]).unwrap();
    rt.replay(&[RGate(0)]).unwrap();
    let observed = rt.observed().unwrap();
    assert!(!observed.calls.iter().any(|c| matches!(c, DevCall::Close { .. })), "no close after F: {:?}", observed.calls);
    assert!(rt.model.snapshot().0.streamers.contains_key(&0), "the handle is retained");
    assert!(rt.abort(JOIN).unwrap().iter().all(|p| p.contains("injected")));
}

/// The same race in a free run, many times: whatever the interleaving, no close is made once `F` is set.
#[test]
fn s5f_free_run_no_close_after_the_failure() {
    for round in 0..60 {
        let mut w = cases::single(None, Vec::new(), params());
        w.cells[0] = cell(Phase::Off, 0, None, 0);
        w.rx[0].fifo.push_back(RxOp::Close);
        let rt = Runtime::start(&w, false, None);
        *rt.sh.panic_in_op.lock().unwrap() = vec![0];
        rt.sh.send(Input::Panic);
        let started = std::time::Instant::now();
        while !(rt.sh.sched.is_exited(Actor::Brain) && rt.sh.sched.is_exited(Actor::Rx(0))) && started.elapsed() < Duration::from_secs(3) {
            rt.model.advance(1);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(rt.sh.sched.is_exited(Actor::Brain) && rt.sh.sched.is_exited(Actor::Rx(0)), "round {round}: brain and R ended");
        let calls = rt.checking.calls();
        assert!(!calls.iter().any(|(name, failed)| *failed && matches!(*name, "rx_close" | "rx_open")), "round {round}: a handle changed after F: {calls:?}");
        rt.stop(Duration::from_secs(1)).unwrap();
    }
}

/// Sol E2a-1 re-review C, through the runtime: a publish computes from a view in which a finished value has no acceptance yet (so
/// the samples consume nothing); `S` records the acceptance (which does not bump the cell's revision) before the write. The write
/// must notice, recompute, and publish the consumption the acceptance qualifies; a runtime that wrote without `unchanged_since`
/// would publish without it.
#[test]
fn publish_recomputes_when_acceptance_lands_between_compute_and_write() {
    let mut w = streaming(Some(16));
    push(&mut w.cells[0], value(0, 8, 1), ValueState::Done(Terminal::Call(Outcome::Ok)));
    w.monitor.reserved.insert(1, (0, seg(0), 1));
    w.device.t = 12;
    let streamer = w.device.streamers.get_mut(&0).expect("stream 0 is open");
    streamer.next = Some(0);
    streamer.buffered = std::collections::VecDeque::from([8, 9, 10, 11]);
    let rt = Runtime::start(&w, true, None);
    rt.replay(&[RCheck(0), RSnap(0), RBegin(0), REnd(0)]).unwrap();
    *rt.sh.before_write.lock().unwrap() = Some(std::sync::Arc::new(|sh: &super::shared::Shared| {
        // The hook runs once: it is the acceptance landing, as S's `accept_all` would record it.
        if sh.before_write.lock().unwrap().take().is_some() {
            sh.with_cells(|cells| super::rules::accept_all(cells, 1, 8));
        }
    }));
    rt.replay(&[RPublish(0)]).unwrap();
    rt.replay(&[RSend(0)]).expect("the recomputed round sends the consumption boundary");
    let observed = rt.observed().unwrap();
    assert_eq!(observed.evidence.len(), 1, "the acceptance that landed before the write qualifies the sample: {:?}", observed.evidence);
    assert_eq!((observed.evidence[0].value, observed.evidence[0].sample), (1, 8));
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// Seeds 346, 1077, 1847 and 1855 of the seeded configuration (wide whole walks): a loss lands between a Move's call and its
/// unlock. The call's own result is the outcome (Ok here; Lost when the loss came before the call); a device read after the call
/// that the protocol does not have would turn the first into Lost.
#[test]
fn s3d_a_move_takes_its_outcome_from_its_own_call_not_from_a_later_probe() {
    for (name, before) in [("loss after the call", false), ("loss before the call", true)] {
        let env = vec![EnvSlot { input: EnvInput::Loss, earliest: 0, latest: 10, delivered: false, after: None }];
        let mut w = World::new(vec![cell(Phase::Streaming(seg(0)), 0, None, 0), cell(Phase::Off, 1, None, 0)], env, params());
        cases::running(&mut w, 0, vec![0]);
        w.settings.fifo.push_back(SetCmd { id: 1, change: Change::Move { channel: 0, to: 1 } });
        let mut script = Script::new(w);
        if before {
            script.ev(&[STake, SCheck, Env(0), SLock, SEffect(Outcome::Lost), SUnlock]);
        } else {
            script.ev(&[STake, SCheck, SLock, SEffect(Outcome::Ok), Env(0), SUnlock]);
        }
        script.rest();
        check(&script.w_initial(), &script.trace).unwrap_or_else(|why| panic!("{name}: {why}"));
        let want = if before { Outcome::Lost } else { Outcome::Ok };
        assert_eq!(expected(&script.w_initial(), &script.trace).unwrap().terminals, vec![(1, Terminal::Call(want))], "{name}");
    }
}

/// A capacity timer replayed on threads reaches the brain as a capacity input: after the trace up to the first reservation that a capacity timer
/// enabled (W10d), the thread run's brain is the world's brain, bookkeeping included.
#[test]
fn b1e_a_forced_replay_of_a_capacity_timer_leaves_the_worlds_brain() {
    let found = (1..=300u64).find_map(|seed| {
        let case = cases::case10(Variant::Full);
        let walked = dst::walk(&case.world, seed, cases::WALK_LIMIT);
        let length = *cases::witness_prefixes(&case.world, &walked.trace).get("W10d")?;
        Some((case, walked.trace[..length].to_vec()))
    });
    let (case, trace) = found.expect("a walk reaches W10d");
    let mut w = case.world.clone();
    for event in &trace {
        w.apply(event);
    }
    let rt = Runtime::start(&case.world, true, None);
    rt.replay(&trace).unwrap();
    let brain = rt.sh.brain();
    assert_eq!((brain.capacity_timer, brain.now, brain.held.len()), (w.brain.capacity_timer, w.brain.now, w.brain.held.len()));
    drop(brain);
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// The threads stamp a `Done` with the time source at its send, as the world does: two ticks in, the same `now`.
#[test]
fn b2e_the_threads_done_carries_the_time_source() {
    use super::world::Event::{SCheck, SEffect, SLock, SResult, SSend, STake, SUnlock, Tick};
    let mut script = Script::new(with_command(&[]));
    script.ev(&[Tick, Tick, STake, SCheck, SLock, SEffect(Outcome::Ok), SUnlock, SResult, SSend]);
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let on_threads = rt.sh.inbox.items();
    assert!(matches!(on_threads.last(), Some(Input::Done { id: 1, now: 2, .. })), "{on_threads:?}");
    assert_eq!(on_threads.last(), script.w.inbox.back());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// B15 (45-experiment §11): a missed start whose recv publishes a cut that is already reached sends the publication's `CutReached` first and the
/// `LateCommand` second, on both wrappers (one function, `rules::recv_inputs`). The brain then stops the stream once; with the late command
/// first it would end the stream at once and stop again. The thread run replays the world's whole walk and must agree on every call.
#[test]
fn b15_a_missed_start_after_the_cut_sends_the_boundaries_first_on_both_wrappers() {
    let mut w = cases::single(Some(8), Vec::new(), params());
    w.cells[0].progress = 8;
    w.device.streamers.get_mut(&0).expect("the stream is open").late = Some(0);
    w.device.t = 4;
    let mut script = Script::new(w);
    script.ev(&[RCheck(0), RSnap(0), RBegin(0), REnd(0), RPublish(0)]);
    let sends: Vec<_> = script.w.rx[0].program.iter().filter_map(|step| if let super::world::RStep::Send(input) = step { Some(input.clone()) } else { None }).collect();
    assert_eq!(sends, vec![Input::Boundary(super::core::Boundary::CutReached { stream: 0 }), Input::LateCommand { stream: 0, now: 4 }], "the world's order");
    script.rest();
    let stops = script.w.calls.iter().filter(|call| matches!(call, super::rules::DevCall::Stop { .. })).count();
    assert_eq!(stops, 1, "stopped once, by the cut: {:?}", script.w.calls);
    check(&script.w_initial(), &script.trace).expect("the thread run sends in the same order and agrees on every call");
}
