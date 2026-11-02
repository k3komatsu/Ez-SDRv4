# Brief: note-45 experiment E2a — real-thread executors, ModelDevice, trace forcing, conformance

You are the implementer (AGENTS.md §9). Repository `~/local_work/Ez-SDRv4`, branch `main`. Edit in place: you are the only editor of the tree. Read AGENTS.md first. Do not cd into `v3/`. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`** (E3's holdout) and do not search for it.

**This step is risky** (threads, lifecycle, the boundary between a fake and the device it stands for). It runs in two phases. **Phase 1 (now): write the test plan only, report, and stop. No code.** Phase 2 (implementation) starts only after Sol has reviewed the plan and the orchestrator has said so.

## Context (read these)
- Design: `plan/phase8/45-uhd-decision-core.md` (note 45), especially §4.2–§4.7, §5 (layers 2 and 3) and §7 (C1–C3, the "再現" and "同期" criteria).
- Plan: `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md`. §1 scope, §3 model, §5 oracles I1–I16 and P1–P5, and §10's rows. **The last row records the decisions this brief implements.**
- E1's code (committed, 5d46833): `crates/ezsdr-radio-uhd/src/brain/`. Key files:
  - `core.rs`: `Brain::step` and its ops;
  - `cell.rs`: the cell procedures;
  - `rules.rs`: the executors' pure rules;
  - `model.rs`: the resumable device model;
  - `world.rs`: the DST world, whose executor programs are `SStep`/`RStep` and whose events are `Event`;
  - `dst.rs`: explore and walk;
  - `cases.rs`: the cases, their witnesses, swarm walks with retained witness traces, and the `counterexamples` module with scripted traces.
- The `Device` trait and FakeDevice: `crates/ezsdr-radio-uhd/src/device.rs`. FakeDevice can run on a manual clock (`FakeConfig::clock`, `ClockMode::Manual`, `src/host.rs`).

## What to build (all under `src/brain/`, test-only like the rest of the module)
1. **Thread runtime.**
   - Threads:
     - one brain thread, with an inbox fed by the environment, the executors' completions, boundaries and timers;
     - one settings executor `S`;
     - one receive executor `R_k` per stream;
     - the clock reader.
   - Shared state:
     - one `Mutex<Cell>` per stream;
     - the failure flag `F` (an atomic);
     - the lifecycle gate `G` (a mutex), as note 45 §4.5 procedure 4.
   - Executors reach the device only through `Arc<dyn Device>`.
   - `R_k` publishes into a test sink that records stream, epoch and sample ticks. The Kernel is not involved.
   - The executors follow the same protocol as the DST programs: check, take, call, result, snapshot, publish, gate and recheck, loss publication.
   - **Every decision is made by the existing code:** `Brain::step` and its ops, `cell.rs` procedures and `rules.rs` functions. If the threads need logic that today lives only inside a `World` method, move its pure part into `cell.rs`, `rules.rs` or `core.rs` and call it from both places. Do not keep a second copy (AGENTS.md §9 Maintainability 1 and 2). What remains duplicated between `world.rs` and the threads must be sequencing only. Name each such pair in the plan.
   - The brain panic guard sets `F`, then calls `Device::mark_lost`. After `F`, no handle is closed.
2. **`ModelDevice`.** `model.rs` behind a mutex and condvar, implementing `Device`.
   - Implement the receive and DDC parts that the cases use. Methods outside that set return a fixed value or an error, and the plan lists which.
   - Virtual device time advances by an explicit `advance(ticks)` call. Where `model.rs` returns "not yet", the caller waits on the condvar.
   - Model the native control lock `L` here, as `model.rs`/`world.rs` do: settings, `rx_stop`/`rx_start`/`rx_start_now`, open and time reads take it.
   - A forcing hook chooses the outcome of the next call where the DST does: `SEffect(Outcome)` and `RFault`.
3. **Trace forcing.** A test-build hook that every thread calls at each event point. A controller replays a DST `Event` trace: it releases exactly the actor whose event is next, advances `ModelDevice` on `Tick`, delivers `Env(i)` and `Timer(i)`, and fails if an actor parks at a point other than the trace's next event, naming both. Outside forcing, the hook does nothing.
   - The plan gives the full table: each `Event` variant, the thread point it maps to, or "merged into X" with the reason.
4. **Observation record.** One `Observed` value extracted from a finished `World` and one extracted from a finished thread run, compared by equality. It holds:
   - dispositions;
   - command terminal outcomes;
   - publications;
   - the device application history (value, tick, channel);
   - each streamer's device calls in order;
   - which actors exited.
5. **I10 oracle.** A thread-local count of held cell guards. A checking `Device` decorator panics if a device method is called while the count is non-zero.
6. **Conformance suite.** The same scripted `Device`-trait call sequences run on FakeDevice (manual clock) and on `ModelDevice`, comparing the returned values, errors and application ticks.
   - Fix a difference that is the model's fault, with a test and a row.
   - List each remaining difference with its reason (for example, the model deliberately omits per-block delivery). Do not change FakeDevice.
7. **FakeDevice smoke runs** on the real clock: a few, per the sequences below.

Out of scope: speed measurement (E2b), FakeDevice changes, production code outside `src/brain/` (allowed only where E1 already opened visibility, and named in the plan), `design/` text, the Kernel, schemas and Python.

## Sequences that matter (each needs a test; the expected result is what must hold)
Hardware and UHD behaviour relied on is as in the plan's §7:
- one caller per streamer handle: VERIFIED;
- settings serialize under the native control lock and wait when the radio command queue is full: VERIFIED (`src/uhd.rs:903-906`);
- `rx_stop`, opening a streamer and time reads take the same lock: VERIFIED in source;
- DDC consumption at acceptance, one value per eligible sample: FakeDevice's model, INFERRED for hardware.

| # | Sequence | Expected |
|---|---|---|
| S1 | Forced replay on threads and `ModelDevice` of (a) every scripted trace in `cases.rs::counterexamples` that is a default test, and (b) the first retained trace of every declared witness of every case. For (b), use a fixed seed range from the swarm sweep; the plan names the range and the case list. | The thread run's `Observed` equals the `World`'s `Observed` at the trace end, and no actor diverges from the trace. A wrapper that reorders the protocol fails (publish without the recheck, a call before take, a send before the result is written). |
| S2 | A device call made while a cell guard is held | The checking decorator panics. One self-test of the oracle. A mutation row makes S hold its cell across `apply`. |
| S3 | `F` set, then S takes a queued settings command | No `Device::apply` call is recorded; the command's terminal outcome is `Abandoned` (I1). |
| S4 | `F` published before R's lifecycle call takes `G` | The recheck inside `G` refuses: no `rx_close` or `rx_open` call (I14). The handles are retained: no close after `F` (I9). |
| S5 | Brain panic at a chosen step, on threads | `F` is set before the `mark_lost` call, in the decorator's call order. Every executor exits at its next check, and all threads join within 1 s. No close after `F`. |
| S6 | FakeDevice, real clock: two streams streaming, a timed DDC value consumed, then Shutdown | Per stream, the device calls run stop → flush → restore → terminate. Every value and command is disposed exactly once (P2), and all threads join within 1 s. |
| S7 | FakeDevice: a value released while streaming | Applied before its instant and consumed by the sample at its instant: the application tick FakeDevice records equals the consumption tick in `Observed`. |
| S8 | Native-lock contention: S waits in `apply` with a full command queue, holding `L`; the drain deadline arrives; R's stop waits for `L`. Use `ModelDevice` if FakeDevice cannot hold the queue full deterministically; say which. | No deadlock. The drain-ended transition happens at the deadline (I16), and the stop is issued and waits for `L` (P1). After `apply` returns, the value is handled as the brain's rules say (pinned, then `left`). All threads join once `L` frees. |
| S9 | Conformance (item 6): (a) a timed start, receives, and a stop with its tail; (b) a start queued behind a stopped tail, promoted at the next recv entry, and late → `LateCommand`; (c) a timed start in the past → missed start, and the next recv gets `LateCommand`, while `rx_start_now` is never late; (d) timed applies: the application tick, two equal-time values on distinct samples, DSP FIFO capacity 32; (e) a full radio command queue | Equal results, or a listed difference with its reason. |
| S10 | Plan change on threads: a restart that advances the epoch while R is inside a recv | The old epoch's buffer is published only up to the retiring cut, and the rest is discarded and counted. The quiesce ack comes after the publication, and no start is sent before the ack (I6, I12). |

Thread tests whose verdict can depend on timing are run 20 times in phase 2 (AGENTS.md §9 step 8). Forced replays are deterministic by construction. Say which tests are which.

## Phase 1 deliverable: `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan-e2a.md`
- For each sequence S1–S10: the test name, what it asserts, how the assertion observes the behaviour, and the wrong behaviour that would fail it (AGENTS.md §9 step 2).
- The `Event` → thread-point table (item 3).
- The `Observed` fields, and how each is extracted on both sides.
- The list of logic moved out of `World` into `cell.rs`/`rules.rs`/`core.rs`, and the remaining sequencing pairs.
- The `ModelDevice` method list: implemented, or fixed value or error.
- The file layout, and a size estimate in lines for code and tests. The ceiling is 2,000; say so if you expect more.
- The mutation rows you plan, one line each, from E2-01. Token anchors for Rust (AGENTS.md §7).
- Open questions for the orchestrator, if any.

Phase 1 reads code but changes nothing in the tree.

## Build and test
- Local cargo is allowed again (owner 2026-11-02). Build output stays in `~/.cache/cargo-target/Ez-SDRv4`. Test temp files go on `/tmp`.
- Never run `mutate.py` or `remote_mutate.py`.
- Never git commit, push, stash, reset or checkout.

## Report (phase 1, at most 15 lines)
The plan's path, the size estimate, the open questions, and anything in this brief you think is wrong, with the reason.
