# N0 (2026-11-09): the switch mark only on SendAsapAndFlag (X1), and RM-15's footprint sentence

First stage of direction D′ (design note [49](../49-time-and-booking-inventory.md) §12). Not risky (no test plan). Implementer: a Claude Sonnet subagent (owner, 2026-11-09; self-review waived). Reviews: the orchestrator, the full mutation run before review, Sol's code review, an Opus re-check of the major fixes. Brief: [n0-brief.md](n0-brief.md). Gate: [n0-gate.json](n0-gate.json).

## What changed
- X1: `ezsdr_radio::staging::is_switch(repeat, late_policy, metadata)` is the one decision both Providers call; a marked repeat under any late policy other than SendAsapAndFlag is refused at receipt by RM-13's metadata check (Session `start_repeat` is always SendAsapAndFlag; a Spec action could carry DropAndFlag). The UHD owner's take-time DropAndFlag refusal and `Bookings::install`'s `moves` parameter are gone: a switch the cursor passed is always moved (UR-38). The owner's remaining refusal (no handed cursor) keeps its row label `refused_past_boundary`; its reason text and the `SwitchOutcome` docs now name the reachable cases.
- RM-15: the footprint sentence (§11.2 (a)), written to match today's code: lateness (RM-14) is decided before the footprint; a footprint is a booking's samples and its own close up to its front; a finite booking UR-22 abandoned keeps it, an abandoned repeat keeps it up to its cursor plus three, a chain a Stop cuts keeps the region RM-16 leaves it; a booking that is not late and meets a footprint is refused; the Stop region of cancelled bookings and dispatch lateness follow RM-16 and RM-14.
- KL-39 (owner, 2026-11-09, option A): a burst booked inside an abandoned burst's unplayed rest is refused; Ceiling. Shrinking the region is the contract change C6 of note 49 §12, made only if the owner later approves it.

## Findings
| Source | Finding | Severity | Disposition |
|---|---|---|---|
| Implementer | the brief's RM-15 clause "refused whatever its late policy" is false for a late SendAsapAndFlag booking (RM-14 places it past) | — | sentence written to the code |
| Orchestrator | N47S5-90 removed as "equivalent": not equivalent, the no-cursor refusal still reaches it | major (test) | row restored, the test asserts the refused switch is not `playing` |
| Full mutation run | N47S5-41 and N47S5-92 survived (the DropAndFlag killers were gone) | missing tests | N47S5-92: finite marked SendAsap case added to the UHD test; N47S5-41: the TIME_ERROR assertion could not see it (the mutant moves to the same boundary but records `moved_to`), pinned by the pure `rm_37_install_at_the_boundary_is_not_a_move` and the row retargeted |
| Sol, code | 1: lateness is judged before overlap on both Providers, so "refused whatever its late policy" is false for late DropAndFlag/RejectAtPlan | major | adopted; sentence rewritten, ordering tests added for both Providers |
| Sol, code | 2: an abandoned repeat (and a Stop-cut chain) is retained up to its bound, not a movable Stop region | major | adopted; sentence rewritten; the behaviour itself is KL-39 (owner) |
| Sol, code | 3: `SwitchOutcome::Refused` docs and the reason still described the removed branch | minor | fixed |
| Opus re-check | PASS WITH MINORS: RM-15 restated RM-14/RM-16 and said "up to B" (a kept short head holds `max(B, own)`); "follows its late policy" clashed with the Stop region's refusal; the Stop half of the Test line was covered; `Switch` doc leftover | minor ×4 | all fixed |
| Opus re-check | note: a switch whose repeat is dropped at dispatch reads `refused_past_boundary`, while RM-37/UR-38 say a drop at dispatch cancels it (predates N0) | note | #84 (fixed structurally in E2a; O1 marks it as a known divergence) |

Mutation rows: added N0-1, N0-2; removed N47S5-76 (anchor gone); retargeted N47S5-41, N47S5-42, N47S5-92, N47S5-93; N47S5-90 kept with a new killer.

Mechanism: Sol's majors were a spec sentence that did not match the code (not counted). The behaviour behind major 2 — an abandoned burst's unplayed rest refuses a booking that device state, not an accepted booking, keeps from its start — fits `mech:refusal-where-late-policy-decides` (third count); note 49 §11.2 is its design note and C6 its candidate change.
