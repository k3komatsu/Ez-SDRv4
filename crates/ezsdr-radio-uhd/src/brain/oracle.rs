//! The oracles of design note 45's experiment (stage E1b, part 2b; 45-experiment §5, test plan
//! section E). Each property is one check that returns `Err` with a text starting with its id
//! (`"I4: ..."`). The checks sit at their evaluation points: a property about the current state is
//! in [`safety`], one about what a step did is in [`transition`], and the progress properties are in
//! [`progress`], checked at terminal states. Safety reads only the key; a fact a check needs from the
//! past is in the monitor (world.rs), never a history.

use super::cell::{Drain, Outcome, Phase, Terminal, ValueState};
use super::core::{Change, Command, Disposition, Op, RxOp, Section, SetCmd};
use super::dst::Model;
use super::rules::RxCall;
use super::world::{Cleanup, Event, Ident, PubStep, RStep, SStep, Who, World};

/// The safety checks of every state: I5 (the cut in force is not below the published progress)
/// and I8 (a chain applies at most one write per sample, in order).
pub(super) fn safety(w: &World) -> Result<(), String> {
    i5_state(w)?;
    i8(w)?;
    p6(w)
}

/// The transition checks of one step: `before` is the state the event was taken in, `after` the
/// state it produced.
pub(super) fn transition(before: &World, event: &Event, after: &World) -> Result<(), String> {
    type StepCheck = fn(&World, &Event, &World) -> Result<(), String>;
    let checks: [StepCheck; 22] = [i1, i7, i2, i3, i4, i5, i6, i9, i11, i12, i13, i14, i15, i16, i17, capacity_timer_inert, p6a, p6b, i18, i19, i20, i21];
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
    p4(w)?;
    i19_progress(w)
}

/// Two facts name the same reservation: the value, its channel, segment and epoch.
fn same_reservation(a: &Ident, b: &Ident) -> bool {
    a.value == b.value && a.channel == b.channel && a.segment == b.segment && a.epoch == b.epoch
}

