# #92 — review record and dispositions (2026-11-12)

Issue #92: when uhd-control panicked, the UHD Module recorded only a `rejected` note — no event, a repeat kept playing, and the Run ended only after KC-21a's 5 s. Risk: **risky** (how a Run ends). Branch `work/n92` in a worktree (lane 2 of [plan 50](../50-remaining-work.md)), landing on `main` as one squashed commit.

## Process

- Test plan first (OpenCode2 `anthropic/claude-sonnet-5-5#high`, [n92-briefs.md](n92-briefs.md), [n92-test-plan.md](n92-test-plan.md)), written overnight while the direction awaited the owner.
- Sol's plan review ([n92-sol-plan.md](n92-sol-plan.md)): FAIL, 5 majors (a manual-clock self-deadlock in T4/T6c, a watchdog that could not see its deadlock, no Policy-`continue` safety case, no KC-21a pending-submit test, an RF oracle that overclaimed), the direction sound. All adopted; the orchestrator's answers to the plan's four open points followed Sol's recommendations.
- Opus re-check of the amended plan ([n92-plan-recheck.md](n92-plan-recheck.md)): pass with one major (tests racing `stop`'s 1 s wall-time join bound) and eight minors, folded into the plan; it endorsed writing UR-30's plan row in `Provider::stop` after the uhd-control join (one writer) and found that KL-2 does not fit a panic.
- **Owner decisions (2026-11-12):** the direction — a uhd-control panic is handled like a uhd-rx/uhd-tx panic (the `rejected` row and `device_lost`: one fatal DEVICE_LOST, streams ended, no new device call; the default Policy ends the Run at once), UR-16 amended; **no KNOWN_LIMITATIONS entry** (a panic is a defect; the behaviour lives in UR-16).
- Implementation (same implementer): rows N92-1..4, 6, 7, 7b, 7c, 8, 8b (killers checked in a copy outside the repository through rdev), P276 removed; UR-16 promises no immediate RF silence. Full mutation run: 2150/2151 (`phase7/G09` known).
- Sol's code review ([n92-sol-code.md](n92-sol-code.md)): PASS WITH MINORS, three minors, fixed.

## Gate

PASS ([n92-gate.json](n92-gate.json)), after a rebase onto `093343d`: 2298 tests, 546 UHD, mutation 2150/2151 (`phase7/G09` known; `sim02:7,sim03:7`).

## Findings by source and severity

| Source | Major | Minor | Note |
|---|---|---|---|
| Sol, test plan | 5 | several | — |
| Opus, plan re-check | 1 | 8 | — |
| Sol, code | 0 | 3 | — |

## Dispositions after the code review

- Minor 1, two integration tests' "repeat playing" precondition matched arm's four-zero flush: fixed (`tx_experiment_sends`).
- Minor 2, UR-16 promised the KC-21a timeout of the next Action after any thread panic under Policy `continue`: qualified to "when uhd-control has died" (after a uhd-rx/uhd-tx panic uhd-control still drains Actions).
- Minor 3, a stale UR sentence on uhd-control reading the end from the stop/loss booking: removed.
- Sol's remark that the implementer's row check (`mutrow.py`: `anchor.apply` in a copy outside the repository, run through rdev) was a process violation: rejected — the brief asks for exactly that to check a killer; `remote_mutate.py` ran the full lists.
- The transmit SampleClock stays open after a post-origin loss (pre-existing, also with `FakeFault::Lost`): Sol finds it no contract defect (UR-23 closes the clock only for a loss or stop at or before origin; TM-12 does not require it); #92 does not depend on it. No action.
