//! The case scripts of design note 45's experiment (45-experiment §4, test plan section D): one
//! builder per case and variant, the seeded configuration, the micro tests, the walk sweeps, the
//! section-C checks of the cases, and the measurement that explores each reduced variant at horizons
//! 8, 16 and 24. A builder sets up the world and declares the witnesses; the verdict is the
//! explorer's, or a walk's.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use super::cell::{Phase, ValueState};
use super::core::{CHAIN_BOUND, Change, DEVICE_BOUND, DRAIN_WINDOW, Input, Request, SetCmd, Timer};
use super::dst::{self, Config, Model, Verdict};
use super::fixtures::{cell, held, push, seg, value};
use super::world::{EnvInput, EnvSlot, Event, Holder, INITIAL_OCCUPANT, Params, PubStep, RStep, World};
use crate::provider::generator::Rng;

/// The state budget of an exhaustive run (45-experiment §3.3).
const BUDGET: usize = 2_000_000;
/// The state budget and wall limit of a default bounded run (a debug build, a few seconds).
const SMALL_BUDGET: usize = 30_000;
const SMALL_WALL: Duration = Duration::from_secs(60);
/// The wall limit of an exhaustive run, in seconds (45-experiment §3.3).
const WALL: Duration = Duration::from_secs(180);
/// The horizons a measurement runs, smallest first (the plan's measure-first step).
const HORIZONS: [i64; 3] = [8, 16, 24];
/// The epilogue of the full and reduced variants: the largest cut, its drain deadline and one recv timeout.
const EPILOGUE: i64 = 32;
/// The seeded configuration's epilogue (its largest cut is 80).
const SEEDED_EPILOGUE: i64 = 96;
const RECV_TIMEOUT: i64 = 8;
const FIFO_CAP: usize = 32;
/// The walk's event limit: far above any terminal state of the seeded horizon.
pub(super) const WALK_LIMIT: usize = 1_000_000;
/// The micro variants' time scale (45-experiment §10, draft 5): a recv timeout, a drain window and an
/// epilogue of a few ticks, so the exhaustive search of each micro case completes within the budget.
const MICRO_RECV_TIMEOUT: i64 = 1;
const MICRO_DRAIN_WINDOW: i64 = 1;
const MICRO_EPILOGUE: i64 = 3;
/// The seeds a walk sweep runs when `EZSDR_SEEDS` is unset, and its wall stop.
const SWEEP_DEFAULT: &str = "1-10000";
const SWEEP_WALL: Duration = Duration::from_secs(30 * 60);

/// A case's variant (45-experiment §3.3, §4). `Full` is the case at its full horizon, the script the
/// walk sweeps run. `Micro` keeps every input, outcome branch and witness of the case, moves its
/// instants early and shortens its horizon and time scale, so the exhaustive search completes.
/// `Reduced(h)` is the measured variant at horizon `h`, whose numbers are the evidence for the
/// owner's decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    Full,
    Micro,
    Reduced(i64),
}

/// The time scale of a variant: the epilogue, the recv timeout, the drain window and how long before
/// the horizon the shutdown may come.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    epilogue: i64,
    recv_timeout: i64,
    drain_window: i64,
    shutdown_lead: i64,
}

impl Variant {
    fn timing(self) -> Timing {
        match self {
            Variant::Micro => Timing { epilogue: MICRO_EPILOGUE, recv_timeout: MICRO_RECV_TIMEOUT, drain_window: MICRO_DRAIN_WINDOW, shutdown_lead: 1 },
            Variant::Full | Variant::Reduced(_) => Timing { epilogue: EPILOGUE, recv_timeout: RECV_TIMEOUT, drain_window: DRAIN_WINDOW, shutdown_lead: 8 },
        }
    }

    /// The horizon of a case in this variant: its full horizon, the micro horizon, or the measured one.
    fn horizon(self, full: i64, micro: i64) -> i64 {
        match self {
            Variant::Full => full,
            Variant::Micro => micro,
            Variant::Reduced(h) => h,
        }
    }
}

/// A case: its world and the explorer's configuration (name, budget, wall limit, witnesses).
pub(crate) struct Case {
    pub(crate) world: World,
    pub(crate) config: Config,
}

fn params(timing: Timing, horizon: i64, queue_cap: usize, call_faults: bool, panic_step: Option<usize>) -> Params {
    Params {
        horizon,
        epilogue: timing.epilogue,
        recv_timeout: timing.recv_timeout,
        queue_cap,
        fifo_cap: FIFO_CAP,
        drain_window: timing.drain_window,
        packet: crate::brain::world::PACKET,
        call_faults,
        panic_step,
        k: DEVICE_BOUND,
        d: CHAIN_BOUND,
    }
}

/// An input delivered once in `[earliest, latest]`, after the slot at index `after` if given.
fn input(input: Input, (earliest, latest): (i64, i64), after: Option<usize>) -> EnvSlot {
    EnvSlot { input: EnvInput::Input(input), earliest, latest, delivered: false, after }
}

/// The device's loss, in its window.
fn loss((earliest, latest): (i64, i64)) -> EnvSlot {
    EnvSlot { input: EnvInput::Loss, earliest, latest, delivered: false, after: None }
}

/// The late command of `stream`: the clock reader reads the time and delivers it, in its window.
fn late(stream: usize, (earliest, latest): (i64, i64)) -> EnvSlot {
    EnvSlot { input: EnvInput::Late { stream }, earliest, latest, delivered: false, after: None }
}

/// The shutdown: no earlier than `horizon - shutdown_lead`, and after every other input (the world orders it).
fn shutdown(horizon: i64, timing: Timing) -> EnvSlot {
    EnvSlot { input: EnvInput::Input(Input::Shutdown), earliest: horizon - timing.shutdown_lead, latest: horizon + timing.epilogue, delivered: false, after: None }
}

/// Stream `k` open on `channels` and started at tick 0, with its handle open.
pub(super) fn running(world: &mut World, k: usize, channels: Vec<usize>) {
    world.device.open(k, channels);
    world.device.start(k, 0);
    world.rx[k].handle = Some(1);
}

/// One stream 0 on channel 0, streaming under segment 0 with `cut` in force, and running.
pub(super) fn single(cut: Option<i64>, env: Vec<EnvSlot>, params: Params) -> World {
    let mut world = World::new(vec![cell(Phase::Streaming(seg(0)), 0, cut, 0)], env, params);
    running(&mut world, 0, vec![0]);
    world
}

