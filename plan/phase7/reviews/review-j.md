# Review J — the reviewer's report

Adversarial review of the Phase 7 design by Claude Opus (AGENTS.md §8), brief [`../prompts/review-j.txt`](../prompts/review-j.txt), run on 2026-09-27 under the luna-primary-engineer review protocol (read-only; probes in a scratch copy outside the repository). Reproduced as delivered; the triage is in [`../design-notes.md`](../design-notes.md) §4.

---

VERDICT: CHANGES_REQUIRED

Reviewer: Claude Opus (independent, read-only). Scope: plan/phase7/{00-overview,18-uhd-radio,19-amendments,bench,design-notes,vision-issues}.md at HEAD 7cc6147 plus the uncommitted plan/phase7/ tree, checked against the code, the accepted specs, the spike branch and the UHD 4.10 headers in /opt/homebrew/include.

Evidence base and probes (all in a scratch copy outside the repository, fresh timestamps, CARGO_TARGET_DIR=$HOME/.cache/cargo-target/Ez-SDRv4-review):
- Probe 1 (KG-7): `set_start_instant` patched to round T0 up to L = lcm of the numerators of `declared_sample_clocks()` on the primary root, recording every move to a file. `cargo test --workspace --no-fail-fast`: 683 passed, 1 failed. The only T0 that moved in the whole suite is `k3_an_off_grid_start_lead_moves_a_burst_to_the_next_transmit_sample` (`T0 2000000500 -> 2000001000 L=1000`), which then fails with `left: (10001, false) right: (10002, true)`, i.e. the burst becomes exact. **The design's honesty claim holds: KG-7 moves no other existing Simulation test's T0.**
- Probe 2 (KG-11): `fn relations(&self) -> Vec<ClockRelation> { Vec::new() }` added to `Authority`. `cargo test -p ezsdr-kernel --test kernel_surface`: `ma_06_role_signatures_name_only_documents_and_handles ... FAILED` with `MA-6: Authority::relations names ["ClockRelation"]`. `ov_23b` passed (NEW count unchanged, as claimed). See P1-9.
- UHD C API: every one of the 66 functions UR-3 names exists in the UHD 4.10.0.0 headers (a grep for each name followed by `(` over uhd.h and uhd/ found none missing). The four structs exist (`usrp.h:58` `uhd_stream_args_t`, `usrp.h:91` `uhd_stream_cmd_t`, `tune_request.h:42`, `tune_result.h:30`). The error codes UR-29 names exist (`error.h`: USB 21, IO 30, OS 31, RUNTIME 44). UR-3's time conversion checks out: `rem < mcr < 2^28`, so `round((rem/mcr)·mcr)` returns `rem` exactly, and `-1` gives `(-1, 0.999999995)`.
- Overview §2 evidence cells: all 18 re-checked against 7cc6147 (`pipeline.rs:376`, `stepping.rs:119/217/349-356`, `pipeline.rs:763-771/842-850/867/1131/1374`, `ending.rs:258`, `design/01-time-model.md:173` duplicate sentence, `state.rs:30-37`, `event.rs:544-551`, `v58.rs:635`, `catalogue.rs`, `session.py:301`). Every cell is correct. Missing holes are listed as P1-3, P1-4, P1-5 and P1-10.

BLOCKERS:

