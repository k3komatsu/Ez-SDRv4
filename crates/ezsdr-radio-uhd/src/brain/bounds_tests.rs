//! The unit sequences of E4a1 (45-experiment §11, B1-B10): the two bounds, device time carried by inputs, the capacity timer and the
//! held re-evaluation, each a test on `Brain::step` and the cells with no harness. Test plan `test-plan-e4a.md` §7 and §13 name what
//! each observes and which wrong behaviour fails it.

use super::cell::{Cell, Drain, Outcome, Phase, Refusal, Terminal, ValueState};
use super::core::{Boundary, Brain, Change, Command, DEVICE_BOUND, Disposition, Input, NotReserved, Reason, Request, RxOp, SetCmd, device_load};
use super::fixtures::{cell, feed, held, push, seg, value};

fn brain(k: usize, d: usize) -> Brain {
    let mut brain = Brain::new();
    brain.device_bound = k;
    brain.chain_bound = d;
    brain
}

/// A streaming cell on `channel` with no cut.
fn streaming(channel: usize) -> Cell {
    cell(Phase::Streaming(seg(0)), channel, None, 0)
}

fn release(brain: &mut Brain, cells: &mut [Cell], stream: usize, id: u64, e: i64) {
    feed(brain, cells, Input::Release(held(stream, value(stream, e, id), 1)));
}

fn done(brain: &mut Brain, cells: &mut [Cell], id: u64, outcome: Outcome, now: i64) {
    feed(brain, cells, Input::Done { id, terminal: Terminal::Call(outcome), now });
}

/// The ids of the settings commands in the outbox, in order.
fn sent(brain: &Brain) -> Vec<u64> {
    brain.outbox.iter().filter_map(|command| if let Command::Settings(SetCmd { id, .. }) = command { Some(*id) } else { None }).collect()
}

fn timers(brain: &Brain) -> Vec<i64> {
    brain.outbox.iter().filter_map(|command| if let Command::CapacityTimer { at } = command { Some(*at) } else { None }).collect()
}

fn held_ids(brain: &Brain) -> Vec<u64> {
    brain.held.iter().map(|held| held.entry.order.1).collect()
}

fn resident(cell: &Cell) -> Vec<u64> {
    cell.in_flight.iter().map(|flight| flight.entry.order.1).collect()
}

/// The call of a reserved value goes through S: taken, then entered (maybe applied).
fn running(cell: &mut Cell, id: u64) {
    assert_eq!(cell.take(id), super::cell::Take::Call);
    cell.start_call(id);
}

#[test]
fn b1_stalled_r_device_time_frees_k_in_order() {
    let mut cells = vec![streaming(0)];
    let mut brain = Brain::new();
    for id in 1..=6u64 {
        release(&mut brain, &mut cells, 0, id, id as i64 * 10);
    }
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![1, 2, 3, 4], vec![5, 6]), "the first K are reserved, the rest wait in release order");
    assert_eq!(brain.reservation_refusal(5, &cells), Some(NotReserved::DeviceBound));
    for id in [1, 2] {
        assert_eq!(cells[0].take(id), super::cell::Take::Call);
    }
    // Two completed values at distinct future instants (20 first, then 10): the timer is at the earliest.
    done(&mut brain, &mut cells, 2, Outcome::Ok, 5);
    assert_eq!(timers(&brain), vec![20]);
    done(&mut brain, &mut cells, 1, Outcome::Ok, 5);
    assert_eq!((sent(&brain), held_ids(&brain), timers(&brain)), (vec![1, 2, 3, 4], vec![5, 6], vec![20, 10]), "still counted at now 5; the earlier instant books its own timer");
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 10 });
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![1, 2, 3, 4, 5], vec![6]), "value 1 no longer counts at its instant: 5 is reserved, 6 is not");
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 20 });
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![1, 2, 3, 4, 5, 6], Vec::<u64>::new()));
    assert!(brain.dispositions.is_empty(), "no consumption is invented");
    assert_eq!(resident(&cells[0]), vec![1, 2, 3, 4, 5, 6], "every value stays resident (D) until R consumes it");
    feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
    assert_eq!((resident(&cells[0]), brain.dispositions.clone()), (vec![2, 3, 4, 5, 6], vec![(1, Disposition::Consumed)]));
}