fn config(variant: Variant, names: [&'static str; 3], witnesses: &'static [&'static str]) -> Config {
    let name = match variant {
        Variant::Full => names[0],
        Variant::Reduced(_) => names[1],
        Variant::Micro => names[2],
    };
    Config { name, budget: BUDGET, wall: WALL, witnesses }
}

/// Case 1 (section D): a value, a cut, and a Stop that shortens the cut.
/// Micro: value instant 1, cut 2, release [0, 1], Stop asks for 0 in [0, 2] (below the value's instant,
/// so a Stop that meets the value untaken gives W1b), horizon 2, recv timeout 1, drain window 1, epilogue 3. Reduced: as the reduced script (release [0, 3], Stop [0, 8], horizon h).
pub(crate) fn case1(variant: Variant) -> Case {
    let (at, cut, release, stop, requested) = match variant {
        Variant::Full => (8, 16, (0, 7), (0, 16), 6),
        Variant::Reduced(_) => (8, 16, (0, 3), (0, 8), 6),
        Variant::Micro => (1, 2, (0, 1), (0, 2), 0),
    };
    let horizon = variant.horizon(48, 2);
    // The stop asks for an instant below the value's, so the value is dropped when taken (W1b).
    let env = vec![input(Input::Release(held(0, value(0, at, 1), 1)), release, None), input(Input::Stop { stream: 0, requested }, stop, None), shutdown(horizon, variant.timing())];
    let world = single(Some(cut), env, params(variant.timing(), horizon, 1, false, None));
    Case { world, config: config(variant, ["case 1 full", "case 1 reduced", "case 1 micro"], &["W1a", "W1b", "W1c"]) }
}

/// Case 2 (section D; no Stop): a value and a device loss at every point.
/// Micro: value instant 1, cut 2, release [0, 1], loss [0, 4], horizon 3.
pub(crate) fn case2(variant: Variant) -> Case {
    let (at, cut, release, loss_window) = match variant {
        Variant::Full => (8, 16, (0, 7), (0, 48)),
        Variant::Reduced(_) => (8, 16, (0, 3), (0, 16)),
        Variant::Micro => (1, 2, (0, 1), (0, 4)),
    };
    let horizon = variant.horizon(48, 3);
    let env = vec![input(Input::Release(held(0, value(0, at, 1), 1)), release, None), loss(loss_window), shutdown(horizon, variant.timing())];
    let world = single(Some(cut), env, params(variant.timing(), horizon, 1, false, None));
    Case { world, config: config(variant, ["case 2 full", "case 2 reduced", "case 2 micro"], &["W2a", "W2b", "W2c"]) }
}

/// Case 3 (section D): a plan change with a recv and a value in flight, and a Stop.
/// Micro: value instant 1, cut 2, release [0, 1]; the plan change (origin 3, epoch 2, plan cut 4) in
/// [0, 2]; Stop asks for 1 in [0, 2]; horizon 3.
pub(crate) fn case3(variant: Variant) -> Case {
    let (at, cut, release, (plan_origin, plan_window), plan_cut, stop, requested) = match variant {
        Variant::Full => (8, 16, (0, 7), (20, (0, 16)), 48, (0, 16), 10),
        Variant::Reduced(_) => (8, 16, (0, 3), (12, (0, 11)), 48, (0, 9), 10),
        Variant::Micro => (1, 2, (0, 1), (3, (0, 2)), 4, (0, 2), 1),
    };
    let horizon = variant.horizon(48, 3);
    let env = vec![
        input(Input::Release(held(0, value(0, at, 1), 1)), release, None),
        input(Input::PlanCancel { stream: 0, origin: plan_origin, epoch: 2, plan_cut: Some(plan_cut) }, plan_window, None),
        input(Input::Stop { stream: 0, requested }, stop, None),
        shutdown(horizon, variant.timing()),
    ];
    let world = single(Some(cut), env, params(variant.timing(), horizon, 1, false, None));
    Case { world, config: config(variant, ["case 3 full", "case 3 reduced", "case 3 micro"], &["W3a", "W3b", "W3c"]) }
}

/// Case 4 (section D): two equal-time values, released in order, and the Stopping transition.
/// Micro: both values at instant 1, cut 2, release [0, 1], horizon 3.
pub(crate) fn case4(variant: Variant) -> Case {
    let (at, cut, release) = match variant {
        Variant::Full => (8, 12, (0, 7)),
        Variant::Reduced(_) => (8, 12, (0, 3)),
        Variant::Micro => (1, 2, (0, 1)),
    };
    let horizon = variant.horizon(48, 3);
    let env = vec![input(Input::Release(held(0, value(0, at, 1), 1)), release, None), input(Input::Release(held(0, value(0, at, 2), 1)), release, Some(0)), shutdown(horizon, variant.timing())];
    let world = single(Some(cut), env, params(variant.timing(), horizon, 1, false, None));
    Case { world, config: config(variant, ["case 4 full", "case 4 reduced", "case 4 micro"], &["W4a", "W4b"]) }
}

/// Case 5 (section D): a settings call blocked past the drain deadline, with conflicting requests.
/// The queue holds an earlier command, due at `occupant`, so the value's call waits holding `L`.
/// Micro: value instant 1, cut 2 (drain deadline 3), the occupant due 4, Restore and StartNext in
/// [2, 3] with the next segment at 3, horizon 4.
pub(crate) fn case5(variant: Variant) -> Case {
    let (at, cut, release, requests, occupant, next) = match variant {
        Variant::Full => (8, 12, (0, 7), (12, 40), 40, 44),
        Variant::Reduced(_) => (8, 12, (0, 0), (12, 16), 40, 44),
        Variant::Micro => (1, 2, (0, 0), (2, 3), 4, 3),
    };
    let horizon = variant.horizon(56, 4);
    let env = vec![
        input(Input::Release(held(0, value(0, at, 1), 1)), release, None),
        input(Input::Request(Request::Restore { channel: 0 }), requests, None),
        input(Input::Request(Request::StartNext { stream: 0, at: next }), requests, None),
        shutdown(horizon, variant.timing()),
    ];
    let mut world = single(Some(cut), env, params(variant.timing(), horizon, 1, false, None));
    world.device.queue.push_back(occupant);
    Case { world, config: config(variant, ["case 5 full", "case 5 reduced", "case 5 micro"], &["W5a", "W5b", "W5c", "W5d"]) }
}

/// Case 6 (section D): case 1's script with a brain panic at brain step `k` (counted from 0). W6a (the
/// panic finds a command in S's hand) is declared only for the step that processes Stop: a panic at an
/// earlier step precedes any settings command, so W6a cannot be reached there (a plan gap, E1b part 3c).
pub(crate) fn case6(variant: Variant, k: usize) -> Case {
    let mut case = case1(variant);
    case.world.params.panic_step = Some(k);
    let stop_step = case6_stop_step(variant);
    let witnesses: &'static [&'static str] = if k == stop_step { &["W6a", "W6b"] } else { &["W6b"] };
    case.config = config(variant, ["case 6 full", "case 6 reduced", "case 6 micro"], witnesses);
    case
}

/// Case 7a (section D): a handoff loss, then a cut-free LateCommand. The loss shares the late
/// command's window, so the `lost` branch of the settings call is enabled once the device is lost.
/// Micro: value instant 1, release [0, 1], late command and loss in [0, 2], horizon 3.
pub(crate) fn case7a(variant: Variant) -> Case {
    let (at, release, late_window) = match variant {
        Variant::Full => (8, (0, 7), (0, 40)),
        Variant::Reduced(_) => (8, (0, 3), (0, 15)),
        Variant::Micro => (1, (0, 1), (0, 2)),
    };
    let horizon = variant.horizon(48, 3);
    let env = vec![input(Input::Release(held(0, value(0, at, 1), 1)), release, None), late(0, late_window), loss(late_window), shutdown(horizon, variant.timing())];
    let world = single(None, env, params(variant.timing(), horizon, 1, false, None));
    Case { world, config: config(variant, ["case 7a full", "case 7a reduced", "case 7a micro"], &["W7a", "W7b"]) }
}

/// Case 7b (section D): a successful stop leaves debt; the flush's final stop fails; restore (D9b-2).
/// Micro: value instant 1, cut 4, release [0, 1], Stop asks for 2 in [0, 2], restore in [2, 4], horizon 4.
pub(crate) fn case7b(variant: Variant) -> Case {
    let (at, cut, release, stop, requested, restore) = match variant {
        Variant::Full => (4, 16, (0, 3), (0, 7), 6, (0, 47)),
        Variant::Reduced(_) => (4, 16, (0, 1), (0, 4), 6, (6, 23)),
        Variant::Micro => (1, 4, (0, 1), (0, 2), 2, (2, 4)),
    };
    let horizon = variant.horizon(48, 4);
    let env = vec![
        input(Input::Release(held(0, value(0, at, 1), 1)), release, None),
        input(Input::Stop { stream: 0, requested }, stop, None),
        input(Input::Request(Request::Restore { channel: 0 }), restore, None),
        shutdown(horizon, variant.timing()),
    ];
    // Every variant enables call faults: the flush's final stop must be able to fail (W7b2).
    let world = single(Some(cut), env, params(variant.timing(), horizon, 1, true, None));
    Case { world, config: config(variant, ["case 7b full", "case 7b reduced", "case 7b micro"], &["W7b1", "W7b2"]) }
}

/// Case 7b with a second Restore from the cut on (W7b3, a Restore after the failed final stop). The first Restore is delivered
/// early in nearly every walk, so only the second can arrive after the failure. A separate case, not an edit of case 7b: the
/// shutdown waits for every input, so the later window removes the early-shutdown schedules the original keeps.
/// Micro: the second Restore in [4, 6].
pub(crate) fn case7b_late_restore(variant: Variant) -> Case {
    let mut case = case7b(variant);
    let again = match variant {
        Variant::Full => (16, 47),
        Variant::Reduced(_) => (16, 23),
        Variant::Micro => (4, 6),
    };
    case.world.env.insert(3, input(Input::Request(Request::Restore { channel: 0 }), again, Some(2)));
    case.config = config(variant, ["case 7b late restore full", "case 7b late restore reduced", "case 7b late restore micro"], &["W7b1", "W7b2", "W7b3"]);
    case
}

/// Case 7c (section D): a begin with outstanding debt, and its start failing. Faults are on, so the
/// begin's failure branch exists; they are on for every call (see the report).
/// Micro: segment 1 cut 3, value instant 1 in [0, 1], Stop asks for 2 in [0, 1] (above the value's
/// instant, below the cut, so it shortens the cut), StartNext at 3 in [0, 3], horizon 4.
pub(crate) fn case7c(variant: Variant) -> Case {
    let (at, cut, release, stop, requested, start) = match variant {
        Variant::Full => (4, 8, (0, 3), (0, 7), 6, (0, 47)),
        Variant::Reduced(_) => (4, 8, (0, 3), (0, 5), 6, (0, 7)),
        Variant::Micro => (1, 3, (0, 1), (0, 1), 2, (0, 3)),
    };
    let horizon = variant.horizon(24, 4);
    let env = vec![
        input(Input::Release(held(0, value(0, at, 1), 1)), release, None),
        input(Input::Stop { stream: 0, requested }, stop, None),
        input(Input::Request(Request::StartNext { stream: 0, at: cut }), start, None),
        shutdown(horizon, variant.timing()),
    ];
    let world = single(Some(cut), env, params(variant.timing(), horizon, 1, true, None));
    Case { world, config: config(variant, ["case 7c full", "case 7c reduced", "case 7c micro"], &["W7c1", "W7c2"]) }
}

/// Case 8 (section D): lifecycle admission across a loss (Close, then Open, then a StartNext).
/// Micro: value instant 1, release and Stop in [0, 1], lifecycle requests in [0, 2] with the next
/// segment at 2, loss in [0, 3], horizon 3.
pub(crate) fn case8(variant: Variant) -> Case {
    let (at, cut, release, stop, requested, lifecycle, loss_window) = match variant {
        Variant::Full | Variant::Reduced(_) => (4, 8, (0, 2), (0, 2), 4, (0, 8), (0, 12)),
        Variant::Micro => (1, 2, (0, 1), (0, 1), 1, (0, 2), (0, 3)),
    };
    let horizon = variant.horizon(32, 3);
    let next = cut;
    let env = vec![
        // S sees the loss while R waits (W8a, W8b): the value is released before the stop. The start follows the reopen: a start on a closed stream is not a request the plan makes (E1b part 3d).
        input(Input::Release(held(0, value(0, at, 1), 1)), release, None),
        input(Input::Stop { stream: 0, requested }, stop, None),
        input(Input::Request(Request::Close { stream: 0 }), lifecycle, None),
        input(Input::Request(Request::Open { stream: 0 }), lifecycle, Some(2)),
        input(Input::Request(Request::StartNext { stream: 0, at: next }), lifecycle, Some(3)),
        loss(loss_window),
        shutdown(horizon, variant.timing()),
    ];
    // Stream 1 (channel 1) streams from 0 with no values and no cut: its R can see the loss in a recv or a stop and publish it holding G
    // while R0's Close or Open waits for G (W8a), or is refused at its recheck (W8b). With one stream those witnesses are unreachable.
    let mut world = World::new(vec![cell(Phase::Streaming(seg(0)), 0, Some(cut), 0), cell(Phase::Streaming(seg(0)), 1, None, 0)], env, params(variant.timing(), horizon, 1, false, None));
    running(&mut world, 0, vec![0]);
    running(&mut world, 1, vec![1]);
    Case { world, config: config(variant, ["case 8 full", "case 8 reduced", "case 8 micro"], &["W8a", "W8b", "W8c", "W8d"]) }
}

/// Case 9 (section D): pending settings work meets a lifecycle request, with partial and refused
/// results. Stream 1 is open and idle, and the move takes channel 0 to it.
/// Micro: value instant 3, cut 4, release [0, 1], Stop asks for 2 in [0, 2], move and restore in [0, 4], horizon 3.
pub(crate) fn case9(variant: Variant) -> Case {
    let (at, cut, release, stop, requested, moved, restore, occupant) = match variant {
        Variant::Full => (8, 16, (0, 3), (0, 12), 6, (0, 16), (0, 16), 4),
        Variant::Reduced(_) => (8, 16, (0, 1), (0, 4), 6, (0, 8), (0, 8), 2),
        Variant::Micro => (3, 4, (0, 1), (0, 2), 2, (0, 4), (0, 4), 1),
    };
    let horizon = variant.horizon(32, 3);
    let env = vec![
        input(Input::Release(held(0, value(0, at, 1), 1)), release, None),
        input(Input::Stop { stream: 0, requested }, stop, None),
        input(Input::Request(Request::Move { channel: 0, to: 1 }), moved, None),
        input(Input::Request(Request::Restore { channel: 0 }), restore, None),
        shutdown(horizon, variant.timing()),
    ];
    let cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(cut), 0), cell(Phase::Off, 1, None, 0)];
    let mut world = World::new(cells, env, params(variant.timing(), horizon, 1, false, None));
    running(&mut world, 0, vec![0]);
    // The channel-0 DSP FIFO holds an earlier timed write (value 0, an initial device state like case 5's queue occupant)
    // and has one slot: v's settings call is refused while the occupant waits, and applies once it drains at its sample.
    world.params.fifo_cap = 1;
    world.device.fifo_cap = 1;
    world.device.dsp.get_mut(&0).expect("the chain is open").fifo.push_back((occupant, INITIAL_OCCUPANT));
    world.device.open(1, vec![1]);
    world.rx[1].handle = Some(1);
    Case { world, config: config(variant, ["case 9 full", "case 9 reduced", "case 9 micro"], &["W9a.ok", "W9a.partial", "W9a.refused", "W9b", "W9c"]) }
}

