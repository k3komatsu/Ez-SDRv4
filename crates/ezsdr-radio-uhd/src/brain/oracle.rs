//! The oracles of design note 45's experiment (stage E1b, part 2b; 45-experiment §5, test plan
//! section E). Each property is one check that returns `Err` with a text starting with its id
//! (`"I4: ..."`). The checks sit at their evaluation points: a property about the current state is
//! in [`safety`], one about what a step did is in [`transition`], and the progress properties are in
//! [`progress`], checked at terminal states. Safety reads only the key; a fact a check needs from the
//! past is in the monitor (world.rs), never a history.

use super::cell::{Drain, Outcome, Phase, Terminal, ValueState};
use super::core::{Change, Command, Disposition, IN_FLIGHT_BOUND, Op, RxOp, Section, SetCmd};
use super::dst::Model;
use super::world::{Event, Ident, RStep, RxCall, SStep, Who, World};

/// The safety checks of every state: I5 (the cut in force is not below the published progress)
/// and I8 (a chain applies at most one write per sample, in order).
pub(super) fn safety(w: &World) -> Result<(), String> {
    i5_state(w)?;
    i8(w)
}

/// The transition checks of one step: `before` is the state the event was taken in, `after` the
/// state it produced.
pub(super) fn transition(before: &World, event: &Event, after: &World) -> Result<(), String> {
    type StepCheck = fn(&World, &Event, &World) -> Result<(), String>;
    let checks: [StepCheck; 14] = [i1, i7, i2, i3, i4, i5, i6, i9, i11, i12, i13, i14, i15, i16];
    for check in checks {
        check(before, event, after)?;
    }
    Ok(())
}

/// The progress checks of a terminal state (no event enabled).
pub(super) fn progress(w: &World) -> Result<(), String> {
    p1(w)?;
    p2(w)?;
    p3(w)?;
    p4(w)
}

/// Two facts name the same reservation: the value, its channel, segment and epoch.
fn same_reservation(a: &Ident, b: &Ident) -> bool {
    a.value == b.value && a.channel == b.channel && a.segment == b.segment && a.epoch == b.epoch
}

/// A consumption fact agrees with the value's device application: the same reservation and, for
/// evidence, the same sample. A leftover consumption comes at a later sample by design (u11b). The
/// channel of each observed fact is the device's (the DSP chain, the receiving streamer), so the
/// fact is also compared with the reservation's intent: a write that reached another channel disagrees.
fn agrees(w: &World, id: u64, fact: &Ident, same_sample: bool) -> bool {
    let intended = w.monitor.reserved.get(&id).is_some_and(|&(channel, segment, epoch)| (channel, segment, epoch) == (fact.channel, fact.segment, fact.epoch));
    match w.monitor.applied.get(&id) {
        Some(applied) => intended && same_reservation(applied, fact) && (!same_sample || applied.sample == fact.sample),
        None => false,
    }
}

/// Qualifying evidence of the value's consumption (I4): receive evidence that agrees with the
/// device application.
fn qualifying(w: &World, id: u64) -> bool {
    w.monitor.evidence.get(&id).is_some_and(|evidence| agrees(w, id, evidence, true))
}

/// The commands the brain routed to an executor's queue in this step. A queue only grows in a brain
/// operation and no event both pops and pushes one, so the routed commands are the tail.
fn routed(before: &World, after: &World) -> Vec<Command> {
    let mut routed: Vec<Command> = after.settings.fifo.iter().skip(before.settings.fifo.len()).cloned().map(Command::Settings).collect();
    for (k, rx) in after.rx.iter().enumerate() {
        routed.extend(rx.fifo.iter().skip(before.rx[k].fifo.len()).map(|op| Command::Rx { stream: k, op: *op }));
    }
    routed
}

/// The values this step reserved, as `(stream, value)`: an entry a cell gained.
fn reserved_by(before: &World, after: &World) -> Vec<(usize, u64)> {
    let mut reserved = Vec::new();
    for (k, cell) in after.cells.iter().enumerate() {
        for flight in &cell.in_flight {
            let id = flight.entry.order.1;
            if !before.cells[k].in_flight.iter().any(|f| f.entry.order.1 == id) {
                reserved.push((k, id));
            }
        }
    }
    reserved
}

/// The channels of the restores this step issues, and of the restore an `S` call makes.
fn restore_channels(before: &World, event: &Event, after: &World) -> Vec<usize> {
    let mut channels: Vec<usize> = routed(before, after)
        .iter()
        .filter_map(|command| match command {
            Command::Settings(SetCmd { change: Change::Restore { channel }, .. }) => Some(*channel),
            _ => None,
        })
        .collect();
    if let (Event::SEffect(outcome), Some(SetCmd { change: Change::Restore { channel }, .. })) = (event, &before.settings.hand) {
        if *outcome != Outcome::Lost {
            channels.push(*channel);
        }
    }
    channels
}

/// The actor whose event is an ordinary device call (I1): a settings call that reaches the device,
/// or a receive call other than a lifecycle call (open or close), which I14 governs.
fn ordinary_call(before: &World, event: &Event) -> Option<Who> {
    match *event {
        Event::SEffect(outcome) if outcome != Outcome::Lost => Some(Who::Settings),
        Event::RCall(k) | Event::RFault(k) => match before.rx[k].program.front() {
            Some(RStep::Call(call)) if !matches!(call, RxCall::Open | RxCall::Close) => Some(Who::Rx(k)),
            _ => None,
        },
        _ => None,
    }
}

/// The values whose terminal this step wrote as `Abandoned`.
fn newly_abandoned(before: &World, after: &World) -> Vec<u64> {
    after.monitor.terminals.iter().filter(|&(id, terminal)| *terminal == Terminal::Abandoned && before.monitor.terminals.get(id) != Some(terminal)).map(|(id, _)| *id).collect()
}

