# #117 — reviews

## GPT-6.1 Terra, review

**Verdict: FAIL.** The admission change closes the intended race at the two start sites, but the stop site can panic after a published loss.

1. **Major — loss before the declaration-failure stop panics the RX owner.** A segment start succeeds, leaving the chain `Armed`; clock declaration fails; another thread publishes device loss before the stop’s gate. The admitted stop returns `None`, then `crates/ezsdr-radio-uhd/src/provider/rx.rs:467` calls `chain.finish()` without marking the chain lost. `finish()` asserts that the chain is off, uncertain, or lost, so the owner panics (`crates/ezsdr-radio-uhd/src/provider/core.rs:160`, `crates/ezsdr-radio-uhd/src/provider/core.rs:229`). Mark the chain lost on `None` before finishing, as the normal stop path does (`crates/ezsdr-radio-uhd/src/provider/rx.rs:324`).

   The claimed test limitation is incorrect: a test can exhaust clock IDs by registering a high local domain ID, then exercise this branch; the kernel already demonstrates that mechanism (`crates/ezsdr-kernel/tests/time_model.rs:560`). A regression test matters because it would catch this panic.

2. **Major — the new mutation tests can fail without exercising the race.** Both probes are invoked inside `admitted` (`crates/ezsdr-radio-uhd/src/provider/core.rs:1553`). N117-1 and N117-2 replace the entire admitted call, removing the probe that publishes loss (`plan/maintenance/tools/mutations.json:28940`). Thus the tests fail because loss was never injected—or because `start` succeeds—not because they observed a call *after* loss. Move the injection point outside the expression being mutated, so the same loss occurs with or without admission. The re-anchored N108-159 has the same probe-removal problem (`plan/maintenance/tools/mutations.json:28930`, `crates/ezsdr-radio-uhd/src/provider/rx.rs:2494`).

3. **Minor — the later-stream test does not exercise an already-running owner.** Its only stream is stream 0 (`crates/ezsdr-radio-uhd/src/provider/mod.rs:1547`), so loss occurs before any RX owner is spawned. It does not verify that an earlier owner exits and is joined on the new `None` path (`crates/ezsdr-radio-uhd/src/provider/mod.rs:720`). Exercise a second stream after the first owner starts and observe cleanup.

The other `None` outcomes match their preceding loss checks: segment start finishes after dropping the chain lock, and provider start returns the same UR-15 refusal (`crates/ezsdr-radio-uhd/src/provider/rx.rs:420`, `crates/ezsdr-radio-uhd/src/provider/mod.rs:666`). Device errors still follow their separate refusal or loss-publication paths.

I found no new reverse acquisition of the RX-chain lock under `device_gate`. The gate’s holders are: loss publication, which takes the streams lock and then records/emits loss; rate checking, which calls the device and releases the gate before taking the rates lock; streamer open, which calls the device and may clear the TX report stash; close, which calls the device; and `admitted`, which calls the device (`crates/ezsdr-radio-uhd/src/provider/core.rs:1076`, `crates/ezsdr-radio-uhd/src/provider/core.rs:1140`, `crates/ezsdr-radio-uhd/src/provider/core.rs:1490`, `crates/ezsdr-radio-uhd/src/provider/core.rs:1538`, `crates/ezsdr-radio-uhd/src/provider/core.rs:1553`). RX open/close check the chain **before** taking the gate. The new segment probe runs while the chain lock is held, but before the gate; its loss callback does not take that chain lock.

P1290 still removes the loss check and admission; P440 still restricts starts to stream 0. N117-1, N117-2 and N108-159 target the stated calls, subject to the test problem above.

No builds, tests, or formatting were run. `gh issue view 117` could not connect to GitHub; the review used the supplied issue purpose and local code.
## Opus re-check reviewer, review (summary as received)

**Verdict: FAIL.** One major at site 2; sites 1 and 3, lock order and the rows are sound.

