# Note 48 S1 — Opus re-checks

Claude Opus re-check reviewer (`general-purpose`, effort high), read-only, with probes in throwaway copies.

## After fix round 1 (C1–C5)

Verdict: FIXES NEED CHANGES. C1 and C2 were correct; there was one new P2 next to C1.

**C1**
- The Mock and UHD agree on the padded front `s + 5` for a short burst Stopped before or after hand-over, and for a repeat burst.
- A burst of exactly four samples: UHD `s + 5`, Mock `s + 6`. MR-17 allows a later Mock bound.
- The `mock_channel.rs` expectation change matches UHD's mirror test `ur_23_stop_does_not_send_unhanded_window_edge_payload`.
- The close-path floor copy only matters under Stop and can never decide a booking, so delete it. → D2.

**C2**
- The verdict comes from one decision, `dispatch_start`. The re-judging is bounded.
- There is no double TIME_ERROR, and no pending-ACK record is lost.
- Continuations are unchanged.
- Both policies are tested at the lead boundary.
- UR-21 does not say when a waiting first send is re-decided. → D3.

**Findings**
- P2: the Mock decided a booking at receipt against handed bursts only. Sequence: a one-sample A held outside the window, then B at `s + 1`. UHD sends A padded and drops B; the Mock plays B. Either receipt order. → D1, then #79.
- P3: the Mock counted a first send handed before four samples fit (→ D4).
- P3: a timing window shorter than the lead + 4 samples moves a SendAsap burst forever (→ D5, rejected).
- P3: no grouped two-channel test (→ D6).
- Note: a second SendAsap move overwrites `late_by` (pre-existing).

## After fix round 2 (D1–D6)

Verdict: FIXES NEED CHANGES. D2–D6 were correct, but D1 as ruled broke the common case.

**P1 (reproduced)**
- D1 made every held burst that has not been handed over reserve the time up to its end + 2.
- On UHD, a booking at or before that end cuts the burst, or continues it when adjacent.
- The Mock therefore dropped or moved these, which UHD plays:
  - A = 16 at `s` with B adjacent at `s + 16`;
  - B inside A at `s + 8`;
  - A, then C at `+5`, then B at `+17`.
- The cause was the ruling's wording, not the implementation.

**Other findings**
- Q1: the D1 cases themselves now matched. The parity tests compared counts only, so a SendAsap start could be wrong unnoticed.
- Q2: the insert assertion was unreachable. The decision code was duplicated in `redecide_displaced` (P3).
- Q3: a real cascade difference (short A, long B at `+2`, C at `+12`, SendAsap). The Mock is stricter there, from the same root.
- P2: a two-sample A with B at `s + 1` radiates one sample of A on UHD and two on the Mock.
- P3: a first send cut by a held burst — UHD hands it over at `min(cut − next, 4)`, the Mock waits for `min(len, 4)`.
- The mechanism is `radio-rule-implemented-twice`, past its third strike. Note 40 D5 had the Mock infer no device-burst framing.

**Outcome:** owner decision E1. D1 was reverted, and the divergence is #79, fixed in note 47 stage 5.
