# Review P — re-review of the fixes to Review O (`git diff fb29295 b3c94ba` on `worktree-phase7-impl`)

Reviewer: Claude Opus, resuming the Review N/O session, 2026-10-01. I changed no repository files. My scratch copies were:
- `/tmp/claude-1000/-home-komatsu-works-Ez-SDRv4/d82454b2-b1d7-4f3c-a624-72d8e684572b/scratchpad/review-p/copy`: the tree at `b7feab0`, made with `rsync -a --no-times` and touched before each build session.
- `…/review-p/copy-9010c5b`: `git archive 9010c5b`, used only for U13.

Every build ran in `ezsdr-v4-dev:uhd4.10` with `CARGO_TARGET_DIR=/cargo-target/review`, one at a time, and the copy was restored to match the tree after each probe. No USRP was used.

One of my runs was invalid and I discarded it. Both copies are mounted at `/work`, so after the 9010c5b build, a rebuild in the HEAD copy reused the 9010c5b library (AGENTS.md §7's mtime hazard). I re-touched the copy and repeated that run; every result below comes from a build of the right tree.

Brief: [`../prompts/review-p.txt`](../prompts/review-p.txt). The report as returned.

## VERDICT

**No blockers. Review O's two P1 findings are closed, in the code, by tests and on the bench.**

**What was fixed.**
- **O-B1:** the held last sample makes a device burst end at the Kernel burst's own end, so a burst one sample later is played. Shown by the fake (which now pads an empty end-of-burst with one zero sample, as UHD does) and by the bench (`hw_b8_burst_one_sample_after_a_burst`, 4 of 4).
- **O-B2:** a receive `cold` change's `e₁` now lies past the receive call in progress. Shown by the phase sweep on the fake and by the bench at both ends of the rates (12 of 12, no `LATE_COMMAND`).
- **N-1, N-2, N-3, N-4, N-8:** fixed.

**Reading the held-sample code against the brief's cases.** I found no sample lost, doubled or moved, and no timed start-of-burst inside an un-ended burst, in any of these: `Stop`, a `cold` switch, `Provider::stop`, preemption, a lost device, a repeat, a one-sample burst. The one exception is a send that fails or comes up short (NB-1, P2).

**O-B2's bound.** It is right where the brief asked:
- With `rx_packet_samples()` = 0 it falls back to `block_len`, which matches the fake's whole-request packet.
- A change from 0 channels goes through the `None` branch, so it is unaffected.
- A change to 0 channels is merely later, which is harmless.
- Transmit changes are unchanged.

**The fake.** Its new receive model is not stricter than the device in anything a test depends on.

**Test results.**

| Run | Result |
|---|---|
| Fake suite, stable | 119/119, two runs |
| Fake suite, 1.85.0 | 119/119, two runs |
| `--features uhd` | 118/118 |
| Rewritten and new rows B03, B05, B08, B14, B15, B16, B17 | All killed, each by the test the JSON names |

**U13.**
- It is pre-existing: the mutant survives 8 of 8 at `9010c5b`.
- At HEAD it is killed only by a race (3 of 9 runs), so it is not stable.
- It is not equivalent: a deterministic probe kills it every time (NB-2).

**Open, none blocking.** Two P2 test gaps on the new mechanism:
- the held sample's position in a late-booked continuation is not checked, and a reorder mutant survives (TG-1);
- U13's test is weak (NB-2).

Plus the P2 items below.

## BLOCKERS

None.

## NONBLOCKING

- **NB-1 (P2) — a failed or short send of a burst's final buffer can move its held last sample earlier on the device.**
  - Location: `tx.rs` `send`, where `self.tail` is set when `hold` is true before the result is checked. On `Err` or `Ok(n < sending)`, `abandon` sends that tail with end-of-burst (`close_device_burst(tail)`).
  - Effect: the device plays it right after the samples it accepted, earlier by the samples it never got.
  - It needs a device error state and no test reaches it (the fake has no failing send short of a lost device; T-d).
  - INFERRED from the code. Fix: in `abandon`, when the failing send is the one that set the tail, drop the tail and close with the empty buffer.
- **NB-2 (P2) — U13 is a pre-existing test gap, not an equivalent mutant.**
  - The mutant (uhd-control drains its queue and only then books) survives 8 of 8 at `9010c5b` and is killed 3 of 9 at HEAD, so §13's "survives" depends on timing.
  - It is observable. The coordinator's `recv()` marks the previous Action finished (`coordinator/state.rs:86–95`, `state.finished = state.taken`), so under the mutant KC-21a's submit returns before the Action is booked.
  - `ur_14_actions_are_finished_before_the_next_recv` cannot see this reliably: its `radio.rx.gain_db` update goes through the fake's timed `apply`, which records `effective=` without `apply_delay`. The test's premise "the fake's `apply` taking 20 ms" does not hold for it.
  - My probe `probe_p_u13_enable_finished_at_return` (fake `apply_delay` 20 ms; submit `radio.tx.channels = 1`; at return, assert an `apply tx` call and a `usrp/tx` clock) passes 8 of 8 iterations on the code and fails 8 of 8 under U13.
  - Fix: make that probe the ur_14 test, or add it.
- **NB-3 (P2) — a one-sample burst is not held** (`!(device_sob && count == 1)`), so a burst booked later at its end meets gap 0 and is dropped.
  - Narrow; the spec states it.
  - UHD would allow holding it too: a zero-sample send with start-of-burst caches its metadata for the next send (`tx_streamer_impl.hpp:194–197`).
  - The guard itself is unpinned: the mutant that holds the first buffer anyway (P-1) survives. Under it the start-of-burst and the time spec are never sent, so the burst would play untimed at the deadline. See TG-2.
- **NB-4 (P2) — O-B2's bound uses `max(block, packet)`, but a call can end up to `block + packet` after it began** (the block's last sample sits in a packet that ends up to one packet later).
  - The restart lead absorbs this: at the worst case measured, 390 625 S/s, `block_len` 65 536, 1 996-sample packets, the switch came 44.8 ms before `e₂`.
  - Only the comment and UR-25's wording overstate the bound. No change needed beyond wording.
