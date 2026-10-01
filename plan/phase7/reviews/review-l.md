# Review L — the Phase 7 implementation (`git diff 25e79f3` on `worktree-phase7-impl`, HEAD 89f02b7)

Reviewer: Claude Opus, 2026-09-28, read-only on the repository; every build and probe in scratch copies under `/Users/komatsu/.claude/jobs/20ef8e0f/tmp/review-l/` (`copy/`, `copy-g36/`, `mut-list/`, `mut-own/`), fresh timestamps, one copy at a time, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` (`…-review-185` for 1.85.0). Brief: [`../prompts/review-l.txt`](../prompts/review-l.txt). The report as returned; the triage is in [`../design-notes.md`](../design-notes.md) §7.

## VERDICT

**CHANGES_REQUIRED** — 3 P0, 8 P1, 19 P2.

The Kernel half (KG-1…KG-14) holds up under attack: no deadlock, lost wake, double or out-of-order delivery, or termination/`delivered` disagreement found; Simulation Runs start no thread and change no behaviour. The UHD half has three defects that would fail or crash on the bench: a use-after-free/double free of every UHD streamer, a rate rule that refuses 487 of the profile's 512 advertised rates, and a from-zero receive enable that ignores its effective instant. Two of the §6.2 survivors differ: G36 is killable deterministically without a Kernel hook (probe below); G09 is not, and does not warrant one.

## BLOCKERS

### P0-1 — `UhdDevice::rx_open`/`tx_open` free the streamer they store (use-after-free, then double free)
- **Location:** `crates/ezsdr-radio-uhd/src/uhd.rs:552-557` and `:640-645`.
- **Evidence:** `let stream = RxStream { h, md: std::ptr::null_mut(), channels }; … *lock(&self.rx) = Some(Arc::new(RxStream { md, ..stream }));`. `RxStream` implements `Drop`; all its fields are `Copy`, so `..stream` copies them and `stream` stays alive, to be dropped at the end of the `unsafe` block — calling `uhd_rx_streamer_free(&mut h)` on the handle the `Arc` now holds. Every later `rx_recv`/`stream_cmd`/`tx_send` uses a freed streamer, and `close_streams` or a reopen frees it a second time. Probe (`probe-fru/main.rs`, the same pattern with a printing `Drop`) printed:
  ```
  end of rx_open's block
  drop: uhd_rx_metadata_free(0x0); uhd_rx_streamer_free(0x1000)
  after rx_open: drops so far = 1 (the Arc still holds h=0x1000)
  drop: uhd_rx_metadata_free(0x2000); uhd_rx_streamer_free(0x1000)
  ```
  Clippy with `--features uhd` is clean; no test reaches `rx_open` without a device; bench B3 is the first place it would show (crash or garbage samples).
- **Fix:** in both functions, `let mut stream = …; stream.md = md; *lock(&self.rx) = Some(Arc::new(stream));` (move, don't copy). Add a no-device unit test with a counting `Drop` or a `uhd_api` test that the handle is freed exactly once, to pin it.

### P0-2 — UR-9 advertises 512 rates and UR-12 refuses 487 of them; MockRadio accepts all 512
- **Location:** `crates/ezsdr-radio-uhd/src/profile.rs:34` (`Grid::Values((1..=512)… 200e6 / n)`); `src/device.rs:205` (`exact_decimation`), called from `provider/core.rs:302` (prepare and every configure) and `provider/control.rs:274` (cold booking); spec 18 UR-12 ("`200 000 000 / N == rate` verified in exact arithmetic").
- **Evidence:** probe `probe-decim` running the shipped `exact_decimation` over the profile's grid printed `advertised: 512, refused by exact_decimation: 487`, `first refused N: [3, 6, 7, 9, 11, 12, …]`, `N=3 (66.666... Msps): None`. `coerce` (RM-26) admits these rates, and the capability `AnyOf` lists them; `prepare` then fails with a UR-12 message. MockRadio's `Profile::decimation` (`crates/ezsdr-mock-radio/src/profile.rs:40-49`) accepts the same rates by IEEE comparison (`200_000_000.0 / n != rate`). So a Spec at 200e6/3 S/s passes on MockRadio and is refused on the X310 by changing the profile — the Vision §59 contract broken, a contradiction inside spec 18 (UR-9 vs UR-12), and a published capability looser than the Provider (§13). UR-12's reading of K13 is the error: K13's hazard is computing `N` by float division (fixed by `round`); `N` is an exact integer ratio whatever the `f64` rate's exact value.
- **Fix:** amend UR-12: `N = round(mcr / rate)` in 1…512, and the read-back rate must equal `mcr as f64 / N as f64` exactly (MockRadio's rule); replace `exact_decimation` accordingly at both call sites; add a test at 200e6/3 that passes `prepare` and a cold switch.

### P0-3 — A receive stream enabled from 0 channels starts at `now + 50 ms` whatever its effective instant (RM-25 vs UR-25)
- **Location:** `provider/control.rs:370` (`Dir::Rx => lattice(now + self.core.ticks(RESTART_LEAD_NS), n),`); spec 18 UR-25 ("for receive at the first lattice instant at or after `now + 50 ms`"); `design/07-radio-model.md:211` RM-25.
- **Evidence:** RM-25: "A change from 0 channels … the new clock starts at the first lattice instant at or after both `e` and the instant the Provider finished configuring the stream". The receive branch never reads `e`, so a Session `radio.rx.channels = 1` with `at` = +1 s starts receiving ~50 ms after receipt. The transmit branch does (`lattice(e.max(now), n)`). My mutation L08 (drop the 50 ms too) survives the whole `fake` suite: no test exercises a from-zero receive enable at all.
- **Fix:** UR-25 "at or after the later of `e` and `now + 50 ms`" and code `lattice(e.max(now + restart), n)`; add `ur_25_rx_channels_from_zero_starts_at_its_instant` (a Session with `rx.channels` 0 and a capture link; `rx.channels = 1` `at` +300 ms; the new receive clock's origin ≥ +300 ms and the first block at index 0 with no gap).

### P1-1 — Timed receive stream commands bypass UR-24's release discipline (design-notes §6.1 row) and reopen Review J P1-6
- **Location:** `provider/rx.rs:147-155` (on `RxCmd::Switch`, uhd-rx calls `self.core.device.rx_stop(Some(e1))` as soon as the switch is booked).
- **Evidence:** UR-24: "The timed stream commands of UR-25 (a stop at `e₁`, a start at `e₂`) … go through the same list, released their own lead ahead". A cold receive change booked `at` +1 s therefore hands the device a command 1 s ahead, and every later-released retune queues behind it in UHD's in-order, back-pressuring queue ("A timed command will back-pressure all subsequent timed commands", spec 18 §2) — exactly what UR-24 exists to prevent. It is not recorded in `applied` either (UR-30). The ponytail note makes the ordering INFERRED, but the timing half (issuing a whole cold lead early) needs no bench.
- **Fix:** uhd-rx already loops every `recv`: issue the timed stop only once `e1 − now ≤ RESTART_LEAD_NS` (the lead UR-24 names) and record it in `applied`; or route it through uhd-control's list as specified. Amend the design-notes row either way.

### P1-2 — UHD error texts come from UHD's process-global error string, which other threads overwrite
- **Location:** `src/uhd.rs:199` (`last_error()` → `uhd_get_last_error`), used by `check()` for every call.
- **Evidence (VERIFIED):** `/opt/homebrew/include/uhd/error.h:89-106`: `UHD_SAFE_C` calls `set_c_global_error_string(...)` on failure and `set_c_global_error_string("None")` on every successful call. uhd-rx calls `rx_recv` continuously, so the text a failing uhd-control/uhd-tx call reads back is often "None" or another thread's error — making the `DEVICE_LOST` messages (MA-9a) and `rejected` reasons unreliable, which misleads bench B9's diagnosis.
- **Fix:** read the per-handle text: `uhd_usrp_last_error` for `usrp` calls (inside the control mutex) and `uhd_rx_streamer_last_error`/`uhd_tx_streamer_last_error` (declared at `uhd.rs`, unused) for streamer calls; add `uhd_usrp_last_error` to UR-3's function list.

### P1-3 — The receive side of a cold change lacks UR-25's fallback and reopens in the wrong order
- **Location:** `provider/rx.rs:147-155`, `:357-372`; `src/uhd.rs` `rx_open`.
- **Evidence:** UR-25: "INFERRED: the X3x0 honours a timed stop of a continuous stream; otherwise uhd-rx stops it untimed when its samples reach `e₁`" — no untimed `rx_stop(None)` is ever issued when the cut is reached, so if the device ignores the timed stop the old stream keeps flowing into the reopen. `do_switch` calls `rx_open` unconditionally, although UR-25 says "reopens its streamer when the channel count changed". And `rx_open` creates the new UHD streamer before the old `Arc` is released (`*lock(&self.rx) = Some(...)` after `uhd_usrp_get_rx_stream`); INFERRED risk: UHD 4's RFNoC graph refusing a second streamer on connected channels. The fake models none of this (`FakeDevice::rx_open` just resets `rx_next`).
- **Fix:** issue `rx_stop(None)` when the first sample at or after `e1` arrives; reopen only on a count change; in `rx_open`/`tx_open`, `take()` the old streamer before `uhd_*_streamer_make`.

### P1-4 — A preempted burst can end without end-of-burst (UR-23), and the fake cannot tell
- **Location:** `provider/tx.rs:361-364`: `self.end_open(true); } else { self.end_open(false); }` — the `h == next` branch sends no end-of-burst.
- **Evidence:** UR-23: "the current one is sent up to the sample before the next start, its last buffer with end-of-burst". With `h == next` the open burst's last buffer went out without EOB (the held burst arrived after it), `end_open(false)` sends none, and the next burst starts with SOB and a time spec inside an un-ended device burst (INFERRED behaviour on the X310); the old burst never gets `BurstAck`, so `unacked` (`tx.rs` `report`) is off by one and a later `TimeError` is attributed to the wrong target (UR-28). My mutation L10 (no EOB on the `h < next` branch either) survives the whole `fake` suite: the fake accepts a timed SOB inside a burst.
- **Fix:** always `end_open(true)`; make the fake record a timed SOB inside an un-ended burst (as it records restarts) and assert on it in `ur_23_a_burst_ends_at_the_next_bursts_start` and `ur_21_a_burst_inside_the_in_flight_window_is_late`.

### P1-5 — KC-21a's "every Action dispatched to it" is not guarded
- **Location:** `coordinator/paced.rs:205-209` (`wait_finished`).
- **Evidence:** my mutation L01 (`*target = if *target == 0 { *pushed } else { (*target).min(*pushed) }` — wait only for the first Action dispatched to an instance) survives every `kg_` test in `device_paced`. No test dispatches two Actions to one threaded instance in one call (the start batch and agenda items at one instant do).
- **Fix:** add `kg_04_every_action_of_one_call_is_finished`: a Spec whose schedule places two `SetParameter`s to the `ThreadedProvider` `.with_action_delay(30 ms)` at one instant (or two entries at T0); assert both "finished" records precede the call's return.

### P1-6 — G36 is killable deterministically, without a Kernel hook
- **Location:** `coordinator/paced.rs:180-199` (KC-46a); design-notes §6.2.
- **Evidence:** KC-46a names the interleaving itself: "a callback that fires before `schedule` has even returned still clears its own wake". A paced Authority may legally block in `schedule` (TM-16c requires only that the callback fires at a `next_wakeup`, not that `schedule` returns first). Probe (`copy-g36/`): I gave `WallTime` a `.firing_before_schedule_returns()` mode — `schedule` of an instant already passed waits (≤ 20 ms) until its callback has run on the waiting `next_wakeup` — and copied `kg_02_every_delivery_wakes_a_waiting_call` onto that Authority (20 marks, 200 ms bound). Unmutated: `ok. 1 passed … finished in 0.22s`, 5/5. With G36's mutation from `mutations.json`: `FAILED … finished in 2.02s` (the `wait_for` ran to its 2 s horizon: `panicked at …device_paced.rs:1208:9: 2.0101295s`), 4/4.
- **Fix:** add the double's mode and the test (`kg_02_a_wake_that_fires_before_schedule_returns_still_clears`); record G36 as killed by it.

### P1-7 — Many UHD-Module obligations have no killing test (my mutations; details in TEST_GAPS)
- **Location:** UR-24, UR-25, UR-26, UR-29, UR-7, EA-7.
- **Evidence:** surviving the whole `fake` (or `protocol` for L13) suite: L07 (UR-24 "a command whose `e` precedes a command already released is applied late"), L08 (from-zero receive enable), L09 (UR-25's configuration "in effect at `e₂`", Review K N-P2-9), L11 (`COMMAND_REJECTED "cancelled by a cold change"`), L12 (abort discard: `ur_26_orderly_stop_delivers_the_tail_abort_does_not`'s abort half passes because KC-46b stops the data thread from stepping the Sink, not because uhd-rx discards anything), L14 (UR-29's 1 s silence bound: `ur_29_a_silent_stream_is_a_lost_device` does not bound the delay), L05 (UR-7's 1 ms bracket rule), L13 (the server passing the binding's `time_source` to `DeviceAuthority`).
- **Fix:** one targeted test each — L07: via the `Direct` harness, a retune at +10 ms then one at +8 ms booked after the first was released; expect `LATE_COMMAND` and the fake's effective instant = the first's. L09: a held retune due before `e₂` appears in the switch's `apply` record. L12: after an abort, no block with `first_sample_time` later than the Provider's stop instant is published (count via a second, never-stepped link, or `stats.rx_blocks` vs the stop's timing row). L14: `DEVICE_LOST` delivered between 1.0 and 2.0 s after the silence starts. L05: a `FakeFault` that delays `time_now` by 5 ms, with the anchor unchanged. L13: a server test with `time_source: external` asserting `set_time_zero pps` in the fake's calls.

### P1-8 — `rehearsal_b7_session_loopback` is flaky
- **Location:** `crates/ezsdr-radio-uhd/tests/common/mod.rs:475` (`assert!(events_of(&manifest, "radio.TIME_ERROR").is_empty(), "no TIME_ERROR (spike K6)")`).
- **Evidence:** running the full `fake` binary 12 times: 11 passed, 1 failed (`rehearsal_b7_session_loopback --- FAILED … common/mod.rs:475:9`); alone it passed 20/20. The untimed `start_repeat` right after `tx.channels = 1` relies on a 3 ms delivery allowance and a 2 ms device lead measured on the wall clock; with 78 wall-clock tests in parallel, each with busy-polling Provider threads, that margin is sometimes exceeded. A red `cargo test --workspace` on a correct tree (or a habit of rerunning) would hide a real K6 regression.
- **Fix:** keep the strict check for the hardware run; on the fake, run the heavy wall-clock rehearsals serially (one `#[test]` that runs them in sequence, or a shared `Mutex`), or assert only on `late_at_device` / on `send_asap` moves > one device lead.

