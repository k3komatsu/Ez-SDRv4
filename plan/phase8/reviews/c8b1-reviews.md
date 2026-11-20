# 8b first clean-up (c8b1) — GPT-6.1 Terra reviews

## review

What this change does: it adds automatic cleanup to the `Direct` test rig, changes how TX headers obtain their contract ID, removes two hardware tests for channel moves between streamers, and adds pure-rule tests and mutation rows.

**Major**

1. **TX now allocates once per block** (`crates/ezsdr-radio-uhd/src/provider/tx.rs:787`). A continuous or repeated burst constructs a header for every block, and `DataContractId` owns a `String`; cloning it allocates and copies that string each time. The previous take/restore reused the allocation. The header values are otherwise unchanged: `Tx` has exclusive access during the operation, and `BurstTracker::on_block` only borrows the header. I would keep the allocation-free take/restore, or keep a reusable `BlockHeader` in `Tx` and update its other fields using split borrows. This hot-path allocation is a behavior and performance change, so the brief’s no-change requirement is not met.

2. **A cleanup panic can turn a failed test into a pass** (`crates/ezsdr-radio-uhd/tests/fake.rs:1591`). If a test reaches the end normally without `finish` and `cleanup()` panics, the unconditional `catch_unwind` discards the panic and the test returns successfully. It can also leave cleanup incomplete: `cleanup` sets `cleaned` before stopping and closing (`crates/ezsdr-radio-uhd/src/provider/mod.rs:766`). Catch cleanup panics only when the thread was already unwinding; otherwise let them fail the test. The existing unwind test checks the already-panicking case, not this one.

The failure scenarios and checks for the remaining questions were:

- **Drop and joining:** A test might still hold a worker in a device call when `Direct` drops, or call `finish` or `cleanup` first. Explicit cleanup is idempotent through `cleaned`; joins have a one-second bound per worker and detach on expiry. The gated `Direct` tests inspected install release guards after the rig, so those guards release during unwind before the rig drops. I did not establish a current unbounded join caused by this change; a failing mutation can nevertheless spend those bounds in teardown, and I did not run mutation tests to measure the effect on their timeouts.
- **Hardware tests:** RM-34 is explicitly withdrawn (`design/07-radio-model.md:344`), and RM-21 says channel membership has no runtime update class. The deleted tests exercise moving the same front end from one streamer to another, so their removal is consistent with the current Provider path.
- **Tables and mutation rows:** The asserted grid boundaries, decimation values, front-end directions, board names, addressed stream bounds, and single-frequency RF band match the inspected implementation and applicable RM-8, RM-19, RM-28 and RM-29 rules. C8B1-1 through C8B1-12 target the decisions their descriptions name; their anchors are appropriately localized. This is a static assessment, not a mutation-run result.
- **`clip_decimation` survivor:** The `||` → `&&` mutation is equivalent for its `u64` clock and `f64` rate inputs. Negative finite rates yield a nonpositive wanted decimation, for which the loop selects 1; positive infinity yields zero, also selecting 1; NaN and negative zero leave the initialized result at 1. Negative infinity still takes the early return.

**Verdict: FAIL.** Fix the per-block allocation and the swallowed normal-path cleanup panic. This was a read-only review; I did not build, format, run tests, or run mutation rows.
## re-check of the fix round

**Verdict: PASS.** Both majors are closed; I found no new issue in the fix.

- **TX:** The diff against the base contains only a comment. The original take/restore is back, so consecutive blocks reuse the contract without cloning its string. `crates/ezsdr-radio-uhd/src/provider/tx.rs:787`
- **Cleanup:** A panic during normal drop now propagates and fails the test; a cleanup panic during an existing unwind is caught. `crates/ezsdr-radio-uhd/tests/fake.rs:1590`
- **Note:** Skipping the normal-path panic test is acceptable for this small test-only branch. The existing unwind test checks cleanup during unwinding, but does not exercise a panic *inside* cleanup. The panic-handling branches were assessed statically.

No builds, tests, or formatting were run.