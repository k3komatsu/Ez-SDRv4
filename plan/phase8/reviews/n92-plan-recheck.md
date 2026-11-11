# #92 revised test plan — re-check (Opus, read-only, 2026-11-12)

Scope: `test-plan.md` (revised), against Sol's `sol-plan.md`, `plan-adjudication.md`, `brief-plan2.md` and the tree at `1371ffa` (`/home/komatsu/local_work/Ez-SDRv4-n92`). Nothing was edited, built or probed: every outcome below is read from source. **VERIFIED** = the cited `path:line` says it; **INFERRED** = my reasoning from those lines, not run. Paths are under `crates/ezsdr-radio-uhd/` unless they say otherwise.

**Verdict: PASS with one major (M1) and minors.** Sol's five majors are closed in substance. The direction holds. The implementer's refinement (the `plan` row written in `Provider::stop` after `join("uhd-control")`) is sound and better than a once-only flag. Before code: fix M1, fix the attribution of N92-8's killer, and settle the plan-row mechanism, because T9 and N92-7/7b depend on it.

## 1. Sol's five majors

| # | Status | Evidence |
|---|---|---|
| 1. No device-time wait while a participant blocks on a std primitive | **Closed** | The blocking scenarios moved to S-spawn unit tests. Their host clock is real, not manual: `prepared_with` takes its core from `test_support::rig()` (`src/provider/mod.rs:799`), which builds `FakeConfig::default()` (`src/provider/test_support.rs:29-30`), whose clock is `ClockMode::Real` (`src/device.rs:415`). `spawn` registers on `core.device.host()` (`src/provider/mod.rs:370`). So no unit test has a manual clock to stop. Their handler paths also never wait for device time: `Core::now` reads the rig's `ManualTimeAuthority`, and `join` is wall-based (`mod.rs:390-398`) (INFERRED that this holds for any rig). In the full-coordinator tests (T1, T2c, T6a, T8) no seam blocks: S-dev and S-auth panic, and S-recv no longer blocks. Residual: a wall-time race of a different kind, M1 below. |
| 2. External `bounded` around every blocking scenario, teardown included | **Closed** | R2 marks T2a, T3, T4, T5, T6, T8 and T9 [B]. `bounded` exists (`tests/fake.rs:1789-1802`). T3 no longer relies on `Direct::until`'s own deadline (`tests/fake.rs:1483-1490`). The unit-test copy goes into `test_support.rs`. |
| 3. Policy `continue` repeat safety | **Closed as coverage; its killer claim is wrong** (m1) | T8 asserts before `finish` under `continue`. But the plan's "fails" lines for T1/T8 and the row N92-8 claim that a loss which skips `lost.store` keeps the repeat feeding. The loss's own `Streams::end` stops it anyway (m1). |
| 4. KC-21a's pending-submit path | **Closed** (residuals m2, m3) | Mechanism VERIFIED: `wait_finished` stops on `shared.end` or `frozen`, rechecked each `RECHECK` (`crates/ezsdr-kernel/src/coordinator/paced.rs:186-205`). `taken` is acknowledged only at the next `recv` (`state.rs:72-80`). The Provider's `ctx.time` is the coordinator's `shared.time` (`pipeline.rs:562`), so an S-auth wrapper put in the Assembly reaches `Core::now` (INFERRED that `shared.time` is the Assembly's authority, since `Assembly::new(Box::new(authority), …)` at `tests/common/mod.rs:390`). No uhd-control path calls `core.now()` before `book`: in the run loop only `host.now()` is called (`src/provider/control.rs:235-249`); `check_device` makes none (`:640-669`); `addressed` makes none. |
| 5. RF oracle: in-flight vs new work, buffered samples, no promise of silence | **Closed in T8 and in the UR-16 text** (residual m4) | Oracle Q's two-window design is sound. Under M4 each waiting participant gets its turns within `W` (`src/host.rs:14-15`), so window 2 does not depend on when a thread runs. In T1 the oracle is vacuous, and item 4's source is not observable before cleanup (m4). |

