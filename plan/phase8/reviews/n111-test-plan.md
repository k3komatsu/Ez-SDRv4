# #111 + #119 test plan v3 (read-only; nothing written or run)

v3 fixes the items the Opus and Terra re-checks of v2 found. Section 0 maps each to its change. Sections 1–7 keep v2's structure, and the parts v3 does not change are restated so the plan stands alone.

## 0. Finding → change

| Finding (source) | Change in v3 |
|---|---|
| T6: `second Serialised < radio:stop` fails on correct code (Opus N2, Terra 1) | T6 asserts `cancel:released < radio:stop` and `cancel:released < second Serialised`. The data thread records `radio:stop` right after its own freeze, so it cannot be ordered against the step thread's `Serialised`. The second freeze is identified by the freeze hook's `Serialise` event with `thread != first freeze's thread`, not by W_f, because `finish()` runs the freeze on a deadline worker (`run.rs:575`). |
| T13 fails on correct code (Opus N4, Terra 6) | `finish` is gated on the data thread's wake `Schedule` return, with the handle filled. T13, T11 and T14 each name the far-future untimed `stop` handle they cancel. |
| Escalation race, a gap between the check and the lock (Opus N1, Terra 11) | The post-`next_wakeup` recheck is dropped. `dispatch_agenda` checks `frozen` right after taking the admission lock and breaks without dispatching or escalating. T9 is rebuilt: new seam `Point::BeforeAgendaLock`, the Spec overrides DEVICE_LOST to a Stop policy (`policy.rs:79` defaults it to Abort). N111-15 is retargeted. KC-20 gets one sentence. |
| Withdrawal cancel outstanding across Provider stop (Terra 10) | The early withdrawal's remove-and-cancel runs under `freeze_gate`. Lock order is `freeze_gate` → `scheduled`, and no `admission` is held there. New test T10c (a gate after removal and before cancel), new row N111-21, and T10a adjusted. |
| T4 sequence (Opus N6, Terra 4) | `disconnect_signal()` runs on worker W_d. Fixed and mutant release paths are explicit: submit on W_s and assert before opening G_s on the fixed path; open G_s first and then fail on the mutant path. |
| RS-6 and KC-24a text (Opus N3, Terra 1 and 9) | RS-6 states RS-8a's exception, labels the second performance "by either thread (KC-46b)", and covers the withdrawal (KC-29b). The wording is no longer unconditional. |
| T5 (Opus N9, Terra 12) | T5's fixture has one scheduled handle. The fired count is recorded in the double and asserted on the test thread, with no assertion inside the hook. |
| T3 (Opus N5, N9) | T3 gets a far-future untimed `stop` (otherwise `cancel` never runs). Its `action_log` assertion is dropped, since it proves nothing in a Spec Run. |
| T2b wording (Terra 3, Opus N9) | T2b waits for the second freeze's `Done`, not for cleanup to end. The data thread is released after that, and step 3 joins it afterwards. |
| KC-46a (Opus N8) | The sentence is not repeated. T2b is cited in KC-46a's `*Test:*` line. |
| Facts Opus could not check | Both verified below, with path:line. |

**Verified facts.**

1. **A resource-targeted untimed `stop` is accepted by reading, never run.**
   - `ActionTemplate::Stop { target: Option<Target> }` is at `event.rs:848-852`. `is_timed()` is false for it (`event.rs:865-867`), so `arm` puts it in the agenda and schedules a handle (`pipeline.rs:801`).
   - `plan/validation.rs:348-350` runs `classify_target` on a present target. `rewrite_template` resolves it through `resolve_target` (`pipeline.rs:1245-1247`; a Resource target must be a Provider).
   - `dispatch_agenda` sends a `Stop` with `target != None` through admission (`stepping.rs:368`, `:373`).
   - No existing test schedules one: every use has `"target": null` (`coordinator.rs:632`, `:666`, `:682`, `:2627`, `:2646`; `device_paced.rs:1206`). The first implementer check is a parse of this fixture.
   - Spelling: `"action": { "kind": "stop", "target": serde_json::to_value(support::target("radio")).unwrap() }`. `support::target` is at `doubles.rs:35-38`, and `Target`'s serde form is at `event.rs:151-163`.
