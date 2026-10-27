# Three flaky fake tests: why they fail, and the smallest fixes

- Read-only investigation, 2026-10-27, against HEAD `660213b` (line numbers are HEAD's; `git show HEAD:<path>`).
  The working tree's uncommitted D9a edits were not analysed.
- Paths: `fake.rs` = `crates/ezsdr-radio-uhd/tests/fake.rs`; `tx.rs`, `control.rs`, `mod.rs` =
  `crates/ezsdr-radio-uhd/src/provider/…`; `device.rs`, `profile.rs` = `crates/ezsdr-radio-uhd/src/…`;
  `pipeline.rs` = `crates/ezsdr-kernel/src/coordinator/pipeline.rs`.
- Logs named without a directory (`repc-17.log`, `report-3c3a.md`, `mutation-3c2c-full.log`) are in this
  report's directory.
- Reproductions ran in a copy outside the repository (`git archive HEAD`, three env-gated patches, deleted
  afterwards), built in `~/.cache/cargo-target/Ez-SDRv4-review`, one test at a time, `--test-threads=1`.
- All three are **test bugs** of one family: the verdict depends on when the **test thread** runs. That breaks
  design note 37's rule (tests/README.md: "No test's verdict may depend on when the test thread runs").
  None is a production timing bug. One side finding in production code is in §4.

| # | Test (HEAD) | Failing assertion | Cause | Fix | Rows |
|---|---|---|---|---|---|
| 1 | `ur_25_an_enable_from_no_stream_configures_once` (fake.rs:4516) | `:4528` `time_errors(..).is_empty()` | The test thread is descheduled between the Kernel's `now` read and the dispatch. An untimed burst has 3 ms of slack. | Delete `:4528` (rule 2) | P368 (maintenance) |
| 2 | `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time` (fake.rs:2574) | `:2581` same | Same as 1 | Delete `:2581`, rename, spec 18 row | P254 (maintenance) |
| 3 | `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took` (fake.rs:3721) | `:3743` or `:3747` | Since `7043a29` (note 40 D3) the fixed `ShortSend`/`FailSend` index lands on a buffer that depends on uhd-tx's 200 µs poll phase. The test thread sets that phase at the Run's start. | Move to a controlled-clock unit test in `tx.rs` (rule 4) | B18, B19, B25 (phase7) |

## 1. Background: why only the test thread can spend a deadline's slack

The manual clock stops while any participant (`uhd-rx`, `uhd-tx`, `uhd-control`, `uhd-clock`) is busy. It
runs at real speed while all of them are idle (plan 30 M3, M5). The test thread and the Kernel's code that it
runs are not participants (M6). So, on the fake, a Provider thread's delay never spends device time, but a
descheduled test thread does whenever the Provider threads are idle waiting. VERIFIED (plan 30 §1, and the
reproductions below).

## 2. Tests 1 and 2: an untimed `start_repeat` stamped on the test thread

**Path (VERIFIED by reading).** `send(&mut run, "start_repeat", None, ..)` (fake.rs:2207) calls
`RunHandle::submit`. It runs **on the test thread**:

- `pipeline.rs:1352` reads `let now = run.shared.now()`.
- `:1380` → `session_earliest` (`:1685-1699`) sets `earliest = now + min_command_lead`, which is 5 ms
  (`profile.rs:11,13,22`: `DEVICE_LEAD_NS` 2 ms + `DELIVERY_ALLOWANCE_NS` 3 ms).
- compile and admission follow.
- `:1498` `admission::dispatch` pushes onto uhd-control's queue, and the thread then waits at `:1502`
  (`wait_finished`, KC-21a).

uhd-control polls every 1 ms (`control.rs:37,321,332`). `book_burst` decides against its own `now` plus
the 2 ms device lead (`control.rs:1120-1128`), and a shortfall gives `SendAsap` plus a `TIME_ERROR {late,
send_asap}` (`:1151-1159`). So the burst is late whenever more than about 3 ms of device time passes between
`pipeline.rs:1352` and uhd-control's read. uhd-control's poll can use up to 1 ms of that. Its busy time
cannot spend any (the clock stops). Only the test thread's own descheduling between `:1352` and `:1498` can
spend the rest.

**Why it was missed (VERIFIED).** 37c's sweep put these tests in the safe class S on an INFERRED reason
(`plan/phase8/reviews/37c-table.md:193`). The reason given was that "the Kernel stamps the untimed booking's
instant, not a test-thread read". The Kernel does stamp it, but **inside `submit`, on the test thread**.

**Reproduction (VERIFIED).** A `std::thread::sleep` inside `submit`, between `:1352` and the dispatch
(untimed Vocabulary actions only), no other change:

| injected delay | test 1 | test 2 |
|---|---|---|
| none | pass | pass |
| 1.0 ms (1 run each) | pass | pass |
| 2.0 / 2.5 / 3.0 / 3.5 ms (3 runs each) | pass | — |
| 4.0 ms (3 runs) | FAIL ×3, `SendAsap`, `late_by_ns` 362 000–600 000 | — |
| 6.0 ms | FAIL, `SendAsap`, `late_by_ns` 1 585 000 | FAIL, `SendAsap`, `late_by_ns` 1 600 000 |

The failure shape matches the logged ones:
- test 2: `repc-17.log:148-151`, `TimeErrorPayload { Late, SendAsap, late_by_ns: 30000, ticks: 7170 }`;
- test 1: "burst 1 ms late" (`report-3c3a.md:32`), which needs a test-thread stall of about 4.5–5 ms in that
  window. That is plausible in `cargo test --workspace`: 16 fake tests in parallel, each with 4–5 uhd threads.
  INFERRED (only the summary line was kept).

**Test or production?** A test bug. On hardware, a caller (the server thread) stalled for more than 3 ms
gets the honest `send_asap` flag that UR-21 designs for. On the fake, nothing in the Module spends more than
its 1 ms poll, which is what the 3 ms allowance is for.

**Fix (rule 2), VERIFIED in the copy.**
- Delete fake.rs:4528 and :2581. Everything left holds whatever the clock did:
  - a `send_asap_and_flag` burst is moved, never dropped;
  - `tx_open` and `apply` counts, `configured < sent`, and `!bursts.is_empty()` do not depend on the time.
- With the 6 ms delay both edited tests pass.
- With the rows' mutants applied in the copy, both are still killed:
  - **P368** by the `apply tx 0 rate=1000000` count, fake.rs:4527;
  - **P254** by the missing `sob=true` send, the `unwrap` at :2584.
- Test 2's name and its spec 18 oracle then claim more than it checks. Rename it to
  `ur_25_tx_channels_from_zero_transmits_the_next_burst` and update:
  - P254's `filter` (`plan/maintenance/tools/mutations.json`);
  - `design/18-uhd-radio.md:289` (*Test:* line);
  - `:498` (drop "no `TIME_ERROR`" from the row; K5's property, "admitted every time, configured before
    sent", stays).
- Test 1 is not cited in spec 18.

**Rejected.**
- A test-side clock `hold()` around `submit` (plan 30 stage B), already rejected by note 37 §4:
  - it needs `HostClock::spawn` exported;
  - `submit` waits for uhd-control (KC-21a), so it only completes if stage B's "polling waits end in real
    time" rule is built (plan 30 §5).
- Booking further ahead: it only lowers the chance (note 37 §4).
- A bigger allowance in the fake profile: tests/README forbids relaxing the envelope.
- Reading `now` later in the Kernel: the window shrinks but stays.

## 3. Test 3: a fixed send index against a poll phase the test thread sets

**Which ur_22 test.** The only kept record of a `ur_22` failure that is not an in-progress implementation
log is mutation row **B18 "BASELINE FAILED"** in the 3c2c gate (`mutation-3c2c-full.log:1596`;
`plan/phase8/reviews/3c2c-opus.md:45`: "load average 30–37 … the original test passed 3 times locally").
B18's filter is `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took`. Its output was not kept,
and `mutate.py:124-128` reports a build failure the same way, so the attribution is INFERRED; the mechanism
below is VERIFIED. (The Phase 7 bench.md note that a `ur_22` test fails under amd64 emulation predates the
manual clock and does not apply.) The other `ur_22` tests are Spec-booked and have no send-index or
send-size oracle.

**Path (VERIFIED by reading).**
- A Spec Run books the burst at T0+40 ms (rule 1, so the booking itself is safe).
- uhd-tx wakes every `IDLE` = 200 µs (`tx.rs:107,153`). Since `7043a29` (note 40 D3), each wake hands over
  only up to the 10 ms in-flight window: `count.min(window_cut - open.next)` (`tx.rs:301-302,414`). So sends
  are 200 samples at 1 Msps, except the first.
- The first send's size `s0` ∈ [1, 200] is the phase of uhd-tx's 200 µs grid against `start − 10 ms`.
- The test thread sets that phase. `Provider::start` registers uhd-tx (`provider/mod.rs:389-396,699`) on
  the test thread after the Kernel chose T0, and device time runs at real speed in between (§1). The phase
  is that real time, modulo 200 µs.
- The faults are fixed call indices. The fake counts every `tx_send` that carries a sample, including a
  padded empty EOB, and a cut takes `n/2` (`device.rs:1533-1556`).

Three phases change the scene:

| `s0` | case | what happens | fails at |
|---|---|---|---|
| 1 | `ShortSend(0)`, 1 000 | Half of 1 sample is 0, so the send stalls and no device burst begins. `abandon(false, 0)` correctly sends no EOB. | `:3747` "after 0" |
| 1 | `FailSend(0)`, 1 000 | Nothing is taken, then an error. `abandon(true, 1)` sends an empty EOB, whose zero sample lands at the device's *now*, not at the burst's start. | `:3747` |
| 199, 200 | `ShortSend(30)`, 6 000 | At 199 the final data buffer is 1 sample, wholly held, so it makes no device call. At 200 it is the 30th call. Either way the 31st call is the **held tail's EOB**, which stalls. `close_device_burst` ignores `Ok(0)` (§4), so nothing is rejected. | `:3743` "ShortSend(30): []" |

**Reproduction (VERIFIED).** A copy-only patch makes uhd-tx's first hand-over wake land at a chosen phase.
Each case runs alone:

| forced `s0` | `ShortSend(0)` | `ShortSend(1)` | `ShortSend(30)` | `FailSend(0)` |
|---|---|---|---|---|
| 1 | **FAIL** `:3747` | pass | pass | **FAIL** `:3747` |
| 100 | pass | pass | pass | pass |
| 198 | pass | pass | pass | pass |
| 199 | pass | pass | **FAIL** `:3743` (31 calls, last `n=0 stalled eob=true`) | pass |
| 200 | pass | pass | **FAIL** `:3743` | pass |

**How often (measured on the unpatched code).**
- 10 full runs at host load about 10 (other jobs) spread the first send over 8–198 samples, so the phase is
  close to uniform. The three affected cases are independent Runs, and 1, 2 and 1 of their 200 phases fail:
  about **2 % per test run** (INFERRED from the spread).
- 40 runs of the three affected cases on the quiet host (load 1–4): 40/40 pass. The first send stayed within
  52–79 samples in every case, far from 1, 199 and 200 (VERIFIED).
- So the phase is stable when the test thread is not preempted, and spreads under load. That matches
  "BASELINE FAILED at load 30–37, passed locally".

**Test or production?** A test bug that `7043a29` introduced, together with the `ShortSend(30), 6_000` case
added to fit the then-new 200-sample cadence. Before it, the case list had three cases and the first send was
the whole chunk, so 37c's S for this test (`37c-table.md:194`) was right when it was written.

**Fix (rule 4, the 37a precedent).** Move the test into `src/provider/tx.rs` as a controlled-clock unit test
beside `ur_22_a_first_send_cut_short_keeps_the_reports_paired` (`tx.rs:4709`, moved by 37a for the same
reason).
- Use the same rig: `rig_with` (Manual) + `Frozen::of` + `device_at`, `Tx::new`, `tx.command(TxCmd::Burst(held(..)))`.
- Call `tx.step()` at fixed instants, so that every case's cut lands on its intended buffer:
  - the first buffer, with ≥ 2 samples;
  - the second buffer;
  - the final buffer whose last sample is held;
  - `FailSend` on the first buffer.
- Keep the same five assertions on `device.transmitted`, `rejected` and the EOB count. Delete the fake test.
- Keep the name, so that B18, B19 and B25 only change `"test_bin": "fake"` → `"lib"`.
- Add "(unit)" to spec 18's row (`design/18-uhd-radio.md:511`).
- This is a test move, not a risky step by §9 (as note 37 §3 judged 37a).

**Rejected.**
- Re-tuning indices or lengths inside fake.rs. No fixed index hits a cut final buffer for every phase: as
  `s0` crosses `(L−1) mod 200`, the buffer count shifts by one, and at one phase the final buffer is wholly
  held.
- A new device-time-keyed `FakeFault`: more mechanism, and at `s0 = 199` B18's scene (a held tail after a
  short send) still cannot occur.
- Moving the burst or retrying: these only lower the chance.

## 4. Side finding (production, INFERRED, minor; not the flake's cause)

`close_device_burst` (`tx.rs:572-582`) checks only `Err`. When the **held tail** (1 sample + EOB) comes back
`Ok(0)` (UHD's send timing out after 1 s), nothing is recorded and the device burst stays open. The
continuation path treats the same tail's `Ok(≠1)` as "UR-22: a send did not complete" and abandons
(`tx.rs:442-453`).

For an *empty* EOB, `Ok(0)` is success, so a fix would check only `Some(tail)`. Reached in tests only by
case 3's unlucky phases.

If adopted, it is `failure-collapsed-into-value` or `radio-rule-implemented-twice` (two paths in one
Provider). Both mechanisms are past three strikes, so AGENTS.md §6's design question comes before a patch.
The orchestrator decides.

## 5. Same class, not yet seen failing (37c-table §4 rows on the same reasoning)

| Test (HEAD) | Exposure | Suggested handling |
|---|---|---|
| `ur_25_tx_channels_to_zero_ends_the_clock_and_a_re_enable_starts_a_new_one` (fake.rs:4475, assert `:4511`) | 2 untimed `start_repeat`, 3 ms slack each | delete `:4511`, as for test 1 |
| `ur_21_an_untimed_send_is_on_time_after_its_delivery` (fake.rs:2340, `:2348`) | 20 untimed `send`s (`drop_and_flag`); a stall drops one, so `bursts == 20` fails too | on-time is its property: rule 4 (booking at receipt + poll on a held clock in `control.rs`), unless an existing unit test already pins it (not checked) |
| `ur_21_an_untimed_burst_after_a_cold_change_is_on_time_at_its_clock_s_origin` (fake.rs:2353, `:2362`) | slack ≈ 48 ms (RS-19 origin bound); its own comment concedes "#60's class" | rule 2 for the fake (target ≥ origin); rule 4 if the on-time part is wanted |
| `ezsdr-server/tests/protocol.rs` loopback (`:340-370`, assert `:370`) | untimed repeat right after the enable, slack ≈ 48 ms | rule 2 (drop the no-`TIME_ERROR` assert) |

`ur_25_a_rate_change_admits_a_burst_on_the_new_clock`, `ur_26_stop_actions` and
`ur_28_the_device_reports_a_late_burst` are already delay-proof. The `time_errors` asserts at fake.rs:2452,
:3593, :3717 and :6069 are on Spec-booked bursts with no send-index oracle.

Suggested one-line addition to tests/README.md's rule: an untimed Session Action is stamped by `submit` on the
calling thread, and a fault keyed by `tx_send` index depends on uhd-tx's poll phase.

## 6. Mutation-row impact

| Row | Now | After the fix |
|---|---|---|
| P368, P254 | a stall can fail the baseline or fake-kill a mutant | still killed (debug run in the copy); P254's `filter` follows the rename |
| B18, B19, B25 | B18 baseline failure seen. At the failing phases a mutant can be "killed" by the phase, and B19/B25's scene (a first send taking ≥ 1 sample) is absent at `s0 = 1` | `test_bin: lib`; rerun the three rows through `remote_mutate.py` |

Until they are fixed, a kill or a BASELINE FAILED on these five rows under load is not evidence.

**Counting (for the orchestrator).** Tests 1 and 2 are a missed item of note 37's sweep. Test 3 is a new
instance introduced after note 37 (by `7043a29`). Both are the `wall-clock-test-oracle` family that note 37
answered. Note 37 keeps the design, so this is not a new design question unless the owner counts them as
strikes.

## 7. Not checked

- B18's 3c2c output is lost, so "test, not build" is INFERRED.
- Test 1's 10-27 failure kept only a summary line.
- The rows were debug-run only in the copy; the official rerun belongs to `remote_mutate.py`.
- Case 3's "≈2 %" assumes a uniform phase under load.
- The §5 rows were read, not run.
- The working tree's D9a edits (device.rs, control.rs, fake.rs) were not checked against these tests.
- The 40-run batch used the binary last built for the P254 debug run (mutant reverted in source, not rebuilt).
  P254 changes only the owner-with-no-clock branch of `retarget`; test 3's TX clock exists from arm, so it is
  not on test 3's path.
