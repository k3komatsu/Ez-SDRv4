//! The observation record of stage E2a (test plan §5, §13, §14): what a finished `World` and a finished thread run did, in
//! one comparable value. Facts are lists in the order they were written; extraction rejects duplicates before anything is
//! collected, so a repeated write cannot hide in a map.

use std::collections::{BTreeMap, BTreeSet};

use super::cell::Terminal;
use super::core::{Disposition, Exit};
use super::force::Actor;
use super::rules::DevCall;
use super::world::{Fact, INITIAL_OCCUPANT, Published, World};
use crate::provider::core::SegmentKey;

/// The raw logs of one run, before extraction.
#[derive(Clone, Debug)]
pub(crate) struct Raw {
    pub(crate) dispositions: Vec<(u64, Disposition)>,
    pub(crate) terminals: Vec<(u64, Terminal)>,
    pub(crate) facts: Vec<Fact>,
    pub(crate) published: Vec<Published>,
    /// `(value, channel, segment, epoch)` of each reservation.
    pub(crate) reservations: Vec<(u64, usize, SegmentKey, u64)>,
    /// Per channel: `(value, tick)` of each application, in device order.
    pub(crate) applications: BTreeMap<usize, Vec<(u64, i64)>>,
    pub(crate) calls: Vec<DevCall>,
    pub(crate) exited: BTreeSet<Actor>,
    pub(crate) brain_exit: Exit,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Observed {
    pub(crate) dispositions: Vec<(u64, Disposition)>,
    pub(crate) terminals: Vec<(u64, Terminal)>,
    pub(crate) published: Vec<Published>,
    /// Qualifying evidence: value, stream, sample tick, channel.
    pub(crate) evidence: Vec<Fact>,
    /// Leftover consumption, its own list (a value left at its segment's end, read later).
    pub(crate) leftovers: Vec<Fact>,
    pub(crate) reservations: Vec<(u64, usize, SegmentKey, u64)>,
    pub(crate) applications: BTreeMap<usize, Vec<(u64, i64)>>,
    pub(crate) calls: Vec<DevCall>,
    pub(crate) exited: BTreeSet<Actor>,
    pub(crate) brain_exit: Exit,
}

fn unique<T: PartialEq + std::fmt::Debug>(what: &str, items: impl Iterator<Item = T>) -> Result<(), String> {
    let all: Vec<T> = items.collect();
    for (i, item) in all.iter().enumerate() {
        if all[..i].contains(item) {
            return Err(format!("duplicate {what}: {item:?}"));
        }
    }
    Ok(())
}

impl Observed {
    /// The names of the fields in which two observations differ.
    pub(crate) fn differing(&self, other: &Observed) -> Vec<&'static str> {
        let Observed { dispositions, terminals, published, evidence, leftovers, reservations, applications, calls, exited, brain_exit } = self;
        [
            ("dispositions", *dispositions == other.dispositions),
            ("terminals", *terminals == other.terminals),
            ("published", *published == other.published),
            ("evidence", *evidence == other.evidence),
            ("leftovers", *leftovers == other.leftovers),
            ("reservations", *reservations == other.reservations),
            ("applications", *applications == other.applications),
            ("calls", *calls == other.calls),
            ("exited", *exited == other.exited),
            ("brain_exit", *brain_exit == other.brain_exit),
        ]
        .into_iter()
        .filter(|(_, same)| !same)
        .map(|(name, _)| name)
        .collect()
    }

    /// Builds the observation, or fails on a duplicate: one disposition and one terminal per id, one consumption per value
    /// and identity `(value, sample, channel)` across the evidence and the leftover lists, and no leftover consumption of a
    /// value that has evidence.
    pub(crate) fn extract(raw: Raw) -> Result<Observed, String> {
        unique("disposition", raw.dispositions.iter().map(|(id, _)| *id))?;
        unique("terminal", raw.terminals.iter().map(|(id, _)| *id))?;
        let (leftovers, evidence): (Vec<Fact>, Vec<Fact>) = raw.facts.iter().cloned().partition(|f| f.leftover);
        unique("evidence value", evidence.iter().map(|f| f.value))?;
        unique("leftover value", leftovers.iter().map(|f| f.value))?;
        if let Some(f) = leftovers.iter().find(|f| evidence.iter().any(|e| e.value == f.value)) {
            return Err(format!("value {} is both evidence and leftover consumption", f.value));
        }
        // A reservation is a map entry by value id (the world keeps it so); the order the threads wrote them in is not a fact.
        let mut reservations = raw.reservations;
        reservations.sort_by_key(|r| r.0);
        Ok(Observed {
            dispositions: raw.dispositions,
            terminals: raw.terminals,
            published: raw.published,
            evidence,
            leftovers,
            reservations,
            applications: raw.applications,
            calls: raw.calls,
            exited: raw.exited,
            brain_exit: raw.brain_exit,
        })
    }

