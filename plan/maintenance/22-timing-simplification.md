# Maintenance spec 22 — One stream timeline, and shorter specs

| Field | Value |
|---|---|
| Status | Accepted by the owner on 2026-10-07, every item as recommended (*Owner decisions*), after three adversarial review rounds (final PASS) and a check of the final trim; not yet implemented. Each step's `design/` text reaches `design/` in the commit that implements it, in the *Implementation order*. It supersedes spec 21's VG-2 and KI-1, which were accepted but never implemented (VH-6's table); spec 21's VG-1 stays as implemented. Drafted by a Claude Opus subagent (*Draft history*, near the end). |
| Scope | **Part A**: the radio streams' timing model (VH-1…VH-8), and the Kernel text it touches (KJ-1). **Part B**: the format, rollout and map of the documents (DA-1…DA-5). This file is the pilot of DA-1: rationale stays here, and Appendix B holds the `design/` text the implementation commits. |
| Amends | `design/07`, `09`, `18` (stream timing; the format); `design/01` TM-13c, `05` UC-3, `06` §10 (KJ-1, DA-5); later `design/08` and `11` (DA-5); `AGENTS.md` §2 (DA-1); Vision §13, §14 and §59 (*Vision issues*). |
| Versions | `radio` 2.0.0 (two envelope keys removed, the verb `start_rx` added); `ezsdr.radio.mock` 2.0.0 with profiles 2.0.0; `ezsdr.radio.uhd` 0.4.0 with profiles 0.3.0; package `ezsdr` 0.4.0; `schemas/radio/envelope.v1.json` and five Kernel schema descriptions regenerated, still v1 (pre-freeze, OV-12). Old bindings and profiles are refused (MR-2, UR-5), never reinterpreted (invariant 39). |

**How to decide.** One item at a time, in the order of *Owner questions* (end of file). Each item has one recommendation and its rejected alternatives, judged on long-term merit only (AGENTS.md §6). VH-1, VH-3 and VH-4 form the model, and the others build on it. If one of those three is rejected, the items that build on it fall back to their own alternatives. The DA items stand alone.

**Evidence.** A claim is VERIFIED when it was read in the tree at `7b6f376` or observed in a probe. It is INFERRED otherwise. The probes were a standalone prototype of VH-7 (Appendix C: 11 tests, 4 of 5 mutations killed, the fifth equivalent and removed) and counts by script. No repo copy was built: no claim here needs the crates' tests. The prototype does not model the ready term or VH-2's replanning; those parts are INFERRED until VH-7 lands.

## Owner decisions

| date | item | decision | reason |
|---|---|---|---|
| 2026-10-07 | VH-1 | Accepted as recommended: a receive stream is on or off; a `Stop`, `Provider::stop` or a loss ends the running segment at its cut and ends its SampleClock there; the next start opens a new clock; a transmit `Stop` ends bursts, not the transmit clock. | Owner accepted after the review's PASS, having checked it against USRP behaviour: the X300 stops where the stop is issued, a restart is a new stream on the same device time (the root, GPS/PPS-disciplined), and only the per-stream SampleClock is cut. |
| 2026-10-07 | VH-3 | Accepted as recommended: no stop tail; a `Stop` cuts at the instant the Provider handles it (an abort at the first sample not yet delivered), every cut floored at the first sample not yet delivered; `radio.timing.stop_tail_ns` is removed and UR-25's call-in-progress term moves into VH-4's ready instant. | Owner: the tail only made the model needlessly hard; the device stops where the stop is issued. |
| 2026-10-07 | VH-4 | Accepted as recommended: one start rule — a segment starts at the first lattice instant at or after its effective instant, the previous cut plus `start_lead_ns` and the Provider's ready instant plus `start_lead_ns`; `restart_lead_ns` is removed; the UHD Module's enable and switch become one path; the ready instant carries the call-in-progress term, which `x310-like` models. | Owner accepted after the review's PASS: one path for #41, #49 and #58, and the switch keeps the full lead. |
| 2026-10-07 | VH-2 | Accepted as recommended: both Providers plan an event on receipt and record only what has happened (a receive clock registered when its segment begins, a transmit clock at booking); a planned event that fails at its instant leaves what is recorded standing and recomputes only the events after it; a refused change or start halts the stream until its next `cold` change or `start_rx`; the read-back refusal is a MockRadio *Ceiling* and follow-up 4. | Owner accepted after the review's PASS. |
| 2026-10-07 | VH-6 | Accepted as recommended: spec 21's `start_rx` carries over (`Rx.start()` sends the current instant, no lateness, order by instant then arrival); a `Stop` cancels no update, only `Provider::stop` drops what is pending; when a `Stop` cancels a segment whose timed start is already queued, uhd-rx stops untimed only after that origin and discards what arrives (#56); spec 21's guidance becomes a note in RM-21. Spec 21's start timing and device-`Stop` decisions change accordingly. | Owner accepted after the review's PASS, after the queued-start case was explained. |
| 2026-10-07 | VH-5 | Accepted as recommended: a fault's row is written when it fires (what it injected, or `applied: false`) and never revised; `RX_OVERFLOW` stays with the block that carries the loss; MR-20a is withdrawn. This reverts the meaning spec 20 VF-4 gave a fault row and answers #45 by definition. | Owner accepted after the review's PASS. |
| 2026-10-07 | VH-7 | Accepted as recommended: one pure, deterministic `ezsdr_radio::timeline` function in the `radio` Vocabulary crate computes segments (origin, cut, configuration) from a stream's commands and faults; MockRadio calls it in its step loop and uhd-control when booking. | Owner, emphatically: the rules must be implemented once. |
| 2026-10-07 | VH-8 | Accepted as recommended: a seeded differential harness in three deterministic layers (timeline against MockRadio; against uhd-control's booking on the `ManualTimeAuthority` rig; uhd-rx carrying out a plan), layers 1 and 2 landing in step 1; the FakeDevice end-to-end tests shrink to a smoke set. | Owner accepted after the review's PASS. |
| 2026-10-07 | KJ-1 | Accepted with VH-1, whose cost it is: UC-3 and TM-13c say a `cold` change restarts only a running function, with the `UpdateClass::Cold` doc comment and five schema descriptions; it replaces spec 21's KI-1. | Explained to the owner as part of VH-1. |
| 2026-10-07 | DA-1 | Accepted as recommended: `design/` holds normative rules only, each a few lines ending in a `*Test:*` line, with no rationale, history or issue numbers; a closing *Changes* table links the decision records; one home per topic; rule IDs never renumbered or reused; an amendment is a decision record whose `design/` text is reviewed as the implementing diff; the AGENTS.md §2 text lands with `rules.py`. | Owner accepted after the review's PASS. |
| 2026-10-07 | DA-2 | Accepted as recommended: Appendix B's pilot text is the `design/` text for the timing rules (RM-16/17/18/21/25, MR-18/20/20a/21/22/25, UR-17/25/26, UC-3, TM-13c), 2 427 words against 5 150, with no history markers. | Owner accepted after the review's PASS. |
| 2026-10-07 | DA-3 | Accepted as recommended: one commit per spec in the stated order, merges included; `rules.py` checks rule IDs, cited tests, markers, long lines, links and mutation spellings; one Opus review per spec marking every removed sentence is the gate; `rules.py --sentences` is an aid only. | Owner accepted after the review's PASS. |
| 2026-10-07 | DA-4 | Accepted as recommended: `plan/` stays where it is as the frozen record; AGENTS.md §2 gains "plan/ is history; design/ is current"; new amendments are DA-1 decision records linked from the *Changes* tables. | Owner accepted after the review's PASS. |
| 2026-10-07 | DA-5 | Accepted as recommended: no large merge; (a) UC-1…UC-6 live only in spec 05 §5, with a pointer in 06 §10; (b) spec 11 merges into 08, its CH-n IDs kept and 11 a stub; (c) the audit and re-review move to `design/archive/` with stubs, and AGENTS.md §2's reading order changes in the same commit. Rule IDs never change. | Owner accepted after the review's PASS. |

## Implementation order

Every step leaves the workspace building and its tests passing.

1. **VH-7 with VH-8's first two layers.** This step adds `ezsdr_radio::timeline` and its tests, and the generator with two comparisons: the timeline against MockRadio, and the timeline against the uhd-control rig. Both comparisons gate only the sequence classes where today's behaviour already equals the timeline, and print every other divergence, which lists today's issues mechanically. For receive `cold` changes that class is nearly empty: today's tail, `e` floor and restart lead all differ from the timeline. So step 1 guards transmit, `hardware_timed` updates and faults more than it guards receive changes. No behaviour changes, and no recorded mutation is touched.
2. **MockRadio and the UHD Module on the timeline, in one coordinated step.** This covers VH-1 to VH-6 on both Providers, KJ-1, the removal of the two envelope keys from the shared `TimingEnvelope` (`ezsdr-radio`), `radio` 2.0.0, both Modules at `^2.0.0`, the profiles, the Appendix B text, and every comparison class turned into a gate. It may be a short series of commits, but it merges as one: the UHD profile builds `TimingEnvelope` with both keys (VERIFIED, `radio-uhd/src/profile.rs:114-122`), so the Vocabulary and its two Modules cannot move apart. It closes #46–#50, #52, #53, #57 and #58.
3. **VH-8's third layer, uhd-rx carrying out a plan**, and the FakeDevice smoke set.
4. **DA-3's passes 2 onwards**, and the Vision issues.

**Re-spelling the recorded mutations, per step** (VERIFIED by count over the six `mutations.json` lists; the split is an estimate):
- **Step 1:** 0 rows.
- **Step 2:** 155 of the 446 rows name `ezsdr-mock-radio/src/lib.rs` or `ezsdr-radio-uhd/src/provider/{rx,control,tx}.rs`. Of these, 67 have `old` text that names a timeline concept (tail, cut, cold, switch, enable, settle, origin, lattice). Those 67 are the likely re-spells or retirements: 19 in the maintenance list, 14 in Phase 3 and 34 in Phase 7. The other 88 move only if their lines are rewritten.
- **Step 3:** 0 rows.

A retired row names the timeline test that replaces it.

---

# Part A — the timing model

**Problem.** MockRadio decides each event at its instant. The UHD Module books each event on receipt, in threads, and registers clocks ahead of time. The two implement one prose rule set spread over RM-16, RM-21, RM-25, MR-18, MR-20, MR-20a, MR-25, UR-17, UR-25, UR-26, UC-3 and TM-13c (5 150 words, Appendix A). Every corner case those rules leave open is decided differently by the two Providers, and each has become an issue: #46–#53, #57 and #58. Spec 21's fix would add a stopped state, a decision rule for starts and a Provider floor, at least 1 750 more words.

**The model.** Appendix B is the full rule text.
- **On or off.** A receive stream is on from the Run's start until a `Stop`, and again after `start_rx`. It runs while it is on, has a channel and a link, and is not halted.
- **Cuts.** Each run is a segment with its own SampleClock. A segment ends at its cut: the first sample at or after the instant of what ends it, never before its origin or the first sample not yet delivered. There is no tail.
- **One start rule.** A new segment starts at the first lattice instant at or after three instants: its effective instant, the previous cut plus the start lead, and the Provider's ready instant plus the start lead.
- **Order.** Events take effect by effective instant and, at one instant, by arrival.
- **Plan on receipt, record at the instant.** A receive clock is registered when its segment begins. A transmit clock is registered at booking. After a failure, what is recorded stands, the events that follow are recomputed, and a refusal halts the stream.
- **Faults.** A fault is recorded when it fires.
- **One implementation.** `ezsdr_radio::timeline` computes all of the above, and both Providers use it.

## VH-1 — A `Stop` ends the receive segment and its SampleClock

**Problem.** A `Stop` leaves the receive clock open. A later change can therefore restart the stream (#46), and `ended_at` means different things on the two Providers (#53).

**Decision.**
- A receive stream is on or off, and runs while it is on with a channel and a link.
- A `Stop`, `Provider::stop` or a loss ends the running segment at its cut, and the clock is ended there when the cut is made. The next start opens a new clock.
- A change to a stream that does not run only applies its value.
- A transmit `Stop` ends bursts, not the transmit clock, which bursts need (KC-24).

This replaces spec 21's stopped state and VG-1's separate sentence. It costs KJ-1.

**Rejected.**
- *Spec 21's stopped state, with the clock left open.* It is the root of #53 and of spec 21's five-term `e₁`.
- *A `Stop` that also ends the transmit clock.* A burst after `tx.stop()` would then have no clock to target.

**Evidence.** E8 and E9.

## VH-2 — Plan on receipt; record at the instant

**Problem.** MockRadio decides at the instant and the UHD Module at booking. They therefore disagree whenever a later event invalidates an earlier plan, such as a `Stop` before a planned origin or a device refusal (#49, #53).

**Decision.**
- **Plan on receipt.** Both Providers plan an event on receipt: `coerce` runs over the configuration projected to the event's instant, and the timeline gives the cuts and origins. The UHD Module drops its refusal of a second `cold` change, and its threads carry out a queue of plans.
- **Record only what has happened.** A receive clock is registered when its segment begins and ended when its cut is made. A transmit clock is registered, and its predecessor ended, at booking, so a burst admitted next can target it (KC-21a). No cut falls before a planned origin. MR-16's *Ceiling* goes: a Spec can now target a transmit clock that a scheduled change creates.
- **A planned event that fails at its instant.** This covers the UHD Module's UR-12 read-back refusal, a loss, and `Provider::stop`.
  - Cuts already made and clocks already ended stand. The event is removed, and only the events after its instant are recomputed. A later event the new plan refuses is refused then.
  - A refused change, or a refused start (`start_rx` or an enable whose configuration the device does not apply), **halts** the stream until its next `cold` change or `start_rx`. On receive there is no segment. On transmit, the clock registered for it ends at its origin, its held bursts are cancelled, and bursts are refused until the next change.
  - A loss or `Provider::stop` ends everything after it.
- **Empty clocks.** "No clock without a sample" holds for receive clocks only. A transmit clock can end at its origin, by one rule on both Providers.
- **The read-back refusal on MockRadio** is a *Ceiling* (MR-18). It is a device fault, which MockRadio models only when injected (Vision §17), and is filed as follow-up 4.

**Rejected.**
- *Decide everything at the instant.* A burst sent right after a rate change would land on the old transmit clock, and the device needs its stop and configuration before the new origin.
- *Register every clock at booking.* That keeps empty receive clocks.
- *Restart automatically after a refusal, at the configuration left.* Halting keeps the failure visible until the user acts. KC-27 holds the refused value however the stream resumes, so a silent restart would leave the user reading a configuration the stream does not have.
- *A `sim.faults` kind for the read-back refusal now.* Its rate is unmeasured, and it can be added later without breaking anything.

**Evidence.** E10 and E11. INFERRED: a `coerce` at booking gives the same verdict as one at the instant, under KC-17's order; where it does not, the failure rule applies.

## VH-3 — No stop tail

**Problem.** The orderly stop tail is a state in which other events can intervene. It produced #47, #50 and spec 21's start-in-tail case, and it is the UHD Module's own policy, not the device's (E1–E3).

**Decision.**
- A receive segment ends at its cut: the first sample at or after the instant the Provider handles the `Stop`. On the UHD Module that instant is uhd-control's booking. Under `abort`, the cut is the first sample not yet delivered.
- Every cut is floored at the first sample not yet delivered. MockRadio already does this (`.max(rx.next)` in `stop_rx`); uhd-rx takes the later of the two and ends the clock there.
- `radio.timing.stop_tail_ns` is removed.
- UR-25's call-in-progress term moves into VH-4's ready instant, because it protects the next timed start, not the cut.
- The tests that assert a tail are renamed (Appendix A).

**Rejected.**
- *Keep the tail.* It keeps four interacting cases alive, with no device reason.
- *A floor on every cut, the "earliest cut"* (draft 1). It is the tail under another name: about 5 ms at the defaults, and 171 ms at `block_len` 65 536 and 390.6 kS/s.
- *Keep the key with a new meaning.* That silently reinterprets a published key (invariant 39).

**Evidence.** E1–E3, E6 and E12. INFERRED: the delivered floor moves a cut by at most the Authority's 0.1 ms lag, which the ready term's 3 ms absorbs.

## VH-4 — One start rule and one lead

**Problem.** A restart and an enable from 0 channels follow two rules with two leads of equal value, and the UHD Module refuses a change back from 0 while the old stream drains (#41, #49, #58).

**Decision.** Every later segment starts at the first lattice instant at or after three instants: its effective instant, the previous cut plus `start_lead_ns`, and the Provider's ready instant plus `start_lead_ns`. `restart_lead_ns` is removed, and the UHD Module's `Enable` and `Switch` become one path. A change to 0 channels and straight back is no longer refused: it waits for the previous cut plus the lead.

The ready instant for a receive segment:

| Provider | ready instant |
|---|---|
| UHD Module | The later of the booking (or, for an enable from no stream, the end of uhd-control's configuration) and the previous cut plus one receive call at the old rate plus 3 ms. One receive call is the longer of `block_len` and `rx_packet_samples()`. |
| MockRadio, `x310-like` | The same formula, with the profile's `rx_packet_samples` of 1 996 (E6) and the same 3 ms, so MockRadio is never earlier than the UHD Module. |
| MockRadio, `ideal` | The receipt. |

A transmit segment's ready instant is the booking. VERIFIED, by arithmetic over UR-25's terms: the switch keeps the full 50 ms after uhd-rx can act, and an immediate change restarts at about booking + `d` + 53 ms, where `d` is one receive call, instead of + `d` + 103 ms.

**Rejected.**
- *Two leads with one formula.* No profile and no test tells them apart (E4).
- *Two mechanisms.* They are the source of #41 and #58.

**Evidence.** E4 to E6. INFERRED: Phase 8 may measure an enable lead well below the restart lead. One lead is then the stricter choice, which §59 allows, and a second key can return as an additive minor version.

## VH-5 — A fault is recorded when it fires

**Problem.** MR-20a settles a fault's row against its stream's end, so an end that later moves earlier leaves the row wrong (#47).

**Decision.**
- A fault's row is written when it fires: `lost = k_g − k_f`, or `applied: false` if the stream was not running. It is never revised.
- The stream's report is unchanged: `RX_OVERFLOW` comes with the block that carries the loss, or not at all (RM-17).
- MR-20a is withdrawn, and with it `settle_at_end`, `settle_ending` and their calls.

*For the owner:* this reverts the meaning spec 20's VF-4 gave a fault row. The row no longer says what the stream lost before its end; it says what the fault injected. #45 is answered by that definition. `ezsdr.radio.mock` 2.0.0 marks the change.

**Rejected.**
- *Settle again at every earlier end* (#47's own option). That adds a rule to fix a rule.
- *Emit the event at the fault.* That is looser than the hardware (E13).

## VH-6 — The resume: `start_rx`, and what a `Stop` does not cancel

**Problem.** After a `Stop` the stream needs an explicit way back (#46). Spec 21's start also carried a decision rule, waits and a floor, which VH-1, VH-2 and VH-4 make unnecessary.

**Decision.**
- Keep spec 21's `start_rx`: a `PeripheralCommand` on `<radio>/rx` with RS-19's `at`. `Rx.start()` sends the current instant, a start has no lateness, and order is by instant and then arrival.
- A `Stop` cancels no update. Only `Provider::stop` drops what is pending, which keeps KC-27 true.
- The timed start is issued once the configuration is done, as today. When a `Stop` cancels a segment whose timed start is already queued, uhd-rx issues its untimed stop only after that origin and discards what arrives. This is the reviewer's design: it answers #56 deterministically on the FakeDevice.
- Spec 21's guidance becomes a note in RM-21.

| spec 21 decision | verdict |
|---|---|
| VG-1, an end moves only earlier | **Survives** as a consequence of VH-1 and VH-3. Its code and tests stay, and its rule sentence goes |
| VG-2 and KI-1, a stopped stream is not restarted | **Survives**, simplified to on/off. KJ-1 replaces KI-1 |
| The start's form, `start_rx` | **Survives** unchanged |
| The start's timing | **Changes.** The order rule and `Rx.start()` at the current instant stay; the decision at `e`, the waits and the `e₁` formula go |
| A device `Stop` cancels `hardware_timed` updates | **Changes**: a `Stop` cancels no update |

**Rejected.**
- *Resume through 0 channels.* It is a configuration side effect, not a command.
- *A Kernel `Start`.* Rejected for spec 21's reasons.
- *Issue the start at its origin less 5 ms* (draft 2). That leaves 3 ms of host slack, so a late wake under load becomes a missed start.
- *Hold VH-4 until the bench case is run.* The X300 is away (handoff.md §4).

## VH-7 — `ezsdr_radio::timeline`

**Problem.** The rules are implemented twice, and that duplication is the defect class.

**Decision.** A pure, deterministic function in the `radio` Vocabulary crate.
- **Input:** a stream's initial state, the lead, the Provider's ready-term parameters, and its commands and faults. Each carries an effective instant, an arrival order and a ready instant, and may be marked refused or pre-empted.
- **Output:** segments, each with an origin, a cut and a configuration.

MockRadio calls it in its step loop, and uhd-control calls it when booking, handing the plans to uhd-rx and uhd-tx. This follows RM-26's precedent. No Module depends on another (Vision §7), and the Kernel gains nothing.

**Rejected.**
- *A shared state-machine object.* It is harder to drive from a discrete-event loop and from threads, and harder to test.

**Evidence.** E14 and Appendix C. INFERRED: the parts the prototype omits are a few lines each.

## VH-8 — A differential harness

**Problem.** Nothing compares the two Providers directly, and the FakeDevice end-to-end tests run on the wall clock and fail under load (E15).

**Decision.** A seeded generator produces short sequences: stop, `start_rx`, `cold` changes of rate and of count (including to and from 0), `hardware_timed` updates, faults and a loss, with ties and with orders across rounds. Each sequence runs in three deterministic layers:
1. the timeline against MockRadio;
2. the timeline against uhd-control's booking, on the `ManualTimeAuthority` rig (E16);
3. uhd-rx carrying out a given plan, on synthetic packets.

The comparison covers clock lists, blocks `(domain, first, len, flags, lost)`, `applied`, `rejected` and `faults`. Layers 1 and 2 land in step 1. The FakeDevice end-to-end tests shrink to a smoke set that asserts on the recorded plan. INFERRED: a FakeDevice on manual time with an idle handshake would remove the load sensitivity (follow-up 3).

**Rejected.**
- *More FakeDevice end-to-end tests.* They are wall-clock bound and fail under load (E15).
- *A model checker.* It is out of proportion for a 40-line function.

## KJ-1 — UC-3 and TM-13c

UC-3 restarts a function only if it was running, and the Vocabulary places the end and the new origin. TM-13c starts a new clock only for a running stream. The `UpdateClass::Cold` doc comment and five schema descriptions follow. This replaces KI-1, and no Kernel behaviour changes. The text is in Appendix B.

## Issues after this spec

| issue | verdict |
|---|---|
| #46 | **Closed** by VH-1 and VH-6 (VG-1 is in `029f48a`) |
| #47 | **Moot** (VH-5) |
| #48 | **Closed.** A `cold` change is ordered by its `e`, and at a tie a scheduled fault comes first (RM-25). This replaces the `(e₁, e, insertion)` loop order; only the record differs |
| #49 | **Closed**: (1) and (2) by VH-2 and VH-4, (3) by VH-4's ready term on `x310-like`, (4) and (5) by VH-7. The enable-from-0 residual is follow-up 5 |
| #50 | **Closed** by VH-3 |
| #51 | **Needs its own fix.** Transmit cancellation reporting is not a timing question; one rule should cover both Providers |
| #52 | **Closed in the model** (VH-1). The device-side residual goes to #56 |
| #53 | **Closed.** No receive clock is empty, and a transmit clock ends at its origin by one rule on both Providers (VH-2) |
| #54 | **Open**, unrelated (RS-14) |
| #55 | **Open**, for the freeze review |
| #56 | **Open, narrowed.** The FakeDevice case is deterministic (VH-6). The bench measures the queued start, the start lead, the ready term and the `Stop` handling lag |
| #57 | **Closed**: no cut before an origin, in one function |
| #58 | **Closed** by VH-4's single start path, which carries the stop guard |

## Follow-ups

1. #51.
2. Phase 8 measures `start_lead_ns` and the ready term. If the enable and the restart differ, a second lead can return.
3. A FakeDevice on manual time with an idle handshake (VH-8).
4. A fault kind for a device read-back refusal, as a device fault class like an overrun. To be filed as an issue.
5. MockRadio's enable from 0 channels is earlier than the UHD Module's by the UHD Module's configuration time. This dates from spec 20's VF-6, and goes with #49.

## Vision issues

- **§13.** Remove `stop_tail_ns` and `restart_lead_ns` from the TimingEnvelope list. "A tail of buffered samples after stop" becomes "the time a device needs before a stream can start again", and rule 1's "`stop` delivers the same tail" becomes "`stop` ends where the device's does".
- **§14.** "stop tail" becomes "restart readiness".
- **§59.** "stop tail" becomes "start lead and restart readiness".

Each section keeps its §N, and the index gains a revision row.

---

# Part B — the documents

**Measured** (VERIFIED by script at `7b6f376`):
- `design/*.md` holds 141 592 words, of which about 125k are the 14 accepted specs and 15.6k the audit and the re-review.
- `plan/` holds 327 177 words, of which 74 683 are applied amendment specs.
- 62 lines in accepted specs are longer than 1 500 characters.
- Inline history markers: the brief counted 424, and a broader pattern finds 636. Spec 18 has 133 of them, spec 06 109 and spec 09 76.

## DA-1 — The format of an accepted spec

**Problem.** The rules carry their history, rationale and copies inline, so they grow with every amendment and drift between copies.

**Decision.**
- **What `design/` holds.** Normative rules only. A rule is one topic in a few lines, then an indented `*Test:*` line (or `*Producer obligation*`). It carries no rationale, history or issue numbers.
- **Changes table.** Each spec ends with a *Changes* table: `| date | rules | change | record |`, linking the decision record in `plan/`.
- **One home per topic.** A Provider spec cites the Vocabulary's rule and adds only its own mechanism.
- **Rule IDs.** They are never renumbered or reused. A withdrawn rule keeps a one-line stub.
- **Amendments.** An amendment in `plan/` is a decision record: problem, decision, rejected alternatives (one line each), evidence, the owner's row and mutations. Its `design/` text is reviewed as the implementing diff. Text the owner must see before deciding goes in an appendix and becomes a link once it is committed.

**Binding text for AGENTS.md §2.** It goes after "Normative schemas…", in the same commit as `rules.py` (DA-3's pass 1) and not before:

> **Format of accepted specs.** A rule in `design/` is normative text only: one topic in a few lines, ending in an indented `*Test:*` line, with no rationale, history or issue numbers. Those go in the spec's closing *Changes* table and the decision record it links. A topic is written once: a Provider spec cites the Vocabulary's rule and adds its mechanism. Rule IDs are never renumbered or reused, and a withdrawn rule keeps a one-line stub. An amendment in `plan/` is a decision record, and its `design/` text is reviewed as the implementing diff. `plan/maintenance/tools/rules.py` must pass.

**Rejected.**
- *Trim the inline markers but keep them.* The markers are where the drift starts.
- *Move the rationale into footnotes.* The weight is the same.
- *Write the design text in full inside every amendment.* That is two copies to review; spec 21 ran 432 lines.

## DA-2 — The pilot

Appendix B covers RM-16, RM-17, RM-18, RM-21, RM-25 and one sentence of RM-26; MR-18, MR-20, MR-20a (a stub), MR-21, MR-22 and MR-25; UR-17, UR-25 and UR-26; UC-3 and its copy in spec 06; and TM-13c.

| | rule texts | words | history markers | lines > 1 500 chars |
|---|---|---|---|---|
| Today (VERIFIED) | 17 | 5 150 | 56 | 10 |
| Spec 21 as accepted (VERIFIED, quoted text only) | 17 | ≥ 6 900 | — | — |
| Pilot (VERIFIED, Appendix B) | 17 + one RM-26 sentence | 2 427 with test lines | 0 | 0 (longest 1 389) |

**Deleted rather than moved:**
- the tail;
- the draining and second-change refusals;
- the two leads and their two cases;
- MR-20a's settlement;
- the device `Stop`'s cancellations;
- MR-18's duplicated sentence, which says both "cut at `e`" and "cut at `e₁`" (lines 125–128);
- the UC copy in spec 06.

**Decision.** Accept the format as shown.

## DA-3 — The rollout plan

**Problem.** Reformatting 14 specs must not lose a normative sentence.

**Order.** One commit per spec, merges included:
1. The timing area with this spec's implementation, and DA-5 (a).
2. The rest of 07, 09 and 18.
3. DA-5 (b), which merges 11 into 08.
4. The Kernel specs 01–06, with 06 first.
5. Specs 02, 10, 14 and 16.
6. DA-5 (c), which moves the audit and the re-review and updates AGENTS.md §2's paths and reading order in the same commit.

**Checks.**
- **Mechanical: `plan/maintenance/tools/rules.py`**, about 60 lines, comparing base and head. It requires that:
  - every rule ID (`^- \*\*[A-Z]{2,3}-\d+[a-z]?\*\*`) survives or is listed as withdrawn;
  - every cited test name survives or is listed as renamed, and exists as a `fn` or `def`;
  - history markers and long lines only fall;
  - every relative link resolves.

  It also runs `mutate.py`'s exact-once match.
- **Semantic, and this is the gate.** `rules.py` cannot see a sentence dropped from a rule whose ID survives, which is how draft 1 lost UR-25's record of the receive stop. So there is one Opus review per spec, one at a time (AGENTS.md §8). The reviewer marks every removed sentence *moved* (and where), *history*, or *deleted by decision* (with its record), and an unmarked sentence fails the pass.
- **Aid only: `rules.py --sentences`**, about 30 lines. It pairs each base sentence with its closest head sentence (`difflib`) and lists those below a 0.6 ratio. It misses small normative edits, such as a changed number or a dropped "not".

## DA-4 — `plan/`

**Problem.** `plan/` is 327k words of history next to the current rules.

**Decision.** Keep `plan/` where it is, as the frozen record, and add one note to AGENTS.md §2: "plan/ is history; design/ is current". New amendments are DA-1 decision records, and the *Changes* rows link the applied ones.

**Rejected.**
- *Move it to `archive/`.* That breaks external `blob/main` links in issues and churns tool paths (E17).
- *Delete the applied specs.* Names that code and mutations cite would resolve nowhere (E17).

## DA-5 — The document map

**Problem.** Does the number of spec files need merging? The defects come from duplication and amendment layering, not from the file count.

**Decision.** No large merge, but three targeted moves:
- **(a)** UC-1…UC-6 get one home, spec 05 §5, with a one-line pointer in 06 §10. They bind targets, which is Module API, and the two copies are identical today (E18).
- **(b)** Spec 11 merges into 08.
  - 08 already holds the whole `sim` Vocabulary and the engine Module, and defers only the `sim.channel` content to 11, in the same crate (E18).
  - So `sim` gets one file; 08's engine section stays as it is.
  - The CH-n IDs stay, and 11 becomes a stub.
- **(c)** The audit and the re-review move to `design/archive/`, with stubs. Their 22 citing files are grep-replaced, and AGENTS.md §2's reading order changes in the same commit.

The stream rules live only in 07 through DA-1, not through a merge.

**Citations.**
- Rule IDs never change.
- A moved file leaves a stub, so `blob/main` links still resolve.
- In-repo links and crate descriptions are grep-replaced, and `rules.py` checks them.
- `file:line` citations inside `plan/` stay pinned to their commits.
- Vision §N, `CMA §N` and `v3/…` citations are untouched.

**Rejected.**
- *One merged spec.* It reads worse, and every amendment would touch it.
- *Merge 07 with 09 and 18.* Vocabularies and Modules carry separate versions (MA-33).
- *Merge 01–06.* They are six domains.
- *Merge 02 with 10.* They are separate crates.
- *Split the engine out of 08.* That moves 2k words for no defect.
- *Leave 08 and 11 split, or UC in both places.* That is the drift risk KI-1 had to manage by hand.

---

## Owner questions, in order

1. VH-1: a `Stop` ends the receive segment and its clock (on/off).
2. VH-3: no stop tail. A `Stop` cuts at the instant it is handled, never before the first sample not yet delivered.
3. VH-4: one start rule and one lead (`restart_lead_ns` removed), with the call-in-progress term in the ready instant, which `x310-like` models.
4. VH-2: plan on receipt and record at the instant. When a planned event fails, what is already recorded stands and the events after it are recomputed. A refused change or start halts the stream until its next change or `start_rx`, which keeps the failure visible until the user acts. The read-back refusal is a Mock ceiling and a follow-up issue.
5. VH-6: `start_rx` carries over, and a `Stop` cancels no update. A `Stop` that meets a timed start already queued stops the stream only after that origin (#56).
6. VH-5: a fault is recorded when it fires.
7. VH-7: `ezsdr_radio::timeline`.
8. VH-8: the differential harness.
9. DA-1: the format, and its AGENTS.md text.
10. DA-2: the pilot text.
11. DA-3: the rollout and its checks.
12. DA-4: keep `plan/` in place.
13. DA-5: the map, (a), (b) and (c).

---

## Appendix A — Mutations, tests and metrics

**Mutations to run at implementation** (`plan/maintenance/tools/mutations.json`; none has been run on the crates):

| id | file | mutation | must be killed by |
|---|---|---|---|
| H01 | `ezsdr-radio/src/timeline.rs` | a segment with no sample is kept | `rm_26_the_timeline_cases` (#53) |
| H02 | same | drop the "previous cut + lead" term | same (#49, two quick changes) |
| H03 | same | order by arrival only | same ("stop at 1 s, start at 2 s") |
| H04 | same | `start_rx` acts on a stream that is on | same (calls in the order made) |
| H05 | same | cut not floored at the origin | `rm_26_…` (#57's transmit case) |
| H06 | same | the ready term's call-in-progress part dropped | `rm_26_…` and `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time` |
| H06a | same | a refused event kept in the plan | `rm_26_…` (replan case) and `ur_25_a_rate_the_device_does_not_apply_at_the_switch_is_rejected` |
| H06b | same | an orphaned transmit clock not ended at its origin | `rm_26_…` (loss after a booked transmit change) |
| H07 | `ezsdr-mock-radio/src/lib.rs` | `Stop` does not end the receive clock | `mr_25_a_stopped_stream_keeps_its_end` |
| H08 | same | fault row written at the stream's end | `mr_21_overrun_shape` (the row's `lost`) |
| H09 | `ezsdr-radio-uhd/src/provider/rx.rs` | register the clock at plan receipt | `ur_25_a_second_change_follows_the_first` |
| H10 | same | start a segment after `Provider::stop` began | `ur_26_…` (#58) |
| H11 | `ezsdr-radio-uhd/src/provider/control.rs` | a device `Stop` cancels held timed updates | `ur_26_stop_actions` |

H01–H04 were killed on the prototype (VERIFIED). H05 on the prototype was equivalent, because `end` already floors at the origin, and the redundant line was removed.

**New tests.**
- `rm_26_the_timeline_cases`: the prototype's 11 cases, plus the ready term, the transmit side, a refusal replanned, and a loss after a booked transmit change.
- `mr_25_a_stop_cuts_at_its_instant` and `ur_26_a_stop_cuts_at_its_booking`: no sample at or after the handling instant, at `block_len` 65 536 and 390.6 kS/s as well.
- `ur_26_a_stop_before_a_queued_start_stops_after_its_origin`: on the FakeDevice, the untimed stop follows the origin and nothing on the new clock is published.
- `rm_26_…` halt cases: a refused change leaves no segment until the next `cold` change or `start_rx`, on both directions; a cut floored at what was delivered.
- `ur_25_a_second_change_follows_the_first`.
- `test_ea_16_rx_stop_then_start`, from spec 21.
- The VH-8 generators: about 1 000 seeded sequences per run against the timeline, MockRadio and the uhd-control rig.

**Renamed tests:** `mr_25_orderly_stop_delivers_the_tail_abort_does_not` and `ur_26_orderly_stop_delivers_the_tail_abort_does_not` both become `…_ends_at_its_instant_abort_at_once`. **Retired with MR-20a:** `mr_20a_a_tail_fault_reports_only_what_its_stream_lost`, `mr_20a_a_stop_ends_the_loss`. `mr_20a_the_overflow_comes_with_the_block_that_carries_it` moves to RM-17. **Retired with UR-25's refusal:** `ur_25_enabling_a_draining_stream_is_refused` and the unit test `ur_25_overlapping_cold_changes_are_refused_before_bookkeeping`.

**Metrics to report at implementation.**
- Words, markers and long lines per spec, before and after (`rules.py`).
- Code lines deleted against added in `ezsdr-mock-radio` and `ezsdr-radio-uhd`.
- FakeDevice tests at `--test-threads=128`.
- The number of open timing or parity issues.

## Appendix B — The pilot text (`design/` after this spec)

Committed by the implementation; this appendix is then replaced by a link to that commit.

- **TM-13c** A SampleClock never changes its rate: a rate change of a running stream ends its clock and starts a new one (UC-3), so a consumer sees the change in the block's domain id.
  *Test:* `tm_13c_sample_clock_new_id_on_rate_change`.

- **UC-3** `cold`: at the effective instant the target stops the function the key belongs to, applies the value and, if the function was running, starts it again; a function that is not running only takes the value. A stream so restarted ends its SampleClock and continues on a new one; between the two it delivers nothing and reports no gap (SC-12). Where the end and the new origin fall is the Vocabulary's (radio: RM-16, RM-25). Absent `at`, the effective instant is the current instant. A `cold` update changes a value, never the graph's structure (RS-4).
  *Producer obligation; tested by each Vocabulary's Providers.*

- **UC-1…UC-6** (spec 06 §10) Spec 05 §5's.

- **RM-16** A receive segment ends at its **cut**: the first sample at or after the instant of what ends it — a `Stop` of `<device>/rx` or `<device>`, `Provider::stop`, a device loss or a `cold` change (RM-21) — and never before its origin or the first sample not yet delivered; under `abort`, the first sample not yet delivered. Nothing at or after the cut is delivered. A `Stop` takes effect at the instant the Provider handles it. On the transmit side a `Stop` of `<device>/tx` or `<device>`, and `Provider::stop`, close the open burst at the first sample at or after the instant (`BurstEnd::Stop`), every earlier sample transmitted, and cancel the held bursts starting at or after it; a later burst is transmitted as usual (RM-15). A Provider that hands samples ahead ends a stopped burst at the last sample handed over, within its in-flight window. A `Stop` cancels no update; `Provider::stop` drops every pending command but one already handed to a device that cannot recall it, recorded as issued.
  *Producer obligation (MA-13, KA-12). Test:* `rm_26_the_timeline_cases`, `mr_25_orderly_stop_ends_at_its_instant_abort_at_once`, `mr_25_a_stopped_stream_keeps_its_end`, `ur_26_orderly_stop_ends_at_its_instant_abort_at_once`, `ur_26_a_stopped_stream_keeps_its_end`, `mr_25_a_stop_cuts_at_its_instant`, `ur_26_a_stop_cuts_at_its_booking`.

- **RM-17** An **overrun** at `f` loses every receive sample in `[f, f + radio.timing.overflow_restart_gap_ns)`. The block in progress ends at `k_f`, the first sample at or after `f`, and is delivered if it holds a sample. The next block begins at the first sample at or after `f + gap`; if a sample was lost it carries `GAP_BEFORE | RESTARTED` and `lost` equal to its whole time jump, any loss already pending included (SC-18). `radio.RX_OVERFLOW { cause: overrun, lost, restart_gap_ns }`, timed `TimePoint(<receive SampleClock>, k_f)`, is emitted with the block that carries the loss and never otherwise; an overrun the Provider causes by dropping a block its link refused is emitted when it drops it (SC-20a).
  *Producer obligation (Vision §23, §58 #6). Test:* `mr_20a_the_overflow_comes_with_the_block_that_carries_it`, `mr_19_backpressure_is_an_overrun`.

- **RM-18** A **sequence error** at `f` loses `radio.rx.block_len` samples from `k_f`; the next block carries `GAP_BEFORE | SEQ_DISCONTINUITY`, not `RESTARTED`, with `lost` its time jump, and `radio.RX_OVERFLOW { cause: sequence, lost, restart_gap_ns: 0 }` is emitted as RM-17 says.
  *Producer obligation.*

- **RM-21** UC-3's `cold` covers the channel counts and sample rates, UC-6's `hardware_timed` the frequencies and gains with a queue of `radio.timing.command_queue_depth`; antennas do not change during a Run. A receive stream is **on** from the Run's start until a `Stop` of `<device>/rx` or `<device>`, and again from a `start_rx` (RM-12); it **runs** while it is on, has a channel and a link, and is not halted by a refusal (RM-25). When a `cold` change, a `Stop`, a `start_rx` or a loss changes whether it runs or how, its segment ends (RM-16) and, if it runs afterwards, the next one starts (RM-25); a change to a stream that does not run only applies its value. A transmit stream has a SampleClock while its channel count is above 0; a `cold` change replaces it, and a `Stop` ends its bursts, not its clock.
  *Note, not a rule:* a `start_rx` a Spec schedules at a `Stop`'s own instant arrives first, at the start round, so the stream stays stopped; a start meant to follow an untimed `Stop` is scheduled at least `start_lead_ns` after it, since a device handles the `Stop` some time after its dispatch.
  *Producer obligation. Test:* `rm_26_the_timeline_cases`.

- **RM-25** Every SampleClock's origin is on its root's lattice — a whole multiple of the numerator, in lowest terms, of its `root_ticks_per_tick` — so clocks of one ratio share every sample instant and T0 is a sample instant of every stream. The first receive segment starts at T0 (MR-11, UR-15), the first transmit clock at the first lattice instant at or after `arm` (MR-9). Every later one starts at the first lattice instant at or after its effective instant, the stream's previous cut plus `radio.timing.start_lead_ns`, and the instant the Provider is ready for it plus that lead; the Provider's spec states its ready instant, and a Mock profile emulating a device states that device's. Commands and faults of one stream take effect in the order of their effective instants and, at one instant, in the order received, faults counting as received at the Run's start. Between a cut and the next origin nothing is delivered and nothing is a gap. A receive SampleClock is registered when its segment begins and ended when its cut is made, so a segment cut at or before its origin has none; a transmit clock is registered, and its predecessor ended, when its change is booked, so that a burst admitted after it can target it (KC-21a).
  A planned event refused or pre-empted at its instant — a configuration the device does not apply, a loss, `Provider::stop` — is removed; cuts made and clocks ended stand, and only the events after that instant are recomputed, a later one the new plan refuses being refused then (`COMMAND_REJECTED`). A refused change or start — a `cold` change, an enable or a `start_rx` whose configuration the device does not apply — **halts** the stream: it has no segment until the next `cold` change or `start_rx`, which starts it by the rule above; a transmit clock registered for the refused change ends at its origin, its held bursts cancelled (`COMMAND_REJECTED`). Only transmit clocks can so end with no sample.
  *Producer obligation. Test:* `rm_26_the_timeline_cases`, `mr_18_a_cold_change_starts_its_clock_on_the_lattice`, `ur_15_every_clock_starts_on_its_lattice`, `ur_25_a_rate_the_device_does_not_apply_at_the_switch_is_rejected`.

- **RM-26** (sentence added) `ezsdr_radio::timeline` computes RM-16, RM-21 and RM-25 once: from a stream's initial state, its lead, its Provider's ready term and its commands and faults (effective instant, arrival, ready instant, refused or pre-empted), the segments with their origins, cuts and configurations; both Providers take their cuts, origins and clock registrations from it.
  *Test:* `rm_26_the_timeline_cases`.

- **MR-18** An `UpdateParameter` must target the device and name a configuration key with that key's class; otherwise `radio.COMMAND_REJECTED`. The Mock books it on receipt: `coerce` over the configuration projected to its effective instant `e` (UC-2's order), a refusal being `COMMAND_REJECTED` with no change (RM-7). `hardware_timed`: `e` is `at`, or `now + lead` when absent; an earlier one is late (`radio.LATE_COMMAND { key, requested, applied: now + lead }`, `e = now + lead`); with `command_queue_depth` pending it is refused with `radio.COMMAND_QUEUE_FULL { key, depth }`. `cold`: `e` is `at`, or the receipt when absent or past (a past `at` emits `LATE_COMMAND`), for receive never before T0; its cut and next origin come from `ezsdr_radio::timeline`, the Mock's ready instant being the receipt and, for a receive segment after a cut, also that cut plus one receive call at the old rate (the longer of `block_len` and MR-3's `rx_packet_samples`) plus MR-3's 3 ms on a profile that emulates a device. A transmit change cancels the held bursts of the old clock at or after its cut (`COMMAND_REJECTED`, "cancelled by a cold change") and gives the new clock a new `BurstTracker` and `DeviceModel` (MR-15). At `e` the value applies and is recorded in `applied` (MR-27); updates at one `e` apply in delivery order; in channel mode a gain or frequency enters its timeline (MR-33, MR-34).
  *Ceiling:* a cold transmit change writes the configuration before it stops the transmitter, so a held burst the stop opens carries the new `radio.tx.channels` in its headers; nothing reads them yet. *Ceiling:* the Mock never refuses a planned change at its instant, where the UHD Module can (its UR-12 read-back), unless a fault is injected.
  *Test:* `mr_18_hardware_timed_updates`, `mr_18_a_cold_rate_change_starts_a_new_sample_clock`, `mr_18_a_cold_transmit_change_replaces_the_tracker`, `mr_18_a_scheduled_pair_is_checked_when_it_applies`, `mr_18_a_cold_change_starts_its_clock_on_the_lattice`, `mr_18_a_receive_enable_from_zero_waits_the_start_lead`.

- **MR-20** A kept fault fires at `f = T0 + at_ns` (SE-4), in the step's ordered loop (RM-25's order), and is recorded then (MR-27). `rx_overflow` and `rx_sequence_error` are MR-21's and MR-22's if the receive stream runs at `f`, and do nothing otherwise. `device_lost` stops the device at `f` as `stop(Abort)` would: the receive blocks before `f` and the block in progress up to `k_f` are published, the transmit side stops at `f`, held bursts and pending commands go to `rejected` with `"MR-20: device lost"`, faults not yet fired are recorded `applied: false`, and the step returns `ModuleError { kind: DeviceLost }`. The losing step's Actions and every later one are refused the same way, with no event; `stop` and `cleanup` still succeed (RS-11).
  *Producer obligation (SC-13). Test:* `mr_20_faults_fire_at_their_instants`, `mr_20_a_lost_device_delivers_every_sample_before_the_loss`, `mr_20_a_lost_device_transmits_nothing_after_the_loss`, `mr_20_a_loss_cancels_what_it_left_pending`, `mr_20_a_stop_received_in_the_losing_step_does_not_cancel_the_loss`, `mr_20_an_action_after_the_losing_step_is_refused`, `mr_20_a_fault_at_a_loss_s_instant_removes_nothing`, `mr_20_a_loss_ends_its_receive_stream_at_its_instant`.

- **MR-20a** Withdrawn (spec 22, VH-5): a fault is recorded when it fires (MR-20, MR-27); its event follows RM-17.

- **MR-21** An overrun at `f` (RM-17): samples `k_f … k_g − 1` are not delivered, `k_g` the first at or after `f + overflow_restart_gap_ns`; the block in progress ends at `k_f`; the next block begins at `k_g` with `GAP_BEFORE | RESTARTED` and `lost` its whole time jump, losses with no block between them adding up; `k_g = k_f` changes nothing. The fault's record holds `lost = k_g − k_f`.
  *Test:* `mr_21_overrun_shape`.

- **MR-22** A sequence error at `f` (RM-18): `block_len` samples from `k_f` are lost, the next block beginning at `k_f + block_len` as MR-21 says, with `GAP_BEFORE | SEQ_DISCONTINUITY`.
  *Test:* `mr_22_sequence_error_shape`.

- **MR-25** `stop(mode)` and a `Stop` are RM-16. The transmit block in progress is emitted up to the first sample at or after the stop instant, every held burst starting before that sample opened and emitted in turn; then the open burst closes (`BurstTracker::stop`, recorded), `DeviceModel::close()` runs, and later held bursts are cancelled into `rejected`. The receive cut comes from `ezsdr_radio::timeline`. `stop(mode)` also cancels the fault wakeups and records the pending commands in `rejected`.
  *Producer obligation (MA-13). Test:* `mr_25_orderly_stop_ends_at_its_instant_abort_at_once`, `mr_25_a_stopped_burst_transmits_every_sample_before_the_stop`, `mr_25_a_stop_on_a_sample_instant_does_not_transmit_that_sample`, `mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter`, `mr_25_a_stop_transmits_every_held_burst_before_it`, `ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order`, `mr_25_a_stream_stop_keeps_the_pending_commands`, `mr_25_a_stopped_stream_keeps_its_end`.

- **UR-17** uhd-rx carries out its plan (UR-25). It calls `rx_recv(block_len, 100 ms)` in a loop, asking for no more than up to a pending cut; it takes as the cut the later of the cut it was given and the first sample it has not delivered, discards samples at or after that cut, and stops the stream untimed once its samples reach it or it has been silent past it for one block plus 3 ms — the X3x0 ignores a continuous stream's stop time —, recording the stop in `applied` with the cut and the instant it was issued, and ends the clock at the cut. For the next segment it reopens the streamer if the channel count changed, configures the direction with the configuration in effect at the origin (UC-2's projection, held timed commands included), issues a timed start there once configured, and registers the clock at its first block, at or after the origin; when a cut cancels a segment whose timed start is already issued, it stops the stream untimed only after that origin has passed, discarding what arrives; a configuration UR-12 refuses halts the direction (RM-25), with `COMMAND_REJECTED` and UR-12's reason.
  A result's first tick `f` gives the index `k = (f − origin) / N`, rounded to the nearest sample off the lattice (`stats.rx_off_lattice`). A block after the expected index carries `GAP_BEFORE` and `lost` its jump, with the flags UR-18 and UR-19 left pending; one overlapping what was delivered is trimmed (`stats.rx_overlapping`) and dropped if empty; one before its segment's origin or the previous stream's untimed stop is dropped (`stats.rx_before_origin`). Its bytes are SC-4's planar `cf32` from a `HostPool` (HD-2), published as `SampleBlock::new_host` with `direction: rx` and `valid` full to every link (SC-11); a publish not `Accepted` is counted (`stats.link_drops_seen`). A `LateCommand` at a segment's start is a missed origin: `radio.LATE_COMMAND { key: null, requested: origin, applied: T1 }`, a timed start at `T1`, the first lattice instant at or after now plus the start lead, and a first block with `GAP_BEFORE` and `lost` its index, the origin unchanged (SC-18, TM-13b); while a cut is pending it is no missed start, and the stream is stopped untimed.
  *Test:* `ur_17_blocks_carry_the_device_timestamps`, `ur_17_a_missed_start_restarts_with_a_gap`, `ur_17_an_overlap_trimmed_to_nothing_is_dropped`, `ur_17_blocks_reach_every_link`, `ur_25_the_receive_stop_is_recorded_when_it_is_issued`, `ur_26_a_stop_before_a_queued_start_stops_after_its_origin`.

- **UR-25** uhd-control books a `cold` update, a `Stop` and a `start_rx` when it takes the Action (UR-14): it converts `at` (one it cannot convert is refused, never taken as absent), runs `coerce` over the configuration projected to `e` (a refusal is `COMMAND_REJECTED`, nothing changed; RM-7), and takes the stream's cuts and origins from `ezsdr_radio::timeline` with this Module's **ready instant**: the booking, or, for a stream enabled from no stream, the end of the configuration uhd-control then does itself, opening the streamer; and, for a receive segment after a cut, at least that cut plus one receive call at the old rate (the longer of `block_len` and `rx_packet_samples()`) plus the 3 ms delivery allowance, since uhd-rx learns of a cut only after the call in progress. A `Stop` cuts at its booking. An `at` already past emits `radio.LATE_COMMAND`. A configuration UR-12 refuses at the switch is a refusal at its instant, which halts the direction (RM-25), with `COMMAND_REJECTED` and UR-12's reason. Before the next Action it registers a transmit clock, ending its predecessor, and hands each owner its updated plan. uhd-rx carries a receive plan out as UR-17 says, uhd-tx a transmit plan as UR-23 says; once `Provider::stop` has begun, neither starts a segment.
  *Test:* `ur_25_a_cold_rate_change_starts_a_new_clock_on_its_lattice`, `ur_25_enabling_tx_applies_the_configuration`, `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time`, `ur_25_rx_channels_from_zero_starts_at_its_instant`, `ur_25_a_rate_change_admits_a_burst_on_the_new_clock`, `ur_25_a_rate_the_device_does_not_apply_at_the_switch_is_rejected`, `ur_25_a_cold_change_the_envelope_refuses_changes_nothing`, `ur_25_the_old_stream_is_stopped_untimed_at_e1`, `ur_25_a_cold_receive_change_delivers_every_sample_before_e1`, `ur_25_a_stream_silent_before_e1_switches_before_e2`, `ur_25_a_long_block_at_a_low_rate_stops_the_stream_at_e1`, `ur_25_a_slow_link_at_a_low_rate_delivers_the_old_clock_to_e1`, `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time`, `ur_25_the_switch_applies_the_configuration_in_effect_at_e2`, `ur_25_a_cold_transmit_change_cancels_the_held_bursts`, `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`, `ur_25_a_second_change_follows_the_first`.

- **UR-26** An update of `radio.{rx,tx}.antenna`, or of a key or target UR-24 and UR-25 do not name, is `COMMAND_REJECTED`, as is a `Stop` of another target and an Action other than `TxBurst`, `UpdateParameter`, `Stop` and `start_rx` on `<id>/rx`. A `Stop` and `Provider::stop` are RM-16, booked as UR-25 says. `Provider::stop` also cancels the held timed commands (UR-24), sends the transmit side's end of burst, allows 100 ms for its last `BURST_ACK`, joins uhd-tx, uhd-rx and uhd-control, and records the stop instant in `timing`; a second `stop` does nothing. `cleanup()` stops what still runs, closes the streamers no detached thread owns (UR-16), writes UR-30's sections and drops what `prepare` kept (MA-7).
  *Test:* `ur_26_stop_actions`, `ur_26_orderly_stop_ends_at_its_instant_abort_at_once`, `ur_26_an_abort_publishes_nothing_after_the_stop_instant`, `ur_26_an_orderly_stop_does_not_restart_the_stream`, `ur_26_cleanup_is_idempotent`, `ur_26_a_stopped_stream_keeps_its_end`.

Outside the counted set, the same commits edit:
- RM-6 and RM-20, which lose two envelope members;
- MR-3's table, where `stop_tail_ns` and `restart_lead_ns` go and `rx_packet_samples` (1 996) comes;
- RM-12, MR-29 and EA-16, which gain `start_rx` as spec 21 wrote it;
- UR-23, which gains the transmit plan;
- MR-27, where `lost` is defined by VH-5.

## Appendix C — The prototype (VERIFIED: 11 tests pass; built outside the tree and deleted)

```rust
pub enum Kind { Start, Stop, Loss, Cold { channels: u16, period: i64 } }
pub struct Item { pub e: i64, pub seq: u64, pub ready: i64, pub kind: Kind }   // e: effect; seq: arrival
pub struct Segment { pub origin: i64, pub end: Option<i64>, pub period: i64, pub channels: u16 }
fn at_or_after(t: i64, origin: i64, p: i64) -> i64 { let d = (t - origin).max(0); origin + (d + p - 1) / p * p }

pub fn plan(t0: i64, channels: u16, period: i64, lead: i64, items: &[Item]) -> Vec<Segment> {
    let mut items = items.to_vec();
    items.sort_by_key(|i| (i.e, i.seq));
    let (mut on, mut ch, mut p) = (true, channels, period);
    let mut cur = (ch > 0).then_some(Segment { origin: t0, end: None, period: p, channels: ch });
    let mut prev_end: Option<i64> = None;
    let mut out = Vec::new();
    let end = |cur: &mut Option<Segment>, e: i64, prev_end: &mut Option<i64>, out: &mut Vec<Segment>| {
        if let Some(mut s) = cur.take() {
            let c = at_or_after(e, s.origin, s.period);
            s.end = Some(c);
            *prev_end = Some(c);
            if c > s.origin { out.push(s); }            // no sample, no clock
        }
    };
    let begin = |e: i64, ready: i64, prev_end: Option<i64>, ch: u16, p: i64| {
        let t = e.max(ready + lead).max(prev_end.map_or(i64::MIN, |x| x + lead));
        Segment { origin: at_or_after(t, 0, p), end: None, period: p, channels: ch }
    };
    for i in items {
        match i.kind {
            Kind::Loss => { end(&mut cur, i.e, &mut prev_end, &mut out); return out; }
            Kind::Stop => { on = false; end(&mut cur, i.e, &mut prev_end, &mut out); }
            Kind::Start => if !on { on = true; if ch > 0 { cur = Some(begin(i.e, i.ready, prev_end, ch, p)); } },
            Kind::Cold { channels, period } => {
                let ran = cur.is_some();
                end(&mut cur, i.e, &mut prev_end, &mut out);
                ch = channels; p = period;
                if on && ch > 0 { cur = Some(begin(i.e, if ran { i64::MIN / 2 } else { i.ready }, prev_end, ch, p)); }
            }
        }
    }
    if let Some(s) = cur { out.push(s); }
    out
}
```

**The cases it was tested on**, at 1 MS/s on a 1 GHz root with a 50 ms lead:
- a `cold` change after a `Stop` starts nothing (#46);
- a second `Stop` keeps the first end (VG-1);
- a start that arrived first, at 2 s, still restarts after a `Stop` at 1 s;
- `stop(); start()` restarts, and `start(); stop()` does not;
- 0 channels and back at once starts a lead after the cut (#49, point 2);
- a `Stop` before a pending enable from 0 starts nothing (#52);
- two quick changes give one restart and no empty clock (#53);
- a `Stop` between `e₁` and `e₂` leaves no clock (#53);
- an enable 500 ms ahead starts at its instant (VF-6);
- an off-lattice `Stop` rounds up (#33);
- a loss at a change's cut is ordered by `e` (#48).

## Draft history

- **Draft 1 → draft 2** (adversarial review: three blocking findings and six non-blocking).
  - **B1.** The "earliest cut" is withdrawn. A `Stop` cuts at its handling instant on both Providers. The call-in-progress term moves into the ready instant (VH-4), and `x310-like` models it with the X300's measured packet length and the 3 ms allowance. Feasibility: the switch keeps the full lead.
  - **B2.** One replanning rule (VH-2, RM-25): a failed event is removed and the plan recomputed, and an orphaned transmit clock ends at its origin. The read-back refusal is recorded as a Mock ceiling. The Manifest's "no empty clock" guarantee is stated as receive-only.
  - **B3.** New implementation order: the harness gates land first, the Providers and the envelope move together, and the mutation re-spells are estimated per step.
  - **Non-blocking.** Verdicts for #45, #48, #49, #52 and #53 restated. Spec 21's guidance carried into RM-21. #56 gets a recommendation (late timed start). UR-25's record of the receive stop is restored, in UR-17. DA-3 makes the per-sentence review load-bearing and adds a sentence aid. DA-5 (b) gets its reason restated and (c) its AGENTS.md reading-order change. The AGENTS.md bullet lands with `rules.py`.
- **Draft 2 → draft 3** (second review: three blocking findings and four non-blocking).
  - **B1.** RM-16 gains one floor for every cut: never before the first sample not yet delivered. uhd-rx takes the later of the cut and what it has delivered, and ends the clock there.
  - **B2.** After a failure, cuts already made and clocks already ended stand, and only the events after the failure are recomputed. A refusal halts the stream until its next change or `start_rx`, for both directions; the alternative of an automatic restart is rejected.
  - **B3.** The late timed start is withdrawn in favour of the reviewer's design: the start is issued once configured, and a `Stop` that meets a queued start stops the stream after its origin.
  - **Non-blocking.** The read-back refusal is now a follow-up issue. Step 1's gate wording is fixed. `--sentences` is called an aid only. MockRadio's earlier enable from 0 is listed as a follow-up. VH-3, VH-4 and VH-7 evidence moved to Appendix D.

- **Draft 3 → this text** (passed review; four fixes and a trim).
  - UR-17's cut sentence is rewritten, and its clock registration now happens at the first block, at or after the origin.
  - The reason for halting is now "it keeps the failure visible until the user acts".
  - A refused start halts the stream too.
  - Every item is cut to DA-1's shape (problem, decision, one-line rejections), with the evidence moved to Appendix D (E7–E18).

## Appendix D — Evidence (VERIFIED by reading at `7b6f376`)

- **E1.** `plan/phase7/bench-results.md:325-338`: an untimed stop ends the X300's continuous stream where it is issued (−0.010 to −0.042 ms), and 2 blocks then arrive in flight. The X300 ignores a timed stop (`radio_rx_core.v`, "timed STOP commands are not supported").
- **E2.** `crates/ezsdr-radio-uhd/src/provider/rx.rs:272-289` and UR-26: the UHD Module makes the tail itself. It sets the cut at `now + 1 ms`, delivers up to it, and discards later samples.
- **E3.** `v3/cpp/uhd_usrp/multiusrp.cpp:678-695`: `stopContinuousReceiveImpl` issues `STREAM_MODE_STOP_CONTINUOUS`, then discards the buffered samples ("バッファーに溜まっている受信データを破棄する").
- **E4.** `crates/ezsdr-mock-radio/src/profile.rs:99-116` and `crates/ezsdr-radio-uhd/src/profile.rs:114-122`: `restart_lead_ns` = `start_lead_ns` = 50 ms on `x310-like`, `x310-ubx`, `x310-obx` and `x310-cbx`; 0 on `ideal`.
- **E5.** The same UHD profile's comment: the bench's raw restarts began on the requested tick at leads down to 1 ms in 19 of 21 rows (B8).
- **E6.** `plan/phase7/bench-results.md:929-931`: the Authority's host-derived time runs about 0.1 ms behind the device's at 200 Msps, and `rx_packet_samples()` read 1 996 at every rate (UHD's `max_num_samps` at MTU 9000).
- **E7.** Spec 21's text, which is the source of the "≥ 1 750 words" figure: about 1 752 words of quoted rule text in the VG-2 and KI-1 sections.
- **E8.** `crates/ezsdr-kernel/src/time/domain.rs:297-322`: `ClockRegistry::end` can be set once only, so a receive clock's end is set when its cut is made.
- **E9.** `crates/ezsdr-kernel/src/coordinator/admission.rs:118-124`: the current transmit clock is the latest registered record without `ended_at`. Ending receive clocks cannot affect it.
- **E10.** KC-17 and KC-18 (`design/06-kernel-coordinator.md:432-433`): a Spec's timed entries are admitted and dispatched in `(instant, schedule index)` order. A Session needs the new transmit clock as soon as `tx.sample_rate = …` returns (E9).
- **E11.** `domain.rs`, the `declare_sample_clock` comment: "TM-13b fixes it at the first sample". So a receive origin belongs to the first sample.
- **E12.** UR-17 and UR-25, and `provider/rx.rs:180-183`: uhd-rx polls its commands between `rx_recv` and publishing, and trims a block that straddles a cut.
- **E13.** UR-17, UR-18 and `provider/rx.rs` (`Pending`): the UHD Module learns an overrun's size only from the next block's timestamp.
- **E14.** Both Modules' `Cargo.toml` depend on `ezsdr-radio` and not on each other. RM-26 already makes `ezsdr_radio::device` the single implementation of the device rules.
- **E15.** handoff.md's three stress probes, at `--test-threads=128`, failed 34, 37 and 35 FakeDevice tests.
- **E16.** `provider/test_support.rs` builds a `Core` on a `ManualTimeAuthority`. FakeDevice reads `Instant::now()` (`device.rs:419, 506, 1140`).
- **E17.** Issue #49 links `blob/main/plan/maintenance/21-amendments.md`. The phase tools read `plan/phase*/tools/mutations.json` by path. Code comments and mutation rows cite review names (Review M, VB-6) that resolve in `plan/`.
- **E18.** `diff` shows UC-1…UC-6 identical in 05 §5 and 06 §10. The headers of 08 and 11 both name `sim` 1.1.0 and the crate `ezsdr-sim`, and 08's own scope defers the `sim.channel` content to 11.
