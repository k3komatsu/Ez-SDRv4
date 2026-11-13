# Note 51 stage 3 — test plan v2 (plan only)

Brief: `~/.cache/ezsdr-fixes/2026-11-13-n51-3/brief-plan-v2.md`. v1: `test-plan.md` (written at `e08c8c1`, kept untouched). Sol's review: `sol-plan.md` (FAIL: M1–M3, m4–m6); the orchestrator's dispositions: `plan-decisions.md`. Base of v2: `main` at `8d96743` (v1's base `e08c8c1` plus `3ef2764`, #67, and `8d96743`, the stage-4 bench tools). Written by the implementer (Claude Code Sonnet subagent). **No file of the tree was edited**; probes ran in `/tmp/n51-3-probe`, a copy made as the brief says (shared `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`), deleted at the end. Nothing was offloaded to DeepSeek and no subagent was used. No mutant was applied and `mutate.py` was not run (AGENTS §7, §9): every statement about a mutation row's outcome is INFERRED unless it says otherwise.

Evidence marks: **V** = read in the source or measured by a probe in this session (conditions stated); **V(v1)** = measured in v1 at `e08c8c1` and not repeated; **I** = inferred.

## 0. What changed from v1 (map of Sol's findings and of the base move)

| Finding | Answered in | Short form |
|---|---|---|
| **M1** wake tests can pass without a wake | §4.1 rows 2–4 and 6, §4.5 | the event is released only after the waiting call is *observed parked* (`WallPark::wait_parked_until`, a condvar predicate, no clock); the call runs on a worker with a 3 600 s horizon, so a lost wake is a hang that ends at a 10 s expiry, not an early return that a bound has to measure; G36 also reads a counter that proves the callback ran before `schedule` returned |
| **M2** SHORT replacement skips the measurement wiring | §5.1 | `restart_stage(...)` is extracted; the pure test calls it with synthetic `LATE_COMMAND` / `START_LEAD_SHORT` events built from the Provider's own payload types and checks decoding, association, the class and the reported row |
| **M3** enforcement ignores survived / equivalent / known-survivor rows | §6 | a marked row without `"kill": "bound"` is `ERROR` whatever its outcome (mutate.py), and the gate rejects it again from the row line's `real_clock` and a new `declared` field; baseline failures stay baseline failures; the cached-baseline path has its own cells |
| **m4** J58 | §3.5, §6.4 | stays `equivalent`; its filter is classified (selects 3 real-by-design tests), declared, no killer search |
| **m5** scan vs the kept 1 s bound | §4.3 rule R1/R2, §4.1 | one rule: a wall upper bound or positive wait expiry is `Wall::from_secs(n)` with n ≥ 3; every 1 s and ms bound is raised or removed; no row of the table keeps a 1 s bound |
| **m6** per-function sleep allowance | §4.3 allow-list | the allowance is per occurrence: `(fn, the call's own tokens, exact count, reason)`; Sol's `sleep(100 ms); assert!(count >= 50)` inside an approved fn is a scan fixture that must be rejected |
| base move | §1, §3, §5, §8 | see below |

**What the base move changed** (V: the whole `ezsdr-radio-uhd`/`ezsdr-acceptance`/`ezsdr-server` suite run with the probe on at `8d96743`, §2 P1):
- The marked set is **the same size**: 188 tests (139 `lib`, 41 `fake`, 8 `x3xx`), 2 254 probe lines. Two **new** real-clock lib tests (both built through `rig_with_links`, so they move with the helper): `provider::rx::tests::ur_26_a_cut_after_the_origin_of_a_silent_start_stops_at_once` and `…::ur_26_an_abort_stops_a_queued_start_when_uhd_rx_reads_the_plan` (row N67F-04). Two **gone** (replaced by #67): `…::ur_26_a_stop_before_a_queued_start_stops_after_its_origin` and `…::ur_26_an_abort_stops_a_queued_start_at_shutdown`. No new `fake` or `x3xx` real-clock test (the stage-4 tools' tests use `fake()`, which is Manual).
- **Marked rows: 200** (the gate's `real-clock (unclassified)` in `plan/phase8/reviews/s4bench-gate.json`): v1's 198 plus **N67F-04** (the new lib test) and **N67F-07** (`rm_26_the_timeline_against_uhd_rx`, already in v1's list, now selected by a second row). 155 `lib` rows (135 maintenance + 20 phase7), 43 `fake`, 2 `x3xx` (V: recomputed from the six `mutations.json` at `8d96743`; the per-test ids are Appendix A and B). After S2 still **27 marked rows**, the same 27 as v1 (V).
- The 14 per-site lib edits are still 14 (the list is in §3.1); the three-helper change still moves 119 of the lib tests. `rm_26_the_timeline_against_uhd_rx` (differential, `FakeDevice::new(FakeConfig { faults, ..default })` at `provider/rx/differential.rs:887`) is one of the 14.
- Every `path:line` into `measurement.rs`, `fake.rs` is re-checked and updated below; the Kernel files did not change in the two commits (`git diff e08c8c1 8d96743 --stat`: no `ezsdr-kernel` path).
- `rx_join_issue_67` is gone from `run.rs`; nothing in this plan referred to it.
- The `rx_start_lead_group_runs_on_the_fake` finding stands at `8d96743` (V, §5.1).

## 1. Summary

| | |
|---|---|
| **S2, the marked tests** | 188 tests build a real-clock FakeDevice. Treatment: move to the manual clock what does not depend on real time, keep real and declare what does. After it: **188 → 34 real tests, 200 → 27 marked rows** (5 `lib`, 22 `fake`), each declared `"kill": "bound"` (the 13 existing declarations stay). The lean route (3 helper lines + per-site edits) moves 154 of the 188 tests; it does **not** change `FakeConfig::default()` (decision 1). |
| **S3, Kernel `device_paced.rs`** | The 12 are identified (§4.1). The two wake tests G08 and G36, the loop test "every delivery wakes" and the cancelled-horizon test are rewritten around an **observed park** (M1). The data-thread drain test G05 and five sibling "N events in a wall interval" tests become condition waits. Every remaining wall expiry is a `Wall::from_secs(n ≥ 3)` literal (m5) or a pinned allowance (m6). A scan in `governance.rs` enforces it. |
| **Deferred items** | `rx_start_lead_group_runs_on_the_fake`: drop the one fragile assertion (a SHORT candidate exists), cover SHORT through the extracted `restart_stage` with synthetic events (M2). The `Gated` waits: lift the markers of 1c's sub-step D (§5.2). |
| **Enforcement** | `mutate.py` turns a marked row without `"kill": "bound"` into `ERROR` whatever its outcome; the row line carries `declared`; `gate.py` reports `real-clock (undeclared)` (a failure) and `real-clock (declared)` and refuses an undeclared marked row from the row lines alone (M3). Lands **last** (3e). |
| **Sub-steps** | 3a lib moves · 3b fake/x3xx moves + `Gated` markers · 3c-1 Kernel rewrite and doubles · 3c-2 Kernel scan · 3d `rx_start_lead` · 3e declarations + enforcement. Every one is below 500 changed lines; about 1 530 in all. §8. |

## 2. What was run (all in the probe copy; none in the tree)

| # | Probe | Result |
|---|---|---|
| P1 | `EZSDR_REAL_CLOCK_PROBE=1 cargo test -p ezsdr-radio-uhd -p ezsdr-acceptance -p ezsdr-server --no-fail-fast -- --show-output` on the unmodified copy of `8d96743`, started with `nohup` | lib 668 ok; `fake` 605 ok, **2 failed**: `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake` (an `EZSDR-TIMING:` marker: host lateness under the parallel load, the 2c rule re-runs it serially; not a base defect) and `t200_bare_uhd_script_judges_a_run_and_bounds_it` (its SIGHUP/SIGQUIT cells fail: INFERRED to come from how the probe was started, `nohup` in a background shell ignoring SIGHUP; I did not re-run it in a clean shell, and `nohup` alone would not explain SIGQUIT). All other binaries ok. 2 254 `EZSDR-CLASS: real-clock` lines, attributed per test from libtest's `---- name stdout ----` blocks: **139 lib, 41 fake, 8 x3xx** (the same counts as v1; the identity differs by two lib tests, §0) |
| P2 | `--lib` with the three `FakeDevice::new` of `test_support.rs` (`rig_with` with `clock: Manual` forced over `config`, `rig_with_inputs`, `rig_with_links_on`) set to Manual | 668 passed, 0 failed; marks fall 139 → **20** tests (6 real by design, 14 per-site) |
| P3 | the same plus `FakeConfig::default()` → Manual (`device.rs:418`) | 667 pass, 1 fails: `device::tests::ur_33_a_real_fake_s_time_counts_from_its_own_creation` (it needs the default to be Real); 5 marked tests remain (the explicit-Real ones). So the 14 per-site tests pass on Manual |
| P4' | the 13 `fake` tests v1 moves, default and `fake_real` and every `ClockMode::Real` of `fake.rs`/`common/mod.rs` forced Manual, each alone (`--exact --test-threads=1`, `EZSDR_TEST_WATCHDOG_SECS=30`), 3 runs each | **13 of 13, 3/3 each (V)**; also `x3xx` 8/8 three times with the default flipped |
| P5' | `rx_start_lead_group_runs_on_the_fake` with `EZSDR_RIG_DELAY_MS`/`EZSDR_RIG_DELAY_AT`: none, 5 ms submit, 60 ms now, 60 ms advance: 3/3 pass each; 20, 60, 200 ms submit: **9/9 fail at `measurement.rs:4323`, `a SHORT candidate`**. Then the same test with that one assertion deleted (probe copy only): 20, 60, 200, 20 ms submit: 4/4 pass | §5.1 |

V(v1), not repeated: the P4 classification of the 28 kept `fake` tests (11 hang on Manual: 101/134; 7 pass on Manual but the real clock is their subject; 10 probe tools, pass 3/3 on Manual but rehearse a tool that waits in real time); the 20 serial + 3×8 parallel lib runs of the helper-only variant; the stage-printing run of §5.1. The 3a/3b sub-steps repeat their own load checks (§8).

Not run: any mutant, `remote_mutate.py`, `gate.py`, the `uhd` feature, `hardware`, a Kernel test, the governance scan on the Kernel file (it does not exist yet).

## 3. S2 — the marked tests, one by one

Classes: note 51 §6.5 and `51-tables.md`. Rule used for each test: **move** to `ClockMode::Manual` when no part of its verdict is a real-time quantity *and* it passes on Manual; **keep real and declare** when real time is its subject or Manual hangs it. Lists in the appendices (regenerated at `8d96743`).

### 3.1 `lib`: 139 tests → 6 stay

Where they come from (V): the crate-internal rig. `rig`, `rig_with`, `rig_with_inputs` and `rig_with_links_on` in `src/provider/test_support.rs` build `FakeDevice::new(FakeConfig::default())`.

| Group | Tests | Treatment | Evidence |
|---|---|---|---|
| through the three `rig*` helpers | 119 | the helpers force `clock: ClockMode::Manual` (`rig_with`: `FakeConfig { clock: Manual, ..config }`, so a caller's explicit `clock` is overridden; a test that needs the real clock builds its device itself and calls `rig_on`) | P2 |
| 14 that build a device themselves | 14 | one token per site. The list (P2's remaining marks minus the six): `provider::control::tests::ur_25_a_start_rx_takes_the_line_lock_once_for_its_instant_and_its_booking` (`LockSpy::new(FakeConfig::default())`), `provider::rx::differential::rm_26_the_timeline_against_uhd_rx`, `provider::rx::tests::{ur_13_a_partly_applied_set_is_configured_whole_without_reopen, ur_25_a_refused_start_is_labelled_as_a_command_with_the_start_s_id, ur_26_a_receive_start_queued_by_the_stop_ends_its_owner_without_detaching, ur_29_a_loss_after_a_configure_starts_no_receive_segment, ur_29_a_loss_between_two_channels_of_a_configure_applies_no_more, ur_29_a_lost_receive_owner_configure_releases_stream_lock_before_reporting, ur_29_an_owner_does_not_configure_a_segment_a_stop_dropped_from_the_plan, ur_36_owner_configure_takes_each_command_s_own_snapshot}`, `provider::tx::tests::{ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts, ur_22_tx_stop_preserves_recovery_for_later_bursts, ur_22_underflow_close_failure_reopens_before_the_next_payload, ur_28_a_later_stream_s_reopen_discards_only_its_correlations}` | P3: they pass on Manual |
| real clock by design | 6 | **keep**: `authority::tests::ur_07_the_host_timeline_is_judged_on_real_time_on_the_real_clock`, `…::ur_08_the_manual_clock_s_relations_hold_only_at_the_instant_measured`, `device::tests::ur_33_a_real_fake_s_time_counts_from_its_own_creation` (needs the default to be Real: no edit), `provider::core::tests::ur_22_t3_ack_read_returned_after_deadline_is_not_a_flush_success`, `…::ur_22_t3_flush_reports_after_the_deadline_stay_the_runs`, `provider::core::t3::ur_75_a_report_read_after_attempt_1_s_deadline_stays_for_the_report_owner` | P3 (they are the five marks that remain with the default flipped, plus `ur_33`, which fails with it) |

After the treatment the marked lib rows are **5**: J58 (filter `provider::`, 136 tests, which includes the `ur_22_t3` pair and `ur_75`), J290, J299, J316, P1190 (V: matching the six tests to the rows at `8d96743`). The other 150 lib rows lose the mark, among them the two new rows N67F-04 and N67F-07.

**Checks the sub-step carries (3a)** — as v1, with the new numbers:
1. **Stability**: 20 serial runs of the lib binary and 3 rounds × 8 parallel processes on the real edit, 0 failures. V(v1) for the helper-only variant (20/20 and 24/24); the probe above ran the lib once, 668/668.
2. **A moved test is not a test with no time in it.** On Manual the test thread is an unregistered driver; a test that sleeps and reads the device clock sees the clock free-run while it idles. The 3a table gets one column per moved test: "no time in the verdict" or "threads plus real waits" (Frozen territory, stage 4: leave). `grep -nE 'thread::sleep|recv_timeout|Instant::now|\.elapsed\(' ` inside each moved fn fills it. I did not read the 133 moved tests for that (I).
3. **Mutants that used to die on a real `recv_timeout`.** A mutant that makes a thread wait for a real deadline stops a manual clock for good and, with no watchdog in the lib binary (V: `grep -n watchdog crates/ezsdr-radio-uhd/src` finds none), ends in `TEST_TIMEOUT` = `ERROR` for an undeclared row. Candidates (I, from `old`/`new`): N51A-13, P466, J55, N92-2/3/7/8, P1190 (stays real), N51A-14/15 (already `bound`), and the two new #67 rows N67F-04/07 (their tests use `time.advance_to`, so no real wait; the exit check settles it). **3a's exit check is `remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <the 155 lib ids>`** (the ids are the union of the bracketed ids of Appendix A: 155 lib rows; v1's `rows-real.json` and `real-tests.json` in this folder are superseded by Appendices A–C), no row `ERROR`; a row that turns `ERROR` is retargeted or declared, not accepted.

### 3.2 The default of `FakeConfig` — decision 1: do not flip it

(a) the lean route (chosen): helpers + per-site edits; `FakeConfig::default()` stays Real; `ClockMode::Real` stays the documented default (`host.rs:54-55`: "the default is `ClockMode::Real`", a public type re-exported by `lib.rs`, named by `ezsdr-server/tests/protocol.rs` and `ezsdr-acceptance/tests/{x3xx,uhd,parity}.rs`). (b) flipping the default (`device.rs:418` plus the doc) is a production change of a public type, and it makes `Gated::new(FakeConfig::default())` silently Manual (P3 in v1: a held `Gated` thread then stops the manual clock and the `fake` binary aborts at the watchdog). Not user-visible; decided by the orchestrator (plan-decisions 1).

### 3.3 `fake` binary: 41 tests (table in appendix B)

| Class | Tests | Rows marked | Treatment |
|---|---|---|---|
| **Hang on Manual** (V(v1): 101/134 on three runs) | 11: `run_a_bound_is_marked_by_advance_to`, `…_run_until_end`, `…_wait_for`, `run_a_dropped_live_run_is_judged`, `run_unwinding_marks_a_bound_and_never_panics_again`, `run_any_a_perf_trial_still_fails_on_a_bound`, `ur_16_*` ×3, `ur_26_*` ×2 | 10: N51-2A-14, -15, -17, P467, P274, P275, P277, U42, P245, P442 | **keep real, declare `bound`**. Their subject is a real-time bound (`JOIN_BOUND`, KC-21a, the wrapper's marker): a held or `TxBlocks` thread stops a manual clock for good |
| **The real clock is the subject** | 7: `ur_07_*` ×5, `ur_21_a_descheduled_timed_send_is_on_the_manual_clock_and_late_on_the_real_one`, `ur_33_a_real_fake_s_time_starts_at_its_own_creation` | 3: B23, B24, J295 | **keep real, declare `bound`** (the five `ur_07_*` and `ur_21_a_desc…` are already on `TIME_ALLOWED`) |
| **Probe tools that wait in real time** | 10: `bench1115_*` ×9, `iii_restart_fill_probe_rehearses_on_the_fake` | 9: B1115-30, -31, -83, -124 … -129 | **keep real, declare `bound`** (decision 2). They pass on Manual (V(v1) 3/3) but the tool's real sleeps then run against a device clock that free-runs while the thread idles, so a pass proves little |
| **Move** | 13: `common::measurement::multichannel::tests` ×4, `common::probe_device::tests` ×2, `h4_*` ×2, `ur_05_*` ×2, `ur_12_every_advertised_rate_is_accepted`, `ur_23_g2/g3` (explicit `recording(ClockMode::Real)`: two literals) | 21 rows (appendix B) | **move to Manual**. **P4' at `8d96743`: 13/13, 3/3 each (V).** `ur_05` ×2 hold no time source, `ur_12`'s only wait is the Run's device-clock `wait`, `ur_23_g2/g3` read in full (no time). `h4_*` and `multichannel` drive shared measurement code (`measure_*`, `run_stage`) that `hardware.rs` also calls: expectations are fake-side (classes, counts, record shape); §7 |

Load check per moved test (3b, as 2c did): 20 serial runs and 3 rounds × 8 parallel processes, 0 failures.

### 3.4 `x3xx` (`ezsdr-acceptance`): 8 tests, 2 rows → move

`d3_*` build `FakeDevice::new(FakeConfig {…, ..Default::default()})` for the "two Providers decide alike" comparisons (V: `x3xx.rs:74, :104, :132, :155, :176, :192, :211, :232`; the file is unchanged by the two commits): no Run, no time. 8/8 on Manual three times (V, with the default flipped). One token per site (`clock: ezsdr_radio_uhd::ClockMode::Manual`, as `uhd.rs:47` does). Rows P17, P18 lose the mark.

### 3.5 What remains marked after S2

| | tests | rows (to declare `bound`) |
|---|---|---|
| lib | 6 | J58, J290, J299, J316, P1190 |
| fake | 28 | N51-2A-14, N51-2A-15, N51-2A-17, P467, P274, P275, P277, U42, P245, P442, B23, B24, J295, B1115-30, B1115-31, B1115-83, B1115-124, B1115-125, B1115-126, B1115-127, B1115-128, B1115-129 |
| x3xx | 0 | — |

27 rows (V: recomputed at `8d96743` from the six lists and the kept-test list). **J58 is `equivalent`** (`mutations.json` J58's `equivalent` field: both happen under the plans' lock; no test can order them). It stays so. Its filter `provider::` selects 136 tests, three of them real by design; no killer is looked for. The classification is in §6.4.

## 4. S3 — Kernel `crates/ezsdr-kernel/tests/device_paced.rs`

Every test in the file runs on `WallAuthority` (`tests/support/paced.rs`): a host clock (nanoseconds since the double was built), no manual mode, no wrapper, no mark. `RunHandle::now()` is therefore the wall. There is no clock-independent observation of "the call returned early" for most tests: what a test can honestly use is **(a)** an *observed precondition* (the waiter is seen parked) followed by a call that can only end if the wake works, with a seconds-long expiry as the failure (P3), or **(b)** a condition wait (a `Probe` condvar), or **(c)** a must-end bound of ≥ 3 s against a horizon at least twice as long. None of them is a clock-free proof of "returned early"; the rows that kill by an expiry stay ordinary `killed` (the Kernel writes no `EZSDR-INFRA` marker, §4.6).

### 4.1 The 12, identified, and their treatment (V: the counts reproduce the note's 4 + 7 + 1; the Kernel files are unchanged by the two commits)

Q5's "6 are phase7 filters" should read "5 phase7 (G05, G08, G36, C05, C06) + maintenance N47S3-28": C07 is `kc_36_`, which selects 8 tests and none holds a wall upper bound that judges a verdict (`kc_36_renew…`'s `while elapsed < 600 ms` only sets how long the test renews a 200 ms Lease: the TTL is its subject, a pinned allowance in §4.3). The other seven of the 12 have no row.

| # | Test | Bound today | Pass | Row | Treatment |
|---|---|---|---|---|---|
| 1 | `kg_02_the_data_thread_drains_a_sink_while_no_call_runs` (:245) | two `sleep(100 ms)`, `second >= first + 50` (:248-252) | ~50 ms | G05 | **condition wait**: `let first = count(..); assert!(probe.wait_until(\|lines\| count_prefix(lines, "rec:block:") >= first + 50, Wall::from_secs(10)))`. The test thread blocks on the `Probe` condvar and makes no call to the Run, so "the data thread drains with no call running" is still the property. A mutant without the data thread (G05) never moves the count: the wait expires at 10 s and the assert fails |
| 2 | `kg_02_wait_for_returns_when_the_data_thread_delivers` (:348) | `elapsed() < 1 s` (:356), `now < horizon - 3 s` (:358) | ~30 ms | G08 | **observed-park shape (§4.5)**, `woken_rounds(1, false)`; both wall reads go |
| 3 | `kg_02_every_delivery_wakes_a_waiting_call` (:422) | `elapsed() < 200 ms` × 50 | ~10 ms each | none | `woken_rounds(50, false)` |
| 4 | `kg_02_a_wake_that_fires_before_schedule_returns_still_clears` (:443) | `elapsed() < 200 ms` × 20 | ~10 ms each | G36 | `woken_rounds(20, true)`, plus the callback-before-return counter (§4.5) |
| 5 | `kc_29_an_end_the_data_thread_requested_is_cleaned_up_at_the_next_call` (:711) | `elapsed() < 1 s` (:730) over `advance_to(+5 s)` | µs–ms | C06 | horizon **10 s**, bound `< Wall::from_secs(5)` (v1 left the 5 s horizon with a 5 s bound: a mutant that waits to the horizon would not have been separated). C06's kill is the `Running` assertion (:719-725), unchanged |
| 6 | `kg_03_a_cancelled_horizon_wakes_the_control_loop` (:865) | `elapsed() < 1 s` (:873) | ~20 ms | none | the cancel wakes a *parked* loop: same worker-plus-park shape as #2 with the loss released through `losing_device_on(gate)` (the double has it) instead of a timed `losing_device(20 ms)`, `run_until_end(+3 600 s)` on the worker, expiry `recv_timeout(Wall::from_secs(10))`; the result `Err(Ended)` is asserted. The result alone cannot decide it (a lost wake sleeps to the horizon and returns the same `Err(Ended)`), which is why the horizon is made unreachable |
| 7 | `kc_21a_an_end_requested_during_the_wait_ends_it` (:967) | `elapsed() < 1 s` (:976) | ~100 ms | C05 | the submit runs on a worker; the Provider is `never_finishing().losing_device_on(gate)`; the test waits for `probe.wait_for("radio:took:UpdateParameter", 10 s)` and then opens the gate, so the loss comes *after* the Provider took the Action (no timed `losing_device(100 ms)`: a timed loss before the submit reached its wait would leave the wait `end.is_some()`-short-circuited and the mutant alive). Expiry `returned.recv_timeout(Wall::from_secs(3))` (KC-21a's budget is 5 s and fixed by the Kernel (`coordinator/paced.rs:22-25`: `RECHECK` 10 ms, `DEFAULT_HOST_BUDGET_NS`); the mutant returns at 5 s). The window between "took" and the dispatcher's wait is microseconds and only hides the mutant (INFERRED survivor direction), never fails the baseline |
| 8 | `kg_04_a_simulation_session_does_not_wait` (:1129) | `elapsed() < 1 s` (:1142) | µs | none | keep the property, bound `< Wall::from_secs(3)` (a KC-21a wait would return at 5 s; the `running(&run)` assert after it already fails that case) |
| 9 | `kg_05_a_device_paced_spec_run_ends_at_its_scheduled_stop` (:1162) | `elapsed() < 1 s` (:1176) | ~30 ms | none | horizon 5 → **10 s**, bound `< Wall::from_secs(5)`; the `Completed` result is the oracle, the bound is a must-end backstop (the scheduled stop is on the sample clock, not gateable) |
| 10 | `kg_10_a_device_lost_from_a_provider_thread_aborts_the_run` (:1319) | `elapsed() < 1 s` (:1325) | ~20 ms | none | as #9 (the loss is the test's own subject; the result and the `Abort` transition are the oracle) |
| 11 | `tm_16c_a_paced_authority_accepts_an_instant_already_passed` (:1298) | `elapsed() < 5 ms` on `next_wakeup` (:1311) | µs | none | **drop the bound**: the assertion that decides is `assert_eq!(next_wakeup(), Some(passed))`; a `next_wakeup` that hung is caught by the test timeout, and no row exists. The scan then has no sub-3 s upper bound left to allow here |
| 12 | `kg_21a_submit_cannot_complete_before_the_booking_gate_opens` (:1091) | `recv_timeout(150 ms).is_err()` (:1105) | n/a (a negative wait) | N47S3-28 | **negative wait, kept and pinned** (P3a, §4.3). A slow host only delays the mutant's early return: the mutant can survive, the baseline never fails |

### 4.2 Siblings the scan finds that are not among the 12 (V: grep + read)

Same class (a positive expectation inside a wall interval); the scan of §4.3 cannot pass over them, so they are included. v1 listed the 500 ms and 200 ms sites; this list is complete against the scan's rules R1/R2 (V: `grep -n 'sleep(\|elapsed()\|recv_timeout\|wait_for' device_paced.rs`, 80 hits read).

| Site | Form | Treatment |
|---|---|---|
| `probe.wait_for("radio:stop", Wall::from_millis(500))` at :474 (G30), :704 (G10), :717, :743, :754; `wait_for("radio:took:…", from_millis(500))` at :1031 | positive wait, 5–25× the pass | → `Wall::from_secs(10)` (the kc_36 tests already use 10 s). Rows G10, G30: their kill is the expiry, now 10 s; no declaration (§4.6) |
| `wait_for("radio:stop", Wall::from_secs(1))` at :506, :775, :776, :807; `wait_for_count("radio:tail", 2, Wall::from_secs(1))` at :848 | positive wait, **1 s** (v1 missed these five: they violate R2) | → `Wall::from_secs(10)` |
| `while begun.elapsed() < Wall::from_secs(2)` at :397 (`kg_02_the_first_delivered_stopping_event_is_the_termination_cause`) | a loop's give-up bound (v1 missed it: violates R1) | → `Wall::from_secs(10)` |
| `sleep(Nms)` then a count judgment: :261 (`kg_02_every_step_before_finish…`, 50 ms, `!before.is_empty()`), :310 (G07: 200 ms, `> 1 000` steps), :824 (`kg_03_step_3…`, 20 ms), :849 (`kc_46b…`, 100 ms, `steps >= 2`), :1032 (`kc_21a_a_module_s_own_submission…`, 100 ms, `later >= 2`) | "N events in a wall interval" | `Probe::wait_until` with a 10 s expiry, as #1. G07's "no STEP_LIVELOCK event" is read after the count passed 1 000 (a livelocking mutant crosses the livelock threshold, 1 000 same-instant rounds, before it can step 1 000 times) |
| `wait_for("radio:stop", from_millis(200))` inside `Gate::check` (:1067) | a helper's bounded pause, the result is ignored (`let _ =`) | not a test fn: not scanned (§4.3 limit); no change |
| the negative sleeps: :329, :331 (G32, count must not move), :718 (`kc_29…`, state still `Running`), :755 (`kg_03_an_abort_stops_the_data_thread`, no step after the stop), :883 (`kg_03_dropping…`, grace before `drop`) | a slow host only weakens them | pinned allowances (§4.3) |
| lower bounds and clock-passing sleeps: :1156 (`>= 50 ms`, not matched by R1), :1287 (a 2 ms sleep in a callback), :1306 (a 6 ms sleep so the clock passes an instant) | cannot false-kill | :1287 and :1306 are pinned allowances; `>=` is not an upper bound |
| TTL subjects: :499-500 (`kc_36_renew…`: `while elapsed < 600 ms` + a 50 ms sleep), :519 (`kc_36_adopt…`: a 300 ms sleep) | a wall TTL *is* the subject (`Lease::detached(ms)`) | pinned allowances; C07's kill is one of eight tests |

### 4.3 The scan the note names (§8, S3 row: "書き直す（段 3）＋走査"), one rule for bounds and scan (m5, m6)

Extend `crates/ezsdr-acceptance/tests/governance.rs` (it tokenises `blank(source)`, has `paren_close` and the allow-list shape of `TIME_ALLOWED` with its "every row is used" test) to `crates/ezsdr-kernel/tests/device_paced.rs`, inside `#[test]` fns only (as 2a; helpers are not scanned, a stated limit).

**The one rule.** A wall expiry that judges a *positive* outcome — a `<`/`<=` against a `Wall::from_*` literal, the timeout of `.wait_for(`, `.wait_for_count(`, `.wait_until(` or `recv_timeout(` — is `Wall::from_secs(n)` with `n >= 3`; anything else is a pinned allowance or a violation. The bound a test keeps is therefore always ≥ 3 s, and it separates the wrong behaviour's own duration (a horizon, or KC-21a's 5 s budget) with margin: 2× in rows 5, 9, 10 (a 10 s horizon, a 5 s bound), 1.67× in rows 7 and 8 (a 5 s budget, a 3 s bound; the floor is the rule's, the budget is the Kernel's).

- **R1** — token `<` (or `<` `=`) followed by `Wall` `:` `:` `from_millis(` , or `from_secs(` with a literal `< 3`, is a violation. The match is on the right-hand side, so `begun.elapsed() < …` and `waited < …` are both caught (v1's rule keyed on `.elapsed()` and missed `waited < from_secs(10)`; that one has `n >= 3` and passes).
- **R2** — inside the parentheses of `.wait_for(`, `.wait_for_count(`, `.wait_until(`, `recv_timeout(`: any `Wall::from_millis(…)` or `Wall::from_secs(n < 3)` is a violation.
- **R3** — any `thread::sleep(` is a violation.

**Pinned allowances (m6).** `KERNEL_WALL_ALLOWED: [(fn, call, count, reason)]`, where `call` is the occurrence's own tokens joined without spaces (the sleep call or the comparison from `<` through the closing `)`, found with `paren_close`) and `count` is how many times that exact call occurs in that fn. The scan collects `(fn, call) → occurrences` for R1–R3 hits and checks the table both ways: a hit whose `(fn, call)` is not in the table, or whose count differs from the row's, is a violation at the hit's line; a row with no hit is a failure of the "every allowance is used" test. The reasons are from {`negative: a slow host only weakens it`, `lower bound: cannot false-kill`, `TTL is the subject`}. The table (9 fns; every other fn has none):

| fn | call | n | reason |
|---|---|---|---|
| `kg_02_a_failed_sink_is_not_stepped_again` | `thread::sleep(Wall::from_millis(100))` | 2 | negative: the count must not move (G32) |
| `kc_29_an_end_the_data_thread_requested_is_cleaned_up_at_the_next_call` | `thread::sleep(Wall::from_millis(50))` | 1 | negative: state still `Running` |
| `kg_03_an_abort_stops_the_data_thread` | `thread::sleep(Wall::from_millis(50))` | 1 | negative: no step after the stop |
| `kg_03_dropping_a_live_device_paced_handle_cleans_up` | `thread::sleep(Wall::from_millis(20))` | 1 | negative: grace before `drop` |
| `kg_21a_submit_cannot_complete_before_the_booking_gate_opens` | `recv_timeout(Wall::from_millis(150))` | 1 | negative (N47S3-28) |
| `tm_16c_a_paced_callback_sees_now_at_or_after_its_instant` | `thread::sleep(Wall::from_millis(2))` | 1 | lower bound |
| `tm_16c_a_paced_authority_accepts_an_instant_already_passed` | `thread::sleep(Wall::from_millis(6))` | 1 | lower bound |
| `kc_36_renew_moves_the_deadline_the_data_thread_checks` | `thread::sleep(Wall::from_millis(50))` and `<Wall::from_millis(600)` | 1 + 1 | TTL is the subject |
| `kc_36_adopt_clears_the_deadline_the_data_thread_checks` | `thread::sleep(Wall::from_millis(300))` | 1 | TTL is the subject |

**What the pin does not do (stated).** It pins the call, not the judgment that follows it: a reviewer who approves a new row reads the next statement (the reason string names it). A wall read hidden in a helper, and a bound read through a variable against a non-literal, are outside the scan (same limit as the 2a scan).

**Scan fixtures (string tests, the 2a style).** Each must hold:
1. `fn t` (not pinned) with `sleep(Wall::from_millis(100)); assert!(count(&probe, "x") >= 50);` → 1 violation (R3).
2. **Sol's bypass**: the pinned fn `kg_02_a_failed_sink_is_not_stepped_again` with its two approved sleeps *plus* `std::thread::sleep(Wall::from_millis(100)); assert!(count(&probe, "rec:block:") >= 50);` → rejected (count 3 ≠ 2).
3. the same fn with a different duration (`from_millis(250)`) → rejected (call not in the table).
4. a pinned call in another fn of the file → rejected.
5. a pinned fn with *one fewer* occurrence than its row → "every allowance is used" fails.
6. `assert!(begun.elapsed() < Wall::from_secs(1))` → R1; `from_secs(2)` → R1; `from_secs(3)` → clean; `Wall::from_millis(5)` → R1; `<=`; and `assert!(waited < Wall::from_secs(10))` clean, `assert!(waited < Wall::from_millis(900))` R1.
7. `probe.wait_for("x", Wall::from_millis(500))`, `from_secs(1)` → R2; `from_secs(10)` → clean; `recv_timeout(Wall::from_millis(100))` → R2; `recv_timeout(Wall::from_secs(5)).is_err()` clean (it is a bound ≥ 3 s).
8. a `thread::sleep` in a non-test helper → clean; the same inside a `#[test]` closure → violation (closures belong to the test).
9. comments and strings holding the patterns → clean.
10. the real tree: clean and every row used.

Mutation rows for the scan itself (a `tools`-side list is not needed; AGENTS §9 step 2: written with the code): drop R1's `3`, drop R2's `from_millis` test, make the count check `>=` instead of `==`, make the unused-row check vacuous; each is killed by one of the fixtures above. `governance.rs` is test code: rows follow the 2a/2c precedent (the Python-free route: `rules.py` anchors, `remote_mutate` runs the `governance` binary).

### 4.4 `Probe::wait_until` (decision 5)

`tests/support/run_doubles.rs`: `pub fn wait_until(&self, pred: impl Fn(&[String]) -> bool, timeout: Duration) -> bool`, one condvar loop over `lines`; `wait_for_count` becomes `self.wait_until(|lines| lines.iter().filter(|l| l.as_str() == line).count() >= count, timeout)` (a behaviour-preserving rewrite that also removes the duplicated count in its timeout branch). The file is shared by other Kernel test files; all of them compile against `wait_for`/`wait_for_count`, which keep their signatures.

### 4.5 The observed-park shape (M1)

**What goes wrong without it (Sol M1, V by reading).** `wait_for` first answers from `delivered` (`stepping.rs:440-443`): a MARK already delivered before the call parks nothing, so removing the wake (G08) or leaving a wake pending (G36) cannot fail a test whose provider marks 30 ms or 10·i ms after the start (`ThreadedProvider::marking(kind, after)`, `support/paced.rs:400`), if the test thread is descheduled past the mark before it calls.

**The shape.**
1. **Gated emission.** `ThreadedProvider::marking_on(kind, release: Arc<(Mutex<u32>, Condvar)>)` (test double, `paced.rs`): its thread emits one `kind` for each unit the release counter holds beyond what it has emitted (polled with the thread's existing 1 ms loop, like `losing_device_on`'s gate). `fn release_one(&Release)` increments it. The timed `marking(kind, after)` stays for the other tests.
2. **Observed park, no clock.** `WallPark::wait_parked_until(tick, timeout)` (exists, `support/paced.rs:210`) waits on the Authority's own `changed` condvar for `parked_until == Some(tick)`, which `next_wakeup` sets under the lock just before it waits and clears after (`support/paced.rs:244-247`). Its timeout is an expiry-equals-failure bound (10 s), not an oracle. This is the #89 S1 park-probe shape that `kc_36_a_disconnect_signal_ends_a_waiting_advance` already uses (:543-559).
3. **The call runs on a worker with an unreachable horizon** (`after(&run, 3 600 s)`), so a lost wake is a hang, not an early return a bound has to measure. The worker sends the horizon's root tick to the test, then calls `wait_for`, then sends the result. The test: `ticks.recv_timeout(10 s)` → `park.wait_parked_until(tick, 10 s)` → `release_one` → `found.recv_timeout(10 s).expect("a delivery woke the parked wait_for")`. A missing wake never returns: the expiry fires at 10 s and the test fails. No `elapsed()` anywhere.
4. **G36's extra proof.** `WallPark::fired_before_schedule_returned() -> u64`: a counter in `WallState`, incremented in `schedule` when, on return, the callback of that call has already run (only with `firing_before_schedule_returns()`). Each round asserts it rose (`> before`), which proves the interleaving the row's name says (the callback ran on the parked `next_wakeup` before `schedule` returned) happened in *that* round. Mutant G36 then leaves `wake_pending` stuck after round 1, round 2's `wake(false)` returns early, nothing wakes the parked call, and the round's `found.recv_timeout` expires.
5. **The double's 20 ms window goes.** `schedule`'s fire-first wait is today a wall window of 20 ms (`support/paced.rs:110-120`): a waiter not scheduled within 20 ms would make the order unproven and the new counter assertion fail under load. It becomes a predicate: `WallState.waiting: bool`, true from the entry of `next_wakeup` to its return (one wrapper, all exits), and `schedule` waits while `!ran.contains(&seq) && waiting`, **capped at 10 s** (`wake()` holds `wake_handle` across `schedule`, `coordinator/paced.rs:164-166`, so an uncapped wait would turn a missed interleaving into a hang rather than a failed assertion; the cap makes it the per-round counter assertion that fails). The waiter runs the due callback or leaves `next_wakeup`, which notifies. With no waiter (the cleanup `wake(true)`) it returns at once instead of after 20 ms. About 25 lines in `paced.rs`; the double has no mutation row; the G36 test proves the double (counter 0 if the wait degraded).
6. **Mutation each test kills**: G08 (`wake(&shared, false)` removed): rounds of #2 and #3 never return → expiry. G36: round 2 of #4. Both kills are real-time expiries without a marker: outcome `killed`, ~10 s per run of the row.

`woken_rounds(rounds, fire_first)` is one helper (~40 lines) behind the three tests; #6 uses the same worker/park/release mechanics with `losing_device_on(gate)`.

**Verified here**: that `advance_to_with` of a device-paced Run parks in `next_wakeup` on the horizon's tick (`stepping.rs:394-413` schedules `t` and `run_loop` calls the Authority; the existing KC-36 test observes it at `:557`). **Not yet run (I)**: the helper itself. 3c-1 starts with 20 serial runs of the helper on the unmutated tree.

### 4.6 Mutation rows of the Kernel file after S3

Tests change; the rows' `old`/`new` do not (no production code changes), so every anchor still matches once. **No declarations are added** (this reverses v1 §4.4's 12; open question E): `"kill": "bound"` only changes an outcome when a run ends in an `EZSDR-INFRA:` marker or a test timeout (`mutate.py` `verdict`), and a Kernel test writes no marker; an expiry panic is an ordinary `killed`. A declaration would be inert and would not be counted by the gate. G16 already has it (its mutant hangs into the timeout). The exit run settles it: any Kernel row that returns `ERROR` (a hang into `TEST_TIMEOUT`) gets its declaration then.

| Row | Effect of S3 | Cost of a killing run |
|---|---|---|
| G05 | killed by the wait expiry (#1) | 10 s |
| G08 | killed by the `found` expiry (#2, #3) | 10 s |
| G36 | killed by round 2's expiry (#4) | 10 s |
| C05 | killed by `returned.recv_timeout(3 s)` (#7) | 3 s |
| G10, G30 | killed by the `wait_for` expiry, now 10 s | 10 s |
| G19, G20 (`kg_06_*`) | unchanged (a 12 s bound over a 5 s budget, a 30 s hang) | as today |
| C89-1-01/02/03/04/05/06 | unchanged (`recv_timeout(10 s)`) | as today |
| C06 | unchanged; killed by the `Running` assertion | — |
| C07 (`kc_36_`) | one of eight tests; its waits are 10 s | — |
| N47S3-28 | unchanged (negative wait, pinned) | — |
| G16 | unchanged (`bound`) | — |

**INFERRED**: the effect of each rewrite on each row (no mutant was applied). Exit check of 3c-1: `remote_mutate.py maintenance,phase7 sim02:8,sim03:8 G05 G08 G36 C05 C06 C07 G10 G30 G19 G20 G16 N47S3-28 C89-1-01 C89-1-02 C89-1-03 C89-1-04 C89-1-05 C89-1-06`: every row `killed` or, for G16, `killed (bound)` (none `ERROR`, none `SURVIVED`).

## 5. The two items earlier stages deferred to stage 3

### 5.1 `rx_start_lead_group_runs_on_the_fake` (`tests/common/measurement.rs:4309`)

**What fails under a delayed test thread (V at `8d96743`, P5').** 20/60/200 ms at `submit`: 9/9 runs fail at `measurement.rs:4323`, `a SHORT candidate`; none, 5 ms submit, 60 ms at `now` and at `advance`: 3/3 pass; with that one assertion deleted, 20/60/200/20 ms submit: 4/4 pass. v1's stage-printing run (V(v1), `e08c8c1`; manual fake, 1 MS/s, apply delay 5 ms, candidates 3/5/10 ms, one run per submit point; each cell is the first trial of the stage, the parentheses the remaining lead in ms) is carried here so that the plan stands alone:

| | restart-same 3 / 5 / 10 ms | restart-retune 3 / 5 / 10 ms |
|---|---|---|
| no delay | SHORT (1.09) / SHORT (4.0) / Success (9.0) | LATE / SHORT (2.0) / Success (7.0) |
| `submit` +5 ms | LATE / SHORT (0.08) / Success (6.0) | LATE / LATE / Success (5.0) |
| `submit` +20, +60, +200 ms | LATE / LATE / Success (5.0) | LATE / LATE / Success (5.0) |

It still holds as the explanation: every stage that is SHORT with no delay turns LATE once the gap between the Stop and the start exceeds its sliver of lead (≈ 1 ms); the LATE and success assertions hold at every injection point tried; the 10 ms candidate stays a Success with a remaining lead of 5.000 ms at the worst injection (`>= 5_000_000` passes at equality; one run per point, I that it never reads 4 999 999).

**Plan.** `restart_trial` is **not changed**; the group's call sites keep their shape.
1. Delete the `a SHORT candidate` assertion (`:4323`) from the Run rehearsal.
2. **Extract the pipeline from the Run's events to the reported stage (M2).** Lines `2423-2455` of `rx_start_lead_group` (decode the two event kinds, associate by origin, annotate, classify, fill the row JSON, `smallest_lead_left_ns`) become

   `fn restart_stage(condition: &str, value: u64, delivered: &[Event], trials: &mut [RxStartTrial], configure_lead: impl Fn(&RxStartTrial) -> Option<i64>) -> (Json, Vec<TrialClass>)`

   with the three event reads inside (`radio.LATE_COMMAND`, `radio.START_LEAD_SHORT`, `requested_tick` and `payload["lead_left_ns"]`), the association (`late.contains(&origin)`, the minimum `lead_left` over the shorts at an equal origin), `rx_start_trial_class`, `candidate_stage_json` and the seven `row[...]` assignments. The caller passes `&manifest.events.delivered` and `|trial| configure_lead_left(&manifest, trial.expected, trial.origin, master_clock_hz)`; the companion fields stay at the call site. The Manifest is needed only for the configure record, which the closure carries, so the pure test needs none (a `Manifest` is a large document with `deny_unknown_fields`; the closure is the lean boundary).
3. **The pure test `a_restart_stage_reports_what_the_events_say`** calls `restart_stage` with synthetic `Event`s whose payloads are made by the producer's own types (`ezsdr_radio::payloads::StartLeadShortPayload`, `LateCommandPayload`, serialised with `serde_json::to_value` as `provider/rx.rs:376-383` and `:495` do) so a payload field rename breaks it. Cases, each a separate stage or trial, no timing:
   - a `START_LEAD_SHORT` at the trial's origin → `trial.short`, class `ProviderFailure`, row `start_lead_short == true`, `late_command == false`, `lead_left_ns` from the closure when it answers and from the warning's payload when it does not;
   - a `LATE_COMMAND` at the origin → `late`, not `short`;
   - **wrong event name**: a `START_LEAD_SHORT` event carrying `kind = radio.LATE_COMMAND` (and the converse) lands in the other field, so reading the wrong name is visible;
   - **undecodable `requested.ticks`**: a short whose payload lacks `requested.ticks` (or holds a string) is not associated → trial neither short nor late;
   - **unassociated trial**: a short at another origin, and a trial with `origin: None`, stay unflagged;
   - two shorts at one origin keep the smaller `lead_left_ns`;
   - the stage's `classes` counts and `smallest_lead_left_ns` follow the trials.
   `rx_start_trial_class` keeps its own pure test (`a_restart_trial_fails_on_a_late_or_short_start_or_an_origin_it_missed`, `:4337`).
4. **Rows**. N47B-19 (`trial.late = late.contains(&origin);` → `= false`) keeps its filter and anchor (the statement is moved whole; `rules.py` must still find it once) and gains the pure test as a second killer through a **second row** with the pure test as its filter (a row's `filter` is a single name). N47B-36 (the sign of `configure_lead_left`) is unaffected. New rows, killed by the pure test and not by the rehearsal: `events_of(...)`'s `"radio.START_LEAD_SHORT"` → `"radio.LATE_COMMAND"` (wrong name); `requested_tick`'s `["ticks"]` → `["tick"]` (undecodable); `*at == origin` → `true` (association); `trial.short = short_event.is_some();` → `= false`; `.min()` → `.max()` over the shorts; `row["start_lead_short"] = json!(trial.short);` → `json!(false)` (reported row). Every `old` is checked by `anchor.py` to match once after rustfmt.
5. Hardware audit (§7): `rx_start_lead_group` is called by `hw_n47_rx_start_lead_group` (`hardware.rs:2485-2492`) and by `hw_measurements_record` through `measurement.rs:1454, :1460`; the extraction is behaviour-preserving there.

**Lost**: the fake no longer shows an end-to-end SHORT stage under load. The Provider-side `START_LEAD_SHORT` emission and its order against `LATE_COMMAND` stay covered by the deterministic lib tests on a manual time (`provider/rx.rs:2744-2754` `short_lead_events`, the order assertion at `:2829`; V by reading the names and the assertion).
**Alternative rejected**: a `Direct`-style rig with a fixed gap (2c §3.4): the trial is shared with the bench, so a different rig stops rehearsing it; and the single fragile assertion does not need it.

### 5.2 The `Gated` real-time waits

`Gated` (`fake.rs:3947`; `Gated::new` builds `FakeDevice::new(config)` at `:3980`, a real clock when the callers pass `FakeConfig::default()`: `fake.rs:4174, :6078, :6116` (`Gated::only`), `:9221, :9242, :9276`). Its waits are `entered` (`:4020`, `recv_timeout(10 s)`), `entered_n` (`:4031`, 20 s) and `ended` (`:4071`, 20 s): expiry = failure, ≥ 100× the pass, so P3 is met; what is missing is the marker, so an expiry reads as a plain assertion failure (a false kill). Plan (as v1): lift **sub-step D of the 1c test plan** (`reviews/n51-1c-test-plan.md` §6 and the table at `:207`, `:227`, rows T10 and N51C-11), with one change so that it adds no marked row: T10's `Gated` is built `Gated::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() })` (nothing drives it, its call never arrives and the wait is a channel timeout, so the clock is irrelevant); with the default `Real` it would be a real-clock test and N51C-11 (filter T10) a 28th marked row. The 3b exit check reads the probe: T10 and N51C-11 write no mark. The sub-step is: `Gated::entered_within(names, bound)` (`entered` calls it with 10 s), `infra(…)` before each of the three panics, T10 `direct_rig_a_gated_wait_that_expires_writes_the_infra_marker` (bound 50 ms, asserts the marker on a fixture's stderr), row **N51C-11** (delete the `infra` call, killer T10). Rows **P245, P442, P467** (their tests are `ur_26_every_stream_s_threads_are_joined_in_order` and `ur_16_a_wedged_later_stream_leaves_the_others_joined`) are declared `bound`: the join bound is the mechanism that detects the wedged threads they exist to show. The wrapper tests that also use `Gated` (`run_a_dropped_live_run_is_judged`, `run_unwinding_…`, `run_any_a_perf_trial_still_fails_on_a_bound`) hang on Manual and stay real. Sub-step 3b.

## 6. Enforcement (§6.5: "その後は，印があって宣言の無い行は broken") — M3, m4

A marked row is one whose **baseline** wrote `EZSDR-CLASS: real-clock`. A declaration is `"kill": "bound"` on the row. **A marked row without a declaration is broken whatever its outcome.**

### 6.1 `mutate.py`

`run_one` already holds `real_clock` (cached with the baseline: `BASELINES[key] = (ok, real_clock)`) and `mutation.get("kill")`. After the outcome is drawn on the normal path:

```
declared = mutation.get("kill") == "bound"
if real_clock and not declared and outcome in ("killed", "SURVIVED"):
    outcome, reason = "ERROR", "real-clock row without a bound declaration"
```
- It applies to `killed` and `SURVIVED` alike; `ERROR`, `COMPILE ERROR`, `NOT FOUND` are already broken and keep their own text; **`BASELINE FAILED` is returned before this line and stays `BASELINE FAILED`** (a baseline failure is not reported twice and is not hidden).
- An `equivalent` row (J58): `main()` rewrites the outcome after `run_one` (`equivalent (survived)` for `SURVIVED`, otherwise `<outcome>, but recorded as equivalent`): an undeclared marked equivalent row now reads `ERROR, but recorded as equivalent` and fails. The rewrite is moved into a small function `settle(mutation, outcome) -> (outcome, failed)` so a self-test can drive it.
- `row_line(...)` gets the field `declared` (`true`/`false`) next to `real_clock`; `why` gets one new value (the docstring lists them).
- The cached-baseline path: the mark is a property of the *baseline key*, the declaration of the *row*, so two rows that share a baseline are judged separately (a declared and an undeclared row on one marked filter: the first is `killed`, the second `ERROR`).

### 6.2 `gate.py` (the accounting, independent of the outcome)

`parse_mutation` reads each row's `real_clock` and `declared` from its `EZSDR-ROW` line (`remote_mutate.row_record` returns the whole object, no change there). A missing `declared` counts as `false` (an older row line). New and renamed report fields: **`real-clock (undeclared)`** = expected rows with `real_clock` and not `declared` and not `BASELINE FAILED` (the baseline failure is already a reason of its own), **`real-clock (declared)`** = expected rows with both. A non-empty `real-clock (undeclared)` adds the reason `mutation: real-clock row(s) without a "kill": "bound" declaration: [...]` whatever the row's outcome: killed, survived, a known survivor from `known-survivors.txt`, `equivalent (survived)`. The old field `real-clock (unclassified)` is removed (its three uses: `gate.py:19-20, :241, :628`, and the docs below). After 3e the gate's expected result is `real-clock (undeclared)` empty and `real-clock (declared)` of 27 rows.

The decision "marked and undeclared" is one predicate on two row-line fields, written in `mutate.py` for the outcome and in `gate.py` for the accounting; the gate does not trust the outcome (a row from a stale image, a hand-merged result) — that is why it is checked twice and not once.

### 6.3 Tests (tool self-tests; stub `cargo_test`, no process)

`mutate.py --self-test`, in the existing stub-queue style (`stub(...)`, `BASELINES.clear()`):
1. marked baseline, undeclared row, mutated run fails plainly → `ERROR`, why `real-clock row without a bound declaration`, row line `declared:false`, `real_clock:true`.
2. marked, undeclared, mutated run **passes** (`SURVIVED`) → `ERROR` (Sol's survived case).
3. marked, declared, killed → `killed`, `declared:true`; marked, declared, survived → `SURVIVED` (unchanged).
4. not marked, undeclared, killed and survived → unchanged.
5. `BASELINE FAILED` with a marked output and no declaration → `BASELINE FAILED` (not rewritten).
6. **cached baseline**: two `run_one` calls on one key, the first declared, the second undeclared, one baseline in the queue → `killed` then `ERROR`, and the baseline ran once (`calls`).
7. `settle`: `equivalent` + `ERROR` → `ERROR, but recorded as equivalent`, failed; `equivalent` + `SURVIVED` → `equivalent (survived)`, passed; no `equivalent` + `killed` → passed.
8. `row_line` carries `declared`.
`mutate.py` and `gate.py` are tools: their self-tests are their check (the existing cells run as the `py`/gate steps and in the mutation image through `remote_mutate.py --e2e`); no mutation row is written for them (V: no row of the six lists has `mutate.py` or `gate.py` as its `file`: `grep -c` gives 0).

`gate.py --self-test` (through `remote_text`, which takes a per-row `real_clock`/`declared`; the helper gets two optional arguments):
9. `killed` + real_clock + undeclared → reason, `real-clock (undeclared) == [k]`.
10. `SURVIVED` known survivor + real_clock + undeclared → reason (the known file does not excuse it).
11. `equivalent (survived)` + real_clock + undeclared → reason.
12. `killed (bound)` + real_clock + declared → no reason, listed under `real-clock (declared)`.
13. `BASELINE FAILED` + real_clock + undeclared → only the baseline reason; not in `real-clock (undeclared)`.
14. a row line without a `declared` key + real_clock → undeclared; a row with no real_clock → no effect.

### 6.4 J58 (m4) and the other 26

J58 is `equivalent`; it is killed by nothing and gets no killer search. Its filter `provider::` selects the 136 provider lib tests including the three real-by-design tests (`ur_22_t3` ×2, `ur_75`), so it is a **marked** row; under 6.1 it must be declared. It gets `"kill": "bound"` and its `equivalent` text gains one clause: *the declaration only classifies the filter (it selects tests whose subject is a real deadline); nothing is killed*. `mutate.py`'s docstring says so (a declaration on an `equivalent` row classes its filter). Rejected: narrowing the filter to a module without a real test: that cuts the empirical evidence the equivalence stands on (the whole provider suite passes with the mutant) to keep a mark off, and chooses a module by guess without a killer to point at; and a second declaration token (`"kill": "none"`), which adds a vocabulary for one row.

The other 26 marked rows each select their real test by name (V: the union of Appendix A and B's row ids against the kept-test list, §3.5), are killed or survive as today, and are declared in 3e; the reasons are in the stage's disposition record (`plan/phase8/reviews/n51-3-*`), not in a new field (the row's `name` is the only prose a row has).

### 6.5 Landing order

**3e**, with the 27 declarations in the same commit. Landing it earlier makes a full `gate.py --mutation all` red until the declarations exist; nothing else is affected (an `ERROR` can only come from a mutation run). `"exact": true` rows are judged like any row (the serial re-run of a timing-marked test is `exact`, but it is a re-run inside `judge`, not a row; the marked flag belongs to the baseline of the row's own filter).

## 7. Hardware audit table (AGENTS §9: shared trials whose expectation could change)

None of S2, S3 or §5 changes a rule that a hardware test computes its expectation from, and none edits `hardware.rs`. Listed because a shared trial or tool is *called* by a test whose clock changes or whose code is extracted:

| Test | Shared code it calls | Called by `hardware.rs` | Expectation | After |
|---|---|---|---|---|
| `rx_start_lead_group_runs_on_the_fake` (3d) | `rx_start_lead_group`, `restart_trial`, the new `restart_stage` | `hw_n47_rx_start_lead_group` (`:2485-2492`), `hw_measurements_record` (`measurement.rs:1454, :1460`) | LATE and SHORT per candidate, remaining lead recorded, smallest candidate that passes | unchanged (a pure extraction); the fake loses only its SHORT assertion, which the new pure test replaces with a stronger one |
| `common::measurement::multichannel::tests` ×4 (3b) | `run_stage`, duplex-pair, guard, PLL-retry latch | `hw_measurements_record` | structure of the record, latch behaviour, no time term | still right (P4' 3/3 on Manual) |
| `h4_*` ×2 (3b) | the H4 load/headroom measurement | `hw_measurements_record` | partial record on underflow, converted time | still right (P4' 3/3) |
| `common::probe_device::tests` ×2 (3b) | `ProbeDevice` | the raw probes | counts and row limits | still right (P4' 3/3) |
| `ur_23_g2/g3`, `ur_05_*` ×2, `ur_12_*` (3b) | wrappers / Provider unit-level | none | forwarded calls, spec order, grid | still right (P4' 3/3) |
| `bench1115_*` ×9, `iii_restart_fill_probe_…` (kept real) | `common::bench1115`, `iii` probes | `hw_bench1115_*` | unchanged | unchanged: not touched |
| Kernel `device_paced.rs` (3c) | Kernel doubles only | none | n/a | n/a |

"Cannot tell" rows: none. The hardware lines that the moved tests' shared code feeds are not re-run (no bench); nothing in them changes.

## 8. Sub-steps (each leaves the tree green; estimates are added + changed lines)

| Step | Content | ~lines | Exit check (beyond `cargo test`, `fmt`, `clippy`, `rules.py`) |
|---|---|---|---|
| **3a** lib | `test_support.rs` three helpers force Manual; 14 per-site edits; the six real-by-design tests keep their explicit Real; the redundant explicit `Manual` at `core.rs:1956` goes (a leftover) | 60 | 20 serial runs of the lib binary + 3 × 8 parallel, 0 failures; `remote_mutate.py maintenance,phase7 sim02:8,sim03:8` over the 155 lib ids, no row `ERROR` (candidates §3.1) |
| **3b** fake / x3xx / `Gated` | 13 `fake` tests and 8 `x3xx` tests to Manual (about 25 tokens); sub-step D of 1c (markers, T10, N51C-11) | 220 | each moved test 20 serial + 3 × 8 parallel; the 21 + 2 relieved rows, P245/P442/P467 and N51C-11 through `remote_mutate.py` |
| **3c-1** Kernel rewrite | `marking_on`/`release_one`, `WallState.waiting` and `fired_before_schedule_returned` in `paced.rs`, `Probe::wait_until`; the rewrite of §4.1–§4.2 (`woken_rounds`, #1, #5–#11, the siblings) | 450 | the Kernel suite 20× serial and under 3 × 8 parallel; the 18 Kernel rows of §4.6 all `killed`; `woken_rounds` 20× on the unmutated tree first |
| **3c-2** Kernel scan | R1–R3, `KERNEL_WALL_ALLOWED`, the 10 fixtures, `governance.rs` walks the Kernel file; P3a (negative waits) and the Q5 correction in note 51 | 300 | `governance` binary; the scan's own rows through `remote_mutate`; `rules.py` |
| **3d** `rx_start_lead` | drop one assertion, extract `restart_stage`, `a_restart_stage_reports_what_the_events_say`, seven new rows and N47B-19's second row | 200 | the rows (N47B-19, -36, the new seven) through `remote_mutate.py`; `hw-build` (compiles the shared code); the rehearsal 20 serial + the 12 injection points (P5' as a script), expecting 12/12 green |
| **3e** declarations + enforcement | `"kill": "bound"` on the 27 rows of §3.5 (J58's `equivalent` text); `mutate.py` rule, `declared` in the row line, `settle`, the 8 self-test cells; `gate.py` fields and the 6 cells; docstrings, `tests/README.md`, note 51 §6.5/§8, `51-tables.md` §6 | 300 | `python3 plan/maintenance/tools/mutate.py --self-test`, `gate.py --self-test`, `remote_mutate.py --e2e`, then the orchestrator's gate with `--mutation all` (expected: `real-clock (undeclared)` empty, `real-clock (declared)` = 27) |

Total about 1 530; the largest is 3c-1 (~450). 3c-1 before 3c-2 (the scan is clean only after the rewrite); 3e last. 3a, 3b and 3d are independent of 3c.

Risk class (AGENTS §9 step 1): none changes a queue, a deadline, an identity, a lifecycle or an FFI owner in production code. The nearest are the test double `WallAuthority`'s `waiting` flag (test code), 3d (shared trial code, a pure extraction) and 3e (gate semantics: a code review by Sol, as for stage M). No test plan before code is needed on that reading; 3c-2's scan and 3e's rule carry their own string and self-tests, and M1's shape is described here down to the helper.

## 9. Decisions (settled: plan-decisions 1–8) and what remains open

Decided (by the orchestrator unless stated; none is the owner's under AGENTS §9 "What the owner decides": no API meaning, refusal, Manifest, KNOWN_LIMITATIONS entry, default or limit changes):
1. `FakeConfig::default()` stays Real (§3.2).
2. The ten probe-tool rehearsals keep Real and are declared (9 rows, §3.3).
3. **P3a**: a negative wait (the expectation is that something does *not* happen within a wall interval; a slow host only weakens it) is allowed when pinned with a reason; added to note 51's P3 in 3c-2.
4. J58 stays `equivalent`; its filter is classified and declared (§6.4) — replaces plan-decisions 4.
5. `Probe::wait_until` in `tests/support/run_doubles.rs`: yes (§4.4).
6. The §4.2 siblings are included in 3c.
7. The SHORT assertion is dropped; a pure test covers SHORT (§5.1, M2's shape).
8. Enforcement lives in `mutate.py`'s `run_one` (the outcome) and again in `gate.py`'s accounting (M3).

Open, with a recommendation — the first three are **decisions this plan proposes beyond the six findings; the orchestrator decides** (none is the owner's: all internal to tests and tools):
- **E. No `"kill": "bound"` on Kernel rows (reverses v1 §4.4's 12 declarations).** Recommend: reverse. A declaration only changes an outcome when a run ends in an `EZSDR-INFRA:` marker or a test timeout (`mutate.py` `verdict`); a Kernel test writes no marker and the expiry panics are ordinary `killed`; the gate counts only `killed (bound)` rows, so the 12 would be inert text. Declare a Kernel row only if the exit run shows it `ERROR`.
- **F. The observed-park shape for the wake tests and the `waiting` predicate with a 10 s cap in the double (§4.5).** Recommend: adopt; it is the answer to M1 and the only route that makes G08/G36 fail when the wake is removed. It changes shared test support (`support/paced.rs`, `run_doubles.rs`), about 70 lines.
- **G. One bound rule, `Wall::from_secs(n ≥ 3)` (§4.3).** Recommend: adopt (m5). The cost is that rows 7 and 8 keep a 3 s bound against KC-21a's 5 s budget (1.67×, not 2×).
- **A. A declaration on an `equivalent` row (J58).** Recommend: accept `"kill": "bound"` there, documented as "classes the filter" (§6.4). V: no rule refuses the pair (`grep -n 'kill\|equivalent'` over `rules.py`, `anchor.py`, `remote/*.py`, `gate.py` finds none; `mutate.py`'s `run_one` only checks `kill in (None, "bound")`; `main()` handles `equivalent` after `run_one`). The alternative is a one-row exemption keyed on `equivalent` in the enforcement; rejected because Sol's finding lists equivalent rows among the cases that must be broken.
- **B. The pin does not cover the judgment after an approved sleep (§4.3).** Recommend: accept; a table row is a review event. Pinning the next statement too is brittle (any edit of the assertion text needs a table edit) and was not asked.
- **C. `#9`/`#10` keep the P3 bound form (horizon 10 s, bound 5 s) instead of the observed-park shape.** Recommend: accept; their subject is the result, not a wake; their rows do not exist. If the orchestrator wants the same kill power as the wake tests for a future row, `losing_device_on(gate)` is already there.
- **D. `rm_26_the_timeline_against_uhd_rx` and the other per-site lib tests** were measured by the default-flip run (P3), not by a run with only the per-site edits; 3a's load runs measure the real edit. Low risk.

## 10. INFERRED, not verified

- **No mutation row was run.** Every "this row is killed by …" (§4.6, §5.1 item 4, §3.1 item 3) is read from `old`/`new` and the test body; 3c-1's, 3d's and 3a's exit runs are where it becomes V.
- **The observed-park helper has not been run.** That `wait_parked_until(horizon tick)` succeeds for a `wait_for` call (not only for `advance_to`, which the existing KC-36 test uses) is read from `stepping.rs:394-413` and `:434-446` (both go through `advance_to_with`); 3c-1 runs it 20× first. The `waiting` predicate (§4.5.5) is a design; the existing 20 ms window and its users (`firing_before_schedule_returns`, one caller) were read in full.
- **The shapes of #6 and #7 (§4.1) have not been run** (the worker/park/release mechanics of §4.5 are the same, with `losing_device_on(gate)`; #7 gates the loss on `probe.wait_for("radio:took:UpdateParameter")`). For #6 it is also INFERRED that nothing earlier than the 3 600 s horizon is pending when the loop parks (`spec_one()` has no `schedule`, the Lease is attached, `kc_36_a_disconnect_signal_ends_a_waiting_advance` observes the same park on a `session(...)` Run at the same horizon); 3c-1's first run settles it, else the test parks on the tick `parked_until` reports.
- That the microsecond window between "the Provider took the Action" and the dispatcher's KC-21a wait (§4.1 #7) can only hide C05's mutant: the loss comes after the take, so a mutant that reached `end.is_some()` before waiting would return at once and survive; never a false kill on the baseline.
- That the 10 ms rehearsal candidate never reads below 5 ms under a delay: one run per point in v1; at `8d96743` the assertion is in the part of the test that passes 4/4 with the SHORT line deleted, not individually varied.
- The classification of the 133 moved lib tests rests on one 668/668 lib run with the helpers forced Manual and on v1's 20 serial + 24 parallel runs; not on reading each for real-time quantities (§3.1 item 2).
- That the 7 + 10 kept `fake` tests would pass on Manual (V(v1)) and the 11 that hang (V(v1), 101/134) still behave so at `8d96743`: not re-run; the 28 stay real, so a change would only make a declaration unnecessary.
- The two probe failures in P1 are environmental (`nohup`) and a marked timing failure; neither is a base defect, but they were not re-run in a clean shell.
- `kc_36_` (C07) has no wall upper bound (V in v1 by reading the grep; at `8d96743` the Kernel file is byte-identical, so the new R1 scan over `.elapsed()`/`waited` hits is the final check: 3c-2's "the real tree is clean" test).

## Appendix A — the 139 `lib` tests (module, test, [rows it marks]) at `8d96743`

Treatment: all move except the six of §3.1 (`authority::tests::ur_07_the_host_timeline…`, `ur_08_…`, `device::tests::ur_33_a_real_fake…`, `provider::core::tests::ur_22_t3_ack_read…`, `…::ur_22_t3_flush_reports…`, `provider::core::t3::ur_75_…`). The 14 per-site tests are listed in §3.1. New against v1: `provider::rx::tests::ur_26_a_cut_after_the_origin_of_a_silent_start_stops_at_once` and `…::ur_26_an_abort_stops_a_queued_start_when_uhd_rx_reads_the_plan` [N67F-04]; gone: `…::ur_26_a_stop_before_a_queued_start_stops_after_its_origin`, `…::ur_26_an_abort_stops_a_queued_start_at_shutdown`.

**`authority::tests`** (2): `ur_07_the_host_timeline_is_judged_on_real_time_on_the_real_clock` [J299], `ur_08_the_manual_clock_s_relations_hold_only_at_the_instant_measured` [J290]

**`device::tests`** (1): `ur_33_a_real_fake_s_time_counts_from_its_own_creation` [J316]

**`provider::control::differential`** (1): `rm_26_the_timeline_against_uhd_control` [J58]

**`provider::control::tests`** (25): `rm_14_the_handed_burst_keeps_its_requested_at` [J58,T36], `ur_14_each_action_is_booked_before_the_next_recv` [J58,U13], `ur_21_a_burst_before_its_clock_s_origin_is_late` [J58,U46], `ur_21_a_burst_before_start_up_is_refused` [J58], `ur_21_a_burst_dropped_at_booking_is_not_booked` [J58,N47S4-44,N47S5-28,U14], `ur_21_a_value_after_the_hand_over_and_before_the_end_is_refused` [J58,N47S4-08], `ur_21_a_value_is_judged_by_a_booking_time_move` [J58,N47S4-08b], `ur_21_a_value_while_a_burst_is_booked_is_refused_with_its_action_id` [J58,N47S4-29], `ur_21_an_untimed_burst_before_the_clock_s_origin_is_placed` [J58], `ur_21_an_untimed_repeat_after_a_stop_starts_at_the_bound_or_its_ordinary_start` [J58,C83-2,N51-2C-49], `ur_21_an_untimed_send_is_placed_after_its_delivery` [J58,N47S5-121,N47S5-122], `ur_21_production_booking_preserves_the_original_repeat_length` [J58], `ur_21_s16_timed_drop_in_the_stop_region_is_refused` [J58,N47S5-127], `ur_21_s16_timed_send_asap_in_the_stop_region_moves_with_time_error` [J58,P650,N47S5-126], `ur_21_the_receipt_site_reserves_the_moved_start_before_any_hand_over` [J58,J315], `ur_23_a_provider_stop_ends_a_repeat_at_its_in_flight_window` [J58,N47S4-43], `ur_23_a_value_at_the_exclusive_end_of_a_burst_is_accepted` [J58,N47S4-45], `ur_23_a_value_is_taken_only_after_a_stopped_repeat_s_in_flight_window` [J58,N47S4-17c], `ur_25_a_start_rx_takes_the_line_lock_once_for_its_instant_and_its_booking` [J58,P357], `ur_25_a_transmit_stream_has_one_clock_for_the_run` [J58,N47S4-35,N47S4-40], `ur_25_the_last_value_before_a_burst_is_the_one_it_plays` [J58,N47S4-21], `ur_26_a_stop_ends_every_line_once` [J58,P243], `ur_26_an_orderly_stop_cuts_at_its_instant` [J58], `ur_30_a_transmit_stop_records_the_bound_its_bookings_end_at` [J58,C83-1], `ur_34_a_transmit_stop_books_the_wait_exit_and_queues_the_owner_stop` [J58,P503]

**`provider::core::t3`** (1): `ur_75_a_report_read_after_attempt_1_s_deadline_stays_for_the_report_owner` [J58]

**`provider::core::tests`** (7): `rx_chain_reuse_guard_rejects_open_and_close_of_an_active_chain` [J58,P1255,P1280], `ur_22_t3_ack_read_returned_after_deadline_is_not_a_flush_success` [J58,P1190], `ur_22_t3_flush_reports_after_the_deadline_stay_the_runs` [J58], `ur_23_a_transmit_clock_cut_at_its_origin_ends_at_the_booking` [J58,P1398,P1399], `ur_26_a_finished_stream_clears_only_its_own_floor` [J58,P558], `ur_26_an_end_takes_delivered_from_each_segment_s_own_stream` [J58,P445,P555], `ur_34_a_rate_refusal_leaves_the_stream_record_unknown` [J58,P514]

**`provider::rx::differential`** (1): `rm_26_the_timeline_against_uhd_rx` [J52,J58,P1058,N67F-07,U32,U40,R13,R14]

**`provider::rx::tests`** (46): `rehearsal_b5_overflow` [J58], `rx_chain_abandon_unowned_records_cleanup_state` [J58,P1264,P1288], `ur_13_a_failed_configure_is_configured_again_without_reopen` [J58], `ur_13_a_failed_open_by_the_owner_is_opened_again` [J58,P360,P369,P376], `ur_13_a_partly_applied_set_is_configured_whole_without_reopen` [J58], `ur_17_a_block_s_channels_are_planar_cf32` [J58,T43,T45], `ur_17_a_cut_is_never_before_what_was_delivered` [J13,J23,J58], `ur_17_a_missed_start_and_overrun_share_the_block_summary` [J58], `ur_17_a_missed_start_without_another_report_is_not_an_overflow` [J58,U49], `ur_17_a_request_ends_at_the_pending_cut` [J58,B05], `ur_17_an_ended_segment_prunes_the_items_before_it` [J49,J51,J58], `ur_17_an_overlap_trimmed_to_nothing_is_dropped` [J58], `ur_18_a_report_on_a_gapless_block_is_consumed_without_an_event` [J58,U48], `ur_19_a_later_receive_stream_s_overflow_names_its_node` [J58,P443], `ur_19_reports_before_one_block_keep_each_kind` [J58,J122,U24,U25,U26,U47], `ur_24_an_abort_keeps_overflow_and_later_samples_off_the_link` [J58], `ur_25_a_booking_the_timeline_cannot_represent_is_refused_with_the_kind_of_its_action` [J58,N47S3-20], `ur_25_a_refused_receive_owner_books_and_gives_back_in_one_line_section` [J58,P248,P484], `ur_25_a_refused_segment_is_booked_by_uhd_rx` [J14,J15,J58,P250], `ur_25_a_refused_start_is_labelled_as_a_command_with_the_start_s_id` [J58,N47S3-22], `ur_25_a_second_change_follows_the_first` [J09,J58], `ur_25_a_silent_stream_past_its_cut_records_its_stop` [J58,B02], `ur_25_a_stream_silent_before_e1_switches_before_e2` [J58,P1269,B01,B12], `ur_25_largest_aligned_cut_is_safe_in_rx_owner` [J58], `ur_26_a_cut_after_the_origin_of_a_silent_start_stops_at_once` [J58], `ur_26_a_late_start_after_abort_does_not_stop_again` [J58], `ur_26_a_late_start_during_orderly_stop_is_recorded_once` [J58], `ur_26_a_receive_start_of_no_planned_segment` [J58,P554], `ur_26_a_receive_start_queued_by_the_stop_ends_its_owner_without_detaching` [J58,P479], `ur_26_a_stopped_stream_keeps_its_end` [G03,G04,G58,J45,J58], `ur_26_a_stream_s_delivered_is_cleared_alone` [J58,P470,P559], `ur_26_an_abort_stops_a_queued_start_when_uhd_rx_reads_the_plan` [J58,N67F-04], `ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts` [J54,J58], `ur_26_no_segment_begins_once_provider_stop_has_begun` [J58], `ur_26_uhd_rx_aborts_when_its_provider_is_gone` [G60,J55,J58,R08], `ur_26_uhd_rx_follows_the_stop_it_reads` [J58], `ur_29_a_loss_after_a_configure_starts_no_receive_segment` [J53,J58], `ur_29_a_loss_between_two_channels_of_a_configure_applies_no_more` [J53,J58,P391], `ur_29_a_loss_ends_every_stream_once` [J53,J58,P247], `ur_29_a_loss_ends_the_stream_at_its_instant` [J46,J47,J53,J58,P258], `ur_29_a_loss_found_after_uhd_rx_read_the_plan_ends_the_stream` [J53,J58], `ur_29_a_loss_takes_effect_before_a_change_at_its_instant` [J53,J58,J60,P257], `ur_29_a_lost_receive_owner_configure_releases_stream_lock_before_reporting` [J53,J58,P521], `ur_29_a_silent_stream_ends_at_a_cut_booked_during_its_wait` [J53,J58], `ur_29_an_owner_does_not_configure_a_segment_a_stop_dropped_from_the_plan` [J58,P513], `ur_36_owner_configure_takes_each_command_s_own_snapshot` [J58]

**`provider::tests`** (21): `c2_a_boundary_send_whose_worker_panicked_is_uncertain_in_the_manifest` [J58,C2-27], `rx_chain_cleanup_close_requires_an_idle_chain` [J58,P1286], `rx_chain_prepare_open_requires_an_idle_chain` [J58,P1283], `ur_13_arm_radiates_nothing` [J58,T42,P1173], `ur_15_every_declared_stream_starts_its_own_thread` [J58,P441,P471], `ur_16_a_uhd_control_panic_after_a_uhd_tx_panic_is_one_device_lost` [J58,N92-2,N92-3,N92-7,N92-8], `ur_16_a_uhd_control_panic_after_it_returns_does_not_write_a_second_plan_row` [J58,N92-2,N92-3,N92-7,N92-7b,N92-8], `ur_16_a_uhd_control_panic_after_the_device_is_already_lost_is_recorded_once` [J58,N92-2,N92-3,N92-7,N92-8], `ur_16_a_uhd_control_panic_after_the_end_is_booked_is_one_device_lost_and_stop_still_joins` [J58,N92-2,N92-3,N92-7,N92-8], `ur_16_a_uhd_control_panic_before_a_uhd_tx_panic_is_one_device_lost` [J58,N92-2,N92-3,N92-7,N92-8], `ur_16_a_uhd_control_panic_between_two_actions_books_the_first_and_leaves_the_second` [J58,N92-2,N92-3,N92-7,N92-8], `ur_16_a_uhd_control_panic_holding_the_line_lock_does_not_deadlock_device_lost` [J58,N92-2,N92-3,N92-4,N92-7,N92-8], `ur_16_a_uhd_control_panic_simultaneous_with_a_uhd_tx_panic_is_one_device_lost` [J58,N92-2,N92-3,N92-7,N92-8], `ur_16_a_uhd_control_panic_with_no_stream_running_is_a_lost_device` [J58,N92-2,N92-3,N92-7,N92-8], `ur_16_join_waits_idle_on_the_manual_clock_so_a_thread_that_needs_the_clock_ends` [J58,N51A-13], `ur_16_the_join_bound_cuts_in_real_time_while_the_manual_clock_is_held` [J58,N51A-14,N51A-15], `ur_25_a_refused_check_at_arm_leaves_the_set_unknown` [J58,P378], `ur_26_a_provider_stop_books_one_end_per_line` [J58,N92-6], `ur_26_the_control_thread_is_joined_before_every_stream` [J58,P466], `ur_36_a_failed_open_moves_no_generation` [J58], `ur_36_a_successful_open_moves_the_generation_on_even_when_its_check_refuses` [J58,P345,P380]

**`provider::tx::tests`** (32): `sc_25_the_underflow_event_carries_its_own_instant` [J58,T06,T32,T33], `ur_21_first_dispatch_preserves_an_on_time_start` [J58], `ur_21_first_dispatch_rechecks_all_late_policies` [J58,P644,N47S5-72,U15], `ur_21_first_dispatch_refuses_a_moved_start_collision` [J58], `ur_22_an_expired_timeline_is_not_resumed_before_its_report` [J58], `ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts` [J58,P1205,P1206], `ur_22_real_starvation_does_not_extend_the_stop_window` [J58], `ur_22_tx_stop_preserves_recovery_for_later_bursts` [J58,P1199], `ur_22_uncertain_reopen_waits_for_owner_claim_and_stops_on_stop_or_loss` [J58,P1193,P1194,E2B-20], `ur_22_underflow_and_loss_end_the_damaged_burst` [J58], `ur_22_underflow_close_failure_reopens_before_the_next_payload` [J58,P1198,P1208,P1209,N-11,E1-U14,R07], `ur_25_an_end_booked_after_the_first_read_cancels_the_held_burst_at_the_origin` [J58,N47S4-37], `ur_26_shutdown_after_loss_cancels_held_bursts_record_only` [J58,P570], `ur_26_stop_cancels_the_held_burst_record_only` [J58], `ur_28_a_later_stream_s_reopen_discards_only_its_correlations` [J58,P379,P454,P483], `ur_28_a_report_is_correlated_by_its_position_in_the_streamer` [J58,P350], `ur_28_a_streamer_the_record_holds_keeps_its_correlations` [J58,P373], `ur_28_a_ticked_ack_does_not_retire_a_future_burst` [J58,P1188], `ur_28_a_tickless_burst_ack_retires_only_the_first_pending_target` [J58,P1189], `ur_28_ambiguous_time_errors_leave_each_channel_pending` [J58,P1180], `ur_28_an_ack_at_a_bursts_end_plus_one_tick_keeps_the_burst_at_the_next_sample` [J58,N48S1-01], `ur_28_delayed_time_error_uses_its_original_clock` [J58], `ur_28_grouped_channels_retire_per_position` [J58], `ur_28_late_burst_error_and_later_ack_use_report_ticks` [J58], `ur_28_per_channel_ack_preserves_the_next_burst` [J58], `ur_28_split_streams_retire_their_own_pending` [J58], `ur_28_streamer_generation_clears_late_report_deduplication` [J58,P1202], `ur_28_tickless_time_errors_keep_the_first_pending_target` [J58], `ur_28_time_error_after_abandoned_burst_does_not_claim_the_next_burst` [J58,P1184], `ur_29_eob_device_errors_are_escalated` [J58], `ur_29_eob_loss_at_the_clock_end_stops_device_calls` [J58], `ur_29_eob_loss_from_report_prevents_next_burst_send` [J58]

**`provider::tx::tests::e2b`** (2): `e2b_r1_a_first_send_of_a_generation_the_device_does_not_hold_panics_in_the_test_build` [J58,E2B-12], `e2b_r1b_a_first_send_on_a_lost_device_does_not_need_the_generation` [J58]

## Appendix B — the 41 `fake` tests at `8d96743` (rows marked, treatment)

| test | rows marked | treatment |
|---|---|---|
| `bench1115_a_failed_receive_call_ends_a_pn_window_with_its_error` | B1115-129 | KEEP Real, declared (decision 2) |
| `bench1115_open_flush_sweep_rehearses_on_the_fake_and_a_dropped_ack_fails` | B1115-83 | KEEP Real, declared (decision 2) |
| `bench1115_pn_sweep_rehearses_on_the_fake` | B1115-124, B1115-125 | KEEP Real, declared (decision 2) |
| `bench1115_routing_and_late_probe_rehearses_on_the_fake` | — | KEEP Real, declared (decision 2) |
| `bench1115_skip_probe_records_a_receive_overflow_and_its_gap` | — | KEEP Real, declared (decision 2) |
| `bench1115_t3_probes_rehearse_on_the_fake` | — | KEEP Real, declared (decision 2) |
| `bench1115_the_cleanup_drain_ends_at_a_failure_and_keeps_draining_past_an_overflow` | B1115-126, B1115-127, B1115-128 | KEEP Real, declared (decision 2) |
| `bench1115_zeros_sweep_refuses_a_gain_above_the_bench_maximum_and_restores_0_db` | B1115-30, B1115-31 | KEEP Real, declared (decision 2) |
| `bench1115_zeros_sweep_rehearses_on_the_fake` | — | KEEP Real, declared (decision 2) |
| `common::measurement::multichannel::tests::the_duplex_pair_is_not_run_without_both_values_and_says_so` | — | MOVE to Manual |
| `common::measurement::multichannel::tests::the_duplex_pair_runs_each_direction_on_the_streams_that_held_its_value` | P1010, P1019 | MOVE to Manual |
| `common::measurement::multichannel::tests::the_guard_latches_on_apply_errors_timeouts_and_losses_only` | P929, P930 | MOVE to Manual |
| `common::measurement::multichannel::tests::the_pll_retries_used_up_latch_the_record_and_nothing_more_is_opened` | P1009, P1011, P1012, P1013 | MOVE to Manual |
| `common::probe_device::tests::probe_rows_stop_at_the_limit_and_count_dropped_rows` | P1153 | MOVE to Manual |
| `common::probe_device::tests::tx_eob_count_tracks_recorded_eob_sends` | P861 | MOVE to Manual |
| `h4_a_tx_underflow_makes_the_load_record_partial_and_converts_its_time` | P1149, P1150, P1156, P1157 | MOVE to Manual |
| `h4_fake_session_records_tx_headroom_and_load_with_no_run_time_update` | — | MOVE to Manual |
| `iii_restart_fill_probe_rehearses_on_the_fake` | — | KEEP Real, declared (decision 2) |
| `run_a_bound_is_marked_by_advance_to` | N51-2A-17 | KEEP Real, hangs on Manual (V) |
| `run_a_bound_is_marked_by_run_until_end` | — | KEEP Real, hangs on Manual (V) |
| `run_a_bound_is_marked_by_wait_for` | — | KEEP Real, hangs on Manual (V) |
| `run_a_dropped_live_run_is_judged` | N51-2A-14 | KEEP Real, hangs on Manual (V) |
| `run_any_a_perf_trial_still_fails_on_a_bound` | — | KEEP Real, hangs on Manual (V) |
| `run_unwinding_marks_a_bound_and_never_panics_again` | N51-2A-15 | KEEP Real, hangs on Manual (V) |
| `ur_05_a_basic_front_end_s_number_is_checked` | P288 | MOVE to Manual |
| `ur_05_the_spec_is_set_before_the_channel_map_is_read` | — | MOVE to Manual |
| `ur_07_cancel_wakes_a_waiting_next_wakeup` | — | KEEP Real, the real clock is the subject |
| `ur_07_no_device_read_follows_the_authority_s_drop` | B24 | KEEP Real, the real clock is the subject |
| `ur_07_now_is_monotonic_across_a_reanchor` | — | KEEP Real, the real clock is the subject |
| `ur_07_schedule_wakes_a_waiting_next_wakeup` | — | KEEP Real, the real clock is the subject |
| `ur_07_the_last_device_handle_is_not_dropped_on_uhd_clock` | B23 | KEEP Real, the real clock is the subject |
| `ur_12_every_advertised_rate_is_accepted` | P14, R11 | MOVE to Manual |
| `ur_16_a_wedged_later_stream_leaves_the_others_joined` | P467 | KEEP Real, hangs on Manual (V) |
| `ur_16_a_wedged_thread_does_not_wedge_stop` | P274, P275, P277 | KEEP Real, hangs on Manual (V) |
| `ur_16_cleanup_keeps_the_streamer_of_a_detached_thread` | U42 | KEEP Real, hangs on Manual (V) |
| `ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one` | J295 | KEEP Real, the real clock is the subject |
| `ur_23_g2_every_wrapper_forwards_the_priority_call` | P1120, P1121, P1122 | MOVE to Manual |
| `ur_23_g3_a_latched_guard_refuses_the_priority_call` | P1123 | MOVE to Manual |
| `ur_26_every_stream_s_threads_are_joined_in_order` | P245, P442 | KEEP Real, hangs on Manual (V) |
| `ur_26_the_sending_threads_join_before_the_receiving_ones` | — | KEEP Real, hangs on Manual (V) |
| `ur_33_a_real_fake_s_time_starts_at_its_own_creation` | — | KEEP Real, the real clock is the subject |

## Appendix C — the 8 `x3xx` tests (all move; rows P17, P18)

- `d3_a_cbx_layout_is_not_coherent_on_either_provider`
- `d3_a_layout_without_receive_channels_requests_none_on_both_providers`
- `d3_a_named_selector_gives_each_provider_its_unnamed_description`
- `d3_the_layout_decides_the_channels_on_both_providers`
- `d3_the_layout_is_a_node_id_of_neither_provider`
- `d3_the_two_providers_default_every_channel_to_no_stream`
- `d3_the_two_providers_describe_one_layout_alike`
- `d3_the_two_providers_refuse_a_transmit_rate_below_one_msps_alike`
