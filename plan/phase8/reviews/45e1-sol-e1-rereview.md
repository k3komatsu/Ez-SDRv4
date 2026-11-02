> plan · gpt-6.1-sol
# REVISE

Findings **1–3 and 5–8 are CLOSED** at the code level. **Finding 4 is NOT CLOSED**: promotion now works, but its timing differs from FakeDevice, and I reject the direct-past-start deviation.

Paths below are relative to `crates/ezsdr-radio-uhd/src/`, unless stated otherwise.

## Findings 1–8

| # | Status | Evidence |
|---|---|---|
| **1** | **CLOSED** | `brain/cell.rs:365–368,419–424,466–481` records the flush attempt separately and retains unread left values. `brain/core.rs:765–770` drops Restore after an attempted flush leaves a value unread. `brain/world.rs:2154–2183` checks retention, no repeat flush, and no Restore. The original deletion-based counterexample is blocked. |
| **2** | **CLOSED** | `brain/core.rs:782–792` restricts Close/Open to Off or Uncertain, with no committed value, retirement or busy lifecycle. Reservations require Armed/Streaming (`brain/cell.rs:171–175`). I3 now checks both routing and execution (`brain/oracle.rs:203–225`). Regressions cover the original reservation-between-request-and-close sequence (`brain/core.rs:1184–1199`). |
| **3** | **CLOSED** | MarkLost sets `Exit::Dead` (`brain/core.rs:350–354`); `BStep` is disabled afterward (`brain/world.rs:540`). The guard exhausts its operations, so no `BOp` remains. Death clears timers and terminates S/every R independently of servicing the brain inbox (`brain/world.rs:801–808`). The regression checks unserved Shutdown/completions and executor exit (`brain/world.rs:2367–2390`). P1’s dead-brain exception is appropriate (`brain/oracle.rs:416–419`). |
| **4** | **NOT CLOSED** | `brain/model.rs:45–59,223–234` implements promotion; `:328–385` tests subsequent samples and late promotion. However, promotion occurs on Tick and at the **end of the tail-returning recv**, rather than at FakeDevice’s **next recv entry**. See P1-A below. Direct past starts also remain incorrectly successful; see P1-B. |
| **5** | **CLOSED** | Application channel comes from the actual DSP-map key (`brain/world.rs:1353–1357`). Consumption is restricted to actual streamer channels (`:1135,1343–1345`), and I4 compares those facts with reservation intent (`brain/oracle.rs:48–52`). Although the evidence channel is recovered from the consumed entry (`brain/world.rs:1167`), that entry can be consumed only on a carried channel. Move affects subsequent Open’s bindings (`:916–920,1233–1235`); regressions cover both bindings and consumption (`:2396–2431`). |
| **6** | **CLOSED** | Every case-7b variant enables faults (`brain/cases.rs:264–266`). Sweeps accumulate declared/reached witnesses and fail on missing coverage (`:570–594`); regressions check both requirements (`:929–948`). **The reported missing witnesses remain unmet coverage**, but no longer silently pass. |
| **7** | **CLOSED** | The key retains `shortened`, all four witness markers read later, `flushed`, and `silent_from` (`brain/world.rs:1396–1397,1411–1423,1437–1462`). The later reads are at `:665,884,1162`; `READ_LATER` covers them (`:22`). The old history-equality test no longer discards those fields (`:1819–1855`). |
| **8** | **CLOSED** | Timers require their actual instant (`brain/world.rs:539`), recv timeouts require their deadline (`:477–478`), and silent Tick remains enabled while either lies ahead (`:488–490,537`). Regressions check actual delivery times (`:1979–1993,2018–2031`). The original early-deadline counterexample is blocked. There is a separate cutoff-boundary defect below. |

## P1 — Major

### A. Tail promotion happens too early and suppresses FakeDevice’s late result

**Evidence:** `brain/model.rs:138,233`; compare `device.rs:1627–1640`.

Minimal sequence:

```text
Start(0)
produce samples 0..3
Stop at T=4
Start(8), queued behind the tail
at T=6, recv the whole tail
delay the next recv until T=10
```

The model promotes during the recv at T=6. Ticks 8 and 9 then buffer the next segment’s samples.

FakeDevice’s tail-returning recv only advances its tail cursor. Promotion happens on the **following recv**, at T=10; because `8 < 10`, that recv returns `LateCommand`.

**Observable wrong outcome:** the model delivers new-segment samples where FakeDevice reports a missed start. This can hide recovery-path defects.

**Fix:** preserve the next-recv promotion boundary. Add this delayed-next-recv regression; the existing test receives every tick and misses it.

### B. Direct past starts remove a required failure branch

**Evidence:** `brain/model.rs:181–190`; compare `device.rs:1195`. The deviation is explicitly documented at `brain/model.rs:4–6`.

Minimal sequence:

```text
Open stream
advance to T=10
Start(at=8), without a stopped tail
Tick
recv
```

The model produces sample 10. FakeDevice records a missed start and returns `LateCommand`.

**Observable wrong outcome:** delayed timed starts succeed in the experiment, bypassing the production late-start handling (`provider/rx.rs:629–673`).

**Judgment:** **reject the deviation**. Unbounded actor lag is exactly why this branch must exist. Making scripted runs easier to complete is not evidence that the device semantics can change. Keep timed-start and stream-now semantics distinct; handle genuinely late timed starts through the modeled recovery path.

## P2 — Minor

### C. Silent-mode settling still permits an application at the first forbidden sample

**Evidence:** `brain/model.rs:94,104,115,133–135,206–209`.

Minimal sequence:

```text
silent_from = 16
Start(0)
queue a timed write for sample 16
Tick sixteen times: samples 0..15 are produced
Stop at T=16
```

Stop calls `settle()` before closing the flow. `drain()` clamps time to **16 inclusively**, so it logs the write as applied at sample 16, although that sample is never produced.

The new regression uses a write at **17**, avoiding this boundary (`brain/world.rs:2002–2013`).

**Fix:** make application eligibility respect the last produced sample, including the cutoff boundary, and test a write exactly at `silent_from`.

## Notes

- **Late-command wiring test overstates its reach.** `brain/world.rs:2438–2457` manually injects `late`, then checks only that `REnd` queues a send. It does not execute that send, brain handling, or restart despite the test’s description. Retain the focused check, but add the actual end-to-end sequence when fixing A/B.
- The handwritten DFS replacement is present: `brain/world.rs:1661–1678,2465–2488` uses `dst::explore` through `Probe`, whose checks preserve the world’s own oracles (`brain/cases.rs:827–849`).

## Walk-coverage gap and swarm scheduling

**A seeded swarm scheduler is sound as a sampling improvement**, provided it changes only selection among currently enabled events—not windows, transitions, or oracles.

Keep these constraints:

- Give enabled choices positive weights; preserve mandatory-input blocking.
- Record reproducible seeds/weights and retain witness traces.
- Keep missing witnesses, incomplete runs, and exhausted walk budgets as failures.
- Do not describe successful weighted walks as exhaustive coverage.

Tick-versus-actor weights can help, but may delay useful actors along with environment inputs. Separately weighting **Tick, environment delivery, and actor work** can target that imbalance without narrowing any legal window.

The currently unreached witnesses remain an acceptance gap. Changing the scheduler does not close it until those witnesses are actually observed.

**Not checked:** No edits, formatting, builds, tests, mutation execution, or state-changing git commands. Runtime results, E2 conformance, performance, and holdout contents were not checked; the holdout directory was not accessed.
rc=0
