# Design note 48 S3 — #78's remaining cause: dispositions (2026-11-06)

Risky (the transmit lifecycle in the UHD Module and the boundary between FakeDevice/MockRadio and the X3x0). Order: brief → implementer's test plan (Haiku 5.5 `anthropic/claude-haiku-5-5#high`, OpenCode2) → owner decision D1 → Sol plan review → plan round 2 → Opus re-check of the plan → implementation → self-review (free DeepSeek timed out, the Go fallback hit a capacity refusal, the third attempt on Go DeepSeek ran: PASS) → Sol code review (FAIL) → fix rounds 1–3 with Opus re-checks after each → orchestrator's minor fixes → gate. Records: `48s3-brief.md`, `48s3-test-plan.md`, `48s3-sol-plan.md`, `48s3-sol-code.md`, `48s3-opus-rechecks.md`, `48s3-probes.md`, `48s3-gate.json`, `48s3-renames.txt`.

## What S3 does

- UHD Module (`provider/tx.rs` `send`): a device burst's first send that stays open carries at least `MIN_OPEN_FIRST_SEND` = 32 samples per channel (the send waits for 32 samples of window; the chunk's lower bound is 32, which replaces S1's first-send widening). What remains of a burst up to its end or a cut, at most 32 samples, goes as one closed send with no held tail; a booking at its exclusive end is decided by its late policy (owner D1). `MIN_FIRST_SEND` = 4 stays for the closed pad.
- FakeDevice (`device.rs` `Radio`, `Burst`): at interpolation ≥ 80, an open first packet of fewer than 30 samples followed by a packet of its burst at a later device tick underflows (one Underflow per channel), then each later timed packet gets a TimeError and the first later untimed one a TimeError, no BurstAck. S1's drop stays for closed 1–3-sample packets. `ur_23_a_burst_whose_window_first_takes_28_samples_plays_clean_to_its_end` fails with HEAD's `tx.rs` through FakeDevice's reports (the owner's 2026-11-05 request that #78 be reproduced in FakeDevice before the fix).
- MockRadio: note 40 D5's frontier unchanged (handed payload end + 2, S1's floor below four); MR-25's hand-over threshold 32. Parity tests state D5's relation: after a closed short burst, a booking at its end + 1 plays on the Module and is late on the Mock.
- Specs: UR-22, UR-23, UR-33 (design/18); MR-17, MR-25 (design/09); KNOWN_LIMITATIONS KL-36 and the device-behaviour line.

## Decisions

| # | By | Decision |
|---|---|---|
| D1 | owner | After a finite burst of ≤ 32 samples, a burst at its next sample is not continued (narrows note 46 F7 (b) to predecessors of more than 32). The Mock/Module divergence for bookings received before the window is issue #79 (widened to 1–32), fixed in note 47 stage 5. KL-36. |
| D2 | orchestrator | FakeDevice's rule fires only when the continuation arrives at a later device tick (the bench's no-gap passes); armed after the SOB's late refusal; a new SOB clears it. |
| D3 | orchestrator | S1's drop stays for closed 1–3-sample packets; open short first packets underflow. |
| D4 | orchestrator | Unmeasured edges (29 open samples, a delayed continuation after 4 open samples, closed 1–3 at 4–10 MS/s) recorded as such. |
| O1 | orchestrator | Two pre-existing inside-window divergences (B→A receipt order; a contiguous booking after a long held burst, Mock stricter) recorded on #79, not fixed, no known-divergence tests. |
| R3 | orchestrator | Fix round 3: the Mock returns to note 40 D5 (below). |

## Sol plan review (6, all adopted)