    /// The observation of a world's state.
    pub(crate) fn of_world(w: &World) -> Result<Observed, String> {
        let mut exited = BTreeSet::new();
        if !w.settings.unspawned && w.s_ended() {
            exited.insert(Actor::Settings);
        }
        for k in 0..w.rx.len() {
            if !w.rx[k].unspawned && w.rx_ended(k) {
                exited.insert(Actor::Rx(k));
            }
        }
        if w.brain.dead() {
            exited.insert(Actor::Brain);
        }
        Observed::extract(Raw {
            dispositions: w.brain.dispositions.clone(),
            terminals: w.terminal_log.clone(),
            facts: w.fact_log.clone(),
            published: w.published.clone(),
            reservations: w.monitor.reserved.iter().map(|(&id, &(channel, segment, epoch))| (id, channel, segment, epoch)).collect(),
            applications: w.device.dsp.iter().map(|(&channel, chain)| (channel, chain.log.iter().filter(|(_, id)| *id != INITIAL_OCCUPANT).map(|&(tick, id)| (id, tick)).collect())).collect(),
            calls: w.calls.clone(),
            exited,
            brain_exit: w.brain.exit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::cell::Outcome;

    fn fact(leftover: bool, value: u64, sample: i64, channel: usize) -> Fact {
        Fact { leftover, value, stream: 0, channel, sample }
    }

    fn base() -> Raw {
        Raw {
            dispositions: vec![(1, Disposition::Consumed)],
            terminals: vec![(5, Terminal::Call(Outcome::Ok))],
            facts: vec![fact(false, 1, 8, 0)],
            published: vec![Published { stream: 0, epoch: 1, ticks: vec![0, 1], discarded: 0 }],
            reservations: vec![(1, 0, SegmentKey { by: Some(1), origin: 0 }, 1)],
            applications: BTreeMap::from([(0, vec![(1, 8)])]),
            calls: vec![DevCall::Stop { stream: 0 }],
            exited: BTreeSet::from([Actor::Settings]),
            brain_exit: Exit::Running,
        }
    }

    #[test]
    fn extract_accepts_distinct() {
        assert!(Observed::extract(base()).is_ok());
    }

    #[test]
    fn extract_rejects_duplicate_disposition() {
        let mut raw = base();
        raw.dispositions = vec![(1, Disposition::Consumed), (1, Disposition::Consumed)];
        assert!(Observed::extract(raw).unwrap_err().contains("duplicate disposition"));
    }

    #[test]
    fn extract_rejects_duplicate_terminal() {
        let mut raw = base();
        raw.terminals = vec![(5, Terminal::Call(Outcome::Ok)), (5, Terminal::Call(Outcome::Ok))];
        assert!(Observed::extract(raw).unwrap_err().contains("duplicate terminal"));
    }

    #[test]
    fn extract_rejects_duplicate_evidence() {
        let mut raw = base();
        raw.facts = vec![fact(false, 1, 8, 0), fact(false, 1, 8, 0)];
        assert!(Observed::extract(raw).unwrap_err().contains("duplicate evidence value"));
    }

    #[test]
    fn extract_rejects_duplicate_leftover() {
        let mut raw = base();
        raw.facts = vec![fact(true, 1, 8, 0), fact(true, 1, 8, 0)];
        assert!(Observed::extract(raw).unwrap_err().contains("duplicate leftover value"));
    }

    #[test]
    fn extract_rejects_same_consumption_in_both_lists() {
        let mut raw = base();
        raw.facts = vec![fact(false, 1, 8, 0), fact(true, 1, 8, 0)];
        assert!(Observed::extract(raw).unwrap_err().contains("both evidence and leftover"));
        let mut raw = base();
        raw.facts = vec![fact(false, 1, 8, 0), fact(true, 1, 9, 0)];
        assert!(Observed::extract(raw).unwrap_err().contains("both evidence and leftover"));
    }

    /// Each field alone makes two observations unequal.
    #[test]
    fn observed_each_field_differs() {
        let one = Observed::extract(base()).unwrap();
        let mut tweaks: Vec<Raw> = Vec::new();
        let mut r = base();
        r.dispositions = vec![(1, Disposition::Cancelled)];
        tweaks.push(r);
        let mut r = base();
        r.terminals = vec![(5, Terminal::Dropped)];
        tweaks.push(r);
        let mut r = base();
        r.facts = vec![fact(false, 1, 9, 0)];
        tweaks.push(r);
        let mut r = base();
        r.facts = vec![fact(true, 1, 8, 0)];
        tweaks.push(r);
        let mut r = base();
        r.published[0].ticks = vec![0];
        tweaks.push(r);
        let mut r = base();
        r.published[0].discarded = 1;
        tweaks.push(r);
        let mut r = base();
        r.reservations[0].3 = 2;
        tweaks.push(r);
        let mut r = base();
        r.applications = BTreeMap::from([(0, vec![(1, 9)])]);
        tweaks.push(r);
        let mut r = base();
        r.calls = vec![DevCall::Stop { stream: 1 }];
        tweaks.push(r);
        let mut r = base();
        r.exited = BTreeSet::new();
        tweaks.push(r);
        let mut r = base();
        r.brain_exit = Exit::Dead;
        tweaks.push(r);
        for (i, raw) in tweaks.into_iter().enumerate() {
            assert_ne!(one, Observed::extract(raw).unwrap(), "tweak {i} must be seen");
        }
    }
}
