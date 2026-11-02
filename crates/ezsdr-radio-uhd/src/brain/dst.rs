//! The generic explorer of design note 45's experiment (stage E1b, 45-experiment §3.3): a depth-first
//! search over a [`Model`] with an exact visited set, a seeded random walk, a deterministic replay and
//! a shrinker. The toy models ([`super::toy`]) and the experiment's `World` run on this one search.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::mem::size_of;
use std::time::{Duration, Instant};

use crate::provider::generator::Rng;

/// What the explorer searches. Two states with equal keys must behave alike, so the key is exact: it
/// holds everything a later verdict depends on, and diagnostic counters and traces stay out of it.
pub(crate) trait Model: Clone {
    /// A transition the model enables.
    type Event: Clone + Debug + PartialEq;
    /// The state the visited set merges on. Ordered, never hashed.
    type Key: Clone + Ord;
    fn key(&self) -> Self::Key;
    /// The events enabled in this state, in a fixed order.
    fn enabled(&self) -> Vec<Self::Event>;
    /// Applies an event that [`Model::enabled`] returned for this state.
    fn apply(&mut self, event: &Self::Event);
    /// Safety: checked on every state a transition produces, before the visited-set lookup. It must
    /// read only data in the key; anything that depends on how a state was reached is a transition check.
    fn safety(&self) -> Result<(), String>;
    /// Transition check: `self` is the state before `event`, `after` the state it produced. Checked on
    /// every step, before the visited-set lookup, so it also catches a step into a visited state.
    fn transition(&self, _event: &Self::Event, _after: &Self) -> Result<(), String> {
        Ok(())
    }
    /// Progress: checked on every terminal state (no enabled event).
    fn progress(&self) -> Result<(), String>;
    /// The declared witnesses this state reaches.
    fn witnesses(&self) -> Vec<&'static str>;
    /// The scheduling class of an enabled event, below [`CLASSES`]: a walk weights the classes (see [`Weights`]). A model
    /// that does not distinguish its events keeps one class and its walks stay uniform.
    fn class(&self, _event: &Self::Event) -> usize {
        0
    }
    /// `Some` when this terminal state leaves an actor waiting (`Incomplete`): the epilogue's hook.
    fn incomplete(&self) -> Option<String> {
        None
    }
}

#[derive(Clone)]
/// One case: its name (the configuration named in every report), its state budget and the witnesses it declares.
pub(crate) struct Config {
    pub(crate) name: &'static str,
    pub(crate) budget: usize,
    /// Wall time after which the search stops as a budget overrun (45-experiment §3.3: a run cut by a budget fails).
    pub(crate) wall: Duration,
    pub(crate) witnesses: &'static [&'static str],
}

/// How often the search reads the clock for its wall limit (in transitions).
const WALL_CHECK_EVERY: usize = 1024;

/// Which check failed: a state's safety, or a terminal state's progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Check {
    Safety,
    Progress,
    Transition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) check: Check,
    pub(crate) message: String,
}

/// The counts of a search. `elapsed` is wall time, measured here and never seen by the model;
/// `memory` is a shallow estimate of the visited map.
#[derive(Debug)]
pub(crate) struct Stats {
    pub(crate) states: usize,
    pub(crate) transitions: usize,
    pub(crate) elapsed: Duration,
    pub(crate) memory: usize,
}

#[derive(Debug)]
pub(crate) enum Outcome<E> {
    /// Every reachable state was checked and every declared witness was reached.
    Pass,
    Violation {
        failure: Failure,
        trace: Vec<E>,
    },
    /// A state was reached again while it was on the DFS stack: a livelock (P5).
    Cycle {
        trace: Vec<E>,
    },
    /// The budget ran out. `frontier` is the trace of the first transition not taken; `pending` counts the transitions still open on the stack.
    BudgetExceeded {
        frontier: Vec<E>,
        pending: usize,
    },
    Vacuous {
        witness: &'static str,
    },
    Incomplete {
        message: String,
        trace: Vec<E>,
    },
}

#[derive(Debug)]
pub(crate) struct Verdict<E> {
    pub(crate) config: &'static str,
    pub(crate) outcome: Outcome<E>,
    pub(crate) stats: Stats,
    pub(crate) reached: Vec<&'static str>,
    pub(crate) missing: Vec<&'static str>,
}

