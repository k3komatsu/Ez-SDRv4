# Review O — re-review of the fixes to Review N (`git diff 9010c5b aaf1e00` on `worktree-phase7-impl`)

Reviewer: Claude Opus, resuming the Review N session, 2026-09-30. I changed no repository files. The scratch copy was `/tmp/claude-1000/-home-komatsu-works-Ez-SDRv4/d82454b2-b1d7-4f3c-a624-72d8e684572b/scratchpad/review-o/copy`, made with `rsync -a --no-times` and touched before each build session. Every build ran in `ezsdr-v4-dev:uhd4.10` with `CARGO_TARGET_DIR=/cargo-target/review`, one at a time, and the copy was restored to match the tree after each probe. No USRP was used.

Brief: [`../prompts/review-o.txt`](../prompts/review-o.txt). The report as returned.

## VERDICT

**Not ready for Gate X yet. Two P1 findings remain; everything else Review N raised is closed or honestly recorded as open.**

**What was fixed.** The fixes close Review N's B1 and B2, close B3 at gap 0, and close T1 and T2.
- **Receive:** `recv_len` bounds each request to the cut. `not_before` drops the old stream's tail after a late switch. `quiet_span` keeps a slow link's last samples before the cut; without it (Review N's rule alone) the 60 ms-link test fails, which justifies the departure.
- **Transmit:** the deferred device end-of-burst plays a burst booked at a sent burst's end, on the fake and on the bench.
- **Fake:** it now has a receive link (latency, packets, an in-flight tail after an untimed stop), and its strict behaviours are pinned. Each loosening I tried is killed by a `ur_33_*` test.

**Test results.**

| Run | Result |
|---|---|
| Fake suite, stable | 115/115, four runs |
| Fake suite, 1.85.0 | 115/115 in 8 of 9 runs; one intermittent failure (N-7) |
| `--features uhd` | 114/114 |
| 14 rewritten or new mutation rows (U17, R13, B01–B03, B05–B13) | All killed. B08 needs its `old2` edit: I reran it with both edits, killed 5 of 5. |

**Open.**
- **O-B1 (P1):** the deferred end-of-burst is sent as an empty buffer, which UHD turns into one zero sample. A burst booked one sample after a burst's end — played at `bc0db98` — now meets the device burst's end tick and is dropped.
- **O-B2 (P1):** R-1 is real, and it reproduces on the committed fake in 2 of 6 booking phases. `ur_25_a_long_block_at_a_low_rate_stops_the_stream_at_e1` passes only because its booking phase happens to be favourable.
- **Fake fidelity:** its cached `TIMEOUT` contradicts the UHD source it cites; it is stricter than the device, but no Module test depends on it (N-1).

## BLOCKERS

### O-B1 (P1) — the deferred device end-of-burst lengthens every device burst by one sample, so a burst booked one sample after another's end is dropped (a regression from `bc0db98`)

**Location.**
- `tx.rs` `send`: `device_eob = false`, so a Kernel burst's last buffer never carries end-of-burst.
- `tx.rs` `step`: the `continues_at` match ends the device burst with `end_open(true)` when another burst is next or at the deadline.
- `tx.rs` `end_open`: sends `tx_send(&empty, None, false, true, …)`.
- `device.rs` `tx_send`: an empty buffer adds no sample to the fake's cursor.

**Mechanism (VERIFIED in UHD 4.10's source).** `host/lib/include/uhdlib/transport/tx_streamer_impl.hpp:266–276` handles a send of zero samples ("such as end of burst") this way: "Send packets need to have at least one sample based on the chdr specification, so we use `_zero_buffs`", with `num samples` = 1. So the device burst of a burst ending at tick `c` ends at `c + 1` sample (one transmitted zero).

**The regression (INFERRED on the device).**
1. A burst B booked at `c + 1` (gap 1) is not the continuation, so `step` ends the device burst with the empty buffer.
2. B's timed start-of-burst at `c + 1` then equals the device burst's end: the X300's gap-0 case (`hw_b8_raw_burst_gap`: `TimeError`, not played, 4 of 4).
3. At `bc0db98` the last data buffer carried end-of-burst, the device burst ended at `c`, and gap 1 was played (same bench test, 4 of 4).

In general every next burst now meets an effective gap one sample smaller. The Review L path (end-of-burst after a buffer already sent) had this before, but only in rare cases; now it is on every burst end.

**Evidence (scratch copy).** Probe `probe_o_a_burst_one_sample_after_a_burst`: burst A (30 000 samples) at a 40 ms lead, then B booked at A's end + 1 sample.
- On the committed fake: played, 3 of 3. The fake is looser than UHD here.
- With the fake's empty end-of-burst made one zero sample, as UHD sends it (a single line in `tx_send`):
  - B was reported `TimeError` / `TIME_ERROR late_at_device` in 3 of 3 runs.
  - The whole `ur_` suite still passes (103), so the faithful model costs no existing test.

**Fix (one recommendation).**
1. Hold back the last sample of a Kernel burst's final buffer instead of sending an empty end-of-burst.
2. Send that sample with end-of-burst:
   - at the deadline,
   - when another burst is next,
   - at `Stop`, a `cold` switch, `Provider::stop` or `abandon`.
3. If a continuation is booked at `c`, send it without end-of-burst, just before the continuation's first buffer.

The device burst then ends exactly at `c`, as at `bc0db98`, and gap 0 still continues. Make the fake's empty end-of-burst one zero sample, as UHD sends it (T-a), and add a gap-1 test (T-a).

Rejected alternatives:
- Continuing the device burst across a gap of one sample: it pads a sample.
- Refusing gap-1 bursts: RM-15 allows them and the Mock plays them.

### O-B2 (P1) — R-1 is real and reproduces on the committed fake; recommendation (a) is sound with one correction

**Location.** `control.rs:323` and the `e` rule of UR-25 (`e ≥ now + 50 ms`). The receive call already in progress when a `cold` change is booked asks for a whole `block_len` and is not bounded by `recv_len`, because the cut did not exist when the call began.

**Evidence (committed fake, only a probe test added).** `probe_o_r1_booking_inside_a_long_call`: bench link 2 ms, `block_len` 65 536, 1 Msps → 390 625 S/s, then a second `cold` change to 2 Msps booked at 200 + j·28 ms, for j = 0…5.

| j | Stop after `e₁` | Switch vs `e₂` | Outcome |
|---|---|---|---|
| 3 | 104.6 ms | 55 ms after | `LATE_COMMAND`, UR-17 restart 105 ms after `e₂` (`rx_restart` at 498 602 700 for 477 589 700) |
| 4 | 76.5 ms | 27 ms after | `LATE_COMMAND`, restart |
| 5 | 47.8 ms | 0.26 ms before | on time, barely |
| 0–2 | 4–18 ms | before | clean |

`not_before` kept every old sample off the new clock: B1's mislabelling stays closed. The bench's 25.5 ms (bench-results part 4) is one such phase.

**Consequences.**
- `ur_25_a_long_block_at_a_low_rate_stops_the_stream_at_e1` asserts "no `LATE_COMMAND`". It passes only because its fixed 60 ms wait books the change at a favourable phase of the 168 ms call, so it is timing-fragile on another machine.
- Spec 18 UR-25's "the start at `e₂` is on time while that delivery and the switch take less than the restart lead" does not hold for this case.

**Is (a) sound?** Yes: if `e` is at least one old-rate block (plus delivery) beyond the restart lead, the call in progress ends before `e₁`, every later call is bounded by `recv_len`, and the stop follows `e₁` by about one delivery. Correction: the bound is `e ≥ now + RESTART_LEAD + block_len · N_old + DELIVERY_ALLOWANCE`. That is the old stream's block duration, not a fixed block, plus the 3 ms the call's last packet needs to arrive.
- Apply it only when a receive stream runs.
- It costs 5–8 ms per receive `cold` change at the default block (2 000 samples = 2–5.1 ms at 1 Msps–390.6 kS/s, plus 3 ms).
- UR-25's `LATE_COMMAND` threshold for `at` moves by the same amount, and the spec should say so.
- Rejected: (b), assembling blocks from short calls, because it changes UR-17's block model.

Add the phase sweep as a test (T-b).

## NONBLOCKING

- **N-1 (P2) — the fake's cached `TIMEOUT` is stricter than UHD, and the spec and a test assert it as UHD's behaviour.**
  - Code: `device.rs` `rx_cached_timeout` (a call cut short by a packet timeout makes the next call return `Timeout` at once). Spec 18 UR-25 ("one cut short is followed by a `TIMEOUT` at once") and UR-33, and `ur_33_a_recv_cut_short_by_a_packet_s_timeout_is_followed_by_a_timeout_at_once`, all say this is UHD's behaviour.
  - UHD's source says the opposite (`rx_streamer_impl.hpp:25–47`): "Timeout errors are an exception … the error is not returned in the next call", and `store()` ignores `ERROR_CODE_TIMEOUT`.
  - Harmless to the Module: removing the cache fails only that `ur_33` test (mutation O-1, 102 others pass).
  - Fix: remove the cache; make the test assert a partial return and a normal next call; correct UR-25 and UR-33.
- **N-2 (P2) — the fake aligns packets to each request's start instead of to the stream's own packet grid.**
  - The first packet of each request waits for `next + packet + latency` (UHD returns the leftover of a partly read packet at once), which is stricter.
  - A short last request near the cut arrives at `next + len·N + latency`, where the device needs the whole packet containing it, up to one packet (5.1 ms at 390.6 kS/s) later. This is looser: the fake understates the stop's delay after `e₁` at low rates.
  - Neither breaks a test or the `e₂` margin. Fix: align packets to `origin + j·spp·N` and return leftovers at once.
- **N-3 (P2) — `not_before` is taken before the `rx_stop` call** (`stop_at_cut`: `now` read before `rx_stop`). The bench shows the X300 producing samples until ~0.37 ms into the call (`hw_b8_raw_rx_timed_stop`, lead 0: call 64 615 084…64 698 267, last sample end 64 689 800). After a late switch (O-B2), a tail block starting in that window could pass. It is practically unreachable, because the tail arrives in one `recv` ending at its end-of-burst. Fix: read the instant after the call and add the device lead (the new stream starts ≥ 50 ms later).
- **N-4 (P2) — no end-of-burst deadline when the held continuation is at or after `e₁`.** In `step`, `continues_at == c` with a burst held at `c` skips the deadline and then returns `false` at `k >= e1`. The device burst is then ended only by `do_switch` at `now ≥ e₁`. When `e₁`'s sample is `c` itself, that is at or after the device runs dry: an underflow. Rare: A's waveform must end exactly at `e₁` with a burst held there. Fix: treat `k == c` with `c ≥ e1_k` as not a continuation.
- **N-5 (P2, INFERRED) — the end-of-burst margin of a lone burst.** Every lone burst's end now relies on the end-of-burst reaching the device within 2 ms less the device lead (0.3–0.5 ms) of its end, so a uhd-tx stall above ~1.5 ms underflows at every burst end. The bench (part 4) showed none. O-B1's held sample does not change this; a 3–4 ms deadline would still take bursts booked at the 5 ms lead.
- **N-6 (P2) — `quiet_span` uses `block_len` although requests near the cut are bounded.** A stream that falls silent before its cut with a block ≥ ~46 ms now ends after `e₂` (at `bc0db98`, ≥ ~50 ms). Fix: use the largest request issued since the cut was set.
- **N-7 (P2) — one intermittent failure.** `ur_25_a_cold_receive_change_delivers_every_sample_before_e1` failed once, on 1.85.0 in the first run after a fresh build. It passed in the other 13 full runs (8 on 1.85.0, 5 on stable) and in 12 targeted runs. I did not capture the message; the cause is INFERRED to be timing.
- **N-8 (P2) — a hang instead of a failure.** With the fake ignoring a timed stop (mutation O-F1), `ur_33_a_timed_receive_stop_stops_the_stream_at_once` hangs rather than fails. It was killed only by my 240 s timeout. Fix: give `blocks_until` a deadline.

## TEST_GAPS

- **T-a (P1):** no test covers a burst one sample after a burst's end, and the fake's empty end-of-burst adds no sample. Add the one-zero-sample model and a gap-1 test (O-B1).
- **T-b (P1):** no test sweeps the booking phase of a `cold` change inside a long receive call. Use `probe_o_r1` as the test, asserting no `LATE_COMMAND` once (a) is in.
- **T-c (P2):** `quiet_span`'s 3 ms delivery allowance is unpinned: without it, mutation O-4 survives the suite.
- **T-d (P2):** `abandon`'s end-of-burst is untested (mutation O-3 survives), as §12 says.
- **T-e (P2):** carried over: N3 (`unacked`), T5 (R18's guard), and T3's `cold` transmit switch during a held continuation.

## PREVIOUS_FINDINGS

| Review N finding | Status | Evidence |
|---|---|---|
| B1 (P1) old-clock samples on the new clock; switch after `e₂` | **CLOSED** for the mislabelling and for requests issued once a cut exists | B05, B06 killed; probe: no old sample on the new clock in 6 of 6 phases. The late switch from the call in progress remains as O-B2 (R-1). |
| B2 (P1) premature end on a short `Timeout` | **CLOSED** | B07 killed; mutation O-2 (Review N's rule without `quiet`) fails the 60 ms-link test, so the departure is justified. |
| B3 (P1) gap-0 drop after a sent end-of-burst | **CLOSED** at gap 0 | B08 (with `old2`) killed 5 of 5; bench part 4 played 3 of 3. It introduced O-B1. |
| N1 (P2) gap in radio ticks | **OPEN** (recorded for Phase 8) | UR-23 carries the INFERRED note. |
| N2 (P2) late report's tick | **CLOSED** | `t.max(now)`; pinned by `ur_33_a_timed_start…`. |
| N3 (P2) `unacked` untested | **OPEN** | As §12 says. |
| N4 (P2) abort's stop repeated | **CLOSED** | B11 killed. |
| N5 (P2) spec and notes text | **CLOSED** | New text wrong about UHD's timeout cache: N-1. |
| N6 marks | **CLOSED** | Nothing to do. |
| N7 (P2) UR-29 during a pending cut; `abandon` | **CLOSED** | B13 killed; `abandon`'s end-of-burst untested (T-d). |
| T1 (P1) fake without latency or tail | **CLOSED** | Fidelity caveats N-1, N-2; O-F4 killed. |
| T2 (P1) fake strictness unpinned | **CLOSED** | O-F1 (by hang, N-8), O-F2, O-F3 killed. |
| T3 (P2) continuation bookkeeping | **Partly closed** | B09, B10 killed; `cold` switch untested. |
| T4 (P2) 1 ms floor | **CLOSED** | B12 killed. |
| T5 (P2) R18's guard | **OPEN** | |
| T6 (P2) bench checks | **Partly closed** | Low rate run on the bench (part 4); N1's rates are Phase 8's. |
