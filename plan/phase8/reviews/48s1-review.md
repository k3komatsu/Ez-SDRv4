# Note 48 S1 — review record

S1 (note 48 §4, issue #78 parts a–c):
- A device burst's first send carries at least four samples per channel. A 1–3-sample burst goes as one timed SOB+EOB padded with zeros to four, and its front is the start + 5. A first send that a clock's cut would leave short is refused.
- The T3 flush is four zeros. The reports of its own channels that come before its ACK or deadline are recorded as the flush's, not the Run's.
- FakeDevice models an undersized first packet (a timed one is dropped through its EOB; an untimed one underflows) and the BurstAck at the burst's end + 1 tick.
- The BurstAck correlation is strict (`<`). The reopen front is the flush's end + 1.
- MockRadio keeps the padded front (MR-17), on Stop too (MR-25). A first send counts as handed only when four samples, or the whole short burst, are in the window.
- A first send that waits for window space is decided again by its late policy once it is inside the device lead (UR-21).
- Every send stays timed (owner, note 48 B1).

Implementer: Haiku 5.5 (OpenCode2):
- test plan rev 1–4;
- implementation (rev 5) and its continuation for the planned tests left out (rev 6);
- fix rounds 1–3 (rev 7–9).

Self-review: free DeepSeek, with Go DeepSeek when the free tier refused. Reviews: Sol 6.1 on the test plan and the code; Claude Opus re-checks after fix rounds 1 and 2.

Records:
- briefs: `48s1-brief.md`;
- test plan: `48s1-test-plan.md` (rev 9, §17 is the implementation record);
- rulings: `48s1-dispositions.md`;
- reviews: `48s1-sol-plan.md`, `48s1-sol-code.md`, `48s1-opus-rechecks.md`;
- gate: `48s1-gate.json`;
- renames: `48s1-renames.txt`.

The hardware facts are in `plan/phase8/bench-results.md` §「2026-11-05（続き）」 (the C++ reproduction).

## Findings by source and severity

| Source | Major | Minor | Mechanism (`plan/maintenance/failure-mechanisms.md`) |
|---|---|---|---|
| Sol, test plan rev 2 | see `48s1-sol-plan.md`; rulings R1–R10, then N1–N5 after the owner kept every send timed | — | none (plan gaps) |
| Orchestrator, implementation | 1 (most planned tests not written; continuation run) | 0 | none |
| Implementer's own test (continuation) | 1 (a first send that waited past its start was abandoned with no TIME_ERROR) | 0 | none |
| Sol, code | 3 P2 (Mock lost the padded front on Stop; a waiting first send rechecked after its start, not its lead; a bench assertion that accepted flush Underflows) | 1 P3 (UR-23 text) | P2-1 `radio-rule-implemented-twice` (Mock vs Module front) |
| Orchestrator, code | 0 | 1 (brief F3 untested) | none |
| Opus re-check, fix round 1 | 1 P2 (the Mock decided at receipt against handed bursts only and played a burst the Module drops after a short held burst) | 3 P3 | `radio-rule-implemented-twice` → issue #79 |
| Opus re-check, fix round 2 | 1 P1 (the round-2 ruling D1 made the Mock drop ordinary back-to-back bookings; reproduced) | 2 P2, 3 P3 | caused by the ruling; D1 reverted |

## Decisions

- Owner (2026-11-05): every send stays timed (B1 dropped; note 48 §5).
- Owner (2026-11-05): revert D1 and fix the receipt-versus-dispatch divergence in note 47 stage 5, where overlapping bookings are refused and a short burst occupies its padded range `[s, s+5)` on both sides by one rule (#79).
- Rejected: D5 (a timing whose window is shorter than the lead + 4 samples makes a SendAsap burst move forever). The production profile is 10 ms against a 2 ms lead.
- #78 stays open until a Module-path bench confirms the four-zero flush draws no Underflow and the padded short sends play (`hw_b8_*`, `hw_p70_*`, `hw_s1_short_first_sends`).
- #78's mechanism is `correlation-by-partial-key`: the flush's reports were attributed to the Run by channel alone, without the flush they belong to. The fix follows note 39 §6.6 (correlate by channel and the owner's window), so it needs no new note.