impl<E: Debug> Verdict<E> {
    /// The text a failing case prints: the configuration, the outcome and the counts.
    pub(crate) fn report(&self) -> String {
        let outcome = match &self.outcome {
            Outcome::BudgetExceeded { frontier, pending } => format!("budget exceeded, frontier {frontier:?}, {pending} transitions still open"),
            Outcome::Incomplete { message, trace } => format!("incomplete: {message}, trace {trace:?}"),
            other => format!("{other:?}"),
        };
        format!(
            "configuration: {}\noutcome: {outcome}\nstates: {}, transitions: {}, elapsed: {:?}, memory: about {} bytes (shallow)\nreached witnesses: {:?}\nmissing witnesses: {:?}",
            self.config, self.stats.states, self.stats.transitions, self.stats.elapsed, self.stats.memory, self.reached, self.missing
        )
    }
}

/// The case wrapper: panics with the report unless the verdict is [`Outcome::Pass`].
pub(crate) fn assert_complete<E: Debug>(verdict: &Verdict<E>) {
    assert!(matches!(verdict.outcome, Outcome::Pass), "{}", verdict.report());
}

/// The bounded-search wrapper (45-experiment §10): fails on a `Violation`, `Cycle`, `Incomplete` or `Vacuous`
/// outcome; a `BudgetExceeded` run succeeds with its report (printed by the caller) and asserts nothing about
/// witnesses; a complete exploration is a `Pass`, which already requires every declared witness.
pub(crate) fn assert_bounded<E: Debug>(verdict: &Verdict<E>) {
    assert!(matches!(verdict.outcome, Outcome::Pass | Outcome::BudgetExceeded { .. }), "{}", verdict.report());
}

struct Frame<M: Model> {
    via: Option<M::Event>,
    model: M,
    key: M::Key,
    events: Vec<M::Event>,
    next: usize,
}

struct Search<'a, M: Model> {
    config: &'a Config,
    started: Instant,
    /// Every state entered so far; `true` while its frame is on the DFS stack.
    visited: BTreeMap<M::Key, bool>,
    reached: BTreeSet<&'static str>,
    transitions: usize,
    stack: Vec<Frame<M>>,
}

impl<M: Model> Search<'_, M> {
    fn trace(&self, last: Option<&M::Event>) -> Vec<M::Event> {
        self.stack.iter().filter_map(|frame| frame.via.clone()).chain(last.cloned()).collect()
    }

    /// The budget overrun, at the transition `event` (the wall limit and the state budget both end here).
    fn overrun(&self, event: &M::Event) -> Outcome<M::Event> {
        Outcome::BudgetExceeded { frontier: self.trace(Some(event)), pending: self.pending() }
    }

    fn pending(&self) -> usize {
        self.stack.iter().map(|frame| frame.events.len() - frame.next).sum()
    }

    fn missing(&self) -> Vec<&'static str> {
        self.config.witnesses.iter().copied().filter(|witness| !self.reached.contains(witness)).collect()
    }

    /// The safety check and the witnesses of a state a transition produces, before any lookup (E1-61).
    fn observe(&mut self, model: &M) -> Result<(), Failure> {
        self.reached.extend(model.witnesses());
        check_safety(model)
    }

    /// Enters a state the search has not seen: a terminal one is checked for progress and the epilogue.
    fn enter(&mut self, via: Option<M::Event>, model: M, key: M::Key) -> Result<(), Outcome<M::Event>> {
        let events = model.enabled();
        if events.is_empty() {
            if let Err(failure) = check_progress(&model) {
                return Err(Outcome::Violation { failure, trace: self.trace(via.as_ref()) });
            }
            if let Some(message) = model.incomplete() {
                return Err(Outcome::Incomplete { message, trace: self.trace(via.as_ref()) });
            }
        }
        self.visited.insert(key.clone(), true);
        self.stack.push(Frame { via, model, key, events, next: 0 });
        Ok(())
    }

    fn run(&mut self, init: M) -> Outcome<M::Event> {
        if let Err(failure) = self.observe(&init) {
            return Outcome::Violation { failure, trace: Vec::new() };
        }
        let key = init.key();
        if let Err(outcome) = self.enter(None, init, key) {
            return outcome;
        }
        while let Some(top) = self.stack.last_mut() {
            if top.next == top.events.len() {
                let frame = self.stack.pop().expect("the frame on top");
                self.visited.insert(frame.key, false);
                continue;
            }
            let event = top.events[top.next].clone();
            top.next += 1;
            let (next, checked) = step(&top.model, &event);
            self.transitions += 1;
            if self.transitions % WALL_CHECK_EVERY == 0 && self.started.elapsed() >= self.config.wall {
                return self.overrun(&event);
            }
            if let Err(failure) = checked.and_then(|()| self.observe(&next)) {
                return Outcome::Violation { failure, trace: self.trace(Some(&event)) };
            }
            let key = next.key();
            match self.visited.get(&key) {
                Some(true) => return Outcome::Cycle { trace: self.trace(Some(&event)) },
                Some(false) => {}
                None => {
                    if self.visited.len() >= self.config.budget {
                        return Outcome::BudgetExceeded { frontier: self.trace(Some(&event)), pending: self.pending() };
                    }
                    if let Err(outcome) = self.enter(Some(event), next, key) {
                        return outcome;
                    }
                }
            }
        }
        match self.missing().first() {
            Some(&witness) => Outcome::Vacuous { witness },
            None => Outcome::Pass,
        }
    }
}

