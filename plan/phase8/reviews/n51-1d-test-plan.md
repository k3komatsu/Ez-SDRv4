# Note 51 stage 1d — test plan (no code in the tree)

## Changes after Sol's review (2026-11-12; sol-plan.md PASS WITH CHANGES, plan-decisions.md; all adopted)

- **m1 (setup timeout is infrastructure).** Unit test 2 of §4.1 waits for `reached` with a 10 s `recv_timeout`. On `Timeout` it now writes `test_support::infra(..)` before it panics; `Disconnected` does not. N51D-02's kill stays the clock-equality `assert_eq!`; no `"kill": "bound"`.
- **m2 (guard pin vs the original regression pin).** `ur_28_…` gets an exact clock-equality assertion right after the deliberate 50 ms step and before `tx.pass()`: the clock must not have moved during the stall. N51D-03's actual observation is therefore that assertion (and, if it is removed, the later `device_at` equality, which also detects an unheld clock during the stepping). The failing-first evidence (§2.1: a 3 ms nap fails the test at `main`, the `.expect("the value is applied after the reopen")`) is with the **original re-freeze** and stays as it is.
- **m3 (wording).** Every "No dependence" in §2 reads "no dependence observed at these injection points and durations": delays inside the owner's operations were not exhaustively tried. The structural guard stays justified.
- **1d-C′ is deferred to stage 4** (the three helper-pattern sites of §1.3 have no `Frozen` hand-over; not a timing closure). Out of 1d: its four rows (P1298, P1302, P1306, P1312) are not re-run, and §6's C′ line and §4.3's re-run note about it do not apply.
- **One run for 1d-A, 1d-B and 1d-C**, one local commit; the per-sub-step self-review is waived (Sonnet implementer, owner 2026-11-03), Sol's code review remains. Note 51's counts are corrected in the text (§4.4: 17 sites, 4 N / 4 G / 5 W1 / 3 W2 / 1 H; `rig._frozen`; §11 W2 VERIFIED; "guard (#76 の形)" → the `drive()` hand-over helper `handed_over`).

Branch `work/n51-1d` from `main` `bfd59d3`. Author: the implementer (OpenCode Sonnet 5.5 #high). For Sol's test-plan review (AGENTS §9 step 3). Scope: crate-internal unit tests of `ezsdr-radio-uhd` (`src/provider/{tx.rs, test_support.rs, tx/tests/{e1,e2b}.rs}`) and the text 1d makes false. **No production code changes.** The main tree is untouched (`git status` clean).

**Evidence.** V = run on 2026-11-12 in a throw-away copy outside the repository, built **locally** into `~/.cache/cargo-target/Ez-SDRv4-review` with `CARGO_INCREMENTAL=0`; I = inferred, not run. Copies: `/tmp/opencode/n51-1d-probe` (= `main` + a diagnostic delay injection `nap()`, env `EZ_NAP_MS` / `EZ_NAP_ONLY=<k>`), `/tmp/opencode/n51-1d-sketch` (= `main` + the sketch of §2–§3; diff in `~/.cache/ezsdr-fixes/2026-11-12-n51-1d/probe/sketch-1d.diff`, 90 changed lines), `/tmp/opencode/n51-1d-mut-*` (one mutant each, made by `probe/mutprobe.sh`: a copy outside the repository; no `mutate.py`, no mutant in the main tree). The injection is diagnostic only (note 51 §5.4): not a gate, not a mutation filter. **Hazard met again** (AGENTS §7): every copy builds to one output file name in the shared review directory, so a build after another copy's build is silently skipped unless its files are touched; I ran `find <copy> -name '*.rs' -exec touch {} +` before every build session and checked for a `Compiling ezsdr-radio-uhd` line. One early 40-run loop that used a stale binary (the M6 mutant's) was thrown away and redone.

## 0. What I recommend (one line each)

1. A test-only helper `test_support::handed_over(frozen: Frozen, clock: &HostClock) -> host::Driver` = `let driver = clock.drive(); drop(frozen); driver`: the test thread becomes a participant **before** the held clock is released, so no instant of the hand-over lets the clock run, and the clock then runs only while the thread waits on it. It replaces the bare `drop(frozen)` at the 4 G sites and the 3 W2 sites.
2. **W2 rewrite works** (§3): the sleeping 150 ms participant and the `join` go; the guard fits. Keep it.
3. Pins: two unit tests of the helper (the one killer of each of its two rows) and, for the one site whose verdict does depend on the test thread today (`ur_28_…`), a deliberate 50 ms slow-thread step in the test itself. The other six guarded sites have **no observable dependence today** (§2), so their guard is by construction only: no mutation row, said plainly (§4).
4. Three more sites with the W2 shape but no `Frozen` (the same sleeping helper + `join`, found by search, §1.3) are an optional sub-step 1d-C′; I recommend taking them (same mechanical rewrite, tried, works) but they are the orchestrator's call.

