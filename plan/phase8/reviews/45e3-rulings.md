# E3 rulings (orchestrator, 2026-11-03), after Sol's translation check (e3/sol-translation-check.md)

- Translations: H01–H06, H09, H11, H14–H16, H18, H19 faithful as written. H07, H08, H13, H20 replaced by Sol's R07, R08, R13, R20 (R13 changes only the decision predicates in core.rs, leaving the cell helpers that the oracles call; R08 keeps the recv-start cut across stale retries; R20 keeps FIFO head blocking and consumes repeatedly at one sample; R07 acknowledges quiescence on cancellation without the recv finishing).
- H10 (loss recording), H12 (native handle release/retention), H17 (join-timeout cleanup): **not found — not modelled**. They count against the score (the experiment does not model a concept note 45's design needs, §4.5, §4.7, §4.9). The report also gives the score over the 17 modelled errors.
- A panic in the DST world is counted as found only when it is the modelled program failing, not an observer unable to represent the illegal state (H01's `reserved()` unreachable is the latter: record it separately, not found unless an oracle fails).
- A case failing only on a declared witness becoming unreachable is **not** found (coverage, not correctness).
- H13 is scored once, by the corrected main row; HO13a/HO13b are diagnostics.
- The criterion stays as note 45 §7 states it: at least 16 of 20.
