# 段階 3g：Sol のレビュー

## コードレビュー

> plan · gpt-6.1-sol
VERDICT: FAIL

### Major

1. **P6 can report zero continuation samples without observing the loopback.**  
   `crates/ezsdr-radio-uhd/tests/hardware.rs:1356–1362`

   If RX times out or loses the queued portion, `queued` is empty and `level` becomes infinity. The continuation count is then necessarily zero—even if the continuation actually transmitted. An independent TX `Underflow` still produces `recovery observed`. Missing samples after the continuation likewise produce zero without evidence of a drop. RX errors are not printed here.

   **Fix:** Report the loopback result as `INCONCLUSIVE` unless a usable queued-tone baseline and sufficient capture coverage of the continuation interval exist. Keep the TX-report observation separate. Add regression assertions for an empty baseline and missing continuation coverage.

### Minor

2. **An initially shifted tone is printed as an exactly dated move.**  
   `crates/ezsdr-radio-uhd/tests/common/tone.rs:88,97`

   The new F7 case correctly avoids `gapped`, but its timestamp is only an upper bound: the tone was already shifted in the first inspected window. The output nevertheless says it “leaves 100 kHz at” that timestamp. `bench.md` explains the intended interpretation, but the probe line itself overstates the measurement.

   **Fix:** Print “at or before” for this case and assert that wording in the F7-shaped regression.

### Checks that passed

- The power-floor and coherence conditions match the requested analysis. Missing floors make `usable()` false; a NaN floor rejects all signal windows.
- The LSB, F7-shaped, noise-only and five-quiet-window cases cover the intended regressions.
- P994–P998 and P1045 each match exactly once and have identifiable killing assertions. Their reported execution results were not independently rerun.
- UHD 4.10 sources support the empty-EOB mechanism and P5’s corrected burst-counted timestamp. P6/P7 clear stale state before their timed tune.
- P4 removes the known blocking-send cause. Stop can still be late after a receive timeout or scheduling delay, but the printed call/return times and `unconsumed` check expose that.
- RF settings remain 0 dB with amplitude 0.4. `git diff --check` passed.

Not checked: no builds, tests, mutations, Docker or hardware were run; no files were changed. Today’s logs contain summaries, not raw IQ, so their exact revised classifications cannot be replayed.

## 見直し

> plan · gpt-6.1-sol
VERDICT: PASS

Both previous findings are closed.

- **P6:** Missing or short queued baselines, zero medians, and insufficient continuation coverage now produce `INCONCLUSIVE`. Hardware output separates the loopback count, TX Underflow observation, and RX errors.
- **Move timing:** An already-shifted first window now prints “at or before.” Regression assertions check both this case and the ordinary on-time case.

The thresholds are reasonable for this controlled 1 Msps loopback probe:
- 10,000 queued samples require over half the available 19,000-sample baseline.
- 1,500 captured samples require about 79% coverage of the inspected 1,900-sample continuation interval.
- Half the queued median accommodates tone-amplitude variation while remaining well above the stated bench noise. This is a relative-amplitude count, not proof that every continuation sample arrived.

P1047–P1051 each match exactly once and have identifiable killing assertions:
- **P1047:** `fake.rs:8437`, “at or before” wording.
- **P1048:** `fake.rs:8465`, rejection of 5,000 queued samples.
- **P1049:** `fake.rs:8467`, rejection of a zero median.
- **P1050:** `fake.rs:8470`, rejection of insufficient continuation coverage.
- **P1051:** `fake.rs:8462`, noise-only continuation counts as zero.

`git diff --check` passed.

Not checked: builds, tests, mutations, Docker, and hardware were not run; reported mutation kills were assessed against the source assertions, not independently reproduced. No files were changed.