/// The bounds of a bounds case: K and D on top of the usual parameters.
fn bounded(timing: Timing, horizon: i64, queue_cap: usize, k: usize, d: usize) -> Params {
    Params { k, d, ..params(timing, horizon, queue_cap, false, None) }
}

/// Case 10 (E4a1; B1, B2, B3, B6, B9): one chain at K = 1 and D = 2 in every variant, three values released in order at spaced instants.
/// Which value waits for K or D, and what frees it (a `Done` read at or after its instant, the capacity timer, a consumption), is left to
/// the schedule: R may lag, S may lag, and time passes between a `Done` and its value's instant. `S.effect` is ok or partial.
/// Micro: instants 1, 2, 3, releases in [0, 1], horizon 4.
pub(crate) fn case10(variant: Variant) -> Case {
    let (instants, release) = match variant {
        Variant::Full | Variant::Reduced(_) => ([6, 10, 14], (0, 3)),
        Variant::Micro => ([1, 2, 3], (0, 1)),
    };
    let horizon = variant.horizon(40, 4);
    let mut env: Vec<EnvSlot> = instants.iter().enumerate().map(|(i, at)| input(Input::Release(held(0, value(0, *at, i as u64 + 1), 1)), release, i.checked_sub(1))).collect();
    env.push(shutdown(horizon, variant.timing()));
    let world = single(None, env, bounded(variant.timing(), horizon, 4, 1, 2));
    Case { world, config: config(variant, ["case 10 full", "case 10 reduced", "case 10 micro"], &["W10a", "W10b", "W10c"]) }
}

/// Case 11 (E4a1; B4, B5): two streams at K = 1. A (channel 0) and B (channel 1) are released in order; Stop on stream 0 asks for an
/// instant below A's, so a queued A is cancelled and frees K at the cancel, and its `Dropped` frees D later.
/// The values are released in instant order (A before B), B on the stream without a cut. Micro: instants 2 and 3, cut 4, Stop asks for 1,
/// windows [0, 1] and [0, 2], horizon 3.
pub(crate) fn case11(variant: Variant) -> Case {
    let (a, b, requested, release, stop, cut) = match variant {
        Variant::Full | Variant::Reduced(_) => (8, 9, 5, (0, 3), (0, 8), 16),
        Variant::Micro => (2, 3, 1, (0, 1), (0, 2), 4),
    };
    let horizon = variant.horizon(32, 3);
    let env = vec![
        input(Input::Release(held(0, value(0, a, 1), 1)), release, None),
        input(Input::Release(held(1, value(1, b, 2), 1)), release, Some(0)),
        input(Input::Stop { stream: 0, requested }, stop, None),
        shutdown(horizon, variant.timing()),
    ];
    let cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(cut), 0), cell(Phase::Streaming(seg(0)), 1, None, 0)];
    let mut world = World::new(cells, env, bounded(variant.timing(), horizon, 4, 1, 64));
    running(&mut world, 0, vec![0]);
    running(&mut world, 1, vec![1]);
    Case { world, config: config(variant, ["case 11 full", "case 11 reduced", "case 11 micro"], &["W10a", "W1b", "W11b"]) }
}

/// Case 12a (E4a1; B8, B10): K = 1, a value A that stays counted until its instant (K full), a held B behind it released later at a later
/// instant, a Stop whose cut (full: 12; micro: 3) is above both instants, so the drain stays open, a plan change and a loss, so a capacity timer
/// can arrive during the drain (W12a), after the epoch advanced (W12p) and after `F` is set (W12c). No reservation or stop may come out of a timer that K, D and the cell do not allow (oracle step check B8, I11′, I17).
/// Micro: A at 1, B at 2 (released in instant order), Stop asks for 3, plan change (origin 3, epoch 2, plan cut 4) and loss both at 3, horizon 4.
pub(crate) fn case12a(variant: Variant) -> Case {
    let (a, b, requested, plan, plan_cut, loss_window) = match variant {
        Variant::Full | Variant::Reduced(_) => (8, 9, 12, (20, (4, 14)), 40, (3, 24)),
        Variant::Micro => (1, 2, 3, (3, (3, 3)), 4, (3, 3)),
    };
    let horizon = variant.horizon(40, 4);
    let env = vec![
        input(Input::Release(held(0, value(0, a, 1), 1)), (0, 2), None),
        input(Input::Release(held(0, value(0, b, 2), 1)), (0, 3), Some(0)),
        input(Input::Stop { stream: 0, requested }, (0, 8), None),
        input(Input::PlanCancel { stream: 0, origin: plan.0, epoch: 2, plan_cut: Some(plan_cut) }, plan.1, None),
        loss(loss_window),
        shutdown(horizon, variant.timing()),
    ];
    let world = single(Some(plan_cut.min(16)), env, bounded(variant.timing(), horizon, 4, 1, 64));
    Case { world, config: config(variant, ["case 12a full", "case 12a reduced", "case 12a micro"], &["W12a", "W12p", "W12c"]) }
}

/// Case 12b (E4a1; H03's second guard, I17): `Streaming` with progress 8, A queued with its command in S's FIFO, Stop(8) booked (cut =
/// progress). When S has taken A by then, the pin keeps the drain open and the phase `Streaming`, and B (instant 4) released after the Stop is
/// refused by the `progress >= cut` check of `reserve` alone while R has not yet published (W12b). Without it B is reserved at the cut (I2).
pub(crate) fn case12b(variant: Variant) -> Case {
    let horizon = variant.horizon(24, 12);
    let env = vec![input(Input::Stop { stream: 0, requested: 8 }, (8, 9), None), input(Input::Release(held(0, value(0, 4, 2), 1)), (8, 10), Some(0)), shutdown(horizon, variant.timing())];
    let mut c = cell(Phase::Streaming(seg(0)), 0, None, 8);
    push(&mut c, value(0, 12, 1), ValueState::Queued);
    let mut world = World::new(vec![c], env, bounded(variant.timing(), horizon, 4, 4, 64));
    world.device.open(0, vec![0]);
    world.device.start(0, 8);
    world.device.t = 8;
    world.rx[0].handle = Some(1);
    let command = SetCmd { id: 1, change: Change::Timed { channel: 0, freq: 1.0, at: 12, generation: 1 } };
    world.settings.fifo.push_back(command);
    world.monitor.reserved.insert(1, (0, seg(0), 1));
    world.monitor.streams.insert(1, 0);
    world.monitor.instants.insert(1, 12);
    world.monitor.commanded.insert(1);
    world.monitor.released.insert(1);
    Case { world, config: config(variant, ["case 12b full", "case 12b reduced", "case 12b micro"], &["W12b"]) }
}

/// A window `(e, l)` with `e <= l` within `[0, hi]`: two independent draws could invert it, and an
/// inverted window never delivers.
fn window(pick: &mut dyn FnMut(i64, i64) -> i64, hi: i64) -> (i64, i64) {
    let e = pick(0, hi);
    (e, pick(e, hi))
}

/// The window of a slot `after` another with window `(pe, pl)`: it starts no earlier and ends no earlier
/// than the predecessor's window. Tick waits for every undelivered slot's latest tick, so a successor that
/// ends before its predecessor can start is never deliverable and the case deadlocks (seed 2 of the
/// reduced seeded configuration).
fn successor_window(pick: &mut dyn FnMut(i64, i64) -> i64, (pe, pl): (i64, i64), hi: i64) -> (i64, i64) {
    let e = pick(pe, hi);
    (e, pick(pl.max(e), hi))
}

/// The seeded configuration (section D): two streams, four values, and the environment inputs each
/// at a position the seed chooses in its window. `with_loss` adds a device loss at a seed-chosen tick.
/// The epilogue is 96 (the largest cut is 80), queue capacity 2, K = 2 and D = 3, and `horizon` is 32 or 96.
pub(crate) fn seeded(seed: u64, with_loss: bool, horizon: i64) -> Case {
    let timing = Timing { epilogue: SEEDED_EPILOGUE, recv_timeout: RECV_TIMEOUT, drain_window: DRAIN_WINDOW, shutdown_lead: 8 };
    let mut rng = Rng(seed ^ 0x5eed_5eed);
    let mut pick = |lo: i64, hi: i64| lo + rng.below((hi - lo + 1) as u64) as i64;
    // Every window lies within [0, horizon], and the fourth slot (after the third) has a successor window.
    let limit = |hi: i64| hi.min(horizon);
    // The releases come in instant order (the timed release's order, note 38): a value queued behind one with a later instant would be
    // applied at that later instant, and the acceptance recorded at the call's return would credit samples before it.
    let first = window(&mut pick, limit(7));
    let second = successor_window(&mut pick, first, limit(11));
    let third = successor_window(&mut pick, second, limit(19));
    let mut env = vec![
        input(Input::Release(held(0, value(0, 8, 1), 1)), first, None),
        input(Input::Release(held(1, value(1, 12, 2), 1)), second, Some(0)),
        input(Input::Release(held(0, value(0, 20, 3), 1)), third, Some(1)),
        input(Input::Release(held(0, value(0, 20, 4), 1)), successor_window(&mut pick, third, limit(19)), Some(2)),
    ];
    let stop0 = pick(4, 40);
    let stop1 = pick(4, 40);
    env.push(input(Input::Stop { stream: 0, requested: stop0 }, window(&mut pick, limit(31)), None));
    env.push(input(Input::Stop { stream: 1, requested: stop1 }, window(&mut pick, limit(31)), None));
    env.push(input(Input::PlanCancel { stream: 1, origin: 24, epoch: 2, plan_cut: Some(80) }, window(&mut pick, limit(31)), None));
    env.push(input(Input::Request(Request::Move { channel: 0, to: 1 }), window(&mut pick, limit(31)), None));
    env.push(input(Input::Request(Request::Restore { channel: 0 }), window(&mut pick, limit(31)), None));
    if with_loss {
        let tick = pick(0, 95);
        env.push(loss((tick, tick)));
    }
    env.push(shutdown(horizon, timing));
    let cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 0), cell(Phase::Streaming(seg(0)), 1, None, 0)];
    // K = 2 over both chains and D = 3 per chain: four values and the Stop, plan change and loss reach both bounds in the swarm.
    let mut world = World::new(cells, env, bounded(timing, horizon, 2, 2, 3));
    running(&mut world, 0, vec![0]);
    running(&mut world, 1, vec![1]);
    Case { world, config: Config { name: "seeded configuration", budget: BUDGET, wall: WALL, witnesses: &[] } }
}

