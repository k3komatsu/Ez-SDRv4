# Review M — re-review of the fixes to Review L (`git diff 89f02b7 e2d4d26` on `worktree-phase7-impl`)

Reviewer: Claude Opus, resuming its Review L session, 2026-09-28, read-only on the repository; every probe and build in scratch copies under `/Users/komatsu/.claude/jobs/20ef8e0f/tmp/review-m/` (`copy/`, `mut/`, `mut-own/`, `probe-free/`), fresh timestamps, `find … -exec touch {} +` before each build session, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` (`…-review-185` for 1.85.0). Brief: [`../prompts/review-m.txt`](../prompts/review-m.txt). The report as returned; the triage is in [`../design-notes.md`](../design-notes.md) §8.

## VERDICT

**CHANGES_REQUIRED** — 0 P0, 2 P1, 8 P2 (new). Every Review L P0 is closed with evidence (a probe against libuhd 4.10 shows the new `uhd_api_a_streamer_is_freed_once` really aborts on a double free; all 512 rates pass; the from-zero receive honours its instant), and every R01…R16 mutation and G36 is killed. Two P1s remain: the new `RxCmd::Cut` path issues a timed receive stop that is usually already late when the device gets it, and uhd-rx treats any `LATE_COMMAND` as a missed start and restarts the stream (the late-stop behaviour on hardware is INFERRED, but the restart logic is plain in the code); and `ur_21_late_policies_on_the_device_lead` now fails 2 in 7 runs under the 1.85.0 build, so `cargo +1.85.0 test --workspace` is red about a third of the time. Both fixes are small.

## BLOCKERS

### P1-A (new) — A late timed stop from `RxCmd::Cut` or `stop_orderly` makes uhd-rx restart the stream (INFERRED on hardware)
- **Location:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:175` (`StopMode::Orderly => self.stop_at(at, at + tail)`), `stop_at` → `rx_stop(Some(cut))`; `rx.rs:143-151` (`self.release_stop(); let result = rx_recv(...); self.poll(); self.receive(result);`); `rx.rs:269-283` (`RxRecv::LateCommand` branch).
- **Evidence:** `Provider::stop` sends `Cut { at: core.now(), mode }` (`provider/mod.rs:439-442`); uhd-rx handles it only after its current `rx_recv` returns — one whole block (2 ms for the default 2 000 at 1 Msps; 20 ms in `ur_26_an_abort_publishes_nothing_after_the_stop_instant`; up to 168 ms at the largest selector `block_len`). The timed stop at `at + 1 ms` is therefore handed to the device at or past that instant — well inside the 2 ms device lead the profile itself declares (§3). UHD's documented rule for a late timed command ("If the time spec is late, the command will be activated upon arrival", spec 18 §2) is for `set_command_time`; for a stream command a late time usually comes back as receive metadata `LATE_COMMAND` (INFERRED; the fake models neither). On `LateCommand` uhd-rx does not look at `stream.cut`: it emits `radio.LATE_COMMAND` and issues `rx_start(now + 50 ms)` — a stream being stopped is started again, and the restarted stream keeps flowing after uhd-rx exits, until `close_streams`. `stop_orderly` (`RxCmd::Stop`, a `Stop` for `<id>/rx`) has the same 1 ms lead (pre-existing).
- **Fix:** in the `LateCommand` branch, when `stream.cut.is_some()`, do not restart; issue `rx_stop(None)` (the cut already discards past it). In `stop_at`, issue no timed stop when `cut − now` is less than the device lead; rely on the untimed fallback in `samples` (which already stops the stream once a sample past the cut arrives). Make `FakeDevice` report `LateCommand` for a timed `rx_stop` in its past, as it does for a start, and add a test: an orderly `finish` with `block_len` 20 000 — no `rx_start` in `device.calls()` after the stop, no `radio.LATE_COMMAND`.

### P1-B (new) — `ur_21_late_policies_on_the_device_lead` is flaky (2 failures in 7 runs of the fake binary on 1.85.0)
- **Location:** `crates/ezsdr-radio-uhd/tests/fake.rs:864-877` (third burst `send(&mut run, "send", Some(ms(4)), …)` expected on time).
- **Evidence:** first `cargo +1.85.0 test --workspace` run: `ur_21_late_policies_on_the_device_lead … FAILED`, `left: [Drop, SendAsap, LateAtDevice]`, `right: [Drop, SendAsap]`; then 5 runs of `cargo +1.85.0 test -q -p ezsdr-radio-uhd --test fake`: one more failure, same assertion; the `--no-fail-fast` workspace rerun passed (836/836). Stable: 0 failures in 11 runs. A 4 ms lead leaves 1 ms margin over the 3 ms "on time after its delivery" budget; the fake binary grew from 78 to 87 wall-clock tests (~23 s → ~33 s), and under that parallel load uhd-tx sometimes hands the burst to the fake after its start, which the fake reports as `TimeError` → `late_at_device`. Same class as Review L P1-8, another test.
- **Fix:** give the on-time case a margin that survives parallel load (e.g. 20 ms), keeping the 1 ms and 0.5 ms cases for the late policies; or run the wall-clock-heavy `fake.rs` cases serially behind one shared mutex.