- **NB-5 (P2) — the TimingEnvelope does not advertise the larger receive `cold` lead.** With `block_len` 65 536 at the lowest rate, a Session's `at` closer than ~221 ms gets `LATE_COMMAND`, where MockRadio (restart lead 0) takes it. Recorded for Phase 8's parity.
- **NB-6 (P2) — the fake's packet grid survives an overflow.** After an overflow skip it keeps the old grid, whereas the radio restarts its packets at the restart. No test combines `rx_packet` with an overflow. INFERRED, minor.
- **NB-7 (P2) — a test comment overstates what it covers.** `ur_23_two_bursts_back_to_back_loop_back_whole` says it checks the held sample sent ahead of the second burst. Both bursts are scheduled up front, so the first one's last buffer ends at the held burst (`ends_at_held`) and nothing is held. The test checks the gap-0 continuation, not the held sample (see TG-1).

## TEST_GAPS

- **TG-1 (P2) — the held sample's position in a late-booked continuation is not checked.**
  - Mutant: send the tail after the continuation's first buffer instead of before it. Survives every `ur_23_*` and `rehearsal_*` test (8 and 10 passed).
  - Mutant P-4: send the tail with end-of-burst. Also survives, but it is close to equivalent on the X300 (an untimed packet after end-of-burst follows at once).
  - B16 is killed only by the 31 000-sample count, which counts samples, not their order.
  - Fix: run `ur_23_two_bursts_back_to_back_loop_back_whole` with the second burst booked after the first's last buffer went out, and compare the exact loopback.
- **TG-2 (P2)** — no test covers a one-sample burst booked well ahead (P-1 survives).
- **TG-3 (P2)** — O-B2's packet term is unpinned (P-5: `block_len` only, survives). The fake test runs `block_len` 65 536, longer than the packet; the bench covers `block_len` 100. Fix: a fake case with `block_len` 100 and `rx_packet` 1 996 at 390 625 S/s.
- **TG-4 (P2)** — N-3's device-lead margin (P-2) and N-4's `e₁` guard (P-3) are unpinned, as §13 records.
- **TG-5 (P2), carried over** — T-c (the delivery allowance in `quiet_span`), T-d (`abandon`), T-e (N3 `unacked`, T5 R18's guard, a `cold` transmit switch during a held continuation).

## PREVIOUS_FINDINGS

| Review O finding | Status | Evidence |
|---|---|---|
| O-B1 (P1) empty end-of-burst lengthens the device burst | **CLOSED** | VERIFIED on the bench (`hw_b8_raw_empty_eob_gap`: gap 1 late 4 of 4 with an empty end-of-burst, 0 of 4 with end-of-burst on data). Fixed by `Tx::tail`; `ur_23_a_burst_one_sample_after_a_burst_is_played` passes; `hw_b8_burst_one_sample_after_a_burst` 4 of 4 played; B14, B16, B17 killed. Gaps: NB-1, NB-3, TG-1, TG-2. |
| O-B2 (P1) R-1, the call in progress | **CLOSED** | `e` ≥ now + 50 ms + max(block, packet)·N + 3 ms for receive; `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time` (my probe as a test); B05, B15 killed; bench 12 of 12 phases at 200 Msps and 390 625 S/s (`block_len` 100 and 65 536). Gaps: NB-4, NB-5, TG-3. |
| N-1 (P2) cached `TIMEOUT` | **CLOSED** | Cache removed; test and spec corrected. |
| N-2 (P2) packets per request | **CLOSED** | Stream grid; P-6 (back to the request grid) killed by `ur_25_a_slow_link…` and `ur_33_a_stopped_stream_s_tail…`. NB-6 minor. |
| N-3 (P2) `not_before` before the call | **CLOSED** | Instant after the call plus a device lead; unpinned (TG-4). |
| N-4 (P2) continuation at or after `e₁` | **CLOSED** | Guard in `step`; unpinned (TG-4). |
| N-5 (P2, INFERRED) end-of-burst deadline margin | **OPEN** (recorded) | Bench parts 4–5: no underflow. |
| N-6 (P2) `quiet_span` uses `block_len` | **OPEN** (accepted) | Harmless with O-B2. |
| N-7 (P2) intermittent 1.85.0 failure | **OPEN** | Not seen here: 4 full runs, plus the mutation runs. |
| N-8 (P2) hang instead of failure | **CLOSED** | `blocks_until` fails after 5 s. |
| T-a (P1) fake's empty end-of-burst | **CLOSED** | `ur_33_an_empty_end_of_burst_is_one_zero_sample`; B17 killed. |
| T-b (P1) booking-phase sweep | **CLOSED** | The `ur_25` phase test; B05, B15 killed. |
| T-c, T-d, T-e (P2) | **OPEN** | As §13 records (TG-5). |
