# Phase 8-end review — briefs

## Opus draft brief (note 52)

# Phase 8-end review: the design note on code and test architecture (draft) — Opus, effort max

Read-only on the repository (`/home/komatsu/local_work/Ez-SDRv4`, use `git show origin/main:<path>` or the worktree `/home/komatsu/local_work/Ez-SDRv4-n111` at main 4679abc1 for reading; never edit either tree, never commit). Write the draft to `/home/komatsu/.cache/ezsdr-fixes/2026-11-20-p8end/draft.md`. Measurements you run (git log/diff stats, line counts, mutation row counts) go in `/home/komatsu/.cache/ezsdr-fixes/2026-11-20-p8end/` with the exact commands so they can be rerun; build output, if any, under `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, temp files on /tmp.

## The ask (owner, 2026-10-31 and 2026-10-19; plan/phase8/50-remaining-work.md §2 "最後"; handoff.md §2 item 2)
Before 8b, a review of how the code and tests are built, as a design note the owner decides from. Read AGENTS.md §6 (adversarial, evidence-driven, minimal-core; lead with one recommendation plus rejected alternatives; VERIFIED / INFERRED), §7 (mutation runs), §9 (Maintainability, Counterexamples, the review roles), and handoff.md §2 item 2 in full.

1. **Measure first** (the owner asked for the growth of past stages before any proposal): per landed step in Phase 8 (`git log --first-parent origin/main` since Phase 7's Gate X; plan/phase8/reviews/*-dispositions.md name them), production lines changed vs test lines changed, mutation rows added/retargeted/removed, tests rewritten, review rounds and majors (from the dispositions' count tables), and which mechanisms (plan/maintenance/failure-mechanisms.md) recurred. Current totals per crate (production, inline tests, `tests/`, mutation-row JSON lines). Say what each number shows and what it does not.
2. **The questions** (one section each, a recommendation plus rejected alternatives, what it makes impossible later, cost):
   a. hand-written mutation rows vs a generator (cargo-mutants or similar) — the division of roles; feature `uhd`'s rules outside the mutation gate (no libuhd in the image; handoff §3 row "feature `uhd` の mutation"); how to choose the tool before the v4.0 freeze;
   b. test setup and fixtures shared across steps (FakeDevice literals vs `common::fake()`, `Direct`/`UhdRadio` without `Drop`, duplicated rigs), pure-decision tests vs wiring/concurrency tests, the wall-clock test oracles left (G10/G29 killers; `n47s5_front_rehearsal_on_the_fake`'s host-lateness marker);
   c. the review pipeline's cost and yield in Phase 8 (Sol / Opus / Terra / Fable rounds; plan-review rounds that did not converge — bt2's three paper rounds; what the sub-step reviews caught vs the final code review), and what to keep for 8b;
   d. the collected debts: every item the step records sent to "the Phase 8-end review" (`git grep -n -i "phase 8-end" origin/main -- plan` plus the n111/n120/n115/tools/e3b/n51 dispositions) — list each with a disposition (do now / 8b / drop, with reason);
   e. the Module-form design note owed by plan 49 §5.13 (plan 50 §3 row ④), and the stat `tx_close_past_record_front` (E3b K2: a Manifest key, the owner's to remove — bring the evidence).
3. **What the owner decides**: a numbered list of decisions, one per item, each with the recommendation, the alternative and the consequence, so the orchestrator can bring them one at a time.

Keep the note proportionate (research project; no framework for its own sake; "this should be deleted" is welcome). Mark every claim about measured growth, tool capability (cargo-mutants on this workspace: say whether you checked) and cost VERIFIED or INFERRED. Return a short summary (Japanese) and the path.

## Sonnet brief: D3 (kc_36) and D7 (the stat)

# Phase 8-end review: two small steps (owner decisions D3 and D7, 2026-11-20) — Sonnet drives Go DeepSeek

Worktree `/home/komatsu/local_work/Ez-SDRv4-n111`, branch `work/p8end` (base main 4679abc1). Work only there; never touch `/home/komatsu/local_work/Ez-SDRv4` (another lane). Do not commit, stash, push, reset or switch branch. The orchestrator edits `AGENTS.md` and `plan/` in this tree meanwhile; do not touch those files. Not risky (test-only change and a stat removal); no test plan; DeepSeek may write the code directly (launch, waits and rules as in `/home/komatsu/.cache/ezsdr-fixes/2026-11-20-n120/brief.md` §Process, with this folder for tasks and logs; DeepSeek may edit the tree; never a mutant in the tree; rows via `simq` + `remote_mutate.py`, `chmod 644` named files only).

1. **kc_36's two TTL tests on the manual HostClock (D3, gate trust).** The draft `/home/komatsu/.cache/ezsdr-fixes/2026-11-20-p8end/draft.md` §2b and debt #15 (and `plan/phase8/reviews/n51-3-*`) name them: the detached-lease TTL tests in `crates/ezsdr-kernel/tests/` that can fail under load because they wait on real time. Move them to a manual `HostClock` (`Lease::detached` takes one) so the verdict no longer depends on host scheduling; keep what each asserts; their mutation rows keep dying by assertion (rerun them).
2. **Remove the stat `tx_close_past_record_front` (D7).** Draft §2e lists the sites: the one statement in `close_unjoined` (`crates/ezsdr-radio-uhd/src/provider/tx.rs` ~546), the `past_front` helper and its checks (tx.rs, e1.rs, e3b.rs), the test `ur_21_stat_counts_owner_closes_past_the_record_front`, row N47S5-60 (plan/maintenance/tools/mutations.json), UR-23's `*Test:*` name in `design/18-uhd-radio.md` (~306) and one row in spec 18's Changes table ("the stat removed; owner, 2026-11-20"). Remove what becomes obsolete (AGENTS §9 leftovers); retarget a row only if its behaviour stays reachable. Pass `rules.py --renames` for any renamed test.

Exit checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the UHD-feature clippy via `plan/maintenance/tools/remote/rdev.py`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 4679abc1`, the rows touched. Report (Japanese, short): `git diff --stat`; per item what changed and the row outcomes; checks.
