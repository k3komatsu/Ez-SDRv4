//! The decision core of design note 45's receive slice (stage E1a): the brain's state
//! machine ([`core`]), the cell procedures it calls ([`cell`]), the executors' pure rules
//! ([`rules`]) and the resumable device model ([`model`]). Test code only until the harness
//! of stage E1b drives it; nothing outside this module calls it.

mod cases;
mod cell;
mod conformance;
mod core;
mod device_tests;
mod dst;
mod force;
mod model;
mod model_device;
mod observed;
mod oracle;
mod rules;
mod shared;
mod thread_tests;
mod threads;
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

    /// Stream 0's stop leaves an accepted value no sample has read (value 1, instant 8, accepted at 8, call finished Ok): the device is
    /// at T = 12 with stream 0 open and not started, R0's FIFO holds a Stop, and the brain's Restore of channel 0 is deferred (enqueued
    /// explicitly: a Stop invents none). The shared start of the world's `harness_stop_debt_is_flushed_before_restore` and S6b.
    pub(crate) fn stop_debt_world(params: super::world::Params) -> super::world::World {
        use super::cell::{Outcome, Terminal};
        use super::core::{Request, RxOp};
        let mut entry = value(0, 8, 1);
        entry.accepted_at = Some(8);
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 0);
        push(&mut c, entry, ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut w = super::world::World::new(vec![c], Vec::new(), params);
        w.rx[0].handle = Some(1);
        w.device.open(0, vec![0]);
        w.device.t = 12;
        w.monitor.reserved.insert(1, (0, seg(0), 1));
        w.rx[0].fifo.push_back(RxOp::Stop { at: None, epoch: 1 });
        w.brain.deferred.push(Request::Restore { channel: 0 });
        w
    }

    /// A recv begun under `epoch` whose buffered sample ticks are `samples`.
    pub(crate) fn recv(epoch: u64, samples: &[i64]) -> Recv {
        Recv { epoch, handle: 7, samples: samples.to_vec() }
    }
}
