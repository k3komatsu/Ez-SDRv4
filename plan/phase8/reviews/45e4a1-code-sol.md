# REVISE

No P1 findings. Three P2 findings remain.

## Findings

1. **P2 — An end-now request frees K without retrying held values.**  
   `crates/ezsdr-radio-uhd/src/brain/core.rs:501,599–602,1020–1029`
   ```
   K=1; A queued on stream 0; B held on stream 1;
   stream 0 has a cut;
   Request(Late(stream 0)) → sends Stop → cancels A.
   ```
   K is now free and B is eligible, but `Decide` is excluded from re-evaluation. B waits for another input, potentially delaying its application until A’s `Dropped` arrives.
   **Fix:** include this decision’s cancellation path in the shared retry rule, with a two-stream regression asserting B is reserved in that step. The current B10 request test checks reopening, not this release.

2. **P2 — P6 still misses the adjudicated Done-before-Shutdown failure.**  
   `crates/ezsdr-radio-uhd/src/brain/oracle.rs:387–397`
   ```
   K=1; A occupies K; B held;
   inbox=[Done(A, now≥e_A), Shutdown];
   process Done; incorrectly omit its retry; finish its operations;
   process Shutdown → cancel B.
   ```
   At the required checking point, the nonempty inbox disables P6. Shutdown then removes B and disables P6 through `exit != Running`. The observable missed application escapes this oracle—the same counterexample adopted in the plan review.
   **Fix:** check the freeing step when its operations finish, before the next `BStep`; an unrelated queued Shutdown must not suppress it. E4-31 being killed on another schedule does not cover this window.

3. **P2 — W12c does not establish the proposed post-F reservation/abandonment behavior.**  
   `crates/ezsdr-radio-uhd/src/brain/world.rs:629–637`;  
   `crates/ezsdr-radio-uhd/src/brain/bounds_tests.rs:325–332`;  
   `plan/phase8/45-experiment.md:226`
   ```
   A completed but counted; B held; cell still Streaming;
   set F; delay Boundary::Lost;
   deliver/process capacity timer at e_A → reserve B;
   S checks F → abandon B → Done(Abandoned).
   ```
   Allowing this reservation is **sound for device safety**: executor admission, not brain reservation, enforces I1. But it contradicts B8’s current “none once F is set” requirement.
   W12c records only timer **delivery** with F set. It need not reach reservation before the loss boundary, nor observe abandonment. B8c starts after that boundary.
   **Fix:** obtain the explicit B8 amendment and add a deterministic DST test asserting this complete sequence, including no device call and terminal-driven retirement.

## Replay recommendation

`crates/ezsdr-radio-uhd/src/brain/thread_tests.rs:647–654`

Q1 A/B are genuine counterfactual replays: they compare the whole `Observed` record, not a whitelist of differing fields. However, intervention B changes visibility as well as timestamps. Equality after that intervention does **not** prove that the original acceptance window contains no other defect; the intervention can remove the state exposing it.

**Recommendation:** give both wrappers the same explicit call-return acceptance point and timestamp, keeping device application time separate. This matches what the existing `Device` interface can actually observe. Then remove the Q1 tolerances from `s1b`/`s1c` and rerun seeds 1–10,000. Do not make normal execution depend on ModelDevice’s effective-time hook.

## Other checks

- K’s predicate and D’s cell-length accounting are centralized; delayed `Done`, consumption-before-Done, `left`, and loss retention have direct behavioral tests.
- I11′ uses a separate monitor-based count; I17 correlates completed starts with their carried epoch. P6’s checking window remains the weakness above.
- The new micro wrappers require actual witness hits, using walks when bounded search misses them; budget exhaustion alone cannot satisfy that requirement.
- E4-01–33 target behavior through unit/DST/runtime tests. E4-34–39 are explicitly oracle self-tests, not behavioral DST evidence.
- `ReserveHeld` and the old bound field are removed; shared `recv_inputs` eliminates the wrapper ordering duplication.

Not checked: builds, test execution, mutation kills, reported sweep/witness counts, hardware, E4a2, or excluded holdout material. No files or git state changed.
