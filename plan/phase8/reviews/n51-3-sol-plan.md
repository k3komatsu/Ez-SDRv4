# Note 51 stage 3 — Sol's test-plan review (plan v1, 2026-11-13)

## Verdict: FAIL

The plan needs three substantive corrections before implementation. Review was read-only; counterexamples below are inferred from verified code, not executed.

Paths below use **`plan`** for `/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-3/test-plan.md`.

### Major

1. **The Kernel wake tests can still pass without exercising a wake.**  
   **Evidence:** `plan:94-106`; `crates/ezsdr-kernel/tests/device_paced.rs:348-358,422-463`; `tests/support/paced.rs:109-119`.

   **Minimal sequence:** start the session → deschedule the test until MARK has already been delivered → call `wait_for`. Even with G08’s wake removed, the call finds the recorded event immediately and passes the proposed five-second assertion. Likewise, pre-delivered marks can bypass G36’s intended callback race.

   **Wrong outcome:** a broken wake mechanism survives because the tested wait never blocked. Increasing bounds does not fix this. Gate event emission on an observed parked call; for G36, also prove the callback-before-schedule-return sequence occurred. **VERIFIED structure; INFERRED execution.**

2. **The replacement SHORT test omits the measurement wiring that loses coverage.**  
   **Evidence:** `plan:166-173`; `crates/ezsdr-radio-uhd/tests/common/measurement.rs:2308-2331,4141`.

   **Minimal sequence:** Provider emits a valid SHORT warning → measurement reads the wrong event name or fails to decode `requested.ticks` → `short` is empty. The proposed pure test supplies an already-decoded nonempty slice, so it passes; Provider emission tests also pass.

   **Wrong outcome:** measurement reports a short-lead trial as successful without detection. Dropping the timing-sensitive assertion is reasonable, but its replacement must cover **event decoding → trial association → reported SHORT/classification**, using synthetic records if necessary. **VERIFIED coverage gap; INFERRED regression outcome.**

3. **Enforcement does not implement “marked and undeclared means broken.”**  
   **Evidence:** `plan:183-194`; `plan/maintenance/tools/mutate.py:531-535`; `gate.py:217-229`.

   **Minimal sequence:** an undeclared real-clock row survives → the proposed check ignores it → it is an accepted equivalent row or known survivor.

   **Wrong outcome:** the gate can pass with an undeclared marked row. Classification must be checked independently of whether the mutant was killed. Preserve baseline failures, but reject successful-baseline marked rows without declarations; test survived/equivalent and cached-baseline cases too. **VERIFIED logic gap; INFERRED gate sequence.**

### Minor

4. **J58 has no killer to discover.**  
   `plan:81,229` treats it as assertion-killed, but `plan/maintenance/tools/mutations.json:758-766` explicitly records it as equivalent. Preserve that disposition and classify its broad filter without searching for a nonexistent killer. **VERIFIED.**

5. **The proposed scan rejects its own planned rewrite.**  
   `plan:103` retains a one-second upper bound; `plan:124` forbids every upper bound below three seconds and claims none remain. Make the treatment and scan agree. **VERIFIED.**

6. **The sleep allow-list admits positive timing oracles inside approved functions.**  
   `plan:126` allows an entire function once it contains an approved sleep. Adding `sleep(100 ms); assert!(count(...) >= 50)` inside that function passes F3, F1 and F2. Pin allowances to the approved occurrence/pattern, or explicitly narrow the claimed protection and add this bypass as a scan fixture. **VERIFIED planned exemption; INFERRED bypass.**

**Not checked:** builds, probes, mutations or reported probe logs. No files were changed.