## 2. The refinement: write the `plan` row in `Provider::stop` after `join("uhd-control")`

**Sound; recommend it.** The evidence:
- **One writer.** `stop` runs at most once, guarded by `stopped` (`src/provider/mod.rs:683-685`). `cleanup` calls it whenever the threads ran and `stop` did not (`:725-727`). A failed `start` calls it too (`:675-678`). The end is always booked before the write (`:694-701`), so `ended_at` is `Some`.
- **It already matches UR-30.** UR-30 says "at `Provider::stop` the plans …" (`design/18-uhd-radio.md:336`).
- **The row comes out at the same point as today.** Today's writer, `Control::finish` (`control.rs:256-260`), runs when control sees `stop`, which is also before the transmit and receive shutdowns. The row's position relative to the other `timing` rows is unchanged.

**When the row is written never.** Only on `stop`'s early returns: no `prepared`, or no `running` (`:686-691`). In both cases no thread was started, so this is the same as today. If the Kernel never calls `stop` or `cleanup`, the row is missing as before (pre-existing).

**When it could be written twice.** Only if `finish` keeps its write. Then a control that finishes normally writes a row, and `stop` writes another. A detached control also writes late, after `write_sections`. So `finish`'s write must go, and its doc comment with it (`control.rs:254-255`).
- `control/differential.rs:241-243` calls `finish()` directly to "record the plan". It is not in the plan's list. Its comment goes stale, and its call becomes pointless.

**A detached uhd-control.** `stop` writes the row while control may still be alive. Control can be detached only inside a device call (`ref_locked` or `time_now` in `check_device`), since it holds no streamer and no gate (`control.rs:642, 658`; no `claim_owner`, `open_streamer` or `device_gate` in `control.rs:1-681`, by grep). On return it reaches the loop top, sees `stop` and exits (`control.rs:237-239`) without booking. So nothing is booked after the row (INFERRED). Today's design instead loses the row when a detached control finishes after `write_sections`, so the refinement is strictly better there.

**A new behaviour to decide.** A `start` refused after `self.running` was set calls `stop` (`mod.rs:675-678`) without uhd-control ever having been spawned: an `rx_start` error or a spawn failure (`:620-669`). Today such a Manifest has no `plan` row. With the refinement it has one, recording lines `Control::new` inserted for streams that never ran. Choose one:
- write the row only if `uhd-control` was in `running.threads` before the join; or
- accept the row and check the `ur_15_*` failed-start tests that read `timing`.

