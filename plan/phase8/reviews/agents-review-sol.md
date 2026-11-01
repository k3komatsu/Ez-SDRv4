# AGENTS.md review (Sol 6.1, 2026-11-01)

Read-only review of AGENTS.md at d549859 for stale text, contradictions and rules that hold agents back. Brief: stale / contradiction / holds-back, keep the safety core.

**Disposition (orchestrator, owner approved 2026-11-01):** R1–R14 all adopted and applied in the same commit. Added by the orchestrator: never use `opencode2 run -s` on an idle session; drop the Orca-tab line (agents run from the CLI); handoff §7 local paths (usrp-lnx03, no gh-pages worktree here). No Sol re-review (minor and wording fixes only); `rules.py` passes.

---

**Verdict: patch AGENTS.md, not rewrite it.** Its safety rules and design boundaries remain useful. The defects are concentrated in role scoping, duplicated workflow instructions, and a few stale operational claims.

Reviewed `main@d549859`. No files changed; `v3/` was neither read nor entered. Candidate problems were listed before repository verification. Findings marked **INFERRED** concern likely agent behavior, not measured waste.

## Findings

### R1 — contradiction · major — Formatting is mandatory even for read-only reviewers

**Section/quote:** Safety core; §9: “Run `cargo fmt --all` before reporting.”

**Evidence:** `AGENTS.md:5,12` explicitly applies this to every agent/task, while `AGENTS.md:8,129,165` requires reviewers to remain read-only. The gate already uses the non-mutating check (`plan/maintenance/tools/gate.py:275`).

**Counterexample:** An implementer leaves an unformatted change → a reviewer follows the reporting instruction → `cargo fmt` changes the frozen review tree. The reviewer has now invalidated the source under review.

**Replacement:**
> Implementers run `cargo fmt --all` before reporting Rust changes. Read-only reviewers never format files; use `cargo fmt --all --check` if formatting verification is needed.

### R2 — contradiction · major — Required gate records trigger an endless re-gating interpretation

**Section/quote:** §9: full checks run “again for whatever changes after it”; the closing commit must contain “the gate’s `gate.json`.”

**Evidence:** `AGENTS.md:144,168` requires both. Gate identity includes tracked changes and every non-ignored untracked file (`plan/maintenance/tools/gate.py:221–228`). Gate output must be outside the checkout (`gate.py:435–438`).

The existing closing record demonstrates the necessary exception: `plan/phase8/reviews/a-gate-gate.json:8–19` identifies a tree containing the implementation files but not the subsequently added review records or manifest.

**Counterexample:**
```text
Gate passes → copy gate.json into reviews/
```

**Replacement:**
> Run the full gate after the final implementation, test, schema, configuration, or tooling change. Afterward, only the closing records may be added or updated without another full gate. Preserve the original manifest, verify that no other files changed, and run the applicable document checks.

### R3 — contradiction · minor — The build-target rule does not distinguish host copies from isolated runners

**Section/quote:** §7: review, mutation, and probe builds use a target “shared by every reviewer; never a per-copy target directory.”

**Evidence:** `AGENTS.md:93` is broader than current tooling:
- Docker gate targets are deliberately per checkout (`plan/maintenance/tools/gate.py:238–240,272`).
- Remote mutation shards have isolated containers and targets (`plan/maintenance/tools/remote/remote_mutate.py:9–13,79–80`).
- This isolation fixed an actual cross-checkout artifact-reuse finding (`plan/phase8/reviews/a-gate-sol.md:11`).

The host-copy timestamp safeguards are still relevant; deleting the entire paragraph would be wrong.

**Replacement:**
> Manual host review/probe copies use the shared review target with the timestamp and serialization safeguards below. Gate Docker builds use the runner’s per-checkout volume; remote mutation shards use their own isolated targets. Do not override runner-managed isolation.

### R4 — contradiction · minor — “Main only” conflicts with the accepted worktree workflow

**Section/quote:** §1: “v4 work happens on `main` only.”

**Evidence:** `AGENTS.md:29` permits only the historical spike exception. Yet the recently accepted gate implementation explicitly used `Ez-SDRv4-gate`, branch `wip/gate` (`plan/phase8/reviews/a-gate-brief.md:3`), and its final gate ran in that worktree (`a-gate-sol.md:3`). The gate expressly supports worktrees (`plan/maintenance/tools/gate.py:2`).