/// I5 (state): the cut in force is never below the progress already published. A retiring epoch is
/// left out: its progress and the new cut are written by different steps.
fn i5_state(w: &World) -> Result<(), String> {
    for (k, cell) in w.cells.iter().enumerate() {
        if let Some(cut) = cell.cut.filter(|&cut| cut < cell.progress && cell.retiring.is_none()) {
            return Err(format!("I5: stream {k} has cut {cut} below its published progress {}", cell.progress));
        }
    }
    Ok(())
}

/// I8 (state): a chain applies at most one write per sample, in order. Its log's ticks strictly increase.
fn i8(w: &World) -> Result<(), String> {
    for (channel, chain) in &w.device.dsp {
        if let Some(pair) = chain.log.windows(2).find(|pair| pair[1].0 <= pair[0].0) {
            return Err(format!("I8: chain {channel} applied value {} at sample {} and value {} at sample {}", pair[0].1, pair[0].0, pair[1].1, pair[1].0));
        }
    }
    Ok(())
}

/// I1: an ordinary device call is not accepted after a check that saw `F` set. With `F` set, a call
/// is accepted only when the actor's latest check, with no call since, saw `F` clear. An abandonment
/// is written only when `F` is set.
fn i1(before: &World, event: &Event, after: &World) -> Result<(), String> {
    if let Some(id) = newly_abandoned(before, after).first() {
        if !before.failure {
            return Err(format!("I1: command {id} abandoned while F is clear"));
        }
    }
    if let Some(who) = ordinary_call(before, event) {
        match before.monitor.checked.get(&who) {
            Some(true) => return Err(format!("I1: {who:?} makes an ordinary device call after a check that saw F set")),
            None if before.failure => return Err(format!("I1: {who:?} makes an ordinary device call with F set and no check since its last call")),
            _ => {}
        }
    }
    Ok(())
}

/// I2: a reservation takes a value only into an armed or streaming segment, on a channel the cell
/// owns, at the cell's generation, at or after the segment's origin, and before the plan cut with
/// progress before the cut.
fn i2(before: &World, _event: &Event, after: &World) -> Result<(), String> {
    for (k, id) in reserved_by(before, after) {
        let cell = &before.cells[k];
        let Some(held) = before.brain.held.iter().find(|held| held.entry.order.1 == id) else {
            return Err(format!("I2: value {id} reserved on stream {k} without being held"));
        };
        let instant = held.entry.order.0;
        let segment = match cell.phase {
            Phase::Armed(segment) | Phase::Streaming(segment) => segment,
            phase => return Err(format!("I2: value {id} reserved on stream {k} in phase {phase:?}")),
        };
        let before_cut = match cell.cut {
            Some(cut) => instant < cut && cell.progress < cut,
            None => true,
        };
        let inside = cell.channels.contains(held.entry.channel as u32) && held.generation == cell.generation && instant >= segment.origin && before_cut;
        if !inside {
            return Err(format!(
                "I2: value {id} reserved on stream {k} outside its conditions (instant {instant}, channel {}, generation {}, origin {}, cut {:?}, progress {})",
                held.entry.channel, held.generation, segment.origin, cell.cut, cell.progress
            ));
        }
    }
    Ok(())
}

/// I7: no reservation succeeds while the phase is `Stopping` or `Off`.
fn i7(before: &World, _event: &Event, after: &World) -> Result<(), String> {
    for (k, id) in reserved_by(before, after) {
        let phase = before.cells[k].phase;
        if matches!(phase, Phase::Stopping(_) | Phase::Off) {
            return Err(format!("I7: value {id} reserved on stream {k} while {phase:?}"));
        }
    }
    Ok(())
}

/// A handle changes only on a chain that is not accepting and holds no committed value: a value reserved
/// ahead of a Close or Open would pin the chain while the handle changes (note 45 §4.5).
fn handle_blocked(w: &World, stream: usize) -> bool {
    let cell = &w.cells[stream];
    cell.committed(None) || matches!(cell.phase, Phase::Armed(_) | Phase::Streaming(_) | Phase::Stopping(_) | Phase::Flushing)
}

/// I3: a restore, a move, a close, an open and a next-segment start are issued only for a chain with
/// no pin (a taken or maybe-applied value). Debt is not a pin. A start in an armed or streaming phase
/// is a late start, which the next-segment rule does not govern. A close or open is also issued only
/// for a chain that is not accepting, and executes only while it still is not and holds no committed
/// value (a reservation between the routing and the call would pin the chain).
fn i3(before: &World, event: &Event, after: &World) -> Result<(), String> {
    if let Event::RCall(k) | Event::RFault(k) = *event {
        if let Some(RStep::Call(call @ (RxCall::Close | RxCall::Open))) = before.rx[k].program.front() {
            if handle_blocked(before, k) {
                return Err(format!("I3: stream {k} executes {call:?} while its chain accepts or holds a committed value"));
            }
        }
    }
    for command in routed(before, after) {
        let pinned = match command {
            Command::Settings(SetCmd { change: Change::Restore { channel } | Change::Move { channel, .. }, .. }) => before.cells.iter().any(|cell| cell.pinned_on(channel)),
            Command::Rx { stream, op: RxOp::Close | RxOp::Open } => handle_blocked(before, stream),
            Command::Rx { stream, op: RxOp::Start { .. } } => before.cells[stream].pinned() && !matches!(before.cells[stream].phase, Phase::Armed(_) | Phase::Streaming(_)),
            _ => false,
        };
        if pinned {
            return Err(format!("I3: {command:?} is issued while its chain holds a pinned value"));
        }
    }
    Ok(())
}

