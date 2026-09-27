# Phase 7 spec 19 — Amendments: a coordinator for device-paced Runs, and what a radio Provider on hardware needs

| Field | Value |
|---|---|
| Status | **Accepted at Gate P** (owner, 2026-09-27, as recommended; [`00-overview.md`](00-overview.md) §11); **implemented** on 2026-09-27 (the owner: "Phase 7を実装してください"; 00-overview §9 steps 1–6, [`design-notes.md`](design-notes.md) §6), not yet reviewed (Review L); earlier the same day: "実装はしないで". Reviewed by Review J and revised for every finding; re-reviewed by Review K ([`design-notes.md`](design-notes.md) §4, §5). Its text reaches `design/` in the commit that implements each amendment (GZ-7); this file stays as the record. |
| Scope | Fourteen Kernel amendments, KG-1…KG-14 (the coordinator drives HardwareInLoop and Hardware; the UHD spike's K1–K3, K5, K6, K8, K12; MA-8; TM-18's relations; child Runs on hardware; UC-3 on a device that restarts later; TM-13b and TM-13e for a device). Six Vocabulary, Module and frontend amendments, VE-1…VE-6 (`radio` 1.3.0, `ezsdr.radio.mock` 1.3.0, the server, the Python package). |
| Amends | `design/01-time-model.md` (TM-13b, TM-13e, TM-16c, TM-18); `design/04-run-and-session.md` (RS-6's coordinator table); `design/05-module-api.md` (MA-8, MA-9a new, MA-10, MA-14b new, MA-20, MA-29, MA-30, MA-46, UC-3, UC-6); `design/06-kernel-coordinator.md` (KA-12's table, KC-2, KC-2a new, KC-12a new, KC-15, KC-21a new, KC-24a new, KC-29, KC-30, KC-31, KC-33, KC-36, KC-37a, KC-45, KC-46…KC-46c new); `design/07-radio-model.md` (RM-1, RM-6, RM-10, RM-11, RM-14, RM-16, RM-20, RM-22, RM-25 new, RM-26 new); `design/09-mock-radio.md` (MR-1, MR-4, MR-6, MR-9, MR-18); `design/16-easy-api.md` (EA-4, EA-7, EA-9, EA-12, EA-14, EA-16, EA-17); `plan/phase2/00-overview.md` (PO-2, PO-11, through GZ-3 and GZ-4 of the overview). |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

Each amendment gives the problem, the rule text as it reads after the amendment, the rejected alternatives, the code, and the tests. Appendix A lists the mutations that must show each test guards its rule (GZ-6). Test names are the ones the implementation must use; the doubles `WallAuthority` and `ThreadedProvider` are specified in §0.

**Terms.** A **device-paced class** is HardwareInLoop or Hardware (MA-41: an Authority whose pacing is `Device`). The **control thread** is the thread that calls `RunHandle` methods; the **data thread** is KC-46's.

---

## 0. The Kernel test doubles

Both live in `crates/ezsdr-kernel/tests/support/run_doubles.rs` (test code: PO-11 does not scan it).

**`WallAuthority`** — an `Authority` with `pacing: Device` whose primary root is a fresh 1 GHz root counting nanoseconds of `std::time::Instant` since the double was built. `now(root)` reads the host clock; `now(host.monotonic)` is the same count (the root *is* host time here, so `now` never goes backwards). `schedule` follows TM-16c as KG-9 amends it for a paced Authority: it refuses only an instant before the last instant `next_wakeup` fired (`InPast`), and an instant at or before `now` is accepted and fires at the next `next_wakeup`; callbacks are kept in a `BTreeMap<(tick, seq)>` under a mutex, and `schedule` and `cancel` both notify a condvar. `next_wakeup` returns `None` at once when nothing is scheduled; otherwise it waits on the condvar until the host clock reaches the earliest instant — re-evaluating whenever it is notified, so an earlier `schedule` shortens the wait and a `cancel` that empties the map makes it return `None` (MA-29 as KG-3 amends it) —, marks that instant fired, runs every callback due at it up to TM-17b's cap, and returns it. `relations()` returns two relations measured when the double was built — to `host.monotonic` (exact: the root counts host nanoseconds; `uncertainty` 0, method `test.wall`) and to `utc` (`offset` from `SystemTime`, `uncertainty` the bracketing interval, `drift` 0, `drift_uncertainty` 1e-4, method `test.wall_utc`) — unless built with `.without_relations()`. It governs the root, its derived domains and `host.monotonic`.

**`ThreadedProvider`** — a Provider of the `test` Vocabulary (MA-44's tree) with `driving.stepped: false`. Its `start` spawns one thread that, every millisecond until `stop`: drains its `ActionReceiver` with the MA-14b loop, recording each Action with the host instant it finished and, when built `.with_action_delay(d)`, sleeping `d` inside each; publishes one block of `block_len` samples on every attached `rx` link every `period` (built `.publishing(block_len, period)`), on a SampleClock it registers at `start` at T0; emits the event kind `test.MARK` once at `start + mark_after` (`.marking(after)`); emits `DEVICE_LOST` from its thread at `start + lost_after` (`.losing_device(after)`, MA-9a). `.with_prepare_delay(d)` and `.with_arm_delay(d)` sleep in those calls; `.with_stop_tail(n)` publishes `n` more blocks inside `stop` before it returns; `.never_finishing()` stops calling `recv()` after it has taken one Action; `.draining_after_stop(d)` makes `stop` keep calling `recv()` for `d` before it joins, recording every Action received after `stop` began. On `UpdateParameter { key: test.tx_clock, value: Int(n) }` it declares and registers a transmit SampleClock for `<device>/tx` with `root_ticks_per_tick` `n` at the first lattice instant at or after now (RM-25's shape, used by KG-4's tests); the test Vocabulary (`tests/support/doubles.rs` `test_vocabulary`) gains that key, `int`, not coercible, update class `cold`, for it. `stop` joins the thread (with a 1 s bound) and records the call and every Action it received after `stop` began (there must be none); `cleanup` records the call. Its call log is shared through a `Probe` (the existing probe type) so tests can read it while the Run runs.

**`RecordingSink`** (existing) also records, for each `step`, the thread it ran on and whether the Run's `stop` of every Provider had been recorded before it; `.always_progressing()` makes every `step` report `progressed` (used by KC-46's test of the pass); `.failing_step_at(t, kind)` makes its first `step` at or after `t` return an error of that kind.

---

## KG-1 — The coordinator drives the device-paced classes

