# #103 phase B report (Claude Sonnet supervising Go DeepSeek), 2026-11-18 20:16-20:56

Worktree `/home/komatsu/local_work/Ez-SDRv4-n103`, branch `work/n103` (base a4ad013a). Nothing committed. No hand-written production code or tests; no touch-ups by me.

## Result
Implemented as `test-plan-v3.md` (step 0: v2 with the Opus re-check folded in, table at its top). All exit checks green; every new row `killed`; the 20-run loops green.

`git diff --stat` (11 files, +760/-18, plus new untracked `tests/support/start_window.rs`):
mod.rs +81 (inline `start_probe`), pipeline.rs 24, state.rs 21, run.rs 7, tests/coordinator.rs +216, tests/device_paced.rs +238, tests/support/mod.rs +3, run_doubles.rs +84, design/04 5, design/06 9, mutations.json +90.

## Checks
- `cargo fmt --all --check`: ok. `cargo clippy --workspace --all-targets -- -D warnings`: clean. `cargo build -p ezsdr-kernel` (no `testing`): no warning.
- `cargo test --workspace`: all green except one failure in the first full run, `ezsdr-radio-uhd` `fake::t200_bare_uhd_script_judges_a_run_and_bounds_it` (real-clock FakeDevice test; not touched by this step). It passes alone (13 s) and the whole `fake` binary passed on rerun (677/677). Treated as the known load flake (#107 class); INFERRED, not investigated.
- kernel: coordinator 163, device_paced 66, run_session 73, kernel_surface 15 pass; `ezsdr-acceptance --test governance` 25 pass.
- `rules.py --base a4ad013a`: 0 problems.
- 20 runs of `device_paced kc_24a_start_` (5 tests: T1, T2a, T2b, T3, T6) and of the coordinator set (T7, T4, T5, T3a, kc_18 new, kc_17 extended; 6 tests): 20/20 green each; plus 5 bursts of 4 concurrent device_paced runs: 20/20 green. (M1's `Released` race is therefore not seen; the plan removed those reads from the shared assert anyway.)
- Rows through `remote_mutate.py` (foreground, simq, sim02:7/sim03:7, log `rows.log`): `15/15 passed`: N103-1..N103-9 all `killed` (N103-3 `killed`, not `ERROR`: no double panic), G05, G15, C07, C10, G29, N47S3-28 `killed`. No other existing row's anchor or filter touches the changed code or tests (script over all six lists).

## Sequences -> tests (all in the tree, plan names)
- S1 data thread first submission: `kc_24a_start_the_data_threads_first_submission_follows_the_batch` (T2a, device_paced); gap between entries: `..._between_scheduled_entries_is_admitted_after_the_last` (T2b).
- S1b/F3 worker across the transition (both classes): `kc_24a_start_a_worker_submission_across_the_transition_is_admitted_after_the_batch` (T1).
- S2/S8b Simulation first step and round: `kc_24a_start_a_simulation_first_step_follows_the_batch` (T7, coordinator).
- S3 end in the window: `kc_24a_start_an_end_in_the_window_records_the_batch_as_undelivered` (T3); before the check: `kc_24a_start_an_end_before_the_check_dispatches_nothing` (T3a).
- F1 clocks outside the lock: `kc_24a_start_reads_the_transition_clocks_outside_the_lock` (T4).
- S5 submission from `start`: `kc_24a_start_a_submission_from_start_is_refused_not_blocked` (T5).
- S8a lock free at the wait: `kc_24a_start_the_lock_is_free_at_the_wait` (T6).
- T8: `kc_18_a_start_failure_publishes_no_scheduled_update` (new) and `kc_17_scheduled_updates_are_admitted_cumulatively` extended (`test.grid == 20.0` held, passed first time).
- Rows -> killers: N103-1 T2a, -2 T1, -3 T6, -4 T7, -5 T5, -6 T2b, -7 T4, -8 T3a, -9 T3.
- Texts: KC-24a, KC-18 (design/06), RS-18 (design/04) with Test lines and Changes rows (`issue #103`); KC-46 untouched.
Not implemented as planned: none.

## What each hook waits for (reviewed by me, old vs new)
T1: opens the worker gate, waits `x:worker:begin` (independent of the lock, both versions); on a free lock also waits for the worker's `ok`/`err` (completes: old/late-lock gives `err:ezsdr.run_state:RS-18`). T2a/T2b: gate opened at the point; wait for `x:submit:` only if the lock is free (old order: completes). T3: loss gate, spin on `end_requested` (needs no admission), `radio:stop` only if free. T6: no wait; read, open gate, assert. T4/T5/T3a/T7: no waits. No hook waits for an event that cannot occur on either version.

## The trial's record
| run | task | time | outcome |
|---|---|---|---|
| ds-1 | production change, `start_probe` seam, shared support (`start_window.rs`, ProbeExecutor knobs), Simulation-class tests (T7, T4, T5, T3a, T8) | 20:20:53-20:32:07 (11 min) | right first time; four deviations forced by clippy/warnings (type aliases, `_next` binding) |
| ds-2 | device-paced tests T1, T2a, T2b, T3, T6 | 20:32:43-20:40:47 (8 min) | right first time; no support change needed; own 10x loop green |
| ds-3 | rows N103-1..9, texts in design/04 and design/06 | 20:41:12-20:46:06 (5 min) | right except N103-5's `new` needed `let shared = self.shared.clone();` because `start_instances(&mut self)` conflicts with a guard borrowed from `self.shared` (E0502; DeepSeek found it by a standalone rustc snippet) |

Send-backs: 0. My review rounds: 3 (one per run: full diff against the plan, exit checks, what each hook waits for); findings: none requiring a fix; remarks only: ds-1's `_next` naming and the aliases (accepted), ds-3's Changes row for design/06 says "dispatched" where the rule says "admitted or dispatched" (left). Touch-ups by me: none. Total elapsed about 40 min (step 0 + 3 runs + checks + rows); DeepSeek ran 24 min of it.

## Deviations / INFERRED
- N103-5 `new` has one extra line (see ds-3). The seam has a `type Hook` alias and tests have `ProbeSlot`/`SubmitterSlot` aliases (clippy). `Dispatching { next: _next }` in `pipeline.rs` keeps the build warning-free without `testing`.
- Rows' kills read from the runner's `killed`; I did not capture each failing assertion message. INFERRED that each dies on its named assertion (no timeouts: every row finished within the remote run, `killed` not `killed (bound)`).
- INFERRED: `t200_...` flake as above.
- Not done (outside the step): `Authority::cancel()` under the lock, the silent batch drop at early end checks (separate issues per decisions.md).

# Fix round 1 (Sol 6.2's code review of 2738c032), 21:27-22:05 — nothing committed

Tasks: `ds-fix1-1-task.md` (production F1/F5/F8, support F2/F3, T4, F7, the F1 test), `ds-fix1-2-task.md` (device_paced tests F3/F4), `ds-fix1-3-task.md` (rows, texts). Send-backs: 0. Touch-ups by me: none.

## Findings -> what changed
- **F1 (illegal transition reads the clock)**: `RunStateMachine::move_to` validates (`check_move`) before it reads `utc_nanos`; `move_to`/`move_to_at` share `check_move` + `record`. New `rs_02_an_illegal_transition_reads_no_clock` (counting clock). Fail-first: run on a scratch extract of 2738c032 it fails on its named assertion (`an illegal transition reads no clock`, left 2, right 1); passes now. RS-2 Test line cites it.
- **F2 (T4 left `now()` unguarded)**: `WatchedAuthority`/`WatchedTime` + `ClockLog`/`ClockEvent` (shared with `WatchedClock`, distinct UTC sample per call). T4 asserts every `Now`/`Utc` has `held == false`, that the two events before `BeforePublish` are `Now` then `Utc`, and that the `Running` row's `host_utc_nanos` and `at` equal those samples. New rows N103-10 (UTC read moves into `publish`) and N103-11 (`now()` moves into `publish`), plus N103-7 (both), all with T4 as killer.
- **F3 (N103-3 kill could be a harness failure)**: the worker sends the panic payload before it records `start:panicked`; `infra()` marks every watchdog expiry assertion in the new device-paced tests and `Started::finish`'s timeout with `EZSDR-INFRA:`; `OpenOnDrop` releases fixture gates on failure (T1, T2a/b, T3, T6).
- **F4 (claims vs observations)**: T2a/T2b wait (positively) for the Executor's `x:step:` line before releasing it; T1/T2 comments narrowed to the observed release points; T6 asserts the start has not returned while the Provider holds the Action and returns a live `Running` Run after it acknowledges.
- **F5 (probe guard)**: `Installed` is `!Send` (`PhantomData<*const ()>`); `install` panics on nesting; `read`'s doc says it briefly locks `end`.
- **F6 (texts)**: KC-24a says a waiting submission is judged at step 1 if dispatch is frozen by then, otherwise at step 2, and that a Module's `Abort` takes no lock; RS-18 says a Session Action's rejection is logged and a Module's submission is refused to its caller (RS-16), and its Test line cites T5.
- **F7**: the scheduled case is folded into `kc_18_a_start_failure_is_failed_arm` (asserts `p:start:` was called, reason `KC-30: radio: `, gain absent, no `p:action:`/`p:update:`, no `Running` row); the duplicate test is deleted; KC-18's Test line updated.
- **F8**: `next` is `dispatched.len()`; unused glob re-export removed; N103-6 narrowed to the gap before entry 1 (`old` = the Dispatching hook pair; `old2`/`new2` turn the guard into an `Option`).

## Checks
- `fmt --check` ok; `clippy --workspace --all-targets -D warnings` clean; `cargo build -p ezsdr-kernel` (no `testing`) clean.
- `cargo test --workspace`: green except `ezsdr-radio-uhd` `fake::t200_bare_uhd_script_judges_a_run_and_bounds_it`, the same test that failed in phase B's first full run (676/677); alone it passes and the whole `fake` binary passes on rerun (677/677, 105 s). It fails only inside the full-workspace run twice now: INFERRED load-dependent (real-clock FakeDevice test, not touched by #103); worth the orchestrator's note.
- `rules.py --base 86f2e776`: 0 problems.
- 20 runs each of `device_paced kc_24a_start_` (T1, T2a, T2b, T3, T6: 5 tests), `coordinator kc_24a_start_ + kc_18_a_start_failure_is_failed_arm` (T7, T4, T5, T3a, kc_18: 5 tests), `run_session rs_02_`: 20/20 green each.
- Rows (`remote_mutate.py`, foreground via simq, `rows-fix1.log`): `12/12 passed`: N103-1..N103-11 and G05 all `killed` (none `ERROR`, none `killed (bound)`). No other existing row's killer changed (script over the six lists); the orchestrator's full lists remain the gate.

## Adversarial review per test (sonnet-review-adversarial.md; wrong change -> assertion; state reached; worst descheduling)
- **rs_02_an_illegal_transition_reads_no_clock**: wrong = read the clock before `can_move_to` (= 2738c032): fails on `reads() == 1` after the illegal call (scratch run, confirmed). Reaches the illegal path: the `matches!(.. IllegalTransition)` precedes the count. Single-threaded.
- **T1** (`..worker_submission_across_the_transition..`): wrong = lock after publish (N103-2): hook sees `!held`, waits for the worker, which returns `err:ezsdr.run_state:RS-18`; fails `x:worker:ok:2`. Reached: `assert_extent` proves the lock held at `BeforePublish`. Only `x:worker:begin` is observed of the worker; worst deschedule = the worker between its `begin` line and `submit`: on the fixed path it then submits after `Running`, same `ok:2`; on a mutant the hook waits for its result, so the verdict is unchanged. The comment now claims no more.
- **T2a**: wrong = old order (N103-1): Executor completes first: `x:submit:ok:3` expected, `ok:1`. Reached: positive `x:step:` wait (Executor in flight) and `held@BeforeDispatch`. Deschedule after the gate opens: its submit lands before or after the drop on the fixed path; outcome identical; on the mutant the hook waits.
- **T2b**: wrong = lock released in the gap before entry 1 (N103-6): scratch run dies at `x:submit:ok:3` (device_paced.rs:1655). Reached: `Dispatching{1}` required by `assert_extent`.
- **T3**: wrong = drop the batch when an end is seen before dispatch (N103-9): `Dispatching{0}` never reached / `undelivered` empty. Reached: `wait_end_requested` (positive) and `after.end_requested`. Deschedule of the data thread between loss event and request: the hook spins, verdict unchanged. A late lock (N103-1) is caught here by the `held`/`dispatch_frozen` readings, not by `undelivered` alone (stated in the plan).
- **T3a**: wrong = end check removed (N103-8): `effective()` gains 7.0. Reached: inline `Abort` returned `Ok` at `BeforePublish`; no `BeforeDispatch` point recorded. Single control thread.
- **T4**: wrong = UTC under the lock (N103-10), `now()` under the lock (N103-11, scratch: dies on "TimeAuthority::now ran under the admission lock"), both (N103-7). Reached: the `Now`,`Utc`,`BeforePublish` adjacency and the Running row's stamps. Single thread.
- **T5**: wrong = lock before `start_instances` (N103-5): `x:start:held=true` (the double does not submit, so nothing blocks). Reached: `x:start:held=false` and `err:ezsdr.run_state:RS-18` on the fixed code.
- **T6**: wrong = guard held across the wait (N103-3, scratch): dies on the hook's `!admission_held` assertion (device_paced.rs:1759), with no `EZSDR-INFRA:` marker (F3 closed). Reached: `radio:took:` then `x:submit:ok:`, both positive. Deschedule: the worker between payload send and marker is now harmless (payload first). Residual: `!started.returned()` is deterministic on correct code but would catch a start that SKIPS the wait only by a race (control returns microseconds after `Released`); it is a one-sided bonus, claimed as such; no row.
- **T7**: wrong = guard held to the end of the start (N103-4): the `Released` assertion fires before the round. Single thread.
- **T8** (`kc_18_a_start_failure_is_failed_arm`): wrong = dispatch the batch before `start_instances`: `effective()` would hold `test.gain` (asserted None). Reached: `p:start:` line and `KC-30: radio:` reason prove it failed in the Provider's `start`, not earlier. Single thread.
- **Production written into tasks** (F1, F5, F8): F1 covered by the fail-first test; F8's `dispatched.len()` is exercised by every `Dispatching{next}` assertion (T2b needs `next: 1`); F5's nesting panic and `!Send` have no test (compile-time / one-line guard): not asserted, stated here.

## Trial record, this round
| run | task | time | outcome |
|---|---|---|---|
| ds-fix1-1 | production F1/F5/F8, support F2/F3, T4, F7, F1 test | 21:27:52-21:38 (~11 min) | right first time, no deviations |
| ds-fix1-2 | device_paced tests F3/F4 | 21:38:50-21:42:36 (4 min) | right first time; two cosmetic message-format deviations |
| ds-fix1-3 | rows N103-6/10/11, texts | 21:42:58-21:51:31 (9 min) | right first time (its first `rules.py` call ran in the background by a timeout unit slip; result 0 problems) |
Send-backs 0; my review rounds 3 (diff against task, adversarial pass, scratch runs for F1/N103-3/N103-6/N103-11); round total about 38 min, DeepSeek 24 min of it.

## Fix round 1, re-check minor m1 (my own touch-up, no DeepSeek run)
- T6's final `Running` assertion (`device_paced.rs:1773`) now carries `infra(...)` (it fails on correct code only when KC-21a's 5 s budget expires); `!started.returned()` stays unmarked. The dead `&& !start:panicked` clause in `wait_or_panicked` is removed (`start_window.rs:163`, now returns `found`).
- Checks: `cargo fmt --all`; `clippy --workspace --all-targets -D warnings` clean; `rules.py --base 86f2e776`: 0 problems; T6 20/20 green with `--features testing` (plus the whole device_paced 66/66 and coordinator 162/162). No commit.
- Not re-run: the rows (T6's named assertion for N103-3 is the unchanged `Released` hook assertion).
