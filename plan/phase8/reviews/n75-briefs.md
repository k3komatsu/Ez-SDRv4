# #75 — briefs (2026-11-13)

---

## Brief — #75: confirmed recovery flush (T3) with a bounded retry — test plan

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n75` at main `4327a66`. You are the implementer (OpenCode Sonnet 5.5 #high). This run writes the **test plan only**, to /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n75/test-plan.md; no code in the tree (probes only in a copy outside it, `rsync -a --no-times --exclude .cargo --exclude target <wt>/ /tmp/n75-probe/`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; delete it at the end).

Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n75/: options.md (your investigation; its last sections are the owner's decisions) and fable-review.md (Fable's design review; the orchestrator adopts F1, F4, F5, F6 and its text list). Decided:
- Option 2: a bounded retry inside one reopen — a fresh streamer and a fresh T3 per attempt, up to 3 attempts; recovery ends only on a **confirmed** T3: the attempt's send was whole and every channel position ACKed (F1: an ACK is not tied to a streamer generation; an attempt whose send put nothing out ends at once and is never confirmed by a stray ACK; the front is taken from the largest ACK tick read).
- (1) After the last unconfirmed attempt the transmit stream ends for the Run with **one** `COMMAND_REJECTED` (F2, owner); each attempt is one row in the Provider's record; a stream recovered on attempt 2 or 3 leaves no rejection. Receive and other streams go on.
- (2) A whole send with no ACK counts as unconfirmed.
- (3) The open-time flush at `arm` follows the same rule (an unconfirmed flush refuses the Connect, a cause of EA-10's `ended`), **implemented but disabled** until the 2026-11-15 open-flush sweep: put it behind one switch the orchestrator turns on after the bench (say how; not a user-facing option).
- F4: the loop lives in `open_tx_device_locked` (both callers: arm and `reopen_tx_streamer`); `Ok` means confirmed (remove `acknowledged` from the success value so the callers cannot collapse it); stop and loss are checked between attempts and no streamer opens after `stopped` (shutdown's 1 s join bound).
- F5: reports stashed during an attempt are recorded before the stash is cleared (UR-28).
- F6: the retry's time (worst case 3 × (open + 100 ms) per stream) goes into the KL text.
- Text: design/18 :258 define "T3 completes", :259 reverse the missing-ACK sentence, UR-28's generation claim, UR-13's refusal (and the "UR-13: UR-22:" prefix), KL-12's workaround, one new KL entry (a stream ended after a failed recovery; the Connect refusal once (3) is on; workaround: connect again), plan/phase8/44-device-chain-state.md:169.
Risk: **risky** (a lifecycle, the boundary between the fake and the hardware). Write the plan as the earlier stages did: the sequences (your options.md §4's twelve, amended by Fable: drop S9 or replace it with a foreign-ACK test; add Stop / Provider::stop / loss during the retries, C2's pending switch and held bookings over the longer window, a multi-channel partial ACK, the stash), each with the test, what it asserts, how it observes it, the wrong behaviour that fails it; the FakeDevice faults you need (a one-shot ACK drop, a foreign ACK, a short/zero send) and their shape; mutation rows (Rust token anchors); the existing tests whose expectations change (`ur_22_owner_keeps_clean_recovery_after_an_unacknowledged_t3`, the e3b golden line) and why; the hardware tests whose expectations change (audit table); sub-steps.

Report: the path of test-plan.md and a short summary; anything INFERRED.

---

## Brief — #75 test plan amendment after Sol's review (plan only)

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n75` at `4327a66`; read-only for this run (probes only in a copy outside it, deleted at the end). You are the implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n75/: your test-plan.md, sol-plan.md (Sol: FAIL, five majors, three minors) and plan-decisions.md (the orchestrator's answers to your questions and to Sol's findings: all adopted, M1 as stated there).

