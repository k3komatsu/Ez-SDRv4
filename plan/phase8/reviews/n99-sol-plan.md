# #99 — Sol's test-plan review (2026-11-13, openai/gpt-6.1-sol, read-only)

## Verdict: PASS WITH CHANGES

No major defect found in the amended sketch by static review. The unread-Halt guard and loss check address F1. Using `end_finite(e)` instead of `(k, e)` appears sound for reachable owner sequences: the end identifies the containing span without relying on prunable join points. **INFERRED; not independently executed.**

Paths below are relative to `/home/komatsu/.cache/ezsdr-fixes/2026-11-12-n99/`.

### Minor findings

1. **The proposed halt exception is too broad. — VERIFIED**  
   `test-plan.md:332` says an abandon following *any* Stop changes nothing. That contradicts P6 and the intended behavior: Stop → accept a new finite booking → abandon that booking must end **its** join. It also unintentionally covers repeat abandons, which remain unchanged.  
   Qualify this as: **a finite abandon does not rewrite the record already ended by a halt of that booking.** Apply the same qualification to UR-23’s unconditional sentence at `test-plan.md:338`.

2. **KL-39 gives the wrong front for an abandoned short standalone booking. — VERIFIED**  
   `test-plan.md:348,352` specifies four samples beyond the finite booking’s end. But a standalone short booking retains its padded-end-plus-`g` footprint (`crates/ezsdr-radio/src/segment.rs:195–199`); the sketch’s no-piece branch preserves it (`probe/reference-sketch2-production.diff:69–72`).  
   Example: one sample at 10,000, failed EOB send → footprint ends at 10,019, not 10,005. The proposed workaround can therefore still be refused. Use **the retained RM-15 front**, distinguishing short standalone heads from long/joined pieces.

3. **Add a composition test for successive abandons and dispatch movement. — VERIFIED coverage gap**  
   `test-plan.md:239–247` tests individual splits and a subsequent drop, but not successive abandons or a dispatch move after splitting. These matter because the change creates overlapping footprint guards beside surviving chain heads.  
   One small pure sequence suffices: A(40)+B(100)+C(10) → abandon A → abandon B → move/drop C. Assert that A/B retain their fronts, C alone moves/leaves, and the first sample beyond the remaining footprints stays free. This also pins the promised handling of an already non-joining surviving chain.

### Evidence notes

- The new rig sketches use frozen time and synchronous hooks; I found no thread-scheduling-dependent verdict.
- No equivalent new mutation row identified statically. However, `probe/n99-mut.py:25–26` accepts any nonzero result containing `FAILED`, without excluding infrastructure markers/signals. Its “25 killed” claim is **not gate evidence**; use the prescribed remote runner at implementation.
- Mock search (`abandon|end_finite` in `crates/ezsdr-mock-radio/src`) returned no matches.

**Not checked:** no builds, probes, mutation runs or hardware execution. Tree remains clean at `cc8c3cb`; runtime conclusions above remain static-review inferences.
