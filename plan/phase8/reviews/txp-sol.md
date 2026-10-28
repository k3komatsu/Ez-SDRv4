# 段階 TXP：Sol のレビュー

## 試験計画のレビュー 1

> plan · gpt-6.1-sol
## Independent failure scenarios

Before reading the plan, I identified:

- Priority requested on the wrong thread, repeatedly, or after sending; forwarding wrappers swallowing it.
- EPERM/unsupported scheduling incorrectly becoming device loss, including through a measurement wrapper.
- An RT owner spinning while window-blocked, starving control/RX or encountering RT throttling.
- Minimum-send waiting crossing the strict window, delaying Stop/preemption/cold cuts, splitting tails incorrectly, or missing the first timed SOB’s lead.
- Waiting hiding timeline expiry, consuming a held continuation tail prematurely, or disturbing timed-command order.

## Comparison

The direction is sound: keep the chunk unchanged, request priority once in `Tx::run`, record refusal without using `device_failed`, and wait only when the window cannot accommodate the otherwise permissible send.

I independently verified UHD 4.10’s [thread implementation](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/utils/thread.cpp) and [C wrapper](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/utils/thread_priority_c.cpp): `(0.5, true)` requests SCHED_RR on `pthread_self()`, and scheduling failure throws `os_error`. The repository maps UHD error 31 to `lost = true`. The plan correctly recognizes that trap—but not every place it can occur.

### Major 1 — Priority refusal can poison `GuardedDevice`

**Evidence:** Plan §1.1 and §8.7; `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:140–159`.

`refuse_or!` passes errors through `note`, which latches any `error.lost`. Mechanically forwarding the new method using that wrapper pattern makes EPERM latch the measurement device **before** `Tx::run` ignores `lost`. Subsequent device calls fail, and the inner handle is leaked. P4’s direct `PrioritySpy` would miss this.

**Required change:** Specify that priority refusal never creates a device-failure latch. Preserve any already-latched guard’s refusal behavior, but do not feed a scheduling error into `note`. Add a test through the actual measurement-wrapper chain: inner priority returns `lost: true`; latch remains empty; payload still transmits; rejection is recorded once. Also verify successful forwarding reaches the inner device.

A required trait method prevents omission, not an implementation that returns `Ok(())` without forwarding.

### Major 2 — The prescribed gate misses newly opened continuations

**Evidence:** Plan §1.2, M4/M9; `crates/ezsdr-radio-uhd/src/provider/tx.rs:312–333,364–396`.

The plan places the idle decision in `step`’s existing-open branch. However, a newly selected continuation reaches the separate `send` at line 395. It is a Kernel burst’s first buffer, but **not** a device SOB; it must wait. Merely following the stated placement sends its undersized payload and possibly its predecessor’s held tail.

M4/M9 are valuable because they expose this, but the implementation prescription contradicts them.

**Required change:** Apply one shared sizing/readiness decision to **both** send entry paths, before any tail or payload device call. Exempt only `device_sob`, not `open.first`. Explicitly specify the newly opened, waiting continuation’s state and false return. Retain M4/M9 and add a mutation bypassing the second entry path.

Also correct §1.2’s pseudocode: an exempt SOB with `avail < wanted` still sends `min(wanted, avail)`, never `wanted`.

### Major 3 — The no-spin/Stop tests have incorrect or insufficient oracles

**Evidence:** Plan M6/M8 and §5; `src/provider/tx.rs:137–156,757–763`; `src/host.rs` M2–M5.

- M8’s count is wrong: its initial state sends three 2,000-sample buffers; over another 10 ms, five more fit. The expected total is **eight**, not at most six.
- Counting payload sends does not distinguish sleeping from repeatedly polling without sending. A wrong implementation can preserve the same send sizes while burning CPU.
- With this manual host clock, a busy participant freezes time. A spin may therefore hang the scenario rather than produce the claimed send-count failure.
- M6 expects a cancellation rejection for an open repeat. `stop_now` records cancellation for held bursts; ending the sole open burst does not create that rejection.
- P1 introduces a real-clock waiting test, contrary to the brief’s deterministic-test constraint; §5 also overlooks P1–P6 when identifying timing-sensitive tests.

