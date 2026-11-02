# Test plan: design-note-45 receive slice, stages E1a and E1b

Status: draft 4 (2026-11-01, implementer = Claude Haiku subagent, test-plan stage only; amended to 45-experiment draft 4, which adopts Sol's re-review (REVISE, R1–R5 and the clarifications) and the coordinator's test-plan adjudication (45-experiment §9–§10)). Contract: [45-experiment.md](../45-experiment.md) (scope, events, cases, oracles, U1–U18). Design: [plan/phase8/45-uhd-decision-core.md](../45-uhd-decision-core.md). Behaviour relied on: [plan/phase8/44-device-chain-state.md](../44-device-chain-state.md) §3, [plan/phase8/39-dsp-timed-commands.md](../39-dsp-timed-commands.md) §6.6–§6.8.

Read: the four files above, `crates/ezsdr-radio-uhd/src/provider/core.rs:86-372` (`RxEntry`, `SegmentKey`, `RxDevice`, `RxChain`), `crates/ezsdr-radio-uhd/src/device.rs` (`RxStreamer` 473, `DSP_FIFO` 501, `Dsp` 507, `open_flow` 558, `close_flow` 568, `Fake` 742, `settle` 1028, `drain` 1047, `start_rx` 1169, `apply` 1407, `rx_stop` 1574, `rx_recv` 1604), `lib.rs`, `provider/mod.rs`, `mutations.json` format, `mutate.py` (filter and test_bin use). The holdout directory was not opened. Nothing was built or run.

Notation: `Done{x}` is the brain input `Done` carrying the terminal `x` (`Call(Ok)`, `Call(Partial)`, `Call(Lost)`, `Dropped` or `Abandoned`). `P` = packet size 4 samples; time `T` and every instant are in ticks, one tick per sample. `e` = a value's instant. "Done" in the cell is the note's "終わった"; a disposition (one per value) is the note's "始末した".

---

## Summary of what this plan adds

- Section A: module sketch, types (terminal command outcomes, verdicts), the draft 4 event list (per-sample `Tick` up to the horizon and epilogue, `B.op` with conditional results, `R.recv_begin`/`R.recv_end`, `S.result`/`S.send`, `S.check` writing `Abandoned`, `Clock.*`, `R.gate`/`R.lifecycle`/`R.ungate`, the split `Loss.*` events, `Timer` observations), the reuse table, the cell procedures.
- Section B: 60 pure-level tests for U1–U18 (draft 3: 54; draft 4 adds u04d, u07d, u09e, u09f, u09g and u13c).
- Section C: 30 harness tests (draft 3: 21): search key and monitor, edge-once DFS, diagnostic counters, scheduler window, epilogue and `Incomplete`, witnesses, the production `Done{Dropped}` producer, `S.result` before `S.send`, loss publication order, violations before pruning, conditional sends, tick boundary.
- Section D: cases 1–9 and the seeded configuration: 24 test functions. Each case has an exhaustive (`#[ignore]`) and a reduced variant; case 7 has three sub-cases. Every case declares witnesses, which the test asserts.
- Section E: 27 oracle tests (I1–I9, I11–I16, P1–P4; draft 3: 23). P5 is the cycle test in section C; I10 is not an oracle (G13).
- Section F: 67 mutation rows, each with a killing test (E1-01 to E1-55 from drafts 2 and 3; E1-56 to E1-67 for draft 4).
- Section G: 20 open questions, resolved in draft 2; G2, G8, G9 and G14 revised in draft 3.

### Changes in draft 4 (45-experiment §10)