/// K = 1: A (instant 10) is running, B (instant 20) is held; an optional time input arrives while the call runs, then A's `Done(ok)` read at
/// `done_now`. Returns the settings sent and the capacity timers booked.
fn slow_call(early: Option<i64>, done_now: i64) -> (Vec<u64>, Vec<i64>) {
    let mut cells = vec![streaming(0)];
    let mut brain = brain(1, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    running(&mut cells[0], 1);
    release(&mut brain, &mut cells, 0, 2, 20);
    if let Some(now) = early {
        feed(&mut brain, &mut cells, Input::CapacityTimer { now });
        assert_eq!((sent(&brain), held_ids(&brain), timers(&brain)), (vec![1], vec![2], vec![]), "a running call counts past its instant and books no timer");
    }
    done(&mut brain, &mut cells, 1, Outcome::Ok, done_now);
    (sent(&brain), timers(&brain))
}

#[test]
fn b2_slow_call_holds_k_until_done_then_frees_in_that_step() {
    assert_eq!(slow_call(Some(12), 12), (vec![1, 2], vec![]), "time passed e_A while the call ran; at its Done B is reserved in that step");
    assert_eq!(slow_call(None, 10), (vec![1, 2], vec![]), "a Done read exactly at e_A frees K");
    assert_eq!(slow_call(None, 9), (vec![1], vec![10]), "one tick before does not: B waits for the capacity timer at e_A");
}

/// `brain.now` only moves forward: a `Done` read earlier than a time already seen does not make a freed value count again.
#[test]
fn b2c_a_stale_time_input_does_not_lower_the_device_time() {
    let mut cells = vec![streaming(0)];
    let mut brain = brain(1, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    running(&mut cells[0], 1);
    done(&mut brain, &mut cells, 1, Outcome::Ok, 12);
    assert_eq!(brain.now, 12);
    release(&mut brain, &mut cells, 0, 2, 30);
    assert_eq!(sent(&brain), vec![1, 2], "value 1 finished past its instant: K is free");
    running(&mut cells[0], 2);
    release(&mut brain, &mut cells, 0, 3, 40);
    // A stale input (read at 3) and a duplicate arrive: the device time stays 12, value 1 does not count again.
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 3 });
    assert_eq!(brain.now, 12);
    assert_eq!(device_load(&cells, brain.now), 1, "only value 2, which is running");
}

#[test]
fn b2b_r_progress_is_not_device_time() {
    let mut cells = vec![streaming(0)];
    let mut brain = brain(1, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    running(&mut cells[0], 1);
    release(&mut brain, &mut cells, 0, 2, 20);
    cells[0].progress = 50;
    done(&mut brain, &mut cells, 1, Outcome::Ok, 0);
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![1], vec![2]), "R is far past e_A, the device time read is 0: A still counts");
    assert_eq!(timers(&brain), vec![10], "and the timer is at A's instant");
}

#[test]
fn b2d_a_pinned_call_keeps_a_lifecycle_request_waiting_while_k_frees() {
    let mut cells = vec![streaming(0)];
    let mut brain = brain(2, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    release(&mut brain, &mut cells, 0, 2, 20);
    running(&mut cells[0], 1);
    running(&mut cells[0], 2);
    done(&mut brain, &mut cells, 1, Outcome::Ok, 12);
    assert_eq!(device_load(&cells, brain.now), 1, "value 1 has freed K by time; value 2 is still running");
    feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
    assert_eq!(brain.deferred, vec![Request::Restore { channel: 0 }], "the freed capacity does not release the pin of value 2");
    assert!(!brain.outbox.iter().any(|command| matches!(command, Command::Settings(SetCmd { change: Change::Restore { .. }, .. }))));
}

#[test]
fn b3_partial_frees_k_and_keeps_a_resident_and_refused_frees_both() {
    for (outcome, resident_after, c_reserved) in [(Outcome::Partial, vec![1, 2], false), (Outcome::Refused, vec![2, 3], true)] {
        let mut cells = vec![streaming(0)];
        let mut brain = brain(2, 2);
        release(&mut brain, &mut cells, 0, 1, 10);
        release(&mut brain, &mut cells, 0, 2, 20);
        running(&mut cells[0], 1);
        release(&mut brain, &mut cells, 0, 3, 30);
        assert_eq!(held_ids(&brain), vec![3]);
        done(&mut brain, &mut cells, 1, outcome, 10);
        assert_eq!(resident(&cells[0]), resident_after, "{outcome:?}");
        assert_eq!(held_ids(&brain).is_empty(), c_reserved, "{outcome:?}");
        if outcome == Outcome::Partial {
            assert_eq!(brain.reservation_refusal(3, &cells), Some(NotReserved::ChainBound), "K is free (A passed its instant) but A is resident: D refuses");
        }
    }
}

#[test]
fn b3c_dropped_frees_k_and_d_at_its_done() {
    let mut cells = vec![streaming(0)];
    let mut brain = brain(1, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    release(&mut brain, &mut cells, 0, 2, 12);
    cells[0].book_stop(5);
    assert_eq!(cells[0].take(1), super::cell::Take::Dropped, "cancelled before the take");
    feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Dropped, now: 0 });
    assert_eq!((resident(&cells[0]), brain.disposition(1)), (vec![], Some(Disposition::Cancelled)));
}