**What the refinement changes in the plan.**
- **T9c becomes moot.** Its sequence (a panic at `finish`'s `expect`) is unreachable from `run()`: `stop` sets `stop` and books `end` under one lock (`mod.rs:694-699`), and `run` calls `finish` only after seeing `stop` (`control.rs:237-238`).
- **T9b's killer changes.** It becomes the killer of "finish still writes", i.e. N92-7b re-anchored to `finish`, or to `run`'s `stop` branch if `finish` is deleted.
- **N92-7 is killed widely.** It becomes "delete `stop`'s write", killed by every `plan_row` reader as well as T9a.
- **Stale text to amend** (add to §7): `mod.rs:692-693` ("then uhd-control records the plans") and `design/18-uhd-radio.md:443` ("the plan uhd-control records").

## 3. KL-2 and UR-16 wording

**UR-16, the proposed text in plan §7** is accurate against the code, with three fixes:
- **Placement.** "the Provider's streams are ended and no new device call is initiated once its owners see the loss" holds under *every* Policy:
  - `device_lost` books the end (`src/provider/core.rs:1064-1066`);
  - the owners idle on `is_lost`: `tx.rs:298-301, 464-466`, rx at `rx.rs:207-212, 296, 310`.

  Placing the clause after "under a Policy that maps `DEVICE_LOST` to `continue`" makes it read as conditional on that Policy. State it on its own, then the Policy consequence.
- **`continue` is not the end of the story.** Under `continue`, the next Action submitted to the instance waits KC-21a's budget and fails the Run, because nothing drains its queue (`paced.rs:177-205`). One clause would say so; the plan mentions it only in T8's wording note.
- **Retention.** "kept until its `args` are opened again in the process or the process exits" is VERIFIED for `UhdDevice`:
  - a marked-lost device is pushed onto `KEPT` at drop (`src/uhd.rs:685-688`);
  - `reclaim` frees a kept one that `answers()` at the next `open` of the same `args`, unless a live device of the same `args` exists (`uhd.rs:643-665`).

  That this works on a healthy, panic-lost X300 is INFERRED: never run.
- **Pre-existing, now touched.** UR-16's detached-thread list names only `uhd-rx-s<k>` or `uhd-tx-s<k>`, but `join("uhd-control")` is bounded and detaches too (`mod.rs:702, 390-404`). Worth one word while the rule is open.
- **`design/18-uhd-radio.md:56`**: the plan's edit keeps "stops the radios at once", which contradicts the no-immediate-silence stance. Say "ends the streams at once".

**KL-2: do not put it there.** KL-2's Symptom (1.1 s / 3.0 s after the link drops; the process aborts at exit over a direct cable), Workaround and Status (a Ceiling of UHD's static teardown) all describe a dead link (`KNOWN_LIMITATIONS.md:23-28`). With "or a Provider thread panicked" added to *When*, they are wrong for a healthy device: a panic ends the Run promptly, and the abort-at-exit mechanism is a dead link's `deinit` (`uhd.rs:668-672`).

What the user can see is different: the device stays opened, and not closed, by the process until the same `args` are opened again or the process exits. INFERRED: on an X300 that keeps it claimed against another process. It also already holds for a uhd-rx/uhd-tx panic. Recommend a separate short KL entry, or none if the owner judges a Provider bug not a "limit". The owner decides; the plan should not present the KL-2 clause as the accurate option.