/// A consumption fact agrees with the value's device application: the same reservation and, for
/// evidence, a sample at or after the application's (the acceptance is recorded where the call returns, which can be after the device
/// applied the write: the host then credits a later sample, never an earlier one). A leftover consumption comes at a later sample by design (u11b). The
/// channel of each observed fact is the device's (the DSP chain, the receiving streamer), so the
/// fact is also compared with the reservation's intent: a write that reached another channel disagrees.
fn agrees(w: &World, id: u64, fact: &Ident, same_sample: bool) -> bool {
    let intended = w.monitor.reserved.get(&id).is_some_and(|&(channel, segment, epoch)| (channel, segment, epoch) == (fact.channel, fact.segment, fact.epoch));
    match w.monitor.applied.get(&id) {
        Some(applied) => intended && same_reservation(applied, fact) && (!same_sample || applied.sample <= fact.sample),
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

/// K as the monitor sees it at the true device time: the values reserved whose command is queued, taken or running (no terminal, not
/// cancelled), plus those that finished ok or partly and whose instant has not come. Written from the monitor's facts alone, a second
/// implementation of the brain's `counts_for_k` on purpose (I11′).
fn k_load(w: &World) -> usize {
    k_load_at(w, w.device.t)
}

/// [`k_load`] at device time `t`: the true time, or the time the brain knows (P6's step check reads the state the step itself saw).
fn k_load_at(w: &World, t: i64) -> usize {
    w.monitor
        .reserved
        .keys()
        .filter(|id| {
            let lost = w.monitor.streams.get(id).is_some_and(|k| w.cells[*k].phase == Phase::Lost);
            match w.monitor.terminals.get(id) {
                None => !w.monitor.cancelled.contains(id),
                Some(Terminal::Call(Outcome::Ok | Outcome::Partial)) => !lost && w.monitor.instants.get(id).is_some_and(|e| t < *e),
                Some(_) => false,
            }
        })
        .count()
}

/// D of stream `k` as the monitor sees it: the values reserved on it, minus those whose `Done` the brain took and whose obligation is over
/// (a terminal other than ok or partial, a consumption seen, or a lost chain).
fn d_resident(w: &World, k: usize) -> usize {
    let lost = w.cells[k].phase == Phase::Lost;
    w.monitor
        .streams
        .iter()
        .filter(|&(_, stream)| *stream == k)
        .filter(|&(id, _)| {
            let over = matches!(w.monitor.terminals.get(id), Some(t) if !matches!(t, Terminal::Call(Outcome::Ok | Outcome::Partial)))
                || w.monitor.evidence.contains_key(id)
                || w.monitor.leftovers.contains_key(id)
                || lost;
            !(w.monitor.done_processed.contains(id) && over)
        })
        .count()
}

/// I11′: a reservation is made only while the device is below K and the chain below D, each counted from the monitor's facts.
fn i11(before: &World, _event: &Event, after: &World) -> Result<(), String> {
    for (k, id) in reserved_by(before, after) {
        let (load, resident) = (k_load(before), d_resident(before, k));
        if load >= before.params.k {
            return Err(format!("I11: value {id} reserved on stream {k} with {load} values counted against the device, K is {}", before.params.k));
        }
        if resident >= before.params.d {
            return Err(format!("I11: value {id} reserved on stream {k} with {resident} resident values, D is {}", before.params.d));
        }
    }
    Ok(())
}

/// D of stream `k` as the brain can know it (P6): the values reserved on it, minus those whose `Done` the brain took and whose obligation the
/// brain has seen over (a terminal other than ok or partial, a consumption the brain took, or a lost chain). A consumption the receive
/// executor published but the brain has not taken yet does not free a place the brain cannot yet use.
fn d_known(w: &World, k: usize) -> usize {
    let lost = w.cells[k].phase == Phase::Lost;
    w.monitor
        .streams
        .iter()
        .filter(|&(_, stream)| *stream == k)
        .filter(|&(id, _)| {
            let consumed = w.monitor.dispositions.get(id) == Some(&Disposition::Consumed) || w.brain.leftovers.contains(id);
            let over = matches!(w.monitor.terminals.get(id), Some(t) if !matches!(t, Terminal::Call(Outcome::Ok | Outcome::Partial))) || consumed || lost;
            !(w.monitor.done_processed.contains(id) && over)
        })
        .count()
}

/// Whether `head`, the head of the held list, could be reserved in `w` by the cell's own conditions.
fn head_reservable(w: &World, head: &super::core::Held) -> bool {
    w.cells[head.stream].clone().reserve(head.entry.clone(), head.generation).is_ok()
}

/// P6 (the progress half of I11′), the state check: at a state with nothing in flight toward the brain (empty inbox and operations, no
/// pending send of an executor) the head of the held list is not reservable by the cell's conditions, K at the true device time and D
/// together, unless a capacity timer already due will deliver the time. A missing or late timer, or time read from R's progress, shows here.
fn p6(w: &World) -> Result<(), String> {
    let executors_quiet = !w.settings.program.iter().any(|step| matches!(step, SStep::Send)) && !w.rx.iter().any(|r| r.program.iter().any(|step| matches!(step, RStep::Send(_))));
    let quiet = w.inbox.is_empty() && executors_quiet && w.brain.ops.is_empty() && !w.brain.dead() && w.brain.exit == super::core::Exit::Running;
    let Some(head) = w.brain.held.first().filter(|_| quiet) else { return Ok(()) };
    if w.timers.iter().any(|(timer, at)| *timer == super::core::Timer::Capacity && *at <= w.device.t) {
        return Ok(());
    }
    let k = head.stream;
    if head_reservable(w, head) && k_load(w) < w.params.k && d_known(w, k) < w.params.d {
        return Err(format!("P6: the held head {} on stream {k} is reservable (K load {}, D {}) and no re-evaluation is due", head.entry.order.1, k_load(w), d_known(w, k)));
    }
    Ok(())
}

/// P6a, in the section: at a reservation section event (`Reserve` of the held head, the one section that tries held values) the cells at
/// that very event, K at the brain's time and D decide: if the head is reservable, the section reserves it. The section is one atomic event, so
/// no executor step can matter; this catches a wrong retry decision.
fn p6a(before: &World, event: &Event, after: &World) -> Result<(), String> {
    let (Event::BOp, Some(Op::Section(Section::Reserve { id })), Some(head)) = (event, before.brain.ops.front(), before.brain.held.first()) else { return Ok(()) };
    if head.entry.order.1 != *id || before.brain.dead() {
        return Ok(());
    }
    let k = head.stream;
    let reservable = head_reservable(before, head) && k_load_at(before, before.brain.now) < before.params.k && d_known(before, k) < before.params.d;
    if reservable && after.brain.held.iter().any(|held| held.entry.order.1 == *id) {
        return Err(format!("P6a: the reservation section left the held head {id} on stream {k} held although the cells, K (at the brain's time {}) and D allow it", before.brain.now));
    }
    Ok(())
}

/// P6b, the obligation: a brain step that took an input of the freeing set (every input but a release, the shutdown and a panic: each can free
/// a bound or open a reservation condition) and ends with values held ran a `Reserve` of the held head among its operations (a `ReserveAll` alone does not count). Checked when the
/// step's operations are done, from the monitor's record of the step; nothing is exempt, neither an executor step between the operations nor
/// anything queued behind the step.
fn p6b(_before: &World, event: &Event, after: &World) -> Result<(), String> {
    if !matches!(event, Event::BOp) || !after.brain.ops.is_empty() || after.brain.dead() {
        return Ok(());
    }
    if after.monitor.step_freeing && !after.monitor.step_retried && !after.brain.held.is_empty() {
        return Err(format!("P6b: a step that took a freeing input ended with {} values held and ran no reservation section", after.brain.held.len()));
    }
    Ok(())
}

/// I18: the loss record and the native `mark_lost` come after the failure flag is set and after the publisher released the gate (the order of one
/// publication is gate, `F`, ungate, record, mark).
fn i18(before: &World, event: &Event, _after: &World) -> Result<(), String> {
    match event {
        Event::Pub(holder, step @ (PubStep::Record | PubStep::Mark)) if !before.failure => Err(format!("I18: {holder:?} publishes {step:?} before F is set")),
        Event::Pub(holder, step @ (PubStep::Record | PubStep::Mark)) if before.gate == Some(*holder) => Err(format!("I18: {holder:?} publishes {step:?} while it still holds the gate")),
        _ => Ok(()),
    }
}

/// I19 (handles): cleanup decides to close only when `F` is clear and nothing is detached; it closes nothing when something is detached; no
/// handle is destroyed while `F` is set or something is detached (the destructor reads the native mark, so a missing mark shows here).
fn i19(before: &World, event: &Event, after: &World) -> Result<(), String> {
    let (Some(was), Some(now)) = (&before.teardown, &after.teardown) else { return Ok(()) };
    match event {
        Event::CleanupDecide if now.decision == Some(Cleanup::Close) && (before.failure || !was.detached.is_empty()) => {
            Err("I19: cleanup decided to close the streamers although F is set or an executor is detached".to_string())
        }
        Event::CleanupClose(k) if !was.detached.is_empty() => Err(format!("I19: cleanup closes stream {k}'s handle although an executor is detached")),
        Event::OwnerRelease if now.destroyed.len() > was.destroyed.len() && before.failure => {
            Err(format!("I19: the native device's final release freed a handle while F is set (native mark {})", before.monitor.marked))
        }
        Event::OwnerRelease if was.detached.iter().any(|j| !ended(before, *j)) => Err("I19: the native device was released while a detached thread still holds it".to_string()),
        Event::Destroy(k) if now.destroyed.len() > was.destroyed.len() && (before.failure || !was.detached.is_empty()) => {
            Err(format!("I19: stream {k}'s handle is destroyed while F is set or an executor is detached (native mark {})", before.monitor.marked))
        }
        _ => Ok(()),
    }
}

/// I19, the progress half: at a terminal state where cleanup decided to close, every handle was closed, and each closed handle's destructor ran
/// (it freed the handle, or retained it because the native mark was set).
fn i19_progress(w: &World) -> Result<(), String> {
    let Some(td) = w.teardown.as_ref().filter(|td| td.decision == Some(Cleanup::Close)) else { return Ok(()) };
    if let Some(k) = w.rx.iter().position(|r| r.handle.is_some()) {
        return Err(format!("I19: cleanup decided to close but stream {k}'s handle is still open at the end"));
    }
    if !td.pending.is_empty() || td.closed.iter().any(|k| !td.destroyed.iter().chain(&td.retained).any(|(stream, _)| stream == k)) {
        return Err("I19: a closed handle's destructor did not run at the end".to_string());
    }
    Ok(())
}

/// Whether joinee `i` of the stopper has ended, as the oracle reads it (a second implementation of the world's `joinee_done`, checked at the
/// moment of a join; a joined thread stays ended): `S`, then each receive executor (an unspawned one has nothing to end), then the brain:
/// finished or dead, nothing left to take or do, no booking pending.
fn ended(w: &World, i: usize) -> bool {
    match i {
        0 => w.settings.unspawned || (w.settings.exited && w.settings.terminate && w.settings.fifo.is_empty() && w.settings.program.is_empty() && w.settings.hand.is_none()),
        i if i <= w.rx.len() => (w.rx[i - 1].exited && w.rx[i - 1].program.is_empty()) || w.rx[i - 1].unspawned,
        _ => w.brain_exited,
    }
}

/// I20: a detach only after the join limit has passed in virtual time; a join only of a joinee that has ended; cleanup's decision only when every
/// joinee has ended or been detached; nothing closed or destroyed with an executor detached (also I19).
fn i20(before: &World, event: &Event, after: &World) -> Result<(), String> {
    let (Some(was), Some(now)) = (&before.teardown, &after.teardown) else { return Ok(()) };
    // A brain thread that returned after terminating takes nothing more: what is in its inbox then (queued before it returned, or sent after) is
    // lost. A dead brain (its guard ran) drops its inbox by design.
    if after.brain_exited && !after.brain.dead() && !after.inbox.is_empty() {
        return Err("I20: an input is queued for a brain whose thread has returned".to_string());
    }
    match event {
        Event::Detach if before.device.t < was.since + was.join_limit => {
            Err(format!("I20: joinee {} detached at {} before the join limit ({} from {})", was.next, before.device.t, was.join_limit, was.since))
        }
        Event::Join if !ended(before, was.next) => Err(format!("I20: joinee {} joined before it ended", was.next)),
        Event::CleanupDecide => {
            let pending = (0..2 + before.rx.len()).find(|i| !was.detached.contains(i) && !was.joined.contains(i));
            match pending {
                Some(i) => Err(format!("I20: cleanup began while joinee {i} had neither ended nor been detached")),
                None => Ok(()),
            }
        }
        Event::CleanupClose(_) | Event::Destroy(_) if !now.detached.is_empty() && (now.closed.len() > was.closed.len() || now.destroyed.len() > was.destroyed.len()) => {
            Err("I20: cleanup closed or destroyed a handle with an executor detached".to_string())
        }
        _ => Ok(()),
    }
}

/// I21: no command is routed to a stream whose executor was never spawned; the brain's spawn-failure step abandons only the failing stream and
/// only when its start had succeeded (`Uncertain`, a loss already known kept), and no other stream's phase changes.
fn i21(before: &World, event: &Event, after: &World) -> Result<(), String> {
    for command in routed(before, after) {
        match command {
            Command::Rx { stream, op } if after.rx[stream].unspawned => {
                return Err(format!("I21: {op:?} routed to stream {stream}, whose executor was never spawned"));
            }
            Command::Settings(cmd) if after.settings.unspawned => return Err(format!("I21: settings command {} routed to the control thread, which was never spawned", cmd.id)),
            _ => {}
        }
    }
    if after.settings.unspawned && after.settings.terminate {
        return Err("I21: the brain terminated the control thread, which was never spawned".to_string());
    }
    if let (Event::BOp, Some(Op::Section(Section::SpawnFailed { stream, started }))) = (event, before.brain.ops.front()) {
        for k in 0..before.cells.len() {
            let (was, now) = (before.cells[k].phase, after.cells[k].phase);
            let want = if k == *stream && *started && was != Phase::Lost { Phase::Uncertain } else { was };
            if now != want {
                return Err(format!("I21: spawn failure of stream {stream} (started {started}) moved stream {k} from {was:?} to {now:?}, expected {want:?}"));
            }
        }
    }
    Ok(())
}

/// I17: a cell that is `Stopping` becomes `Armed` or `Streaming` only at the executor's completed start, the one the brain issued for the
/// epoch the cell is in (the call carries the epoch of its `Start`).
fn i17(before: &World, event: &Event, after: &World) -> Result<(), String> {
    for (k, cell) in after.cells.iter().enumerate() {
        if !matches!(before.cells[k].phase, Phase::Stopping(_)) || !matches!(cell.phase, Phase::Armed(_) | Phase::Streaming(_)) {
            continue;
        }
        let confirmed = match (event, before.rx[k].program.front()) {
            (Event::RResult(j), Some(RStep::Result(RxCall::Begin { epoch, .. }))) => *j == k && *epoch == cell.epoch && before.monitor.starts.contains(&(k, *epoch)),
            _ => false,
        };
        if !confirmed {
            return Err(format!("I17: stream {k} was Stopping and is now {:?} without a completed start the brain issued for epoch {}", cell.phase, cell.epoch));
        }
    }
    Ok(())
}

/// A capacity timer's operation only re-evaluates: it changes no cell's phase, drain, outstanding command or epoch, and routes no receive command.
fn capacity_timer_inert(before: &World, event: &Event, after: &World) -> Result<(), String> {
    if !matches!((event, before.brain.ops.front()), (Event::BOp, Some(Op::Section(Section::CapacityTimer)))) {
        return Ok(());
    }
    let same = before.cells.iter().zip(&after.cells).all(|(a, b)| (a.phase, a.drain, a.outstanding, a.epoch) == (b.phase, b.drain, b.outstanding, b.epoch));
    let sends = routed(before, after).iter().any(|command| matches!(command, Command::Rx { .. }));
    if !same || sends {
        return Err("B8: a capacity timer changed a cell's phase, drain, command or epoch, or sent a receive command".to_string());
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
        if !w.settings.exited && !w.settings.unspawned {
            return Err("P3: S did not exit after shutdown".to_string());
        }
        for (k, rx) in w.rx.iter().enumerate() {
            if !rx.exited && !rx.unspawned {
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
        Params {
            horizon: 8,
            epilogue: 8,
            recv_timeout: 8,
            queue_cap: 4,
            fifo_cap: 4,
            drain_window: DRAIN_WINDOW,
            packet: crate::brain::world::PACKET,
            call_faults: false,
            panic_step: None,
            k: crate::brain::core::DEVICE_BOUND,
            d: crate::brain::core::CHAIN_BOUND,
        }
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

    /// The acceptance is recorded where the call returns, which can be after the device applied the write: the host then credits a later
    /// sample (legal); a sample before the application is never evidence.
    #[test]
    fn inv_i4_evidence_after_the_application_is_legal_and_before_it_is_not() {
        let applied_at = |sample: i64| Ident { sample, ..ident() };
        let mut before = world(vec![streaming()]);
        before.monitor.reserved.insert(5, (0, seg(0), 1));
        before.monitor.applied.insert(5, applied_at(4));
        let mut later = before.clone();
        later.monitor.evidence.insert(5, applied_at(6));
        assert_eq!(i4(&before, &Event::BOp, &later), Ok(()));
        let mut same = before.clone();
        same.monitor.evidence.insert(5, applied_at(4));
        assert_eq!(i4(&before, &Event::BOp, &same), Ok(()));
        let mut earlier = before.clone();
        earlier.monitor.evidence.insert(5, applied_at(3));
        violates(i4(&before, &Event::BOp, &earlier), "I4:");
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
        before.rx[0].recv = Some(RecvCall { recv: recv(1, &[]), deadline: 20, late: None });
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

    /// A value the monitor knows: reserved on stream 0 with instant `e`, and its settings terminal if any.
    fn known(w: &mut World, id: u64, e: i64, terminal: Option<Terminal>) {
        w.monitor.reserved.insert(id, (0, seg(0), 1));
        w.monitor.streams.insert(id, 0);
        w.monitor.instants.insert(id, e);
        if let Some(terminal) = terminal {
            w.monitor.terminals.insert(id, terminal);
        }
    }

    /// `before` with value 9 held, and `after` with it reserved (the transition under test).
    fn reserving(before: &mut World) -> World {
        before.brain.held.push(held(0, value(0, 12, 9), 1));
        let mut after = before.clone();
        after.brain.held.clear();
        push(&mut after.cells[0], value(0, 12, 9), ValueState::Queued);
        after
    }

    fn k_world(k: usize, d: usize) -> World {
        let mut w = world(vec![streaming()]);
        w.params.k = k;
        w.params.d = d;
        w
    }

    #[test]
    fn inv_i11_device_over_k_is_violation() {
        let mut before = k_world(2, 64);
        known(&mut before, 1, 20, None);
        known(&mut before, 2, 20, Some(Terminal::Call(Outcome::Ok)));
        before.device.t = 5;
        let after = reserving(&mut before);
        violates(before.transition(&Event::BOp, &after), "I11:");
    }

    #[test]
    fn inv_i11_k_frees_at_the_instant_and_not_for_refused_dropped_or_cancelled() {
        let mut before = k_world(2, 64);
        known(&mut before, 1, 20, Some(Terminal::Call(Outcome::Ok)));
        known(&mut before, 2, 20, Some(Terminal::Call(Outcome::Ok)));
        before.device.t = 20;
        let after = reserving(&mut before);
        assert_eq!(before.transition(&Event::BOp, &after), Ok(()), "a finished value is not counted once the device time reaches its instant");
        for terminal in [Terminal::Call(Outcome::Refused), Terminal::Dropped, Terminal::Abandoned] {
            let mut before = k_world(1, 64);
            known(&mut before, 1, 99, Some(terminal));
            let after = reserving(&mut before);
            assert_eq!(before.transition(&Event::BOp, &after), Ok(()), "{terminal:?} is not counted");
        }
        let mut before = k_world(1, 64);
        known(&mut before, 1, 99, None);
        before.monitor.cancelled.insert(1);
        let after = reserving(&mut before);
        assert_eq!(before.transition(&Event::BOp, &after), Ok(()), "a cancelled value is not counted");
    }

    #[test]
    fn inv_i11_chain_over_d_is_violation_and_a_consumed_done_value_is_not_resident() {
        let mut before = k_world(64, 2);
        known(&mut before, 1, 3, Some(Terminal::Call(Outcome::Ok)));
        known(&mut before, 2, 3, Some(Terminal::Call(Outcome::Ok)));
        before.device.t = 50;
        let after = reserving(&mut before);
        violates(before.transition(&Event::BOp, &after), "I11:");
        // Done taken and consumed: value 1 has left the cell.
        before.monitor.done_processed.insert(1);
        before.monitor.evidence.insert(1, ident());
        let after = reserving(&mut before);
        assert_eq!(before.transition(&Event::BOp, &after), Ok(()));
    }

    #[test]
    fn inv_p6_a_reservable_head_with_nothing_in_flight_is_violation() {
        let mut w = k_world(1, 64);
        w.brain.held.push(held(0, value(0, 12, 9), 1));
        violates(w.safety(), "P6:");
    }

    #[test]
    fn inv_p6_is_silent_while_an_input_is_on_its_way_or_a_due_timer_waits() {
        let mut w = k_world(1, 64);
        w.brain.held.push(held(0, value(0, 12, 9), 1));
        let mut busy = w.clone();
        busy.inbox.push_back(Input::CapacityTimer { now: 0 });
        assert_eq!(busy.safety(), Ok(()), "an input on its way");
        let mut due = w.clone();
        due.timers.push((crate::brain::core::Timer::Capacity, 0));
        assert_eq!(due.safety(), Ok(()), "a due capacity timer");
        let mut blocked = w.clone();
        known(&mut blocked, 1, 99, None);
        assert_eq!(blocked.safety(), Ok(()), "K is full");
    }

    /// The Done step of a K = 1 world (A finished past its instant, B held, `inbox = [Done(A, now >= e_A), Shutdown, tail]`), taken event by event
    /// with every oracle checked: `interleave` runs an executor step (`RCheck`, which changes no cell and sends nothing) right after the
    /// `BStep`. Returns the world after the step's operations.
    fn done_step(tail: &[Input], interleave: bool) -> World {
        let mut c = streaming();
        push(&mut c, value(0, 10, 1), ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut w = k_world(1, 64);
        w.cells[0] = c;
        w.rx[0].handle = Some(1);
        known(&mut w, 1, 10, Some(Terminal::Call(Outcome::Ok)));
        w.brain.held.push(held(0, value(0, 14, 2), 1));
        w.inbox = VecDeque::from([Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 12 }, Input::Shutdown]);
        w.inbox.extend(tail.iter().cloned());
        w.device.t = 12;
        let mut events = vec![Event::BStep];
        if interleave {
            events.push(Event::RCheck(0));
        }
        for event in events.into_iter().chain(std::iter::repeat_n(Event::BOp, 8)) {
            if event == Event::BOp && w.brain.ops.is_empty() {
                break;
            }
            let before = w.clone();
            w.apply(&event);
            before.transition(&event, &w).unwrap_or_else(|why| panic!("{tail:?} {event:?}: {why}"));
        }
        w
    }

    /// P6b: the step that took the freeing `Done` runs a reservation section and reserves B before any queued input is taken, with three queue
    /// tails behind the `Shutdown` (nothing, an unrelated boundary, a capacity timer) and with an executor step in between.
    #[test]
    fn inv_p6b_the_step_that_frees_k_reserves_the_held_head_whatever_is_queued_behind_it() {
        let tails: [&[Input]; 3] = [&[], &[Input::Boundary(super::super::core::Boundary::FirstSample { stream: 0 })], &[Input::CapacityTimer { now: 12 }]];
        for tail in tails {
            for interleave in [false, true] {
                let w = done_step(tail, interleave);
                assert!(w.monitor.step_freeing && w.monitor.step_retried, "{tail:?} {interleave}: the step ran its reservation section");
                assert!(w.cells[0].state_of(2).is_some() && w.brain.held.is_empty(), "{tail:?} {interleave}: B was reserved by the Done's step");
                assert_eq!(w.inbox.len(), 1 + tail.len(), "{tail:?}: the Shutdown and the tail are still queued");
                // The same step with the retry omitted: the record says no reservation section ran, and B is still held. Neither the queue
                // behind the step nor an executor step in between excuses it.
                let mut omitted = w.clone();
                omitted.monitor.step_retried = false;
                omitted.cells[0].in_flight.retain(|f| f.entry.order.1 != 2);
                omitted.brain.held.push(held(0, value(0, 14, 2), 1));
                violates(omitted.transition(&Event::BOp, &omitted.clone()), "P6b:");
                // A step of an input that does not free (a release) owes no retry, and a step that left nothing held owes none.
                let mut release = omitted.clone();
                release.monitor.step_freeing = false;
                assert_eq!(release.transition(&Event::BOp, &release.clone()), Ok(()));
                let mut empty = omitted.clone();
                empty.brain.held.clear();
                assert_eq!(empty.transition(&Event::BOp, &empty.clone()), Ok(()));
            }
        }
    }

    /// P6a: at the `Reserve` of the held head, the cells at that event, K at the brain's time and D decide; a section that skips a reservable
    /// head is the violation, and a head the bounds refuse owes nothing.
    #[test]
    fn inv_p6a_a_reservation_section_reserves_a_reservable_head() {
        let section = Op::Section(Section::Reserve { id: 9 });
        let mut before = k_world(1, 64);
        before.brain.held.push(held(0, value(0, 12, 9), 1));
        before.brain.ops = VecDeque::from([section.clone()]);
        let skipped = before.clone();
        violates(before.transition(&Event::BOp, &skipped), "P6a:");
        let reserved = {
            let mut after = before.clone();
            after.brain.held.clear();
            after.brain.ops.clear();
            push(&mut after.cells[0], value(0, 12, 9), ValueState::Queued);
            after
        };
        assert_eq!(p6a(&before, &Event::BOp, &reserved), Ok(()));
        // K full at the brain's time: the head is not reservable, and skipping it is right.
        let mut full = before.clone();
        known(&mut full, 1, 99, None);
        push(&mut full.cells[0], value(0, 99, 1), ValueState::Taken);
        assert_eq!(p6a(&full, &Event::BOp, &full.clone()), Ok(()));
        // The oracle counts K at the brain's time, not the true time: A's instant has come in true time but not for the brain.
        let mut stale = before.clone();
        push(&mut stale.cells[0], value(0, 10, 1), ValueState::Done(Terminal::Call(Outcome::Ok)));
        known(&mut stale, 1, 10, Some(Terminal::Call(Outcome::Ok)));
        stale.device.t = 12;
        stale.brain.now = 9;
        assert_eq!(p6a(&stale, &Event::BOp, &stale.clone()), Ok(()));
        stale.brain.now = 10;
        violates(p6a(&stale, &Event::BOp, &stale.clone()), "P6a:");
    }

    /// A world at the start of cleanup: one open handle (stream 0, handle 1) and a stopper that has joined its one executor.
    fn teardown_world() -> World {
        use super::super::world::Teardown;
        let mut w = world(vec![streaming()]);
        w.rx[0].handle = Some(1);
        w.rx[0].exited = true;
        w.settings.exited = true;
        w.brain.exit = super::super::core::Exit::Terminated;
        w.teardown = Some(Teardown { join_limit: 4, began: true, next: 3, since: 0, joined: [0, 1, 2].into(), ..Teardown::default() });
        w
    }

    #[test]
    fn inv_i18_record_or_mark_before_f_is_violation() {
        for step in [PubStep::Record, PubStep::Mark] {
            let before = world(vec![streaming()]);
            let event = Event::Pub(super::super::world::Holder::Settings, step);
            violates(before.transition(&event, &before.clone()), "I18:");
            let mut set = before.clone();
            set.failure = true;
            assert_eq!(set.transition(&event, &set.clone()), Ok(()), "{step:?} after F");
        }
    }

    /// The record and the mark come after the publisher released the gate.
    #[test]
    fn inv_i18_record_or_mark_while_the_gate_is_held_is_violation() {
        use super::super::world::Holder;
        for step in [PubStep::Record, PubStep::Mark] {
            let mut before = world(vec![streaming()]);
            before.failure = true;
            before.gate = Some(Holder::Settings);
            let event = Event::Pub(Holder::Settings, step);
            violates(before.transition(&event, &before.clone()), "I18:");
            before.gate = None;
            assert_eq!(before.transition(&event, &before.clone()), Ok(()), "{step:?} with the gate free");
        }
    }

    /// A loss publication taken event by event is in the order gate, F, ungate, record, mark, and passes every oracle.
    #[test]
    fn inv_i18_the_publication_order_is_f_then_record_then_mark() {
        use super::super::world::Holder;
        let mut w = world(vec![streaming()]);
        w.settings.program = super::super::world::PUB_STEPS.iter().map(|step| SStep::Pub(*step)).collect();
        for step in super::super::world::PUB_STEPS {
            let event = Event::Pub(Holder::Settings, step);
            assert!(w.enabled().contains(&event), "{step:?} is enabled in order");
            let before = w.clone();
            w.apply(&event);
            before.transition(&event, &w).unwrap_or_else(|why| panic!("{step:?}: {why}"));
        }
        assert!(w.failure && w.monitor.marked);
    }

    #[test]
    fn inv_i19_cleanup_decides_to_close_only_without_f_and_detach() {
        use super::super::world::Cleanup;
        for (failure, detached) in [(true, false), (false, true), (true, true)] {
            let mut before = teardown_world();
            before.failure = failure;
            if detached {
                before.teardown.as_mut().unwrap().detached.insert(0);
            }
            let mut after = before.clone();
            after.teardown.as_mut().unwrap().decision = Some(Cleanup::Close);
            violates(before.transition(&Event::CleanupDecide, &after), "I19:");
            after.teardown.as_mut().unwrap().decision = Some(Cleanup::Retain);
            assert_eq!(before.transition(&Event::CleanupDecide, &after), Ok(()), "retain is right with F {failure}, detached {detached}");
        }
        let before = teardown_world();
        let mut after = before.clone();
        after.teardown.as_mut().unwrap().decision = Some(Cleanup::Close);
        assert_eq!(before.transition(&Event::CleanupDecide, &after), Ok(()));
    }

    /// The destructor reads the native mark: with F set and the mark set the handle is retained (legal); with F set and no mark (the guard that
    /// forgot `mark_lost`) it is freed, which is the violation; a close after a loss that came after the check is legal.
    #[test]
    fn inv_i19_a_destroy_while_f_is_set_is_a_violation_and_the_mark_retains() {
        use super::super::world::Cleanup;
        let mut before = teardown_world();
        before.teardown.as_mut().unwrap().decision = Some(Cleanup::Close);
        before.teardown.as_mut().unwrap().pending.insert((0, 1));
        before.failure = true;
        let mut marked = before.clone();
        marked.monitor.marked = true;
        let mut retained = marked.clone();
        retained.apply(&Event::Destroy(0));
        assert_eq!((retained.teardown.as_ref().unwrap().destroyed.len(), retained.teardown.as_ref().unwrap().retained.len()), (0, 1), "the mark retains it");
        assert_eq!(marked.transition(&Event::Destroy(0), &retained), Ok(()));
        let mut freed = before.clone();
        freed.apply(&Event::Destroy(0));
        assert_eq!(freed.teardown.as_ref().unwrap().destroyed.len(), 1, "without the mark the destructor frees it");
        violates(before.transition(&Event::Destroy(0), &freed), "I19:");
        // A close that comes after the loss (the check came before it) drops the reference only.
        let mut closing = teardown_world();
        closing.teardown.as_mut().unwrap().decision = Some(Cleanup::Close);
        closing.failure = true;
        closing.monitor.marked = true;
        let mut after = closing.clone();
        after.apply(&Event::CleanupClose(0));
        assert_eq!(closing.transition(&Event::CleanupClose(0), &after), Ok(()));
    }

    /// The native device's final release reads the native mark: with `F` set and the mark set the open handle is kept (legal); with `F` set and no
    /// mark it is freed, which is the violation; a handle a detached executor holds is not freed by the release.
    #[test]
    fn inv_i19_the_owner_release_frees_a_handle_only_without_the_mark() {
        use super::super::world::Cleanup;
        let mut before = teardown_world();
        before.teardown.as_mut().unwrap().decision = Some(Cleanup::Retain);
        before.failure = true;
        let mut marked = before.clone();
        marked.monitor.marked = true;
        let mut kept = marked.clone();
        kept.apply(&Event::OwnerRelease);
        assert_eq!((kept.teardown.as_ref().unwrap().destroyed.len(), kept.teardown.as_ref().unwrap().retained.len()), (0, 1));
        assert_eq!(marked.transition(&Event::OwnerRelease, &kept), Ok(()));
        let mut freed = before.clone();
        freed.apply(&Event::OwnerRelease);
        assert_eq!(freed.teardown.as_ref().unwrap().destroyed.len(), 1, "without the mark the release frees it");
        violates(before.transition(&Event::OwnerRelease, &freed), "I19:");
        let mut held = before.clone();
        held.teardown.as_mut().unwrap().detached.insert(1);
        held.rx[0].exited = false;
        assert!(!held.enabled().contains(&Event::OwnerRelease), "a detached thread still holds the device");
        held.rx[0].exited = true;
        held.rx[0].program.clear();
        assert!(held.enabled().contains(&Event::OwnerRelease), "released once it has ended");
    }

    /// The owner release waits for a detached thread (it holds the device), and nothing is sent to a control thread that was never spawned.
    #[test]
    fn inv_i19_i21_the_owner_release_spares_a_detached_holder_and_the_unspawned_control_gets_nothing() {
        use super::super::world::Cleanup;
        let mut before = teardown_world();
        before.teardown.as_mut().unwrap().decision = Some(Cleanup::Retain);
        before.teardown.as_mut().unwrap().detached.insert(1);
        before.rx[0].exited = false;
        let after = before.clone();
        violates(i19(&before, &Event::OwnerRelease, &after), "I19:");
        before.rx[0].exited = true;
        before.settings.unspawned = true;
        let mut told = before.clone();
        told.settings.terminate = true;
        violates(i21(&before, &Event::BOp, &told), "I21:");
        let mut sent = before.clone();
        sent.settings.fifo.push_back(super::super::core::SetCmd { id: 1, change: Change::Restore { channel: 0 } });
        violates(i21(&before, &Event::BOp, &sent), "I21:");
    }

    #[test]
    fn inv_i19_progress_every_closed_handle_is_destroyed_at_the_end() {
        use super::super::world::Cleanup;
        let mut w = teardown_world();
        w.monitor.shutdown = true;
        w.teardown.as_mut().unwrap().decision = Some(Cleanup::Close);
        violates(i19_progress(&w), "I19:");
        w.rx[0].handle = None;
        w.teardown.as_mut().unwrap().closed.insert(0);
        violates(i19_progress(&w), "I19:");
        w.teardown.as_mut().unwrap().destroyed.insert((0, 1));
        assert_eq!(i19_progress(&w), Ok(()));
        // A destructor that retained the handle (the mark was set) has run too.
        w.teardown.as_mut().unwrap().destroyed.clear();
        w.teardown.as_mut().unwrap().retained.insert((0, 1));
        assert_eq!(i19_progress(&w), Ok(()));
    }

    #[test]
    fn inv_i20_detach_join_and_cleanup_follow_their_conditions() {
        use super::super::world::{Cleanup, Teardown};
        // A detach before the join limit.
        let mut before = teardown_world();
        before.rx[0].exited = false;
        before.teardown = Some(Teardown { join_limit: 4, began: true, next: 1, since: 10, ..Teardown::default() });
        before.device.t = 13;
        let mut after = before.clone();
        after.teardown.as_mut().unwrap().detached.insert(1);
        after.teardown.as_mut().unwrap().next = 2;
        violates(before.transition(&Event::Detach, &after), "I20:");
        before.device.t = 14;
        assert_eq!(before.transition(&Event::Detach, &after), Ok(()), "at the limit");
        // A join of a joinee that has not ended.
        violates(before.transition(&Event::Join, &after), "I20:");
        // Cleanup's decision while a joinee has neither ended nor been detached.
        let mut open = before.clone();
        open.teardown.as_mut().unwrap().next = 3;
        let mut decided = open.clone();
        decided.teardown.as_mut().unwrap().decision = Some(Cleanup::Retain);
        violates(open.transition(&Event::CleanupDecide, &decided), "I20:");
        open.teardown.as_mut().unwrap().joined.extend([0, 2]);
        open.teardown.as_mut().unwrap().detached.insert(1);
        assert_eq!(open.transition(&Event::CleanupDecide, &decided), Ok(()));
        // A close or a destroy with an executor detached.
        let mut closing = teardown_world();
        closing.teardown.as_mut().unwrap().detached.insert(1);
        closing.teardown.as_mut().unwrap().decision = Some(Cleanup::Close);
        let mut closed = closing.clone();
        closed.teardown.as_mut().unwrap().closed.insert(0);
        violates(i20(&closing, &Event::CleanupClose(0), &closed), "I20:");
        // The brain thread returning with an input queued, and an input sent after it returned.
        let mut returning = teardown_world();
        returning.inbox.push_back(Input::Shutdown);
        let mut returned = returning.clone();
        returned.brain_exited = true;
        violates(i20(&returning, &Event::BExit, &returned), "I20:");
        let mut late = teardown_world();
        late.brain_exited = true;
        let mut sent = late.clone();
        sent.inbox.push_back(Input::Shutdown);
        violates(i20(&late, &Event::RSend(0), &sent), "I20:");
    }

    #[test]
    fn inv_i21_a_command_to_an_unspawned_stream_and_a_wrong_abandonment_are_violations() {
        use super::super::core::Section;
        let mut before = world(vec![streaming(), cell(Phase::Streaming(seg(0)), 1, None, 0)]);
        before.rx[1].unspawned = true;
        let mut after = before.clone();
        after.rx[1].fifo.push_back(RxOp::Terminate);
        violates(before.transition(&Event::BOp, &after), "I21:");
        // The spawn-failure step: stream 1 started and failing becomes Uncertain; nothing else moves.
        before.brain.ops = VecDeque::from([Op::Section(Section::SpawnFailed { stream: 1, started: true })]);
        let mut right = before.clone();
        right.cells[1].phase = Phase::Uncertain;
        assert_eq!(before.transition(&Event::BOp, &right), Ok(()));
        let mut other = right.clone();
        other.cells[0].phase = Phase::Uncertain;
        violates(before.transition(&Event::BOp, &other), "I21:");
        let mut unstarted = before.clone();
        unstarted.brain.ops = VecDeque::from([Op::Section(Section::SpawnFailed { stream: 1, started: false })]);
        violates(unstarted.transition(&Event::BOp, &right), "I21:");
        // A loss already known stays.
        let mut lost = before.clone();
        lost.cells[1].phase = Phase::Lost;
        let mut overwritten = lost.clone();
        overwritten.cells[1].phase = Phase::Uncertain;
        violates(lost.transition(&Event::BOp, &overwritten), "I21:");
        assert_eq!(lost.transition(&Event::BOp, &lost.clone()), Ok(()));
    }

    #[test]
    fn inv_i17_stopping_reopening_without_a_completed_start_is_violation() {
        let mut before = world(vec![cell(Phase::Stopping(seg(0)), 0, Some(16), 0)]);
        let mut after = before.clone();
        after.cells[0].phase = Phase::Armed(seg(0));
        violates(before.transition(&Event::BOp, &after), "I17:");
        // The legal path: the completed start of the cell's epoch the brain issued.
        before.rx[0].program = VecDeque::from([RStep::Result(RxCall::Begin { at: 0, epoch: 1 })]);
        before.monitor.starts.insert((0, 1));
        assert_eq!(before.transition(&Event::RResult(0), &after), Ok(()));
        // A start of another epoch, or one the brain never issued, does not reopen it.
        before.rx[0].program = VecDeque::from([RStep::Result(RxCall::Begin { at: 0, epoch: 0 })]);
        before.monitor.starts.insert((0, 0));
        violates(before.transition(&Event::RResult(0), &after), "I17:");
        before.rx[0].program = VecDeque::from([RStep::Result(RxCall::Begin { at: 0, epoch: 1 })]);
        before.monitor.starts.remove(&(0, 1));
        violates(before.transition(&Event::RResult(0), &after), "I17:");
    }

    #[test]
    fn inv_capacity_timer_that_stops_or_changes_a_cell_is_violation() {
        let mut before = world(vec![streaming()]);
        before.brain.ops = VecDeque::from([Op::Section(Section::CapacityTimer)]);
        let mut after = before.clone();
        assert_eq!(before.transition(&Event::BOp, &after), Ok(()));
        after.cells[0].drain = Drain::Ended;
        violates(before.transition(&Event::BOp, &after), "B8:");
        let mut after = before.clone();
        after.rx[0].fifo.push_back(RxOp::Terminate);
        violates(before.transition(&Event::BOp, &after), "B8:");
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
        let params = Params { horizon, epilogue, recv_timeout: 1, drain_window: 1, packet: 1, ..params() };
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
    /// window of one tick. The search is complete: 304,486 states, 701,574 transitions, about 29 s in debug (E4a1 fix rounds: acceptance at the call's return, P6b's step record in the key; 215,949 states before, about 5 s in release on sim02; 215,949 states, 516,848 transitions, about 5 s in release on sim02 before;
    /// 165,497 before a start below `now` could be missed, E1 fix round 2: the StartNext at 1 is delivered at 1, so a lagging
    /// actor misses it; 64,763 before the key kept the witness state a later check reads and time went on silently after
    /// the end). Every oracle passes, and all six declared witnesses are reached.
    #[test]
    fn oracle_happy_path_passes() {
        let config = Config { name: "oracle happy path", budget: 400_000, wall: Duration::MAX, witnesses: HAPPY_WITNESSES };
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