/// Explores every state `init` reaches, depth first. The first violation, cycle, overrun or vacuity
/// is the verdict; a search that completes without one is [`Outcome::Pass`].
pub(crate) fn explore<M: Model>(config: &Config, init: M) -> Verdict<M::Event> {
    let mut search = Search { config, started: Instant::now(), visited: BTreeMap::new(), reached: BTreeSet::new(), transitions: 0, stack: Vec::new() };
    let outcome = search.run(init);
    Verdict {
        config: config.name,
        outcome,
        stats: Stats { states: search.visited.len(), transitions: search.transitions, elapsed: search.started.elapsed(), memory: search.visited.len() * size_of::<(M::Key, bool)>() },
        reached: search.reached.iter().copied().collect(),
        missing: search.missing(),
    }
}

fn check_safety<M: Model>(model: &M) -> Result<(), Failure> {
    model.safety().map_err(|message| Failure { check: Check::Safety, message })
}

fn check_progress<M: Model>(model: &M) -> Result<(), Failure> {
    model.progress().map_err(|message| Failure { check: Check::Progress, message })
}

/// Applies `event` to a copy of `model`, and checks the step's transition.
pub(crate) fn step<M: Model>(model: &M, event: &M::Event) -> (M, Result<(), Failure>) {
    let mut after = model.clone();
    after.apply(event);
    let checked = model.transition(event, &after).map_err(|message| Failure { check: Check::Transition, message });
    (after, checked)
}

/// Safety, and progress when the state is terminal: what a walk or a replay checks at each state.
fn check_state<M: Model>(model: &M) -> Result<(), Failure> {
    check_safety(model)?;
    if model.enabled().is_empty() {
        check_progress(model)?;
    }
    Ok(())
}

/// The number of scheduling classes a walk weights (a world's: time, environment delivery, actor work).
pub(crate) const CLASSES: usize = 3;

/// A walk's class weights (swarm scheduling): each seed draws one positive weight per class, and the walk picks among the
/// currently enabled events with them. They change only which enabled event is taken, never which are enabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Weights(pub(crate) [u64; CLASSES]);

/// The weights of `seed`: 1, 2, 4, 8, 16 or 32 per class, a function of the seed alone, so a replay of the seed has them.
pub(crate) fn weights_for(seed: u64) -> Weights {
    let mut rng = Rng(seed ^ 0x0077_6569_6768_7473);
    Weights([0; CLASSES].map(|_| 1 << rng.below(6)))
}

/// A walk's trace, and the failure it stopped at if it stopped at one.
#[derive(Debug)]
pub(crate) struct Walk<E> {
    pub(crate) trace: Vec<E>,
    pub(crate) failure: Option<Failure>,
}

/// A random walk from `init` over the enabled events, drawn from the seeded RNG with the seed's [`Weights`]. It stops at a
/// terminal state, at the first failure, or after `limit` events. The RNG is not part of the model.
pub(crate) fn walk<M: Model>(init: &M, seed: u64, limit: usize) -> Walk<M::Event> {
    walk_weighted(init, seed, weights_for(seed), limit)
}

pub(crate) fn walk_weighted<M: Model>(init: &M, seed: u64, weights: Weights, limit: usize) -> Walk<M::Event> {
    let mut rng = Rng(seed);
    let mut model = init.clone();
    let mut trace = Vec::new();
    loop {
        if let Err(failure) = check_state(&model) {
            return Walk { trace, failure: Some(failure) };
        }
        let events = model.enabled();
        if events.is_empty() || trace.len() == limit {
            return Walk { trace, failure: None };
        }
        let mut pick = rng.below(events.iter().map(|event| weights.0[model.class(event)]).sum());
        let index = events
            .iter()
            .position(|event| {
                let weight = weights.0[model.class(event)];
                let hit = pick < weight;
                pick -= weight.min(pick);
                hit
            })
            .expect("the pick is below the total weight");
        let event = events[index].clone();
        let (after, checked) = step(&model, &event);
        trace.push(event);
        model = after;
        if let Err(failure) = checked {
            return Walk { trace, failure: Some(failure) };
        }
    }
}