**Required change:** Correct the count and Stop oracle. Specify synchronization that observes entry into the actual idle wait, then queues Stop and observes EOB processing at the next wake. Use the existing host-clock test facilities, with bounded failure handling, rather than payload counts as proof of sleep. Make priority tests deterministic and update the repetition list.

### Major 4 — Cold-cut behavior is not tested at the new waiting boundary

**Evidence:** Plan M4/M4b and X14; `src/provider/tx.rs:303–332,410–414`.

The new arithmetic independently handles `held_cut` and `switch_cut`, but M4/M4b exercise only a held burst. Removing only `switch_cut` from `wanted` is not demonstrated to fail any named new test. Generic existing cold-change coverage is not a specified substitute.

**Required change:** Add a manual-clock cold-cut case: while window-blocked, the remaining distance to the cut is below `chunk`; wait until that smaller remainder fits, transmit exactly up to the exclusive cut, then end/switch without payload beyond it. Assert the new clock/stream transition and add a switch-cut-only mutation.

### Major 5 — The hardware acceptance plan does not observe the main scheduling risks

**Evidence:** Plan B1 and §8.4; `plan/phase8/bench-results.md:151–173`.

B1 tests TX alone at 100 MS/s. It neither covers the motivating 200 MS/s condition nor observes control/RX starvation. A successful priority request is not evidence that those threads continue meeting their obligations.

