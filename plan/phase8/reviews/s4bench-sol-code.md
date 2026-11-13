# Stage 4 bench tooling — Sol's code review (2026-11-13)

My independent failure scenarios, before reading the brief/report:

- TX accepts enough samples but has no playback acknowledgement—or reports a time/sequence error—yet the 10-second window is called clean.
- Bare UHD reports zero underruns but missing samples or TX timeouts; its summary looks usable for comparison.
- Bare UHD transmits without establishing frequency, gain, or slot-A mapping.
- S4’s new high-rate cells exceed admission or capture-storage limits, leaving the intended sweep unmeasured.
- A sweep-rate override runs one rate but the decision line summarizes the hard-coded default.
- S5 loses evidence on an early error, or its rehearsal checks rates/channels without checking the resulting lead verdict.

I’ll now compare these with the brief and report and trace the relevant paths.
What this change does: Adds S2, T200, S5 and S4 measurement runners, decision summaries, rehearsals and bench instructions.

**Verdict: FAIL — fix the four majors before bench use.**

Paths below are relative to `crates/ezsdr-radio-uhd/tests/` unless otherwise stated.

### Major findings

1. **Bare UHD bypasses the required RF settings — VERIFIED omission; actual RF state UNVERIFIED.**  
   `plan/phase8/tools/t200-bare-uhd.sh:63–74`; `plan/phase8/bench.md:1407`.  
   **Sequence:** confirm attenuation → script opens a fresh UHD device → starts TX without establishing or reading back 1 GHz and safe gain. The documentation explicitly admits these values are unknown, contrary to the brief’s safety rule. Historical runs do not establish this invocation’s settings.  
   **Fix:** establish and verify the settings in the transmitting process, or disable this comparison pending an approved safe method.

2. **An untimestamped underflow can count as a clean ten-second window — VERIFIED logic; hardware occurrence INFERRED.**  
   `common/measurement/multichannel.rs:845–848`; `common/s4bench.rs:146–155`. Connected code: `src/provider/tx.rs:1126–1132`.  
   **Sequence:** underflow occurs before ten seconds without a device timestamp → report is processed after ten seconds → Provider substitutes current time → measurement treats that timestamp as the underflow instant. With enough accepted samples, `window_held()` returns `Some(true)`. Its missing-time safeguard never sees the missing raw timestamp.  
   **Fix:** retain timestamp provenance from the async report; an unplaceable underflow must make this window unknown. Add a rehearsal with an untimestamped report.

3. **Bare UHD has no whole-life deadline — VERIFIED.**  
   `plan/phase8/tools/t200-bare-uhd.sh:69–81`.  
   **Sequence:** `benchmark_rate` blocks during device creation, send or thread shutdown → synchronous Docker invocation never returns → no decision summary or later trial. `--duration` limits the benchmark interval, not these operations. This misses the brief’s whole-life-bound requirement.  
   **Fix:** bound each invocation, ensure the container stops on expiry, and report expiry as infrastructure failure/INCONCLUSIVE.

4. **The bare-UHD summary accepts incomplete or failed TX evidence — VERIFIED.**  
   `plan/phase8/tools/t200-bare-uhd.sh:76–84`; `plan/phase8/bench.md:1512`.  
   **Sequence:** benchmark exits zero with zero underruns but TX timeouts, sequence errors, insufficient transmitted samples, or missing sample/error fields → script prints an ordinary `[0 …]` summary without INCONCLUSIVE. That is insufficient evidence for the documented “bare clean versus Module failure” comparison.  
   UHD 4.10’s [source](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/examples/benchmark_rate.cpp) confirms that exit zero does not exclude TX timeouts or sequence errors with these arguments.  
   **Fix:** validate all required fields and transmitted volume; distinguish observed failure from incomplete evidence. Test those cases, not only label extraction.

### Minor findings / maintainability

5. **The sweep-rate override is ignored by its summary — VERIFIED.**  
   `common/s4bench.rs:665–679`; `hardware.rs:4146–4148`.  
   Setting `EZSDR_S4_STALL_RATE_HZ=50000000` runs the sweep at 50 MS/s, but summary selection remains hard-coded to 100 MS/s and drops the requested comparison. Carry the actual sweep rate into the summary, or remove the override.

6. **S5 retains unused high-rate sample payloads — VERIFIED; performance impact INFERRED.**  
   `common/measurement.rs:2386–2388`; `common/probe_device.rs:404–405`.  
   Each rung enables `keep_rx_blocks()`, copying every first-channel sample, although restart classification uses metadata rows, not those saved payloads. Extending this path from 1 to 100 MS/s substantially increases allocation and memory traffic during the measurement. Remove the unused retention from this path.

The report acknowledges unknown bare-UHD RF settings, but that is a safety deviation, not merely an evidence caveat. Existing rehearsals do not cover findings 2–4.

**Not checked:** No builds, hardware runs or mutation execution; mutation kill claims were not independently verified. Worktree unchanged; no probe copy created.
