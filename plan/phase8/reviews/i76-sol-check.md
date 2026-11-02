# Sol 6.1 — i76-check (issue #76), report section

## REVISE

- **Scheduling P2 closed:** neither replacement has a submission window. The probe test asserts synchronous recording, not acceptance (`crates/ezsdr-radio-uhd/tests/fake.rs:9308–9318`); the action test is pure (`crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs:707–710`).
- **Named kills are valid:** P1139 makes `at` become `None`, failing line 710; P1142 removes `host_entry_ns`, failing `fake.rs:9317`; P1154 selects `tx_send`, failing the lookup at `fake.rs:9314`. Filters match these tests (`plan/maintenance/tools/mutations.json:14638–14675,14788–14795`).

**P2 — Successful measurement lookup wiring is now unpinned.**
- At `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs:331`, mutate `probe.matching_timed_apply(e)` to `probe.matching_timed_apply(e + 1)`.
- Sequence: submit gain at `e` → probe records apply at `e` → measurement searches `e + 1` → reports no apply and skips subsequent updates.
- The removed test required two matched applies. Both replacements bypass this wrapper; remaining rehearsals permit zero observed applies (`crates/ezsdr-radio-uhd/tests/fake.rs:9228–9249,9291–9304`). The not-submitted test also accepts this failure path (`fake.rs:9322–9333`).
- This mutation would survive by inspection. Add a scheduling-independent test through the measurement lookup using a pre-recorded timed apply.

Not checked: no tests or mutations rerun; kill explanations are source-based. No files changed; forbidden holdout not read.
