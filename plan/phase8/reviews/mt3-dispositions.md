# Maintenance table 3 — a burst the X310 finds late: review record and dispositions (2026-11-14)

The X310 drops a late, all-timed device burst whole. The FakeDevice now does the same, through a drain that is stricter than the device at its boundary. Device reports alone retire correlations: a `TimeError` at `t` retires every pending device burst on that channel whose first instant is at or before `t`, with one `late_at_device` per Kernel burst. A late one-shot burst is abandoned, as one that underflowed is. A repeat no longer ends after a late drop, an underflow or an expired timeline. It resumes at `R`, the first point of its period lattice at or after `max(busy + 10 ms, now + lead, front)`, and splits its record. Its events carry `resumes_at`, and a `tx_resume` timing row records the plan. A part that plays less than the stall before it ends the repeat (b′). A switch across the gap moves to the next safe boundary, and a missed `R` moves on. A bench cell C1 checks the drop on hardware, with its FakeDevice rehearsal. KL-13 is rewritten, KL-16, KL-39 and KL-41 gain a line each, and KL-45 is new.

The step was risky: it changes a queue, a deadline and a lifecycle, and the boundary between the fake and the X310. AGENTS §9 step 0 ran (Opus drafts, the owner's direction, Fable 5.2 reviews), then the test plan, Sol's plan reviews, sub-steps A, B, C, D1 and D2 with a fresh Opus review each, the full lists at each sub-step, and Sol's code review. Branch `work/mt3` from `5121cdf`, squashed and rebased onto `e498b08`. Implementer: Claude Code Sonnet 5.5 subagents, effort high ([mt3-briefs.md](mt3-briefs.md)). Design: [mt3-design.md](mt3-design.md), [mt3-fable-design.md](mt3-fable-design.md). Every decision with its date and the owner's words: [mt3-decisions.md](mt3-decisions.md). Test plan: [mt3-test-plan.md](mt3-test-plan.md) (v6).

## Owner decisions (details in mt3-decisions.md)

- **Direction:** A, and O1–O7: a late drop takes the whole device burst; `late_at_device` means "not one sample was sent"; there is a device-behaviour KL; the bench cells come after 11-15.
- **Continuing after an underflow:** D9b-1b (a), which continued the rest of a one-shot burst after an underflow, is dropped.
- **Repeats resume:** a repeat resumes instead of ending, after a late drop as after an underflow (the owner's proposal). The owner decided:
  - R's formula from the Module's own `busy`, with m = 10 ms;
  - the b′ stall rule, where equality resumes;
  - split records;
  - the triggers and the endings;
  - `resumes_at` plus one `timing` row;
  - a switch across a gap moves to the next safe boundary (the owner's proposal);
  - a missed `R` moves on;
  - the KL and bench texts.
- **The `switches` row:** its `boundary` is the boundary the switch settled at (A).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Fable 5.2, direction | 3 | 3 | — (design review, before code) |
| Fable 5.2, repeat-resume addendum | 5 | — | — (design review, before code) |
| Sol, test plan §1–§8 (v2) | 1 | 3 | — (C1's verdict could pass without RX evidence: a plan defect) |
| Sol, test plan §9 (v4) | 1 | 4 | — (b′ not applied to a delayed two-entry Underflow: a plan defect) |
| Opus, re-checks of the plan (v3, v5) | 0 | 7 | — |
| Opus, sub-step A and its two re-checks | 3 | several | the fake's drain boundary looser than the X310, three times: the three-strikes note below |
| Opus, sub-step B | 0 | 3 | — |
| Opus, sub-step C and its re-check | 1 | 4 | — (the "late" burst was about 13 ms late, so a per-packet drop could pass: a test without power) |
| Opus, sub-step D1 | 1 | 5 | `planned-state-read-as-actual` (a switch whose boundary fell in the gap reached the `stale` branch), closed by D2 |
| Opus, sub-step D2 | 0 | 6 | — |
| Sol, code ([mt3-sol-code.md](mt3-sol-code.md)) | 2 | 1 | M1 `failure-collapsed-into-value` (a lost sequence while paused became `Dropped::Nothing`); M2 `correlation-by-partial-key` (a pause read as a report's reach without the identity of the part that made it) |
| Opus, re-check of Sol's fixes | 0 | 2 | — |

The full lists ran at every sub-step: A–B (with C) 2795/2797 (G09 and J154, a TM-6 scan miss in B's helper, fixed in D1), D1 2834/2835, D2 2872/2873; only G09, the known survivor, remained.

## Three strikes: the fake's drain boundary (AGENTS §6)

Fable's M3, sub-step A's review M1 and its re-check each found the fake looser than the X310 at the end of a drain: short sends, the DUC's wait after each EOB, and bursts dropped behind a late one. The design question was whether the fake should model the drain boundary at all. The orchestrator decided yes: without it, a burst dropped behind a late one plays on the fake, which UR-33 forbids. The term-by-term refinement against INFERRED device costs stopped. The pad is now the sum of every cost term the evidence names, each traced to its source: `ceil(S/100) + 2·P + K·(g + ceil(10 µs × rate))`. A new term is then an addition, not a redesign. The fake is stricter than the device at the boundary by construction, and no test pins a device value there (UR-33). Bench cells C4 and C7 measure the terms.

## Dispositions (decided by the orchestrator unless marked)

- **A′ (reports alone retire correlations), the drain record of its own (Q12), and the pad as a sum:** see the decisions record.
- **D1 review M1:** held over to D2. D1 was a local commit only, and D2 closed it: the switch moves to `R`, its row settles, and a later switch is accepted.
- **Q-D5, re-decided after the D2 review:** a part that starts late (a moved switch or a missed `R`) carries `send_asap` in its burst record as `dispatch_start` does (RM-14). It is no longer told only by the event.
- **Sol M1:** a lost sequence while paused ends the repeat, as when it runs.
- **Sol M2:** `Paused.source` is the first instant of the device burst whose drop made the pause. The b′ rule applies only to a report that reaches that part, or while a device burst is open. Both counter-sequences became tests first.
- **Re-check note (d), accepted:** an older part's delayed Underflow no longer ends a later pause. The running case is treated the same, and UR-22 states it.
- **`hw_h4_load`:** a resumed repeat (`late_at_device` or `tx_resume`) marks the run `partial`.
- **`n47s5_front_rehearsal_on_the_fake`:** its failures under load predate sub-step A (5121cdf fails as well). It is the host-lateness marker by design, and is noted for the Phase 8-end review.
- **Bench cells C2–C7** (C7: a repeat stalled on the Module path, m swept 10/1/0.2 ms, a host that cannot keep up, a two-channel repeat) go to the bench-tools brief after 11-15. C1 is ready.