P0-1: KC-46a's wake, KC-29's `advance_to`/`wait_for` and UR-7 all rely on scheduling at or before the current instant of a moving clock, which TM-16c forbids and no amendment relaxes.
- Location: spec 19 KG-2 (KC-46a), KG-9 (TM-16c); spec 18 UR-7; spec 19 §0 (`WallAuthority`).
- Evidence: TM-16c (design/01-time-model.md:173): "`schedule(t, f)` requires `t` in a governed domain at or after `now`, failing with `InPast` otherwise". The trait doc (time/authority.rs:52) says the same: "Fails with `InPast` when `t` precedes `now`." KG-9 replaces only "the callback clause" and deletes the duplicate `cancel` sentence; the `InPast` clause stands. KC-46a: "The data thread wakes the control loop by scheduling a no-op callback on the Authority at the current instant". On a Device- or WallPaced Authority, `now` has advanced by the time `schedule` checks it, so a TM-16c-conformant Authority refuses almost every wake. KC-46a says a refused wake "is not retried", so the wake mechanism fails. The same race breaks existing code: `stepping.rs:455-462` checks `t.ticks <= now` and then calls `schedule(t)`, mapping `Refused` to `Err(NotOnPrimaryRoot)`. A Python `sdr.sleep(tiny)` or a short `wait_for` horizon on a device root would then fail spuriously. UR-7 quietly substitutes a different rule: "at or after the last fired instant (`InPast` otherwise)". This is the spike's `if tick < s.fired` (`origin/spike/uhd:spike/uhd/src/authority.rs`). It contradicts TM-16c without amending it. The `WallAuthority` double (§0) states no `InPast` rule at all.
- Severity: P0. It contradicts a normative rule that it does not amend, and the mechanism is load-bearing for KG-2, KG-3, KG-5, KC-29 and KC-29b.
- Fix: in KG-9, amend TM-16c. Under a `WallPaced` or `Device` Authority, `schedule` refuses only an instant before the last fired instant. An instant at or before `now` fires at the next `next_wakeup`, which returns that instant. State the same rule for `WallAuthority` and for UR-7, and add `tm_16c_a_paced_authority_accepts_an_instant_already_passed`.

P0-2: A 10 ms transmit in-flight window combined with a 2 ms device lead makes UR-23's "a burst ends at the next held burst's start" impossible. RM-15 is not amended, and the named test and FakeDevice hide the failure.
- Location: spec 18 UR-23, UR-21, UR-9 (`min_timed_command_lead_ns` 5 ms), decision U7; spec 19 VE-2 (the RM-16 sentence only); RM-15 (design/07-radio-model.md).
- Evidence: UR-23: "uhd-tx keeps at most 10 ms (`tx_in_flight_ns`) of samples sent ahead of the device's time", and a burst ends "at the start of the next held burst — the current one is sent up to the sample before the next start". VE-2's own RM-16 sentence admits a Provider "cannot recall" samples it has handed over, but the amendment covers only `Stop`. RS-19 admits an untimed Session send at `now + 5 ms`. It is received up to 3 ms later and judged `OnTime` against the 2 ms device lead (UR-21). If a repeat is running, its samples up to device `now + ~10 ms` are already on the device, so the new burst's start-of-burst time is in the past when it reaches the head of the device FIFO. The device then reports `EVENT_CODE_TIME_ERROR` and drops the burst (→ `late_at_device`). Meanwhile the repeat's record claims it ended at the new burst's start. MockRadio starts the new burst on time, so the same Session diverges between Mock and USRP (§59). The named test `ur_23_a_burst_ends_at_the_next_bursts_start` uses "a repeat, then a burst 5 ms later", which is inside the window. FakeDevice's loopback is keyed by instant ("the transmitted sample whose instant it is", UR-33), and a new burst's time spec is not "in its past" when sent, so the test passes on the fake while it fails on hardware.
- Severity: P0. The rule cannot be implemented as written, and RM-15 is contradicted without amendment.
- Fix (recommended): the late decision for a burst that would preempt an open burst uses `max(now + device lead, first sample after the samples already handed over)` and applies the burst's LatePolicy to that bound: `send_asap` moves the start there with `TIME_ERROR { send_asap }`, and `drop` drops the burst. Amend RM-15 as VE-2 amended RM-16 (the previous burst ends at the later of the two instants, and its record says so). Make FakeDevice a FIFO: a timed start-of-burst earlier than the end of already-queued samples → `TimeError`. Rejected alternatives: an in-flight window ≤ the device lead (underflow risk, unmeasured), or a declared lead ≥ 12 ms (every Session command becomes slower).

