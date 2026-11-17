# Stage 4a — Sol's test-plan review (openai/gpt-6.1-sol, read-only; the final answer, tool transcript left out)

What this plan does: it tests the profile changes, receive-start head, confirmed open flush, and probe repair. **It is not ready for implementation:** it still selects two behaviours superseded by the recorded decisions.

References below: **P** = `/home/komatsu/.cache/ezsdr-fixes/2026-11-17-stage4a/test-plan.md`; **D** = the adjacent `decisions.md`.

Read-only review; no files changed, builds, tests or mutation runs. Bench measurements were not independently reverified against raw archives.

1. **Major — The plan still implements the rejected 5 ms placement rule.**  
   P:69–75 expects the Mock’s late SendAsap target at decision + **5 ms**; P:276 changes join expectations and gives Mock/UHD different oracle parameters. The final decision instead requires **5 ms admission, 2 ms placement, and a shared oracle** (D:8). Although P:75 describes a B alternative, it is not adopted consistently; P:387 still leaves RM-14 unchanged under the selected A branch.

   **Counterexample:** after the transmit origin, receive a late SendAsap burst with no occupied region. Following the main plan moves it 3 ms later on Mock than UHD. Adopt B throughout; separately test admission immediately below/at 5 ms and placement at receipt + 2 ms. Preserve the existing join/untimed-placement expectations (`crates/ezsdr-mock-radio/src/lib.rs:1630–1657`).

2. **Major — Overflow tests enforce the opposite of the final decision.**  
   P:145–148 requires **no** new head after overflow, and S4A-40 kills implementations that add one (P:354). The decision explicitly requires a head at overflow restart (D:9); the proposed normative text and KL entry also retain the rejected behaviour (P:388–400).

   **Counterexample:** receive a continuous nonzero signal, inject an overrun, then read the restarted samples. The planned test passes immediate signal delivery where the adopted model requires zeros first. Cover both injected overflow and Mock backpressure, since they advance the cursor through separate paths without replacing the segment origin (`crates/ezsdr-mock-radio/src/lib.rs:580–593,720–755`). Keep the SampleClock origin and gap accounting unchanged.

3. **Major — FakeDevice’s head is restricted to successful loopback, not every receive start.**  
   P:104,126–138 deliberately leaves the Fake’s ramp fallback unheaded. That fallback is reached when RX/TX frequencies or rates differ, or no matching transmitted samples exist (`crates/ezsdr-radio-uhd/src/device.rs:1275–1294`). The adopted rule says FakeDevice delivers the head at every receive start (D:3).

   **Counterexample:** configure RX at 100 MS/s and TX at 1 MS/s, start RX, and read its first 13 samples. The planned implementation returns the ramp rather than the required zeros. Add receive-only and mismatched-rate/frequency cases; apply the Fake’s head independently of which source supplies later samples.

4. **Minor — The on-time-signal assertion can use a fixture that cannot detect a shift.**  
   B2 permits “a `field` that is non-zero everywhere,” without requiring time variation, while B6 claims the caller tests kill the shifted-signal mutant S4A-29 (P:114–118,141–143,345). A constant field is unchanged by sampling at `sample − head`.

   Require a time-varying waveform with distinguishable samples around the boundary, and compare several post-head samples against their original ticks. Otherwise zeros inserted before a delayed signal can pass the advertised no-shift check.

5. **Minor — Receive tests do not explicitly exercise a head split across reads or an unread predecessor tail.**  
   B2/B5 specify start kinds and rates, but no read sizes or requirement to retain the stopped stream’s unread tail (P:113–139). FakeDevice separately stores the old origin and a pending successor, then switches them after draining the tail (`crates/ezsdr-radio-uhd/src/device.rs:473–494,1687–1700`).

   Add a short-read sequence—for example, a 24-sample head read as 7 + 7 + 11 samples—and a Stop/start with unread old samples. Assert that reads do not restart the head, the old tail is unchanged, and the successor receives exactly one complete head.

6. **Minor — Several proposed mutation checks cannot demonstrate their stated property.**
   - **S4A-12:** swapping RX for TX capacity is equivalent on the proposed fixture because both capacities are equal (P:79–81,331). Use a synthetic asymmetric Description, or remove this row.
   - **S4A-35:** replacing the shared function with an identical local table changes no verdict; the stated boundary test cannot distinguish them (P:355–356). Check shared-function use during review rather than claim a behavioural kill.
   - **S4A-64:** increasing the cap is not necessarily killed by “fill past the limit.” The existing test generates `PROBE_ROW_LIMIT + 1` rows and derives its expectations from that constant (`crates/ezsdr-radio-uhd/tests/common/probe_device.rs:507–515`; P:219,374). Pin an independent budget if the exact cap is contractual.
   - The mutation ledger also names A3’s killer under `parity`, although A3 places it in `tests/x3xx.rs` (P:65,326–327); C’s prose uses S4A-40–44 while its actual rows are S4A-50–54 (P:165–189,358–365).

7. **Minor — Migrated flush tests need a held clock, not merely timestamps changed to `Some(now)`.**  
   P:289 proposes that conversion for ReportFeed tests. Existing attribution tests enqueue scripted acknowledgements before invoking the flush, without a clock driver (`crates/ezsdr-radio-uhd/src/provider/core.rs:1975–1985,2042–2051`). The confirmed path reads a fresh device-time floor before sending (`core.rs:1264–1276`).

   A host delay between creating the scripted timestamp and reading the floor makes that acknowledgement stale. Explicitly hold the manual clock while establishing the floor and reports, as the existing strict-test fixture does (`crates/ezsdr-radio-uhd/src/provider/core/t3.rs:39–45`). Otherwise this migration introduces scheduling-dependent failures.

8. **Minor — The hardware boundary rehearsal claim is not backed by the listed Fake test.**  
   P:151–152 says the Fake rehearsal books both sides of the bound. B5 actually specifies a successful 24-zero case and retains the tone verdict’s tolerance of 40 (P:135–138); neither rejects a 25-sample settling head. The separate pure sweep-verdict test does book `n`/`n+1`, but that is not the claimed FakeDevice rehearsal.

   Make the audit precise: identify the hardware verdict being exercised and add a Fake-backed over-bound case through that same verdict, or explicitly record that only its pure decision is boundary-tested.

**Verdict: FAIL — amend findings 1–3 and repair the test/mutation claims before implementation.**
