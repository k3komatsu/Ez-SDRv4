> plan · gpt-6.1-sol
## Frozen holdout set: H01–H20

These are **behavioural mutations**, not disabled assertions. This response freezes the selection before implementation; no files were changed.

**Basis:** all of [note 45](../45-uhd-decision-core.md), [note 44 §3](../44-device-chain-state.md), [note 39 §6.6–§6.8](../39-dsp-timed-commands.md), and [spec 18 UR-24](../../../design/18-uhd-radio.md). Where note 45 explicitly changes existing semantics—especially late completion and partial failure—the experiment’s proposed semantics govern.

**Notation:** `O` = segment origin; `C` = logical cut; `P` = first-undelivered sample; `e` = command instant; `D` = drain deadline. “Case N” means note 45 §7’s exhaustive case N. Exposure mappings identify required schedules and inputs, not a claim that the cases already specify them.

### H01 — Procedure 1: reservation conditions checked outside the cell

- **Mutation:** Validate phase, channel, generation, origin, cut and progress against a cell snapshot, then insert the reservation without revalidating inside the committing section.
- **Minimal sequence:** Read eligibility for `(channel A, generation 7, O=0, C=100)`, with `e=50`. Pause before insertion. Retire that segment; move/reopen as `(channel B, generation 8, O=200, C=300)`. Resume the stale reservation; settings takes it.
- **Wrong outcome:** A command qualified for the old chain is accepted into the replacement. **Reservation identity and coverage** are broken: the current channel, generation and origin do not qualify.
- **Exposure:** Seeded search with replacement/channel move. None of the six explicitly requires a reservation spanning replacement; case 3 would need this extra operation.

### H02 — Procedure 1: inclusive upper cut

- **Mutation:** Accept `e ≤ C` instead of `e < C`; leave every other condition unchanged.
- **Minimal sequence:** `Armed(O=0,C=100), P<100; reserve(e=100); take; apply`.
- **Wrong outcome:** A timed value is handed to a segment that does not cover its instant. **Half-open segment coverage** is broken.
- **Exposure:** Case 1, with the value exactly at the cut; also seeded boundary inputs.

### H03 — Procedure 1: exhausted executor still qualifies

- **Mutation:** Accept `P ≤ C` instead of `P < C`.
- **Minimal sequence:** Publish through the last permitted sample, advancing `P` to `C`; pause before the executor’s Stopping transition. Reserve an otherwise qualifying value with `O ≤ e < C`; settings takes it.
- **Wrong outcome:** New timed work enters after the executor has exhausted its publishing authority. **No reservation after executor exhaustion** is broken, potentially adding a new drain obligation at termination.
- **Exposure:** Case 1, scheduling reservation between progress advancement and Stopping.

### H04 — Procedure 1: take uses a stale queued-state check

- **Mutation:** Check that a command is queued outside the cell; later mark it taken without atomically checking that it remains queued.
- **Minimal sequence:** Settings observes `queued`; RX atomically enters Stopping and cancels that command; settings resumes, changes it to taken and applies it.
- **Wrong outcome:** A cancelled command reaches the device. **Exclusive take-or-cancel** is broken.
- **Exposure:** Cases 1 or 4, with cancellation between the settings executor’s check and take.

### H05 — Stopping: an old first-sample event reopens acceptance

- **Mutation:** Let a delayed first-sample notification change `Stopping(s)` to `Streaming(s)`, rather than restricting that transition to the appropriate Armed state.
- **Minimal sequence:** Start segment `s`; delay its first-sample notification. RX decides to stop and enters Stopping. Deliver the delayed notification; reserve and take another value.
- **Wrong outcome:** Acceptance reopens without a newly confirmed start. **Stopping is closed until an authorized start** is broken.
- **Exposure:** Case 4 if it includes a delayed first-sample notification; otherwise seeded search. The six cases do not explicitly require that notification.

### H06 — Procedure 2: Stop reads progress before its committing section

- **Mutation:** Compute Stop’s progress floor from a snapshot taken outside the cell; write the shortened cut without rereading progress.
- **Minimal sequence:** Stop snapshots `P=80` for a requested cut of 85. RX publishes samples through 89 and advances `P=90`. Stop commits `C=85` using the old snapshot.
- **Wrong outcome:** Samples already published lie at or beyond the newly committed cut. **Stop never cuts behind published progress** is broken.
- **Exposure:** Cases 1 or 3, interleaving publication with Stop’s snapshot and commit.

### H07 — Procedure 3: epoch observation substitutes for quiescence

