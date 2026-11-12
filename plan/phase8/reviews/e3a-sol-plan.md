# E3a — Sol's test-plan review (2026-11-12)

## Verdict: FAIL — amend the tests before implementation

E3a’s scope is sound, and the E3a/E3b split leaves the later device-state design open. The blockers are test timing and observability, not the extraction itself.

References below to `test-plan.md` mean `/home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3/test-plan.md`.

### Major

1. **Q1/Q2 still depend on when the test thread runs.** **VERIFIED mechanism; INFERRED failure of the proposed tests.**  
   `test-plan.md:81–86,191` relies on 2–3 ms margins and twenty repetitions. But `UhdRig` does not register its driver with the manual clock (`crates/ezsdr-acceptance/tests/parity.rs:319–331,400–409`).

   **Minimal sequence:** book Q1’s burst just beyond the intended Stop horizon → return from `wait_until` → deschedule the test thread → submit Stop after the burst has become handed or completed. Correct production code now produces a record instead of the expected cancellation. Likewise, Q2’s fixed cross-provider/one-chunk comparison can fail after delayed Stop receipt.

   **Required:** use a frozen/stepped paired rig, or derive assertions from independently observed receipt/application times without requiring the intended window. Margins and repetitions do not satisfy note 51 §3.

2. **MC5 does not observe the wakeup it claims to protect.** **VERIFIED.**  
   `test-plan.md:79,127` manually steps around the block boundary and names MC5 as the killer for ignoring the held start in the wake calculation. Direct `step(until)` emits from the supplied time, independently of the previously scheduled wake (`crates/ezsdr-mock-radio/src/lib.rs:2460–2484,2053–2064`).

   **Minimal sequence:** open a repeat → hold its successor inside the next block → remove the held-start limit from `schedule_wakeup` → manually step at the correct boundary. Emission still uses the correct held cut, so MC5 passes despite the wrong scheduled deadline.

   **Required:** inspect the scheduled deadline, or drive the test through the Authority’s next wakeup. Assert both the deadline and resulting block.

### Minor

3. **MC4’s aggregate counts are not block characterization.** **VERIFIED coverage gap.**  
   `test-plan.md:78` observes `tx_blocks`, record block/sample totals and refusals—not ordered headers. For example, `[1000,1000,500]` versus `[999,1000,501]` can retain the same totals and valid contiguous burst record.

   **Required:** snapshot ordered `(first_sample_time, len, flags)` for finite end, wrap, held start, clock cut and Stop cut. This also preserves the distinction between an EOB-producing held cut and a non-EOB hard cut (`crates/ezsdr-mock-radio/src/lib.rs:1854–1873`).

4. **Some proposed mutants are equivalent under the planned guards.** **VERIFIED arithmetic.**  
   `test-plan.md:121,124`:
   - `ends_at_held: == → <=` changes nothing when extent already guarantees `end <= held_cut`.
   - `start < we → <=` changes nothing at `start == we` when the positive first-send requirement still rejects that booking.

   The old P631 killer directly exercises the unfiltered helper; that does not establish a killer for the combined window-only function (`plan/maintenance/tools/mutations.json:6861–6868`).

   **Required:** replace these with observable mutations, and provide an explicit old-row → replacement/killer mapping. Preserve Mock-only threshold and reduction checks at their owning call sites rather than silently changing their meaning during retargeting (`test-plan.md:114,140–144`).

5. **The hardware audit is incomplete.** **VERIFIED.**  
   `test-plan.md:150–163` omits `hw_p70_one_sample_burst_then_timed_burst`, whose onset comparison depends on precisely the padding/close behavior being extracted (`crates/ezsdr-radio-uhd/tests/hardware.rs:2797–2825`). It also leaves `hw_p72`’s rehearsal unresolved despite its rule-derived closing-send expectations (`hardware.rs:2770–2792`).

   **Required:** complete those audit rows and name matching rehearsals or add the missing coverage.

### Otherwise sound

- Window-only mode can preserve MR-25: keep open-versus-held thresholds, receipt filtering, saturation, previous-cut merging and the Stop-sample floor. Do **not** introduce real-time held-start/wrap restrictions into that model.
- Verbatim differential copies are useful transitional evidence, not independent correctness oracles. Keep them frozen through both Provider wiring checks.
- Keeping FakeDevice’s packet rules independent—including separating its `4` from the Module—is correct.

Not checked: no builds, probes, edits or hardware runs. Findings are from source and plan inspection.
