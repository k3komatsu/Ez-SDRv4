<!-- opus-s3-plan2-recheck.md -->
**Verdict: NOT CLOSED.** M1, M3, m1, m2, m3 and m4 are closed. M2 is not, because rev 2 still uses the full-lead threshold that ruling 7 overturns. I also found one new major in D2.

**(1) Dispositions** (code claims checked at d8f23af):
- **M1 closed.** B3 (plan:155) asserts the refusal. A scheduled update carries `at: Some` (`ezsdr-kernel/src/event.rs:933`), and timed entries go out as one batch while `Stop` waits in the agenda (`pipeline.rs:753-787`).
- **M3 closed.** The test at plan:133 needs a streaming stream, an admitted first value and a Provider refusal, and asserts that no second update and no `start_rx` are sent. Today's `_admitted` (`python/ezsdr/session.py:112-115`) fails it.
- **m1 closed.** The test at plan:176 calls `owner_configure` twice for one segment, which reaches `Owner::Nothing` (`core.rs:1319`, `owner_step` at `:612-620`). That kills both N3-33 and N3-34.
- **m2 closed.** The wait and its early exit are real (`pipeline.rs:1036-1038`, `paced.rs:185`), and a finish is only acknowledged at the next `recv` (`state.rs:72-80`). One gap: `ThreadedProvider` has no "hold after take" knob yet; it has only `never_finishing` and `with_action_delay` (`tests/support/paced.rs:347,395`). The plan should name the new knob.
- **m3 closed** (N3-10 and N3-23 rewritten, the event-kind row dropped). **m4 closed** (H1 and H2 corrected).

**(2) Ruling 7: coherent with today's code.** The origin is already fixed at booking (`ready: now` in `control.rs:297` and mock `lib.rs:1459`; `timeline.rs:510-523`), so MockRadio keeps exact origin, cut and `by` parity. I found no sequence where the lead is eaten silently in a way that matters, as long as the bench counts a short lead as a failure (fix 6 below). Below the threshold the stream command still goes out with the Provider's lead; at or above it, `START_LEAD_SHORT` or `LATE_COMMAND` reports it. Two minor wording points:
- `min_timed_command_lead_ns` is 5 ms on UHD: the 2 ms device lead plus the 3 ms delivery allowance (`radio-uhd/src/profile.rs:21`, spec 18:63). The owner thread doesn't spend the allowance, so call the threshold "the declared command lead (RM-6)", not "the device's own lead". It's conservative and needs no new constant.
- "Counts from the booking" is wrong after a cut. There the ready instant is the later of the booking and cut + release window + one receive call + allowance (`timeline.rs:516-521`), and the owner configures only after it has seen the cut (`rx.rs:337-350`). Spec 07:81 should keep "from the instant its Provider is ready" and add "and covers the start's configure".

