# Sol 6.1 — i76-code (issue #76), report section

## REVISE

All paths below are under `crates/ezsdr-radio-uhd/`.

### Findings

1. **P2 — Both EOB-recovery tests still depend on scheduling** (`src/provider/tx.rs:6099–6110,6152–6167`).
   - Counterexample: delay the test thread 20 ms before the recovery `tx.pass()`. The flush ACK advances `chain.front` beyond the fixed Core clock’s 10 ms submission window. `SendAsapAndFlag` moves the next burst beyond `window_cut`; `step()` requeues it instead of sending. The send-count assertion fails.
   - Fix: run recovery as a clock participant, then hold `Frozen` and synchronize both clocks before booking and dispatching the next burst. Merely changing the late policy is insufficient.

2. **P2 — The successor receive still has an overflow window** (`src/provider/control.rs:8123–8125`).
   - Counterexample: after `drop(frozen)`, delay 200 ms before `rx_recv`. Its first unread block is now over 100 ms behind; `FakeDevice::rx_recv` returns `Overflow`, not `Samples` (`src/device.rs:1716–1724`).
   - Fix: perform the receive on a registered participant, registered before releasing `Frozen`. The earlier handoff assertions are now protected correctly.

3. **P2 — H4 still assumes the first update cannot be late** (`tests/fake.rs:9233–9235`; `tests/common/measurement/h4.rs:395–424`).
   - Counterexample: read `first`, then deschedule this thread for 100 ms before submitting update zero. Its target has only 53 ms lead. Rejection leaves `apply_called_count == 0`, failing `applied > 0`.
   - Fix: allow zero applied updates in the real-clock smoke test; pin successful timed-apply wiring separately with controlled timing. Keep P1139/P1142/P1154 covered there rather than relying on this scheduling-sensitive assertion.

4. **P2 — H4 lost end-selection and successful-mapping coverage** (`tests/fake.rs:9242–9247,9298–9304`).
   - Counterexample: an underflow ends TX, followed by successful gain applies. Changing `tx_end_ticks.into_iter().min()` to `.max()` (`tests/common/measurement/h4.rs:544`) can classify those applies as concurrent; the new accounting checks still pass and the conditional exclusion assertion is skipped.
   - Likewise, forcing the dispatch-mapping call at `h4.rs:445` to return `Err` now passes: every row becomes unavailable/ambiguous. Direct `dispatch_delay` tests do not cover that wiring.
   - Fix: add deterministic fixtures covering **underflow → later apply → exclusion**, continued gain application, and successful mapping through result assembly. The new `tx_overlap` test alone does not preserve these properties.

5. **P2 — Nearby receive tests retain the same pattern** (`src/provider/rx.rs:1182–1184,2018–2032`; `src/provider/control.rs:4166–4168`).
   - Counterexamples: delay 200 ms after the first/third clock release: receive returns `Overflow`. Delay 10 ms after `rx.rs:2018`, before worker registration: the asserted exact 7 ms return time is already past.
   - Fix: register receive workers before dropping `Frozen`; retain participant ownership until any exact-time observations finish.

### Other checks

- `ur_24_fake_ddc_distinguishes…` is now clock-controlled and preserves its assertions; P1221 still targets the intended distinction.
- `ur_23_m14e…` retains its size/channel/SOB checks. Exact new-clock timestamp coverage remains in `ur_25_cold_change_does_not_inherit_old_tx_front` (`tx.rs:3051`); late relocation is pinned at `tx.rs:2476–2480`.
- `tx_overlap` is identical to the old match: start inclusive, end exclusive, ambiguity unchanged. P1151 now directly checks its intended boundary.
- The other inspected mutation rows retain relevant assertions, but the remaining flaky paths mean remote “killed” results cannot establish scheduling independence. P1158 remains recorded equivalent.
- `assert_h4_accounting` is reasonable shared arithmetic, with no maintainability blocker; it is not a replacement for behavioral coverage.

Not checked: tests or mutations were not rerun; counterexamples above are code-derived. No files changed, and the prohibited directory was not read.
