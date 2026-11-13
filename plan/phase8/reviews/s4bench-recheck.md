# Stage 4 bench tooling — Opus re-checks (2026-11-13)

Two read-only re-checks (Claude Opus subagents, effort high) after Sol's code review ([s4bench-sol-code.md](s4bench-sol-code.md)).

## Re-check of fix round 1 (`1608751`)

- **M1 (RF): closed for frequency, gain, slot and antenna.**
  - `tx_repro`'s `main` sets 1 GHz, gain 0, `A:0` and `TX/RX`, and aborts unless the frequency is within ±500 kHz of 1 GHz, the gain is exactly 0, and antenna, rate and master clock read back as set.
  - The `t200` case reads back again right before transmitting and aborts (exit 3, nothing sent) outside 999.5–1000.5 MHz or above 20 dB.
  - The only earlier transmit is the two-zero-sample initial flush, sent after the first check.
  - Not read back: the subdev spec (UHD throws on an invalid one, INFERRED).
  - The attenuator confirmation is once per invocation (three 300 s runs), as `run-record.sh` asks once per suite.
  - If the process dies mid-burst, the device runs dry and reports an underflow (INFERRED).
- **M2: closed.** The FFI maps a missing device time to an empty tick, so the count sees a real untimed report. The Provider dates such a report at its read time, never earlier, so "dated inside the window counts as inside it" is sound. A window is held only with 0 untimed reports and no underflow missing its time. Bench logs so far: every underflow carries a device time (2 656 from `tx_repro`, 52 Provider rows).
- **M3, M4, m5, m6, rows: closed.** Each row dies on a deterministic assertion.
- **New N1 (major, introduced by the fix): Ctrl-C did not stop a bare-UHD transmit.**
  - `timeout` runs in its own process group, so SIGINT reached only the script, which went on to runs 2 and 3.
  - `tx_repro`, PID 1 in the container without a handler, ignored proxied signals.
  - The tone could go on to the bound (about 24 min in all). VERIFIED by simulation.
- **New N2 (minor):** a throwing `send` with the async thread joinable → `std::terminate`; stdout fully buffered, so a kill lost the RF read-back line.
- **New N3 (minor):** no stop after an expired run, contrary to bench.md's stop rule; `docker kill` unbounded.

## Narrow re-check of fix round 2 (`18c3dca`)

- **N1 closed for every path checked**, each VERIFIED with a stub docker:
  - Ctrl-C through a real pty: exit 130, container killed by name, no run 2.
  - SIGTERM to the script.
  - A second Ctrl-C during the trap.
  - Ctrl-C during the expiry's kill.
  - The in-container build.
  - The trap is installed before the first run.
  - `tx_repro`'s own handler ends with an EOB on TERM (read only, not executed).
- **N2, N3, rows (S4B-38…48), bench.md: closed.**
- **New N4 (minor):** a signal between `docker run` creating and starting the container (or before `$!`) could leave a container running after the script said it stopped it. Docker starting it after a forwarded TERM is INFERRED; the window is milliseconds.
- **New N5 (minor):** SIGHUP (an ssh drop) was not trapped; the container kept transmitting to the bound. VERIFIED with the stub.
- **Notes:**
  - The async thread's lambda had no try.
  - The catches took only `std::exception`.
  - The usage text implied Ctrl-C ends the burst cleanly; it sends SIGKILL, so there is no EOB.

Dispositions in [s4bench-dispositions.md](s4bench-dispositions.md).
