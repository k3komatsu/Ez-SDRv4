## What this change does

S1 pads short first sends to four samples, attributes flush reports separately, and corrects ACK timing/correlation. Every payload send remains timed.

## Independent failure scenarios

- Window space becomes available only after the first send’s lead expires.
- Stop removes a short burst but loses its padded safe-start frontier.
- Adjacent bookings arrive before versus after dispatch.
- Flush reports cross channel, deadline, stream or generation boundaries.
- An ACK shares the next burst’s start tick.
- Bench tests pass despite reproducing the flush fault.

## Findings

### 1. P2 — Mock loses the padded frontier on Stop

**Locations:** `crates/ezsdr-mock-radio/src/lib.rs:1122–1133`, `:1359–1360`, `:1755–1759`.

Minimal sequence, at 1 MS/s:
1. At 91 ms, book a one-sample A at `s = 100 ms`; ensure UHD has sent its padded packet.
2. Stop the stream before playback.
3. Book B at `s + 3 samples`.

UHD retains A’s front, `s + 5`, and drops B. Mock discards the frontier returned by `tx_handed_front`, emits/removes A, and retains only its payload end plus two: `s + 3`. It therefore admits B.

**Fix:** preserve the computed padded frontier across Stop, separately from the payload cut.

The new test does not catch this: `crates/ezsdr-acceptance/tests/parity.rs:1467–1479` queues A and Stop together without observing A’s dispatch, then expects **`s + 5` to drop**. That is the opposite of its comment and the planned regression. Require A’s dispatch before Stop; assert `s + 3` drops and `s + 5` plays.

### 2. P2 — Waiting first sends are rechecked after their start, not after their lead expires

**Location:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:426–449`.

Minimal deterministic sequence using the existing controlled rig:
1. Set the window to 2.004 ms; book a long burst at 3 ms.
2. Dispatch at 0.999 ms: only three window samples fit, so its first send waits.
3. Advance to 1.005 ms and step again.

Now nine samples fit. The code sends them without reapplying the late policy, although only 1.995 ms remains against the required 2 ms lead. `DropAndFlag` produces no required late/drop verdict.

The new tests at `tx.rs:4947–4987` advance past the **start**, so they miss this interval.

**Fix:** recheck an unsent device-burst start when its lead expires, before sending. Pin both policies immediately across the lead boundary.

### 3. P2 — The revised hardware assertion accepts the flush failure it should detect

**Location:** `crates/ezsdr-radio-uhd/tests/hardware.rs:1598–1600`.

Counterexample: the four-zero flush still generates Underflows; attribution correctly marks them `tx_flush`; both later bursts behave correctly. The revised assertion passes because it explicitly permits flush Underflows.

R9 requires the corrected four-zero flush to draw **no Underflow**, including attributed ones.

**Fix:** reject all raw Underflows in this clean-flush bench test. Keep the separate attribution tests for defensive handling of injected flush reports.

### 4. P3 — UR-23’s new window sentence contradicts the implementation

**Location:** `design/18-uhd-radio.md:257`.

“A later buffer takes what fits” is false: `tx.rs:546–547` still waits for the continuation’s entire `wanted` count. Existing continuation-wait tests require that behavior.

The Changes row at `design/18-uhd-radio.md:721` also says first sends of “four or more” are not held; the exemption is **at most four**, and larger final buffers can retain a tail.

**Fix:** correct both descriptions; do not change the working continuation policy.

## The 97–98 ms observation

It does **not establish a production Module/Mock divergence**. The new `submit` helper leaves Mock at the requested instant, but UHD’s barrier polling advances its device-host clock in 1 ms waits (`parity.rs:1041–1049`, `:433–443`). Repeated submissions therefore need not have matching receipt times. That explains the observed millisecond staircase.

Moving to 91 ms avoids that boundary; it does not validate equal-time parity. Observe actual receipt/dispatch times or hold the clock during booking before drawing a production conclusion.

## Verdict: CODE NEEDS CHANGES

No edits, formatting, builds, tests or mutations were run; `git diff HEAD --check` passed. Hardware behavior remains unverified through the Module, so #78 must remain open.