P0-3: Enabling a stream or adding channels at runtime configures nothing on the device. That is the Phase 6 snippets' own path, so a USRP transmits with its leftover TX frequency and gain while RM-19's envelope judged the configuration.
- Location: spec 18 UR-25 ("A change from 0 channels …"), UR-12, UR-24, UR-9 (defaults "rx 1 channel, tx 0"); bench.md B7.
- Evidence: UR-12 applies settings only "for each direction whose channel count is > 0". The default transmit count is 0, so prepare never configures the TX frontend. UR-24: "A direction with no channel records the value and applies nothing (the configuration holds it for when the stream starts)". UR-25's 0 → n transmit case: "uhd-control opens the streamer, registers the new clock … and starts uhd-tx and uhd-async". The receive case "opens the streamer, registers the new clock at e₂, issues the timed start". Neither applies rate, frequency, gain or antenna. A count increase ("the streamer reopened for a new channel count") leaves the added channels unconfigured. Python `tx.repeat` performs exactly this 0 → 1 change (`python/ezsdr/session.py:213-219`), and so do `examples/minimal.py`, §57 and bench B7. On the X310, the TX LO, gain and DUC rate are whatever UHD or a previous session left. Admission (RM-19, KC-27) judged `radio.tx.frequency_hz = 1e9` and `gain_db = 0`. The transmit SampleClock's ratio may not even match the device's rate. FakeDevice's loopback ignores frequency, so `ea_07_a_session_on_the_fake_device` and `uhd_57_the_session_loopback_captures_what_it_transmits` pass.
- Severity: P0. It is an RF safety problem, and it breaks the guarantee that the admitted configuration is what the device applies.
- Fix: UR-25 enabling a direction, or raising its count, must first apply the direction's full current configuration to every channel of the new count: untimed, read back, with UR-12's exact-rate rule. A mismatch is `COMMAND_REJECTED` with no clock change. Only then register the clock and start the threads. Add `ur_25_enabling_tx_applies_the_configuration` (the fake records every `apply`). Make the fake's loopback require equal TX/RX frequency. Add a bench B0 check that prints the device's TX frequency and gain before any transmitting step.

P0-4 (text only): four producer rules are contradicted by the design without amendment. Each fix is one or two sentences in spec 19.
- (a) TM-13b vs UR-17. TM-13b: the receive origin "must be the root tick of the stream's first sample, so that the first block of the stream carries `ticks = 0` … This is not a recommendation." UR-17's missed start keeps origin T0 and publishes a first block "with `GAP_BEFORE` with `lost` its index" (ticks > 0). Fix: amend TM-13b in spec 19 to allow exactly UR-17's form (the origin stays at the requested start, and the first block carries `GAP_BEFORE` with `lost` = its index). That keeps §61's index↔instant mapping.
- (b) TM-13e vs RM-25/MR-9 and UR-25. TM-13e: the transmit origin is fixed "from the root tick at which the stream is armed", and a stream re-created by a cold update "takes that update's effective instant as the origin". RM-25 moves the arm origin to the next lattice instant. UR-25 starts the new clock at `e₂ ≥ e₁ + 50 ms`, which is not the effective instant `e₁`. The Amends line of spec 19 lists only TM-16c and TM-18 in spec 01. Fix: amend TM-13e with RM-25's lattice and KG-13's restart instant.
- (c) RM-16 vs UR-26. RM-16: "`Provider::stop` and a `Stop` for `<device>` also cancel every pending timed command". UR-26: "UHD cannot recall a timed command already issued; those are recorded in `applied` as issued". Fix: hold timed commands host-side until a release window (see P1-6), which makes cancellation real, or amend RM-16 with the device limitation, as VE-2 did for in-flight samples.
- (d) RM-25 (VE-2) vs UR-25. RM-25: the cold change's lattice instant "becomes the change's effective instant: the old clock ends there". UR-25 ends the old clock at `e₁` on the old clock's lattice and starts the new one at `e₂` on the new lattice, after the restart lead. MockRadio (VE-4) instead ends at the new lattice instant, so the two Providers follow different rules for one Vocabulary rule, which Phase 8 will compare. Fix: write one RM-25 cold rule that cites KG-13's restart lead, and make MR-18 (VE-4) the zero-lead case of it.
- Severity: P0 under the brief's letter (normative rules contradicted and not amended). Each fix is text, and none needs code beyond the owning rule.

NONBLOCKING:

P1-1: Lost wakeup. KC-46a's "unless one it scheduled earlier has not fired yet" can suppress KC-46b's final wake.
- Location: spec 19 KC-46a, KC-46b, §0 `WallAuthority`; MA-29.
- Evidence: in the pass that requests an end, the data thread starts the stopper and schedules wake W1 (`wake_pending = true`). The stopper's RS-6 step 1 cancels every scheduled handle, W1 included (`ending.rs:442-448`: cancel does not fire). The flag is never cleared, so the stopper's own wake is skipped. A control loop waiting in `next_wakeup` on the now-cancelled horizon is then woken only if the Authority notices the cancel. `WallAuthority` is specified to wake only "when an earlier one is scheduled", so it sleeps until the cancelled horizon. `kg_10_a_device_lost_from_a_provider_thread_aborts_the_run` ("within 1 s" of a 5 s horizon) therefore becomes racy. The Authority contract (MA-29, TM-16c) says nothing about `cancel` and a waiting `next_wakeup`.
- Fix: the stopper's wake is unconditional. FreezeDispatch clears `wake_pending`. MA-29 states that a `cancel` wakes a waiting `next_wakeup`, which returns `None` when nothing remains (UR-7 and `WallAuthority` alike).