- Terminal command outcomes: `Terminal = Call(Outcome) | Dropped | Abandoned` (§A types, the `Done{x}` notation, U10, U13c, P2). `S.check` writes `Abandoned` for each discarded settings command, in its own cell section, so no brain is needed after a panic (R2). `R.check` writes nothing, because Rx commands have no settings terminal (contradiction 8).
- Epilogue and `Incomplete` (R1): `Tick` runs to horizon + epilogue; `Timer` is enabled only at its deadline; an actor still waiting at the end of the epilogue gives the verdict `Incomplete`, which fails the case and is not a violation. The horizon-end timer rule of draft 3a is withdrawn. Tests: `harness_epilogue_covers_deadlines`, `harness_timer_never_early`, `harness_deadline_beyond_last_input_is_observed`, `harness_recv_timeout_loop_at_end_is_incomplete`.
- I16, the drain-ended transition made while a pin exists (R3): u04d, `inv_i16_drain_ended_waits_for_pin_is_violation`, E1-59; case 5 witness W5c.
- I15, channel-wide restore eligibility (R4): u09e, u09f, `inv_i15_restore_with_left_on_channel_is_violation`, `inv_i15_restore_with_debt_on_other_stream_is_violation`, E1-58; case 5 extended through leftover consumption (W5d); case 9 gets `Request(Restore: ch0)` (contradiction 9).
- Loss publication split into `Loss.gate`, `Loss.set`, `Loss.ungate` and `Loss.mark`, after `L` is released (R5): `harness_loss_publish_releases_l_first`, `harness_lifecycle_inline_loss_publish_no_recursion`, E1-56, E1-57; case 8 witnesses W8a–W8d; u13b's log; E1-48 revised.
- Conditional `B.op` (`harness_failed_reservation_leaves_no_send`, E1-63); recv identity kept on recompute (u07d, E1-60); invariants checked before the visited-set lookup (`harness_violation_into_visited_state_is_caught`, E1-61); tick boundary (`harness_first_tick_produces_sample_0`, E1-62).
- Overrun report fields (§D, `harness_budget_overrun_is_failure`). Case 7c: no further streamer call after the failed begin. Cases 2, 6 and 7a use the terminal notation.
- After Sol review 3 (in place, still draft 4): the next segment is not held back by debt or `left` (only restore is, I15): U9, u09a, u09g (new), case 5 (W5d reached through the next segment's first sample). The loss-publication test checks that the publisher does not hold `L`, not that `L` is free. u07d advances the cell to epoch 2 before the recomputation (the cut-only case stays in u07c). The monitor identities (`Ident`: value, channel, segment, epoch, sample) are in the types, the key test and I4. E1-67 (new) kills an over-restricted next segment.
- Fixes: E1-43 names `u15f_recv_delivers_buffered_samples_in_order` (it said `packets`, which matched no test). Stage items 1–5 are removed (resolved in draft 3a); items 8–10 are added.

### Changes in draft 3 (45-experiment §10, history)

- Granularity: per-sample ticks with the input-window rule; a recv's entry and its return are separate events (cases 3 and 7 need the epoch of a call in progress); clock lock, read, release and send are separate; brain steps run as ops.
- Lifecycle gate `G` and I14 (case 8). I1 keeps the ordinary-call window; I14 is the stronger rule for open, close and rate reads.
- Three kinds of fact replace "settled": device application, qualifying evidence (including unpublished drain samples), publication. A value has one disposition; a settings command has one result; a leftover consumption is a separate fact. P2 counts dispositions and command results.
- P1 is stated after the deadline's observation. The scheduler contract (§C, §D) is the only physical-time assumption. Witnesses make a vacuous case fail.
- Cases 3, 5 and 7 are rewritten; cases 8 and 9 are new; the seeded configuration has a channel move, partial and refused outcomes.
- Sol's minors 8–10: E1-42 goes through the dst producer; E1-30 counts edges on a reconverging toy; U15d follows `start_rx`; U12d is an adapter check; U7b is the snapshot revision test; the key and the diagnostic counters are separated.
- Contradictions found while amending are listed in the stage notes (end of this file); they need the orchestrator's decision.

## A. Module sketch

New files under `crates/ezsdr-radio-uhd/src/brain/`; `lib.rs` gets `#[cfg(test)] mod brain;`.

| File | Holds |
|---|---|
| `mod.rs` | module doc, re-exports |
| `core.rs` | `Brain`, `Input`, `Command`, `Op`, `Disposition`, `CommandResult`, `Brain::step` |
| `cell.rs` | `Cell`, `CellSet`, `Phase`, `InFlight`, `ValueState`, `Snapshot`, `Publish`, `Take`, the cell procedures |
| `rules.rs` | the executors' pure rules: `consume_packet` (via the production `RxChain::consume`, see below), `plan_publish`, epoch discard count |
| `model.rs` | the resumable device model `Device` (D). Each function names the FakeDevice function it mirrors |
| `dst.rs` | `World`, `Event`, `enabled`, `apply`, `key`, `explore` (DFS), `walk` (seeded), `replay`, `shrink`, the R and S executor effects (E1b) |
| `oracle.rs` | I1–I9, I11–I16, P1–P5 as functions over `World` and the monitor (E1b) |
| `cases.rs` | scripts of cases 1–6, the seeded configuration, the reduced variants (E1b) |
| `toy.rs` | deliberately broken toy models for the harness tests (E1b) |

The split into `oracle.rs` and `toy.rs` keeps `dst.rs` below the 1,500-line stage budget. It is a deviation from the list in 45-experiment §1 only in file names.

### Types (only the fields the tests name)

```
Brain   { held: Vec<Held>, dispositions: Vec<Disposition>, results: Vec<CommandResult>,
          outbox: Vec<Command>, deferred: Vec<Request>, ops: VecDeque<Op>,
          drain: Option<Deadline>, next_id: u64, guard: Guard }
Op      = Section(CellSection) | Send(Actor, Command)           /* one event each (B.op) */
Input   = Release(Held) | Stop { requested: i64 } | PlanCancel(Plan) | Request(Request)
        | Done { cmd: CmdId, terminal: Terminal } | Boundary(Boundary) | Timer(Timer) | Clock(i64)
        | LateCommand(stream) | Loss | Panic | Shutdown
Request = Restore | Move | Close | Open | StartNext | Flush        (U9; Flush: case 7b)
Command = Settings(SetCmd { id, generation, change: Timed{channel, freq, at} | Restore })
        | Rx { stream, op: Start{at} | Stop{at} | Flush | Open | Close | Plan{segment, cut, epoch} | Terminate }
        | Timer(Timer, at) | Record(Record)
Outcome = Ok | Partial | Refused | Lost   /* a call result, only for a call actually entered */
Disposition   = Consumed | Cancelled | Reported(reason) | Refused     /* one per released value (I4, P2) */
Terminal = Call(Outcome) | Dropped | Abandoned   /* one per settings command: a call result; Dropped = cancelled before its call; Abandoned = discarded after F, written by the discarding executor */
CommandResult = Terminal   /* one per settings command */
Ident   { value: ValueId, channel: Channel, segment: SegmentKey, epoch: u64, sample: i64 }   /* the reservation's segment and epoch; sample = the matching sample tick */
Monitor { applied: per value Option<Ident> (device application: the sample D applied it to),
          evidence: per value Option<Ident> (qualifying receive evidence: the consuming sample, published or not),
          leftovers: per value Option<Ident> (a leftover consumption, a separate fact),
          published: progress per epoch, dispositions, results,
          deadlines: observed flags, drain-ended flag per chain (I16), witnesses: set of Witness }   /* finite summaries, not histories (§C) */
Cell    { phase: Phase, channels: Channels, generation: u64, epoch: u64, plan_cut: i64,
          retiring_cut: Option<i64>, progress: i64, in_flight: Vec<InFlight>, revision: u64,
          quiesce: Option<u64>, recomputes: u32 /* diagnostic */, discarded: u64 /* diagnostic */ }
Phase   = Off | Armed(SegmentKey) | Streaming(SegmentKey) | Stopping(SegmentKey)
          | Flushing | Uncertain | Lost   /* note 44 §3.2 device states (G15) */
InFlight { entry: RxEntry, state: ValueState }
ValueState = Queued | Cancelled | Taken | MaybeApplied | Done(Terminal)
Snapshot { epoch: u64, cut: i64, progress: i64, revision: u64 }
Device  { t: i64, queue: VecDeque<Queued>, queue_cap: usize, last_command: i64,
          dsp: BTreeMap<Channel, Dsp>, streamers: Vec<Streamer>, lost: bool, calls: Vec<Call> }
Dsp     { fifo: VecDeque<(i64, f64)>, log: Vec<(i64, f64, bool)>, flows: Vec<(i64, Option<i64>)>, cursor: i64 }
Streamer{ open: bool, channels: Vec<Channel>, next: Option<i64>, end: Option<i64>,
          then: Option<i64>, origin: i64, buffered: VecDeque<i64> /* ticks of received samples */ }
World   { brain: Brain, cells: CellSet, settings: SettingsExec, rx: Vec<RxExec>, device: Device,
          native: Option<Holder>, gate: Option<Holder>, failure: bool /* F */, env: Vec<EnvSlot>,
          monitor: Monitor, trace: Vec<Event> }
SettingsExec { fifo: VecDeque<Command>, in_hand: Option<(Command, Phase)> }
RxExec  { fifo: VecDeque<Command>, pc: LoopPos, handle: Handle /* Open | Closed | LostRetained */,
          recv: Option<RecvCall> /* { epoch, len, timeout, one_packet } while in recv */ }
Holder  = Settings | Rx(usize) | Clock
Verdict = Pass | Violation { rule, trace } | Cycle { trace } | Deadlock { actors } | BudgetExceeded { report } | Vacuous { witness } | Incomplete { actor, trace }   /* Incomplete: an actor still waiting at the epilogue end; it fails the case and is not a violation (45-experiment §3.2) */
```

Pins are not a stored field. A chain is pinned when some `InFlight` of its cell is `Taken` or `MaybeApplied` (note 45 §4.4). Outstanding consumption debt (a value whose call finished but no qualifying sample has consumed it) is not a pin; it is held in the monitor's evidence and in the leftovers. Keeping the two apart is Sol's review item 6.

### Reuse and deviations

| Reused as is | From |
|---|---|
| `RxEntry`, `SegmentKey` | `provider::core` (needs `pub(crate) mod core;` in `provider/mod.rs`, see G16) |
| `RxChain::consume` (the rule) | `provider::core:307`, called through a scratch chain in `rules::consume_packet` (no copy) |
| `timeline::Channels`, `device::{Settings, DeviceError, Iq}` | `ezsdr-radio`, `device.rs` |

| Not reused | Reason |
|---|---|
| `RxDevice`, `RxIntent`, `RxChain` lifecycle | E1 does not reuse the chain's lifecycle type. Its `Phase` carries the note-44 device states that case 7 and U16–U18 need (`Flushing`, `Uncertain`, `Lost`, G15) next to the note-45 set (`Off`/`Armed`/`Streaming`/`Stopping`). The chain's `debt`/`left`/`latest` lists are the old representation of the same facts that the cell's `InFlight` list holds. |
| `DSP_FIFO` (`device.rs:501`) | private to `device`. The model takes the capacity as a parameter (32, from the constant) and E2 asserts equality (G17). |

### Cell procedures (signatures; each takes `&mut Cell`, never the device)

```
fn reserve(&mut self, entry: RxEntry, generation: u64, segment: SegmentKey) -> Result<(), Refusal>
fn book_stop(&mut self, requested: i64) -> i64            // cut = min(plan_cut, max(requested, progress)); cancels Queued with e >= cut
fn take(&mut self, order: (i64, u64)) -> Take             // Queued -> Taken; Cancelled -> Dropped
fn start_call(&mut self, order)                           // Taken -> MaybeApplied
fn finish_call(&mut self, order, outcome: Outcome)        // MaybeApplied -> Done(outcome)
fn stop_accepting(&mut self) -> Vec<Obligation>           // Stopping transition: Queued -> Cancelled; returns Taken and MaybeApplied
fn snapshot(&self) -> Snapshot
fn publish(&mut self, seen: Snapshot, progress: i64) -> Publish   // Published{progress} | Stale (epoch, cut or revision changed; recomputes += 1); the consumption evidence is from rules::consume_packet, matched by value identity
fn advance_epoch(&mut self, plan: Plan)                   // retiring_cut := the cut in force (the Stop's, if it shortened it)
fn ack_quiesce(&mut self, epoch: u64) -> bool
```

Maintainability note for review: `book_stop` (cancel Queued with e ≥ cut) and `stop_accepting` (cancel every Queued) are two cancellation sites. They should share one `cancel_queued(pred)` helper. Named here so the implementer does not write two copies.

### Events (45-experiment §3.2, draft 4)

Each event is one atomic step. The test plan names the events the tests use.

| Event | Actor | Used by |
|---|---|---|
| `Tick` (one sample: `T += 1`; D produces and consumes the sample and appends it to each running streamer's buffer; disabled when an undelivered input has `latest ≤ T`, or when `T` reaches horizon + epilogue; the first Tick produces sample 0) | D | every device case; `harness_time_only_advances_on_tick`, `harness_tick_cannot_pass_mandatory_input` |
| `Env(x)` (delivery to the brain inbox, or an effect: Loss, Panic) | environment | cases 1–9, seeded |
| `B.step` (pure computation; queues its ops) | brain | U1–U18 |
| `B.op` (one cell section or one send; a cell result feeds the remaining ops, so no send precomputed before a failed section survives) | brain | U1–U18 in the harness; `harness_failed_reservation_leaves_no_send`; `harness_result_precedes_send` |
| `S.take` (queued → taken; cancelled → no call, `Done{Dropped}` queued) | S | u03, `harness_take_cancelled_sends_done_dropped`, case 4 |
| `S.check` (reads `F`; set → discard the FIFO and write `Abandoned` for each discarded settings command, one cell section each; exit) | S | I1, U13, u13c |
| `S.lock` / `S.effect(outcome)` / `S.unlock` | S | U4, cases 2, 5, 7a, 9 |
| `S.result` (cell section writing the terminal: a call result, or `Dropped`/`Abandoned` where no call was entered) | S | u10, u11a, case 9 |
| `S.send` (`Done` enters the brain inbox) | S | `harness_result_precedes_send` |
| `R.cmd` (loop top: start, stop, close, open, plan change, flush, terminate; a brain-commanded stop makes the Stopping transition before its call) | R | u05c, cases 3, 7b |
| `R.check` (reads `F`; set → discard the FIFO and exit; Rx commands carry no settings terminal, so nothing is written) | R | U13a |
| `R.snap` (copies epoch, cut and revision) | R | u07a–c, u08a, case 3 |
| `R.recv_begin` (records handle, the epoch of its snapshot, requested length, timeout, `one_packet`) | R | case 3, u08a |
| `R.recv_end` (returns buffered samples up to the length; or timeout; or failure when lost) | R | u08a, u15f, case 3 |
| `R.publish` (refused if epoch, cut or revision changed since `R.snap`; a refused publish recomputes from a fresh snapshot and keeps the recv's handle, epoch and sample identities; else writes progress and consumption and, at the cut, the Stopping transition) | R | u07a–d, u08a–b, case 3 |
| `R.lock` / `R.call(op)` / `R.unlock` (`rx_start`, `rx_start_now`, `rx_stop`) | R | case 5, case 7b, u17 |
| `R.gate` / `R.lifecycle(op)` / `R.ungate` (open, close, rate reads: take `G`, recheck `F` inside it, call, release `G`; a native open also takes `L` under `G`; order `G` then `L`; loss publication never holds `L`, R5) | R | case 8 |
| `R.result` (writes the phase and device state after a call) | R | u17a–d, cases 7b, 7c |
| `R.send` (boundary input) | R | U9, U11 |
| `Clock.lock` / `Clock.read` / `Clock.unlock` / `Clock.send` | clock reader | `harness_clock_read_holds_native_lock` |
| `Timer(d)` (enabled only when `T ≥ d`, never early; the delivery is an observation in the monitor) | brain | U4, case 5, P1; `harness_timer_never_early` |
| `Loss.gate` / `Loss.set` / `Loss.ungate` / `Loss.mark` (the actor that saw a call return lost first returns from the native call and releases `L`; then takes `G`, sets `F`, releases `G`, then calls `mark_lost`; a lifecycle caller that already holds `G` sets `F` inside it, after releasing `L`, and does not re-enter `Loss.gate`; the panic guard runs the same four events) | the actor that saw a loss; the guard | case 2, case 6, case 8; `harness_loss_publish_releases_l_first`; `harness_lifecycle_inline_loss_publish_no_recursion` |
| Epilogue end (no timer before its deadline; the run ends at horizon + epilogue; an actor still waiting there is `Incomplete`) | — | `harness_epilogue_covers_deadlines`; `harness_deadline_beyond_last_input_is_observed`; `harness_recv_timeout_loop_at_end_is_incomplete` |

### Executor rules and the device model

- `rules::consume_packet(left, debt, sample_ticks, channels, first_tick, samples)` (name kept): builds a scratch `RxChain` (`adopt_start`, `hand` for each entry, `stop_result(Ok)`, `finish` to move the left list, `adopt_start` again, `hand` for the debt entries), then calls `RxChain::consume`. The harness calls it per sample. Its result is consumption evidence whether or not the sample is published (Sol 3).
- `rules::plan_publish(snapshot, recv)`: the samples to publish (up to `min(cut, recv end)` for a recv begun under the snapshot's epoch) and the samples to discard.
- `Device::tick()` (one sample) mirrors FakeDevice's per-sample progression of `drain` and `settle`. `Device::apply_timed` mirrors `FakeDevice::apply`, timed branch (`device.rs:1407-1470`): it returns `Blocked` when the queue is full and `Refused` when an RX DSP FIFO is full; the effective instant is `max(at, last_command, now)` (`device.rs:1433`).
- `Device::settle` mirrors `settle` (1028). `Device::drain` mirrors `drain` (1047), head-of-line (G6). `Device::start` mirrors `start_rx` (1169), including the stopped-tail rule (U15d). `Device::stop` mirrors `rx_stop` (1574). `Device::recv` mirrors `rx_recv` (1604) for the length and the timeout only; it returns buffered samples, never generated ones (G8). `open_flow`/`close_flow` mirror 558/568.
- Host buffer: a streamer's buffer holds sample ticks. A stop's tail delivers its buffered samples before the end, as the FakeDevice tail does. The depth is an E2 conformance question. Shortcut: not checked against hardware, INFERRED.

## B. Pure-level tests (U1–U18)

Format: name; setup and operation; what is asserted; how the assertion observes (returned value, cell field, outbox, disposition list, device call list); wrong behaviour that fails it. Each test has one or two assertions. U2 and U5 are split; U1 has a boundary test.

### U1 — reservation of a value in a streaming segment

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u01_streaming_value_in_segment_reserves` | Cell `Streaming(s)`, s origin 0, channels {0}, generation 1, plan_cut 16, progress 4. Held v (ch 0, e 8, id 1, gen 1). `step(Release v)` | (a) `in_flight` holds v in `Queued`; (b) outbox is exactly one `Settings(Timed{ch0, e 8, gen 1, id 1})` | `cell.in_flight`, `outbox` | Reservation skipped (value stays held), or command without its generation or id |
| `u01b_armed_value_reserves` | As u01 with phase `Armed(s)` | (a) as u01 (a) | `cell.in_flight` | Armed refused: the start path would never get a value (G1: I2 says Armed reserves) |
| `u01c_instant_equal_origin_reserves` | As u01 with e = origin = 0 | (a) as u01 (a) | `cell.in_flight` | `e > origin` instead of `e >= origin` at the boundary |

### U2 — refusals

Each row starts from u01 and changes one condition; the test asserts the value stays Held and the outbox is empty (one assertion: the returned `Refusal` variant, plus the empty outbox).

| Test | Change from u01 | Wrong behaviour caught |
|---|---|---|
| `u02a_off_refuses` | phase `Off` | reservation without a segment |
| `u02b_stopping_refuses` | phase `Stopping(s)` | I7: reservation after the stop transition |
| `u02c_other_channel_refuses` | value on ch 1, cell channels {0} | channel check missing |
| `u02d_stale_generation_refuses` | value gen 0, cell gen 1 | generation check missing (note 45 §4.5 procedure 1) |
| `u02e_instant_before_origin_refuses` | origin 4, e 2 | origin check missing |
| `u02f_instant_at_cut_refuses` | e 16, plan_cut 16 | `e <= cut` instead of `e < cut` |
| `u02g_progress_at_cut_refuses` | progress 16, plan_cut 16, e 8 | progress check missing (I2's `progress < cut`) |

Note: the U2 row in 45-experiment lists `phase Armed` as refused. That contradicts I2 and note 45 §4.5 procedure 1 (Armed accepts). The tests follow I2 (u01b). See G1.

### U3 — cancel before take

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u03_cancel_before_take_drops_without_call` | Queued v (e 8, progress 4, plan_cut 16). `book_stop(8)` gives cut 8, so v is Cancelled. Then `take(v)` returns `Dropped`; the brain receives `Done{Dropped}` (fed by the test as the input the dst producer sends) | (a) `take` returns `Dropped`; (b) on `Done{Dropped}` the brain writes exactly one disposition `Cancelled`; a duplicate `Done` for the same id adds nothing | return value, `brain.dispositions` | Cancelled value gets a device call; disposed zero or two times. The producer of `Done{Dropped}` is tested in section C (E1-42) |

### U4 — taken, then Stop

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u04_taken_then_stop_is_pinned_not_cancelled` | v Taken (take done). `book_stop(8)` gives cut 8 | (a) v still `Taken`; (b) outbox holds `Timer(Drain, 16)` (deadline = cut + 8) and the chain is pinned (a `Taken` value exists) | `cell.in_flight`, `outbox` | v cancelled after take (device may still apply it); no drain obligation |
| `u04d_drain_deadline_sends_stop_while_pinned` | v Taken pins the chain; `Input::Timer(Drain)` is delivered at its deadline while the pin stands | (a) the step's ops send `Rx{Stop}` to R while v stays `Taken`; (b) the stop is not deferred for the pin | `outbox`, `cell.in_flight` | the drain-ended transition waits for the pin (I16, R3; E1-59) |

### U5 — Stopping transition

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u05a_stopping_cancels_queued_and_lists_obligations` | Streaming cell with one Queued q, one Taken t, one MaybeApplied m. `stop_accepting()` | (a) q is `Cancelled`; (b) returned obligations are exactly {t, m} | `cell.in_flight`, return value | Queued value still applied after stop; obligation missing |
| `u05b_reservation_after_stopping_refused` | After u05a, `Release(v2)` | v2 stays Held, outbox empty | `brain.held`, `outbox` | I7 broken |
| `u05c_brain_stop_closes_acceptance_before_call` | Streaming cell with a held v2; the brain decides a Stop (`step(Stop)`) before R's `rx_stop` runs | (a) phase is `Stopping` before any `R.result`; (b) `Release(v2)` after the decision leaves v2 `Held` | `cell.phase`, `brain.held` | acceptance still open during the device stop (I7; note 45 §4.5: the stop decision closes acceptance in the same transition) |

### U6 — Stop booking

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u06a_stop_below_progress_clamps_to_progress` | plan_cut 16, progress 8, `book_stop(4)` | returned cut == 8 | return value | cut below published progress (I5) |
| `u06b_stop_beyond_plan_cut_keeps_plan_cut` | plan_cut 16, progress 4, `book_stop(20)` | returned cut == 16 | return value | Stop extends the cut (no `min` with plan_cut). See G11 |

### U7 — publish with a stale snapshot

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u07a_stale_snapshot_refused_and_counted` | `seen = snapshot()` (cut 16, progress 4). `book_stop(6)` gives cut 6. `publish(seen, progress 8)` | (a) result is `Stale`; (b) `recomputes == 1` | return value, `cell.recomputes` | stale write accepted; not counted |
| `u07b_result_between_snapshot_and_publish_refused` | `seen = snapshot()` (cut 16, progress 4, revision r). A settings result for another value is written (revision r+1). `publish(seen, progress 8)` | (a) result is `Stale` and `recomputes == 1`; (b) the written settings result is unchanged and no entry is discharged | return value, `cell.recomputes`, `cell.in_flight` | publish ignores the revision: it overwrites the new result, or discharges an entry on acceptance information that has changed (Sol 9; E1-46) |
| `u07c_recompute_publishes_only_to_new_cut` | After u07a, `publish(snapshot(), progress 6)` | `cell.progress == 6` | `cell.progress` | recompute publishes past the new cut (samples 6..8 would be published) |
| `u07d_recompute_keeps_recv_identity` | epoch 1, cut 16, progress 4. `R.snap` (epoch 1). A recv begun under epoch 1 with handle h buffers samples 4..7. `book_stop(6)` (cut 6). `advance_epoch(epoch 2, origin 20, plan_cut 48)` (retiring cut 6). `R.publish` with the epoch-1 snapshot recomputes from the epoch-2 snapshot | (a) the recv's epoch is still 1 and its handle is still h; (b) samples 4 and 5 publish with epoch-1 identity, and 6 and 7 are discarded (`cell.discarded == 2`) | a recompute that gives the old recv the current epoch (2) publishes 4..7 up to the epoch-2 cut, so (b) fails (E1-60). The cut-only recompute is u07c |

### U8 — epoch advance while R is in recv

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u08a_old_buffer_published_to_retiring_cut` | epoch 1, cut 16, `book_stop(10)` (cut 10). `R.recv_begin` under epoch 1; `advance_epoch(epoch 2, origin 20)` makes retiring_cut 10. The recv returns samples 8..11 (`R.recv_end`) after the advance; `plan_publish` publishes up to the retiring cut | `cell.progress == 10` | `cell.progress` | published to the plan cut 16 or to the recv end 12 |
| `u08b_rest_of_old_buffer_discarded_and_counted` | As u08a | `cell.discarded == 2` (ticks 10, 11) | `cell.discarded` | discarded samples not counted |
| `u08c_quiesce_ack_after_publish` | As u08a. `ack_quiesce(1)` before publish, then after | (a) false before publish; (b) true after | return values | ack before publish (I6) |

### U9 — pinned chain, deferred requests

Each test: a Taken value pins the chain; the request arrives; (a) no command while pinned; (b) after `Done{Call(Ok)}` for the value, exactly one command, unless the row says more is needed. The next segment (u09d, u09g) waits for pin clearance and the finish and quiesce conditions only. Restore also waits for channel-wide debt and `left` (I15: u09a, u09e). Move, close and open wait for pins only (I3; case 9); see stage notes, item 11.

| Test | Request | Wrong behaviour caught |
|---|---|---|
| `u09a_restore_deferred_while_pinned` | Restore; `Done{Call(Ok)}` for v; then a sample at or after v's instant consumes v (the debt is consumed first) | restore issued while v may be on the device (I3), or expected straight after `Done{Call(Ok)}` although debt stands (contradicts u09e); (a) no restore while Taken, (b) exactly one restore after the consumption |
| `u09b_move_deferred_while_pinned` | Move | move issued while pinned |
| `u09c_close_and_open_deferred_while_pinned` | Close, then Open | close or open of the streamer while pinned |
| `u09d_start_next_deferred_while_pinned` | StartNext | next segment started while pinned |
| `u09e_restore_waits_for_debt_or_left_on_channel` | v applied and its call finished, so debt and no pin on ch0; Restore of ch0 requested | no restore while the channel holds debt or `left` (I15; note 44 §3.7); a rule that checks pins only fails (E1-58) |
| `u09f_restore_waits_for_left_on_other_chain` | after a move, stream 0's chain holds `left` on ch0 and stream 1 now carries ch0; Restore of ch0 requested on stream 1; then the `left` entry is consumed by a sample | (a) no restore while the `left` exists, though no pin does; (b) one restore after the consumption (I15, R4) |
| `u09g_start_next_not_blocked_by_debt_or_left` | v Taken; StartNext requested; `Done{Call(Ok)}` for v, so v is debt; the segment finishes, so v is `left`; no sample has consumed it | one start is issued while v is still debt or `left` (the start is not held back by them) | a next segment held back by debt or `left`, so the samples that would consume them never run (Sol 3; case 5 W5d; E1-67) |

### U10 — partial failure

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u10_partial_result_is_maybe_applied_not_failure` | MaybeApplied v. `Done{Call(Partial)}` | (a) v is `Done(Call(Partial))` and no longer pins the chain; (b) the command's terminal is `Call(Partial)` and no disposition is written yet | `cell.in_flight`, `brain.results` | partial counted as failure; partial still pins; a disposition written before the segment's finish |

G18 says what happens to v after `Done(Partial)`. The assertion (a) follows the note's "終わった" (pin released). The value's carry to the next segment is covered by u11b.

### U11 — late completion after the segment finished

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u11a_late_completion_recorded_once` | v MaybeApplied; `finish()` of the segment (G5: allowed while pinned); later `Done{Call(Ok)}` for v, then the same `Done` again | (a) exactly one command result for v's call; (b) v's disposition is the report written at finish (`left`), written once; the second `Done` adds nothing | `brain.results`, `brain.dispositions` | a second result or disposition; the report at finish erased |
| `u11b_later_sample_is_leftover_consumption` | After u11a, the next segment of the stream; `R.recv_end` returns a sample at tick ≥ e | a leftover consumption fact for v is recorded; v's disposition is unchanged | `brain.leftovers`, `brain.dispositions` | the later consumption written as a second disposition (I4), or not recorded |
| `u11c_next_start_waits_for_late_completion` | v still MaybeApplied after finish; `begin` requested | next start not issued (deferred) | `outbox`, `brain.deferred` | next start while pinned (I3) |

### U12 — consumption

| Test | Setup | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u12a_equal_time_values_take_distinct_samples` | debt [v1 (e 8, order (8,1)), v2 (e 8, order (8,2))], samples at ticks 8, 9, 10 | consumed list is `[(v1, 8), (v2, 9)]` | `rules::consume_packet` return | both consumed at tick 8; reversed order |
| `u12b_value_not_consumed_before_acceptance` | v (e 8, accepted_at 10); samples at ticks 8, 9, 10 | consumed list is `[(v, 10)]` | return | accepted_at check dropped (consumed at 8) |
| `u12c_unconsumed_left_entry_holds_later_debt` | left [x (e 100)], debt [v (e 20)]; samples at ticks 20..23 | nothing consumed | return | consumption that does not follow the device's head-of-line FIFO (G6). This test pins the production rule; it is the rule the device model mirrors |
| `u12d_scratch_adapter_matches_rxchain_consume` | 200 cases from a fixed LCG seed (entries, samples, channels) | `consume_packet` output equals the output of `RxChain::consume` on the same state | both | the scratch-chain adapter drops or reorders an entry. This is an adapter check, not evidence for the rule: both sides call `RxChain::consume`; the rule's evidence is the case runs (Sol 8) |

### U13 — F set

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u13a_loop_check_stops_before_call` | F set; R at the loop check with a FIFO of two commands | (a) no device call (call list unchanged); (b) R exits and the FIFO is discarded | `device.calls`, `rx[k].pc` | check missing or after the call (I1) |
| `u13b_panic_guard_sets_flag_before_mark_lost` | `step(Panic)` | the guard's event log is `[Gate, FlagSet, Ungate, MarkLost]` (`Loss.gate`, `Loss.set`, `Loss.ungate`, `Loss.mark`; the guard takes `G` and no `L`) | guard log | `mark_lost` before the flag (a native destructor would see no loss) |
| `u13c_settings_discard_writes_abandoned_per_command` | F set; S holds one settings command in hand and two more in its FIFO | (a) each of the three has terminal `Abandoned`, written by S (one cell section per command); (b) no device call; S exits | the terminal records, `device.calls` | a discarded command with no terminal, which P2 counts as a failure after a panic when no brain can write it (E1-66) |

### U14 — in-flight bound

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u14_in_flight_bound_holds_value` | Four Queued values in the cell (bound 4); `Release` a fifth | (a) fifth is Held; (b) no `Record::Ceiling` in outbox | `brain.held`, `outbox` | bound not checked (I11); a new visible Ceiling record |

The bound value 4 is proposed (G14).

### U15 — device model (mirrors FakeDevice)

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u15a_full_queue_blocks_then_accepts` | queue cap 1, one command due 40; `apply_timed` (e 8) | (a) first call `Blocked`; (b) after `settle` at T 40, `Accepted{effective: 40}` | return values | queue over capacity; effective not `max(at, last_command, now)` (`device.rs:1433`) |
| `u15b_dsp_fifo_full_refuses_rx_write` | RX chain with 32 entries, then one more freq write; with 31 entries, one more | (a) 33rd is `Refused` (ACK timeout, `device.rs:1437-1441`); (b) 32nd is `Accepted` | return values | off-by-one in the capacity check |
| `u15c_stop_with_tail_keeps_end` | streamer next < now, `stop` at now | `end == Some(now)` | `device.rxs[k].end` | tail dropped (`rx_stop`, `device.rs:1591-1595`) |
| `u15d_start_behind_stopped_tail_sets_then` | streamer with `end = Some(40)` and `next` at the tail's cursor; `start(at 50)` (FakeDevice `start_rx`, `device.rs:1196–1199`) | (a) `then == Some(50)`; (b) `end == Some(40)` and `next` unchanged (the tail is kept) | streamer fields | start that replaces the tail, or moves `next` to 50, before the tail is delivered (E1-27) |
| `u15e_drain_is_head_of_line` | DSP FIFO [(40, f1), (20, f2)] in booking order, flow from 0; samples 20..41 | f2 not taken before f1; f1 at tick 40, f2 at tick 41 | `dsp.log` | any entry taken (drain not head-of-line, `device.rs:1047-1080`) |
| `u15f_recv_delivers_buffered_samples_in_order` | streamer buffer [8, 9, 10, 11] (sample ticks); `R.recv_begin` with length 2; `R.recv_end` twice | the first end returns [8, 9], the second [10, 11] | return values | samples delivered out of order (E1-43; G8, per-sample buffer) |

---

### U16 — LateCommand (draft 2, G15)

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u16a_cut_free_late_command_restarts` | cell `Armed(s)`, no cut in force, stream 0; `LateCommand` input | outbox holds one `Rx{Start}` for stream 0 | outbox | cut-free restart dropped (the L1 path restarts a segment with no cut) |
| `u16b_late_command_with_cut_ends_no_start` | cell `Streaming(s)`, cut 12 in force; `LateCommand` input | (a) no `Rx{Start}` in outbox; (b) outbox holds `Rx{Stop{at: T}}` (end-now) | outbox | a LateCommand with a cut restarts a segment that is ending (note 44 §3.8) |
| `u16c_lost_no_call` | cell device `Lost`, cut-free; `LateCommand` input | outbox holds no `Rx` command | outbox | restart after `Lost` (L1 M1; note 44 §3.3: `begin` only from `Off` or `Uncertain`) |

### U17 — failed device transitions (draft 2, G15)

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u17a_failed_begin_from_uncertain_stays_uncertain` | phase `Uncertain`; begin requested; `R.result` with a failed `rx_start` | (a) phase is `Uncertain`; (b) no `Settings(Restore)` in outbox | phase, outbox | a failed start clears `Uncertain` (note 44 §3.3: a failed begin keeps the previous state) |
| `u17b_failed_stop_is_uncertain` | phase `Streaming(s)`; brain-commanded stop; `R.result` with a failed `rx_stop` | phase is `Uncertain` | phase | a failed stop read as success (`Off`), so a restore could run on a live stream |
| `u17c_flush_start_failure_stays_off` | phase `Off`; flush requested; `R.result` with a failed `rx_start_now` | (a) phase is `Off`; (b) no `Settings(Restore)` in outbox | phase, outbox | a failed flush start moved to `Uncertain` or `Streaming` (note 44 §3.3: stays `Off`) |
| `u17d_flush_final_stop_failure_uncertain_no_restore` | phase `Off`; flush started (phase `Flushing`), reads done; `R.result` with a failed final `rx_stop`; then `Request(Restore)` | (a) phase is `Uncertain` after the failed final stop; (b) no `Settings(Restore)` after the request | phase, outbox | the D9b-2 path: the flush rewrites the stop result (`RxDsp.stopped`, note 44 §2.3), or a restore is issued from `Uncertain` (I13); phase written as `Off` (E1-36, E1-37) |

### U18 — held values at a segment end and at shutdown (draft 2, G4)

| Test | Setup and operation | Asserts | Observed by | Wrong behaviour caught |
|---|---|---|---|---|
| `u18a_held_carried_when_later_segment_covers` | held v (ch 0, e 12); segment 1 (origin 0, cut 8) finishes with v unreserved; segment 2 (origin 8) starts | (a) v has no disposition at segment 1's finish; (b) segment 2's start reserves v (a `Settings` command with v's id) | `brain.dispositions`, outbox | a held value disposed at a segment end although a later segment covers its instant |
| `u18b_reported_once_when_no_segment_covers` | held v (ch 0, e 12); segment 1 (origin 0, cut 8) finishes; no later segment | `brain.dispositions` holds exactly one `Reported(NoReservation)` for v | `brain.dispositions` | value never disposed (P2), disposed twice, or carried without the coverage check (E1-38) |
| `u18c_shutdown_settles_held_by_settle_rule` | held v, due and with no channel left; `Shutdown` | (a) v has one disposition, with the outcome the `settle` rule gives (`no channel`, control.rs:913–938); (b) a second `Shutdown` adds nothing | `brain.dispositions` | a held value left undisposed at shutdown, or disposed by another rule (E1-39) |

---

## C. Harness tests

Harness = `dst.rs`, `oracle.rs` exploration machinery. Toy models live in `toy.rs`. Each test: name; assertion; wrong behaviour caught.

### Search state, monitor and scheduler (draft 3, Sol 5 and 10)

- The visited-set key is the exact serialized state (no hash): the production state (brain, cells, executors, `L`, `G`, `F`, inboxes, ops in progress, device `T`, queue, DSP FIFOs and logs, streamer state and buffers, recv in progress), the pending deadlines, and the monitor facts that decide a later verdict. The monitor keeps finite summaries, not histories: per value its disposition, its `applied` and `evidence` identities (value, channel, segment, epoch, sample; §A Monitor); the last published progress per epoch; per command its terminal (a call result, `Dropped` or `Abandoned`); the drain-ended flag per chain (I16); the observed deadlines; the witness set. Resolved in draft 3a (45-experiment §10).
- Excluded from the key: the diagnostic counters (`recomputes`, `discarded`) and the trace.
- The scheduler contract (§3.3) is the only physical-time assumption. A case's `Tick` window is part of its legal schedule.
- The run continues past the horizon to horizon + epilogue (§D). A timer is enabled only at its deadline (`harness_timer_never_early`); a deadline beyond the last input is reached in the epilogue (`harness_deadline_beyond_last_input_is_observed`); an actor still waiting at the end is `Incomplete` (`harness_recv_timeout_loop_at_end_is_incomplete`). The horizon-end rule of draft 3 is withdrawn.

| Test | Assertion | Wrong behaviour caught |
|---|---|---|
| `harness_dfs_expands_each_edge_once` | on a toy with reconverging paths (n distinct reachable states, m edges), each distinct state is expanded once and each of its outgoing edges is traversed exactly once: the expansion count is n and the edge count is m | a state re-expanded on each incoming path, or an outgoing edge skipped when a state is reached again (E1-30) |
| `harness_reconvergence_is_not_a_cycle` | toy diamond (two paths meet) returns `Pass` | "visited" treated as "on the stack": false cycle |
| `harness_key_separates_each_behaviour_field` | for each field listed in §A and in the search state above (Brain: held, dispositions, ops; Cell: phase, in-flight states, revision, quiesce; Device: queue, DSP FIFO and log, streamer buffer, next/end/then, lost; World: native, gate, failure, executor FIFOs, loop position, recv in progress, in_hand, env fired flags, handle state, monitor summaries (each field of each `Ident`), deadlines), a single-field edit changes the key. `World` is destructured without `..`, so a new field fails to compile until the key decides it | a behaviour field omitted from the key merges distinct states (E1-29 generalised) |
| `harness_key_ignores_trace` | two worlds equal except `trace` have equal keys | trace in key (no merging; state explosion) |
| `harness_key_ignores_diagnostic_counters` | two worlds equal except `recomputes` or `discarded` have equal keys | a counter in the key defeats reconvergence |
| `harness_cycle_is_reported` | toy with an event that leaves its state unchanged: `explore` returns `Cycle` with a trace | livelock reported as pass (P5) |
| `harness_diagnostic_counter_cycle_is_reported` | toy whose only change per event is a diagnostic counter: the key repeats and `explore` returns `Cycle` | a loop hidden because a counter made each state look new, or counter-only loops reported as progress (toy-F) |
| `harness_budget_overrun_is_failure` | toy of 10^4 states, budget 100 → `BudgetExceeded` whose report names the configuration, states, transitions, time, memory, reached and missing witnesses and the frontier; the case wrapper `assert_complete` panics on it | truncated search reported as pass |
| `harness_replay_is_deterministic` | the toy-A failing trace, replayed twice: same violation text and same final key | iteration-order dependence (a `HashMap` in state); hidden global state |
| `harness_walk_same_seed_same_trace` | `walk(seed 7)` twice: identical traces; across seeds 1..10 at least two traces differ | RNG shared with model state; degenerate walk |
| `harness_shrink_reaches_one_minimal_failure` | toy-A with 20 padding events, shrunk: length 4 (A.read, B.read, A.write, B.write) and still fails | shrinker returns a passing trace, or leaves events that are not needed |
| `harness_per_actor_fifo_order` | executor FIFO [c1, c2]: enabled set has Take(c1), not Take(c2) | an actor takes its commands out of order |
| `harness_no_effect_before_take` | in every visited state of case 1 (reduced), no `S.effect` is enabled while `in_hand` is None | device call without acceptance (causality, §3.2) |
| `harness_every_event_changes_state` | in every visited state of case 1 (reduced), every enabled event changes the key | state-preserving event (busy loop): false P5 cycles, or a missed livelock |
| `harness_time_only_advances_on_tick` | in every visited state, `device.t` changes only by `Tick`, and each `Tick` by exactly one sample | a non-tick event moves device time, or a tick advances several samples |
| `harness_tick_cannot_pass_mandatory_input` | in every visited state with an undelivered environment input whose latest ≤ `T`, `Tick` is not enabled | time passes an input window: the input's latest is missed (Sol 5; E1-53) |
| `harness_clock_read_holds_native_lock` | in every visited state of case 1 (reduced), `Clock.lock` is enabled only while `native` is `None`, `Clock.read` only while `native` is `Some(Clock)`, and `native` is `None` again after `Clock.unlock` | a clock read outside `L`, during a settings or RX call (note 45 §4.7; E1-41) |
| `harness_take_cancelled_sends_done_dropped` | in a run where a cancelled value reaches `S.take`, the brain inbox receives `Done{Dropped}` produced by the `S.take` event itself (the test does not supply it) | a silent drop: the producer does not send the completion (E1-42; Sol 8) |
| `harness_result_precedes_send` | in every visited state, each `Done` in the brain inbox has its command result already written in the cell (`S.result` before `S.send`) | a completion delivered before its cell write (E1-54) |
| `harness_vacuous_case_fails_on_missing_witness` | a toy case that declares a witness no path reaches fails with `Vacuous`, naming the witness | a case passing without exercising its declared path (§3.3) |
| `harness_broken_toy_is_caught` | toy-A (non-atomic read/write counter, final must be 2) → violation; toy-B (two actors, opposite lock order) → P4 deadlock | the harness passes a broken model: no power |
| `harness_first_tick_produces_sample_0` | fresh world (`T = 0`, one streamer running from 0): one `Tick` produces sample 0 into the buffer and leaves `T = 1` | origin sample 0 omitted or produced twice at the tick boundary (E1-62) |
| `harness_timer_never_early` | in every visited state of case 5 (reduced), a `Timer(d)` is enabled only when `T ≥ d` | a deadline delivered before its time, which would make P1 untestable (E1-64) |
| `harness_epilogue_covers_deadlines` | the case builder for each case, reduced and exhaustive, checks the epilogue: 32 ticks in the cases (cut 16 + drain 8 + recv timeout 8), 96 in the seeded configuration (cut 80 + 8 + 8); it fails if the epilogue is shorter than the latest drain deadline plus one recv timeout | a case whose deadline can fall after the run has ended, so P1 is never observed |
| `harness_deadline_beyond_last_input_is_observed` | a case on the core with a drain deadline 24 and the last input at 8, epilogue 32: the Timer is delivered at `T = 24`, and the verdict is not `Incomplete` (Sol R1a) | a deadline invented early at the horizon (R1), or one never reached |
| `harness_recv_timeout_loop_at_end_is_incomplete` | R in `recv` with timeout 8 and no data ever arriving: each timeout re-enters `recv` with a later deadline; at the end of the epilogue R is still in `recv` (Sol R1b) | the verdict is `Incomplete` naming R; `Pass` (E1-65) and `Cycle` are both wrong, because time advances at each timeout and no zero-time loop exists |
| `harness_loss_publish_releases_l_first` | in every visited state of case 2 (reduced), `Loss.gate` is enabled only when the publishing actor does not hold `L` (another actor may hold it), and `Loss.mark` only after the `Loss.ungate` of the same publication; the run reaches a state where `Loss.gate` is enabled while another actor holds `L` (the legal interleaving, a witness so the test is not vacuous) | a loss published while the publisher still holds `L`, or `mark_lost` before the flag is released (R5; E1-56) |
| `harness_lifecycle_inline_loss_publish_no_recursion` | case 8 (reduced): when a lifecycle caller holding `G` sees the loss, it sets `F` inside its `G`; no `Loss.gate` is enabled for a caller that already holds `G`; no state is a `Deadlock` | recursive acquisition of `G` on the inline path (E1-57) |
| `harness_violation_into_visited_state_is_caught` | toy with two paths into one state X, where the transition on the second path violates an invariant of its resulting state: the verdict is `Violation` | the invariant checked only on newly inserted states, so a violating edge into a visited state is missed (E1-61) |
| `harness_failed_reservation_leaves_no_send` | `step(Release(v))` in a cell where the reservation is refused (e.g. `Stopping`): the executed ops contain no `Send(S, Settings)` for v | the step's sends are queued before the section result is fed back (conditional `B.op`; E1-63) |

Toys: toy-A lost update (two actors, read then write, 4 events); toy-B lock-order deadlock; toy-C self-loop; toy-D diamond; toy-E small acyclic graph for coverage; toy-F a counter that changes on every event while the protocol state repeats (for `harness_diagnostic_counter_cycle_is_reported`). Each toy has its own tests; none uses the core.

---

## D. Cases 1–9 and the seeded configuration

### Common parameters and witnesses (draft 4)

- P = 4 samples per packet (UHD's packet size); one tick per sample; origin 0; horizon as stated.
- Recv timeout 8 ticks; in-flight bound 4 per chain; drain deadline = cut + 8. Epilogue (45-experiment §3.2, draft 4): the run ends at horizon + epilogue, which is 32 ticks in the cases (it covers the largest cut, 16, its drain deadline and one recv timeout) and 96 in the seeded configuration (largest cut 80); `harness_epilogue_covers_deadlines` checks it. A timer fires only at its deadline; nothing is observed early.
- Radio queue capacity 1 (cases 1–9), 2 (seeded).
- Environment windows are closed intervals `[a, b]` in environment time (the `[0, 8)` of draft 2 is read as [0, 7]). The brain's processing time is not bounded. `Tick` is disabled once an undelivered input's latest has passed (§3.2).
- Shutdown is an Env input enabled after every other Env input has been delivered and `T ≥ horizon − 8`.
- Every Env input is explored at every position its window allows (DFS). For the seeded configuration, a seed picks each position.
- Safety oracles I1–I9 and I11–I14 apply to every state (I10 is not an oracle, G13). Progress oracles P2–P5 apply to terminal states; P1 applies where a drain deadline is observed.
- **Witnesses.** Each case declares witnesses: monitor facts that must occur on some explored path (for example "R.recv_begin under epoch 1, R.recv_end after the advance"). The monitor records a witness when its event happens. The case's run returns the union of witnesses over all explored paths, and the test asserts that every declared witness is in the union; otherwise it fails with `Vacuous` (`harness_vacuous_case_fails_on_missing_witness`). The seeded configuration's witnesses are the union over its seeds.
- **Sizes and budget.** The first E1b task is measurement: each case's reduced variant at horizons 8, 16 and 24, with states and time recorded, then extrapolation (§3.3). The estimates below are hand estimates, redone for per-sample ticks (INFERRED). A full run over 10 minutes on one sim02 core, or with an extrapolation above 5 × 10^6 states, is stopped and reported with its counts. No partial-order reduction, and no weakening of a case to fit (Sol 11). On an overrun the report names the configuration, states, transitions, elapsed time, memory, reached and missing witnesses and the unexplored frontier (case 6: the affected k); a counterexample is kept; a partial exploration is never reported as passing (45-experiment §3.3).

### Case 1 — value, cut, Stop shortens the cut

- Script: stream 0 `Streaming(seg origin 0, ch {0})`, gen 1, epoch 1, plan_cut 16, progress 0. Stream 0 open and started at 0.
- Env: `Release(v: ch0, e 8, id 1)` window [0, 7]; `Stop(requested 10)` window [0, 16]; `Shutdown`.
- Horizon 48. Applies: I1–I9, I11, I14; P1–P5.
- Expected: cut = min(16, max(10, progress at booking)). v has exactly one disposition (P2), agreeing with the monitor (I4).
- Witnesses: W1a v consumed by a published sample; W1b v cancelled before take; W1c v taken, the cut shortened to 10, and consumed by a sample at or after the cut (unpublished drain evidence).
- Test names: `case1_value_cut_stop_exhaustive` (ignored), `case1_value_cut_stop_reduced`.
- Reduced: horizon 16; release window [0, 3]; Stop window [0, 8]. W1c needs sample 10, which the reduced horizon contains.
- Estimate: 10^4 to 10^6 (INFERRED).

### Case 2 — value, loss at every point

- Script: as case 1 without Stop. Env: `Release(v)` [0, 7]; `Loss` [0, 48]; `Shutdown`.
- Horizon 48. Applies: I1–I9, I11, I14; P2–P5.
- Expected: I1, I9, I14; v disposed once (reported, on loss); the brain ends the segment without device calls after it learns of the loss (`Done{Call(Lost)}`); R and S exit; handles retained.
- Witnesses: W2a loss before `S.lock`; W2b loss between `S.effect` and `S.result`; W2c loss while R is in `R.recv_end` waiting.
- Test names: `case2_loss_everywhere_exhaustive` (ignored), `case2_loss_everywhere_reduced`.
- Reduced: horizon 16; release [0, 3]; loss [0, 16].
- Estimate: 10^5 to 10^6 (INFERRED; loss is absorbing).

### Case 3 — plan cancel with a recv and a value in flight, Stop (rewritten, draft 3)

- Script: stream 0 `Streaming` epoch 1, cut 16, gen 1; value v (ch0, e 8, id 1).
- Env: `Release(v)` [0, 7]; `PlanCancel(origin 20, epoch 2, plan_cut 48)` [0, 16]; `Stop(requested 10)` [0, 16]; `Shutdown`.
- Horizon 48. Applies: I1–I9, I11, I12, I14; P1–P5.
- Expected: I6 and I12. A recv begun under epoch 1 that returns after the advance publishes only up to epoch 1's latest cut (10 if Stop came first, else 16), and the rest is counted as discarded. No epoch-2 start, close or open before the quiesce ack and before v's pin clears. The quiesce ack follows the publication.
- Witnesses: W3a `R.recv_begin` under epoch 1 before the advance, and `R.recv_end` after it; W3b Stop shortens epoch 1's cut after `R.snap` (a publication refused or recomputed); W3c v pinned (taken or maybe-applied) at the advance.
- Test names: `case3_plan_cancel_recv_stop_exhaustive` (ignored), `case3_plan_cancel_recv_stop_reduced`.
- Reduced: horizon 16; PlanCancel origin 12, window [0, 11]; Stop [0, 9]; Release [0, 3]. The reduced variant must still show W3a; if it cannot, the reduced variant is not evidence for case 3 and is reported.
- Estimate: 10^6 to 10^7 (INFERRED: per-sample ticks, a recv in flight, two epochs, a value). At horizon 48 this is likely over the 10-minute budget; the measurement decides.

### Case 4 — two equal-time values and the Stopping transition

- Script: stream 0 `Streaming` plan_cut 12, gen 1. Values v1 (e 8, id 1) and v2 (e 8, id 2), booked in that order.
- Env: `Release(v1)` [0, 7]; `Release(v2)` [0, 7]; `Shutdown`.
- Horizon 48. Applies: I1–I9, I11; P1–P5.
- Expected: I7, I8. v1 and v2 are taken by distinct samples (v1 first, ticks 8 and 9). A value that cannot be consumed before the cut is drained by unpublished evidence or reported, once.
- Witnesses: W4a a reservation attempted after Stopping (refused, I7 checked); W4b both values consumed, by distinct samples.
- Test names: `case4_equal_time_pair_stopping_exhaustive` (ignored), `case4_equal_time_pair_stopping_reduced`.
- Reduced: horizon 16; release window [0, 3].
- Estimate: 10^4 to 10^6 (INFERRED).

### Case 5 — settings call blocked past the drain deadline, with conflicting requests (rewritten, draft 3)

- Script: stream 0 `Streaming` plan_cut 12. Radio queue (capacity 1) occupied by an earlier settings command (a gain write, no DSP effect) due at 40. Value v (ch0, e 8) reserved, so S blocks in `S.effect` holding `L`. Drain deadline 20.
- Env: `Release(v)` [0, 7]; `Request(Restore)` [12, 40]; `Request(StartNext, origin 44)` [12, 40]; `Shutdown`.
- Horizon 56 (it covers the sample after v's late application, for W5d). Applies: I1–I9, I11, I15, I16; P1–P5.
- Expected: `R.call` for the stop waits for `L` (note 45 §4.7). The drain deadline is observed (Timer delivered) while S holds `L`. R then stops without waiting for v (P1). v stays pinned, is not cancelled, and gets one disposition after its call returns and the pin clears. The restore and the next segment are deferred while v is pinned (I3, U9) and each is issued once after; the next segment is not held back by v's `left` (u09g). The command result is recorded once. The drain-ended transition is made in the step that processes the deadline, while v is still pinned (I16, R3). A restore is issued only when no chain holds debt or `left` on the channel (I15, R4); the next segment is not held back by them, so its first sample (44) consumes v as leftover, and the restore follows that consumption. The run terminates (P3).
- Witnesses: W5a the deadline observed while S holds `L` (`native == Settings`); W5b a deferred request (restore or next segment) issued after the pin cleared; W5c the drain-ended transition made while v is pinned (I16); W5d the first sample after v's late application, the next segment's first sample (44), consumes v as leftover before any restore of ch0 (I15).
- Test names: `case5_blocked_settings_drain_deadline_exhaustive` (ignored), `case5_blocked_settings_drain_deadline_reduced`.
- Reduced: release window [0, 0]; Restore and StartNext windows [12, 16]; horizon 56.
- Estimate: 10^5 (INFERRED; blocking removes most interleavings).

### Case 6 — brain panic at every step, executor loops

- Script: case 1's script plus `Panic` at brain step k, one run per k.
- Applies: I1–I9, I11, I14; P2–P5 (P1 as in case 1).
- Expected: the guard publishes `F` and calls `mark_lost` (`Loss.gate`, `Loss.set`, `Loss.ungate`, `Loss.mark`) before any further accepted device call; R and S exit at their next check; no handle is closed or opened after `F` (I9, I14); all actors exit. A settings command discarded by an executor gets `Abandoned` from that executor, not from the dead brain (R2).
- Witnesses: W6a a panic while S holds a taken value; W6b a panic while R is in `R.recv_end` waiting.
- Test names: `case6_brain_panic_every_step_exhaustive` (ignored; runs k over all brain steps of case 1), `case6_brain_panic_first_and_stop_reduced` (default; k = first step and the step that processes Stop).
- Reduced: horizon 16.
- Estimate: 10^6 to 10^7 per k, and 10 to 15 values of k (INFERRED). The full sweep is likely over budget; a k over budget is reported, not dropped.

### Case 7 — paths behind the past bugs (rewritten, draft 3)

Three sub-cases, each with an exhaustive (`#[ignore]`) and a reduced variant. Note 44 §3.3–3.4 decides which device-state transitions exist. Where a script depends on a reading of it, the sub-case says so (see case 7c, resolved in draft 3a).

#### Case 7a — handoff loss, then a cut-free LateCommand (L1 M1)

- Script: stream 0 `Streaming(seg origin 0, ch {0})`, gen 1, epoch 1, progress 0, with no plan cut (an open-ended segment, the cut-free shape of L1 M1). Stream 0 open and started at 0.
- Env: `Release(v: ch0, e 8, id 1)` [0, 7]; `LateCommand(stream 0)` [0, 40]; `Shutdown`. Branch: the `S.effect` for v returns `lost`.
- Horizon 48. Applies: I1, I3, I9, I13; P2–P5.
- Expected: no `Rx{Start}` is issued after the brain has received the loss (`Done{Call(Lost)}`). A Start issued before that is stopped by R's check (I1) with no device call. v is disposed once (reported). Handles are retained (I9, P3). U16c tests the brain's decision in isolation.
- Witnesses: W7a a `LateCommand` delivered after `LossPublish`; W7b a `LateCommand` whose restart decision is taken after the brain has received `Done{Call(Lost)}`.
- Test names: `case7a_handoff_loss_late_restart_exhaustive` (ignored), `case7a_handoff_loss_late_restart_reduced`.
- Reduced: horizon 16; release [0, 3]; LateCommand [0, 15].
- Estimate: 10^3 to 10^4 (INFERRED).

#### Case 7b — a successful stop leaves outstanding debt; the flush's final stop fails; restore (D9b-2)

- Script: stream 0 `Streaming(seg origin 0, plan_cut 16)`, gen 1, epoch 1. Value v (ch0, e 4, id 1) is taken and its call finishes, so it is not pinned; no qualifying sample has consumed it when the stop comes.
- Env: `Release(v)` [0, 3]; `Stop(requested 6)` [0, 7]; `Request(Restore)` [0, 47]; `Shutdown`. The brain issues the flush itself when the stop leaves debt (note 44 §3.4). Branches: the stop's `rx_stop` succeeds (chain `Off`); the flush's final `rx_stop` fails (`Uncertain`).
- Horizon 48. Applies: I1, I3, I9, I13; P2–P5.
- Expected: the flush starts only from `Off`, with v as outstanding debt and no pin (E1-50 is the case that blocks it). When the final stop fails the chain is `Uncertain`; the restore request then yields no `Settings(Restore)` (I13, u17d). v is disposed once (P2). No call on a closed handle (I9).
- Witnesses: W7b1 the flush started with outstanding debt and no pin; W7b2 the flush's final stop failed (`R.result` gives `Uncertain`); W7b3 the restore requested after W7b2.
- Test names: `case7b_flush_final_stop_restore_exhaustive` (ignored), `case7b_flush_final_stop_restore_reduced`.
- Reduced: horizon 24; release [0, 1]; Stop [0, 4]; Restore [6, 23].
- Estimate: 10^4 to 10^5 (INFERRED).

#### Case 7c — a begin with outstanding debt (rewritten, draft 3)

- Script: stream 0 segment 1 (origin 0, plan_cut 8), gen 1, epoch 1, `Streaming`. Value v1 (ch0, e 4, id 1) is taken and its call finishes, leaving outstanding debt, not a pin. Segment 2 (origin 8, plan_cut 16) on the same stream.
- Env: `Release(v1)` [0, 3]; `Stop(requested 6)` [0, 7] (segment 1's cut becomes min(8, max(6, progress))); `Request(StartNext)` [0, 47]; `Shutdown`. Branch: segment 2's `rx_start` fails when issued.
- Horizon 24 (exhaustive); reduced: horizon 16; Release [0, 3]; Stop [0, 5]; StartNext [0, 7].
- Applies: I1, I3, I4, I9; P2–P5.
- Expected: I3 holds: the begin waits for pins, not for debt, so it may be issued while v1 is outstanding debt (resolved in draft 3a) and never while v1 is taken or maybe-applied. After a failed begin the chain stays as note 44 §3.3 says (`Off` stays `Off`; the segment is refused). No further streamer call is made for the refused segment after the failed begin; the failed start itself is the one call. v1 has one disposition (P2).
- Witnesses: W7c1 the begin issued while v1 is outstanding debt; W7c2 the begin's start fails.
- Test names: `case7c_failed_begin_debt_exhaustive` (ignored), `case7c_failed_begin_debt_reduced`.
- Estimate: 10^4 to 10^5 (INFERRED). Note 44 §3.4 says begin only with empty debt; draft 3a's decision (begin waits for pins only) differs. Resolved in draft 3a (45-experiment §10); note 44 §3.4 still needs its amendment (owner decision).

### Case 8 — lifecycle admission across loss (new, draft 3)

- Script: stream 0 segment 1 (origin 0, plan_cut 8), gen 1, no values. Radio queue 1.
- Env: `Stop(requested 4)` [0, 2] (finishes segment 1); `Request(StartNext, channel set change: close and open, origin 8)` [0, 8]; `Loss` [0, 12]; `Shutdown`.
- Horizon 32 (exhaustive); reduced: horizon 16; Stop [0, 2]; Request [0, 8]; Loss [0, 12].
- Applies: I1, I9, I14; P2–P5.
- Expected: I14: no open, close or rate read is admitted after `F` is published; I9: no close after `F`, handles retained. A lifecycle call admitted before `F` completes its single call.
- Witnesses: W8a R waits for `G` between `Loss.gate` and `Loss.ungate` (the loss publisher holds `G`); W8b R's recheck inside `G` refuses an open because `F` is set; W8c a lifecycle call admitted before `F` is set and completed; W8d a lifecycle caller that sees the loss while holding `G` sets `F` inside it (the inline path, `harness_lifecycle_inline_loss_publish_no_recursion`).
- Test names: `case8_lifecycle_across_loss_exhaustive` (ignored), `case8_lifecycle_across_loss_reduced`.
- Estimate: 10^4 to 10^5 (INFERRED).

### Case 9 — pending settings work meets a lifecycle request, partial and refused results (new, draft 3)

- Script: stream 0 `Streaming` plan_cut 16, gen 1. Value v (ch0, e 8, id 1) is taken, its settings call in progress, so v is pinned. Stream 1 open and idle. Radio queue 1.
- Env: `Release(v)` [0, 3]; `Stop(requested 6)` [0, 12] (cut 6 is below v's e 8); `Request(Move: ch0 from stream 0 to stream 1)` [0, 16]; `Request(Restore: ch0)` [0, 16] (added in draft 4 so that I15 is exercised; stage notes, item 9); `S.effect` for v branches over {ok, partial, refused}; `Shutdown`.
- Horizon 32 (exhaustive); reduced: horizon 24; Release [0, 1]; Stop [0, 4]; Move [0, 8]; Restore [0, 8].
- Applies: I1–I9, I11, I15, I16; P1–P5.
- Expected: the move is deferred while v is pinned (I3). A partial result keeps v as possibly on the device, carried as a leftover of the chain it was applied to (stream 0), not moved to the destination. A refusal disposes v once as `Refused`. The deferred move is issued once after the pin clears (U9). The command result is recorded once. No restore of ch0 is issued while v is debt or `left` in either chain (I15).
- Witnesses: W9a each outcome of {ok, partial, refused} occurs with the move pending; W9b the move issued after the pin cleared; W9c a restore of ch0 requested while v is `left` or debt, and not issued until no such entry remains.
- Test names: `case9_pending_move_partial_refused_exhaustive` (ignored), `case9_pending_move_partial_refused_reduced`.
- Estimate: 10^5 to 10^6 (INFERRED).

### Seeded configuration (rewritten, draft 3)

- Script: two streams (channels 0 and 1); four values (v0 ch0 e 8 id 1; v1 ch1 e 12 id 2; an equal-time pair v2, v3 on ch0 with e 20, ids 3 and 4, booked in that order). Env, each at a seed-chosen position in its window: `Stop(requested ∈ [4, 40])` on all chains; `PlanCancel(stream 1, origin 24, epoch 2, plan_cut 80)`; a channel move of ch0 from stream 0 to stream 1; `Request(Restore)`; `Loss` at a tick in [0, 95]; one `Release` per value; `S.effect` outcomes random over {ok, partial, refused} (lost only through `Loss`); `Shutdown`.
- Radio queue capacity 2; horizon 96; epilogue 96 (§D common parameters).
- Applies: every safety oracle and every progress oracle. The witness union (W9a, W9b and the others) is reported.
- Tests: `seeded_config_reduced` (default; horizon 32; 50 seeds; no loss; asserts no failure for every seed and the union of witnesses), `seeded_config_sweep` (ignored; 10,000 seeds or 30 minutes on sim02, whichever first; every failing seed recorded).
- Exhaustive search is not planned: the reachable product is estimated above 10^9 (INFERRED). Only seeds.

---

## E. Oracles (I1–I9, I11–I16, P1–P5; I10 is not an oracle)

Each oracle is a function in `oracle.rs` over the world and the monitor. Its test is a crafted record or world that violates it (the oracle names the rule), plus the mutation rows and case runs that exercise the real core. The three kinds of fact are kept apart (45-experiment §5): device application, qualifying evidence and publication. A value has one disposition, a command has one result, and a leftover consumption is a separate fact.

| Id | Evaluated at | Reads | Wrong core behaviour it must catch | Oracle test |
|---|---|---|---|---|
| I1 | each ordinary device call, each `F` set, each `Abandoned` write | the actor's last check result, whether a call came since, F | an ordinary call accepted after a check that saw F set; check moved after the call; `Abandoned` written while F is clear | `inv_i1_call_after_failure_seen_is_violation`; `inv_i1_abandoned_without_failure_is_violation` |
| I2 | each reservation | phase, channels, generation, origin, cut, progress, value | reservation at e ≥ cut, wrong channel or generation, or in Stopping | `inv_i2_reservation_outside_conditions_is_violation` |
| I3 | each brain command of restore, move, close, open, start-next | the in-flight list (a pin is a Taken or MaybeApplied value) | a command issued while pinned (M14); outstanding debt wrongly treated as a pin, or a pin missed | `inv_i3_restore_while_pinned_is_violation`; `inv_i3_outstanding_debt_is_not_pin` |
| I4 | each disposition, each leftover consumption, each terminal state | per value: the `applied` and `evidence` identities (value, channel, segment, epoch, sample) and the `leftovers` identity (§A Monitor); publication per epoch; dispositions | a value with two dispositions; consumed without `D` applying it, or without qualifying evidence; consumption whose evidence sample, channel, segment or epoch differs from the application's; reported though qualifying evidence existed when it was reported. Applied to a sample the host never received is legal and ends reported | `inv_i4_double_disposition_is_violation`; `inv_i4_unpublished_drain_evidence_discharges`; `inv_i4_unreceived_application_reported_is_legal` |
| I5 | each state; each publication | cut, progress, published ticks | cut below progress (M06); published sample at or after the cut of its epoch | `inv_i5_cut_below_progress_is_violation` |
| I6 | each publication of an older epoch's recv; each ack | retiring cut, published ticks, ack order | old buffer published past the retiring cut (M12); ack before publication (M13) | `inv_i6_old_epoch_past_retiring_cut_is_violation` |
| I7 | each reservation | phase before the event | reservation while Stopping or Off (M01) | `inv_i7_reservation_while_stopping_is_violation` |
| I8 | each take | per-tick take count per channel; booking order | two values taken by one sample; reversed order (M18) | `inv_i8_two_takes_one_sample_is_violation` |
| I9 | each device call; each close | handle state, F | call on a closed handle; handle closed after F | `inv_i9_close_after_failure_is_violation` |
| I10 | not an oracle (G13) | — | a cell procedure touching the device | compile-time; no test |
| I11 | each reservation | in-flight count before the event | in-flight above the bound (M22) | `inv_i11_in_flight_over_bound_is_violation` |
| I12 | each start, close or open of a new epoch's streamer or chain; each quiesce ack | the old epoch's ack state; the command's epoch | a new-epoch command before the old epoch's quiesce ack (G12) | `inv_i12_new_epoch_command_before_ack_is_violation` |
| I13 | each restore | the chain's device state (`Uncertain` or `Lost`) | a restore from an `Uncertain` or `Lost` chain (D9b-2) | `inv_i13_restore_from_uncertain_is_violation` |
| I14 | each open, close and rate read, at its admission | `F` at the admission, under `G` | a lifecycle admission after `F` is published (the window the gate closes; Sol 2) | `inv_i14_admission_after_loss_is_violation` |
| I15 | each restore; each change of a channel's debt or `left` (including a move) | the channel's debt and `left` on every chain; pins | a restore while any chain holds debt or `left` on the channel; a restore that checks pins only (R4) | `inv_i15_restore_with_left_on_channel_is_violation`; `inv_i15_restore_with_debt_on_other_stream_is_violation` |
| I16 | each step that processes a drain deadline input (checked at the step, not at terminal states) | the step's ops; the pin of the chain | the drain-ended transition deferred until the pin clears (R3; P1's safety half) | `inv_i16_drain_ended_waits_for_pin_is_violation` |
| P1 | at each observed drain deadline, and at terminal states | the deadline observation; R's stop status; the holder of `L` at the observation | R keeps waiting for pinned values after the observation (drain never ends) | `prog_p1_observed_deadline_stops_r`; `prog_p1_unobserved_deadline_makes_no_claim` |
| P2 | terminal states | every released value's disposition; every settings command's terminal (a call result, `Dropped` or `Abandoned`) | a value without one disposition, or with two; a command without one terminal; a held value left at shutdown (G4) | `prog_p2_undisposed_value_is_violation`; `prog_p2_command_without_terminal_is_violation`; `prog_p2_shutdown_settles_held_by_settle_rule` |
| P3 | terminal states | actor FIFOs, exit flags, handle states, F | an actor stuck with work; handle closed after F | `prog_p3_stuck_actor_is_violation` |
| P4 | terminal states | actors with pending work and no enabled event | deadlock: an actor waits for an event that is never enabled | `prog_p4_pending_work_without_event_is_violation` |
| P5 | during DFS | the on-stack set | livelock, including a loop that changes only counters | `harness_cycle_is_reported`, `harness_diagnostic_counter_cycle_is_reported` (section C) |

Note on P1 (draft 3): the oracle checks protocol progress after an observation. It says nothing about the time between the deadline and its observation, which the scheduler contract leaves free.

---

## F. Mutation rows (to be added with the code)

Rows go in `plan/maintenance/tools/mutations.json` with `crate: "ezsdr-radio-uhd"`, `test_bin: "lib"`, and `filter` set to the killing test's full name (`mutate.py:96-97` passes it as a cargo test filter). IDs `E1-01` … `E1-67` (G19, accepted in draft 2; E1-33 to E1-45 are the draft 2 rows; E1-46 to E1-55 the draft 3 rows; E1-56 to E1-67 the draft 4 rows). Rust rows use token anchors (AGENTS.md §7). The exact `old` text is fixed with the code; each `old` must match exactly once through `anchor.py`. Rows run only through `remote_mutate.py` on sim02/sim03; they run with the E1b gate, not on their own (45-experiment stage table).

| Row | File | Mutation (intended) | Killing test |
|---|---|---|---|
| E1-01 | `brain/cell.rs` `reserve` | `Stopping` accepted in the phase check | `u02b_stopping_refuses` |
| E1-02 | `brain/cell.rs` `reserve` | `e < plan_cut` becomes `e <= plan_cut` | `u02f_instant_at_cut_refuses` |
| E1-03 | `brain/cell.rs` `reserve` | progress check removed | `u02g_progress_at_cut_refuses` |
| E1-04 | `brain/cell.rs` `reserve` | `e >= origin` becomes `e > origin` | `u01c_instant_equal_origin_reserves` |
| E1-05 | `brain/cell.rs` `reserve` | channel check removed | `u02c_other_channel_refuses` |
| E1-06 | `brain/cell.rs` `reserve` | generation check inverted | `u02d_stale_generation_refuses` |
| E1-07 | `brain/cell.rs` `reserve` | `Armed` refused | `u01b_armed_value_reserves` |
| E1-08 | `brain/cell.rs` `book_stop` | cut = requested (no max with progress) | `u06a_stop_below_progress_clamps_to_progress` |
| E1-09 | `brain/cell.rs` `book_stop` | no `min` with plan_cut | `u06b_stop_beyond_plan_cut_keeps_plan_cut` |
| E1-10 | `brain/cell.rs` `book_stop` | Queued values with e ≥ cut not cancelled | `u03_cancel_before_take_drops_without_call` |
| E1-11 | `brain/cell.rs` `stop_accepting` | Queued values not cancelled | `u05a_stopping_cancels_queued_and_lists_obligations` |
| E1-12 | `brain/cell.rs` `take` | a Cancelled value taken (no drop) | `u03_cancel_before_take_drops_without_call` |
| E1-13 | `brain/cell.rs` `publish` | stale check removed | `u07a_stale_snapshot_refused_and_counted` |
| E1-14 | `brain/cell.rs` `advance_epoch` | retiring cut = plan cut | `u08a_old_buffer_published_to_retiring_cut` |
| E1-15 | `brain/cell.rs` `ack_quiesce` | ack accepted before publish | `u08c_quiesce_ack_after_publish` |
| E1-16 | `brain/core.rs` `step` | restore not deferred while pinned | `u09a_restore_deferred_while_pinned` |
| E1-17 | `brain/core.rs` `step` | `Partial` counted as `Failed` | `u10_partial_result_is_maybe_applied_not_failure` |
| E1-18 | `brain/core.rs` `step` | flag set after `mark_lost` in the guard | `u13b_panic_guard_sets_flag_before_mark_lost` |
| E1-19 | `brain/core.rs` `step` | in-flight bound check removed | `u14_in_flight_bound_holds_value` |
| E1-20 | `brain/rules.rs` `consume_packet` | `RxChain::consume` call replaced by first-entry-only consumption | `u12a_equal_time_values_take_distinct_samples` |
| E1-21 | `brain/rules.rs` `consume_packet` | scratch chain drops the left list | `u12d_scratch_adapter_matches_rxchain_consume` |
| E1-22 | `brain/dst.rs` R loop | R's check before its call removed | `case2_loss_everywhere_reduced` |
| E1-23 | `brain/dst.rs` S loop | S's check before its call removed | `case2_loss_everywhere_reduced` |
| E1-24 | `brain/model.rs` `apply_timed` | queue-full comparison `<` becomes `<=` | `u15a_full_queue_blocks_then_accepts` |
| E1-25 | `brain/model.rs` `apply_timed` | RX DSP FIFO capacity off by one | `u15b_dsp_fifo_full_refuses_rx_write` |
| E1-26 | `brain/model.rs` `stop` | tail `end` not kept | `u15c_stop_with_tail_keeps_end` |
| E1-27 | `brain/model.rs` `start` | start behind a tail not deferred (`then` unset) | `u15d_start_behind_stopped_tail_sets_then` |
| E1-28 | `brain/model.rs` `drain` | head-of-line removed (any entry taken) | `u15e_drain_is_head_of_line` |
| E1-29 | `brain/dst.rs` `key` | the executor FIFO left out of the key | `harness_key_separates_each_behaviour_field` |
| E1-30 | `brain/dst.rs` `explore` | a visited state re-expanded, or an outgoing edge of a reconverging state skipped | `harness_dfs_expands_each_edge_once` |
| E1-31 | `brain/dst.rs` `explore` | an on-stack revisit treated as "ok" | `harness_cycle_is_reported` |
| E1-32 | `brain/dst.rs` `explore` | budget overrun returns `Pass` | `harness_budget_overrun_is_failure` |
| E1-33 | `brain/core.rs` `step` | a LateCommand after `Lost` issues a Start | `u16c_lost_no_call` |
| E1-34 | `brain/core.rs` `step` | a LateCommand with a cut restarts instead of ending | `u16b_late_command_with_cut_ends_no_start` |
| E1-35 | `brain/dst.rs` `R.result` | a failed `rx_stop` leaves the phase unchanged (not `Uncertain`) | `u17b_failed_stop_is_uncertain` |
| E1-36 | `brain/core.rs` `step` | a restore is issued from `Uncertain` (I13) | `u17d_flush_final_stop_failure_uncertain_no_restore` |
| E1-37 | `brain/dst.rs` `R.result` | a failed flush final stop writes `Off` (not `Uncertain`) | `u17d_flush_final_stop_failure_uncertain_no_restore` |
| E1-38 | `brain/core.rs` `finish` | a held value is carried without the later-segment coverage check | `u18b_reported_once_when_no_segment_covers` |
| E1-39 | `brain/core.rs` shutdown | held values are settled by a rule other than `settle` (control.rs:913–938) | `u18c_shutdown_settles_held_by_settle_rule` |
| E1-40 | `brain/core.rs` `step` | a brain-commanded stop does not close acceptance before its call | `u05c_brain_stop_closes_acceptance_before_call` |
| E1-41 | `brain/dst.rs` `Clock.lock` | `Clock.lock` is enabled while `L` is held | `harness_clock_read_holds_native_lock` |
| E1-42 | `brain/dst.rs` `S.take` | a cancelled value is dropped without its `Done{Dropped}` | `harness_take_cancelled_sends_done_dropped` |
| E1-43 | `brain/model.rs` `recv` | buffered packets are delivered in reverse order | `u15f_recv_delivers_buffered_samples_in_order` |
| E1-44 | `brain/dst.rs` R loop | a new-epoch start is issued before the old quiesce ack | `case3_plan_cancel_recv_stop_reduced` |
| E1-45 | `brain/core.rs` `step` | a begin is issued while a value of the chain is pinned | `case5_blocked_settings_drain_deadline_reduced` |
| E1-46 | `brain/cell.rs` `publish` | the obligation revision check removed | `u07b_result_between_snapshot_and_publish_refused` |
| E1-47 | `brain/dst.rs` `R.gate` | the recheck of `F` inside `G` removed | `case8_lifecycle_across_loss_reduced` |
| E1-48 | `brain/dst.rs` `Loss.set` | `F` set without holding `G` (the `Loss.gate` before it removed) | `case8_lifecycle_across_loss_reduced` |
| E1-49 | `brain/rules.rs` `consume_packet` | consumption by unpublished drain samples not recorded | `case1_value_cut_stop_reduced` |
| E1-50 | `brain/core.rs` `step` | outstanding debt treated as a pin (the flush is blocked) | `case7b_flush_final_stop_restore_reduced` |
| E1-51 | `brain/core.rs` `step` | a partial result drops the value from the leftovers | `case9_pending_move_partial_refused_reduced` |
| E1-52 | `brain/dst.rs` `R.recv_end` | samples returned under the current epoch, not the epoch of `R.recv_begin` | `case3_plan_cancel_recv_stop_reduced` |
| E1-53 | `brain/dst.rs` `Tick` | the mandatory-input window check removed | `harness_tick_cannot_pass_mandatory_input` |
| E1-54 | `brain/dst.rs` `S.send` | `Done` delivered before its `S.result` | `harness_result_precedes_send` |
| E1-55 | `brain/dst.rs` `Timer` | a deadline delivery is not recorded as an observation | `case5_blocked_settings_drain_deadline_reduced` |
| E1-56 | `brain/dst.rs` `Loss.gate` | `Loss.gate` enabled while the publishing actor still holds `L` (publication before its native return and release) | `harness_loss_publish_releases_l_first` |
| E1-57 | `brain/dst.rs` lifecycle path | a lifecycle caller holding `G` enters `Loss.gate` again (recursive acquisition) | `harness_lifecycle_inline_loss_publish_no_recursion` |
| E1-58 | `brain/core.rs` `step` | restore eligibility checks pins only, not debt or `left` on the channel | `u09e_restore_waits_for_debt_or_left_on_channel` |
| E1-59 | `brain/core.rs` `step` | the drain-ended transition waits for the pin to clear | `u04d_drain_deadline_sends_stop_while_pinned` |
| E1-60 | `brain/cell.rs` `publish` | a recompute relabels the recv with the new epoch | `u07d_recompute_keeps_recv_identity` |
| E1-61 | `brain/dst.rs` `explore` | invariants checked only on newly inserted states | `harness_violation_into_visited_state_is_caught` |
| E1-62 | `brain/dst.rs` `Tick` | `T` advances before the sample is produced (the first sample is 1) | `harness_first_tick_produces_sample_0` |
| E1-63 | `brain/dst.rs` `B.op` | a step's remaining sends are queued before the section result is fed back | `harness_failed_reservation_leaves_no_send` |
| E1-64 | `brain/dst.rs` `Timer` | a timer is enabled before its deadline | `harness_timer_never_early` |
| E1-65 | `brain/dst.rs` `explore` | an actor still waiting at the epilogue end is reported as `Pass` | `harness_recv_timeout_loop_at_end_is_incomplete` |
| E1-66 | `brain/dst.rs` `S.check` | a discarded settings command gets no `Abandoned` | `u13c_settings_discard_writes_abandoned_per_command` |
| E1-67 | `brain/core.rs` `step` | the next segment waits for empty debt or `left` (over-restriction) | `u09g_start_next_not_blocked_by_debt_or_left` |

Equivalent mutants are not planned: each row changes a behaviour the test asserts. A row that turns out equivalent is reported, not silently dropped.

---

## G. Open questions

Each item: what is unclear or contradictory, the proposal, and its status. All twenty are resolved in draft 2 (45-experiment §9). The line under each item says how; the tests follow the adopted form.

- **G1 (decision). U2 vs I2.** U2 lists phase `Armed` as refused; I2 and note 45 §4.5 procedure 1 accept `Armed(s)`. Proposal: `Armed` reserves (u01b). The U2 row is dropped for Armed.
  - *Resolved in draft 2:* adopted as proposed: `Armed` reserves (u01b). 45-experiment draft 2 §6 (U2) was changed to match.
- **G2. Clock actor missing.** Note 45 §4.7 and §3.1 of 45-experiment say the clock reader takes L, but §3.2 has no event for it and no Clock input. Proposal: `ClockRead` event (take L, read T, release L, emit `Clock(t)`).
  - *Resolved in draft 2, split in draft 3:* the clock reader's events are `Clock.lock`, `Clock.read`, `Clock.unlock`, `Clock.send` (§A; `harness_clock_read_holds_native_lock`).
- **G3. Events missing from §3.2.** Needed by the tests: `R.snap` (U7 needs a window between snapshot and publish); `R.result` (writes the Armed/Streaming/Off/Stopping transitions after a call); `R.cmd(stop)` performing the Stopping transition before the call (note §4.5: stop decision closes acceptance in the same transition); `S.check` (note §4.5 procedure 4: every executor checks before a device call; §3.2 has only R.check); `S.take` producing `Done{Dropped}`. Proposal: add these events to §3.2 with the approval.
  - *Resolved in draft 2:* adopted: `R.snap`, `R.result`, the stop transition in `R.cmd`, `S.check` and `S.take` with `Done{Dropped}` are in the §A event table.
- **G4 (decision). Held values have no settlement rule.** A value held and never reserved (e ≥ cut, Stopping, in-flight bound) is not settled by any rule, but P2 requires every released value settled once (case 4, U2, U14). Proposal: at finish, a held value with e below the next origin stays held for the next segment; a held value with e in the segment and never reserved is settled once as `Reported(NoReservation)`. P2 is then "settled, or held with its reason recorded".
  - *Resolved in draft 2:* adopted with the shutdown rule (45-experiment §9): held values are carried or reported once at segment end (u18a, u18b); at shutdown they are settled by today's `settle` rule (u18c; `prog_p2_shutdown_settles_held_by_settle_rule`).
- **G5 (decision). Pins after `finish`.** `RxChain::finish` (provider/core.rs:335) moves the debt to `left` and clears the segment. The note (§4.4) says a Taken or MaybeApplied value keeps the chain pinned until it is Done, so restore, close, and the next start wait. Proposal: pins come from the cell's in-flight list (state Taken or MaybeApplied), not from the segment; `finish` does not clear them; `begin` waits for none. This is what U11a–c test.
  - *Resolved in draft 2:* adopted: pins come from the in-flight list; `finish` does not clear them; `begin` waits (u11a–c).
- **G6. Device FIFO is head-of-line.** `FakeDevice::drain` (device.rs:1047) takes only the front entry, so a later-booked entry waits behind an earlier one with a later instant. §3.2 of 45-experiment says each value is taken by "the first passing sample at or after its instant and arrival", which is wrong for a non-head entry. Proposal: §3.2 says head-of-line; `RxChain::consume` already gives the same answer per channel (its `left`-first check). u12c and u15e pin it.
  - *Resolved in draft 2:* adopted: head-of-line (u12c, u15e).
- **G7. In-order queue sets the effective instant.** `FakeDevice::apply` sets `effective = max(at, last_command, now)` (device.rs:1433). A value queued behind a command due at 40 takes effect at 40, not at its e = 8. Case 5 as written therefore has v's DDC instant at 40. Proposal: model the rule; case 5's expectation uses 40 (kept, because it tests pinning past the deadline).
  - *Resolved in draft 2:* adopted: the model sets the effective instant as `FakeDevice::apply` does; case 5 keeps 40.
- **G8. Host packet buffer.** §3.2 of 45-experiment buffers packets (`Tick` produces, `R.recv` takes). FakeDevice has no host buffer: `rx_recv` generates on demand (device.rs:1604). U8 and case 3 need packets buffered under an old epoch. Proposal: keep a host buffer in the model (UHD's streamer buffer is real). E2's conformance test then states which FakeDevice behaviour is buffer depth 0 or 1, and a deviation is a finding.
  - *Resolved in draft 2, revised in draft 3:* a host buffer per streamer holding sample ticks; `R.recv_begin` and `R.recv_end` (u15f; case 3).
- **G9. Consumption timing.** D applies a DDC value when the sample is produced (Tick), while the executor's rule records consumption at publish. A value can be applied to a sample that is then discarded (beyond the cut) and so reported as left. Proposal: I4 is checked at settlement; "applied to a published sample" is the test, and applied-but-unpublished is legal.
  - *Resolved in draft 2, superseded in draft 3:* the three kinds of fact (§5): consumption evidence includes unpublished drain samples; I4 is checked at each disposition, and application to a sample the host never received is legal (Sol 3).
- **G10. Seeded configuration count.** 45-experiment says "three values (one on each channel and one equal-time pair)", which is four values. Proposal: four (section D), and state that Stop applies to all chains.
  - *Resolved in draft 2:* adopted: four values; Stop applies to every chain.
- **G11. Case 1's cut formula.** "max(10, progress at booking)" omits the min with the plan cut, which U6b and the note (`request_cut` takes min) require. Proposal: cut = min(plan_cut, max(requested, progress)).
  - *Resolved in draft 2:* adopted: cut = min(plan_cut, max(requested, progress)) (u06a, u06b; case 1).
- **G12. Missing oracle for case 3.** Case 3 requires that no start, close, or open of the new epoch happens before the old epoch's quiesce ack. I1–I11 do not check it (I6 orders publish before ack only). Proposal: I12 "no new-epoch streamer or chain command before the old epoch's quiesce ack", checked at each such command.
  - *Resolved in draft 2:* adopted: I12 (`inv_i12_new_epoch_command_before_ack_is_violation`), checked in case 3.
- **G13. I10 cannot fail.** Cell sections are atomic events, so no device event can run inside one. Proposal: drop I10 as an oracle; enforce by the signature (cell procedures take `&mut Cell` only). The E2 check is a compile-time property of the real cell procedures.
  - *Resolved in draft 2:* adopted: I10 is not an oracle; it holds by signature, with no test in E1 (I10 row of §E).
- **G14. Unset parameters.** In-flight bound (U14), recv timeout, drain allowance, Shutdown trigger, and whether `mark_lost` takes L (case 6). Proposal: bound 4; recv timeout 8 ticks; drain = cut + 8; Shutdown after all other Env inputs and `T ≥ horizon − 8`; `mark_lost` takes no L (it is a flag read by the native destructor, note §4.5 procedure 4).
  - *Resolved in draft 2:* adopted: bound 4, recv timeout 8 ticks, drain cut + 8, Shutdown after the other inputs once `T ≥ horizon − 8`; `mark_lost` takes no lock. Resolved in draft 4: the epilogue (§D; `harness_deadline_beyond_last_input_is_observed`).
- **G15. Paths outside E1.** LateCommand (start behind now), flush, `Uncertain` (failed start), overflow and sequence faults, DUC. The start failure (`Uncertain`) and LateCommand are the two most relevant to the past bugs of §7 (D9b-2, L1). Proposal: state them out of scope in the E1 report; add LateCommand and the failed start to E1b if the owner wants them before E2.
  - *Resolved in draft 2:* answered: case 7 (7a, 7b, 7c), U16–U18 and I13 bring the three past-bug paths into E1 (45-experiment §9).
- **G16 (decision). Reuse of `RxChain::consume` needs visibility.** `provider` has `mod core;` (private); the brain cannot see `RxEntry`, `SegmentKey`, or `RxChain`. Proposal: `pub(crate) mod core;` in `provider/mod.rs`, one line, the only production edit in E1a. It is not exactly "deleting src/brain restores today's crate" (the visibility stays), so the owner should accept it. Alternative: copy the rule into `rules.rs` and keep U12d as a differential test against `RxChain::consume`. I prefer the reuse (the note says "reuse, do not copy").
  - *Resolved in draft 2:* accepted: `pub(crate) mod core;` is the one production edit in E1a (45-experiment §1 says so).
- **G17. `DSP_FIFO` is private.** Proposal: the model takes capacity 32 as a parameter; E2 checks it equals FakeDevice's constant (needs `pub(crate)` then).
  - *Resolved in draft 2:* adopted: the capacity is a parameter (32); E2 checks that it equals `DSP_FIFO`.
- **G18. U10 wording.** "pinned to `left` handling" is undefined. Proposal: `Done(Partial)` releases the pin (note: 終わった) and the value joins the maybe-applied obligations that the segment's finish carries as `left`. U10 asserts the release and the absence of a failure count; u11b covers the carry.
  - *Resolved in draft 2:* adopted: `Done(Partial)` releases the pin, and the value joins the `left` obligations (u10, u11b).
- **G19. Mutation IDs.** The existing prefixes are used (`P…`, `J…`, `T…`, `F…`, `H…`, `G…`). Proposal: `E1-NN`. The tools do not restrict the form, so this is a naming choice.
  - *Resolved in draft 2:* accepted: `E1-01` to `E1-45` (45-experiment §9).
- **G20. Wording of I5.** "R never publishes a sample at or after the current cut" should say "the cut of the epoch the sample belongs to", because after an epoch advance the retiring cut bounds old samples. Also a line-count note: 45-experiment's references to note 45 (`core.rs:152-156` for `eligible`) are stale; `eligible` is now at `core.rs:176`. Both are wording only.
  - *Resolved in draft 2:* adopted (wording only): I5 is stated per epoch; this plan does not cite the stale `core.rs:152-156`.

---

## Stage and gate notes

- E1a code (A, the pure tests in B, the rules, the model) plus E1b code (C, D, E, and the F rows that touch `dst.rs` and `oracle.rs`) are one gate. Mutation rows for E1a code run with E1b.
- **First E1b task: measurement.** Each case's reduced variant at horizons 8, 16 and 24: states, transitions and time, then extrapolation, reported before any full horizon runs (§3.3). The estimates in §D are hand estimates (INFERRED).
- No wall-clock waits: model time is `Device.t`, advanced only by `Tick`. The review checks `brain/` for `sleep`, `Instant` and `SystemTime`.
- Default `cargo test` runs the B and C tests and the reduced D and E tests. `#[ignore]` runs the exhaustive D tests and the seeded sweep on sim02, through `rdev.py`.
- Test temp files on `/tmp` only (the harness keeps traces in memory).

### Contradictions and proposals for the orchestrator (found while amending drafts 3 and 4)

Items 1–5 (case 7c, the lock order, the monitor in the key, horizon-end timers and window notation) were resolved in draft 3a (45-experiment §10) and are removed here.

6. **Budget.** At per-sample ticks, case 3, case 6 (every k) and the seeded configuration are likely over the 10-minute budget at full horizon. The measurement decides. Report counts; do not compress (Sol 11).
7. **Case 7a's cut-free shape.** Draft 3 says the segment is cut-free. The plan takes that as "no plan cut", so `LateCommand` is the only restart path. A segment with a plan cut that has not yet been reached would not be cut-free. Confirm.
8. **`R.check` and `Abandoned`.** 45-experiment §3.2 says `R.check` writes `Abandoned` "for each discarded command". Rx commands carry no settings terminal and P2 counts settings commands, so this draft writes `Abandoned` from `S.check` only. Proposal: confirm that R writes nothing. Status: adopted in draft 4, for the orchestrator to confirm.
9. **Case 9 and I15.** The case 9 script in 45-experiment §4 has no restore request, so I15 cannot be exercised there. Draft 4 adds `Request(Restore: ch0)` [0, 16] (reduced [0, 8]). The cost is one more input. If the budget forbids it, I15 stays with case 5 and the unit tests (u09e, u09f) and the oracle tests are unaffected. Confirm.
10. **Epilogue values.** 45-experiment §3.2 requires an epilogue of at least the latest deadline plus one recv timeout but gives no value. Draft 4 sets 32 ticks for the cases and 96 for the seeded configuration, checked by `harness_epilogue_covers_deadlines`. Confirm.
11. **Move and debt.** Draft 4b lets a move wait for pins only (case 9; I15 names restore only). A move of a channel with `left` on its old chain could also need the I15 condition. Proposal: keep pin-only unless the orchestrator extends I15 to moves. Confirm.