2. **The Stop override spelling.** `spec["policies"] = serde_json::json!({ "failure": { "ezsdr.DEVICE_LOST": "stop" } });` is at `coordinator.rs:1748`. The paced tests use the same shape with `"continue"` (`device_paced.rs:339`, `:805`). `Reaction` is `snake_case`, with `Stop` as orderly (`policy.rs:17-29`), mapped to `CleanupMode::Orderly` at `stepping.rs:224`. The default is `Reaction::Abort` (`policy.rs:79`).

## 1. Scenarios and the code shape

| # | Site | Lock | Callback | Status |
|---|---|---|---|---|
| A | `ending.rs:313` | `admission` | `cancel` | #111, fixed here. |
| B | `ending.rs:307` with `paced.rs:166-167` | `admission` and `wake_handle` | `schedule` | Fixed here. The wake handle is taken after the drop. |
| C | `ending.rs:30`, `:42` | `machine` | `utc_nanos` | #119, fixed here by `sample_transition` then `publish`. |
| D | `admission.rs:160`, `:203` | `admission` | checks and `coerce` | By design. KC-24a's Ceiling is rewritten (section 6). |
| E | `ending.rs:421`; `paced.rs:156-166` | `links`; `wake_handle` | `drops()`; an ordinary wake against `disconnect_signal` | #121 and #120, not fixed here. |

Machine sites: `ending.rs:16`, `:30`, `:42`, `:83`, `mod.rs:296`, `state.rs:293`, `:393`, `admission.rs:43`. Admission sites: `ending.rs:293`, `stepping.rs:372`, `pipeline.rs:333`, `:1011`, `:1220`.

FreezeDispatch after the fix:

```rust
reach_freeze(Serialise);
let _gate = self.shared.lock_freeze();            // cleanup-only; no Module thread takes it
reach_freeze(Serialised);
let admission = self.shared.lock_admission();
self.shared.frozen.store(true, Ordering::Release);
/* clear queues; lock(&undelivered).extend(..) */
reach_freeze(BeforeDrop);
drop(admission);
reach_freeze(BeforeWakeTake);
let wake = lock(&self.shared.wake_handle).take();
let pending: Vec<_> = lock(&self.shared.scheduled).drain(..).chain(wake).collect();
/* cancels, F25 count unchanged */
reach_freeze(Done);
```

The early withdrawal in `advance_to_with` (`stepping.rs:416-421`) becomes:

```rust
if live && self.shared.now().try_cmp(t).is_ok_and(|o| o.is_lt()) {     // now() read before the gate
    let _gate = self.shared.lock_freeze();
    let removed = { let mut s = lock(&self.shared.scheduled); let n = s.len(); s.retain(|p| *p != handle); s.len() < n };
    #[cfg(feature = "testing")] reach(Point::Withdrawn);
    if removed && self.shared.cancel(handle).is_err() { /* unchanged request(Failed, Abort) */ }
}
```

`dispatch_agenda` takes `let _admission = self.shared.lock_admission();` and then `if self.shared.frozen.load(Ordering::Acquire) { break; }`.

- No recheck of `end` after `next_wakeup`.
- `frozen` is stored under that same lock, so there is no gap.
- In the Simulation class no freeze runs during `run_loop`, so behaviour there is unchanged.
- The `Stop{target: None}` branch before the lock only calls `request(Completed)`, which RS-10 drops if an end is already pending.

Lock order is `freeze_gate` → `admission` → leaf locks, and `freeze_gate` → `scheduled` (withdrawal). No submitter takes the gate. `stop_early` has no deadline. A control freeze waiting on a gate held by a hung `cancel` is abandoned at RS-8a, which is permitted.

Cleanup becomes `let transition = self.shared.sample_transition(); self.shared.publish(RunState::Stopping{mode: request.mode}, transition);` and the same for `CleanedUp`. The public `begin_stopping`/`finish` stay as they are.

## 2. Seams and doubles (all `testing`-gated)

