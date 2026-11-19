# #110 + #113 — test plan v2 (v1 `test-plan.md` with Sol's review `sol-plan.md` F1–F8 applied)

Worktree `~/local_work/Ez-SDRv4-n111`, branch `work/n110`, base `d4f840ff`. This file is the whole plan; no paper re-check follows. v1 stays as the evidence record for the code reading (§1 there); where v2 repeats a fact it is VERIFIED in v1 at the `path:line` named there.

## 0. Decisions carried out (owner 2026-11-19; orchestrator's decisions.md)

- #110: an admitted start-batch Action dropped by an end before its dispatch is recorded in `termination.undelivered`; its `ActionId` is assigned at admission (once the whole batch is admitted). The batch is a new pending holder in `Shared`, drained by the one RS-6 step 1 freeze (design note `note.md`).
- #113: a MockRadio receive segment cut at or before its origin ends at its origin, with or without a registered clock.
- Q1 accepted: a batch admitted before a failed `start()` (`Failed { arm }`) is in `undelivered`. Q2 accepted: `start_probe::Point::Admitted`.

## 1. #110 — the change (production code)

1. `state.rs` `Shared`: new field `start_batch: Locked<Vec<ActionId>>` (a leaf lock, `Locked::new`; initialised empty in `pipeline.rs` beside `undelivered`). Helper `Shared::take_action_ids(&self, count: usize) -> Vec<ActionId>`: one `next_action.fetch_add(count as u64, SeqCst)`, so a batch's ids are always contiguous and nothing can be drawn between two of them.
2. `admission.rs`: `dispatch(shared, admitted)` keeps its signature and draws one id through `take_action_ids(1)` (Session, agenda and Module submissions are unchanged); a new `dispatch_with(shared, admitted, id)` does the rest (configuration, push) and `dispatch` calls it. The queue push keeps the text `Dispatched { id, ...` so row J181 matches.
3. The Module `Abort` branch (`pipeline.rs:1218`) is not touched: it still reads `shared.next_action.fetch_add(...)` at its own call site, so J195's anchor and killer stay as they are (F3).
4. `pipeline.rs` `admit_schedule`: unchanged until every entry is admitted; then `let ids = take_action_ids(admitted.len())`, `*start_batch.lock() = ids`, and return the admitted vector. A refused entry returns before any id is taken (F4: atomic — no id is consumed by a refused batch).
5. `pipeline.rs` dispatch loop (admission lock held, after `Running`): `let ids = std::mem::take(&mut *start_batch.lock())`; each admitted Action is dispatched with `dispatch_with(.., id)` (ids zip admitted, in KC-17 order). A new testing-only `start_probe::Point::Admitted`, reached in `pipeline()` right after `admit_schedule` returns `Some` and before its end check, gives S1 an end between admission and `start_instances`.
6. `ending.rs` `freeze_held`: after the existing `.collect()` and before `sort()`, `undelivered.extend(std::mem::take(&mut *self.shared.start_batch.lock()))` (a separate statement; the `.chain(...)` collection is untouched so rows J187, J193, J198 still match). A batch that was dispatched is in the queues and not in `start_batch`; one an end dropped is in `start_batch` and in no queue: every id is recorded at most once, whichever thread runs the freeze (its two callers are `ending.rs:336-337` and `paced.rs:143-145`; both take the admission lock first, and the dispatch loop empties `start_batch` and pushes under that same lock).
7. The dispatch loop has no early exit: the lock is held through the last push and a freeze waits for it (`freeze_held` takes the admission lock), so the freeze sees either the whole batch in `start_batch` or the whole batch in the queues. No code path outside `freeze_held` writes `undelivered` (the rule is stated once).

## 2. Numbering visible to users (F5)

The counter starts at 1 and one origin, the schedule's start batch, draws first on a Run that reaches `Running`: its Actions get `ActionId(1)…(n)`, in KC-17 order (entries ordered by instant, then schedule index), contiguous, taken together once the batch is wholly admitted. Session, agenda and Module submissions are refused until `Running` (KC-24 step 2) and then wait for the lock until the batch's last dispatch, so they draw `n+1…` as before. The one other drawer is a Module `Abort` (`submit` takes no lock and checks no state): it may take an id at any time, so a batch can be `2…n+1` when an Abort in the arm window took 1. That Abort's id is returned to the submitting Module and recorded nowhere (RS-15).