#[test]
fn b4_stop_cancel_frees_k_and_d_frees_at_dropped() {
    let mut cells = vec![streaming(0)];
    let mut brain = brain(1, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    release(&mut brain, &mut cells, 0, 2, 4);
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![1], vec![2]));
    feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 5 });
    assert_eq!(cells[0].state_of(1), Some(&ValueState::Cancelled));
    assert_eq!(sent(&brain), vec![1, 2], "the cancel freed K: B (instant 4, before the cut 5) is reserved in the Stop step");
    assert_eq!(resident(&cells[0]), vec![1, 2], "A is still resident until its Dropped");
    assert_eq!(cells[0].take(1), super::cell::Take::Dropped);
    feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Dropped, now: 0 });
    assert_eq!(resident(&cells[0]), vec![2]);
}

#[test]
fn b5_k_is_shared_across_chains() {
    let mut cells = vec![streaming(0), streaming(1)];
    let mut brain = brain(2, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    release(&mut brain, &mut cells, 0, 2, 20);
    release(&mut brain, &mut cells, 1, 3, 5);
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![1, 2], vec![3]), "chain 1 is empty but the device is at K");
    assert_eq!(brain.reservation_refusal(3, &cells), Some(NotReserved::DeviceBound));
    running(&mut cells[0], 1);
    done(&mut brain, &mut cells, 1, Outcome::Ok, 10);
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![1, 2, 3], vec![]), "a chain-0 value freed K");
    assert_eq!(resident(&cells[1]), vec![3]);
}

/// Held order is the device's: a refused head holds back a later value of another chain that K and D would admit.
#[test]
fn b5b_held_order_is_one_head_of_line_for_the_device() {
    let mut cells = vec![streaming(0), streaming(1)];
    cells[0].phase = Phase::Off;
    let mut brain = brain(4, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    release(&mut brain, &mut cells, 1, 2, 10);
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![], vec![1, 2]), "the head waits on chain 0's phase, so the later value of chain 1 does not overtake it");
    cells[0].phase = Phase::Streaming(seg(0));
    feed(&mut brain, &mut cells, Input::Boundary(Boundary::FirstSample { stream: 0 }));
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![1, 2], vec![]), "in release order once the head is reservable");
}

#[test]
fn b6_d_holds_until_a_resident_leaves_and_no_timer_frees_it() {
    let mut cells = vec![streaming(0)];
    let mut brain = brain(4, 3);
    for id in 1..=3 {
        release(&mut brain, &mut cells, 0, id, id as i64 * 2);
        running(&mut cells[0], id);
        done(&mut brain, &mut cells, id, Outcome::Ok, 50);
    }
    release(&mut brain, &mut cells, 0, 4, 40);
    assert_eq!((held_ids(&brain), brain.reservation_refusal(4, &cells)), (vec![4], Some(NotReserved::ChainBound)));
    let before = brain.outbox.len();
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 99 });
    done(&mut brain, &mut cells, 99, Outcome::Ok, 100);
    assert_eq!((held_ids(&brain), brain.outbox.len()), (vec![4], before), "time does not free D and books no timer for it");
    feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
    assert_eq!((held_ids(&brain), resident(&cells[0])), (vec![], vec![2, 3, 4]), "the consumption retired value 1 and reserved the held value in that step");
}