**Required change:** Add owner-run acceptance checks at low rate (window-bound), 100/200 MS/s, and duplex with timed control updates plus Stop. Record actual owner policy/priority, underruns, RX progress, control issue times and Stop completion. Record effective RT bandwidth settings; Linux’s [default throttling allowance](https://man7.org/linux/man-pages/man7/sched.7.html) is not a latency guarantee. Keep hardware conclusions INFERRED until measured.

## Minor changes

1. **Feature-gated mutation coverage:** X8–X10 modify `uhd.rs`, excluded without `--features uhd` (`src/lib.rs:13`). The remote mutation image lacks UHD. Specify a runnable feature-enabled gate or explicitly identify these rows as deferred; ordinary `lib`/`fake` mutation runs do not cover them.
2. **Small-window claims:** Replace “always reaches `wanted`” and the claimed guaranteed `0.75W` lead with qualified statements accounting for polling delay, scheduling delay, rounding, and `chunk = 1`. M11 needs an admissible concrete setup; do not imply TXP fixes existing `W ≤ lead` dispatch problems.
3. **Error evidence:** Keep the numeric UHD code authoritative and error text advisory. Add an unsupported/error-code case without asserting that every unsupported platform returns ENOSYS—UHD also has a dummy priority implementation.
4. **Timed-command order:** Name the existing UR-24 queue-order/DUC tests retained as regression coverage. The cited control-open ownership test alone does not establish command order.

## Verdict: **FAIL — 5 majors, 4 minors**

Revise the plan before implementation. Keep the proposed narrow design; no chunk enlargement, wrap-spanning buffers, RX-priority change, or investigation of device #2 is needed here.

Read-only review of committed code and primary sources; no edits, builds, tests, mutations, or hardware runs. Actual scheduling and throughput remain unverified.

## 試験計画のレビュー 2

> plan · gpt-6.1-sol
VERDICT: FAIL

The main design corrections are sound: G1–G3/X21–X23 cover forwarding and latch behavior; both send paths share the readiness decision; only `device_sob` is exempt; waiting continuations preserve their state. M8’s total is correctly eight. M14/X14b, feature-gate deferral, small-window qualifications, and F2’s numeric codes/dummy-build caveat address their earlier findings.

The following plan changes remain necessary. References below are to `txprio-test-plan.md`.

## Major

1. **M6 still cannot detect an overlong idle sleep** (lines 92, 157).

   Observing `host.blocked()` and EOB at the “next wake,” with a 10-second failure bound, does not distinguish IDLE = 200 µs from sleeping the missing buffer duration. Both implementations enter a wait and process Stop at their next wake. X19 can survive.

   **Change:** Hold the owner at a controlled wake, queue Stop before releasing it, and assert the owner’s host-clock interval from idle entry to that wake is IDLE. Then assert Stop produces EOB before another payload send. Keep the real-time timeout only for bounded failure, not as the responsiveness oracle.

2. **M8b does not yet specify a reachable, isolated three-wake observation** (lines 95, 120–122).

   FakeDevice time advances with its HostClock (`device.rs:845–856`). Holding it fixed using `Frozen` also prevents the owner’s timed sleeps from completing. Conversely, releasing the host clock allows device time to advance.

   Also, `hooks::passes()` is not a count of completed idle sleeps: it includes wait entry and condition-variable wakes. Reading its total after Shutdown includes Shutdown’s report waits (`tx.rs:939–955`). One idle wait plus Shutdown can satisfy a threshold of three without proving three idle cycles.

   **Change:** Specify a bounded host-clock observation interval, with explicit wake handshakes and snapshots taken **before Shutdown**. Count distinct idle cycles, exclude report/shutdown waits, and keep the interval below the next payload-ready boundary. Drop the fixed-device-time claim unless the test explicitly decouples device time from host time. Require unchanged payload calls throughout that interval.

## Minor

3. **P2 asserts the priority call occurs after sending** (line 65).

   With increasing shared call numbers, `priority > first tx_send` is the reverse of the required ordering. Correct code fails; the named wrong implementation passes.

   **Change:** Assert `priority_call_number < first_tx_send_number` separately for each owner, and explicitly require that its first send exists.

4. **B0’s “each send ≥ chunk” criterion contradicts the accepted rule** (line 109).

   A correct implementation may send less than `chunk` at the first timed SOB, waveform wrap, final remainder, or held/cold cut. Section 1.2 deliberately preserves those cases.

   **Change:** Specify the repeat waveform length and evaluate ordinary, non-boundary continuation sends separately. Require those sends to equal `chunk`; validate boundary sends against their permitted smaller sizes. Define a measurable CPU-reduction comparison rather than an unqualified “goes down.”

5. **The UR-24 queue-order regressions are still not named explicitly** (line 103).

   The named queue-count test checks capacity; “`ur_24_*` queue-order tests” leaves the intended ordering coverage unspecified.

   **Change:** Name at least `ur_24_the_device_queue_order_is_the_effective_order` and `ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order` in `provider/control.rs`. Retain the already named DUC regressions.

Read-only review against HEAD `d0a0e64`; no edits, builds, tests, or mutations. Hardware scheduling and throughput remain unverified.

## 試験計画のレビュー 3

> plan · gpt-6.1-sol
VERDICT: FAIL

Two findings remain in `txprio-test-plan.md`.

1. **Major — M8b still depends on when the test thread runs** (line 95).

   Polling every ≤1 ms does not guarantee a snapshot before the 12.5 ms payload-ready boundary. If the test thread is descheduled for 20 ms, a correct owner continues sleeping and sending. The eventual snapshot then contains more than three payload calls and is past the boundary. This violates design note 37; enlarging the boundary only reduces the probability.

   **Concrete change:** Have `TxSpy` capture the snapshot on the owner thread at the fifth qualifying idle `tx_async`, then hold that owner with a channel handshake until the test queues Shutdown and releases it. This ordinary channel wait keeps the registered owner busy, freezing the manual clock. Assert the five distinct host timestamps, their intervals, and unchanged payload calls from that captured snapshot. Bound receipt of the handshake only to turn hangs into finite failures. No production clock API change is needed.

2. **Minor — B0’s wrap-size bound rejects a permitted short SOB** (line 109).

   A 31,000-sample waveform produces a 1,000-sample wrap remainder only after an aligned full-chunk start. Section 1.2 permits a smaller first timed send. For example, SOB = 1,500, followed by fourteen 2,000-sample sends, leaves a valid **1,500-sample** first-wrap block. B0’s ≤1,000 assertion rejects it. The existing send path computes the remainder from the actual offset (`crates/ezsdr-radio-uhd/src/provider/tx.rs:407–409`).

   **Concrete change:** Validate every wrap-ending block against `min(chunk, 31_000 − offset_before_block)`, using the accumulated actual lengths. Keep ordinary non-boundary continuations equal to chunk. The CPU comparison against `d0a0e64`, with reduction recorded rather than an invented percentage gate, addresses the previous finding.

**Other round-2 findings are resolved:**
- **M6:** The controlled wake retains its deadline until release (`host.rs:228–250`), so delayed test execution cannot inflate `T_wake`. IDLE passes; X19’s 12.5 ms sleep fails the `< 3·IDLE` check. Stop queued before release can be checked for EOB before further payload.
- **P2:** Correct per-owner ordering and explicit send existence.
- **UR-24:** Both queue-order regressions and the retained DUC regressions exist at HEAD.

Read-only review; no edits, builds, tests, or mutations. Hardware scheduling and throughput remain unverified.

## コードレビュー

> plan · gpt-6.1-sol
VERDICT: FAIL

The production change appears sound. Two validation issues need fixing; one planned sequence remains incomplete.

## Independent scenarios checked

- Priority requested once on each TX owner’s thread, before sending; refusal recorded without device loss or poisoning the measurement guard.
- Exact-fit and insufficient window space; first timed SOB exemption; continuation waiting through both send entries.
- Held-tail preservation, Stop, preemption, timeline expiry, repeat wrap, finite-burst remainder, and cold cuts.
- Waiting reports no work, sleeps rather than spins, and does not delay Stop until a whole buffer fits.
- Removal of the old window-edge return preserves strict payload exclusion.

UHD 4.10’s [thread implementation](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/utils/thread.cpp) and [C wrapper](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/utils/thread_priority_c.cpp) confirm the calling-thread operation, scheduling-error path, and dummy-build caveat.

## Major findings

### 1. M8b can fail correct code after a setup delay

**Location:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:5438`

**Problem:** The assertion compares an **absolute** host timestamp against 12.5 ms, rather than measuring elapsed time from the window-filled state.

**Evidence:** `txp_idle_owner` creates the device at line 5402 but freezes its host clock only at line 5407, after constructing the rig. If the test thread is descheduled for 20 ms between those points, that time becomes part of every recorded host timestamp. `device_at(..., 0)` resets device time, not the host clock. Correct idle behavior then fails the absolute bound.

The adjacent-gap bounds at lines 5439–5441 are different: the manual clock stops at participant deadlines, so ordinary scheduling delays do **not** inflate those intervals. M6’s controlled-wake interval is likewise protected.

**Fix:** Subtract the third payload send’s host timestamp—or the first qualifying idle timestamp—from the snapshot timestamp. Keep the owner-side snapshot and handshake. Do not enlarge the absolute bound.

### 2. B0 requires per-buffer information the Manifest does not contain

**Location:** `plan/phase8/bench.md:216`

**Problem:** B0 instructs the operator to classify `bursts[].blocks` by accumulated sample offset and check individual block lengths. That field is a block **count**, not a sequence of blocks.

**Evidence:** `crates/ezsdr-kernel/src/stream/burst.rs:177–189` defines `blocks: u32` and aggregate `samples: u64`. TX serializes those records at `crates/ezsdr-radio-uhd/src/provider/tx.rs:538–542`. The Manifest therefore cannot distinguish full-sized continuation buffers from window-edge slivers.

**Fix:** Specify an actual per-send observation source for B0, such as a bounded diagnostic trace using the existing `ProbeDevice::tx_send` recording (`crates/ezsdr-radio-uhd/tests/common/probe_device.rs:339–351`). Explain how to exclude empty EOB calls. Keep aggregate Manifest counts for blocks/second; do not add production per-block storage merely for this bench.

## Minor finding

### 3. M14e stops before proving transmission on the new clock

**Location:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:5291–5296`

**Problem:** The test checks clock/channel adoption but never sends a burst after the switch.

**Evidence:** The planned M14(e) sequence explicitly includes the new clock’s first send (`plan/phase8/txprio-test-plan.md:101`). The implemented test ends after checking `clock`, `channels`, and `at`.

**Fix:** Continue this controlled-clock sequence with a two-channel burst on the new clock. Assert its first send has the new clock’s timed SOB and the expected sample counts.

## Other review conclusions

- Priority forwarding, refusal handling, success counting, and `UhdDevice`’s `lost: false` mapping match the intended contract.
- Sharing the readiness rule in `send` correctly covers both entries before either tail or payload transmission.
- U19’s removal is justified: a non-SOB cursor at the window edge waits through `avail < wanted`. Strict payload exclusion remains covered by the boundary tests and P637; P1109 separately covers exact-fit readiness.
- P1100–P1123 and the adjusted P637/U17/R06/B31 are consistent with their intended failure mechanisms on static inspection. X8–X10 are explicitly deferred.
- Chunk sizing, wrap boundaries, RX priority, and R4/R2/F7 remain outside the change.

**Not checked:** No edits, builds, tests, mutations, or hardware runs. Reported execution results were not independently reproduced; actual scheduling and throughput remain unverified.

## 見直し 1

> plan · gpt-6.1-sol
VERDICT: FAIL

### Major — B0’s diagnostic run is still not runnable as written

**Location:** `plan/phase8/bench.md:215`

The observation source is now correct: `ProbeDevice` records per-send `sample_counts`, and excluding empty EOB calls is appropriate. However, no existing test or command performs the specified five-second, 31,000-sample repeat through that wrapper at 1 and 5 MS/s. B0 supplies no command.

Repository-wide inspection of `ProbeDevice::new` found only four real-device uses (`crates/ezsdr-radio-uhd/tests/hardware.rs:2106,2229,2319,2442`): H5 and three Stop/RX probes. None implements B0. The bench tools contain no `ProbeDevice` runner.

**Smallest fix:** Add one ignored hardware test beside the existing probes. Reuse `usrp()`, `tx_session`, and the existing repeat/Stop actions; wrap the device in `ProbeDevice`, explicitly select block length 2,000, run a safe-amplitude 31,000-sample repeat for approximately five seconds at each rate, and export the filtered send rows. Document its exact Docker/Cargo invocation in B0. Keep this separate from the uninstrumented 60-second performance run; no production tracing is needed.

### Closed findings

- **M8b:** Closed. `provider/tx.rs:5447–5451` measures elapsed time from the third payload send and preserves the owner-side snapshot and adjacent-idle checks.
- **M14e:** Closed. `provider/tx.rs:5296–5303` now sends a two-channel burst after the switch and checks its 2,000-sample timed SOB against the new clock.

No additional regression found in the adjacent sequences inspected.

**Not checked:** No edits, Cargo commands, tests, mutations, or hardware runs. Execution results in the fix report were not independently reproduced.

## 見直し 2

> plan · gpt-6.1-sol
VERDICT: PASS

No findings. The ignored test and documented Docker command close the remaining B0 major.

- The test runs a 31,000-sample repeat through `ProbeDevice` at both 1 and 5 MS/s for approximately five seconds each.
- The default block length is asserted as 2,000. Non-empty send sizes are printed in order, allowing B0’s manual check of ordinary continuation sends and waveform-wrap boundaries.
- Gain is constrained to 0 dB, waveform magnitude is asserted ≤ 0.5, and the procedure explicitly requires owner confirmation of attenuation before transmission.
- The diagnostic remains separate from the uninstrumented 60-second performance run. Changed-file statistics match the previous review except for the new test and expanded B0 instructions.

Not checked: No Cargo commands or hardware runs, as instructed. Actual execution and RF wiring remain unverified; send-size classification is manual, not asserted by the test.