Before: the batch's ids were drawn at dispatch, after `Running`, so a Module `Abort` submitted between arm and dispatch drew first (1) and the batch took `2…n+1`; a dropped batch drew none and was recorded nowhere. After: the batch draws at admission, so such an Abort now draws `n+1` when it comes after admission and `1` when it comes before (the Abort in the arm window); a dropped batch is named in `undelivered` by its ids; a refused batch draws none (a later Abort gets 1). Session, Module-submission and agenda ids after `Running`: same values. Visible beyond the decision: (a) the Abort's id above; (b) `Failed { arm }` from a failed `start()` lists the batch (Q1). No fixture or golden file holds an ActionId (v1 §2.3 sweep, `ds-1-out.md`). The server and the Python client only compare ids for equality (`ezsdr-server/src/lib.rs:457-488`, `python/ezsdr/session.py:441`); their existing correlation tests stay and are re-run (`cargo test --workspace`, the Python suite).

## 3. #110 — sequences and tests

New tests are pure (no wall-clock verdict): each hook runs on the pipeline thread (`start_on_worker`'s thread-local hook), so the end is requested at a fixed place. Tests are written before the production change; those marked FAILS-NOW fail on the current code by an assertion on `undelivered`, the others pass on it and pin the new code (S5, S6, S7, S8, S9). No new test uses a timing-dependent wait except S9's infra-backstopped waits (repeated ×20).

| # | Sequence | Test (file) | Asserts and how observed | Wrong behaviour that fails it |
|---|---|---|---|---|
| S1 | one timed entry (`update_parameter`) and one far-future agenda `Stop` (F6); a Module `Abort` submitted from the hook at `Point::Admitted` (end 1); `WatchedAuthority` as in `kc_24a_freeze_runs_cancels_outside_the_admission_lock` | `kc_24a_start_an_end_after_admission_records_the_batch_as_undelivered` (`coordinator.rs`) FAILS-NOW | `termination.undelivered == [ActionId(1)]`; the Abort's returned id `== ActionId(2)`; no `p:action:`/`p:update:`/`p:start:` line; trace reached `Admitted` and not `BeforePublish`; the far-future handle is cancelled exactly once with `held.freeze` and neither `held.admission` nor `held.machine` | batch never recorded (`[]`); ids drawn at dispatch (Abort draws 1); a guard held across the cancel when the freeze drained a populated `start_batch` |
| S2 | one timed entry; `Provider::start` fails (`FailAt::Start`, end 2) | extend `kc_18_a_start_failure_is_failed_arm` (`coordinator.rs`) FAILS-NOW | add `termination.undelivered == [ActionId(1)]` to its assertions (no `p:action:`, no `Running`, `Failed { arm }`) | recording tied to ends after `Running` |
| S2b | one timed entry; an Executor's `start()` submits `Abort` (`submitting_from_start`; the end is seen inside `start_instances`, `pipeline.rs:865-867`) | `kc_24a_start_an_end_during_start_records_the_batch_as_undelivered` (`coordinator.rs`) FAILS-NOW | `undelivered == [ActionId(1)]`; the Abort's id (`x:start:ok:2` line) is 2; no `Running` transition | a recording tied to check 2's `false` return |
| S3 | one timed entry; `Abort` at `BeforePublish` (end 3) | existing `kc_24a_start_an_end_before_the_check_dispatches_nothing` (`coordinator.rs:5068`), assertions changed FAILS-NOW | `undelivered == [ActionId(1)]` (was `is_empty()`, `:5106`); the Abort's id `== ActionId(2)` (was `is_ok()`, `:5098`); `BeforeDispatch` still absent | batch recorded only for ends 1 and 2 |
| S4 | two timed entries at offsets 2000 then 1000 (schedule order is not instant order); end after admission as in S1 | `kc_24a_start_an_end_after_admission_records_every_entry_in_admission_order` (`coordinator.rs`) FAILS-NOW for the first Run; second Run passes | first Run: `undelivered == [ActionId(1), ActionId(2)]`. Second Run, same Spec, no end: the Provider double records one new line `id:<name>:<n>:<variant>` (outside the `<name>:` lines exact sequences read, as `source:` is; `UpdateParameter` also `:<key>=<value>`) for each Dispatched Action; the entry at offset 1000 (KC-17's first) carries id 1 and the one at 2000 id 2. The existing `p:action:` line is not changed (F7) | ids in schedule-index order, or reversed; ids per entry before the whole batch is admitted |
| S5 | the single-entry Spec, no end before dispatch; the Provider takes the Action; the Run finishes orderly | `kc_24a_start_a_delivered_batch_is_not_undelivered` (`coordinator.rs`) | the Provider logged `id:p:1:update:...` (the Action with id 1); `undelivered.is_empty()` | `start_batch` not emptied at dispatch: batch recorded although delivered |
| S6 | the deaf-Provider Run, end after the dispatch (existing) | existing `kc_24a_start_an_end_in_the_window_records_the_batch_as_undelivered` (`device_paced.rs:1687`), unchanged | `undelivered == [ActionId(1)]` (whole-vector `assert_eq!`) | `start_batch` not emptied: `[1, 1]` |
| S7 | two timed entries; the second is refused at admission (Provider `coerce` rejecting it under the `reject` policy, the fixture of `rs_17_a_scheduled_rate_change_under_reject_is_refused`), so the Run fails at `Arm`; an Executor submits a Module `Abort` from the cleanup (its id is captured) | `kc_17_a_refused_entry_leaves_no_batch_ids` (`coordinator.rs`) | `Failed { arm }` naming entry 1; `undelivered.is_empty()`; no `p:action:`; the cleanup Abort's id `== ActionId(1)` (the refused batch consumed none: F4) | ids taken per entry as admitted (`[ActionId(1)]` recorded or the Abort gets 2) |
| S8 | an empty batch: the existing arm-failure/`Failed { arm }` test with no schedule and one that reaches `Running` and ends orderly | an extra assertion in an existing deterministic test that reads `termination` (`kc_18_a_start_failure_is_failed_arm`'s sibling with an empty schedule, or the nearest test of this shape in `coordinator.rs`); the implementer names it | `undelivered.is_empty()` | a phantom id for an empty batch (an off-by-one in `take_action_ids`) |
| S9 | two timed entries; a deaf `ThreadedProvider` with `losing_device_on(loss)`; the hook at `Dispatching { next: 1 }` (A pushed, B not) opens the loss gate and waits until the data thread's freeze reaches `FreezePoint::Serialised` (the freeze gate is held, the admission lock is not; installed with `Probe::on_freeze` at `Entered` as in `kc_24a_freeze_runs_cancels_outside_the_admission_lock`) while the dispatch still holds admission, then returns (F2) | `kc_24a_start_an_end_between_two_pushes_records_both_ids` (`device_paced.rs`) | `undelivered == [ActionId(1), ActionId(2)]` (A and B, neither received); the hook's readings show `admission_held` and the freeze at `Serialised` before B's push; B's `Dispatching { next: 1 }` was reached; the Provider's `took:` lines are absent. Waits are 10 s infra backstops with `infra(...)` messages | a loop that stops on the end after A's push (`[ActionId(1)]` only, B in neither the queue nor the Manifest); `start_batch` emptied but the loop not completed |

The Provider in S9 is deaf, so "not received by the Provider" is every id; the assertion is that the Manifest's `undelivered` and the Provider's receipt partition the batch (here the receipt is empty). Interleavings not covered: an end by another thread between check 3 and the lock (the freeze waits for the lock, the batch is dispatched whole, S6 covers the dispatched result); a freeze before the batch's dispatch on the data thread (impossible, it starts after check 3); a Module submission across the transition (the four existing `kc_24a_start_a_worker_submission…` tests, ids `x:worker:ok:2`, `x:submit:ok:3`, `x:submit:ok:2`, which stay right).

## 4. #110 — mutation rows (Rust token anchors; each dies by an assertion, never a timeout)

| Row | Mutation | Killed by |
|---|---|---|
| N110-1 | the freeze does not move `start_batch` into `undelivered` (drop the `extend` statement) | S1, S2, S2b, S3, S4 |
| N110-2 | the dispatch loop does not take `start_batch` (leaves it set) | S5 (`undelivered == []` fails with `[1]`), S6 (`[1, 1]`) |
| N110-3 | `admit_schedule` stores the ids inside the entry loop (per entry) | S7 (`[ActionId(1)]` recorded or the Abort's id 2) |
| N110-4 | `dispatch_with` ignores its `id` and draws a fresh one | S4's second Run (ids 3, 4), S5 |
| N110-5 | `admit_schedule` stores the ids reversed | S4's second Run |
| N110-6 | `admit_schedule` stores no ids (`start_batch` stays empty; the old behaviour) | S1, S3 |
| N110-7 | the dispatch loop returns when an end is requested after a push | S9 |
| N110-8 | `take_action_ids` takes one id fewer than asked (off by one on the range) | S8, S4 |
| J195 | unchanged (Abort branch untouched) | `kc_24_a_module_abort_ends_the_run`, inequality assertion `coordinator.rs:1220` |

Rows J181, J187, J193, J198 stay matchable (anchors kept); `rules.py` and `anchor.py` confirm. Every new row's `old` matches exactly once.

## 5. #110 — `design/` and schema text (no issue numbers; DA-1: normative, one topic, `*Test:*`)

- `design/06-kernel-coordinator.md` KC-17: after the working-copy sentence — the batch's `ActionId`s are taken together, in that order, once every entry is admitted (an entry refused fails `arm` and takes none) and held with the Run until dispatched; the batch is admitted as a whole. KC-18: an end before the dispatch (a failed `start`, an end requested before `Running` or before the lock) leaves the batch undelivered (RS-16); `*Test:*` gains S2, S2b, S3. KC-24: replace the id-at-dispatch sentence by: an id comes from the Run's counter, which starts at 1, once every Action of its call or of the start's batch has been admitted, so an entry some of whose Actions are refused consumes no id; the id travels with the Action into its queue; the batch's ids are contiguous. KC-24a: RS-6 step 1's description gains "and moves the start batch's ids not yet dispatched"; the batch sentence gains that the dispatch empties that record under the lock; its test list gains S1, S2b, S4, S5, S9.
- `design/04-run-and-session.md` RS-6 step 1 (`:134`): gains "and the start's batch not yet dispatched". RS-16: "one that RS-6 step 1 discards before its Module received it" gains "(from a queue, or from the start's batch before its dispatch)"; `*Test:*` gains S1, S2, S3, S5, S9. RS-15 and the Module-`Abort` sentence stay.
- `design/05-module-api.md:118` (`Dispatched`): "the id KC-24 gave the Action at dispatch" → "the id KC-24 gave the Action".
- `crates/ezsdr-kernel/src/event.rs:672` `ActionId`'s description no longer says "dispatched": "identifies one Action the Run admitted" (the id is also named in `undelivered`); `manifest.rs:226-229` `TerminationSection.undelivered`: "The admitted Actions RS-6 step 1 discarded before their Module took them: a queued one, or one of the start's batch the end came before dispatching". Regenerate with `EZSDR_UPDATE_SCHEMAS=1` (`schema_freeze.rs`): `schemas/manifest.v1.json` (`ActionId` description at `:4` and `undelivered`) and `schemas/server/reply_frame.v1.json` and any other schema embedding `ActionId`'s description change; no version bump, no changelog.
- Changes-table rows in specs 04, 05, 06. `rules.py --base d4f840ff` passes. KNOWN_LIMITATIONS: none lists this.

## 6. Audit of existing tests the step touches (test, expectation, verdict)

| Test | Expectation | Verdict |
|---|---|---|
| `kc_24a_start_an_end_before_the_check_dispatches_nothing` (`coordinator.rs:5068`) | `undelivered.is_empty()`; Abort id unasserted | fixed (S3) |
| `kc_18_a_start_failure_is_failed_arm` (`coordinator.rs:564`) | no dispatch, `Failed { arm }` | still right; extended (S2) |
| `kc_24a_start_an_end_in_the_window_records_the_batch_as_undelivered` (`device_paced.rs:1687`) | `[ActionId(1)]` | still right (S6) |
| `x:worker:ok:2` (`device_paced.rs:1604`), `x:submit:ok:3` (`:1660`), `x:submit:ok:2` (`:1780`, `coordinator.rs:4432`, `:4978`, `:5018`), `id.0 == 3` (`:4979`) | a Module submission after an n-entry batch draws n+1 | still right: the batch holds 1…n from admission and nothing draws between |
| `kc_24_an_admission_refusal_leaves_no_configuration_and_no_action_id` (`coordinator.rs:4217`) | Session `dispatched == [ActionId(1)]` | still right |
| `rs_16_*` freeze tests (`device_paced.rs:1034,1055,2037`; `coordinator.rs:1248,4850`) | ids from queues | still right; no batch |
| `kc_24_a_module_abort_ends_the_run` (J195's killer) | the Abort's id advances the counter | still right; row unchanged (F3) |
| `coordinator.rs:4915`, `:4662` (`undelivered.is_empty()` in Runs that reached `Running`) | nothing queued | still right (S5 is the explicit twin); the implementer re-reads each batch-bearing Run among them and records "cannot tell" if it ends before dispatch |
| every other test asserting `termination` | named fields | still right |
| the server and Python correlation tests | id equality only | still right; re-run in the exit checks |

## 7. #113

MockRadio `rx_settle`'s `None =>` arm (`crates/ezsdr-mock-radio/src/lib.rs:1292-1325`) writes the `segments` row's `ended_at` as `if registered.is_some() { origin } else { t }`; the registered clock ends at the origin (stage 4b-2d, `mr_19_an_abort_at_a_refused_first_block_ends_its_clock_at_the_origin`, `mock_radio.rs:2487`) and the transmit twin writes the clock's origin (`mr_25_a_transmit_clock_ends_at_its_origin_on_a_stop_at_arm`, `:3712`). Change: `self.end_layout_segment(Direction::Rx, k, origin, by, origin);` and the comment above states the rule once. Nothing else in `rx_settle` moves.

UHD (one rule, both Providers; INFERRED from reading, F8): the UHD Module has no `segments` section; its receive segment's only end record is the SampleClock's `ended_at`, and it registers a clock at its first non-empty block so a registered UHD clock has `expected > 0` (`rx.rs:656-683`, `:332-374`); a segment cut at or before its origin without a block has no clock and no row. The FakeDevice test `rx_stop_untimed` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:3959`) asserts `cut == origin` with the stop at 0 and that the cancelled segment registers no clock, and the TX test `core.rs:2446` pins a clock end at its origin. They are the basis for leaving UHD code unchanged.

Tests (`crates/ezsdr-mock-radio/tests/mock_radio.rs`, pure; `Harness`, `arm_start(2_000_000_000)`):

| # | Sequence | Test | Asserts | Wrong behaviour |
|---|---|---|---|---|
| M1 | `mock.stop(Orderly)` right after `arm_start` (instant 0 < origin; no block, no clock) | `mr_25_a_receive_segment_cut_before_its_origin_ends_its_row_at_the_origin` cell 1 | no sample-clock record for `mock/rx/s0`; the rx `segments` row's `ended_at.ticks == origin.ticks` | row ends at 0 |
| M2 | an untimed `Action::Stop { target: rx s0 }` pushed at arm, then `step(1_000_000_000)`, strictly before the origin (F1) | same test, cell 2 | same assertions | row ends at the step instant |
| M3 | `mock.stop(Abort)` at arm | same test, cell 3 | same assertions | row ends at 0 |
| M4 | the registered case | existing `mr_19_an_abort_at_a_refused_first_block_ends_its_clock_at_the_origin`, unchanged (clock and row end at the origin) | as is | registered path regressed |

The new test has the three cells only; the registered cell stays in its existing test (F1). Row N113-1: `end_layout_segment(Direction::Rx, k, origin, by, origin)` → `end_layout_segment(Direction::Rx, k, origin, by, t)`, killed by M1, M2, M3; existing N108-99 stays (M4).

`design/07-radio-model.md` RM-25: gains "A segment cut at or before its origin ends at its origin, empty, whether or not it registered a clock; a clock it registered ends there." `*Test:*` gains the new test. `design/09-mock-radio.md` MR-25: the last sentence becomes: a segment cut at or before its origin (an abort, a Stop booked ahead of the origin) ends at its origin: its clock if one was registered, and its `segments` row's `ended_at` always (MR-27), whatever instant the cut was made at. `*Test:*` gains the test. MR-27's `ended_at` is not changed. Changes-table rows in 07 and 09. No schema change.

## 8. Checks at the end of the step

`cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `schema_freeze` after `EZSDR_UPDATE_SCHEMAS=1`; the Python suite (`PYTHONDONTWRITEBYTECODE=1`, venv `~/.cache/ezsdr-venv`, `EZSDR_SERVER` from this tree) because the Manifest schema text changes; S9 and S1 ×20 with all cores busy; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base d4f840ff`; each new or touched row through `anchor.py` (once each) and then `remote_mutate.py` via `simq` for N110-1…8, N113-1, J181, J187, J193, J195, J198, N108-99. No `#[ignore]` hardware test computes an expectation from either rule (v1 §8).

## 9. INFERRED

- UHD never ends a registered receive clock at its origin (§7), read not run.
- `ProbeExecutor::submitting_from_start` passes `Action::Abort` through unchanged (S2b); the implementer checks first.
- `Harness.step` accepts an instant before the origin (M2); if it refuses, stop and report.