**Replacement:**
> v4 changes target `main`. An orchestrator-approved temporary branch/worktree may be used for implementation or verification; only the orchestrator integrates and pushes it. The historical branch restrictions below still apply.

### R5 — contradiction · minor — Commit ownership has two answers

**Section/quote:** §7: “The user usually commits themselves.”

**Evidence:** `AGENTS.md:90` conflicts with the explicit current workflow: the orchestrator closes the gate with a commit and push (`AGENTS.md:127,167`). `handoff.md:65` delegates workflow authority to §9.

**Replacement:**
> Stop exactly where the owner asks. Under the normal §9 workflow, only the orchestrator commits and pushes; implementers and reviewers do neither.

### R6 — stale · minor — The advertised gate command is not executable as written

**Section/quote:** §9: “The gate is one command, `plan/maintenance/tools/gate.py --out <dir> --mutation all`.”

**Evidence:** Command output:
```text
stat -c '%A %n' plan/maintenance/tools/gate.py
-rw-r--r-- plan/maintenance/tools/gate.py
```

`handoff.md:68` supplies the working Python invocation.

**Replacement:**
```sh
PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/gate.py --out <outside-checkout-dir> --mutation all
```
Add:
> Set `--base` to the pre-change commit when HEAD already includes changes being checked.

### R7 — stale · minor — DeepSeek is still named as the implementer

**Section/quote:** §9: “the Sol review waits … while DeepSeek … keeps working.”

**Evidence:** `AGENTS.md:162` embeds an implementation-model choice despite §9’s single-source rule (`AGENTS.md:128`). The current implementer is Haiku 5.5; DeepSeek performs self-review (`handoff.md:66`).

**Replacement:**
> When the Sol window runs low, postpone the Sol review until reset. The current implementer may continue independent work that does not require that review.

### R8 — stale · minor — The required search agent is missing locally

**Section/quote:** §8: “use `haiku-scout` for searches instead”; definitions live in `~/.claude/agents/`.

**Evidence:** Reading `~/.claude/agents/` returned only `sonnet-researcher.md`. A project search for `.claude/agents/*` returned no files. The installed researcher is explicitly read-only (`~/.claude/agents/sonnet-researcher.md:3,12,29–33`).

**Replacement:**
> Use an installed read-only research agent for bounded fact-finding, selecting its model explicitly where supported. If the named agent is unavailable, perform the search directly; do not assume the definition exists.

### R9 — contradiction · minor — Self-review is unconditional in the rule, optional in accepted practice

**Section/quote:** §9: “Each implementation step runs in this order”; “The implementer runs its own self-review loop.”

**Evidence:** `AGENTS.md:131,136` provides no waiver. The accepted gate-tooling brief expressly says “No self-review round (orchestrator and Sol review)” (`plan/phase8/reviews/a-gate-brief.md:33,48`), and the completed record confirms that (`a-gate-sol.md:3`).

**Concrete cost:** An agent either launches an unnecessary extra reviewer despite its brief, or silently violates the universal procedure.

**Replacement:**
> One fresh self-review round is the default. For a non-risky step, the orchestrator’s brief may explicitly waive it and name the remaining review. Risky-step test-plan review and independent code review remain required.

### R10 — stale · minor — Mutation timing no longer describes the current gate

**Section/quote:** §7: “about 8 minutes”; “about 15–20× faster.”

**Evidence:** `AGENTS.md:95` retains the older timing. Current handoff says approximately twenty minutes remotely (`handoff.md:68`). The committed gate measured **1,268.2 seconds**, approximately 21 minutes (`plan/phase8/reviews/a-gate-gate.json:174–180`).

The historical speedup ratio was not independently verified.

**Replacement:**
> Use handoff §5 for current gate duration and operating details. Remote mutation runs remain sharded across sim02/sim03.

Keep the CPU-limit and load-related false-kill warning.

### R11 — stale · note — The fixed v3 citation count has drifted

**Section/quote:** §1: “the design documents cite 22 `v3/...` paths.”

**Evidence:** Applying the documented extraction expression to `design`, `plan`, and the Vision index produced **24 unique citation strings**. This inspected citation text only, not any `v3/` target.

**Replacement:**
> Never delete the `v3/` worktree: design and planning documents cite it as behavioral evidence. The citation-check command is in handoff §7.

