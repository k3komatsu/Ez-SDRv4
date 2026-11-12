# Note 51 stage 1b — Sol's code review (2026-11-12)

## Verdict: FAIL

The registration and 71→72 ms adjustment are sound. Two major issues remain in N4/N5 and their mutation accounting.

### Major

1. **N4/N5 still have a scheduling-dependent setup.**  
   `crates/ezsdr-acceptance/tests/parity.rs:3045–3056`


   `idle` now includes that extra reference. Later counts never exceed it, so the helper exhausts its loop and fails on correct code. Two consecutive *subsequent* readings do not establish that the initial baseline was idle.

   **Fix:** establish the reference-free baseline or pending read explicitly, independently of constructor scheduling.  
   **VERIFIED:** ownership and clock paths (`authority.rs:447–459`, `device.rs:1133–1135`). **INFERRED:** interleaving; not reproduced.

2. **N51B-03 remains an undeclared bound-dependent kill.**  
   `plan/maintenance/tools/mutations.json:18392–18401`; `crates/ezsdr-acceptance/tests/parity.rs:3070–3084`


   The assertion is reachable only after the real-time expiry (`authority.rs:502–509`). Storing its evidence before releasing Driver fixes the former false-pass race, **not** the bound dependency. Correct ordering can also produce the same assertion if `uhd-clock` is descheduled beyond that bound.

   **Fix:** use an expiry-independent ordering assertion, or emit the infra marker for this detach and declare `"kill": "bound"`.  
   **VERIFIED** from control flow.

### Otherwise

- Constructor-last registration, Driver-first teardown, separate registrations for two rigs, and `TxBlocks` rejection are appropriate.
- **71→72 ms is justified:** a Stop received at 61 ms reserves through approximately 71.005 ms; 72 ms clears either poll ordering without weakening a sample-exact boundary test.
- N51B-08 correctly declares its bound kill. The last probe field preserves evidence until observation.
- Note 51/table updates remain the orchestrator’s recorded landing work; the README correctly describes registration.

Not checked: no builds, probes, or mutation runs performed; no files changed. The remaining risk is false failures and falsely classified mutation kills in the new Drop tests.