**Plan changes ruling 7 needs:**
1. `lead_left(origin, completion, min_lead)` returns `Some(origin − completion)` only when `completion < origin` and `origin − completion < min_lead` (D4 and question 8; delete D4's caveat paragraph). Keep "completion ≥ origin goes to the missed-start path" and state it as the reading of the ruling.
2. In the pure test (plan:114), test the boundary at `origin − completion == min_lead` (no report) and `min_lead − 1` (report). Replace the Tx-lead case with one where `D = 25 ms`, 50 ms lead → no report; that case kills a full-lead-threshold mutant.
3. Partial test (plan:115): use D = 46 ms (4 ms left → report), D = 45 ms (5 ms left → none), D = 25 ms (none). Take the numbers from the lattice origin. Plan:117 uses D = 46 ms instead of 25 ms.
4. `SlowApply` must move both the Core's manual clock and the fake device's clock (`test_support.rs:190-196`, `device_at`). The fake judges "late" by its own time (`device.rs:1633`), so moving only the Core clock gives no `LateCommand` in the past-origin case. The existing `probe(&core, "apply", …)` hook (`rx.rs:2412`) can do this without a new wrapper.
5. The loose fake test (plan:119) is flaky even in rev 2: under load the apply can pass the origin, giving no `START_LEAD_SHORT`. Assert "SHORT or LATE ≥ 1" for an apply just under the lead, and keep the 4× lead → LATE case.
6. Payload: drop `lead_ns` or rename it `min_lead_ns`. Rows: N3-35 becomes `<` → `<=`; add a row "threshold = `start_lead_ns(Rx)`", killed by the D = 25 ms case.
7. Measurement group (plan:207): a stage fails on `LATE_COMMAND` **or** `START_LEAD_SHORT`. `restart-retune` crosses an LO step on every restart. Spec 07:81, RM-25, UR-25 and KL-16 wording follow these changes.

**(3) New major: D2's arrival-order flag disagrees with the timeline** (plan:26, the oracle at :142, §5 items 2–3 at :214-215). A `Stop` is booked at `e = now` (`control.rs:273`) and does not cancel a later-effective start: the plan is replayed sorted by `(e, seq)` (`timeline.rs:389-392,439-451`).
- Sequence: `Stop` → `start_rx(at = now + 1 s)` → `Stop` → set c0 f2.
- Staging: stopped → streaming → stopped, so f2 is accepted with no event.
- The plan: the start still begins at +1 s with its f1 snapshot. Every later set is accepted (no `Rejected`) while samples flow at f1. That is exactly S2's wrong outcome.
- §5 item 3 and S10's "cancelled start" only hold when the start has no `at`. §5 item 2's "the planner ignores it" is false when a later Stop precedes the start in effective order.
- The generator gives every `StartRx` `e` = its receipt (`generator/mod.rs:121,139`), so the differential cannot see this, and an oracle that keeps its own flag in arrival order would agree with the flaw.
- **Fix (recommended):** decide "streaming" from the line's final state after replay (`on`) instead of a separate flag. That keeps D2's refused-start rule and removes a second record of the same fact. Add the sequence above as a test, and add an `at` to generated `StartRx`. Rejected alternative: a `Stop` that cancels pending starts, which changes RM-21 and MockRadio parity.

Rev 2 broke nothing else. Rev 1's N3-35 (restart ignores the start lead) has no row now, but the past-origin test's `applied` assertion still covers it.

Not checked: I did not build or run anything; the timeline behaviour above is read from code. I did not read UHD C++ source or the server's `refused` window code (it does not exist yet).
<!-- opus-s3-fix-recheck.md -->
**Verdict: CLOSED.** None of the three P2 fixes leaves a major finding. All eight rows N47S3-34..41 die when probed one at a time in a throwaway copy, and that copy has been deleted. No test processes are left running.

**Row 1 (refusing a second start):**
- **Revision of a booked start:** none left. When a start is accepted, every earlier Stop takes effect at or before it and was booked earlier (`e = now`). Any start booked for the future would already count as streaming. So the accepted start always replays last (only an End can follow), and the plan only grows at the end.
- **Stop between booking and the start:**
  - A Stop that takes effect before the start's instant does nothing: the stream is already off.
  - A Stop after that instant but before the origin cuts the segment to nothing. The next start then begins at least one lead after that origin, so uhd-rx's cursor cannot pass it (this relies on the lead being greater than 0).
- **Other cases checked:**
  - A halted stream counts as streaming until a Stop.
  - Loss and `Provider::stop` book nothing afterwards (`control.rs:214`; MockRadio's `refuse_actions`).
  - Each of two streams uses its own line.
  - A stream with no channel is never streaming. `Staging::set` only asks about the channel's own stream, which holds that channel, so staging never reaches such a stream. Its start is accepted and does nothing in the replay.
- **Do the differential layers exercise booked-ahead starts?** Yes. In the copy I booked the refused starts again. Layer 3 then failed 102 of 1000 seeds, among them "segment 1 at 437460000: 0 blocks, the plan's 146" (seeds 38 and 159), which is Sol's skipped segment. Layers 1 and 2 count the Providers' refusals; layer 3 only checks that the shared decision agrees.

**Row 3 (ids on booking failures):** closed for uhd-control: `Stop`, `start_rx` and the clock-end failure in `changed` carry the id, and the submit reply returns it.

**Row 2 (bad channel keys):** closed. `session.py:369` rejects bool, non-int/non-str and negative keys before indexing. The suite passes against the server built from the copy, and N47S3-40 dies.

**Findings:**
- **P3, MockRadio overflow kills the Run** (`crates/ezsdr-mock-radio/src/lib.rs:1511`). This was already true at HEAD. `book_rx(...)?` passes the overflow up as an error from `step`. Sequence on a stepped Run: Stop, then `start_rx(at.ticks = i64::MAX)`. The reply is `Error Ended` with "KC-30: radio: RM-25: tick arithmetic overflowed" and the whole Run fails. The fake device instead refuses with the Action's id and the Run continues. Python sees an error, so the call does not look successful; still, the claim "every immediate booking failure carries its id" is false for Mock, and the snapshot is inserted before the failed booking. Fix: in that arm, turn the error into `reject_action_at(Some(id), "command", "RM-25: …")`, and add a stepped run to the `ea_11_a_start_the_line_cannot_represent_…` overflow test.
- **Nit:** `core.rs:676` labels the clock-end failure kind `stop` even when a Start booking reached it. It is untested (plan note 12).
- **Nit:** no test passes a float or `None` channel key.
- **Nit:** two lines of RM-25 in `design/07-radio-model.md` are now stale:
  - line 262 says "until the next `start_rx`, Stop or end cuts it", but a start can no longer cut a halted stream;
  - line 261 says "MockRadio refuses no `start_rx`", which is no longer literally true.

**4(a): is `|| halted` unreachable?** Yes. For any start a Provider accepts, the replay state just before it is off; a channel-less stream is never halted. When the stream is off nothing is running, so the `self.cut(stream, item, false)?` in that Start branch is also a no-op for every Provider input. What to delete:
- **Code:** in `crates/ezsdr-radio/src/timeline.rs:468`, drop `|| self.halted` and the `self.cut` line with its comment.
- **Rows:** delete P1059, which is now equivalent. J04 is equivalent too; only a pure test with an input no Provider can produce kills it, so delete it or keep it only as a test of the pure function.
- **Pure tests:** rewrite `rm_25_a_start_after_a_refusal_begins_from_the_refused_segment_s_cut` (`crates/ezsdr-radio/tests/timeline.rs:542`) as Stop then start, and do the same for the H04 case in `rm_26_the_timeline_cases`.
- **Keep:** the `halted` field and rows P90 and P460, which a Stop followed by a start still reaches.

**4(b): should `ur_25_a_short_lead_and_a_missed_start…` get a bound?** Yes, but not a wall-clock one. At `rx.rs:2530`, advance the fake device's clock to `origin.max(completion) + core.ticks(5_000_000)` before the first `turn()`. I checked this in the copy:
- With `LateStart` removed, the test fails in 0.00 s on "the owner restarted the missed start" instead of hanging.
- With `LateStart` in place, both tests that use the helper pass.

I did not run the full workspace, clippy, Python 3.9 or `rules.py`, nor repeat N47S3-37's wall-clock fake test under load. That test is killed only by its wait for the refusal timing out after about 12.6 s, which happens the same way every run.
