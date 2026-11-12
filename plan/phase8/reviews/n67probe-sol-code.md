# #67 bench probe — Sol's code review (2026-11-13, openai/gpt-6.1-sol, read-only)

## Verdict: FAIL — fix two blockers before the bench

### Expected X310 behaviour

**VERIFIED against UHD `v4.10.0.0`:**

- `rx_streamer` forwards stream commands upstream; X300 inherits `radio_control_impl`, which writes the start timestamp into RX command registers—not a register write delayed until that timestamp. See [streamer forwarding](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_rx_streamer.cpp#L131-L149), [radio command submission](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/radio_control_impl.cpp#L1066-L1134).
- **STOP explicitly bypasses and clears the acquisition-command FIFO.** While waiting for a timed start, STOP takes priority. Thus `stop_before_origin` should cancel the queued start without producing its samples; control should stop an already-running acquisition, allowing buffered tail. See [FIFO handling](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v#L163-L184), [waiting-state handling](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v#L524-L550).
- The generic “untimed command waits behind timed command” rule is therefore **not sufficient to predict this acquisition STOP**.
- Late starts generate `LateCommand`; overflow is a separate receive-throughput failure, not proof cancellation failed.
- **Close is not equivalent to STOP:** the RFNoC streamer destructor disconnects its graph connections; it does not explicitly submit STOP. See [destructor](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_rx_streamer.cpp#L109-L114), [disconnect callback](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L340-L354).

**INFERRED:** STOP should permit an exact-origin subsequent start on the same streamer, provided commands arrive early and transport keeps up. Close/reopen cancellation and recovery remain empirical questions; silence on a replacement transport alone does not prove the old acquisition was cancelled.

### Findings

**Major 1 — cancellation can wait past origin and still PASS. VERIFIED**

`crates/ezsdr-radio-uhd/tests/hardware.rs:475–483`  
`crates/ezsdr-radio-uhd/tests/common/iii.rs:943–944`

`cancelled_at` is captured **after** stop—or close **and reopen**—returns. Nothing requires cancellation to complete before origin; all blocks timestamped before `cancelled_at + 2 ms` are excused.

Minimal counterexample:

```text
start(origin=2 s)
at 0.1 s: stop blocks until 2.05 s
old acquisition produces samples at 2.00–2.05 s
after-read sees those samples, but classifies every block as tail
fresh start succeeds → PASS
```

That both violates “no cancelled-start block” and leaves #67’s one-second join problem unresolved.

**Required:** record call-entry/return times separately, validate the before-origin precondition, report cancellation latency, and apply tail tolerance only to the running control. Add this counterexample to the pure tests. Also require observation actually to reach `origin + 500 ms`: `read_until` currently silently terminates on `time_now()` failure (`hardware.rs:468`).

**Major 2 — the advertised whole-probe hang bound excludes device creation and destruction. VERIFIED**

`crates/ezsdr-radio-uhd/tests/hardware.rs:511–531`

`raw()` runs before the 900-second watchdog; the device drops after it is disarmed. Either opening/configuration or final UHD teardown can therefore hang indefinitely. Unwinding also disconnects watchdog channels, disabling protection during outer cleanup.

**Required:** protect the entire device lifetime, including teardown and failure paths. The documented `hw` wrapper supplies no external timeout.

**Minor — bench conclusions overstate what failures establish. VERIFIED**

`plan/phase8/bench.md:1251–1253`

“No fresh block” does not establish device corruption or “cannot fix.” Overflow/LateCommand/host scheduling can invalidate observations without establishing #67’s mechanism. A passing control does not exclude a transient overload in another trial. Preserve the stated strict FAIL classification if desired, but distinguish **trial failure** from **evidence that cancellation is unsafe**.

**Minor — leak counter does not observe trial-level streamer leaks. VERIFIED**

`hardware.rs:469,499`; `src/uhd.rs:667–688`

`uhd_kept_count` grows when a **UhdDevice is dropped** into `KEPT`; this device remains alive throughout every comparison. These checks cannot substantiate the advertised streamer-leak check. Use appropriate lifecycle evidence or narrow the claim.

### Other checks

- **VERIFIED:** receive-only path; no transmit calls. Three rates × three variants × ten trials; standalone trace/verdict output and cleanup drain are present.
- Two consecutive timeouts establish present quiet, **not cancellation of a future queued start**; the full post-origin observation is essential.
- Finite/nonzero samples can pass on finite garbage or a stuck nonzero value. This is a weak sanity check, not signal-integrity evidence.
- Rows are named **N67-01…11**, not N67P-*. Their selected assertions distinguish their mutations. Existing logs record ten kills plus N67-08 killed after its compile-error correction; none are bound-only kills. They do not cover the blockers above.

No edits, builds, hardware runs, or mutation runs performed. Actual UHD-feature build and bench behaviour remain unverified.
