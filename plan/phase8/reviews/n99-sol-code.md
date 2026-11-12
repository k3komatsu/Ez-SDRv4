# #99 — Sol's code review (2026-11-13, openai/gpt-6.1-sol, read-only)

## Verdict: PASS WITH CHANGES

No major code defect found by static review.

### Minor — adopted m1 remains unfixed in UR-23 — VERIFIED
**`design/18-uhd-radio.md:266`**

“An abandon that follows a `Stop` … changes nothing” is still unconditional.

Counterexample: Stop → consume its halt → accept a new finite booking → abandon that booking. Its abandon **must end its own join**, as implemented in `crates/ezsdr-radio/src/staging.rs:965–970` and tested in `crates/ezsdr-radio/tests/staging.rs:1645–1654`.

Replace it with the accepted qualification: **“A finite abandon does not rewrite what a Stop, Provider::stop or device loss already ended in the record.”** RM-15 already uses this wording.

### Checks
- Traced all abandonment paths, Stop/loss ordering, pruned joins, successive abandons, surviving-head moves/drops, and repeat/switch isolation. No additional defect identified **(INFERRED from code)**.
- Reviewed U1–U11, pure composition coverage, the C1 expectation change and C6 split. The residual acceptance-before-abandon race remains explicitly tested and documented.
- All **25 new/changed mutation anchors match exactly once**; replacements statically produce their intended changes. Extracted helper preserves existing behavior.
- Mock search `abandon|end_finite` in `crates/ezsdr-mock-radio/src` returned no matches.

**Not checked:** builds, runtime probes, mutation execution or hardware. No files changed; runtime correctness remains unverified independently.
