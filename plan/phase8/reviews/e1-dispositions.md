# E1 (2026-11-10): bookings and Stops in one receipt-ordered list

Third stage of direction D′ (design note [49](../49-time-and-booking-inventory.md) §5.4.4 item 6, §12, §12.1). **Risky**: a queue and ownership change.

People and order of work:
- The code shape was sketched by a Claude Opus subagent at effort max (owner, 2026-11-10). It went into the brief, and no new step-0 design was done, because note 49 was that design.
- The implementer was a Claude Sonnet subagent (owner; self-review waived).
- Reviews:
  - Sol on the test plan and the sketch (FAIL);
  - an Opus re-check of the amended plan (PASS WITH MINORS);
  - the orchestrator on each sub-step;
  - the full mutation run before review;
  - Sol on the code (PASS WITH MINORS).
- Sub-steps: E1a (`TxList` and the UHD Module) and E1b (MockRadio), on a local work branch.

Records: brief [e1-brief.md](e1-brief.md), sketch [e1-sketch.md](e1-sketch.md), test plan [e1-test-plan.md](e1-test-plan.md), gate `e1-gate.json` (added after the gate).

## What changed
- `ezsdr_radio::staging::TxList<P>` sits beside `Bookings`, in each transmit stream's state and under the same lock.
  - Its unread FIFO holds receipts in order: bookings and halts (Stop, `Provider::stop`'s Run end).
  - Its start-keyed `held` map holds what the executor has read and not yet taken.
  - A halt cancels exactly what was read before it.
- UHD Module:
  - `TxCmd::Burst`, `TxCmd::Stop`, `Control.to_tx`, uhd-tx's own `held`, `Tx::command`, `end_clock`'s cancel loop and the clock-domain-mismatch arm are gone. Only `TxCmd::Shutdown` stays on the mpsc.
  - uhd-tx reads the list in its run loop (`read_list`, the old mpsc drain).
  - `step` reads `now` in the critical section that finds no unread halt. When a halt is unread, it reads the list first.
  - `book_burst` decides in one critical section, from reading the instant to booking on the list, with the cut checked again there. It emits refusals after the guard is released.
- MockRadio: `TxStream.list` replaces `held`. Receipt books on the list and reads it at once. `stop_tx` pushes the halt before the MR-25 cut and reads the list at the end.
- Behaviour, as RM-16 already states it:
  - A pre-origin `Provider::stop` cancels with the `ProviderStop` note (no `COMMAND_REJECTED`), whatever the order of the Run end and the thread's shutdown.
  - `Provider::stop` closes the open burst at uhd-tx's next turn, not after the uhd-control join.
  - A turn never hands past a Stop it has not read (note 49 §4's second window).
  - A booking that races a `Provider::stop` cut is refused at receipt.
- Unchanged:
  - When a booking is read: the step guard is on halts only. Guarding on any unread booking would install switches earlier and remove UR-38's moved path, which is E2a's change.
  - `cancel_unstarted` (ClockEnded) after a failed reopen.
  - #84, #85 and #86. #85 is not fixed by E1: the dispatched, unsent burst lives in `open`, outside the list. Its pin now says E2a.
- Tests:
  - Added: `e1_*` (UHD, `tx/tests/e1.rs`), `e1_mock_q10_…`, 8 pure `txlist_*` and `ur_21_m1_joins_booked_early_continue_the_device_burst`.
  - Deleted, with the argument that their state is unreachable or covered: `ur_23_a_clock_end_cancels_every_held_burst`, `ur_25_a_burst_booked_on_an_ended_clock_is_cancelled_when_received`, `ur_21_s2_a_short_head_is_not_held_for_a_piece_handed_after_it`, `ur_21_m1_held_tail_waits_for_a_join_received_before_its_txcmd_is_drained` (renamed in the Test lines).
  - Rewritten: `ur_25_an_end_booked_after_the_first_read_…` (ProviderStop; `design/18-uhd-radio.md` UR-25 row), `ur_21_a_late_joined_burst_cannot_move_and_is_refused` (finite abandon) and two split-receipt tests.
- Mutation rows:
  - Added: E1-U*, E1-K1–K3 and P569b.
  - Retargeted: P453, P569, P570, P1117, P1199, P1206, P1302, N47S4-41, N47S4-44, N47S4-50, N47S5-23, N47S5-151, P635, M76 and U18.
  - New killer: R07 and N47S5-57.
  - Removed: N47S4-33 (below).

## Findings
| Source | Findings | Disposition |
|---|---|---|
| Opus sketch | §12.1's claim that `cancel_unstarted` becomes unreachable is false: a failed reopen still reaches it; a third ClockEnded test and a spec row; `Provider::stop` on the list ends transmit earlier (RM-16 already requires it); a receipt race with the cut; #85 not fixed by E1 | all into the brief |
| Sol, test plan + sketch | FAIL: 1 major. The sketch removed `continuation_open`'s joined clause (N47S5-57), but a join accepted after the step's unread check and before `close_unjoined` still needs it. 6 minors: Q10 not one step, Q5 killer order, D24 rebuild promoted B to head, Q7 refusal arm missing, mutation mappings, origin-end two-pass | all adopted; clause and row kept, killer Q14 through a probe |
| Opus re-check, plan | PASS WITH MINORS: Q5 had to abandon through the report path, not FailSend inside `step`; D24 time move; leftovers | all adopted. The row reading `now` before the lock was dropped (no deterministic killer; an observation limit in the plan) |
| Implementer, E1a | the sketch's guard on any unread entry turned three switch tests red (`o1_uhd_t4b_…`, `ur_21_s12_…`, `ur_23_stop_before_the_moved_boundary_…`): it installs switches earlier | orchestrator: the guard is on an unread halt only; the run loop reads bookings as the mpsc drain did |
| Full mutation run | N47S4-33 survived: with the halt guard, a configure turn that continues into `step` is equivalent | row removed; the guard's rows and Q2b cover M2 |
| Sol, code | PASS WITH MINORS: `book_burst` emitted early refusals under the streams lock, which can stall uhd-tx behind the event collector (INFERRED) | fixed: decide in the section, emit after. N47S4-44 re-anchored, now also covering the PlanViolation drop. No lock test (the rig's sink is fixed) |
| Orchestrator | `held_first()` and similar take the streams lock several times per step | note: no consistency risk (one writer); E2a moves `held` into the state machine |

Mechanism: none counted. Sol's plan major was a wrong claim in the sketch, caught before code.
