# Sol 6.1 — E2b fix3 review, report section

# REVISE

Paths: `B` = `crates/ezsdr-radio-uhd/src/brain/`; `R4` = `~/.cache/ezsdr-fixes/2026-11-01-n45/e2b-results-4/`.

## Findings
1. **P2 — Timer-test readiness remains incomplete.** `B/device_tests.rs:689–691`; `B/threads.rs:366–375`; `plan/maintenance/tools/mutations.json:18975–18982`.
   `s15b` can observe the routed command **before** routing inserts its timer, then mistake the still-empty timer list for delivery.
   Sequence: pause routing after `sink.routed.push`; both polls succeed; resume routing; shutdown proceeds without proving deadline delivery.
   Thus **E2-89 can escape for the wrong reason**. Establish the pending booking before awaiting removal, or retarget E2-89 to `s15d`.

2. **P2 — The README overstates the quantitative interpretation.** `R4/README.md:29–35,52–54`.
   “3–5x” does not describe bursty failures: pooled reduction is **7,912 → 5,044, about 1.57x**.
   The stated model `9 µs + H × 0.9 ns` crosses the 10 µs budget near **H=1,111**, not 4,000.
   “Every run … first 20 s … 0.06 ms” also overstates the data: the second spaced/bound-16 run reaches **40.805 ms in second 20**.
   Correct these claims; retain the qualitative conclusion with its uncertainty.

## Closure and evidence
- **Closed:** trial misreading; reader-side B3 loss; B1 identity persistence/INFERRED labeling. Independently recomputed all **32 saved runs**: loss numbers and comparisons do not change.
- **Mostly closed:** wake readiness and missing regression coverage. `s15a`, `s15d`, stale-view recheck, delayed shutdown and measured-unlogged wiring address the requested cases; finding 1 remains.
- **VERIFIED:** the bound defaults to `IN_FLIGHT_BOUND`; the only non-test override is measured-runtime setup (`B/core.rs:305`; `B/speed/newrt.rs:169–170`). DST/forcing/default E1/E2a paths retain their bound. Bound-16 results are explicitly diagnostic.
- **VERIFIED:** the fake scans DSP history per block; H=10,000 increases whole-R CPU cost and saturates R. Successful commands retain slots until sample consumption (`src/device.rs:548–553,1800`; `B/core.rs:629–674`).
- **INFERRED:** that scan is the dominant cause of collapse in the update runs. The preload experiment and synchronized backlog/bound observations strongly support it, but do not isolate scan-only time or exclude host noise and retry amplification.
- **Today is exposed to the same scan**, through the same fake receive path. It keeps up with **settings**, not lossless reception: command slots expire at device time independently of R (`src/provider/control.rs:659–666,688–689`). Saved high-rate runs show zero settings failures but substantial receive loss.
- Static review supports E2-92–112’s intended mutant/assertion pairs and the re-anchors; all 31 checked rows have unique token anchors. This is not execution evidence.

Not checked: test or mutation execution, timing repetitions, hardware. No files changed; holdout was not accessed.
