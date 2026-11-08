# Bench additions for Monday after note 47 stage 5: dispositions (2026-11-08)

The owner adopted two additions to the stage 5 bench, both bench-only (production code unchanged):
- the transmit-rate acceptance probe;
- the ssh drop, with #2 as its independent receiver.

This is not a risky step. The implementer was Haiku 5.5 and the self-review ran on Go DeepSeek.

**Records:**
- `bench5b-brief.md`: the brief, the monitor brief and the two fix briefs.
- `bench5b-gate.json`.

## What changed

1. **The transmit-rate probe.** On 2026-11-06, #2's three INCONCLUSIVE results were a test defect.
   - `tone_start` reads onsets on a fixed 25-sample window grid: 5 000 ticks, coarser than the 1 000-tick tolerance. The control and B fell at unrelated phases of that grid.
   - B is now placed on the control's phase (`n47.rs` `same_window_phase`). PASS now means "B's onset is in the predicted 25-sample window". That is enough to catch a rate or tick-conversion error, which would be off by milliseconds.
   - The owner accepted this resolution. A per-sample onset would be a new mechanism.
2. **The ssh drop's independent receiver.** The owner's setup: #1 transmits through the ssh path. Its output passes ≥ 30 dB of attenuation and then a splitter to #1's receive and #2's receive.
   - #2 runs `hw_rx_tone_monitor` directly, not over ssh. It writes `tone`/`quiet`/`unknown` every 200 ms.
   - `rx-monitor-state.sh` gives C2 the latest fresh state.
   - `rx-monitor.sh` starts the monitor and stops it cooperatively.
3. **C3** (`hw_noise_floor_receive`) counts a window as tone only when it has the drop tone's shape and is above a 4 LSB amplitude floor.
4. **The ssh-drop script still matches the code** after stages 4 and 5 (profile, Python API, `client_disconnect`). This PC's sshd listens on port 10022; with the owner's approval the orchestrator added `Host localhost / Port 10022` and the owner's key to `~/.ssh`.
5. **The rule "the two boards are never connected at once" is removed** (owner, 2026-11-08) from `bench.md` and `bench-results.md`. Both boards have been connected at once, each on its own NIC subnet, since 2026-10-29.

## Sol code review (FAIL, 5 × P2; all adopted)

1. Quiet noise could refuse the monitor's start. The start is now refused only when the drop's shape fills half of the first second.
2. C3 could fail a transmitter that had stopped. It now uses the monitor's shape decision.
3. A fresh line could describe old samples. A line is now `unknown` when its last sample lags the device clock by more than 300 ms.
4. The stop bypassed the clean receive shutdown. A stop file is now checked, then `rx_stop`/`rx_close` run before a bounded wait.
5. The splitter drawing had no attenuation on #1's branch. One ≥ 30 dB attenuator now sits before the splitter.

## Opus re-checks

**Round 1 (FAIL, 1 major, 4 minor).**
- C3 still tripped on 0.3–0.4 LSB quantized noise. With a 45° lag product, coherence reads 1 at exactly 125 kHz, which was inside ±25 kHz. The fix:
  - the frequency bound becomes ±20 kHz;
  - C3 gets an amplitude floor of 4 LSB, about 30 dB below the loopback tone (INFERRED);
  - tests at σ 0.3 and 0.4 that failed before the fix.
- The minors: create the state file before receive, longer monitor seconds, and the guard's message.

**Round 2 (PASS WITH MINORS).** Two runbook items were fixed by the orchestrator:
- the guard is now an `if`, so `pipefail` cannot print a false message;
- the stale ±25 kHz note was updated.

## Mutation

The full lists ran before Sol's review: 1963/1964 passed, with G09 as the known survivor. New rows N47B-43…57 were probed in throwaway copies.

## Counts and mechanisms

- Sol: 5 P2.
- Opus: 1 major, then 6 minor.
- No production mechanism is involved. The pattern is "a quiet-or-tone judgement on noise near the quantization floor", which recurs in the bench probes (P5–P7 were INCONCLUSIVE on 2026-10-26 for the same reason). For C3 the quantization lattice was found and pinned by tests.
