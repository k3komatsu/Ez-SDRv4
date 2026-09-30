# Review N — the fix of design-notes §11 F1–F3 (`git diff ca5aebb bc0db98` on `worktree-phase7-impl`)

Reviewer: Claude Opus, 2026-09-30. I made no changes to the repository. Every build ran in `ezsdr-v4-dev:uhd4.10` against `CARGO_TARGET_DIR=/cargo-target/review`, on a scratch copy at `/tmp/claude-1000/-home-komatsu-works-Ez-SDRv4/d82454b2-b1d7-4f3c-a624-72d8e684572b/scratchpad/review-n/copy`. I made it with `rsync -a --no-times`, touched it before each build session, and never interleaved builds. The probe test file is kept beside it as `fake.rs.probe`. I did not use the USRP.

Brief: [`../prompts/review-n.txt`](../prompts/review-n.txt). The report as returned.

## VERDICT

**Not ready for Gate X as committed. Three P1 findings need fixing first.**

On the path the bench measured (X300 + OBX, 1 Msps, `block_len` 2 000, one `cold` receive change, one preemption at 20 ms), the fix is correct.

- **F1:** a timed receive stop no longer reaches the device, and the old clock's samples reach `e₁`.
- **F2:** the switch runs about one block after `e₁`.
- **F3:** a burst held at, or preempting at, the open burst's next sample continues the device burst. The Kernel's two `BurstRecord`s and its `START_OF_BURST`/`END_OF_BURST` blocks are unchanged (RM-15, RM-16).
- **Fake suite:** 102/102 on stable (cargo 1.98.1) and on 1.85.0; 101/101 with `--features uhd`.
- **Mutations:** all nine changed or added rows are killed in the scratch copy (R06, R12, R13, R14, U17, B01–B04), each by the test the JSON names; every baseline passes.
- **FPGA citations:** the ones §11 F1 gives are right (`radio_rx_core.v:171`, `:526`, and `:570`, where `ST_RUNNING` stops on `cmd_stop` whatever the time).

The change is still not safe outside that measured point:

- **B1.** At a `block_len` the selector allows and a low rate, the untimed stop comes after `e₂`. The old stream's in-flight tail is then published on the new clock.
- **B2 (INFERRED).** At the lowest rates, the new short receive wait lets a `Timeout` end the old stream before its samples up to `e₁` have arrived.
- **B3.** F3's gap-0 drop is still reachable when the next burst is admitted after the open burst's last buffer, carrying end-of-burst, was already handed over. This reproduces on the committed fake.

The fake's three new strict behaviours are not pinned by any test. None of the three blockers needs a Kernel, Vocabulary or profile change.

## BLOCKERS

### B1 (P1) — above a certain `block_len` at low rates, the untimed stop lands after `e₂` and old-clock samples are published on the new clock

**Location.**
- `rx.rs:149`: `rx_recv(self.core.block_len, …)` always asks for a whole block.
- `rx.rs:316`: the stop is issued only when a whole block reaching the cut has arrived.
- `rx.rs:302`: `samples` drops a block only when `first_tick < clock.origin`.
- `control.rs:323`: `e₂ = lattice(e₁ + 50 ms)`, with no term for the block.
- `mod.rs` UR-5: `block_len` may be anything in 1…65 536.

**Mechanism.**
1. The stop is issued up to one block plus the host's delivery latency after `e₁`.
2. A block at the lowest rate, 200 MHz / 512 = 390 625 S/s, is up to 65 536 / 390 625 = 167.8 ms long. Once one block plus the latency plus the switch (~0.3 ms on the bench) exceeds the 50 ms restart lead (above about 19 000 samples at 390.6 kS/s, or about 47 000 at 1 Msps), the switch runs after `e₂`.
3. The timed start is then late: `LATE_COMMAND` and a UR-17 restart gap.
4. The old stream's samples still in flight after the untimed stop were produced between the block's end and the stop. Their ticks are now ≥ `e₂` = `new.origin`, so the `rx_before_origin` guard (Review M, N-3) no longer catches them.
5. With the channel count unchanged the streamer is not reopened, and those old-rate samples are published as the new clock's.

Before the fix this could not happen: on an X300 the early stop ended the stream ~50 ms before `e₁`, and on a device that honours a timed stop the stream would have ended at `e₁`.

The bench shows that such a tail exists after an untimed stop. At `bc0db98`, `hw_b8_cold_change_capture` has `stats.rx_before_origin: 1`, and `hw_b8_raw_rx_timed_stop` says "after the stop: 2 blocks".