- **Mutation:** Acknowledge the new epoch when a cancellation notification is observed, even though a receive call begun under the retiring epoch has not returned.
- **Minimal sequence:** `recv(old epoch)` is outstanding. Brain advances the epoch. Executor acknowledges immediately. Brain issues close/open or channel move; the old receive call then returns.
- **Wrong outcome:** Lifecycle work proceeds while an old call still uses the retiring streamer or chain. **Quiescence before dependent lifecycle operations** is broken.
- **Exposure:** Case 3, with replacement waiting on the epoch acknowledgement.

### H08 — Procedure 3: stale buffer uses its original cut

- **Mutation:** Clip an old-epoch buffer using the cut captured when `recv` began, rather than the retiring epoch’s latest cut.
- **Minimal sequence:** Begin receive with `C=100`. Stop shortens the retiring epoch’s cut to 60; advance epoch. Receive returns samples `[50,90)`.
- **Wrong outcome:** Samples `[60,90)` are published instead of discarded and counted. **No publication beyond the latest retiring cut** is broken.
- **Exposure:** Case 3 directly.

### H09 — Procedure 3: publication and progress advancement are split

- **Mutation:** Commit external publication and `P` advancement in separate cell sections, publishing first.
- **Minimal sequence:** With `P=50`, publish `[50,70)`; pause before advancing `P`. Stop reads 50 and commits `C=55`. Resume the progress update.
- **Wrong outcome:** Publication has escaped the floor used by Stop; samples at or beyond 55 were already delivered. **Atomic publication/progress accounting** is broken.
- **Exposure:** Cases 1 or 3, with Stop between the two sections. This differs from H06: Stop itself uses a correctly locked read.

### H10 — Procedure 4: loss is recorded before the failure flag is raised

- **Mutation:** Perform loss recording or enqueue its record before publishing the instance failure flag.
- **Minimal sequence:** One participant detects loss and pauses during recording, before setting the flag. Another executor checks the still-clear flag, accepts queued device work and starts it. Loss publication then finishes.
- **Wrong outcome:** Preventable device work is admitted after loss was detected because recording delayed the safety signal. **Failure publication precedes recording** is broken.
- **Exposure:** Case 2, explicitly separating loss detection, flag publication and recording. Do not confuse this with the permitted check-to-call window: the executor’s check here occurs after detection but before the wrongly delayed flag.

### H11 — Procedure 4: autonomous loop checks failure only on entry

- **Mutation:** Check the failure flag when entering a receive/drain loop, but omit the per-iteration check.
- **Minimal sequence:** Enter a multi-iteration drain with the flag clear. First call returns. Another participant sets the flag. The loop begins its next receive or flush call.
- **Wrong outcome:** An executor starts another device call despite a failure flag already visible before that iteration. **No new autonomous work after observed failure** is broken.
- **Exposure:** Cases 2 and 6, with failure between iterations.

### H12 — Procedure 4: panic guard omits native loss publication

- **Mutation:** On brain panic, set the instance failure flag but omit native `mark_lost`.
- **Minimal sequence:** Open a streamer; panic the brain; its guard sets only the host-side flag. Executors stop. Cleanup drops native ownership.
- **Wrong outcome:** Native teardown sees no loss indication and releases handles that must be retained. **Loss-safe handle retention**, including panic, is broken.
- **Exposure:** Case 6, extended through cleanup and the native destruction/retention decision.

### H13 — Pinning: only maybe-applied commands pin the chain

- **Mutation:** Exclude the taken state from the pin predicate; pin only once the device call has begun.
- **Minimal sequence:** Settings atomically takes a command, then pauses before calling the device. Brain processes replacement and issues restore, channel move, close/open or next-segment start. Settings resumes its old command.
- **Wrong outcome:** A non-cancellable old command can apply across conflicting lifecycle work. **Taken-or-maybe-applied pins every conflicting operation** is broken.
- **Exposure:** Seeded search; case 3 can expose it if a taken command is included. Exercise the conflicting operation kinds as separate branches of this same mutation.

### H14 — Result write and five command states: finish is mistaken for settlement

- **Mutation:** When the executor writes a returned result into the cell, mark the command settled rather than finished. Keep the brain’s normal “already settled” handling unchanged.
- **Minimal sequence:** Reserve, take and apply a value. Executor writes its successful result as settled; deliver `Done` to the brain. The brain skips its normal settlement.
- **Wrong outcome:** The device effect exists, but the brain never performs the corresponding record/chain update. **Every completed command is settled exactly once by the brain** is broken—here, zero times.
- **Exposure:** Cases 1 or 2 with completion delivered through to settlement; seeded search.

### H15 — Partial failure is treated as a clean rejection

