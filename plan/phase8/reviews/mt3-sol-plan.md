# Maintenance table 3 — Sol 6.1 test-plan reviews

## §1–§8 (v2)
Before reading the test plan, my failure scenarios are:

- A late first chunk is dropped, but a later timed chunk plays. The regression test must fail on today’s fake.
- A drain advances too little, produces a false Underflow, leaks into the next burst, or survives a reopen incorrectly.
- TimeError retires only one eligible device burst, misses a joined Kernel burst, or computes lateness from the joiner’s start.
- A host-side abandon precedes report delivery and loses the eventual event.
- Underflow followed by TimeError labels a partly played burst “nothing transmitted,” including when the host has already closed it.
- One channel’s report retires another channel’s state, or reports from both channels produce duplicate events.
- A delayed report for an older burst abandons a newer open burst.
- A hardware rehearsal follows changed production constants and silently changes its expected result.

I’ll compare the plan against these, excluding repeat resumption.
## Verdict: FAIL

The plan covers the main correlation failures well: U6 preserves an event after an earlier abandon; U7 covers Underflow followed by TimeError on a host-closed burst; U9 checks cross-channel deduplication; U10 protects a newer burst from an older report. F0’s explicit pass-then-flip procedure also addresses the original fake looseness.

References below use **P** = `/home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/test-plan-v2.md`.

### 1. Major — the hardware verdict can mistake missing reception for a whole-burst drop

**Evidence:** P:100–102. **VERIFIED** omission in the planned verdict; the resulting false PASS is **INFERRED**.

The verdict requires no heard interval, no ACK and at least one TimeError. It does not require successful receive coverage or a positive loopback control. The fake’s `transmitted` observation cannot establish that the hardware receiver was working.

**Minimal sequence:**
1. A late chunk produces a TimeError.
2. A defective implementation transmits a later chunk.
3. RX fails to start, misses the relevant interval, or the loopback is disconnected.
4. No ACK is delivered.

**Wrong observable outcome:** C1 reports PASS despite partial transmission—the precise behaviour it must detect.

**Required amendment:** Make adequate RX coverage and a successfully heard control burst prerequisites. Missing or invalid observation must not PASS. Add synthetic verdict cases for absent/incomplete RX evidence and observed partial transmission.

### 2. Minor — T2 does not establish that its two bursts actually joined

**Evidence:** P:93–94; `crates/ezsdr-radio-uhd/src/provider/tx.rs:205–221`; `crates/ezsdr-radio-uhd/src/device.rs:1823–1826`. **VERIFIED** ordering; the resulting test execution is **INFERRED**.

`SlowSend(20 ms)` delays the first send. Its TimeError is read before the next `step`, which will abandon the one-shot device burst before the second booking can join it. A later second booking could independently become late and produce the second event, satisfying T2 without exercising joined-target registration.

**Fix:** Delay report delivery until both Kernel bursts have been handed into one device burst. Assert one payload SOB and that B was handed before the first report was consumed. U3 already provides useful unit-level coverage; T2 currently does not add the claimed Session-level witness.

### 3. Minor — F3c does not kill the proposed pad-floor mutation

**Evidence:** P:25, P:59, P:169. **VERIFIED** arithmetic.

For `S = 1,000`, removing the floor gives:

```text
pad = ceil(1000 / 100) = 10
limit = D + (10 + 15) = D + 25 samples
```

F3c probes at `D + 22`, so B remains refused. The plan’s stated mutated limit, `D + 20`, is incorrect. MT3-22 therefore lacks its claimed distinguishing assertion.

**Fix:** Separate the floor witness from the guard witness. For example, probe at `D + 27` to distinguish limits 25 and 30; keep an independently suitable guard-removal probe.

### 4. Minor — the planned UR-33 amendment still describes the unpadded v1 model

**Evidence:** P:195 versus P:25–29. **VERIFIED** contradiction.

The amendment says the fake’s drain ends “exactly at the samples handed” and describes a looser cascade boundary. V2 instead adds a pad and guard, with hardware coverage explicitly INFERRED pending C4.

**Fix:** Update the amendment checklist to describe the padded model and its unverified hardware boundary. Do not let sub-step A land the obsolete claim.

**Not checked:** No files changed, builds/tests or mutation tools run. Repeat resumption was excluded. FPGA radio-state behaviour was checked against UHD v4.10 source; H1 raw records were not independently re-read, so bench claims remain attributed to the supplied reviews.