#[test]
fn b7_left_value_stays_resident_and_is_consumed_later() {
    let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(10), 10)];
    push(&mut cells[0], value(0, 20, 1), ValueState::MaybeApplied);
    let mut brain = brain(4, 1);
    brain.held.push(held(0, value(0, 5, 2), 1));
    feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: Some(0), epoch: 1 }));
    assert_eq!((brain.disposition(1), cells[0].is_left(1), resident(&cells[0])), (Some(Disposition::Reported(Reason::Left)), true, vec![1]));
    done(&mut brain, &mut cells, 1, Outcome::Ok, 0);
    assert_eq!(resident(&cells[0]), vec![1], "reported left, applied, not consumed: still resident");
    cells[0].phase = Phase::Streaming(seg(0));
    cells[0].cut = None;
    feed(&mut brain, &mut cells, Input::Boundary(Boundary::FirstSample { stream: 0 }));
    assert_eq!((held_ids(&brain), brain.reservation_refusal(2, &cells)), (vec![2], Some(NotReserved::ChainBound)), "the left value counts in D");
    feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
    assert_eq!((brain.leftovers.clone(), held_ids(&brain), resident(&cells[0])), (vec![1], vec![], vec![2]), "a later sample consumed it: it left, and the held value took its place");
}

/// A loss report covers the resident values without removing the ones whose terminal has not arrived.
#[test]
fn b7b_lost_values_stay_resident_until_their_terminal_input() {
    let mut cells = vec![streaming(0)];
    let mut brain = brain(4, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    release(&mut brain, &mut cells, 0, 2, 12);
    running(&mut cells[0], 1);
    feed(&mut brain, &mut cells, Input::Boundary(Boundary::Lost { stream: 0 }));
    assert_eq!(
        (brain.disposition(1), brain.disposition(2), resident(&cells[0])),
        (Some(Disposition::Reported(Reason::Loss)), Some(Disposition::Reported(Reason::Loss)), vec![1, 2]),
        "reported, resident"
    );
    done(&mut brain, &mut cells, 1, Outcome::Ok, 3);
    assert_eq!(resident(&cells[0]), vec![2], "a Done delayed past the report removes its value, with no second disposition");
    feed(&mut brain, &mut cells, Input::Done { id: 2, terminal: Terminal::Abandoned, now: 3 });
    assert_eq!(resident(&cells[0]), Vec::<u64>::new());
    assert_eq!(brain.dispositions.len(), 2);
}

/// A cell with a value counted past its instant (K = 1) and a value held behind it, in a drain.
fn timer_world() -> (Vec<Cell>, Brain) {
    let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 0)];
    let mut brain = brain(1, 64);
    release(&mut brain, &mut cells, 0, 1, 14);
    running(&mut cells[0], 1);
    done(&mut brain, &mut cells, 1, Outcome::Ok, 3);
    release(&mut brain, &mut cells, 0, 2, 9);
    assert_eq!((held_ids(&brain), timers(&brain)), (vec![2], vec![14]));
    (cells, brain)
}

#[test]
fn b8a_capacity_timer_during_a_drain_only_re_evaluates() {
    let (mut cells, mut brain) = timer_world();
    feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 16 });
    assert_eq!(cells[0].drain, Drain::Draining);
    let before = brain.outbox.len();
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 10 });
    assert_eq!(
        (cells[0].drain, cells[0].outstanding, cells[0].phase, brain.outbox.len()),
        (Drain::Draining, None, Phase::Streaming(seg(0)), before + 1),
        "early: K is still full; only a new timer is booked"
    );
    assert!(matches!(brain.outbox.last(), Some(Command::CapacityTimer { at: 14 })));
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 14 });
    assert_eq!(sent(&brain), vec![1, 2], "a due timer that frees K reserves the held value, which the cell allows");
    assert!(!brain.outbox.iter().any(|command| matches!(command, Command::Rx { .. })), "no stop, no restart");
    assert_eq!((cells[0].drain, cells[0].outstanding), (Drain::Draining, None), "the drain goes on");
}

