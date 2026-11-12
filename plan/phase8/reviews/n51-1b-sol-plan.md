# Note 51 stage 1b — Sol's test-plan review (2026-11-12)

## Verdict: FAIL

The registration design is sound, but the planned Drop tests still have scheduling-dependent verdicts.

Paths below use `P = ~/.cache/ezsdr-fixes/2026-11-12-n51-1b/`.

### Major

1. **N4/N5 do not establish a stable read-in-progress state.**  
   **Evidence:** `P/test-plan.md:60–61`; `P/probe/sketch-1b.diff:145–155,159–176`; `crates/ezsdr-radio-uhd/src/authority.rs:456–459`.



   **Fix:** establish the pending clock sleep explicitly, and observe join/detach before releasing the condition that preserves the evidence. Do not use transient `Arc` counts alone.  
   **VERIFIED:** reference lifetimes and observation order. **INFERRED:** these interleavings; not reproduced here.

2. **N51B-03 is an undeclared bound-dependent kill.**  
   **Evidence:** `P/test-plan.md:76`; `P/probe/mut2.log:1`; `crates/ezsdr-radio-uhd/src/authority.rs:502–509`.


   **Fix:** use an ordering assertion independent of expiry, or explicitly classify this as a bound kill with a test-side infra marker and `"kill": "bound"`. N51B-08 already does this correctly.  
   **VERIFIED.**

### Minor

3. **The proposed comment still attributes device-time movement to owner scheduling.**  
   `P/test-plan.md:89` says a slow host can move `uhd-control`’s wake. M3/M4 stop the manual clock while that participant is busy or awaiting execution at its deadline (`host.rs:14–20`). Describe residual **poll phase**, not host scheduling delay. **VERIFIED.**

### Otherwise satisfactory

- Raw logs support all 14 diagnostic failures before registration and passes afterward: `P/probe/A6-all-20.log:190`, `B1-14-all-20.log:23`.
- The busy/idle construction—not injection alone—supports eliminating test-thread delay from receipts.
- Constructor-last registration, `TxBlocks` rejection, two-clock coverage, and replacing both skip branches with assertions are appropriate.
- The repeated off-clock-wait search found no existing parity test-body channel/helper-thread joins.

Not checked: no builds or new probes run; no files changed. The unresolved risk is the new Drop tests’ scheduling sensitivity.