/// The brain step (counted from 0) that processes the first input matching `wanted`, on the
/// canonical path: the first enabled event that is not a tick, and a tick only when nothing else
/// is enabled (the order of the oracle tests' settle).
fn canonical_step(world: &World, wanted: impl Fn(&Input) -> bool) -> usize {
    let mut w = world.clone();
    let mut steps = 0;
    for _ in 0..100_000 {
        let events = w.enabled();
        let Some(event) = events.iter().find(|e| **e != Event::Tick).or(events.first()).cloned() else {
            panic!("the canonical path ends before the input");
        };
        if event == Event::BStep {
            if w.inbox.front().is_some_and(&wanted) {
                return steps;
            }
            steps += 1;
        }
        w.apply(&event);
    }
    panic!("the canonical path does not end within its budget");
}

/// The number of brain steps on the canonical path of `world` (the walk sweep of case 6 picks k among them).
pub(super) fn canonical_brain_steps(world: &World) -> usize {
    let mut w = world.clone();
    let mut steps = 0;
    for _ in 0..100_000 {
        let events = w.enabled();
        let Some(event) = events.iter().find(|e| **e != Event::Tick).or(events.first()).cloned() else {
            return steps;
        };
        if event == Event::BStep {
            steps += 1;
        }
        w.apply(&event);
    }
    panic!("the canonical path does not end within its budget");
}

/// The step that processes Stop in case 6's script in `variant` (run k = that step).
pub(super) fn case6_stop_step(variant: Variant) -> usize {
    canonical_step(&case1(variant).world, |input| matches!(input, Input::Stop { .. }))
}

/// The peak resident set of this process (`VmHWM`), for the measurement line.
fn peak_rss() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| status.lines().find(|line| line.starts_with("VmHWM:")).map(|line| line.trim_start_matches("VmHWM:").trim().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Explores the case at each horizon in turn, and prints one line per run. A run that overruns its
/// budget or wall limit ends the measurement: the larger horizons are not run.
fn measure(name: &str, build: impl Fn(i64) -> Case) {
    let mut overran = false;
    for horizon in HORIZONS {
        if overran {
            println!("measure {name} h={horizon}: not run (smaller horizon overran)");
            continue;
        }
        let Case { world, config } = build(horizon);
        let init = world.clone();
        let verdict = dst::explore(&config, world);
        let (label, reason) = match &verdict.outcome {
            dst::Outcome::Pass => ("pass", String::new()),
            dst::Outcome::BudgetExceeded { pending, .. } => {
                overran = true;
                let reason = if verdict.stats.states >= BUDGET { "budget" } else { "wall" };
                ("budget exceeded", format!(" reason={reason} pending={pending}"))
            }
            dst::Outcome::Violation { failure, .. } => ("violation", format!(" failure={failure:?}")),
            dst::Outcome::Cycle { .. } => ("cycle", String::new()),
            dst::Outcome::Vacuous { witness } => ("vacuous", format!(" witness={witness}")),
            dst::Outcome::Incomplete { message, .. } => ("incomplete", format!(" message={message}")),
        };
        println!(
            "measure {name} h={horizon}: {label}{reason} states={} transitions={} seconds={:.2} vmhwm={} reached={:?} missing={:?}",
            verdict.stats.states,
            verdict.stats.transitions,
            verdict.stats.elapsed.as_secs_f64(),
            peak_rss(),
            verdict.reached,
            verdict.missing
        );
        match &verdict.outcome {
            dst::Outcome::Violation { trace, .. } => println!("measure {name} h={horizon}: shrunk trace {:?}", dst::shrink(&init, trace)),
            dst::Outcome::Incomplete { trace, .. } => println!("measure {name} h={horizon}: shrunk trace {:?}", dst::shrink_incomplete(&init, trace)),
            dst::Outcome::Cycle { trace } => println!("measure {name} h={horizon}: trace {trace:?}"),
            _ => {}
        }
    }
}

/// The witnesses a trace reaches, from its initial state (the union over the trace's states).
fn witnesses_along(init: &World, trace: &[Event]) -> BTreeSet<&'static str> {
    witness_prefixes(init, trace).into_keys().collect()
}

/// Each witness the walk reaches, with the length of the shortest prefix of `trace` that reaches it.
pub(super) fn witness_prefixes(init: &World, trace: &[Event]) -> BTreeMap<&'static str, usize> {
    let mut model = init.clone();
    let mut reached: BTreeMap<&'static str, usize> = model.witnesses().into_iter().map(|witness| (witness, 0)).collect();
    for (at, event) in trace.iter().enumerate() {
        model.apply(event);
        for witness in model.witnesses() {
            reached.entry(witness).or_insert(at + 1);
        }
    }
    reached
}

/// Explores a case within `budget` states (the full budget also keeps the 180 s wall): a complete exploration must pass with every
/// declared witness, a budget overrun is reported and succeeds, any other outcome fails (45-experiment §10). Prints the report
/// (states, transitions, elapsed time, witnesses reached and missing).
fn explore_bounded(mut case: Case, budget: usize) -> Verdict<Event> {
    bound(&mut case.config, budget);
    let verdict = dst::explore(&case.config, case.world);
    println!("{}", verdict.report());
    dst::assert_bounded(&verdict);
    verdict
}

/// The micro search of a bounds case: bounded as [`explore_bounded`], and every declared witness must be reached, by the search or
/// else by one of 300 seeded walks of the same case (a budget overrun with a missing witness is not evidence that its window was tested).
fn explore_reaching(case: Case, budget: usize) {
    let (world, config) = (case.world.clone(), case.config.clone());
    let verdict = explore_bounded(case, budget);
    let mut reached: BTreeSet<&'static str> = verdict.reached.iter().copied().collect();
    for seed in 1..=300 {
        if config.witnesses.iter().all(|witness| reached.contains(witness)) {
            break;
        }
        let walked = dst::walk(&world, seed, WALK_LIMIT);
        assert!(walked.failure.is_none(), "{}: walk seed {seed} fails: {:?}", config.name, walked.failure);
        reached.extend(witnesses_along(&world, &walked.trace));
    }
    let missing: Vec<_> = config.witnesses.iter().filter(|witness| !reached.contains(*witness)).collect();
    assert!(missing.is_empty(), "{}: declared witnesses not reached by the search or 300 walks: {missing:?}", config.name);
}

/// A default and an ignored full micro test of a bounds case that require every declared witness ([`explore_reaching`]).
macro_rules! reaching_pair {
    ($small:ident, $full:ident, $case:expr) => {
        #[test]
        fn $small() {
            explore_reaching($case, SMALL_BUDGET);
        }

        #[test]
        #[ignore = "micro search: 2M-state budget or 180 s wall in release (E4a1)"]
        fn $full() {
            explore_reaching($case, BUDGET);
        }
    };
}

fn bound(config: &mut Config, budget: usize) {
    config.budget = budget;
    config.wall = if budget >= BUDGET { WALL } else { SMALL_WALL };
}

/// A default bounded test and an ignored full one (the 2,000,000-state budget, release) for one micro case.
macro_rules! micro_pair {
    ($small:ident, $full:ident, $case:expr) => {
        #[test]
        fn $small() {
            explore_bounded($case, SMALL_BUDGET);
        }

        #[test]
        #[ignore = "micro search: 2M-state budget or 180 s wall in release (E1b part 3d)"]
        fn $full() {
            explore_bounded($case, BUDGET);
        }
    };
}

/// The seeds a walk sweep runs: `EZSDR_SEEDS` as `a-b` (or a single seed), else the default range.
fn seed_range() -> (u64, u64) {
    let spec = std::env::var("EZSDR_SEEDS").unwrap_or_else(|_| SWEEP_DEFAULT.to_string());
    let (first, last) = spec.split_once('-').unwrap_or((spec.as_str(), spec.as_str()));
    let first: u64 = first.trim().parse().expect("EZSDR_SEEDS is a-b with decimal seeds");
    let last: u64 = last.trim().parse().expect("EZSDR_SEEDS is a-b with decimal seeds");
    (first, last)
}

/// One walk of `case` from `seed` (with the seed's weights): the failure it ends at, if any, with its shrunk trace.
fn walk_failure(case: &Case, seed: u64) -> Option<(String, Vec<Event>)> {
    judge(case, &dst::walk(&case.world, seed, WALK_LIMIT))
}

/// The failure of a finished walk. A walk that ends at a terminal state the epilogue left waiting is a failure, as the
/// explorer reports it; so is one that runs out of events without a terminal state (a livelock candidate).
fn judge(case: &Case, walked: &dst::Walk<Event>) -> Option<(String, Vec<Event>)> {
    if let Some(failure) = &walked.failure {
        return Some((format!("{failure:?}"), dst::shrink(&case.world, &walked.trace)));
    }
    if walked.trace.len() >= WALK_LIMIT {
        return Some((format!("no terminal state within {WALK_LIMIT} events"), walked.trace.iter().take(40).cloned().collect()));
    }
    let mut end = case.world.clone();
    for event in &walked.trace {
        end.apply(event);
    }
    match end.incomplete() {
        Some(message) if end.enabled().is_empty() => Some((format!("Incomplete: {message}"), dst::shrink_incomplete(&case.world, &walked.trace))),
        _ => None,
    }
}

/// Walks each seed of `EZSDR_SEEDS` (default `1-10000`, stopped after 30 minutes) through `build`.
/// Each walk uses the seed's class weights (`dst::weights_for`), which every report names with the seed.
/// Prints each failing seed with its weights and shrunk trace, and the seeds run, the wall time, the witness
/// union with the number of seeds that reached each witness and the first seed (with its weights) that did. The sweep
/// fails if any seed fails, and fails as vacuous when a witness the cases declare is reached by no walk. It is
/// evidence of the failures it finds, never of completeness.
fn sweep(name: &str, build: impl Fn(u64) -> Case) {
    sweep_seeds(name, seed_range(), build);
}