**Why `drive()` and not `released` (the "#76 shape").** In code, #76's shape is `released(frozen, clock, work)`: the owner's work runs on a spawned participant and the test thread blocks on `recv`. The tables (`51-tables.md` §2.5 rows 5–7) name the guard `drive()`; note 51 §4.4 says "guard (#76 の形)". I took the *principle* (the thread that runs the owner's code is a participant, registered before the clock is released) and the table's API. Alternatives rejected:
- **`released` at each G/W2 site.** Works (E2b/E3b use it 13 times) but moves `tx` into a closure and back at each site, loses the test-thread's local hooks (`on_next_lock`, `LINE_HELD` are thread-local, `test_support.rs:202-208`), and the W2 sites would need `(tx, …)` returned from a closure just to call `shutdown()`. Larger diff, no better pin.
- **Inline `let _d = host.drive(); drop(frozen);` at 7 sites.** The order (drive first) is the whole point and a swapped copy would pass every site test (§4, row N51D-02); one helper pins it once.
- **Make `owner()`/`switch_rig*()` hand out a `Driver` instead of a `Frozen`.** Would remove the G sites by construction, but changes ~75 `Frozen` uses and the N/W1/H sites (stage 4). Not 1d.

## 1. The `drop(frozen)` sites today

### 1.1 The inventory, and the searches

`main` `bfd59d3`. The tables' command (`grep -rn "drop(frozen)\|drop(_frozen)" crates/ezsdr-radio-uhd/src`, 51-tables §2.5) gives **16** matches today, not 15: tx.rs 9, rx.rs 3, `test_support.rs` 1, `tx/tests/o1.rs` 1, `tx/tests/e1.rs` 1 (spelled `drop(_frozen)`) = 15, plus `tx/tests/e2b.rs:88` which is **new** since `093343d` (E2b, `bdf3493`, a copy of `e1::shutdown`). And the command **misses a spelling**: `tx.rs:7004` `drop(rig._frozen)` (`:7138` at `093343d`, so the tables missed it then too). **17 sites.**

Searches I ran to be sure nothing else releases a `Frozen` (all in `crates/`, 2026-11-12):
- `grep -rnE "drop\((_?frozen|rig\._frozen|held|hold|rig)\b|mem::drop|let _ = .*frozen"` → the 17 above; `uhd.rs:1648 drop(held)` is an `Arc`, `host.rs:1380 drop(holder)` a `Driver`, `parity.rs:3083 drop(rig)` an acceptance rig: none a `Frozen`.
- Every other mention of the identifier: `grep -rnE "\b_?frozen\b" crates/ezsdr-radio-uhd/src | grep -vE "Frozen::(of|on)|let \(.*frozen|_frozen: |released\(frozen|drop\(frozen\)"` → only the rig constructors' returns, `shutdown_owner(&core, &mut tx, frozen)` (3 calls), `pass_released(tx, frozen, …)` (e3b), and the `SwitchRig { …, _frozen: frozen }` literals. A `Frozen` bound as `_frozen` and never named again is dropped at scope end (N-like); destructurings `let SwitchRig { …, _frozen } = rig` (e2a.rs:90/198/421, e2b.rs:240/356/426) keep it to scope end.
- Consumers of a rig by value: `grep -rn "rig: SwitchRig)" provider` → `e1::shutdown` only.

### 1.2 The table (each type re-checked by reading what follows the release)

