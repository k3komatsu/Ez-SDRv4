# Note 47 stage 4: transmit values change on an empty stream (test plan, rev 8)

Status: rev 11 (gate fix round, 2026-11-07: two mutation rows, §8 rev 11). Rev 10 is `test-plan.v10.md`. Rev 10 (implementer, 2026-11-06, part (b) sub-step b6: the rev 9 left list closed except F9 and S7 through the Kernel, see §8 rev 10; fix round 1 of 2026-11-07, the last bullet of §8). Rev 9 is `test-plan.v9.md` (b5; its left list is in §8 rev 9); rev 8 is `test-plan.v8.md` (b4); rev 7 is `test-plan.v7.md` (b3b); rev 6 is `test-plan.v6.md` (b3); rev 5 is `test-plan.v5.md` (b2); rev 4 is `test-plan.v4.md`; rev 3 is `test-plan.v3.md`; rev 2 is `test-plan.v2.md`; rev 1 is `test-plan.v1.md`. Base `main` at `aee03ea`, parts (a) and (b) uncommitted in the tree.

**Part (b) status at rev 4.** Only the pure value rule of §3.1 is in the tree: `Busy` and `Until` in `crates/ezsdr-radio/src/staging.rs`, `Refusal::TxBusy`, nine `rm_21_*` tests in `crates/ezsdr-radio/tests/staging.rs`, and the rows N47S4-01, -02, -03, -04, -17, -17b, -28 (each probed killed in a throwaway copy; logs in `probes-b/`). No Provider calls `Busy` yet, so no behaviour changes and no spec text is changed yet. **Not started:** the transmit form of `Staging::set` (the transmit keys, `NoStream` for a transmit channel, the receipt instant); each Provider's per-stream `Busy` (booking, `Stop`, loss, `Provider::stop`, value receipt); the `value_gen` snapshot and `ColdConfig::of` for transmit; the UHD apply turn (R4, M1, M2, `apply_due`, the generation mark's publication); FakeDevice's persistent refusal (M3); MockRadio's value history (B6) and its `Busy`; §3.5 owner-guard tests; S1–S14 and F-cases beyond the pure tests; the Python `repeat` keywords and checks (S13); the removal of `radio.timing.tx_start_lead_ns` (P8, §6.3) and the schema regeneration; §58 #16's transmit form (S12); every spec, Vision and KNOWN_LIMITATIONS text of §6.2. Each is for the next implementation session; §8 lists them as open.

