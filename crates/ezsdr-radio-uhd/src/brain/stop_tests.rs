//! The sequences of E4a2 (45-experiment §11, B11-B14) at the level of the world: the loss record's order, the provider's cleanup and the
//! destructor, the join limit and the detach, the spawn failure. The oracles I18-I21 judge every step of each run; the tests name what each
//! sequence must end in. Test plan `test-plan-e4a.md` §16.

use super::cases::{self, Case13, Case15, Variant};
use super::cell::Phase;
use super::core::{Disposition, Input, Reason};
use super::dst::{self, Model};
use super::world::{Cleanup, Event, PubStep, World};

/// Takes enabled events, non-ticks first, until none is enabled, with every oracle checked at each step.
fn run_checked(w: &mut World, limit: usize) {
    for _ in 0..limit {
        let enabled = w.enabled();
        let Some(event) = enabled.iter().find(|e| **e != Event::Tick).or(enabled.first()).cloned() else { return };
        step_checked(w, &event);
    }
    panic!("no terminal state");
}

fn step_checked(w: &mut World, event: &Event) {
    let before = w.clone();
    w.apply(event);
    before.transition(event, w).unwrap_or_else(|why| panic!("{event:?}: {why}"));
    w.safety().unwrap_or_else(|why| panic!("{event:?}: {why}"));
}

/// The first walk of `build` over seeds 1..=300 whose end state satisfies `wanted`; its end state.
fn walked_end(build: impl Fn() -> cases::Case, wanted: impl Fn(&World) -> bool) -> World {
    for seed in 1..=300 {
        let case = build();
        let walked = dst::walk(&case.world, seed, cases::WALK_LIMIT);
        assert!(walked.failure.is_none(), "seed {seed}: {:?}", walked.failure);
        let mut end = case.world.clone();
        for event in &walked.trace {
            end.apply(event);
        }
        if wanted(&end) {
            return end;
        }
    }
    panic!("no walk over 300 seeds ends as wanted");
}

/// B11: the loss publication of a run is the gate, `F`, the ungate, the record, the mark (I18 checked at each step); the record is a step of its own.
#[test]
fn b11_the_loss_record_is_a_step_after_f_and_before_the_mark() {
    let end = walked_end(|| cases::case13(Variant::Full, Case13::Loss), |w| w.monitor.marked);
    let order: Vec<&Event> = end.trace.iter().filter(|e| matches!(e, Event::Pub(_, PubStep::Set | PubStep::Record | PubStep::Mark))).collect();
    let position = |step: PubStep| order.iter().position(|e| matches!(e, Event::Pub(_, s) if *s == step)).expect("the step ran");
    assert!(position(PubStep::Set) < position(PubStep::Record) && position(PubStep::Record) < position(PubStep::Mark), "{order:?}");
}

/// B12 (no loss, no detach): every opened handle is closed and then destroyed once, the destructor having found the native mark clear.
#[test]
fn b12_a_clean_run_closes_and_destroys_every_handle_once() {
    let mut w = cases::case13(Variant::Micro, Case13::Clean).world;
    run_checked(&mut w, 100_000);
    let td = w.teardown.as_ref().expect("the stop model");
    assert_eq!(td.decision, Some(Cleanup::Close));
    assert_eq!((td.closed.len(), td.destroyed.len(), td.retained.len(), w.rx[0].handle), (1, 1, 0, None), "{td:?}");
}

/// B12 (loss): cleanup reads `F`: it decides to retain, closes nothing and destroys nothing.
#[test]
fn b12_after_a_loss_cleanup_retains_the_handles() {
    let end = walked_end(|| cases::case13(Variant::Full, Case13::Loss), |w| w.failure && w.teardown.as_ref().is_some_and(|td| td.detached.is_empty() && td.decision.is_some()));
    let td = end.teardown.as_ref().unwrap();
    assert_eq!(td.decision, Some(Cleanup::Retain), "F is set and nothing is detached: the loss alone decides");
    assert!(td.closed.is_empty() && td.destroyed.is_empty() && end.rx[0].handle.is_some(), "{td:?}");
}

/// B12 (a brain panic): the guard publishes `F` and the native mark; cleanup then retains the handle (it reads `F`) and closes nothing.
#[test]
fn b12_a_brain_panic_publishes_f_and_the_mark_and_cleanup_retains_the_handle() {
    let end = walked_end(|| cases::case13(Variant::Full, Case13::Panic(cases::case13_last_step(Variant::Full))), |w| w.brain.dead() && w.teardown.as_ref().is_some_and(|td| td.decision.is_some()));
    let td = end.teardown.as_ref().unwrap();
    assert!(end.failure && end.monitor.marked, "the guard set F and called mark_lost");
    assert_eq!(td.decision, Some(Cleanup::Retain));
    assert!(td.closed.is_empty() && td.destroyed.is_empty() && end.rx[0].handle.is_some(), "{td:?}");
}

