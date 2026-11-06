# Note 47 stage 4 — Sol's code review (2026-11-07, `openai/gpt-6.1-sol`, read-only)

## Scenarios considered
- Unchanged and changed generations; partial configure failure, retry, and corrected resend.
- Stop, Provider stop, and loss during configure; loss during reopen.
- Configure after padded sends, held-tail closes, dispatch-time moves, and recovery.
- Different-generation bursts meeting an open device burst.
- Identical accepted bookings with reordered updates, booking-time move/drop, and repeat Stop.
- Clock end/abort before origin; report identity after reopen.
- Python validation and first rejection preventing subsequent requests.
- Fractional sample instants and delayed SimulationChannel evaluation.

## Findings

### 1. P2 — A fractional-start burst radiates its first sample with the old settings
`crates/ezsdr-mock-radio/src/lib.rs:970–977`  
`crates/ezsdr-mock-radio/src/time.rs:14–18`  
`crates/ezsdr-mock-radio/src/channel.rs:99–105`

`open_held` rounds the burst’s start **up to an integer root tick** before recording its frequency and gain. The transmitter subsequently reads those timelines at the sample’s **exact rational instant**, which precedes that rounded tick.

Minimal sequence:
1. Connect an ideal Mock transmitter at 3 MS/s on the 1 GHz root, initially at f1.
2. While empty, stage f2 and book a burst starting at sample 1999.
3. Its first sample is at **666333⅓ root ticks**, but f2 is recorded at **666334**.
4. A receiver tuned to f2 misses that first sample; a receiver at f1 can hear it. Later samples use f2.

Gain changes have the same defect. This remains wrong even after the transmitter has processed the start.

**Fix:** make the burst’s values effective at its exact sample instant, preserving history and stepping-order independence. Add a fractional-start radiated-sample test. The new S1 test uses an integral sample grid (`tests/mock_channel.rs:762–799`), so it misses this boundary.

### 2. P3 — The required fail→fix→resend admission test is still absent
`crates/ezsdr-radio-uhd/src/provider/tx.rs:5748–5775`  
`crates/ezsdr-radio-uhd/src/provider/tx.rs:5802–5831`

S7 inserts constructed `Held` bursts directly into the owner. Its corrected burst supplies generation 2 directly, rather than submitting a corrected value and burst through Kernel admission. F9 exercises Control, but also bypasses admission.

Consequently these tests cannot detect the specific failure the brief requires them to exclude: configure refusal → clock becomes inadmissible → corrected burst rejected before reaching the Provider. Also, S7’s final assertions check configuration and mark, **not that the corrected burst was sent**.

**Fix:** complete the Kernel-path sequence and assert the refusal’s ActionId, busy boundary, corrected admission, unchanged clock identity, and successful payload send.

### 3. P3 — The accepted UHD contract still contradicts the new configure path
`design/18-uhd-radio.md:210`  
`design/18-uhd-radio.md:294`

UR-12 still says configuration occurs at prepare, receive-segment start, or after failure, **“and nowhere else”**, and applies the full configuration including rate. Normal between-burst TX configuration now occurs elsewhere and deliberately passes `rate: None`.

UR-26 mentions no open device burst, but still omits the previous-device-end guard, recovery exclusion, separate configure turn, and post-configure late-policy decision. Its reference to “UR-25’s `apply_due`” does not supply those rules.

**Fix:** reconcile UR-12 with TX configuration and state the safety conditions and refused-TX-configure behavior in UR-26.

## Verdict: FAIL

The fractional-start settings defect must be fixed. No separate UHD lifecycle blocker or stage-5 scope expansion was found.

Read-only review; no edits, formatting, builds, tests, or mutation runs. Kill claims and hardware behavior were not independently rerun.