/// The key a replay ended in, and the first failure on the way.
#[derive(Debug, PartialEq)]
pub(crate) struct Replayed<K> {
    pub(crate) key: K,
    pub(crate) failure: Option<Failure>,
}

/// Applies `trace` from `init`, checking each state. An event that is not enabled when its turn
/// comes is an error, not a failure: the trace does not describe a run of the model.
pub(crate) fn replay<M: Model>(init: &M, trace: &[M::Event]) -> Result<Replayed<M::Key>, String> {
    let mut model = init.clone();
    for (index, event) in trace.iter().enumerate() {
        if let Err(failure) = check_state(&model) {
            return Ok(Replayed { key: model.key(), failure: Some(failure) });
        }
        if !model.enabled().contains(event) {
            return Err(format!("event {index} ({event:?}) is not enabled"));
        }
        let (after, checked) = step(&model, event);
        model = after;
        if let Err(failure) = checked {
            return Ok(Replayed { key: model.key(), failure: Some(failure) });
        }
    }
    Ok(Replayed { key: model.key(), failure: check_state(&model).err() })
}

/// A 1-minimal failing trace: it still fails, and no single event can be dropped and still fail.
/// A trace that does not fail is returned unchanged.
pub(crate) fn shrink<M: Model>(init: &M, trace: &[M::Event]) -> Vec<M::Event> {
    shrink_by(trace, |trace| matches!(replay(init, trace), Ok(Replayed { failure: Some(_), .. })))
}

/// The same shrink for an `Incomplete` trace: the replay still ends at a terminal state that leaves
/// an actor waiting.
pub(crate) fn shrink_incomplete<M: Model>(init: &M, trace: &[M::Event]) -> Vec<M::Event> {
    shrink_by(trace, |trace| {
        let mut model = init.clone();
        for event in trace {
            if !model.enabled().contains(event) {
                return false;
            }
            let (after, checked) = step(&model, event);
            if checked.is_err() {
                return false;
            }
            model = after;
        }
        model.enabled().is_empty() && model.incomplete().is_some()
    })
}

