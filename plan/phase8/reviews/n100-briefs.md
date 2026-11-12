# #100 — briefs (2026-11-12)

## First run


Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n100` from `main` (the commit that lands C2; `git log -1`). You are the implementer (OpenCode Sonnet 5.5 #high). Another lane edits the main tree (`~/local_work/Ez-SDRv4`, note 51 stage 2a: `crates/ezsdr-radio-uhd/tests/**`); do not touch `crates/ezsdr-radio-uhd/tests/` here.

Issue: `gh issue view 100`. On the `ideal` profile, a repeat booked from sample 2 000 and a repeat switch received at 2 000 panics in a debug build at `debug_assert!(self.open.is_none(), "a booking is already open")` (`crates/ezsdr-radio/src/staging.rs`, `TxBook::take`). Found by the C2 implementer (C2's Mock test M1 receives later to avoid it; record plan/phase8/reviews/c2-dispositions.md "Implementer's findings").

Do:
1. Reproduce with a failing test (Mock level, deterministic). Find the root cause: which sequence of `TxBook` calls opens a second booking while one is open, and why the `ideal` profile reaches it (and whether `x3xx` can, with which receipt time). Say whether the cause is in the Mock's use of `TxBook` or in `TxBook` itself — `TxBook` is shared with the UHD Provider: if the cause is in `TxBook`, show whether the UHD path can reach the same sequence (a frozen-rig lib test in `crates/ezsdr-radio-uhd/src/provider/tx/tests/`, not the integration tests) and what a release build does there (two bookings open at once: what is sent and recorded).
2. Fix the root cause once, in the shared code if that is where it is. The fix must not change any other test's expectation; if one changes, stop and report it (that is an owner-visible behaviour question).
3. Text: MR-/RM-/UR- rules the fix makes false or that should name the case; KNOWN_LIMITATIONS only if it was listed (it is not).
4. Mutation rows (Rust token anchors) killed by an assertion; run them and every row whose code or killing test you touched through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids> > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n100/mut.log 2>&1` — the sims may be busy with the other lane (simq waits); **do not end your run until it has finished and you have read its result**.

Rules: risk not risky unless step 1 shows the UHD path reaches it (then say so and stop after step 1 with a report: the orchestrator re-plans). **Build and test locally**, not on sim02/sim03. Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e449397` = 0. No commit.

Report: `git diff --stat`; the root cause with path:line; the failing test (its failure before the fix); the fix; rows and outcomes; whether UHD can reach it.

## Fix round 1 (after Sol's code review)


Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n100` (base `e449397`), your uncommitted diff from the first run is in place. You are the implementer (OpenCode Sonnet 5.5 #high). Read: brief.md, the end of impl.log (your report), and sol-code.md (Sol: FAIL, one major) in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n100/.

Adopted (orchestrator):
1. **Major** — the same panic is reachable after the repeat's first block: on `ideal`, repeat 2 000 samples at 2 000, step to 4 000 (emits [2000, 4000), `open.next = 4000`, `first == false`), then a switch received at 4 000 → `open_held` from `advance` → `take` with an open booking. Write Sol's sequence as a failing test first (debug: the assertion; also say what a release build does: tracker discontinuity / device-model rejection). Then handle an open predecessor at the held booking's start **whether or not it has emitted**, the way UHD's `join_open` does (`crates/ezsdr-radio-uhd/src/provider/tx.rs` ~403-405, 1002-1009, tracker closure ~751-755): the switch continues the same device burst with no seam, the record, the tracker and the device model stay consistent, and the switch row settles `applied`. One code path for both cases (the `first` guard goes). Assert observable results: no rejection, the `bursts` records (what each holds), the samples at the seam (continuous, the new waveform from 4 000), and the switch row.
2. **N100-1 is killed only by the internal `debug_assert!`**: the regression tests must also fail on an observable assertion (records / samples / rejections) when the fix is removed, so the row would die in a release build too. Re-check N100-1, N100-2 and add a row for the emitted case.
3. MR-42's sentence: widen it to the emitted case if the behaviour you implement needs words (a switch at the open repeat's next sample continues its burst); keep it one sentence.

Rules as in brief.md (local build/test, `cargo fmt --all`, workspace tests green, clippy clean, `rules.py --base e449397` = 0, no commit, never a mutant in the tree, kill by PID). Run your rows plus N100-1/N100-2 through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids> > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n100/fix1-mut.log 2>&1` — the other lane may hold the sims (simq waits); **do not end your run until it has finished and you have read its result**.

Report: `git diff --stat`; the failing test before the fix (debug and release behaviour); the fix with path:line; each row and the assertion that kills it; MR-42 text.
