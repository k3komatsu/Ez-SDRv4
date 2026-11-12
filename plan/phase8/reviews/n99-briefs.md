# #99 — briefs (2026-11-12/13)

## brief-plan


Worktree /home/komatsu/local_work/Ez-SDRv4-e3 (branch `work/n98`, under its gate now: **do not edit, format or build in this tree**; read it; probes only in a copy outside it: `rsync -a --no-times --exclude .cargo --exclude target <wt>/ /tmp/n99-probe/`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; delete the copy at the end). You are the implementer (OpenCode Sonnet 5.5 #high). This run writes **design options and a test plan only**, to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n99/test-plan.md; no code in the tree.

Issue: `gh issue view 99` (sequence at 1 MS/s on the frozen UHD rig: A finite 2 000 at 10 000 abandoned by `ShortSend(0)`; B 10 samples at 12 000 accepted as joined but sent as its own closed burst; C 4 samples `DropAndFlag` at 12 014 accepted at receipt and dropped at dispatch with `radio.TIME_ERROR "drop"`). Background: note 49 (plan/phase8/49-time-and-booking-inventory.md, D′; stage E3b kept receipt on the record's prediction, "shape S", owner) and E3b's records (plan/phase8/reviews/e3b-*; the device burst's state is the book's `DeviceChain`, crates/ezsdr-radio/src/staging.rs). The issue says the fix — the device burst's actual state decides whether a join is possible — is a user-visible change.

Do:
1. Reproduce #99 on the current tree (a probe test in the copy); confirm or correct the issue's sequence and outputs.
2. Find every path where the record's predicted front and the `DeviceChain`'s actual state can disagree at receipt (abandon by short send, failed send, sequence-error report, reopen, loss, Stop): for each, what receipt predicts, what dispatch does, and what the user sees (refusal at receipt, `TIME_ERROR` drop at dispatch, a burst sent closed instead of joined).
3. Design options, at least two, each with: the user-facing contract (what a user sees for B and C, and when — before vs after the abandon is known), how it fits RM-13/RM-15/RM-16, UR-21/UR-22 and the front rule A (owner 2026-11-10: a ≤ 32-sample closed burst → end + max(3 samples, 15 µs), else end + 3), the Mock's behaviour (does the Mock have the same split? the Mock's sends do not fail, M5), what it costs (where the code changes: receipt reads the chain? the abandon rewrites the record's front?), and what it makes impossible later. Recommend one, with the reason; mark each claim VERIFIED or INFERRED with path:line. Note the race: a receipt can come before the abandon is known; say what each option does then.
4. A test plan for the recommended option, as the earlier plans do: the sequences with expected results, each test, what it asserts and how, the wrong behaviour that fails it, mutation rows (Rust token anchors).

Report: the path of test-plan.md and a short summary: the reproduction, the disagreement paths, the options and the recommendation, and what in it is user-visible.

## brief-plan2


Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n99` at `cc8c3cb` (main). Read-only for this run: **do not edit, format or build in the tree**; probes only in a copy outside it (`rsync -a --no-times --exclude .cargo --exclude target <wt>/ /tmp/n99-probe/`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; delete it at the end). You are the implementer (OpenCode Sonnet 5.5 #high). Your first plan: /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n99/test-plan.md (written on `44eb9b2`; main has since landed #98 — recheck line numbers). Read in the same folder: design-decisions.md (owner: direction B; the orchestrator's adoptions) and fable-review.md (Fable's findings F1, F2, the plan errors and the text corrections).

Amend test-plan.md with a section "Changes after the owner's direction and Fable's review" at the top, and update the body so the plan is the one to implement:
1. F1: the guard that keeps a finite abandon from changing a chain a halt has ended — choose it with evidence (Fable's Halt-unread-in-the-list check, or better), probe it on the reference sketch with Fable's sequence (pure) and a rig sequence with a Stop received at the `tx_send_planned` seam and a report-driven abandon, and list what it must not change.
2. F2: `end_finite(k, e)` keyed by the abandoned piece; Fable's prune sequence as a pure test; the stale-halted-chain match.
3. S9 corrected (a Stop after an abandon gives a re-framed standalone piece its region [t, stop)), and every sequence's expected result re-derived on the amended sketch.
4. Hardware: the report-driven abandons (underflow / sequence report, prompt send error) as primary rig cases; the short send's 1 s timeout noted.
5. Text: RM-15, UR-23, KL-39 (and the KL-38 cross-reference) as Fable corrected them; quote the exact sentences you will write.
6. Rows (Rust token anchors), the existing rows to re-anchor, sub-steps, and the risk class: risky.

Report: the path of test-plan.md and a summary of what changed in the plan, with anything not verified marked INFERRED.

## design-decisions


- Owner: direction B (the abandon of a finite booking is an owner's close for the record; KL-39's footprint kept; a booking at the abandoned burst's end refused with UR-21).
- Fable design review (fable-review.md): sound with changes. Orchestrator adopts F1, F2 and every text and plan correction:
  - F1: a finite abandon never changes a chain a halt (Stop, loss, Provider::stop) has ended; the guard is decided in the plan (Fable's suggestion: `TxBook::abandon` skips the record write while a Halt is unread in its list, and when lost), probed, with a pure test and a rig test (a Stop at the `tx_send_planned` seam).
  - F2: the record write is keyed by the abandoned piece (`end_finite(k, e)`), the chain holding `k`, split at the first join point >= e.
  - S9 corrected: after an abandon, a Stop gives a re-framed standalone piece its region [t, stop) (RM-16). Decided by the orchestrator as an application of B; listed for the owner at landing.
  - Risky: Sol test-plan review before code.
  - B+ (grow-only front for a late abandon, P4) not included; P4 stays by design as P3 (late policy at dispatch).
  - The dispatch stat (Fable's optional catch-all) not included.

## Sol's plan review (sol-plan.md, PASS WITH CHANGES) — all three minors adopted (orchestrator, 2026-11-13)
- m1: the halt exception is narrowed to "a finite abandon does not rewrite the record a halt already ended" (not "after any Stop"); a Stop, then a new finite booking, then its abandon ends that booking's join; repeat abandons unchanged. Same qualification in UR-23.
- m2: KL-39's front: the retained RM-15 front (a short standalone head keeps padded end + g; a long or joined piece end + 4), not "four samples beyond the end" for every case; example one sample at 10 000 → 10 019.
- m3: a pure composition test: A(40)+B(100)+C(10) → abandon A → abandon B → move/drop C; A and B keep their fronts, C alone moves or leaves, the first sample beyond the remaining footprints stays free.
- `end_finite(e)` (no `k`) accepted: the end identifies the chain without prunable join points.
- The implementer's local mutant check (n99-mut.py) is not gate evidence; rows run through remote_mutate.py.

## brief-impl


Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n99` at `829b7b0` (main; stage 2a and #98 landed since your plan's base — recheck lines). You are the implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n99/: design-decisions.md (owner: direction B; the orchestrator's adoptions of Fable's and Sol's findings), test-plan.md (your amended plan), fable-review.md, sol-plan.md, and the sketches in probe/. Another lane edits the main tree (`~/local_work/Ez-SDRv4`, note 51 stage 2b: `crates/ezsdr-radio-uhd/tests/**`): do not touch `crates/ezsdr-radio-uhd/tests/` here.

Do, in the plan's sub-steps (refactor, record, Provider and text), keeping each sub-step's files separable in your report: amend test-plan.md with "Changes after Sol's review" (m1–m3); implement the plan with Sol's three minors (the narrowed halt exception and its text in RM-15 and UR-23; KL-39's front as the retained RM-15 front; the composition test); the existing expectations that change are only those the plan names (`e3b_c1`'s F3 receipt column, `e3b_c6`'s F4 half) — any other changed expectation: stop and report; rows (new, re-anchored; a re-anchored row must produce the same program or say why it mutates the new code); the text (RM-15, UR-23, KL-39, the KL-38 cross-reference, note 49 §5.4.4 where it says what an abandon does, the #99 row of plan/maintenance/failure-mechanisms.md, KNOWN_LIMITATIONS updated in the same commit).

Rules: tests must not depend on when a thread runs (note 51 §3); new timing-dependent tests 20 runs. **Build and test locally.** Never a mutant in the tree (and never apply one in a copy and call it a mutation result: rows run only through remote_mutate.py); kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 829b7b0` = 0. No commit. Then run your new and re-anchored rows plus every row whose code or killing test you touched: `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <id> <id> ... > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n99/impl-mut.log 2>&1` (ids as separate arguments; the other lane may hold the sims): **do not end your run until it has finished and you have read its result**; settle every row not killed.

Report: `git diff --stat`; per sub-step what changed with path:line; each test and what it asserts; the changed expectations; rows and their killing assertions; the text changed.