## NONBLOCKING

1. `RxCmd::Switch` after a Cut moves the cut later: `rx.rs` sets `stream.cut = Some(e1)` without taking the minimum, and uhd-control can still book a cold change between `Provider::stop` sending `Cut` and joining uhd-control (up to 1 s later), so the Cut's `at + tail` (or `at` under abort) is replaced by a later `e₁` — the tail counted from shutdown again (the N-9 case). Same overwrite after a `Stop` for `<id>/rx`. Fix: `stream.cut = Some(stream.cut.map_or(e1, |c| c.min(e1)))`, and ignore `Switch` once a Cut arrived.
2. The lock held across `schedule` (`paced.rs:190-194`) can stall `FreezeDispatch` (`ending.rs:486`): the data thread holds `wake_handle` inside `Authority::schedule`; if a paced Authority's `schedule` waits for a `next_wakeup` on the control thread, and the control thread is in `FreezeDispatch` waiting for `wake_handle`, neither proceeds until the Authority's own bound (the new `firing_before_schedule_returns` double: 20 ms). `DeviceAuthority` never waits, so this is a ceiling, not a deadlock — but the fix also narrows N-1 rather than closing it (a wake scheduled after `FreezeDispatch` passed stays scheduled, a harmless no-op), and my mutation M05 (revert N-1) survives. Say in KC-46a that a paced `schedule` must not wait for `next_wakeup`, or drop the lock (N-3's reasoning — the leftover is harmless — applies equally).
3. With the untimed fallback and no reopen (UR-25), old-rate samples still in flight after `rx_stop(None)` are read by the new stream; their `first_tick` precedes `e₂`, so most are dropped as `rx_overlapping` (a mislabel), and a block spanning > ~45 ms (a large selector `block_len` at a low rate) would be trimmed and published as new-clock samples at index 0 with no flag. Fix: in `samples`, drop a block whose `first_tick` precedes the stream's `start` without counting it as overlap.
4. Enabling a stream while the old one drains (1 → 0 → 1 channels within the restart lead; pre-existing, surfaced by the new `poll` after receive): uhd-control's `enable` calls `rx_open` while uhd-rx is still streaming the old clock to `e₁`, and uhd-rx's `Enable` replaces the stream while the `Switch` to 0 is pending. Book such an enable as a switch (`e₂` after `e₁`), or refuse it (`COMMAND_REJECTED`) until the drain ends.
5. Some calls still read UHD's global error text on objects other threads touch (`tx_send`'s `uhd_tx_metadata_make`; `ref_locked`'s string-vector and sensor-value calls): their text may be another thread's. Minor.
6. Test gap: the orderly half of N-9 is untested — M02 (tail counted from uhd-rx's processing instead of the stop instant) survives; R15/R16 test abort only. Assert `end ≤ stop + 1 ms + one sample` in `ur_26_orderly_stop_delivers_the_tail_abort_does_not`.
7. Test gap: the per-channel depth (N-6) is untested — M03 (count one per update again) survives. Add a two-channel case to `ur_24_hardware_timed_updates` (`COMMAND_QUEUE_FULL` at the 9th update).
8. Test gap: N-7's `Inexact` host instant is untested — M04 survives. No caller schedules a `host.monotonic` instant on a paced Authority (checked: `advance_to`, the agenda, the wake, the Providers), so it is harmless; a unit test is optional.

## TEST_GAPS

**Mutation list** (`mutate.py`, scratch `mut/`): R01…R16 all killed; G36 killed (by `kg_02_a_wake_that_fires_before_schedule_returns_still_clears`); G09 SURVIVED in this run (killed in the implementer's last) — a race the tests sometimes win, recorded as not killable by a black-box test, as agreed.

**New tests kill what they claim.** `uhd_api_a_streamer_is_freed_once`: probe `probe-free/` makes an unattached rx streamer and frees it — once returns 0 and exits normally; freeing the streamer twice kills the process with exit 134 (SIGABRT); freeing the metadata twice gives exit 133 (SIGTRAP) — so the test's lifecycle path detects a copy-then-drop regression inside `RxStream::make`/`TxStream::make`. `ur_12_every_advertised_rate_is_accepted` (R11 killed); `ur_25_rx_channels_from_zero_starts_at_its_instant` (R04); `ur_25_a_timed_receive_stop_is_released_a_restart_lead_ahead` (R12; my M01 — release only at `e₁` — killed too); `ur_25_a_device_that_ignores_the_timed_stop_is_stopped_at_e1` (R13, R14); `ur_26_an_abort_publishes_nothing_after_the_stop_instant` (R08, R15, R16); `kg_04_every_action_of_one_call_is_finished` (R01).

**My own new mutations** (`own.json`, scratch `mut-own/`, each against its whole test binary): M01 held receive stop released only at `e₁` — killed; M02 orderly Cut tail counted from processing — SURVIVED (NONBLOCKING 6); M03 UR-24 released count per update — SURVIVED (NONBLOCKING 7); M04 `host.monotonic` floored again — SURVIVED (NONBLOCKING 8); M05 N-1 reverted — SURVIVED (NONBLOCKING 2).

**Review L's L01…L14:** L01, L05, L07–L14 are now R01…R10, all killed; L02 already killed; L03 equivalent (KC-46b says so); L06 recorded near-equivalent; L04 still open (KC-24a's Module-submission clause has no test; accepted by the design since no Phase 7 Module submits).

**Flaky:** `ur_21_late_policies_on_the_device_lead` 2/7 under 1.85.0, 0/11 stable (P1-B). `rehearsal_b7_session_loopback`: no failure in 18 runs of the fake binary (Review L P1-8 closed).

**Runs** (scratch `copy/`):

| command | result |
|---|---|
| `cargo +stable test --workspace` | 836 passed, 0 failed |
| `cargo +1.85.0 test --workspace` | first run 750 passed, 1 failed (P1-B; cargo stopped at that binary); `--no-fail-fast` rerun 836 passed, 0 failed |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo +stable test -p ezsdr-radio-uhd --features uhd` | 90 passed, 10 ignored |
| `cargo +stable clippy -p ezsdr-radio-uhd -p ezsdr-server --features ezsdr-radio-uhd/uhd,ezsdr-server/uhd --all-targets -- -D warnings` | clean |
| Python suite, 3.13 and 3.9 | Ran 23, OK each |
| `check_links.py` | `ok: 508 links` |
| `fake` repeated | stable 10 × 87/87; 1.85.0 5 runs, 4 × 87/87, 1 × 86/87 |
| `device_paced` repeated | 5 × 41/41 |
| `protocol` repeated | 5 × 32/32 |

## PREVIOUS_FINDINGS

- **P0-1 CLOSED** — `RxStream::make`/`TxStream::make` own the handles, moved into the `Arc` (`uhd.rs`); the probe confirms the new test detects a double free.
- **P0-2 CLOSED** — `decimation` is MockRadio's rule; all 512 rates, 200e6/3 through `prepare`, 200e6/7 through a cold switch; R11 killed.
- **P0-3 CLOSED** — `lattice(e.max(now + 50 ms), n)`; UR-25 amended; R04 killed.
- **P1-1 CLOSED** — held, released a restart lead ahead, recorded in `applied`; R12 and M01 killed; the spec says what the code does.
- **P1-2 CLOSED** for handle calls (`uhd_usrp_last_error` under the control mutex; the streamers' own); residual NONBLOCKING 5.
- **P1-3 CLOSED** — fallback, reopen only on a count change, old streamer released first; R13, R14 killed; residuals NONBLOCKING 3 and 4.
- **P1-4 CLOSED** — `end_open(true)` always; `unended_bursts` asserted; R06 killed.
- **P1-5 CLOSED** — R01 killed.
- **P1-6 CLOSED** — G36 killed.
- **P1-7 CLOSED** except L04 (OPEN, a recorded ceiling).
- **P1-8 CLOSED** (P1-B is a new flaky test of the same kind).
- **NONBLOCKING 1 CLOSED** (residual: NONBLOCKING 2). **2 OPEN** (a ceiling, accepted). **3 CLOSED** (text). **4 PARTLY** (`clear_command_time` closed; getters a ceiling). **5 CLOSED.** **6 CLOSED** (untested: NONBLOCKING 7). **7 CLOSED** (untested: NONBLOCKING 8). **8 CLOSED.** **9 CLOSED** for abort (orderly untested: NONBLOCKING 6; a race: NONBLOCKING 1). **10 CLOSED.** **11 OPEN** (a ceiling). **12 OPEN** (a ceiling). **13 recorded.** **14–18 CLOSED.** **19 recorded.**
- **DEVIATIONS 1–15 CLOSED** — the spec text now says what the code does for each (KG-8's code line, the `mr_32` note in VE-4, the dev-dependencies in overview §5, UR-3's brackets, UR-7's counters, UR-24's stream commands, UR-32 deferred by name, UR-21's refusal list, UR-34's first block, EA-7's `Config::new`, NX-4); 2, 11, 12 and 14 were already acceptable.
- **TEST_GAPS from Review L:** G36 CLOSED; L01, L05, L07–L14 CLOSED (as R01…R10); G09 recorded (race); L03 recorded (equivalent); L06 recorded (near-equivalent); L04 OPEN (accepted).
