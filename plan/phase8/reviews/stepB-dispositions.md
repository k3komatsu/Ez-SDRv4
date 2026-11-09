# Step B (2026-11-09): the front after a closed device burst of at most 32 samples is its padded end + 10 (#81, #82)

Owner decision (2026-11-09): one value, + 10, after a closed device burst of ≤ 32 transmitted samples (1–3 padded to 4). Evidence: [bench-results.md](../bench-results.md) "2026-11-09（午後）" (UHD-only `tx_repro` sweep). Implementer: a Claude Haiku 5.5 subagent (owner, 2026-11-09). Sub-steps B1 (record: `ezsdr_radio::staging`, MockRadio, RM-15/RM-16, KL-36) and B2 (uhd-tx, FakeDevice, hardware tests, UR-22/23/33, KL-13/KL-38), each with a cheap read-only Haiku review, then Sol once at the end. Brief: [stepB-brief.md](stepB-brief.md); accepted test plan (revision 3 + orchestrator amendment 3a): [stepB-test-plan.md](stepB-test-plan.md); gate: [stepB-gate.json](stepB-gate.json) (run 2; run 1 failed on two needless clones and load flakes, below).

## Owner decisions during the step
- A kept short head keeps its own front at a Stop (Q2 rejected: a short head is handed as one closed send, so a Stop never cuts it).
- The abandon close and an acknowledged T3 flush also front at their end + 10 (the abandon close's two zeros end at cursor + 2, so cursor + 12).
- Not adopted: per-length values; a FakeDevice exception for a short closed B (the fake is stricter than the X300 there, VERIFIED that the X300 plays it).

## Findings and dispositions
| Source | Findings | Disposition |
|---|---|---|
| Sol, test plan | 1 × P1 (Q2 payload-end test unsafe), 6 × P2 (FakeDevice refusal through EOB; B1 red list; configure-refusal exception; parity geometry; raw long-B regression; text), 1 × P3 | all adopted (plan rev 2) |
| Opus re-check, plan rev 2 | 2 majors (B1 red list still short; window-edge dropped head reachable at production windows), 5 minors | adopted (rev 3) |
| Opus re-check, plan rev 3 | 2 majors (tx.rs:6307 belongs to B2: value wait follows the owner's front; per-Provider pinning of the window-edge Stop), minors | adopted as orchestrator amendment 3a; then PASS WITH MINORS |
| Cheap review B1 | 2 × P1 (seq 5a cross-Provider test; N-P6 rows), 3 × P2, P3s | adopted except design/18 text (moved to B2) and the wall-clock parity test (gate decides); the reported Mock divergence was a test geometry error |
| Cheap review B2 | 1 × P1/P2 (abandon front + 3 vs fake + 10 → owner decision + 10), 6 × P2 (dropped armed close; ratio < 80; burst count reset; `set_time_zero`; flush counts; UR-33 text), P3s | all adopted; > 32 owner + 1 vs record + 2 recorded only (pre-existing) |
| Full mutation after B2 | N49B-10 survived (killer reached the held-tail close) | test rewritten to drive the non-SOB close path; killed |
| Sol, code | 2 × P2 (a short head left by a configure refusal lost its front at a Stop; a refused burst's close cleared the FakeDevice guard), 1 × P3 | adopted, failing tests first |
| Opus re-check, Sol fixes | 1 × P2 (over-hold: the record kept long joined pieces' front behind a short head), minors | adopted (`Chain::own_fronts`); then PASS WITH MINORS; minors fixed by the orchestrator |
| Gate run 1 | clippy (two `.clone()` on `Copy`); load flakes: `s15_parity_untimed_booking_after_stop` (wall clock), baseline of N47B-19/-36, P824 (52 s fake RX/measurement tests) | clones fixed; flakes passed alone and in run 2 (pre-existing, untouched by the step) |

## Mechanisms (`plan/maintenance/failure-mechanisms.md`)
- `planned-state-read-as-actual`: Sol code P2-1 and the re-check's over-hold — the receipt-time record and uhd-tx's sends disagreed after a configure refusal. This is the drift note 49's D′ removes.
- #81 itself: the FakeDevice was looser than the X300 (it played a burst right after a short closed burst) — the hardware envelope rule (AGENTS §3); no mechanism label yet (first of its kind).
- `wall-clock-test-oracle`: `s15_parity_untimed_booking_after_stop` and the 52 s fake tests under 16-shard load (pre-existing; not counted against the step).

## Open after the step
- Bench: re-run (c)1 with the new front, (d) R5-m5 at c + 13, and the long-B raw regression N-H1 on both boards before closing #81 and #82.
- The > 32 owner + 1 vs record + 2 front mismatch (pre-existing); the abandon/flush + 10 and the refused-continuation rule in the fake are INFERRED (unmeasured).
