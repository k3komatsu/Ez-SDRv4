# #98 — briefs (2026-11-12)

## Test plan


Worktree /home/komatsu/local_work/Ez-SDRv4-e3 (branch `work/n100` at `0697ed8`, under its gate now: **do not edit, format or build in this tree**; read it, and build probes only in a copy outside it: `rsync -a --no-times --exclude .cargo --exclude target <wt>/ /tmp/n98-probe/`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; delete the copy at the end). You are the implementer (OpenCode Sonnet 5.5 #high). This run writes the **test plan only**, to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n98/test-plan.md; no code in the tree.

Issue: `gh issue view 98`. A configure turn that fails because the device is lost between two channels' applies (`Err` with the core lost) refuses the booking: `complete_configure(id, false)` takes it out of `held` and drops its region (`crates/ezsdr-radio/src/staging.rs` ~819-827), and `configure_turn` (`crates/ezsdr-radio-uhd/src/provider/tx.rs` ~507-536) reports it through `device_failed`, which emits the one `DEVICE_LOST` but no note naming the booking. A loss a moment earlier or later (before the configure's mark, or after the turn returned) leaves the booking held and `Provider::stop` cancels it with `UR-26: cancelled by Provider::stop`. Pinned today by `e2b_c4_a_loss_around_the_configure` (`crates/ezsdr-radio-uhd/src/provider/tx/tests/e2b.rs` ~292-340): C4(ii) expects no note.

**Owner's decision (2026-11-12):** after E2b (landed: Configure is an operation of the book), the booking stays in the book during its configure, so the loss path accounts for it like a held booking. Keeping the booking also keeps its footprint (RM-15) and changes which shutdown note it gets; the fix states both.

Risk: **risky** (a lifecycle: what the loss path does with a booking whose configure failed as lost). Sequences that matter, each with its expected result (the plan confirms or corrects each with evidence):
- S1 loss between the channels of a configure (C4(ii)): one `DEVICE_LOST`, no refusal, the booking held until `Provider::stop`, then `UR-26: cancelled by Provider::stop`; nothing sent. Its record region stays (the footprint a held booking has) until the stream ends.
- S2 a configure refused by the device with the core **not** lost (an ordinary `Err`): unchanged — refused (R5), region dropped, `COMMAND_REJECTED` through `device_failed`; the stream goes on.
- S3 a loss before the mark (C4(i)) and after the turn (C4(iii)): unchanged.
- S4 a lost configure while a `Stop` is pending, and while a repeat switch is pending on that stream (C2's `Bookings`): the note and the switch outcome are what a held booking gets under the same loss.
- S5 two held bookings, the loss in the first one's configure: both get the same note.
- S6 the error race: the configure's `Err` was not caused by the loss but the core is lost by the time the result is read (a loss landing during an ordinary refusal) — which wins, and why (cite the code that orders them).
Also: does any `design/` rule (UR-26, R5 text in design/18-uhd-radio.md, RM-15) or KNOWN_LIMITATIONS say what a lost configure does? List the text that changes. The Mock has no lost configure (its configure does not fail, M5): confirm.

For each sequence: the test (new, or the C4 characterization changed — say which expectation changes and that this is the decided change), what it asserts, how it observes it, the wrong behaviour that fails it, and the mutation rows (Rust token anchors, smallest uniquely matching text). Propose the code change in a few lines (where the decision "refuse or keep" is made: `configure_turn` or `complete_configure`, and why one place), and say how `device_failed` / `device_lost` still emits exactly one `DEVICE_LOST`.

Report: the path of test-plan.md and a summary of the sequences and the proposed change; anything you could not verify marked INFERRED.

## Orchestrator decisions on the plan


Sol's plan review (sol-plan.md): PASS WITH CHANGES, three minors, all adopted.
- m1: add a case where `Provider::stop` books the end (streams.stop / streams.end, Shutdown queued) before a lost configure completes: one cancellation, no refusal, nothing sent, mark cleared, claim returned (also covers a loss after `Streams::ended`, core.rs ~652-655).
- m2: N98-01 and N98-05 are not killed by C4; name their real killers (T4, T2's device-reported-loss mode) and do not claim C4.
- m3: the "no second configure" assertion must observe something: the `ApplyFails` wrapper's attempt count or the configure probe's invocations across later turns; drain events after shutdown before asserting one DEVICE_LOST and no refusal.
- Wording: the normative sentence says "a configure that returns `lost: true`", not "cut by a loss" (S6: an ordinary refusal racing a loss stays a refusal).
- The implementer's correction of the brief is accepted: the booking is already held through its configure since E2b; the fix is in the result (`configure_turn`), one bit (`error.lost`) deciding the book and the events.
- Pending-switch reachability (S4b): INFERRED unreachable, accepted; T5 optional (keep only if it pins a book-level rule cheaply).

## Implementation


Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n98` from `main` (`git log -1`; the commit that landed #100). You are the implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n98/: brief-plan.md, your test-plan.md, draft-e2b.rs, sol-plan.md and plan-decisions.md (all adopted). Another lane edits the main tree (`~/local_work/Ez-SDRv4`, `crates/ezsdr-radio-uhd/tests/**`); do not touch `crates/ezsdr-radio-uhd/tests/` here.

Do: amend test-plan.md with a "Changes after Sol's review" section; implement the change in `configure_turn` (`crates/ezsdr-radio-uhd/src/provider/tx.rs`) as planned; the tests T1–T4 (+ m1's case; T5 only if cheap), C4(ii)'s changed expectation (the decided change; no other existing expectation may change — if one does, stop and report); rows N98-01… with their real killers and E2B-13/E2B-16 retargeted (a re-anchored row must produce the same program as before or say why it now mutates the new code); the text (UR-26, RM-15 one sentence each with "a configure that returns `lost: true`", their Changes rows; note 49 §5.4.4, §5.4.6; the #98 row of plan/maintenance/failure-mechanisms.md).

Rules: tests must not depend on when a thread runs (note 51 §3); new timing-dependent tests 20 runs. **Build and test locally.** Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 13364eb` = 0. No commit. Then run your new and retargeted rows, plus every row whose code or killing test you touched: `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <id> <id> ... > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n98/impl-mut.log 2>&1` (ids as separate arguments; the other lane may hold the sims): **do not end your run until it has finished and you have read its result**.

Report: `git diff --stat`; the change with path:line; each test and what it asserts; C4(ii)'s change; rows and the assertion that kills each; text changed.