- **Lock helpers.** Three, on `Shared` in `state.rs`: `lock_admission()`, `lock_machine()`, `lock_freeze()`. In testing builds they return a wrapper guard that bumps a thread-local counter on acquire and on drop. The guards are `!Send`. `coordinator::lock_record::held() -> Held { admission, machine, freeze }` reads the calling thread's counters. Without `testing`, they are plain `MutexGuard` aliases over `lock(&…)`.
- **Watched doubles** (`tests/support/start_window.rs`). `WatchedTime` (`now`, `schedule`, `cancel`, `wait_until`), `WatchedClock` (`utc_nanos`, `monotonic_millis`) and `WatchedAuthority` (`next_wakeup`, `relations`) push a `Call{kind, held, thread, at}` event at entry, before forwarding. The result slot (`returned`, `handle`) is filled after. This replaces the probe-based `held`, and #103's `ClockEvent` arms migrate to `Held`.
- **Hooks on the doubles.** `on_cancel`, `on_schedule`, `on_now` and `on_next_wakeup_return`. Each only records or parks. Assertions run on the test thread.
- **Freeze hook.** `Probe::on_freeze(Arc<dyn Fn(FreezePoint, FreezeReading) + Send + Sync>)`, stored in `Shared`. `FreezeReading` is `{held, dispatch_frozen, thread}`. Points: `Serialise`, `Serialised`, `BeforeDrop`, `BeforeWakeTake`, `Done`. It runs on whichever thread performs the freeze. Unlike the thread-local `start_probe` hook, it reaches the step and data threads.
- **Control-thread points** (thread-local, installed by the worker that owns the Run, as `start_probe`):
  - `Point::Withdrawal`: in `advance_to_with`, after `live` is computed and before `now()`.
  - `Point::Withdrawn`: after the removal, before the cancel, with the gate held.
  - `Point::Scheduled`: after the horizon `schedule` returns, before `push`.
  - `Point::BeforeAgendaLock { index }`: in `dispatch_agenda`, before the admission lock.

Audit table for the implementer: `kc_43_*`, `rs_05_*`, `kc_24a_start_reads_the_transition_clocks_outside_the_lock`, `kc_36_*` and any test counting `utc_nanos` calls, and `run_session.rs`'s public `begin_stopping`/`finish`.

## 3. Sequences and tests

Every wait is on a positive event with a 10 s backstop marked `infra()`. No assertion reads wall time. Scheduled handles are named by the `Schedule` event at arm whose `at` equals the entry's instant. The "far-future stop" fixture is `{"kind":"stop","target":null}` at an offset about an hour out (`add_schedule`, as in kc_30).

**T1 `kc_24a_freeze_runs_cancels_outside_the_admission_lock`** (`coordinator.rs`). Simulation Spec Run with two far-future stops, watched doubles and the freeze hook, then `finish()`.
- Per `Cancel` call: `held.admission == false`, `held.machine == false`, `held.freeze == true`, `returned == true`.
- At `BeforeDrop`: admission held, `dispatch_frozen`, gate held. At `BeforeWakeTake`: admission not held, `dispatch_frozen`, gate held.
- Manifest: `cleanup_failures` and `undelivered` empty, `reason == Stopped{client}`.
- Interleaving proof: exactly two `Cancel` calls and all five points present.
- Wrong change: N111-1 fails on the `Cancel` reading, N111-14 on `dispatch_frozen` at `BeforeDrop`, and N111-2 on `BeforeWakeTake`.
- Deschedule: single thread.

**T2a `kc_24a_freeze_cancels_the_wake_handle_exactly_once`** (`device_paced.rs`). It builds on `kg_03`'s rig (`device_paced.rs:882`), with a watched `WallAuthority`.
- `advance_to(1 h)` runs on W_c. The loss gate opens and the data thread runs freeze #1. Its horizon cancel wakes the control loop.
- W_c's `on_next_wakeup_return` parks until the test has seen the data thread's unconditional `wake` enter `schedule` (an `on_schedule` entry by the data thread, which already holds `wake_handle`). The test then releases W_c.
- Control freeze #2 blocks on `wake_handle` until the wake's `schedule` returns, then takes the handle and cancels it.
- Asserts: the horizon handle and the wake handle each have exactly one `Cancel`, with no admission or machine held and the gate held. `cleanup_failures` is empty.
- Wrong change: N111-6 gives the wake handle a count of 0.
- Deschedule: forced by the gate and the `wake_handle` lock.