/// B12 (the reachable missing-mark regression): the publisher that forgets `mark_lost` (the injected fault `omit_mark`; a panic guard without the
/// mark) leaves the native mark clear while `F` is set; cleanup reads `F` and retains, but the native device's final release reads only the mark
/// and frees the handle: the DST finds the I19 violation. (The test below that injects `F` and the mark by hand is destructor-unit coverage, not this
/// lifecycle sequence.)
#[test]
fn b12_a_missing_native_mark_is_found_when_the_native_owner_is_released() {
    let case = cases::case13(Variant::Micro, Case13::Panic(cases::case13_last_step(Variant::Micro)));
    let mut world = case.world.clone();
    world.omit_mark = true;
    let verdict = dst::explore(&case.config, world);
    match &verdict.outcome {
        dst::Outcome::Violation { failure, trace } => {
            assert!(failure.message.contains("I19") && failure.message.contains("final release"), "{}", failure.message);
            assert!(trace.contains(&Event::OwnerRelease), "{trace:?}");
        }
        other => panic!("the missing mark must be found: {}", verdict.report().lines().next().unwrap_or("").chars().take(200).collect::<String>() + &format!("{:?}", std::mem::discriminant(other))),
    }
}

/// B12, destructor-unit coverage (a loss injected between cleanup's check and the destructor; not a reachable lifecycle sequence: the reachable one is the missing-mark test above): cleanup decided to close with `F` clear; the loss then publishes `F` and the native
/// mark (the brain guard's two steps); the close drops the reference and the destructor, reading the mark, retains the handle. Without the mark
/// (the guard that forgot `mark_lost`) the destructor frees it, and I19 names the violation.
#[test]
fn b12_a_loss_after_the_check_and_before_the_destructor_retains_the_handle_by_the_mark() {
    let mut w = cases::case13(Variant::Micro, Case13::Clean).world;
    while w.teardown.as_ref().unwrap().decision != Some(Cleanup::Close) {
        let enabled = w.enabled();
        let event = enabled.iter().find(|e| **e != Event::Tick).or(enabled.first()).cloned().expect("an event");
        step_checked(&mut w, &event);
    }
    let missing_mark = {
        let mut m = w.clone();
        m.failure = true;
        m
    };
    w.failure = true;
    w.monitor.marked = true;
    run_checked(&mut w, 100_000);
    let td = w.teardown.as_ref().unwrap();
    assert_eq!((td.destroyed.len(), td.retained.len()), (0, 1), "the mark retains the handle: {td:?}");
    assert!(w.monitor.witnesses.contains("W13c"));
    // The missing-mark path: the same loss without the mark; the destructor frees the handle while F is set.
    let mut m = missing_mark;
    let violation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_checked(&mut m, 100_000)));
    let message = violation.expect_err("I19 must fire").downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("I19"), "{message}");
}

/// B13: a detached executor makes cleanup retain the handles and close nothing, and the executor, still running, ends later.
#[test]
fn b13_a_detached_executor_makes_cleanup_retain_and_the_executor_ends_later() {
    let end = walked_end(|| cases::case14(Variant::Full), |w| w.teardown.as_ref().is_some_and(|td| !td.detached.is_empty()));
    let td = end.teardown.as_ref().unwrap();
    assert_eq!(td.decision, Some(Cleanup::Retain));
    assert!(td.closed.is_empty(), "cleanup closed nothing: {td:?}");
    assert!(end.settings.exited, "the detached S went on and ended once the queue drained");
    // The native device is released only once no thread holds it; the mark is clear, so that release frees the streamer then (not at cleanup).
    assert!(td.released && td.destroyed.len() == 1 && td.retained.is_empty(), "{td:?}");
    assert!(end.teardown.as_ref().unwrap().detached.iter().all(|i| end.joinee_done(*i)), "every detached thread had ended before the release");
}

/// B14: the first spawn fails: the later spawn is suppressed, the runtime tells the brain of every executor not spawned and that the control
/// thread was never spawned, the failure itself is the failed start's shutdown (the stop begins, no Action is admitted afterwards), and no Action
/// reaches the brain before the start has finished.
#[test]
fn b14_a_failed_spawn_suppresses_the_later_spawns_and_actions_wait_for_the_start() {
    let mut w = cases::case15(Variant::Micro, Case15::First).world;
    assert!(w.enabled().contains(&Event::Spawn(0)) && !w.enabled().contains(&Event::Spawn(1)) && !w.enabled().iter().any(|e| matches!(e, Event::Env(_))), "{:?}", w.enabled());
    step_checked(&mut w, &Event::Spawn(0));
    assert!(!w.enabled().iter().any(|e| matches!(e, Event::Spawn(_))), "the later spawn is suppressed");
    let told: Vec<_> = w.inbox.iter().cloned().collect();
    assert_eq!(told, vec![Input::SpawnFailed { stream: 0, started: true }, Input::SpawnFailed { stream: 1, started: false }, Input::ControlUnspawned, Input::Shutdown]);
    assert!(w.env.iter().all(|slot| slot.delivered) && w.monitor.released.is_empty(), "no scripted Action is admitted after the failure");
    assert!(w.teardown.as_ref().unwrap().began, "the stopper begins with the failed start's shutdown");
    assert!(!w.enabled().iter().any(|e| matches!(e, Event::Env(_))));
}

