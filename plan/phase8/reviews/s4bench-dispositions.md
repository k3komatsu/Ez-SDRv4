# Stage 4 bench tooling for 2026-11-15 — review record and dispositions (2026-11-13)

Measurement tooling for the owner's stage-4 values (decided 2026-11-13; several depend on these measurements). Test-side code and `plan/phase8/` text only; no production or profile change.

| Item | What it does |
|---|---|
| **S2** (`hw_s2_throughput_repeats`) | RX and TX one channel at 100 and 200 MS/s, × 10 × 60 s; duplex RX 200 + TX 100 and RX 100 + TX 100, × 5. TX decision: "the device accepts and plays" over the first 10 s, versus how long it holds. An unplaceable underflow leaves the window unknown. |
| **T200** | The record suite's 200 MS/s TX stage alone, × 3. Bare UHD: `plan/phase8/tools/t200-bare-uhd.sh` runs `tx_repro --case t200` (no Ez-SDR code). It reads back 1 GHz and gain 0 before transmitting and aborts outside 999.5–1000.5 MHz or above 20 dB. Each run is bounded; Ctrl-C, TERM, HUP and QUIT kill the container. The verdict is clean / failed / INCONCLUSIVE. Host settings H1 (C-states) and H2 (IRQ pinning) are commands in bench.md that the owner runs with sudo. |
| **S5** (`hw_s5_lead_rungs`) | Command-lead rungs 3.5–5 ms at TX 1, 10 and 100 MS/s; receive start-lead rungs 3–10 ms at 10 and 100 MS/s, one and two channels. |
| **S4** (`hw_s4_overflow_gap`) | Overflow restart gap at 1–200 MS/s × 20, plus a stall-length sweep. |

Each item has a FakeDevice rehearsal in the default suite, and bench.md has the 2026-11-15 stage-4 procedure.

Risk: not risky (measurement tooling), so there is no test plan and no self-review. Branch `work/s4bench` from `79f55e1`: one commit and three fix rounds, squashed and rebased. Implementer: OpenCode Sonnet 5.5 #high for the tooling and fix round 1; Claude Code Sonnet subagents for fix rounds 2 and 3. The fix-round-3 subagent stopped on an API 403 after its edits, so the orchestrator verified that round. Records: [s4bench-briefs.md](s4bench-briefs.md).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, code | 4 | 2 | — (M1: an RF hazard, `benchmark_rate` transmits at an unset and unread frequency and gain; M2–M4: decision lines on incomplete evidence) |
| Opus, re-check of fix round 1 | 1 | 2 | — (N1: Ctrl-C did not stop a bare transmit, introduced by the fix) |
| Opus, narrow re-check of fix round 2 | 0 | 2 + notes | — (N4: a signal before the container exists; N5: SIGHUP not trapped) |

## Dispositions

**Sol, code (FAIL)**: all adopted in fix round 1.
- M1: `benchmark_rate` is replaced by our own `tx_repro --case t200`, with the RF read-back and abort.
- M2: untimed underflow reports are counted, and an unplaceable one makes the 10 s window unknown.
- M3: each bare run has a whole-life bound and a kill by name.
- M4: the bare verdict validates every field and the transmitted volume.
- m5: the S4 summary uses the actual sweep rate.
- m6: S5 keeps no rx blocks.

**Opus, re-check 1**: M1 (frequency, gain, antenna), M2–M4, m5, m6 and the rows are closed. Fix round 2:
- N1: a trap kills the container; `--init`; `tx_repro` handles SIGINT/SIGTERM and ends with an EOB.
- N2: try/join around the loop; line-buffered stdout.
- N3: the series breaks after an expiry, and `docker kill` is bounded.
- bench.md: the attenuator is confirmed before each command that transmits.

**Opus, re-check 2**: N1 is closed for every path, and N2, N3 and the rows are closed. Fix round 3 (minor-only, no re-review):
- N4: the trap kills the client job, waits, then `docker rm -f` by name.
- N5: the trap also covers HUP and QUIT.
- Notes: a try around the async thread; `catch (...)`; the usage text corrected.
- Rows S4B-49…51.
- Verified by the orchestrator: `tx_repro` builds with `-Wall` in the bench container; self-check all pass; fake t200 3/3; rows S4B-38…51, 14/14 killed.

## Decided by the orchestrator

- The bare-UHD comparison uses `tx_repro`, not `benchmark_rate` (RF safety).
- One attenuator confirmation per transmitting command, as `run-record.sh` does.
- A device failure (overflow or underflow) in S2/S4/S5/T200 is an observation, not a failed test; the decision lines carry the verdict.
- S3 (two-channel throughput with the CBX back) is not built; it needs the CBX.

## Not verified on hardware

Everything here runs first on 2026-11-15. `tx_repro --case t200` has been compiled but never run on a device, and its signal handling at 200 MS/s is INFERRED.

## Gate

First gate (on `3ef2764`): FAIL — `s4bench_s2_an_untimestamped_underflow_does_not_make_a_window_held` compared the untimed run's read-time date with the *other* run's device time; two runs' dates differ by host timing, so the window could end after the untimed date (`Some(false)`). Fixed by the orchestrator: the window ends just before the untimed run's own date (5/5 passes); the same test is S4B-37's baseline.

Second gate: PASS ([s4bench-gate.json](s4bench-gate.json)) on sim02:8 and sim03:8: 2796 tests, 686 UHD, `hw-build` (the probes compiled with `--features uhd`), Python 121 × 2, e2e 16/16, mutation 2723/2724 (the known survivor `phase7/G09`), no `ERROR`, no timing re-runs.
