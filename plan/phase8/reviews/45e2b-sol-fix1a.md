# Sol 6.1 — E2b fix1a review, report section

# REVISE — Part A

1. **P1 — A freed slot can reserve an already-past value.**  
   `crates/ezsdr-radio-uhd/src/brain/core.rs:643–647`; `crates/ezsdr-radio-uhd/src/brain/cell.rs:242–253`  
   Counterexample: fill four slots; release held v5 at **e=13**, with cut=32; publish a block through tick 15, advancing progress to 16; process an earlier value’s consumption. The retry reserves v5 and sends its timed command at 13, although 13 has already passed. The device can move its effective instant forward.  
   The shared reservation rule checks progress against **cut**, not **e**. Thus the implementation preserves that existing rule but cannot satisfy your explicit no-late-reservation requirement. Adjudicate this conflict and add the counterexample test; do not invent a disposition silently.

2. **P2 — S7b does not pin a non-step-aligned origin.**  
   `crates/ezsdr-radio-uhd/src/brain/device_tests.rs:55–64,237–250`  
   At ratio 2, this setup starts at origin 40. An incorrect mapping that rounds device ticks onto the global even lattice would still pass. Use an explicitly odd origin and assert consumption on its origin-relative lattice, including an instant between samples. The implementation’s `first_tick + index × step` mapping itself appears correct.

**Other checks**
- All slot removals reach the retry wrapper; phase, channel, generation, origin and cut restrictions remain shared. Successful reservation removes the held entry; another refusal leaves it held without spinning.
- Pruning is safe under unique value IDs: `consume_samples` and `flush_pending`—also through `flush_over`—read only IDs present in current `in_flight`. Retired IDs cannot affect subsequent decisions.
- Jump-ahead consumption preserves per-channel head ordering, left-before-debt precedence and one consumption per channel per sample. World and threads retain the shared rules.
- The three new and four re-anchored mutation rows have the intended source semantics. `rules.py --base HEAD`: **0 problems**.
- Independent remote brain run: **328 passed, 53 ignored**.
- **HEAD also misses W7b for seeds 1–2000**, with no failing seeds: the reported vacuity is not new.
- No additional maintainability finding.

Not checked: Part B, hardware, or mutation execution. No source files changed; holdout contents were not read.
