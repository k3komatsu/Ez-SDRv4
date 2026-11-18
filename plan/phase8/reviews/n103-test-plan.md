# #103 — test plan (final: v3)


Worktree `/home/komatsu/local_work/Ez-SDRv4-n103`, branch `work/n103`, base a4ad013a. v3 is the plan implemented in phase B and the one Sol reads. Sources: `test-plan-v2.md`, `recheck-v2.md` (M1, m1–m4, n1–n3), `decisions.md` (last section wins). No cargo was run when this plan was written. `path:line` are in that tree; crate prefix `crates/ezsdr-kernel/` is dropped. INFERRED names the test or step that settles it. `test-plan.md` (v1) is kept; this file replaces it. Sources: `sol-plan.md` (findings F1–F9), `decisions.md` ("Sol 6.2's plan review", binding).

## 0a. What v3 changes against v2 (the Opus re-check, `recheck-v2.md`)

| id | change | where |
|---|---|---|
| M1 | `Reading.admission_held` is a `try_lock`: "held by *any* thread". At `Released` a competitor released earlier and parked on the lock can take it right after the drop, and the data thread's `FreezeDispatch` can store `frozen`. So `assert_extent` no longer checks `Released` or `dispatch_frozen` there: `!held` at `Released` is asserted only in tests with no competitor at the drop (T7, T5's none, T4 via the clock) and in T6 after its Executor is gated (read first, then open the gate, then assert); `dispatch_frozen == false` is checked only at the held points; T3a never reaches `Released`, so `assert_extent` checks the points present | §5.0, T1, T2a, T2b, T3, T6, T7 |
| m1 | the worker's `catch_unwind` `Err` branch records `start:panicked` in the test's `Probe`; every test-thread wait also ends on that line, then `recv` + `resume_unwind`, so N103-3 dies on its named assertion and not on the 10 s expiry; T6's "assert the Run is still `Running`" is dropped (no handle exists then) | §5.0, T6 |
| m2 | N103-6: the release comes before `reach(Dispatching { next })` and the re-take after it | §8 |
| m3 | T8, first half: `effective()[radio]["test.grid"] == 20.0` (the `requires` value the prepare report seeds), not "absent" | T8 |
| m4 | `BeforePublish` is called from `pipeline.rs` between `lock(admission)` and `publish` (no `Arc` in `Shared::publish`; `state.rs` stays free of the seam); `publish` has no seam line | §3.1, §4 |
| n1 | §4's panicking points follow T5: only `Released` in T6/T7 panics; `Entered`-time reads in `start()` are recorded, never panic (a panic in `Executor::start` is contained as `Failed { arm }`) | §4 |
| n2 | T3 has no Executor that submits (one would block the data thread in `pass` before `drain_and_react`); its hook reads `held` before opening the loss gate | T3 |
| n3 | a test's own trace `Mutex` is not locked across an `assert!` (poison); hook panics leave `admission` poisoned, which `lock()` and `Probe` ignore | §5.0 |
| — | `RunStateMachine::move_to_at` is `pub(crate)`; N103-3 is run once through `remote_mutate.py` in phase B to confirm `killed`, not `ERROR` (no double panic aborts the binary); T1, T2a, T2b, T3, T3a, T6, T8 run 20× green on the final tree (M1's race is argued, not measured) | §3.1, §8, exit checks |

## 0. What v2 changed, by finding (Sol's review)

| F | Sol's finding (short) | Disposition | Where the plan changed |
|---|---|---|---|
| F1 major | `Shared::move_to` calls `TimeAuthority::now()` and (inside `RunStateMachine::move_to`) `HostClock::utc_nanos()`; under `admission` a Provider worker that holds a mutex those callbacks need and submits closes a cycle | adopted | §3: both clocks sampled **before** the lock, `Running` published under it with those values (`RunStateMachine::move_to_at`); §6 lock table drops both callbacks from the region; new test T4 + row N103-7 (a `HostClock` double reads the lock at every `utc_nanos` call) |
| F2 major | T6's `wait_for` is a whole-line match; its kill rested on KC-21a's budget | adopted | §5 T6: prefix `wait_until`; a non-blocking `Released` read (`!admission_held`) at the real wait boundary is the kill; the timed receive is a backstop only; N103-3 killed by that assertion |
| F3 major | a worker that calls `submit` while `Armed`, parks on the lock, then passes step 2 after `Running`: which instant governs? | adopted, decided by the orchestrator: the state at KC-24 step 2 governs | §3.3 and §9 (RS-18 gets the clause); T1 pins it at `Point::BeforePublish` (lock held, state still `Armed`), with a worker started by an Executor's `start()` and gated; result `Ok`, id after the batch |
| F4 major | T3 could pass with no end in the window; a deaf Provider cannot show a push after the freeze | adopted | §5 T3: the hook waits (positively) for the end request before releasing the start, reads `dispatch_frozen`; ordering assertion = `dispatch_frozen == false` at every dispatch point; the `undelivered` equality is kept but is not the oracle for lock-late mutants (stated) |
| F5 major | nothing catches an unlock between scheduled entries | adopted | §5 T2b: two distinct scheduled values, competitor released at `Dispatching { next: 1 }`, sees the last value, ids and final value asserted; T8: a batch that fails admission or whose instance fails `start` publishes nothing; row N103-6 |
| F6 minor | T2's whole-line match; `progressed`/park claim too wide; `Seen` also runs at arm | adopted | §5: result via prefix `wait_until`; T2 claims only exclusion (guard read) and continued stepping after release; `Seen` assertions filter on `proposed == B` |
| F7 minor | N103-5 adds a second acquisition; N103-3/-4 identical; watchdog expiry in kills | adopted | §8: N103-5 moves the one acquisition; N103-3 (drop after the wait) and N103-4 (drop removed) differ; each row's filter is its one killer; every deadlock-capable boundary has a non-blocking guard read that panics the start (resumed on the test thread) before the blocking call |
| F8 minor | use a private `cfg(test)` seam if tests can reach it | adopted as conditional; **not reachable** | §4: evidence (doubles are 3 462 lines in `tests/`, name the crate as `ezsdr_kernel::`, a `--lib` test would need them re-written in `src/`); the exact-gated inline `testing` module stays, with nothing `pub` beyond what the integration tests call |
| F9 note | keep the early-end check, drop the redundant `frozen` read, include T3a/N103-8 with a positive dispatch observation; record the Manifest outcome; amend KC-24a/KC-18, keep KC-46 | adopted | §7.3, T3a (asserts `effective()` never took the scheduled value), §9 wording, §10 (mechanism not decided here) |

Counts: tests 9 new (T1, T2a, T2b, T3, T3a, T4, T5, T6, T7) plus T8 (one new fn, one existing fn extended); rows 9 new (N103-1…N103-9; N103-8 optional), 0 retired, 0 re-anchored; seam: 5 gated call sites, 1 inline module.

## 1. Verdict

1. **Direction right, with a corrected extent.** Not "lock before `move_to`", but: sample the transition's clocks, take `admission`, publish `Running`, hold through the batch's last dispatch, drop, then wait and round. The clocks leave the region (F1). The batch's admission (`pipeline.rs:320`) stays outside the lock; KC-24a and KC-18 get the clause (§9).
2. **The old window needs a `testing` seam** (§4); a seam-free deterministic pause does not exist (re-verified with Sol: both clock callbacks ran before the flip even in the old code, and nothing else runs between `move_to(Running)` and the old lock).
3. **Lock order: no cycle** (§6), and F1's cycle is closed by construction: no Module or user callback runs under `admission` in the new region.
4. **Every new row dies by an assertion of its named test.** Where a mutant would block (guard held across the wait or the round, a lock before `start_instances`), a non-blocking guard read at a boundary before the blocking call panics the start; the test thread resumes the panic (§5.0), so the verdict is the assertion, not a timeout or a race.

## 2. Facts (VERIFIED by reading unless marked)

- `Run::start` tail today: `admit_schedule` `pipeline.rs:320-322`; `start_instances` `:326`; `move_to(Running)` `:329`; end check `:330-332`; `start_data_thread` `:334-336`; lock + `frozen` + dispatch `:337-340`; `wait_finished` `:341-343`; `round` `:344`.
- The whole schedule is the batch (`resolve_schedule`, `:690-780`); `admit_schedule` admits cumulatively against a working copy (`:804-826`); only `dispatch` writes the shared configuration (`admission.rs:284-292`); ids are taken at dispatch.
- `Shared::move_to` = `at = self.now()` (Authority, contained) then `lock(machine).move_to(state, at, host_clock)` where `RunStateMachine::move_to` calls `clock.utc_nanos()` inside the machine lock, before writing the state (`state.rs:381-385`, `run.rs:649-657`). `time_failure` reads the state (still `Armed` at the call) to pick `Stage::Arm` (`state.rs:286-305`).
- `Submitter::submit`: `Abort` returns before any lock (`pipeline.rs:1181-1189`); else lock → `admit` (clones the configuration inside) → `dispatch`. KC-24 step 2 reads `machine` at admission, under the lock (`admission.rs:44`).
- `frozen` has one writer, `FreezeDispatch` under `admission` (`ending.rs:291-294`); callers: `run_cleanup` (control thread, which cannot run before `pipeline()` returns: `mod.rs:159,176`) and the data thread's `stop_early` (`paced.rs:134-142`), whose thread is spawned inside the new region. `Ops::perform` has no other caller (`paced.rs:138`, `run.rs:534,600`, `ending.rs:253`).
- `RunStateMachine::move_to(to, at, clock)` is public and used by `tests/run_session.rs` (`:57-99`); `begin_stopping`/`finish` read the clock under `machine` only (`ending.rs:30,42`), never under `admission`.
- A client has no handle until `start_spec`/`connect` return; so only lease expiry, device loss and Policy reactions (via the data thread) and a Module `Abort` (any thread, no lock) can end the Run in the window.
- `Probe::wait_for` is a whole-line match; `wait_until(pred, expiry)` takes a predicate (`tests/support/run_doubles.rs:62`, used at `device_paced.rs:~1066`). `device_paced.rs` is scanned by `governance.rs:~714-770`: positive expiry `from_secs(n ≥ 3)`, no `sleep`, no shorter constructor, helpers included.

## 3. The change (§2 of v1, rewritten)

### 3.1 Shape

```rust
// state.rs
pub(super) struct Transition { at: Option<TimePoint>, utc: i64 }
/// The two reads of a transition: a Module call and a supplied clock, never made under `admission` (KC-24a).
pub(super) fn sample_transition(&self) -> Transition { Transition { at: Some(self.now()), utc: self.ctx.host_clock.utc_nanos() } }
pub(super) fn publish(&self, state: RunState, t: Transition) {   // machine lock only, no callbacks, no seam line (m4)
    let result = lock(&self.machine).move_to_at(state, t.at, t.utc);
    debug_assert!(result.is_ok(), ...);
}
pub(super) fn move_to(&self, state: RunState) { let t = self.sample_transition(); self.publish(state, t) }   // every other transition, as today

// run.rs: `pub(crate) fn move_to_at(to, at, host_utc_nanos)` is the body (the check `can_move_to` stays in it); move_to(to, at, clock) = move_to_at(to, at, clock.utc_nanos()).

// pipeline.rs, replacing :329-340
#[cfg(feature = "testing")] reach(Entered)             // first line of pipeline(), for the handle (§4)
let transition = self.shared.sample_transition();       // after start_instances; callbacks outside the lock (F1)
// KC-24a: taken after `start_instances` (a submission from `start` is refused at KC-24 step 2 and must not meet the lock) and held from the publication of `Running` through the batch's last dispatch.
let admission = lock(&self.shared.admission);
#[cfg(feature = "testing")] reach(BeforePublish)        // lock held, state still `Armed`, `machine` free (m4)
self.shared.publish(RunState::Running {}, transition);
if lock(&self.shared.end).is_some() { return; }
let device_paced = self.shared.device_paced();
if device_paced { super::paced::start_data_thread(&self.shared); }
#[cfg(feature = "testing")] reach(BeforeDispatch)
let mut dispatched = Vec::with_capacity(batch.len());
for (next, admitted) in batch.into_iter().enumerate() {
    #[cfg(feature = "testing")] reach(Dispatching { next })
    dispatched.push(super::admission::dispatch(&self.shared, admitted).1);
}
drop(admission);
#[cfg(feature = "testing")] reach(Released)
if device_paced { wait_finished }  ;  round   // unchanged
```

- The `frozen` branch is gone (§7.3). Explicit `drop(admission)`; an early `return` still releases the guard.
- **Preserved from today (F1's condition):** the sample order (`now()` then `utc_nanos()`, as `move_to` then `RunStateMachine::move_to` read them); the recorded `at` and `host_utc_nanos` of the `Running` row; the clock-failure path (`now_ticks` → `time_failure` → `ending::request` with `Stage::Arm`, because the state is still `Armed` at the sample, exactly as today; the end check after the publish then returns, as today). One difference, none observable: a refused (illegal) transition now reads the clock before refusing; INFERRED harmless, settled by `tests/run_session.rs`.
- `Running`'s `at`/`utc` are read a few instructions earlier than today (before the lock). No rule fixes them against the lock.
- G05's anchor `super::paced::start_data_thread(&self.shared);` still matches once; it is the only existing row whose `old` touches `:318-346` (every `pipeline.rs` row of all six lists, token-normalised against that region).

### 3.2 Alternatives rejected (unchanged from v1, one added)

Admission under the lock (nests `provider.object` under `admission` across `start_instances`, and holds the lock over device `start`); dispatch before `Running` (contradicts KC-18; Providers see Actions in `Armed`); a "started" flag read by step 2 (a second representation of `RunState::Running`); data thread started after the dispatch (incomplete: other Module threads); re-admitting the batch at dispatch (changes KC-17's `Failed { arm }`, double `coerce`). Added: keeping the clock reads inside `publish` under the lock (Sol F1's cycle).

### 3.3 The instant that governs a submission (F3, decided)

A Module that calls `submit` while the Run is `Armed` and is parked on the lock is judged by KC-24 step 2 **when it gets the lock**, after `Running` is published and the batch dispatched: admitted, with an id after the batch. Today the same submission is refused `RS-18` if it reaches step 2 before the flip, and admitted before the batch if after. A submission that reaches the lock *before* it is taken (while `start_instances` still runs) is still refused. RS-18's "arrives" is read as "is admitted" (§9). Observable consequence: a Module thread that races the transition is admitted rather than sometimes refused; no new refusal.

## 4. The seam (F8 re-decided)

**Question:** can a private `cfg(test)` probe with `--lib` tests (precedent `time/authority.rs:334`, `test_bin: "lib"` in `mutate.py:139`) reach the doubles the tests need?

**Evidence: no.**
- `src/` unit tests exist only in `session.rs`, `event.rs`, `hash.rs`, `time/authority.rs` (`grep -rln "cfg(test)" src`); none builds a Run, and the precedent is a ready-channel for one waiter on a `ManualTimeAuthority`.
- The doubles every sequence needs are `tests/support/` (`doubles.rs` 1 134 lines, `paced.rs` 697, `run_doubles.rs` 1 439, `mod.rs` 192): a wall Authority, a threaded Provider, a stepped Provider, `ProbeExecutor`, the registries and checks. They name the crate `ezsdr_kernel::…` (67 occurrences). In a `--lib` test `ezsdr_kernel` is the dev-dependency self copy (`Cargo.toml:21`), a second compilation of the lib whose `Provider`, `Executor` and `Authority` are different types from `crate::`'s (INFERRED, settled by one compile); they cannot be handed to `crate::coordinator::start_spec_run`.
- Re-writing the minimum (Authority + Provider + Executor + registry + check + Spec/profile fixtures) with `crate::` paths under `cfg(test)` in `src/coordinator/` is ~1 000+ lines of doubles in `src/`, which OV-20 forbids in spirit (shared doubles live in `tests/support/`), duplicated against the suite's own.

**Decision:** the exact-gated inline `#[cfg(feature = "testing")] pub mod start_probe` in `coordinator/mod.rs` (inline: `kernel_surface.rs:285` skips a gated `mod` whole; an out-of-line file would be parsed by `sources()` and need allow-list lines; PO-11, `governance.rs:185-212`, scans `coordinator/` for `thread::spawn`, `Instant::now`, `HashMap`: the module uses none). Public surface = exactly what the integration tests call:

```rust
pub enum Point { Entered, BeforePublish, BeforeDispatch, Dispatching { next: usize }, Released }
#[derive(Clone)] pub struct Probe(Weak<Shared>);          // Weak: holding it does not change the data thread's `strong_count == 1` backstop
pub struct Reading { pub admission_held: bool, pub end_requested: bool, pub dispatch_frozen: bool }
impl Probe { pub fn read(&self) -> Option<Reading> }       // try_lock WouldBlock, lock(end).is_some(), frozen.load(); all non-blocking or leaf
pub fn install(hook: impl FnMut(Point, &Probe) + 'static) -> Installed   // thread-local; Drop clears
pub(super) fn reach(shared: &Arc<Shared>, point: Point)    // take the closure out of the cell, call it, put it back
```

- Per-thread: `pipeline()` runs on the caller's thread; a test that runs the start on a worker installs inside the worker. No cross-test interference.
- Production: none (feature off in every shipped build: every `features = ["testing"]` is under `[dev-dependencies]`, `awk` over `crates/*/Cargo.toml`: acceptance, exec-native, hostmem, kernel, link-host, mock-radio, radio-uhd, radio, server, sim-engine, sim, sink-capture, sink). With the feature on, a caller can pause the start of a Run it starts itself; inert unless `install` is called.
- Orchestrator's call (the plan cannot make it): this is a src seam of a new kind; OV-20's named exception is `ManualTimeAuthority`. Recommend accept (rule is a lock extent; nothing else proves it deterministically). Fallback if refused: a spinner thread, killing the old code only by race, plus known-survivor lines in the style of `phase7/G09`.
- Why points read the lock itself (`Reading`) and are not "just before the lock": a position proxy passes when the lock moves without the hook. `Released` is placed right after the drop and (in T6, T7) asserts `!held`, so a mutant that moves or removes the drop leaves the lock held at a hook that stayed put.
- Hook bodies read `admission_held` once and only the *wait* depends on it (old order: the lock is free, the competitor completes, positive wait; new order: held, do not wait). At the held points (`BeforePublish`, `BeforeDispatch`, `Dispatching`) nobody else can hold the lock at that read: the competitor is gated until the hook releases it. At `Released` that is **not** true when a competitor was released earlier and is parked on the lock: the drop just before the read wakes it and it may take the lock before the read (M1). So a `!held` assertion at `Released` is made only where no competitor is parked at the drop (T7, T6 after gating). Never submit inline from a body that runs while held (same-thread relock); competitors are other threads (T1) or the data thread (T2). The only points that panic on a bad reading are `Released` in T6 and T7 (continuing would block on the wait or relock in the round); `x:start:held=` in T5 is recorded, never a panic (n1).

## 5. Tests

### 5.0 Common rig

- **Bounded start with assertion verdicts.** Every new test runs the start on a worker: `std::thread::spawn(move || { let _installed = install(hook); catch_unwind(AssertUnwindSafe(|| start_spec_run(...))) })`, sends the `Result` on an `mpsc`; the test thread does `recv_timeout(Wall::from_secs(10))` and, on `Err(payload)`, `resume_unwind(payload)`, so a hook assertion that fired before a blocking call surfaces as that assertion. The 10 s expiry is only the backstop for an unforeseen hang, never the oracle (`RunHandle` is `Send`; `kg_21a_…` sends one over an `mpsc`).
- **Trace.** The hook appends `(Point, Reading)` to a shared `Vec`; `assert_extent(&trace, required)` checks the points present, on correct code: `Entered`: `!held`; `BeforePublish`, `BeforeDispatch`, every `Dispatching`: `held` **and** `dispatch_frozen == false`; `Released` is **not** checked (M1); the caller lists the points the test must have reached (T3a reaches only `Entered` and `BeforePublish`). Never lock the trace `Mutex` across an `assert!` (n3).
- **`start:panicked`.** The worker's `catch_unwind` `Err` branch records `start:panicked` in the test's `Probe` (the support line log) before sending the `Err` on the channel; every test-thread `wait_until` predicate is `line_present || start:panicked`, and after any wait the test calls `recv` and `resume_unwind` on `Err`, so a hook assertion is the verdict even while the test thread is mid-wait (m1).
- **Fixtures.** Device-paced: `executor_docs()` (`device_paced.rs:210`), a schedule on `radio` offset 0, 1 (entries A1 = 1.0, A2 = 3.0; single-entry tests use A = 1.0), `ThreadedProvider::new("radio","radio",&probe).publishing(100, from_millis(1))` (declares the clock the schedule resolves against; cf. `kg_04_every_action_of_one_call_is_finished`, `:940-960`; INFERRED that a `publishing` Provider with no output does; fallback `output_docs`). Simulation: `SimAuthority`, `SteppedProvider … .declaring("radio/rx",1,1)`, `environment = {}` (cf. `kg_04_a_simulation_session_does_not_wait`, `:1164`; `coordinator.rs:157-170,961-1005,3394-3440`). B = 2.0 everywhere.
- **Support additions** (tests/support, both binaries): `Seen` (an `AdmissionCheck`, stage `Runtime`, section `test.seen`, registered like `Gate`, `device_paced.rs:1073`; profile `environment["test.seen"] = {}`) logging `(proposed test.gain, effective[radio][test.gain])`; it runs for the schedule's admissions at arm, **outside** the lock, as well as for Module/Session admissions, so every assertion filters on `proposed == B`. `ProbeExecutor` knobs: `submitting_after(gate, a)` (waits on a `(Mutex<bool>,Condvar)` gate inside `step` before the first submit), `submitting_from_worker(gate, a)` (`start()` spawns a thread that waits on the gate, records `x:worker:begin`, submits, records `x:worker:ok:<id>` or `x:worker:err:<check>:<reason>`), `submitting_from_start(a, probe_slot)` (in `start()`: read the probe; record `x:start:held=<bool>`; submit only if not held, record the result), `sharing_submitter(slot)`. A `WatchedClock` (`HostClock` forwarding to `SystemHostClock`, logging `admission_held` at each `utc_nanos` once a probe is in its slot).
- ActionId order is the cheapest observable of dispatch order: the counter starts at 1 and an id is taken at dispatch. With a batch of n entries correct code gives the batch ids 1…n and a competitor n + 1.
- All waits are positive, expiry `from_secs(10)`, prefix predicates through `wait_until` (never `wait_for` on a prefix); no `sleep`, no short constructor; nothing joins `KERNEL_WALL_ALLOWED`. One spin in T3 (`while !read().end_requested { yield_now() }` under an `elapsed() < from_secs(10)` assertion; `<` against a ≥ 3 s constructor passes the R1 scan).

### T1. A worker across the transition — `kc_24a_start_a_worker_submission_across_the_transition_is_admitted_after_the_batch` (device_paced.rs; loops `[device-paced, Simulation]`; one start per iteration at the one point `BeforePublish`)

- Pins F3 and the defect for any Module-owned thread. Executor `submitting_from_worker(gate, B)`: its `start()` (control thread, before the lock) spawns the worker; the hook at `Entered` stores the probe; at `BeforePublish` (lock held, state `Armed`) it reads `held`, opens the gate, waits (prefix `wait_until`) for `x:worker:begin`, and `if !held` also waits for `x:worker:ok:` or `x:worker:err:`.
- Expected (new code): `held`; the worker parks on the lock and, after the batch's dispatch and the drop, passes step 2 (the Run is `Running` by then): `x:worker:ok:2`; `Seen` has `(B, Some(A))`; `effective() == B`; Run `Running`; `assert_extent`.
- Wrong behaviour, deterministic: lock taken after `publish` or later (N103-1, N103-2): at `BeforePublish` the lock is free, the hook waits for the worker, which completes inside the hook while the state is still `Armed`: `x:worker:err:ezsdr.run_state:RS-18` — `ok:2` fails, and `held@BeforePublish` fails. Old code: same.
- The worker's `begin` is recorded before its call into the Kernel, not after it blocks, so the test does not claim "parked"; it claims the outcome of a submission that was called before `Running` was visible, which is the F3 decision.
- Simulation instance: the same body; no data thread, so only the Module thread competes.
- Row: N103-2 (named killer).

### T2a. The issue's sequence — `kc_24a_start_the_data_threads_first_submission_follows_the_batch` (device_paced.rs, device-paced; Executor `submitting_after(gate, B)`; two scheduled entries)

- Hook at `BeforeDispatch` (data thread started): read `held`, open the gate, `if !held` wait for `x:submit:` (prefix).
- Expected: held; the Executor's step blocks on the lock inside `submit`, the batch dispatches (ids 1, 2), the drop, then the submit is admitted and dispatched: `x:submit:ok:3`; `Seen` `(B, Some(A2))`; `effective() == B`; the Executor's `step` count grows after the submit (`wait_until`: the data thread is running again after the guard dropped). Claimed, no more: exclusion (the `held` read) and continued stepping; nothing about `progressed` or the park value (a growing step count cannot show them).
- Wrong, deterministic: old order (N103-1): `held == false`, the Executor completes first (`x:submit:ok:1`, `Seen` `(B, not A2)`), the batch overwrites: `effective == A2`.
- Row: N103-1 (named killer).

### T2b. The last scheduled entry — `kc_24a_start_a_submission_between_scheduled_entries_is_admitted_after_the_last` (same rig and body; release point `Dispatching { next: 1 }`)

- Expected: `held` at `next: 1` (between A1 and A2); the Executor's submit is admitted after A2: `x:submit:ok:3`, `Seen` `(B, Some(A2))`, final B.
- Wrong, deterministic: the guard released between entries (N103-6): `held == false` at `next: 1`, the Executor completes there (it sees A1, id 2) and A2 (id 3) overwrites: `ok:2`, `Seen (B, Some(A1))`, final A2.
- Not killed by T2b: the old order (the lock is held across the loop there); that is T2a's.
- Row: N103-6.

### T3. An end in the window — `kc_24a_start_an_end_in_the_window_records_the_batch_as_undelivered` (device_paced.rs, device-paced; one entry A; `ThreadedProvider … .deaf().losing_device_on(gate)`)

- Ends that can occur: §2. The plan table (same as v1's: end set at the `:330` check → nothing dispatched, `undelivered == []`, unchanged; end after it → batch dispatched, freeze lists it) stands.
- The rig has no Executor (n2): one that submitted would block the data thread in `pass` before `drain_and_react`, and the spin below would never see the end on correct code.
- Hook at `BeforeDispatch`: **read `held` first** (n2: after the gate opens the data thread's freeze may be the holder under N103-1), then open the loss gate; spin until `read().end_requested` (positive proof that the end was requested **inside** the window, F4); then `if !held` wait `radio:stop` (exact line, as `Gate::check` does). Record the reading taken after those waits.
- Expected (new code): `held`; `end_requested`; `dispatch_frozen == false` **at the reading taken at `BeforeDispatch`, before the gate opened and while the lock is held** (not after the drop: M1); the data thread's `stop_early` parks in `FreezeDispatch` until the drop, then clears the queue: `manifest.termination.reason == Stopped { cause: Policy { event: DEVICE_LOST } }` (not KC-21a's failure: `wait_finished` returns on the end), `undelivered == [ActionId(1)]` (deaf: nothing takes it; INFERRED id 1, the test settles it).
- Wrong, deterministic: **ordering** — whenever the lock is late (N103-1) or released across the window the freeze can win: the reading after the waits then shows `dispatch_frozen == true`, and the `held` read taken first is `false`; with the lock late the hook (`!held`) waits for `radio:stop`, after which `dispatch_frozen` is read again and must be `false`. **Record** — `undelivered == []` if the batch is dropped when an end is seen after the check (N103-9) or with the old `frozen` branch. Stated honestly: with the amended body (the `frozen` branch removed) a lock-late mutant would dispatch after the freeze and the control thread's own later freeze would still list the id, so the `undelivered` equality alone would pass there; the guard and `dispatch_frozen` assertions are what kill it.
- The "no dispatch after the freeze" half of KC-24a for the batch holds by construction; `kg_04_no_action_is_dispatched_after_the_freeze` keeps the Session half.
- Row: N103-9 (named killer). T3 is the second witness for N103-1 (filter names T2a only).

### T3a. An end before the dispatch check — `kc_24a_start_an_end_before_the_check_dispatches_nothing` (coordinator.rs, Simulation; optional)

- Executor `sharing_submitter`; hook at `BeforePublish` calls `submit(Action::Abort{..})` **inline** (an `Abort` takes no lock, safe while held); the check after the publish must return before the dispatch.
- Asserts: `effective()[radio]` never took the scheduled value (A = 7.0, chosen unlike any default): `dispatch` writes the configuration, so a dispatched batch is positively visible; `undelivered == []`; the Abort's id is 1 (consistent, not the oracle); termination `Stopped { cause: Abort }`. (No deaf target needed; `effective()` is the observation.)
- Wrong: the check removed (N103-8): the batch is dispatched into an ending Run, `effective()` holds A.
- Row: N103-8 (optional; the check has no row today: grep of all lists for `shared.end` in `pipeline.rs`: none).

### T4. The transition's clocks are read outside the lock — `kc_24a_start_reads_the_transition_clocks_outside_the_lock` (coordinator.rs, Simulation)

- `Assembly.host_clock = WatchedClock`. The hook at `Entered` stores the probe in the clock's slot. Every `utc_nanos` after that records `read().admission_held`. (`now()` is the other half of `sample_transition`; one function, so the `HostClock` read stands for both; INFERRED, the row's `old` spans the whole `sample_transition` call.)
- Expected: every recorded read `false`, including the one at the `Running` transition.
- Wrong: the F1 regression — `move_to`/sample inside the region (N103-7): the `Running` read records `true`. The callback only reads (`try_lock`), so a mutant does not block, and the assertion, not a timeout, kills it.
- Row: N103-7.

### T5. A submission from `start` — `kc_24a_start_a_submission_from_start_is_refused_not_blocked` (coordinator.rs, Simulation; `start_instances` runs on the control thread in both classes)

- Executor `submitting_from_start(B, slot)`: `start()` reads the probe; records `x:start:held=false`; submits (the Run is `Armed`); records `x:start:err:ezsdr.run_state:RS-18…`.
- Expected: `held == false`; refusal RS-18; the Run reaches `Running`; `effective()` has no B; `Seen` has no entry for B.
- Wrong: the lock acquired before `start_instances` (N103-5): `x:start:held=true`; the double then does **not** submit (a same-thread relock would hang), so the verdict is the `held == false` assertion, not a deadlock.
- Row: N103-5.

### T6. The lock is not held across KC-21a's wait — `kc_24a_start_the_lock_is_free_at_the_wait` (device_paced.rs, device-paced)

- Provider `hold_after_take(gate)`; one entry A; Executor **`submitting_after(gate_x, B)`** (M1: ungated, the Executor's thread would be parked on the lock at the drop and could take it before the read). Hook at `Released`: **read `admission_held` first, then open `gate_x`, then assert `!held`** (panic otherwise, before `wait_finished`; opening first keeps the Executor from sitting on its gate holding its object lock while `RunHandle::drop` waits to join the data thread, which would turn the kill into the 10 s expiry). The Provider has taken A and holds it, so the control thread enters the real wait (5 s budget, `paced.rs:177-205`).
- Test thread: `wait_until` prefix `radio:took:` (or `start:panicked`); `wait_until` prefix `x:submit:ok:` (or `start:panicked`; the Executor's submit is admitted *while* the control thread waits; positive, backstop expiry 10 s); open the Provider's gate; the worker returns; `x:submit:ok:2`, `effective() == B`. (No "the Run is still `Running`" assertion: no handle exists before the start returns.)
- Wrong: the guard held across the wait (N103-3, drop after the wait): `held == true` at `Released`: the panic is the kill (the test thread's waits end on `start:panicked`), before the wait and before the 5 s budget can matter. Nothing in the verdict depends on host speed.
- Row: N103-3.

### T7. Simulation: the first step follows the batch; the round takes no lock — `kc_24a_start_a_simulation_first_step_follows_the_batch` (coordinator.rs)

- One entry A; `ProbeExecutor::submitting(B)`, stepped by the control thread in the KC-21 round after the dispatch (fixtures `coordinator.rs:3394-3440`, `:961`). Hook at `Released`: **assert `!admission_held` (panic before the round)**; Simulation, so no competitor is parked at the drop.
- Expected (unchanged behaviour, pinned): `x:submit:ok:2`; `Seen` `(B, Some(A))`; `effective() == B`.
- Wrong: the drop removed, guard to the end of the function (N103-4): `held` at `Released` → panic before the round (an unchecked mutant would relock on the control thread and hang). Dispatch after the round (reorder): `ok:1`, `Seen (B, not A)`.
- Row: N103-4.

### T8. A batch that is refused or whose instance fails publishes nothing — pinned, no row

- Extend `kc_17_scheduled_updates_are_admitted_cumulatively` (`coordinator.rs:961`), first half (entry 1 refused at arm, `Failed { arm }`): assert `run.effective()[radio][test.grid] == 20.0` (the `requires` value the prepare report seeds, `coordinator.rs:965`; not the scheduled 40.0 — entry 0 was not published; INFERRED that `TestProvider` reports the required value, the test settles it).
- New `kc_18_a_start_failure_publishes_no_scheduled_update` (coordinator.rs): one scheduled entry, `TestProvider … failing_at(FailAt::Start)` (as `kc_18_a_start_failure_is_failed_arm`, `:526`, with a schedule): `Failed { arm }`; `effective()` lacks the scheduled key; no `p:action:` line; no `Running` row.
- Both pass before and after by design (they guard the order admission → start → publish against drift). No mutation row claims them: the rows below fail them only if the implementer moves the dispatch before `start_instances`, which the brief does not do; say so in the review record.

## 6. Lock order (revised for F1)

**What the control thread takes while it holds `admission` after the change** (from `publish(Running)` to the drop):

| lock / call | where | note |
|---|---|---|
| `machine` | `publish` → `move_to_at` | held with **no callback**: the `at` and `utc` values were sampled before `admission` was taken (F1). `Authority::now()` and `HostClock::utc_nanos()` are no longer called under `admission` in this region (T4). |
| `end` (then `also` on a failure path) | the check after the publish; `ending::request` from `start_data_thread`'s failure | order `end → also` everywhere (`ending.rs:210-228`); never held while waiting for `admission` |
| `data` | `start_data_thread` (`paced.rs:38`) | `stop_data_thread` takes it as a temporary and joins with the guard released (`paced.rs:47-56`) |
| `configuration`, `Queue.state` | `dispatch` | leaves |
| atomics | `next_action`, `frozen` | n/a |

No join, no condvar wait, no channel wait, no callback to a Module or a supplied clock inside the region. Thread spawn (`start_data_thread`) is not waited for.

**Reverse edges (a thread that takes `admission` while holding another lock):**

| path | holds | then takes | cycle with the region? |
|---|---|---|---|
| data thread `pass` → Module → `submit` (`paced.rs:91-110`, `pipeline.rs:1193`) | `executors[i].object` / `sinks[i].object` | `admission` | no: the region takes no object lock |
| control `round` → Module → `submit` (`stepping.rs:116-176`) | every instance's object lock | `admission` | no: the round runs after the drop; T7 asserts it |
| `Submitter::submit` from a Module/Provider thread | **a Module's own lock M possible** | `admission` → `machine`, `configuration`, `store`, `ClockRegistry`, queue, user checks | **F1's cycle needed a callback in the region that waits for M; none remains.** The Authority's and HostClock's reads precede the lock; T4 pins it |
| `Run::submit` (`pipeline.rs:938`) | none | `admission` → `machine`, `configuration`, `providers[i].object` (`coerce_provider`, Session/Schedule origin) | not in the region; the Provider-thread-during-`stop` ceiling of KC-24a is unchanged |
| `dispatch_agenda` (`stepping.rs:360`) | none | `admission` per item | no |
| `FreezeDispatch` (control's cleanup worker, or the data thread's `stop_early`, `ending.rs:291`) | nothing | `admission` → queues, `undelivered`, `wake_handle`, `scheduled`, Authority `cancel()` | the data thread's freeze waits for the region (≤ one batch dispatch); the region waits for nothing from it |
| `stop_data_thread` (cleanup) | none | joins | after `pipeline()` has returned; every early `return` drops the guard first |

**Verdict:** no cycle; F1's was a Module-internal lock reached through a callback under `admission`, and the callbacks are out. Remaining Module call under `admission` anywhere: `coerce`, registered checks, `Authority::cancel()` in the freeze, all pre-existing. INFERRED: the freeze's `cancel()` and a Provider worker that holds a lock the Authority needs have the same shape as F1; pre-existing, outside #103, and worth its own issue if the orchestrator agrees.

## 7. Other calls, leftovers (changes from v1 only)

### 7.1 Survey (S7)

Unchanged from v1 (start batch: this step; `Run::submit`, `Submitter::submit`, `dispatch_agenda` per item, `FreezeDispatch`: hold KC-24a; `run_child`, cleanup, `round`: n/a). One addition: after the change the batch's *admission* is the only admission in the survey that precedes its lock; no other origin can dispatch before `Running`.

### 7.2 Existing tests

Unchanged from v1 (none combines a start batch with a concurrent Module submission; none should change). The extension of `kc_17_scheduled_updates_are_admitted_cumulatively` is T8.

### 7.3 The `frozen` check

Delete it (unreachable once the lock covers the publish: `FreezeDispatch` needs `admission`, the data thread is spawned inside the region, `cleanup` needs the handle). No row exists for it today. INFERRED (code reading); T3 and the gate settle it.

## 8. Mutation rows

List `plan/phase7/tools/mutations.json` (it holds G05, G29, C10, C07, every KC-24a/KC-46 row of `pipeline.rs`/`ending.rs`). Ids `N103-n`. Filters are the one killer test's **full name** (not a family prefix), so an unrelated failure cannot give a row credit (F7). `old` anchors are Rust token runs matching once through `anchor.py`; the implementer states the exact text when the code lands. Every kill below is an assertion in the named test; none is a timeout, a race or a bound, so no row declares `"kill": "bound"`.

| id | mutation (skeleton) | named killer (bin) | the assertion that fires |
|---|---|---|---|
| N103-1 | the old order: `let admission = lock(…)` moved from before `publish` to after the `BeforeDispatch` call (span: lock … hook) | T2a (`device_paced`) | `held@BeforePublish`/`@BeforeDispatch == false` in `assert_extent`; `x:submit:ok:1`; `effective == A2` |
| N103-2 | the lock moved from before `reach(BeforePublish)` to right after `publish`, before the end check (the hook stays before the lock) | T1 (`device_paced`) | `held@BeforePublish == false`; `x:worker:err:ezsdr.run_state:RS-18` where `ok:2` is expected |
| N103-3 | `drop(admission)` moved after the `if device_paced { wait_finished }` block, before `round` | T6 (`device_paced`) | `!admission_held` at `Released` (panic before the wait; the test thread's waits end on `start:panicked`). Run once in phase B through `remote_mutate.py`: must be `killed`, not `ERROR` (no double panic aborts the binary) |
| N103-4 | the `drop(admission);` line removed (guard to the end of `start`) | T7 (`coordinator`) | `!admission_held` at `Released` (panic before the round) |
| N103-5 | the one acquisition moved before `if !self.start_instances() …` (no second acquisition) | T5 (`coordinator`) | `x:start:held=true`, expected false (the double does not submit, so nothing blocks) |
| N103-6 | the guard released and re-taken between scheduled entries (the guard held as `Option`; inside the loop body the release `admission = None` comes **before** `reach(Dispatching { next })` and the re-take `admission = Some(lock(…))` **after** it, so the gap contains the hook (m2); one contiguous span from the lock line to the loop) | T2b (`device_paced`) | `held@Dispatching{next:1} == false`; `x:submit:ok:2`; final A2 |
| N103-7 | `sample_transition()` moved inside the region (after the lock), as `move_to` does today | T4 (`coordinator`) | the `utc_nanos` read at `Running` recorded `held == true` |
| N103-8 (optional) | the end check after `publish` removed | T3a (`coordinator`) | `effective()[radio]["test.gain"] == A` |
| N103-9 | an end seen after the check discards the batch: `if lock(&self.shared.end).is_some() { return; }` inserted after the `BeforeDispatch` call | T3 (`device_paced`) | `undelivered == []` where `[ActionId(1)]` is expected |

Retired: none (no row mentions `frozen` at the start). Re-anchored: none expected; G05 keeps its text; `rules.py` after `cargo fmt --all` settles it. Rows run only through `remote_mutate.py`; a fix round re-runs these and G05, G29, C10.

## 9. Rule text and process (the orchestrator's)

- **KC-24a** (`design/06:329`): "from its first admission … the start's batch" is false for the batch today and would stay so. Substance: the start's batch is admitted at arm (KC-17), when no other origin can pass KC-24 step 2 and the schedule's own admissions run only on the control thread; its dispatch holds the lock the coordinator takes after the instances start and releases after the batch's last dispatch, with the Run's entry into `Running` published under it, and the clocks of that transition are read before it is taken; the lock is released before KC-21a's wait and the round (already stated). *Test:* add T1, T2a, T2b, T3, T3a, T4, T5, T6, T7; state that the no-dispatch-after-freeze half for the batch holds because the freeze cannot interleave.
- **RS-18** (`design/04:178`): "An Action that *arrives* while the Run is not `Running` is rejected" → add: "is admitted" is what KC-24 step 2 judges; an Action in flight across the entry into `Running` is judged when it holds the admission lock (F3).
- **KC-18** (`design/06:253`): cross-reference KC-24a ("the dispatch holds the admission lock, KC-24a"). **KC-46** (`:435`) "before the start dispatch": kept; the plan keeps the thread start between the publish and the dispatch. KC-17, KC-21a, RS-16: no sentence becomes false. `design/06` closing *Changes* table: one row (no version bump); `rules.py --base <pre-change commit>` must pass.
- **Manifest** (orchestrator's, one line at landing): an end requested in the window can no longer drop the batch silently; its ids are then in `termination.undelivered` when the Provider has not taken them. The field and its meaning do not change; the outcome the race allowed becomes the only one. Not an owner question per `decisions.md` Q5/F9; listed so the owner can reverse it.
- **EA-16 / user-visible:** no new refusal, error text or failure mode; a competing submission waits at most one batch dispatch; the race-only change in §3.3 turns a sometimes-refused submission into an admitted one.

## 10. Open questions (with recommendations)

1. **Seam (§4):** accept the exact-gated inline `start_probe` (5 gated call sites). Recommend yes; evidence that a private `cfg(test)` seam cannot reach the doubles is in §4. If one more point must go: `Entered` serves only T4/T5's handle; both could instead read through a hook-installed clock at the first later point, at the cost of T4's coverage of the `now()`-side reads.
2. **T3a / N103-8:** recommend include (decided in F9).
3. **Authority `cancel()` in `FreezeDispatch` (a Module call under `admission`) has F1's shape** (a Provider worker holding a lock the Authority needs while it submits): pre-existing, outside #103. Recommend a separate issue; not fixed here.
4. **The early-end batch drop (neither dispatched nor recorded):** decided: own issue (`decisions.md` Q4).
5. **Mechanism label:** not decided here (F9). Reading: closest in `failure-mechanisms.md` is `mech:planned-state-read-as-actual` (admission judged against a configuration snapshot that dispatch later changes: line 9); the lock-gap case Sol cites is in the same family. The orchestrator weighs it at the record.
6. **Optional trim:** T8's two assertions have no row; recommend keep (cheap drift guards).

## 11. INFERRED, in one place

`try_lock` of a mutex held by the same thread returns `WouldBlock` (T1's first run); a `publishing` Provider without an output declares a usable schedule clock (T2/T3 setup); the batch's id is 1 (T3); `frozen` unreachable after the change (T3, gate); the lib compiled twice makes the doubles' trait types distinct from `crate::`'s (one compile); a refused transition reading the clock first changes nothing observable (`run_session.rs`); no `ezsdr-acceptance` HardwareInLoop Executor+schedule test depends on the order (gate); the pre-existing `cancel()` cycle shape (§10.3); `now()` and `utc_nanos()` are covered together by T4 because both sit in `sample_transition`.
