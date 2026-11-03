**Revise before running.** Checked HEAD `cb7c2a3`. Fidelity is separate from detection; unit-only kills, lost witnesses, and observer panics do not establish DST detection.

Paths below are relative to `crates/ezsdr-radio-uhd/src/brain/`.

| ID | Verdict | Reason / replacement |
|---|---|---|
| N01 | **not faithful** | Behaviourally exact, but also weakens P6/P6a through their call to `Cell::reserve`; inadmissible under the E3 shared-oracle ruling. Use R01, leaving the oracle’s reservation predicate intact. |
| N02 | **faithful** | Natural translation of the captured obligation set. Its callers discard that set and retain MaybeApplied through other predicates: unit-only is plausible. Removing those other protections would introduce a different error. |
| N03 | **faithful** | Exactly changes cancellation’s upper-bound equality. |
| N04 | **faithful** | “Epoch only” requires dropping revision too; dropping cut alone is redundant. DST computes and writes atomically in `publish_round`, using the live cut, so this cannot reproduce the proposed compute–commit race there. No simple relocation fixes that missing interleaving. |
| N05 | **no faithful translation exists** | Record-before-Mark equals the model’s baseline and is permitted by the note; externally observable event-before-Mark is absent. My holdout conflated these. See clarification below. |
| N06 | **faithful** | Open/Close substitute for the absent rate reader but exercise the same gate-admission error. |
| N07 | **no faithful translation exists** | Parent native-device destruction is absent. `OwnerRelease` models streamer disposition, not a separately retained/freed parent. N07d invents a sentinel violation: diagnostic only, never scored. |
| N08 | **not faithful** | Move additionally disregards **Queued** commitments; both edits recognize only successful/partial finishes, not any Finished command. R08 preserves queued commitments and implements the named Move/StartNext counterexample. It is not a global pin-policy mutation. |
| N09 | **faithful** | Narrow representative: late failed/dropped/abandoned completions duplicate settlement. Successful completion is unchanged, so the holdout’s specific success sequence is **not** reproduced; do not claim otherwise. |
| N10 | **not faithful** | Renewing at deadline delivery is not renewing after empty receives. Empty receives exist: use R10. |
| N11 | **no faithful translation exists** | Command-time state and its inheritance by Stop are absent. Lock/Effect/Unlock are separate events, but Effect abstracts the entire command-time bracket into one effect. Unlocking alone models another error. |
| N12 | **faithful** | K becomes per-chain; the independent monitor-based capacity oracle remains intact. |
| N13 | **faithful** | Partial immediately stops counting while its device effect remains. |
| N14 | **faithful** | Elapsed time releases unfinished reservations. |
| N15 | **faithful** | Reported residuals remain resident but cease counting toward D. |
| N16 | **faithful** | Cancellation removes identity before Dropped. The shared cancellation helper naturally broadens the affected cancellation paths. |
| N17 | **faithful** | Capacity wakeup executes the existing drain-deadline transition. Earlier termination requires an already-Draining stream; that is a valid representative. |
| N18 | **not faithful** | Credits evidence before return, with no acceptance threshold, rather than retaining it and crediting retrospectively after return. Use R18. |
| N19 | **no faithful translation exists** | Stopper notifications/unsuccessful join polls are absent. Brain input consumption is neither; finite inputs merely delay timeout. Adding a notification/poll operation would extend the model, not translate this error within its existing events. |
| N20 | **faithful** | Shutdown now waits on an impossible condition involving unspawned executors. This expresses the missing-executor wait without inventing acknowledgements. |

**N05 clarification:** I overread §4.5. “記録より先に印を立て，その後 … `mark_lost`” clearly requires flag-before-record and flag-before-mark, but ambiguously leaves record-versus-mark ordering implicit; it does **not** establish my stronger mark-before-any-record requirement. `world.rs:70,624` explicitly places an inert Record before Mark. Production `provider/core.rs:1304–1311` instead does internal stream accounting → flag → mark → timing/event. Thus production and model are **not identical overall**, but neither publishes `DEVICE_LOST` before marking. The holdout’s teardown consequence does not follow merely from internal recording.

**Replacement rows:** each `old` below was mechanically verified to match once with `anchor.find`; all edits belonging to a replacement run together. No replacement changes an oracle/assertion.