#[test]
fn b8b_stale_capacity_timer_after_an_epoch_change_sends_nothing() {
    let (mut cells, mut brain) = timer_world();
    feed(&mut brain, &mut cells, Input::PlanCancel { stream: 0, origin: 30, epoch: 2, plan_cut: Some(40) });
    let (outbox, epoch, outstanding) = (brain.outbox.clone(), cells[0].epoch, cells[0].outstanding);
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 25 });
    assert_eq!(held_ids(&brain), vec![2], "the cell is Stopping: the value stays held");
    assert_eq!((cells[0].epoch, cells[0].outstanding, cells[0].phase), (epoch, outstanding, Phase::Stopping(seg(0))));
    assert_eq!(brain.outbox[outbox.len()..].iter().filter(|command| !matches!(command, Command::CapacityTimer { .. })).count(), 0, "only a timer, no command");
}

#[test]
fn b8c_capacity_timer_after_the_loss_boundary_sends_nothing() {
    let (mut cells, mut brain) = timer_world();
    feed(&mut brain, &mut cells, Input::Boundary(Boundary::Lost { stream: 0 }));
    let before = brain.outbox.len();
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 99 });
    assert_eq!((held_ids(&brain), brain.outbox.len()), (vec![2], before), "the lost cell refuses by phase; nothing is sent");
    assert_eq!(brain.reservation_refusal(2, &cells), Some(NotReserved::Refused(Refusal::Phase)));
}

#[test]
fn b8d_a_duplicate_capacity_timer_changes_nothing_observable() {
    let (mut cells, mut brain) = timer_world();
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 14 });
    let observed = |brain: &Brain, cells: &[Cell]| (sent(brain), held_ids(brain), brain.dispositions.clone(), resident(&cells[0]), cells[0].phase, brain.deferred.clone());
    let once = observed(&brain, &cells);
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 14 });
    assert_eq!(observed(&brain, &cells), once);
    feed(&mut brain, &mut cells, Input::CapacityTimer { now: 7 });
    assert_eq!((observed(&brain, &cells), brain.now), (once, 14), "a decreasing timestamp changes neither the outcome nor the time");
}

#[test]
fn b9_consumption_before_done_leaves_the_cell_at_done() {
    let mut cells = vec![streaming(0)];
    let mut brain = brain(1, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    running(&mut cells[0], 1);
    release(&mut brain, &mut cells, 0, 2, 20);
    // S wrote the result into the cell, but the `Done` input has not reached the brain: the consumption comes first.
    cells[0].finish_call(1, Outcome::Ok);
    feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
    assert_eq!(
        (brain.disposition(1), resident(&cells[0]), held_ids(&brain)),
        (Some(Disposition::Consumed), vec![1], vec![2]),
        "its Done has not been taken: still resident, and counted (the device time read is stale)"
    );
    done(&mut brain, &mut cells, 1, Outcome::Ok, 0);
    assert_eq!(
        (resident(&cells[0]), held_ids(&brain), sent(&brain)),
        (vec![2], vec![], vec![1, 2]),
        "at Done it leaves, and the held value is reserved in that step (a consumed value's instant has passed)"
    );
}

#[test]
fn b10_no_input_reopens_a_stopping_cell() {
    let inputs = |stream: usize| -> Vec<Input> {
        vec![
            Input::Release(held(stream, value(0, 5, 9), 1)),
            Input::Stop { stream, requested: 5 },
            Input::PlanCancel { stream, origin: 30, epoch: 2, plan_cut: Some(40) },
            Input::Request(Request::Restore { channel: 0 }),
            Input::Request(Request::Close { stream }),
            Input::Request(Request::Open { stream }),
            Input::Request(Request::StartNext { stream, at: 30 }),
            Input::Request(Request::Flush { stream }),
            Input::Request(Request::Late { stream, now: 5 }),
            Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 5 },
            Input::Timer { stream, now: 99 },
            Input::CapacityTimer { now: 99 },
            Input::LateCommand { stream, now: 5 },
            Input::Boundary(Boundary::Consumed { id: 1 }),
            Input::Boundary(Boundary::CutReached { stream }),
            Input::Boundary(Boundary::FirstSample { stream }),
            Input::Boundary(Boundary::Armed { stream }),
            Input::Boundary(Boundary::Quiesced { stream }),
            Input::Boundary(Boundary::Flushed { stream }),
            Input::Boundary(Boundary::BeginFailed { stream }),
            Input::Boundary(Boundary::Opened { stream }),
            Input::Boundary(Boundary::Closed { stream }),
            Input::Shutdown,
        ]
    };
    for input in inputs(0) {
        let mut cells = vec![cell(Phase::Stopping(seg(0)), 0, Some(16), 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Queued);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, input.clone());
        assert!(!matches!(cells[0].phase, Phase::Armed(_) | Phase::Streaming(_)), "{input:?} reopened the cell: {:?}", cells[0].phase);
        assert!(
            brain.outbox.iter().all(|command| !matches!(command, Command::Rx { op: RxOp::Start { .. }, .. })
                || matches!(input, Input::Request(Request::StartNext { .. }) | Input::LateCommand { .. } | Input::Request(Request::Late { .. }))),
            "{input:?}"
        );
    }
}