- **Major M1 — site 2's `None` panics uhd-rx.** `rx.rs:467-472`: on `rx_stop_admitted` → `None`, `stop_result` is skipped and `chain.finish()` runs on a chain `begin_result` left `Armed` (`core.rs:165`); `RxChain::finish` asserts `Off | Uncertain | Lost` (`core.rs:230`). The ordinary stop path calls `chain.lose()` first (`rx.rs:324-326`). Sequence: begin (`Some(Ok)`, chain Armed) → `declare_sample_clock` Err → another thread `device_lost` before the stop's gate (it never takes `rx_chain`) → `None` → `finish()` panics while holding `rx_chain[k]`. Reachability in production probably nil (INFERRED: ratio term > 2^31 or clock-id exhaustion, `ezsdr-kernel/src/time/domain.rs:293-305`, `:205-209`); still major as the change adds a production panic in the branch it touched. Fix: `chain.lose()` in the `None` arm.
- **Site 2's missing test:** probably reachable via `plan`/`rx_with_plan` (`rx.rs:877-903`) with a ratio term above `RATIO_TERM_CAP` (INFERRED); it would fail first on M1.
- **Lock order:** no cycle. Sites 1 and 2 take `rx_chain[k]` then `device_gate` (as `rx.rs:305`, `:310`, `:582`); site 3 holds no lock. `device_gate` holders (`core.rs:1078`, `:1140`, `:1490`, `:1542`, `:1555`) take no `rx_chain` inside.
- **Sites 1 and 3:** `None` matches the loss check just above (`rx.rs:441-445`; `mod.rs:676-688`, same UR-15 refusal).
- **Tests:** FakeDevice's `mark_lost` records but does not set `gone` (`device.rs:2212`), so "no rx_start" is a real assertion; on the old code and N117-1/2 the probe disappears with the gate and the tests fail on an earlier assertion. N108-136 keeps the probe and drops the re-check.
- **Minor m1:** site-1 test: `rx.stream.is_none()` already holds before `begin`; assert `rx_chain[0].device() == RxDevice::Lost` (as `rx.rs:2501`).
- **Minor m2:** `design/18-uhd-radio.md:233` (UR-17) and `:374` (UR-29) say three admitted receive calls; now six; new tests not in the Test lines; same wording at `core.rs:1559` and N108-135's name.
- **Note n1:** the site-3 test has only stream 0; UR-15's other half (begun owners joined, no call after the loss) not observed.
- **Rows:** N117-1/2 killed by the new tests; N108-159, P1290 re-anchored to the same program; P440 same decision; no row left on the removed direct calls. Uniqueness not checked.

## GPT-6.1 Terra, re-check of the fix round

**Verdict: FAIL.** The site-2 panic is fixed, but one mutation test still fails before checking the post-loss device call.

1. **Major — N117-2 is killed by the wrong assertion.** Its replacement preserves the loss probe but calls `rx_start` anyway (`plan/maintenance/tools/mutations.json:28954`). With the fake, that call succeeds, so `start()` can return `Ok`; the test then fails at `unwrap_err()` before inspecting the calls (`crates/ezsdr-radio-uhd/src/provider/mod.rs:1573`). Check the post-loss calls before requiring the refusal.

2. **Minor — the two-stream test does not prove the owner ran or was joined.** Stream 0’s owner is spawned before the stream-1 probe (`crates/ezsdr-radio-uhd/src/provider/mod.rs:691`), but there is no synchronization showing it ran before loss, nor an assertion distinguishing a successful join from a detached owner. The call log does check for calls after loss (`crates/ezsdr-radio-uhd/src/provider/mod.rs:1582`). Add an owner-entry and completion observation.

3. **Minor — the new admission rule is duplicated.** UR-17 and UR-29 both enumerate the same six calls and gate requirement (`design/18-uhd-radio.md:233`, `design/18-uhd-radio.md:374`). AGENTS §2 requires each topic once; retain the rule in one place and cite it from the other.

The site-2 `None` path now marks the chain `Lost` before `finish()` and then books the refusal (`crates/ezsdr-radio-uhd/src/provider/rx.rs:465`). Its clock exhaustion is deterministic: registering `MAX - 1` advances the allocator to its exhaustion sentinel, and the rig creates a fresh registry per test (`crates/ezsdr-kernel/src/time/domain.rs:249`, `crates/ezsdr-radio-uhd/src/provider/test_support.rs:107`).

All four mutation replacements retain their probes. By code inspection, the expected first failures for the other three are:

- **N117-1:** no `rx_start` assertion (`crates/ezsdr-radio-uhd/src/provider/rx.rs:4398`).
- **N117-3:** no `rx_stop` after loss assertion (`crates/ezsdr-radio-uhd/src/provider/rx.rs:4426`).
- **N108-159:** restart call count assertion (`crates/ezsdr-radio-uhd/src/provider/rx.rs:2501`).

The added spec test-table entries describe the assertions present, subject to the owner-observation gap above. No builds, tests, formatting, or edits were performed; mutation outcomes are inferred from the code, not run logs.