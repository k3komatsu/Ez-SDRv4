# E3b — Sol's test-plan review (2026-11-12)

## Verdict: PASS WITH CHANGES

Shape **S** is consistent with no behaviour change. I treated `plan-decisions.md` as overriding the plan: **keep the stat**, leave Mock’s chain undriven, reset only the new owner’s chain, and preserve F4.

References below use `P` = `/home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3b/test-plan.md`.

### Major

**M1 — Recovery/shutdown characterization cannot run entirely frozen as specified.**  
**VERIFIED:** P:94–97 includes flush, reopen and shutdown; P:196 says every new test is single-threaded and frozen/stepped. However, T3 waits on the device’s host clock (`crates/ezsdr-radio-uhd/src/provider/core.rs:1209–1234`), while `Frozen` deliberately prevents that clock advancing (`provider/test_support.rs:136–157`).

Minimal sequence:
```text
```
**INFERRED wrong outcome:** the characterization hangs until an infrastructure bound rather than observing recovery. Shutdown’s acknowledgement window has the same problem.

**Required:** explicitly use the existing `released` participant helper (`provider/test_support.rs:170–184`) for clock-waiting operations, returning the owner and a new freeze before snapshots. Specify this for C1/C3/C4 and associated mutation killers; do not simply drop the freeze around an unregistered test thread.

### Minors

**m1 — Acquisition counts do not characterize write placement.**  
**VERIFIED:** C4 records only counts (P:97,135); C3 and trace comparison observe operation exits (P:96,194). Those observations cannot distinguish a write moved between two existing sections with unchanged final state. W1 adds only a pre-completion receipt seam (P:133).

Add ordered snapshots at the relevant section boundaries, especially booking completion versus chain effect, pre-close versus close result, and flush-front versus reopen completion. Assert the seam fired and receipt still reads the prediction. Do not require identical intermediate state where folding intentionally changes it—state the permitted difference.

**m2 — P-MH’s implication needs the join exemption.**  
**VERIFIED:** P:111 says “first send fits ⇒ may have handed.” But `handed_end` ignores preceding chains (`segment.rs:135–147`), whereas `may_be_handed` suppresses a chain behind an open join (`staging.rs:1103–1107`).

Counterexample: C fits the window, but an earlier chain ends before C and remains joinable. `handed_end(C)` exists; `may_be_handed(C)` is false. Qualify the implication with “no preceding open join,” and pin this counterexample separately.

**m3 — Arithmetic equivalence needs explicit overflow semantics.**  
**VERIFIED:** receipt front uses checked addition (`staging.rs:1128–1133`); executor fronts use saturating addition (`tx.rs:826,866,942,1026`). P:14’s single formula does not specify that distinction, and P-G2 omits the flush expression (P:110).

Specify checked versus saturating results and boundary rows for every close shape, including flush conversion. Receipt overflow must remain refusal, not a saturated accepted footprint.

### Other checks

The hardware audit identifies production-derived expectations and existing rehearsals; retaining those independent literal checks is appropriate. Correct note 49’s stronger state-sharing claims without changing RM/UR receipt semantics. Remove the now-overridden stat-deletion instructions from the executable plan.

No files changed; no builds, probes or mutation runs performed. Mutation effectiveness remains unverified until implementation.