**T2b `kc_24a_a_wake_issued_after_the_freeze_is_not_cancelled`** (`device_paced.rs`). It is the inverse order, which KC-46a states ("stays scheduled").
- The data thread's `on_now` parks the first `now()` after freeze #1's `Done`. That call is `wake`'s own read.
- The test waits for the second freeze's `Done` (a hook event). It then releases the data thread, which issues the late wake. Cleanup's step 3 joins the data thread after that.
- Asserts: the late wake's handle has `Cancel` count 0, and `cleanup_failures` is empty.
- It has no row: nothing in this step's text can break it without adding a re-freeze.

**T3 `kc_24a_freeze_a_provider_holding_what_cancel_needs_while_it_submits_does_not_deadlock`** (`coordinator.rs`). Spec Run with one far-future stop (handle H), an Executor `sharing_submitter` and a deaf Provider.
- Step 0: the test thread submits a known Module Action. It is admitted with id k and stays queued.
- W locks M and records `w:holds_m`, waits on gate G_w, then submits an `UpdateParameter`. Just before dropping M it records `w:releasing_m`, while still holding M.
- The test waits for `w:holds_m`, then runs `finish()`.
- The `cancel` double records its reading first, then opens G_w. If `held.admission` it returns without taking M, so the mutant dies on the assertion below. Otherwise it blocks on M's real lock and records `cancel:done`.
- The test joins W first, then asserts:
  - H's `Cancel` has `held.admission == false` and `held.freeze == true`.
  - Line order `w:holds_m` < `cancel:entered` < `w:submit:Err` < `w:releasing_m` < `cancel:done`.
  - W's refusal is check `ezsdr.dispatch` with reason starting "RS-6: dispatch is frozen", not RS-18.
  - `undelivered == [k]` exactly, and W's refused submit takes no id.
  - `cleanup_failures` is empty and `reason == Stopped{client}`.
- Wrong change: N111-1 fails on `held.admission`.
- Deschedule: M is held until W releases it, and W is gated by `cancel` itself, so no stall flips the verdict.