**Deviations recorded in rev 4 (§8, part (b)):**
- D-1. `Busy::book` takes the exclusive end in root ticks (`Option<i64>`, `None` for a repeat), not `(start, length)`. The Provider computes `clock.instant(start_k + len)` (m5), so `Busy` needs no clock. The moved start is what the Provider passes (booking-time `SendAsap`, P4).
- D-2. `Busy::stageable(receipt)` returns `Result<(), Refusal>` (the refusal is built in one place), not `bool`. `Refusal::TxBusy { until: Until }` carries the busy end; the stream number is not in the reason (the Provider's rejection names the Action id, UR-26).
- D-3. `Until` is public (the refusal carries it). `Busy` derives `Default` (an idle stream).
- D-4. The pure rule is testable without a Provider: §3.1's table of call sites is still the contract for the wiring.
- D-5 (self-review P2-1). `mr_25_a_transmit_clock_ends_at_its_origin_on_a_stop_at_arm` also asserts the layout row's `ended_at` equals its origin (MR-27). Killer: row N47S4-39 (the layout end call removed). N47S4-38 mutates only the clock end and is unchanged.
- D-6 (self-review P2-2, S11). The two S11 Provider-path tests were attributed to N47S4-35 (a lib test; wrong). Their killers are now: UHD `ur_23_a_burst_after_a_stop_targets_the_one_clock` ← N47S4-41 (a Stop drops the stream's clock); Mock `mr_09_a_burst_after_a_stop_targets_the_arm_clock` ← N47S4-42 (arm does not keep its registered clock). The Mock test gained a second Stop (`registered` must not be re-registered by it). A Stop-time mutant on the Mock was tried first and was not killed: the second Stop finds no plan segment, so it returns before it registers; recorded, not kept.
- D-7 (self-review P2-3, seed). The arm seed `control.rs` `clock = start.tx…` had no row (P263 was removed in part (a)). Row N47S4-40 (the seed set to `None`), killer `ur_25_a_transmit_stream_has_one_clock_for_the_run`.
- D-8 (self-review P3-3, `Tx.cut`). Part (a) removed `self.cut = None` from `end_clock` and `halt_failed_reopen`, so `cut` could outlive `clock`. Both lines are restored (`tx.rs`, beside `self.clock = None`). This writes `cut` a second time (the reviewer's "clear where the clock is cleared"); §2.2's "pass is the one writer" is amended by this line: `cut` is cleared with the clock it belongs to. Consequence: `ur_25_an_end_booked_after_the_first_read_…` asserted `cut == Some(origin)` after its `pass()`, and that `pass()` also ends the clock, so the cut is cleared by then. Its assertion is now `cut.is_none()`; its checks on the cancellation, the reservation, the UR-25 reason and the absence of a send are unchanged, and N47S4-37 still kills it (re-probed).
- D-9 (self-review P3-1, P3-2). The stale comment at `control.rs` ("KC-21a: …") now says "the transmit clock registered at `arm`". `design/09` gains its Changes row for MR-32 (the segment key).
- Self-review (opencode2, `opencode/deepseek-v4.1-flash-free#max`, one round; `self-review.log`): P1 "part (b) absent" is the state recorded at rev 4, not a defect of part (a); P2-1, P2-2, P2-3, P3-1, P3-2, P3-3 are adopted and fixed as D-5 to D-9. Its "Not checked" items are the end checks, run after these fixes.

## Changes in rev 3

Read for rev 3 (path:line, this session): `crates/ezsdr-radio-uhd/src/provider/tx.rs:196–342, 411–534, 535–784, 812–1099`; `core.rs:245–379, 520–689`; `control.rs:180–200, 360–389, 560–584`; `crates/ezsdr-mock-radio/src/lib.rs:186–235, 806–955, 1112–1206, 1336–1435, 1980–1990, 2100–2159, 2212–2226, 2330–2400`; `plan/maintenance/tools/mutations.json` (rows matching `version`, `plan_section`, `cut`, `clocks`, `tx_gen`, `tx_reconcile`).

| Ruling | Where it lands in rev 3 |
|---|---|
| O-1 (accept scalar `Busy`; RM-21 says "bounded by one in-flight window after a Stop") | §0 O-1 (rewritten justification: 13 is the tightest bound knowable at receipt); §6.2 RM-21 text |
| O-2 (drop `version`; the read is in `pass`, not `step`; no `planned()`) | §2.1 (rewritten); §2.2 rows `Tx.version`, `Tx.plan`; §2.3; §2.4 (N47S4-37's killer drives `pass()`) |
| O-3 (no transmit cut other than the origin; delete `refuse_first_send` and UR-22's "or … a clock cut"; delete `ur_23_a_cut_before_four_first_samples_is_refused`, not retarget) | §2.2 (path table, `refuse_first_send`); §2.3 (rows for `ur_23_a_cut_before_four…`, `ur_23_a_clock_cut_leaving_n…`, `ur_23_m14…` are **deleted**; the preemption test is the reachable equivalent and is retained unchanged) |
| Clock end | §2.1: the code's semantics stated (UHD `core.rs:668`: an End booked at S ≤ O ends the registry clock at O; after O never ended; the owner's `end_clock` ends only its own clock; the Mock's twin `end_tx_clock`) |
| M1 (arm publishes `Some(0)`) | §3.2 (publication rule and the arm's publication) |
| M2 (the configure is a turn of its own; publication only if not lost; `Provider::stop` asserts) | §3.2; §3.4 S10 rows; §4 N47S4-12b, -33 |
| M3 (FakeDevice refusal persists, keyed to a channel and frequency) | §3.3; §3.4 S7; §4 N47S4-16 |
| M4 (keep the Mock's transmit clock end as a one-clock function; a test mirroring `core.rs:2063`; a row) | §2.2 (`tx_reconcile`), §2.4 (`mr_25_a_transmit_clock_ends_at_its_origin_on_a_stop_at_arm`), §2.5 (row N47S4-38) |
| m1 | §2.1 (the read in `pass`, as O-2) |
| m2 (rows as concrete text substitutions) | §2.5 (P1398-family rows, `N47S4-A1` replaced); §4 rows 09a–c, 11, 12c, 35 |
| m3 (receive callers of `timeline::start_lead_ns` and `design/07:445`) | §6.3 |
| m4 (between-burst configure passes `rate: None`) | §3.2 |
| m5 (`Busy` in root ticks) | §3.1 |
| m6 (ActionId travels with the snapshot) | §3.2 (`ColdConfig::of(snapshot, id)` on `Held`) |
| m7 (apply log format unchanged; observe `calls()` order and the manual clock) | §3.3 (replaces the log-tick change) |
| m8 (N47S4-12 equivalent in behaviour; killer is the mark accessor) | §4 N47S4-12 |
| m9 (receive at exactly `e`, hold device time in `[e, e+2)`) | §3.5 (c) |
| m10 (transmit abort case in `rm_26_the_timeline_cases`) | §2.4 |
| Opus recheck, `UR-22` text | §2.3 (UR-22's clock-cut wording), part (a) |

Deviations that the implementation records: see §2.6 (rev 3 notes on the part (a) implementation).

Scope: brief R1–R8 as amended by the rulings; part (a) B4 (one transmit clock per stream) before part (b) (transmit values). Out of scope: overlap refusal, repeat switch, removing `Tx::preempt`, the in-flight window's value, #79 (except the parity note in §5), transmit rate change.

Conventions: **VERIFIED** = read in this session at path:line. **VERIFIED (earlier)** = read in an earlier session and not re-read. **INFERRED** = reasoned, not run. **CITED** = taken from a record. Mutation rows are proposals: each `old` anchor is checked by `anchor.py` at implementation (Rust token anchors, exact substrings elsewhere). "New" means the test does not exist yet.

---

## Changes in rev 2

| Ruling | What it decides | Where it lands in this plan |
|---|---|---|
| P1 (Q1) | No Ceiling entry; no configure-time exception test | §0 Q1; §5 (P-1 removed); §3.4 S14 (exception test removed); S8 owns the descheduled host |
| P2 (Q2, Q10) | The end after a Stop is bounded by `s + tx_in_flight_ns` | §0 Q2, Q10; §3.1 (`Busy::stop`); §3.4 S3; rows N47S4-17, -17b, -17c |
| P3 (Q3) | A dispatch-time move does not change the record; owner guard covers it | §0 Q3; §3.2 (owner rule); §3.5 owner-guard tests (a)(b)(c); F1 replaced; N47S4-05 dropped |
| P4 (Q9) | One `Busy` per transmit stream, in `ezsdr_radio`; a refused burst stays counted | §0 Q9; §3.1; §3.3; F9 inverted; rows N47S4-08, -08b, -13b |
| P5 (Q8) | The existing `configured` mark is the transmit generation; no `Tx.applied` | §0 Q8; §3.2 (mark, generation, publication); S7, S9, F5; rows N47S4-11, -11b, -12, -12b, -12c, -20, -20b |
| P6 (Q4) | Python `repeat` only; no Python `send` | §0 Q4; §3.4 Python rows; §6.2 |
| P7 (Q5) | Configure failures are asynchronous (`radio.COMMAND_REJECTED`, burst id) | §0 Q5; §3.4 S7 (UHD and the Python view) |
| P8 (Q6, Q7) | `radio.timing.tx_start_lead_ns` and `timeline::start_lead_ns`'s transmit arm are deleted | §0 Q6, Q7; §2.2; §6.3 |
| P9 (S6, UHD form) | FakeDevice supports two transmit streams; the UHD form is written | §1 F-14; §3.4 S6 (`ur_23_two_transmit_streams_…`) |
| P10 (part (a) row A1) | Dropped with the reason; the one-clock property is pinned by the type and a test | §2.5 |
| Sol 1 | Busy inputs that differ under #79 are recorded, not fixed; parity uses bookings both accept alike | §0 Sol-1; §5 |
| Sol 2 | Owner-guard tests (a) moved-at-dispatch burst, (b) padded one-sample burst, (c) Stop with held tail | §3.5; rows N47S4-09a, -09b, -09c |
| Sol 3 | S7's fault is armed after prepare; the partial case is added | §3.4 S7; §3.3 (`FakeDevice::arm_tx_apply_refusal`) |
| Sol 4 | Mark published only if not lost and claim held (under the streams lock); loss clears the mark; probes | §3.2; §3.4 S10; rows N47S4-12b, N47S4-20b, N47S4-33 |
| Sol 5 | Inventory with a disposition for every `cut`/`plan`/`at`/`version` user; the plan-reload replacement observes the End | §2.1 (mechanism), §2.3 (inventory), §2.4 (`ur_25_an_end_booked_after_the_first_read…`), row N47S4-37 |
| Sol 6 | N47S4-19 anchors the real admission check; N47S4-12's killer asserts the mark is `None`; snapshot rows use the generation | §4 |
| Sol 7 | Equal-value updates configure (generation-based, deliberate) | §0 Sol-7; §3.2; S9 tests; §6.2 |
| Sol 8 | Python all-or-nothing validation; RM-20's member count and spec 18 §3's start-lead text in the inventory | §3.4 S13; §6.2 |

**Corrections to rev 1** (found while verifying the rulings' questions):

- C1. `Streams::changed()` bumps `version` at every booking and every refusal (`core.rs:660–661`), not only at Stop. Rev 1's F-c said Stop only.
- C2. A transmit timeline has at most one segment, and its only cut is sample 0. `Kind::Stop` is ignored on transmit and `end_at_origin` cuts at the origin. Rev 1 treated mid-stream transmit cuts as live (§0 Q2 of rev 1). See §1 F-2 and §2.2.
- C3. A device loss does not clear `configured` today. Rev 1 did not say so. See §1 F-6.
- C4. `Tx.applied` is dropped (P5); the existing mark carries the generation.
- C5. The Mock's `held` is removed at hand-over (`lib.rs:941`) and at Stop (`lib.rs:1158`); Busy must not read it (§1 F-10).
- C6. The apply guard's previous end is the owner's closed-device-burst front (`tx.rs:139, 166–168`), not the booking end.
- C7. `design/07-radio-model.md:204` (RM-20) says the envelope's `timing` holds exactly six members; deleting `tx_start_lead_ns` makes it five.

---

## 0. Questions and their rulings (decided)

Each question keeps its number from rev 1. The ruling that decided it is named.

**Q1. Apply time versus the booked start (P1).** Decided: no Ceiling entry, no exception test. The argument, with its marked steps:

1. A changed-value burst is booked only after a value receipt at or after the stream's busy-until (Q9's `Busy`). The receipt is at or after the previous burst's exclusive end `e`.
2. The new burst's start is at least one booking lead after its receipt: `DEVICE_LEAD_NS` = 2 ms on the Module (`control.rs:412` books at `self.core.ticks(DEVICE_LEAD_NS)`; `profile.rs:11`). On MockRadio the lead is `min_timed_command_lead_ns` (`lib.rs:1601`), which the UHD profile sets to `DEVICE_LEAD_NS + DELIVERY_ALLOWANCE_NS` (`profile.rs:21`); the Mock's own profile value was not read, so the "at least 2 ms" on the Mock is INFERRED.
3. The apply runs when the owner next steps after device time has passed `e` (the owner's guard). INFERRED: the owner steps promptly, so the apply starts within a few µs of `e`.
4. The Module's configure takes 15–42 µs (T6, CITED from `bench-results.md`, 2026-11-05). So the apply ends about 42 µs after `e`, well before `e + 2 ms`.

The only way the apply ends after the start is a host that stops running for more than about 2 ms: that is S8 (UHD only, `apply_delay` on the manual host, its late policy decides). S8 is the test; no Ceiling entry is written. The orchestrator should note in the review record that the argument rests on step 3 (INFERRED).

**Q2. The end of a Stop-cut repeat (P2, Q10).** Decided: a Provider that hands ahead closes no later than `s + radio.timing.tx_in_flight_ns`. The spec already says this, at `design/07-radio-model.md:167`: "A Provider that hands samples ahead closes at its current handed cursor, no later than `s + radio.timing.tx_in_flight_ns`." (VERIFIED.) After a Stop taking effect at `s`, the stream is busy until `min(busy_until, s + tx_in_flight_ns)`; an open repeat's busy-until is unbounded, so after the Stop it is `s + tx_in_flight_ns`. `tx_in_flight_ns` is 10 ms on x3xx (`x3xx.rs:387`, VERIFIED) and 0 on the ideal layout (`channels.rs:352`, VERIFIED). A value received before `s + tx_in_flight_ns` is refused; at or after it, accepted (EA-16 states the consequence, §6.2).

**Q3. A dispatch-time `SendAsap` move (P3).** Decided: rejected as a record update. The record holds what booking decided, including the booking-time `SendAsap` move (`control.rs:434–436`, VERIFIED (earlier)), and nothing later. Updating it from the owner's dispatch would make the receipt answer depend on the owner's progress again (#79's mechanism). A dispatch-time move happens only when the host was late. The air stays safe through the owner's apply guard (§3.2). Replaces F1 with the owner-guard tests (§3.5). N47S4-05 is dropped.

**Q4. Python `send` (P6).** Decided: no Python transmit `send` in this stage. `Tx.repeat(x, at)` (`python/ezsdr/session.py:469`, VERIFIED) is the only transmit verb, and it takes `frequency=`, `gain=`, `antenna=` (R7 for `repeat` only). The review record names this.

**Q5. A refused configure (P7).** Decided: asynchronous, as note 47 §2.4 already says. Two refusals exist and only one is synchronous:
- A value refused at receipt (a booked stream, an envelope violation, an instant on a transmit key) is refused by the Provider at the Action's receipt and reaches the Python setter as `Rejected` (the existing setter path, EA-16).
- A burst whose configure the device refuses is refused after `repeat()` returned the admitted entry: `radio.COMMAND_REJECTED` with the burst's Action id, kind `tx_burst` (UR-26, the shape of today's refused receive configure). The Kernel change is none. Python's view of it is the event, not an exception (§3.4 S7).

**Q6. Removing `radio.timing.tx_start_lead_ns` (P8).** Decided: delete. Recommendation accepted. Reader list in §6.3.

**Q7. The transmit line's lead (P8).** Decided: delete `timeline::start_lead_ns` and its Tx arm; transmit lines carry `lead: 0`. `begin` reads a line's lead only for `Start` (`timeline.rs:510–521`, VERIFIED (earlier)); transmit lines book only `Stop` and `End`, and `timeline.rs:457` ignores them on transmit. Removing `lead` from the transmit line type is not needed.

**Q8. What the device holds (P5).** Decided: the existing `configured: Option<u64>` mark (`core.rs:367`, sequence mark on receive; `record_configured` `core.rs:654–656`) is the transmit generation. The stream's staging counter (`value_gen`, new, per transmit stream) is bumped by each accepted transmit value, including a value equal to the one before (Sol 7). A burst's snapshot carries `(value_gen, values)`. Prepare's configuration is generation 0 and sets the mark to `Some(0)` (today prepare configures at `provider/mod.rs:517` without setting it). Equal generation → no device call. A failed configure → `None`. No `Tx.applied`. RX keeps the sequence mark; a shared representation is a follow-up, not this stage.

**Q9. The receipt answer (P4).** Decided: one `Busy` per transmit stream, defined once in `crates/ezsdr-radio/src/staging.rs` (§3.1). Each Provider stores one per stream and calls it at its booking, its Stop, its loss or `Provider::stop`, and its value receipt; nothing else. `core.held` stays the duplicate-start reservation for dispatch and is never read for the receipt answer (N47S4-08 stays). A burst refused later (late at dispatch, a refused configure) stays counted to its booked end: the record has no removal. A booking dropped at booking by its late policy is not booked and does not count.

**Q10.** See Q2.

**Sol-1 (#79, parity).** Recorded, not fixed. Where the Mock and the Module accept different bookings, their `Busy` differ. Sol's sequence (a one-sample A at `s`, a 100-sample B at `s + 1` sample under `DropAndFlag`, a value at `s + 5` samples): the Mock drops B at booking by its padded front; the Module accepts B and decides it at dispatch; so the Mock accepts the value and the Module refuses it (Sol, `sol-plan-final.md` §1). Stage 5 removes it by deciding bookings at receipt by one rule. The parity tests use bookings both Providers accept alike (§5). No known-divergence test (as 48 S3's O1). The orchestrator adds Sol's sequence to the #79 comment.

**Sol-7 (equal values).** Decided: generation-based, deliberately. An accepted transmit value always makes the next burst configure, even when the values equal what the device holds (`f1` again; `f1→f2→f1` before a burst). The extra configure falls between bursts under the owner guard and costs 15–42 µs (T6). R4's "configure once" means: no configure for a burst whose generation the device already holds. The spec says so (§6.2).

**Open for the orchestrator (decide before part (b)):**

- **O-1 (ruled: accept the scalar; justification rewritten, rev 3).** After a Stop, `Busy` is `min(e, s + tx_in_flight_ns)`. The rule is bounded by one in-flight window after the Stop, not "conservative" in an unbounded sense. Why 13 and not 5 in the example (`A [0, 5)` in flight, `B [10, 20)` booked, Stop at `s = 3`, window 10): at `s = 3` the window reaches 13; UHD hands a held burst once `k < window_cut` (`tx.rs:419, 485`); MockRadio counts a held burst as handed once its first send fits the window (`mock-radio/src/lib.rs:1376–1380`); RM-16 then closes the stream at its handed cursor (`design/07-radio-model.md:167`), so B's `[10, 13)` may already be on the air. 13 is the tightest bound knowable at receipt; an exact answer needs the owner's progress (#79's mechanism). Rows N47S4-17 and -17b pin the bound. RM-21 says so (§6.2).
- **O-2. The owner's replacement for the plan re-read (Sol 5).** Removing `version` and `plan_section` takes away the only way the owner learns of a booking-time End (§2.2). **Decided here: the owner reads the stream's single segment at each `step`, under the streams lock, and sets nothing else.** The alternative (the owner reads `clocks.ended_at` directly) is a second, unshared representation of the end. Cost: one lock per step (INFERRED: not measured). The lock-free device-call test (`tx.rs:3347`) still pins that no lock is held across a device call.
- **O-3. The mid-stream cut arithmetic in `send` (B4).** The timeline never produces a transmit cut other than sample 0 (§1 F-2), so the arithmetic is unreachable from production. **Decided: delete it in part (a)** (the B4 rule, Sol 5: "unreachable → delete with evidence"). RM-16 already says the transmit Stop closes at the handed cursor (`design/07-radio-model.md:167`), not at a planned cut. The orchestrator may reverse this; the cost of reversal is the tests in §2.3 that are marked "retained".
- **O-4. `Clock::at_or_after` overflow.** Test `ur_25_largest_aligned_cut_is_safe_in_tx_owner` (`tx.rs:3119`) guards the arithmetic at `i64::MAX`. It is deleted with the cut arithmetic (O-3). Whether `Clock::at_or_after` has its own overflow test was not checked (§7). If it has none, the orchestrator decides whether to add one.

---

## 1. Facts this plan relies on

| # | Fact | Status | Source |
|---|---|---|---|
| F-1 | `Streams::end` books `Kind::End` on every stream (receive and transmit) with the same instant; a loss uses `seq` 0 | VERIFIED | `crates/ezsdr-radio-uhd/src/provider/core.rs:561–571` |
| F-2 | The timeline ignores `Kind::Stop` on transmit (the catch-all arm `Kind::Stop` with `Kind::Start` does nothing for transmit); `Kind::End` on transmit only sets `ended`. The only transmit cut is `end_at_origin`, at sample 0 | VERIFIED | `crates/ezsdr-radio/src/timeline.rs:441–448, 457, 465–468, 499–503`; `segment.cut` is set only in `cut()` (`:487`) |
| F-3 | The transmit clock is ended at booking only when the line's first segment has a cut (an End at or before the origin, S ≤ O); after the origin no cut, no registry end; a booking after the end does not end it again | VERIFIED | `core.rs:660–672` (`changed()`); test `core.rs:2063–2081` (S = O−4 000 and S = O end at O; S = O+400 open; loss ends at O) |
| F-4 | `changed()` bumps `version` at every booking and refusal, not only at Stop | VERIFIED | `core.rs:660–661` (`changed` called from `book` `:534` and `refuse` `:547`) |
| F-5 | `configured` is cleared at `record_open` (`core.rs:627`) and at `record_refused` (`core.rs:649`); `record_configured` sets it (`core.rs:654–656`). A device loss does not clear it | VERIFIED | `core.rs:623–656`; the loss path `core.rs:942` (VERIFIED (earlier)) books no clear |
| F-6 | A streamer reopen restores the configured mark it read before the reopen, when the reopen succeeds | VERIFIED | `tx.rs:826` (read), `:838–839` (`record_configured(key, configured)` on `Ok`), `:849` (`give_back`) |
| F-7 | `reopen_streamer` halts on a failed reopen (`halt_failed_reopen`); a loss calls `chain.lose()` | VERIFIED | `tx.rs:840–848` |
| F-8 | The owner reads the plan at `run` (`plan_section` at `tx.rs:301`), and `plan_section` reads `version` (`tx.rs:329–337`); `retarget` sets `cut` from the plan (`tx.rs:340–342`) | VERIFIED | `tx.rs:296–309, 329–342` |
| F-9 | `step` ends the clock when no burst is open and `now ≥ cut`; the window cut and `cut_k` are computed each step | VERIFIED | `tx.rs:416–424` |
| F-10 | MockRadio's `held` is removed at hand-over and at Stop; Busy must use its own record | VERIFIED (earlier) | `crates/ezsdr-mock-radio/src/lib.rs:941, 1158` |
| F-11 | `end_clock` sets `cut = None` before anything else; so a `cut` assigned before an `end_clock()` call is never read | VERIFIED | `tx.rs:1071–1072` |
| F-12 | The transmit frequency envelope check runs at Validate, Prepare and Runtime admission; its transmit branch is `if !*is_rx` | VERIFIED | `crates/ezsdr-radio/src/lib.rs:132` (`CHECK_STAGES`), `:289`, `:292–311`, `:426–447` (branch at `:437`); the admission call `admission.rs:186–199` (VERIFIED (earlier)) |
| F-13 | Python `_param.set_` raises the generic "cannot change during a Run yet" for transmit values and transmit `sample_rate` | VERIFIED (earlier) | `python/ezsdr/session.py:206–226` |
| F-14 | FakeDevice supports two transmit streams: `tx_open(stream, channels)` keys a `TxStreamer` by stream (`device.rs:1694`, the map at `:791`); the test helper `tx_two()` configures two transmit streams with their own clocks (`fake.rs:5389–5396`, `:5607`) | VERIFIED | `crates/ezsdr-radio-uhd/src/device.rs:791, 1694`; `crates/ezsdr-radio-uhd/tests/fake.rs:5389–5396, 5607` |
| F-15 | FakeConfig has no per-direction channel count; its fields are the clock, faults, front ends and rates (`apply_delay` is a `Duration`) | VERIFIED | `device.rs:401–414` |
| F-16 | FakeDevice's `apply` sleeps `apply_delay` on the host clock and logs `apply {dir} {chan} …` to `st.calls` without a tick | VERIFIED (earlier) | `device.rs:368, 1335, 1341` |
| F-17 | The manual-host pattern for an apply delay exists | VERIFIED (earlier) | `device.rs:2418–2421` (`untimed_apply_sleeps_apply_delay_on_the_host_clock`) |
| F-18 | FakeDevice has `RxApplyFails` (a one-shot receive refusal, fired at the first apply) and no transmit apply refusal | VERIFIED (earlier) | `device.rs:357, 1343–1346` |
| F-19 | `FakeFault::TxReport`, `TxAsyncFails`, `FailSend`, `StalledSend` exist; no transmit apply fault | VERIFIED (earlier) | `device.rs:292–325` |
| F-20 | Mid-stream transmit cuts are set only by tests: `tx.cut` writers are `tx.rs:341` (retarget), `:885` (finished), `:1072` (end_clock), and the test sites in §2.3 | VERIFIED | grep of `tx.rs` |
| F-21 | Probes exist: `core.probe` (`core.rs:845`), `test_support::probe` (`test_support.rs:242`), and the reopen probe `tx.rs:828` (`probe("tx_reopen_claimed")`) | VERIFIED | those lines |
| F-22 | The decision lead for a burst booking is `DEVICE_LEAD_NS` on UHD (`control.rs:412–415`); on the Mock it is `min_timed_command_lead_ns` (`lib.rs:1601`, VERIFIED (earlier)); neither reads `tx_start_lead_ns` for a burst | VERIFIED | `control.rs:412–415`; `profile.rs:11, 21` (UHD profile values; the Mock's profile value not read) |
| F-23 | `tx_in_flight_ns` is 10 ms on x3xx and 0 on ideal | VERIFIED | `x3xx.rs:387`; `channels.rs:352` |
| F-24 | Bench rule: a first send of an open device burst of ≤ 28 samples underflows at interpolation ≥ 80 when continuation arrives late; `MIN_OPEN_FIRST_SEND = 32` | CITED | `plan/phase8/bench-results.md` (~319–352); `device.rs:656` |
| F-25 | T5: untimed transmit settings applied ≈ 0.2 ms after a burst's end left its tail intact; 4.6 ms before the end changed the tail at once (both boards) | CITED | `plan/phase8/bench-results.md` ~293–294 |
| F-26 | T6: configure durations 15–42 µs for transmit frequency (raw control-call medians) | CITED | `bench-results.md` (2026-11-05); not the configure path |
| F-27 | `set_tx_rate` and transmit settings do not wait for a burst to finish | CITED (note 47 Q2, UHD v4.10.0.0 URLs); NOT re-read | `plan/phase8/47-no-change-while-streaming.md` §3 Q2 |
| F-28 | A streamer reopen keeps the device's transmit settings | INFERRED (the mark is kept across reopen, F-6) | — |
| F-29 | The Mock's transmit `ChannelModel` keeps history; `Timeline.set` appends (`channel.rs:13–31`); radiated at `channel.rs:78–108` | VERIFIED (earlier) | `crates/ezsdr-mock-radio/src/channel.rs` |
| F-30 | `Timeline.set` history is what a burst's start reads; a later `set` does not change an earlier sample | VERIFIED (earlier) | `channel.rs:13–31` |
| F-31 | UR-23 already says the stream's one clock is registered at `arm` | VERIFIED (earlier) | `design/18-uhd-radio.md` UR-23 |
| F-32 | The `radio.rf_envelope` check is named in the Python test that expects `ezsdr.update_class` today | VERIFIED (earlier) | `python/tests/test_easy_api.py:130–139` |

---

## 2. Part (a): B4, one transmit clock per stream (no behaviour change)

Part (a) is a refactor and lands first, with the tree green after it. Its test gate is the existing suite minus the tests named here, plus the replacements.

### 2.1 Mechanism: what the owner learns of a clock end, and how it will after part (a)

Today (VERIFIED, F-3 and F-8): a booking-time End at or before the origin ends the registry clock at the origin (`core.rs:666–672`). The owner learns of it only by re-reading the plan: `plan_section` sees a new `version` (`tx.rs:330`), copies the segment, and `retarget` sets `cut` to the origin (`tx.rs:341`). The next `step` then ends the clock (`tx.rs:421–424`), which cancels held bursts and releases their reservations (`end_clock` `tx.rs:1071–1083`; `cancel_unstarted` `tx.rs:345`).

Removing `version` and `plan_section` removes that path. The replacement (O-2, m1) is: `pass` reads the stream's one segment's cut in the critical section it already takes (`crates/ezsdr-radio-uhd/src/provider/tx.rs:297`), through `streams.line(key)…plan.first()…cut`, not `streams.planned(key)` (which clones every segment's settings, `core.rs:596–608`), and stores it as `Tx.cut` in root ticks. `step` uses the field as today. An End booked after the owner's previous pass is seen at the next pass, the same observation as today.

**Clock end (rev 3 statement, from the code).** UHD ends a transmit registry clock only at booking, in `Streams::changed` (`core.rs:666–672`): an End at S ≤ O, where O is the origin, ends the registry clock at O. After the origin the registry never ends it. The owner's `end_clock` (`tx.rs:1071`) ends only its own clock. R1's phrase "the clock ends at the stream's end" is looser than this and is not used.

**Why the origin cut is the only one (O-3, VERIFIED).** The only producer of a transmit cut is `end_at_origin` (`timeline.rs:465–468`), which cuts at sample 0 (`:480–487`). Paths: a loss (`core.rs:942`) and `Provider::stop` (`mod.rs:694`) reach `Streams::end`; a transmit `Stop` (`control.rs:276–279`) books `Kind::Stop`, which the transmit side ignores (`timeline.rs:457`). So a cut exists only when S ≤ O, and S ≤ O holds only within one lattice step of arm (`mod.rs:563–565`, `:692`). In that step no burst can be held or open: bursts are refused before start-up (`control.rs`, "begins before the device's start-up") and the control thread polls every 1 ms; after a loss the owner sends nothing (`tx.rs:272, 402`). So the guards that a non-origin cut would need (`tx.rs:440, 445, 460, 463, 482, 499` with `cut_k`) are unreachable, and so is every mid-stream cut arithmetic in `send`. (INFERRED: the timing bound, from the lattice and the poll interval.)

### 2.2 Removed pieces and why nothing reaches them

| Removed | Where (path:line) | Why nothing reaches it after part (a) |
|---|---|---|
| `Tx.plan: Vec<Planned>` (one section per clock) | `tx.rs:214–216` (field), writers `:334, :880`; `plan_section` callers `:301, :1645, :1895` | One segment per transmit line (F-2). The owner reads it at each `step` (O-2); no copy is kept |
| `Tx::plan_section` | `tx.rs:329–337` | Its job is the version cache (F-4, F-8). With no version and no copy, the owner's read is a plain read |
| `Tx::retarget` | `tx.rs:339–342` (called from `:335`) | Its job is to choose a section and set `cut`; one segment, read in place |
| `Tx.version` | `tx.rs:217, 251, 330, 333, 882` | Nothing needs a version: the owner reads the segment each step (O-2) |
| `Streams.version` and its bump | `core.rs:269, 290, 661` | Its only reader is `plan_section` (F-4); the bump has no other reader |
| `Tx.at` (index of the consumed section) | `tx.rs:218, 252, 881, 1079`; the assertion `tx.rs:5419` | Single segment: `end_clock` took `self.clock` already; `at` is never read but there |
| `Tx.cut` as a field | `tx.rs:213, 249`; writers `:341` (retarget), `:885` (halt), `:1072` (end_clock) | Stays as the field `pass` refreshes (§2.1). Its only reachable non-`None` value is the origin (F-2). The writers `:341, :885, :1072` go; `pass` is the one writer |
| `Tx::refuse_first_send` | `tx.rs:761–768`, called from `send` at `:532` | Dead once the origin is the only cut (O-3): its only trigger is `clock_cut`, and `step` returns at `:445` or `:482` before `send` for an origin cut (`clock_cut` is never `Some` in a `send` call). UR-22's "or, at most 32 samples long, a clock cut" (`design/18-uhd-radio.md:250`) goes with it |
| The mid-stream cut arithmetic in `send` and `step` (O-3) | `tx.rs:419–420` (`cut_k`), `:440`, `:445–448`, `:459–466`, `:482`, `:499`, `:520–554` (`send`'s `clock_cut`, `closes_at_cut`, `h_lt`) | F-2 and §2.1: no transmit cut other than the origin is produced. `held_cut` (the next held burst's start) is **kept**: it is the reachable preemption and burst-boundary arithmetic (`ends_at_held`). The `cut`-based guards go with `cut_k` |
| `StreamState.clocks: Vec<Clock>` | `control.rs:197, 375, 572`; `core.rs:666, 2050`; `tx.rs:1341, 1887` | One clock per transmit stream, registered at `arm` (F-31). Becomes `clock: Option<Clock>`, which cannot hold a second clock |
| Mock `TxStream.clocks: Vec<TxClock>` and `cur` | `crates/ezsdr-mock-radio/src/lib.rs:192–226` (struct), `:229` (`TxClock`); 9 uses of `.cur`, 13 of `clocks` (including test helpers ~2363–2389) | One clock per stream: `registered: Option<TxClock>` (the name `clock` is taken by the declared handle, `lib.rs:201`). `cur` goes; `tx_now` reads `registered` |
| Mock `tx_gen` | `lib.rs:1343` (3 uses: `cut_segments` `:907`, `end_open_segment` `:923`, the key), in the segment key (`design/09` MR-32, `:195`) | Its value was `index + 1` for the one clock, so 1 whenever a clock is current. Removed from the key and from `cut_segments`; the key becomes `(origin, by, k, start)` (`design/09` `:195` changes) |
| Mock `tx_reconcile` (M4) | `lib.rs:1398–1426`, called from `stop_at` at `:1189` | Its registration half runs only when `arm` registered no clock (a stream with no channels has no declared handle, `lib.rs:1985–1989`): it keeps that fallback in the one slot (`registered.is_none()`). Its end half is the Mock twin of `core.rs:666–672` and stays, as `end_tx_clock` (§2.4 `mr_25_a_transmit_clock_ends_at_its_origin_on_a_stop_at_arm`, row N47S4-38). Its per-segment loop goes: the plan has one segment per transmit line (F-2). The front (`remember_tx_front`, `:1391`) stays |
| Mock `tx_now`, `tx_cut` | `lib.rs:1338, 1348` | Kept as one-line accessors of the one clock (`tx_cut` reads the line's first segment's cut) |

### 2.3 Every user of `cut`, `plan`, `at` and `version`, with its disposition (Sol 5)

Production users first (F-20 and the grep of `tx.rs` and `core.rs`):

| User | Site | Disposition |
|---|---|---|
| `Tx.cut` read by `step` for the origin end | `tx.rs:419–424` | **Kept**, read from the segment (O-2). Behaviour unchanged: the origin end (VERIFIED, F-3, F-9) |
| `Tx.cut` read by `send` and `step` for a mid-stream cut | `tx.rs:440, 445–448, 459–466, 520–554` | **Deleted** (O-3): unreachable (F-2). Their tests are in the rows below |
| `retarget` | `tx.rs:335, 339–342` | **Deleted**: replaced by the read of §2.1 |
| `Streams::changed` `version` bump | `core.rs:661` | **Deleted** (F-4) |
| `changed` clock end at booking | `core.rs:666–672` | **Kept**. Today's semantics, stated (Sol 5, §2.4): an End at or before the origin ends the registry clock at the origin, at booking; after the origin nothing does; a loss or `Provider::stop` ends it at its own origin rule (F-1, F-3) |
| `halt_failed_reopen` resets `plan`, `at`, `version`, `cut` | `tx.rs:871–885` | **Retained, minus** `plan`, `at`, `version` and `cut` (the reset of `clock` and `tracker` stays) |
| `end_clock` | `tx.rs:1071–1083` | **Kept**; drops `self.cut = None` (F-11) and `self.at += 1` |

Test users (each row names its disposition; a "dead" assignment is one that `end_clock` overwrites before any read, F-11):

| Test | Site | Disposition |
|---|---|---|
| `ur_25_uhd_tx_reads_the_plan_again_when_it_changes` | `tx.rs:1876–1902` | **Replaced**: `ur_25_an_end_booked_after_the_first_read_cancels_the_held_burst_and_releases_its_reservation` (§2.4). The old test observed the origin cut after a re-read; the replacement observes the cancellation and the reservation release, which is what matters |
| `ur_23_a_transmit_clock_the_plan_cuts_at_its_origin_ends_there` | `core.rs:2063–2081` | **Retargeted**: renamed `ur_23_a_transmit_clock_cut_at_its_origin_ends_at_the_booking`; assertions unchanged; setup drops the plan build (it uses `streams.planned`, which stays) |
| `ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts` | `tx.rs:1645` | **Retargeted**: the `plan_section` call becomes the §2.1 read; assertions unchanged |
| `ur_29_eob_loss_at_the_cut_stops_device_calls` | `tx.rs:3082–3098` (`cut` at `:3090`) | **Retained**, renamed `ur_29_eob_loss_at_the_clock_end_stops_device_calls`. The `cut` line is dead (F-11) and goes |
| `ur_25_largest_aligned_cut_is_safe_in_tx_owner` | `tx.rs:3118–3129` (`cut` at `:3126`) | **Deleted** (O-3, O-4): the value `i64::MAX`-aligned is unreachable (F-2) |
| `ur_28_delayed_time_error_uses_its_original_clock` | `tx.rs:3322–3344` (`cut` at `:3332`) | **Retained**; the `cut` line is dead (F-11) and goes |
| `ur_25_the_line_lock_is_free_inside_every_transmit_device_call` | `tx.rs:3347–3373` (`cut` at `:3367`) | **Retained**; the `cut` line is dead and goes. Its comment "the clock's end at its cut" becomes "the clock's end" |
| `ur_23_a_clock_ending_at_its_cut_cancels_a_held_burst_and_releases_its_reservation` | `tx.rs:3513–3531` (`cut` at `:3520`) | **Retained**, renamed `ur_23_a_clock_end_cancels_a_held_burst_and_releases_its_reservation`. Its `cut` line is dead and goes |
| `ur_23_txs_cut_and_stop_do_not_send_past_a_high_rate_frontier` | `tx.rs:4503–4532` | **Split**: the first half (`cut` at `:4512`) is **deleted** (O-3). The second half (`TxCmd::Stop`, `:4522–4531`) is **retained** as `ur_23_txs_stop_does_not_send_past_a_high_rate_frontier`. The Stop path is reachable |
| `ur_23_a_clock_cut_leaving_n_closes_the_device_burst_with_that_send` | `tx.rs:4884–4894` (`cut` at `:4890`) | **Deleted** (O-3): the send arithmetic it exercises is deleted. Its reachable equivalent is the preemption test below |
| `ur_23_a_preemption_cut_leaving_n_closes_the_device_burst` | `tx.rs:4896–4910` (no `cut`) | **Retained**, renamed `ur_23_a_preemption_leaving_n_closes_the_device_burst`. It drives the reachable booking-time path (`preempt`) to the same closing send |
| `ur_23_a_cut_before_four_first_samples_is_refused` | `tx.rs:4912–4923` (`cut` at `:4917`) | **Deleted** (O-3, rev 3; rev 2's retarget is withdrawn, because the preemption path pads and sends, `tx.rs:4896–4909`, so the retarget would fail). Its refusal (`refuse_first_send`) has no reachable trigger (§2.2). No replacement: the reachable preemption test below stays unchanged. `design/18-uhd-radio.md:263` drops its name (§6.2) |
| `ur_23_m14_a_cut_is_not_waited_past_or_crossed` | `tx.rs:5400–5420` (`cut` at `:5406`) | **Deleted** (O-3): its subject is the wait before a mid-stream cut, which no production path creates. A Stop's close is immediate at the handed cursor (RM-16, `design/07:167`) and is pinned by the retained Stop test above |
| `ur_25_a_transmit_stream_has_one_clock_for_the_run` | new | See §2.4 |
| Mock `mr_09_each_transmit_stream_has_its_own_clock_and_line` | `crates/ezsdr-mock-radio/tests/mock_layout.rs:537` | **Stays** (per-stream clocks are the point) |
| Mock internal helpers using `clocks` push and index | `crates/ezsdr-mock-radio/src/lib.rs` ~2363–2389 | **Retargeted** to `clock` |
| `a_cancelled_held_burst_releases_the_shared_reservation` | `lib.rs:2375` | **Stays** (reservation release on cancel) |

### 2.4 Part (a) tests

| Test | File | Status | Assertion and observation | Wrong behaviour it fails | Killing row |
|---|---|---|---|---|---|
| `ur_25_an_end_booked_after_the_first_read_cancels_the_held_burst_and_releases_its_reservation` | `tx.rs` (replaces `:1876`) | **new** (replacement) | Setup as `:1876–1890` with a held burst at 20 000 and `core.held` containing it. The owner steps once (no cut yet). Then `lock(&core.streams).end(&core, 500_000_000, false, false)` (S ≤ O: a registry end at the origin, F-3). Time advances to the origin; `tx.step()`. Observed: `tx.held` empty; `core.held` no longer contains `(domain, 20_000)`; rejected reason `"UR-25: its transmit clock ended before the burst could start"`; no `tx_send` in `device.calls()` | the held burst survives the End (reservation leaks); the End is seen only after a re-read | N47S4-37 (per-step read removed) |
| `ur_23_a_transmit_clock_cut_at_its_origin_ends_at_the_booking` | `core.rs:2063` (renamed) | **retargeted** | `planned(...)[0].segment.cut` is `Some(0)` for S ≤ O and `None` after; the registry end is O for S ≤ O (`tx_clock_end`) | the clock is not ended at booking (`P1398`) | P1398 (anchor kept) |
| `ur_25_a_transmit_stream_has_one_clock_for_the_run` | `tx.rs` (new) | **new** | Two bursts after `arm` on stream 0; `streams.stream(key).clock` is `Some(c0)` before and after; both bursts target `c0.domain` | a second clock is registered per burst (a `Vec` coming back) | N47S4-35 (P136 registration) |
| `ur_23_a_burst_after_a_stop_targets_the_one_clock` | `fake.rs` (new, part (b) rig) | **new** (also S11) | Stop, then a value, then a burst; the burst's domain equals the arm clock's | a second clock (P10 dropped row A1; this test is its replacement) | N47S4-35 |
| `mr_09_a_burst_after_a_stop_targets_the_arm_clock` | `crates/ezsdr-mock-radio/tests/mock_layout.rs` (new) | **new** (also S11) | As above, on the Mock | as above | N47S4-35 |
| `ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts` | `tx.rs:1645` | **retargeted** | the `plan_section` call becomes the §2.1 read (`tx.read_cut`, a helper `pass` also calls); assertions unchanged | a held burst survives a failed reopen | no new row; the rows that already killed it are unchanged (the test's anchor is `tx.rs:871–885`, which §2.3 edits only for `plan`, `at`, `version`, `cut`) |
| `rm_26_the_timeline_cases` (new case: a transmit abort, m10) | `crates/ezsdr-radio/tests/timeline.rs` | **new case** | an `End { abort: true }` on a transmit line at a booking before the origin ends the clock at the origin as an `End` without abort does (the transmit `End` ignores `abort`, `timeline.rs:442` vs `:446`) | an abort that changes the transmit cut | N47S4-36 (new row, §2.5) |
| `mr_25_a_transmit_clock_ends_at_its_origin_on_a_stop_at_arm` (M4) | `crates/ezsdr-mock-radio/tests/mock_radio.rs` (new; the Mock twin of `core.rs:2063`) | **new** | arm at `t`, `Provider::stop` at `t`: the SampleClock record for the transmit stream ends at `t` (`clocks.sample_clock_records()`), and the layout row's `ended_at` is `t` (`mock_layout.rs` reads it; the record shape of `mock_layout.rs:337`) | the Mock keeps the transmit clock open (no `end_tx_clock` end), or the layout row is not written | N47S4-38 |

### 2.5 Part (a) mutation rows

| Row | Status | Change |
|---|---|---|
| `J59` (`tx.rs`, `if self.version == Some(streams.version) {`) | **goes** with `version` | — |
| `P262` (`core.rs`, `self.version += 1;`) | **goes** | — |
| `P1398` (`core.rs`, `core.clocks.end(clock.domain, core.at(clock.instant(cut)))`) | **retarget**: anchor kept; killer is `ur_23_a_transmit_clock_cut_at_its_origin_ends_at_the_booking` | — |
| `P1399` (`core.rs`, `is_ok_and(|domain| domain.ended_at.is_some())`) | **retarget** to the renamed test | — |
| `P478` (`tx.rs`, `let cut_k = self.cut.map(|cut| clock.at_or_after(cut));`) | **goes**: its anchor is the deleted arithmetic (§2.2) | — |
| `P1166` (`tx.rs`, `if h <= next && cut_k.is_none_or(|cut| h < cut) {`) | **re-anchored** to `if h <= next {`; the mutant is `if h < next {`; killer unchanged (`ur_23_txs_late_preemption_uses_the_handed_frontier`) | — |
| `P1111` (`tx.rs`, `let cut = [held_cut, clock_cut]…`) | **re-anchored** to the surviving line `if let Some(cut) = held_cut {`; the mutant is `if let Some(cut) = None::<i64> {` (the next held start ignored); killer `ur_23_m4b_the_buffer_before_a_held_start_is_not_split` | — |
| `P1112` (killer `m14`, `ur_23_m14_…`) | **killer re-pointed**: `m14` is deleted (§2.3); the anchor is the same surviving line as P1111, the mutant is `if let Some(cut) = None::<i64> {` with killer `ur_23_txs_a_held_cut_splits_a_high_rate_repeat_at_the_target` | — |
| `P263` (`tx.rs`, the `send` cut branch) | **goes**: the `clock_cut` branch is deleted with `send`'s arithmetic (O-3) | — |
| `P656` (`tx.rs`, `let device_eob = eob && (!ends_at_held || closes_at_cut) && !hold;`) | **re-anchored** to `let device_eob = eob && (!ends_at_held || short_first) && !hold;`; the mutant drops `|| short_first`; killer `ur_23_a_short_first_send_closes_before_a_booking_at_its_end` | — |
| `N48S1-11`, `N48S3-12` (`tx.rs`, the `refuse_first_send` guard) | **goes**: its anchor and killer (`ur_23_a_cut_before_four_first_samples_is_refused`, `ur_23_a_clock_cut_leaving_n…`) are deleted | — |
| `N48S3-13` (`tx.rs`, `let eob = ends_waveform \|\| ends_at_held \|\| closes_at_cut;`) | **goes**: its mutation removes a term that no longer exists. Its reachable behaviour (a short first send that a held start follows is closed) is killed by `P656` and `ur_23_a_preemption_leaving_n_closes_the_device_burst` | — |
| `P509` (`tx.rs`, `if self.close_device_burst(tail, cursor, clock) {`) | **stays**, name only changed (the close on a clock end is kept) | — |
| `F36`, `F40` (the lead helper) | **retargeted** to the transmit-line lead of 0 (P8); the swap is the rx/tx lead swap | — |
| `P11` (`timeline.rs`, the Tx arm of `start_lead_ns`) | **retired** (P8) | — |
| `P569`, `P570` (Mock/UHD "cold-change report policy", anchors `CancelReason::Stop` / `ProviderStop`) | **stay**, rename to "UHD TX Stop uses the wrong cancel reason" | — |
| `P563`, `P565` (Mock, same naming) | **stay**, rename | — |
| `P136` (`lib.rs`, `let line = self.line(Direction::Tx, k, origin, &self.config)?;`) | **stays** (per-stream clock registration at arm); re-anchor if the `clocks` push text changes. Killer of N47S4-35 | — |
| `P1395` (`control.rs`, `ColdConfig::of(snapshot, id)`) | **stays** (part (b) reuses the pattern) | — |
| Name-only stale rows at `mutations.json` ~6640–6690 ("cold-change cancellation", "Mock loss uses the cold-change report policy") | **rename** | — |
| **N47S4-A1** (rev 1: a second clock registered per burst) | **dropped** (P10, rev 3 confirms). Part (a) has no code that registers a clock in the burst path. The property is pinned by the type (`clock: Option<Clock>`) and by the test below, killed by N47S4-35 | — |
| **N47S4-35** (new, `control.rs` `book_burst`, `:375`) | the burst's target clock is not the stream's one registered clock: `streams.stream(stream).clock.zip(` → `streams.stream(stream).clock.filter(\|_\| false).zip(` (every burst refused as having no channel). Killer: `ur_25_a_transmit_stream_has_one_clock_for_the_run` (two bursts after arm are both accepted on `c0.domain`, and `streams.stream(key).clock` is `Some(c0)` before and after) | — |
| **N47S4-36** (new, `timeline.rs`, the transmit arm of `Kind::End`) | a transmit `End` that aborts ends no clock: `Kind::End { .. } => self.ended = self.ended.or(Some(item.e)),` → `Kind::End { abort } => self.ended = self.ended.or(Some(item.e)).filter(|_| !abort),` (VERIFIED anchor: `timeline.rs:445`; the transmit arm ignores `abort` today). Killer: the m10 case of `rm_26_the_timeline_cases` (its abort case expects the origin end, which the mutant lacks) | — |
| **N47S4-37** (new, `tx.rs`, `pass`) | the owner's pass never sets the cut: anchor `self.cut = self.clock.and_then(` in `read_cut` (`tx.rs:319`) → `self.cut = None; let _ = self.clock.and_then(`. The cut stays `None` after every pass, so an origin end is never seen by the owner. Killer: `ur_25_an_end_booked_after_the_first_read_cancels_the_held_burst_and_releases_its_reservation`, which drives `pass()` (O-2), not `step()` | — |
| **N47S4-38** (new, `crates/ezsdr-mock-radio/src/lib.rs`, `end_tx_clock`) | the transmit clock end at the origin is not written: the `clocks.end(clock.domain, TimePoint::new(root, at))` call of the end half is removed (anchor `clocks.end(clock.domain, TimePoint::new(root, at))`, exact substring, → `Ok::<(), ezsdr_kernel::time::TimeError>(())`, the `map_err` after it still compiles). Killer: `mr_25_a_transmit_clock_ends_at_its_origin_on_a_stop_at_arm` | — |

### 2.6 Implementation notes and deviations (part (a))

Recorded when part (a) landed (2026-11-06). Line numbers are the tree after this change.

- **Kept, not removed (rev 2 → rev 3 deviation).** Retained tests that set `tx.cut` and then call `step()` are not dead: `step` reads the field at its origin-end check (`tx.rs`, the `open.is_none() && now >= cut` test). Those writes stay. Only the writes whose test no longer reaches a cut were removed (`ur_23_a_clock_ending_at…`, `ur_29…`, `ur_28…`, `ur_25_the_line_lock…`, rev 3 §2.3).
- **Deleted with no replacement (O-3).** `ur_23_a_cut_before_four_first_samples_is_refused` (`refuse_first_send` has no reachable trigger). Listed in `renames.txt`.
- **Guards unreachable: INFERRED.** §2.1's timing bound (a cut exists only when S ≤ O, and that holds only within one lattice step of `arm`) rests on the lattice rule and the 1 ms control-thread poll. It was not measured. The removal of the mid-stream cut arithmetic in `send` and `step` (`cut_k`, `clock_cut`, `closes_at_cut`, `h_lt`) follows from it.
- **`mr_09_a_burst_after_a_stop_targets_the_arm_clock` and `ur_23_a_burst_after_a_stop_targets_the_one_clock` (S11) have no value step.** The plan's sequence is stop, then a value, then a burst. The Mock test has no value-update helper in its harness for the transmit side, and the fake test's Session has no value verb used elsewhere in `fake.rs`. The one-clock property does not depend on the value, so the sequence is stop then burst. The value step belongs to part (b) (its S-sequences). The UHD fake test asserts one clock record for `usrp/tx/s0` and no rejection; the Mock test asserts the clock list and domain.
- **Mock segment key.** `TxPlan.segments` key is `(origin, by, stream, start)` (`channel.rs:52`; `design/09` MR-32 updated). `generation` is gone because every segment of a stream is on its one clock. `registered: Option<TxClock>` is the Mock's one-clock slot (named `registered` because `clock` is the declared handle).
- **Rows, re-anchored by context, not by a moved line.** `P1117` now anchors on `self.open = Some(Open { held, next, first: true });` plus the two lines after it (the `send` call site at `tx.rs:475`), with the mutant returning `true` after the send; its old text `…cut_k, window_cut` no longer exists. `P1205` anchors on `streams.finished(key);` + `self.clock = None;` (`halt_failed_reopen`), because `self.clock = None;` also appears in `end_clock`. `P1166` anchors on `if h <= next {` (`tx.rs:417`). `P656` anchors on `(!ends_at_held || short_first)`. `P1111` and `P1112` both anchor on `if let Some(cut) = held_cut {` (`tx.rs:499`); they carry the same mutant with two killers, as §2.5 says. `P1112`'s name is updated: its old name ("a cold change's cut") described the removed clock cut.
- **Rows in `plan/phase3` and `plan/phase7` (the Mock's `generation` key and the cut arithmetic).** `phase3` `M45` (the segment's origin shifted) anchors on `channel::Segment { origin, ratio, start, len: length,` (`lib.rs`, the production insert); `M49` anchors on the `cut_segments` loop (`if *stream == k {`, its generation test gone); `M68` anchors on the same loop with `|| true`; `M69` anchors on `cut_segments`' `retain`, without the generation term. `phase7` `U17` (the repeat's held cut) anchors on `pass`'s held-cut line and its `chain.next()` follow-on (the `cut_k` argument gone); `B03` anchors on `device_eob` with `short_first` (the `closes_at_cut` term gone). Killers unchanged. Probes in `probes-a/run2.log`.
- **Removed tests, with reasons.** Each name in `renames.txt` as `-` is in §2.3's table with its reason; the renamed names are in the same file.
- **Retargeted filters.** `P1398` and `P1399` name `ur_23_a_transmit_clock_cut_at_its_origin_ends_at_the_booking` (the rename of `core.rs:2059`).
- **Spec text changed.** `design/18` UR-22 (the clock-cut clause of the first-send rule removed), UR-23 ("reading it again whenever it has changed" → "reading the cut at each of its turns"), the test-name list and the test table (`ur_25_uhd_tx_reads…` replaced, `ur_25_a_transmit_stream_has_one_clock_for_the_run` added), and a revision row. `design/09` MR-32 (segment key, `generation`). `KNOWN_LIMITATIONS.md` (the floors entry's sentence on a cut burst cancelled under four samples, which described the removed refusal).
- **Open question, not changed.** `KNOWN_LIMITATIONS.md` (the preemption entry) says a booking that starts 1–3 samples into a long burst cancels the cut burst. In HEAD `refuse_first_send` fired only for `clock_cut`, not for the held cut, so a held cut of 1–3 samples is padded and sent, and its booking is late. That parenthetical was not verified by a run and is left as it was: it is outside part (a)'s change.
- **Verification.** Before the rows: `cargo test -p ezsdr-radio -p ezsdr-mock-radio --all-targets` and `cargo test -p ezsdr-radio-uhd --all-targets` both pass (363 lib, 338 fake; ezsdr-radio and ezsdr-mock-radio all green).

---

## 3. Part (b): transmit values change on an empty stream

### 3.1 The value rule, written once (R2, P4, P2)

New pure code in `crates/ezsdr-radio/src/staging.rs`, beside `Staging`:

```
pub struct Busy { until: Option<Until> }        // None: nothing booked
enum Until { At(i64), Open }                     // At: exclusive end; Open: a repeat with no Stop yet

impl Busy {
    pub fn book(&mut self, start: i64, length: Option<i64>)  // length None: an open repeat; start after any booking-time move
    pub fn stop(&mut self, at: i64, in_flight: i64)           // a Stop, a loss or Provider::stop taking effect at `at`
    pub fn stageable(&self, receipt: i64) -> bool             // None, or At(e) with e <= receipt
}
```

- `book`: `until` becomes the later of the old and `start + length`; `Open` dominates. Nothing removes a booking: a refused burst stays counted to its booked end (P4).
- `stop`: `At(e)` becomes `At(min(e, at + in_flight))`; `Open` becomes `At(at + in_flight)`; `None` stays `None` (nothing is in flight). This is P2's bound. Conservative after a cancelled booking (O-1).
- `stageable`: `Refusal::TxBusy { until }` (a new variant beside `Refusal::{Instant, NotStaged, NoStream, Streaming, StartWhileStreaming}`, `staging.rs:17–43`) when it returns false. Receive keys keep their `streaming` closure (`staging.rs:51–93`); transmit keys use `Busy`.
- The receipt instant is the Action's `now` at the Provider. The test supplies it; no wall clock.
- **Units (m5, rev 3):** `Busy` holds root ticks. A burst's exclusive end is `clock.instant(start_k + len)`; `stop(at, ticks(in_flight_ns))`. The same on both Providers.

Where each Provider calls it (P4):

| Event | UHD (`ezsdr-radio-uhd`) | Mock (`ezsdr-mock-radio`) |
|---|---|---|
| A burst is booked (after any booking-time move) | `control.rs` `book_burst` (~411–458), `book` | `lib.rs:1358` (MR-17 window) |
| A Stop takes effect | `control.rs` Stop arm (`:262–293` booking) | `lib.rs:1126` (MR-25 window) |
| Loss or `Provider::stop` | `Streams::end` (`core.rs:561–571`) per transmit stream | the same instants in the Mock's stop path |
| A value's receipt | `control.rs` (the function that books bursts; line to confirm at implementation) | `handle_update`, `lib.rs:1782` |

Each Provider stores one `Busy` per transmit stream: UHD in `StreamState` (`core.rs`), Mock in `TxStream` (`lib.rs:192–226`). The Mock's accepted-burst record is this `Busy` (its `held` is not used, F-10). The two answers agree because both feed the same accepted bookings through the same rule. The parity tests use bookings both accept alike (§5, Sol 1).

### 3.2 Owner rule, generation, mark and publication (R4, P5, Sol 4, Sol 7)

**Value generation.** `value_gen: u64` per transmit stream (a new field beside the staged values). Each accepted transmit value bumps it, including a value equal to the previous one (Sol 7). A burst's snapshot is `(value_gen, values)` taken at booking (R3; `ColdConfig::of(snapshot, id)`, the receive pattern at `control.rs`, `P1395`).

**Mark.** `configured: Option<u64>` (existing, `core.rs:367`) holds the generation the device holds on every channel of the stream. Prepare's configuration sets it to `Some(0)` on success (Q8). Changes:
- a failed configure of any channel sets it to `None`; a partial success is not recorded (§3.4 S7 partial case);
- a loss clears it (F-5 today does not; this stage adds it, Sol 4);
- a reopen restores the value it read before the reopen (F-6, unchanged);
- `record_open` clears it (F-5, unchanged).

**Arm publishes generation 0 (M1).** `prepare` configures at `provider/mod.rs:517`; `arm` then opens the transmit streamer (`mod.rs:559` → `core.rs:1248` `record_opened` → `record_open`, which sets `configured = None`, `core.rs:627`). So arm publishes `Some(0)` after the transmit open and its rate check succeed, under the streams lock. (INFERRED, as F-28: the open keeps the device's radio settings.) Without it, the first burst with no new value makes an `apply tx` (`ur_25_a_burst_whose_snapshot_equals_the_device_s_makes_no_apply` fails).

**The configure is a turn of its own (M2).** `TxCmd::Stop` is read only at the top of the run loop (`tx.rs:265–271`), and `Holder` has only `Free` and `Owner` (`core.rs:324–330`). So the owner's configure does not end in the same step as the burst's dispatch: on success it publishes `Some(g)` only if `!is_lost()`, under the streams lock, and returns with the burst still held. The next turn drains the commands (a `Stop` → `stop_now`, `CancelReason::Stop`) and re-decides the burst with `dispatch_start` (R4). A `Stop` during the configure therefore leaves the device holding `g` and the mark `Some(g)`, and the burst is cancelled before it is sent. The "owner still holds its claim" condition is dropped: it is always true, since only the owner claims (`core.rs:417–424`).

**Publication.** The owner publishes `Some(generation)` only under the streams lock, and only if the stream is not lost (a loss clears the mark, Sol 4). Otherwise it publishes nothing. Probes sit between channels and before publication (§3.4 S10): the existing `apply` probe between channels (`core.rs:1012`) and `core.probe("tx_configure_before_mark", …)` (the pattern of `core.rs:845`).

**Between-burst configure passes `rate: None` (m4).** `ColdConfig` sets the rate (`control.rs:73`), and UHD's `apply` calls `uhd_usrp_set_tx_rate` whenever the rate is `Some` (`uhd.rs:895–899`). The rate is fixed for the Run (R1); T5 covered frequency and gain only (`bench-results.md:293`). So the between-burst transmit configure passes `rate: None`, and keeps the read-back check where it is today.

**The snapshot carries its ActionId (m6).** `Held` carries no ActionId (`control.rs:458`). The burst's snapshot is `ColdConfig::of(snapshot, id)` on `Held`, as `start_rx` does, so that R5's refusal names the burst's Action.

**Apply rule.** A pure function in `tx.rs`, table-tested (no device):

```
fn apply_due(mark: Option<u64>, wanted: u64, device_open: bool, device_now: i64, previous_end: Option<i64>, recovering: bool) -> bool
```

It is true iff `mark != Some(wanted)`, `!device_open`, `!recovering`, and `previous_end.map_or(true, |e| device_now >= e)`. `previous_end` is the end of the owner's own last closed device burst, read from `chain.front()` (`tx.rs:139, 166–168`), not the booking end (C6). It is the device's actual end, including a moved start and the padding (§3.5).

**Consequences** (stated for the spec, §6.2):
- Equal generation → no device call (R4's "configure once"). A new generation with equal values configures (Sol 7).
- The configure runs with the owner's claim, before the burst's first send, outside a device burst (R4).
- After the configure, `dispatch_start` decides the start again by the burst's late policy (S8).

### 3.3 FakeDevice additions (for the tests)

- `FakeDevice::arm_tx_apply_refusal(channel: usize, freq: f64)` (M3, rev 3): **persists**, keyed to a value on a channel. Every transmit apply on `channel` whose frequency is `freq` fails with `DeviceError::failed("fake: scripted refusal of a transmit apply")` until the test disarms it (`disarm_tx_apply_refusal()`). It is counted in `fired` beside `RxApplyFails`. Armed by the test after prepare (Sol 3). It serves the second refusal, the partial case (on c1) and the read-back case, and N47S4-16 anchors it. Not a `FakeConfig` fault (F-15). A one-shot could not produce "refused again" (M3: the second burst would configure and be sent).
- **The apply log format does not change (m7, rev 3).** The tests observe the order in `device.calls()` and the manual clock's time at the apply (the tick is `device.time_now()` read at the call, which the test controls). Grep of the `apply ` assertions is not needed.
- `tx_two()` (`fake.rs:5389`) is the two-stream layout for S6.

### 3.4 Sequences S1–S14 and failure scenarios F1–F14

Legend: **new** = to be written; **change** = an existing test whose assertion changes. Each row names the killing row (§4).

#### S1 — connect at f1; burst A plays; after A's end, value f2; burst B

| Test (file · name) | Asserts and observes | Wrong behaviour | Killing row |
|---|---|---|---|
| `staging.rs` · `rm_21_a_transmit_value_at_the_exclusive_end_is_accepted_and_one_sample_before_refused` (S5) | `stageable(e)` true, `stageable(e−1)` false | the boundary moves | N47S4-01 |
| `rm_21_a_transmit_value_after_the_end_is_accepted` (new, pure) | `Busy` with `At(e)`, receipt `e + 1000` → true | refused after the end (too strict) | N47S4-28 |
| `fake.rs` · `ur_23_a_value_set_after_a_burst_ends_is_applied_before_the_next_burst` | the `apply tx 0 0 … freq=f2` entry comes after A's last `tx_send` and its tick is ≥ A's end, and before B's first `tx_send`; no `tx_send` between | apply before A's end (destructive, F-25) or after B's first send (B late) | N47S4-09 (guard removed), N47S4-28 (too strict: no apply ever) |
| `mock_channel.rs` · `mr_33_a_transmit_value_takes_effect_at_the_next_burst_s_start` (new) | A's radiated samples carry f1; B's carry f2 (`radiated` at the burst's indices; the pattern of `mock_channel.rs:722`) | A changes mid-burst, or B keeps f1 | N47S4-18 (history lost), N47S4-28 |
| `test_easy_api.py` · `test_ea_16_a_transmit_value_on_an_empty_stream_is_used_by_the_next_repeat` (new) | after a `repeat` ends, `tx_channels[0].frequency = f2`, then `repeat`; the loopback capture is f2 | stale f1 | N47S4-21 (connect values used), N47S4-28 |

#### S2 — A booked for the future; value before A starts → refused with the Action's id

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `staging.rs` · `rm_21_a_transmit_value_is_refused_while_a_booked_burst_has_not_ended` (new) | `stageable` false, `Refusal::TxBusy { until }` | accepted | N47S4-02 |
| `fake.rs` · `ur_21_a_value_while_a_burst_is_booked_is_refused_with_its_action_id` (new) | event `radio.COMMAND_REJECTED` with `payload.action` = the `UpdateParameter`'s id; no `apply` in `calls`; A plays with f1 | the apply happens; the event names no id | N47S4-02; N47S4-29 (no id) |
| `test_easy_api.py` · `test_ea_16_a_transmit_value_for_a_booked_stream_is_rejected_and_the_admitted_value_differs` (new, modelled on `:829`) | `Rejected` with the event kind; the value is not staged, so a later burst uses f1 | no `Rejected`, or the device uses f2 | N47S4-02 |

#### S3 — repeat playing: value refused; Stop: value after the in-flight window; B uses it

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `staging.rs` · `rm_21_a_transmit_value_is_refused_while_a_repeat_plays_and_taken_after_its_stop` (new) | `Open` → refused; after `stop(cut, in)`: refused at `cut + in − 1`, accepted at `cut + in` | a repeat treated as idle (N47S4-03); the window dropped (N47S4-17) | N47S4-03, N47S4-17 |
| `staging.rs` · `rm_21_a_stop_bounds_an_open_repeat_by_the_in_flight_window` (new) | `Open` then `stop(s, in)` → `At(s + in)` | the bound missing | N47S4-17 |
| `staging.rs` · `rm_21_a_stop_never_extends_a_booked_end` (new) | `At(5)`, `stop(3, 10)` → `At(5)` (`min`) | `max` used: `At(13)`, the value at 6 refused | N47S4-17b |
| `fake.rs` · `ur_23_a_value_is_taken_only_after_a_stopped_repeat_s_in_flight_window` (new) | a value at `cut + in − 1` refused; at `cut + in` accepted; the apply's tick ≥ `cut + in`; B plays with it | apply inside the tail (F3, F-25); the Stop not applied to `Busy` | N47S4-17, N47S4-17c (UHD Stop does not update `Busy`) |
| `mock_radio.rs` · `mr_25_a_transmit_value_after_a_stop_takes_effect_after_the_in_flight_window` (new) | the same numbers, via the Mock's `stop_at` (`lib.rs:1126`) | as above | N47S4-17, N47S4-18c (Mock Stop not applied) |
| `test_easy_api.py` · `test_ea_16_a_transmit_value_after_a_stop_waits_for_the_in_flight_window` (new) | `repeat`, `stop`, a value before the window → `Rejected`; after it → accepted | as above | N47S4-17 |

Note: Q5 applies here too. A value received during the window is refused at receipt (synchronous `Rejected`), not at dispatch.

#### S4 — empty → f2 → f3 → burst: f3; empty → f2 → B booked → f4 refused; B uses f2

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `staging.rs` · `rm_21_the_last_transmit_value_wins_on_an_empty_stream` (new; the pattern of `rm_21_the_last_staged_value_wins`, `staging.rs:54`) | the staged snapshot holds f3 | the first value kept | N47S4-30 |
| `fake.rs` · `ur_25_the_last_value_before_a_burst_is_the_one_it_plays` (new) | exactly one `apply … freq=f3`; no `f2` apply | two applies, or the connect values | N47S4-21 (snapshot from connect values) |
| `fake.rs` · `ur_21_a_value_after_a_burst_is_booked_is_refused_and_the_burst_uses_the_earlier_value` (new) | f4 refused with its id; B's apply is `freq=f2` | f4 accepted | N47S4-02 |
| `test_easy_api.py` · `test_ea_16_the_last_transmit_value_before_a_repeat_is_the_one_it_transmits` (new) | the loopback capture is f3 | f2 used | N47S4-21 |

#### S5 — the boundary: a value at A's exclusive end (accepted) and one sample before it (refused)

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `staging.rs` · `rm_21_a_transmit_value_at_the_exclusive_end_is_accepted_and_one_sample_before_refused` (new) | `e` → true; `e−1` → false | boundary off by one | N47S4-01 |
| `fake.rs` · `ur_23_a_value_at_the_exclusive_end_of_a_burst_is_accepted` (new) | `COMMAND_REJECTED` for `e−1`; accepted at `e`; the apply waits for device time past `e` | boundary shifted on the Module only | N47S4-01 (UHD form) |
| `mock_radio.rs` · `mr_16_a_transmit_value_at_the_exclusive_end_is_accepted` (new) | the same two receipts on the Mock's `handle_update` (`lib.rs:1782`) | boundary shifted on the Mock only | N47S4-01 (Mock form; the `Busy` is shared code, so this row's `old` is in `staging.rs`) |

#### S6 — two transmit streams: stream 0 busy, stream 1 empty

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `staging.rs` · `rm_21_a_peer_stream_s_state_does_not_matter` (change: `staging.rs:70` becomes the transmit form) | two `Busy` values, per stream: the answer for stream 1 is true while stream 0 is `At(e)` | one `Busy` shared | N47S4-31 |
| `fake.rs` · `ur_23_two_transmit_streams_busy_and_empty` (new; `tx_two()`, `fake.rs:5389`; P9) | stream 0 repeats; a value for stream 1's channel accepted and applied on stream 1; stream 0's `tx_send` sequence has no gap | stream 0 disturbed or refused | N47S4-31 |
| `test_easy_api.py` · `test_ea_16_a_value_for_an_empty_transmit_stream_leaves_the_busy_one_undisturbed` (new; `tx=[Stream([0]), Stream([1])]`, `:218`) | stream 0's capture shows no discontinuity | stream 0 refused or disturbed | N47S4-31 |

#### S7 — the device refuses the configure

Post-prepare trigger (Sol 3): the test prepares, starts, then calls `arm_tx_apply_refusal(0)`.

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `fake.rs` · `ur_21_a_refused_configure_refuses_the_burst_and_the_fixed_value_plays_on_the_same_clock` (new) | burst A with f2 → `radio.COMMAND_REJECTED` (kind `tx_burst`, A's id); no `tx_send` for A; no halt-kind event; the mark is `None` (test accessor, the `ur_22_*` pattern); burst A' with f2 → refused again and the configure is tried again (a call in `calls`); a corrected f3 via `UpdateParameter` → accepted (the event shows it); B plays with f3 on the arm clock's domain (the same as A's, the one-clock check) | a sent burst after a refused apply (N47S4-13); a halt (N47S4-14); a kept mark, so A' skips the configure (N47S4-12) | N47S4-12, N47S4-13, N47S4-14, N47S4-16 |
| `fake.rs` · `ur_21_a_partial_configure_refusal_sends_nothing_and_the_retry_configures_both` (new, Sol 3) | two channels; `arm_tx_apply_refusal(1)`: c0 applies, c1 fails → no burst sent; mark `None`; the retry configures **both** c0 and c1 (two `apply` entries) | the mark kept (c0's apply counted as success); the retry configuring only c1 | N47S4-12, N47S4-12c |
| `fake.rs` · `ur_21_a_refused_burst_stays_counted_until_its_booked_end` (new; F9, inverted by P4) | a value received before the refused burst's booked end is refused; at or after it, accepted | the refused burst removed from `Busy` (value accepted early) | N47S4-13b |
Python view of S7 (P7): none on the Mock (P-4, so no Python test row). The Rust test asserts the event; `repeat()` returned the admitted entry.

#### S8 — a configure that ends too late for B's start: the late policy decides

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `fake.rs` · `ur_25_a_slow_configure_makes_a_burst_late_by_its_policy` (new; manual host, `apply_delay` 3 ms) | with `SendAsap`: `TIME_ERROR` and the moved start; with `Drop`: dropped and reported; never a `tx_send` before the apply's end without a report | B sent at its booked start before the apply finished | N47S4-15b |
| `tx.rs` · `ur_25_the_late_decision_uses_the_start_after_the_configure` (new, pure: explicit `device_now`) | `dispatch_start` after the configure returns the moved or dropped result | the decision made before the configure | N47S4-15b |

#### S9 — no apply while a device burst is open; configure once

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `tx.rs` · `ur_23_apply_is_due_only_between_bursts_and_only_for_a_new_generation` (new; table over `apply_due`) | open device burst → false; `device_now < previous_end` → false; `mark == Some(wanted)` → false; recovering → false; all clear → true | any row flips | N47S4-09, N47S4-10, N47S4-11b, N47S4-15 |
| `fake.rs` · `ur_23_a_contiguous_burst_with_the_same_generation_makes_no_configure_call` (new; bursts > 32 samples, F-24) | burst A and contiguous B → one device burst (one `tx_open`), no `apply` after A's first | a configure mid-burst | N47S4-10 |
| `fake.rs` · `ur_25_an_equal_value_after_a_new_one_configures_once` (new, Sol 7) | `f1` (connect) → gain change → `f1` again → one burst → one apply | the generation compared by value (no configure) | N47S4-11 |
| `fake.rs` · `ur_25_a_second_burst_with_no_new_value_configures_nothing` (new) | A with f2 (one apply), then B with no new value → no apply | the mark ignored | N47S4-11b |
| `fake.rs` · `ur_25_a_burst_whose_snapshot_equals_the_device_s_makes_no_apply` (new) | A with the connect value f1 → no apply | the apply made for an unchanged generation | N47S4-11b |

#### S10 — recovery and configure; Stop, loss and `Provider::stop` during or before the configure

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `fake.rs` · `ur_28_a_value_during_an_underflow_recovery_is_applied_after_the_recovery` (new; the existing underflow fault, `device.rs:272–325`) | the apply's tick is after the recovery's last `tx_send` | apply during recovery | N47S4-15 |
| `fake.rs` · `ur_22_a_loss_during_the_configure_clears_the_mark` (new; Sol 4; probe `tx_configure_before_mark`) | the probe pauses the configure after the last channel; another thread publishes loss; the configure resumes: the mark is `None`; no later payload or device call; the claim released | `Some(g)` published after the loss | N47S4-12b, N47S4-20b |
| `fake.rs` · `ur_22_a_stop_during_the_configure_leaves_no_payload_after_it` (new; probe `apply` between channels, M2) | a `Stop` while paused after the last channel's apply: the configure completes, the device holds `g`, the mark is `Some(g)` (the configure is a turn; nothing aborts it, M2); the next turn drains the `Stop` and cancels the burst (`CancelReason::Stop`); no `tx_send` after the Stop; the claim released | a burst dispatched after the Stop (the configure turn does not re-check commands) | N47S4-33 |
| `fake.rs` · `ur_22_a_provider_stop_during_the_configure_leaves_no_half_applied_record` (new; probe as above) | `Provider::stop` during the configure: the mark is `Some(g)` only if the device got every channel (partial: `None`); the claim's release; the burst cancelled with `ProviderStop` (UR-25 allows a start the owner read before the end). Asserts the mark, the claim and the cancellation, not "no payload" | a mark published after the stop without the not-lost check; a claim kept | N47S4-33 |
| `mock_radio.rs` · `mr_25_a_stop_before_the_burst_with_a_new_value_cancels_it_and_keeps_the_value_of_record` (new) | a Stop before the burst is dispatched cancels it; the value stays the value of record | the Mock keeps the burst | N47S4-33 (Mock form: its Stop arm) |

#### S11 — B4: loss and `Provider::stop` before the origin; a burst after a Stop targets the one clock

Covered by §2.4 (`ur_23_a_transmit_clock_cut_at_its_origin_ends_at_the_booking`, `ur_23_a_burst_after_a_stop_targets_the_one_clock`, `mr_09_a_burst_after_a_stop_targets_the_arm_clock`). Killing row N47S4-35.

#### S12 — §58 #16, transmit; sample rate refused; Python names the bench condition

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `crates/ezsdr-acceptance/tests/v58.rs` · `v58_16_a_transmit_value_outside_the_rf_envelope_is_refused_on_an_empty_stream` (new) | `radio.tx.c0.frequency_hz` outside the band → refused at admission naming `radio.rf_envelope`; the Action is logged; no `apply` in `calls` (UHD) and nothing dispatched (Mock) | accepted, then refused at dispatch | N47S4-19 |
| `staging.rs` · `rm_21_a_transmit_rate_is_refused_as_before` (new) | `sample_rate_hz` on a transmit key → `Refusal::NotStaged` | staged | N47S4-07 |
| `test_easy_api.py` · `test_ea_16_a_transmit_sample_rate_names_the_bench_condition` (new) | the transmit stream's `sample_rate` setter raises `Error` with the `BENCH_RATE` text (`session.py:186–197`) | the generic text (F-13) | N47S4-25 |
| change: `test_easy_api.py:130–139` · `test_v58_16_a_rejected_call_raises_and_is_logged` | the idle stream now accepts an in-band frequency; the test sets `2.6e9` and expects `radio.rf_envelope` (not `ezsdr.update_class`); the comment at `:131` is rewritten | the old check name | N47S4-19 |

#### S13 — Python: all-or-nothing and the setters (P6, Sol 8)

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `test_easy_api.py` · `test_ea_16_repeat_with_a_frequency_on_a_busy_stream_raises_and_sends_no_burst` (new) | `repeat(x)` playing, then `repeat(x, frequency={0: f2})` → `Rejected`; the action log has no second `start_repeat`; the first repeat keeps going (the pattern of `test_v3_parameters_through_attributes`, `:163`) | a burst sent after the refused update (update order); a second `start_repeat` logged | N47S4-22, N47S4-23 |
| `test_easy_api.py` · `test_ea_16_repeat_validates_every_channel_before_sending` (new, Sol 8) | valid `frequency={0: f2}` with an invalid second channel (a bad type, or an unknown logical channel), or an invalid waveform: nothing submitted; the action log is unchanged | a valid first value submitted before the second is checked | N47S4-26 |
| `test_easy_api.py` · `test_ea_16_an_admitted_first_value_followed_by_a_refused_second_sends_no_burst` (new, Sol 8) | `frequency={0: f2, 1: out_of_band}`: the first is admitted and stays staged; the second refused; no later value sent; no burst | the first value dropped on refusal; a burst sent | N47S4-27 |
| `test_easy_api.py` · `test_ea_16_a_channel_gain_on_an_empty_stream_is_accepted_and_the_next_repeat_uses_it` (new) | `tx_channels[0].gain = g` on an idle stream accepted; `repeat(x)` → the loopback gain is g | the setter raises (`session.py:221–226`) | N47S4-24 |
| change: `test_easy_api.py:143–163` · `test_v3_parameters_through_attributes` | lines 159–161 (`tx_channels[0].frequency = 2.4e9` expecting `Error`) go; the action-log count `["set_parameter"] * 3` (line 163) becomes four; the comment at `:145` is rewritten | — | N47S4-24 |

#### S14 — parity: S1–S5 decide the same on both Providers

| Test | Asserts and observes | Wrong | Killing row |
|---|---|---|---|
| `crates/ezsdr-acceptance/tests/parity.rs` · `s14_transmit_values_on_an_empty_stream_decide_the_same_on_both_providers` (new) | for S1 (accepted), S2 (refused with id), S3 (refused while a repeat plays; accepted after Stop plus the window), S4 (last wins; a later value refused), S5 (boundary): the same outcome kinds and ids on `mock_binding` and `uhd_binding`. Not the sample values | a decision that differs | N47S4-17c, N47S4-01 |

The Q1 exception test of rev 1 (`s14_a_transmit_burst_starting_inside_the_configure_time…`) is deleted (P1).

### 3.5 Owner-guard tests (Sol 2)

Each holds device time before the relevant end (assert **no apply**), then crosses it (assert the apply before B's first send, with its tick). Manual host (`FakeConfig { clock: ClockMode::Manual, .. }`, as `tx.rs:3351`).

| Test | Setup and observation | Wrong behaviour | Killing row |
|---|---|---|---|
| (a) `fake.rs` · `ur_23_a_value_waits_for_a_burst_moved_at_dispatch_past_its_booked_end` | A is booked to end at E; the host is late, so dispatch moves A to `E + m` (as `tx.rs:1922`); the value is accepted at receipt E (the record still says E, P3); no apply while device time < `E + m`; apply after A's closing EOB, before B's first send | the apply guard uses the booked end (E): apply inside A's moved samples | N47S4-09a |
| (b) `fake.rs` · `ur_23_a_value_waits_for_a_padded_one_sample_burst` | a one-sample payload padded to four (UR-22); the value accepted at its payload end (`Busy` uses the booked payload end); no apply until the padded device burst's end (`chain.front()`) | the guard uses the payload end: apply inside the padding | N47S4-09b |
| (c) `fake.rs` · `ur_23_a_value_waits_for_a_stop_s_held_tail` | a Stop with a two-sample held tail: the value accepted after `min(busy, s + in)`; no apply until the tail's closing end. The value is received exactly at `e` and device time is held in `[e, e+2)` (m9): the close is `close_start + 3` (`tx.rs:748`), so the window is two samples, and a value received later passes without the guard | the guard uses the Stop instant: apply inside the tail | N47S4-09c |

Each row breaks the guard in one place (N47S4-09a, -09b, -09c); each has the named killer above.

### 3.6 Failure scenarios beyond S1–S14 (F1–F14)

| ID | Scenario | Test (file · name) | Wrong behaviour | Killing row |
|---|---|---|---|---|
| F1 | A value after a dispatch-time move (replaced by P3) | §3.5 (a) | apply inside moved samples | N47S4-09a |
| F2 | A value after the last sample was handed over and before the burst's end (#79) | `fake.rs` · `ur_21_a_value_after_the_last_hand_over_and_before_the_end_is_refused` (new) | the answer from hand-over state (`core.held`): accepted | N47S4-08 |
| F3 | A value during a Stop-cut repeat's window | S3 `ur_23_a_value_is_taken_only_after…` | apply inside the tail | N47S4-17 |
| F4 | An underflow recovery when a new-value burst is due | S10 `ur_28_…` | apply during recovery | N47S4-15 |
| F5 | Device loss, then reopen | `fake.rs` · `ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it` (new) | a reopen clears the mark (N47S4-20); a loss keeps it (N47S4-20b) | N47S4-20, N47S4-20b |
| F6 | A burst keeps the values in effect at its start after a later `set` | `mock_channel.rs` · `mr_33_a_burst_keeps_the_values_of_its_own_start_after_a_later_set` (new) | the earlier burst's samples computed with the newer value | N47S4-18 |
| F7 | A value with an instant (`at=`) on a transmit key | `staging.rs` · `rm_21_a_transmit_value_with_an_instant_is_refused` (new) | staged | N47S4-06 |
| F8 | Another channel of the stream while a burst plays | S3 (the rule is per stream) | — | — |
| F9 | A refused burst leaves a booking (P4 inverts rev 1) | S7 `ur_21_a_refused_burst_stays_counted_until_its_booked_end` | a value accepted early | N47S4-13b |
| F10 | Mock and UHD answer for a repeat with no Stop: `Open` on both | S14 | disagreement | N47S4-17c |
| F11 | Python form of F2 (hand-over state) | not testable from Python (no hand-over is visible); the Rust test is the killer | — | — |
| F12 | Snapshot taken at booking vs dispatch (R3) | **no row**: a booked burst's values cannot change (any receipt while booked is refused), so the two times are equivalent. Recorded here, not added (equivalent mutant) | — | — |
| F13 | After a refused configure, the device keeps the old value while the record shows the admitted one | S7 `ur_21_a_refused_configure_…` (Rust); not on the Mock (P-4) | — | N47S4-12 |
| F14 | A short first send (≤ 32 samples) of a burst after an apply (F-24) | not tested here; KL-36's and out of this stage | — | — |

---

## 4. Mutation rows (part (b), new)

Proposed ids `N47S4-nn`. Anchors are checked at implementation. The killing test is named in the last column; every test named in §3 that kills a row appears here.

| Row | File | Mutation | Killing test |
|---|---|---|---|
| N47S4-01 | `ezsdr-radio/src/staging.rs` | `e <= receipt` → `e < receipt` | `rm_21_a_transmit_value_at_the_exclusive_end_is_accepted_and_one_sample_before_refused` (plus `ur_23_a_value_at_the_exclusive_end…`, `mr_16_…`) |
| N47S4-02 | same | `stageable` ignores `At(_)` (always stageable) | `rm_21_a_transmit_value_is_refused_while_a_booked_burst_has_not_ended` |
| N47S4-03 | same | `Open` treated as `None` | `rm_21_a_transmit_value_is_refused_while_a_repeat_plays_and_taken_after_its_stop` |
| N47S4-04 | same | `book` keeps the first end instead of the later one | `rm_21_the_last_booked_burst_decides` (new, multi-burst) |
| N47S4-06 | same | `has_at` ignored for transmit keys | `rm_21_a_transmit_value_with_an_instant_is_refused` |
| N47S4-07 | same | transmit `sample_rate_hz` staged | `rm_21_a_transmit_rate_is_refused_as_before` |
| N47S4-17 | same | `stop` bounds by `at`, not `at + in_flight` | `rm_21_a_stop_bounds_an_open_repeat_by_the_in_flight_window` (plus `ur_23_a_value_is_taken_only_after…`) |
| N47S4-17b | same | `stop` takes `max` instead of `min` | `rm_21_a_stop_never_extends_a_booked_end` |
| N47S4-17c | `ezsdr-radio-uhd/src/provider/control.rs` (Stop arm) | the Stop does not call `Busy::stop` | `ur_23_a_value_is_taken_only_after_a_stopped_repeat_s_in_flight_window` |
| N47S4-18 | `ezsdr-mock-radio/src/channel.rs` | `.set()` keeps only the latest value (history lost) | `mr_33_a_burst_keeps_the_values_of_its_own_start_after_a_later_set` |
| N47S4-18b | `ezsdr-mock-radio/src/lib.rs` | Mock `Busy` fed from `held` (hand-over state) | `mr_21_a_value_while_the_last_burst_is_in_flight_is_refused` (new) |
| N47S4-18c | same (Mock Stop, `lib.rs:1126` window) | the Stop does not call `Busy::stop` | `mr_25_a_transmit_value_after_a_stop_takes_effect_after_the_in_flight_window` |
| N47S4-19 | `ezsdr-radio/src/lib.rs` (the transmit branch of the frequency check, `:437`) | the branch is not reached for transmit keys (`if !*is_rx` → `if *is_rx`) | `v58_16_a_transmit_value_outside_the_rf_envelope_is_refused_on_an_empty_stream` |
| N47S4-21 | `ezsdr-radio-uhd/src/provider/control.rs` (the transmit snapshot at booking, `ColdConfig::of`) | the burst takes the connect values, not the staged ones | `ur_25_the_last_value_before_a_burst_is_the_one_it_plays` |
| N47S4-28 | `ezsdr-radio/src/staging.rs` | `stageable` always false (too strict) | `rm_21_a_transmit_value_after_the_end_is_accepted` (plus S1 and S5 positives) |
| N47S4-29 | `ezsdr-radio-uhd/src/provider/control.rs` | the refusal event omits the Action id | `ur_21_a_value_while_a_burst_is_booked_is_refused_with_its_action_id` |
| N47S4-30 | `ezsdr-radio/src/staging.rs` | the first transmit value kept (last-wins broken) | `rm_21_the_last_transmit_value_wins_on_an_empty_stream` |
| N47S4-31 | same (`Busy` placement) | one `Busy` for every stream of a Provider | `rm_21_a_peer_stream_s_state_does_not_matter`, `ur_23_two_transmit_streams_busy_and_empty` |
| N47S4-08 | `ezsdr-radio-uhd/src/provider/control.rs` (the UHD answer) | the receipt answer from `core.held` (hand-over) instead of `Busy` | `ur_21_a_value_after_the_last_hand_over_and_before_the_end_is_refused` |
| N47S4-08b | `ezsdr-radio-uhd/src/provider/control.rs` (booking, `:434–436`) | the booking-time `SendAsap` move not fed to `Busy::book` | `ur_21_a_value_is_judged_by_a_booking_time_move` (new) |
| N47S4-13b | `ezsdr-radio-uhd/src/provider/control.rs` | a refused burst removed from `Busy` | `ur_21_a_refused_burst_stays_counted_until_its_booked_end` |
| N47S4-09 | `ezsdr-radio-uhd/src/provider/tx.rs` (`apply_due`) | the `device_now >= previous_end` test removed | `ur_23_a_value_set_after_a_burst_ends_is_applied_before_the_next_burst` |
| N47S4-09a | `tx.rs` (`apply_due` caller, the previous end) | `previous_end` → `None` (the guard is dropped): no `previous_end` test; killed by (a), (b) and (c) (m2) | §3.5 (a) `ur_23_a_value_waits_for_a_burst_moved_at_dispatch…` |
| N47S4-09b | `tx.rs:712` | `pad as i64 + 1` → `1` (the padded end is the payload end): killed by (b) | §3.5 (b) `ur_23_a_value_waits_for_a_padded_one_sample_burst` |
| N47S4-09c | `tx.rs:748` | `close_start.saturating_add(3)` → `close_start` (the Stop's closing end is not counted): killed by (c) | §3.5 (c) `ur_23_a_value_waits_for_a_stop_s_held_tail` |
| N47S4-10 | same | the `device_open` test removed | `ur_23_a_contiguous_burst_with_the_same_generation_makes_no_configure_call` |
| N47S4-11 | same | the generation compared by value (equal values not configured) | `ur_25_an_equal_value_after_a_new_one_configures_once` |
| N47S4-11b | same | the mark not compared (always configure) | `ur_25_a_second_burst_with_no_new_value_configures_nothing` |
| N47S4-12 | same | the mark not set to `None` on a failed configure. Equivalent in behaviour to the mark kept at an old generation, since generations never repeat, so the killer is the mark accessor alone (m8) | `ur_21_a_refused_configure_refuses_the_burst_and_the_fixed_value_plays_on_the_same_clock` (asserts `None`); `ur_21_a_partial_configure_refusal…` (asserts `None` after the partial) |
| N47S4-12b | same | the mark published without the lost and claim check | `ur_22_a_loss_during_the_configure_clears_the_mark` |
| N47S4-12c | same | the retry configures only the failed channel | `ur_21_a_partial_configure_refusal_sends_nothing_and_the_retry_configures_both` |
| N47S4-13 | same | a refused configure does not refuse the burst (sent anyway) | `ur_21_a_refused_configure_refuses_the_burst_…` |
| N47S4-14 | same | a refused configure halts the stream | `ur_21_a_refused_configure_refuses_the_burst_…` (no halt event) |
| N47S4-15 | same | the recovery test removed from `apply_due` | `ur_28_a_value_during_an_underflow_recovery_is_applied_after_the_recovery` |
| N47S4-15b | same | `dispatch_start` decided before the configure | `ur_25_a_slow_configure_makes_a_burst_late_by_its_policy` |
| N47S4-16 | `ezsdr-radio-uhd/src/device.rs` (`apply`) | the armed transmit refusal (`arm_tx_apply_refusal`, persistent, M3) ignored | `ur_21_a_refused_configure_refuses_the_burst_…`; `ur_21_a_partial_configure_refusal…` |
| N47S4-20 | `ezsdr-radio-uhd/src/provider/tx.rs` (reopen, `:838–839`) | the mark cleared on reopen | `ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it` |
| N47S4-20b | `ezsdr-radio-uhd/src/provider/core.rs` (the loss path, `:942`) | the loss does not clear the mark | `ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it` (loss part) and `ur_22_a_loss_during_the_configure_clears_the_mark` |
| N47S4-33 | `ezsdr-radio-uhd/src/provider/tx.rs` (the configure turn, M2) | the configure's success publishes the mark and returns without the Stop check: the turn continues into the dispatch | `ur_22_a_stop_during_the_configure_leaves_no_payload_after_it`; `ur_22_a_provider_stop_during_the_configure…` |
| N47S4-38 | `ezsdr-mock-radio/src/lib.rs` (`end_tx_clock`, M4) | the Mock's transmit clock end at the origin is not written (§2.5) | `mr_25_a_transmit_clock_ends_at_its_origin_on_a_stop_at_arm` |
| N47S4-35 | `ezsdr-radio-uhd/src/provider/control.rs` (`book_burst`, `:375`) | the burst's clock is not the stream's one registered clock (§2.5) | `ur_25_a_transmit_stream_has_one_clock_for_the_run` |
| N47S4-37 | `ezsdr-radio-uhd/src/provider/tx.rs` (`step`) | the per-step read of §2.1 removed (cut read once) | `ur_25_an_end_booked_after_the_first_read_cancels_the_held_burst_and_releases_its_reservation` |
| N47S4-22 | `python/ezsdr/session.py` (`repeat` values) | updates sent after the burst | `test_ea_16_repeat_with_a_frequency_on_a_busy_stream_raises_and_sends_no_burst` |
| N47S4-23 | same | the first `Rejected` does not stop the rest | same test (action-log count) |
| N47S4-24 | same (`_param.set_`, `:206–226`) | a transmit value still raises the generic Error | `test_ea_16_a_channel_gain_on_an_empty_stream_is_accepted_and_the_next_repeat_uses_it` |
| N47S4-25 | same (`sample_rate` transmit branch) | the bench text missing for transmit rate | `test_ea_16_a_transmit_sample_rate_names_the_bench_condition` |
| N47S4-26 | same (`repeat` validation) | channels sent one by one (a valid first value sent before the second is checked) | `test_ea_16_repeat_validates_every_channel_before_sending` |
| N47S4-27 | same | an admitted first value dropped when a later one is refused | `test_ea_16_an_admitted_first_value_followed_by_a_refused_second_sends_no_burst` |

Stale rows touched by part (b): `N47S3-07` (`staging.rs`, transmit keys refused as receive-only): its test `rm_21_a_channel_in_no_stream_an_instant_or_another_key_is_refused` asserts the transmit refusal, which R2 reverses. **Retarget**: the anchor stays (the receive-only check); its killing test loses the transmit assertion, which moves to the new tests. No row is removed. `P1395`, `P1392`, `N47S3-15/16/36/37`, `N47S3-03..06, 08, 34`: unchanged (receive paths).

---

## 5. Parity limits (to write in the spec and KNOWN_LIMITATIONS, or the review record)

- **P-1 (Q1): removed.** No parity exception. A burst after a value starts at least 2 ms after its receipt on both Providers (§0 Q1); the Module's configure (15–42 µs) runs inside that lead. A host that stops for longer is S8 (UHD only).
- **P-2:** a transmit configure refusal (S7) exists only on the Module. MockRadio refuses only out-of-envelope values at admission (both Providers). The refusal path is tested on FakeDevice (`arm_tx_apply_refusal`) and is not in the parity suite. The first hardware check is an owner bench run, not in this step.
- **P-3:** the FakeDevice's configure takes host time (`apply_delay`), not the Module's 15–42 µs. The timing tests use a manual host and a delay the test chooses, so they measure nothing about the Module.
- **P-4:** the Python form of S7 is not testable on the Mock (no device refusal). The event shape is covered by the Rust test.
- **P-5 (O-1):** after a Stop, `Busy` is the conservative bound `min(busy, s + in)` on both Providers. It can refuse a value up to one window longer than the air needs (counterexample §0 O-1). Identical on both, so the parity suite does not see it; it is stated in RM-21 (§6.2).
- **P-6 (Sol 1, #79):** where the Mock and the Module accept different bookings (Sol's sequence, §0 Sol-1), their `Busy` and their answers differ. Recorded here; not tested as a known divergence; stage 5 removes it. **The parity suite uses bookings both Providers accept alike: bursts of more than 32 samples, with gaps of at least `tx_in_flight_ns`.**
- **P-7 (Sol 7):** the extra untimed configure after an equal-value update (`f1` again) costs 15–42 µs between bursts. It is deliberate and stated in the spec.

---

## 6. Spec rules pinned, and text that must change

### 6.1 Rules pinned by the new tests

RM-12 (send, repeat); RM-13 (refusal causes: transmit `Busy`); RM-16 (cut, the in-flight window and the Stop bound, `:164–167`); RM-19 (the transmit frequency is judged at admission); RM-20 (the envelope's members; C7); RM-21 (staging: transmit form, the value rule); RM-25 (one clock, origin cut at booking); RM-27 (update classes: transmit frequency, gain and antenna `cold`; rate unclassed); RM-30 (defaults); MR-16–MR-18 (burst refusals, receive-only staging text); MR-25 (Stop, held bursts); MR-32–MR-35 (values in effect, history); UR-21 (refusal at admission); UR-22 (the padded first send); UR-23 (one clock, end at the next start); UR-25 (uhd-tx: one clock, no plan re-read); UR-26 (refusals with the Action id); UR-36 (restart configured with the connect values, and now with the burst's generation); EA-16 (`repeat` with `frequency=`, `gain=`, `antenna=`; channel setters raise `Rejected`); EA-17 (captures unchanged); KC-27 (admitted value vs the value in use); SB-30 (admission checks); §58 #16 (transmit form).

### 6.2 Spec and limitation text that changes (path:line)

New or changed rules (the text goes in the implementation, not here):
- **RM-21 (`design/07-radio-model.md:232`):** a transmit channel's `frequency_hz`, `gain_db` and `antenna` are `cold`; a value is staged only when the channel's stream's `Busy` (the rule of §3.1) allows it; otherwise `Refusal::TxBusy` with the reason naming the booked end. After a Stop the stream is busy at most one in-flight window longer than the air needs: the value rule is bounded by `tx_in_flight_ns` after the Stop, not exact (O-1).
- **RM-16 (`:164–167`):** already says the Provider closes at its handed cursor, no later than `s + tx_in_flight_ns` (`:167`). Add: after a Stop taking effect at `s`, the stream is busy until `min(busy_until, s + tx_in_flight_ns)` (an open repeat's busy-until is unbounded).
- **Generation (RM-21, MR-33):** a transmit value bumps the stream's generation; a burst's snapshot carries it; the device is configured before the burst when its mark differs; an equal value still configures (deliberate, Sol 7).
- **EA-16 (`design/16-easy-api.md:96–100`):** `repeat` gains `frequency=`, `gain=`, `antenna=`; channel setters raise `Rejected` at receipt; a burst refused at configure is reported as `radio.COMMAND_REJECTED` after `repeat()` returned (async, P7); a value set just after a Stop takes effect after the in-flight window (Q10); `sample_rate=` raises `Error` (bench). Python validates every channel before sending anything (Sol 8).
- **Sentences the stage makes false (part (a), rev 3):** UR-22's "or, at most 32 samples long, a clock cut" (`design/18-uhd-radio.md:250`) goes with `refuse_first_send`; UR-23's "the clock's cut" wording (the `:263` test list, §6.2 below) and UR-25 "reads the plan again" follow the one-clock design. Each is rewritten in the same commit as its code.
- **Sentences the stage makes false:**
  - `design/vision/05-coherence-calibration-and-runtime-semantics.md:154` — "A sample-rate change is always `cold` (§15)." False since stage 2. Rewrite: frequency, gain and antenna of a channel are `cold`; a stream's rate is fixed for the Run.
  - `design/vision/10-implementation-path-and-testing.md:73` — §58 #16: add the transmit form.
  - `design/vision/03-simulation-mock-and-time.md` §15 (from line 101): no matching sentence found by grep for "cold", "update class", "transmit value"; re-grep at implementation before stating "no change".
  - `design/07-radio-model.md:199` — "No transmit key …" (RF envelope, RM-19 area): the transmit frequency is judged at admission and staged; rewrite.
  - `design/07-radio-model.md:204` — RM-20: `envelope.v1.json`'s `timing` holds exactly six members (C7); with `tx_start_lead_ns` deleted, five. Rewrite the member list and the "exactly" sentence.
  - `design/07-radio-model.md:232` — RM-21: "no other radio key has one" → transmit frequency, gain and antenna have `cold`; rate has none.
  - `design/07-radio-model.md:236` — the test `rm_04_the_key_table_is_exactly_the_declared_one` (`radio_model.rs:125–126`): update the table.
  - `design/07-radio-model.md:269–282` — RM-27 table: transmit rows `—` → `cold` for frequency, gain and antenna.
  - `design/07-radio-model.md:73–79` (RM-4 key table, including the lead row), `:332` (the `LowerBound` list): remove `radio.timing.tx_start_lead_ns` (P8).
  - `design/09-mock-radio.md:72` (MR-3 table): remove the transmit lead row. `:148` (MR-18) and `:198` (MR-33: "no transmit key changes during a Run") rewritten. `:195` (MR-32 key order): `tx_gen` leaves the segment key (§2.2).
  - `design/18-uhd-radio.md:63` and `:390`: the start-lead text, "one start lead per direction" and the two-lead Ceiling (Sol 8, spec 18 §3); the transmit part is deleted (its line numbers to be re-checked at implementation). `:263` (the `*Test:*` list of UR-23): renames `ur_23_a_clock_cut_leaving_n…` and `ur_23_a_cut_before_four_first_samples_is_refused` to the names of §2.3.
  - `KNOWN_LIMITATIONS.md:113–118` (KL-11, "no transmit retune is issued between bursts"): false; rewrite. `:134–139` (KL-36): check; no change expected. `:160` (the `tx_start_lead_ns` row): removed. `:232–238` (KL-35, "every transmit key fixed at connect"): rewritten. **No new Ceiling entry** (P1).
  - `crates/ezsdr-radio-uhd/tests/README.md:9` ("cold-change exclusion") and comments in `fake.rs:1951, 4967, 5264` ("cold change"): renamed to the current terms.
  - `python/ezsdr/session.py:221` (comment "until note 47's stage 4") and `python/tests/test_easy_api.py:145`: rewritten.
- History rows (`design/*` change logs, the 2026-11-04 and 2026-11-05 entries) are not edited.

### 6.3 Removed `tx_start_lead_ns` (P8), every reader (VERIFIED)

Receive callers of `timeline::start_lead_ns` (m3, rev 3; they stay, with the receive lead): `crates/ezsdr-radio-uhd/src/provider/rx.rs:491, 776, 1277` and `rx/differential.rs:95`. The Kernel's `binding::start_lead_ns` (`crates/ezsdr-kernel/src/binding.rs:148`, key `ezsdr.time`) is a different key and stays. `design/07-radio-model.md:445` (a receive-lead sentence) is re-read at implementation.

Radio crates: `crates/ezsdr-radio/src/lib.rs:60` (key), `:540` (field); `src/timeline.rs:91–92` (doc), `:357–360` (`start_lead_ns`, its Tx arm); `src/channels.rs:352, 456–457` (description and capabilities); `src/x3xx.rs:386` (the value); `src/measurement.rs:365` (key mapping); tests `crates/ezsdr-radio/tests/timeline.rs:76–83` (the Tx arm test, retargeted: a transmit line's lead is 0), `measurement.rs:513–514`, `channels.rs:439`, `radio_model.rs:126, 305` (key table and envelope fixture); fixtures `crates/ezsdr-radio/tests/fixtures/measurement/records/{valid,uhd-only-rejects,mock-only-rejects}.json:30` and `invalid-comparisons.json:30` (remove the row; re-check the fixture tests' counts).

Beyond the radio crates: `crates/ezsdr-mock-radio/src/lib.rs:1461` (Mock line lead, same function), `crates/ezsdr-mock-radio/tests/mock_radio.rs:538, 558`; `crates/ezsdr-radio-uhd/src/provider/control.rs:161` (line lead), `provider/tx.rs:1888` (test; the `start_lead_ns` call in the plan-reload test, which §2.3 replaces), `provider/rx.rs:2259` (test helper `tx_line`), `provider/mod.rs:880, 895–896` (candidate struct), `tests/fake.rs:158–159, 507, 520, 532–547`, `tests/common/measurement.rs:226, 4405`; `schemas/radio/envelope.v1.json:78, 88–89` (regenerate with `EZSDR_UPDATE_SCHEMAS=1`, `schema_freeze` green, `crates/ezsdr-kernel/tests/schema_freeze.rs:19`); `design/07`, `design/09`, `design/18` (§6.2); `KNOWN_LIMITATIONS.md:160`; `mutations.json` swap rows around 3759–3760 (rx/tx lead swap): removed or retargeted with the reason. No version bump, no `SCHEMA_CHANGELOG` entry (AGENTS §6).

Not edited: `plan/` records (`plan/phase8/*`, `plan/maintenance/*`) keep the history.

### 6.4 R8 items

- `CancelReason::ColdChange`: already gone (`crates/ezsdr-radio/src/lib.rs:113–123`, VERIFIED (earlier)). Stale names: rows P563, P565, P569, P570, and the rows at `mutations.json` ~6640–6690 (names only).
- MockRadio `applied` section: already absent (no section; `crates/ezsdr-mock-radio/src/lib.rs:241, 766–770, 2173` are fault and late-command fields only, VERIFIED (earlier)). Nothing to delete; state this in the review record.
- §58 #16 transmit form: S12's acceptance test and §6.2's Vision text.

---

## 7. What is not verified

- F-27 (`set_tx_rate` and transmit settings not waiting for a burst): cited, not re-read.
- F-25 and F-26: bench records, not re-run.
- `Clock::at_or_after` overflow test provenance (O-4): not checked.
- Whether the Kernel accepts an event that refuses an admitted id (Q5): not traced, for transmit or receive.
- The count of existing `apply ` assertions in `fake.rs` (the log format change, §3.3): not counted.
- The per-step read's lock cost (O-2): INFERRED, not measured.
- The "needs 4 samples" refusal on the preemption path (§2.3, `ur_23_a_preemption_leaving_four_first_samples_is_refused`): INFERRED.
- The UHD receipt function's exact line (§3.1 table): to confirm at implementation.
- Whether `schema_freeze` regenerates the envelope schema by a named command beyond `EZSDR_UPDATE_SCHEMAS=1`: not checked (handoff §5).
- Parity suite's FakeDevice wiring (`parity.rs` `uhd_binding`): not read in full.

---

## 8. Process notes

- Implementation order: part (a), then its gate; then part (b) in the order S-tests → owner rule → Python. The orchestrator's gate runs the full lists after (b).
- Risky step: Sol reviews this plan first; the implementer self-review (step 5) runs on the model handoff §5 names.
- Maintainability (AGENTS §9): the two marks are one now (Q8, `configured` as the generation); the two booking records are one `Busy` per stream (Q9). The review record names both.
- No wall-clock test: the apply delay uses the manual host (F-17); the Mock uses virtual time; the Python suite uses the Mock.
- Counterexamples: each major finding comes with its sequence (for example, "value accepted at `e`, apply at `e − 1`, burst applied inside the tail").
- Rows and killers: every new or changed test names its row (§3, §2.4); every row names its test (§4, §2.5).
**Rev 8 (sub-step b4, 2026-11-06): what was built, and what is left.** Built: the class flip (`radio_keys()`: `cold` for every channel's frequency, gain and antenna in both directions; the rate stays classless), the Kernel-path S12 form (`v58_16_a_transmit_value_outside_the_rf_envelope_is_refused_on_an_empty_stream`, N47S4-19 probed), `radio.timing.tx_start_lead_ns` removed (§6.3, schema regenerated), the Python EA-16 form (§3.4 S13 and Sol 8 tests), and the texts of §6.2 in `design/07`, `design/09`, `design/16`, `design/18`, the Vision (§27, §58 #16 and the index row) and `KNOWN_LIMITATIONS.md` (KL-11, KL-35, the row, the parity limit P-5 in KL-35's Ceiling). **Not built (§8 rev 8, D-35 to D-38):** the Mock radiated-sample S1 test (N47S4-46/-47), the S14 parity test, the Python S1/S2/S3/S4/S6 forms, and the self-review round (brief step 7).


- **Rev 4 deviations and status (part (b), 2026-11-06):** recorded at the top of this file (D-1 to D-4) and in its "Part (b) status" paragraph. The pure rule's probes: `probes-b/N47S4-*.log`. Part (b) is open beyond §3.1; the open list is in that paragraph.
- **Rev 5 (sub-step b2, 2026-11-06).** The deviations below are the b2 record (the rows are in `mutations.json`; the probe logs in `probes-b2/`, `summary.txt`).
  - D-10 (b2, placement). The UHD S2–S5 tests are Provider-level in `control.rs` (through `Control::book`), not `fake.rs`: the Kernel refuses transmit keys in a Run until b4 (the brief's b2 scope). Names: `ur_21_a_value_while_a_burst_is_booked_is_refused_with_its_action_id` (S2), `ur_23_a_value_at_the_exclusive_end_of_a_burst_is_accepted` (S5), `ur_25_the_last_value_before_a_burst_is_the_one_it_plays` (S4, with the generation), `ur_23_a_value_is_taken_only_after_a_stopped_repeat_s_in_flight_window` (S3), `ur_23_a_provider_stop_ends_a_repeat_at_its_in_flight_window` (`Streams::end`), `ur_21_a_burst_dropped_at_booking_is_not_booked`.
  - D-11 (b2, Mock placement). The Mock's S2, S3 and S5 tests are Harness tests on `x3xx` (the burst at sample 2 010 000, the Mock's sample grid at 1 MS/s with origin 0): `mr_21_a_transmit_value_while_a_burst_is_booked_is_refused_with_its_id`, `mr_16_a_transmit_value_at_the_exclusive_end_is_accepted`, `mr_25_a_transmit_value_after_a_stop_takes_effect_after_the_in_flight_window`. The Mock's snapshot takes values only (no configure refusal in b2), so the generation is not carried on the Mock (`let (_, values) = …`).
  - D-12 (b2, F6). F6 is pinned at the Timeline level: the existing `channel.rs` test `mr_33_a_timeline_keeps_the_value_in_force_at_each_instant` kills N47S4-18 (history cleared). A Mock-level radiated-sample test of the burst-start values is not built in b2; rows N47S4-46 (the burst's frequency push removed) and N47S4-47 (the snapshot's values emptied) are not written. They wait for b4's loopback test (EA-16). The Mock pushes a burst's values when it opens (`open_held`, instant order), not at booking: `channel::Timeline::at` reads the last pushed entry at or before `t`, so booking order could be wrong.
  - D-13 (b2, S6 on the Provider path). Not built: `layout_description()` has one transmit channel, and the two-stream FakeDevice fixture is b3's. The staging peer test (`rm_21_a_peer_stream_s_state_does_not_matter`, N47S4-31) is in place.
  - D-14 (b2, F9). The booking-time drop is covered (`ur_21_a_burst_dropped_at_booking_is_not_booked`, N47S4-44). The refused-later case (a burst refused at dispatch stays counted to its end) needs the owner's refusal path: b3.
  - D-15 (b2, N47S4-08). The lock-based hand-over mutant (a receipt that reads `core.held`) is not written: the lock order between `streams` and `held` is not verified (`tx.rs` takes `held` in several places). The row's mutant is `Busy::default()` in place of the stream's `busy`, and its killer is the F2 test. This does not test the hand-over reading itself; a lock-order check comes with b3.
  - D-16 (b2, N47S3-07). Retargeted: its old anchor (`Some(Concrete::Channel { rx: true, … })`) no longer exists after the transmit form. The new anchor `if rx && streaming(stream) {` → `if streaming(stream) {` is killed by `rm_21_a_transmit_value_is_not_judged_by_the_receive_stream_s_state`.
  - D-17 (b2, rows). Added: N47S4-06, -07, -08, -17c, -18, -18b, -18c, -21, -29, -30, -31, -43, -44, -45, -48. Each anchor was asserted to occur once in its file, and each killer was probed in a throwaway copy (`probes-b2/`, `summary.txt`): all KILLED.
  - D-18 (b2, retargeted rows). Their old anchors met b2's edits: P503 (the booking-time `busy` update now sits inside the `Stop` book's `if`; its anchor is the `if streams.book(…) {` line, its mutant `if false {`), N47S3-03 and N47S3-04 (`if rx && streaming(stream) {`; mutants `if rx && !streaming(stream) {` and `if rx && streaming(0) {`), and N47S3-16 (the `receipt` argument of `Staging::set`). Probed with the rows of D-17; `rules.py` passes.

- **Rev 6 (sub-step b3, 2026-11-06).** The UHD owner's configure turn, the generation mark, the refusal and FakeDevice's persistent refusal. What b3 built, with names (the rows: `mutations.json`, N47S4-09 … -50; probes in `probes-b3/summary.txt`, all KILLED on the final tree):
  - Tests, `tx.rs` (Provider-level, through `Tx::step` on a manual clock; `owner`, `valued`, `turns`, `record_applies`): `ur_23_apply_is_due_only_between_bursts_and_only_for_a_new_generation` (S9's table, pure), `ur_25_a_burst_whose_snapshot_equals_the_device_s_makes_no_apply` (M1), `ur_25_a_value_after_a_burst_ends_is_applied_before_the_next_burst` (S1), `ur_23_a_burst_at_a_held_tail_with_a_new_generation_is_configured_before_it` (S1, continuation; D-19), `ur_23_a_contiguous_burst_with_the_same_generation_makes_no_configure_call` (S9), `ur_25_an_equal_value_after_a_new_one_configures_once` (Sol 7), `ur_21_a_refused_configure_refuses_the_burst_and_the_fixed_value_plays_on_the_same_clock` (S7; R5, M3), `ur_21_a_partial_configure_refusal_sends_nothing_and_the_retry_configures_both` (S7 partial), `ur_21_a_refused_burst_stays_counted_until_its_booked_end` (F9, Control rig), `ur_22_a_stop_during_the_configure_leaves_no_payload_after_it` (M2, Stop), `ur_22_a_loss_during_the_configure_clears_the_mark` (S10, Sol 4).
  - Test, `fake.rs` (Session): `ur_25_a_first_burst_with_the_connect_values_makes_no_apply` (M1, arm publishes generation 0).
  - Each probe-based test (`ur_22_a_stop…`, `ur_22_a_loss…`, `ur_21_a_refused_burst…`) was run 20 times: 0 failures.
  - **D-19 (a finding; withdraws §3.2's "continuations skip the configure check: unreachable").** A burst booked at an open device burst's next sample with a newer generation continued that device burst and was sent with the old values (no apply). Reproduced by `ur_23_a_burst_at_a_held_tail_…` before the fix (its one apply count was 0). The fix: the open device burst is continued only when the device holds the burst's generation (`Tx::device_holds`, the chain branch of `step`); otherwise the device burst ends and the guard configures before the burst. Row N47S4-50. A burst with no generation (tests' `held`) has none to hold, so it continues as before.
  - **D-20 (placement).** Arm publishes `Some(0)` right after `open_streamer` returns `Ok` (`provider/mod.rs`, after the open and its rate check: `open_streamer` includes `check_opened`). `prepare` does not publish. The existing rigs that bypass arm publish `Some(0)` themselves: `booking_control_on` (`tx.rs`) and `ur_21_tx_front_is_per_stream`. Their assertions are unchanged.
  - **D-21 (FakeDevice).** `Fake.tx_refused: Option<(chan, freq)>`: a transmit apply on `chan` at `freq` is refused and logged (the log line is pushed first, so a refused apply shows in `calls`). `arm_tx_apply_refusal` / `disarm_tx_apply_refusal` are `#[cfg(test)]`. No `fired` counter (no test needs the count).
  - **D-22 (rig).** F9's rig is a Control with `radio.tx.c0.stream` = 0 in its configuration, built directly (`booking_control_on` does not set it, and the other tests use it as it is). The corrected value after the refusal is an owner burst at generation 2, not a Control receipt (S7's "corrected value via `UpdateParameter`" is the receipt path, b2's tests).
  - **D-23 (rows, mutants).** N47S4-11b was first pointed at `ur_25_an_equal_value…`; it survived, because `values_turn` returns early on an equal mark and the `apply_due` check behind it is redundant in production. Its filter is now the table test (which kills it), and N47S4-11c (the early return in `values_turn`) is the row the equal-value test kills. N47S4-11 (the generation compared by value) is not written: no value comparison exists to mutate (equivalent).
  - **D-24 (not built in b3; open for b4 or a later sub-step).** These §3.4–§3.5 items are not in b3: owner-guard tests (a) moved at dispatch, (b) one sample padded to four, (c) a Stop's held tail at `e` (rows N47S4-09a/-09b/-09c); S8, the slow configure and the late policy (N47S4-15b); `Provider::stop` during the configure (the M2 case with `Provider::stop`); F5's reopen keeping the mark (N47S4-20: `tx.rs` reopen restores the mark it read, unchanged); S10's underflow-recovery case (`ur_28_…`); S6 on the Provider path (P9, the two-transmit-stream fixture, N47S4-31's owner form); N47S4-14 (a refused configure does not halt: no mutant written); N47S4-12c (the retry configures only the failed channel: the partial test asserts four applies, so it would kill it, but no row is written); N47S4-33 (the M2 turn's Stop check; no clean mutant yet).
  - **D-25 (end checks).** `cargo fmt --all --check`; clippy `--workspace --all-targets -D warnings` (the owner rig's tuple is the alias `OwnerRig`, for `clippy::type_complexity`); clippy `-p ezsdr-radio-uhd --features uhd --all-targets` on sim02 via `rdev.py`; `cargo test --workspace` (1 865 passed, 0 failed); `cargo test -p ezsdr-radio-uhd --lib` (381) and `--test fake` (339); the Python suite on 3.14 (`~/.cache/ezsdr-venv`, 92 tests OK, with `EZSDR_SERVER` on the tree's `ezsdr-server`). The Python 3.9 container run is not done: no Python file changed in b3, and the command is not in handoff §5. `rules.py --base HEAD --renames renames.txt`: 0 problems. No renames.

- **Rev 7 (sub-step b3b, 2026-11-06).** The rest of the owner-guard tests, S6, S8, S10, F5, `Provider::stop` during the configure, and the rows N47S4-09a/b/c, -14, -20, -15b, -33. Tests in `tx.rs` (Provider-level, through `Tx::step` on a manual clock; helper `turns_at`, device time in nanoseconds). Probes: `probes-b3b/summary.txt` (one log per row, `probe_b3b.py`).
  - Tests: `ur_23_a_value_waits_for_a_burst_moved_at_dispatch_past_its_booked_end` (guard (a)), `ur_23_a_value_waits_for_a_padded_one_sample_burst` (guard (b)), `ur_23_a_value_waits_for_a_stop_s_held_tail` (guard (c), m9), `ur_25_a_slow_configure_makes_a_burst_late_by_its_policy` (S8), `ur_22_a_provider_stop_during_the_configure_keeps_the_mark_and_cancels_the_burst` (M2, `Provider::stop`), `ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it` (F5), `ur_28_a_value_during_an_underflow_recovery_is_applied_after_the_recovery` (S10), `ur_23_two_transmit_streams_busy_and_empty` (S6, P9).
  - Rows (`mutations.json`, text edits; each anchor matches once after `cargo fmt`): N47S4-09a (`previous_end` → `None`), -09b (`pad as i64 + 1` → `1`), -09c (`close_start.saturating_add(3)` → `close_start`), -15b (the late policy decided before the configure), -14 (a refused configure loses the device), -20 (a reopen clears the mark it read), -33 (the configure turn continues into `step` without draining commands). Probes: all seven KILLED on the tree of this step, each by its named killer.
  - Repeats: each new test that runs a probe or a thread (all nine above, plus the b3 probe tests) ran 20 times, 0 failures.
  - **D-26 (placement, receipts).** The guard, S8, F5, S10 and S6 tests give the owner its burst and its generation directly; they do not go through the Control's receipt. So the guard tests do not assert that the value is *accepted* at E, at the padded end or at `min(busy, s + in)`: that is the Control's `Busy` (b2's `ur_21_*`, `ur_23_a_value_at_the_exclusive_end…`, `ur_23_a_provider_stop_ends_a_repeat…`). The plan's "value accepted at receipt E" for (a) is the same receipt, not repeated here.
  - **D-27 (the guard (a) move).** A is booked at 20 000 with `SendAsapAndFlag` (the `valued` helper builds `DropAndFlag`: the first run dropped A, which the test caught, not a production defect). A is dispatched at 19 ms and moved to 21 000; its device burst closes at 21 005 with the front. B (25 000, in the window from 15 ms) waits for 21 005.
  - **D-28 (S8's SendAsap case).** The SendAsap case does not kill N47S4-15b (by reasoning, not probed): with the mutant, the first decision moves A and the decision after the configure moves it again, so the send still follows the apply. The Drop case kills it (the burst is dropped at 19 ms without an apply). The test asserts both cases.
  - **D-29 (S10, a defensive guard).** `apply_due`'s `!recovering` is not reachable through `pass()`, which reopens before any step (`tx.rs`, `pass`). The test drives `step` directly with a pending recovery; N47S4-15 kills the guard, not a production path. The rule stays, as a guard of `apply_due`.
  - **D-30 (rig).** The reopen needs the device's clock to run; `Frozen` is dropped before `pass()` and taken again after it (F5, S10). Device time drifts while no `Frozen` holds it, so a `device_at` equality check after a drop fails.
  - **D-31 (`Provider::stop` in the probe).** The test's probe does what `Provider::stop` does (`streams.stop`, `streams.end`, `TxCmd::Shutdown`) on the second channel's apply, then the owner's next step sees the `Shutdown` (the run loop's order). `shutdown()` runs on a manual host with a driver thread (`ur_26`'s pattern). The `ProviderStop` cancellation is a reject note (`cancellation_reports` is false for it), so the test reads `rec.rejected`, not an event.
  - **D-32 (N47S4-12c: not written).** No single-text mutant of the current code makes the retry configure only the failed channel: the design keeps no per-channel progress for a retry to skip. The partial test (`ur_21_a_partial_configure_refusal_sends_nothing_and_the_retry_configures_both`) asserts four applies after the retry, so any design that skips the applied channel fails it. The row is not written; the orchestrator decides whether a per-channel progress is worth a row.
  - **D-33 (N47S4-14).** The refused configure's halt is modelled as a device loss (`core.device_lost`): the stream then halts. The killer's assertion is that the only event is `COMMAND_REJECTED` and the device is not lost.
  - **D-34 (S6).** Built: two Owners on `two_tx_description` (stream 1's channel set is `Channels::first(0).with(1)`, device channel 1), stream 0 streams a 30 000-sample burst, stream 1's new value configures channel 1 alone during it, and stream 0's sends are contiguous (`at` advances by `n × 200`). Its value is a direct generation-1 burst, not a Control receipt (D-26).
  - Not built, by the brief's scope: the Python form of S6 (`test_ea_16_a_value_for_an_empty_transmit_stream…`) and the Session form of the guard tests (the Session's receipt path is b2's; the Python suite is b4's).

### Rev 8 deviations (sub-step b4)

- **D-35 (Mock S1 radiated test, not built).** `mr_33_a_transmit_value_takes_effect_at_the_next_burst_s_start` (N47S4-46/-47) needs the exact transmit-clock tick of burst A's end and of burst B's start in the Mock harness; the sample grid was not measured in b4. Left for the next sub-step. Until then N47S4-18 (history lost) and N47S4-28 (too strict) are killed by the Harness and Timeline tests of rev 7 (D-12).
- **D-36 (S14 parity, not built).** `parity.rs` `s14_transmit_values_on_an_empty_stream_decide_the_same_on_both_providers` is not written. The Provider-level S2–S5 tests of b2 and b3 stand; N47S4-17c and N47S4-01 are killed by their own tests.
- **D-37 (Python forms, partly built).** Built: S13 (`test_ea_16_repeat_with_a_frequency_on_a_busy_stream_raises_and_sends_no_burst`), the two Sol 8 tests, and the empty-stream gain test (renamed to `..._is_accepted_before_a_repeat`: its loopback check is not built). Not built: the S1 and S2 Python forms of §3.4, and the S3, S4 and S6 forms. The rows N47S4-22 to -27 are not in `mutations.json`; they are not added in b4.
- **D-38 (self-review, not run).** Brief step 7 (one round, `self-review-checklist.md`) is deferred to the next sub-step, as the brief allows when the context runs short. The orchestrator's gate, not this record, decides the self-review findings.
- **Probes.** Only N47S4-19 was probed in b4: under the mutant (`if *is_rx {` at `lib.rs:433`), `v58_16_a_transmit_value_outside_the_rf_envelope_is_refused_on_an_empty_stream` fails with the transmit value accepted (`probes-b4/N47S4-19.log`, 2 failed, EXIT 101), so the row is killed. The rows retargeted in b4 (N47S3-01, N47S3-33, P11, P624, P1393, F36, F40 and the swap rows) were checked for an exact anchor match by `rules.py`, not probed; their probes are part of the next sub-step.
- **F9 through the Kernel (not built).** The refused-later case (a burst refused at dispatch stays counted to its end) is still Provider-only (D-14).
- **End checks (b4).** `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean (EXIT 0); `cargo test --workspace --no-fail-fast`: 1874 passed, 0 failed (`/tmp/opencode/b4-ws-test.log`); Python 3.14 (`~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests`, `EZSDR_SERVER` the debug `ezsdr-server`): 97 OK; Python 3.9 (`docker run --rm -v "$PWD:/w" -w /w -v <server>:/srv/ezsdr-server:ro -e EZSDR_SERVER=/srv/ezsdr-server -e PYTHONDONTWRITEBYTECODE=1 python:3.9-slim sh -c "pip -q install --user numpy; python -m unittest discover -s python/tests"`): 97 OK; `rules.py --base HEAD --renames renames.txt`: 0 problems. No new test is timing-dependent (the b4 tests are pure, the Python tests are request-ordered), so the 20-repeat rule did not apply.

- **Rev 9 (sub-step b5, 2026-11-06).** What was built, with names. Probe logs: `probes-b5/` (`summary.txt`, one log per row; `probe_b5.py`, a copy outside the tree per row, anchor applied once, the killer run).
  - **Item 1 (D-35 closed).** `mr_33_a_transmit_value_takes_effect_at_the_next_burst_s_start` (`crates/ezsdr-mock-radio/tests/mock_channel.rs`): burst A radiates at f1, the value f2 is received after A's exclusive end, burst B radiates at f2. Receive channel 0 is tuned to f1 and channel 1 to f2, so each burst is heard on its own channel (the loopback receive, as EA-17 does; `radiated` is private). The test asserts non-empty channels and every sample. Its helpers: `tx_root` and a `Radio::received_all()` that drains the link once for every channel (`received(channel)` now calls it; the 28 existing callers are unchanged in what they read; the drain of one channel no longer empties the others).
  - **D-39 (rows N47S4-46, -47).** `N47S4-46` (the burst's frequency is not set at its start, `open_held`), `N47S4-47` (the snapshot's values are emptied): both KILLED by the test above.
  - **Item 4, Python S1–S4 (D-37 partly closed).** `test_ea_16_a_transmit_value_on_an_empty_stream_is_used_by_the_next_repeat` (S1), `test_ea_16_a_transmit_value_for_a_booked_stream_is_rejected_and_the_burst_keeps_its_value` (S2), `test_ea_16_a_transmit_value_after_a_stop_waits_for_the_in_flight_window` (S3), `test_ea_16_the_last_transmit_value_before_a_repeat_is_the_one_it_transmits` (S4). All four pass on 3.14. **Not built: the S6 form** (`test_ea_16_a_value_for_an_empty_transmit_stream_leaves_the_busy_one_undisturbed`).
  - **Item 5, rows.** `N47S4-22` to `-23`, `-25`, `-26`, `-27` added to `mutations.json` (exact substrings). **N47S4-24 removed (D-42)**: it was the same mutant as P1393 (the identical `old` and `new`); P1393's killer `test_v3_parameters_through_attributes` and the gain test both kill it (probed for P1393; the gain-test killer was probed under N47S4-24 before removal). Probes, each KILLED by its named killer: N47S4-46, -47, P11, P624, F36, F40, N47S3-33, P1393, N47S4-19 (from b4), N47S4-22, -23, -25, -26, -27, and N47S4-20 (retargeted at Item 5, see D-45).
  - **D-40 (N47S3-01: the brief's premise does not hold, row kept).** The brief said N47S3-01's `new` is the tree's own text. It is not: b4 swapped `old` and `new`, so the row now replaces the tree's `let cold = Some(UpdateClass::Cold);` with `let cold = (direction == "rx").then_some(UpdateClass::Cold);`, which makes the transmit keys classless. It is a live mutant, and it is KILLED by `rm_04_the_key_table_is_exactly_the_declared_one`. Kept; not deleted or retargeted.
  - **D-41 (N47S4-26, -27: mutant design).** `N47S4-26` sends each channel's value inside `_channel_values`, before the next channel is checked (the anchor `numbered[number] = _value(name, value)`). `N47S4-27` sends only the last of a call's values (`values[-1:]`), so an admitted earlier value is never staged. The brief's and the plan's wording ("dropped when a later one is refused") describes a rollback that the code does not have; the row names the mutant it actually makes.
  - **D-45 (a reopen after a loss, self-review P3).** `tx.rs` (`reopen_streamer`): the successful reopen wrote its old mark back under the streams lock even when the device was lost in the meantime (`record_configured(key, configured)`). It now writes `None` when `core.is_lost()` (`if core.is_lost() { None } else { configured }`), so the mark follows the loss whichever order the two threads take. Row N47S4-20 (the reopen clears the mark it read) is retargeted to the new text; KILLED by `ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it`. **Not written:** a row for the new loss branch (its mutant, `configured` again, survives: no test reopens and loses in the racing order). Recorded as a survivor to close with a test.
  - **Self-review (D-44).** Brief step 7 ran: `opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free#max --agent plan --auto` returned "Free usage limit reached" (`self-review-b.log`, exit 1). Per the brief, `opencode-go/deepseek-v4.1-flash#max` ran once (`self-review-b.go.log`, exit 1 from the CLI's own status, with the review text complete at the end of the log; the review is the section "Self-review: note 47 stage 4 (part (a) B4 + part (b))"). Its findings and dispositions:
    - P2, S14 parity absent (`parity.rs`): **open**, not built this sub-step (§8 rev 9, "Left").
    - P2, S8 pure test `ur_25_the_late_decision_uses_the_start_after_the_configure` missing, and the SendAsap case of `ur_25_a_slow_configure…` does not kill N47S4-15b (D-28): **open** (left).
    - P2, S9's separate later burst with the same generation (`ur_25_a_second_burst_with_no_new_value_configures_nothing`): **open** (left).
    - P2, S10's Mock case `mr_25_a_stop_before_the_burst_with_a_new_value_cancels_it_and_keeps_the_value_of_record`: **open** (left).
    - P2, S6 Python form: **open** (left, Item 4).
    - P2, S7/R5 through the Kernel's admission (the corrected value as a Control receipt, D-22): **open**, by design of the rig; the Kernel-level form is left with F9 (Item 3).
    - P2, R2's booking-time `SendAsap` move unpinned (no `ur_21_a_value_is_judged_by_a_booking_time_move`, no N47S4-08b): **open** (left).
    - P3, `reopen_streamer` mark after a loss: **fixed** (D-45).
    - P3, `record_configured` doc: **fixed** (`core.rs`).
    - P2, `design/07` RM-19's Test line: **fixed** (names `v58_16_a_transmit_value_outside_the_rf_envelope_is_refused_on_an_empty_stream`; Changes row added).
    - P2, `design/18` UR-36: **fixed** (the record holds a transmit stream's generation; Changes row).
    - P2, `design/18` UR-26: **fixed** (`apply_due`'s "no device burst open" condition is stated; Changes row). The full guard (past the previous end, not recovering) stays in UR-25's `apply_due`, which UR-26 cites.
    - P3, `design/18` UR-9's "start leads": **fixed**.
    - P3, `design/07` RM-15's transmit start-lead condition: **fixed** (Changes row).
    - P3, `handoff.md:34` and the "測って決め直す値" row (a transmit start lead): **rejected here**: `handoff.md` is the orchestrator's file, and the review asks for it at the gate.
    - P3, `test-plan.md` §8's stale "not built" statements (D-35, D-37, D-12): **fixed** by this rev.
    - P3, P1393 and N47S4-24 as one mutant: **fixed** (N47S4-24 removed, D-42).
    - The review's "no finding" items (the busy rule written once, the one snapshot, the one mark owner, the fixtures reused, `tx_gen`/`Tx.plan`/`version`/`at`/`retarget`/`refuse_first_send` with no dangling use, scope, and the "only/never" claims of the receipt and the configure) stand; its "not checked" item (the rows not run) is covered by the probes above, except the survivors named here.
  - **End checks (b5), after the fixes.** `cargo fmt --all --check`: clean. `cargo clippy --workspace --all-targets -- -D warnings`: EXIT 0. `cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` on sim02 through `rdev.py`: EXIT 0. `cargo test --workspace --no-fail-fast`: 1875 passed, 0 failed (`/tmp/opencode/b5-ws-test.log`). Python 3.14 (`~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests`, `EZSDR_SERVER` the debug `ezsdr-server`): 101 OK. Python 3.9 (`docker run --rm -v "$PWD:/w" -w /w -v $HOME/.cache/cargo-target/Ez-SDRv4/debug/ezsdr-server:/srv/ezsdr-server:ro -e EZSDR_SERVER=/srv/ezsdr-server -e PYTHONDONTWRITEBYTECODE=1 python:3.9-slim sh -c "pip -q install --user numpy; python -m unittest discover -s python/tests"`): 101 OK. `rules.py --base HEAD --renames renames.txt`: 0 problems. The new tests are pure or request-ordered (the Mock test runs a manual clock; the Python tests run simulated time), so the 20-repeat rule did not apply.
  - **Left for the next sub-step (open):** Item 2, the S14 parity test in `parity.rs` (not started; the rig is `MockRig`/`UhdRig`, and its design needs the bookings both Providers accept alike, P-6); Item 3, F9 through the Kernel (the refused-later case, still Provider-only, D-14); Item 4, the Python S6 form; the self-review's test gaps listed above (S8 pure and SendAsap, S9 later burst, S10 Mock stop, R2 booking-time move, the loss branch of the reopen, S7 through the Kernel). The gate's orchestrator decides the self-review record's findings.

- **Rev 10 (sub-step b6, 2026-11-06).** Closes rev 9's left list except Item 3. Probe logs: `probes-b6/` (one per row, `summary.txt`).
  - **Item 1, the reopen's loss branch.** Test `ur_22_a_loss_during_the_reopen_clears_the_mark_the_reopen_read` (`tx.rs`); row N47S4-51 (`if core.is_lost() { None } else { configured }` → `configured`) KILLED. Test scaffolding: a probe `tx_reopen_done` in `reopen_streamer` right after `core.reopen_tx_streamer`, where the test drives the loss. Not a production change. Repeats: 20 runs, 0 failures.
  - **Item 2, S14 parity.** `s14_transmit_values_on_an_empty_stream_decide_the_same_on_both_providers` (`parity.rs`) covers S1–S4 on `MockRig` and `UhdRig`: accepted/refused and the Action ids (both providers get a barrier per submission so the ids align). Row N47S4-17c (existing) KILLED by it. Repeats: 20 runs, 0 failures.
    - **D-46 (S5 out of parity).** The sample-exact boundary of S5 is not pinned on the rig: the device clock runs 0.2–1.2 ms ahead of the submitted tick (probed with a debug print, since removed), longer than a 40-sample burst (about 8 µs). S5 stays per-Provider (`ur_23_a_value_at_the_exclusive_end_of_a_burst_is_accepted`, `mr_16_a_transmit_value_at_the_exclusive_end_is_accepted`).
  - **Item 3, S7 and F9 through the Kernel: left.** `FakeDevice::arm_tx_apply_refusal` is `#[cfg(test)]` and crate-internal, so `tests/fake.rs` cannot reach it. The option is a public `FakeFault::TxApplyRefused(usize, f64)` wired into `tx_refused` (`device.rs`, `FakeDevice::new`). Timed receipts on the free-running FakeDevice would need 20 repeats. Not done in b6.
  - **Item 4, S8.** The SendAsap case: `ur_25_a_slow_configure_makes_a_burst_late_by_its_policy` now asserts `late_by_ns >= 3_000_000` (the mutant gives 1 ms, the correct code 5 ms). Row N47S4-15b (existing; D-28 said it was not killed by the SendAsap case) KILLED by it. This closes D-28.
    - **D-47 (pure late-decision test not built).** `dispatch_start` reads the device clock, so a pure form needs a production refactor (the decision taken from a value, not a clock read). The brief allowed production changes only for a defect the test shows; none was shown. Left: the pure test is a candidate for a later refactor step.
  - **Item 5, S9.** Covered by the existing `ur_25_an_equal_value_after_a_new_one_configures_once`, which asserts the second burst with the same generation makes no configure call; row N47S4-11c (existing) is its killer. Not re-probed in b6. The self-review's "missing" finding (P2) is mistaken: the test exists.
  - **Item 6, S10 Mock.** `mr_25_a_stop_before_the_burst_with_a_new_value_cancels_it_and_keeps_the_value_of_record` (`mock_channel.rs`); row N47S4-52 (`self.cut_segments(k, cut);` → `let _ = cut;`) KILLED.
  - **Item 7, R2 booking-time move.** `ur_21_a_value_is_judged_by_a_booking_time_move` (`control.rs`); row N47S4-08b (booked end uses the requested start, not the moved one) KILLED. No production change.
  - **Item 8, Python S6.** `test_ea_16_a_value_for_an_empty_transmit_stream_leaves_the_busy_one_undisturbed` (`python/tests/test_easy_api.py`): the Mock profile declares two transmit streams, so the test is written. Row N47S4-53 (`python/ezsdr/session.py`: a channel's key uses channel 0, `c0`) KILLED: the value for stream 1 is refused as the busy stream 0 is streaming.
  - **Production changes:** none. `cargo fmt --all` was run on the tree.
  - **End checks (b6).** `cargo fmt --all --check`: clean. `cargo clippy --workspace --all-targets -- -D warnings`: EXIT 0. `cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` on sim02 through `rdev.py`: EXIT 0. `cargo test --workspace --no-fail-fast`: 1879 passed, 0 failed (`/tmp/opencode/b6-ws-test.log`). Python 3.14: 102 OK; Python 3.9 container: 102 OK. `rules.py --base HEAD --renames renames.txt`: 0 problems. No renames.
  - **Left after b6:** Item 3 (D-46 and the design above; the F9 and S7 Kernel forms); D-47's pure late-decision test; the self-review's P2 items for S7/R5 through the Kernel's admission (open with Item 3).

- **Fix round 1 (2026-11-07, still rev 10).** Implements `dispositions-code.md` (Sol's code review, `sol-code-final.md`): finding 1 (fractional burst start), finding 2 (refused configure through the Kernel), finding 3 (`design/18` text). Probe logs: `probes-fix1/` (`summary.txt`). Rev 9 is `test-plan.v9.md`.
  - **Finding 1 (test first, then the fix).** `mr_33_a_transmit_value_takes_effect_at_a_fractional_burst_start` (`mock_channel.rs`): 3 MS/s, the burst's sample 1 999 is at a fractional root tick, receive channel 0 at 1 GHz and channel 1 at 2 GHz. Red before the fix (`channel at f1, sample 1999`), green after. The fix: `channel::Timeline` is keyed by `RootInstant` (`set`, a tick, wraps `RootInstant::tick`; `set_at` takes an instant; `at` compares exactly); `open_held` records the values at `RootInstant::of_sample`; the held burst's opening tick (`advance()`) and its wake use the new `time::v_floor` (floor, not ceil), and `time::v_after` is `v_floor + 1`.
    - **D-48 (why floor, not ceil).** A sample at instant `t` is radiated only when its instant is strictly before the round's tick (rx and tx emission is strict). A ceil'd opening tick can land after the sample's instant and let the sample go out on the old values. A floor tick is at or before the instant, and after the previous sample when a sample spans at least one root tick. This holds on the 1 GHz root for every rate the Mock accepts (ideal 1–1e9; x3xx 5 MS/s upward). On a 200 MHz root with an ideal rate above 200 MS/s it would not hold. Not a production root; the shortcut comment at `advance()` names it.
    - **D-49 (the comparison is inline).** `RootInstant` (`ezsdr-sim`, CH-3) has `is_at_or_after(tick)` only, so `Timeline::at` compares `from.num * t.den <= t.num * from.den` itself. `ezsdr-sim` is unchanged.
  - **Finding 2 (the refused configure through the Kernel).** `FakeFault::TxApplyRefused(usize, f64)` (`device.rs`, public, persistent, value-keyed) replaces the `#[cfg(test)]` arm and disarm hooks, which are removed with the `tx_refused` field. `provider/tx.rs`: the S7 and partial tests use `owner_with(channels, faults)`; the partial retry uses the corrected value F3 (no disarm needed); F9's device is built with `TxApplyRefused(0, F2)`. This closes b6's Item 3 for S7 and F9.
    - Test `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` (`tests/fake.rs`): a transmit burst (10 ms, since the device lead is 2 ms and a 2 ms burst went late) with a value its configure refuses. Asserts `COMMAND_REJECTED` for the burst and the value, in that order, by Action id; no experiment send before the next burst; the corrected apply `freq=3000000000` before the first send, on the same tx clock. The early value gets `wait(ms(1)/2)` after submit so its receipt lands inside the window.
  - **Finding 3 (`design/18`).** UR-12 "Configuring a stream" covers a transmit burst's configure (frequency, gain, antenna; no rate) as "nowhere else". UR-26 states the conditions of the owner's configure turn (no device burst open; device time at or after the previous device burst's end, padding and held tail included; no recovery; the owner's own turn before the late policy; the generation mark's rules; a refused configure refuses its burst with the burst's id and does not halt). The reference to UR-25's `apply_due` (a private function) is removed. UR-26's Test line gains four names. Changes row 2026-11-07. UR-26's paragraph is split over continuation lines (the `rules.py` long-line rule). `design/09` MR-33 needs no change: "the value in force at its own instant" is what the fix does.
  - **Rows.** Added: N47S4-54 (`v_floor` → `v_of` at the held burst's tick), N47S4-55 (the value key rounded through `v_of`), N47S4-56 (`core.device_failed(None, "tx_burst", &error);` removed). Retargeted to this round's text: N47S4-16 (the apply check's direction: `Dir::Tx` → `Rx`, the S7 Provider test kills it), N47S4-18 (`self.changes.push((at, value))`), N47S4-46 (`model.frequency_hz.set_at(at, value.frequency_hz)`), and in `plan/phase3/tools/mutations.json`: M24 (`v_after` becomes `v_of`, its old `v_floor(o, q, k)?.checked_add(1)`), M30 and M54 (the `from` comparison). `rules.py` finds every anchor once.
  - **Probes** (`probes-fix1/`, one log per row, each in a copy outside the tree): N47S4-54, N47S4-55 (killer `mr_33_a_transmit_value_takes_effect_at_a_fractional_burst_start`), N47S4-56 (`ur_21_a_refused_configure_refuses_its_burst_through_the_kernel`), N47S4-16 (`ur_21_a_refused_configure_refuses_the_burst_and_the_fixed_value_plays_on_the_same_clock`), N47S4-18, N47S4-46, M30, M54 — all KILLED by their named killers. **M24**: its filter `mr_14` is a prefix, and the probe runs `--exact`, so the first probe was NOT-RUN; re-probed with the full name `mr_14_blocks_appear_when_their_last_sample_has_occurred`: KILLED (the row is unchanged; `probes-fix1/summary.txt` has both lines).
  - **Repeats (20 runs each, timing-dependent).** `mr_33_a_transmit_value_takes_effect_at_a_fractional_burst_start`: 0 failures. `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel`: 0 failures.
  - **Mistake found at the gate's end checks, fixed.** The retargeting of the stale rows was done after `cargo fmt` and the tests, and `rules.py` first failed on the five stale anchors and on UR-26's line length; both are fixed above and `rules.py` passes.
  - **End checks (fix round 1).** `cargo fmt --all --check`: clean. `cargo clippy --workspace --all-targets -- -D warnings`: EXIT 0. `cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` through `rdev.py`: EXIT 0. `cargo test --workspace --no-fail-fast` in the foreground: 1881 passed, 0 failed (`/tmp/opencode/fix1-test.log`; b6 was 1879, plus the two new tests). Python 3.14 (`~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests`, `EZSDR_SERVER` the tree's debug server): 102 OK. The 3.9 container run is not repeated: no Python file changed in this round (`python/` is unchanged since b6). `rules.py --base HEAD --renames renames.txt`: 0 problems. No `__pycache__` in the tree (the probe's `import anchor` wrote one, removed).
  - **Left after fix round 1:** D-46 (S5 is not on the rig), D-47 (the pure late-decision test needs a refactor), and the self-review items of the step's record. Self-review was not run in this round (the brief says so).
- **Rev 11 (gate fix round, 2026-11-07).** The gate (`gate/mutation.log`) failed on two rows; both are text edits in `mutations.json`, probed in a throwaway copy (`/tmp/opencode/gf-probe-n47s4`, logs in `probes-gate-fix/`).
  - **phase7/R07** (`plan/phase7/tools/mutations.json`): its filter named the test renamed in part (a). Retargeted to `ur_23_a_clock_end_cancels_a_held_burst_and_releases_its_reservation` (`crates/ezsdr-radio-uhd/src/provider/tx.rs:3528`; `renames.txt` line 2). Baseline passes (`r07-baseline.log`); the mutant (`if false {` at `tx.rs:1079`) fails it at `tx.rs:3543` (`r07-mutant.log`): killed. A grep of every `plan/*/tools/mutations.json` for the 12 old names in `renames.txt` finds no other filter (only R07 names one).
  - **phase3/M52** (MR-25, `crates/ezsdr-mock-radio/src/lib.rs:961`, the `filter(|start| *start < cut)` branch). **Reachable, not dead code.** The branch is taken when a Stop is handled while a held burst still waits for its floor tick. `advance` opens a held burst only at `time::v_floor` of its start (`lib.rs:1872–1875`), and actions are handled before `advance` in a step, so a held burst that starts after the Stop's instant but before its cut (pushed by the in-flight window, `tx_handed_front`, `lib.rs:1389–1417`) is still held when `transmit_until_cut` (`lib.rs:953`) runs. The test that holds such a burst across a Stop is `mr_25_a_stop_transmits_every_held_burst_before_it` (`crates/ezsdr-mock-radio/tests/mock_channel.rs:1433`): bursts at 1 500, 2 500 and 3 000 are held when the Stop arrives at 3 200 500 ns, and the mutant withdraws the burst at 3 000 (`m52-mutant-mr25.log`: left `[(1500, Eob)]`, right the four records). The row's filter `ch_09_a_burst_starting` never ran that test: its test `ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order` passes under the mutant (`m52-mutant-ch09.log`), because `ordered()` steps through the burst's floor tick before the Stop. Decision: the branch stays; **M52's filter is retargeted to `mr_25_a_stop_transmits_every_held_burst_before_it`** (`plan/phase3/tools/mutations.json`). No test is added.
  - **Not changed:** the phase 3 planning and implementation records (`plan/phase3/20-implementation-plan.md:492`, `implementation-notes.md:724`) still name the old killer; they record the phase 3 state.
