# Brief for Sol: re-review of the E2a-2 fix round, read-only

You are the independent reviewer of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, format nothing, run no cargo command that writes into the tree, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.**

## Inputs
- Your review `~/.cache/ezsdr-fixes/2026-11-01-n45/sol-e2a2-code.md` (REVISE, 4 P2 and 2 P3), and the dispositions `dispositions-sol-e2a2-code.md`, all adopted.
- The results of the fix round are in the plan `test-plan-e2a.md` §17, with §17.1 and §17.2.

What changed:
- **#2:** the packet-granular receive fixed in **E1's device model** (`model.rs` `Device::recv` returns nothing unless `packet_ready`), so the DST world, `ModelDevice` and FakeDevice agree. Script (i) was added. `Params.packet` defaults to 4; the oracle happy-path world uses packet 1. After the fix, E1 was rerun:
  - all sweeps at 2,000 seeds pass;
  - `s1b_full` passes;
  - the micro `_full` runs: case 6 now completes and passes; case 4's bounded search misses W4b, which the walks reach.
- **Thread-wrapper fix:** a Move's outcome comes from its own call (`ModelDevice::time_now` errs lost at the call). The post-call `serial()` probe is removed. The test is `s3d_…`.
- **Q1 wide run:** seeds 1–2000, 26,000 walks, 1,787 divergences, 0 unclassified.
- **#1:** S6 and S7 on the manual clock.
- **#3:** channel shape asserted.
- **#4:** `q1_g2_is_not_reachable_through_the_booking_floor` pins why g2 is not reachable through production's booking floor.
- **#5:** `s8b` advances 3 ticks between the read and the send.
- **#6:** `fixtures::stop_debt_world` is shared.
- **Rows:** E2-55…E2-58 new; E2-20 re-anchored.
- **A FakeDevice clock quirk** (§17.2), with a 20 ms nudge in `Lock::settle`.

## Check
1. Is each of your six findings closed? Give the evidence at `path:line`.
2. **The packet change in E1's model.**
   - Is it faithful to FakeDevice's `rx_recv` packet semantics (`device.rs` around 1730-1770), including one-packet mode, the tail at a stop, and the deadline?
   - Could it hide a scenario E1 used to exercise? Look at which witnesses or cases now depend on `packet: 1` or longer epilogues.
3. **The `serial()` probe removal.** Is the thread wrapper now protocol-equal to the world for every settings command, not only Move?
4. **The clock quirk (§17.2).** From `src/host.rs`, is it a HostClock defect (a lost wake-up when a non-participant's wait ends at the instant the clock stops)? If it is, give the minimal sequence; it is a production test-infrastructure bug. Does the 20 ms nudge make any verdict depend on wall time?
5. **#4.** Is the booking-floor argument sound? Does it cover different channels and the 3 ms window edge?
6. **Readiness.** Is E2a, parts 1 and 2 together, ready for the full gate? Name any blocker.

## Output
- Verdict: PASS or REVISE.
- A table with one row per finding.
- New findings P1/P2/P3. A P1 comes with a minimal sequence and its observable wrong outcome.
- At most 70 lines.
- End with "Not checked:".