## NONBLOCKING

1. `paced.rs:191` stores the wake handle after `schedule` returns, so a concurrent `FreezeDispatch` can miss it; the leftover is a harmless no-op, but RS-6 step 1's "every callback cancelled" is not literally true. Store under a lock taken by both.
2. `paced.rs:116`: the pass checks `done` before locking the slot; if RS-8a abandons step 3 (data thread still running), step 5 can clean up a Sink the pass then steps. Re-check `done` under the slot lock.
3. L03 (KC-46b's wake made conditional) is an equivalent mutant: `FreezeDispatch` cancels the horizon and the cancel already wakes `next_wakeup`; say so in KC-46b or drop the unconditional wake.
4. `uhd.rs:522`: `uhd_usrp_clear_command_time`'s result is ignored (a failure leaves every later "untimed" call timed); `uhd.rs:408` `channels()` turns an error into 0; `max_num_samps` and metadata-getter results ignored — against UR-3's "every call's `uhd_error`".
5. `uhd.rs` `rx_recv` maps `BROKEN_CHAIN` to `lost: true`, which UR-29 does not list.
6. `control.rs` UR-24's depth counts one per update, while release issues one device command per channel (two with two channels) against the fake's 16-deep queue.
7. `authority.rs` `root_tick` for `host.monotonic` floors instead of returning TM-16c's `Inexact`.
8. `control.rs` `enable` (transmit) with an `at` already passed starts at `now` without `LATE_COMMAND`, unlike the old-clock branch.
9. `mod.rs:442-447`: uhd-rx's stop instant is taken after uhd-tx's ≥ 100 ms `BURST_ACK` wait, so the orderly tail ends ~100 ms after `Provider::stop`'s recorded `at`, and an abort publishes ~100 ms more blocks (unconsumed). State it in UR-26 or pass the cut instant from `stop()`.
10. `core.rs:302` message "the device applied X S/s for the claimed X, which is no exact division" is confusing (moot after P0-2).
11. `mod.rs:461` `cleanup` closes both streamers or neither, including when only uhd-control is detached — conservative; UR-16 says per streamer.
12. `FakeDevice::tx_send` never returns a partial count (UHD does on timeout), so `tx.rs:231`'s partial-send path is reached only through `TxBlocks`.
13. The U07 row mutates the order of `set_sources` / `set_time_zero`; the spec row's claim (time source set before clock source, inside `uhd.rs::set_sources`) stays unguarded until the bench.
14. `tm_16c_ties_fire_in_insertion_order` (spec 19 KG-9, "existing, kept") does not exist; the tie test is `tm_16_authority_order_and_now` (`time_model.rs:784`). Fix the spec's name.
15. `design/14-native-executor.md:85` NX-4's reason ("which only the Simulation class does (MA-30's table)") is now false: MA-30's device-paced row steps Executors on the data thread.
16. `ur_16_a_wedged_thread_does_not_wedge_stop` asserts `< 6 s`, where UR-16/§6 say 3 s.
17. `crates/ezsdr-mock-radio/src/lib.rs:6` has a blank line left where `mod coerce;` was.
18. `uhd.rs`: `uhd_rx_streamer_last_error` / `uhd_tx_streamer_last_error` declared and unused (they are P1-2's fix).
19. L06 (`next_wakeup` naps the whole wait instead of 20 ms naps) survives: near-equivalent while uhd-clock re-anchors every 100 ms (residual drift error ≤ 100 ppm × the wait). UR-7's per-nap re-anchor is untested; say so or test it with `drift_ppm`.

## DEVIATIONS

Design-notes §6.1, one line each:

1. KG-8 as a `//` comment — acceptable (GZ-2/`schema_freeze`); needs a spec change (KG-8's "Code" line should say where the sentence lives).
2. Test locations (`device_paced.rs`, `radio/tests/device.rs`) — acceptable; the names match.
3. `mr_32` burst index 2 instead of 3 — acceptable: the direct consequence of RM-25's transmit origin moving onto its lattice; add a note to VE-4.
4. PO-8 dev-dependency allow-list widened (`ezsdr-sink`, `ezsdr-sink-capture`, `ezsdr-link-host` for `ezsdr-radio-uhd`) — acceptable (workspace crates only, enforced by `governance.rs`); needs a spec change (a GZ/PO-8 amendment in the overview).
5. UR-24 timed stream commands issued by uhd-rx — needs a code change for the receive stop, issued a whole cold lead early (P1-1); the start at `e₂` and the missed-start restart (~50 ms ahead) are acceptable with a UR-24 wording change.
6. UR-3 brackets around `Device::time_now` (mutex wait included) — acceptable (UR-7's 1 ms discard rule covers it, untested: L05); needs a spec change to UR-3's sentence.
7. UR-7 failed reads not in the Manifest — needs a spec change (the Authority has no section), or the Provider could record `DeviceAuthority::failed_reads()` in its `timing` row (it does not hold the Authority).
8. UR-32 `rx_recv` allocates per call — needs a spec change (UR-32 is a producer obligation the code does not meet), deferred to Phase 8 by name.
9. UR-21 "at or before the open burst's next sample" decided on uhd-tx — acceptable, consistent with N-P1-2; needs a spec change to UR-21's refusal list.
10. UR-34 `with_rx_stall` counted from the first block — acceptable; needs a spec change (UR-34's wording).
11. The `Direct` harness — acceptable; the only way to reach the Provider's own lateness and envelope checks past KC-24.
12. `empty_assembly` with a Simulation Engine — acceptable (`Assembly.authority` is not optional; the Kernel refuses the child first).
13. `assemble` takes `open_device`; `Config::new` — acceptable; needs a spec change (EA-7's signature).
14. Fake tests with a 2 s start lead — acceptable (UR-15); it costs wall time, and is part of the cause of P1-8's load.
15. Spec 19's text as applied to `design/` — RM-1's "three payload types and one outcome value" is right (spec 19 VE-1 miscounts "four"); KC-20/KC-45 consistent; RM-26's prefix fine; RM-11's source rule for the new kinds is stated in spec 18 UR-27/UR-28 and acceptable; design/14 NX-4's reason is stale and needs a spec change (NONBLOCKING 15).

## TEST_GAPS

**Rows of the list re-run** (`mutate.py` on `plan/phase7/tools/mutations.json`, scratch `mut-list/`): G29 killed, U05 killed, U13 killed, U34 killed, U41 killed, U44 killed (as design-notes claims, deterministic here though the spec expected probable); G09 SURVIVED, G36 SURVIVED.

**Judgement of the two survivors (design-notes §6.2):**
- **G36** — a better test exists without Kernel code: a paced Authority double whose `schedule` returns only after its callback ran kills it deterministically (P1-6: 4/4 killed, 5/5 unmutated pass).
- **G09** — between `drain` and the append only Kernel code runs (the collector, the Policy, `ending::request`); no Module or double can widen the window, so a black-box test cannot win the race. A `testing`-only barrier in `drain_and_react` would be Kernel code for one test; I do not recommend it. Record G09 as "not killable by a black-box test; guarded by review", and this review confirms `stepping.rs:235`: the `delivered` lock is held across `collector.drain()`, the reactions and the append.
- The rest of the list's claim (91 of 93 killed) I did not re-run.

**Rows of the list whose mutation differs from its spec row:** G11/G12/G29 file names differ from the table (acceptable); G36 reads as the spec intends; U07 guards a different claim (NONBLOCKING 13); U18/U22 acceptable as documented; G33 and U33 are the same mutation (duplicated ids).

**My own mutations** (`own-mutations.json`, scratch `mut-own/`, each against the whole relevant test binary — `device_paced` `kg_`, `fake` `ur_`, `protocol`):

| id | mutation | result |
|---|---|---|
| L01 | KC-21a waits for the first Action to an instance, not the last (`paced.rs`) | SURVIVED (P1-5) |
| L02 | data thread parks 20 ms instead of 200 µs (`paced.rs`) | killed |
| L03 | KC-46b's wake made conditional (`paced.rs`) | SURVIVED — equivalent (NONBLOCKING 3) |
| L04 | a Module's submission takes no admission lock (`pipeline.rs` `Submitter`) | SURVIVED — KC-24a's Module-submission clause has no test (no Phase 7 Module submits; add one with a Sink double that submits from `step` while a control call admits) |
| L05 | UR-7's 1 ms bracket rule disabled (`authority.rs`) | SURVIVED |
| L06 | `next_wakeup` naps the whole wait (`authority.rs`) | SURVIVED — near-equivalent |
| L07 | UR-24: a command before one already released not moved behind it (`control.rs`) | SURVIVED |
| L08 | receive enable from 0 without the restart lead (`control.rs`) | SURVIVED (no test of that path at all; P0-3) |
| L09 | switch configuration ignores held commands due by `e₂` (`control.rs`) | SURVIVED |
| L10 | preempted burst gets no end-of-burst (`tx.rs`) | SURVIVED (P1-4) |
| L11 | cold transmit switch emits no `COMMAND_REJECTED` for cancelled bursts (`tx.rs`) | SURVIVED |
| L12 | abort stop of the receive side delivers the orderly tail (`rx.rs`) | SURVIVED |
| L13 | server's UHD Authority ignores `time_source` (`catalogue.rs`) | SURVIVED |
| L14 | silent-stream bound 3 s instead of 1 s (`rx.rs`) | SURVIVED |

**Other gaps:** `kg_02_the_simulation_class_starts_no_thread` sees only the Sink's step threads (G21's test covers `ezsdr-bounded` for `prepare`; together adequate); `ur_26`'s abort half proves KC-46b, not UR-26 (L12). **Flaky:** `rehearsal_b7_session_loopback` 1 in 12 full-binary runs (P1-8); `device_paced` 5/5 runs 39 passed each (including the 200 ms wake bounds and the 50-Session termination test); `protocol` 5/5 runs 31 passed; `ur_07_now_tracks_the_device_while_no_call_runs` (100 µs) no failure in 12 runs.

## RUNS

In the scratch copy `copy/`, fresh timestamps, `touch` before each build session:

| command | result |
|---|---|
| `cargo +stable test --workspace` | 824 passed, 0 failed, 0 ignored (exit 0) |
| `cargo +1.85.0 test --workspace` (`…-review-185`) | 824 passed, 0 failed, 0 ignored (exit 0) |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean (exit 0) |
| `cargo +stable test -p ezsdr-radio-uhd --features uhd` | 80 passed, 0 failed, 10 ignored (the `hw_*`); `uhd_api` 3 passed |
| `cargo +stable clippy -p ezsdr-radio-uhd -p ezsdr-server --features ezsdr-radio-uhd/uhd,ezsdr-server/uhd --all-targets -- -D warnings` | clean (exit 0) |
| Python suite, `~/.cache/ezsdr-venv/bin/python` (3.13), `EZSDR_SERVER` from the copy, `PYTHONDONTWRITEBYTECODE=1` | Ran 23, OK |
| Python suite, `/usr/bin/python3` (3.9) | Ran 23, OK |
| `python3 plan/phase3/tools/check_links.py` | `ok: 504 links` |

Repeated: `device_paced` ×5 more (39/39 each); `fake` (78) ×10 more (9 × 78/78, 1 × 77/78 `rehearsal_b7_session_loopback`; with the first two, 11/12 green); `rehearsal_b7` alone ×20 (20/20); `protocol` ×5 (31/31). Mutations: list 6 killed (G29, U05, U13, U34, U41, U44), 2 survived (G09, G36); own 14: 1 killed, 13 survived (table above); G36 probe: new test kills 4/4, passes unmutated 5/5. Probes: `probe-fru/` (the `..stream` pattern: handle freed at the end of `rx_open` and again at `close_streams`); `probe-decim/` (`exact_decimation` over the profile's grid: 487/512 refused). `Cargo.lock` gains only workspace packages (`ezsdr-radio-uhd`, server 0.2.0) — PO-4 holds; no Kernel schema changed (the `schemas/` diff is `radio/*` (11 files), `server/reply_frame`, `SCHEMA_CHANGELOG.md`, with entries); `design/vision/` and the Vision index unchanged; `ma_03`, `po_02` (GZ-3), `po_11` (GZ-4), `gz_08`, `kernel_surface` pass.
