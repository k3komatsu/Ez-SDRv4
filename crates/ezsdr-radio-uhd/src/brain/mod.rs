//! The decision core of design note 45's receive slice (stage E1a): the brain's state
//! machine ([`core`]), the cell procedures it calls ([`cell`]), the executors' pure rules
//! ([`rules`]) and the resumable device model ([`model`]). Test code only until the harness
//! of stage E1b drives it; nothing outside this module calls it.

mod cases;
mod cell;
mod core;
mod dst;
mod model;
mod oracle;
mod rules;
mod toy;
mod world;

/// Shared test setup for the brain's modules.
#[cfg(test)]
pub(crate) mod fixtures {
    use ezsdr_radio::timeline::Channels;

    use super::cell::{Cell, Drain, InFlight, Phase, Recv, ValueState};
    use super::core::{Brain, Held, Input};
    use crate::provider::core::{RxEntry, SegmentKey};

    pub(crate) fn seg(origin: i64) -> SegmentKey {
        SegmentKey { by: Some(1), origin }
    }

    /// A value on `channel` with instant `e` and id `id` (its order is `(e, id)`).
    pub(crate) fn value(channel: usize, e: i64, id: u64) -> RxEntry {
        RxEntry { order: (e, id), channel, value: id as f64, accepted_at: None }
    }

    pub(crate) fn held(stream: usize, entry: RxEntry, generation: u64) -> Held {
        Held { stream, entry, generation }
    }

    /// A generation-1 cell that owns `channel`, in `phase`, with the cut in force and progress.
    pub(crate) fn cell(phase: Phase, channel: usize, cut: Option<i64>, progress: i64) -> Cell {
        let origin = match phase {
            Phase::Armed(segment) | Phase::Streaming(segment) | Phase::Stopping(segment) => segment.origin,
            _ => i64::MIN,
        };
        Cell {
            phase,
            channels: Channels::first(0).with(channel as u32),
            drain: Drain::Idle,
            outstanding: None,
            generation: 1,
            epoch: 1,
            origin,
            cut,
            retiring: None,
            progress,
            in_flight: Vec::new(),
            flushed: false,
            revision: 0,
            recomputes: 0,
            discarded: 0,
        }
    }

    /// Puts a value into `cell` in `state`, the state a test sets up directly.
    pub(crate) fn push(cell: &mut Cell, entry: RxEntry, state: ValueState) {
        cell.in_flight.push(InFlight { entry, state, left: false });
    }

    /// One input, then every operation it queued.
    pub(crate) fn feed(brain: &mut Brain, cells: &mut [Cell], input: Input) {
        brain.step(input);
        brain.run(cells);
    }

    /// A recv begun under `epoch` whose buffered sample ticks are `samples`.
    pub(crate) fn recv(epoch: u64, samples: &[i64]) -> Recv {
        Recv { epoch, handle: 7, samples: samples.to_vec() }
    }
}
