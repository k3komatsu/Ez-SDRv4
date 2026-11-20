# 8b-1 — one shared 1 s join bound per stop stage (D-6) — lane 2

Worktree `/home/komatsu/local_work/Ez-SDRv4-n111`, branch `work/8b1` (from `main` e178a2e5). Do not commit, stash, push, reset or switch branch. Never touch `/home/komatsu/local_work/Ez-SDRv4` (the main tree, another lane). Work dir ``.

Design (decided, owner 2026-11-20): `plan/phase8b/01-device-bundle.md` §3.5 and D-6 in `01-device-bundle-decisions.md`; staging §13 (this is 8b-1, N = 1 only; no `Bundle`, no binding change).

**Risk: risky** (a deadline and a lifecycle: `Provider::stop`'s joins). AGENTS §9 steps 2–3 apply: **phase A — write the test plan only, no code**; Sol reviews it; phase B (a later brief) implements.

## The change
Today `UhdRadio::stop` (`crates/ezsdr-radio-uhd/src/provider/mod.rs` ~722–764) joins uhd-control, then sends `TxCmd::Shutdown` to every uhd-tx and joins them one by one, then the same for uhd-rx; `join` (~419–439) gives **each thread a fresh 1 s** (`JOIN_BOUND`), so a stuck stop takes 1 + n_tx + n_rx s (debt #20, `plan/phase8/52-phase8-end-review.md` row 20). After 8b-1 the bound is **one 1 s of real time per stage**: stage 1 uhd-control, stage 2 every `uhd-tx-s<k>` (all Shutdowns sent first, as today), stage 3 every `uhd-rx-s<k>`. Worst case about 3 s plus polling whatever the stream count. Unchanged: the stage order (transmit before receive, RM-16), the idle wait on the manual clock (the joiner is idle while it waits, `host.wait`), the `rejected` note text `did not join within 1 s; left detached` and its thread name, `detached` ⇒ `cleanup` skips `close_streams` and records the leak (UR-16), a second `stop` does nothing, uhd-control's plan record after its join (UR-30), the `stop` timing row. The `start` failure path that calls `stop(Abort)` and `cleanup`'s own `stop` take the same staged joins.

## Sequences (expected results)
Rig: the unit-level `running_radio()` (mod.rs, two streams per direction, threads spawned by the test) and/or `wedged()` in `tests/fake.rs`; a thread "stuck" = held inside a call the test releases at the end.
1. Two uhd-tx stuck → stage 2 ends at one deadline: both named in `rejected`, both in `detached`; the stage takes ≈ 1 s, not ≈ 2 s; stage 3 still runs (rx get Shutdown and join).
2. One uhd-tx stuck, the other ends promptly → the prompt one is joined (not named), the stuck one detached.
3. Deadlines do not carry across stages: uhd-control stuck (detached at 1 s), then a uhd-tx that needs ~0.5 s after its Shutdown is still joined (stage 2 has its own 1 s).
4. A thread that ends after its stage's deadline is detached even though a per-thread bound would have joined it: tx0 stuck, tx1 ends 1.3 s after the stage starts → both detached (the old code joins tx1, whose fresh 1 s began when tx0 was cut at 1 s).
5. All three stages stuck (control, 2 tx, 2 rx) → `stop` returns in ≈ 3 s (not 5 s); all five named; `cleanup` calls no `close_streams` and records the leak.
6. Order: no `RxCmd::Shutdown` is sent before stage 2 has ended (joined or detached) — the existing `ur_26_the_sending_threads_join_before_the_receiving_ones` keeps holding.
7. Manual clock: a stage of two threads that each need the manual clock to advance to end (e.g. the shutdown's 100 ms `BURST_ACK` window on the device host clock) joins both — the joiner is idle while it waits (extends `ur_16_join_waits_idle_on_the_manual_clock_so_a_thread_that_needs_the_clock_ends`); the real-time bound still cuts while the clock is held (extends `ur_16_the_join_bound_cuts_in_real_time_while_the_manual_clock_is_held` to two threads).
8. Healthy stop: unchanged results for every existing stop test (no thread detached, `close_streams` called once).

Hardware/UHD reliance: none new — the joins are host-side; a thread stuck in a UHD call is UR-16's existing case (VERIFIED in code, `mod.rs` and design/18 UR-16).

## Test-timing rules
Upper bounds on wall time are the `wall-clock-test-oracle` mechanism (`plan/phase8/51-test-timing.md` §3, `plan/maintenance/failure-mechanisms.md`). Prefer observing the decision (which threads are named/detached, which joined) over elapsed time; where elapsed time is the point (1, 5), use a margin that separates the old and new behaviour by ≥ 1 s (e.g. three stuck threads in one stage: new ≈ 1 s, old ≈ 3 s, assert < 2 s), state why it cannot fail on correct code under load, and give the mutation row `"kill": "bound"` only if its kill is a bound. Lower bounds (≥ 1 s) are safe.

## Spec text to change (list in the plan; written in phase B)
- `design/18-uhd-radio.md` UR-16 (~line 224: "1 s of real time per thread") and UR-26's stop sentence (~352: "joins its threads … in stream order"); its test table row `ur_16_a_wedged_thread_does_not_wedge_stop` (~567) if its bound changes.
- `design/06-kernel-coordinator.md` KC-46b (~446: "about 3 s at worst") — becomes true for every layout; check the wording.
- Rule format AGENTS §2; `rules.py`.

## Phase A deliverable
`test-plan.md`: for each sequence 1–8 (and any you add from reading the code — e.g. the `start` failure path's `stop(Abort)`, `cleanup` after a detached stage, a panic in a thread during the stage), the test name (new or extended), the rig, what it asserts, how the assertion observes it, the wrong behaviour that fails it (the old per-thread bound must fail at least sequences 1, 4 and 5), whether its verdict depends on timing (then it is repeated 20 times in phase B) and the mutation row(s) planned (Rust token anchors, smallest unique text; rows go in `plan/maintenance/tools/mutations.json`'s maintenance list in phase B). Also list the spec sentences to change. Read-only in phase A: no edits to the worktree.

Report (Japanese, short): the plan's path, the sequences added beyond 1–8, open questions.

## Phase B brief

# 8b-1 phase B — implement (lane 2)

Worktree `/home/komatsu/local_work/Ez-SDRv4-n111`, branch `work/8b1` (from e178a2e5; `main` has since moved to 5bc555d3 — do not rebase, base stays e178a2e5). Do not commit, stash, push, reset or switch branch. Never touch `/home/komatsu/local_work/Ez-SDRv4` (another lane is working there). Work dir ``.

Read, in order: `brief.md` (the step, the sequences, the spec sentences), `test-plan.md` (v2, your plan), `sol-dispositions.md`, `recheck-dispositions.md` — the last one **amends the plan and binds**: S9 rebuilt on the unit rig with rx0 already finished (no `fail_rx_spawn_of`), `finished(name)` asserts on a name the stage was not given, S6 observes at the decision points, production keeps only `join_now` (hook points and types `#[cfg(test)]`, `join_stage(&mut self, core, names: &[String])`), and the listed minors.

Do:
1. Write the new and changed tests first and show each fails on the current code where the plan says it does (record the failing output's key line per test in your report); then implement in `crates/ezsdr-radio-uhd/src/provider/mod.rs`.
2. Spec text: design/18 UR-16 (shared 1 s per stage, the completion sweep), UR-26 (all Shutdowns of a direction enqueued before that direction's wait; cites UR-16), their `*Test:*` lines and test-table rows, design/06 KC-46b (≈ 3 s of join waiting plus polling/scheduling overhead; RS-8a's 5 s is separate), a *Changes* row in both specs (cite the decision `plan/phase8b/01-device-bundle.md` D-6 and debt #20). Rule format per AGENTS.md §2.
3. Mutation rows as the plan (with the re-check amendments) in `plan/maintenance/tools/mutations.json` (maintenance list), Rust token anchors, smallest unique text; retarget the existing rows the plan lists. Confirm with `cargo test … -- --list` that each OnceLock filter selects exactly one test. Run every new and touched row via `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:2,sim03:2 <ids>`; all must be killed (a `kill: bound` row only where the plan says).
4. Timing-dependent tests (those the plan marks, and S7b) repeated 20 times; pure ones once.
5. Before reporting: `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `plan/maintenance/tools/remote/rdev.py -- cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`; `cargo test --workspace` (local or rdev); `python3 plan/maintenance/tools/rules.py --base e178a2e5`.

Rules: mutation rows only via `remote_mutate.py` under `simq`, never `mutate.py` locally, never apply a mutant to the tree; build output never in the tree (the worktree has its own `.cargo/config.toml`); test temp files on /tmp; kill by PID only; chmod 644 any new file. Self-review is waived (Sonnet implementer); Terra and Opus review the diff after you report.

Report (Japanese, short): diff stat; each test with its failing-first line; rows and outcomes; spec sentences changed; deviations from the plan with reasons; open questions.