**Evidence (scratch copy).** The probe used `Direct::build` with `radio.rx.sample_rate_hz` 390 625, selector `block_len` 65 536, and a `cold` change to 2 Msps at now + 300 ms + j·40 ms.
- **On the committed fake:** 2 of 4 changes switched after `e₂`, for example `rx_switch` at 488 010 352 against `e₂` 478 583 500. Each emitted `LATE_COMMAND` and restarted (`rx_start 498010600`).
- **On a probe fake** with 1.5 ms delivery latency, 1 996-sample packets, UHD's per-packet timeout and partial return (`rx_streamer_impl.hpp:139–203`), and the device's in-flight tail kept after an untimed stop:
  - 3 of 4 changes published an old-rate block on the new clock, for example block k = 224 183, len 2 551, first sample 0.92995 where the new clock's ramp gives 0.44133.
  - `rx_off_lattice` was 1 and `rx_before_origin` was 0.
  - At the same `block_len`, the one change whose block ended 38 ms after `e₁` was clean (tail dropped, `rx_before_origin` 1).

**Fix (one recommendation).**
1. While a cut is pending, ask `rx_recv` only for the samples up to the cut: `n = clamp(cut_k − expected, 1, block_len)`. The last block then ends at `e₁`, and the stop is issued about one delivery latency after `e₁` whatever the `block_len`.
2. As a defence, record the root tick at which `stop_at_cut` issued the untimed stop. In `samples`, drop and count (`rx_before_origin`) any block whose first tick precedes it, as well as blocks before the origin. On the X300 an untimed stop ends the stream at its issue (bench: −0.01…−0.04 ms), and a late start never streams (`radio_rx_core.v`, `ST_TIME_CHECK` → `ST_REPORT_ERR`), so nothing new precedes that instant.
3. Add a test on a fake that keeps an in-flight tail (see TEST_GAPS T1), at `block_len` 65 536 and 390 625 S/s.

Rejected alternatives:
- Adding the block duration to `e₂`: it moves every `cold` change for a block-size choice, and still leaves the tail unguarded.
- Capping `block_len` against the lowest rate: this narrows a selector that UR-5 defines.

### B2 (P1, INFERRED on hardware) — at the lowest rates a short receive timeout can end the old stream before its samples up to `e₁` have arrived

**Location.** `rx.rs:210–214` (`recv_timeout`: `cut − now + block`, floored at 1 ms) and `rx.rs:243–246` (a `Timeout` with `now ≥ cut` ends the stream and stops it).

**Mechanism.** Before the fix, a `Timeout` meant 100 ms of silence, so `now ≥ cut` implied the stream had stopped. Now the wait shrinks to 1 ms near the cut. UHD's timeout applies per packet. Once the time left before the cut is less than the host's delivery latency, a wait of 1 ms can see no packet at all. That happens when:
- the packet period is well above 1 ms (1 996 samples are 5.1 ms at 390.6 kS/s; this packet size is INFERRED), and
- the delivery latency is a few ms.

The empty `Timeout` then arrives with `now ≥ cut`. The stream is stopped, and the samples between the last delivered block and `e₁` are never delivered. There is no gap, no flag and no event — F1's defect class, smaller.

**Evidence (probe fake as in B1, `block_len` 2 000, `cold` change from 390 625 S/s).**

| Delivery latency | Changes that lost samples before `e₁` | Samples lost |
|---|---|---|
| 0.5 ms | 0 of 10 | — |
| 1.5 ms | 0 of 10 | — |
| 2.5 ms | 2 of 10 | 276–326 |
| 3.5 ms | 4 of 10 | 271–774 (0.7–2 ms) |

At 1 Msps, no change lost samples at 2.0 or 3.5 ms.

The bench's orderly stops landed 2.0–3.9 ms after their cut at 1 Msps with 2 ms blocks, which puts the delivery latency at up to ~2–3.9 ms. That figure is INFERRED: the bench did not measure 390 kS/s.

**Fix.**
- End on a `Timeout` only once `now ≥ cut + block duration + DELIVERY_ALLOWANCE_NS` (3 ms, already in `profile.rs`).
- Bound the wait by that same instant rather than by `cut + block`.
- Together with B1's `n` bounded by the cut, this costs at most a few ms against the 50 ms restart lead.
- Check it on the bench with `hw_b8_cold_change_capture` at 390 625 S/s.

