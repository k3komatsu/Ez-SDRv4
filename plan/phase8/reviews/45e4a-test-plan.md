# Test plan: note-45 experiment E4a (E4a1 + E4a2), phase 1

Author: implementer (Sonnet 5.5). Status: phase-1 draft for Sol's review. No code was written. Paths are under `crates/ezsdr-radio-uhd/src/` unless they start with `plan/`. "Verified" means I read the line; "inferred" says so.

## 0. Decisions this plan takes (each is an open question in §12 if you disagree)

1. **Device time rides on `Done`, the two timers and `LateCommand`.** `Input::Done { id, terminal, now }`, `Input::Timer { stream, now }` (drain), new `Input::CapacityTimer { now }`; `LateCommand` already has `now`. No separate `Clock` input. The brain keeps `now = max(now, input.now)`; nothing else is read as time (R's progress is never time).
2. **K and D are computed by two pure functions over the cells, never stored.** D is today's `at_bound` (cell length) made precise by the retire rule; K is new.
3. **H03: the second guard is not redundant.** `start_confirmed` makes progress >= cut without closing acceptance (§4). So the guard stays, a separating case is added, and the "accepting => progress < cut" state invariant is not added.
4. **B15 order: publication boundaries first, then `LateCommand`** (the order the threads already use), from one function in `rules.rs` (§5).
5. **E4a is about 3,500 lines (over 3,000)**: E4a1 about 1,800, E4a2 about 1,700 (§10). Both stages pass the 1,500-line stop-and-report checkpoint of experiment §3.3; I stop and report at E4a1's end.

## 1. Inputs, commands and who sends device time

| Change | Type | DST (world) | Threads |
|---|---|---|---|
| `Input::Done` gains `now: i64` | `core.rs` | `SSend` stamps `device.t`; `s_take`'s `Dropped` and `abandon_command` stamp `device.t` at their event | S reads `(sh.now)()` (the instance time source, a pure read; no native lock) right before `sh.send`; same in `threads.rs:563`, `shared.rs:263` |
| `Input::Timer { stream, now }` | `core.rs` | `Event::Timer` delivery stamps `device.t` (never early: enabled only at `T >= at`) | `run_timer` already reads `now`; it passes it |
| `Input::CapacityTimer { now }`, `Command::CapacityTimer { at }` | new | `timers: Vec<(TimerKind, i64)>`, `TimerKind = Drain(stream) \| Capacity`; same enabling rule | same `timers` vector with the kind; `run_timer` delivers either kind |
| `Brain.now` | `core.rs` | in the key (the brain tuple) | cloned with the brain |

The drain timer's `drain_observed` and `observed` monitor facts are written only for `Drain` timers, so a capacity timer never satisfies P1. Replay stays equal because both wrappers stamp the same instant at the same hook: world `device.t` at the `SSend`/`STake`/`Timer` event, threads `model.now()` in forcing mode.

Brain methods that read time use `self.now` only. A test that sets `cell.progress` large while `brain.now` is small must still see K counted (B2b).

## 2. How K and D are computed (and why nothing else stores them)

```
fn counts_for_k(f: &InFlight, now: i64) -> bool    // core.rs, pure
  Queued | Taken | MaybeApplied              => true      // regardless of the value's instant
  Done(Call(Ok | Partial))                   => now < f.entry.order.0   // until device time reaches e
  Cancelled | Done(Call(Refused | Lost) | Dropped | Abandoned) => false
fn device_load(cells, now) -> usize = cells.iter().flat_map(in_flight).filter(counts_for_k).count()   // K
fn resident(cell) -> usize = cell.in_flight.len()                                                      // D
reserve(): refuse (stay held) if device_load(cells, self.now) >= device_bound || resident(cell) >= chain_bound
```

Facts read: `ValueState`, the value's instant, `brain.now`. No counter, no flag. Consistency with the rest: a value leaves `in_flight` only through `retire` (refused, dropped, abandoned, lost: at `Done`/loss; ok or partial: when a consumption is also seen). This is today's rule, so D is today's `at_bound` with the default 64 and K is the new device-wide predicate. Resolution of a tension in the design, for Sol to confirm: B9 says "K counts it until `Done(ok)` and now >= e" while the cell removes the value at `Done`. A consumed value is device evidence that device time >= e (consumption needs a sample at or after the instant), so removal at `Done` after consumption loses nothing; the brain's stale `now` is never the reason to keep a record outside the cell.

`Done(Call(Lost))` is not counted (the device is lost; no reservation follows on any chain whose `Lost` boundary has arrived). Open question 4.

Defaults `DEVICE_BOUND = 4`, `CHAIN_BOUND = 64` replace `IN_FLIGHT_BOUND`. Brain fields `device_bound`, `chain_bound` (both set from `Params.k`, `Params.d` in `World::new`, so a case or the E2b diagnostic overrides them; the key keeps `params` and drops the brain fields with that reason). `Params` gains `k`, `d` (six literal sites: `cases.rs:90`, `oracle.rs:518`, `thread_tests.rs:229`, `world.rs:1471`, `speed/newrt.rs:166`; the `..` sites follow). `NotReserved::Bound` splits into `DeviceBound`, `ChainBound` (speed's `REASONS` and `bound_rejections` become K and D counts; the rest of E4b's counters are E4b's).

## 3. Held re-evaluation and the capacity timer

`Section::ReserveAll`: attempts every held value in global release order (one `held` Vec, so equal-time order and K contention follow release order; a K or D refusal refuses every later value too). `fn reevaluate()` front-queues one `ReserveAll` (when `held` is non-empty) and then books the capacity timer. It is called at the end of: `Done` (every terminal), `Consumed`, `Stop` and `PlanCancel` and `stop_send` (cancels free K), `Timer`, `CapacityTimer`, `Armed`, `FirstSample`, `Lost`, `Finished`. `retired()`'s special `ReserveHeld` (only when the chain was at the bound) goes away: re-evaluation is unconditional on the freeing event, so the E2b liveness defect (`u14b`) has one rule, not two.

Capacity timer: after re-evaluation, if a held value was refused by K, book `Command::CapacityTimer { at }` with `at` = the earliest `e` among K-counted `Done(ok|partial)` values with `e > now`; none if no such value (Queued/Taken/MaybeApplied free by `Done`, which re-evaluates; D frees by no timer). `Brain.capacity_timer: Option<i64>` remembers the booked instant to avoid one timer per evaluation (re-book only if earlier than the booked one; cleared when a `CapacityTimer` input arrives). It is timer bookkeeping, not part of K or D; Sol may prefer deriving duplicates away differently (open question 5). A stale, duplicate or post-`F` timer takes `Section::CapacityTimer`: update `now`, `reevaluate()`, nothing else; it never touches `drain`, `outstanding`, `epoch`, the phase or sends an `Rx` command.

## 4. H03: the search and the decision

Every transition that changes `progress`, `cut` or `phase` (search: `rg 'progress =|\.cut =|phase =' brain/{cell,core,rules}.rs` plus `book_stop`, `advance_epoch`, `start_confirmed`, `finish`):

| Transition | progress | cut | Closes acceptance in the same section? |
|---|---|---|---|
| `publish_planned` (cell.rs:361) | writes | no | yes: `cut_reached` calls `stop_accepting` |
| `book_stop` (cell.rs:261) | no | `<= progress`-clamped (`requested.max(progress)`) | no, but the brain step that calls it front-queues `Section::Drain` whose `stop_send` closes acceptance, and no other brain step runs between (ops finish before the next step); executor events do not reserve |
| `advance_epoch` (cell.rs:425) | no | writes plan cut | `PlanCancel` called `stop_accepting` first |
| `finish` (cell.rs:331) | resets to 0, cut None | yes | `stop_accepting` first |
| **`start_confirmed` (cell.rs:415, from `rules::after_call` Begin ok)** | **writes origin** | **`cut.max(origin)`: a Stop booked below the new origin gives cut == origin == progress** | **no**: the phase is `Armed` with progress >= cut until a publish or the brain's `Armed` section runs `drain_booked` |

So between `RResult(Begin ok)` and the brain's `Armed` boundary, a queued `Release` step reaches `reserve` with progress >= cut and an open phase. Only the `progress >= cut` guard refuses it (`I2` checks the same condition at reservation, which is why a missing guard would be caught if a case reached the window; none does: E3's H03 was found only by the unit `u02g`). **Decision: keep the guard; add case 12b** (§9) that reaches the window (Stop booked while `Off`, `StartNext` above the cut, a `Release` in the window), witness `W12b: a reservation refused with Refusal::Progress while the phase is Armed`. The alternative (close acceptance in `start_confirmed` when the clamped cut equals the origin, then delete the guard and add the invariant) is cleaner but changes the cell's phase at the start and the `Armed` boundary's expectations; I recommend not taking it in E4a. The invariant "accepting => progress < plan cut" is therefore not added; I2 stays the oracle.

## 5. B15: the replay order

World: `r_end` pushes `LateCommand` into the program at the recv's end, then `r_publish` appends the boundaries behind it (`world.rs:1044,1094`). Threads send the boundaries, then `LateCommand` (`threads.rs:911-921`). Verified in both (Sol diagnosis items 1, 2).

**Chosen order: boundaries, then `LateCommand`.** Reason: a missed-start recv carries no samples, but `publish_planned` still returns `cut_reached` when progress is already at the cut, so `CutReached` (the Stopping transition's boundary) can be among the sends. The brain's `Late` decision reads the phase (`decide`: only `Armed`/`Streaming` restart or end-now): after `CutReached` it sees the closed cell and decides `Never`; in the other order it can restart or end-now a stream whose stop is already decided. This is also production's order (`provider/rx.rs`: the recv's result is handled after the buffer is published; inferred from the thread tests, not re-read line by line).

One function: `rules::recv_inputs(round_sends: Vec<Input>, late_send: Option<Input>) -> Vec<Input>` (boundaries first, late last), called by `world.rs::r_publish` (the late input moves from the program into `RecvCall.late`, so a stale recompute keeps it) and by `threads.rs` (replacing its loop plus trailing `if let`). Maintainability 1: both wrappers had a copy; there is now one.

Shrink display (`thread_tests.rs:643-653`): shrink every divergence up to a cap of 10 and print "not shrunk (cap)" instead of `shrunk to 0 events: []` for the rest; the report line prints the first differing payload as well. Classification run: `s1c_whole_walks_wide` seeds 1-10,000 on sim02/sim03 via `rdev.py`; every divergence is named Q1-A, Q1-B, or by its first-difference cause in `plan/phase8/` (a table in the E4a1 report). A divergence with no cause blocks E4a1's gate.

## 6. Oracles

The monitor gains facts recorded at events (the oracle reads these, never the brain's state): `instants: id -> e` (at `Release`), `cancelled: BTreeSet<id>` (recorded in `sync` when a flight is first seen `Cancelled`), `done_processed: BTreeSet<id>` (at the `BStep` of `Input::Done`), `starts: BTreeSet<(stream, epoch)>` (when the brain routes `RxOp::Start`). All enter the key (they decide later verdicts). `terminals` and `reserved` exist already.

| Oracle | Statement | Check | Observation | Wrong behaviour that fails it |
|---|---|---|---|---|
| **I11′** safety (replaces I11) | at each reservation transition, K and D each, from monitor facts and `params.k`, `params.d`: `K_true = #{v in reserved : not gone(v) and live(v)}` with `live(v)` = no terminal and not cancelled, or terminal ok/partial with `T < instants[v]`; `D_k = #{v reserved on k : not (done_processed(v) and (terminal not ok/partial or evidence/leftover seen))}`; `gone` = brain disposition `Reported(Loss)` (retired at loss). Violation when `K_true >= k` or `D_k >= d` before the reservation | `oracle::i11` | `reserved_by(before, after)` as today | K per chain; K never counts a done-ok value; K frees at `Done` before `T >= e`; D counts only K values; D off by one; a mutated brain bound |
| **I11′ progress** (P6, an addition) | at a terminal state before shutdown, no value is held whose stream accepts it (phase, channel, generation, origin, cut) while `K_true < k` and `D_k < d` | `oracle::p6` | terminal states | the E2b defect: a value held forever after a freeing event had no re-evaluation; R progress used as time |
| **I17** | a cell transition `Stopping(_) -> Armed/Streaming` happens only at `RResult(k)` of a Begin whose start is in `monitor.starts` for `(k, cell.epoch)` | `oracle::i17`, per transition | before/after cell phases | `FirstSample` (or any section) writing a running phase over `Stopping`; `device_result(Begin)` admitted from `Stopping`; a start of a retired epoch reopening the cell |
| **I18** | `Pub(_, Record)` and `Pub(_, Mark)` are enabled only after `Set` (`before.failure`); the order of one publication is Gate, Set, Ungate, Record, Mark | `oracle::i18` | `PubStep` events and `world.failure` | record or `mark_lost` before `F`; mark before the ungate (already a case test) |
| **I19** | a handle is closed and destroyed exactly once at cleanup when `F` is clear and nothing is detached; after `F` or a detach no cleanup close or destroy happens, and a destroy never happens while the native mark is set | `oracle::i19` (transition) + progress half at the cleanup's end | `Cleanup(Close/Destroy)` events, `device.handles`, `monitor.marked`, `failure`, `detached` | cleanup closes after `F`; destructor frees although `marked`; panic guard without `mark_lost`; a handle destroyed twice or never |
| **I20** | `Detach(i)` only at `T >= join_start(i) + join_limit` (virtual time); `Cleanup` begins only when every executor exited or is detached; with any detached, cleanup closes and destroys nothing | `oracle::i20` | `Stopper` state in the world, `monitor.join_started` | detach before the limit; join without limit (P4 hang); cleanup while an executor still runs; handles closed despite a detached S |
| **I21** | no `Command::Rx` is routed to an unspawned stream; the failing stream's cell is abandoned (`Uncertain`, `Lost` kept, no segment, `Abort`) iff its `rx_start` had succeeded; no other stream is abandoned; no wait for an unspawned executor's quiesce or exit (`incomplete()` skips it); P2 and P3 hold for the rest | `oracle::i21` + `incomplete` | `routed()`, cell phases, `monitor.spawn` facts | abandoning R0; abandoning R1 when `rx_start` had not run; sending R1 a command; waiting for R1's quiesce (livelock/incomplete); overwriting `Lost` with `Uncertain` |

Each oracle has unit tests with hand-built before/after worlds (the style of `inv_i11_*` in `oracle.rs`): one legal and one violating state, and the violating one named in the table. I17 and I19–I21 cannot fail on the unmutated model, so their power is shown by mutation rows that a DST test (not only a unit test) must kill (§11).

## 7. Sequences B1-B15

Levels: **U** = unit test in `core.rs`/`cell.rs` through `feed` (`fixtures.rs`, no harness); **W** = world/DST case (§9); **T** = threads (`thread_tests.rs`/`device_tests.rs`). Names are `b<n>_...` unless stated; fixtures add `brain_kd(k, d)` and `drain_timer(stream, now)`.

**B1** (U `b1_stalled_r_device_time_frees_k_in_order`; W case 10 micro). Default K = 4, one chain, six releases at e = 10, 20, …, 60, no `Consumed`. Asserts: the first four produce `Settings` commands 1-4 in the outbox; 5 and 6 stay in `held`; `Done(1, ok, now = 5)` leaves both held (1 is counted: now < 10) and books a `CapacityTimer { at: 10 }`; `CapacityTimer { now: 10 }` reserves 5 (not 6), books `at: 20`; no disposition is written; all six are in `cell.in_flight` (D) until `Consumed` arrives; then each consumed value is retired at its `Done`. Observation: outbox, `held`, `in_flight`, `dispositions`. Fails: K per chain or no K (5 reserved at once); a freed slot at `Done` before `e` (E4-01, E4-04); no re-evaluation on the timer; timer at the latest instant (E4-11); invented consumption.
**B2** (U `b2_slow_call_holds_k_until_done_then_frees_in_that_step`, `b2b_r_progress_is_not_device_time`; W case 10). K = 1, A taken and maybe-applied, B held; `Done` is not yet delivered. B2: the call outlasts e_A; time input is the next `Done`: with `now = e_A` exactly (boundary) B is reserved in the `Done` step (same `feed`); before it, no `Settings` for B. B2b: set `cell.progress` far past e_A with `brain.now` below e_A: B stays held after `Done(ok)`. Fails: A freed by time while still `MaybeApplied` (E4-02 variant); B not reserved at `Done` (E4-08); `<=` instead of `<` (E4-01); progress used as time (E4-15).
**B3** (U `b3_partial_frees_k_keeps_a_resident`, `b3b_refused_frees_both`; W case 10 branches `S.effect` over ok/partial/refused). D = 2, K = 1: A `Done(partial, now >= e)`: B reserved, A still in `in_flight`, a third value C stays held by D (resident 2); refused: A leaves the cell at `Done`, K and D free, C reserved. Fails: partial treated as refusal (A removed); refused kept counted (E4-03); D not counting a partial resident (E4-06).
**B4** (U `b4_stop_cancel_frees_k_and_d_frees_at_dropped`; W case 11). K = 1, A queued (e = 10), B (e = 4) held; `Stop { requested: 5 }`: A cancelled, B reserved in the `Stop` step (cut 5 > 4); A still resident until `S.take` then `Done(Dropped)` removes it. Fails: K counts `Cancelled` until `Dropped` (E4-02: B waits); D freed at the cancel (A absent before `Dropped`, `Take::Absent`); no re-evaluation after a cancel (E4-10).
**B5** (U `b5_k_is_shared_across_chains`; W case 11). Two cells, K = 2, chain 0 holds two future-instant values, chain 1's value (e = 5) is held; `Done(ok, now >= e)` of one chain-0 value reserves chain 1's value; D of chain 1 stayed 0 throughout. Fails: per-chain K (chain 1 reserved at once, E4-05).
**B6** (U `b6_d_holds_until_a_resident_leaves`; W case 10). D = 3 resident (done ok, R stalled), K free: the fourth value is held; `CapacityTimer` and `Done` with later `now` leave it held and book no timer (no K-blocking value); `Consumed(id1)` retires id1 and reserves the held value in that step. Fails: D freed by time (E4-06); D `>`/`>=` (E4-07); no re-evaluation on consumption (E4-09).
**B7** (U `b7_left_value_stays_resident_and_is_consumed_later`; W case 12a). A applied (done ok, e = 20), cut 10, drain deadline: `Finished` reports A `Reported(Left)`, A stays in `in_flight` and in `resident`; with D = 1 a new value stays held; a later `Consumed(A)` records a leftover and then removes A and reserves the held value. Fails: `Finished` retiring left values (E4-17); cut report treated as consumption.
**B8** (U `b8a_capacity_timer_during_drain`, `b8b_stale_capacity_timer_after_epoch_change`, `b8c_capacity_timer_after_loss`, `b8d_duplicate_capacity_timer`; W case 12a). (a) cell `Draining`: the timer leaves `drain`, `outstanding` and the outbox's `Rx` commands unchanged and sends no `Stop`; (b) after `PlanCancel`: no `Start`/`Plan`/`Close`; (c) cell `Lost` (after `Boundary::Lost`): no command at all; (d) delivering the same timer twice leaves `(brain, cells)` equal to after once. W: step check `capacity_timer_inert` on `Section::CapacityTimer` ops. Fails: capacity timer routed through `TimerFired` (E4-12); timer lowers `now`; timer reserves after `F` (cells refuse by phase, so this row targets the section, E4-13).
**B9** (U `b9_consumption_before_done`; W case 10). K = 1, A taken/maybe-applied, B held; `Consumed(A)` first: disposition `Consumed`, A resident, B held (A is still counted: its call has not returned); then `Done(ok)` with a stale `now`: A leaves the cell and B is reserved in that step. Fails: A removed at `Consumed` (E4-16); A retained after `Done`.
**B10** (U `b10_stopping_is_never_reopened_by_an_input`; W case 12a, 12b). For `Phase::Stopping` cells, every `Input` variant and `Boundary` variant (a table) is fed; the phase stays `Stopping` (or leaves it only through a stop's result); the legal path (`Stop` result then `Start`'s `RResult`) is a separate test that does reach `Armed`. Fails: `FirstSample` writing over `Stopping` (E4-19), `Armed` boundary writing a phase.
**B11** (W case 13; T `b11_loss_publication_order_f_record_mark`). A loss at every point (as case 2) plus the record event: I18 on each transition; the thread test reads `sink.order == ["F", "record", "mark"]`, and `checking.calls()` shows `mark_lost` with F set. Fails: record before F (E4-22), mark before record or before the ungate, a publication without F.
**B12** (W case 13; T `b12_cleanup_closes_destroys_once_or_retains`). Three runs: no loss/detach (every opened handle closed and destroyed once, `device.handles` all `Destroyed`), a loss (retained: no cleanup close or destroy), a panic (guard sets F and mark, destructor retains). Thread version: `ModelDevice::close_streams` implemented (today it panics "the executors never close every stream"), `mark_lost` records the mark; the test calls `Runtime::cleanup()`. Fails: close after F (E4-25), destructor ignoring the mark (E4-26), guard without mark (E4-27), double destroy.
**B13** (W case 14; T `b13_shutdown_detaches_after_the_join_limit`). S blocked in `apply` holding `L` (queue capacity 1 already full, as case 5), `Shutdown` delivered; the stopper's join of S begins at `T0`; `Detach` becomes enabled at `T0 + join_limit` and not before; cleanup then retains handles and closes nothing; R joins in order after S. Thread version: advance the model clock to the limit, mark the blocked actors detached, then `cleanup()` and check the log. Fails: detach early (E4-29) or never (P4 hang, `Incomplete`), cleanup with a running executor (E4-30), handles closed after a detach (E4-31).
**B14** (W case 15 with two variants; T `b14_spawn_failure_abandons_only_a_started_stream`). Two receive streams. (a) R1's spawn fails after `rx_start(1)` succeeded: `Input::SpawnFailed { stream: 1, started: true }`: the start fails (Abort shutdown), only cell 1 becomes `Uncertain` (stays `Lost` if lost), no command to R1, no wait for R1's quiesce, R0 stops in the normal order, every value disposed once. (b) failure before `rx_start(1)` (`started: false`): nothing abandoned. Fails: R0 abandoned (E4-33), (b) abandoned (E4-34), command to R1 (E4-35), wait for R1 (E4-36), `Lost` overwritten (E4-37).
**B15** (U `b15_recv_inputs_order`; T `s1c_whole_walks_wide` 1-10,000). The unit test calls `rules::recv_inputs` with `[CutReached]` and a late input and asserts boundaries first; a world test and a thread test run the same recv (a missed start with progress at the cut) and compare the inbox order; the wide sweep must classify every divergence (§5). Fails: late before boundaries (E4-21).

## 8. What the four model concepts add

| Concept | `world.rs` / `model.rs` | `core.rs` | `threads.rs` / `shared.rs` / `model_device.rs` | Exercised by |
|---|---|---|---|---|
| Loss record (E4a2 #8) | `PubStep::Record` after `Ungate` (`PUB_STEPS` has five); `monitor.recorded`; brain guard keeps four steps (open question 6) | none | `publish_step` gains `Record` and `sink.order` records it | case 13, case 2 and 8 (every loss walk), seeded config |
| Handle destruction (#9) | `Device.handles: id -> Open/Destroyed`; events `Cleanup(Close(k))`, `Cleanup(Destroy(k))`, `CleanupRetain`; destroy checks `monitor.marked`; `Cleanup` is a stopper step, not a brain step | none | `ModelDevice::close_streams` implemented, `mark_lost` stores the mark, `Runtime::cleanup()` | case 13 (three variants: clean, loss, panic via case 6's k = 0) |
| Join timeout and detach (#10) | `Params.join_limit`; `Stopper { next, started_at }`; events `Join`, `Detach`, then `Cleanup`; `detached: BTreeSet<Actor>`; a detached actor keeps its enabled events (it is still running) | none | `Runtime::shutdown_with_limit(ticks)` detaches in model time | case 14 (case 5's blocked S plus `Shutdown`) |
| Spawn failure (#11) | a prefix of the run: `Start(k)` (`rx_start` call), `Spawn(k)` outcome ok/fail (`Params.spawn_fail: Option<(usize, Before/After)>`); `RxExec.spawned`; an unspawned executor has no program and skips `Terminate` | `Input::SpawnFailed { stream, started }`; `Brain.unspawned: BTreeSet<usize>` (written once by the input, read by `decide` and `terminates_ready`); `abandon_unowned` on the cell when `started` | `Runtime::start` option skips spawning R_k and sends the input | case 15 (two variants), seeded config with `spawn_fail` off |

`unspawned` is an input-delivered fact like `exit`, not a derived quantity; the cell gets no new field.

## 9. Cases and the seeded configuration

Numbering: E4a1 cases 10, 11, 12a, 12b; E4a2 cases 13, 14, 15 (15a, 15b). Each has Full, Reduced and Micro variants like cases 1-9; `Params.k`/`d` are small in micro (k = 1, d = 2; K = 2 in case 11) and default (4, 64) in Full.
- **10** (one chain; B1, B2, B3, B6, B9): values v1..v3 released early with instants spaced so a time input and a `Done` interleave; `S.effect` over ok/partial/refused; `queue_cap` large so S does not block. Witnesses: `W10a` held at K, `W10b` reserved after a `Done` freed K, `W10c` held at D, `W10d` K freed by time (`Done` ok value past e), `W10e` a capacity timer delivered with a held value.
- **11** (two streams; B4, B5): K = 1, a on channel 0 (e = 3), b on channel 1 (e = 3), a Stop on stream 0 below e_a. Witnesses: `W11a` b held while a counts, `W11b` K freed by a cancel, `W11c` a `Dropped` after the cancel.
- **12a** (one stream; B7, B8, B10): value applied, Stop cut < e, drain deadline, a held value waiting for time, capacity timers delivered during the drain, after a `PlanCancel`, after `Loss`. Witnesses: `W12a` capacity timer while `Draining`, `W12b'` after an epoch advance, `W12c` after `F`, `W12d` a left value resident then consumed.
- **12b** (H03 + I17; B10): cell `Off`, handle open; Stop requested 2 (booked, drain `Booked`), `StartNext { at: 4 }`, `Release` (e = 6) in a window covering the gap. Witnesses: `W12b` a reservation refused with `Refusal::Progress` while `Armed`; `W12e` a `FirstSample` boundary after the Stopping transition (the I17 path).
- **13** (B11, B12): case 2's loss points with the record; plus the cleanup after shutdown (clean / loss / panic).
- **14** (B13): case 5's blocked S, then `Shutdown`; `join_limit` 4 (micro 2). Witnesses: `W14a` detach with S holding `L`, `W14b` cleanup with a detached S retaining handles.
- **15a/15b** (B14): two streams, `spawn_fail = (1, After)` and `(1, Before)`. Witnesses: `W15a` abandoned, `W15b` not abandoned, `W15c` R0 stopped normally after the failure.
- **Seeded configuration** gains: K and D at their small values with one value more than K (instants spaced so time frees), a capacity timer, the loss record and cleanup after shutdown, `join_limit`; spawn failure stays off in it (a start prefix changes every script's start; cases 15 carry it).

Search cost is the risk: each case adds events (stopper, cleanup, timers); micro variants stay bounded searches (the experiment's ruling): they report states and witnesses and fail only on a violation, a cycle or an incomplete run. If a micro case does not reach a declared witness within `SMALL_BUDGET`, I report it and shrink the script, never drop the witness.

## 10. File layout and size

No new file except none: the pieces go where their siblings are (core.rs, cell.rs unchanged except two small helpers, rules.rs for `recv_inputs` and the K helper's tests, world.rs, oracle.rs, cases.rs, threads.rs, shared.rs, model_device.rs, thread_tests.rs, speed/). `oracle.rs` and `cases.rs` grow most; I do not propose splitting them in this step (Phase-8-end review).

| Stage | Body | Tests | Notes |
|---|---|---|---|
| E4a1 | about 750 (core 150, world 120, oracle 110, cases 260, threads/shared 60, rules 25, speed 25) | about 1,050 (units 380, oracle 180, world 120, cases 110, thread 200, churn of about 50 `Done`/`Timer` sites) | stop-and-report at 1,500 |
| E4a2 | about 650 (world/model 420, core 60, oracle 150, cases 200 incl. variants) | about 1,050 (world 300, oracle 200, threads 250, case tests 300) | |
| Total | about 1,400 | about 2,100 | **about 3,500, over 3,000**: the units use tables (one test per sequence family where the setup repeats) and I cut first the Reduced variants of cases 13-15 |

## 11. Mutation rows (planned; ids from E4-01; token anchors for Rust; anchors are re-fixed against the final text in phase 2)

| Id | Row | Killed by |
|---|---|---|
| E4-01 | `counts_for_k`: `now < f.entry.order.0` -> `now <= f.entry.order.0` | b2 (boundary now == e) |
| E4-02 | `counts_for_k`: `ValueState::Cancelled => false` -> `true` | b4 |
| E4-03 | `counts_for_k`: `Done(Terminal::Dropped) ... => false` -> `true` (and refused) | b3b |
| E4-04 | `counts_for_k`: `Done(Call(Ok \| Partial)) => now < e` -> `false` | b1; case 10 I11′ |
| E4-05 | `device_load`: `cells.iter().flat_map` -> `cells[stream].in_flight.iter()` | b5; case 11 |
| E4-06 | D: `resident` counts only `counts_for_k` values | b6, b3 |
| E4-07 | D: `>= self.chain_bound` -> `> self.chain_bound` | b6 |
| E4-08 | `reevaluate` not called in `done` | b2 |
| E4-09 | `reevaluate` not called in `consumed_by_sample` | b6, b9 |
| E4-10 | `reevaluate` not called after `stop_send`/`Stop`/`PlanCancel` | b4 |
| E4-11 | capacity timer `.min()` -> `.max()` | b1 |
| E4-12 | `Input::CapacityTimer` handled as `Section::TimerFired` | b8a; case 12a step check |
| E4-13 | `Section::CapacityTimer` also clears `drain` | b8a, b8b |
| E4-14 | `self.now = self.now.max(now)` -> `self.now = now` | b8d, b2 |
| E4-15 | K reads `cell.progress` instead of `self.now` | b2b |
| E4-16 | `consumed_by_sample`: drop `&& done_ok(cells, id)` | b9 |
| E4-17 | `finished`: retire left values | b7 |
| E4-18 | `cell.reserve`: delete the `self.progress >= cut` check | case 12b (W12b witness test and I2) |
| E4-19 | `Section::FirstSample`: `Phase::Armed(segment)` -> `Phase::Armed(segment) \| Phase::Stopping(segment)` | b10; case 12b DST (I17) |
| E4-20 | `device_result(Begin)`: allow from `Stopping` | I17 unit; case 12b DST |
| E4-21 | `recv_inputs`: late before boundaries | b15 |
| E4-22 | `PUB_STEPS`: `Record` before `Set` | I18 unit; case 13 |
| E4-23 | `Mark` before `Ungate` | case 13 |
| E4-24 | oracle `i18`: drop the `before.failure` test | I18 violation unit |
| E4-25 | cleanup closes without the `failure` test | I19; case 13 |
| E4-26 | destroy ignores `monitor.marked` | I19; case 13 panic variant |
| E4-27 | guard drops `MarkLost` | case 6 / case 13 |
| E4-28 | handle destroyed twice (cleanup loops over closed handles) | I19 progress half |
| E4-29 | `Detach` enabled at `join_limit - 1` | I20; case 14 |
| E4-30 | `Cleanup` enabled with an executor neither exited nor detached | I20 |
| E4-31 | cleanup closes despite `detached` | I19/I20; case 14 |
| E4-32 | no join limit (detach never enabled) | case 14 (`Incomplete`) |
| E4-33 | spawn failure abandons every started stream | I21; case 15a |
| E4-34 | abandon when `started` is false | case 15b |
| E4-35 | brain sends a command to an unspawned stream | I21 |
| E4-36 | `terminates_ready` / `incomplete` wait for an unspawned executor | case 15 (`Incomplete`) |
| E4-37 | abandon overwrites `Lost` with `Uncertain` | case 15 with loss |
| E4-38 | oracle `i11`: `K_true >= k` -> `K_true > k` | I11′ unit |
| E4-39 | oracle `i17`: remove the `monitor.starts` test | I17 unit |
| E4-40 | `world.rs` `ClockSend`/`SSend` stamps `now` 0 (a time input that carries nothing) | b2, thread replay |

Rows run only through `remote_mutate.py` at the gate. Old rows for `IN_FLIGHT_BOUND` (E1/E2 `u14*`, E1-xx I11) are retargeted or removed in the same step (Maintainability 4: leftovers); I list them when I implement.

## 12. Open questions and what in the brief I think is wrong

1. **Device time on `Done`** (decision 1): is S allowed to read the instance time source at `Done`? The alternative is an `Input::Clock` fed by uhd-clock with a cadence nobody has defined; without either, K frees only by capacity timers (a delay of one timer wake-up per value in the bursty measurement). I recommend `Done`.
2. **B9 / cell removal** (§2): confirm "a consumed value implies device time >= e", so removal at `Done` needs no K record outside the cell.
3. **B8's "or reserves"**: a due capacity timer legitimately reserves a held value when K has freed by time; I read B8 as "no reservation that K, D and I2 would not permit, and no stop, restart or drain change".
4. `Done(Call(Lost))` is not counted in K. Confirm.
5. `Brain.capacity_timer` as timer bookkeeping versus a derived dedupe.
6. Does the panic guard record a loss? The design says it sets F and `mark_lost`; I model Record in the executors' publication only, so I18 is vacuous for the guard (the guard path stays under I1 and case 6).
7. The brief's H03 recipe ("if each transition closes acceptance, remove the guard, add the invariant") does not apply: `start_confirmed` is the counterexample (§4). Please confirm keeping the guard. E3 reported only the unit test as pinning the second guard; case 12b is the DST pin.
8. I17 and I19-I21 cannot fail on the unmutated model, so their power rests on the mutation rows being killed by a DST test and on hand-built violation units; the brief's phrase "new cases for B1-B10, each with witnesses" does not by itself give I17 a reachable witness except case 12b's `W12e`.
9. I add one progress oracle (P6) to I11′: without it a missing re-evaluation is invisible to DST (the E2b defect was liveness, not safety).
10. Size: over 3,000 lines (decision 5). If Sol wants a cut, the first candidates are case 14's Reduced variant, the spawn-failure threads test and the micro variants of case 13.
11. Spawn failure needs a start prefix in the world (`Start(k)`, `Spawn(k)`); I keep it to cases 15 so other cases' initial states are unchanged.
12. The one risk I cannot size before building: search states. I add the stopper, cleanup and timers as events; if a micro case exceeds `SMALL_BUDGET` I report counts and shrink the script, as §3.3 of the experiment note requires.

## 13. Amendments after Sol review (amended after Sol review; these override §§3, 4, 6, 7, 9 where they differ)

Scope of phase 2: E4a1 only (E4a2 and the rows E4-22 onwards wait).

- **Held order (amends §3):** global head-of-line. `ReserveAll` walks `held` once in release order and stops at the first value that K, D or a reservation condition refuses. P6 (§6) checks the head only, after the ops of every step end (at the step, not at terminal states): no state where the held head is reservable. A witness shows the head is reserved after the freeing input.
- **H03 case (amends §4 and case 12b):** the separating sequence is Sol's pinned drain: `Streaming(origin 0, progress 8); A Taken; Stop(8); Release(B, e = 4)`; the pin keeps the drain open and the cell `Streaming`, so only the `progress >= cut` guard refuses B (witness `W12b`, `Refusal::Progress`). Row E4-18 (delete the guard) must be killed by this DST case. `start_confirmed` is not the witness (it fails `Cut`/`Origin` first).
- **Behavioural cases (amends §9):** cases 10 and 11 use small explicit K/D in every variant (case 10: K = 2, D = 3, six releases; case 11: K = 1) and every declared witness must be reached; the test asserts the witness list, not only `assert_bounded`.
- **Loss (amends §2, §3):** on `Lost` the values are reported but stay resident until their terminal input (`Done`/`Abandoned`) arrives; unit tests cover a `Done` delayed past the loss report. K does not count `Done(Lost)` or values of a lost cell once their call returned; a lost cell's `Taken`/`MaybeApplied` still count (the call may be running).
- **Time source (amends §1):** forced: `ModelDevice::now`; fake: `FakeDevice::time_now` via `sh.now`; measured: host-clock extrapolation (`sh.now`). All three are the closure `sh.now` already used by the timer thread. Stale (decreasing) time inputs never lower `brain.now`.
- **B8 (amends §7):** includes the F-set window: capacity timer delivered after `F` but before `Boundary::Lost`: no reservation, no command.
- **Extra tests:** a lifecycle request (`Request::Restore`) while K frees but the call is pinned: it stays deferred; decreasing time input; two distinct completed future instants for the timer min; explicit Dropped and Refused rows.
- **I17 (amends §6):** `RxCall::Begin` carries the start's epoch from `RxOp::Start` (`call_of`); the monitor's `starts` is checked against that call epoch.
- **B15 (amends §5):** boundaries-first is the experiment's chosen semantics (the thread wrapper's existing order), not production's: production's LateCommand branch does not publish (`provider/rx.rs:623-673`); `CutReached` is sent after the publication closed acceptance.

## 14. What E4a1 added beyond §§1-13 (extension after implementation; every new state or path)

- **Brain state:** `device_bound`, `chain_bound` (from `Params.k`, `Params.d`), `now` (max of the times inputs carry), `capacity_timer` (timer bookkeeping). `Input::Done { now }`, `Input::Timer { now }`, `Input::CapacityTimer { now }`, `Command::CapacityTimer`, `core::Timer { Drain, Capacity }`, `Section::ReserveAll`, `Section::CapacityTimer`. `Release` queues `Reserve { id: head }` (a pass starts at the head); a reservation queues `ReserveAll` only if values remain held; every section but `Reserve`, `ReserveAll`, `Decide`, `Shutdown` is followed by one re-evaluation (`reevaluate`, deduplicated).
- **Retirement rules the DST found (seed 6242 of case 10):** a consumption removes a value from the cell only once the brain has taken its `Done` (`has_result`), whatever the cell's state says (S writes the cell before the `Done` input travels); after a loss report a value stays resident until its `Done`, and `done` then removes it (with `Reported(Loss)` already written). Fixtures that put a `Done(..)` flight into a cell directly now also record the result in the brain.
- **Monitor facts (all in the key):** `instants`, `streams`, `cancelled`, `done_processed`, `starts`, `k_refused`, `last_input`. Oracles: I11′ (K and D from those), P6 (the held head, at quiet states), I17 (from `Stopping`, with the call's epoch carried by `RxCall::Begin { at, epoch }`), step check `capacity_timer_inert` (B8). Witnesses: W10a (refused at K), W10b (reserved after a K refusal), W10c (refused at D), W10d (after a capacity timer), W11b (after a Stop), W12a/W12p/W12c (capacity timer delivered while draining / after an epoch advance / after `F`), W12b (progress-at-cut refusal in a phase that accepts).
- **Replay:** `rules::recv_inputs`, `RecvCall.late`; the shrink display caps at 10 and says so; `s1b` classifies a Q1 divergence (the pinned finding) like `s1c` instead of failing.
- **B8 and F:** the brain never reads `F`; a capacity timer delivered after `F` and before `Boundary::Lost` may reserve a value, and `S` abandons it (I1, P2, case 12a's W12c). "No reservation once F is set" is not a brain rule; the cell refuses after the loss boundary (unit `b8c`).
- **Tests added:** `bounds_tests.rs` (B1-B10 units, b2b-b2e, b3c/b3d, b5b head of line, b7b loss), oracle units, world `harness_done_carries_the_device_time_of_its_send`, threads `b2e_*`, `b15_*`, `s15e_*`, cases 10/11/12a/12b micro (`reaching_pair!`: a declared witness must be reached by the search or 300 walks), walk sweeps for them, and the default `bounds_cases_walk_clean` (500 seeds of cases 10, 11, 12a, 12b, 5, 7c) so rows can be killed by walks.
- **Rows:** E4-01..E4-39 appended; retargeted E1-19, E1-42, E1-51, E1-63, E1-87, E2-77, E2-78, E2-79, E2-88, E2-94, E2-99, E2-100, E2-105; removed E2-66 (the single-chain retry is now one rule for every freeing input, E4-09..12).
- **Design text made false:** `plan/phase8/45-experiment.md` §3.4 defaults ("in-flight bound 4"), §5 I11, §4 case text "in-flight bound" (the orchestrator amends; I did not touch the file); note 45 itself is consistent with the code.

### 14a. Self-review round (DeepSeek v4.1 flash free) dispositions
- Case 10 is K = 1, D = 2 with three values (not §13's K = 2, D = 3 and six): the same two ceilings are reached with a far smaller search; `S.effect` is ok or partial (the log shows `SEffect(Partial)`; refused needs a full DSP FIFO, covered by the unit rows). Adopted as a record. Witness names are the monitor's fact names (W10a-d, W11b, W12a/p/c/b), not §9's.
- Row ids differ from §11's numbering (plan row 13 "capacity timer clears drain" is covered by E4-16 and b8a; rows 30-33 are the DST kills; 34-39 oracle self-tests). Adopted as a record.
- "Design text made false" pointer: the stale case text is 45-experiment §6 U14, besides §3.4 and §5 I11.
- `timer_input` duplicated in the forced replay: fixed (one function; E4-25 and E4-26 kill its mapping by different tests).
- `is_bound_hit`: fixed to read K and D by `reason_index`; the single `bound_rejections` counter stays K plus D until E4b splits its counters.
- Q1-class tolerance in `s1b`/`s1c`: kept (the pinned finding; the count depends on the seeds, so it is printed, not asserted). Rejected the count assertion.
- `bounds_cases_walk_clean`: 7.6 s in a debug build locally (3,000 walks); recorded.

## 15. E4a1 fix round (Sol's code review: three P2 and the replay recommendation)

1. **End-now cancel frees K (P2-1).** `Section::Decide` is no longer excluded from the re-evaluation: a request's decision can send a stop, and `sent` cancels the queued values. Test `b4b_an_end_now_cancel_frees_k_and_reserves_the_other_chains_value_in_that_step` (two streams, `Request::Late`; failed before the fix, `left [1] right [1, 2]`). Row E4-40 (Decide excluded again).
2. **P6 window (P2-2).** P6 is also checked at a step's end (`p6_step`: a `BOp` after which the step's operations are done), before the next input is taken. At that point the queued inbox may hold anything but a boundary or a capacity timer (a boundary reports a cell change the brain has not looked at; a capacity timer delivers the time awaited), so a queued `Shutdown` no longer hides an omitted retry. D for P6 counts a consumption only once the brain took it (`d_known`); I11′ keeps its independent count. Test `inv_p6_the_step_that_frees_k_reserves_the_held_head_before_a_queued_shutdown` (`inbox = [Done(A, now >= e_A), Shutdown]`: the real step reserves B before the Shutdown; the omitted-retry state is the violation; a queued boundary hides it). Rows E4-41 (retry omitted after a Done, killed by this test), E4-42 (the step check off).
3. **B8 after F (P2-3).** The reading stands (the brain does not read `F`; executor admission enforces I1; `45-experiment.md` §11 B8 amended). Test `b8e_a_capacity_timer_after_f_reserves_and_s_abandons_with_no_device_call`: B held behind A, loss delivered and `F` set with the loss boundary not yet taken, the capacity timer reserves B, S takes it and its check sees `F` (no `Apply` call), B is `Done(Abandoned)` and still resident until the brain takes the terminal, then retired with `Reported(Loss)`; every transition and state passes the oracles, and the thread run agrees on the whole sequence. It passed at once (no code change needed); no new row (E4-09/E4-31 cover the reservation path, I1's rows the abandonment).
4. **Replay: acceptance at the call's return (recommendation, adopted).** Both wrappers record a timed value's acceptance where the call returns, with the same timestamp: the world at `S.unlock` (`s_accept`, `device.t`), the threads right after `call_settings` (`(sh.now)()`); the device's application instant (`Apply::Accepted`) stays the device's fact. Removed: `Quirk::AcceptAtEffective`, `Quirk::AcceptInEffect`, `ModelDevice::accept_hook`, `Shared::effective`, `Sources::effective`, `q1_class`, the Q1 tolerance in `s1b` and `s1c` (any divergence fails), and `finding_q1_*` (replaced by `acceptance_is_recorded_at_the_calls_return_in_both_wrappers`, which pins the single rule on both wrappers). Rows E4-43..E4-45 and the retargeted E2-54; oracle I4's evidence check is `application sample <= evidence sample` (the host credits a later sample when the return is after the application; never an earlier one), test `inv_i4_evidence_after_the_application_is_legal_and_before_it_is_not`, self-test rows E4-46, E4-47.
   Consequence found by the DST: with the return time as the acceptance, a value released out of instant order (queued behind a later instant, whose effective instant is later than the return) is credited before the device applied it. Production releases in instant order (note 38), so the cases do: the seeded configuration chains its four releases (`after`), case 11 releases A (instant 8) before B (instant 9, on the stream without a cut), case 12a releases A before B. Case 12b keeps B (instant 4) behind A (instant 12) by design: B is refused. `oracle_happy_path_passes` is complete at 302,471 states (budget 400,000).

### 15a. Second fix round (Sol's re-review, P2: P6's queue exemption)
- The blanket exemption (a queued boundary or capacity timer) is gone. `p6_step` checks at the instant a brain step's operations finish (a `BOp` that empties them, before any other event), with no condition on the inbox, against the state the step saw: the cell's own reservation conditions, K counted at the brain's time (`k_load_at(brain.now)`), D as the brain knows it (`d_known`). Queued inputs are not excuses: the brain has not taken them. The state-level `p6` keeps the true-time K and the due-timer exemption (it checks liveness of the timers), under its strict quiet conditions.
- **One exact exemption**, found by the DST at seeded seed 255 (reported, not hidden): an executor took a step between the brain's operations of the step (`monitor.interleaved`, reset at `BStep`, part of the key only while operations are queued). In that run the head was refused at K while S's value was `Taken`; S's `SCheck` then wrote `Done(Abandoned)` before the step's last operation (a `Send`), so the cells at the end were not the cells the retry decided on. That change reaches the brain as its own `Done`, whose step owes the retry. Without an interleaved executor step the check has no excuse.
- Test `inv_p6_the_step_that_frees_k_reserves_the_held_head_whatever_is_queued_behind_it`: three tails behind `[Done(A, now >= e_A), Shutdown]` (nothing, a boundary, a capacity timer); the real step reserves B first; the omitted-retry state is a violation with every tail; an interleaved step and a brain time before A's instant are not. Rows E4-41, E4-42 retargeted to it; E4-48 (an inbox exemption returns), E4-49 (K at the true time), E4-50 (no interleaving exemption) added; E4-39 retargeted to `k_load_at`.
- `oracle_happy_path_passes` is complete at 414,041 states (budget 800,000; about 40 s in debug).
- Case 12a's description corrected (the cut is above both instants; the drain stays open).

### 15b. Third fix round (Sol's narrow re-check: the interleaving exemption was not exact)
- `p6_step` and the interleaving flag are gone (E4-48 and E4-50 removed with them; E4-42, E4-49 retargeted). Two checks that need no exemption replace them:
  - **P6a (in the section):** at the `Reserve` event of the held head (the one section that tries held values; `ReserveAll` only queues it), evaluated on the state at that very event (`before`): the cell's reservation conditions, K counted at the brain's time (`k_load_at(brain.now)`), D as the brain knows it. If the head is reservable, the section must have reserved it. One atomic event, so no executor step can matter. Test `inv_p6a_a_reservation_section_reserves_a_reservable_head` (skipped head is the violation; K full owes nothing; K at the brain's time, not the true time). Rows E4-51 (a section skips a reservable head while another value is held; killed by `bounds_cases_walk_clean`), E4-52 (check off), E4-49.
  - **P6b (the obligation):** the monitor records per brain step whether its input is in the freeing set and whether a reservation section (`Reserve` or `ReserveAll`) ran in it (`step_freeing`, `step_retried`; in the key only while operations are queued). When the step's operations are done, a freeing step that ends with values held and ran no reservation section is the violation. The freeing set is every input but a release (which tries the head itself), the shutdown and a panic: `Done` (every terminal), `Stop`, `PlanCancel`, `Request` (the `Decide` end-now path included), the boundaries `Consumed`, `CutReached`, `Finished`, `FirstSample`, `Armed`, `Quiesced`, `Lost`, `Flushed`, `BeginFailed`, `Opened`, `Closed`, `Timer` (drain), `CapacityTimer`, `LateCommand`; these are exactly the sections after which `section()` queues the re-evaluation, so no other freeing path exists in the code. Test `inv_p6b_the_step_that_frees_k_reserves_the_held_head_whatever_is_queued_behind_it`: the real Done step with the three tails behind `Shutdown` (nothing, a boundary, a capacity timer), each also with Sol's `RCheck` executor step right after the `BStep`; the omitted-retry record is the violation in all of them; a non-freeing step or an empty held list owes nothing. Rows E4-41 (retry omitted after a Done), E4-42, E4-53 (the step record never notes the section), E4-54 (an inbox exemption returns).
- The state-level `p6` stays (true-time K, due-timer exemption: the liveness of the timers). `oracle_happy_path_passes` is complete at 304,486 states (budget 400,000).
- No P6a or P6b failure in any sweep (s1c at 1–10,000, the 17 walk sweeps), seed 255 included.

### 15c. Fourth fix round (P6b credits only a `Reserve` of the held head)
`world.rs` notes `step_retried` only when a `Reserve` op runs whose id is the head of `held` at that event; a `ReserveAll` alone no longer credits the step, so a `ReserveAll` that queues no `Reserve` leaves the freeing step with values held and no retry (P6b). Negative regression: row E4-55 removes the queuing in `ReserveAll` and is killed by `inv_p6b_the_step_that_frees_k_reserves_the_held_head_whatever_is_queued_behind_it` (the real Done step, all tails and the `RCheck` variants). Logs: /tmp/e4a1i-s1c-a.log, /tmp/e4a1i-s1c-b.log, /tmp/e4a1i-walks.log, /tmp/e4a1i-probe.log.