**T4 `kc_24a_freeze_takes_the_wake_handle_with_the_lock_free`** (`device_paced.rs`).
- Setup: an `on_schedule` hook parks the next call on G_s and signals `schedule:parked`.
- W_d runs `disconnect_signal()()`. It requests the end, then `wake(true)` parks inside `schedule` with `wake_handle` held.
- The test waits for `schedule:parked`, then runs `finish()` on W_f. It receives the freeze worker's `BeforeWakeTake` reading on a channel.
- Fixed path (`!held.admission && dispatch_frozen`): the test starts W_s, which submits and returns `Err("RS-6: dispatch is frozen")`. Only then does the test open G_s and join everything.
- Mutant path (anything else): the test opens G_s first, so the frozen freeze can finish, joins everything, then fails with "the wake handle was taken under the admission lock".
- Final: `cleanup_failures` empty and `reason == Stopped{client_disconnect}` (the signal's request came first).
- Wrong change: N111-2.
- Deschedule: W_s starts only after the reading, so its delay only postpones the assertion. W_f is bounded by RS-8a's 5 s.

**T5 `kc_24a_freeze_a_handle_fired_before_its_cancel_is_not_a_failure`** (`coordinator.rs`). Simulation Spec Run with **one** far-future stop.
- The `on_cancel` hook, on its first call, calls `rig.manual.advance_to(instant)` and records the count it returned and the call's `returned`.
- Asserts on the test thread: the recorded fired count is 1 and `returned == false`. `cleanup_failures` is empty, `reason` is the first request, `also` is empty, `undelivered` is empty.
- Wrong change: N111-4 produces a "KC-30 … 1 of 1 handles" cleanup failure, which is the assertion.
- Deschedule: single thread.

**T6 `kc_24a_overlapping_freezes_serialise_and_cancel_before_the_single_stop`** (`device_paced.rs`).
- Setup: paced Spec Run with a far-future stop (H) and a known Action k on a deaf Provider. The Provider is `losing_device_on(loss)`.
- The test opens the loss gate. The data thread runs freeze #1 and parks in `cancel` on G_c.
- Only then does the test start `finish()` on W_f. It waits for the freeze hook's second `Serialise` event (a freeze on a thread other than the data thread). It does not wait on W_f. Then it opens G_c.
- Asserts:
  - The first `Cancel` has `held.freeze == true` and `held.admission == false`, and the second freeze's `Serialised` reading has `held.freeze == true`.
  - Line order `cancel:released` < `radio:stop` and `cancel:released` < second `Serialised`. (`radio:stop` is recorded on the data thread right after its own freeze, so it is not ordered against the second `Serialised`.)
  - Exactly one `radio:stop`.
  - H's total `Cancel` count is exactly one.
  - `undelivered == [k]` once, and `cleanup_failures` is empty.
- Wrong change: N111-11 and N111-12 fail on `held.freeze`. N111-3 fails on a count of two.
- Deschedule: the second freeze cannot start before the first freeze's `Cancel` was entered, and G_c opens only after its `Serialise`.

**T7 `kc_24a_cleanup_reads_the_stopping_and_cleaned_up_clocks_outside_the_machine_lock`** (`coordinator.rs`). Simulation Run, watched `WatchedAuthority` and `WatchedClock`, `finish()`.
- For each of the Stopping and CleanedUp transitions: a `Now` call immediately followed by the `Utc` call on the same thread, both with `held.machine == false` and `held.admission == false`.
- The `Utc` sample equals the transition's `host_utc_nanos`, and the `Now` sample equals its `at` ticks. `manifest.termination.at` and `host_utc_nanos` equal the CleanedUp samples.
- The `Utc` call count equals the transition count (`utc_nanos` has only the sites `state.rs:388`, `run.rs:615`, `run.rs:631`).
- Wrong change: N111-7 and N111-9 fail on the `Utc` pair's `held.machine`. N111-8 and N111-10 fail on the `Now` pair's.

**T8 `kc_24a_an_abort_in_the_unlocked_cleanup_window_escalates_the_remaining_steps`** (`coordinator.rs`). Executor `sharing_submitter`, and a stepped Provider that records its `stop` mode (extend its line with the mode if it lacks one).
- W_f runs `finish()`. `on_now` parks the first `now()` on that thread, which is the Stopping sample. The test submits `Action::Abort` through the submitter and releases.
- Asserts: `reason` is the first request (`Stopped{client}`), `also == [Stopped{Abort}]`, the Stopping transition's mode is `Orderly`, and the Provider's recorded `stop` mode is `Abort`.
- It pins RS-10 and KC-39 in the new unlocked window. It has no new row.

**T9 `kc_24a_an_agenda_item_reached_after_the_freeze_is_not_admitted_and_requests_nothing`** (`device_paced.rs`).
- Setup: paced Spec Run. `spec["policies"] = {"failure": {"ezsdr.DEVICE_LOST": "stop"}}` (spelling verified, `coordinator.rs:1748`). One near-term untimed `stop` with a Resource target (`radio`), so the item is admitted rather than the Run-wide `Stop`. The Provider is `losing_device_on(loss)`.
- W_c installs a hook and runs `run_until_end`. `next_wakeup` returns the entry's instant. The hook at `Point::BeforeAgendaLock{index: 0}` parks W_c. At that point `end_requested == false`: this is the window between the old recheck point and the lock.
- The test opens the loss gate. The data thread requests an orderly Policy stop and runs freeze #1 through `Done` (a hook event). The test then releases W_c, which takes the lock, sees `frozen`, and breaks.
- Asserts: `reason == Stopped{Policy{DEVICE_LOST}}`, `also` empty, the Stopping record's mode `Orderly`, the Provider's stop mode `Orderly`, and the Provider received no `Stop` action.
- Wrong change: N111-15 (delete the frozen break) yields `also == [Failed{KC-17 agenda entry}]` and an `Abort` mode: assertions, not a timeout.
- Deschedule: nothing flips it. The freeze has completed before the release, and the park is positive.

**T10a `kc_24a_an_early_withdrawal_does_not_cancel_a_handle_the_freeze_took`** (`device_paced.rs`). W_c installs a hook and runs `wait_for(&[MARK], 0, horizon)`. The test releases the MARK gate, the control loop returns early, and `Point::Withdrawal` parks.
- The test opens the loss gate. Freeze #1 takes and cancels the horizon handle. The test waits for that `Cancel`'s return and releases W_c.
- W_c reads `now() < horizon`, takes the gate, finds the handle already removed, and does not cancel.
- Assert: the horizon handle's `Cancel` count is exactly one.
- Wrong change: N111-16 gives a count of two.

**T10b `kc_24a_a_horizon_handle_published_after_the_early_drain_is_claimed_by_the_control_freeze`** (`device_paced.rs`). `Point::Scheduled` parks W_c after `schedule` and before `push`.
- The loss gate opens and freeze #1 drains without that handle. The test releases W_c. It pushes, `run_loop` returns on the end, and `settle` runs the control freeze, which claims the handle.
- Assert: the horizon handle's `Cancel` count is exactly one, after freeze #1's `Done`.
- Wrong change: N111-13 leaves a count of 0.

**T10c `kc_24a_a_withdrawal_cancel_precedes_the_provider_stop`** (`device_paced.rs`). The sequence is as T10a up to the early return, with W_c parked at `Point::Withdrawn`: the handle is removed and the gate is held.
- The test opens the loss gate. The data thread requests the end and its freeze reaches the freeze hook's `Serialise` event, then blocks on the gate. The test waits for that event and releases W_c.
- W_c cancels, then drops the gate, and the data thread's freeze body and `StopProviders` follow.
- Asserts: the horizon `Cancel` has `held.freeze == true`. Line order `cancel:released(horizon)` < `Serialised` < `radio:stop`. The count is exactly one.
- Wrong change: N111-21 gives `held.freeze == false` at the `Cancel`, which is an assertion regardless of the race.
- Deschedule: for the fixed code the order is forced by the gate.

## 4. The sweep (T11–T15)

One helper, `sweep(run_fn, required)`, called by four tests so a failure names its Run. Each asserts all of the following.

1. **The rule on every call.** No `Call` of `now`, `schedule`, `cancel`, `wait_until`, `utc_nanos`, `monotonic_millis`, `next_wakeup` or `relations` has `held.admission || held.machine`. The record is per thread, so the data thread's calls during the control thread's start-batch hold are no false failure.
2. **Every transition pair.** For each transition after Created: a `Now` call immediately followed by a `Utc` call on the same thread. The `Utc` count equals the transition count, and the samples equal the manifest's `host_utc_nanos`, in order.
3. **Per-Run positive phase proofs.** Windows are delimited by the transitions' `Utc` calls.
   - **Running window:** after Running's `Utc` call and before Stopping's.
   - **Stopping window:** after Stopping's and before CleanedUp's.

| Test | Run | Required |
|---|---|---|
| T11 | Simulation, orderly: a timed dispatch, a Module submission, `finish`; one far-future stop (H) | `Schedule` of H in the Armed window; exactly one `Cancel` of H in the Stopping window; the Provider's `took` line; `x:submit:ok` |
| T12 | Simulation, explicit abort (a Module `Abort` during Running); H as in T11 | as T11, with the Stopping record `Stopping{abort}` and `reason == Stopped{Abort}` |
| T13 | Paced, device loss with H. `finish` is gated on the data thread's wake `Schedule` **return** (handle filled), after freeze #1's `Done` | `Cancel` of H in the Running window (data thread); the wake `Schedule` in the Running window; `Cancel` of the wake handle in the Stopping window; at least one `Now` by the data thread |
| T14 | Paced, orderly `finish`, no loss; H | exactly one `Cancel` of H in the Stopping window |

**T15 `kc_24a_every_admission_and_machine_lock_goes_through_the_recording_helpers`** is a source scan in the style of `tm_06_no_crate_reads_ticks_in_their_own_domain` (`time_model.rs:494`). It reads `src/coordinator/*.rs` for `.admission` or `.machine` reached by a raw `lock(`, `.lock(` or `try_lock` outside the three helper bodies. `Probe::read`'s `try_lock` is allow-listed with a one-line reason.

**What the sweep cannot see:**
- Locks other than the three: `end`, `also`, `delivered`, `links` (#121), `wake_handle` across `schedule` (#120; T4 covers the freeze side), slot locks.
- Paths no Run drives. A new call on an undriven path passes.
- Supplied objects other than the Authority's time handle and the HostClock. Checks and `coerce` run under `admission` by design, so the fixtures' doubles must call no time inside them.
- Guards bypassing the helpers. T15 covers text only.
- The deadlock itself: the sweep checks its precondition.

## 5. Mutation rows

New rows go in `plan/phase7/tools/mutations.json`, the list that holds N103 and G29. Anchors are Rust tokens in `crates/ezsdr-kernel/src/coordinator/`. Anchors marked with a cfg line include it, as N103-1 did.

| ID | File | Mutation | Killer (named assertion) |
|---|---|---|---|
| N111-1 | `ending.rs` | delete `drop(admission);` | T1 (`Cancel` reading) |
| N111-2 | `ending.rs` | move the `let wake = …take();` and its hook above `drop(admission);` | T4 (`BeforeWakeTake` reading); T1 also |
| N111-3 | `ending.rs` | `drain(..)` → `.clone().into_iter()` | T6 (H count) |
| N111-4 | `ending.rs` | `.is_err()` → `!matches!(…, Ok(true))` | T5 (`cleanup_failures` empty) |
| N111-6 | `ending.rs` | `let wake: Option<ScheduleHandle> = None;` | T2a (wake count 0) |
| N111-7 | `ending.rs` | Stopping: `at = now()` then `lock_machine().begin_stopping(.., &*host_clock)` | T7 |
| N111-8 | `ending.rs` | Stopping: `utc` read first, then `move_to_at(.., Some(self.shared.now()), utc)` | T7 |
| N111-9, -10 | `ending.rs` | the same two mutations for CleanedUp | T7 |
| N111-11 | `ending.rs` | delete the `lock_freeze()` line in the freeze | T6 (`held.freeze`) |
| N111-12 | `ending.rs` | drop the gate right after `drop(admission)` | T6 (`held.freeze`) |
| N111-13 | `ending.rs` | `CleanupStep::FreezeDispatch if frozen => Ok(())` | T10b (count 0) |
| N111-14 | `ending.rs` | move the `frozen.store(true, …)` line below `drop(admission)` | T1 (`BeforeDrop`) |
| N111-15 (retargeted) | `stepping.rs` | delete the new `if frozen … { break; }` in `dispatch_agenda`. The old post-`next_wakeup` anchor is dropped. | T9 (`also`/mode) |
| N111-16 | `stepping.rs` | `if removed && cancel…` → `if cancel…` | T10a (count 2) |
| N111-17 | `admission.rs` | a `let _ = shared.now();` in `dispatch` (under admission) | T11 |
| N111-18 | `pipeline.rs` | the same in `Submitter::submit` (Module path) | T11 |
| N111-19 | `stepping.rs` | `dispatch_agenda` uses a raw `lock(&…admission)` | T15 |
| N111-20 | `state.rs` | `publish` reads `Some(self.now())` under the machine lock | T12 |
| N111-21 (new) | `stepping.rs` | delete `let _gate = self.shared.lock_freeze();` in the withdrawal | T10c (`held.freeze`) |

Existing rows:
- **G29** is re-anchored. `old` is `let admission = self.shared.lock_admission();\n self.shared.frozen.store(true, Ordering::Release);`, `new` is the `frozen` line alone, and `old2` is `drop(admission);` with an empty `new2`. Its killer, `kg_04_no_action_is_dispatched_after_the_freeze`, is unchanged, including its 200 ms gate (not this step's).
- **J95, J111, J193, J198** keep their tokens. Confirm with `rules.py`.
- N103-10 and -11's killer migrates with `ClockEvent`'s `Held`.

I checked each row for a named assertion kill:
- **N111-1, -2, -11, -12, -14, -21:** they die on captured readings. Their mutated code can block, but the readings are recorded before blocking, and the tests' release paths (T4's explicit mutant path, T6's gate opened after the hook event, T10c's release after `Serialise`) never wait on the mutant.
- **N111-3, -13, -16:** counts.
- **N111-4, -15:** a cleanup failure and `also`/mode, all on the manifest.
- **N111-6:** a count of 0.
- **N111-7 to -10, -17, -18, -20:** per-call readings.
- **N111-19:** the source scan.

**No row needs `"kill": "bound"`.** The 10 s waits are backstops reached only on harness failure.

## 6. Spec text (spec 22 DA-1 format: normative, a `*Test:*` line, a Changes row each)

**`design/06-kernel-coordinator.md` KC-24a.** After "(RS-16).", replace the freeze clause and add the rule:

> RS-6 step 1 holds it only while it sets `frozen`, empties the queues and records the discarded ids in `termination.undelivered` (RS-16); it releases it before it takes the handles it cancels and before it cancels them. The coordinator calls no `TimeAuthority` method and no `HostClock` method while it holds the admission lock or the machine lock: a value it records is read before the lock, and a handle it cancels is taken under its own lock and cancelled after every lock but the one that serialises the performances of RS-6 step 1 and the control loop's own withdrawal (KC-29b), which no Module takes.

Replace the Ceiling:

> *Ceiling:* a registered check (KC-24 step 13; SB-31's purity) and a Provider's `coerce` (KC-26) run under the admission lock, and `Provider::stop` runs under its slot lock; a `coerce` that waits for a mutex held by a Provider thread blocked in `submit` waits on itself. The rule above does not cover them. MockRadio and the UHD Provider submit from no thread of their own.

**KC-20.** After the list: "An agenda item whose turn comes when dispatch is already frozen (RS-6 step 1) is not admitted, and that is not a failure."

**KC-43.** Append: "Each transition's two reads are taken before the machine lock (KC-24a)."

**KC-46a.** Add T2b to its `*Test:*` line. No new sentence.

**`design/04-run-and-session.md` RS-6 step 1:**

> **1** under the admission lock (spec 06 KC-24a) set `frozen`, empty every undelivered Action queue and record the ids of the Actions it discards in `termination.undelivered` (RS-16); then, with that lock released, cancel every callback the coordinator holds a handle for, once each. A second performance of step 1, by either thread (spec 06 KC-46b), waits for the first to finish its cancels, and so does the control loop's withdrawal of its own horizon (KC-29b), unless RS-8a abandons the waiting performance; step 2 therefore does not start while a cancel that step 1 took, or a withdrawal, is outstanding. An `Abort` is not an Action subject to admission (KC-24): it is accepted throughout and escalates the cleanup (RS-10).

*Test:* lines name the tests that assert each property: T1 and T4 for the lock extent, T6 and T10c for the cancel-before-stop order, T7 for KC-43's sampled UTC, T9 for KC-20, T3 for the refusal and id. Add Changes rows citing issues #111 and #119. Run `rules.py --base <pre-change commit>`.

No `KNOWN_LIMITATIONS.md` change, no version bump.

## 7. Open questions (recommendations)

1. Keep the `freeze_gate` helper `lock_freeze` under the withdrawal as well? Recommended, as in section 1.
2. Keep T2b? Recommended; KC-46a states the late wake.
3. Migrate #103's test to `Held`? Recommended, for one lock representation.
4. #120 and #121 stay filed and unfixed. Section 4 and the Ceiling state the limits.
5. `kernel_surface`: the new public `lock_record` and `start_probe` additions need the same treatment as `start_probe` (no allow-list entry). Verify with the surface test.

**Not checked:**
- I ran no build and no `rules.py`.
- I did not confirm that `ThreadedProvider`'s `radio:stop` line carries the stop mode. T8 and T9 extend it if not.
- The resource-targeted `stop` fixture is accepted by reading only. The implementer should confirm it with a first parse test.

Key files:
- `~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/src/coordinator/ending.rs`
- `~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/src/coordinator/stepping.rs`
- `~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/src/coordinator/state.rs`
- `~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/tests/support/start_window.rs`
- `~/local_work/Ez-SDRv4-n111/plan/phase7/tools/mutations.json`