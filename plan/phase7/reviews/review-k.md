# Review K — the reviewer's report

The re-review of Review J's fixes, by the same Claude Opus reviewer resuming its Review J session (luna-primary-engineer's sticky re-review), brief [`../prompts/review-k.txt`](../prompts/review-k.txt), 2026-09-27, read-only. Reproduced as delivered; the triage is in [`../design-notes.md`](../design-notes.md) §5.

---

VERDICT: CHANGES_REQUIRED

Review K: a re-review by Claude Opus (the Review J reviewer, same session), read-only. Inputs: fix-delta-1.md, design-notes.md §4, and the revised plan/phase7/{00-overview,18-uhd-radio,19-amendments,bench}.md, checked against the code at 7cc6147 (coordinator/{admission,ending,pipeline,state,stepping}.rs, run.rs `Lease`, stream/burst.rs, the capture Sink, the server) and the UHD 4.10 headers. Probe: one Python computation (below). No repository file changed.

Summary: every Review J P0 and P1 is fixed in the text, and the fixes are mostly sound. The new concurrency has no hard deadlock in any Phase 7 configuration: KC-24a's admission lock, the data thread's inline RS-6 steps 1–2, the Lease mirror and the paced `schedule`/`cancel` rules were each walked against `Shared`'s locks and the Module slots. However, the unified cold-change rule plus the moved switch ownership break the path the Phase 6 snippets depend on (enabling transmit, then transmitting at once). Several new tests also contradict their own rules or cannot kill their mutations.

BLOCKERS:

N-P0-1: Enabling a stream (0 → n channels) now starts its SampleClock ≥ ~100 ms after the Action, on a thread that is not running. The immediately following untimed burst is admitted to an instant before its own clock's origin, and no rule says what happens to it. This breaks `tx.repeat` (Python), §57, `minimal.py` and bench B7 on the USRP.
- Location: spec 18 UR-25 ("The effective instant `e` … moved to at least `now + 50 ms`"; "`e₂`, the first instant at or after `e₁ + 50 ms`"; "A change from 0 channels has no old stream and follows the same steps from the configuring on"), UR-15, UR-21, UR-13; spec 19 VE-2 RM-25 ("`e₁ = e` when no clock runs … `e₂` … at or after `e₁` plus the Provider's restart lead").
- Evidence:
  1. Python `tx.repeat` submits `radio.tx.channels = 1` and then `start_repeat` (`python/ezsdr/session.py:213-219`).
  2. The cold change is booked with `e ≥ now + 50 ms` and `e₂ ≥ e + 50 ms`, so the new transmit clock is registered with origin ≈ now + 100 ms.
  3. RS-19 admits the untimed `start_repeat` at `now + 5 ms` (UR-10's lead). KC-24 finds that clock (the record exists, `ended_at` absent), and `admit_burst_target` converts the root instant onto it with no lower bound (`stream/burst.rs:35-54`), which yields a negative tick about 95 ms before the origin.
  4. UR-21's refusal list has no "before the stream's origin" case, and its late decision says `OnTime`. RM-15, RM-14 and MR-16 are silent too.
  5. The held burst goes to uhd-tx. UR-25 names uhd-tx as the switch's owner, but UR-15 starts uhd-tx only "when the transmit clock exists" at `start`, and UR-13 opens no transmit streamer for a count of 0. So nothing is running to reopen, configure and start the stream.
  6. Every outcome contradicts a rule. Sent before the switch, the burst transmits unconfigured (Review J's P0-3 again). Sent after configuring at `e₁`, its time spec is in the device's past, so it is dropped as `late_at_device` (B7 expects "`y` holds the waveform" and "no `TIME_ERROR`"). Either way its samples precede their clock's origin, which UC-3 as KG-13 amends it says "belong to no clock".
  7. `ur_25_tx_channels_from_zero_admits_the_next_burst` checks only admission. `ur_25_a_rate_change_admits_a_burst_on_the_new_clock` expects "transmitted from `e₂`", which no rule provides (a `send_asap` move to `e₂` would also emit `TIME_ERROR`). MockRadio is unaffected (restart lead 0), so this is a new Mock/USRP divergence on §59's main path. The first draft avoided it by registering a from-zero clock at the current instant.
- Severity: P0. The design is not implementable as written on the snippets' path, and it contradicts UC-3 and RM-25's "neither delivered nor a gap".
- Fix (recommended):
  1. A change from 0 channels has no stream to stop, so it needs no restart lead. uhd-control has the owner (started idle at `start` for both directions, or started here) configure the direction synchronously before its next `recv()`. KC-21a already bounds that wait, and configuring is a device call, not a wait for a device instant, so UR-14 still holds. It then registers the new clock at the first lattice instant at or after the configuration's completion. A burst admitted after KC-21a is then on a configured, started stream.
  2. Independently, amend RM-15 and UR-21 so that a stream's origin is a lower bound of the earliest start, like `S` and the preemption bound (the `LatePolicy` applies), so a burst before its clock's origin can never be `OnTime`.
  3. `ur_25_tx_channels_from_zero_admits_the_next_burst` should assert transmission at the admitted instant with no `TIME_ERROR` on the fake, not only admission.

NONBLOCKING:

New P1:

N-P1-1: KC-46a's pending-wake guard has a check-then-store race. It can suppress every later wake for the rest of the Run, a lost wakeup of a new kind.
- Location: spec 19 KC-46a ("unless a wake it scheduled has not fired yet … the callback clears it when it fires"); UR-7 and §0 `WallAuthority` (callback execution).
- Evidence: the data thread checks `Shared.wake` is empty, calls `schedule(now)`, and stores the returned handle. `schedule` notifies a waiting `next_wakeup` and the instant has already passed, so the control thread can fire the callback before the data thread stores the handle. The callback then clears nothing, and the stored handle names a callback that has already fired. `wake` stays `Some` until RS-6 step 1, and from then on every `wait_for` returns only at its horizon (Python's `capture` waits for `CAPTURE_WRITTEN` this way). Holding `Shared.wake`'s lock across `schedule` closes the race only if a paced Authority runs callbacks without its own lock, which neither UR-7 nor `WallAuthority` states. With the opposite choice (callback under the Authority lock, data thread holding `wake` across `schedule`), the two threads deadlock. `kg_02_wait_for_returns_when_the_data_thread_delivers` delivers one event, so it cannot see this.
- Fix: bump a generation or set the pending flag before calling `schedule`, and have the callback clear it only for its own generation. Alternatively, hold the `wake` lock across `schedule` and add to MA-29 that a paced Authority runs callbacks without holding a lock `schedule` takes. Add `kg_02_every_delivery_wakes_a_waiting_call` (for example 50 sequential `wait_for`s of 50 marks, each returning in well under its horizon).

N-P1-2: The preemption bound races uhd-tx's continuing hand-over. A `send_asap` preemption is placed exactly on the moving boundary.
- Location: spec 18 UR-21 ("`now'` … the first sample of that burst uhd-tx has not yet handed to the device less the device lead"), UR-23; spec 19 VE-2 RM-15; test `ur_21_a_burst_inside_the_in_flight_window_is_late`.
- Evidence: uhd-control reads the first unhanded sample F on its own thread while uhd-tx keeps handing buffers to hold the in-flight window full, so F advances continuously. `SendAsap` moves the target to `max(now + 2 ms, F)`, which is F whenever a repeat is running (window 10 ms > lead 2 ms). If uhd-tx hands `[F, F + buffer)` before it picks up the held burst, the new burst's start-of-burst time lies inside samples the device already holds. The device then drops it (`TIME_ERROR` late), although the burst was decided on time and flagged `send_asap`. The named test expects "the fake transmits it there without a `TimeError`", which is timing-dependent against a faithful fake (UR-33's transmit queue).
- Fix: make the decision atomic with uhd-tx's hand-over. uhd-tx computes the bound itself when it takes the burst, and either moves it (per `LatePolicy`) or reports it. Otherwise uhd-control uses `F + one buffer + a margin` and uhd-tx refuses, with `TIME_ERROR { refused }`, a burst it can no longer honour.

N-P1-3: `Session.after`'s "computed exactly (Python's `fractions`)" contradicts its test, and the test passes the mutant V11 and fails the specified implementation.
- Location: spec 19 VE-6 (EA-16 `Session.after`), `test_after_names_an_instant_ahead`, mutation V11.
- Evidence (probe, `python3`): `math.ceil(Fraction(0.001) * 10**9)` = **1000001**, and `math.floor(...)` = 1000000. For 0.05, 0.1 and 0.01 the ceil is likewise one tick above the round value. The test expects "`after` equals the `now` read next plus 1 000 000 ticks". A correct exact-ceil implementation fails it, and V11 ("rounds the tick count down") passes it.
- Fix: specify the decimal the user wrote (`Fraction(repr(seconds))` or `Decimal`), and test with a value whose ceil and floor differ in ticks (for example 1.5 ns on a 1 GHz root), so V11 dies.

N-P1-4: `kg_04_no_action_is_dispatched_after_the_freeze` asserts something a correct implementation violates.
- Location: spec 19 KG-4 test table; mutation G29.
- Evidence: the expected outcome is "every entry whose Action was not received is logged `Rejected` with `ezsdr.dispatch`". An entry admitted and dispatched just before the freeze, whose Action RS-6 step 1 then clears from the queue before the `ThreadedProvider` (1 ms poll) takes it, is correctly logged `Admitted` (KC-28 appends before KC-21a's wait) and never received. With 50 Sessions and a submit loop this happens routinely, so the test is flaky on a correct implementation. The kill of G29 is probabilistic as well.
- Fix: instrument the queue so that no push may follow `frozen` being set, and assert that. Drop the log-outcome claim, or allow "Admitted, cleared by the freeze".

N-P1-5: `ur_25_enabling_tx_applies_the_configuration` requires an order opposite to UR-25's.
- Location: spec 18 §6 test table vs UR-25; mutation U39.
- Evidence: the test expects "an untimed `apply` of rate, 2.4 GHz, 5 dB and the antenna … **before the transmit clock is registered**". UR-25 has uhd-control register the new clock at booking ("before taking the next Action … registers the new one at `e₂`; then it hands the switch to the stream's owner"), and the owner configures later, at the switch. U14 says "before its clock **starts**". A correct UR-25 implementation fails the test. N-P0-1's fix makes the from-zero order "configure, then register", which removes the contradiction for this case.
- Fix: align the test with the chosen order ("before the clock's origin / before any sample on it").

New P2 (one line each):
- N-P2-1 Appendix A G06 still names `kg_02_every_step_runs_on_the_data_thread`; the test is now `kg_02_every_step_before_finish_runs_on_the_data_thread`.
- N-P2-2 KC-36's mirrored deadline is "in the host clock's nanoseconds", but `Lease.expires_at_host` is `monotonic_millis` (`run.rs:324`, `:401`, `:410`). Mirror the millisecond value.
- N-P2-3 MA-29's added sentence ("then returns the earliest instant still scheduled") reads as returning an instant before it is due. Say that it re-evaluates and waits, or returns `None`.
- N-P2-4 While the data thread is inside `Provider::stop` (KC-46b; up to about 3.1 s for UHD by UR-16/UR-26, unbounded for another Provider), no Sink is stepped, no event is drained and no Lease is checked. An orderly data-thread end, such as a Lease expiry, therefore drops receive samples a `finish` would keep. This is P2-9's trade-off; state it or bound it.
- N-P2-5 UR-3's control-call mutex can delay `time_now` behind a UBX `apply`. An anchor that takes the host `Instant` before acquiring the mutex (as the spike did) is skewed by the wait. Bracket the device read inside the mutex and discard wide brackets.
- N-P2-6 `ur_07_now_tracks_the_device_while_no_call_runs` allows 20 µs between two separate reads on a loaded machine, which is flaky. U35 would show about 200 µs at 100 ppm over 2 s, so use about 100 µs.
- N-P2-7 KG-14's TM-13e ("the first lattice instant at or after [the effective instant or the restart instant]") and UC-3 ("ends its SampleClock at that instant") differ from RM-25's `e₁` (old lattice) and `e₂ ≥ e₁ + lead`. State both in RM-25's terms.
- N-P2-8 UR-14 and UR-24 do not say whether a command already inside the release window at booking is released before the next `recv()`. `ur_14_actions_are_finished_before_the_next_recv` requires that it is. Say so.
- N-P2-9 "Configuring a direction" applies "the full current configuration". State that this is the configuration in effect at `e₂`, and that held commands with a later `e` stay held (UC-2: never before the effective instant).
- N-P2-10 UR-17's missed-start rule names only T0. A missed timed start at `e₂` after a receive switch is unspecified.
- N-P2-11 There is a latent lock cycle. The control thread holds KC-24a's lock and waits for a Provider slot (KC-26). The data thread holds that slot in KC-46b's `stop`, which joins a Provider thread waiting for the admission lock (a Provider submitting from its own thread, which MA-14a allows). The join bound breaks it after 1 s. It is unreachable in Phase 7 (UHD submits nothing); note it in KC-24a.
- N-P2-12 Timed stream commands (UR-25's stop at `e₁` issued ≥ 50 ms ahead, UR-17's restart, UR-15's start) sit outside UR-24's release discipline and outside UR-3's control-call mutex. If they share the radio's in-order command queue (INFERRED), a retune released later with an earlier `e` is applied late, silently. B8 should check this.
- N-P2-13 `mr_18_a_cold_change_starts_its_clock_on_the_lattice` at instant 1 000 037 precedes T0 under `x310-like` (start lead ≥ 2 s). Name the `ideal` profile.
- N-P2-14 Overview §7's server bullet still says "`EZSDR_PROFILE` is used", while the library now reads `Config.default_profile`.

TEST_GAPS:
- G09 is still not reliably killed. The mutant misorders cause and index only when the data thread and a control-thread round drain concurrently. `kg_02_the_first_delivered_stopping_event_is_the_termination_cause` makes no control call during the marks, so the data thread alone drains, and order holds with or without the lock. Fix: run a `wait_for` loop, whose rounds drain, while the marks fire.
- G31 ("`FreezeDispatch` leaves the pending wake set") is not killed by `kg_03_a_cancelled_horizon_wakes_the_control_loop`, because KC-46b's wake is unconditional and wakes the loop anyway. For the same reason, that test does not isolate MA-29's new cancel-wakes rule on the Kernel side (UHD's `ur_07_cancel_wakes_a_waiting_next_wakeup` does).
- G29's kill is probabilistic, and its test's second assertion is wrong (N-P1-4).
- V11 survives `test_after_names_an_instant_ahead` (N-P1-3).
- U39's killer contradicts UR-25's order (N-P1-5).
- `ur_21_a_burst_inside_the_in_flight_window_is_late` is timing-dependent (N-P1-2).
- `ur_25_tx_channels_from_zero_admits_the_next_burst` checks admission only and misses N-P0-1. `ur_25_a_rate_change_admits_a_burst_on_the_new_clock`'s "transmitted from `e₂`" has no rule behind it.
- The wake race (N-P1-1) has no test; every wake test delivers a single event.
- `ur_07_now_tracks_the_device_while_no_call_runs`: its 20 µs tolerance is flaky (N-P2-6).
- G33 mutates the Kernel's test double. The Kernel has no paced Authority of its own, so its only real-code carrier is U33 in the UHD crate. Acceptable; name it as such.

PREVIOUS_FINDINGS:
- P0-1 CLOSED: KG-9 amends TM-16c's `InPast` for paced Authorities; `WallAuthority` and UR-7 follow; tests and G33/U33 exist.
- P0-2 CLOSED in text: RM-15 is amended with the preemption bound, UR-21/UR-23 follow, the fake has a transmit queue, and the test is at 20 ms. A new race in the bound is N-P1-2.
- P0-3 CLOSED: "configuring a direction" is defined and required on enabling; the fake is frequency-aware; `hw_b1_probe` prints the TX settings. The from-zero timing and ownership broken by the rewrite is N-P0-1.
- P0-4 a CLOSED (KG-14 TM-13b).
- P0-4 b CLOSED (KG-14 TM-13e; wording residue N-P2-7).
- P0-4 c CLOSED (RM-16 amended; UR-24 holds commands).
- P0-4 d CLOSED (one RM-25 rule; MR-18 is its zero-lead case; V13).
- P1-1 CLOSED: the wake handle is kept apart, step 1 clears it, the wake is unconditional, and cancel wakes a wait. A different lost wakeup is N-P1-1.
- P1-2 CLOSED (KC-24a; KC-21a corrected); its test is flawed (N-P1-4).
- P1-3 CLOSED (KC-36 mirror; unit nit N-P2-2).
- P1-4 CLOSED (`uhd-clock` 100 ms; fake drift; nits N-P2-5, N-P2-6).
- P1-5 CLOSED (EA-17 made honest; `root_rate`; `after`; `ea_17_*`); its test is broken (N-P1-3).
- P1-6 CLOSED for updates (holding and release; the fake's command FIFO); stream-command residue is N-P2-12.
- P1-7 CLOSED for a running stream (owners named; uhd-control never waits); the from-zero case is N-P0-1.
- P1-8 CLOSED (UR-16/UR-26; Arc-held streamers).
- P1-9 CLOSED (MA-46 and the `ma6_documents!` list in KG-11 and GZ-2).
- P1-10 CLOSED (failed instances are never re-stepped; the data thread stops after an abort; tests G32/G35).
- P1-11 CLOSED (thread-id tests restricted to before `finish`; capacity 64); stale mutation name N-P2-1.
- P2-1 through P2-23 are all CLOSED:
  - P2-1 `Config.open_device` named.
  - P2-2 `Config.default_profile`, with a child-process test.
  - P2-3 wake handle kept apart.
  - P2-4 KC-15 bound stated.
  - P2-5 reason corrected.
  - P2-6 KC-29 amended.
  - P2-7 MA-20 amended.
  - P2-8 10 ms re-check and notifying `clear`.
  - P2-9 stopper thread removed (trade-off N-P2-4).
  - P2-10 latency stated.
  - P2-11 `Time` dropped.
  - P2-12 idle time read, and `RUNTIME` only on streaming calls (whether a lost 10 GbE device's control read reports IO rather than RUNTIME stays INFERRED; B9 checks).
  - P2-13 exact `N`.
  - P2-14 empty trim dropped.
  - P2-15 UR-30 note.
  - P2-16 control-call mutex (residue N-P2-12).
  - P2-17 ceiling.
  - P2-18 Phase 8 inputs table.
  - P2-19 TM-18/UR-8 text.
  - P2-20 rpath removed.
  - P2-21 token-ban note.
  - P2-22 full Session profile, dedicated USRP2 probe, B5 stall and buffers.
  - P2-23 suite time.
- TEST_GAPS (Review J):
  - `ur_23` at 5 ms: CLOSED (20 ms; transmit queue).
  - Frequency-blind fake: CLOSED.
  - Thread-id tests: CLOSED.
  - Capacity-4 drain test: CLOSED.
  - G07: CLOSED (always-progressing Sink).
  - **G09: OPEN** (see TEST_GAPS).
  - G12: CLOSED (off-data-thread step criterion).
  - G18: CLOSED (withdrawn as equivalent; the argument holds, since every device-paced loop has a horizon scheduled).
  - G16: CLOSED (mechanism named).
  - U13: CLOSED (20 ms `apply`; see N-P2-8).
  - U19: CLOSED (20 ms margin).
  - U22: CLOSED (the "against the new clock" assertion kills it; see N-P0-1 for the other clause).
  - U16: CLOSED (restart record).
  - Missing tests: CLOSED (Detached Lease, drift, far-future update, rate mismatch at the switch, Hardware class added), with the flaws of G29/G31 noted above.
  - `ea_09` environment race: CLOSED.
  - VE-6 carrier: CLOSED (`ea_17_*`).
- Remaining OPEN from Review J: 1 (the G09 test gap).