fn sweep_seeds(name: &str, (first, last): (u64, u64), build: impl Fn(u64) -> Case) -> BTreeMap<&'static str, (u64, Vec<Event>)> {
    let started = Instant::now();
    let mut reached: BTreeMap<&'static str, (u64, u64)> = BTreeMap::new();
    let mut traces: BTreeMap<&'static str, (u64, Vec<Event>)> = BTreeMap::new();
    let mut declared: BTreeSet<&'static str> = BTreeSet::new();
    let mut failing = Vec::new();
    let mut ran = 0u64;
    for seed in first..=last {
        if started.elapsed() >= SWEEP_WALL {
            println!("{name}: wall stop after {ran} seeds");
            break;
        }
        let case = build(seed);
        declared.extend(case.config.witnesses.iter().copied());
        ran += 1;
        let walked = dst::walk(&case.world, seed, WALK_LIMIT);
        if let Some((failure, shrunk)) = judge(&case, &walked) {
            println!("{name} seed {seed} weights {:?}: {failure}\n  shrunk trace {shrunk:?}", dst::weights_for(seed));
            failing.push(seed);
        }
        for (witness, length) in witness_prefixes(&case.world, &walked.trace) {
            reached.entry(witness).or_insert((0, seed)).0 += 1;
            traces.entry(witness).or_insert_with(|| (seed, walked.trace[..length].to_vec()));
        }
    }
    for (witness, (seed, trace)) in &traces {
        println!("{name}: first trace to {witness}, seed {seed} weights {:?}: {trace:?}", dst::weights_for(*seed));
    }
    let unreached: Vec<&'static str> = declared.iter().filter(|witness| !reached.contains_key(*witness)).copied().collect();
    let counts: Vec<String> = reached.iter().map(|(witness, (count, first))| format!("{witness}: {count} seeds, first seed {first} weights {:?}", dst::weights_for(*first).0)).collect();
    println!("{name}: {ran} seeds ({first}-{last}), wall {:?}, failing seeds {failing:?}, witnesses reached {counts:?}, unreached declared witnesses {unreached:?}", started.elapsed());
    assert!(failing.is_empty(), "{name}: failing seeds {failing:?}");
    assert!(unreached.is_empty(), "{name}: Vacuous: declared witnesses never reached over {ran} seeds: {unreached:?}");
    traces
}

/// The case builders, each in one variant. Case 6 is the panic at k = 0.
fn all_cases(variant: Variant) -> Vec<Case> {
    vec![
        case1(variant),
        case2(variant),
        case3(variant),
        case4(variant),
        case5(variant),
        case6(variant, 0),
        case7a(variant),
        case7b(variant),
        case7b_late_restore(variant),
        case7c(variant),
        case8(variant),
        case9(variant),
        case10(variant),
        case11(variant),
        case12a(variant),
        case12b(variant),
    ]
}

#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case1() {
    measure("case 1", |h| case1(Variant::Reduced(h)));
}

#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case2() {
    measure("case 2", |h| case2(Variant::Reduced(h)));
}

#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case3() {
    measure("case 3", |h| case3(Variant::Reduced(h)));
}

#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case4() {
    measure("case 4", |h| case4(Variant::Reduced(h)));
}

#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case5() {
    measure("case 5", |h| case5(Variant::Reduced(h)));
}

/// Case 6 at the two declared steps: the first brain step, and the step that processes Stop.
#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case6() {
    measure("case 6 k=first", |h| case6(Variant::Reduced(h), 0));
    measure("case 6 k=stop", |h| case6(Variant::Reduced(h), case6_stop_step(Variant::Reduced(h))));
}

#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case7a() {
    measure("case 7a", |h| case7a(Variant::Reduced(h)));
}

#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case7b() {
    measure("case 7b", |h| case7b(Variant::Reduced(h)));
}

#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case7c() {
    measure("case 7c", |h| case7c(Variant::Reduced(h)));
}

#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case8() {
    measure("case 8", |h| case8(Variant::Reduced(h)));
}

#[test]
#[ignore = "measurement (stage E1b part 3a): run through rdev, one case per command"]
fn measure_case9() {
    measure("case 9", |h| case9(Variant::Reduced(h)));
}

// The micro variants (45-experiment §10: bounded searches). Each case has a default test with a small debug budget and an
// ignored `_full` test with the 2,000,000-state budget (release, one per command); both fail only on a `Violation`, `Cycle`,
// `Incomplete` or `Vacuous` verdict, and a budget overrun succeeds with the report. Full runs of E1b part 3d (release, sim02: states
// 2,000,000 unless noted; transitions, seconds, witnesses missing):
// case 1: 5,268,267 tr, 64 s, W1a W1b; case 2: 6,023,729 tr, 123 s; case 3: 5,016,081 tr, 59 s, W3a W3b; case 4: 5,054,783 tr, 65 s;
// case 5: 5,485,936 tr, 69 s; case 6 stop: 5,149,503 tr, 61 s; case 7a: 6,336,056 tr, 153 s; case 7b: 5,209,646 tr, 62 s, W7b3;
// case 7c: 5,079,303 tr, 61 s; case 8: wall at 1,932,400 states, 6,144,000 tr, 180 s, W8a W8b; case 9: 5,929,435 tr, 84 s, W9a.refused.
micro_pair!(case1_value_cut_stop_micro, case1_value_cut_stop_micro_full, case1(Variant::Micro));
micro_pair!(case2_loss_everywhere_micro, case2_loss_everywhere_micro_full, case2(Variant::Micro));
micro_pair!(case3_plan_cancel_recv_stop_micro, case3_plan_cancel_recv_stop_micro_full, case3(Variant::Micro));
micro_pair!(case4_equal_time_pair_stopping_micro, case4_equal_time_pair_stopping_micro_full, case4(Variant::Micro));
micro_pair!(case5_blocked_settings_drain_deadline_micro, case5_blocked_settings_drain_deadline_micro_full, case5(Variant::Micro));
// Case 6 at its first step completes (177,421 states, about 6 s in debug), so its default test runs the full budget and requires every witness.
#[test]
fn case6_brain_panic_first_micro() {
    explore_bounded(case6(Variant::Micro, 0), BUDGET);
}

micro_pair!(case6_brain_panic_stop_micro, case6_brain_panic_stop_micro_full, case6(Variant::Micro, case6_stop_step(Variant::Micro)));
micro_pair!(case7a_handoff_loss_late_restart_micro, case7a_handoff_loss_late_restart_micro_full, case7a(Variant::Micro));
micro_pair!(case7b_flush_final_stop_restore_micro, case7b_flush_final_stop_restore_micro_full, case7b(Variant::Micro));
micro_pair!(case7b_late_restore_micro, case7b_late_restore_micro_full, case7b_late_restore(Variant::Micro));
micro_pair!(case7c_failed_begin_debt_micro, case7c_failed_begin_debt_micro_full, case7c(Variant::Micro));
micro_pair!(case8_lifecycle_across_loss_micro, case8_lifecycle_across_loss_micro_full, case8(Variant::Micro));
micro_pair!(case9_pending_move_partial_refused_micro, case9_pending_move_partial_refused_micro_full, case9(Variant::Micro));
reaching_pair!(case10_k_d_and_device_time_micro, case10_k_d_and_device_time_micro_full, case10(Variant::Micro));
reaching_pair!(case11_shared_k_and_cancel_micro, case11_shared_k_and_cancel_micro_full, case11(Variant::Micro));
reaching_pair!(case12a_capacity_timer_stopping_micro, case12a_capacity_timer_stopping_micro_full, case12a(Variant::Micro));
reaching_pair!(case12b_pinned_drain_progress_guard_micro, case12b_pinned_drain_progress_guard_micro_full, case12b(Variant::Micro));

/// The seeded configuration at horizon 32: 50 walks, each asserted free of failure, and the union of
/// the witnesses they reach (a walk is evidence of the failures it finds, never of completeness).
#[test]
fn seeded_config_reduced() {
    let mut union: BTreeSet<&'static str> = BTreeSet::new();
    for seed in 1..=50 {
        let case = seeded(seed, false, 32);
        if let Some((failure, shrunk)) = walk_failure(&case, seed) {
            panic!("seed {seed} weights {:?}: {failure}; shrunk trace {shrunk:?}", dst::weights_for(seed));
        }
        union.extend(witnesses_along(&case.world, &dst::walk(&case.world, seed, WALK_LIMIT).trace));
    }
    println!("seeded_config_reduced: 50 walks at horizon 32, witness union {union:?}");
}

/// The bounds cases at their full horizon, 500 seeds each (a default test, so a mutation row can be killed by the walks): no walk fails the
/// oracles (I11′, P6, I17, the capacity-timer step check among them). A walk is evidence of the failures it finds, not of completeness.
#[test]
fn bounds_cases_walk_clean() {
    for (name, build) in [("case 10", case10 as fn(Variant) -> Case), ("case 11", case11), ("case 12a", case12a), ("case 12b", case12b), ("case 5", case5), ("case 7c", case7c)] {
        for seed in 1..=500 {
            let case = build(Variant::Full);
            if let Some((failure, shrunk)) = walk_failure(&case, seed) {
                panic!("{name} seed {seed} weights {:?}: {failure}; shrunk trace {shrunk:?}", dst::weights_for(seed));
            }
        }
    }
}