**Problem.** KC-2 refuses every class but Simulation (`coordinator/pipeline.rs:372-380`), so no Run with a device timekeeper can start. The spike removed the refusal for `Device` pacing and no test failed (K1): nothing but the refusal stood in the way, and nothing else pinned it. Two things a device-paced Run cannot honour must be refused instead of ignored: a stepped Provider (MA-30's table steps none in these classes, so its `step` would never be called), and an Island's affinity or real-time policy (MA-38 declares them; nothing applies them before Phase 10's threaded Executors).

**KC-2** (replaced):

> The coordinator drives the Simulation, HardwareInLoop and Hardware classes. HardwareInLoop and Hardware are the **device-paced classes**, to which KC-2a, KC-12a, KC-21a, KC-24a, KC-46…KC-46c and the device-paced clauses of KC-29, KC-33, KC-36, KC-37a and KC-45 apply. A plan whose derived class is RealtimeEmulation is refused after `plan()` with `Failed { plan }` and the reason `"KC-2: RealtimeEmulation needs a wall-paced Simulation Engine, which no phase has built yet (Phase 10)"` (Phase 7, KG-1).

**KC-2a** (new):

> In a device-paced class the coordinator also refuses, after `plan()` and before KC-9, with `Failed { plan }`: a Provider instance whose `instance().driving.stepped` is true, with the reason `"KC-2a: <first fragment> is a stepped Provider, and a device-paced class steps none (MA-30)"`; and an Island whose `IslandDecl` has `affinity` or `rt_policy`, with the reason `"KC-2a: <island id> declares <affinity | rt_policy>, which a device-paced class does not apply before Phase 10"`. The first offending instance in plan order is named (Phase 7, KG-1).

**Rejected.** Admitting RealtimeEmulation too (it needs a wall-paced Engine; the Simulation Engine would silently run free-running under a `realtime_emulation` label). Ignoring a stepped Provider (it would never run, silently). Applying affinity best-effort (a hint the Kernel does not implement would read as honoured).

**Code.** `pipeline.rs`: the KC-2 branch keeps only `RealtimeEmulation`; a `device_paced()` helper on `Shared` (`routing.plan.class` is `HardwareInLoop | Hardware`); the KC-2a checks right after `routing` is installed.

**Tests** (`crates/ezsdr-kernel/tests/coordinator.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `kc_02_a_wall_paced_authority_is_refused` (kept; its reason changes) | the existing WallPaced case | `Failed { plan }`, reason starting `"KC-2: RealtimeEmulation"`, class RealtimeEmulation | KC-2 |
| `kg_01_a_device_paced_run_reaches_running` | `WallAuthority` as `authority`, a `ThreadedProvider` resource, `ezsdr.rf_path: cabled`, `ezsdr.time: { class: hardware_in_loop, start_lead_ns: 10 000 000 }` | `Running` after entry; after `finish`, the Manifest's `execution_class` is `HardwareInLoop` and `deterministic` false; the Provider's log is `prepare, arm, start, stop, cleanup` | KC-2, RS-42 |
| `kg_01_the_hardware_class_reaches_running` (Review J) | the same with `ezsdr.rf_path: over_the_air`, `class: hardware` | `Running`; the Manifest's class is `Hardware` | KC-2 |
| `kg_01_a_stepped_provider_is_refused_in_a_device_paced_class` | the same profile with the stepped `SteppedProvider` | `Failed { plan }`, reason `"KC-2a: radio is a stepped Provider…"`; `prepare` never called | KC-2a |
| `kg_01_an_island_with_an_rt_policy_is_refused` | a device-paced Run with a `ProbeExecutor` Island declaring `rt_policy`; again with `affinity` | `Failed { plan }` naming the Island and the field, both times | KC-2a |

---

## KG-2 — The data thread

**Problem.** MA-30's table runs Executors and Sinks "on threads" in HardwareInLoop and Hardware, but the coordinator steps every instance only in a round, and rounds run only inside `RunHandle` calls. A device publishes blocks whether a client calls or not, so between calls a link fills and drops (spike K8: 298 drops after a capture), and the events a Sink emits reach the Policy only at the next call. A `wait_for` in a device-paced class would sleep in `next_wakeup` until its horizon even when the awaited event arrived earlier, because in these classes nothing else schedules a wakeup. And a Detached Lease is checked only inside calls (KC-36, the Lease being a field of `RunHandle`), so a client that detaches and never calls again leaves a transmitter on past its TTL indefinitely — RS-22's own counterexample, on hardware (Review J, P1-3).

**KC-46** (new):

> In a device-paced class, from the moment the Run enters `Running` (KC-18, before the start dispatch) until RS-6 step 3 stops it (KA-12 as amended by KG-3), a **data thread** of the coordinator repeats a **pass**: it takes `t`, the current instant on the primary root; steps every Executor and every Sink once, `step(t)`, in MA-30's order (role rank, then instance id), skipping an instance whose step-5 `cleanup` has been called and an instance whose `step` has failed earlier in the Run, each call contained and attributed as KC-30 says for a round; handles the pass's failures as KC-30 handles a round's (the first decides; every `DeviceLost` is reported), so that an instance's failure is reported once; drains the collector and applies the reactions (KC-31); checks the Lease's expiry deadline (KC-36); then acts on an end it requested in the pass (KC-46b); and wakes the control loop (KC-46a) when the drain delivered at least one event. When at least one instance reported `progressed`, the next pass starts at once; otherwise the thread parks for 200 µs (`DATA_IDLE_PARK`, private). The data thread calls no `step_until_quiescent`: the device's clock moves on between passes, so repetition — not quiescence at one instant — is what drains a stream, and a Sink that finds a new block on every pass under a high block rate would reach MA-30's cap, which guards against a livelock at one instant. A pass's latency, and so how soon the Policy reacts to an event, is bounded by its slowest step (a capture Sink hashing a finished file, for example). In a device-paced class the control thread's rounds (KC-20, KC-21, KC-18's round) step no instance — no Provider is stepped (KC-2a) and the data thread steps the rest — so no instance is ever stepped by two threads; they still run the agenda, drain the collector, apply reactions and check the Lease (Phase 7, KG-2; after Review J, P1-3, P1-10, P2-5, P2-10). *Checked by `kg_02_the_data_thread_drains_a_sink_while_no_call_runs`, `kg_02_every_step_before_finish_runs_on_the_data_thread`, `kg_02_the_simulation_class_steps_on_the_callers_thread`, `kg_02_a_sink_that_always_progresses_does_not_livelock` and `kg_02_a_failed_sink_is_not_stepped_again`.*

**KC-46a** (new):

> The data thread **wakes** the control loop by scheduling a no-op callback on the Authority at the current instant — an instant a paced Authority accepts however far its clock has moved on (TM-16c as KG-9 amends it) — unless a wake it scheduled has not fired yet. Pending wakes are counted by generation: before calling `schedule` the data thread stores the next generation `g` as the pending one, and the callback clears the pending generation only if it is still `g`, so a callback that fires before `schedule` has even returned still clears its own wake, and a later wake is never mistaken for an earlier one. The pending wake's handle is kept in `Shared` on its own, not among the handles `advance_to` schedules, and RS-6 step 1 cancels it with every other scheduled callback. A control loop waiting in `next_wakeup` then returns and runs a round, whose end check (KC-29) and `wait_for` match check (KC-29b) see what the data thread requested and delivered. A wake the Authority refuses (a panic or an error, which a conforming paced Authority does not return) clears its generation and is not retried; the control loop sees the state at its next wakeup (Phase 7, KG-2; after Review J, P0-1, P1-1, P2-3, and Review K, N-P1-1). *Checked by `kg_02_wait_for_returns_when_the_data_thread_delivers` and `kg_02_every_delivery_wakes_a_waiting_call`.*

**KC-31** (sentence added):

> The drain, the reactions and the append to `delivered` happen under one lock, so that events drained by the data thread (KC-46) and by the control thread are appended in drain order, index `i` names one event for the whole Run (KC-29a), and the Policy reacts in the order of `delivered`: when two events of one Run each request an end, the Termination's cause is the first of them in `delivered` (KC-32) (Phase 7, KG-2). *Checked by `kg_02_the_first_delivered_stopping_event_is_the_termination_cause`.*

**KC-36** (sentence added):

> In a device-paced class the Lease's expiry deadline, in the host clock's monotonic milliseconds as `Lease::expires_at_host` keeps it, is kept in `Shared` as well as in the handle, and is updated whenever the Lease changes (`disconnect` starting a Detached Lease's TTL, `Renew`, `Adopt`, `Release`); the data thread compares it with `host_clock` after every pass and, when it has passed, requests the end `Stopped { lease_expiry }` in orderly mode, on which it acts as KC-46b says (Phase 7, KG-2; Review J, P1-3). *Checked by `kg_02_a_detached_lease_expires_while_no_call_runs`.*

**MA-30** (the table's last row and a sentence after the table):

> | HardwareInLoop, Hardware | none | the data thread (KC-46) | the data thread (KC-46) |
>
> In the device-paced classes the coordinator's data thread steps every Executor and Sink in the fixed order above, one pass at a time and without the cap (KC-46); the Islands' declared affinity and real-time policy are Phase 10's, and until then KC-2a refuses an Island that declares either (Phase 7, KG-2).

**MA-20** (sentence added): "In a device-paced class `until` is the instant the data thread's pass took, and an event a step stamps with the current instant may lie after it, because the device's clock does not stop for a pass, as it does not for a callback (TM-16c) (Phase 7, KG-2; Review J, P2-7)."

**PO-11** is amended by the overview's GZ-4 (`coordinator/paced.rs` may spawn threads and read the host clock; it runs only in device-paced classes).

**Rejected.** A thread per Executor and per Sink (the order between two instances would become a race, and no Phase 7 Run has two); stepping from client calls only (K8); `step_until_quiescent` on the data thread (above); a wake by a condition variable the control loop waits on (the control loop waits inside the Authority's `next_wakeup`, which only a scheduled or cancelled callback can end early; a second wait primitive would need a second loop); refusing Detached Leases in device-paced classes (a Session a client may leave running for its TTL is exactly what a lab needs, and the data thread already runs after every pass).

**Code.** New private module `coordinator/paced.rs`: `start_data_thread(&Arc<Shared>)`, `stop_data_thread(&Shared)` (sets the stop flag, unparks, joins), `wake(&Shared)`, the pass (a copy of `round`'s wrapping over Executors and Sinks, without `step_until_quiescent`, with a `failed: BTreeSet<Inst>` of its own). `Shared` gains `data: Mutex<Option<(Arc<AtomicBool>, JoinHandle<()>)>>`, `wake_pending: Arc<AtomicU64>` (0: none) with `wake_next: AtomicU64`, `wake_handle: Mutex<Option<ScheduleHandle>>`, and `lease_deadline: Mutex<Option<u64>>` (the Lease's `expires_at_host`, in the host clock's monotonic milliseconds, as `Lease` keeps it, `run.rs:324`). `stepping::round` gains a parameter saying which instances it steps: all (Simulation, and the Simulation drain), none (a device-paced control round), Executors and Sinks (KG-3's final round). `drain_and_react` holds the `delivered` lock across `collector.drain()`, the reactions and the append. `pipeline.rs` starts the data thread right after `move_to(Running)`; `RunHandle`'s Lease-changing paths update `lease_deadline`. The thread holds an `Arc<Shared>` and also exits when it is the last holder (a handle dropped without cleanup, KC-46c).

**Tests** (`coordinator.rs`, with the §0 doubles). Wall-clock margins: a factor of two on durations, and counts read while the Run runs.

| test | input | expected | rules |
|---|---|---|---|
| `kg_02_the_data_thread_drains_a_sink_while_no_call_runs` | a device-paced Spec Run: `ThreadedProvider` publishing 100-sample blocks every 1 ms into a `drop_oldest` link of capacity 64 feeding `RecordingSink`; the test sleeps 200 ms without calling the handle, reading the Sink's block count through the probe at 100 ms and at 200 ms | the count grows between the two reads by at least 50 blocks; after `finish`, the link's `drops` in `ezsdr.links` is 0 | KC-46 |
| `kg_02_every_step_before_finish_runs_on_the_data_thread` | the same Run; the Sink records each `step`'s thread | every `step` recorded before `finish` was called ran on one thread, not the test's (cleanup's step 3 runs its round on a `run_cleanup` step thread, KG-3, and is excluded) | KC-46 |
| `kg_02_the_simulation_class_steps_on_the_callers_thread` | a Simulation Run with `RecordingSink` | every `step` recorded before `finish` ran on the test thread | KC-46, GZ-4 |
| `kg_02_a_sink_that_always_progresses_does_not_livelock` (Review J, G07) | a device-paced Run with `RecordingSink.always_progressing()` for 200 ms | the Run is `Running`; no `STEP_LIVELOCK` delivered; the Sink stepped more than 1 000 times | KC-46 |
| `kg_02_a_failed_sink_is_not_stepped_again` (Review J, P1-10) | `RecordingSink.failing_step_at(20 ms, DeviceLost)` with the Policy's `DEVICE_LOST` reaction overridden to `continue` | the Sink's `step` count stops at the failing step; exactly one `DEVICE_LOST` from `sink/<id>` delivered over 200 ms | KC-46, KC-30 |
| `kg_02_wait_for_returns_when_the_data_thread_delivers` | a device-paced Session whose `ThreadedProvider` emits `test.MARK` 30 ms after start; `wait_for([test.MARK], 0, now + 5 s)` | `Ok(Some(i))`, returned within 1 s of wall time, `now()` well before the horizon | KC-46a, KC-29b |
| `kg_02_the_first_delivered_stopping_event_is_the_termination_cause` (Review J, G09; Review K) | two `ThreadedProvider`s marking kinds whose Policy reaction is `stop` at the same instant, in 50 fresh Sessions, while the test runs a loop of `wait_for` calls with 1 ms horizons, so that control-thread rounds drain concurrently with the data thread | in every Session the Termination's cause is the kind of the first stopping event in `events.delivered` | KC-31, KC-32 |
| `kg_02_every_delivery_wakes_a_waiting_call` (Review K, N-P1-1) | a device-paced Session whose `ThreadedProvider` marks `test.MARK` 50 times, 10 ms apart; 50 sequential `wait_for([test.MARK], from, now + 5 s)` calls | each returns its mark within 200 ms of wall time; none reaches its horizon | KC-46a |
| `kg_02_a_detached_lease_expires_while_no_call_runs` (Review J, P1-3) | a device-paced Session with a Detached Lease of TTL 50 ms; `disconnect()`, then no call for 500 ms | the Provider's `stop` recorded within 500 ms; `finish` returns `Stopped { lease_expiry }` | KC-36, KC-46b |

---

## KG-3 — An end requested off the control path stops the radios at once

**Problem.** With KG-2 the data thread applies the Policy while no client call runs. But ending a Run is the control thread's (KC-38 runs cleanup inside a `RunHandle` call), and the Session log, the Lease and the Manifest are behind `&mut RunHandle`. Without more, a `DEVICE_LOST`, a `CLOCK_LOST`, a `COMMAND_QUEUE_FULL` or an expired Lease that the data thread turns into an end leaves the transmitters on until the client calls again, which may be minutes later.

**KC-46b** (new):

> After a pass in which the data thread itself requested an end (a step failure, KC-30; a reaction, KC-31; an escalation, RS-36; an expired Lease, KC-36), it performs, at most once per Run and on its own thread, RS-6 step 1 and then step 2 for every fragment in the reverse order, through the coordinator's cleanup operations (KC-39) with the mode then pending, and then wakes the control loop unconditionally. Each step it performs is recorded as done (KC-39), so when the control thread later runs cleanup (KC-38) `run_cleanup` performs every step as usual and acts on no instance twice; RS-6 step 3 joins the data thread before any Executor or Sink stops (KA-12 as amended below), so a `stop` the data thread is still making finishes first. Under `abort` the data thread then stops stepping and ends; under `orderly` it goes on stepping Executors and Sinks, so that the tail the Providers deliver in step 2 reaches them. How soon this happens after the event is bounded by the pass's slowest step (KC-46). While the data thread is inside a Provider's `stop` — bounded for the UHD Provider by its joins, about 3 s at worst (spec 18 UR-16, UR-26), and by nothing the Kernel enforces for another Provider — it steps no Sink, drains no event and checks no Lease, so an orderly end the data thread requested (an expired Lease, a Policy `stop`) may lose receive samples, counted by the drop-class links, that an end requested by a client call would have delivered (the trade-off of doing without a separate thread, Review K, N-P2-4). The Run stays `Running` until the control thread's cleanup moves it to `Stopping`, whose transition records that instant; each Provider records in its own sections when it stopped (Phase 7, KG-3; after Review J, P1-1, P1-10, P2-9). *Checked by `kg_03_a_fatal_event_stops_the_providers_without_a_client_call`, `kg_03_a_step_done_early_is_not_repeated` and `kg_03_an_abort_stops_the_data_thread`.*

**KC-46c** (new):

> In a device-paced class, dropping a `RunHandle` whose Run is not `CleanedUp` runs cleanup as `finish` does (KC-34), with `Stopped { client }` in orderly mode, and discards the Manifest; this joins the data thread. In the Simulation class dropping a handle does what it did before (nothing) (Phase 7, KG-3). *Checked by `kg_03_dropping_a_live_device_paced_handle_cleans_up`.*

**MA-29** (sentences added): "A `WallPaced` or `Device` Authority's `cancel` wakes a `next_wakeup` that is waiting, which then re-evaluates: it goes on waiting for the earliest instant still scheduled, or returns `None` when none is. It runs its callbacks without holding a lock that `schedule` or `cancel` takes, so a callback may schedule and another thread may schedule while one runs (Phase 7, KG-3; Review J, P1-1; Review K, N-P2-3)." *The Kernel has no paced Authority of its own; the rule's carriers are spec 18's `ur_07_cancel_wakes_a_waiting_next_wakeup` and the `WallAuthority` double, which follows it.*

**KC-29** (the last sentence amended): "… and before returning runs cleanup when an end has been requested (KC-38), so that no end request is left pending across calls — except, in a device-paced class, an end the data thread requested while no call was running: the radios stop at once (KC-46b), and the rest of cleanup and the Manifest follow at the next call, until which `state()` reports `Running` and `events()` already shows what ended the Run (Phase 7, KG-3; Review J, P2-6)."

**RS-6 / KA-12's coordinator table** (step 3, sentence added):

> In a device-paced class step 3's drain is instead: stop and join the data thread (KC-46), if it exists; then, under `orderly`, one `step_until_quiescent` at the current instant over every Executor and Sink whose step-5 `cleanup()` has not been called — the Providers stopped in step 2, so what their tails published is in the links and the round is finite — and under `abort` none; then `Executor::stop` and `Sink::stop` as before (Phase 7, KG-3). *Checked by `kg_03_step_3_runs_a_final_round_over_the_sinks`.*

**Rejected.** Waiting for the client's next call (an unbounded delay with a transmitter on); running the whole cleanup on the data thread (it would need the control-path state behind `&mut RunHandle`, and a client call arriving meanwhile would race it); a separate stopper thread (the first draft: a thread and a join more, for a tail that the drop-class links hold while the data thread is inside a `stop` — Review J, P2-9); stopping only transmitters (RM-16 already orders transmit before receive inside `stop`).

**Code.** `paced.rs`: after a pass, when `end` became `Some` during it by this thread's request (the request function returns whether it set the end), it builds the same `Ops` value `ending.rs` gives `run_cleanup` and calls `perform(FreezeDispatch, None, mode)` and `perform(StopTx, Some(f), mode)` for each `f` of `routing.reverse`, then `wake` with the pending check skipped. `ending.rs` `FreezeDispatch` also cancels and clears `Shared.wake`; `StopRx`: the device-paced drain above. `impl Drop for RunHandle` (device-paced only).

**Tests** (`coordinator.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `kg_03_a_fatal_event_stops_the_providers_without_a_client_call` | a device-paced Session; `ThreadedProvider` `.losing_device(20 ms)` (MA-9a); the test waits on the Provider's probe for `stop`, without calling the handle | `stop` recorded within 500 ms; then `finish` returns a Manifest with `Stopped { policy { kind: DEVICE_LOST } }` and `stop` exactly once in the call log | KC-46b, MA-9a |
| `kg_03_a_step_done_early_is_not_repeated` | a device-paced Spec Run whose `RecordingSink` fails its `step` at 20 ms (`Failed { run }`); two `ThreadedProvider`s | each Provider's `stop` once; `cleanup` once; `run_cleanup` records no failure for step 2 | KC-46b, KC-39 |
| `kg_03_an_abort_stops_the_data_thread` (Review J, P1-10) | the `DEVICE_LOST` case of the first row | the Sink records no `step` after the Providers' `stop` | KC-46b |
| `kg_03_step_3_runs_a_final_round_over_the_sinks` (Review J, G12) | an orderly `finish` of a device-paced Spec Run whose `ThreadedProvider` has `.with_stop_tail(2)` | the Sink records at least one `step` that ran on a thread other than the data thread's, after every Provider's `stop`, and before its own `stop`; the two tail blocks were received | KA-12 step 3 |
| `kg_03_a_cancelled_horizon_wakes_the_control_loop` (Review J, P1-1) | a device-paced Spec Run in `run_until_end(now + 5 s)`; a `ThreadedProvider` `.losing_device(20 ms)` | `run_until_end` returns `Err(Ended)` within 1 s of wall time | MA-29, KC-46a, KC-46b |
| `kg_03_dropping_a_live_device_paced_handle_cleans_up` | a device-paced Session handle dropped without `finish` | the Provider's `stop` and `cleanup` are recorded; the Sink records no `step` after `cleanup` | KC-46c |

---

## KG-4 — A threaded instance has finished with its Actions before the call returns

**Problem.** KC-21: "After the coordinator dispatches any Action on the control path … it runs one `step_until_quiescent` at the current instant … The target therefore sees the Action at the instant it was admitted." For a stepped Provider that round is where a `cold` change is applied and a new transmit SampleClock is registered, so the next Action of the same Session is admitted against it. A Provider that is not stepped applies the change on its own thread, after `submit` returned, and the next admission races it: the spike's `start_repeat` right after `radio.tx.channels = 1` was refused with "SC-23: usrp/tx has no running transmit SampleClock" (K5). With KG-2 the same holds for a Sink's capture request in a device-paced class. And with KG-2 and KG-3, admission is no longer on one thread: the data thread freezes dispatch (RS-6 step 1) and a Module on it may submit while a control call admits — `admit_with` checks `frozen` at entry and `dispatch` pushes without looking again (`admission.rs:37-42`, `:246-256`), so an Action admitted just before a freeze could be pushed after the queues were cleared (Review J, P1-2).

**MA-14b** (new):

> An instance that the control thread does not step — a Provider whose `driving.stepped` is false, and in a device-paced class every Executor and Sink (KC-46) — has **finished** with every Action it took from its `ActionReceiver` when it next calls `recv()`. It must therefore carry out each Action it takes, or emit the event that refuses it (MA-14), before its next `recv()`, and having taken an Action it must call `recv()` again without waiting for another to be dispatched, until `recv()` returns `None`. The loop `while let Some(action) = actions.recv() { handle(action) }` does both. "Carrying out" an Action means doing what it asks at receipt — booking it, or applying it — not waiting for its effective instant: a Provider books a timed or future change and returns to its queue (Phase 7, KG-4). *Producer obligation; the Kernel half is KC-21a's.*

**KC-21a** (new):

> In a device-paced class, after the coordinator dispatches Actions on the control path — a Session entry (KC-28), the start dispatch (KC-18), an agenda item (KC-20) — and before KC-21's round, it waits, in host time, until every instance it dispatched to in that call has finished with every Action dispatched to it (MA-14b), for at most `DEFAULT_HOST_BUDGET_NS` from the dispatch, re-checking at least every 10 ms whether an end has been requested or dispatch is frozen, which stop the wait. An instance that has not finished by then makes the coordinator request the end `Failed { run }` in `abort` mode with the reason `"KC-21a: <first fragment> did not finish its Actions within <n> ms"`. An Action a Module submits from a step (MA-14a) is not waited for: that step is still running on the data thread, and waiting would stall the thread that steps its target's peers (Phase 7, KG-4; after Review J, P1-2, P2-8). *Checked by `kg_04_a_session_call_returns_after_the_provider_has_finished`, `kg_04_the_next_admission_sees_the_state_the_previous_action_made` and `kg_04_a_provider_that_never_finishes_fails_the_run`.*

**KC-24a** (new):

> Admission and dispatch are serialized by one **admission lock** in `Shared`: a control-path call holds it from its first admission (KC-24) through its last dispatch (KC-25) — a Session entry's every compiled Action, the start's batch, an agenda item —, a Module's submission (MA-14a) holds it from its admission through its dispatch, and RS-6 step 1 holds it while it sets `frozen` and clears the queues. No Action is therefore dispatched after dispatch is frozen, and each entry is judged against the configuration the previous one left. The lock is never held across a round or across KC-21a's wait, so a Module's submission from inside a round never waits for the call that runs the round (Phase 7, KG-4; Review J, P1-2). *Ceiling (Review K, N-P2-11): a Provider that submits Actions from a thread of its own (MA-14a allows it) could, during KC-46b's `stop`, wait for this lock while the control thread holds it and waits for that Provider's slot (KC-26); the Provider's bounded join breaks the cycle after its bound. No Phase 7 Provider submits Actions.* *Checked by `kg_04_no_action_is_dispatched_after_the_freeze`.*

**Code.** `coordinator/state.rs`: the Kernel's queue counts `pushed`, `taken` and `finished`: `push` increments `pushed` and returns it; `recv()` sets `finished = taken`, pops, increments `taken` when it popped, and notifies a condvar; `clear()` (RS-6 step 1) removes the undelivered Actions from `pushed` and notifies. `Shared` gains `admission: Mutex<()>`. `paced.rs`: `fn wait_finished(shared, targets: &[(Inst, u64)])` waits on the queues' condvars with a host-clock deadline and 10 ms re-checks. `admission::dispatch` returns the instance and its `pushed` count; `submit`, the start dispatch and `dispatch_agenda` take the admission lock around their admit-and-dispatch loops, release it, and then call `wait_finished` in a device-paced class; `Submitter::submit` and `FreezeDispatch` take it too. No public item changes (`ActionReceiver` keeps its one method).

**Rejected.** A synchronous dispatch hook on the role traits (a trait change for what the queue already sees, and a Provider thread may be inside a device call when it is invoked); a fixed delay after each call (the spike's `advance_to(+20 ms)`: slow and still a race); retrying a refused admission (it would hide a real refusal); counting `finished` when the Module takes an Action (a Module that has taken a `cold` change has not applied it); re-checking `frozen` in `dispatch` alone (it closes the freeze race but not the stale-configuration one between a Module's submission and a control entry).

**Tests** (`coordinator.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `kg_04_a_session_call_returns_after_the_provider_has_finished` | a device-paced Session; `ThreadedProvider` `.with_action_delay(30 ms)`; `submit(SetParameter test.gain 3)` | the Provider's recorded finish instant for that Action precedes `submit`'s return | KC-21a, MA-14b |
| `kg_04_the_next_admission_sees_the_state_the_previous_action_made` | the same Session: `submit(SetParameter test.tx_clock 10)`, then at once the test Vocabulary's `start_repeat` (it compiles to a `TxBurst`) on `radio/tx` with a waveform | the second entry is admitted (SC-23 finds the clock the first registered); repeated over 20 fresh Sessions, never refused | KC-21a (spike K5) |
| `kg_04_a_provider_that_never_finishes_fails_the_run` | a `ThreadedProvider` `.never_finishing()` | `submit` returns after about `DEFAULT_HOST_BUDGET_NS`; the Run ends `Failed { run }`, reason starting `"KC-21a: radio did not finish"` | KC-21a |
| `kg_04_no_action_is_dispatched_after_the_freeze` (Review J, P1-2; made deterministic after Review K, N-P1-4) | a device-paced Session whose profile carries an environment section `test.gate` read by a runtime-stage test admission check (registered in the test's `AdmissionCheckRegistry`) that, on its first call, signals the test and then waits up to 200 ms for the Provider's `stop` to begin; the `ThreadedProvider` is `.losing_device(…)` triggered by that signal and `.draining_after_stop(50 ms)` (it keeps calling `recv()` for 50 ms after `stop` begins, recording what it receives); the test submits one `SetParameter test.gain` | the Provider receives no Action after its `stop` began: with KC-24a the freeze waits for the admission lock, the check times out, the Action is dispatched and then cleared by the freeze (its entry is `Admitted`, which is correct: it was admitted before the freeze); without the lock the freeze runs during the check's wait and the Action is pushed after the clear and received during the drain | KC-24a, RS-6 |
| `kg_04_a_simulation_session_does_not_wait` | the Simulation-class equivalent with a stepped double | behaviour unchanged (`kc_28_*` still pass); `wait_finished` is not reached (a call counter in the double's queue reads show no wait) | KC-21a's scope |

---

## KG-5 — A device-paced Run waits for its horizon

**Problem.** KC-33 ends a Spec Run `Completed` when `next_wakeup` returns `None`: in the Simulation class nothing can happen after that. A device keeps streaming with nothing scheduled, so a device-paced Spec Run ended right after its first block (spike K2; the spike re-armed a heartbeat callback to keep the agenda non-empty). And `run_until_end(horizon)` schedules nothing, so in a device-paced class it would not wait for its horizon at all.

**KC-29** (sentence replaced: "`run_until_end(horizon)` schedules nothing: …"):

> `run_until_end(horizon)` schedules nothing in the Simulation class: it runs the loop until the Run ends (`Err(Ended)`), until a round has run at an instant at or after `horizon` (`Ok`; the last round's instant may be later than `horizon`, because an Authority cannot stop between its wakeups), or, in a Session, until `next_wakeup` returns `None` (`Ok`). In a device-paced class it schedules a no-op callback at `horizon` as `advance_to` does, and returns `Ok` after a round at or after `horizon` or `Err(Ended)` (Phase 7, KG-5).

**KC-33** (the second clause replaced):

> … `next_wakeup` returns `None` in a Spec Run of the Simulation class (`Completed {}`, orderly); in a device-paced class `None` ends no Run — the loop returns to its caller, and the Run ends by its schedule's `Stop {}`, `finish`, the Policy, the Lease or a failure (Phase 7, KG-5); …

**MA-30** (in the rule's `text` block): "None ends a Spec Run (KC-33)" becomes "None ends a Spec Run of the Simulation class (KC-33)".

The device-paced `None` clause is defensive: every device-paced loop has a callback scheduled at its horizon, so `next_wakeup` returns `None` there only after RS-6 step 1 has cancelled it, when an end is already requested and fixes the Termination (KC-32). The first draft's mutation of this clause (G18) was an equivalent mutant and is withdrawn (Review J).

**Rejected.** A heartbeat callback the coordinator re-arms (the spike's workaround: rounds with nothing to do, and the Run still ends if the heartbeat is ever cancelled); ending a device-paced Spec Run when every Sink is "complete" (no trait method says so; decision K12 of spec 06 stands).

**Code.** `stepping.rs`: `run_until_end` delegates to `advance_to`'s scheduling in a device-paced class; `run_loop`'s `None` branch requests `Completed` only in the Simulation class.

**Tests** (`coordinator.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `kg_05_a_device_paced_spec_run_runs_to_its_horizon` | a device-paced Spec Run with nothing scheduled; `run_until_end(now + 50 ms)` | `Ok` after at least 50 ms of wall time; the Run is `Running` | KC-29, KC-33 |
| `kg_05_a_device_paced_spec_run_ends_at_its_scheduled_stop` | a schedule entry `Stop {}` at T0 + 30 ms; `run_until_end(now + 5 s)` | `Err(Ended)` with `Completed {}`, within 1 s of wall time | KC-33, KC-17 |

---

## KG-6 — MA-8 is enforced in device-paced classes

**Problem.** MA-8 leaves bounding `prepare` and `arm` to the Module, and its ceiling says a hung in-process Module hangs the transaction; RS-8a's cleanup deadlines apply only after `Stopping`. Phase 2 named this a Gate X risk and Phase 4 moved it to Phase 7, because the UHD Provider is the first Module whose calls wait on I/O: opening streams, tuning, a reference that never locks.

**MA-8** (the ceiling replaced):

> … Enforcing the budget and returning `Timeout` is the Module's duty. In a device-paced class the coordinator enforces it as well (KC-12a). *Ceiling: in the Simulation class an in-process Module that hangs hangs the transaction; there every call is in-process and instant, and a wall-clock timeout would make the outcome of a deterministic Run depend on the machine (PO-11) (Phase 7, KG-6).*

**KC-12a** (new):

> In a device-paced class the coordinator makes each `prepare` (KC-12) and each `arm` (KC-14) on a worker thread of its own that holds the instance's slot, and waits for it at most the fragment's `host_budget` (`DEFAULT_HOST_BUDGET_NS`, KC-11) — the mechanism RS-8a uses for a cleanup step. A call that has not returned by then is abandoned — its thread keeps the slot, as an abandoned cleanup step does — and counts as `ModuleError { kind: Timeout, message: "KC-12a: <prepare | arm> of <first fragment> did not return within <n> ms" }`: from `prepare` it fails the stage at once, `Failed { prepare }` with that reason, **without** calling `collect_prepare`, which locks every instance's slot (`with_inputs`, KC-12) and would wait for the abandoned call forever; from `arm` it is `Failed { arm }`. Cleanup then finds that slot held: step 2's `stop` waits for it until RS-8a's deadline abandons the step, and steps 5 and 8 record KC-39's and KC-44's failures for it (Phase 7, KG-6). *Checked by `kg_06_a_prepare_that_hangs_fails_the_run_within_its_budget`, `kg_06_an_arm_that_hangs_fails_the_run_within_its_budget` and `kg_06_the_simulation_class_prepares_on_the_callers_thread`.*

**Rejected.** Every class (a timeout on the wall clock inside the Simulation class is a nondeterministic outcome); `start` and `stop` too (MA-8 names `prepare` and `arm`; `stop` already has RS-8a's deadline, and a `start` that blocks is caught by the Provider's own bound, UR-16); cancelling the call (an in-process call cannot be cancelled, which is RS-8a's own ceiling).

**Code.** `paced.rs`: `fn bounded<T: Send + 'static>(budget, f: impl FnOnce() -> Result<T, ModuleError> + Send + 'static) -> Result<T, Bounded>` with `Bounded::{Failed(ModuleError), TimedOut}`, a thread plus `recv_timeout`, with the call contained (KC-30) inside the thread. `pipeline.rs` `prepare_fragments` and `arm_instances` move a clone of the slot's `Arc`, the `Fragment` or `IslandDecl` clone and the `PrepareContext` into it in a device-paced class; on `TimedOut` `prepare_fragments` calls `self.fail(Stage::Prepare, …)` and returns before `collect_prepare`.

**Tests** (`coordinator.rs`; each takes about one host budget, five seconds, plus RS-8a's deadline for the held slot's `stop`).

| test | input | expected | rules |
|---|---|---|---|
| `kg_06_a_prepare_that_hangs_fails_the_run_within_its_budget` | `ThreadedProvider` `.with_prepare_delay(30 s)` in a device-paced Run | entry returns within 12 s; `Failed { prepare }`, `ezsdr.failure.reason` names KC-12a; the Manifest has a cleanup failure for the held slot | KC-12a, MA-8 |
| `kg_06_an_arm_that_hangs_fails_the_run_within_its_budget` | `.with_arm_delay(30 s)` | `Failed { arm }` naming KC-12a | KC-12a |
| `kg_06_the_simulation_class_prepares_on_the_callers_thread` | a Simulation Run whose double records its `prepare` thread id | the test thread's id | KC-12a's scope |

---

## KG-7 — T0 lies on every declared stream's grid

**Problem.** A transmit SampleClock is registered at `arm` (MR-9, TM-13e) and a receive one at T0 (MR-11); T0 is "the end of arm plus the start lead" (KC-15). When `T0 − arm` is not a whole number of samples, the two grids are out of step, and a burst the Spec places at "T0 + n receive samples" is moved to the next transmit sample, visible only in `requested_target` (spike K3: 415 ns at 1 Msps; reachable in Simulation with an odd `start_lead_ns`, pinned by `k3_an_off_grid_start_lead_moves_a_burst_to_the_next_transmit_sample`). The owner put the fix in Phase 7, with the transmit model. The Kernel's half is to choose T0 on the streams' grids; the Providers' half is RM-25 (VE-2).

**KC-15** (replaced):

> **T0**, the Run's start instant, is the smallest instant at or after `now + lead` that is a whole multiple of `L`, where `now` is the Authority's current instant on its primary root after the last `arm`, `lead` is `start_lead_ns` (KA-8) rescaled from `host.monotonic` to the primary root with TM-9 and rounded up to a whole tick, and `L` is the least common multiple of the numerators, in lowest terms, of `root_ticks_per_tick` of every SampleClock declared on the primary root when T0 is fixed (`ClockRegistry::declared_sample_clocks`); `L` is 1 when none is. An overflow of `L` or of T0 is `Failed { arm }` with the reason `"KC-15: overflow"`. T0 is later than `now + lead` by less than `L` root ticks (less than one period of the slowest declared stream when the ratios divide each other, as MockRadio's and the X310's `N` values of one Run usually do). Every declared stream then has a sample instant at T0, and with RM-25 two streams of one rate share every sample instant (Phase 7, KG-7; the bound after Review J, P2-4). *Checked by `kg_07_t0_lies_on_every_declared_grid`, `kg_07_with_no_declared_clock_t0_is_not_rounded` and `kg_07_a_burst_at_t0_plus_n_samples_is_exact`.*

**Rejected.** Refusing an off-lattice origin in `register_sample_clock` (TM-13b fixes a receive origin at a device's first sample, and a Kernel rule must not second-guess where a device's first sample was); moving the receive origin onto the transmit grid (the spike's workaround: T0 would no longer be the first receive sample, which v3 behaviour 3 needs); registering the transmit clock at T0 (KC-17 admits scheduled bursts before `start`, and SC-23 needs a running transmit clock then).

**Code.** `pipeline.rs` `set_start_instant`: `L` by `checked` arithmetic over `declared_sample_clocks()` whose `root` is the primary root; T0 rounded up with `div_euclid`.

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `kg_07_t0_lies_on_every_declared_grid` (coordinator) | a stepped double declaring clocks of ratios 6 and 10 (1 GHz root, `L` = 30) and a start lead of 1 000 ns after an arm at instant 7 | T0 = 1 020 (the first multiple of 30 at or after 1 007) | KC-15 |
| `kg_07_with_no_declared_clock_t0_is_not_rounded` | a Provider declaring no clock, lead 1 007 | T0 = arm instant + 1 007 | KC-15 |
| `kg_07_a_burst_at_t0_plus_n_samples_is_exact` (acceptance; **replaces** `k3_an_off_grid_start_lead_moves_a_burst_to_the_next_transmit_sample`) | the K3 case: MockRadio 1.3.0, `start_lead_ns` 2 000 000 500, a burst at receive sample `n` | the burst record's `start` is transmit sample `n` of a clock on the receive clock's lattice; `requested_target` absent; `late` absent | KC-15, RM-25, MR-9 |

GY-3 applied to Phase 7: the K3 test is removed because the behaviour it pinned is the defect KG-7 fixes; `design-notes.md` records it.

---

## KG-8 — A command lead is counted from dispatch

**Problem.** MA-10 defines `min_command_lead` as "the least lead this instance needs between receiving a timed Action and that Action's instant", while RS-19 adds it to the current instant at admission. For a stepped Provider the two coincide (KC-21's round runs at the admission instant). For a Provider that receives Actions on its own thread they do not: the spike's untimed Session transmissions were all `TIME_ERROR send_asap`, 1.83 ms of a 2 ms lead spent before the Provider saw them (K6). UC-6 has the same gap for a `hardware_timed` update.

**MA-10** (the `min_command_lead` sentence replaced):

> `ProviderInstance.min_command_lead` is the least lead this instance needs between the coordinator **dispatching** a timed Action to it and that Action's instant — the time the instance takes to receive the Action included — as a `Duration` in `host.monotonic`; absent means zero. A stepped Provider receives an Action in the round that follows its dispatch, at the same instant (KC-21), so its **delivery allowance** is zero; a Provider that is not stepped receives it when its own thread next calls `recv()`, which KC-21a bounds, and states in its own spec the delivery allowance its declared lead includes. What remains, the lead less the allowance, is the lead the instance needs from receipt, which UC-6 and a Vocabulary's lateness rule (RM-14) use (Phase 7, KG-8). It is the only envelope value the Kernel reads: …

**UC-6** (the third sentence replaced):

> Absent `at`, the effective instant is the current instant at receipt plus the target's lead from receipt (`min_command_lead` less its delivery allowance, MA-10). An `at` closer than that lead is late: the target applies the value at the current instant plus that lead and emits its Vocabulary's late-command event (Phase 7, KG-8).

**Rejected.** The Kernel adding an estimate of each Provider's delivery (a guess about a Module); stamping each Action with its admission instant (a Kernel Action schema change); keeping MA-10 and letting a hardware Provider declare a lead that covers delivery while testing lateness against the whole lead (every untimed command would be late by its own delivery, K6).

**Code.** None in the Kernel (RS-19's computation is unchanged: admission and dispatch happen at one instant on the control path). The doc comment of `ProviderInstance::min_command_lead` follows the rule. The Provider side is RM-14 (VE-2) and spec 18 UR-21.

**Tests.** No Kernel test (no Kernel code); the rule is carried by MockRadio's unchanged `mr_17_*` and `mr_18_*` (a zero allowance) and by spec 18's `ur_21_an_untimed_send_is_on_time_after_its_delivery` and `ur_24_an_untimed_retune_is_on_time_after_its_delivery`.

---

## KG-9 — TM-16c under an Authority that does not drive its clock

**Problem.** TM-16c says callbacks "observe `now()` equal to their fire time while running", which a Simulation Engine can promise because it drives its clock, and a device-paced Authority cannot: the device's time moves on while a callback runs (spike K12). TM-16a1 already says that in HardwareInLoop and Hardware the Authority reads the real clock. And TM-16c's `schedule(t, f)` "requires `t` … at or after `now`, failing with `InPast` otherwise": on a clock that keeps moving, `now` has passed any instant a caller computed a moment earlier, so a conforming paced Authority would refuse KC-46a's wake at `now` and, spuriously, `advance_to`'s callback at a `t` that was ahead when `advance_to` checked it (`stepping.rs:455-462` maps the refusal to `NotOnPrimaryRoot`). The spike's Authority refused only an instant before the last one it fired, which is the rule a paced Authority needs, but no text says so (Review J, P0-1). TM-16c also states "`cancel` reports whether the callback was still pending" twice.

**TM-16c** (the callback clause replaced; the repeated sentence removed):

> `schedule(t, f)` requires `t` in a governed domain and a tick of that domain's root (`Inexact` otherwise), and at or after `now` under an Authority whose pacing is `FreeRunning` (`InPast` otherwise); under a `WallPaced` or `Device` Authority it requires `t` only at or after the last instant `next_wakeup` fired (`InPast` otherwise), and an accepted `t` at or before `now` fires at the next `next_wakeup`, which returns `t`. Callbacks fire in ascending time, ties in insertion order; under a `FreeRunning` Authority they observe `now()` equal to their fire time while running, and under a `WallPaced` or `Device` one, whose clock does not stop for a callback (TM-16a1), they observe `now()` at or after their fire time (Phase 7, KG-9; the `schedule` clause after Review J, P0-1). A reading may round and a firing may not: … `cancel` reports whether the callback was still pending.

The doc comment of `TimeAuthority::schedule` (`time/authority.rs:52-54`) follows. The Simulation Engine and `ManualTimeAuthority` (`FreeRunning`) are unchanged.

**Rejected.** Making a device-paced Authority freeze its reported time during callbacks (a lie about device time that a Provider's lateness decisions would then inherit); keeping `InPast` at `now` and retrying a refused schedule at a later instant (the wake and `advance_to` would chase a moving clock, and a round at a later instant than the caller asked for would change when agenda items run).

**Tests** (`time_model.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `tm_16c_a_paced_callback_sees_now_at_or_after_its_instant` | `WallAuthority`; a callback scheduled 5 ms ahead reading `now()` after sleeping 2 ms inside itself | the reading is at least the fire time plus 2 ms | TM-16c |
| `tm_16c_a_paced_authority_accepts_an_instant_already_passed` (Review J) | `WallAuthority`: schedule at `now − 1 ms` after a wakeup at `now − 5 ms` has fired; then at an instant before that fired wakeup | the first accepted and returned by the next `next_wakeup` at once, its instant as scheduled; the second `InPast` | TM-16c |
| `tm_16c_ties_fire_in_insertion_order` (existing, kept) | — | unchanged | TM-16c |

---

## KG-10 — A Provider thread reports a lost device

**Problem.** KC-30 emits `DEVICE_LOST` when a Module call returns `DeviceLost`. A hardware Provider learns that its device is gone on its receive or transmit thread, where no call returns to the coordinator; the spike logged such errors to a section and the Run went on.

**MA-9a** (new):

> A Provider that is not stepped and finds on one of its own threads that its device is gone reports it by emitting `DEVICE_LOST` itself, through `emit_control`, with source its `instance().id`, severity `fatal`, time the current instant on the primary root and payload `{ "message": <text> }` — KC-30's event, whose pair KC-8 registers for every instance's source root. It then stops using the device; the Policy applies (KC-31, KC-46b), and its `stop` and `cleanup` must still succeed so that cleanup reaches its end (RS-11) (Phase 7, KG-10). *Producer obligation; checked for the Kernel's half by `kg_10_a_device_lost_from_a_provider_thread_aborts_the_run`.*

**KC-30** (sentence added after the list): "A `DeviceLost` found by a Provider on its own thread, off any call, is reported as MA-9a says; the coordinator then treats the event like KC-30's own."

**Rejected.** A Kernel callback a Provider calls to report the loss (a new handle for what an event already carries); making the next `stop` return `DeviceLost` (the Run would go on until then).

**Tests** (`coordinator.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `kg_10_a_device_lost_from_a_provider_thread_aborts_the_run` | `ThreadedProvider` `.losing_device(20 ms)` in a device-paced Spec Run; `run_until_end(now + 5 s)` | `Err(Ended)` with `Stopped { policy { kind: DEVICE_LOST } }`, mode abort, within 1 s; the event in `events.delivered` has the Provider's id as source | MA-9a, KC-31 |

---

## KG-11 — A device-paced Run records its root's relations

**Problem.** TM-18: "A Provider that owns a device timekeeper publishes a `ClockRelation` from that device's root to `host.monotonic`, and the Run's Authority does so for its primary root. Every Run records in its Manifest a `ClockRelation` from its root domain, device or virtual, to `utc`, with its uncertainty. The Kernel defines the record; measuring it is the Provider's work." Vision §15 repeats it: the device timekeeper "publishes a relation to host monotonic". KA-16 exempted the Simulation class from the UTC relation. But the coordinator writes `relations: Vec::new()` for every class (`coordinator/ending.rs:258`), and no Kernel interface lets an Authority publish a relation.

**MA-29** (sentence added):

> An Authority whose root is a timekeeper's measures its primary root's relations and returns them from `relations()`, a provided method whose default returns none: the relation to `host.monotonic` and the relation to `utc` (TM-18, KC-45) (Phase 7, KG-11).

```rust
pub trait Authority: Send + Sync {
    // … the three existing methods …
    /// TM-18: the measured relations of this Authority's primary root — to `host.monotonic`
    /// and to `utc` — or none when the root is virtual (a Simulation Authority, KA-16).
    fn relations(&self) -> Vec<ClockRelation> { Vec::new() }
}
```

**TM-18** (sentence added):

> In HardwareInLoop and Hardware the Run's Authority returns these relations from `Authority::relations` (MA-29), each with `source` its primary root and an `uncertainty` that bounds its own error. The relation to `utc` cannot bound the host clock's own error against UTC, which the Authority does not know: its `uncertainty` excludes that error and its `method` says so, and a reader who needs a bound on UTC itself adds the host's (from NTP or PTP, outside the Run). A second device's root (a later phase) is related by the Provider that owns it (Phase 7, KG-11; the exclusion stated after Review J, P2-19).

**KC-45** (the `clocks` clause replaced):

> `clocks` = `{ domains: clocks.domains(), relations: in a device-paced class the relations `authority.relations()` returns, read under `catch_unwind` when the Manifest is assembled, in the order returned, and none in the Simulation class (KA-16), sample_clocks: clocks.sample_clock_records() }`. A relation whose `source` is not the primary root is not recorded and adds a step-8 `CleanupFailure` with `fragment: None` and the reason `"TM-18: a relation whose source is not the primary root"`; a device-paced Run whose recorded relations include none with `target` `utc` — the Authority returned none, or panicked — adds one with the reason `"TM-18: the Authority published no relation of its root to utc"` (Phase 7, KG-11).

**Rejected.** The coordinator measuring the relations itself from `now()` and the host clock (TM-18 puts the measurement with the timekeeper, and only the Authority knows the error of its own `now()`, which extrapolates between device reads in spec 18 UR-7); recording them in a Provider section (TM-18 names the Manifest's clock record, and a reader must not need to know the Module); failing the Run when the UTC relation is missing (the samples are still valid; the missing provenance is recorded); an `Option<ClockRelation>` for UTC alone (TM-18's first sentence, the relation to `host.monotonic`, would still have no carrier).

**MA-46** (the list amended): `ClockRelation` joins the document types whose schemas are fixed for a Plugin, since a role-trait signature now carries it; its schema has been committed since Phase 1 (`schemas/clock_relation.v1.json`) (Phase 7, KG-11; Review J, P1-9).

**Code.** `module_api.rs`: the provided method (a method, not an item: `kernel_surface`'s `ov_23b` unchanged). `ending.rs`: `relations` as KC-45 says. `tests/kernel_surface.rs`: `time::ClockRelation` joins the closed `ma6_documents!` list, without which `ma_06_role_signatures_name_only_documents_and_handles` refuses the new signature — Review J's probe: `MA-6: Authority::relations names ["ClockRelation"]`.

**Tests** (`coordinator.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `kg_11_a_device_paced_run_records_its_root_s_relations` | a device-paced Run on `WallAuthority` | `manifest.clocks.relations` holds exactly the two relations the double returns, to `host.monotonic` and to `utc`, in its order | KC-45, TM-18 |
| `kg_11_a_simulation_run_records_none` | a Simulation Run whose Authority double would return relations | `relations` empty | KC-45, KA-16 |
| `kg_11_a_device_paced_run_without_a_utc_relation_says_so` | `WallAuthority::without_relations()`; then a double whose `utc` relation has `source` `host.monotonic` | `relations` empty and a step-8 failure naming TM-18 the first time; the second, one failure for the foreign source and one for the missing `utc` relation | KC-45 |

---

## KG-12 — A device-paced Session runs no child Run

**Problem.** KC-37a builds a child from its own Assembly, whose Providers are new objects for the parent's binding descriptions. In the Simulation class that makes a second MockRadio; on hardware it would open the parent's USRP a second time while the parent streams from it. KC-37a's ceiling left the choice to "the first hardware Provider".

**KC-37a** (step 2's list of checks gains a first entry after "the Run is `Running`"; the ceiling sentence replaced):

> … the Run's class is Simulation (reason `"KC-37a: a device-paced Session runs no child Run: its devices are the parent's, and a child would open them again (Phase 7, KG-12)"`); … *Ceiling: a child of a device-paced Session needs the parent's devices handed over, and its streams quiesced, for the child's time; a later phase that needs §54's sweep on hardware designs it.*

**Rejected.** Letting the child open the device again (UHD refuses a second claim, or two handles disturb one stream); quiescing the parent's streams for the child now (a design with no Phase 7 consumer).

**Code.** `pipeline.rs` `child_refusal`.

**Tests** (`coordinator.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `kg_12_run_child_is_refused_in_a_device_paced_session` | a device-paced Session; `run_child` with a valid child | `Ok((entry, None))`, entry `Rejected` with one violation `ezsdr.run_child` naming KG-12; `drive` never called; the child Assembly's Provider never prepared | KC-37a |

---

## KG-13 — A `cold` change on a device that must stop before it restarts

**Problem.** UC-3: "A stream so restarted ends its SampleClock at that instant … and continues on a new SampleClock whose origin is that instant". MockRadio can; a USRP cannot: a receive rate change stops the stream, reconfigures the DDC and starts it again, which takes time the device does not spend at one instant (spec 18 UR-25's restart lead, INFERRED 50 ms). Read literally, UC-3 forbids the only thing hardware can do, and a Provider that pretended otherwise would lose samples between the two instants without saying so.

**UC-3** (the second sentence replaced):

> A stream so restarted ends its SampleClock at `e₁`, the first instant at or after the effective instant at which the old clock has a sample, and continues on a new SampleClock whose origin is `e₂`, the first instant at or after `e₁` plus the Provider's restart lead at which the new clock may have its first sample (for a radio stream, both on their clocks' lattices, RM-25); a device that can restart at once has a restart lead of zero, and one that must stop a stream before it restarts it states its lead in its spec. The samples between `e₁` and `e₂` are neither delivered nor a gap (SC-12), because they belong to no clock. A stream started from no stream (a count changed from 0) has no old clock and needs no restart lead: its clock starts at the first instant at or after the effective instant at which the Provider has configured the stream and the clock may have its first sample (Phase 7, KG-13; the from-zero case after Review K, N-P0-1).

**Rejected.** Recording the samples between as a gap on the new clock (they were never on it: the new clock starts after them, and a gap flag on its first block would claim a loss the new stream did not suffer); keeping UC-3 and ending the old clock at the restart (the old stream did stop earlier, and its clock would claim instants it never delivered).

**Code.** None in the Kernel (text only, spec 05). The producers are MockRadio (unchanged: its restart lead is zero) and spec 18 UR-25.

**Tests.** No Kernel test; carried by `mr_18_a_cold_rate_change_starts_a_new_sample_clock` (a zero restart lead: the two instants equal) and spec 18's `ur_25_a_cold_rate_change_starts_a_new_clock_on_its_lattice` (`e₂ − e₁` ≥ the restart lead; the capture across the change has two continuity maps and no gap flag).

---

## KG-14 — TM-13b and TM-13e for a device

**Problem.** Two time rules state a Simulation-shaped origin that spec 18 and RM-25 depart from (Review J, P0-4 a, b). TM-13b: a receive origin "**must** be the root tick of the stream's first sample, so that the first block of the stream carries `ticks = 0`"; when a USRP misses its timed start (spec 18 UR-17), its first sample is later than T0, and an origin moved to that sample would make "sample index `k`" name a different instant than on MockRadio, the very thing TM-13b guards. TM-13e: a transmit origin is "the root tick at which the stream is armed", and one re-created by a `cold` update "takes that update's effective instant as the origin"; RM-25 moves the first to the lattice, and KG-13 lets the second start after a restart lead.

**TM-13b** (sentence added):

> A receive stream started by a timed command at T0 whose device misses the start keeps T0 as its origin: its first block carries `GAP_BEFORE` with `lost` equal to its first index (SC-18), so that an index still names the instant `T0 + index · root_ticks_per_tick`, on hardware as on MockRadio (Phase 7, KG-14).

**TM-13e** (the origin clauses amended):

> … its origin is fixed at **`arm`**: the first instant at or after the root tick at which the stream is armed at which the new clock may have its first sample — for a radio stream, the first on its root's lattice (RM-25) … A transmit stream re-created by a `cold` update (UC-3) takes as the origin of its new SampleClock the new clock's start UC-3 as KG-13 amends it gives (for a radio stream, RM-25's `e₂`) (Phase 2, KA-10; Phase 7, KG-14; wording after Review K, N-P2-7).

**Rejected.** Moving a late receive origin to the first sample (TM-13b's own reason); a lattice rule in the Kernel's `register_sample_clock` (KG-7's rejected alternative: TM-13b's first-sample origin on a device is not the Kernel's to judge, and TM-13b now names the one exception).

**Code.** None in the Kernel (text only). The producers are MockRadio (VE-4) and spec 18 UR-13, UR-17, UR-25.

**Tests.** No Kernel test; carried by `mr_09_the_transmit_clock_starts_on_its_lattice`, `ur_15_every_clock_starts_on_its_lattice` and `ur_17_a_missed_start_restarts_with_a_gap`.

---

## VE-1 — `radio` 1.3.0: one implementation of RM-2, RM-5, RM-7 and RM-8

**Problem.** Vision §13: "The Radio Model Vocabulary defines coercion rules." The only implementation of RM-5's defaults, RM-7's refusal and RM-8's coercion is MockRadio's (`crates/ezsdr-mock-radio/src/coerce.rs`, `profile.rs::tree`), so a second radio Provider must copy it or depend on MockRadio, which MA-3 forbids (spike K13). Two copies of one Vocabulary rule would be two things for Phase 8 to compare.

**RM-26** (new):

> `ezsdr_radio::device` implements, once, RM-2's tree with the capabilities in MR-4's form, RM-5's defaults, RM-7's refusal and RM-8's coercion, over a `DeviceDescription`:
>
> ```rust
> pub enum Grid { Values(Vec<f64>), Integer { lo: i64, hi: i64 }, Step { lo: f64, hi: f64, step: f64 } }
> pub struct DeviceDescription {
>     pub profile: ProfileRef,
>     pub rates: Grid,                  // both directions; `Values` → an `AnyOf` capability, otherwise a `Range`
>     pub whole_hertz_rates: bool,      // an `Eq` rate that is not a whole number of hertz is refused
>     pub frequency: Grid,              // `Step`; step 0 means no grid
>     pub gain: Grid,                   // `Step`
>     pub max_channels: i64,            // per direction
>     pub rx_antennas: Vec<String>, pub tx_antennas: Vec<String>,
>     pub coherent: bool, pub full_duplex: bool, pub hardware_time: bool,
>     pub phase_behavior_on_retune: String,
>     pub repeat_max_samples: u64, pub repeat_align_samples: u64,
>     pub block_len: u32,
>     pub tx_path_delay_samples: i64, pub rx_path_delay_samples: i64,
>     pub timing: TimingEnvelope, pub performance: PerformanceEnvelope,
>     pub defaults: BTreeMap<Key, Value>,   // RM-5: exactly the ten configuration keys
> }
> impl DeviceDescription {
>     pub fn tree(&self, device: &ResourceId) -> Resource;                                           // RM-2
>     pub fn coerce(&self, device: &ResourceId, request: &Requested) -> Result<CoerceReport, ModuleError>; // RM-5, RM-7, RM-8
>     pub fn envelope(&self) -> RadioEnvelope;                                                      // RM-20
> }
> ```
>
> `coerce` is pure (MA-11). Its refusal reasons begin `"RM-4: "` (a key that is not a radio key, or a value of the wrong kind), `"RM-5: "` (a `Present` with no default), `"RM-7: "` and `"RM-8: "`. A radio Provider may use it; MockRadio 1.3.0 and `ezsdr.radio.uhd` 0.1.0 do. A Provider that does meets RM-7 and RM-8 at `coerce` by construction and keeps RM-7's obligations at `prepare` and at runtime (Phase 7, VE-1). *Checked by `rm_26_the_description_builds_the_tree_mockradio_built`, `rm_26_coerce_cases` and, for equivalence, MockRadio's unchanged `mr_06_*` and `mr_04_instance`.*

**RM-1**: the version becomes 1.3.0, with the sentence "version 1.3.0 adds RM-25, RM-26, the device lead of RM-14 and the four payload types of VE-3 (Phase 7)".

**Rejected.** A copy in the UHD crate; the UHD crate depending on MockRadio (MA-3); a Kernel envelope type (OV-21).

**Code.** `crates/ezsdr-radio/src/device.rs`: MockRadio's `Grid` and `coerce.rs` moved with their logic unchanged, parameterised by the description instead of `Profile`; the tree builder moved from `profile.rs`. The reasons that began `"MR-6: "` begin with the RM rule they apply.

**Tests** (`crates/ezsdr-radio/tests/radio_model.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `rm_26_the_description_builds_the_tree_mockradio_built` | the `x310-like` values as a description | a tree equal, node for node, to the one MockRadio 1.2.0 builds (the expected tree is written out in the test) | RM-26, RM-2 |
| `rm_26_coerce_cases` | MockRadio's `mr_06_*` requests against the description: 19.5 Msps, 7 GHz, gain 31.7, four channels at 200 Msps on 1 GB/s, a `Set` of antennas, `Present` of every key | 20 Msps with a coercion; RM-8 rejection; 31.5 with a coercion; RM-7 rejection naming the rate key; the first available antenna; the defaults | RM-26, RM-5, RM-7, RM-8 |

---

## VE-2 — `radio` 1.3.0: the lattice (RM-25) and the device lead (RM-14)

**Problem.** KG-7 puts T0 on every declared clock's grid; the transmit clock and a clock a `cold` change starts must be on the same lattice for "T0 + n samples" to be one instant on two streams (spike K3). RM-14 tests lateness against the whole TimingEnvelope lead, which, after KG-8, includes a hardware Provider's delivery allowance (spike K6). And three things a device cannot do are stated nowhere (Review J, P0-2, P0-4 c, d): recall transmit samples it has already been handed, so a burst that preempts an open one cannot start inside what was handed over (RM-15), and a stopped burst ends within a window (RM-16); recall a timed command it has already been handed (RM-16's cancellation); and restart a stream at the instant the old one stops (KG-13) — for which MockRadio and the UHD Provider must follow one rule, not two.

**RM-25** (new):

> A radio Provider registers every SampleClock at an origin on its root's **lattice**: a whole multiple of the numerator, in lowest terms, of the clock's `root_ticks_per_tick`, so that two clocks of one ratio share every sample instant and T0 (KC-15) is a sample instant of every stream. The receive clock's origin is T0 (MR-11, spec 18 UR-15). A transmit clock registered at `arm` takes the first lattice instant at or after the arm instant (MR-9). On a `cold` change with effective instant `e` (UC-1), the old clock, if one runs, ends at `e₁`, the first instant at or after `e` on the old clock's lattice, and the new clock, if the new channel count is > 0, starts at `e₂`, the first instant at or after `e₁` plus the Provider's restart lead (UC-3 as KG-13 amends it; zero for MockRadio) on the new clock's lattice; the samples between `e₁` and `e₂` are neither delivered nor a gap. A change from 0 channels has no old clock and needs no restart lead: the new clock starts at the first lattice instant at or after both `e` and the instant the Provider finished configuring the stream (Phase 7, VE-2; one rule for both Providers after Review J, P0-4 d; the from-zero case after Review K, N-P0-1). *Producer obligation; MockRadio's tests are `mr_09_the_transmit_clock_starts_on_its_lattice` and `mr_18_a_cold_change_starts_its_clock_on_the_lattice`; the UHD Provider's is `ur_15_every_clock_starts_on_its_lattice`.*

**RM-14** (the first two sentences replaced):

> On receipt of a `TxBurst` a radio Provider evaluates `LatePolicy::decide(registry, at, now, device_lead)` with `now` its current instant in the transmit SampleClock rounded up to a sample … and `device_lead` its **device lead**: its TimingEnvelope's `radio.timing.min_timed_command_lead_ns` less the delivery allowance it declares (MA-10, KG-8), in `host.monotonic` (SC-27). A stepped Provider's allowance is zero, so for MockRadio the device lead is the TimingEnvelope's lead. `OnTime`: the burst starts at `at`. `SendAsap`: the target moves to the first transmit sample at or after `now + device_lead`; … (Phase 7, VE-2)

**RM-15** (sentences added): "A burst never starts before its transmit clock's origin: a target before it is late, and the burst's `LatePolicy` applies (`send_asap` moves it to the origin with `TIME_ERROR { send_asap }`, `drop` drops it) — which a burst admitted right after a `cold` transmit change can meet on a Provider whose restart lead is not zero (Review K, N-P0-1). A Provider that hands a burst's samples to a device ahead of their instants, and cannot recall them, cannot start the next burst before the first sample it has not yet handed over: for a burst that would start while another is open, the Provider decides it again when it takes it to transmit, against that first unhanded sample — which only the part of the Provider that hands samples over advances, so the decision and the hand-over cannot race (Review K, N-P1-2) —, and a target before it is late, with the burst's `LatePolicy` applied as above; the open burst ends at the next burst's start as decided. The Provider's spec states the in-flight window, the most it hands over ahead of the device's time. MockRadio hands nothing over ahead and restarts at once, so its bounds are `now + device_lead` and the origin (Phase 7, VE-2; Review J, P0-2)."

**RM-16** (sentences added): "A Provider that hands transmit samples to a device ahead of their instants ends a burst stopped at instant `s` at the last sample it had handed over, no later than `s` plus its in-flight window; the burst's record (`BurstEnd::Stop`) says where it ended. `Provider::stop` and a `Stop` for `<device>` cancel every pending timed command the Provider still holds; a command it has already handed to a device that cannot recall it — within the release window its spec states — is not cancelled, and is recorded as issued (Phase 7, VE-2; the cancellation clause after Review J, P0-4 c)." MockRadio's in-flight window and release window are zero.

**RM-6** (sentence added): "The value of `radio.timing.min_timed_command_lead_ns` is the lead the Provider needs from the coordinator's dispatch (MA-10), its delivery allowance included, so that a Spec's timing constraint is compared with what the Provider can promise end to end (Phase 7, VE-2)."

**Rejected.** A Kernel rule on origins (KG-7's rejected alternative); a separate capability key for the delivery allowance (a Spec constrains the end-to-end lead, which is what a Reactor needs; the split is the Provider's own business, stated in its spec); an in-flight window no longer than the device lead, so that preemption is never late (a 2 ms window on a non-real-time host risks underflow, unmeasured — B8 measures the least window without underflow, a Phase 8 input); a declared lead of at least the window (every Session command would pay 12 ms for a case only preemption needs). The preemption bound and the in-flight window are recorded as Phase 8 inputs: a Spec that preempts a running repeat with a lead between the Mock's 2 ms and the UHD Provider's window passes on MockRadio and is late on the X310.

**Code.** `design/07-radio-model.md` only; the producers are VE-4 and spec 18.

---

## VE-3 — `radio` 1.3.0: what a device reports

**Problem.** `TimeErrorPayload`'s outcomes (`send_asap`, `drop`, `plan_violation`, `refused`) cannot say that the Provider sent a burst on time by its own clock and the device reported it late (UHD's `EVENT_CODE_TIME_ERROR`); the spike called it `refused` (K11). `TX_UNDERFLOW`, `ALIGNMENT_ERROR` and `CLOCK_LOST` are declared kinds (RM-10) with no payload type, and the UHD Provider emits all three.

**RM-22** (additions):

```rust
pub enum TimeErrorOutcome { SendAsap, Drop, PlanViolation, Refused, LateAtDevice }   // snake_case: late_at_device
pub struct TxUnderflowPayload    { pub cause: TxUnderflowCause }
pub enum   TxUnderflowCause      { Starved, Lost }                                   // snake_case
pub struct AlignmentErrorPayload { pub lost: u64 }
pub struct ClockLostPayload      { pub reference: ClockReference }
pub enum   ClockReference        { Frequency }                                      // snake_case; `time` is additive when a Provider monitors its PPS
```

**RM-11** (the payload table gains three rows; one sentence added):

> | `radio.TX_UNDERFLOW` | `{ "cause": "starved" \| "lost" }`: `starved`, the device ran out of samples inside a burst because the host was late; `lost`, samples of a burst were lost between host and device |
> | `radio.ALIGNMENT_ERROR` | `{ "lost": int }`: the samples, on every channel of the stream, that the misalignment removed, which is the next block's time jump |
> | `radio.CLOCK_LOST` | `{ "reference": "frequency" }`: the frequency reference (10 MHz) was lost; no Phase 7 Provider monitors its time reference (PPS), so no other value is declared (Review J, P2-11) |
>
> `TIME_ERROR`'s outcome `late_at_device` means that the Provider handed the burst to the device in time by its own clock and the device reported that it arrived too late, so nothing was transmitted; `late_by_ns` is the device's reported instant less the target when the device reports one, and 0 otherwise. Its source is the transmit stream (Phase 7, VE-3).

**RM-10** (the last sentence replaced): "`radio.TX_UNDERFLOW`, `radio.TX_DISCONTINUITY`, `radio.ALIGNMENT_ERROR` and `radio.CLOCK_LOST` are declared so that a Spec's `policies.failure` may name them (SB-18); MockRadio never emits them; the UHD Provider emits all but `TX_DISCONTINUITY` (spec 18 UR-19, UR-27, UR-28)."

**RM-20**: the schemas `tx_underflow_payload`, `alignment_error_payload` and `clock_lost_payload` join `schemas/radio/`, and `time_error_payload.v1.json` is regenerated with the new enum value (eleven files); `schemas/SCHEMA_CHANGELOG.md` gains a Phase 7 entry ("v1 — Phase 7 — what a device reports": still version 1 before the freeze; `time_error_payload` gains the value `late_at_device`, which a reader validating against the Phase 2 file would refuse, so a validating reader regenerates from 1.3.0).

**Rejected.** Mapping the device's late report to `refused` (K11: `refused` says the Provider refused the burst, which it did not); UHD's own code names in the payloads (`underflow_in_packet`, `seq_error_in_burst`: device words in a Vocabulary; the UHD Provider records the raw code in its `async` section, UR-28); a payload-free `TX_UNDERFLOW` (RM-11 requires every emitted control-path payload to be a typed object).

**Tests** (`radio_model.rs`): `rm_20_schema_freeze` (eleven files), `rm_22_payloads_round_trip` (the new types and value), `rm_22_late_at_device_is_snake_case`.

---

## VE-4 — `ezsdr.radio.mock` 1.3.0

**Problem.** MockRadio registers its transmit clock at the arm instant and a `cold` change's clock at the change's instant (MR-9, MR-18), which RM-25 now puts on the lattice; and its `coerce` and tree become RM-26's.

**MR-1**: `version: 1.3.0`, `vocabularies: [{ radio, ^1.3.0 }, { sim, ^1.1.0 }]`, `impl_hash: Some(ContentHash::of_bytes(b"ezsdr.radio.mock 1.3.0"))`; the profiles stay 1.1.0 (their values are unchanged).

**MR-4, MR-6**: `instance().tree` is `Profile::description().tree(id)` and `coerce` is `Profile::description().coerce(…)` (RM-26); `Profile::description` builds MR-3's values into a `DeviceDescription` (`whole_hertz_rates` true for `ideal`).

**MR-9** (the last sentence replaced): "If the transmit channel count is > 0, it registers the transmit SampleClock with origin the first instant at or after `A` on the root's lattice (RM-25; Phase 7, VE-4)."

**MR-18** (`cold`, sentence added after "At `e`:"): "The old clock ends at `e₁` and the new one starts at `e₂` as RM-25 says, with a restart lead of zero: the receive block in progress is cut at `e₁`, the transmit side stops at `e₁`, and streaming continues from tick 0 of the new clock at `e₂`; the steps below that say `e` mean `e₁` for the old stream and `e₂` for the new (Phase 7, VE-4)."

**Tests** (`crates/ezsdr-mock-radio/tests/mock_radio.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `mr_09_the_transmit_clock_starts_on_its_lattice` | the harness at virtual instant 7 (1 GHz root), 1 Msps (ratio 1 000), `arm` | the transmit clock's origin is 1 000 | MR-9, RM-25 |
| `mr_18_a_cold_change_starts_its_clock_on_the_lattice` | the `ideal` profile (no start-up latency, so T0 may be 0), a `cold` receive rate change from 1 Msps (ratio 1 000) to 20 Msps (ratio 50) at instant 1 000 037; then, in a fresh harness, from 20 Msps to 1 Msps at 1 000 037 | the old clock ends at 1 001 000 and the new one starts there; then the old ends at 1 000 050 and the new starts at 1 001 000, with no block and no gap between | MR-18, RM-25 |
| every existing MockRadio test | — | unchanged, except that `mr_01_descriptor_registers` reads 1.3.0 | MR-1 |

The profiles that name MockRadio 1.2.0 (`crates/ezsdr-acceptance/src/rig.rs`, `crates/ezsdr-server/src/catalogue.rs`, MockRadio's own tests) name 1.3.0.

---

## VE-5 — The server builds a UHD Run

**Problem.** The catalogue refuses every Authority but the Simulation Engine and every Module it does not list (`crates/ezsdr-server/src/catalogue.rs`), and `connect()` with no profile always means the simulated default; Phase 6's S7 left "the lab server's own default" to Phase 7.

**EA-7** (sentences added):

> The catalogue also registers `ezsdr.radio.uhd` 0.1.0 with the Provider and Authority roles (spec 18). For each binding of it the server opens the device named by the selector's `args` with `Config.open_device` — `ezsdr_radio_uhd::open` in the binary, which without the server's cargo feature `uhd` refuses with `"EA-7: ezsdr.radio.uhd: this server was built without UHD; rebuild ezsdr-server with --features uhd"` — and builds the Provider from the binding and that device. A profile with more than one binding of `ezsdr.radio.uhd` is refused (`"EA-7: one USRP per Run in this server (Phase 7)"`). The Authority is the Simulation Engine, or, when `authority` names a binding of `ezsdr.radio.uhd`, that device's Authority (spec 18 UR-6) (Phase 7, VE-5).

`Config` gains `open_device: Option<Arc<dyn Fn(&str) -> Result<Arc<dyn ezsdr_radio_uhd::Device>, String> + Send + Sync>>`, `None` meaning `ezsdr_radio_uhd::open`. A test embedding passes a closure that returns a `FakeDevice`; no document can (GZ-9).

**EA-9** (sentence added):

> `Config.default_profile: Option<PathBuf>` names a BindingProfile document that `connect` without a profile uses instead of the built-in default; a file that cannot be read or is not JSON is the error `refused` naming the path, and the document is used as written (its capture Sink's `dir` included). The binary sets it from the environment variable `EZSDR_PROFILE` when that is set and not empty; the library reads no environment variable (Phase 7, VE-5; Review J, P2-2).

**EA-14** (sentence added):

> In a device-paced Session (the server built a UHD Authority for it) the server hands `run_child` an Assembly that holds no Module object and opens no device (`catalogue::empty_assembly`), and the Kernel refuses the child before it reads the Assembly (KC-37a, KG-12); the reply is `ran` with the rejected entry (Phase 7, VE-5).

The server's version becomes 0.2.0; `ezsdr-server` gains the dependency `ezsdr-radio-uhd` and the feature `uhd = ["ezsdr-radio-uhd/uhd"]`.

**Rejected.** A `--profile` flag (T15); linking libuhd into every server (T16); building a child's Assembly from the child profile in a device-paced Session (it would open the device a second time before the Kernel's refusal); reading `EZSDR_PROFILE` in the library (in-process tests would race on the process environment).

*Ceiling (Review J, P2-17): the server reads its input only between requests, so a client that dies during a long `wait_for` or `advance` is noticed when the call returns, and the device runs until then. A Detached Lease bounds it (KC-36 as KG-2 amends it); an Attached Session's client should keep its waits short on hardware.*

**Tests** (`crates/ezsdr-server/tests/protocol.rs`).

| test | input | expected | rules |
|---|---|---|---|
| `ea_07_a_uhd_profile_without_the_feature_is_refused` | `connect` with a UHD profile, default `Config` (built without `uhd`) | `refused` naming the feature | EA-7 |
| `ea_07_a_session_on_the_fake_device` | `Config.open_device` returning a `FakeDevice`; `connect` with a UHD profile; `submit` of `radio.start_repeat`; `wait_for` of `sink.CAPTURE_WRITTEN` after a capture request; `finish` | `connected`; the capture's samples contain the repeated waveform (the fake loops transmit to receive, UR-33); the Manifest's class is HardwareInLoop | EA-7, KG-2, KG-4 |
| `ea_07_two_uhd_bindings_are_refused` | two radio bindings of `ezsdr.radio.uhd` | `refused` | EA-7 |
| `ea_09_the_configured_default_profile_is_used` | `Config.default_profile` naming the default profile with another seed; `connect {}`; then naming a missing file | the Session's profile is the file's; then `refused` naming the path | EA-9 |
| `ea_09_the_binary_reads_ezsdr_profile` | the binary spawned with `EZSDR_PROFILE` set to such a file (a child process, so no in-process environment is touched) | its `connected` reply's profile is the file's | EA-9 |
| `ea_14_a_child_of_a_device_paced_session_opens_no_device` | the fake Session; `run_child` | `ran` with a rejected entry naming KG-12; `open_device` was called once (the parent's) | EA-14, KG-12 |

---

## VE-6 — Naming an instant from Python

**Problem.** On a wall-paced Session a `capture` without `at` starts where the server admits it, a client round trip after the `sleep` before it (Phase 6 §3: "a helper that names 'd seconds from now' as a `TimePoint` … belong[s] with Phase 7's coordinator loop"). The first draft had `sleep` return the instant the Run stood at and promised that `capture(n, at=t)` then starts at `t`. Review J (P1-5) showed the promise false on hardware: KG-2's data thread steps the capture Sink continuously, the Sink processes every block as it arrives whether a capture is pending or not (`crates/ezsdr-sink-capture/src/lib.rs:574-588`), and HD-10 starts a request at "the first delivered sample whose instant is at or after max(at, …)" among the blocks it receives after the request — so the samples at `t` may be gone before the request arrives. A client that wants a capture at an instant must name one ahead of the Run's time, which needs the root's rate, which the client does not have.

**EA-4 / EA-12** (the `status` reply gains a member):

> `status {}` → `status { run, state, now, root_rate, effective, events }`, where `root_rate` is the primary root's nominal rate in ticks per second as `{ num, den }` (`ClockRegistry::nominal_rate`) (Phase 7, VE-6).

**EA-16** (rows):

> | `Session.after(seconds) -> TimePoint` | the instant `seconds` after the Run's current instant: `status`'s `now` plus `ceil(s · root_rate)` ticks, where `s` is the decimal the caller wrote (`Fraction(repr(seconds))`, so `0.001` is one thousandth and not the binary float nearest it, whose ceiling would be one tick more), computed exactly (Phase 7, VE-6; the decimal after Review K, N-P1-3) |
> | `Session.sleep(seconds) -> TimePoint`, `Session.wait_until(t) -> TimePoint`, `Session.now` | `advance { by_ns }`, `advance { to }`, each returning the `advanced` reply's `now`, the instant the Run stands at; the Run's current `TimePoint` (Vision §54: the client never uses `time.sleep` for Run time) (Phase 7, VE-6) |

**EA-17** (sentence added):

> A capture starts at `at` when the recorder has not yet received the samples at `at` when the request reaches it, and otherwise at the first sample it receives after the request; the artifact's ContinuityMap says where it started. In the Simulation class the two are the same (the client's calls move the Run's time). On a device-paced Session, whose recorder the data thread steps between calls (KC-46), a capture meant to start at an instant names one ahead of the Run's time — `sdr.rx.capture(n, at=sdr.after(0.05))` —, and `t = sdr.sleep(d)` followed by `capture(n, at=t)` starts at `t` only when the request outruns the device's receive latency (INFERRED for a server on the device's host; bench B7 records it) (Phase 7, VE-6; after Review J, P1-5).

The package version becomes 0.2.0; `schemas/server/reply_frame.v1.json` is regenerated (additive) with a `SCHEMA_CHANGELOG.md` entry.

**Rejected.** A pre-trigger buffer in the capture Sink, keeping recent blocks so that a request for a passed instant can be served (a new Sink behaviour and memory bound for a case `after` avoids); a `delay` argument on `capture` computed by the server (a second way to say what `at` says); a server request "now + d" (one more request type where one member of `status` lets every client compute it).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `ea_12_status_carries_the_root_rate` (server) | `status` on the default profile | `root_rate` = `{ num: 1000000000, den: 1 }` (the Simulation Engine's root) | EA-4, EA-12 |
| `ea_17_a_capture_ahead_starts_at_its_instant` (server, `FakeDevice` embedding) | a device-paced Session; `status`; a capture `at` `now + 50 ms` converted with `root_rate` | the artifact's first continuity map starts at that instant, converted to the receive clock | EA-17, KC-46 |
| `ea_17_a_capture_at_a_passed_instant_says_where_it_started` (server, `FakeDevice` embedding) | a capture `at` an instant 20 ms in the Run's past | the artifact's first sample is at or after that instant, and its ContinuityMap names it | EA-17 |
| `test_sleep_returns_the_instant` (Python) | `sleep(0.01)` then `sdr.now`; `wait_until(t)` | equal; `t` rounded as the server rounds | EA-16 |
| `test_after_names_an_instant_ahead` (Python) | on the default profile (a 1 GHz root): `sdr.after(0.001)`, then `sdr.after(1.5e-9)`, each followed by `sdr.now` | the first is the `now` read next plus 1 000 000 ticks, the second plus 2 (the ceiling of 1.5 ticks; a floor would give 1) — Simulation time does not move between the calls; a capture `at` the first starts there | EA-16, EA-17 |

---

## Appendix A — Mutations the implementation must record (GZ-6)

Each row names the mutation and the test that must kill it. They are run with `plan/phase7/tools/mutate.py` (Phase 6's tool, copied) over `plan/phase7/tools/mutations.json`, which the implementation writes from this table.

| id | file (after implementation) | mutation | killed by |
|---|---|---|---|
| G01 | `coordinator/pipeline.rs` | KC-2 refuses `HardwareInLoop` again | `kg_01_a_device_paced_run_reaches_running` |
| G02 | `coordinator/pipeline.rs` | KC-2 admits `RealtimeEmulation` | `kc_02_a_wall_paced_authority_is_refused` |
| G03 | `coordinator/pipeline.rs` | KC-2a's stepped-Provider check removed | `kg_01_a_stepped_provider_is_refused_in_a_device_paced_class` |
| G04 | `coordinator/pipeline.rs` | KC-2a checks `affinity` only | `kg_01_an_island_with_an_rt_policy_is_refused` |
| G05 | `coordinator/pipeline.rs` | the data thread is not started | `kg_02_the_data_thread_drains_a_sink_while_no_call_runs` |
| G06 | `coordinator/stepping.rs` | the device-paced control round steps Sinks too | `kg_02_every_step_before_finish_runs_on_the_data_thread` |
| G07 | `coordinator/paced.rs` | the pass uses `step_until_quiescent` | `kg_02_a_sink_that_always_progresses_does_not_livelock` |
| G08 | `coordinator/paced.rs` | no wake after a delivery | `kg_02_wait_for_returns_when_the_data_thread_delivers` |
| G09 | `coordinator/stepping.rs` | `drain_and_react` applies the reactions outside the `delivered` lock | `kg_02_the_first_delivered_stopping_event_is_the_termination_cause` |
| G10 | `coordinator/paced.rs` | the data thread does not perform RS-6 steps 1–2 after requesting an end | `kg_03_a_fatal_event_stops_the_providers_without_a_client_call` |
| G11 | `coordinator/paced.rs` | the data thread's `StopTx` not recorded as done | `kg_03_a_step_done_early_is_not_repeated` |
| G12 | `coordinator/ending.rs` | the device-paced step 3 skips the final round | `kg_03_step_3_runs_a_final_round_over_the_sinks` |
| G13 | `coordinator/mod.rs` | `Drop` does nothing | `kg_03_dropping_a_live_device_paced_handle_cleans_up` |
| G14 | `coordinator/state.rs` | `recv()` counts an Action finished when it takes it | `kg_04_a_session_call_returns_after_the_provider_has_finished` |
| G15 | `coordinator/pipeline.rs` | `submit` does not wait (KC-21a) | `kg_04_the_next_admission_sees_the_state_the_previous_action_made` |
| G16 | `coordinator/paced.rs` | the wait has no deadline | `kg_04_a_provider_that_never_finishes_fails_the_run`, which then never returns: killed by `mutate.py`'s per-mutation timeout (900 s), which is the mechanism |
| G17 | `coordinator/stepping.rs` | `run_until_end` schedules no horizon in a device-paced class | `kg_05_a_device_paced_spec_run_runs_to_its_horizon` |
| G18 | — | *withdrawn*: `None` completing a device-paced Spec Run is an equivalent mutant (KG-5's note) | — |
| G19 | `coordinator/pipeline.rs` | `prepare` called directly in a device-paced class | `kg_06_a_prepare_that_hangs_fails_the_run_within_its_budget` |
| G20 | `coordinator/pipeline.rs` | `arm` called directly in a device-paced class | `kg_06_an_arm_that_hangs_fails_the_run_within_its_budget` |
| G21 | `coordinator/pipeline.rs` | the bounded call used in the Simulation class | `kg_06_the_simulation_class_prepares_on_the_callers_thread` |
| G22 | `coordinator/pipeline.rs` | T0 not rounded to `L` | `kg_07_t0_lies_on_every_declared_grid` |
| G23 | `coordinator/pipeline.rs` | `L` from the first declared clock only | `kg_07_t0_lies_on_every_declared_grid` |
| G24 | `coordinator/pipeline.rs` | T0 rounded down | `kg_07_t0_lies_on_every_declared_grid` |
| G25 | `coordinator/ending.rs` | `relations` always empty | `kg_11_a_device_paced_run_records_its_root_s_relations` |
| G26 | `coordinator/ending.rs` | the relation recorded in the Simulation class | `kg_11_a_simulation_run_records_none` |
| G27 | `coordinator/ending.rs` | a relation with a foreign source recorded | `kg_11_a_device_paced_run_without_a_utc_relation_says_so` |
| G28 | `coordinator/pipeline.rs` | KG-12's check removed | `kg_12_run_child_is_refused_in_a_device_paced_session` |
| G29 | `coordinator/admission.rs` | the admission lock not taken by `FreezeDispatch` | `kg_04_no_action_is_dispatched_after_the_freeze` |
| G30 | `coordinator/paced.rs` | the Lease deadline not checked by the data thread | `kg_02_a_detached_lease_expires_while_no_call_runs` |
| G31 | — | *withdrawn*: after RS-6 step 1 the data thread's own wake is unconditional (KC-46b) and the Run is ending, so a pending generation left set is unobservable; KC-46a no longer asks step 1 to clear it (Review K) | — |
| G32 | `coordinator/paced.rs` | a failed instance stepped again | `kg_02_a_failed_sink_is_not_stepped_again` |
| G33 | `ezsdr-radio-uhd/src/authority.rs` (the rule's only real-code carrier: the Kernel has no paced Authority; the same mutation of the `WallAuthority` double is killed by `tm_16c_a_paced_authority_accepts_an_instant_already_passed`) | the paced `schedule` refuses an instant before `now` | `ur_07_schedule_accepts_an_instant_already_passed` |
| G34 | `coordinator/pipeline.rs` | KC-2 refuses `Hardware` | `kg_01_the_hardware_class_reaches_running` |
| G35 | `coordinator/paced.rs` | the data thread keeps stepping after an abort | `kg_03_an_abort_stops_the_data_thread` |
| G36 | `coordinator/paced.rs` | the callback clears the pending wake without comparing generations, and the handle is stored after `schedule` returns | `kg_02_every_delivery_wakes_a_waiting_call` |
| V01 | `ezsdr-radio/src/device.rs` | RM-8's tie goes to the higher grid value | `rm_26_coerce_cases` |
| V02 | `ezsdr-radio/src/device.rs` | RM-7's check skips `tx` | `rm_26_coerce_cases` |
| V03 | `ezsdr-radio/src/device.rs` | the rate capability built as a `Range` for `Values` | `rm_26_the_description_builds_the_tree_mockradio_built` |
| V04 | `ezsdr-radio/src/payloads.rs` | `LateAtDevice` serialised as `lateatdevice` | `rm_22_late_at_device_is_snake_case` |
| V05 | `ezsdr-mock-radio/src/lib.rs` | the transmit origin at `A` again | `mr_09_the_transmit_clock_starts_on_its_lattice` |
| V06 | `ezsdr-mock-radio/src/lib.rs` | the `cold` origin at `e` again | `mr_18_a_cold_change_starts_its_clock_on_the_lattice` |
| V07 | `ezsdr-server/src/catalogue.rs` | two UHD bindings accepted | `ea_07_two_uhd_bindings_are_refused` |
| V08 | `ezsdr-server/src/lib.rs` | `Config.default_profile` ignored | `ea_09_the_configured_default_profile_is_used` |
| V09 | `ezsdr-server/src/lib.rs` | the child Assembly built from the child profile in a device-paced Session | `ea_14_a_child_of_a_device_paced_session_opens_no_device` |
| V10 | `python/ezsdr/session.py` | `sleep` returns `None` | `test_sleep_returns_the_instant` |
| V11 | `python/ezsdr/session.py` | `after` rounds the tick count down | `test_after_names_an_instant_ahead` |
| V12 | `ezsdr-server/src/lib.rs` | `status` reports the rate of the receive clock instead of the root | `ea_12_status_carries_the_root_rate` |
| V13 | `ezsdr-mock-radio/src/lib.rs` | the new clock starts at `e₁` instead of `e₂` | `mr_18_a_cold_change_starts_its_clock_on_the_lattice` (its second case has `e₁ ≠ e₂`) |

Spec 18 §8 lists the UHD Module's mutations (U01…).