P1-2: Admission and dispatch are no longer serialized, but the design treats them as if they were.
- Location: spec 19 KC-21a, KC-46b; `admission.rs:37-42`, `:246-256`; `ending.rs:431-449`.
- Evidence: `admit_with` checks `frozen` at entry, and `dispatch` pushes onto the queue without re-checking. KC-46b runs FreezeDispatch (set `frozen`, clear queues) on the stopper thread concurrently with `submit`. An Action admitted just before the freeze is pushed after the clear, logged `Admitted` with an id, and may reach uhd-control while `stop` is still joining it. RS-6 step 1 ("refuse every further submission, empty every undelivered Action queue") no longer holds. KC-21a also says "An Action a Module submits from inside a step (MA-14a) is not waited for, since no control call is running". That is false: the data thread steps Executors and Sinks concurrently with control calls. `submit` and `admit` each judge a clone of `configuration` (`pipeline.rs` `lock(&run.shared.configuration).clone()`), which is the stale-configuration hazard KC-17's working copy exists to prevent. Phase 7 exposure: the freeze race is reachable; the Module-origin race is latent (no Phase 7 data-thread instance submits Actions).
- Fix: one admission mutex in `Shared`, held from admission through dispatch and by FreezeDispatch, or dispatch re-checks `frozen` under the queue lock. Correct KC-21a's sentence.

P1-3: A Detached Lease's expiry is not enforced while no client call runs.
- Location: spec 19 KG-2 (KC-46: the control rounds "check the Lease"); KC-36; RS-22, RS-23.
- Evidence: `lease` is a field of `RunHandle` (`coordinator/mod.rs`), not of `Shared`, so the data thread cannot read it. KC-36 checks it "at every RunHandle call and after every round", and in a device-paced class rounds happen only inside calls. A disconnected Detached client leaves a transmitting repeat on past its TTL indefinitely. That is RS-22's own counterexample ("would keep a detached transmitter alive forever"), now on hardware. The server hides this today, because `Server::disconnect` finishes the Run on EOF (`ezsdr-server/src/lib.rs:170-176`), but the Kernel contract is broken for any embedder.
- Fix: keep the expiry deadline in `Shared` and have the data thread request `Stopped { lease_expiry }`, which the stopper then acts on. Otherwise refuse a Detached Lease in device-paced classes (KC-2a) until it does.

P1-4: The DeviceAuthority is never re-anchored while no client call runs. That is the free-running clock U4 rejects, and it defeats KG-8's fix for untimed Session commands.
- Location: spec 18 UR-7, U4; spec 19 KC-46.
- Evidence: UR-7 re-reads the anchor only in `next_wakeup` and `wait_until`, which run only inside `advance_to`, `wait_for` and `run_until_end`. `submit` never calls them, and neither the data thread (it calls `now`) nor uhd-control does. UR-8's own host drift bound is `drift_uncertainty: 1e-4`, so after 20 s of client think-time the extrapolation error can equal the whole 2 ms device lead. `now + 2 ms` timed commands are then in the device's past ("activated upon arrival", silently late: UC-6 is broken with no `LATE_COMMAND`), and bursts become `late_at_device`. FakeDevice counts ticks from the host `Instant`, so it has zero drift and no test can show this. B2 re-anchors continuously, and B7 sleeps right before capturing (which re-anchors), so the bench misses it too. Magnitude INFERRED; structure VERIFIED.
- Fix: re-anchor from every received block's `first_tick` in uhd-rx, and on a ≤100 ms timer in uhd-control. Add a FakeConfig drift in ppm and `ur_07_now_tracks_the_device_while_no_call_runs`.