### B3 (P1) — F3's drop is still reachable: a burst admitted after the open burst's last buffer (with end-of-burst) went out, starting at that burst's end

**Location.** `tx.rs:234–241` and `tx.rs:289–295`. A burst that ends at its waveform (`ends_waveform`, no held burst yet) sends `device_eob = true` on its last buffer and leaves `continues_at` unset. A burst booked afterwards at that end tick goes through `step` → open → `send` with `device_sob` and a time spec at the device cursor, which is exactly the X300's gap-0 case (`hw_b8_raw_burst_gap`).

**Reachability.** The last buffer goes out up to the in-flight window (10 ms) before the burst's end, and admission needs only 5 ms of lead. So a client that submits the next burst 5–10 ms before the previous one ends is admitted and then dropped by the device. RM-15 allows this start, and the Mock plays it.

**Evidence (committed fake, only a test added).** `probe_n_a_burst_at_the_end_of_a_sent_burst` sends a 30 000-sample burst A at lead 20 ms, polls until A's end is ~7.8 ms away, then sends B at A's end tick.
- 2 of 3 Runs: the fake saw `tx_send … eob=true` for A, then `tx_send n=1000 at=410826400 sob=true` exactly at A's end (404 826 400 + 30 000 × 200).
- It reported `TimeError`, and the Manifest has `TIME_ERROR { outcome: late_at_device }`.
- The third Run's B was dropped by UR-21 as 0.33 ms late: the probe's timing, not the path.

**Fix.** Defer the device end-of-burst of a burst that ends at its waveform:
1. Send its last buffer without end-of-burst and set `continues_at = end`.
2. If no burst is held at `end` by the time `end` is a device lead (2 ms) away, send the empty end-of-burst buffer. This extends `step`'s existing "vanished continuation" branch (`tx.rs:199`) with a deadline.
3. A burst booked at `end` before then continues the device burst, as UR-23's path does now.
4. Spec 18 UR-23 then says that every end of burst is continued when the next Kernel burst starts at its next sample, whenever that burst was booked.

Rejected alternative: treating such a B as late in uhd-tx. RM-15 allows the start and the Mock plays it.

## NONBLOCKING

- **N1 (P2, INFERRED) — F3's gap threshold is in radio ticks, not samples.**
  - `hw_b8_raw_burst_gap` ran at 1 Msps only (1 sample = 200 ticks). The late report came 15 ns (3 ticks) after the target.
  - `radio_tx_core.v:337–379` confirms why: after an end-of-burst word the FSM goes `ST_TRANSMIT` → `ST_IDLE` → `ST_TIME_CHECK`, and the registered `time_now` / `time_past` compare against the new head packet's timestamp only after that.
  - So at rates with N ≤ 3 or so (200, 100, 66.7 Msps), a next burst 1–3 samples after the previous end is probably late too. The fix continues only at gap 0, and the fake's rule (`device.rs:825`, `t <= tx_cursor`) is in samples at any rate.
  - Fix: add `hw_b8_raw_burst_gap` at 200 and 100 Msps to bench.md for Phase 8, and make the fake's rule a tick threshold once measured. §11 F3's "at T + 1 sample and later, never" should say "at 1 Msps".
- **N2 (P2) — the fake's `TimeError` tick is `now` at send time** (`device.rs:826`). For a future target at the cursor, `late_by_ns` comes out negative (probe: −6 076 825 ns in `TIME_ERROR late_at_device`). The X300 reports a tick just after the target. Fix: report `max(now, t)` or the cursor.
- **N3 (P2) — `unacked` accounting is right but untested.** One push per device start-of-burst (`tx.rs:274`) and one pop per `BurstAck` match the bench (a dropped burst gives `TimeError` without `BurstAck`). The mutation "push per Kernel burst" survives (T4).
- **N4 (P2) — the Abort path records a stop that was already issued.** `RxCmd::Cut { Abort }` stops untimed at once and records nothing. The following `Timeout` past the cut then calls `stop_at_cut`: a second `rx_stop now`, harmless, plus an `applied` `rx_stop` row whose `issued` is later than the real stop. `Shutdown(Abort)` records none. Fix: record the abort's stop in `applied` and skip `stop_at_cut` when the stream was already stopped.
- **N5 (P2) — spec and design-notes text.**
  - Spec 18 UR-17 still says `rx_recv(block_len, 100 ms)`, although UR-25 now bounds the wait.
  - UR-25's "the start at `e₂` is not missed" holds only while one block plus the delivery latency is under the restart lead (B1).
  - Design-notes §11's lead paragraph (line 291) still says "No Module code … has changed; the three findings below are open", while its last subsection says they are closed.
  - The F1 recommendation said the honouring fake behaviour would stay as a fault; the change removed it. "Tests first" explains why, but "as written" is then not literal.
  - F1's rejected alternative (b) is VERIFIED from source rather than INFERRED: `radio_rx_core.v:570` ends a continuous command only on `cmd_stop` or overrun, and the FIFO pops only in `ST_STOP`.