| # | Site (`main`) | Test / fn | After the release | Type | 1d does |
|---|---|---|---|---|---|
| 1 | `provider/tx/tests/o1.rs:755` | `o1_uhd_t6d_…` | end of the test, assertions only (`released` already ran the owner's work) | N | nothing |
| 2 | `provider/tx.rs:4396` | `ur_22_a_failed_eob_close_recovers_…` | end of the loop body, after asserts | N | nothing |
| 3 | `provider/tx.rs:4463` | `ur_22_a_failed_one_sample_padded_eob_…` | end of the test | N | nothing |
| 4 | `provider/rx.rs:1000` | `ur_24_nonloss_rx_stop_failure_…` | `rx.try_end()` ran *before* the drop; asserts after | N | nothing |
| 5 | `provider/tx.rs:6559` | `ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it` | test thread: `record_configured`, `mark_reopen_needed`, **`tx.pass()`** (a reopen: its flush waits on the clock) | **G** | guard |
| 6 | `provider/tx.rs:6574` | `ur_22_a_loss_during_the_reopen_clears_the_mark_the_reopen_read` | same; the probe `tx_reopen_done` runs on the test thread | **G** | guard |
| 7 | `provider/tx.rs:6596` | `ur_28_a_value_during_an_underflow_recovery_…` | `assert!(tx.pass())`, then `Frozen::of` again | **G** | guard; the thread keeps the clock to the end of the test |
| 8 | `provider/tx.rs:7004` | `ur_22_f8_recovery_that_ends_the_repeat_cancels_the_pending_switch` (`drop(rig._frozen)`) | `rig.tx.chain().mark_reopen_needed()`, **`rig.tx.pass()`** (reopen) on the test thread | **G (missed by the table)** | guard |
| 9 | `provider/tx.rs:5952` | `txp_owners` | owners are participants; the test thread polls `spy.recs()` with `std::thread::sleep(1 ms)` under a 10 s `Instant` | W1 | nothing |
| 10 | `provider/tx.rs:6059` | `ur_23_m8b_…` | `snapshots.recv_timeout(10 s)` on a channel the participant owner fills | W1 | nothing |
| 11 | `provider/tx.rs:6092` | `ur_23_m6_…` | `woken_rx.recv_timeout(10 s)`, then a `sleep(1 ms)` poll | W1 | nothing |
| 12 | `provider/rx.rs:901` | `ur_24_a_cut_receive_uses_one_short_packet_…` | `gate.wait()`, `finished.recv_timeout(10 s)` | W1 | nothing |
| 13 | `provider/rx.rs:946` | `ur_24_a_far_future_cut_…` | `first_turn.recv_timeout(10 s)`, `second_turn.recv_timeout(10 s)` | W1 | nothing |
| 14 | `provider/tx.rs:6542` | `ur_22_a_provider_stop_during_the_configure_…` | spawns a participant that `host.sleep(150 ms)`, `tx.shutdown()` on the test thread, `driver.join()` | **W2** | rewrite (§3) |
| 15 | `provider/tx/tests/e1.rs:88` (`drop(_frozen)`) | `e1::shutdown(rig)`, called by `e1_q3_…`, `e1_q6_…`, `e1_q6b_…`, `e2a_loss_between_…`, `e2a_85_…` | same pattern | **W2** | rewrite (§3) |
| 16 | `provider/tx/tests/e2b.rs:88` | `shutdown_owner(core, tx, frozen)`, called by `e2b_c4_…`, `e2b_c6_…`, `e2b_c9b_…` | same pattern (**new since the tables**) | **W2** | rewrite (§3) |
| 17 | `provider/test_support.rs:182` | `released` | hands the clock to a spawned participant, then `result.recv()` / `worker.join()` | H | nothing |

Counts: 4 N / **4** G / 5 W1 / **3** W2 / 1 H = 17. (Tables: 4/3/5/2/1 = 15.)

**W1 + the guard hangs (VERIFIED, was INFERRED).** Probe `W1` (`mutprobe.sh`): `ur_23_m8b` with `handed_over` instead of `drop(frozen)` → `FAILED` after 10.00 s at its `recv_timeout(10 s)` `expect("five idle looks")` (a busy driver stops the clock the idle owner sleeps on). So W1 stays as the table says.

### 1.3 The same shape without a `Frozen` (found by search, not in the tables)

`grep -rn '\-deadline"' crates/ezsdr-radio-uhd/src`: besides the W2 sites above, the sleeping-helper-plus-`join` pattern is in `tx.rs:1590` (`ur_22_owner_keeps_clean_recovery_after_an_unacknowledged_t3`, `tx-flush-deadline`, in a loop), `tx.rs:1621` (the same test, second part, `tx-partial-ack-deadline`) and `tx.rs:1684` (`ur_26_provider_stop_ends_recovery_after_stop_and_keeps_shutdown_report_reads`, `tx-shutdown-deadline`, in a loop). These tests never hold a `Frozen`: the test thread runs `tx.pass()` / `tx.shutdown()` with the manual clock running from the start. They are not `drop(frozen)` sites; they are the W2 shape's other copies.

The broader search for "G shape without a `Frozen`" — a test with a manual-clock device, no hold, that runs owner code on the test thread: a script over the `#[test]` fns of `src/**` (no `brain/`) flagged 29 fns with `ClockMode::Manual` and no `Frozen`/`frozen`/`hold(`/`released(`/`drive()` in the body, 6 of which call `pass/step/shutdown/start/…` (`tx.rs` `ur_29_non_lost_…`, `ur_25_the_line_lock_is_free_inside_every_transmit_device_call`, `tx/tests/e2b.rs` `e2b_r2a_…`, `provider/mod.rs` `measurement_timing_candidate_updates_…`, `ur_15_a_failed_rx_owner_spawn_…`, `ur_15_a_loss_published_by_an_owner_…`); the script's `hold(` test also hid the three helper-pattern tests above. **Not examined for dependence:** the 23 flagged fns that run no owner code (`authority.rs`, `device.rs`, `core.rs`, `rx.rs`, `mod.rs` reads and flush tests) and `ur_29_non_lost_…` (its owner is a participant, the test thread only waits on channels: W1-like). **Examined (§2.4):** the other 5 and the three helper-pattern tests: no dependence. These belong to stage 4's classification of `Frozen`/no-hold manual-clock tests, not to 1d.

## 2. Does the verdict depend today on when the test thread runs? (G and W2 sites)

Method (diagnostic, copy `n51-1d-probe` = `main` + `nap()`): `nap()` is a real `thread::sleep` of `EZ_NAP_MS` inserted after the release and after **every** following top-level statement of the test; `EZ_NAP_ONLY=k` fires only the k-th. A site "depends" if some nap fails the test at the unfixed code.

### 2.1 G sites

| Site (test) | naps tried | Result at `main` |
|---|---|---|
| 5 `ur_22_a_reopen_keeps_…` | every statement, 0 / 20 / 200 / 1000 ms | passes every time. **No dependence observed at these injection points and durations.** |
| 6 `ur_22_a_loss_during_the_reopen_…` | same | passes. **No dependence observed at these injection points and durations.** |
| 8 `ur_22_f8_recovery_…` | same | passes. **No dependence observed at these injection points and durations.** |
| 7 `ur_28_a_value_during_an_underflow_recovery_…` | same | **fails** at 20 / 200 / 1000 ms. `EZ_NAP_ONLY=1` (the gap between `drop(frozen)` and `tx.pass()`) fails; `ONLY` 2…8 (after the pass, the re-freeze is next) pass. Threshold: 1 and 2 ms pass, **3, 5, 10 ms fail** (V). Failure: the `.expect("the value is applied after the reopen")` (no `apply` call after `before`; INFERRED cause: device time ran past the value's booked 20 ms during the nap, so the burst was late; not traced further). |

So: of the four G sites **one** (`ur_28_…`) has a real window, about 3 ms wide, between `drop(frozen)` and `tx.pass()` — narrower than a load spike. It is a flake candidate of the #76 kind that no issue lists. (The 5 and 6 and 8 are the table's G by shape; their verdicts do not depend because their asserts read marks and records, not device time.)

### 2.2 W2 sites

| Site | naps (after the drop, after the spawn, after `shutdown`) | Result at `main` |
|---|---|---|
| 14 `tx.rs:6542`, 15 `e1::shutdown` (5 callers), 16 `shutdown_owner` (3 callers) | 200 and 1000 ms at each of the three positions | passes (`tests::e1`, `e2a`, `e2b`, `ur_22_a_provider_stop_…`: 60 tests, 0.48 s / 1.83 s / 9.03 s). **No dependence observed at these injection points and durations.** |

Why none: after the release the clock is idle-running with no busy participant; the helper's 150 ms sleep starts when it is spawned and `shutdown`'s 100 ms window is a clock wait, so a late test thread only shifts both. The sleeping helper came in `695e1f4` (stage L2); I did not establish why a participant was thought necessary, and nothing in the rewritten tests needs it.

### 2.3 What 1d therefore does, honestly

- `ur_28_…`: failing-first evidence (a 3 ms nap fails it at `main`), a fix, and a test that pins the fix deterministically (§4).
- The other 3 G sites and the 3 W2 sites: **no failing-first evidence exists** (none of the naps fails them). The guard is by construction; the table's premise ("the test thread runs the owner's code itself") is true but does not make those verdicts depend on timing. I propose to guard them anyway because (a) it is the stage's brief, (b) the 150 ms helper thread and `join` disappear, (c) a later assertion on device time in these tests would otherwise silently become a flake. If Sol prefers, the 6 could be left and only `ur_28_…` fixed; that is a smaller 1d but leaves a G site per table.

### 2.4 The no-`Frozen` tests of §1.3

Same method (`/tmp/opencode/n51-1d-nap`, 7 tests: `ur_22_owner_keeps_clean_recovery_…`, `ur_26_provider_stop_ends_recovery_…`, `ur_25_the_line_lock_is_free_inside_every_transmit_device_call`, `e2b_r2a_…`, the three `mod.rs` ones; a nap after every statement): 0 / 20 / 200 / 1000 ms all pass (0.61 s … 94.4 s). **No dependence observed at these injection points and durations.**

## 3. W2: the rewrite tried, and the decision

Tried in the sketch (`n51-1d-sketch`), V:
```
before (tx.rs:6542, e1.rs:88, e2b.rs:88):          after:
  drop(frozen);                                      let _driver = handed_over(frozen, &core.device.host());
  let host = core.device.host();                     tx.shutdown();
  let clock_host = host.clone();
  let driver = host.spawn("tx-shutdown-deadline", move || clock_host.sleep(150 ms)).expect(..);
  tx.shutdown();
  driver.join().unwrap();
```
- The three W2 sites (9 tests in all: 5 callers of `e1::shutdown`, 3 of `shutdown_owner`, 1 inline) pass: `tests::e1 tests::e2a tests::e2b ur_22_a_provider_stop_during…` 60/60, 0.31 s (the 100 ms ack window really passes: the test thread is idle in `host.now()`-bounded `tx_async` waits, so the clock runs).
- With the injection at the unfixed code's three positions at 200 / 1000 ms (probe, rewrite applied): still 60/60. (After the rewrite there is no position where a nap can matter: the driver is busy from before the release.)
- 40 runs unloaded and 30 runs with 24 busy loops (load average about 16 at the end): 0 failures.
- Why the guard fits only now: `JoinHandle::join` is a non-clock wait; a busy driver blocked in it would stop the clock the joined thread sleeps on. The rewrite removes the joined thread; the guard has nothing left to wait on but the clock.
- Decision: **keep** (the brief: "kept only if it works"). Also tried: the same rewrite on the three helper-pattern sites of §1.3 without `handed_over` (`let _driver = core.device.host().drive();` for `ur_26_provider_stop…`; `let driver = …drive(); tx.pass(); drop(driver);` for the two in `ur_22_owner_keeps_clean_recovery…`): 3 runs of both tests pass (0.41 s). Optional (§6).

Not changed and not needed: `Tx::shutdown` (production), its window, any assertion of the 9 tests.

## 4. Tests and mutation rows

### 4.1 The helper and its unit tests (new, `test_support.rs`, `#[cfg(test)] mod tests`)

```rust
pub(super) fn handed_over(frozen: Frozen, clock: &HostClock) -> crate::host::Driver {
    let driver = clock.drive();
    drop(frozen);
    driver
}
```
1. `handed_over_holds_the_clock_while_the_test_thread_is_slow`: manual clock, `Frozen::on`, `handed_over`; `held = clock.now()`; `std::thread::sleep(100 ms)`; `assert_eq!(clock.now(), held)`. A thread that is not the clock's participant after the release lets the clock run 100 ms: the assert fails with `left: 100.17ms, right: 103µs` (observed on mutant N51D-01).
2. `handed_over_leaves_no_instant_between_the_frozen_and_the_driver_for_the_clock_to_run`: `Frozen::on(clock)` on the main thread; a worker thread arms `host::pause_next_state_lock(reached, release)` (the existing hook, `host.rs:659`) and calls `handed_over`; the main thread waits for `reached`, reads `clock.now()`, sleeps 50 ms, asserts it did not move, then releases the worker. In the right order the worker's first state-lock is `drive()`'s registration, with the frozen participant still busy: the clock stands. With the order swapped the worker is paused after the frozen participant has gone: the clock runs and the assert fails (`left: 50.17ms, right: 108µs`, observed on mutant N51D-02). The only waits are the 10 s `recv_timeout` for `reached` (a bound on setup, with an `expect` message) and the real sleep that is the injection; the verdict is an `assert_eq`.

### 4.2 The sites

- **Sites 5, 6, 8 (`ur_22_a_reopen_keeps_…`, `…loss_during_the_reopen_…`, `ur_22_f8_…`):** `drop(frozen)` → `let _driver = handed_over(frozen, &device.host());` (f8: `rig._frozen`, a partial move of `SwitchRig`, which has no `Drop`). Nothing else changes; no new assertion (none can observe the guard, §2.1).
- **Site 7 (`ur_28_…`):**
```
// The reopen needs the device's clock to run: the test thread takes the clock from the frozen participant and holds it from then on. It is
// slow before the reopen on purpose: no device time passes while it is not waiting on the clock (note 51 §4.4).
let _driver = handed_over(frozen, &device.host());
std::thread::sleep(Duration::from_millis(50));
assert!(tx.pass(), "the reopen runs first");
… (the `Frozen::of(&device)` after the pass is deleted: the thread's registration holds the clock; the later `turns` do no clock wait, or they would hang today)
```
  The deliberate 50 ms sleep is the failing-first made permanent: at `main` a 3 ms step fails the test (§2.1); with the guard 50 ms changes nothing. It is an injection, not a bound: nothing in the test depends on the sleep returning in time. (The scan that forbids `thread::sleep` in driving-rig tests, note 51 §4.3, is for `tests/**`; this is a `src` unit test.) I considered a gap-free re-freeze (`Frozen::of` before dropping the driver) and chose to delete it instead: with the driver kept the test has no gap at all.
- **Sites 14, 15, 16 (W2):** the 5-line sequence of §3 → `let _driver = handed_over(frozen, &core.device.host()); tx.shutdown();`. `e2b.rs` imports `handed_over`. `e1::shutdown` and `shutdown_owner` stay two functions (they are two copies; merging them is not part of 1d; I note it).
- **Counts after:** `Frozen` uses unchanged (~75); `drop(frozen)` sites 17 → 10; the `host.spawn("…-deadline")` helper: 3 left (§1.3).

### 4.3 Mutation rows (new; Rust token anchors, verified exactly-once with `anchor.apply` on the sketch; each mutant built in a copy outside the repository and run with the named filter)

| id | file | old → new | filter (killer) | killer fails on |
|---|---|---|---|---|
| N51D-01 | `test_support.rs` | `let driver = clock.drive();` → `let driver = HostClock::real().drive();` (the thread joins no clock) | `handed_over_holds_the_clock_while_the_test_thread_is_slow` | `assert_eq!(clock.now(), held, "the clock ran while the thread that holds it did not wait on it")` — observed `100.17ms` vs `103µs` |
| N51D-02 | `test_support.rs` | `let driver = clock.drive(); drop(frozen); driver` → `drop(frozen); let driver = clock.drive(); driver` | `handed_over_leaves_no_instant_between_the_frozen_and_the_driver_for_the_clock_to_run` | its `assert_eq!` on `clock.now()` — observed `50.17ms` vs `108µs` |
| N51D-03 | `tx.rs` | `let _driver = super::super::test_support::handed_over(frozen, &device.host()); std::thread::sleep(std::time::Duration::from_millis(50));` → `drop(frozen); let _driver = (); std::thread::sleep(std::time::Duration::from_millis(50));` (the unguarded site) | `ur_28_a_value_during_an_underflow_recovery_is_applied_after_the_recovery` | `device_at`'s `assert_eq!` "the Core's clock and the device's are not at the same instant" (`test_support.rs:205`; observed `(3200000, 3200465)`), an assertion, not a bound |

Neither N51D-01 nor -02 is killed by the other's test as the *only* killer: the first mutant also fails `ur_28_…` and test 2 (by its `expect` on `reached`, since the unregistered clock never reaches the lock); the second is killed by test 2 alone (verified: `ur_28_…` and test 1 pass on it). Filters name one test each.

**Rows deliberately not added (equivalent mutants, shown):** the unguarded form of site 5 (`drop(frozen)` instead of the helper): `ur_22_a_reopen_keeps_…` **passes** (probe `M5`); the unguarded `e1::shutdown` (`drop(_frozen)`): `tests::e1 tests::e2a tests::e2b` 59/59 **pass** (probe `M6`). Sites 6, 8, 14, 16 are the same. Their guard has no observable effect today, so a row would be a known survivor; I do not list them in `known-survivors.txt` (no row, no survivor). The `std::thread::sleep(50 ms)` in `ur_28_…` is not mutated either (deleting it leaves a passing test with a guard: equivalent).

**Existing rows to re-run** (their killer test is one of the rewritten tests; no filter or anchor changes): P1298, P1302, P1306, P1312 (`ur_22_owner_keeps_clean_recovery…`, `ur_26_provider_stop_ends_recovery…` — only if 1d-C′ is taken, else unaffected), N47S4-20, N47S4-51, N47S5-44, E1-U1, E1-U2, E1-U11, E2A2B-09, E2B-13, E2B-14 (13 rows); J58 (`provider::`) is broad, covered by the full list at the sub-step commit. Found with: rows whose `filter` is a substring of a rewritten test's path (script over `mutations.json`). No row anchors on `drop(frozen)`, `Frozen`, `-deadline`, `released(` (`grep` over `old`/`old2`: 0); the only `test_support.rs` row is `E2A2A-10` (its anchor is an `assert!`, untouched). `rules.py --base bfd59d3` must find every row once.

### 4.4 Sweep after the change (not a gate)

At the end, the injection sweep of §2 on the final tree for the 4 G + 3 W2 sites (nap 20 / 200 / 1000 at every statement): expected 0 failures; plus 20 × the affected set under 24 busy loops (done on the sketch: 40 + 30 runs, 0 failures).

## 5. Text 1d makes false

- `plan/phase8/51-test-timing.md`: §4.4 heading "15 か所" and its table (4/3/5/2/1 → 4/**4**/5/**3**/1 = **17**; the `tx.rs` line numbers; the `drop(rig._frozen)` G site; the `e2b.rs:88` W2 site; the `(_frozen)` spelling; the guard is `drive()` through `handed_over`, not `released`); §7.1 row `src/provider/{tx,rx}.rs…` ("`drop(frozen)` 15 … 1d (G の 3，W2 の 2)" → 17 … 1d (G の 4，W2 の 3)); §7.2 row 1d (result); §8 first S1 row ("unit の G" → "unit の G と W2", after 1d); §11 first bullet ("W2 の書き直しは 1d で INFERRED のまま" → VERIFIED); the head line (status: 1d); a "段 1d の後" paragraph in the appendix. Q6's "the 15 `drop(frozen)` sites become the participant guard" (appendix 6) is history: leave.
- `plan/phase8/51-tables.md`: §2.5 heading and its re-grep command (give the spelling-proof command of §1.1), rows 5–7 (lines), row 13/14 (W2 now 3: add `e2b.rs:88`), a new row for `tx.rs:7004`, the closing "No contradiction with §4.4" line, §0's "15 `drop(frozen)`" and the count line (`51-tables.md:23, :61`); T13's line list is from `093343d` and untouched (1d moves no `recv_timeout`).
- `crates/ezsdr-radio-uhd/tests/README.md`: nothing false. Optional one clause where it says "where one needs the device's clock to stand still … a participant holds the clock until the test releases it": the unit tests that run the owner's code on the test thread take the clock with `test_support::handed_over`. I suggest adding it.
- Code comments: `test_support.rs` `Frozen` doc ("the drop gives the clock back") stays true; add a pointer to `handed_over`. `e1.rs:85` (`shutdown` doc "with the manual clock released so that its 100 ms acknowledgement window can pass") and `e2b.rs:86` stay true. `tx.rs` ur_28's old comment is replaced (§4.2).
- `plan/phase8/37-test-thread-timing.md`, `design/*`, KNOWN_LIMITATIONS: nothing (grep: no mention).
- `handoff.md`: the 1d line, at the gate (orchestrator).

## 6. Sub-steps (each leaves the tree green)

- **1d-A** the helper, its two unit tests, rows N51D-01 and -02. Nothing else uses it yet. (`cargo test -p ezsdr-radio-uhd --lib`; fmt; clippy.)
- **1d-B** the four G sites; the `ur_28_…` slow-thread step; row N51D-03; re-run N47S4-20, N47S4-51, N47S5-44. (Failing-first: at `main` plus the 50 ms step, with the unguarded site, `ur_28_…` fails — that is mutant N51D-03.)
- **1d-C** the three W2 sites (`tx.rs:6542`, `e1::shutdown`, `shutdown_owner`); re-run E1-U1, E1-U2, E1-U11, E2A2B-09, E2B-13, E2B-14.
- **1d-C′ (optional, recommended)** the three helper-pattern sites of §1.3 with a bare `drive()` (no `Frozen` to hand over); re-run P1298, P1302, P1306, P1312.
- Then, at the orchestrator's gate: the full mutation lists once, the step's text (§5).

Self-review (AGENTS §9 step 5): the brief marks 1d risky (small) and asks for Sol's plan review first; I propose one self-review round at the end (not per sub-step: the whole diff is about 90 lines).

## 7. Not checked / risks

- `--lib` of `ezsdr-radio-uhd` only (582/582, fmt, clippy `-D warnings` clean on the sketch). The integration binaries (`fake`, `parity`) do not use `handed_over`/`released` (grep: `src/provider` is `#[cfg(test)]`-only); not run.
- No `mutate.py` / `remote_mutate.py` (rule). The three rows' kills are from by-hand mutant copies, one build each; the 13 existing rows are not run.
- Whether `ur_28_…`'s ~3 ms window has ever failed in a gate: not looked at (no issue lists it). The load runs (24 busy loops, average about 16) of the *unfixed* test were not done; the 3 ms threshold is from injection, not from load.
- 50 ms (ur_28) and the 100 ms/50 ms of the helper tests are real sleeps chosen well above the 3 ms threshold; the 10 s `recv_timeout` in unit test 2 is a setup bound, not a verdict.
- The `drive()`-on-the-test-thread shape means a **non-clock wait inside the owner's code run on the test thread would hang** (clock stopped), exactly as for a production owner; none of the 7 sites has one (they pass), and W1 shows the failure mode (§1.2).
- `Driver` is `!Send`: the helper's result cannot leave its thread; no site moves it.
- The 23 unexamined flagged fns of §1.3 and the `ur_29_non_lost…` test.

## Changes in 1d (implementation, 2026-11-12; one run for 1d-A, 1d-B, 1d-C; no commit)

Diff: 8 files, +137 −58 (`test_support.rs` +51: helper, doc pointer, 2 unit tests; `tx.rs` 4 G sites, 1 W2 site, ur_28 step; `e1.rs`, `e2b.rs` the W2 helpers; README clause; 3 rows; the two note-51 files).

- **1d-A.** `test_support::handed_over` exactly as §4.1, with `Frozen`'s doc pointing to it. Unit test 2 writes `infra(..)` on `RecvTimeoutError::Timeout` only (m1) and then `expect`s the result (so `Disconnected` is an ordinary failure). Rows N51D-01, N51D-02 in `plan/maintenance/tools/mutations.json`.
- **1d-B.** Sites `tx.rs:6555, 6570, 6593, 7003` use `handed_over` (`handed_over` is imported next to `Frozen`, `device_at`). `ur_28_…`: the old comment is replaced; after `handed_over` it reads `held = device.host().now()`, sleeps 50 ms, and asserts `device.host().now() == held` **before** `tx.pass()` (m2); the second `Frozen::of(&device)` is deleted (the thread's registration holds the clock). Row N51D-03 (the anchor includes the `let held = …` statement so that it matches once among the three `handed_over(frozen, &device.host())` lines).
- **1d-C.** `tx.rs:6542`, `e1::shutdown`, `e2b::shutdown_owner`: the 150 ms helper thread and the `join` are gone, `handed_over(frozen, &core.device.host())` then `tx.shutdown()`. `e2b.rs` imports `handed_over`.
- **Not done:** 1d-C′ (deferred to stage 4); the §4.4 injection sweep on the final tree (not a gate; the 20 × runs below stand for it).
- **Text:** note 51 head line, §4.4 (heading, 17 sites, table, counts), §7.1 row, §7.2 row 1d, §8 S1 row, §11 W2 bullet, new paragraph "段 1d の後"; `51-tables.md` head tree line, §0 counts (two places), §2.5 (17 rows, the re-grep command, lines after 1d), T13's count text and the §2.4 text with "15"; the tests README clause. Q6's "15 `drop(frozen)` sites" in the appendix is history and left.

### Runs
- `cargo test --workspace`: 2454 passed, 0 failed (`ezsdr-radio-uhd --lib` 582, up from 580 + 2). `cargo clippy --workspace --all-targets -- -D warnings` clean. `cargo fmt --all --check` clean. `rules.py --base bfd59d3` = 0 problems.
- 20 × `--lib` with the filters `handed_over ur_22_a_ ur_22_f8 ur_28_a_value tests::e1 tests::e2a tests::e2b ur_22_a_provider_stop` (80 tests per run): 20/20 green, 0.33 s each. Not run under load (no new injection or load sweep).

### Rows (`REMOTE_SSH="ssh -o BatchMode=yes" simq python3 remote_mutate.py maintenance,phase7 sim03:4 <ids>`, finished 14/14; the remote output gives only the outcome, so each failing assertion below is from a by-hand copy outside the repository, built locally with `anchor.apply` and the row's filter)

| row | outcome | killer's failing assertion |
|---|---|---|
| N51D-01 | killed | `test_support.rs:1140` `handed_over_holds_…`: `assertion left == right failed: the clock ran while the thread that holds it did not wait on it` (left 100.15 ms, right 116 µs) |
| N51D-02 | killed | `test_support.rs:1160` `handed_over_leaves_no_instant_…`: `… the clock ran between the release of the frozen participant and the driver's registration` (left 50.18 ms, right 118 µs) |
| N51D-03 | killed | `tx.rs:6595` `ur_28_…`: `… the clock ran while the test thread, which holds it, was slow` (left 51.2 ms, right 1.15 ms) |
| N47S4-20 | killed | `tx.rs:6560` `ur_22_a_reopen_keeps_…`: `a reopen keeps the mark` (None vs Some(1)) |
| N47S4-51 | killed | `tx.rs:6577` `ur_22_a_loss_during_the_reopen_…`: `a loss during the reopen clears the mark the reopen read` (Some(1) vs None) |
| N47S5-44 | killed | `tx.rs:7006` `ur_22_f8_…`: `assertion left == right failed` ("pending" vs "cancelled") |
| E1-U1 | killed | `e1.rs:193` `e1_q3_…`: `nothing read before the origin: []` |
| E1-U2 | killed | `e1.rs:193` `e1_q3_…`: reason `UR-26: cancelled by Stop` vs `…by Provider::stop` |
| E1-U11 | killed | `e1.rs:254` `e1_q6b_…`: `[]` vs `["UR-26: cancelled by Stop"]` |
| E2A2B-09 | killed | `e2a.rs:354` `e2a_85_…`: `[]` vs `["UR-26: cancelled by Provider::stop"]` |
| E2B-13 | killed | `e2b.rs:333` `e2b_c4_…`: `C4(ii)` (`["UR-26: cancelled by Provider::stop"]` vs `[]`) |
| E2B-14 | killed | `e2b.rs:474` `e2b_c9b_…`: `(b): the loss's clear stands` (Some(1) vs None) |
| E2A2A-10 | killed | `e2a.rs:49` `e2a_lock_reentry_…`: `the re-entry did not panic: Timeout` — **a 10 s bound, not an assertion** (the row is pre-existing, its anchor is untouched by 1d; included because `test_support.rs` changed; the remote runner reports `killed`; the row has no `"kill": "bound"` and I did not look into why that is not `ERROR` under stage M's rule: pre-existing, left to the orchestrator) |
| J58 | equivalent (survived) | equivalent (survived), as the row declares (`"equivalent"` field); the filter `provider::` is broad; 1d changed no behaviour it reads) |

N51D-01's mutant also fails test 2 (by its `expect` on `reached`) and `ur_28_…`; N51D-02 is killed by test 2 alone; the two `handed_over` tests are the sole filters of their rows (as §4.3 said).

### Deviations
- E2A2A-10 and J58 were added to the re-run list (the first because `test_support.rs` changed, the second as the broad `provider::` row); the brief's list was the plan's re-run list minus C′'s four.
- The §4.4 injection sweep on the final tree was not run (§4.4 of this plan calls it "not a gate").
- Unit test 2: `reached` is read once (`recv_timeout`) and `.expect`ed after the infra marker, not read twice as in the sketch.
- E2A2A-10's kill is a 10 s bound by its own test; not investigated (pre-existing).
