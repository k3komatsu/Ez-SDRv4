My failure scenarios to check first:
- A repeat starts late, so a switch observes a send but not the required playing state.
- Two switches submitted together are processed across a boundary, allowing both instead of refusing the second.
- The test thread pauses after switch receipt; the “pending” switch applies before Provider stop.
- A late timed booking and its untimed follower share target ticks, hiding a wrongly attributed TIME_ERROR.
- Changing the repeat policy preserves successful playback but loses a refusal or timing assertion the original test covered.

I’ll check these against the contracts and run a separate-delay probe outside the repository.
The requested injection passed at 5 ms (12 tests once) and 50 ms (12 tests, five runs). A different scheduling gap still fails: pausing 50 ms between the helper’s two UHD pushes lets the first switch apply, so the second is correctly accepted and the test fails. The helper does not guarantee “one receipt.”
The Provider-stop test also still has a scheduling window: a 5-second pause after switch receipt produces `["applied"]` instead of `["cancelled"]`. The 1-second target reduces the chance of failure but does not satisfy note 37’s rule. The differing Mock/UHD times are otherwise valid for comparing cancellation outcomes.
## Verdict: FAIL — two majors

Both are **VERIFIED**, `mech:wall-clock-test-oracle`.

1. **The 1-second switch target still leaves a scheduling window.**  
   `crates/ezsdr-acceptance/tests/parity.rs:2299–2304`  
   Minimal sequence: observe repeat → submit switch → test thread pauses → call Provider stop. A **5-second sleep after `o1_uhd_at` returns** reproduces `["applied"]` instead of the expected `["cancelled"]`. Correct Provider behaviour therefore fails the test.  
   Different Mock/UHD times are meaningful for comparing cancellation **provided `B ≤ boundary`**; the extra second does not guarantee that precondition. Use the frozen rig for the unconditional pending-cancellation assertion, not more lead.

2. **The iterator helper does not submit switches atomically.**  
   `crates/ezsdr-acceptance/tests/parity.rs:2598–2600`, caller `:2189`  
   Minimal sequence: enqueue first switch → test thread pauses **50 ms** → enqueue second switch. The first applies during the pause, so the second is correctly accepted. The test fails with refusal kinds `[]`, expected `["tx_burst"]`.  
   Enqueue the complete batch under the existing queue mutex; separate `push` calls do not establish “one receipt.”

Probe logs: `/tmp/opencode/n96-sol-gaps.log` and `/tmp/opencode/n96-sol-stop5000.log`.

### Other checks

- **Requested injection:** 12 affected/O1 tests passed with 5 ms once and 50 ms five times, injected after `wait_until` at all four named helpers.
- **Policies and witnesses:** SendAsapAndFlag matches RM-12 and preserves these playback assertions. Observing the first timed payload send is sufficient for UR-38/KL-41 here: `TxBook::issue` marks the repeat started before that device call. Helpers reuse the existing machinery without substantive duplication.
- **TIME_ERROR attribution:** target ticks correctly distinguish B from the untimed booking (`30 ms` versus `30 ms + 3 samples`); removing the impossible non-switch action-id expectation is appropriate.
- **Pre-existing caveat:** the wrong-policy test drains events at `:2233`, then checks another drain at `:2236–2237`; its “no TIME_ERROR” assertion is not a reliable check.
- **Deferred tests:** independently reproduced both `s15_parity_b_first_accepted_booking_rule` and `s1_parity_stop_during_padded_device_burst` failing with 50 ms injection. Their hand-over/post-dispatch receipt windows support moving those properties to frozen rigs; “no possible sequence” remains **INFERRED**. The report actually lists 14 remaining tests; its 15-failure log includes the subsequently fixed payload-padding test.
- **Mutations:** the requested grep found no changed test as a killer. N47S5-103/104 still target untouched `s15_parity_b_first_accepted_booking_rule`; their intended mismatch assertions remain unchanged, but its scheduling failures can still produce false kills.

Not rerun: mutation suite, full workspace, or hardware. Repository files were not modified.