P1-5: VE-6/EA-17's promise ("captures from the instant the sleep ended, whatever the client's round trip") is false in a device-paced Session, because KG-2 makes the Sink consume blocks between calls.
- Location: spec 19 VE-6 (EA-17); HD-10; `ezsdr-sink-capture/src/lib.rs:574-588`.
- Evidence: HD-10: a request "starts at the first delivered sample whose instant is at or after max(at, …)". The capture Sink's `step` drains and processes every queued block whether or not a capture is pending. The data thread steps it continuously, so the blocks covering `[t, admission)` are gone before the request arrives. The capture starts about one round trip plus KC-21a after `t`. The artifact's ContinuityMap is honest, but the stated behaviour (hole 17's fix) does not occur. The only Python test is on the Simulation class.
- Fix: amend EA-17 to say the capture starts at max(t, the first sample delivered after admission), with the ContinuityMap authoritative, and recommend a future `at`. Rejected: a pre-trigger buffer in the Sink (scope creep).

P1-6: UR-24 issues every `hardware_timed` update to UHD at receipt, but UHD's command FIFO is in order and back-pressures.
- Location: spec 18 UR-24, UR-26, UR-33; §2 "Timed commands".
- Evidence: `multi_usrp.hpp:332-337`: "A timed command will back-pressure all subsequent timed commands, assuming that the subsequent commands occur within the time-window" (VERIFIED). A Spec's scheduled retune at T0+10 s is dispatched at start and issued at once, so every later command on that queue waits behind it. That may include UR-25's timed stream stop and start and UR-17's restart (INFERRED: shared radio command queue). An update with an earlier effective instant queued after it is applied late, which breaks UC-2 and UC-6 (unreachable today through Kernel paths, reachable through Module-origin updates). UR-24 counts "16 timed commands" as Provider updates, while one UBX timed tune is several device register writes (INFERRED). The device FIFO can therefore fill first and block uhd-control inside UHD, which ends the Run through KC-21a's 5 s bound rather than `COMMAND_QUEUE_FULL`. RM-16's cancellation cannot work for commands already issued (P0-4c). FakeDevice has no FIFO.
- Fix: hold timed updates host-side in effective-instant order and release each to the device no earlier than a release window before its instant; count held plus issued updates against the depth. B8 measures how many timed UBX tunes fit before `set_rx_freq` blocks. The fake models an in-order FIFO.

P1-7: UR-25 does not say which thread performs the device-side switch at `e₁`/`e₂`.
- Location: spec 18 UR-25, UR-14, UR-2.
- Evidence: "The device side follows at the instants: … stopped by a timed stop at e₁ …, reconfigured (rate; the streamer reopened …) and restarted by a timed start at e₂". UHD's `uhd_usrp_set_rx_rate` is not a timed command (INFERRED), so some thread must wait until `e₁` (≥50 ms). If uhd-control does it, it blocks later Actions far beyond UR-14's 3 ms delivery allowance, and untimed sends after a rate change become `TIME_ERROR` (spike K6 again). If uhd-rx or uhd-tx do it, UR-2 and UR-3 must let those threads call `apply` and `rx_open`, and UR-3's "each streamer is used by one thread" must cover the reopen. There is also no rule for a runtime read-back that differs from the claimed rate: the clock registered at `e₂` would then be wrong (the fake's "a rate it applies wrongly" fault has no runtime test).
- Fix: name uhd-rx and uhd-tx as the switch owners, state that uhd-control never waits for a device instant, and apply UR-12's exact-rate rule at the switch (`COMMAND_REJECTED`, the old clock kept or ended with no new one).

P1-8: `cleanup` can free a streamer while a detached thread is still inside UHD, a use-after-free in the crate's only `unsafe` module.
- Location: spec 18 UR-26 vs UR-16 and UR-3.
- Evidence: UR-3: "`close_streams` runs only after those threads have been joined". UR-16: "a thread that does not join in time is recorded in `rejected` and left detached". UR-26: "`cleanup()` stops what is still running, closes the streamers (`close_streams`)", unconditionally.
- Fix: `cleanup` skips `close_streams` and records the leak when any streaming thread is detached. Each streamer handle is kept alive by an `Arc` its thread holds.