/// I4: a value has one disposition and a command one terminal. Consumption needs qualifying evidence
/// of the device's application. A cancellation needs that the device never applied the value. A report
/// needs that no qualifying evidence exists. Each receive fact agrees with the application: evidence
/// in its sample too, a leftover consumption in the reservation only.
fn i4(before: &World, event: &Event, after: &World) -> Result<(), String> {
    for &(id, disposition) in after.brain.dispositions.iter().skip(before.brain.dispositions.len()) {
        let writes = after.brain.dispositions.iter().filter(|(value, _)| *value == id).count();
        if writes > 1 || before.brain.disposition(id).is_some() {
            return Err(format!("I4: value {id} has a second disposition"));
        }
        match disposition {
            Disposition::Consumed if !qualifying(after, id) => {
                return Err(format!("I4: value {id} consumed without a device application and qualifying evidence that agree"));
            }
            Disposition::Cancelled if after.monitor.applied.contains_key(&id) => {
                return Err(format!("I4: value {id} cancelled although the device applied it"));
            }
            Disposition::Reported(reason) if qualifying(after, id) => {
                return Err(format!("I4: value {id} reported ({reason:?}) although qualifying evidence of its consumption exists"));
            }
            _ => {}
        }
    }
    if let (Event::SResult, Some(cmd)) = (event, &before.settings.hand) {
        if before.monitor.terminals.contains_key(&cmd.id) {
            return Err(format!("I4: command {} has a second terminal", cmd.id));
        }
    }
    if let Some((id, _)) = before.monitor.terminals.iter().find(|&(id, terminal)| after.monitor.terminals.get(id) != Some(terminal)) {
        return Err(format!("I4: command {id} has its terminal overwritten"));
    }
    for (id, fact) in after.monitor.evidence.iter().filter(|&(id, _)| !before.monitor.evidence.contains_key(id)) {
        if !agrees(after, *id, fact, true) {
            return Err(format!("I4: value {id} received as evidence without the device application at its sample"));
        }
    }
    for (id, fact) in after.monitor.leftovers.iter().filter(|&(id, _)| !before.monitor.leftovers.contains_key(id)) {
        if !agrees(after, *id, fact, false) {
            return Err(format!("I4: value {id} consumed as leftover without the device application of its reservation"));
        }
    }
    Ok(())
}

/// I5 (publication): a publication of the current epoch ends at or before the cut in force.
fn i5(before: &World, event: &Event, after: &World) -> Result<(), String> {
    let Event::RPublish(k) = *event else { return Ok(()) };
    let cell = &before.cells[k];
    if !before.rx[k].recv.as_ref().is_some_and(|call| call.recv.epoch == cell.epoch) {
        return Ok(());
    }
    if let Some(cut) = cell.cut.filter(|&cut| after.cells[k].progress > cut) {
        return Err(format!("I5: stream {k} published up to {} past its cut {cut}", after.cells[k].progress));
    }
    Ok(())
}

/// I6: a recv begun under an older epoch publishes only up to that epoch's latest cut, and the
/// quiesce ack follows its publication.
fn i6(before: &World, event: &Event, after: &World) -> Result<(), String> {
    if let Event::RPublish(k) = *event {
        let cell = &before.cells[k];
        let older = match (before.rx[k].recv.as_ref().map(|call| call.recv.epoch), cell.retiring) {
            (Some(epoch), Some(retiring)) if epoch != cell.epoch && retiring.epoch == epoch => retiring.cut,
            _ => None,
        };
        if let Some(cut) = older.filter(|&cut| after.cells[k].progress > cut) {
            return Err(format!("I6: stream {k} published an older epoch's recv up to {} past its retiring cut {cut}", after.cells[k].progress));
        }
    }
    for (k, cell) in after.cells.iter().enumerate() {
        let acked = before.cells[k].retiring.is_some_and(|retiring| !retiring.published) && cell.retiring.is_none();
        if acked {
            return Err(format!("I6: stream {k} acknowledged its quiesce before the old epoch's recv published"));
        }
    }
    Ok(())
}

/// I9: no device call on a closed streamer handle, and no handle closed after `F`. A loss keeps the
/// handles, so a close is only ever a device call that `F` did not stop.
fn i9(before: &World, event: &Event, _after: &World) -> Result<(), String> {
    let (device, k) = match *event {
        Event::RCall(k) => (true, k),
        Event::RFault(k) => (false, k),
        _ => return Ok(()),
    };
    let Some(RStep::Call(call)) = before.rx[k].program.front() else { return Ok(()) };
    if before.rx[k].handle.is_none() && matches!(call, RxCall::Begin { .. } | RxCall::Stop { .. } | RxCall::PlanStop { .. } | RxCall::Close) {
        return Err(format!("I9: stream {k} calls {call:?} on a closed handle"));
    }
    if device && before.failure && *call == RxCall::Close {
        return Err(format!("I9: stream {k} closes its handle after F"));
    }
    Ok(())
}

/// I11: a reservation is refused while the cell holds the in-flight bound.
fn i11(before: &World, _event: &Event, after: &World) -> Result<(), String> {
    for (k, id) in reserved_by(before, after) {
        let held = before.cells[k].in_flight.len();
        if held >= IN_FLIGHT_BOUND {
            return Err(format!("I11: value {id} reserved on stream {k} with {held} values in flight, the bound is {IN_FLIGHT_BOUND}"));
        }
    }
    Ok(())
}

/// I12: no start of the new epoch, and no close or open, reaches a stream's queue while the old
/// epoch's quiesce is not yet acknowledged. A start carries its epoch: a start of the old epoch (a
/// late restart queued before the plan change) is not the new epoch's.
fn i12(before: &World, _event: &Event, after: &World) -> Result<(), String> {
    for command in routed(before, after) {
        if let Command::Rx { stream, op } = command {
            let cell = &before.cells[stream];
            let Some(retiring) = cell.retiring.as_ref() else { continue };
            let violation = match op {
                RxOp::Start { epoch, .. } => epoch == cell.epoch && epoch != retiring.epoch,
                RxOp::Close | RxOp::Open => true,
                _ => false,
            };
            if violation {
                return Err(format!("I12: stream {stream} gets {op:?} for the new epoch before the old epoch's quiesce ack"));
            }
        }
    }
    Ok(())
}

