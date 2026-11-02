**Verdict: REVISE.**

Independent failure scenarios were recorded before reading the brief. `B` below means `~/.cache/ezsdr-fixes/2026-11-01-n45/brief-e2b.md`. `brain/` was treated as work in progress.

### Findings

1. **P1 — Apply-entry latency omits part of the promised end-to-end path.**  
   `B:19-24`; `plan/phase8/45-uhd-decision-core.md:159`; `crates/ezsdr-radio-uhd/src/device.rs:1423-1458`.  
   A new-owner call can enter at release + 50 µs, wait 500 µs for FakeDevice’s mutex, then enqueue successfully before `e`. Its DSP log still shows `e`, so both checks pass although submission took 550 µs. Native-control contention is likewise downstream of entry. Keep entry latency as a diagnostic; add a binding completion/acceptance metric covering that wait, and distinguish it from DSP consumption.

2. **P1 — Effective instants are fixed, but submission instants are not.**  
   `B:34-38,53-54`; `crates/ezsdr-radio-uhd/src/provider/control.rs:641-673`.  
   Submitting all 10,000 updates upfront rejects the seventeenth pending command by design. Submitting each at `e − 3 ms` leaves only 1 ms before the 2 ms booking floor moves it. Either can falsely condemn today’s owner while the new harness bypasses booking. Fix an ordered, bounded-lookahead ingress table; record requested/effective instants and ingress lateness. Four-update bursts are safe **with that scheduling**, not from spacing alone.

3. **P1 — Equal device configuration does not establish equal receive work.**  
   `B:53-54`; `crates/ezsdr-radio-uhd/src/provider/rx.rs:809-836`; `crates/ezsdr-radio-uhd/src/brain/threads.rs:633,678-682`.  
   Today’s owner converts samples, constructs blocks and publishes links; the current new runtime requests `PACKET` samples and stores publication records in a test sink. A smaller receive request or cheaper publication path can produce an apparent ownership win. Require identical effective receive lengths, packet policy, payload/publication obligations and bounded sink consumption.

4. **P2 — The rate trial can accept a workload that cannot sustain the measurement.**  
   `B:40-43`; `crates/ezsdr-radio-uhd/src/device.rs:1716-1724`.  
   FakeDevice tolerates roughly 100 ms of backlog. A consumer running 4% slow accumulates only 80 ms during the two-second trial: no overflow or gap, yet the 25–50-second update run overruns. Check achieved blocks/s and backlog growth, including late-run windows. Fix the fallback candidates beforehand, verify their actual rates after decimation, and report “unsupported” if none sustains 100,000 blocks/s.

5. **P2 — Per-block timing instrumentation is asymmetric.**  
   `B:48-60`.  
   Only the new owner receives cell wait/hold timing on every block. At 100,000 blocks/s, timestamp reads and distribution bookkeeping can themselves push its p99 over the threshold, falsely rejecting the design. Separate diagnostic runs from verdict runs, or demonstrate bounded overhead with paired instrumentation-on/off runs. Bounding the device decorator does not bound this instrumentation.

6. **P2 — Loss is not defined in comparable units or at a common endpoint.**  
   `B:25,48-50`; `crates/ezsdr-radio-uhd/src/provider/rx.rs:827-836`.  
   “Overflow results plus … lost samples” mixes events with samples; device-returned block counts also miss downstream publication drops. The new owner could receive every block but discard publications and still pass. Compare overflow events, missing samples and publication drops separately, over identical observation boundaries.

7. **P2 — The release timer’s resolution remains unspecified.**  
   `B:21,54`; `crates/ezsdr-radio-uhd/src/brain/threads.rs:701-730`; `crates/ezsdr-radio-uhd/src/brain/force.rs:114,147-161`.  
   Currently K serves clock reads; a separate timer delivers stream deadlines through a scheduler with a 2 ms timed wait. Reusing that wait for releases can fail the 300 µs criterion merely because no notification arrives near the deadline. Specify the release-input deadline mechanism, include its lateness, and prohibit early delivery or synchronous harness assistance.

### Checklist disposition

- **Hidden wake-up delay:** prevented by using the scheduled `e − window`, not actual timer delivery. Downstream waits remain omitted: finding 1.
- **Dropped updates / DSP application:** substantially prevented by all-update accounting and exact `(tick, frequency, timed)` DSP-log checks. Match one-to-one; do not use log-observation time.
- **Load bias:** partially prevented by host selection and interleaving. ABAB still correlates owner with run position; use counterbalanced ABBA/BAAB and retain per-run results.
- **Statistics:** 10,000 updates is sufficient for the stated count rule. Nearest-rank p99.9 is zero-based index **9989**, with ten samples above it; compute separately per condition and repetition.
- **Clock mapping:** zero drift fixes the slope, not the host/device epoch offset (`device.rs:1331-1332`). Establish and report the offset/bracket uncertainty after time-zero setup.

Not checked: builds, executions, live host load, unfinished E2a behavior, or the prohibited holdout directory.
