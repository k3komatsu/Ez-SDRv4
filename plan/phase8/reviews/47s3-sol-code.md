## Verdict: NEEDS CHANGES

What this change does: restores receive frequency, gain and antenna changes on stopped streams, snapshots them at booking, and exposes Provider refusals through the Easy API.

### P2 — Must fix

1. **A surviving earlier start can be skipped permanently.**  
   `crates/ezsdr-radio-uhd/src/provider/rx.rs:348–362`; `crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:117–125`
   - **Sequence:** stop and settle → `start_rx(at=now+1s)` → let UHD queue that start → before its origin, submit `start_rx(at=now+200ms)`.
   - Replay makes the earlier start the surviving segment; the +1s start becomes a no-op. But `uhd-rx.from` already exceeds the +1s origin. It waits out/discards the obsolete segment, then skips the surviving earlier segment. Capture receives no restarted stream although the line says it is on.
   - **Decision:** reachable through ordinary surviving Actions; **defect, fix now**, not a Ceiling. Stripping `at` hides it.
   - **Fix:** reconcile queued-segment identity and the begin cursor with the surviving plan, including the hardware’s uncancellable queued start. Add this deterministic regression and restore timed starts in layer 3. Do not silently drop the replacement.

2. **Negative channel numbers change another channel.**  
   `python/ezsdr/session.py:365–374`
   - **Sequence:** with channels 0/1 in stream 0 and channel 2 in stream 1, call `rx_streams[0].start(frequency={-2: 2.4e9})`.
   - `channels[-2]` resolves to channel 1. The client submits `radio.rx.c1.frequency_hz` and then starts, instead of rejecting the invalid logical channel number. I reproduced this with the existing mocked connection.
   - **Fix:** reject negative numeric keys before indexing; add a nothing-submitted assertion for negative keys.

3. **An immediate booking refusal can still look successful through O2.**  
   `crates/ezsdr-radio-uhd/src/provider/core.rs:530–533`; `crates/ezsdr-radio-uhd/src/provider/control.rs:315–316`
   - **Sequence:** stop → submit `start_rx` on a 1 MS/s stream with root-domain `at.ticks=i64::MAX`.
   - Lattice rounding overflows. The Provider emits `COMMAND_REJECTED` with **no Action id**, despite having that id at booking. The server’s id filter omits it, so `Submitted.refused=[]` and Python returns successfully.
   - **Fix:** propagate the dispatched id into immediate booking failures. Extend the existing overflow test through `Control::book` and the submit reply, asserting `Rejected` with the matching id.

### P3 — Should fix

4. **The receive-lead measurement deliverable is missing.**  
   `plan/phase8/reviews/47s3-test-plan.md:332`
   - Ruling 7 relies on measuring restart-with-configure and rejecting a candidate on **SHORT or LATE**. Documentation is not that measurement.
   - **Fix:** restore the compile-tested bench group before closing this step, or obtain an explicit scope disposition and named follow-up before bench use.

5. **S7’s implemented tests are narrower than its reviewed plan.**  
   `crates/ezsdr-radio-uhd/src/provider/rx.rs:2513–2539`; `plan/phase8/reviews/47s3-test-plan.md:119–123`
   - The past-origin test manually injects `RxRecv::LateCommand`; it does not observe the fake producing it or the first block’s gap/lost count. The planned SHORT-then-LATE sequence and loose slow-apply integration test are absent.
   - **Fix:** add the deterministic SHORT→LATE case and observe actual receive recovery through its first block. Record any remaining omissions explicitly.

6. **Accepted-spec edits contain false claims and remove unrelated API contracts.**  
   `design/07-radio-model.md:237–240`; `design/09-mock-radio.md:205–208`; `design/03-spec-and-binding.md:287`; `design/16-easy-api.md:99–100`
   - RM-21 still says the Kernel refuses every scheduled radio value; receive schedules now pass validation and are Provider-refused. MR-34 retains the old validate-refusal citation; SB-30 still says runtime checks see no radio updates.
   - EA-16 accidentally loses the unchanged rows for capture/request/result, time helpers, targets, raw submission, child Runs and artifact helpers.
   - **Fix:** correct those statements and restore the unrelated rows. Keep process history in Changes/decision records rather than normative rules.

### Remaining review conclusions

- Shared `Staging` and replay-based `Line::streaming` avoid duplicated streaming state. Booking snapshots, refused-value retention, peer isolation and configure-once wiring are otherwise sound.
- The lead threshold, unchanged origin, event source/severity and both schema changes match ruling 7 by inspection.
- O2’s normal paced/stepped paths and Python stopping at the first refusal are sound apart from finding 3.
- The missing server interrupted-wait twin is acceptably covered by the Kernel interrupted-wait test plus the pure server rule. Restricting generated values to c0/s0 is acceptable alongside explicit peer tests.
- Missing standalone parity/control-loss tests do not independently block this change; however, the three differential layers cannot replace timed owner coverage while `at` is stripped.
- The 31 retained `N47S3-*` rows and eight retargeted rows match their stated behaviours by inspection. Their probe log reports kills; I did not independently execute them.

**Checked:** read-only review at `d8f23af`; all five `StartValues` tests pass, and the negative-key counterexample reproduces.  
**Not checked:** Rust builds, mutation execution or hardware. No files changed; the timed-start defect is code-traced, not bench-reproduced.