Amend test-plan.md: a "Changes after Sol's review" section at the top and the body updated so it is the plan to implement (each major and minor resolved with its tests, gates and rows; for M1 state, VERIFIED with path:line, what today's `Reopen::Failed` path records and emits for the open and held bookings). Report the path and a short summary, incl. anything INFERRED and the M1 finding.

---

## Brief — #75 test plan, second amendment (plan only, no builds)

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n75` at `4327a66`. Read-only, and **no cargo builds or tests at all** (a hardware bench is running on this machine). You are the implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n75/: test-plan.md, plan-decisions.md (its last section: the orchestrator's decisions on the Opus re-check — N1, N2/M5, N3, N4, N5, N6, all to adopt). Amend test-plan.md with a "Changes after the Opus re-check" section at the top and the body updated (tests, gates, rows, KL text, line citations corrected). Report the path and a short summary.

---

## Brief — #75 implementation (sub-steps per the test plan)

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n75` at `4327a66`. You are the implementer (OpenCode Sonnet 5.5 #high). Self-review is waived (Sonnet-implemented step); Sol reviews the code at the end.

**A timing-sensitive hardware bench is running on this machine until about 15:00.** Until the orchestrator tells you otherwise: NO local cargo builds, tests or clippy. Build and test only through `plan/maintenance/tools/remote/rdev.py -- <command>` (runs on sim02/sim03), and mutation rows only through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <list> sim02:8,sim03:8 <ids...>`, **in the foreground** (never wait for a notification). `cargo fmt --all` locally is fine (no build).

Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n75/: options.md (owner decisions at the end), fable-review.md, plan-decisions.md (every section binds, the last one included), test-plan.md (the plan to implement). First fold the last section of plan-decisions.md into test-plan.md (a short "Changes after the second Opus re-check" section; padded-end floor; minors 1–3, 5, 6), then implement in the plan's sub-steps.

Rules:
- The `arm` open-flush rule (owner decision (3)) is implemented behind the `OPEN_FLUSH_CONFIRMED` switch, **off**; with it off the legacy path is unchanged and pinned by C11 (f).
- Per sub-step: tests green for the crates you touched (via rdev), the sub-step's mutation rows killed (via remote_mutate, foreground), `cargo fmt --all`, `python3 plan/maintenance/tools/rules.py --base 4327a66` 0 problems. Then write `sub-step-<n>-report.md` in the directory above and **exit** (do not start the next sub-step: the orchestrator commits each sub-step on `work/n75` and relaunches you). Do not commit.
- Every planned sequence gets its test, every test its killing row; record the audit table for hardware tests whose expectations change (AGENTS §9).
- Text: design/18 (UR-13, UR-22, UR-28, Changes table), KNOWN_LIMITATIONS (KL-12 workaround, KL-44 new entry as the plan words it), plan/phase8/44-device-chain-state.md:169, failure-mechanisms.md row — in the sub-step the plan assigns.
- Never edit the main tree /home/komatsu/local_work/Ez-SDRv4. Test temp files on /tmp. Kill processes only by PID.

Report per sub-step: files changed, tests run (command + result), rows run (ids + outcome), plan deviations with reasons, anything INFERRED.

---

## Brief — #75 implementation, next sub-step

Continue from brief-impl.md (same rules: no local builds/tests until the orchestrator says the bench is over — rdev.py and remote_mutate.py in the foreground only; never edit the main tree; do not commit). Sub-step A is committed on `work/n75` as 3afe922 (your report: sub-step-1-report.md; the orchestrator accepted your four deviations, see the last section of plan-decisions.md). Implement the next sub-step of test-plan.md, then write `sub-step-2-report.md` and exit. If the plan has no sub-step left after this one, say so in the report.

---

## Brief — #75 implementation, last sub-step (C)

Continue from brief-impl.md. The bench is over: run cargo tests and clippy locally now (not rdev); mutation rows still only through remote_mutate.py in the foreground. Never edit the main tree; do not commit. Sub-steps A and B are committed on `work/n75` (3afe922 and the next commit; your reports sub-step-1/2-report.md; B's deviations accepted).

Implement sub-step C of test-plan.md: the texts (design/18 UR-13/UR-22/UR-28 and the Changes table, KNOWN_LIMITATIONS KL-12's workaround and the new KL-44 as the plan words it, including the time-read bound "about 3 × (open + 100 ms + one time read)" and the failed-time-read cause, plan/phase8/44-device-chain-state.md:169, the failure-mechanisms.md row), then run the **full mutation lists** once, foreground and through the sim queue: `~/.cache/ezsdr-fixes/tools/simq env REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8` (it may wait for another lane; that is expected) and settle every row that fails. `cargo fmt --all`, `rules.py --base 4327a66` 0 problems. Write `sub-step-3-report.md` (files, tests, full-list result with any settled rows, the landing list: every user-visible change and every orchestrator decision) and exit.

---

## Brief — #75 fix round 1 (Sol's code review)

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n75` (sub-steps committed, HEAD 3e03286). You are the implementer (OpenCode Sonnet 5.5 #high). Local cargo tests and clippy; mutation rows only through remote_mutate.py in the foreground (touched and new rows only; the orchestrator runs the full lists at the gate). Never edit the other trees; do not commit.

Read sol-code.md (Sol: FAIL) and the last section of plan-decisions.md (all four findings adopted, with the required tests). Turn each major's sequence into a failing test first, then fix. New tests whose verdict can depend on timing (threads, the manual clock) run 20 times. `cargo fmt --all`, `rules.py --base 4327a66` 0 problems. Write fix1-report.md (per finding: test, fix, rows and outcome, 20-run results) and exit.
