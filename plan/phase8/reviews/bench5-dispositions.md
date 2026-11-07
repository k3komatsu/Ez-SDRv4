# Bench preparation after note 47 stage 5: dispositions (2026-11-08)

The step adds bench-only hardware tests and a runbook. Production code is unchanged.
- **Risk:** not risky, so no test plan and no plan review.
- **Implementer:** Haiku 5.5 `anthropic/claude-haiku-5-5#high`.
- **Self-review:** Go DeepSeek.
- **Pre-check:** Sol checked the brief before any code (AGENTS §9; the memory "Sol pre-check for hardware briefs").

**Records:** `bench5-brief.md` (the brief, Sol's pre-check section and the three fix briefs) and `bench5-gate.json`.

**What the next bench checks**, in the runbook section 「<DATE> の手順（note 47 段階 5 の後）」 of `plan/phase8/bench.md`. Each item is checked on both boards.
- (b) Regression of the Module transmit tests:
  - #70 and #78 stay closed.
  - At end + 1, `hw_b8_burst_one_sample_after_a_burst` now expects a refusal at receipt (RM-15; the front is end + 2).
- (c) Fronts after 1–32-sample bursts at 1 and 2 MS/s, and the raw gap table of what the device itself allows.
- (d) The held tail at `J(e)`, a booking at `e+2` after it, and `e+1` refused.
  - The Stop's two-zero close, raw, at `c+3`.
  - R5-m5 (h), raw.
- (e) The repeat switch: the boundary, no EOB or new SOB at it (from the recorded sends), and the tone before and after.
- (f) Untimed `repeat; stop; repeat`, and an untimed burst placed after a playing burst.
- (g) Where a Stop ends the transmit, against the record's end (RM-16; `tx_in_flight_ns` is printed).

Out of scope:
- KL-37's refusal: it is decided at receipt only, and parity tests cover it.
- R5-m5 (i): it needs window 0, which no hardware has.
- The transmit-rate acceptance and the ssh drop: they are not stage 5 and are listed separately.

## Sol pre-check (FIX FIRST, 4 + 4 SHOULD + 1 NOTE)

8 adopted:
- S1: T6 is judged against the recorded stop.
- S2: the continuation control is C at `e`.
- S3: T3 (b) and (h) use a separate bound and cursor, raw where the Session cannot place them.
- S4: "no EOB" comes from the recorded sends.
- S5: the capture starts before the switch.
- S6: the raw axis.
- S7: "on time" needs a measured onset.
- S9: 2 MS/s is added.

S8 was rejected: mutation probes in a throwaway copy outside the repository are what AGENTS §9 allows.

## Sol code review (FAIL, 7 × P2, 1 × P3; all adopted)

1. T3 deleted its captures before judging them, so it would have panicked on hardware.
2. Provider refusals were judged as Kernel admission.
3. Capture gaps were erased when spans were rebuilt from the first one.
4. T3 (b) did not test the start after the actual closing cursor; a raw two-zero close case was added.
5. The raw matrix omitted the Module's four-zero startup flush.
6. Raw waveforms reached magnitude 0.57, above the bench's 0.5 bound (RF safety). They are now built through `transmit_samples()`.
7. T6 invented a power edge on noise.
8. The runbook kept stale closing rules for #70 and #78.

## Opus re-checks

**Round 1 (FAIL, 3 major, 1 minor; all adopted).**
- T6 could never reach a verdict. It repeated PN, so no tone onset was found; it now uses a timed tone.
- T3 (f) failed a correct device when the booking reached the Provider late (host latency). It is now INCONCLUSIVE, recognised by C's dropped-as-late signature.
- A Provider refusal of a booking that should play gave INCONCLUSIVE; it is now FAIL.
- Row 183 was killed by an index panic.

**Round 2 (PASS WITH MINORS).**
- Minors fixed without re-review:
  - The late text now reads "at receipt or at dispatch".
  - The window text is corrected.
  - A row was added for the untimed refusal.
  - A refused switch prints its reason.
  - At the Stop sample, only an "overlaps an accepted booking" refusal is INCONCLUSIVE; any other refusal is FAIL.
- Deviation accepted: the Session case at the Kernel's stop sample is an observation. The Provider takes its bound from its own receipt of the Stop, which the Manifest does not record (`control.rs:313-316`, `core.rs:399-402`). The raw close case judges the geometry.

## Self-review

- **First run:** completed on the third try after transport errors. 5 adopted, 4 rejected with reasons.
- **Fix round 1:** failed twice (ECONNRESET), so no self-review.
- **Fix round 2:** completed; one P2 was fixed (an untimed refusal with no `bursts` row).

## Mutation

The full lists ran once before Sol's code review, as the new rule asks (AGENTS §9 step 6): 1933/1934 passed, and G09 is a known survivor. New rows N47S5-159…196 were probed in throwaway copies.

## Counts and mechanisms

- Sol pre-check: 4 FIX FIRST, 4 SHOULD, 1 NOTE.
- Sol code: 7 P2, 1 P3.
- Opus: 3 major, then 4 minor.
- Mechanism: none new. The repeated pattern is "a VERDICT judged from the wrong evidence" (Kernel admission for a Provider refusal, a recomputed timeline across capture gaps, a fallback delay). It is a test-design pattern of bench probes, not a production mechanism.

Tooling:
- The mutation watchdog false-positived on row 163 when its anchor moved (as in stage 5), so it was stopped.
- While it was down, an orphaned mutant test that looped forever ran for 2 h 15 min; it was killed by PID.
- The implementer's brief already forbids leaving background jobs. The probe was a foreground test whose parent exited.