- **Mutation:** Collapse “setting may have applied, then clear-command-time/read-back failed” into ordinary no-effect failure, removing its residual obligation.
- **Minimal sequence:** Take a timed value. Device accepts the frequency change; a later operation in that command fails. Return partial failure. Stop the chain before qualifying samples consume the value.
- **Wrong outcome:** The core reports a clean rejection and forgets a value that may remain in the DDC; restore or later chain reuse proceeds without accounting for it. **Unknown device effect remains an obligation** is broken.
- **Exposure:** Case 2 only if partial non-loss outcomes are included; otherwise seeded fault search. Loss alone does not cover this mutation.

### H16 — Drain deadline: expiry cancels a taken command

- **Mutation:** At `D`, discard/cancel taken or maybe-applied commands along with queued commands, clearing their pin and residual tracking.
- **Minimal sequence:** Settings takes a command and remains delayed beyond `D`. RX reaches the deadline, stops when able and reports the remaining value. The mutated core deletes the command. Settings later completes successfully.
- **Wrong outcome:** Late completion cannot be reconciled with the reported residual; the value can affect a stopped or reused chain without persistent `left` accounting. **Deadline expiry does not revoke taken work; late completion remains accountable exactly once** is broken.
- **Exposure:** Case 5, including eventual completion and subsequent chain reuse—not merely reaching the deadline.

### H17 — Native lock: timeout cleanup bypasses executor termination

- **Mutation:** Let shutdown timeout authorize ordinary handle closure even while an executor remains blocked on or inside the native control lock, instead of choosing handle retention.
- **Minimal sequence:** Settings holds the native lock while blocked in UHD. RX stop waits for it. Shutdown’s join bound expires. Cleanup closes/drops handles while those executors remain live.
- **Wrong outcome:** Live native operations outlast the resources cleanup releases. **Cleanup requires stopped executors or retained handles** is broken. The correct design does **not** promise deadline-bounded stop while the lock remains unavailable.
- **Exposure:** Case 5, extended through shutdown timeout and cleanup; seeded search.

### H18 — Bounded in-flight values: full cell consumes the held head

- **Mutation:** When the cell is full, advance/remove the head of the held queue as if reservation succeeded, without creating a cell reservation.
- **Minimal sequence:** Fill a chain’s in-flight capacity with delayed commands. Present one more eligible held value. It encounters the full cell and disappears. Complete existing work; retry release.
- **Wrong outcome:** The missing value is neither handed over nor left available for later release or end settlement. **Every held value remains accounted for; capacity is backpressure, not loss** is broken.
- **Exposure:** None of the six necessarily fills capacity. Seeded search exposes it only if its three values exceed the chosen capacity. Required additional case: **capacity plus one**, followed by freeing capacity and retrying.

### H19 — Consumption: use receive-call start time instead of device acceptance

- **Mutation:** Determine a value’s eligible sample threshold using the start of its device call rather than the device’s actual acceptance time.
- **Minimal sequence:** Command `e=10` starts at time 8, but device acceptance is delayed until 20. After the call returns, observe a buffered sample timestamped 15. Then cut before any sample at or after 20.
- **Wrong outcome:** The sample falsely consumes the obligation; the real unconsumed device value is omitted from drain/`left` reporting. **Consumption requires a post-return observation at or after both `e` and acceptance time** is broken.
- **Exposure:** Case 5 if it supplies such a buffered sample after delayed acceptance; otherwise seeded search. A queue delay without sample timestamps straddling acceptance is insufficient.

### H20 — Consumption: one sample consumes the whole eligible prefix

- **Mutation:** For each eligible sample, remove every eligible queued value on that channel rather than at most one.
- **Minimal sequence:** Successfully hand over two equal-time values on one channel. Observe exactly one qualifying sample after both calls return. Stop with no further qualifying sample.
- **Wrong outcome:** Both values are declared consumed, although the second still requires a distinct sample; its residual report is missing. **One value per eligible sample, including equal-time values**, is broken.
- **Exposure:** Case 4 directly, provided it supplies only one qualifying sample before termination and checks the remaining obligation.

### Weak coverage in the six cases

The six cases are strong on scheduling races but leave important **input and lifecycle dimensions** implicit: exact origin/cut/progress boundaries; phase, channel and generation changes between snapshot and reservation; delayed first-sample notifications; all five pin-conflicting operations; partial non-loss failure; capacity-plus-one and held-head ordering across streams; panic followed by native teardown; and late completion followed by reuse or channel move, including persistent `left` blocking restore on another chain. Consumption also needs deliberately chosen sample timestamps around device acceptance, not just delayed calls. Seeded search is not guaranteed to supply these witnesses: three values need not fill the configured capacity, and arbitrary timestamps rarely hit equality boundaries.

**Not checked:** No `crates/` files, implementation, tests or hardware were inspected. Exposure mappings remain requirements for the future DST plan, not measured detection results.