/// B14 (after the start's `rx_start`): only the failing stream's chain is abandoned, the brain sends it nothing and waits for nothing from it, the
/// control thread is never spawned and gets nothing, and the executor that was spawned stops (the failed start's abort): its stream is stopped and it
/// exits.
#[test]
fn b14_only_a_started_failing_chain_is_abandoned_and_the_rest_ends_normally() {
    let mut w = cases::case15(Variant::Micro, Case15::AfterStart).world;
    run_checked(&mut w, 100_000);
    assert_eq!((w.cells[1].phase, w.brain.unspawned.iter().copied().collect::<Vec<_>>()), (Phase::Uncertain, vec![1]));
    assert!(w.rx[1].fifo.is_empty() && !w.rx[1].exited, "R1 was sent nothing and never ran");
    assert!(w.rx[0].exited && w.cells[0].phase != Phase::Uncertain, "R0 stopped in the normal order: {:?}", w.cells[0].phase);
    assert!(w.settings.unspawned && w.settings.fifo.is_empty() && !w.settings.terminate && w.brain.control_unspawned, "S does not exist and was sent nothing");
    let stopped = |stream: usize| w.calls.iter().any(|call| matches!(call, super::rules::DevCall::Stop { stream: s } if *s == stream));
    assert!(stopped(0) && !stopped(1), "the spawned executor stopped its stream; the unspawned one's was not touched: {:?}", w.calls);
    assert_eq!(w.cells[0].phase, Phase::Off);
}

/// B14 (before the start): nothing is abandoned.
#[test]
fn b14_a_stream_whose_start_had_not_run_is_not_abandoned() {
    let mut w = cases::case15(Variant::Micro, Case15::BeforeStart).world;
    run_checked(&mut w, 100_000);
    assert_eq!(w.cells[1].phase, Phase::Off);
    assert!(w.rx[1].fifo.is_empty());
}

/// B14 (a prior loss): the failing chain stays `Lost`.
#[test]
fn b14_a_loss_the_start_published_stays_a_loss() {
    let mut w = cases::case15(Variant::Micro, Case15::PriorLoss).world;
    run_checked(&mut w, 100_000);
    assert_eq!(w.cells[1].phase, Phase::Lost);
    assert!(!w.rx[1].exited && w.rx[1].program.is_empty() && w.rx[1].fifo.is_empty(), "R1 never ran, although F is set");
}

// ------------------------------------------------------------------------------------------------------------ the thread runtime

use super::cell::Outcome;
use super::force::Actor;
use super::shared::Gate;
use super::thread_tests::{JOIN, Script, check, streaming};
use super::threads::Runtime;

