The new tests pass, including the 1,000-seed differential. The sort is stable, so equal instants retain document order. I’m checking generator coverage and UHD’s records next.
**PASS** — no major findings.

**Minor — document the full ordering contract.** `design/09-mock-radio.md:157` specifies instant order only for loss, but the change also orders pending rows at ordinary `Provider::stop` and `cleanup` (`crates/ezsdr-mock-radio/src/lib.rs:1204,2428`). State that ordering once for all pending-fault records, preferably in MR-27.

**VERIFIED**
- Instant order is consistent with RM-25; stable `sort_by_key` preserves SE-3/SE-4’s document-order ties. Existing rows remain append-only as MR-27 requires.
- Both `record_pending_faults` callers are covered by the shared fix. No competing pending-fault writer was found.
- UHD has no corresponding pending-fault Manifest list: its records and section writer enumerate other records (`crates/ezsdr-radio-uhd/src/provider/core.rs:786`, `provider/mod.rs:416`). Search: `sim.faults|section\("faults"\)|pending_fault|faults"` across its source.
- Generator short-circuiting preserves every other class’s sequences, including Loss. Fault retains its original non-loss faults and sometimes appends a loss; loss-free coverage remains.
- The hand-built test checks absolute `at`, repeated fault kinds, ties, cleanup and both profiles (`crates/ezsdr-mock-radio/tests/mock_radio.rs:2536`).
- N77-01/02 each have one exact anchor. Seed 362 concretely generates two reversed, different-kind pending faults after loss.
- Tests passed: 10 MR-20 tests, the 1,000-seed Mock differential, and 26 Radio timeline tests. No introduced maintainability concern.

**INFERRED:** Both mutation rows should be killed: booking order contradicts the explicit assertions and seed 362’s oracle order. This was established by inspection, not mutation execution.

Not checked: mutation execution, full workspace gate or hardware. No files changed or formatting performed.
