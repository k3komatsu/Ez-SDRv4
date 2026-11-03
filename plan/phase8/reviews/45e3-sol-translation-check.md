# Sol 6.1 — E3 translation-check, report section

H13 has an additional problem: its mutated predicates are also called by the oracles, so the row weakens both the behavior and its checks. H08’s stale-snapshot retry also neutralizes the proposed mutation; a faithful row must preserve the recv-start cut across retries.
**Revise before running.** Faithfulness is not detection: a faithful mutant can survive because another guard prevents its effect.
Paths below are relative to `crates/ezsdr-radio-uhd/src/brain/`. Replacement rows follow the table.

| Error | Verdict | Replacement | Note |
|---|---|---|---|
| H01 | faithful | — | Splitting validation/insertion expresses the race. The observer adjustment must remain bookkeeping only; see panic ruling below. |
| H02 | faithful | — | Exactly changes the upper-cut equality. |
| H03 | faithful | — | Exactly permits reservation at exhausted progress. |
| H04 | faithful | — | Delayed take overwrites cancellation without rechecking. |
| H05 | faithful | — | Delayed FirstSample reopens Stopping. |
| H06 | faithful | — | Correct stale-progress-floor race; missing W1c alone is not detection. |
| H07 | unfaithful | R07 | Clearing retirement inside epoch advancement is not an executor’s premature acknowledgement. Quiescence is modelled; a faithful translation exists. |
| H08 | unfaithful | R08 | Proposed stale computation is rejected, then recomputed correctly. The erroneous clipping must survive snapshot refresh. Concept is modelled. |
| H09 | faithful | — | Publication becomes visible before its progress update. Early cut-reached bookkeeping does not eliminate that window. |
| H10 | none | — | **not found: not modelled**: no loss-recording operation exists. Mark is a native-loss marker, not recording; its reordering is a different error. Full design needs flag-before-recording (§4.5). |
| H11 | faithful | — | An already-receiving executor ignores failure at subsequent loop tops. |
| H12 | none | — | **not found: not modelled**: the marker exists, but native destruction/retention does not. Changing only `monitor.marked` cannot express the handle-release error. Full design needs it (§4.5). |
| H13 | unfaithful | R13 | Behavioural intent matches, but the row also weakens I3 through shared predicates (`oracle.rs:206,224–226`). Keep those predicates unchanged. |
| H14 | faithful | — | Executor retirement makes brain settlement skip the value. |
| H15 | faithful | — | Device effect remains while Partial becomes no-effect Refused. |
| H16 | faithful | — | Deadline deletes non-revocable work and its residual tracking. |
| H17 | none | — | **not found: not modelled**: no join timeout or timeout-driven handle cleanup/retention decision. Full design explicitly requires it (§4.7, §4.9). |
| H18 | faithful | — | Full-cell backpressure silently consumes the held head. |
| H19 | faithful | — | With consumption’s `max(e, accepted_at)`, this substitutes call-start eligibility for acceptance eligibility. |
| H20 | unfaithful | R20 | Current filter also consumes eligible entries behind an ineligible FIFO head. Preserve head blocking; repeatedly consume at the same sample. Mutating the brain adapter is legitimate here. |

**Replacement rows — literal Rust-token `old` → `new`; all edits in each R belong together.**
**R07:** inject an immediate executor acknowledgement on cancellation observation, leaving the old recv outstanding:
- `world.rs`: `self.brain.op(&mut self.cells);` → `self.brain.op(&mut self.cells); if let Op::Section(Section::PlanCancel { stream, .. }) = op { self.rx[stream].program.push_front(RStep::Send(Input::Boundary(Boundary::Quiesced { stream }))); }`
- `cell.rs`: `Some(retiring) if retiring.epoch == epoch && retiring.published =>` → `Some(retiring) if retiring.epoch == epoch =>`
This is one error: observation authorizes acknowledgement without quiescence; neither edit alone implements it.

