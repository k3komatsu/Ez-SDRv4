# Bench tools (plan 50 §2 row 2) — dispositions and counts

S4 (overflow gap at 200 MS/s) and S5 (two-channel receive lead) bench tools, and C2's deferral. Way of working: a Sonnet subagent supervised Go DeepSeek; Sol 6.2 reviewed the code ([tools-sol-code.md](tools-sol-code.md)); Opus and GPT-6.1 Terra re-checked the fix round side by side ([tools-recheck.md](tools-recheck.md), [tools-terra-recheck.md](tools-terra-recheck.md)). Brief: [tools-brief.md](tools-brief.md); reports: [tools-report.md](tools-report.md). Gate: `tools-gate.json` (beside this file). No issue closes.

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Sol 6.2, code | 2 | 3 | 3 | none counted (test tooling; F2 is a missing test) |
| Opus re-check | 0 | 3 | 3 | none |
| Terra re-check | 0 | 1 | 4 | none |

## Decisions (orchestrator)

- S4 at 200 MS/s: the overflow trial captures only what S4 reads (`overflow_capture_samples`: after + min(stall, 2×fill) + 100 ms, capped at 125 M samples = the largest capture that met RS-8a on the bench); a trial whose capture ends before the hole closes is Incomplete → INCONCLUSIVE. Accepted. The SIGSEGV after an RS-8a abandon is INFERRED to be UHD's exit-time crash (`multi_usrp_rfnoc.cpp`, same ip three times), not the Sink's or the Module's; not pursued.
- S5 two channels: a selector built from the device's slot A and slot B receive fronts, restored afterwards, with a `set_subdev_spec` fallback. Accepted.
- C2 at 100/200 MS/s: not solvable by the tool without a measured host→radio margin; the verdict stays INCONCLUSIVE with its margin (no false PASS). Decided: C2's 100/200 MS/s cells leave the next bench's plan until bt2; C2 stays at 1 and 10 MS/s (PASS on both boards, 2026-11-18).

## Sol 6.2 code review (sol-code.md): FAIL, 2 majors, 3 minors, 3 notes — dispositions (orchestrator)
- F1 major adopted: S4 stops depending on an IQ capture for its evidence. The trial reads the overflow gap (position, length, loss, flags) from the PUBLISHED block headers through a test-only recording link (the S9 pattern, `s9bench.rs`'s FirstBlockLink, extended to record every header's index, flags and lost), so no capture size can end before the resumed block and nothing large is written at 200 MS/s. If S4 still needs a capture for something else, keep it minimal and say why. Remove `overflow_capture_samples` if it becomes unused (AGENTS §9 leftovers).
- F2 major adopted: the default suite pins the wiring: a FakeDevice rehearsal of the S4 trial that books the same boundary (a scripted overflow, the gap read from the recorded headers), plus an assertion on what the real S4 spec asks of the Sink (no capture, or its size).
- F3 adopted: S5's selector override uses the existing scoped selector helper (save and restore on unwind); the override and the device's RX map are kept distinct.
- F4 adopted: S5's decision line carries the `set_subdev_spec` error; a #2 rehearsal (UBX in A, empty B, the one-channel RX map the earlier passes leave) gives one two-channel INCONCLUSIVE line.
- F5 adopted: bench.md's S4 text (tail, Incomplete, SIGSEGV observed after the timeouts, teardown attribution INFERRED); the stale `+250 ms` comment.
- F8 adopted: the next bench's checklist marks C2's 100/200 MS/s cells deferred to bt2. F6, F7 notes.
- Re-check of the fix round, Opus and Terra side by side: F1–F5 and F8 CLOSED by both; both PASS. Both: bench.md said the header map is "the same `ContinuityMap` the Sink makes" — it uses the same builder but runs to the Run's end and has no link drops (minor, fixed by the orchestrator). Opus only: `HeaderMapsState.current` carried an unread domain and channel count (minor, removed by the orchestrator); the recorder's fold copies the capture Sink's `push_continuity` and its link wrapper repeats S9's `FirstBlockLink` (minor, to the Phase 8-end review); notes — the #2 fake refuses the widening while hardware may accept B as unknown (both give one INCONCLUSIVE line); the mixed-fake widening assertion alone is weak (TOOLS-05/32 die by other assertions); no stall near the fake's threshold is tested. Terra only: the per-block lock and fold may affect 200 MS/s timing, and RS-8a at 200 MS/s stays open until the bench (INFERRED; note). Time: Terra 1.5 min; Opus 4.5 min.
