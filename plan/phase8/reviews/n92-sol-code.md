My independent failure scenarios:
- Control panics while holding stream state, between Actions, or after Stop is booked: loss must publish once without deadlock.
- Control and a data owner panic together: only one loss; surviving owners must stop new device work after observing it.
- Default Policy must end the Run; `continue` must leave the radio inert, including queued/future Actions.
- Stop, cleanup-only, failed start, and a detached control thread must not lose or duplicate the plan row.
- A loss during active TX must be checked separately for stream termination and SampleClock closure; these are not necessarily the same operation.

I’ll compare these against the implementation and final test plan.
The open TX SampleClock matches the current contract: UR-23 explicitly limits clock closure to a loss/stop at or before its origin. TM-12 permits an unset `ended_at`; I found no general loss-closure requirement. #92 does not depend on changing that model.

I also found a validation-process breach: the report and `mutrow.py` show mutation rows applied manually in a copy and tested through `rdev`, bypassing the mandatory `remote_mutate.py` runner. I won’t count those runs as mutation-gate evidence.
A later `mut.log` records the full remote mutation run: all ten N92 rows killed, with only the known phase7/G09 survivor overall. That supplies later evidence, though it does not erase the earlier runner bypass.

Two minor issues remain: the “repeat has started” checks also match arm’s four-zero flush, and UR-16’s next-Action timeout claim applies only when control died—not when an RX/TX panic leaves control alive to drain Actions.
## Verdict: PASS WITH MINORS

The change fixes #92 through the existing once-only loss path. No major correctness finding.

### Minor findings

1. **Repeat-start checks match arm’s flush. — VERIFIED**  
   `crates/ezsdr-radio-uhd/tests/fake.rs:8170,8275` accept any `tx_send`, including the four-zero arm flush (`:8212`). They do not establish the planned “repeat playing” precondition. Require an experiment payload send before arming the panic.

2. **UR-16 overgeneralizes the next-Action timeout. — VERIFIED**  
   `design/18-uhd-radio.md:220` promises KC-21a failure after any thread panic under `continue`. After an RX/TX panic, surviving control still drains and acknowledges Actions without booking them (`provider/control.rs:241–245`; `ezsdr-kernel/src/coordinator/state.rs:72–80`). Qualify that consequence with **“when uhd-control has died.”**

3. **Removed control responsibility remains documented. — VERIFIED**  
   `design/18-uhd-radio.md:309` still says control reads `S` from the stop/loss booking. That reader disappeared with `Control::finish` and `Streams::ended_at`. Remove the stale sentence.

### Requested checks

- **Loss handling:** poison recovery and serialized once-only publication are intact (`provider/core.rs:1039–1060`). Owner loss checks and shutdown paths prevent further device work after observing loss. No concrete second-panic or deadlock counterexample found.
- **UR-30:** one writer, guarded by control having started; repeated stop/cleanup cannot duplicate it (`provider/mod.rs:679–704,723–741`). Cleanup-only follows stop. Detached-control preservation is **INFERRED**, not directly tested. Differential assertions read plans directly, so removing `finish` preserves their observations.
- **TX SampleClock:** staying open after a post-origin loss is **not a demonstrated contract defect**. UR-23 explicitly limits closure to loss/stop at or before origin (`design/18-uhd-radio.md:260,271`); TM-12 does not require universal loss closure. The planner implements this deliberately (`timeline.rs:434–464`). **#92 does not depend on changing it.**
- **Tests:** unit panic scenarios wait for finished handles before stop, avoiding the identified one-second join race. Seams are test-only. Integration tests still exercise ordinary bounded shutdown.
- **Mutations:** all ten N92 anchors uniquely match; P276 is absent. Spot-checked N92-1 and N92-7b: plausible, assertion-driven killers. Later `mut.log` records all N92 rows killed. The earlier `mutrow.py:12–15` runner bypass remains a process violation, not valid gate evidence.
- **RF wording:** no immediate-silence promise remains.

No edits, builds, or runtime probes performed. Hardware silence timing and detached-control behavior remain unverified experimentally.