| # | Sev. | Finding | Disposition |
|---|---|---|---|
| 1 | P1 | the Mock's handed end + 2 drops an end + 1 booking the Module plays | adopted as "Mock end + 1 for closed short bursts"; **reversed in R3** (contradicted note 40 D5: the orchestrator's ruling error) |
| 2 | P2 | no preemption-cut tests; the 32 boundary written `< 32` | fixed: `<= 32` once; cut tests at 10/31/32; reopen at 31/32 |
| 3 | P2 | a planned test rewrite contradicted the D2 model | fixed |
| 4 | P2 | three rows' killers did not discriminate | resolved by removing the first-send widening (chunk ≥ 32); rows dropped |
| 5 | P2 | `ur_22_a_first_send_cut_short_keeps_the_reports_paired` would stall | fixed (window moved) |
| 6 | P2 | UR-23's continuation rule left false | fixed |

## Opus re-check of the plan (GO WITH LISTED CHANGES)

N1 (major, the Mock's Stop path +2 vs the Module's closed front): adopted, later reversed with P1 in R3. N2–N8 (minor: two FakeDevice tests to 32-sample first packets, one copy of the Mock boundary, drop N48S3-06, the 29-sample edge, Q1's red forced deterministically, Q5's booking order, `ur_22_a_first_send_is_never_undersized` strengthened): fixed.

## Self-review (Go DeepSeek, one round, PASS)

Fixed 8 (a missing 33-sample Mock case, a duplicate test, a double Underflow on a read between arming and continuation with test and row, an over-broad spec sentence, KL citing #79 outside Evidence, renames, a missing row, a test name); rejected 1 (UR-21/22 test allocation: the plan's proposal, not a rule).

## Sol code review (FAIL; 6)

| # | Sev. | Finding | Disposition |
|---|---|---|---|
| 1 | major | a second Stop loses the Mock's closed short front | fixed in round 1; code removed in R3 (D5 has no closed short front) |
| 2 | major | FakeDevice took "timed" from the DUC's residual time: an extra TimeError for untimed continuations | fixed: `DucPacket.stamped`; test with two channels |
| 3 | minor | UR-23/MR-17 text, history in UR-33 | fixed |
| 4 | minor | KL-36 workaround wrong for 1–3 samples | fixed |
| 5 | minor | two renames missing | fixed |
| 6 | note | no deterministic HEAD reproduction through FakeDevice | fixed (the orchestrator's finding too) |

## Re-checks after fixes (Opus, `48s3-opus-rechecks.md`)

| Round | Verdict | Major | Disposition |
|---|---|---|---|
| fix 1 | FAIL | R1: the Mock gave the closed short front to a short burst continuing a long one (Mock looser) | fixed in round 2 with a `continues` flag |
| fix 2 | FAIL | R2: the flag, set at booking, is wrong when the successor is received first (Mock looser) | **R3:** third major from the Mock inferring device-burst framing, against note 40 D5; the inference removed, the Mock's frontier back to HEAD's (14 rows and 6 tests removed, parity tests restate D5) |
| fix 3 | FAIL | M1: bookings received before the window — (a) at a closed short burst's end, (b) 4–32 samples into a longer unhanded burst (the Module cuts and closes it, then drops the booking) — the Mock plays, the Module drops; HEAD agreed | ruled within D1 (#79, note 47 stage 5); (b) added to KL-36; minors: MR-17 cites MR-25's first-send condition, KL-36 "When" covers (b) — fixed by the orchestrator |

## Gate

`48s3-gate.json`: every step PASS except mutation 1737/1740: G09 (known survivor) and two rows BASELINE FAILED under host load (N47B-19, P889, both real-clock FakeDevice rehearsals). Rerun alone on sim02 (load 2.6): both killed (2/2). Per the rerun rule (2026-10-27) these are not survivors.

## Counts

Sol plan 6 (1 P1, 5 P2); Opus plan re-check 1 major, 7 minor; self-review 9 (8 fixed, 1 rejected); Sol code 6 (2 major, 3 minor, 1 note); re-checks 3 majors (R1, R2, M1), 3 minors. Mechanisms (`plan/maintenance/failure-mechanisms.md`): `radio-rule-implemented-twice` — Sol code 1, R1, R2 (the Mock inferring the Module's device-burst framing; caused by the orchestrator's plan ruling against note 40 D5, answered by returning to D5, R3); M1 is #79's existing mechanism. Sol code 2 is a FakeDevice modelling defect (none).