/// I13: no restore of a channel whose chain is uncertain or lost, at its issue or at its call.
fn i13(before: &World, event: &Event, after: &World) -> Result<(), String> {
    for channel in restore_channels(before, event, after) {
        if before.cells.iter().any(|cell| cell.channels.contains(channel as u32) && matches!(cell.phase, Phase::Uncertain | Phase::Lost)) {
            return Err(format!("I13: restore of channel {channel} while its chain is uncertain or lost"));
        }
    }
    Ok(())
}

/// I14: no lifecycle admission after `F`. The recheck under `G` admits none, and a lifecycle call
/// sees `F` clear.
fn i14(before: &World, event: &Event, after: &World) -> Result<(), String> {
    match *event {
        Event::RGate(k) if before.failure && !after.rx[k].program.is_empty() => Err(format!("I14: stream {k} is admitted under G to a lifecycle call after F")),
        Event::RCall(k) if before.failure && matches!(before.rx[k].program.front(), Some(RStep::Call(RxCall::Open | RxCall::Close))) => Err(format!("I14: stream {k} makes a lifecycle call after F")),
        _ => Ok(()),
    }
}

/// I15: no restore of a channel while any chain holds debt (a finished call no sample has consumed)
/// or left values on it. Pins do not make a restore eligible. A move is not governed here (the test
/// plan's contradiction 11: a move waits for pins only).
fn i15(before: &World, event: &Event, after: &World) -> Result<(), String> {
    for channel in restore_channels(before, event, after) {
        for (k, cell) in before.cells.iter().enumerate() {
            let left = cell.in_flight.iter().any(|flight| flight.left && flight.entry.channel == channel);
            if left || cell.owed_on(channel) {
                return Err(format!("I15: restore of channel {channel} while stream {k} holds debt or left values on it"));
            }
        }
    }
    Ok(())
}

/// I16: a processed drain deadline ends the drain in the same step, whether or not a pin exists.
fn i16(before: &World, event: &Event, after: &World) -> Result<(), String> {
    let timer = match (event, before.brain.ops.front()) {
        (Event::BOp, Some(Op::Section(Section::TimerFired { stream }))) => Some(*stream),
        _ => None,
    };
    match timer {
        Some(stream) if before.cells[stream].drain == Drain::Draining && after.cells[stream].drain != Drain::Ended => {
            Err(format!("I16: stream {stream} processed its drain deadline with the drain still open (a pin holds it)"))
        }
        _ => Ok(()),
    }
}

/// P1: a drain deadline that was delivered for the current drain (its instant is the cut plus the
/// drain window) has ended that drain. A deadline not yet delivered is no claim, and a dead brain
/// (a panic took it after the delivery) ends no drain.
fn p1(w: &World) -> Result<(), String> {
    if w.brain.dead() {
        return Ok(());
    }
    for (k, cell) in w.cells.iter().enumerate() {
        if let Some(cut) = cell.cut {
            if cell.drain == Drain::Draining && w.monitor.drain_observed.contains(&(k, cut + w.brain.drain_window)) {
                return Err(format!("P1: stream {k}'s drain deadline was delivered but its drain never ended"));
            }
        }
    }
    Ok(())
}

/// P2: once the run is complete (no actor waits, see `incomplete`), every released value has one
/// disposition after shutdown, and every settings command has one terminal outcome. After a panic
/// a value may end without its disposition: see [`panic_settled`].
fn p2(w: &World) -> Result<(), String> {
    if w.incomplete().is_some() {
        return Ok(());
    }
    if w.monitor.shutdown {
        for &id in &w.monitor.released {
            let count = w.brain.dispositions.iter().filter(|(value, _)| *value == id).count();
            if w.monitor.panicked && panic_settled(w, id) {
                continue;
            }
            if count != 1 {
                return Err(format!("P2: released value {id} has {count} dispositions at the end"));
            }
        }
    }
    for &id in &w.monitor.commanded {
        if !w.monitor.terminals.contains_key(&id) {
            return Err(format!("P2: settings command {id} has no terminal outcome"));
        }
    }
    Ok(())
}

/// A released value after a panic need not have its disposition when its settings command reached an
/// executor's terminal (the executor wrote it), when the panic replaced its release (it never reached
/// the brain), or when the value never left the brain or its cell's queue (no command was issued for it).
fn panic_settled(w: &World, id: u64) -> bool {
    w.monitor.terminals.contains_key(&id)
        || w.monitor.panic_reported.contains(&id)
        || w.brain.held.iter().any(|held| held.entry.order.1 == id)
        || w.cells.iter().any(|cell| cell.state_of(id) == Some(&ValueState::Queued))
}

/// P3: after shutdown, `S` and every receive executor have exited (a complete run).
fn p3(w: &World) -> Result<(), String> {
    if w.monitor.shutdown && w.incomplete().is_none() {
        if !w.settings.exited {
            return Err("P3: S did not exit after shutdown".to_string());
        }
        for (k, rx) in w.rx.iter().enumerate() {
            if !rx.exited {
                return Err(format!("P3: receive executor R{k} did not exit after shutdown"));
            }
        }
    }
    Ok(())
}