Delete the fixed count from both copies.

### R12 — holds-back · minor · INFERRED — Onboarding reading is mandatory for every implementation task

**Section/quote:** §2: “Reading order for implementers: index … Part 01 … §65 … applicable accepted specs and phase overview … §58.”

**Evidence:** `AGENTS.md:45` has no first-session or task-relevance qualification. Current sizes include:
- Vision index: 32 KB.
- Part 01: 15 KB.
- Phase 8 overview: 119 KB.

These figures come from `wc -l -c` on the named files. Much of the overview is historical record rather than the active brief.

**Concrete cost:** A bounded fix can require repeated ingestion of broad architecture and phase history before reaching its actual contract. The cost is inferred; no task-duration measurement was made.

**Replacement:**
> On first joining the project, follow the Vision’s reading guide. For subsequent tasks, read handoff, the brief, affected contracts, and connected code. Consult broader architecture and phase history when the task touches those boundaries.

### R13 — holds-back · minor · INFERRED — The universal-claim grep has no bounded completion condition

**Section/quote:** §9 checklist: “a grep over every position for each ‘every / only / all’ claim.”

**Evidence:** `AGENTS.md:139` does not define which claims, source paths, or positions count. The later maintainability rule does scope findings to problems introduced or worsened by the step (`AGENTS.md:148`), but this checklist item does not inherit an explicit boundary.

**Concrete cost:** A reviewer may scan unrelated code and old specifications indefinitely, or satisfy the rule ceremonially by searching English words without checking their semantics.

**Replacement:**
> For each new or changed universal claim, identify its relevant callers, state transitions, or data positions. Search those paths and report any counterexample or coverage gap.

### R14 — holds-back · minor — “Formatting in its own commit” also captures normal new code

**Section/quote:** Safety core; §9: “Formatting goes in its own commit, never mixed with a structural change.”

**Evidence:** `AGENTS.md:12,164` does not distinguish formatting-only churn from formatting code being added or changed. Meanwhile, implementers must format before reporting, and the gate requires formatted code before closure (`AGENTS.md:136,167`; `plan/maintenance/tools/gate.py:275`).

**Concrete cost:** A literal implementation separates a functional change from its normal formatting, complicating staging and potentially requiring an intermediate unformatted commit.

**Replacement:**
> Format new and modified Rust code before review. Put unrelated formatting-only changes in a separate commit; do not mix broad formatting churn with structural changes.

## Checked and found correct

- HEAD is `d549859`, branch `main`, with a clean working tree.
- `origin` points to `k3komatsu/Ez-SDRv4`; the separate `v3` remote points to `k3komatsu/Ez-SDR`.
- The listed historical branches remain represented by remote refs; no stale branch-history claim was established.
- Phases 1–7 acceptance, Phase 7’s date, and the outstanding spec-format migrations agree with handoff and their records.
- All relative Markdown link targets in AGENTS.md exist.
- The Vision contains eleven parts and sections §1–§68; §65 contains 42 invariants.
- `CLAUDE.md` imports AGENTS.md rather than maintaining competing instructions.
- Gate flags, manifest checks, known-survivor handling, and per-checkout Docker isolation match the implementation.
- `anchor.py` and `rules.py` support the stated Rust-token/non-Rust-exact matching; handoff size limits match.
- Remote mutation list/ID syntax, upload exclusions, shard isolation, and default four-CPU limit match the runner.
- sim02 and sim03 are reachable directly, each reports 32 CPUs and approximately 60 GiB RAM, and Docker works without sudo.
- The main Cargo target configuration and executable post-commit cleanup hook exist as described.
- The external venv contains Python 3.14 and numpy; the gate uses it and a Python 3.9 container.
- Installed OpenCode CLI help confirms the documented `run`, `--standalone`, session, model, and `--auto` flags.
- Haiku 5.5 and Sol 6.1 model IDs exist in the available model catalog.
- The safety rules for remote-only mutation, RF confirmation, one editor, PID-based termination, and protected histories should remain.

**Overall: patch the file.** Keep the safety core and architectural constraints; scope operational rules by role and make handoff the sole home for changing model and timing details.

Not checked dynamically: live Pages configuration, historical model-alias behavior/pricing, or full build/mutation gates. Some read-only Git ancestry/tag queries were denied by project permissions.