- **N6 (P2) — VERIFIED/INFERRED marks otherwise check out.**
  - F1: `radio_rx_core.v:171` ("All commands go into the command FIFO except STOP") and `:526` ("timed STOP commands are not supported") are verbatim, and the X3xx image cores include the RFNoC radio (`x3xx_radio_base.yml`).
  - "UHD's `recv` on a stopped stream waits its whole timeout" (UR-33) is VERIFIED: `rx_streamer_impl.hpp:278–319` honours the timeout and reports `TIMEOUT`.
  - F3's INFERRED continuation point is now VERIFIED-on-bench as not late (`hw_b8_preemption` at `bc0db98`: no `TimeError`, no `L`). That the continued samples play at the intended sample remains INFERRED; no correlation was run.
- **N7 (P2, pre-existing) — two issues outside the fix.**
  - UR-29's 1 s silence check is off whenever a cut is pending (`rx.rs:247`), so a `cold` change booked far ahead disables lost-device detection until `e₁`.
  - A send failure or a short send (`tx.rs:247–254`, `abandon`) leaves a device burst without end-of-burst, now including a continuation's first buffer.

## TEST_GAPS

- **T1 (P1) — the fake has no delivery latency and no in-flight tail after a stop.** It is looser than the bench-measured device (`rx_before_origin` 1; "after the stop: 2 blocks"). That is why B1 and B2 are invisible; Vision §13/§59 treat a looser double as a defect. `rx_before_origin` (`rx.rs:302`) is reached by no test.
  - Fix: a `FakeConfig` delivery latency with packet-granular, partial-return `rx_recv`, and an untimed stop that leaves the samples before it deliverable ahead of the next stream's (the probe needed ~100 lines in `device.rs`).
  - Add tests: the B1 case (`block_len` 65 536 at 390 625 S/s, no new-clock block with old data, no `LATE_COMMAND`) and the B2 case (390 625 S/s at ~3 ms latency, old clock delivered to `e₁`).
- **T2 (P1) — the fake's three new strict behaviours are not pinned.** Each loosening survives the whole `ur_` suite (90 tests):
  - a timed `rx_stop` keeps the stream running;
  - a stopped stream's `rx_recv` returns after 10 ms;
  - a timed start-of-burst at the cursor is accepted.

  R12, B03 and B04 kill only because the fake is strict, so a later loosening would silently disarm them. UR-33 says each named way has a test. Fix: extend `ur_33_the_fake_device_keeps_the_devices_queues`, or add a `ur_33_*` test, asserting each behaviour directly on the device.
- **T3 (P2) — F3's device-burst bookkeeping is untested.** These mutations survive:
  - `end_open(true)` ignoring a continuation with no open burst (Stop, `cold` switch or `Provider::stop` in the window between a Kernel burst's end and its continuation's first send);
  - removing `step`'s vanished-continuation check (`tx.rs:199`);
  - `abandon` keeping `continues_at`;
  - `unacked` per Kernel burst.

  Fix: a test with a held continuation and a `Stop` (and a `cold` transmit change) delivered before the continuation's first buffer. `FakeFault::SlowSend` or a hold on uhd-tx can widen the window. Each should assert one closing end-of-burst and `unended_bursts() == 0`. Also a `TimeError` after a continued burst, checking the reported target.
- **T4 (P2) — `recv_timeout`'s 1 ms floor is untested.** Removing it (0 allowed) survives. The floor keeps UHD's `timeout_ms` above 0 (the cast truncates) and prevents a spin. Fix: assert in the silent-stream test that the fake saw no `rx_recv` with a zero timeout, or count calls.
- **T5 (P2) — R18's guard (a `LateCommand` while a cut is pending) is reached by no test**, as design-notes §11 records. With B1's scenario (a switch after `e₂`) a late start can now meet a pending cut more often. B1's test (T1) can cover it.
- **T6 (P2) — bench checks for Phase 8:** the B2 check at 390 625 S/s, and N1's `hw_b8_raw_burst_gap` at 200 and 100 Msps.