/// P4: at a terminal state no actor waits on a lock or a gate. Nothing is enabled, so any pending
/// step other than a receive's end or the device queue's room (time waits, which the epilogue ends)
/// is stuck. At the end of the epilogue a wait is `Incomplete`, which `enter` reports after this check.
fn p4(w: &World) -> Result<(), String> {
    if w.at_end() {
        return Ok(());
    }
    if !w.brain.ops.is_empty() {
        return Err("P4: the brain has operations to run and no event is enabled".to_string());
    }
    if matches!(w.settings.program.front(), Some(SStep::Lock | SStep::Unlock | SStep::Pub(_))) {
        return Err("P4: S waits for the native lock or the gate, and no actor releases it".to_string());
    }
    for (k, rx) in w.rx.iter().enumerate() {
        if let Some(step) = rx.program.front() {
            if !matches!(step, RStep::End) {
                return Err(format!("P4: R{k} waits at {step:?}, and no actor releases it"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Duration;

    use super::super::cell::{Cell, InFlight, Retiring, ValueState};
    use super::super::core::{DRAIN_WINDOW, Input, Reason, Request};
    use super::super::dst::{Config, assert_complete, explore};
    use super::super::fixtures::{cell, held, push, recv, seg, value};
    use super::super::world::{EnvInput, EnvSlot, Params, RecvCall};
    use super::*;

    fn params() -> Params {
        Params { horizon: 8, epilogue: 8, recv_timeout: 8, queue_cap: 4, fifo_cap: 4, drain_window: DRAIN_WINDOW, call_faults: false, panic_step: None }
    }

    fn world(cells: Vec<Cell>) -> World {
        World::new(cells, Vec::new(), params())
    }

    fn ident() -> Ident {
        Ident { value: 5, channel: 0, segment: seg(0), epoch: 1, sample: 0 }
    }

    /// `Ok` only when `result` is a violation of `property`, which the message names.
    fn violates(result: Result<(), String>, property: &str) {
        match result {
            Err(text) => assert!(text.starts_with(property), "expected a violation of {property}, got: {text}"),
            Ok(()) => panic!("expected a violation of {property}, got none"),
        }
    }

    fn streaming() -> Cell {
        cell(Phase::Streaming(seg(0)), 0, Some(16), 0)
    }

    #[test]
    fn inv_i1_call_after_failure_seen_is_violation() {
        let mut before = world(vec![streaming()]);
        before.settings.hand = Some(SetCmd { id: 9, change: Change::Restore { channel: 0 } });
        before.settings.program = VecDeque::from([SStep::Effect]);
        before.failure = true;
        before.monitor.checked.insert(Who::Settings, true);
        let after = before.clone();
        violates(before.transition(&Event::SEffect(Outcome::Ok), &after), "I1:");
    }

    #[test]
    fn inv_i1_abandoned_without_failure_is_violation() {
        let before = world(vec![streaming()]);
        let mut after = before.clone();
        after.monitor.terminals.insert(9, Terminal::Abandoned);
        violates(before.transition(&Event::SCheck, &after), "I1:");
    }

    #[test]
    fn inv_i2_reservation_outside_conditions_is_violation() {
        let mut before = world(vec![streaming()]);
        before.brain.held.push(held(0, value(3, 10, 5), 1));
        let mut after = before.clone();
        push(&mut after.cells[0], value(3, 10, 5), ValueState::Queued);
        violates(before.transition(&Event::BOp, &after), "I2:");
    }

    #[test]
    fn inv_i3_restore_while_pinned_is_violation() {
        let mut before = world(vec![streaming()]);
        push(&mut before.cells[0], value(0, 10, 5), ValueState::Taken);
        let mut after = before.clone();
        after.settings.fifo.push_back(SetCmd { id: 9, change: Change::Restore { channel: 0 } });
        violates(before.transition(&Event::BOp, &after), "I3:");
    }

    #[test]
    fn inv_i3_outstanding_debt_is_not_pin() {
        let mut before = world(vec![streaming()]);
        push(&mut before.cells[0], value(0, 10, 5), ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut after = before.clone();
        after.settings.fifo.push_back(SetCmd { id: 9, change: Change::Restore { channel: 0 } });
        assert_eq!(i3(&before, &Event::BOp, &after), Ok(()));
    }

    /// Sol E1 finding 2: a Close routed to an accepting chain, and a Close that executes after a reservation pinned the chain.
    #[test]
    fn inv_i3_close_to_an_accepting_chain_is_violation() {
        let before = world(vec![streaming()]);
        let mut after = before.clone();
        after.rx[0].fifo.push_back(RxOp::Close);
        violates(before.transition(&Event::BOp, &after), "I3:");
    }

    #[test]
    fn inv_i3_close_executing_with_a_committed_value_is_violation() {
        for (phase, taken) in [(Phase::Off, true), (Phase::Streaming(seg(0)), false)] {
            let mut c = cell(phase, 0, Some(16), 0);
            if taken {
                push(&mut c, value(0, 10, 5), ValueState::Taken);
            }
            let mut before = world(vec![c]);
            before.rx[0].handle = Some(1);
            before.rx[0].program = VecDeque::from([RStep::Call(RxCall::Close)]);
            let after = before.clone();
            violates(before.transition(&Event::RCall(0), &after), "I3:");
        }
        let mut quiet = world(vec![cell(Phase::Off, 0, Some(16), 0)]);
        quiet.rx[0].handle = Some(1);
        quiet.rx[0].program = VecDeque::from([RStep::Call(RxCall::Close)]);
        let after = quiet.clone();
        assert_eq!(quiet.transition(&Event::RCall(0), &after), Ok(()), "a close on a stopped, empty chain is legal");
    }

    /// Sol E1 finding 5: a command reserved for channel 0 that the device applied on channel 1 (the chain whose log holds it)
    /// does not agree with the reservation, whatever the receive evidence says.
    #[test]
    fn inv_i4_write_applied_on_another_channel_is_violation() {
        let mut before = world(vec![streaming()]);
        before.monitor.reserved.insert(5, (0, seg(0), 1));
        before.device.dsp.entry(1).or_insert_with(super::super::model::Dsp::new).log.push((0, 5));
        before.sync();
        assert_eq!(before.monitor.applied[&5].channel, 1, "the application's channel is the chain's");
        // The receive evidence agrees with the application (the same channel), but not with the reservation's intent.
        before.monitor.evidence.insert(5, Ident { channel: 1, ..ident() });
        let mut after = before.clone();
        after.brain.dispositions.push((5, Disposition::Consumed));
        violates(i4(&before, &Event::BOp, &after), "I4:");
    }

    #[test]
    fn inv_i4_double_disposition_is_violation() {
        let mut before = world(vec![streaming()]);
        before.brain.dispositions.push((5, Disposition::Cancelled));
        let mut after = before.clone();
        after.brain.dispositions.push((5, Disposition::Cancelled));
        violates(before.transition(&Event::BOp, &after), "I4:");
    }

    #[test]
    fn inv_i4_unpublished_drain_evidence_discharges() {
        let mut before = world(vec![streaming()]);
        before.monitor.reserved.insert(5, (0, seg(0), 1));
        before.monitor.applied.insert(5, ident());
        before.monitor.evidence.insert(5, ident());
        let mut consumed = before.clone();
        consumed.brain.dispositions.push((5, Disposition::Consumed));
        assert_eq!(i4(&before, &Event::BOp, &consumed), Ok(()));
        let mut reported = before.clone();
        reported.brain.dispositions.push((5, Disposition::Reported(Reason::Left)));
        violates(i4(&before, &Event::BOp, &reported), "I4:");
    }

    #[test]
    fn inv_i4_unreceived_application_reported_is_legal() {
        let mut before = world(vec![streaming()]);
        before.monitor.applied.insert(5, ident());
        let mut after = before.clone();
        after.brain.dispositions.push((5, Disposition::Reported(Reason::Loss)));
        assert_eq!(i4(&before, &Event::BOp, &after), Ok(()));
    }

    #[test]
    fn inv_i5_cut_below_progress_is_violation() {
        let w = world(vec![cell(Phase::Streaming(seg(0)), 0, Some(4), 8)]);
        violates(w.safety(), "I5:");
    }

    #[test]
    fn inv_i6_old_epoch_past_retiring_cut_is_violation() {
        let mut before = world(vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 6)]);
        before.cells[0].epoch = 2;
        before.cells[0].retiring = Some(Retiring { epoch: 1, cut: Some(8), origin: 0, published: false });
        before.rx[0].recv = Some(RecvCall { recv: recv(1, &[]), deadline: 20 });
        let mut after = before.clone();
        after.cells[0].progress = 10;
        violates(before.transition(&Event::RPublish(0), &after), "I6:");
    }

    #[test]
    fn inv_i7_reservation_while_stopping_is_violation() {
        let mut before = world(vec![cell(Phase::Stopping(seg(0)), 0, Some(16), 0)]);
        before.brain.held.push(held(0, value(0, 10, 5), 1));
        let mut after = before.clone();
        push(&mut after.cells[0], value(0, 10, 5), ValueState::Queued);
        violates(before.transition(&Event::BOp, &after), "I7:");
    }

    #[test]
    fn inv_i8_two_takes_one_sample_is_violation() {
        let mut w = world(vec![streaming()]);
        w.device.open(0, vec![0]);
        w.device.start(0, 0);
        w.device.dsp.get_mut(&0).expect("the chain is open").log = vec![(3, 5), (3, 6)];
        violates(w.safety(), "I8:");
    }

    #[test]
    fn inv_i9_close_after_failure_is_violation() {
        let mut before = world(vec![cell(Phase::Off, 0, Some(16), 0)]);
        before.rx[0].handle = Some(1);
        before.rx[0].program = VecDeque::from([RStep::Call(RxCall::Close)]);
        before.failure = true;
        let after = before.clone();
        violates(before.transition(&Event::RCall(0), &after), "I9:");
    }

    #[test]
    fn inv_i11_in_flight_over_bound_is_violation() {
        let mut c = streaming();
        for id in 1..=IN_FLIGHT_BOUND as u64 {
            push(&mut c, value(0, 10, id), ValueState::Queued);
        }
        let mut before = world(vec![c]);
        before.brain.held.push(held(0, value(0, 10, 9), 1));
        let mut after = before.clone();
        push(&mut after.cells[0], value(0, 10, 9), ValueState::Queued);
        violates(before.transition(&Event::BOp, &after), "I11:");
    }

    #[test]
    fn inv_i12_new_epoch_command_before_ack_is_violation() {
        let mut before = world(vec![streaming()]);
        before.cells[0].retiring = Some(Retiring { epoch: 0, cut: Some(8), origin: 0, published: false });
        let mut after = before.clone();
        after.rx[0].fifo.push_back(RxOp::Start { at: 8, epoch: 1 });
        violates(before.transition(&Event::BOp, &after), "I12:");
    }

    #[test]
    fn inv_i12_old_epoch_start_during_quiesce_is_legal() {
        let mut before = world(vec![streaming()]);
        before.cells[0].retiring = Some(Retiring { epoch: 0, cut: Some(8), origin: 0, published: false });
        let mut after = before.clone();
        after.rx[0].fifo.push_back(RxOp::Start { at: 8, epoch: 0 });
        assert_eq!(before.transition(&Event::BOp, &after), Ok(()));
    }

    #[test]
    fn inv_i12_close_before_ack_is_violation() {
        let mut before = world(vec![cell(Phase::Off, 0, Some(16), 0)]);
        before.cells[0].retiring = Some(Retiring { epoch: 0, cut: Some(8), origin: 0, published: false });
        let mut after = before.clone();
        after.rx[0].fifo.push_back(RxOp::Close);
        violates(before.transition(&Event::BOp, &after), "I12:");
    }

    #[test]
    fn inv_i13_restore_from_uncertain_is_violation() {
        let before = world(vec![cell(Phase::Uncertain, 0, None, 0)]);
        let mut after = before.clone();
        after.settings.fifo.push_back(SetCmd { id: 9, change: Change::Restore { channel: 0 } });
        violates(before.transition(&Event::BOp, &after), "I13:");
    }

    #[test]
    fn inv_i14_admission_after_loss_is_violation() {
        let mut before = world(vec![cell(Phase::Lost, 0, None, 0)]);
        before.failure = true;
        let mut after = before.clone();
        after.rx[0].program = VecDeque::from([RStep::Call(RxCall::Open)]);
        violates(before.transition(&Event::RGate(0), &after), "I14:");
    }

    #[test]
    fn inv_i15_restore_with_left_on_channel_is_violation() {
        let mut c = streaming();
        c.in_flight.push(InFlight { entry: value(0, 10, 5), state: ValueState::Queued, left: true });
        let before = world(vec![c]);
        let mut after = before.clone();
        after.settings.fifo.push_back(SetCmd { id: 9, change: Change::Restore { channel: 0 } });
        violates(before.transition(&Event::BOp, &after), "I15:");
    }

    #[test]
    fn inv_i15_restore_with_debt_on_other_stream_is_violation() {
        let mut debtor = streaming();
        push(&mut debtor, value(0, 10, 5), ValueState::Done(Terminal::Call(Outcome::Ok)));
        let before = world(vec![cell(Phase::Streaming(seg(0)), 1, Some(16), 0), debtor]);
        let mut after = before.clone();
        after.settings.fifo.push_back(SetCmd { id: 9, change: Change::Restore { channel: 0 } });
        violates(before.transition(&Event::BOp, &after), "I15:");
    }

    #[test]
    fn inv_i16_drain_ended_waits_for_pin_is_violation() {
        let mut before = world(vec![cell(Phase::Stopping(seg(0)), 0, Some(8), 0)]);
        before.cells[0].drain = Drain::Draining;
        before.brain.ops = VecDeque::from([Op::Section(Section::TimerFired { stream: 0 })]);
        let after = before.clone();
        violates(before.transition(&Event::BOp, &after), "I16:");
    }

    #[test]
    fn prog_p1_observed_deadline_stops_r() {
        let mut w = world(vec![cell(Phase::Stopping(seg(0)), 0, Some(8), 0)]);
        w.cells[0].drain = Drain::Draining;
        w.monitor.drain_observed.insert((0, 8 + DRAIN_WINDOW));
        violates(w.progress(), "P1:");
    }

    /// Case 6 seeds 13, 30, 39 ...: the deadline was delivered, then the brain panicked before it handled it.
    #[test]
    fn prog_p1_dead_brain_makes_no_claim() {
        let mut w = world(vec![cell(Phase::Stopping(seg(0)), 0, Some(8), 0)]);
        w.cells[0].drain = Drain::Draining;
        w.monitor.drain_observed.insert((0, 8 + DRAIN_WINDOW));
        w.brain.exit = super::super::core::Exit::Dead;
        assert_eq!(w.progress(), Ok(()));
    }

    #[test]
    fn prog_p1_unobserved_deadline_makes_no_claim() {
        let mut w = world(vec![cell(Phase::Stopping(seg(0)), 0, Some(8), 0)]);
        w.cells[0].drain = Drain::Draining;
        assert_eq!(w.progress(), Ok(()));
    }

    #[test]
    fn prog_p2_undisposed_value_is_violation() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.monitor.shutdown = true;
        w.monitor.released.insert(5);
        violates(w.progress(), "P2:");
    }

    #[test]
    fn prog_p2_command_without_terminal_is_violation() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.monitor.commanded.insert(9);
        violates(w.progress(), "P2:");
    }

    #[test]
    fn prog_p2_shutdown_settles_held_by_settle_rule() {
        let mut w = world(vec![streaming()]);
        w.brain.held.push(held(0, value(0, 10, 5), 1));
        w.monitor.released.insert(5);
        w.inbox.push_back(Input::Shutdown);
        settle(&mut w);
        assert_eq!(w.progress(), Ok(()));
        assert_eq!(w.brain.disposition(5), Some(Disposition::Cancelled));
    }

    #[test]
    fn prog_p2_after_panic_accepts_executor_terminal() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.monitor.shutdown = true;
        w.monitor.panicked = true;
        w.monitor.released.insert(5);
        w.monitor.terminals.insert(5, Terminal::Abandoned);
        w.settings.exited = true;
        w.rx[0].exited = true;
        assert_eq!(w.progress(), Ok(()));
    }

    #[test]
    fn prog_p2_after_panic_value_never_issued_is_settled() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.monitor.shutdown = true;
        w.monitor.panicked = true;
        w.monitor.released.insert(5);
        w.brain.held.push(held(0, value(0, 10, 5), 1));
        w.settings.exited = true;
        w.rx[0].exited = true;
        assert_eq!(w.progress(), Ok(()));
    }

    #[test]
    fn prog_p2_after_panic_command_without_terminal_is_violation() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.monitor.shutdown = true;
        w.monitor.panicked = true;
        w.monitor.commanded.insert(9);
        w.settings.exited = true;
        w.rx[0].exited = true;
        violates(w.progress(), "P2:");
    }

    #[test]
    fn prog_p2_after_panic_value_lost_in_flight_is_violation() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.monitor.shutdown = true;
        w.monitor.panicked = true;
        w.monitor.released.insert(5);
        w.settings.exited = true;
        w.rx[0].exited = true;
        violates(w.progress(), "P2:");
    }

    #[test]
    fn prog_p3_settings_not_exited_is_violation() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.monitor.shutdown = true;
        w.rx[0].exited = true;
        violates(w.progress(), "P3:");
    }

    #[test]
    fn prog_p3_stuck_actor_is_violation() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.monitor.shutdown = true;
        violates(w.progress(), "P3:");
    }

    #[test]
    fn prog_p4_pending_work_without_event_is_violation() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.brain.ops = VecDeque::from([Op::Section(Section::TimerFired { stream: 0 })]);
        violates(w.progress(), "P4:");
    }

    #[test]
    fn prog_p4_wait_at_epilogue_end_is_incomplete_not_violation() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.brain.ops = VecDeque::from([Op::Section(Section::TimerFired { stream: 0 })]);
        w.device.t = w.params.horizon + w.params.epilogue;
        assert_eq!(w.progress(), Ok(()));
    }

    #[test]
    fn inv_i4_terminal_overwrite_is_violation() {
        let mut before = world(vec![streaming()]);
        before.monitor.terminals.insert(9, Terminal::Call(Outcome::Ok));
        let mut after = before.clone();
        after.monitor.terminals.insert(9, Terminal::Call(Outcome::Refused));
        violates(i4(&before, &Event::BOp, &after), "I4:");
    }

    /// Takes the first enabled event that is not a tick, and a tick only when nothing else is
    /// enabled, until no event is enabled.
    fn settle(w: &mut World) {
        for _ in 0..10_000 {
            let events = w.enabled();
            let Some(event) = events.iter().find(|e| **e != Event::Tick).or(events.first()).cloned() else { return };
            w.apply(&event);
        }
        panic!("the world did not settle within the budget");
    }

    /// The happy world: one stream, a value released at tick 1 and consumed (accepted at sample `e`),
    /// a Stop requested at 4 and delivered at `stop`, and a Shutdown at `shutdown`. Open is delivered at
    /// 0 and StartNext at 1: the environment slots are otherwise independent, and a StartNext before
    /// Open is not a legal input.
    fn happy_world(e: i64, horizon: i64, epilogue: i64, stop: i64, shutdown: i64) -> World {
        let env = [
            (Input::Request(Request::Open { stream: 0 }), 0),
            (Input::Request(Request::StartNext { stream: 0, at: 1 }), 1),
            (Input::Release(held(0, value(0, e, 5), 1)), 1),
            (Input::Stop { stream: 0, requested: 4.min(horizon) }, stop),
            (Input::Shutdown, shutdown),
        ]
        .map(|(input, at)| EnvSlot { input: EnvInput::Input(input), earliest: at, latest: at, delivered: false, after: None })
        .to_vec();
        // A recv timeout and a drain window of one tick: after the epilogue's end time goes on silently until the drain deadline
        // (the Stop's cut plus the window) and the recv's timeout, and every silent tick multiplies the states.
        let params = Params { horizon, epilogue, recv_timeout: 1, drain_window: 1, ..params() };
        World::new(vec![cell(Phase::Off, 0, None, 0)], env, params)
    }

    /// One legal run, taken with non-tick events first: every step and state passes the oracles, and
    /// the run ends complete.
    #[test]
    fn oracle_legal_run_passes_every_step() {
        let mut w = happy_world(2, 8, 12, 5, 12);
        for _ in 0..10_000 {
            let events = w.enabled();
            let Some(event) = events.iter().find(|e| **e != Event::Tick).or(events.first()).cloned() else {
                assert_eq!(w.incomplete(), None);
                assert_eq!(w.progress(), Ok(()));
                return;
            };
            let mut after = w.clone();
            after.apply(&event);
            assert_eq!(w.transition(&event, &after), Ok(()), "step {event:?}");
            assert_eq!(after.safety(), Ok(()), "after {event:?}");
            w = after;
        }
        panic!("the legal run did not end within the budget");
    }

    const HAPPY_WITNESSES: &[&str] = &["value applied", "value consumed", "publication", "disposition written", "command terminal", "drain ended"];

    /// Exhaustive search of the smallest happy world that keeps the value, the Stop and the Shutdown:
    /// the value at sample 1, the Stop at 2 (asking for 1), the Shutdown at 2, horizon 1, epilogue 1, recv timeout and drain
    /// window of one tick. The search is complete: 215,949 states, 516,848 transitions, about 5 s in release on sim02
    /// (165,497 before a start below `now` could be missed, E1 fix round 2: the StartNext at 1 is delivered at 1, so a lagging
    /// actor misses it; 64,763 before the key kept the witness state a later check reads and time went on silently after
    /// the end); every oracle passes, and all six declared witnesses are reached.
    #[test]
    fn oracle_happy_path_passes() {
        let config = Config { name: "oracle happy path", budget: 300_000, wall: Duration::MAX, witnesses: HAPPY_WITNESSES };
        let verdict = explore(&config, happy_world(1, 1, 1, 2, 2));
        println!("{}", verdict.report());
        assert_complete(&verdict);
    }

    /// The full-length happy world (value at sample 2, horizon 8, epilogue 12, Stop at 5, Shutdown at
    /// 12), searched with a 2M budget and a 180 s wall. Not run by default. Measured in release on sim02
    /// (E1b part 3c): the budget runs out at 2,000,000 states (5,549,181 transitions, 62 s; 5,941,306
    /// transitions and 122 s before the canonical key, part 3a), 108 transitions still open, no
    /// violation, all six declared witnesses reached and deadline observed too. Not complete: the
    /// state space grows far past the budget with the horizon, so this is a bounded check only.
    #[test]
    #[ignore = "bounded deep search, about 2 min"]
    fn oracle_happy_path_full() {
        let config = Config { name: "oracle happy path full", budget: 2_000_000, wall: Duration::from_secs(180), witnesses: HAPPY_WITNESSES };
        let verdict = explore(&config, happy_world(2, 8, 12, 5, 12));
        println!("{}", verdict.report());
        assert!(matches!(verdict.outcome, super::super::dst::Outcome::BudgetExceeded { .. }), "{}", verdict.report());
    }
}
