# Sol 6.1 — E2b fix2 review, report section

# REVISE

Targeted wakes remove substantial adapter overhead, and both 1 MS/s conditions pass. **The saved evidence does not support calling the high condition “unsupported.”**

Paths below: `B` = `crates/ezsdr-radio-uhd/src/brain/`; `R` = `~/.cache/ezsdr-fixes/2026-11-01-n45/e2b-results-3/`.

## Independent failure scenarios
Before comparing prior findings, I considered: push between predicate-check and sleep; stop/loss during continuous receiving; an earlier timer booking; cell changes between first view and publication; four unretired values despite advancing device time; late returns; and shutdown/timestamp gaps escaping accounting.

## Findings
1. **P1 — The headline misclassifies the trial.** `R/trial-today.txt:6–8`, `R/trial-new.txt:6–8`.
   Both owners **pass candidate 2, 50 MS/s × 500**, at **99,999.98 blocks/s**, zero overflows/gaps, and stable approximately **0.05 ms** backlog.
   The quoted **62.6k/75.3k** rates belong to **candidate 0, 200 MS/s × 2,000**.
   The brief’s amendment 4 therefore does **not** permit excluding the high verdict. Correct the report: supported no-update trial; timed-update verdict FAIL.

2. **P2 — B3 still excludes reader-side missing samples from the verdict.** `B/speed/runs.rs:403–407,522–537`.
   Loss uses `tap_stats.missing`, although reader gaps and its trailing gap are calculated separately.
   Counterexample: publish contiguous blocks; the bounded link drops the final block; drain. Reader missing samples increase, but reported `missing_samples` remains zero.
   Use reader leading/between gaps plus the trailing gap once, or obtain explicit approval to redefine this metric. Publication-drop blocks remain a separate number.

3. **P2 — B1’s booking-identity evidence remains incomplete.** `B/speed/mod.rs:626–656`; `B/speed/runs.rs:385–388,468–489`.
   Return expiry is fixed, but requests and apply rows are still paired by instant, without booking identity; stray instants are merely listed.
   Counterexample: booking moves one request onto another table instant. The report cannot identify its actual booking or reliably attribute the return.
   Persist request-to-booking identity/effective instant; do not describe requested-instant matching as observed booking evidence.

4. **P2 — Some new-path guarantees lack targeted regression tests.** `B/device_tests.rs:613–632,671–683`; `B/threads.rs:130–131,630–635,808–857`.
   `s15a` pushes without proving actors reached their waits: a wrong-wake mutant can escape if actors first inspect already-populated queues.
   Establish waiter readiness, then push; also test an earlier booking while the timer is already sleeping.
   Of the four “no killable row” claims: **snapshot-after-stop order** needs a delayed-shutdown test; **unlogged start_measured wiring** needs an integration assertion; **single evaluation** is acceptable by inspection; **first-view reuse** needs a controlled nonforcing stale-view/recheck test. Forced replay deliberately bypasses that optimization.

## High-rate interpretation
- **VERIFIED — Retirement couples settings capacity to host receive progress.** `B/core.rs:626–671,910–917`: successful calls retain their slot until consumption evidence arrives. Today’s command slots instead expire at device time (`src/provider/control.rs:659–666,688–689`).
- Minimal sequence: apply four values at **100–100.3 ms**; R remains behind 100 ms; release the next group at **107 ms**, for **110–110.3 ms**. All four slots remain occupied. If R catches up after 110 ms, those commands are necessarily late—even though the device had capacity.
- **VERIFIED — The measured failures strongly implicate that coupling.** High repetitions have **1,136–4,094 failures**, all recorded reservation rejections are **bound**, and some values remain held for **1.5–6.1 seconds** (`R/verdict-high-{spaced,bursty}.txt:9,11,15,21`). Today accounts for every update despite more receive loss.
- **VERIFIED — Backlog can reach tens of milliseconds:** `R/trial-after.txt:7` records **31–69 ms** excursions. However, the selected candidate-2 trials stay near **0.05 ms**; the verdict files lack synchronized backlog samples. Those trial excursions alone do not prove each verdict failure’s cause.
- **VERIFIED — FakeDevice’s history scan is O(H)/block**, common to both owners (`src/device.rs:548–553`). **INFERRED:** it triggers receive backlog; the bound converts backlog into settings lateness. Remaining adapter cost and held-value retry amplification (`B/core.rs:399–401`) may contribute. “Only the fake” and “intrinsically slow cell locking” are both unproved.

## Checks and remaining work
- No lost-wake defect found: predicate evaluation and sleeping share the scheduler mutex; push/stop/abort take it before notification. Continuously receiving R checks stop/failure without waiting. Timer booking wakes the deadline waiter.
- Forcing behavior is preserved in the reviewed branches—not byte-identical source. S1/full/walk evidence supports equivalence; independently, the brain suite passed **348 tests, 54 ignored**, including conformance. `rules.py --base HEAD`: **0 problems**.
- B2 shutdown cleanliness, B4 diagnostic labeling, B5 bounded reason attribution, B6 fallback removal, and B1 expiry are corrected; **B1 identity and B3 reader loss remain open**.
- E2-73–83 and E2-90–91 have appropriate mutant/assertion pairs. E2-84–89 target the intended wake behavior, but readiness needs tightening above. These are static assessments, **not evidence that all 19 killed**.
- **No lower-rate ladder is required by the saved trial.** To isolate causality, run one paired high-rate diagnostic at bound 4 versus the device’s actual DSP capacity, recording backlog, occupancy, retirement and apply stamps. Keep the larger-bound result diagnostic: changing the bound is not retroactive evidence for the original design.
- If FakeDevice cost still prevents attribution, authorize an isolated scan-cost diagnostic before rerunning the affected high conditions; do not repeatedly rerun the entire matrix without a discriminator.

Not checked: mutation execution, 20-run timing repetitions, hardware, or holdout contents. No repository files edited or formatted.
