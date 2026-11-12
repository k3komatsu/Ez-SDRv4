# Note 51 stage 2b — Sol's code review (2026-11-13, openai/gpt-6.1-sol, read-only)

## FAIL

### Major — new Direct test leaks `uhd-control` — VERIFIED
`crates/ezsdr-radio-uhd/tests/fake.rs:2009–2010`

The test returns without `direct.finish()`. Neither `Direct` nor `UhdRadio` implements cleanup on drop; dropping `Running` detaches its handles. The control loop exits only when `streams.stop` is set (`src/provider/control.rs:234–250`).

**Minimal sequence:** run the new test successfully → its rig drops → continue the suite. The old `uhd-control` remains alive, retaining its Core/device and polling instead of being stopped and joined.

**Fix:** collect observations, call the existing `direct.finish()`, then assert.

### Minor — Stop-stamp weakening is unbounded, not approximately 1 ms — VERIFIED
`plan/phase8/51-test-timing.md:569`; `crates/ezsdr-acceptance/tests/uhd.rs:490–505`

The Kernel stamps before compilation/dispatch (`pipeline.rs:939`). A thread stalled after that stamp can delay Provider receipt arbitrarily. Consequently, `gap ≥ 9_900` no longer proves clearance relative to the actual Stop receipt; the documented “約 1 ms だけ” understates the lost coverage. Record this limitation accurately and retain the exact-window obligation for 2c.

### Minor — deferred-set description overstates the sweep — VERIFIED
`crates/ezsdr-radio-uhd/tests/README.md:125`; `plan/phase8/51-test-timing.md:568`

`n47s5_stop_end_rehearsal_on_the_fake` also did **not** fail in the 2a sweep. Thus nine, not ten, of the thirteen deferred tests failed. Tables §3.2 additionally classifies this test as needing the Kernel log, not Provider `receipts`. Keeping it grouped for 2c is reasonable; correct the blanket explanation.

Otherwise, the split preserves the refusal/window assertions, filters remain correctly selected, and recorded mutation outcomes distinguish J317/J318’s bound kills.

No edits, builds, probes, remote runs, or mutation execution performed; runtime leakage was established by source inspection, not measured.
