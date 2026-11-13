# Note 51 stage 2c — Sol's test-plan reviews (2026-11-13)

## Plan v1

Before opening the proposed test plan, my failure scenarios are:
- The receipt row uses a different clock read from the booking decision, or loses stream identity on a device-wide Stop.
- Boundary equality, sample rounding, or a production-derived threshold misclassifies an Action under load.
- A late C masks an unrelated fault, missing acknowledgement, or wrong phase in on-time A.
- Shortening C changes the held-tail scenario rather than merely making its timing observable.
- A timing marker masks a genuine failure; hardware’s zero-conclusive rule is counted across unlike cases.
- Capture timing, Stop bounds, or fake assertions still rely on the test thread’s clock read rather than recorded evidence.

I’ll now compare these against the proposed plan and its hardware audit.
## Verdict: FAIL

The receipt-record design is reasonable, but the proposed verdict rules can excuse Provider regressions. Amend the plan before implementation.

Below, **P** means `/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2c/test-plan.md`. **VERIFIED** denotes source/plan inspection, not reproduced probe results.

### Majors

1. **The wider windows change the owner’s rule, rather than implement it — VERIFIED.**  
   **P:20,58,95–97,209** changes R1’s receipt threshold from 2 ms to 5 ms, and invents a 3 ms threshold for after-`J(e)` cases. UR-14 describes the delivery allowance as polling and booking preceding Actions, not an additional receipt-to-dispatch allowance (`design/18-uhd-radio.md:216`; `src/profile.rs:9–21`).  
   **Sequence:** Action arrives at `T−2.2 ms`; dispatch later drops it. The accepted rule makes this FAIL; the proposed 5 ms classifier makes it timing-INCONCLUSIVE. This is exactly the post-receipt lateness that note 51 forbids excusing (`51-test-timing.md:100,229–236`).  
   **Required:** retain the accepted receipt criterion, or bring this premise back to the owner with the contradictory evidence. Add a test specifically covering receipt lead between 2 and 5 ms followed by a dispatch drop. Fake measurements do not establish the 3 ms hardware window.

2. **A capture’s first sample is not an independent receipt witness — VERIFIED; counterexample INFERRED.**  
   **P:26,101,163,207** claims a two-sided equivalence that the Sink does not implement. It selects the first eligible sample from subsequently processed blocks; it neither timestamps Action receipt nor proves sample availability (`crates/ezsdr-sink-capture/src/lib.rs:208–245,451–463`).  
   **Sequence:** capture Action arrives before `at`; a receive/data-path defect omits samples around `at`; first available sample is later. The proposed witness calls the Action late and can mask the defect as timing-INCONCLUSIVE. Conversely, buffered timestamped blocks can make sample timing differ from receipt timing.  
   **Required:** keep Q17’s accepted one-sided log witness unless independent receipt evidence is established. Test on-time receipt with unavailable/skipped initial data—not merely delayed submission.

3. **Fault priority remains incomplete, including a new unconditional PASS — VERIFIED.**  
   **P:79** checks event counts/refusals, but does not check unrelated missing sends/acks or wrong phase before returning timing-INCONCLUSIVE. Those are explicitly protected by note 51 (`51-test-timing.md:229–230`).  
   **Sequence:** A arrives in-window; late C supplies its own `TIME_ERROR`; A’s close/send sequence is independently broken without another fault event. The proposed guard holds and skips the structural verdict.  
   Separately, **P:135** returns PASS for a correct overlap refusal without checking faults. **Sequence:** repeat A underflows; Stop occurs; next booking correctly overlaps its receipt-derived bound → PASS despite A’s underflow.  
   **Required:** evaluate independent faults/structural evidence before excusing L, and apply fault checks to every Stop-table branch. Add both counterexamples as pure tests.

4. **The timing-based long-close INCONCLUSIVE is not resolved — VERIFIED.**  
   **P:116,148** preserves the fake’s separate shape assertion, but does not change the shared hardware outcome. `n47s5.rs:241–248` still returns ordinary INCONCLUSIVE when the required close shape is absent; `iii.rs:690–693` explicitly attributes this to a delayed owner turn.  
   **Sequence:** A and B arrive in-window; Provider produces the wrong close shape, with clean reports → hardware remains INCONCLUSIVE and passes because the proposed Q15 helper recognizes only `TIMING_PREFIX`.  
   **Required:** explicitly resolve N3’s fidelity decision and test the shared hardware-facing verdict. The fake-only assertion is insufficient.

5. **The RX restart case is still an investigation, not an implementation-ready test plan — VERIFIED.**  
   **P:124,190,198,215** leaves its window, observation rule and mutation filters undecided. This is one of the deferred timing-sensitive sequences, not peripheral work.  
   **Sequence needing resolution:** Stop → frequency/gain updates → delayed `start_rx`; the required SHORT/LATE candidate disappears (`measurement.rs:2155–2170,4127–4158`). No specified bound currently distinguishes that missed setup from a Provider regression.  
   **Required:** derive the window first, specify missing-record/fault precedence, and name deterministic assertions and mutation filters before approving sub-step C.

