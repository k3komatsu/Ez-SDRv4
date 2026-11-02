> plan · gpt-6.1-sol
# REVISE

The three device-model findings are closed. The round-3 shutdown fix still misses a reachable ordering that can leave the device streaming after its executor exits.

Paths below are relative to `crates/ezsdr-radio-uhd/src/`, except mutation rows.

## Closure checks

| Item | Status | Evidence |
|---|---|---|
| **P1-A: tail promotion timing** | **CLOSED** | `brain/model.rs:44–58,237–241` promotes only at recv entry; neither Tick nor the tail-returning recv promotes. `brain/world.rs:1092` wires that entry. Regression `model.rs:393–420` exercises the delayed next recv and asserts LateCommand, once, without new samples. Matches `device.rs:1627–1646`. |
| **P1-B: direct past starts** | **CLOSED** | `brain/model.rs:186–204` records a past timed start as late without opening a flow; start-now uses current device time. `brain/world.rs:1225–1227` uses start-now for flushing. Assertions at `model.rs:425–443` cover both. Matches `device.rs:1182–1206`. |
| **P2-C: silent boundary** | **CLOSED** | `brain/model.rs:94–95,110–122` excludes `silent_from` from application eligibility, including Stop’s settle. The regression queues a write **exactly at 16**, stops at 16, and verifies that write remains pending (`:447–460`). |
| **Late-command wiring note** | **CLOSED** | `brain/world.rs:2448–2465,2487–2520` now runs recv entry, LateCommand delivery, brain handling and the receive executor’s answer. Unlike the previous test, it does not stop after merely queuing the input. See the assertion-strength note below. |
| **Restore precondition** | **CLOSED** | `brain/core.rs:784–798` requires every current owner to be Off and not busy, while retaining committed-value, debt, leftover and failure checks. This follows production’s stop → flush → Off check → restore order (`provider/rx.rs:302–337,434–458`; `provider/core.rs:263–305`). |
| **Pending settings map** | **CLOSED**, within the specified scope | One map records Restore/Move decisions before their sends (`brain/core.rs:797,805`), gates the named requests (`:773–781`), and clears at Done (`:598–605`). It is retained in the search key (`brain/world.rs:1427–1435`). I found no new key merge caused by this map. |
| **Shutdown after start completion** | **NOT CLOSED** | Finding 1 below. |

Mutation rows E1-167–185 generally target the stated changes and corresponding assertions. Their execution was not verified.

## New findings

### 1. P1 — Shutdown can terminate a stream before sending its Stop

**Evidence:** `brain/core.rs:441–454,509–513,730–742`; `brain/cell.rs:288–298`.

Minimal operation sequence:

```text
An open stream is Off, with a planned cut and no values.
Brain sends StartNext.
Shutdown is handled before the start's result updates the cell.
R completes the start and sends Armed; brain delivery is delayed.
R receives through the cut and publishes:
    cell becomes Stopping;
    FirstSample and CutReached are queued behind Armed.
Brain handles Armed.
R receives and executes Terminate before brain handles CutReached.
```

`Armed` clears the outstanding Start. However, `stop_if_shutting_down()` accepts only Armed/Streaming, so it skips the already-**Stopping** cell. `terminates_ready()` checks neither that phase nor whether its device Stop was issued, and therefore sends Terminate.

**Observable wrong outcome:** R exits while the device stream remains running. A later CutReached can queue Stop behind the exited executor, where it cannot run. This can also escape the terminal checks: Terminate clears the FIFO (`brain/world.rs:1058–1062`), and `incomplete()` ignores an exited stream’s running device state and outstanding command (`:1546–1568`).

**Fix:** cover the cut-reached-before-Armed-delivery ordering in the shutdown helper and regression. Assert an actual device Stop before executor termination, not merely successful actor exit.

### 2. P2 — The second Restore indirectly narrows existing shutdown schedules

**Evidence:** `brain/cases.rs:68–69,108–110,252–264`; `brain/world.rs:437–442`.

The original input windows remain unchanged, but Shutdown must follow **every** input:

- Micro: Shutdown previously could arrive at **3**; the new Restore cannot arrive before **4**.
- Reduced(8): the new Restore starts at **16**, although the original Restore starts at **6**.
- Reduced(16): the same new lower bound removes earlier shutdown schedules.

Thus the extended scripts are not additive under projection onto the original inputs: some original early-shutdown sequences disappear.

**Fix:** retain the original script alongside the extended one, or adjudicate an ordering/window arrangement that preserves those schedules while still allowing a post-failure Restore.

### 3. P2 — Swarm reports do not retain witness traces

**Evidence:** `brain/cases.rs:581–608`.

The sweep retains witness counts and first seeds, and prints their weights. It does **not** retain or print the corresponding witness traces; each walk’s trace is discarded after counting. Seeds permit reconstruction against unchanged code, but that is not the requested trace retention.

**Fix:** retain or emit the first trace reaching each declared witness, together with its seed and weights.

### Note — Late-command assertions observe the queued answer, not its effect

`restarted()` and `ended()` inspect a pending `RStep::Call` (`brain/world.rs:2479–2485`). The settling loop subsequently executes it, so the previous wiring gap is closed. However, a no-op device implementation of that call could still satisfy these assertions. A post-call streamer-state assertion would make the test’s claim stronger.

## Case-script judgments

- **Case 8 second stream: CLOSED.** It adds an independently receiving stream without changing the existing windows, fault branches or witnesses (`brain/cases.rs:295–317`). Its loss publication can legitimately hold G while R0 waits or rechecks. The addition preserves the original lifecycle test.
- **Case 7b second Restore: NOT CLOSED as a non-narrowing change.** It strengthens post-failure request coverage and retains the original Restore, but removes early-shutdown schedules in the variants identified above.

## Swarm scheduler

**Selection semantics: CLOSED.** Positive, reproducible per-seed weights are generated at `brain/dst.rs:296–299`; selection uses only enabled events (`:315–338`). World classification is Tick / Env+Timer / other actors (`brain/world.rs:1488–1494`). Mandatory-input blocking remains intact (`:537`). Missing witnesses, incomplete runs and per-walk event-budget exhaustion remain failures (`brain/cases.rs:554–568,607–608`), and the sweep explicitly disclaims completeness.

**Trace-retention condition: NOT CLOSED**, as finding 3 describes.

**Not checked:** No edits, formatting, builds, tests, mutation execution or state-changing git commands. Runtime sweep results, E2 conformance and hardware behavior were not independently verified. The holdout directory was not accessed.
rc=0