/// The only writes of a running phase: a begin that the executor completes (from a stopped cell), never from `Stopping`.
#[test]
fn b10b_only_a_stopped_cell_takes_a_completed_start() {
    use super::cell::Call;
    for (from, opens) in [(Phase::Off, true), (Phase::Uncertain, true), (Phase::Stopping(seg(0)), false)] {
        let mut c = cell(from, 0, None, 0);
        c.device_result(Call::Begin(seg(4)), &Ok(()));
        assert_eq!(matches!(c.phase, Phase::Armed(_)), opens, "{from:?}");
    }
}

#[test]
fn b0_the_design_bounds_are_the_defaults() {
    let brain = Brain::new();
    assert_eq!((brain.device_bound, brain.chain_bound), (DEVICE_BOUND, super::core::CHAIN_BOUND));
}

/// K's predicate over every value state and the instant: the rows of the design (note 45 §4.5).
#[test]
fn b3d_each_state_counts_or_not_for_k() {
    use super::cell::InFlight;
    use super::core::counts_for_k;
    let ok = Terminal::Call(Outcome::Ok);
    let partial = Terminal::Call(Outcome::Partial);
    // (state, counted before the instant, counted at the instant)
    let table = [
        (ValueState::Queued, true, true),
        (ValueState::Taken, true, true),
        (ValueState::MaybeApplied, true, true),
        (ValueState::Done(ok), true, false),
        (ValueState::Done(partial), true, false),
        (ValueState::Cancelled, false, false),
        (ValueState::Done(Terminal::Dropped), false, false),
        (ValueState::Done(Terminal::Call(Outcome::Refused)), false, false),
        (ValueState::Done(Terminal::Call(Outcome::Lost)), false, false),
        (ValueState::Done(Terminal::Abandoned), false, false),
    ];
    let live = cell(Phase::Streaming(seg(0)), 0, None, 0);
    for (state, before, at) in table {
        let flight = InFlight { entry: value(0, 10, 1), state: state.clone(), left: false };
        assert_eq!((counts_for_k(&live, &flight, 9), counts_for_k(&live, &flight, 10)), (before, at), "{state:?}");
    }
    // A lost chain's finished call no longer counts; one that may still be running does.
    let lost = cell(Phase::Lost, 0, None, 0);
    let finished = InFlight { entry: value(0, 10, 1), state: ValueState::Done(ok), left: false };
    let running = InFlight { entry: value(0, 10, 1), state: ValueState::MaybeApplied, left: false };
    assert_eq!((counts_for_k(&lost, &finished, 0), counts_for_k(&lost, &running, 0)), (false, true));
}

/// An end-now request cancels the queued value of its stream, which frees K for the held value of another stream: the retry follows the
/// `Decide` that sent the stop, in that step (not at some later input).
#[test]
fn b4b_an_end_now_cancel_frees_k_and_reserves_the_other_chains_value_in_that_step() {
    let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 0), streaming(1)];
    let mut brain = brain(1, 64);
    release(&mut brain, &mut cells, 0, 1, 10);
    release(&mut brain, &mut cells, 1, 2, 11);
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![1], vec![2]), "K is full with A queued");
    feed(&mut brain, &mut cells, Input::Request(Request::Late { stream: 0, now: 5 }));
    assert_eq!(cells[0].state_of(1), Some(&ValueState::Cancelled), "the end-now stop cancelled A");
    assert_eq!((sent(&brain), held_ids(&brain)), (vec![1, 2], Vec::<u64>::new()), "B on stream 1 is reserved in the same step");
}

