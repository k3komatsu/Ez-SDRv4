# Opus re-check: fix round e04a2d7d (7795f104..e04a2d7d), bench tools S4 / S5

Read-only. I read the diff, the connected code (`ezsdr-sink-capture` `push_continuity`, kernel `ContinuityBuilder::push`/`finish`, `provider/mod.rs` UR-5 subdev setting, `s9bench.rs` `FirstBlockLink`, `extract_overflow_trial`), Sol's review and the implementer's report. I also ran the eight named tests locally (`CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `--test fake`, 8 passed, 15.96 s) and read `rows-fix.log` (13/13 killed, all `real_clock: false`, no reruns).

## Step 1: my scenarios, written before I read the review

S1. The recorder misses blocks the Provider publishes: some publish path does not go through `assembly.links`, for example a companion TX stream. Result: a wrong map. **Cleared.** The profile has one placement link (`radio/rx0 → rec/in`, `common/mod.rs:306`). The companion is a `tx_burst` schedule action with an input waveform, not a DataLink (`mod.rs:343-353`). Every entry in `assembly.links` is replaced (`measurement.rs:2942-2944`). The end-to-end fake test finds the gap through the real Run path.

S2. The recorder sees blocks that the old Sink map did not, or the reverse.
- (a) Link drops (DropOldest, capacity 64). The Sink folded them in as a `DropCarry`, which could produce `link`/`mixed` causes. The recorder folds the Provider's headers with no carry, so its map is the Provider's publication. That is what S4 measures, so this is an improvement.
- (b) The window. The Sink map ended at the capture's `n`. The recorder's map runs to the Run's end. The only things that could now count are extra `overflow_restart` gaps or a domain change late in the Run. Overflow events were always counted over the whole Run, so a second overflow was already disqualifying. A domain change needs a retune or rate change, and S4 has neither.
- (c) The first block. The Sink clipped a leading gap at its capture start (`clip_leading_gap`). The recorder does not, and counts a first-block `GAP_BEFORE` back by `lost`. This only moves `first` earlier, which makes the restart-after-resume estimate larger (the conservative side). A timed RX start publishes no first-block gap (INFERRED).

S3. A 1-sample capture completes, and the Run then ends or the link closes before the stall, so no overflow is ever seen. **Cleared (VERIFIED on the fake, same Kernel/Sink/link code as hardware).** `s4_overflow_manifest_reads_the_gap_from_the_headers_and_captures_one_sample` reaches `Success` with a 300 ms stall that starts 200 ms after the first block. That is long after the one sample was written.

S4. A valid 200 MS/s overflow still reads INCONCLUSIVE because evidence ends before the gap closes. **Closed by construction.** No capture length gates the map any more. The map runs to the horizon, `2000 + after + stall_recovery + 500` ms (`overflow_wait_ms`). The gap closes at the first post-restart block.

S5. An invalid trial reads PASS. A refused fold (`TimeOverlap`, `GapFlagWithoutJump`) yields `{}`, so the trial is Incomplete. No blocks also yields `{}`. Two maps make the trial Incomplete. `partial: false` is hard-coded, but `valid` still needs exactly one RX_OVERFLOW event of cause overrun plus one matching gap (`lost == len == payload.lost`, same domain/tick/stream). An aborted Run cannot pass without those. One weakening: the event-vs-gap cross-check now compares two Provider outputs, where before one side had passed through the link and the Sink. The header is the source in both cases, so nothing substantive is lost (note only).

S6. Timing and back-pressure on hardware. The recorder adds an uncontended mutex and one `ContinuityBuilder::push` per published block on the publishing thread. `push` is O(channels) with no allocation on a contiguous block (`continuity.rs:180-298`). At 200 MS/s and a few thousand samples per block, that is about 10^5 pushes/s, well under 1 µs each (INFERRED). The Sink no longer writes about 1.6 GB/s of cf32 or hashes it, so total host load falls. RS-8a cleanup no longer waits on a large drain. Whether RS-8a at 200 MS/s came from the Sink at all is still open (INFERRED). bench.md keeps the stop / power-cycle / owner procedure (`bench.md:1514`).

S7. The S5 widening and unwind.
- #1 (OBX A + UBX B): the earlier passes leave `A:0`. `widen_rx_map` sets `A:0 B:0`, giving 2 channels, OBX/UBX, and the selector `{rx: [A:obx, B:ubx], tx: [A:obx]}`. Then `with_selector` runs and restores the slot-A pin afterwards, also after a panic. The Module sets its own map at every prepare (`provider/mod.rs:276-286`, VERIFIED in code), so leaving the physical map widened is harmless.
- #2 (UBX A, empty B): UHD enumerates slot B as `Unknown (0xffff) - 0` (VERIFIED: `~/.cache/ezsdr-bench/2026-11-16/probe.37.txt:117-121`). The widening is therefore most likely accepted (INFERRED). `front_end(Rx,1)` then gives "not a known board", which yields one INCONCLUSIVE line. If UHD refuses instead, the new error path yields one INCONCLUSIVE line that names the widening.
- A latched guard (`channels()` = 0): the widening is attempted and its own error goes on the decision line.

## Step 2: adopted findings

| Sol finding | Closed? | Evidence |
|---|---|---|
| 1 (major) 200 MS/s cap ends evidence before the gap | Yes | The cap and its formula are deleted. Evidence comes from published headers through `HeaderMapLink` (`measurement.rs:2816-2923`), and the Sink writes 1 sample (`:2807`). |
| 2 (major) the changed behaviour was untested at the call site | Yes | `fake.rs` `s4_overflow_manifest_reads_the_gap_from_the_headers_and_captures_one_sample` runs the real `overflow_manifest`. It checks one map, one `overflow_restart` gap with `lost == len > 0`, `overflow_trial` Success with an estimate, `rec[0].size_bytes == 8`, and the other side of the boundary (stall 0 gives no gap and Incomplete). The pure `overflow_receive_spec_asks_for_one_sample` pins the Spec's count. Killers: TOOLS-21/22/23/24/25, all assertion or test-panic kills. |
| 3 (minor) override leaked on unwind | Yes | `with_selector` restores the previous override in `Drop` (`mod.rs:161-176`). `selector_tests::with_selector_restores_the_previous_override_on_a_return_and_after_a_panic` covers return, panic and none. TOOLS-03 is retargeted to the Drop and killed. All 19 `with_selector` callers keep their behaviour: none relied on the old reset to `None`. The workspace suite was green per the report. |
| 4 (minor) widening error discarded; the #2 fake bypassed the widening | Yes | `widen_rx_map` (`s4bench.rs:503-509`) carries the error. `s5_two_channel_selector` is now read-only. New tests: `…widens_a_one_channel_map_before_it_reads_the_slots` (TOOLS-32) and `…puts_a_failed_widening_on_the_decision_line` (TOOLS-31). TOOLS-05 is re-anchored and killed. |
| 5 (minor) bench.md tail / SIGSEGV wording, stale comment | Yes | `bench.md:1437,1512-1514`: SIGSEGV is now "observed", and the teardown cause is INFERRED. The stale `+250 ms` comment in `fake.rs` and the `hardware.rs:4212` comment are updated. |
| 8 (note) C2 deferral in the checklist | Yes | `bench.md:1584,1626,1633,1711` mark 100/200 MS/s as deferred to bt2 and "do not run"; C3–C6 lists are adjusted. |

Hardware-test rule (AGENTS §9): `hw_s4_overflow_gap` and the record suite's `measure_overflow_restart_gap*` both go through the changed `overflow_manifest`. The FakeDevice rehearsal books both sides of the gap-closing boundary (stall 300 ms: Success; stall 0: Incomplete) through the same function. The 65 536-block fake also still reaches Success. `hw_s5_lead_rungs` goes through `s5_passes`, which is rehearsed on the mixed OBX+UBX fake and on both #2 branches (refuse: new test; accept + unknown board: the existing `one_obx` case, which starts at 2 channels).

## Findings

1. **Minor: the recorder duplicates two existing pieces of code.**
   - `HeaderMapsState::push` (`measurement.rs:2828-2849`) is a line-for-line copy of `ezsdr-sink-capture` `push_continuity` (`lib.rs:569-591`), including the SC-30c domain/channel rollover.
   - `HeaderMapLink` / `HeaderRecording` (`measurement.rs:2875-2923`) repeat S9's `FirstBlockLink` / `Recording` delegating wrapper (`s9bench.rs:37-90`).

   What would have to change together: a change to SC-30c rollover in the Sink would leave S4's evidence map built by the old rule, and the compiler would not flag it. A new `DataLink` method would need adding to both wrappers, but the compiler does flag that one. Test-only code, so no correctness risk now. A single `HeaderTap(Fn(&BlockHeader))` link in `common/` could serve S9 and S4. Optional; it can wait for the Phase 8-end test-architecture review.

2. **Minor: dead state in the recorder.** `HeaderMapsState.current` is `Option<(ContinuityBuilder, ClockDomainId, u16)>` (`measurement.rs:2819`). The domain and channel fields are written at `:2830` and `:2845` but never read; every destructuring is `(builder, _, _)`. They were carried over from the Sink. `Option<ContinuityBuilder>` is enough.

3. **Minor (docs): "the same `ContinuityMap` the Sink makes" overstates the equivalence.** `bench.md:1437,1512` say this. The recorder's map differs from what a Sink capture of the same Run would produce in two ways: it spans the whole Run rather than the capture window, and it carries no link-drop carry (`DropCarry::default()`). Both differences are deliberate and favourable for S4. The text should say "the same builder, folded over every published header" (the code's doc comment at `measurement.rs:2815` already says it correctly), so that a reader comparing against an old Sink artifact does not expect identical gap lists.

4. **Note: the #2 rehearsal models the branch hardware is less likely to take.** `s5_two_channel_receive_lead_widens_a_one_channel_map_before_it_reads_the_slots` (`fake.rs`) uses a fake that refuses `B:0`. Real #2 enumerates slot B as `Unknown (0xffff)` (VERIFIED, probe.37.txt:121), so UHD probably accepts the widening and the pass stops at "not a known board" (INFERRED). That accept branch is rehearsed only from an already two-channel fake (`one_obx` in the existing test), not as one chain starting from a 1-channel map. Both branches end in one INCONCLUSIVE line, so the verdict on hardware is the same either way. The test comment says this honestly.

5. **Note: the new widening assertion in the mixed-fake test is redundant.** In `s5_passes_run_all_three_passes_on_a_mixed_obx_ubx_fake_and_restore_the_slot_a_pin`, the `calls[..third]` search for `set_subdev_spec rx A:0 B:0` would also match the Module's own UR-5 call at the two-channel Run's prepare (`provider/mod.rs:284`), which precedes `rx_open`. The assertion still holds only when the earlier map was 1-channel, because without the helper the selector fails first. TOOLS-05/32 are killed by other assertions. Correct, but weak on its own.

6. **Note: the stall-0 side shows only that no overflow is published.** The "other side of the boundary" (stall 0, `fake.rs`) shows that a non-overflowing Run reads Incomplete. It does not exercise a near-threshold stall on the fake's 100 ms behind threshold. The implementer reports no dedicated probe of this assertion. Adequate for a measurement tool: the verdict logic itself is pinned by the pure tests on `extract_overflow_trial`, which this round left unchanged.

No leftovers found: `OVERFLOW_CAPTURE_MAX_SAMPLES`, `OVERFLOW_TAIL_MS`, `OVERFLOW_BUFFER_BYTES`, `overflow_capture_samples` and TOOLS-11..14 are gone from code, rows, bench.md and `plan/maintenance` (grep over `crates/`, `plan/maintenance/`, `plan/phase8/bench.md`). There are 13 TOOLS rows and all are killed.

Verdict: PASS