P1-9: KG-11's provided method fails `ma_06` and makes MA-46's sentence false. GZ-2 says neither changes.
- Location: spec 19 KG-11, overview GZ-2; `kernel_surface.rs:1125-1130`; MA-46.
- Evidence: Probe 2: `MA-6: Authority::relations names ["ClockRelation"]`. The test's closed `ma6_documents!` list lacks `time::ClockRelation`. MA-46 says its list "includes every document type a role-trait signature carries", and `ClockRelation` is not in it.
- Fix: KG-11 amends MA-46's list and the `ma6_documents!` list (the schema `clock_relation.v1.json` exists, so MA-46's frozen-schema assertion passes). GZ-2 records the change.

P1-10: In a device-paced Run, a failed instance is stepped again on every pass until the client's next call. A `DeviceLost` step error re-emits `DEVICE_LOST` on every pass.
- Location: spec 19 KC-46 ("an instance whose `step` fails is not stepped again at `t`"), KC-46b ("keeps stepping … meanwhile").
- Evidence: each pass takes a new `t`, so the failed Sink or Executor is stepped again about every 200 µs. `stepping.rs:196-199` calls `emit_device_lost` for every `DeviceLost` failure, through `emit_control` into an unbounded control-path vector, and each drain appends it to `delivered`. In Simulation the Run ends in the same call, so this never repeats. Here it repeats for as long as the client does not call: unbounded memory and a Manifest flooded with `DEVICE_LOST`. Phase 7 reachability is low (no Phase 7 Sink returns `DeviceLost`), but the rule allows any Module to.
- Fix: the data thread never steps an instance again after its step failed, and under an `abort` end it stops stepping altogether (the tail matters only for `orderly`).

P1-11: Several specified tests fail on a correct implementation or are flaky (details under TEST_GAPS).
- `kg_02_every_step_runs_on_the_data_thread` and `kg_02_the_simulation_class_steps_on_the_callers_thread` assert a single thread id, but cleanup's step 3 runs the Sink's `step` inside a `run_cleanup` step thread (`run.rs:614` spawns one per step).
- `kg_02_the_data_thread_drains_a_sink_while_no_call_runs` requires zero drops on a capacity-4 link at 1 ms per block under parallel test load.
- Fix: restrict the thread-id assertions to steps recorded before `finish`; use capacity ≥ 32 and assert on counts read during the sleep.