/// B8 after `F` (45-experiment §11 B8): the brain does not read `F`. B is held behind A (K full); `F` is set and the loss boundary is still on its way;
/// A's instant comes and the capacity timer reserves B; S takes B, checks `F` and abandons it with no device call; B leaves the cell at its terminal
/// `Done(Abandoned)`, not at the reservation. Every oracle holds at each step and the threads agree on the whole run.
#[test]
fn b8e_a_capacity_timer_after_f_reserves_and_s_abandons_with_no_device_call() {
    use super::core::Timer;
    use super::dst::Model;
    use super::thread_tests::{Script, check, params};
    use super::world::{EnvInput, EnvSlot, Event::*};
    let env = vec![EnvSlot { input: EnvInput::Loss, earliest: 14, latest: 14, delivered: false, after: None }];
    let mut world = super::cases::single(None, env, super::world::Params { k: 1, d: 64, ..params() });
    push(&mut world.cells[0], value(0, 16, 1), ValueState::Done(Terminal::Call(Outcome::Ok)));
    world.brain.device_bound = 1;
    world.brain.held.push(held(0, value(0, 30, 2), 1));
    world.brain.capacity_timer = Some(16);
    world.timers.push((Timer::Capacity, 16));
    world.brain.results.push((1, Terminal::Call(Outcome::Ok)));
    for (id, e) in [(1, 16), (2, 30)] {
        world.monitor.instants.insert(id, e);
    }
    world.monitor.reserved.insert(1, (0, seg(0), 1));
    world.monitor.streams.insert(1, 0);
    world.monitor.terminals.insert(1, Terminal::Call(Outcome::Ok));
    world.monitor.done_processed.insert(1);
    world.monitor.released.insert(2);
    let mut script = Script::new(world);
    let run = |script: &mut Script, events: &[super::world::Event]| {
        for event in events {
            let before = script.w.clone();
            script.ev(std::slice::from_ref(event));
            before.transition(event, &script.w).unwrap_or_else(|why| panic!("{event:?}: {why}"));
            script.w.safety().unwrap_or_else(|why| panic!("{event:?}: {why}"));
        }
    };
    run(&mut script, &[RCheck(0), RSnap(0), RBegin(0)]);
    run(&mut script, &std::iter::repeat_n(Tick, 14).collect::<Vec<_>>());
    run(&mut script, &[Env(0), REnd(0), Pub(super::world::Holder::Rx(0), super::world::PubStep::Gate), Pub(super::world::Holder::Rx(0), super::world::PubStep::Set)]);
    assert!(script.w.failure && script.w.cells[0].phase != Phase::Lost, "F is set and the loss boundary has not reached the brain");
    run(&mut script, &[Pub(super::world::Holder::Rx(0), super::world::PubStep::Ungate), Pub(super::world::Holder::Rx(0), super::world::PubStep::Mark), Tick, Tick, Timer(0), BStep]);
    while !script.w.brain.ops.is_empty() {
        run(&mut script, &[BOp]);
    }
    assert_eq!(script.w.cells[0].state_of(2), Some(&ValueState::Queued), "the capacity timer reserved B although F is set");
    run(&mut script, &[STake, SCheck]);
    assert!(script.w.calls.iter().all(|call| !matches!(call, super::rules::DevCall::Apply { .. })), "S checked F and made no device call: {:?}", script.w.calls);
    assert_eq!(script.w.cells[0].state_of(2), Some(&ValueState::Done(Terminal::Abandoned)), "abandoned, and still resident until the brain takes the terminal");
    run(&mut script, &[BStep]);
    while !script.w.brain.ops.is_empty() {
        run(&mut script, &[BOp]);
    }
    assert_eq!((script.w.cells[0].state_of(2), script.w.brain.disposition(2)), (None, Some(super::core::Disposition::Reported(super::core::Reason::Loss))), "retired at its terminal");
    script.rest();
    check(&script.w_initial(), &script.trace).expect("the thread run agrees on the whole sequence");
}