**R08:** retain the recv-start cut across retries, but keep validation against the live cell:
- `world.rs`: `pub(crate) snap: Option<Snapshot>,` → `pub(crate) snap: Option<Snapshot>, pub(crate) recv_cut: Option<i64>,`
- `world.rs`: `let snap = self.rx[k].snap.expect("a snapshot");` → `let snap = self.rx[k].snap.expect("a snapshot"); self.rx[k].recv_cut = snap.cut;`
- `world.rs`: `let RxExec { fifo, program, handle, handles, recv, snap, call, consumed, flush_end, exited } = r;` → `let RxExec { fifo, program, handle, handles, recv, snap, recv_cut, call, consumed, flush_end, exited } = r;`
- `world.rs`: `(fifo, program, handle, handles, recv, snap, call, consumed, flush_end, exited)` → `(fifo, program, handle, handles, recv, snap, recv_cut, call, consumed, flush_end, exited)`
- `world.rs`: `match rules::publish_round(&mut self.cells[k], k, snap, &call.recv, carried, &self.rx[k].consumed) {` → `match { let mut view = self.cells[k].clone(); if let Some(retiring) = view.retiring.as_mut().filter(|r| r.epoch == call.recv.epoch) { retiring.cut = self.rx[k].recv_cut; } let computed = rules::publish_compute(&view, &call.recv, carried, &self.rx[k].consumed, true); rules::publish_write(&mut self.cells[k], k, snap, &call.recv, computed) } {`

**R13:** change only decision predicates in `core.rs`; leave cell helpers and their oracle callers untouched:
- `reached && !cells[stream].pinned()` → `reached && !cells[stream].in_flight.iter().any(|f| f.state == ValueState::MaybeApplied)`
- `if cell.pinned() || cell.flush_owed()` → `if cell.in_flight.iter().any(|f| f.state == ValueState::MaybeApplied) || cell.flush_owed()`
- `cell.committed(Some(channel)) || cell.owed_on(channel)` → `cell.in_flight.iter().any(|f| f.entry.channel == channel && matches!(f.state, ValueState::Queued | ValueState::MaybeApplied)) || cell.owed_on(channel)`
- `cell.committed(Some(channel)) || (cell.channels.contains(channel as u32) && cell.busy())` → `cell.in_flight.iter().any(|f| f.entry.channel == channel && matches!(f.state, ValueState::Queued | ValueState::MaybeApplied)) || (cell.channels.contains(channel as u32) && cell.busy())`
- `cell.committed(None) || cell.retiring.is_some()` → `cell.in_flight.iter().any(|f| matches!(f.state, ValueState::Queued | ValueState::MaybeApplied)) || cell.retiring.is_some()`
- `!cell.committed(None) && !cell.busy()` → `!cell.in_flight.iter().any(|f| matches!(f.state, ValueState::Queued | ValueState::MaybeApplied)) && !cell.busy()`

**R20**, `rules.rs`:
- Old: `for (id, at) in consume_packet(&left, &debt, 1, channels, tick, &one_sample) { left.retain(|entry| entry.order.1 != id); debt.retain(|entry| entry.order.1 != id); consumed.push((id, at)); }`
- New: `loop { let batch = consume_packet(&left, &debt, 1, channels, tick, &one_sample); if batch.is_empty() { break; } for (id, at) in batch { left.retain(|entry| entry.order.1 != id); debt.retain(|entry| entry.order.1 != id); consumed.push((id, at)); } }`

**Proposed rulings**
1. **World panic:** not automatically “found.” Count a genuine modelled-program panic, not an observer’s inability to represent the illegal state. H01’s `reserved()` unreachable is observer failure (`rules.rs:593–595`), so record it separately; it does not establish DST detection.
2. **Missing witness only:** do **not** count. Witnesses establish coverage, not correctness. H06 losing W1c because instrumentation follows the former atomic boundary is not evidence of the behavioural error.
3. **H13:** score once using corrected main HO13; HO13a/b are diagnostics, not additional points or substitutes for the main result.

Read-only; nothing executed, edited, formatted or committed. Replacement anchors/builds are statically proposed, not mechanically verified.
