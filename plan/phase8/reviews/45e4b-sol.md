**REVISE — tooling and records. The fixed speed verdict is FAIL. No P1 found.**

### Verdicts and experimental coverage
- The four idle-host **PASS** verdicts and both preload **FAIL** verdicts are computed correctly. Independent recomputation from `E2B_UPDATES` matches the reported percentiles; per-repetition checks prevent pooling from hiding failures.
- Preload spaced fails p99; preload bursty fails one repetition’s p99 and another’s loss comparison. These failures remain binding under §11.
- Preload entries are excluded before DSP accounting, without excluding measured updates. K/D counts and attempt-time maxima match the reported results.
- ABBA/BAAB order is implemented. The much lower host load is disclosed; E4b does not isolate the design change from E2b’s host-load difference.
- The orchestrator’s sustained-backlog conclusion is right: **that condition was not demonstrated**. Separate preload diagnostics show only an initial excursion, then sampled backlog ≤0.32 ms. They cannot establish backlog throughout the unsampled verdict repetitions.

### Findings
**P2 — Coupling timestamps can falsely report a preceding hit.**  
`crates/ezsdr-radio-uhd/src/brain/speed/runs.rs:379–387,593`
- Sequence: release A → first attempt rejected at K → later retry rejected at D → A applies late. `own_d` reports a D hit preceding A’s release even though its first D hit occurred afterward.
- Rejected updates also use first-attempt time as “release”, while other updates use nominal release. A hit during brain-queue delay can therefore be misclassified as preceding release.
- This can falsely attribute lateness to D under the brief’s temporal definition. Use one release definition and compare hit timestamps explicitly; otherwise rename the diagnostic. Current zero-hit verdicts are unaffected.

**P2 — Diagnosis omits an evidenced alternative to host disturbance.**  
`~/.cache/ezsdr-fixes/2026-11-01-n45/e4b-results/README.md:83–84`
- In failing spaced rep 4, the top 100 delays enter apply within **96.146 µs**, then spend **1,011.514–1,481.819 µs inside apply**.
- Fake receive holds the device lock across history scanning and sample construction (`crates/ezsdr-radio-uhd/src/device.rs:1785–1803`); settings apply takes that same lock (`:1423`). The scan grows with DSP history (`:552`).
- Receive-scan lock contention/starvation is therefore a concrete alternative, even without sustained receive backlog. Attributing the excursion to an external tenant or scheduling disturbance alone could incorrectly dismiss workload-induced contention. The data do not distinguish these causes.

**P2 — Overflow timing is promoted from inference to verification.**  
`~/.cache/ezsdr-fixes/2026-11-01-n45/e4b-results/README.md:82,86`
- Line 82 places every overflow at/before the first update, but line 86 correctly acknowledges that verdict output has no per-gap timing.
- Late-tail clustering is verified; the exact placement of verdict-run loss is not. Treating startup placement as proven can wrongly exclude a later receive disruption. Downgrade that claim and limit startup-backlog observations to the sampled diagnostic runs.

**P3 — Recompute the diagnosis table’s exact counts.**  
`~/.cache/ezsdr-fixes/2026-11-01-n45/e4b-results/README.md:77,79`
- Spaced rep 4’s largest 100 delays span **1,107.660–1,564.011 µs**, with **98**, not 99, in 41.6–50 s.
- Bursty rep 4 has **61**, not 64, in 16.7–19.05 s and **18**, not 20, in 22.6–23.9 s. The broad clustering conclusion survives.

### Tooling rows
E4-118…122 each have one token-anchor match and a named test whose assertions distinguish the mutation: 118/121 → reservation attribution; 119/120 → coupling computation; 122 → preload filtering. No compile defect apparent by inspection, but these rows do not cover the timestamp counterexample above.

Not checked: cargo tests, mutation execution, fresh measurements, hardware, or either prohibited holdout directory.
