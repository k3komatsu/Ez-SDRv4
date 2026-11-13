# #67 — Sol's test-plan review (2026-11-13)

Before reading the plan, my failure scenarios are:
- Cancel only a zero-length/dropped segment; preserve a kept segment’s start and pre-cut samples.
- Cancel once, then restart and Stop again; no stale samples, duplicate stop, or calls after observed loss.
- Never close a healthy queued start without cancellation—including cleanup, spawn failure and failed-stop paths.
- Keep fake and scripted-device behavior aligned: cancellation versus close/reopen, channel identity, clock reset and rate changes.
- Exercise abort and an in-flight receive; prove cancellation precedes the distant origin, not merely that the 1 s join eventually returns.
## Verdict: FAIL

The one-conjunct fix is supported by the bench. The failed-stop cleanup policy needs correction before implementation.

References below use `test-plan.md` and `plan-decisions.md` under `/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n67/`.

### Major — Failed cancellation still permits closing a queued start
**VERIFIED plan contradiction; INFERRED device outcome after failure.**

`test-plan.md:29,77–78` and `plan-decisions.md:4` require a best-effort stop followed by `close_streams` even when cancellation fails. That does not establish “never close a queued start.”

Minimal sequence:
```text
rx_start(B, now + 5 s)
owner spawn fails → Uncertain
cleanup rx_stop fails without cancelling B
close_streams
reopen before B → B's old samples arrive
```

C2 explicitly accepts the unsafe close. Also cover a cleanup stop returning `lost: true`: it must publish loss and prevent subsequent stops/close, not merely append `rejected` (UR-29, `design/18-uhd-radio.md:339`).

**Required:** settle the unsuccessful-cancellation policy; test successful, non-loss-failed and loss-returning cancellation separately. Observe the reopened device through the old origin. Do not assert successful cleanup from an attempted stop alone.

### Minor — Spawn-failure test does not actually queue a future start
**VERIFIED.**

C1 retains `rx_start 0 0` (`test-plan.md:77`; existing fixture `src/provider/mod.rs:1066–1072`). The proposed fake preserves only future starts, so reopening after this fixture cannot establish cancellation of a queued start.

**Required:** use a frozen clock and an origin several seconds ahead; verify stop-before-close and silence through that origin.

### Minor — Whole-orphan adoption loses outstanding channels
**VERIFIED proposed algorithm; INFERRED hardware generalization.**

`test-plan.md:86,92` adopts and removes an entire orphan on *any* channel overlap. Sequence:
```text
queue start on [0,1]; close
open [0] → whole orphan removed
open [1] → its queued start has disappeared
```

The disjoint-channel D4 test misses this. Stream-number changes and reordered channel lists also need an explicit decision.

**Required:** preserve remaining per-channel pending state, or explicitly constrain the model to the measured one-channel case. Do not silently generalize overlap to complete adoption.

### Minor — Rehearsal timing premise is false
**VERIFIED.**

`test-plan.md:105` says the manual clock makes 2.5 s “free.” M5 advances **no faster than real time** (`design/18-uhd-radio.md:378`), and `tests/common/raw.rs:71–73` uses real-time observation limits.

**Required:** budget the real duration and establish clock participation explicitly. Avoid the proposed real-clock fallback becoming a new scheduling-sensitive verdict.

### Audit correction
**VERIFIED:** `test-plan.md:145` confuses scheduled origin with host-call order. An early stop can precede e₂ while still following the `rx_start` call. The hardware label compares host call timestamps (`tests/hardware.rs:2138–2148`); its stated new interpretation is incorrect.

I checked both raw bench logs in memory and matched the archive SHA-256: cancellation evidence supports removing the wait. No worktree edits, builds, mutations or probes were performed; implementation correctness remains untested.