/// B11 on threads: the same whole walk replays equal, and in the order the threads published, `F` comes before the record and the record before
/// the native mark.
#[test]
fn b11t_the_threads_publish_f_then_the_record_then_the_mark() {
    let case = cases::case2(Variant::Full);
    let (seed, trace) = (1..=300u64)
        .find_map(|seed| {
            let walked = dst::walk(&case.world, seed, cases::WALK_LIMIT);
            walked.trace.iter().any(|e| matches!(e, Event::Pub(_, PubStep::Record))).then_some((seed, walked.trace))
        })
        .expect("a walk publishes a loss");
    let rt = Runtime::start(&case.world, true, None);
    rt.replay(&trace).unwrap_or_else(|why| panic!("seed {seed}: {why}"));
    let order = rt.sh.sink().order.clone();
    let at = |name: &str| order.iter().position(|o| *o == name).unwrap_or_else(|| panic!("{name} in {order:?}"));
    assert!(at("F") < at("record") && at("record") < at("mark"), "{order:?}");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// The world run up to the moment every joinee has ended (the executors, the brain and `S`), without the provider events: the trace the thread
/// runtime replays before the stopper's joins and the cleanup.
fn drive(world: &World) -> (World, Vec<Event>) {
    let mut w = world.clone();
    let mut trace = Vec::new();
    for _ in 0..100_000 {
        if (0..w.joinees()).all(|i| w.joinee_done(i)) {
            return (w, trace);
        }
        let enabled = w.enabled();
        let provider = |e: &Event| matches!(e, Event::Spawn(_) | Event::Join | Event::Detach | Event::CleanupDecide | Event::CleanupClose(_) | Event::Destroy(_) | Event::OwnerRelease);
        let event = enabled.iter().find(|e| **e != Event::Tick && !provider(e)).or_else(|| enabled.iter().find(|e| **e == Event::Tick)).cloned().expect("an event");
        w.apply(&event);
        trace.push(event);
    }
    panic!("the joinees never ended");
}

/// The thread runtime after the same run, with the stopper's joins done in the provider's order (`S`, each receive executor, the brain): every
/// joinee has ended on threads too.
fn joined_runtime(world: &World) -> Runtime {
    let (_, trace) = drive(world);
    let rt = Runtime::start(world, true, None);
    rt.replay(&trace).unwrap_or_else(|why| panic!("{why}"));
    for actor in [Actor::Settings, Actor::Rx(0), Actor::Brain] {
        assert!(rt.join_within(actor, 0).unwrap(), "{actor:?} ended before the join");
    }
    rt
}

/// B12 on threads, in the provider's order: shutdown, the executors end, the brain thread returns, the joins, then cleanup and the final owner
/// release. A clean run closes and destroys the streamer once; after a loss cleanup leaves it and the owner release keeps it (the mark is set).
#[test]
fn b12t_the_stop_joins_every_thread_then_cleans_up_and_the_owner_release_reads_the_mark() {
    let rt = joined_runtime(&cases::case13(Variant::Micro, Case13::Clean).world);
    assert_eq!(rt.cleanup(false), Cleanup::Close);
    assert_eq!(rt.destructors(), (vec![0], vec![]), "closed and destroyed once");
    rt.release_owner();
    assert_eq!(rt.destructors(), (vec![0], vec![]), "nothing is left for the owner");
    assert!(rt.abort(JOIN).unwrap().is_empty());

    let world = {
        let mut w = cases::case13(Variant::Micro, Case13::Loss).world;
        // The loss arrives in the window; the run is the world's.
        w.teardown.as_mut().unwrap().join_limit = 4;
        w
    };
    let rt = joined_runtime(&world);
    assert!(rt.sh.failed(), "the loss set F");
    assert_eq!(rt.cleanup(false), Cleanup::Retain, "cleanup reads F");
    assert_eq!(rt.destructors(), (vec![], vec![]));
    rt.release_owner();
    assert_eq!(rt.destructors(), (vec![], vec![0]), "the native mark (published by the loss) keeps the streamer");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// B12 on threads (the destructor unit, not a lifecycle sequence): the native owner's release reads the mark and not `F`: with `F` set and no mark
/// it frees the streamer; with the mark it keeps it.
#[test]
fn b12t_the_native_destructor_alone_reads_the_mark_and_not_f() {
    let world = streaming(Some(16));
    let rt = Runtime::start(&world, true, None);
    rt.sh.failure.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(rt.cleanup(false), Cleanup::Retain);
    rt.release_owner();
    assert_eq!(rt.destructors(), (vec![0], vec![]), "no mark: freed (the hazard I19 names)");
    assert!(rt.abort(JOIN).unwrap().is_empty());

    let rt = Runtime::start(&world, true, None);
    rt.sh.failure.store(true, std::sync::atomic::Ordering::SeqCst);
    rt.sh.device.mark_lost();
    rt.release_owner();
    assert_eq!(rt.destructors(), (vec![], vec![0]), "the mark keeps it");
    rt.release_owner();
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// The brain thread's return on threads: it returns once it has terminated with nothing queued, and at once after its guard, and the join sees it.
#[test]
fn b12t_the_brain_thread_returns_after_termination_and_after_its_guard() {
    let clean = joined_runtime(&cases::case13(Variant::Micro, Case13::Clean).world);
    assert!(clean.sh.sink().exited.contains(&Actor::Brain));
    assert!(clean.abort(JOIN).unwrap().is_empty());
    let panic = cases::case13(Variant::Micro, Case13::Panic(cases::case13_last_step(Variant::Micro))).world;
    let rt = joined_runtime(&panic);
    assert!(rt.sh.sink().exited.contains(&Actor::Brain) && rt.sh.failed());
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// The brain thread returns only when nothing can still send to it: it has terminated, its inbox is empty, and `S` and every spawned receive
/// executor have ended with nothing left to send (found by the full-horizon sweep: an executor's loss boundary sent after the brain returned was
/// never served, and the value it would have disposed stayed undisposed, P2).
#[test]
fn b12_the_brain_thread_returns_only_when_nothing_can_still_send_to_it() {
    use super::core::Exit;
    use super::world::RStep;
    let mut w = cases::case13(Variant::Micro, Case13::Clean).world;
    w.brain.exit = Exit::Terminated;
    w.settings.exited = true;
    w.settings.terminate = true;
    w.rx[0].exited = true;
    assert!(w.enabled().contains(&Event::BExit), "terminated, empty, every sender ended");
    w.rx[0].program.push_back(RStep::Send(Input::Boundary(super::core::Boundary::Lost { stream: 0 })));
    assert!(!w.enabled().contains(&Event::BExit), "an executor still has a send to make");
    assert!(!w.joinee_done(1), "an executor whose loop ended but whose final send is pending has not ended");
    w.rx[0].program.clear();
    w.clock = Some(super::world::ClockHold { slot: 0, now: Some(0) });
    assert!(!w.enabled().contains(&Event::BExit), "the clock reader holds a serve it has not sent");
    w.clock = None;
    w.rx[0].exited = false;
    assert!(!w.enabled().contains(&Event::BExit), "an executor is still running");
    w.rx[0].exited = true;
    w.settings.exited = false;
    assert!(!w.enabled().contains(&Event::BExit), "S is still running");
    w.settings.exited = true;
    w.inbox.push_back(Input::Shutdown);
    assert!(!w.enabled().contains(&Event::BExit), "an input is queued");
    w.inbox.clear();
    w.env.push(super::world::EnvSlot { input: super::world::EnvInput::Late { stream: 0 }, earliest: 0, latest: 100, delivered: false, after: None });
    let lock = Event::ClockLock(w.env.len() - 1);
    w.brain.ops.clear();
    assert!(w.enabled().contains(&lock), "the clock reader may start a serve before the brain returned");
    w.brain_exited = true;
    assert!(!w.enabled().contains(&lock), "the clock reader has stopped once the brain returned");
}

/// B13 (the join limit in virtual time): the stopper detaches the joinee it waits for exactly when the limit has passed, joins it only once it has
/// ended, and cleanup waits for every joinee, the brain's thread included.
#[test]
fn b13_detach_waits_for_the_limit_join_for_the_end_and_cleanup_for_every_joinee() {
    let mut w = cases::case14(Variant::Micro).world;
    let limit = w.teardown.as_ref().unwrap().join_limit;
    // S has not ended; the stopper has been waiting for it since tick 10.
    {
        let td = w.teardown.as_mut().unwrap();
        td.began = true;
        td.next = 0;
        td.since = 10;
    }
    w.device.t = 10 + limit - 1;
    assert!(!w.enabled().contains(&Event::Detach) && !w.enabled().contains(&Event::Join), "before the limit: neither");
    w.device.t = 10 + limit;
    assert!(w.enabled().contains(&Event::Detach) && !w.enabled().contains(&Event::Join), "at the limit: detach, not join");
    w.settings.exited = true;
    w.settings.terminate = true;
    w.settings.fifo.clear();
    w.settings.program.clear();
    w.settings.hand = None;
    assert!(w.enabled().contains(&Event::Join) && !w.enabled().contains(&Event::Detach), "an ended joinee is joined, not detached");
    // The brain is the last joinee: its join waits for its thread to have returned, and cleanup for that join.
    w.teardown.as_mut().unwrap().next = w.rx.len() + 1;
    assert!(!w.brain_exited && !w.enabled().contains(&Event::Join), "the brain thread has not returned: no join");
    w.brain_exited = true;
    assert!(w.enabled().contains(&Event::Join) && !w.enabled().contains(&Event::CleanupDecide), "joined first, then cleanup");
    w.teardown.as_mut().unwrap().next = w.rx.len() + 2;
    assert!(w.enabled().contains(&Event::CleanupDecide));
}

/// B13 on threads: `S` blocked in its settings call holding `L` (the queue is full until tick 6) is not joined within the limit; the stopper
/// detaches it, cleanup leaves the streamers alone, and `S`, still running, finishes its call once the queue has room (late execution after the detach).
#[test]
fn b13t_a_blocked_settings_executor_is_detached_after_the_limit_and_finishes_later() {
    let case = cases::case14(Variant::Micro);
    let mut script = Script::new(case.world);
    script.ev(&[Event::Env(0), Event::BStep]).ops().ev(&[Event::STake, Event::SCheck, Event::SLock]);
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    let ended = rt.join_within(Actor::Settings, 2).unwrap();
    assert!(!ended, "S is blocked in the queue, holding L");
    assert_eq!(rt.cleanup(true), Cleanup::Retain);
    assert_eq!(rt.destructors(), (vec![], vec![]), "nothing closed or destroyed");
    // Time goes on: the queue has room at tick 6 and S finishes its call.
    rt.replay(&std::iter::repeat_n(Event::Tick, 4).collect::<Vec<_>>()).unwrap();
    rt.replay(&[Event::SEffect(Outcome::Ok), Event::SUnlock, Event::SResult, Event::SSend]).unwrap();
    assert!(rt.sh.sink().terminals.iter().any(|(id, _)| *id == 1), "the detached S wrote its result");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// B14 on threads: from the state after a failed start (stream 1's executor never spawned, the brain told), the thread runtime spawns no thread for
/// that stream, the brain routes it nothing, and the whole run replays equal to the world's.
#[test]
fn b14t_the_threads_spawn_no_executor_for_an_unspawned_stream_and_agree_with_the_world() {
    let mut world = cases::case15(Variant::Micro, Case15::AfterStart).world;
    step_checked(&mut world, &Event::Spawn(0));
    step_checked(&mut world, &Event::Spawn(1));
    world.startup = None;
    world.teardown = None;
    assert!(world.rx[1].unspawned && !world.rx[0].unspawned);
    let mut script = Script::new(world);
    script.rest();
    assert!(script.w.rx[1].fifo.is_empty() && script.w.rx[0].exited && script.w.cells[1].phase == Phase::Uncertain);
    check(&script.w_initial(), &script.trace).expect("the thread run agrees with the world");
    let rt = Runtime::start(&script.w_initial(), true, None);
    rt.replay(&script.trace).unwrap();
    assert!(rt.sh.r_fifo[1].items().is_empty() && !rt.sh.sink().exited.contains(&Actor::Rx(1)), "R1 was sent nothing and never ran");
    assert_eq!(rt.sh.sched.state_of(Actor::Rx(1)), "None", "no thread was spawned for R1");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// A release for a stream whose executor was never spawned is disposed at once, once (the brain's own rule: it holds nothing it can never reserve).
#[test]
fn b14_a_release_for_an_unspawned_stream_is_disposed_at_once() {
    use super::core::Brain;
    use super::fixtures::{cell, feed, held, seg, value};
    let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 0), cell(Phase::Off, 1, None, 0)];
    let mut brain = Brain::new();
    feed(&mut brain, &mut cells, Input::SpawnFailed { stream: 1, started: false });
    feed(&mut brain, &mut cells, Input::Release(held(1, value(1, 5, 7), 1)));
    assert_eq!((brain.disposition(7), brain.held.len()), (Some(Disposition::Reported(Reason::NoReservation)), 0));
}

/// The clean micro run stopped where the brain has terminated and queued every terminate, with no executor having ended: the state a free run
/// starts from to watch the executors end and the brain return on its own.
fn terminated_world(mode: Case13) -> World {
    let mut w = cases::case13(Variant::Micro, mode).world;
    for _ in 0..10_000 {
        if w.brain.exit == super::core::Exit::Terminated && w.brain.ops.is_empty() {
            assert!(!w.settings.exited && !w.rx[0].exited && w.rx[0].program.is_empty(), "the terminates are queued, nothing has ended");
            return w;
        }
        let enabled = w.enabled();
        let event = enabled.iter().find(|e| **e != Event::Tick && !matches!(e, Event::Spawn(_))).or(enabled.first()).cloned().expect("an event");
        w.apply(&event);
    }
    panic!("the brain never terminated");
}

fn poll(what: &str, f: impl Fn() -> bool) {
    let started = std::time::Instant::now();
    while !f() {
        assert!(started.elapsed() < JOIN, "gave up waiting for: {what}");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// P2 (Sol, second review) on free-running threads, no forced `BExit`: the brain has terminated and checked for senders before any executor
/// ended; the executors end afterwards without another input. The brain's thread still returns (sender completion wakes it and satisfies its
/// wait), within the join limit, so the joins succeed, nothing is detached and cleanup closes every handle.
#[test]
fn b12t_a_free_running_brain_returns_when_the_last_sender_ends_after_it_checked() {
    let rt = Runtime::start(&terminated_world(Case13::Clean), false, None);
    poll("the brain returns", || rt.sh.sink().exited.contains(&Actor::Brain));
    let exited = rt.sh.sink().exited.clone();
    assert!(exited.contains(&Actor::Settings) && exited.contains(&Actor::Rx(0)), "{exited:?}");
    assert!(!rt.sh.failed());
    assert_eq!(rt.cleanup(false), Cleanup::Close);
    assert_eq!(rt.destructors(), (vec![0], vec![]), "closed and destroyed once");
    assert!(rt.stop(JOIN).unwrap().is_empty());
}

/// A test's holds on a free run's gates: a thread that reaches a held gate waits there until it is released.
struct Holds {
    held: std::sync::Mutex<std::collections::BTreeSet<String>>,
    reached: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

impl Holds {
    fn new(held: &[Gate]) -> std::sync::Arc<Holds> {
        std::sync::Arc::new(Holds { held: std::sync::Mutex::new(held.iter().map(|g| format!("{g:?}")).collect()), reached: Default::default() })
    }
    fn hook(self: &std::sync::Arc<Holds>) -> super::shared::GateHook {
        let holds = self.clone();
        std::sync::Arc::new(move |_, gate| {
            let name = format!("{gate:?}");
            if holds.held.lock().unwrap().contains(&name) {
                holds.reached.lock().unwrap().insert(name.clone());
                while holds.held.lock().unwrap().contains(&name) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        })
    }
    fn reached(&self, gate: Gate) -> bool {
        self.reached.lock().unwrap().contains(&format!("{gate:?}"))
    }
    fn release(&self, gate: Gate) {
        self.held.lock().unwrap().remove(&format!("{gate:?}"));
    }
}

/// Waits until every actor of `who` has returned.
fn all_returned(rt: &Runtime, who: &[Actor]) {
    poll("the senders return", || who.iter().all(|a| rt.sh.sched.is_exited(*a)));
}

const SENDERS: [Actor; 4] = [Actor::Settings, Actor::Rx(0), Actor::Clock, Actor::Timer];

/// Regression (a), P2-1 of Sol's third review, on a targeted free run with the order controlled by gates: the brain is waiting (blocked) while the
/// last sender is still registered, the sender then deregisters, and its deregistration wakes the brain, which returns. Once with `S` as the last
/// sender (it has ended, parked at its return) and once with a receive executor. A wake that did not reach the brain's own condition leaves it asleep.
#[test]
fn b12t_a_targeted_brain_waiting_for_the_last_sender_wakes_when_it_deregisters() {
    for last in [Actor::Settings, Actor::Rx(0)] {
        let holds = Holds::new(&[Gate::Deregister(last)]);
        let rt = Runtime::start_held(&terminated_world(Case13::Clean), holds.hook());
        poll("the last sender is held before it deregisters", || holds.reached(Gate::Deregister(last)));
        let others: Vec<Actor> = SENDERS.iter().copied().filter(|a| *a != last).collect();
        all_returned(&rt, &others);
        poll("the brain is waiting", || rt.sh.sched.is_blocked(Actor::Brain));
        assert!(!rt.sh.sink().exited.contains(&Actor::Brain), "{last:?} is still registered");
        holds.release(Gate::Deregister(last));
        poll("the deregistration woke the brain and it returned", || rt.sh.sink().exited.contains(&Actor::Brain));
        assert!(rt.stop(JOIN).unwrap().is_empty());
    }
}

/// Regression (b), P2-2 of Sol's third review (hand-built state: the failure flag is set after the brain terminated, and an input is queued so the
/// brain takes a step): the brain's step is held before its exit test; meanwhile `R0` sends `Lost` and deregisters, and every other sender
/// returns. The exit test then finds the inbox not empty and the brain serves `Lost` before it returns, however the reads would interleave.
#[test]
fn b12t_an_executor_that_sends_lost_and_deregisters_during_the_brains_step_is_served_first() {
    let holds = Holds::new(&[Gate::ExitTest]);
    let mut failed = terminated_world(Case13::Clean);
    failed.failure = true;
    failed.inbox.push_back(Input::LateCommand { stream: 0, now: 0 });
    let rt = Runtime::start_held(&failed, holds.hook());
    poll("the brain's step is held before its exit test", || holds.reached(Gate::ExitTest));
    all_returned(&rt, &SENDERS);
    assert_ne!(rt.sh.with_cell(0, |c| c.phase), Phase::Lost, "Lost is queued, not served");
    assert!(rt.sh.inbox.nonempty() && !rt.sh.sink().exited.contains(&Actor::Brain));
    holds.release(Gate::ExitTest);
    poll("the brain returns", || rt.sh.sink().exited.contains(&Actor::Brain));
    assert_eq!(rt.sh.with_cell(0, |c| c.phase), Phase::Lost, "the Lost boundary was served before the brain returned");
    assert!(!rt.sh.inbox.nonempty());
    assert!(rt.stop(JOIN).unwrap().is_empty());
}

/// Regression (b'), P3 of the second review: a receive executor held before its final `Lost` send keeps the brain from returning; when it sends and
/// deregisters the brain serves `Lost` and returns (targeted free run).
#[test]
fn b12t_the_brain_does_not_return_before_an_executor_that_is_about_to_send_lost() {
    let holds = Holds::new(&[Gate::FinalSend(Actor::Rx(0))]);
    // hand-built state: the failure flag is set after the brain terminated, so the executor ends its loop and still has the `Lost` to send
    let mut failed = terminated_world(Case13::Clean);
    failed.failure = true;
    let rt = Runtime::start_held(&failed, holds.hook());
    poll("R0 is held before its final send", || holds.reached(Gate::FinalSend(Actor::Rx(0))));
    all_returned(&rt, &[Actor::Settings, Actor::Clock, Actor::Timer]);
    poll("the brain is waiting", || rt.sh.sched.is_blocked(Actor::Brain));
    assert!(!rt.sh.sink().exited.contains(&Actor::Brain), "the brain waits for the executor's send");
    assert_ne!(rt.sh.with_cell(0, |c| c.phase), Phase::Lost);
    holds.release(Gate::FinalSend(Actor::Rx(0)));
    poll("the brain returns", || rt.sh.sink().exited.contains(&Actor::Brain));
    assert_eq!(rt.sh.with_cell(0, |c| c.phase), Phase::Lost, "the Lost boundary was served before the brain returned");
    assert!(rt.stop(JOIN).unwrap().is_empty());
}

/// Regression (c), P2-3 of Sol's third review (hand-built: the clock reader holds a serve and the timer a removed due booking when the brain,
/// already terminated, tells them to stop): both finish the send they have begun, the brain serves both inputs, and only then returns; nothing is
/// stranded in its inbox.
#[test]
fn b12t_a_clock_read_and_a_due_booking_pending_at_termination_are_delivered_and_served() {
    use super::core::Timer;
    use super::shared::ClockCmd;
    let (clock, timer) = (Gate::FinalSend(Actor::Clock), Gate::FinalSend(Actor::Timer));
    let holds = Holds::new(&[Gate::BrainTop, clock, timer]);
    let rt = Runtime::start_held(&terminated_world(Case13::Clean), holds.hook());
    poll("the brain is at its loop top", || holds.reached(Gate::BrainTop));
    rt.sh.push(&rt.sh.k_fifo, ClockCmd::Serve { stream: 0 });
    rt.sh.timers.lock().unwrap().push((Timer::Capacity, 0));
    rt.sh.sched.notify();
    poll("the clock has read the time and the timer removed the booking", || holds.reached(clock) && holds.reached(timer));
    holds.release(Gate::BrainTop);
    all_returned(&rt, &[Actor::Settings, Actor::Rx(0)]);
    poll("the brain is waiting", || rt.sh.sched.is_blocked(Actor::Brain));
    assert!(!rt.sh.sink().exited.contains(&Actor::Brain), "the clock and the timer still hold a send");
    holds.release(clock);
    holds.release(timer);
    poll("the brain returns", || rt.sh.sink().exited.contains(&Actor::Brain));
    all_returned(&rt, &[Actor::Clock, Actor::Timer]);
    assert!(!rt.sh.inbox.nonempty(), "both inputs were served before the brain returned");
    assert!(rt.stop(JOIN).unwrap().is_empty());
}

/// Forced: a receive executor whose loop has exited but whose final `Lost` send is pending has not ended: the stopper's join waits for the send.
#[test]
fn b12t_the_join_of_an_executor_waits_for_its_final_send() {
    let mut w = cases::case13(Variant::Micro, Case13::Loss).world;
    let mut trace = Vec::new();
    while !(w.rx[0].exited && !w.rx[0].program.is_empty()) {
        let enabled = w.enabled();
        let event = enabled.iter().find(|e| **e != Event::Tick && !matches!(e, Event::Spawn(_))).or(enabled.first()).cloned().expect("an event");
        w.apply(&event);
        trace.push(event);
    }
    let rt = Runtime::start(&cases::case13(Variant::Micro, Case13::Loss).world, true, None);
    rt.replay(&trace).unwrap_or_else(|why| panic!("{why}"));
    assert!(!rt.join_within(Actor::Rx(0), 0).unwrap(), "the loop exited, the send is pending");
    rt.replay(&[Event::RSend(0)]).unwrap();
    assert!(rt.join_within(Actor::Rx(0), 0).unwrap(), "the final send is made");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}

/// Regression, P2-1 of Sol's fourth review (hand-built state: terminated, failure set, so `R0` ends its loop and sends a final `Lost`): every sender
/// is registered before the brain thread starts, so a zero count never means "not registered yet". The builder is slowed just before it registers its first
/// sender; a brain that had started by then would see an empty inbox and no sender, return, and leave `R0`'s `Lost` unserved.
#[test]
fn b12t_every_sender_is_registered_before_the_brain_thread_starts() {
    let slow: super::shared::GateHook = std::sync::Arc::new(|_, gate| {
        if gate == Gate::Registering(Actor::Settings) {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    });
    let mut failed = terminated_world(Case13::Clean);
    failed.failure = true;
    let rt = Runtime::start_held(&failed, slow);
    poll("the brain returns", || rt.sh.sink().exited.contains(&Actor::Brain));
    all_returned(&rt, &SENDERS);
    assert_eq!(rt.sh.with_cell(0, |c| c.phase), Phase::Lost, "R0's final Lost was served before the brain returned");
    assert!(!rt.sh.inbox.nonempty());
    assert!(rt.stop(JOIN).unwrap().is_empty());
}

/// Regression, P2-2 of Sol's fourth review (hand-built from Sol's sequence): `S`'s loop exited on a failure while a command routed afterwards is
/// still in its queue. `S` has not ended: the brain thread may not return and the stopper's join waits, until the discarder has taken the command
/// and its `Abandoned` `Done` has been served.
#[test]
fn b13_a_failed_s_has_not_ended_while_a_routed_command_awaits_its_discard() {
    use super::core::{Change, Exit, SetCmd};
    let mut w = cases::case13(Variant::Micro, Case13::Clean).world;
    w.brain.exit = Exit::Terminated;
    w.brain.ops.clear();
    w.settings.exited = true;
    w.settings.terminate = true;
    w.rx[0].exited = true;
    w.settings.fifo.push_back(SetCmd { id: 9, change: Change::Timed { channel: 0, freq: 1.0, at: 8, generation: 1 } });
    assert!(!w.joinee_done(0), "S's discarder still has a command");
    assert!(!w.enabled().contains(&Event::BExit), "the brain thread may not return before S has discarded the command");
    assert!(w.enabled().contains(&Event::SDiscard));
    w.apply(&Event::SDiscard);
    assert!(w.joinee_done(0), "the queue is empty, the terminate has arrived");
    assert!(!w.enabled().contains(&Event::BExit), "the abandoned Done is queued for the brain");
    assert!(matches!(w.inbox.front(), Some(Input::Done { id: 9, .. })));
}

/// Regression, P2 of Sol's fifth review, forced on threads (Sol's sequence): `S`'s loop exits on a failure at `SCheck`, then the brain routes another
/// command B. `S` has not completed: the stopper's join must not succeed while B is not abandoned and its `Done` not sent; it succeeds only when
/// the discarder has taken B and the terminate has arrived and `S` has returned.
#[test]
fn b13t_the_join_of_a_failed_s_waits_for_its_discarder() {
    use super::cell::Terminal;
    use super::core::{Change, SetCmd};
    let command = |id| SetCmd { id, change: Change::Restore { channel: 0 } };
    let mut w = cases::case13(Variant::Micro, Case13::Clean).world;
    w.failure = true;
    w.settings.fifo.push_back(command(1));
    let rt = Runtime::start(&w, true, None);
    rt.replay(&[Event::STake, Event::SCheck]).unwrap_or_else(|why| panic!("{why}"));
    assert!(rt.sh.sink().terminals.contains(&(1, Terminal::Abandoned)), "A was abandoned by the failed check");
    rt.sh.push(&rt.sh.s_fifo, command(2));
    assert!(!rt.join_within(Actor::Settings, 0).unwrap(), "S's loop exited but B awaits its discard");
    rt.replay(&[Event::SDiscard]).unwrap_or_else(|why| panic!("{why}"));
    assert!(rt.sh.sink().terminals.contains(&(2, Terminal::Abandoned)));
    assert!(!rt.join_within(Actor::Settings, 0).unwrap(), "the terminate has not arrived: the discarder still waits");
    rt.sh.s_terminate.store(true, std::sync::atomic::Ordering::SeqCst);
    rt.sh.sched.notify();
    rt.sh.sched.settle().unwrap();
    assert!(rt.join_within(Actor::Settings, 0).unwrap(), "terminate arrived, queue empty: S returned");
    assert!(rt.abort(JOIN).unwrap().is_empty());
}