### Minors

- **Receipt-read fidelity:** **P:52,184–188** tests exact ticks on a stepped clock, but does not distinguish reusing the decision read from a second `now()` returning the same T. Add a scripted successive-read test or equivalent observation.
- **Q15 aggregation:** **P:143,199** must define case keys and aggregation for every repeated case, including the four long-close trials per rate (`hardware.rs:3764–3769`). Test mixed timing-missed/conclusive trials and all-timing-missed trials.
- **Stop coverage:** make **P:139**’s positive played-path coverage mandatory, or cite equivalent Provider-path coverage. A correct refusal alone no longer verifies the hardware claim of transmission after Stop.
- **Shortened C:** **P:149** appropriately says hardware onset fidelity remains unverified. Also verify the changed send/close shape; shortening 10,000 samples to 1,000 changes more than onset duration.

No files changed, builds, probes, remote runs or mutations performed. The plan’s probe measurements and historical bench claims were not independently reproduced.

## Plan v3 (after the owner's §5.3b)

I’ll first derive failure scenarios from the governing rule and existing code, then compare them with v3. This review will use file reads only—no builds, tests, probes, or remote execution.
Independent scenarios to check against v3:

- A late trial also has an underflow/refusal: skip that whole trial as decided, but a later unmarked failure must win without a timing marker.
- An Action misses neither evidence condition—no `cause=late`, stamp within its literal window—but fails: it must fail unmarked, including `late_at_device`.
- One rehearsal site omits classification, or asserts admission/shape first: lateness becomes a false kill.
- Baseline timing → serial pass/failure/three marked runs must use the same judging rule before applying a mutant.
- A delay-producing row becomes ERROR, or shortening C removes its killing path: it needs a deterministic direct-rig killer or justified equivalence.
- C=1,000 and Stop N2 change the hardware scene: verify geometry and revise the claim, not just the label.
- Serial execution still marks: require 20 marker-free runs before switching assertions; three marked attempts must remain red/ERROR.
## Verdict: FAIL

The overall approach matches §5.3b, but two coverage gaps remain before implementation.

Paths below use **P** = `/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2c/test-plan-v3.md`.

### Major

1. **The wiring guard does not verify every classification site—or `finish()`.**  
   **VERIFIED:** P:196,301 checks only that each test contains `Rehearsal::`. Multi-trial tests contain several independently judged sites (P:106–114).  
   **Counterexample (INFERRED):** keep `Rehearsal::new`, remove the wrapper around one trial → that trial receives late evidence → its bare assertion fails unmarked → mutation reports a false kill. Alternatively, omit `finish()` → skipped trials can pass silently.  
   **Required:** guard each named trial site and finalization, with deterministic negative checks for removing one wrapper and removing `finish()`.

2. **Admission checking is tested in isolation, not wired into the replacement path.**  
   **VERIFIED:** P:60 replaces admission assertions with `book`; P:80–88 specifies evidence classification and closure execution, but does not place `kernel_refused` in that path. S-5 only tests the standalone function (P:182). Existing `front_outcome` checks only the final booking’s admission (`crates/ezsdr-radio-uhd/tests/common/n47s5.rs:192–199`).  
   **Counterexample (INFERRED):** Kernel wrongly rejects calibration; head and final tone are admitted; no lateness evidence or reported fault → calibration’s old assertion has disappeared, and the Fake’s missing-tone INCONCLUSIVE can satisfy `!Fail`.  
   **Required:** specify admission-check placement and add an integrated negative case for an earlier Action being rejected. Preserve §5.3b’s retained priority 2 (`plan/phase8/51-test-timing.md:289`).

### Minor / required corrections

- **Apply the binding decisions throughout the plan.** P:52–56,180,210–211,282–285 still include clause 3; P:196 still calls governance optional. Add an explicit `cause=late, outcome=late_at_device → no evidence → unmarked failure` test when applying O1. Authority: `plan-decisions.md:20–25`. **VERIFIED.**

- **N2 does not make the combined hardware verdict conclusive.** P:149 combines N1 INCONCLUSIVE with N2 PASS using `all_of`, which remains INCONCLUSIVE (`common/iii.rs:843–856`). Revise “the case can conclude” (P:268); report N2 separately if conclusiveness is intended. N2 can still detect a failure. **VERIFIED.**

- **The sweep acceptance criterion contradicts the accepted residual limitation.** P:253 prohibits every plain failure, while P:257 explicitly retains capture delays inside `submit` as unmarked failures. Record those separately rather than requiring broader evidence or treating every occurrence as an implementation defect. **VERIFIED.**

The delayed-marker design, baseline-through-`judge` tests, N7 preservation, M4, and 20-serial-run prerequisite are otherwise appropriately planned. Keeping rows 192–194 on N7 is supported by their actual anchors and assertions.

No files changed; no builds, tests, probes, or remote execution. Runtime and bench claims remain unverified by this review.
