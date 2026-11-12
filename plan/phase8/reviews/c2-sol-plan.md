Independent failure scenarios before reading the plan: device time passes `b` while its send is unresolved; Stop arrives during that send, followed by success, zero/positive short, or exception; loss occurs before versus after playback; abandon/reopen or shutdown leaves a row pending; a second switch overwrites the first switch’s unresolved evidence. I’ll also check Mock’s Stop cut against emitted payload, and keep the accepted window-edge difference separate from other parity failures.
## Verdict: FAIL

The plan covers the main C2 cases, but misses a loss during the unresolved boundary send.

### Major

1. **Deferred settlement can forget the loss cutoff.** **VERIFIED by inspection; resulting failure INFERRED, not executed.**

   Minimal sequence:
   ```text
   issue the send carrying b
   loss occurs at t < b
   send completes successfully
   shutdown writes the switch row
   ```
   Q1 requires `cancelled`. The proposed sketch instead retains only the Stop bound while in flight, then completes with `played = i64::MAX`: `/tmp/opencode/c2-probe/crates/ezsdr-radio/src/staging.rs:1223–1238`. This produces `applied` although the device was lost before `b`.

   The plan tests deferred **Stop** settlement and ordinary **loss** separately (`test-plan.md:103,124,131`), but never their intersection. The production loss path can run independently of the send’s completion (`crates/ezsdr-radio-uhd/src/provider/core.rs:1047–1056`; `crates/ezsdr-radio-uhd/src/provider/tx.rs:579–642`).

   **Required:** preserve the loss cutoff through deferred settlement. Add deterministic record/book and UHD-hook tests for loss while the boundary send is unresolved: success/positive short → `cancelled` before `b`; exception → `uncertain`; zero → `cancelled`. Assert the intermediate `pending` row and final Manifest outcome. Add a mutation that discards the deferred cutoff—not just C2-15’s immediate loss argument.

### Minor

2. **Two accepted switches lack an outcome-isolation test.** **VERIFIED coverage gap.**

   `test-plan.md:129` covers a refused second switch. The existing positive twin checks only acceptance and predecessor metadata (`crates/ezsdr-radio-uhd/src/provider/tx/tests/e2a.rs:464–472`).

   Add: S₁ sends and applies → accept S₂ → fail S₂’s boundary send → rows remain `[applied, uncertain]`, with the correct action IDs and predecessor. This catches carry evidence leaking between switches.

3. **The hardware evidence overstates the underflow timestamp claim.** **VERIFIED citation mismatch.**

   `test-plan.md:57` labels H4 using evidence about **BurstAck**, not an underflow’s timestamp. The cited `plan/phase8/bench-results.md:311,331` establishes ACK timing. It does not establish that underflow ticks identify the transmitted end.

   Mark that inference separately or supply the matching primary evidence. U6 can still test the specified software behavior.

4. **Fold the owner’s Python-doc decision into the implementation scope.** **VERIFIED omission.**

   `plan-decisions.md:5` requires `switch()`’s docstring to explain Manifest outcomes. The text/sub-step inventories omit that edit (`test-plan.md:196–215`). Add `python/ezsdr/session.py:498–504` explicitly.

### Other checks

The schema correction is sound: no constrained switch-outcome schema needs regeneration. The hardware audit correctly retains the clean `applied` expectation and shared FakeDevice rehearsal. Q5’s window-edge difference is accepted, not a finding. Concrete mutation anchors remain implementation-time checks.

Not checked: builds, executable probes, mutation runs or raw bench archives. No files changed.
