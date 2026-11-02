# Sol 6.1 — i76-check2 (issue #76), report section

**PASS — P2 closed.**

- `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs:707–718`: the new test pre-records the apply synchronously, then exercises the measurement lookup with both manual modes and rejects `e + 1`.
- `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs:334–346`: no scheduling window. With `manual=false`, the existing row returns **before** the deadline check or sleep; descheduling cannot change the verdict. The negative lookup uses `manual=true`, so never waits.
- `plan/maintenance/tools/mutations.json:18695–18702`: P1318 targets the correct wrapper and test. Searching `e + 1` misses the sole recorded apply at `e`, failing the first `expect` at `h4.rs:714`—its named reason, not a timeout.

Not rerun: tests or mutations; kill verified by source inspection. No files changed; forbidden holdout not read.
