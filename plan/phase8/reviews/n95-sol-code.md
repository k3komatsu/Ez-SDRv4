My initial failure scenarios, before reading the implementation report:
- A burst fits its sample clock but overflows the device clock (or vice versa); refusal must retain the Action id and leave later bookings usable.
- Repeat/switch arithmetic crosses either integer limit, including negative origins, exact boundaries, and nonpositive periods; valid arithmetic must retain its previous result.
- An untimed booking or Stop derives an overflowing boundary after admission; neither Provider may panic or turn a receipt error into a Run failure.
- Refusing a switch must not partially change the active repeat or reserve queue space.
- Paired tests must prove refusal parity and subsequent service through explicit clock/worker synchronization, not scheduling luck.

I’ll trace all callers and receipt paths, then compare the tests, contracts, and mutation rows.
The boundary helper itself is sound, and all 18 changed mutation anchors match uniquely. Two gaps surfaced: receipt-derived Stop boundaries still have overflow escapes, and the independent oracle retains the old overflowing grid formula.

The new `TimeOverflow` arms should stay: X3xx supports 200 MS/s on its 200 MHz root (`n = 1`), so the report’s “no real profile” justification is incorrect.
## Verdict: FAIL

The targeted burst/switch fixes look correct, but the requested overflow audit is not complete.

### Major — Stop can still fail the Mock Run
**VERIFIED by source; pre-existing residual, not a regression.**

`crates/ezsdr-mock-radio/src/lib.rs:1174` still returns `ModuleError::rejected` when `now + window` overflows. That escapes through `:1513` and `:2467`, without an Action-id refusal.

Minimal sequence:
```text
Prepare/arm/start x3xx Mock with a registered TX clock.
Queue Stop(tx/s0).
Step at root tick i64::MAX - 1.
```
With the positive in-flight window, the result is `MR-25: hand-over horizon overflow`, ending the step/Run rather than refusing the Action and continuing. No transmitted waveform is required. The report’s exclusion of this path because it derives from the Run clock does not satisfy the requested Stop audit.

### Minors

1. **Independent oracle retains #94’s arithmetic. — VERIFIED**  
   `crates/ezsdr-acceptance/tests/oracle/mod.rs:171–173` still computes the grid in `i64`. `grid(0, 10, i64::MAX)` panics/wraps. Update the oracle’s overflow semantics and pin them independently.

2. **Paired tests do not verify the correct refusal id or rule. — VERIFIED**  
   `crates/ezsdr-acceptance/tests/parity.rs:2193–2194` accepts *any* numeric Action id and strips *any* rule prefix. A stale id or incorrect rule would pass. Assert each Provider’s expected id and rule before comparing normalized reasons.

3. **“Unreachable arms” justification is incorrect. — VERIFIED**  
   Report `report.md:133` says no real profile has one root tick per sample. X3xx supports exactly that at 200 MS/s: `crates/ezsdr-radio/src/x3xx.rs:17,34,65–66`. Keep the `TimeOverflow` arms and add a rate-appropriate test that reaches them; do not remove them as dead code.

### Checks that passed

- `repeat_boundary`’s `i128` calculation, narrowing, nonpositive-period handling, and unchanged nonoverflow results are sound.
- Production callers and exhaustive refusal handling are updated.
- New paired scenarios demonstrate subsequent service; I found no new scheduling-dependent assertion.
- RM-13/RM-37/MR-16/UR-21 match the targeted implementation.
- All **18 changed mutation anchors** match uniquely. Static spot-checks of **N95-01** and **N95-05**, corroborated by `impl.log:5612,5618`, show plausible compiling mutants killed at the intended assertions.

No edits, builds, or mutation executions performed. Runtime counterexamples were not independently reproduced.