P2 (one line each):
- P2-1 Overview §7 says `ServerConfig::assembler`; VE-5 says `Config.open_device`. Pick one.
- P2-2 EA-9 reads `EZSDR_PROFILE` from the process environment in the library, so `ea_09` would race other in-process protocol tests (edition 2024 `set_var` is `unsafe` and process-global). Read it in `main.rs` into `Config`.
- P2-3 Fired wake handles are never removed from `Shared.scheduled`, so the vector grows with every wake and cleanup step 1 cancels them all. Prune fired handles.
- P2-4 KG-7 can move T0 by up to one second of root time (L divides the root rate for integer rates). State the bound in KC-15.
- P2-5 The claim that `step_until_quiescent` "would end a healthy Run" holds only at high block rates. At 1 Msps with 2000-sample blocks a round quiesces in 1–2 iterations. One pass is still fine; fix the reason.
- P2-6 KC-29's "no end request is ever left pending across calls" is now false (KG-3 defers cleanup), and `status`/`events` report `Running` with an end pending. Amend KC-29.
- P2-7 MA-20's "emit nothing after `until`" cannot hold for a data-thread step whose Sink stamps events at `now`. Amend it as KG-9 does TM-16c.
- P2-8 KC-21a's early exit on an end or freeze needs the waiter to be notified (`ending::request` and FreezeDispatch notify the condvar) or to poll. Otherwise it waits for the Module's next `recv`.
- P2-9 KG-3's separate stopper thread is not needed. The data thread can perform steps 1–2 inline: the tail is buffered in the link, and UR-12 refuses `Block` links. That is one thread and one join fewer.
- P2-10 The reaction latency of "stops the radios at once" is bounded by the slowest Sink step on the single data thread (for example a capture's read-back hash at completion). Say so.
- P2-11 UR-27 never emits `ClockReference::Time` (no PPS monitor), so the payload value has no producer. Drop it or monitor PPS.
- P2-12 UR-29 never detects a lost device in a transmit-only Run, and `UHD_ERROR_RUNTIME` is generic (false positives). B9 should cover both.
- P2-13 UR-12 should say `N = round(mcr / rate)`, verified, because `200e6 / (200e6/3)` in f64 is not exactly 3.
- P2-14 UR-17's trim can produce a zero-length block (`k + len ≤ e`), which SC-18 says is never published. Say it is dropped.
- P2-15 UR-28's burst record keeps "the host's view", so the `bursts` section counts samples the device dropped. Mark the record, or say so in UR-30.
- P2-16 UR-3's `set_command_time`, setting, `clear_command_time` bracket is per-motherboard state shared by threads (INFERRED). Serialize every device control call under one `UhdDevice` mutex.
- P2-17 Server: a client that dies during a long `wait_for` is noticed only when the wait returns, so the device keeps running until the horizon.
- P2-18 x310-like's 2 ms lead is looser than x310-ubx's 5 ms, so a lead constraint between 2 and 5 ms passes the Mock and fails the UHD Provider. Record it in the Phase 8 input list.
- P2-19 UR-8's UTC relation excludes the host's UTC error, while TM-18 says "with its uncertainty". Name the exclusion in KC-45's text, not only in `method`.
- P2-20 `build.rs`'s rpath through `rustc-link-arg` does not reach dependent binaries such as `ezsdr-server`. It is unnecessary with Homebrew's absolute install names and Linux system paths. Drop it or use `DEP_` metadata.
- P2-21 Remind implementers that OV-23a fails the Kernel on the token "uhd", even in comments (the spike hit it).
- P2-22 bench.md: B9 runs `hw_b1_probe` on the USRP2, whose pass criterion (200 MHz, 2+2 channels) fails by design and which does not exercise UR-5. B7 needs the full Session profile written out, not a delta. B5's 300 ms stall at 10 Msps (12 MB of sc16) may be absorbed by host buffers (INFERRED), so record the buffer sizes and lengthen the stall if no overflow occurs.
- P2-23 The fake-device suite pays ≥2 s per coordinator-driven test (UR-15: T0 ≥ arm + 2 s, a profile value), and `kg_06_*` about 10 s each. State the expected suite time, and size the jitter margins for parallel load.

Decisions T1–T18:
- T1 Supported: design first with Gate P matches the owner's instruction and the concurrency risk.
- T2 Supported: `UHD_SAFE_C` is verified, and all 66 C functions and 4 structs exist in UHD 4.10. A C++ shim or bindgen would add a toolchain or a crate for no gain.
- T3 Supported: `deny` plus a module-level `allow` works (`forbid` would not).
- T4 Supported in shape. FakeDevice's fidelity gaps (zero drift, no command FIFO, instant-keyed transmit, frequency-blind loopback) let P0-2, P0-3, P1-4 and P1-6 pass on the fake.
- T5 Supported: one data thread keeps MA-30's order. The stated reason against `step_until_quiescent` is weak (P2-5).
- T6 Supported: finish counting is the minimal, trait-free fix for K5. It needs P1-2's serialization and P2-8's wake.
- T7 Supported for an idle transmitter (K6). The lead must also cover the in-flight window when a burst preempts another (P0-2).
- T8 Supported: the lattice is anchored at root 0, and Probe 1 shows only the K3 ceiling test moves. TM-13b and TM-13e need amending (P0-4).
- T9 The outcome is supported (radios must stop without a client call). The separate thread is unnecessary (P2-9), and the wake protocol has a lost wakeup (P1-1).
- T10 Supported: a timeout in the Simulation class would be nondeterministic. The deadlock found in draft (`collect_prepare`) is correctly avoided.
- T11 Supported: MA-3 forbids the alternative, and one implementation of RM-8 is what Phase 8 should compare.
- T12 Supported as 0.x and INFERRED. The looser Mock lead is a known §59 break for 2–5 ms constraints (P2-18).
- T13 Supported (SB-22c, spike K4).
- T14 Supported: refusing the child is minimal, and a hand-over has no Phase 7 consumer.
- T15 Supported in substance. Read the variable in `main.rs` into `Config` (P2-2).
- T16 Supported: the default build and CI need no libuhd (§13).
- T17 Supported: TM-18 puts the measurement with the Authority. It needs the MA-6 and MA-46 list amendment (P1-9).
- T18 Supported: a "Native UHD Provider" that never touched a USRP does not meet §59. B0–B8 is the right gate, and the owner decides.

Decisions U1–U12:
- U1 Supported.
- U2 Supported, but the owner of UR-25's device switch is unnamed (P1-7).
- U3 Supported (RFNoC is Phase 11+ in Vision §67).
- U4 Not supported as written: re-anchoring only in wakeups is the free-running case it rejects whenever no client call runs (P1-4).
- U5 Supported.
- U6 Supported in principle (book the clocks before the switch). Switch ownership and RM-25/TM-13e consistency are missing (P1-7, P0-4).
- U7 Not supported: a 10 ms window with a 2 ms device lead breaks UR-23 (P0-2).
- U8 Supported (Review D's UHD source evidence).
- U9 Acceptable as INFERRED. Transmit-only Runs are undetected (P2-12).
- U10 Supported.
- U11 Supported.
- U12 Supported.

TEST_GAPS:
- `ur_23_a_burst_ends_at_the_next_bursts_start` (5 ms after a repeat) passes on FakeDevice's instant-keyed loopback where hardware drops the burst (P0-2).
- `ea_07_a_session_on_the_fake_device`, `uhd_57_the_session_loopback_captures_what_it_transmits` and `ur_25_tx_channels_from_zero_admits_the_next_burst` pass with a TX frontend never configured, because the fake ignores frequency (P0-3).
- `kg_02_every_step_runs_on_the_data_thread` and `kg_02_the_simulation_class_steps_on_the_callers_thread` fail on a correct implementation: the orderly step-3 (and Simulation drain) steps run on `run_cleanup`'s spawned step threads (P1-11).
- `kg_02_the_data_thread_drains_a_sink_while_no_call_runs`: a capacity-4 `drop_oldest` link at 1 ms per block is flaky under parallel `cargo test` load (P1-11).
- G07 ("the pass uses `step_until_quiescent`") is not killed by `ur_22_repeat_is_continuous_across_the_wrap`. At the test's rate a round quiesces after 1–2 iterations, so the cap is never reached.
- G09 ("drain before taking the `delivered` lock") cannot be killed by `kg_02_event_indices_hold_across_both_threads`. `delivered` is append-only, so every read is a prefix with or without the lock. What the lock protects is reaction order versus delivered order (which termination cause wins), and the test does not assert it.
- G12 ("step 3 skips the final round") is not deterministically killed by `kg_03_an_orderly_stop_still_delivers_the_tail`: the still-running data thread consumes the tail before step 3.
- G18 ("`None` completes a device-paced Spec Run") is an equivalent mutant. Every device-paced loop has a horizon scheduled, so `next_wakeup` never returns `None` there, and the named test passes with the mutation.
- G16 is killed only by `mutate.py`'s 900 s timeout. `cargo test` has no timeout, so name that as the mechanism.
- U13 needs the fake's `apply` to take measurable time; with an instant `apply`, `ur_14` passes most of the time under the mutation.
- U19: the fake accepts up to 50 ms ahead, so `ur_23_a_stop_ends_the_burst_within_the_in_flight_window` kills "no in-flight bound" only if its margin is < 40 ms. The stated default margin is 50 ms.
- U22 ("clock registered at the switch") is not killed by `ur_25_tx_channels_from_zero_admits_the_next_burst`. For 0 → 1 channels the switch and receipt coincide; it needs a transmit rate change followed at once by a burst on the new clock.
- U16 ("a repetition sent with start-of-burst") depends on unspecified FakeDevice semantics for a start-of-burst without a time spec. Specify them (a new start-of-burst restarts the CORDIC and the fake reports a gap).
- No test for: a wake cancelled by step 1 (P1-1); dispatch after freeze (P1-2); Detached Lease expiry with no calls (P1-3); drift while idle, since the fake has no drift (P1-4); a far-future timed update delaying later commands, since the fake has no FIFO (P1-6); a runtime rate read-back mismatch at a cold switch (P1-7); the `Hardware` (over-the-air) class admitted by KC-2 (only HardwareInLoop is exercised, so "KC-2 refuses Hardware" survives).
- `ea_09_ezsdr_profile_names_the_default` mutates the process environment inside an in-process test binary, which races the other `connect {}` tests (P2-2).
- VE-6's device-paced behaviour has no automated carrier (Python runs only on Simulation; B7 is manual), so P1-5 would reach the bench unseen.

PREVIOUS_FINDINGS: none (first review)