**A hardware consequence the UR-16 text does not name** (INFERRED, owner's call, not a #92 blocker). After the loss, a healthy device gets no transmit end of burst and no `rx_stop`: no device call follows the loss (UR-29, `design/18-uhd-radio.md:326`), and `cleanup` skips `close_streams` (`mod.rs:733`). Whether the next `uhd_usrp_make` on the same `args` resets the radio's DUC timeline is unverified. UR-33's model keeps it "until its EOB word" (`design/18-uhd-radio.md:347`). This also already holds for a uhd-rx/uhd-tx panic.

## 4. Thread-timing dependence, oracles that miss, rows whose killer does not observe the path

### Major

**M1. T4a, T4b and T5 (and T9b if it ends via `radio.stop`) race `stop`'s 1 s wall join bound.** R3 says the `bounded` limit is the only wall time, but `join` gives up after `JOIN_BOUND` = 1 s of wall time, records `did not join`, and detaches the thread (`src/provider/mod.rs:390-404`).
- **T5.** By design, the panic must happen *inside* that window: stop books the end, then a helper thread polls and releases the body, the body panics, and the handler takes the gate and `streams`.
- **T4a/T4b.** They call `radio.stop` right after releasing the second body, without waiting for it to finish.

Minimal sequence (T5): `radio.stop` books the end and enters `join("uhd-control")` → the helper thread, polling with a wall sleep, is not scheduled for > 1 s under the 8-shards × 4-CPU load of `remote_mutate.py`, or the default panic hook spends long capturing a backtrace when `RUST_BACKTRACE` is set in a debug binary → `join` gives up → a `did not join` row, `detached = {uhd-control}` → `cleanup` takes the leak branch (`mod.rs:728-730`) → T5's "no `did not join`" and "the `leaked` row absent" fail on correct code.

Observable wrong outcome: a spurious failure, so any row whose killer is T5 counts as killed (AGENTS §7: a hidden survivor). For N92-3 in T4a/b, a late second emission can land after the assertions, so a real survivor can pass intermittently.

Fix, with no new mechanism:
- **T4a/T4b.** Before `radio.stop`, poll with wall sleeps inside `bounded` until both bodies' `JoinHandle`s in `radio.running.threads` report `is_finished()`. The child `tests` module can read the private field.
- **T5.** Book the end the way `stop` does (`test_support::end_streams`, `test_support.rs:248`, or the same locked `stop = Some(..)` plus `end`). Release the body, wait until its handle is finished, then call `radio.stop`. `stop`'s `end` is a no-op on an ended line (`core.rs:641-643`), so "one `End`" still holds. The asserted end instant becomes the one the test booked. That is the same state ("a panic after the end is booked") with no 1 s window. A literal panic during `join` adds only scheduler luck.
- **T9b.** Use the "direct `streams.end`" variant the plan already allows, not `radio.stop`.

### Minor

**m1. N92-8 (delete `self.lost.store(true, Ordering::Release)`) is not killed by T8's window 2 or its clock check, and T1's and T8's "fails" lines about an unset `lost` are wrong.**
- Even with `lost` unset, `device_lost` still runs `streams.end(.., lost = true)`, which calls `stop_at` on every transmit stream (`core.rs:645-646`).
- `stop_at` → `Bookings::stop` → `record.stop(bound)` and `record.halt(..)` end the repeat at the in-flight bound (`crates/ezsdr-radio/src/staging.rs:511-516`).
- Booking the `End` ends the transmit clock in `Streams::changed` (`core.rs:746-756`).

So uhd-tx sends at most up to `at + tx_in_flight` (about 10 ms), then goes quiet inside window 1, and the clock has `ended_at`. INFERRED end to end: the mechanism lines are VERIFIED, and the run is not. Only Q item 2's count bound `I` could catch the extra sends, and since uhd-tx already sends about one window ahead it may not.

The row is still killed:
- by T4a/T4b and T6c: without `lost` set, the early return at `core.rs:1060` never fires, so a second `device_lost` emits again;
- probably by the existing `ur_29_*`/`e2a` loss tests.

What to change:
- Name those tests as N92-8's killer, with the unit `test_bin` for T4.
- Correct the T1/T8 "fails" lines.
- If T8 should have a row of its own, it is the deletion of `streams.end(self, at, false, true)` in `device_lost`: with `lost` set the owners idle, and the transmit clock then has no `ended_at` before `finish`. An existing row has that anchor (`mutations.json:661`), so add a row with T8 as its filter, or widen that row's filter.

**m2. T2c's undelivered-Action variant depends on when the data thread runs.** `dispatch_agenda` takes the admission lock once per agenda item (`crates/ezsdr-kernel/src/coordinator/stepping.rs:371-373`), and `FreezeDispatch` takes the same lock (`ending.rs:293`).

Sequence:
1. Action A is pushed and the admission guard is dropped.
2. uhd-control takes A, panics, and emits `DEVICE_LOST`.
3. The data thread drains it, reacts, and runs `stop_early` → `FreezeDispatch` (`paced.rs:133-148`), all before the agenda loop takes the lock for B.
4. B's `admit` is refused as frozen (`admission.rs:39-40`).

Result: B is never dispatched, `undelivered` is empty, and the assertion fails on correct code. The Session path dispatches a whole entry under one guard (`pipeline.rs:1034-1035`). Use that if a two-Action entry exists; otherwise assert "B is undelivered or never dispatched", or drop the variant. Rare, but it breaks R3.

**m3. T2c's expected return value.** `submit` returns `Ok(entry)` after `wait_finished` returns (`pipeline.rs:1036-1049`), and the `send` helper returns a `LogEntry`, not a `Result` (`tests/fake.rs:1845-1848`). "Returns `Err(Ended)`" will not hold. The discriminator is the termination reason, which the plan does assert, so drop the `Err(Ended)` clause. `send` builds a burst from samples; the transmit-`Stop` submit path is still unverified (the plan says so).

**m4. Oracle Q in T1 and item 4.**
- **T1.** Under the default Policy the data thread stops the Provider itself (`stop_early`, `paced.rs:133-148`) before `run_until_end` returns. T1's "Q items 1–5 taken before `finish`" therefore observe the shutdown, not the loss path. Keep only Q items 1 and 5 in T1; T8 is where Q has power.
- **Item 4.** "Totals at `M`" cannot be read from `stats`: they reach the Manifest only at `cleanup` (`mod.rs:421`, `core.rs:999-1001`). Take the totals from `device.calls()` send sizes, or from FakeDevice's `transmitted` record (`design/18-uhd-radio.md:347`).

**m5. S-recv (T2a).**
- `Pushed::recv` and the oracle `queue.0.len()` use `.lock().unwrap()` on one std mutex (`tests/fake.rs:1316`). A panic raised *after* `self.0.lock()` poisons it, and the oracle then panics. The seam must panic before locking.
- "Panics on the n-th call" races uhd-control's empty polls every 1 ms (`control.rs:240-243`): if the polls count, the panic can come before A1 arrives. Key the panic on "the call after the one that returned A1", and push A1 and A2 under one lock.
- T2a's `tx_stop_applied` row exists only for a transmit stream with a clock (`core.rs:435-447`, `control.rs:317-323`), so the Direct setup needs an armed transmit clock.

**m6. S-spawn wiring that the plan leaves implicit.**
- `spawn` keeps the handle only when `self.running` is `Some` (`mod.rs:383-385`). Otherwise `stop` returns early and joins nothing (`:689-691`). T4, T5, T6 and T9 must set `Running` by hand, as `ur_26_the_control_thread_is_joined_before_every_stream` does (`mod.rs:1220`).
- `prepared_with`'s radio device and its core's device are two different fakes (`mod.rs:791, 799`). `mark_lost` goes to `core.device`, and `close_streams` to `radio.device` (`mod.rs:735`). The oracles must read the right one.
- T6b's "`count(dir) == 0`" is false: `with_streams` keeps at least one slot per direction (`core.rs:306-307`). The test still holds, because `Streams::book` returns `false` for a stream with no line (`core.rs:595`).

**m7. N92-1's killer list.** T4a cannot kill it: the uhd-tx panic still emits the one `DEVICE_LOST`. T4b kills it only through the `bounded` timeout of its "wait for the event" poll. T1, T2c, T6a and T8 are the real killers.

**m8. S-dev must panic before it delegates.** If the wrapper calls `FakeDevice::time_now` first, it may hold the fake's clock state. Only the thread name and the flag should be read before the panic. INFERRED; `FakeDevice::time_now` was not read.

## 5. Does anything refute the direction?

No.
- **A lock left held.** uhd-control holds no gate or streamer, by grep of `control.rs:1-681`. Every `Core` lock recovers from poison: `lock` at `core.rs:68`, the gate at `core.rs:941-949`.
- **Thread exit on the manual clock.** A dead thread is unregistered by its guard (M1, `host.rs:8-10, 388-393`).
- **Event kind.** `DEVICE_LOST` is the only event an unstepped Provider has (MA-9a).
- **Costs.** No end of burst and no close on a healthy device; the device retained until a re-open. These are real but already accepted for uhd-rx and uhd-tx (§3), and the adjudicated fail-closed choice (B) owns them. The remaining unknowns are the ones the plan lists as INFERRED in §9: a panic inside `device_lost` reachable from a half-booked line, and RF stop time.

## Not checked

- `tx.rs`'s `stop_now` and `Tx::shutdown` under loss: whether they make any device call. I relied on UR-29's text.
- `FakeDevice::time_now`'s internals (m8).
- `admit`'s full path for a frozen dispatch beyond `admission.rs:39-40`.
- Whether `Lines::prune` can change a recorded `plan` (I relied on `timeline.rs:219-224`'s replay).
- `tests/hardware.rs` audit rows (I took the plan's table as read).
- `mutations.json:661`'s filter.
- Whether the Session API takes a two-Action entry (m2).
- No run in a copy and no `rdev.py` probe; every "kills"/"survives" above is INFERRED.
