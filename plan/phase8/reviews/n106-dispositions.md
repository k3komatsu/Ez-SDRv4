# #106 — dispositions of every finding

Step: a host-fed repeat resumes after every drop (the stutter bound b′ and its zero case removed), and the UHD Module's per-Run records are bounded per kind (`record_bound`, selector `record_rows_per_kind`). Risky (a lifecycle and the Manifest's Provider records). Decisions: [n106-decisions.md](n106-decisions.md). Design: [n106-design.md](n106-design.md) (Opus, effort max), reviewed by Fable 5.2 ([n106-fable-design.md](n106-fable-design.md); 292 683 subagent tokens, 44 tool uses, about 19 min). Test plan: [n106-test-plan.md](n106-test-plan.md) (v2, Sonnet), reviewed by Sol ([n106-sol-plan.md](n106-sol-plan.md)). Implementer: Claude Code Sonnet 5.5 subagent (effort high), two sub-steps, each reviewed by a fresh Opus subagent (effort high); code reviewed by Sol ([n106-sol-code.md](n106-sol-code.md)); the fix of Sol's major re-checked by Opus. Briefs: [n106-briefs.md](n106-briefs.md).

| Source | Finding | Severity | Disposition |
|---|---|---|---|
| Opus design draft | P-1: the Kernel keeps every delivered event for the Run (the ring bounds only between drains); the orchestrator had told the owner otherwise | premise | owner Q1: accept now, KL-46, freeze-review item (handoff §4) |
| Opus design draft | P-2, P-3: TimeError rows per drop 1–48 (not 10–30); bare UHD sends untimed | premise | recorded; texts use the measured figures |
| Fable | M1: a gap can be in no event (expired-check resume; one report retiring two entries; a second channel's TX_UNDERFLOW without plan) | major | owner F-M1: promise reworded, `stats.tx_resumes` and `tx_gap_samples` |
| Fable | M2: the bound cuts rows of a clean Run (`BurstAck`, `tx_stop_applied`) | major | owner F-M2: Action-only kinds exempt by name, a booked burst's ACK never bounded |
| Fable | M3: an environment variable would be read nowhere on the bench path | major | owner F-M3: selector key `record_rows_per_kind`, checked before the device opens |
| Fable | m1–m8 (rejected key, flush ACK key, wording, 64 kinds, push-only type, per Run, wrong-m loop, more closes) | minor | adopted (decided by the orchestrator, decisions.md) |
| Fable | omissions (a test, four rows, five texts, a reader) | minor | in the plan |
| Sonnet test plan | OQ-1…OQ-11 | questions | decided by the orchestrator (decisions.md) |
| Sol plan review | 1: a booked switch joined onto a resumed part loses its ACK | major | adopted: W9, the join clears `part` |
| Sol plan review | 2: closing records at abandon and failed reopen not tested when saturated | major | adopted: W4d, W4e (rebuilt in B's fix round) |
| Sol plan review | 3: N106-27's killer never runs its arm | major | adopted: W5 |
| Sol plan review | 4–8 (scripts, MT3D-55, B6 oracle, reader guard, texts) | minor | adopted |
| Opus review A | 1: "carried once per resume" false when one report retires two entries | minor | "at most once" (owner told); later superseded by Sol's code major |
| Opus review A | 2–6, 8–11 (texts, a stale comment, rate bound tied to m, test line, independent gap sum) | minor/note | fixed (8 in B) |
| Opus review A | 7: `remember_part` merged | minor | accepted by the orchestrator; B re-added the bit |
| Opus review B | 1: W4d/W4e closed only the booked record, so a wrong key at two writers survived | major | fixed: rebuilt on a resumed part, rows N106-37/38 retargeted and killed |
| Opus review B | 2–5 (closing record closed by a switch, booked-ACK wording, EA-7 scope, KL-46 wording) | minor | fixed |
| Opus review B | 6: flush ACKs of a clean Run are bounded | minor | accepted by the orchestrator, KL-46 says so |
| Opus review B | 7–10 (W7 floor, W9/W2 shortcuts, one local, test line) | note | fixed |
| Sol code review | 1: a joined switch makes two TIME_ERROR events with the same plan, against "at most once" | major | owner: code kept, promise reworded (each `late_at_device` event of one dropped device burst carries the plan; count with `tx_resumes`); test asserts both plans, row N106-40 |
| Opus re-check | F1–F5: the TX_UNDERFLOW-only rule stated generally, "once" after a join, a lowercase start, a note | minor/note | fixed by the orchestrator |

Counts: majors 9 (Fable 3, Sol plan 3, Opus B 1, Sol code 1, plus the draft's premise P-1, which the owner decided as Q1); minors and notes as listed. Mechanisms (`plan/maintenance/failure-mechanisms.md`): none counted — b′ was a deliberate rule the bench data argued against, the majors are test gaps and text promises found before landing, and unbounded records fit no recorded mechanism.

Rows: retired MT3D-10, -11, -12, -13, -27, -28, -53, -79, -80, -81; retargeted MT3D-15, -26, -37, -38, -55; re-anchored P350, P281, P1209, MT3-07, MT3-12, MT3-16, MT3D-09, B25, B26; new N106-01 … N106-40. Full lists before Sol's code review: 2996/2997 (the known survivor phase7/G09).

Not measured: the new behaviour on hardware (a repeat that resumes after every drop, a part dropped late every time: bench cell C7), row and memory sizes, a Run past N on the bench.

Gate: the first run (2026-11-17) failed one test outside the step, `mt3_c4_rehearsal_the_fake_drops_b_before_its_own_limit_and_plays_it_at_and_after`, which fails 2 runs in 10 on main before #106 as well (issue #107, filed by the orchestrator); every other step passed, mutation 2997/2998 (phase7/G09). The gate was run again.
