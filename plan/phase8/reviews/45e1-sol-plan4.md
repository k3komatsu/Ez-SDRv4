> plan · gpt-6.1-sol
All four fixes are closed in `test-plan.md`:

1. **Eligibility:** U9/u09a/u09g separate pin clearance from restore’s I15 checks. Case 5 can stop before sample 40, start the next segment at 44 with `left` outstanding, consume it, then restore (lines 253–263, 453–457).
2. **Loss publication:** The test checks that the publisher—not every actor—has released `L`, with a witness allowing another holder. E1-56 matches (385, 634).
3. **Recomputation:** u07d advances to epoch 2 while retaining epoch-1 samples and handle; its identity and retiring-cut assertions distinguish E1-60 (241, 638).
4. **Monitor:** Types, key tests and I4 now carry value, channel, segment, epoch and matching sample, consistent with the experiment plan (79–84, 352–361, 550).

No files changed or tests run; holdout not read. The supplied backup path was unavailable, so comparison used the prior review’s findings.

**VERDICT: PASS (event granularity and cases approved for implementation).**