## §9 (v4)
Before reading §9, my failure scenarios are:

- `busy` omits a send-return time, close/T3 samples, rate conversion, or owner/reopen carry; R then falls inside the drain.
- The resumed SOB or report correlation retains the old start; a delayed old report kills the healthy resumed part, or channel reports cause duplicate resumes.
- b′ compares against the aligned R rather than `busy + m`, uses the wrong drop cursor, or ends at equality.
- A missed R moves off-lattice, emits repeatedly, or leaves a switch behind the new cursor; a moved switch resumes the old waveform.
- Split records omit discontinuity, retain the old target, or report a planned R as the actual start.
- Stop, shutdown, loss, or a delayed terminal report during the gap uses R as the handed cursor, sends a stray close, or leaves bookings alive.
- Recovery failures resume when they should end; existing abandon tests and mutation killers become vacuous after Underflow changes meaning.

I’ll now compare those against §9 and Q-D1–Q-D8.
## Verdict: FAIL

§9 covers most lifecycle risks well, but its delayed-report stutter oracle needs correction before implementation.

**P** = `/home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/test-plan-v4.md`. Code paths below are relative to `/home/komatsu/local_work/Ez-SDRv4-e3/`.

### Major — delayed Underflow can bypass b′ in the planned oracle

**P:257, 289**, versus **P:265, 297**.

V3b says the newest entry’s Underflow “resumes again,” without applying b′. RT3 repeats that unconditional expectation.

Minimal sequence:
1. The original repeat drops; its report remains unread.
2. The resumed part starts at R, with preceding stall threshold P.
3. It underflows at `R + P − 1`.
4. Before reports are read, the expired path schedules another resume.
5. The Underflow retires both original and resumed entries.

**Wrong observable outcome:** RT3 permits another resume, although b′ requires termination because the resumed part played less than its preceding stall threshold.

**Fix:** Cross the delayed/two-entry case with `P−1`, `P`, and `P+1`. Also make the newly calculated stall threshold differ from the preceding one, so comparing against the wrong gap cannot pass.

**VERIFIED:** contradictory planned expectations. **INFERRED:** implementation failure they could permit.

### Minor — stale Underflow must not disappear

**P:266, 298** require “no event” for a stale Underflow. Existing handling emits `TX_UNDERFLOW` before checking whether it reaches the current device burst: `crates/ezsdr-radio-uhd/src/provider/tx.rs:1151–1164`.

Assert **no new resume, timing row, or effect on the healthy part**, while retaining the report’s Underflow event without a new resume promise.

**VERIFIED.**

### Minor — RT4’s claimed playback discriminator is unsupported

**P:258, 290** claim removing `r` places R inside the drain. The stated second stall is 6 ms, while the unchanged margin is 10 ms; those numbers alone do not establish that result. This same numerical weakness was identified in `fable-addendum-review.md:54`.

Require the **exact hand-calculated R**, which can distinguish missing `r` even when both candidates play; alternatively provide geometry that actually distinguishes playback.

**VERIFIED:** insufficient specified geometry; the claimed drop is **INFERRED**, not established.

### Minor — RT20 does not establish its distinguishing state

**P:244, 275, 306, 338** propose killing `unacked.back_mut()` through RT20, but describe only an ordinary join. If the correct entry is also the last entry, that mutation passes.

Specify a reachable setup where they differ—or show that D makes this state unreachable and remove the unnecessary replacement/mutation. Explicitly distinguish an absent, already-retired correlation from a merely non-last correlation.

**VERIFIED:** missing distinguishing precondition; reachability remains **INFERRED**.

### Minor — Q-D7’s unconditional bound is false

**P:380** proposes `R − busy < m + N`, but R also depends on `now + lead` and `front`. Either can exceed `busy + m` arbitrarily.

Use `B = max(busy+m, now+lead, front)` and assert `0 ≤ R−B < N`; state the busy-relative bound only when its term dominates.

**VERIFIED.**

**Q-D1–Q-D8:** Otherwise reasonable. Q-D3’s equality-resumes rule is already owner-decided, not an internal choice to change to `<=`. Q-D6 appropriately preserves abandon-footprint coverage.

No files changed; no builds, tests, formatting, or mutation tools run. Hardware timing remains attributed to the supplied evidence, not independently measured here.