fn shrink_by<E: Clone>(trace: &[E], fails: impl Fn(&[E]) -> bool) -> Vec<E> {
    let mut best = trace.to_vec();
    if !fails(&best) {
        return best;
    }
    let mut dropped = true;
    while dropped {
        dropped = false;
        let mut index = 0;
        while index < best.len() {
            let mut candidate = best.clone();
            candidate.remove(index);
            if fails(&candidate) {
                best = candidate;
                dropped = true;
            } else {
                index += 1;
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;
    use crate::brain::toy::{DAG, DIAMOND, Flip, Graph, LockOrder, LostEvent, LostUpdate, SELF_LOOP, SHORTCUT};

    fn config(name: &'static str, witnesses: &'static [&'static str]) -> Config {
        Config { name, budget: 1_000, wall: Duration::MAX, witnesses }
    }

    #[test]
    fn harness_dfs_expands_each_edge_once() {
        // toy-E: six states and seven edges, with two reconvergences (nodes 3 and 5)
        let verdict = explore(&config("dag", &[]), Graph::new(DAG, &[]));
        assert_complete(&verdict);
        assert_eq!((verdict.stats.states, verdict.stats.transitions), (6, 7));
    }

    #[test]
    fn harness_reconvergence_is_not_a_cycle() {
        assert_complete(&explore(&config("diamond", &[]), Graph::new(DIAMOND, &[])));
    }

    #[test]
    fn harness_cycle_is_reported() {
        let verdict = explore(&config("self-loop", &[]), Graph::new(SELF_LOOP, &[]));
        match verdict.outcome {
            Outcome::Cycle { trace } => assert_eq!(trace, vec![0]),
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    #[test]
    fn harness_diagnostic_counter_cycle_is_reported() {
        let verdict = explore(&config("flip", &[]), Flip::new());
        assert!(matches!(verdict.outcome, Outcome::Cycle { .. }), "{}", verdict.report());
        assert_eq!(verdict.stats.states, 2);
    }

    #[test]
    fn harness_budget_overrun_is_failure() {
        // more than 10^4 states; the budget stops the search after 100 of them
        let verdict = explore(&Config { name: "count", budget: 100, wall: Duration::MAX, witnesses: &["never"] }, LostUpdate::new(10_000));
        assert!(matches!(verdict.outcome, Outcome::BudgetExceeded { .. }), "{}", verdict.report());
        assert_eq!((verdict.stats.states, verdict.missing.clone()), (100, vec!["never"]));
        assert!(verdict.report().contains("configuration: count"));
        assert!(catch_unwind(AssertUnwindSafe(|| assert_complete(&verdict))).is_err());
    }

    #[test]
    fn harness_replay_is_deterministic() {
        let init = LostUpdate::new(0);
        let trace = [LostEvent::Read(0), LostEvent::Read(1), LostEvent::Write(0), LostEvent::Write(1)];
        let first = replay(&init, &trace).expect("the trace is a run of the model");
        assert!(first.failure.is_some());
        assert_eq!(first, replay(&init, &trace).expect("the trace is a run of the model"));
    }

    #[test]
    fn harness_walk_same_seed_same_trace() {
        let init = LostUpdate::new(3);
        let (first, second) = (walk(&init, 7, 100), walk(&init, 7, 100));
        assert_eq!((&first.trace, &first.failure), (&second.trace, &second.failure));
        let traces: Vec<_> = (1..=10).map(|seed| walk(&init, seed, 100).trace).collect();
        assert!(traces.iter().any(|trace| *trace != traces[0]));
    }

    /// The seed's weights are positive, a function of the seed, and drive the walk: a walk with the seed's weights is the
    /// walk of the seed, and a model whose classes are weighted 1 : 32 takes the heavy class far more often.
    #[test]
    fn harness_walk_weights_are_the_seeds_and_steer_the_pick() {
        assert!((1..=50).all(|seed| weights_for(seed).0.iter().all(|w| *w >= 1)));
        assert_eq!(weights_for(9), weights_for(9));
        assert!((1..=50).any(|seed| weights_for(seed) != weights_for(1)));
        let init = LostUpdate::new(3);
        assert_eq!(walk(&init, 7, 100).trace, walk_weighted(&init, 7, weights_for(7), 100).trace);
        let pads_first = |weights: Weights| (1..=200).filter(|seed| walk_weighted(&init, *seed, weights, 100).trace[0] == LostEvent::Pad).count();
        assert!(pads_first(Weights([32, 1, 1])) > 150 && pads_first(Weights([1, 32, 1])) < 50);
    }

    #[test]
    fn harness_shrink_reaches_one_minimal_failure() {
        let init = LostUpdate::new(20);
        let mut trace = vec![LostEvent::Pad; 20];
        trace.extend([LostEvent::Read(0), LostEvent::Read(1), LostEvent::Write(0), LostEvent::Write(1)]);
        let shrunk = shrink(&init, &trace);
        assert_eq!(shrunk, vec![LostEvent::Read(0), LostEvent::Read(1), LostEvent::Write(0), LostEvent::Write(1)]);
        assert!(matches!(replay(&init, &shrunk), Ok(Replayed { failure: Some(_), .. })));
    }

    #[test]
    fn harness_vacuous_case_fails_on_missing_witness() {
        let verdict = explore(&config("dag", &["terminal", "never"]), Graph::new(DAG, &[]));
        assert!(matches!(verdict.outcome, Outcome::Vacuous { witness: "never" }), "{}", verdict.report());
        assert_eq!(verdict.reached, vec!["terminal"]);
    }

    #[test]
    fn harness_broken_toy_is_caught() {
        let lost = explore(&config("lost-update", &[]), LostUpdate::new(0));
        assert!(matches!(lost.outcome, Outcome::Violation { failure: Failure { check: Check::Safety, .. }, .. }), "{}", lost.report());
        let deadlock = explore(&config("lock-order", &[]), LockOrder::new());
        assert!(matches!(deadlock.outcome, Outcome::Violation { failure: Failure { check: Check::Progress, .. }, .. }), "{}", deadlock.report());
    }

    #[test]
    fn harness_violation_into_visited_state_is_caught() {
        // node 2 is entered first through (0, 2), which is allowed; the path through node 1 enters it again
        // through (1, 2), which is forbidden. The second step is into a visited state and must still be checked.
        let verdict = explore(&config("shortcut", &[]), Graph::new(SHORTCUT, &[(1, 2)]));
        match verdict.outcome {
            Outcome::Violation { failure, trace } => {
                assert_eq!(failure.check, Check::Transition);
                assert_eq!(trace, vec![1, 2]);
            }
            other => panic!("expected a violation, got {other:?}"),
        }
    }
}