**R01 — `core.rs`:**
- old: `if cells[held.stream].reserve(held.entry.clone(), held.generation).is_err() {`
- new: `if matches!(cells[held.stream].phase, Phase::Armed(s) | Phase::Streaming(s) if held.entry.order.0 == s.origin) || cells[held.stream].reserve(held.entry.clone(), held.generation).is_err() {`

**R08 — `core.rs`, two edits:**
- old: `if cells.iter().any(|cell| cell.committed(Some(channel)) || (cell.channels.contains(channel as u32) && cell.busy())) {`
- new: `if cells.iter().any(|cell| cell.in_flight.iter().any(|f| f.entry.channel == channel && (f.state == ValueState::Queued || (matches!(f.state, ValueState::Taken | ValueState::MaybeApplied) && !cell.in_flight.iter().any(|g| matches!(g.state, ValueState::Done(_)))))) || (cell.channels.contains(channel as u32) && cell.busy())) {`
- old: `if cell.pinned() || cell.flush_owed() || cell.busy()`
- new: `if (cell.pinned() && !cell.in_flight.iter().any(|f| matches!(f.state, ValueState::Done(_)))) || cell.flush_owed() || cell.busy()`

**R10 — `world.rs`:**
- old: `let ended = rules::end_of_recv(received, k, self.device.t);`
- new: `let ended = rules::end_of_recv(received, k, self.device.t); if !ended.lost && ended.samples.count == 0 && self.cells[k].drain == Drain::Draining { self.timers.retain(|(timer, _)| *timer != Timer::Drain(k)); self.timers.push((Timer::Drain(k), self.device.t + self.params.drain_window)); }`

**R18 — `world.rs`, seven edits; fault-owned buffering is included in the state key:**
- old: `pub(crate) steps: usize,` → new: `pub(crate) steps: usize, pub(crate) pre_return: BTreeMap<u64, (i64, Vec<(usize, Channels, Ticks)>)>,`
- old: `steps: 0,` → new: `steps: 0, pre_return: BTreeMap::new(),`
- old: `omit_mark, steps, } = self;` → new: `omit_mark, steps, pre_return, } = self;`
- old: `format!("{steps:?}"),` → new: `format!("{steps:?}"), format!("{pre_return:?}"),`
- old: `self.device.apply_timed(*channel, cmd.id, *at);` → new: `self.device.apply_timed(*channel, cmd.id, *at); self.pre_return.insert(cmd.id, (self.device.t, Vec::new()));`
- old: `let carried = self.carried(k); match rules::publish_round` → new: `let carried = self.carried(k); if let Some(id) = self.settings.hand.as_ref().map(|cmd| cmd.id) { if self.cells[k].state_of(id) == Some(&ValueState::MaybeApplied) { if let Some((_, packets)) = self.pre_return.get_mut(&id) { packets.push((k, carried, call.recv.samples)); } } } match rules::publish_round`
- old: `rules::accept_all(&mut self.cells, cmd.id, self.device.t);` → new: `let id = cmd.id; rules::accept_all(&mut self.cells, id, self.device.t); if let Some((accepted, packets)) = self.pre_return.remove(&id) { for (k, carried, samples) in packets { let mut view = self.cells[k].clone(); view.accept(id, accepted); for (value, sample) in rules::consume_samples(&view, carried, &self.rx[k].consumed, &samples).into_iter().filter(|(value, _)| *value == id) { let left = self.cells[k].is_left(value); let ident = self.ident(value, view.in_flight.iter().find(|f| f.entry.order.1 == value).unwrap().entry.channel, sample); self.fact_log.push(Fact { leftover: left, value, stream: k, channel: ident.channel, sample }); if left { self.monitor.leftovers.entry(value).or_insert(ident); } else { self.monitor.evidence.entry(value).or_insert(ident); } self.rx[k].consumed.insert(value); self.inbox.push_back(Input::Boundary(Boundary::Consumed { id: value })); } } }`

**Oracle-only/assertion changes:** none in the supplied rows. N01 nevertheless changes a shared oracle dependency; N07d fabricates an observed failure rather than modelling parent destruction.

Not checked: replacement compilation or execution, mutation/DST results, or detection rates. No files, formatting, or git state changed.