/// The walk sweeps (ignored): one per case at its full horizon, and one for the seeded configuration
/// at horizon 96 with its device loss. Seeds come from `EZSDR_SEEDS`.
#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case1_value_cut_stop_walks() {
    sweep("case 1", |_| case1(Variant::Full));
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case2_loss_everywhere_walks() {
    sweep("case 2", |_| case2(Variant::Full));
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case3_plan_cancel_recv_stop_walks() {
    sweep("case 3", |_| case3(Variant::Full));
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case4_equal_time_pair_stopping_walks() {
    sweep("case 4", |_| case4(Variant::Full));
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case5_blocked_settings_drain_deadline_walks() {
    sweep("case 5", |_| case5(Variant::Full));
}

/// Case 6's walk takes k from the seed, over the brain steps of the canonical path.
#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case6_brain_panic_every_step_walks() {
    sweep("case 6", |seed| {
        let steps = canonical_brain_steps(&case1(Variant::Full).world).max(1);
        case6(Variant::Full, (seed.saturating_sub(1) as usize) % steps)
    });
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case7a_handoff_loss_late_restart_walks() {
    sweep("case 7a", |_| case7a(Variant::Full));
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case7b_flush_final_stop_restore_walks() {
    sweep("case 7b", |_| case7b(Variant::Full));
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case7b_late_restore_walks() {
    sweep("case 7b late restore", |_| case7b_late_restore(Variant::Full));
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case7c_failed_begin_debt_walks() {
    sweep("case 7c", |_| case7c(Variant::Full));
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case8_lifecycle_across_loss_walks() {
    sweep("case 8", |_| case8(Variant::Full));
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case9_pending_move_partial_refused_walks() {
    sweep("case 9", |_| case9(Variant::Full));
}

#[test]
#[ignore = "walk sweep (E4a1): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case10_k_d_and_device_time_walks() {
    sweep("case 10", |_| case10(Variant::Full));
}

#[test]
#[ignore = "walk sweep (E4a1): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case11_shared_k_and_cancel_walks() {
    sweep("case 11", |_| case11(Variant::Full));
}

#[test]
#[ignore = "walk sweep (E4a1): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case12a_capacity_timer_stopping_walks() {
    sweep("case 12a", |_| case12a(Variant::Full));
}

#[test]
#[ignore = "walk sweep (E4a1): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn case12b_pinned_drain_progress_guard_walks() {
    sweep("case 12b", |_| case12b(Variant::Full));
}

#[test]
#[ignore = "walk sweep (stage E1b part 3b): EZSDR_SEEDS=a-b, run on sim02/sim03"]
fn seeded_config_walks() {
    sweep("seeded configuration", |seed| seeded(seed, true, 96));
}

/// The builders of the full-horizon cases, by name, for the thread runtime's replay (stage E2a, S1). Case 6 takes its `k` from the
/// seed over the brain steps of the canonical path, as its walk sweep does.
/// A case builder: the case for a seed.
pub(super) type Builder = Box<dyn Fn(u64) -> Case>;

pub(super) fn full_builders() -> Vec<(&'static str, Builder)> {
    vec![
        ("case 1", Box::new(|_| case1(Variant::Full))),
        ("case 2", Box::new(|_| case2(Variant::Full))),
        ("case 3", Box::new(|_| case3(Variant::Full))),
        ("case 4", Box::new(|_| case4(Variant::Full))),
        ("case 5", Box::new(|_| case5(Variant::Full))),
        (
            "case 6",
            Box::new(|seed| {
                let steps = canonical_brain_steps(&case1(Variant::Full).world).max(1);
                case6(Variant::Full, (seed.saturating_sub(1) as usize) % steps)
            }),
        ),
        ("case 7a", Box::new(|_| case7a(Variant::Full))),
        ("case 7b", Box::new(|_| case7b(Variant::Full))),
        ("case 7b late restore", Box::new(|_| case7b_late_restore(Variant::Full))),
        ("case 7c", Box::new(|_| case7c(Variant::Full))),
        ("case 8", Box::new(|_| case8(Variant::Full))),
        ("case 9", Box::new(|_| case9(Variant::Full))),
        ("case 10", Box::new(|_| case10(Variant::Full))),
        ("case 11", Box::new(|_| case11(Variant::Full))),
        ("case 12a", Box::new(|_| case12a(Variant::Full))),
        ("case 12b", Box::new(|_| case12b(Variant::Full))),
        ("seeded configuration", Box::new(|seed| seeded(seed, true, 96))),
    ]
}

/// A probe: a `World` with per-state and per-step checks of one case, and an extra witness. The
/// checks are written here, not in `World`, and see only what the world exposes.
#[derive(Clone)]
pub(super) struct Probe {
    pub(super) world: World,
    pub(super) state: StateCheck,
    pub(super) step: StepCheck,
    pub(super) witness: WitnessFn,
    /// Checked at every terminal state, after the world's own progress checks.
    pub(super) end: StateCheck,
}

impl Probe {
    /// A probe with no extra checks; a test sets the ones it wants.
    pub(super) fn new(world: World) -> Probe {
        Probe { world, state: no_check, step: no_step, witness: no_witness, end: no_check }
    }
}

pub(super) type StateCheck = fn(&World) -> Result<(), String>;
pub(super) type StepCheck = fn(&World, &Event, &World) -> Result<(), String>;
pub(super) type WitnessFn = fn(&World) -> Vec<&'static str>;

impl Model for Probe {
    type Event = Event;
    type Key = String;

    fn key(&self) -> String {
        self.world.key()
    }

    fn enabled(&self) -> Vec<Event> {
        self.world.enabled()
    }

    fn apply(&mut self, event: &Event) {
        self.world.apply(event);
    }

    fn safety(&self) -> Result<(), String> {
        self.world.safety()?;
        (self.state)(&self.world)
    }

    fn transition(&self, event: &Event, after: &Probe) -> Result<(), String> {
        self.world.transition(event, &after.world)?;
        (self.step)(&self.world, event, &after.world)
    }

    fn progress(&self) -> Result<(), String> {
        self.world.progress()?;
        (self.end)(&self.world)
    }

    fn witnesses(&self) -> Vec<&'static str> {
        let mut names = self.world.witnesses();
        names.extend((self.witness)(&self.world));
        names
    }

    fn incomplete(&self) -> Option<String> {
        self.world.incomplete()
    }
}

/// Explores `case` with the probe's checks within `budget` states, as [`explore_bounded`].
fn probed(case: &Case, budget: usize, state: StateCheck, step: StepCheck, witness: WitnessFn) -> Verdict<Event> {
    let mut config = case.config.clone();
    bound(&mut config, budget);
    let probe = Probe { state, step, witness, ..Probe::new(case.world.clone()) };
    let verdict = dst::explore(&config, probe);
    println!("{}", verdict.report());
    dst::assert_bounded(&verdict);
    verdict
}

fn no_check(_: &World) -> Result<(), String> {
    Ok(())
}

/// A fired timer's drain deadline is recorded as observed in the successor (P1 depends on it).
fn timer_recorded(before: &World, event: &Event, after: &World) -> Result<(), String> {
    if let Event::Timer(i) = event {
        let (Timer::Drain(stream), at) = before.timers[*i] else { return Ok(()) };
        if !after.monitor.drain_observed.contains(&(stream, at)) {
            return Err(format!("timer {i} fired for stream {stream} at {at} without recording its drain deadline"));
        }
    }
    Ok(())
}

fn no_step(_: &World, _: &Event, _: &World) -> Result<(), String> {
    Ok(())
}

fn no_witness(_: &World) -> Vec<&'static str> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seeds 18 and 27 walked to failures (I15: a Restore decided past a queued value; I3: a Move
    /// issued after S took a value) before the committed-value rule. They walk clean now.
    #[test]
    fn harness_seeds_18_and_27_walk_clean() {
        for seed in [18, 27] {
            let case = seeded(seed, false, 32);
            assert!(walk_failure(&case, seed).is_none(), "seed {seed}: {:?}", walk_failure(&case, seed));
        }
    }

    /// Every builder's script is well formed in every variant: each window is not empty, each `after`
    /// names an earlier slot, a shutdown is present, and every witness is declared.
    #[test]
    fn harness_case_scripts_are_well_formed() {
        let variants = [Variant::Full, Variant::Micro, Variant::Reduced(8), Variant::Reduced(16), Variant::Reduced(24)];
        for variant in variants {
            for case in all_cases(variant) {
                assert!(!case.config.witnesses.is_empty(), "{}", case.config.name);
                assert!(case.world.env.iter().any(|slot| matches!(slot.input, EnvInput::Input(Input::Shutdown))), "{}: no shutdown", case.config.name);
                for (i, slot) in case.world.env.iter().enumerate() {
                    assert!(slot.earliest <= slot.latest, "{}: empty window at {i}", case.config.name);
                    assert!(slot.after.is_none_or(|j| j < i), "{}: after at {i}", case.config.name);
                }
            }
        }
    }

    /// The seeded configuration is built from its seed: the same seed gives the same script, and
    /// another seed a different one for some input.
    #[test]
    fn harness_seeded_script_depends_on_seed() {
        let windows = |seed: u64| seeded(seed, true, 32).world.env.iter().map(|slot| (slot.earliest, slot.latest)).collect::<Vec<_>>();
        assert_eq!(windows(3), windows(3));
        assert!((0..8).any(|seed| windows(seed) != windows(3)), "every seed gives the same script");
    }

    /// Sol E1 finding 6: a sweep fails as vacuous when a declared witness is reached by no walk, and passes when every one is.
    #[test]
    fn harness_sweep_fails_on_an_unreached_declared_witness() {
        let build = |witnesses: &'static [&'static str]| {
            move |_: u64| {
                let mut case = case1(Variant::Micro);
                case.config.witnesses = witnesses;
                case
            }
        };
        let reached = std::panic::catch_unwind(|| sweep_seeds("reached", (1, 2), build(&["publication"])));
        assert!(reached.is_ok(), "a declared witness the walks reach passes");
        // The sweep keeps the first trace to each witness it reached, and the trace reaches it when replayed.
        let traces = reached.expect("passed");
        let (seed, trace) = &traces["publication"];
        assert!(*seed >= 1 && witnesses_along(&case1(Variant::Micro).world, trace).contains("publication"));
        let missed = std::panic::catch_unwind(|| sweep_seeds("missed", (1, 2), build(&["a witness no walk reaches"])));
        assert!(missed.is_err(), "an unreached declared witness fails the sweep");
    }

    /// Sol E1 finding 6: case 7b's failed final flush stop needs call faults in every variant.
    #[test]
    fn harness_case7b_enables_call_faults_in_every_variant() {
        for variant in [Variant::Full, Variant::Micro, Variant::Reduced(8)] {
            assert!(case7b(variant).world.params.call_faults && case7b_late_restore(variant).world.params.call_faults, "{variant:?}");
        }
    }

    /// Case 8's lifecycle requests come in the plan's order in every variant, Close, Open, StartNext: a start on a closed
    /// stream, or an Open before the Close, is not a request the plan makes (the weighted walk of seed 3259 delivered the Open
    /// before the Close, and R then began on the closed handle: a harness script error, I9).
    #[test]
    fn harness_case8_start_follows_open() {
        for case in [case8(Variant::Full), case8(Variant::Reduced(0)), case8(Variant::Micro)] {
            let env = &case.world.env;
            assert!(matches!(env[2].input, EnvInput::Input(Input::Request(Request::Close { .. }))));
            assert_eq!(env[3].after, Some(2));
            assert!(matches!(env[3].input, EnvInput::Input(Input::Request(Request::Open { .. }))));
            assert!(matches!(env[4].input, EnvInput::Input(Input::Request(Request::StartNext { .. }))));
            assert_eq!(env[4].after, Some(3));
        }
    }

    /// Case 8 has a second stream, 1 on channel 1, streaming from 0 with no cut: its R can publish a loss while R0's lifecycle waits for G (W8a, W8b).
    #[test]
    fn harness_case8_has_a_streaming_second_stream() {
        for case in [case8(Variant::Full), case8(Variant::Reduced(0)), case8(Variant::Micro)] {
            let world = &case.world;
            assert_eq!(world.cells.len(), 2);
            assert!(matches!(world.cells[1].phase, Phase::Streaming(_)) && world.cells[1].cut.is_none() && world.cells[1].channels.contains(1));
            assert_eq!(world.rx[1].handle, Some(1));
            assert!(world.device.dsp.contains_key(&1));
        }
    }

    /// Case 7b's second Restore comes after the first and from the cut on, so it can arrive after the failed final stop (W7b3).
    #[test]
    fn harness_case7b_has_a_late_second_restore() {
        for (variant, original) in [(Variant::Full, case7b(Variant::Full)), (Variant::Reduced(0), case7b(Variant::Reduced(0))), (Variant::Micro, case7b(Variant::Micro))] {
            assert_eq!(original.world.env.len(), 4, "the original script keeps its four inputs");
            let case = case7b_late_restore(variant);
            let env = &case.world.env;
            assert_eq!(env.len(), 5);
            assert!(matches!(env[3].input, EnvInput::Input(Input::Request(Request::Restore { channel: 0 }))) && env[3].after == Some(2));
        }
    }

    /// Sol's second re-review P1: an Off stream with a cut is told to start, and Shutdown is handled before the start's result;
    /// the cut can be reached before the brain handles `Armed`. Whatever the order, the brain must send the stop and R must
    /// have issued the device Stop (the streamer no longer running) before it takes Terminate.
    #[test]
    fn harness_shutdown_stops_the_device_before_terminate() {
        fn running(w: &World) -> bool {
            w.device.streamers.get(&0).is_some_and(|streamer| streamer.next.is_some() && streamer.end.is_none())
        }
        fn stop_before_terminate(before: &World, event: &Event, after: &World) -> Result<(), String> {
            if matches!(event, Event::RCmd(0)) && before.rx[0].fifo.front() == Some(&crate::brain::core::RxOp::Terminate) && running(after) {
                return Err(format!("R took Terminate with the device stream running: {:?}", after.trace));
            }
            Ok(())
        }
        let timing = Variant::Micro.timing();
        let env = vec![input(Input::Request(Request::StartNext { stream: 0, at: 0 }), (0, 1), None), shutdown(3, timing)];
        let mut world = World::new(vec![cell(Phase::Off, 0, Some(2), 0)], env, params(timing, 3, 1, false, None));
        world.device.open(0, vec![0]);
        world.rx[0].handle = Some(1);
        let case = Case { world, config: config(Variant::Micro, ["case 8 full", "case 8 reduced", "case 8 micro"], &[]) };
        probed(&case, 400_000, no_check, stop_before_terminate, no_witness);
    }

    /// Every seeded window lies in `[0, horizon]` (the shutdown excepted), and a slot's window does not end
    /// before its predecessor's starts or begins before it: the case stays deliverable (seeds 1..200).
    #[test]
    fn harness_seeded_windows_fit_and_chain() {
        for seed in 1..=200 {
            for (with_loss, horizon) in [(false, 32), (true, 96)] {
                let case = seeded(seed, with_loss, horizon);
                let env = &case.world.env;
                for (i, slot) in env.iter().enumerate() {
                    if matches!(slot.input, EnvInput::Input(Input::Shutdown)) {
                        continue;
                    }
                    assert!(slot.earliest >= 0 && slot.latest <= horizon, "seed {seed}: slot {i} window {:?} leaves [0, {horizon}]", (slot.earliest, slot.latest));
                    if let Some(j) = slot.after {
                        let pred = &env[j];
                        assert!(pred.earliest <= slot.earliest && pred.latest <= slot.latest, "seed {seed}: slot {i} can end before its predecessor {j} starts");
                    }
                }
            }
        }
    }

    /// Case 6's declared step: the step that processes Stop is a later brain step than the first.
    #[test]
    fn harness_case6_stop_step_is_after_the_first() {
        assert!(case6_stop_step(Variant::Micro) > 0);
        assert!(case6_stop_step(Variant::Reduced(8)) > 0);
    }

    /// The epilogue covers the latest drain deadline past the horizon plus one recv timeout (cut plus
    /// drain window, then the recv timeout): every builder, variant and the seeded configuration. The
    /// deadline is read from the cells' cuts, the plan cuts and the stop requests; a deadline inside
    /// the horizon is covered by the run's own epilogue.
    #[test]
    fn harness_epilogue_covers_deadlines() {
        let mut cases: Vec<Case> = Vec::new();
        // The measured Reduced variants are not checked: case 3 at horizon 8 keeps its plan cut 48 under the
        // shared epilogue 32, so the measurement reports a deadline its epilogue does not cover.
        for variant in [Variant::Full, Variant::Micro] {
            cases.extend(all_cases(variant));
        }
        cases.push(seeded(1, true, 32));
        cases.push(seeded(1, true, 96));
        for case in cases {
            let p = case.world.params;
            let mut cuts: Vec<i64> = case.world.cells.iter().filter_map(|c| c.cut).collect();
            cuts.extend(case.world.env.iter().filter_map(|slot| match &slot.input {
                EnvInput::Input(Input::PlanCancel { plan_cut, .. }) => *plan_cut,
                EnvInput::Input(Input::Stop { requested, .. }) => Some(*requested),
                _ => None,
            }));
            let Some(latest_cut) = cuts.into_iter().max() else { continue };
            let needed = (latest_cut + p.drain_window - p.horizon).max(0) + p.recv_timeout;
            assert!(p.epilogue >= needed, "{}: epilogue {} is shorter than the latest drain deadline (cut {latest_cut}) plus one recv timeout ({needed})", case.config.name, p.epilogue);
        }
    }

    /// Section C, no Stop: in every visited state of case 1 (micro), no `S.effect` is enabled while
    /// nothing is in hand (causality, section 3.2).
    /// Release, sim02 (E1b part 3c): budget at 2,000,000 states, 5,268,267 transitions, 107 s (case 1 micro).
    fn harness_no_effect_before_take(budget: usize) {
        let state = |w: &World| {
            if w.enabled().iter().any(|event| matches!(event, Event::SEffect(_))) && w.settings.hand.is_none() {
                return Err("S.effect enabled with nothing in hand".to_string());
            }
            Ok(())
        };
        probed(&case1(Variant::Micro), budget, state, no_step, no_witness);
    }

    /// Section C: in every visited state of case 1 (micro), every enabled event changes the key (no
    /// state-preserving event, which would be a false cycle or a missed livelock).
    /// Release, sim02 (E1b part 3c): the 180 s wall stops it at 1,291,766 states (3,433,472 transitions), not complete.
    fn harness_every_event_changes_state(budget: usize) {
        let step = |before: &World, event: &Event, after: &World| {
            if before.key() == after.key() {
                return Err(format!("{event:?} leaves the state unchanged"));
            }
            Ok(())
        };
        probed(&case1(Variant::Micro), budget, no_check, step, no_witness);
    }

    /// Section C: in every visited state of case 1 (micro), `T` changes only by a tick, by exactly one sample.
    /// Release, sim02 (E1b part 3c): budget at 2,000,000 states, 5,268,267 transitions, 111 s (case 1 micro).
    fn harness_time_only_advances_on_tick(budget: usize) {
        let step = |before: &World, event: &Event, after: &World| {
            let expected = before.device.t + i64::from(*event == Event::Tick);
            if after.device.t != expected {
                return Err(format!("{event:?} moves T from {} to {}", before.device.t, after.device.t));
            }
            Ok(())
        };
        probed(&case1(Variant::Micro), budget, no_check, step, no_witness);
    }

    /// Section C: in every visited state of case 1 (micro), a Tick is not enabled while an undelivered
    /// input's window has closed (its latest is at or before `T`).
    /// Release, sim02 (E1b part 3c): budget at 2,000,000 states, 5,268,267 transitions, 91 s (case 1 micro).
    fn harness_tick_cannot_pass_mandatory_input(budget: usize) {
        let state = |w: &World| match w.env.iter().find(|slot| !slot.delivered && slot.latest <= w.device.t) {
            Some(slot) if w.enabled().contains(&Event::Tick) => Err(format!("Tick enabled past the window of {:?}", slot.input)),
            _ => Ok(()),
        };
        probed(&case1(Variant::Micro), budget, state, no_step, no_witness);
    }

    /// Section C: in every visited state of case 7a (micro, the case with a LateCommand, the only one
    /// whose script has clock events), `Clock.lock` is enabled only while `L` is free, `Clock.read`
    /// only while `L` is held by the clock, and `L` is free after `Clock.unlock`.
    /// Release, sim02 (E1b part 3c): budget at 2,000,000 states, 6,336,056 transitions, 73 s (case 7a micro).
    fn harness_clock_read_holds_native_lock(budget: usize) {
        let state = |w: &World| {
            let enabled = w.enabled();
            if enabled.iter().any(|event| matches!(event, Event::ClockLock(_))) && w.native.is_some() {
                return Err("Clock.lock enabled while L is held".to_string());
            }
            if enabled.contains(&Event::ClockRead) && w.native != Some(Holder::Clock) {
                return Err("Clock.read enabled outside L".to_string());
            }
            Ok(())
        };
        let step = |_: &World, event: &Event, after: &World| {
            if *event == Event::ClockUnlock && after.native.is_some() {
                return Err("L is still held after Clock.unlock".to_string());
            }
            Ok(())
        };
        let witness = |w: &World| if w.enabled().contains(&Event::ClockRead) { vec!["probe: clock read enabled"] } else { Vec::new() };
        let verdict = probed(&case7a(Variant::Micro), budget, state, step, witness);
        if matches!(verdict.outcome, dst::Outcome::Pass) {
            assert!(verdict.reached.contains(&"probe: clock read enabled"), "no clock read was reached: {:?}", verdict.reached);
        }
    }

    /// Section C: in every visited state of case 1 (micro), each `Done` in the brain inbox has its
    /// command result already written (`S.result` before `S.send`).
    /// Release, sim02 (E1b part 3c): budget at 2,000,000 states, 5,268,267 transitions, 91 s (case 1 micro).
    fn harness_result_precedes_send(budget: usize) {
        let state = |w: &World| {
            for input in &w.inbox {
                if let Input::Done { id, terminal, .. } = input {
                    if w.monitor.terminals.get(id) != Some(terminal) {
                        return Err(format!("Done for {id} ({terminal:?}) is delivered before its command result is written"));
                    }
                }
            }
            Ok(())
        };
        probed(&case1(Variant::Micro), budget, state, no_step, no_witness);
    }

    /// Section C: in every visited state of case 2 (micro), `Loss.gate` is enabled only when the
    /// publishing actor does not hold `L`, and `Loss.mark` only after its `Loss.ungate` released the
    /// gate. The run reaches a state where `Loss.gate` is enabled while another actor holds `L`.
    /// Release, sim02 (E1b part 3c): budget at 2,000,000 states, 6,023,729 transitions, 83 s (case 2 micro).
    fn harness_loss_publish_releases_l_first(budget: usize) {
        let state = |w: &World| {
            for event in w.enabled() {
                if let Event::Pub(holder, PubStep::Gate) = event {
                    if w.native == Some(holder) {
                        return Err(format!("{holder:?} publishes the loss while it holds L"));
                    }
                }
            }
            Ok(())
        };
        let step = |before: &World, event: &Event, _: &World| match event {
            Event::Pub(holder, PubStep::Mark) if before.gate == Some(*holder) => Err(format!("{holder:?} marks the loss before its ungate")),
            _ => Ok(()),
        };
        let witness = |w: &World| {
            let elsewhere = w.enabled().iter().any(|event| matches!(event, Event::Pub(holder, PubStep::Gate) if w.native.is_some_and(|l| l != *holder)));
            if elsewhere { vec!["probe: loss gate while another holds L"] } else { Vec::new() }
        };
        let verdict = probed(&case2(Variant::Micro), budget, state, step, witness);
        if matches!(verdict.outcome, dst::Outcome::Pass) {
            assert!(verdict.reached.contains(&"probe: loss gate while another holds L"), "the interleaving was not reached: {:?}", verdict.reached);
        }
    }

    /// Section C: in every visited state of case 8 (micro), no receive executor that holds `G` has
    /// its loss-gate step at the front of its program (the inline path sets `F` inside `G`, no recursion).
    fn harness_lifecycle_inline_loss_publish_no_recursion(budget: usize) {
        let state = |w: &World| match (0..w.rx.len()).find(|&k| w.gate == Some(Holder::Rx(k)) && w.rx[k].program.front() == Some(&RStep::Pub(PubStep::Gate))) {
            Some(k) => Err(format!("R{k} holds G and publishes the loss gate again")),
            None => Ok(()),
        };
        probed(&case8(Variant::Micro), budget, state, no_step, no_witness);
    }

    /// Section C: in every visited state of case 5 (micro), a `Timer` is enabled only at or after its
    /// deadline (or at the end of the epilogue, where a deadline beyond the run is observed).
    /// Release, sim02 (E1b part 3c): budget at 2,000,000 states, 5,674,468 transitions, 67 s (case 5 micro).
    fn harness_timer_never_early_in_case5(budget: usize) {
        let state = |w: &World| {
            for event in w.enabled() {
                if let Event::Timer(i) = event {
                    let at = w.timers[i].1;
                    if w.device.t < at && !w.at_end() {
                        return Err(format!("timer {i} is enabled at T = {} before its deadline {at}", w.device.t));
                    }
                }
            }
            Ok(())
        };
        probed(&case5(Variant::Micro), budget, state, timer_recorded, no_witness);
    }

    #[test]
    fn harness_no_effect_before_take_bounded() {
        harness_no_effect_before_take(SMALL_BUDGET);
    }

    #[test]
    #[ignore = "explores a micro case to its 2M budget: release only"]
    fn harness_no_effect_before_take_full() {
        harness_no_effect_before_take(BUDGET);
    }

    #[test]
    fn harness_every_event_changes_state_bounded() {
        harness_every_event_changes_state(SMALL_BUDGET);
    }

    #[test]
    #[ignore = "explores a micro case to its 2M budget: release only"]
    fn harness_every_event_changes_state_full() {
        harness_every_event_changes_state(BUDGET);
    }

    #[test]
    fn harness_time_only_advances_on_tick_bounded() {
        harness_time_only_advances_on_tick(SMALL_BUDGET);
    }

    #[test]
    #[ignore = "explores a micro case to its 2M budget: release only"]
    fn harness_time_only_advances_on_tick_full() {
        harness_time_only_advances_on_tick(BUDGET);
    }

    #[test]
    fn harness_tick_cannot_pass_mandatory_input_bounded() {
        harness_tick_cannot_pass_mandatory_input(SMALL_BUDGET);
    }

    #[test]
    #[ignore = "explores a micro case to its 2M budget: release only"]
    fn harness_tick_cannot_pass_mandatory_input_full() {
        harness_tick_cannot_pass_mandatory_input(BUDGET);
    }

    #[test]
    fn harness_clock_read_holds_native_lock_bounded() {
        harness_clock_read_holds_native_lock(SMALL_BUDGET);
    }

    #[test]
    #[ignore = "explores a micro case to its 2M budget: release only"]
    fn harness_clock_read_holds_native_lock_full() {
        harness_clock_read_holds_native_lock(BUDGET);
    }

    #[test]
    fn harness_result_precedes_send_bounded() {
        harness_result_precedes_send(SMALL_BUDGET);
    }

    #[test]
    #[ignore = "explores a micro case to its 2M budget: release only"]
    fn harness_result_precedes_send_full() {
        harness_result_precedes_send(BUDGET);
    }

    #[test]
    fn harness_loss_publish_releases_l_first_bounded() {
        harness_loss_publish_releases_l_first(SMALL_BUDGET);
    }

    #[test]
    #[ignore = "explores a micro case to its 2M budget: release only"]
    fn harness_loss_publish_releases_l_first_full() {
        harness_loss_publish_releases_l_first(BUDGET);
    }

    #[test]
    fn harness_lifecycle_inline_loss_publish_no_recursion_bounded() {
        harness_lifecycle_inline_loss_publish_no_recursion(SMALL_BUDGET);
    }

    #[test]
    #[ignore = "explores a micro case to its 2M budget: release only"]
    fn harness_lifecycle_inline_loss_publish_no_recursion_full() {
        harness_lifecycle_inline_loss_publish_no_recursion(BUDGET);
    }

    #[test]
    fn harness_timer_never_early_in_case5_bounded() {
        harness_timer_never_early_in_case5(SMALL_BUDGET);
    }

    #[test]
    #[ignore = "explores a micro case to its 2M budget: release only"]
    fn harness_timer_never_early_in_case5_full() {
        harness_timer_never_early_in_case5(BUDGET);
    }
}

/// Counterexamples the walks found (E1 fix rounds 2 and 3): one that the plan answers is a default test, one it does not is ignored.
#[cfg(test)]
pub(super) mod counterexamples {
    use super::*;
    use crate::brain::cell::Outcome;
    use Event::*;

    /// Case 7b (full), seed 43409 (31 of 300,000 seeds, none below 43,409): the brain issues a Restore on an idle streaming
    /// chain (t = 38) and S takes it late (t = 66); meanwhile the drain ends, the brain stops the stream, and R's Stop call
    /// faults, so the chain is Uncertain when S's effect applies the Restore: I13 at its call. The Restore was legal when issued.
    /// Classification (b)/(c): a decision made from cell state while work already handed to S would change that state. Fixed: a Restore waits until every chain of its channel is Off.
    #[test]
    fn ce_restore_issued_streaming_applied_after_faulted_stop() {
        // The scripted trace no longer replays: the Restore is not decided while the chain streams (E1-179), so the seed's walk is the test.
        let case = case7b(Variant::Full);
        assert_eq!(walk_failure(&case, 43409), None);
    }

    /// Case 5 (micro), found by the full micro search after E1-179 (fixed: E1-184, a chain that comes up during shutdown is stopped): StartNext is
    /// decided while value 1's debt keeps the Restore waiting; Shutdown arrives with that start in flight, so `Armed` finds no
    /// stop sent, the chain stays Armed, the Restore waits for it to be Off, and B and R0 wait at the end of the epilogue.
    /// The scripted trace of `ce_shutdown_during_start_leaves_restore_waiting` (case 5, micro), also replayed on threads (stage E2a, S1).
    pub(crate) fn shutdown_during_start_trace() -> Vec<Event> {
        // Re-recorded when the device model became packet-granular (a recv that finds no whole packet returns nothing and sends
        // nothing): the original's three receives that sent boundaries are the events the world no longer enables, so they are gone.
        vec![
            Env(0),
            Tick,
            Tick,
            Tick,
            BStep,
            BOp,
            BOp,
            STake,
            SCheck,
            SLock,
            Env(1),
            BStep,
            BOp,
            Env(2),
            Tick,
            Tick,
            Tick,
            BStep,
            BOp,
            SEffect(Outcome::Ok),
            SUnlock,
            RCheck(0),
            RSnap(0),
            RBegin(0),
            REnd(0),
            RPublish(0),
            RSend(0),
            BStep,
            BOp,
            BOp,
            Timer(0),
            BStep,
            BOp,
            RCheck(0),
            BOp,
            RSnap(0),
            RBegin(0),
            Tick,
            REnd(0),
            RPublish(0),
            RSend(0),
            BStep,
            BOp,
            SResult,
            RCheck(0),
            RCmd(0),
            RLock(0),
            RCall(0),
            RUnlock(0),
            RResult(0),
            RSend(0),
            BStep,
            BOp,
            BOp,
            SSend,
            BStep,
            BOp,
            Env(3),
            BStep,
            BOp,
            RCheck(0),
            RCmd(0),
            RLock(0),
            RCall(0),
            RUnlock(0),
            RResult(0),
        ]
    }

    #[test]
    fn ce_shutdown_during_start_leaves_restore_waiting() {
        let trace = shutdown_during_start_trace();
        let world = case5(Variant::Micro).world;
        let replayed = dst::replay(&world, &trace).expect("the trace is a run of the world");
        assert_eq!(replayed.failure, None);
        let mut end = world.clone();
        for event in trace {
            end = dst::step(&end, &event).0;
        }
        // The trace ends where the old run stalled; finish it with walks from there (the stop is now sent, so more events follow).
        let rest = case5(Variant::Micro);
        for seed in 1..=20 {
            let walked = dst::walk(&end, seed, WALK_LIMIT);
            assert_eq!(judge(&Case { world: end.clone(), config: rest.config.clone() }, &walked), None, "seed {seed}");
        }
    }
}
