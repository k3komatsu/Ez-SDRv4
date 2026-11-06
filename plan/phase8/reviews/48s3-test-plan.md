# Test plan: design note 48 S3 (#78's remaining cause)

Round 2 of step 1 (`brief-impl.md`, `brief-plan2.md`). No tree edits; this file is the only output. Written against HEAD `f1d502e`. It carries the rulings in `rulings-plan.md` (D1–D4) and `rulings-sol-plan.md` (Sol's P1–P6, reviewing `sol-plan-final.md`). Probe copies: `/tmp/n48s3/p0` (HEAD, probe test only) and `/tmp/n48s3/p1` (S3 Module edits and the P1 Mock frontier, with probe tests); logs and the edit diff are in `probes/`.

**Stop point.** One decision is needed before step 2 (O1, §1): two inside-window bookings are judged differently by the Mock and the Module, and the ruling "parity for bookings received inside the window must hold in S3" cannot be met for them without a mechanism. The rest of step 2 can start on the orchestrator's go; the B→A known-divergence test and KL-32's wording wait for O1.

---

## 1. Decisions

### Carried from the rulings

| Item | Ruling | Where this plan carries it |
|---|---|---|
| D1 | No receipt-time mechanism. #79 is widened to 1–32 samples and stays in note 47 stage 5. KL-32 entry. In-window parity (Q5) must hold. | §4 Q5 (parity, probe-verified for A→B), §4 "#79 early-booked", §8 (KL-32, MR-17, failure-mechanisms #79 row), §9 G1, G8 |
| D2 | FakeDevice rule: variant B (fires only at a later device tick), both traps fixed, one Underflow per device burst. | §2 model, §4 Q9, §8 (UR-33) |
| D3 | S1's drop stays for closed (SOB+EOB) timed packets of 1–3 samples; open first packets of 1–29 samples at interpolation ≥80 underflow. `ur_33_an_undersized_timed_first_packet_is_dropped` becomes a closed packet. | §4 Q9, §5 (device.rs:2710) |
| D4 | Unmeasured gaps are stated, not decided. The model keeps the brief's 30. | §9 G3, G4 |
| P1 | Mock frontier for a closed finite burst of ≤32 samples is its padded end plus one, replacing its handed end plus two. Test end and end+1 at 4, 10, 31, 32 inside the window. Rename the S1 test by what it covers. Record B→A on both Providers. | §3 (probes), §4 Q5 and Q10, §7 (N48S3-27), §8 (MR-17), §4 O1 (B→A outcome) |
| R3 (rulings-fix3, reverses P1's frontier) | The Mock returns to note 40 D5: its frontier is handed end + 2 (HEAD's, with S1's floor below four), the framing inference (`continues`, the closed-short floor, the second-Stop change) is gone, and MR-25 keeps the 32 threshold. Parity tests state D5's relation: `s3_parity_booking_after_a_closed_short_burst_follows_d5`, `s3_parity_booking_after_a_device_burst_s_short_tail_follows_d5`. Rows: §7. | §4 (parity pair), §7, §8 (MR-17 unchanged, MR-25, Changes row) |
| P2 | Preemption cut (a held booking inside a not-yet-handed long A) leaving 10, 31, 32 samples, separate from the clock cut. Window-limited SOB after abandon (31 nothing, 32 handed). Close bound is `<= 32`, not `< 32`. | §4 Q1 (reopen), Q7 (clock cut), Q7b (preemption cut), §7 |
| P3 | Keep the untimed underflow test as a closed-packet control. Add a separate delayed open-burst test asserting the full report sequence, no BurstAck, and the continuation not transmitted. | §4 Q9, §5 (device.rs:2727) |
| P4 | Discriminating setups for N48S3-03, -05, -09, -11. Remove equivalent rows (none found; one duplicate removed). One named killer per row. | §7 |
| P5 | `ur_22_a_first_send_cut_short_keeps_the_reports_paired` retargeted to a 32-sample window, keeping its report-pairing observation. | §5 |
| P6 | UR-23 continuation rules get the short-burst exception. KL-32 kept. No receipt-time mechanism. | §8 (UR-23) |
| Sol's note | Unequal channel lengths are unreachable (`provider/control.rs:386–403`). No test. | §9 G13 |

### Open: O1 (new, orchestrator decision)

Two inside-window cases are judged differently by the two Providers. Both are **pre-existing at HEAD** (`f1d502e`, unchanged code) and S3's P1 change does not affect them (§3, rows marked B→A and long A):

- **B→A receipt order.** Booking B is received first, then booking A, with A's target earlier than B's. Both are received at 91 ms, inside the window. The Mock drops A (it is judged against B's front, received earlier) and plays B. The Module plays A and drops B (B at A's end) or plays both (B at A's end+1). Probed at 1, 4, 10, 32 and 100 samples (HEAD); at 1, 4, 10, 31, 32 with S3 and P1.
- **Contiguous booking after a long held burst.** A of 100 samples at 100 ms, B at A's end, both received inside the window. HEAD: the Mock drops B; the Module continues the held tail and plays both. Not probed with S3; the Module's behaviour for bursts over 32 samples is unchanged (INFERRED).

Why these differ: MR-17 (`design/09-mock-radio.md:143`) lets an earlier-received burst set the Mock's front on a later receipt, so the Mock judges the later-received A against B's front. UR-21 (`design/18-uhd-radio.md:246`) decides preemption at dispatch against the open burst; a burst that is not yet handed does not preempt. The only UHD test of receipt order (`ur_21_booking_receipt_order_changes_preemption`, tx.rs:1999) covers "handed before later receipt". So the two specs disagree about a not-yet-handed burst, and this is #79's mechanism (the Mock decides at receipt, the Module at dispatch), in a case #79 does not name.

Options:

- **(a) Recommended.** Record both cases in KL-32 (its scope widened to these two forms, with the D1 symptom). Pin them with known-divergence parity tests that assert the recorded outcome, so a fix turns them red. Tie them to #79 in note 47 stage 5 (one `timeline` rule on both sides). Q5 keeps the A→B tests, which hold.
- **(b) Rejected.** Make MR-17 ignore a received but unhandled burst. Not probed. By inference from the §3.1 rows, it would fix only the reverse-order end+1 case, because the Module still drops B at A's end, and it is a receipt-time mechanism that D1 rules out.
- **(c) Rejected.** Make the Module decide by receipt order. This changes UR-21's dispatch rule and `ur_21_booking_receipt_order_changes_preemption`, and is a Module mechanism beyond S3.

---

## 2. Model facts (what the tests rely on)

| Fact | Status | Source |
|---|---|---|
| Open device burst, first send ≤28 samples, continuation 0.5–2 ms later, underflows at interpolation ≥80; ≥30 never; ≤50 fine at 2 samples | VERIFIED (bench) | `plan/phase8/bench-results.md` §2026-11-06; `plan/phase8/tools/uhd-repro/tx_repro.cpp` case `head` |
| No-gap continuation: 28 samples passed 3 of 5 | VERIFIED (bench) | same |
| Closed (SOB+EOB) 2–3 samples timed dropped at 1–2 MS/s; ≥4 fine | VERIFIED (bench 2026-11-05) | brief line 11 |
| The Module hands the window's share as the first send (6–28 in failing trials, 145–156 passing) | VERIFIED (bench `ProbeDevice` log) | brief line 12 |
| `send` closes a short first send even with a booking at its end (`device_eob`, count `<` 4 at HEAD, line 572); holds the last sample of a longer burst unless booked there (`hold`, line 570) | VERIFIED (code) | `provider/tx.rs:570–572` |
| The first send waits for `min_first` samples of window space (line 547); the first send is widened to `min_first` (line 540) | VERIFIED (code) | `provider/tx.rs:538–549` |
| A clock cut leaving <4 first samples refuses the send (`ClockEnded`) | VERIFIED (code) | `provider/tx.rs:531–533`, refusal text 764 |
| `send` has **no close term for a clock cut**: `clock_cut` is used only at 531, 542 and 553; `eob` at 554 is the waveform end or the held cut | VERIFIED (code) | `provider/tx.rs:531, 542, 553–554` |
| Held-tail close is padded to two; next safe start = payload end + 2 (the closing send's exclusive end, padding included, plus one) | VERIFIED (UR-22 text, brief line 54) | `design/18-uhd-radio.md:249` |
| Mock's frontier: `max(previous, handed end + 2, device floor)` | VERIFIED (code) | `ezsdr-mock-radio/src/lib.rs:1380–1384` |
| Mock's receipt-time front counts an earlier burst only if its start is before the horizon | VERIFIED (code and MR-17 text) | `design/09-mock-radio.md:143`; `lib.rs:278–284` |
| `short_burst_floor` applies at `len < 4` only at HEAD | VERIFIED (code) | `lib.rs:294–295` |
| A booking at the closed front (the device end plus one) is on time; at the device end, late | VERIFIED (probe, §3.1 A→B rows) | §3.1 |
| Window reach: at 85 ms a booking at 100 ms is not yet in the Mock's window; at 91 ms it is (both Providers) | VERIFIED (probe, §3) | §3 |
| Mock and Module agree on A→B bookings inside the window at end and end+1 for 1–32 samples, once P1 is in the Mock | VERIFIED (probe, §3) | §3 |
| The Mock and Module disagree on the reverse receipt order inside the window, at HEAD and with S3 | VERIFIED (probe, §3) | §3, O1 |
| All 27 parity tests, 8+29+27+69+1 mock-radio tests and tests/fake.rs (336) pass with the S3 Module and P1 Mock | VERIFIED (probe run, §3) | §3 |
| 8 Module lib tests fail with the S3 Module and P1 Mock (list §5) | VERIFIED (probe run, §3) | §3, §5 |

---

## 3. Probe record (throwaway copies only)

Copies are rsynced from the tree with fresh timestamps (`--no-times --exclude .cargo --exclude target --exclude .git --exclude v3`; `find -exec touch`). Build: `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`. The probe test reads the burst length from `PROBE_LEN`; that switch is in `p1` only and is not kept. The edit diff is `probes/p1-probe-edits.diff`.

**Edits in `p1` (the S3 Module and P1 frontier):** tx.rs 540 (`32`), 547 (`wanted.min(32)`), 570 (`count <= 32`), 572 (`count <= 32`); lib.rs 294–295 (`len <= 32`, floor `len.max(4) + 1`), 1377 (`32`), and the `handed` merge skipped for a closed short burst (`if repeat || len > 32`). Device rule (D2) **not** in `p1`.

### 3.1 Inside the window (both bookings received at 91 ms)

A is `n` samples at 100 ms. B is `n` samples at A's start plus `off`. "Drop" means a `TIME_ERROR` drop of that burst. ✔ agree, ✗ disagree.

| n | Order, off | HEAD (Mock / Module) | S3 Module + P1 Mock |
|---|---|---|---|
| 1 | A→B, off 1 | ✔ both drop B | ✔ both drop B |
| 1 | A→B, off 2 | ✔ both drop B | ✔ both drop B |
| 1 | B→A, off 1 and 2 | ✗ Mock drops A, plays B; Module drops B, plays A | ✗ same |
| 4 | A→B, off 4 | ✗ Mock drops B; Module plays both | ✔ both drop B |
| 4 | A→B, off 5 | ✗ Mock drops B; Module plays both | ✔ both play |
| 4 | B→A, off 4 | ✗ Mock drops A; Module plays both | ✗ Mock drops A; Module drops B |
| 4 | B→A, off 5 | ✗ Mock drops A; Module plays both | ✗ Mock drops A; Module plays both |
| 10 | A→B, off 10 | ✗ Mock drops B; Module plays both | ✔ both drop B |
| 10 | A→B, off 11 | ✔ both drop B | ✔ both play (S3 Module with HEAD Mock: ✗ Mock drops B, Module plays both) |
| 10 | B→A, off 10 | ✗ Mock drops A; Module plays both | ✗ Mock drops A; Module drops B |
| 10 | B→A, off 11 | ✗ Mock drops A; Module drops B | ✗ Mock drops A; Module plays both |
| 31 | A→B, off 31 and 32 | not run | ✔ both drop B at 31; ✔ both play at 32 |
| 31 | B→A, off 31 and 32 | not run | ✗ same pattern as 10 |
| 32 | A→B, off 32 | ✗ Mock drops B; Module plays both | ✔ both drop B |
| 32 | A→B, off 33 | ✔ both drop B | ✔ both play |
| 32 | B→A, off 32 | ✗ Mock drops A; Module plays both | ✗ Mock drops A; Module drops B |
| 32 | B→A, off 33 | ✗ Mock drops A; Module drops B | ✗ Mock drops A; Module plays both |
| 100 | A→B, off 100 (contiguous, long A) | ✗ Mock drops B; Module plays both | not run (INFERRED unchanged) |
| 100 | A→B, off 101 | ✔ both drop B | not run |
| 100 | B→A, off 100 and 101 | ✗ Mock drops A; Module plays both or drops B | not run (INFERRED unchanged) |

Log files: `probes/p0-head-n{1,4,10,32,100}.log` (HEAD; n=1 in `p0-head-n1.log`), `probes/p1-run1-n{4,10,31,32}.log` (S3 Module with HEAD Mock), `probes/p1-fix-n{1,4,10,31,32}.log` (S3 Module with P1 Mock).

### 3.2 Early-booked (received at 85 ms, before the window reaches A): #79

| n | Booking | S3 Module + P1 Mock |
|---|---|---|
| 4 | B at end | ✗ Mock plays both; Module drops B |
| 4 | B at end+1 | ✔ both play |
| 32 | B at end | ✗ Mock plays both; Module drops B |
| 32 | B at end+1 | ✔ both play |
| 10 | B at end | ✗ (earlier run, S3 Module with HEAD Mock; the P1 frontier does not change it, because the early A is not counted at B's receipt, INFERRED) |

Logs: `probes/p1-early-n{4,32}.log`; the 10-sample row is `probes/probe-s3-full.log` (P6 in the first round, same shape).

### 3.3 Earlier rounds (red copy, probe edits; kept for reference)

The first round's probes (`probes/probe-s3-*.log`, `probes/probe79.log`, `probes/red-*.log`) used the early-booked submit (85 ms). P5 (10 samples, end+1, both play) and P6 (10 samples, contiguous: Mock plays both, Module drops B) reproduce §3.2. The red copy's line 572 used `count < 32` (not the `<= 32` the brief asks for); its results are superseded by §3.1–3.2.

### 3.4 Regression under S3 Module + P1 Mock (`p1`)

| Suite | Result | Log |
|---|---|---|
| `ezsdr-mock-radio` (all binaries) | 8, 29, 27, 69, 1 passed, 0 failed | `probes/p1-mock-tests.log` |
| `ezsdr-acceptance` parity | 27 passed, 0 failed | `probes/p1-parity-tests.log` |
| `ezsdr-radio-uhd` lib | 336 passed, **8 failed** (§5 list) | `probes/p1-uhd-tests2.log` |
| `ezsdr-radio-uhd` tests/fake.rs | 336 passed | `probes/p1-uhd-tests2.log` |

The 8 failing lib tests: `ur_22_a_first_send_cut_short_keeps_the_reports_paired` (tx.rs:3867), `ur_23_a_first_send_waits_for_four_window_samples` (4726), `ur_23_a_four_sample_window_takes_a_first_send_of_four` (4840), `ur_23_a_held_tails_keep_each_channel_sample` (5208), `ur_23_a_wait_past_the_start_lead_moves_a_send_asap_first_send` (5015), `ur_23_a_first_send_at_its_lead_boundary_is_sent_by_drop_and_flag` (5044), `ur_23_a_first_send_at_its_lead_boundary_is_sent_by_send_asap_and_flag` (5058), `ur_23_a_first_send_one_tick_inside_its_lead_follows_send_asap_and_flag` (5052).

Not probed: the D2 device rule (not in `p1`); 2- and 3-sample bursts inside the window (the S1 tests cover 1 and 2); 33-sample bursts; short continuations (G12); the clock-cut close (no code yet, §4 Q7).

### 3.5 Probe rows that did not change between rounds

The first round's `probe79.log` (1-sample, early-booked) shows the same shape as §3.2: Mock plays both, UHD drops B late. Under S3 with P1 the shape still holds at 4 and 32 (§3.2); 1 and 10 were not rerun with P1.

---

## 4. Sequences and tests

Levels: **pure** (a function, no device), **Module** (`ezsdr-radio-uhd` tx.rs unit with the FakeDevice rig), **FakeDevice** (device.rs unit or tests/fake.rs integration), **Mock** (ezsdr-mock-radio tests, lib.rs unit tests included), **parity** (ezsdr-acceptance parity.rs, both Providers). Tuples are `(samples, sob, eob)` as the existing tx.rs tests read them.

Test names marked **(new)** are written in step 2; **(changed)** are existing tests whose expectation moves (§5); **(kept)** pass unchanged (§6). "Passes now" means it passes in `p1` (§3.4).

### Q1 — the first send of a burst waits for 32 samples of window space

| Test | Level | Asserts | Observed through | Wrong behaviour that fails it |
|---|---|---|---|---|
| `ur_23_a_first_send_waits_for_32_window_samples` (changed, from `…_four_…`, tx.rs:4717; fails now at 4726) | Module | for a 10 000-sample burst and for a 1 000-sample burst, the window fitting 28, then 31: no `tx_send`; fitting 32: one `(32, true, false)` | the rig's device send log | threshold 4 (HEAD hands 28 at once); 31 handed; threshold 33 (nothing at 32) |
| `ur_23_a_short_window_first_send_plays_clean_at_1_mhz` (new, tests/fake.rs) | FakeDevice (full Provider) | the 10 000-sample burst at 1 MS/s: no Underflow and no TimeError among the async reports, one BurstAck, transmitted samples equal the waveform | `tx_async` reports and `device.transmitted(…)` | HEAD: 28 open first, the D2 rule underflows, the burst is dropped |
| `ur_23_a_new_sob_after_an_abandon_waits_for_32_window_samples` (new, tx.rs) (P2) | Module | after a `FailSend` abandon, the next timed burst's SOB with the window fitting 31: no send; fitting 32: one `(32, true, …)` | the device send log | a lower threshold on the reopen path (no current test pins it) |

*Red evidence:* the fake.rs test must fail at HEAD + the D2 rule before the Module change; step 2 records the actual red. The probe's shape is the same (INFERRED from the DeviceFailure message).

Killers: N48S3-01 (547, 32→31), N48S3-02 (547, 32→33). The reopen test observes the same line and adds no row.

### Q2 — finite bursts of 1, 2, 3 samples

| Test | Level | Asserts | Wrong behaviour |
|---|---|---|---|
| `ur_22_a_short_first_send_is_padded_to_four` (kept, tx.rs:4686) | Module | one `(4, true, true)` send with payload then zeros | a 1–3 sample burst sent unpadded or as open |

Step 2 checks that it covers 1, 2 and 3 samples; if only one, the others go into the same test.

### Q3 — finite bursts of 4, 31, 32 samples, nothing booked after

| Test | Level | Asserts | Wrong behaviour |
|---|---|---|---|
| `ur_23_a_four_sample_first_send_is_not_held` (kept, tx.rs:4765) | Module | `(4, true, true)` | a held tail |
| `ur_23_a_burst_of_31_or_32_is_sent_whole_without_a_held_tail` (new, tx.rs) | Module | for 31 and for 32, separately: one `(n, true, true)`, nothing after | the last sample held (`hold` at 31 or 33); or split 30 + 1 |

Killer: N48S3-07 (hold 32→31).

### Q4 — finite burst of 33 samples

| Test | Level | Asserts | Wrong behaviour |
|---|---|---|---|
| `ur_23_a_33_sample_burst_sends_32_then_holds_one` (new, tx.rs) | Module | first send `(32, true, false)`; the held sample then goes as a continuation of 2 (payload then pad), `(2, false, true)`, pad zero (INFERRED from `hold`; step 2 confirms) | the whole 33 closed (`hold` at 33) |

Killer: N48S3-08 (hold 32→33).

### Q5 — bookings at, and one sample after, a closed short burst's end (inside the window)

**Module (tx.rs, new):**

| Test | Asserts | Wrong behaviour |
|---|---|---|
| `ur_23_a_burst_booked_at_a_closed_short_burst_s_end_is_late` (10 samples) | the burst closed; a burst booked at its exclusive end is decided by `drop_and_flag` against the padded front: one `TIME_ERROR` drop, its samples not transmitted | the Module continues it (no close term) |
| `ur_23_a_burst_booked_at_a_32_sample_burst_s_end_is_late` (32 samples; a 31-sample case in the same test) | the 32-sample burst closed (one `(32, true, true)`); a booking at its end is late by its policy; the 31-sample case likewise | the close bound at 31 (the 32-sample burst would continue) |
| `ur_23_a_burst_booked_one_after_a_closed_short_burst_s_end_is_on_time` (10 samples) | the burst at end+1 is on time and transmitted | a front one sample early (end, not end+1) |

Observed through the Module's late-drop event and the device send log.

**Parity (parity.rs, new), inside the window (received at 91 ms, as the S1 tests):**

| Test | Asserts | Wrong behaviour |
|---|---|---|
| `s3_parity_adjacent_after_a_closed_short_burst_inside_the_window` | for n in 4, 10, 31, 32, A→B: at end both Providers drop B (`late_drops` names B's target); at end+1 both play both bursts (`manifest_rows` count 2 on each). Probe-verified (§3.1) | Mock and Module disagree, or the Mock front is end, not end+1 |

Step 2 needs a length-parameterised waveform in the rig (the probe used an env var, which is not kept). The S1 1–2-sample cases stay with their tests.

**Renamed (parity.rs):** `s1_parity_short_burst_then_adjacent_burst_both_orders` (1557) becomes `s1_parity_short_burst_then_adjacent_burst_before_and_after_dispatch`. Its observation does not change. It does not cover receipt order (Sol P1).

**Not added until O1 is decided:** the B→A known-divergence test (O1 option (a)).

Killers: N48S3-09 (572 close bound 32→31: the 32-sample test; a 31-sample booking would still close at 31), N48S3-10 (572 short-case close term removed: the 10-sample test). The S1 parity test kills nothing new.

### Q6 — repeat of a short waveform

| Test | Level | Asserts | Wrong behaviour |
|---|---|---|---|
| `ur_22_a_repeat_of_one_to_three_samples_tiles_its_first_send` (changed, tx.rs:4730; **passes now**, asserting only `>= 4`, so it does not discriminate) | Module | repeats of 1, 3 and 5 samples with a window that fits exactly 32 and a chunk below 32: the first send is exactly `(32, true, …)` and its 32 payload samples are the waveform tiled | a first send below 32 (the tiled minimum at 4) |
| `mr_25_a_repeat_first_send_is_judged_at_32_samples` (changed, mock_channel.rs:590, rename; **passes now**) | Mock | window fits 31: not handed; fits 32: handed | judged at 4 |

The chunk setup matters: the widening at line 540 decides the count only when the chunk is below 32. `txp_rig(None, …)` has no window and cannot kill N48S3-11 (Sol P4); step 2 picks a window with the chunk below 32 and the window fitting 32.

Killers: N48S3-11 (540 tiled minimum 32→4, Module), N48S3-17 (Mock repeat threshold 32→4).

### Q7 — clock cut

| Test | Level | Asserts | Wrong behaviour |
|---|---|---|---|
| `ur_23_a_cut_before_four_first_samples_is_refused` (kept, tx.rs:4774) | Module | a cut leaving 3: refused `ClockEnded` (unchanged) | refusal raised to 32 would refuse 10 (see the next row) |
| `ur_23_a_clock_cut_leaving_n_closes_the_device_burst_with_that_send` (new, tx.rs) | Module | for n in 10, 31, 32: one `(n, true, true)` | refused (threshold 32 on the cut); or open `(n, true, false)` (no clock-cut close) |

**Code point (G10):** `send` has no close term for a clock cut (§2). The S3 rule needs one: `eob` must hold for a first send of at most 32 that reaches a clock cut. Step 2 adds it; it is not an existing term.

Killers: N48S3-12 (the refusal `cut - next < min_first` with 4→32, killed by n=10), N48S3-13 (the new clock-cut close term removed, killed by n=10).

### Q7b — preemption cut (a held booking inside a not-yet-handed long burst)

| Test | Level | Asserts | Wrong behaviour |
|---|---|---|---|
| `ur_23_a_preemption_cut_leaving_n_closes_the_device_burst` (new, tx.rs) (P2) | Module | a 1 000-sample burst A at s, not yet handed; a held booking B at s+n with n in 10, 31, 32 and the window fitting all: one `(n, true, true)`, then B meets the closed front and is late by its policy (one `TIME_ERROR` drop) | the cut left open (`count < 32` at 572, the Sol P2 case at 32); or continuing into B |

This path uses the existing close term at 572 (`ends_at_held`), so it needs no new code. It is kept separate from Q7 by design (P2).

Killer: N48S3-09 (the same term as Q5's 32-sample test; a second observer, not a second row).

### Q8 — continuation at its next sample

| Test | Level | Asserts | Wrong behaviour |
|---|---|---|---|
| `ur_23_a_one_sample_continuation_eob_is_padded_to_two` (kept, tx.rs:4701) | Module | a one-sample continuation closing the burst is padded to two | a minimum of 32 applied to a continuation (the S3 rule must not reach it) |

### Q9 — FakeDevice model (D2 and D3)

| Test | Level | Asserts | Wrong behaviour |
|---|---|---|---|
| `ur_33_a_short_open_send_at_10_mhz_is_clean` (new, device.rs) | FakeDevice | a 2-sample timed open first packet, continuation at a later tick, interpolation 20: no Underflow, no TimeError, one BurstAck | the rule without the ratio gate; the S1 drop applied to an open packet (D3) |
| `ur_33_the_open_short_rule_starts_at_interpolation_80` (new, device.rs) | FakeDevice | the same packet at interpolation 79: clean; at 80: one Underflow, then TimeError for the later timed packet | the rule at ≥81 or ≥79 |
| `ur_33_an_open_first_send_of_30_is_clean` (new, device.rs) | FakeDevice | a 30-sample open first packet, continuation at a later tick, interpolation 200: clean | the count below 30 |
| `ur_33_a_short_open_send_continued_at_the_same_tick_is_clean` (new, device.rs) | FakeDevice | a 28-sample open first packet at interpolation 200, continuation at the same device tick: no Underflow (the bench's no-gap case, 3 of 5 passed) | the gap clause removed, so the same-tick continuation underflows (the D2 residual risk is that a no-gap failure is unmodelled) |
| `ur_33_a_refused_sob_packet_does_not_arm_the_short_rule` (new, device.rs) | FakeDevice | a SOB packet refused as late does not arm the rule; a later packet at a later tick: no Underflow | arming before the refusal check (D2 trap) |
| `ur_33_a_new_burst_s_sob_clears_the_short_rule` (new, device.rs) | FakeDevice | a short open burst, then a new burst's SOB, then a non-SOB packet of the new burst at a later tick: no Underflow | the flag kept through a new SOB (D2 trap) |
| `ur_33_an_open_short_burst_reports_one_underflow_per_channel` (new, device.rs) | FakeDevice | one Underflow per channel per device burst, even when two later packets arrive | the second Underflow (the probe's artefact) |
| `ur_33_a_delayed_untimed_open_burst_underflows_with_no_ack` (new, device.rs) (P3) | FakeDevice | untimed open SOB of 2 samples, `set_time` advance, untimed EOB: the full report sequence is one Underflow per channel, one TimeError for the untimed continuation, no BurstAck, and the continuation's samples not in `device.transmitted` | a BurstAck for the dropped burst (INFERRED, G5); no Underflow (the rule not armed); two Underflows |

Killers: N48S3-14 (ratio gate 80→81), N48S3-15 (ratio gate removed), N48S3-16 (count 30→31), N48S3-22 (gap clause removed), N48S3-23 (arming before the refusal), N48S3-24 (new SOB does not clear), N48S3-25 (Underflow twice per burst).

**Closed packets (kept; D3).** `ur_33_an_undersized_untimed_first_packet_underflows_then_acks` (device.rs:2727) becomes a closed-packet control (SOB+EOB, untimed, 2 samples; the closed model is unchanged). `ur_33_an_undersized_timed_first_packet_is_dropped` (device.rs:2710) becomes a closed packet (SOB+EOB, 2 samples, timed). Its drop observation stays, so N48S1-13 keeps its killer. See §5.

### Q10 — MockRadio hand-over threshold and closed front (P1)

| Test | Level | Asserts | Wrong behaviour |
|---|---|---|---|
| `mr_25_a_first_send_that_does_not_fit_the_window_is_not_handed_over` (changed, mock_channel.rs:576; **passes now**) | Mock | a 40-sample burst: window fits 31: not handed; fits 32: handed | threshold 31 (or 4, the old value) |
| `mr_17_a_short_finite_burst_up_to_32_sets_its_closed_front` (new, mock_channel.rs, next to the existing front tests) | Mock | a 10-sample burst: front = start+11 (a booking at start+11 on time, at start+10 late); a 32-sample burst: front = start+33; a 4-sample burst: start+5; a 33-sample burst: ordinary frontier, no closed front | the old floor `len < 4` (N48S3-18); the 32 boundary `len < 32` (N48S3-19); the padded end replaced by start+5 (N48S3-20); handed end + 2 kept for a closed burst (N48S3-27) |
| `mr_17_a_one_sample_burst_uses_the_safe_sample` (kept, mock_channel.rs:605) | Mock | front = start+5 for 1 sample | a floor of 32 applied to 1 sample |
| `mr_17_a_finite_burst_front_stops_at_its_end` (kept, mock_radio.rs:1273; passes now) | Mock | unchanged | — |
| `mr_17_handed_front_reduction_cases` (kept, lib.rs:2413; passes now with P1) | Mock | unchanged; step 2 reads its frontier cases against P1 | — |

Killers: N48S3-18, N48S3-19, N48S3-20 (the 10- and 32-sample cases of the new test), N48S3-21 (MR-25 threshold 32→31: the 40-sample test), N48S3-27 (replacement removed: the 10-sample case; the parity end+1 case).

### Extra sequences from reading `send`, `step`, the window path and Sol's review

| Test | Level | Asserts | Why |
|---|---|---|---|
| `ur_23_a_31_sample_burst_waits_for_all_31` (new, tx.rs) | Module | a window fitting 30: no send; 31: `(31, true, true)` | the first send waits for `min(32, remaining)` (lines 540, 547) |
| `ur_23_a_first_send_is_widened_to_32_when_the_chunk_is_smaller` (new, tx.rs) | Module | a 1 000-sample burst, a window whose chunk is below 32 but which fits 32 samples: the first send is `(32, true, …)`, not the chunk | the widening at 540: this is the only test where it decides the count (Sol P4 on N48S3-03 and -05) |
| `ur_23_a_two_channel_first_send_waits_for_32_per_channel` (new, tx.rs) | Module | two channels, window fits 31 per channel: no send; 32: `(32, …)` with both channels, counted per channel | counts are per channel (ur_23_m12); a total-sample mutant would send at 16 |
| `ur_23_a_burst_at_the_first_safe_sample_after_a_held_tail_close_is_played` (check, tx.rs; UR-23 test list) | Module | a booking at the held-tail close's safe start (payload end + 2) is played | uses a held tail; if its burst is ≤32 samples it no longer holds; step 2 moves it to ≥33 keeping the observation |
| `ur_23_a_wait_past_the_start_lead_follows_the_late_policy` (kept, tx.rs:4987; passes now) | Module | a late first send is judged by its late policy before dispatch | — |
| `ur_23_a_wait_past_the_start_lead_moves_a_send_asap_first_send` (changed, tx.rs:5003; fails now at 5015) | Module | the moved first send opens a device burst (window retargeted to 32) | the 4-sample window |
| `ur_23_stop_while_the_first_send_waits_sends_nothing` (kept, tx.rs:4961) | Module | a stop while waiting for 32 sends nothing | the wait must not send early |

Sol's scenarios not already above: a long burst's first send is not reduced to 31 by the hold (covered by the 1 000-sample window steps 28, 31, 32 in the first Q1 test); a clock cut or preemption truncating an unhandled burst to 3, 10, 31 or 32 (Q7 and Q7b; 3 is refused); fake controls (immediate continuation, delayed continuation, interpolation ≤50, a fresh SOB after failure: Q9 and Q1); grouped channels (above).

**Closed ≥4 fake control.** Step 2 checks whether an existing FakeDevice test already pins "a closed 4-sample timed packet is not dropped"; if none does, it adds `ur_33_a_closed_four_sample_packet_is_not_dropped` (new, device.rs).

### #79 early-booked (the widened divergence; the case D1 names)

- **Sequence:** A of 4 or 32 samples at 100 ms, received at 85 ms (before the window reaches A); B at A's end or end+1, received in the same submit.
- **Result (VERIFIED, §3.2):** at end, the Mock plays both and the Module drops B late; at end+1 both play. So #79 is widened from 4 to 4–32 samples, as D1 rules.
- **Parity test:** none. The S1 parity tests book inside the window (submit at 91 ms), so they do not see it, and no in-window test can (G8). KL-32 documents it (§8).
- **Owner:** note 47 stage 5 (one receipt-time rule in `ezsdr_radio::timeline` on both sides; `plan/phase8/47-no-change-while-streaming.md`, whose stage text step 2 locates). Not fixed here.

### O1 — inside-window receipt order and the long contiguous case (pending the orchestrator)

- **B→A and the long contiguous booking:** §1 and §3.1. Their parity tests wait for O1. Under option (a), a known-divergence test pins each recorded outcome, so a fix turns it red.

---

## 5. Existing tests whose expectations change

Status is under `p1` (S3 Module + P1 Mock), §3.4.

| Test (path:line) | Status now | Why it changes | Change |
|---|---|---|---|
| `ur_23_a_first_send_waits_for_four_window_samples` (tx.rs:4717) | **fails** (4726) | the first send waits for 32 | window fits 32 (and the 28 and 31 steps); rename to `…_32_…` |
| `ur_22_a_repeat_of_one_to_three_samples_tiles_its_first_send` (tx.rs:4730) | passes, asserts `>= 4` only | a repeat's first send tiles to 32; the test does not discriminate | assert exactly 32 payload samples; add a 5-sample repeat; a window whose chunk is below 32 (Q6) |
| `ur_23_a_four_sample_window_takes_a_first_send_of_four` (tx.rs:4834) | **fails** (4840) | the window's first share is 32, not 4 | window end `2_004_000` → `2_032_000` (the rig's 1 000 ticks per sample from cursor 2 000, step 2 checks); expect the 32-sample first send |
| `first_send_at_its_lead` (tx.rs:5022) and its callers | the helper's callers fail: `…_at_its_lead_boundary_is_sent_by_drop_and_flag` (5044), `…_one_tick_inside_its_lead_follows_send_asap_and_flag` (5052), `…_at_its_lead_boundary_is_sent_by_send_asap_and_flag` (5058); `…_one_tick_inside_its_lead_follows_drop_and_flag` passes | same window (rig `2_004_000` → `2_032_000`); per-call expectations | same change; expectations per call |
| `ur_23_a_wait_past_the_start_lead_moves_a_send_asap_first_send` (tx.rs:5003) | **fails** (5015) | the moved first send needs 32 samples of window | window end → `2_032_000` |
| `ur_23_a_held_tails_keep_each_channel_sample` (tx.rs:5193) | **fails** (5208) | a 5-sample burst is now sent whole `(5, true, true)`; the held-tail observation needs ≥33 samples | burst length 5 → 33 (the held tail is what it checks) |
| `ur_22_a_first_send_cut_short_keeps_the_reports_paired` (tx.rs:3831) (P5) | **fails** (3867: B's timed send does not happen) | the window reaches only the 4 samples B's first send used to need; B now waits for 32 | advance the owner's clock (`core.ticks(31_006_000)` today) so the window fits 32 samples past B's first send; keep the A close ack, the report pairing and the device-late observation (`set_time(70 000 000)` and the ack asserts) |
| `mr_25_a_first_send_that_does_not_fit_the_window_is_not_handed_over` (mock_channel.rs:576) | passes; not discriminating | the window must fit 31 or 32 of a 40-sample burst, not 4 | window size and the 31/32 pair (Q10) |
| `mr_25_a_repeat_first_send_is_judged_at_four_samples` (mock_channel.rs:590) | passes; not discriminating | name and expectation: judged at 32 | rename; window 31 / 32 (Q6) |
| `s1_parity_short_burst_then_adjacent_burst_both_orders` (parity.rs:1557) | passes | the name claims receipt order; it covers before/after dispatch (P1) | rename only (§4 Q5) |
| `ur_33_an_undersized_timed_first_packet_is_dropped` (device.rs:2710) | not in `p1` (D2 absent) | it uses an open packet, which is now the rule's (D3) | closed packet (SOB+EOB, 2 samples); the drop observation stays; N48S1-13 keeps its test |
| `ur_33_an_undersized_untimed_first_packet_underflows_then_acks` (device.rs:2727) | not in `p1` | the Underflow now comes from a later tick (D2 variant B); and its two-send expectation (ack after a delay) contradicts the D2 model (P3) | closed-packet control (SOB+EOB, untimed, 2 samples): expectation unchanged; the delayed open-burst test is new (§4 Q9) |
| `ur_33_a_late_continuation_without_eob_advances_the_duc_before_radio_drops_the_next_packet` (device.rs:2530) and `ur_33_a_duc_timeline_survives_radio_underflow` (device.rs:2591) | not in `p1` | both failed under variant B in the earlier probe (`probes/red-B.log`); the probe emitted a second Underflow per burst (artefact) | step 2 checks; change only the Underflow count if that is the artefact |
| `hw_s1_short_first_sends` (crates/ezsdr-radio-uhd/tests/hardware.rs:1408) and the #70 four-sample control (~2824) | hardware-only | 4-sample expectations | step 2 reads them; compile-check with `rdev.py` (G6) |

**Expected to stay green, verified under `p1`:** `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements` (tests/fake.rs:617), `rejected_runs_are_admission_rejections_not_group_failures` (fake.rs:1141), `two_channel_transmit_throughput_runs_the_grouped_split_and_mixed_conditions` (fake.rs:6423), `the_duplex_pair_runs_the_two_measured_limits_together_once_and_decides_no_value` (fake.rs:6428). All of tests/fake.rs passes in `p1`. Those four fail under the D2 rule only if D2 is wrong for the Module's own short sends, and the Module no longer sends open shorts (INFERRED); step 2 re-checks them with the rule in.

---

## 6. Existing tests kept (checked against the new rule)

tx.rs: 2770 `ur_23_a_one_sample_burst_restarts_after_its_padded_device_burst`; 4686 `ur_22_a_short_first_send_is_padded_to_four`; 4701 `ur_23_a_one_sample_continuation_eob_is_padded_to_two`; 4746 `ur_23_a_short_first_send_closes_before_a_booking_at_its_end` (passes; step 2 checks it still covers 4 samples); 4765 `ur_23_a_four_sample_first_send_is_not_held`; 4774 `ur_23_a_cut_before_four_first_samples_is_refused`; 4903 `ur_22_a_first_send_is_never_undersized`; 4961 `ur_23_stop_while_the_first_send_waits_sends_nothing`; 4987 `ur_23_a_wait_past_the_start_lead_follows_the_late_policy` (passes now); 5063 `ur_23_a_long_burst_in_many_chunks_plays_contiguously`; 5135 `ur_23_m5_the_first_timed_buffer_takes_what_fits` (200 samples, window fits ≥32, unchanged `(200, true, false)`). All other tx.rs tests pass under `p1` except the eight in §5.

tx.rs 2223 `ur_21_send_does_not_cross_the_in_flight_window` (passes).

tests/fake.rs: all 336 pass under `p1`. 2842 `ur_23_a_burst_alone_is_ended_before_it_runs_dry` passes; step 2 re-checks that it does not depend on a first send of 4–31.

mock_channel.rs 605 `mr_17_a_one_sample_burst_uses_the_safe_sample` (passes); mock_radio.rs 1273 `mr_17_a_finite_burst_front_stops_at_its_end` (passes); lib.rs 2413 `mr_17_handed_front_reduction_cases` (passes under P1).

parity.rs 1437, 1461, 1486, 1511, 1535 (`s1_parity_*`): the 1–2 sample S1 shapes; all pass under `p1`.

---

## 7. Mutation rows

Rules: text edits only in `plan/maintenance/tools/mutations.json`; never `mutate.py`, `remote_mutate.py`, `gate.py` in step 1. Anchors are exact substrings for Rust token-insensitive matching (`anchor.py`); each `old` must match exactly once. The new rows' `old` text is the S3 code text, which exists only after step 2; each row is anchored then and its count checked.

### Re-anchored S1 rows (HEAD text checked: each matches once, by grep, unless noted)

| Row | HEAD `old` (file) | Change in S3 | Killer |
|---|---|---|---|
| P656 | `let device_eob = eob && (!ends_at_held \|\| device_sob && count < min_first) && !hold;` (tx.rs:572) | `min_first` → 32 (`count <= 32`) | `ur_23_a_burst_booked_at_a_closed_short_burst_s_end_is_late` (10 samples) |
| P1108, P1109, P1113, P1115, P1116 | `if avail < if device_sob { wanted.min(min_first) } else { wanted } {` (tx.rs:547; one match) | the same line with 32; each row's own `new` (the five rows share the line, as in `mutations.json`) | `ur_23_a_first_send_waits_for_32_window_samples` (and each row's own killer as before) |
| N48S1-12 | `let hold = eob && !ends_at_held && open_ended && !(device_sob && count <= min_first);` (tx.rs:570) | 32 | `ur_23_a_33_sample_burst_sends_32_then_holds_one` |
| N48S1-15 | the 547 line (one match) | 32 | `ur_23_a_first_send_waits_for_32_window_samples` |
| N48S1-25 | `held.len.min(DEVICE_MIN_FIRST_SEND)` (lib.rs:1377; one match) | `DEVICE_MIN_OPEN_FIRST_SEND` (32) | `mr_25_a_first_send_that_does_not_fit_the_window_is_not_handed_over` (40-sample burst) |
| N48S1-29 | `if held.repeat { DEVICE_MIN_FIRST_SEND } else { held.len.min(DEVICE_MIN_FIRST_SEND) }` (lib.rs:1377; one match) | 32 | `mr_25_a_repeat_first_send_is_judged_at_32_samples` |

Unchanged (the S3 text keeps the 4-constant there): **N48S1-07** (`MIN_FIRST_SEND.saturating_sub(sending)`, the closed pad), **N48S1-11** (`cut - next < min_first`, the refusal; its killer is the Q7 test with n=10 under N48S3-12), **N48S1-13** (`if undersized && packet.radio_time.is_some() {`, device.rs; one match; its killer's packet becomes closed under D3, the anchor still matches).

*Note:* the line 540 widening (`wanted = wanted.max(...)`) has no S1 row; N48S3-03, -04, -05, -11 cover it.

### New rows (N48S3-xx; each has one named killer)

| Row | Mutation (post-S3 text) | Killer |
|---|---|---|
| N48S3-01 | first-send threshold 32 → 31 (tx.rs:547) | `ur_23_a_first_send_waits_for_32_window_samples` (window fits 31) |
| N48S3-02 | 32 → 33 (tx.rs:547) | `ur_23_a_first_send_waits_for_32_window_samples` (window fits 32) |
| N48S3-03 | 32 → 31 for the open first-send minimum (tx.rs:540) | `ur_23_a_first_send_is_widened_to_32_when_the_chunk_is_smaller` (chunk below 32; a 31-sample waveform cannot kill it, Sol P4) |
| N48S3-04 | `min(len - offset)` removed on 540 | `ur_23_a_31_sample_burst_waits_for_all_31` (the widened minimum of 32 then exceeds the remaining 31, so no send; not equivalent) |
| N48S3-05 | `wanted.max(...)` removed on 540 | `ur_23_a_first_send_is_widened_to_32_when_the_chunk_is_smaller` (chunk below 32; the 1 MHz window scenario does not kill it, Sol P4) |
| N48S3-06 | two-channel count as a total (×channels) | `ur_23_a_two_channel_first_send_waits_for_32_per_channel` |
| N48S3-07 | hold threshold 32 → 31 (tx.rs:570) | `ur_23_a_burst_of_31_or_32_is_sent_whole_without_a_held_tail` (32 held) |
| N48S3-08 | hold threshold 32 → 33 (tx.rs:570) | `ur_23_a_33_sample_burst_sends_32_then_holds_one` |
| N48S3-09 | close bound 32 → 31 (tx.rs:572, `count <= 32`) | `ur_23_a_burst_booked_at_a_32_sample_burst_s_end_is_late` (a 31-sample booking would still close; the 32 does not) |
| N48S3-10 | short-case close term `device_sob && count <= 32` → `false` (tx.rs:572) | `ur_23_a_burst_booked_at_a_closed_short_burst_s_end_is_late` (10 samples) |
| N48S3-11 | tiled repeat minimum 32 → 4 (tx.rs:540, tiled branch) | the Q6 Module test with a chunk below 32 (step 2 sets the window) |
| N48S3-12 | refusal `min_first` 4 → 32 (tx.rs:531) | `ur_23_a_clock_cut_leaving_n_closes_the_device_burst_with_that_send` (n = 10 refused under the mutant) |
| N48S3-13 | the new clock-cut close term removed (exists only after step 2) | `ur_23_a_clock_cut_leaving_n_closes_the_device_burst_with_that_send` (n = 10 left open) |
| N48S3-14 | FakeDevice ratio gate 80 → 81 | `ur_33_the_open_short_rule_starts_at_interpolation_80` |
| N48S3-15 | FakeDevice ratio gate removed | `ur_33_a_short_open_send_at_10_mhz_is_clean` |
| N48S3-16 | FakeDevice open-short count 30 → 31 | `ur_33_an_open_first_send_of_30_is_clean` |
| N48S3-17 | Mock repeat threshold 32 → 4 (lib.rs:1377) | `mr_25_a_repeat_first_send_is_judged_at_32_samples` |
| N48S3-18 | Mock floor `len <= 32` → `len < 4` (lib.rs:295) | `mr_17_a_short_finite_burst_up_to_32_sets_its_closed_front` (10 samples) |
| N48S3-19 | floor `len <= 32` → `len < 32` (lib.rs:295) | `mr_17_a_short_finite_burst_up_to_32_sets_its_closed_front` (32 samples) |
| N48S3-20 | floor end `len.max(4) + 1` → `5` (lib.rs:295) | `mr_17_a_short_finite_burst_up_to_32_sets_its_closed_front` (10 samples) |
| N48S3-21 | Mock MR-25 threshold 32 → 31 (lib.rs:1377, non-repeat) | `mr_25_a_first_send_that_does_not_fit_the_window_is_not_handed_over` (40 samples) |
| N48S3-22 | FakeDevice gap clause removed (fires at the same tick) | `ur_33_a_short_open_send_continued_at_the_same_tick_is_clean` |
| N48S3-23 | FakeDevice arming before the refusal check | `ur_33_a_refused_sob_packet_does_not_arm_the_short_rule` |
| N48S3-24 | FakeDevice: a new SOB does not clear the flag | `ur_33_a_new_burst_s_sob_clears_the_short_rule` |
| N48S3-25 | FakeDevice: one Underflow per device burst becomes two | `ur_33_an_open_short_burst_reports_one_underflow_per_channel` |
| N48S3-27 | Mock: a closed short burst's handed end + 2 is counted again (`if repeat \|\| len > 32` → `if true`, lib.rs:~1367) | `mr_17_a_short_finite_burst_up_to_32_sets_its_closed_front` (10 samples: front s+12 under the mutant) |

**Removed:** N48S3-26 (close bound 32 → 31 at the preemption cut): the same term as N48S3-09, so it is a duplicate. The Q7b test is a second observer of N48S3-09, not a row.

**Equivalent rows:** none found. N48S3-04 is killed by the 31-sample test (it is not equivalent, because the widened minimum then exceeds the remaining 31). Step 2 confirms each count and kill.

Fix rounds run only the rows whose code or killing test changed (§7 of AGENTS.md); the full list runs once at the step's end, via `remote_mutate.py`, after step 2.

---

## 8. Spec amendments (the text, not the tests)

Each change is normative text plus its `*Test:*` line, with a Changes row dated 2026-11-06.

- **design/18 UR-21** (line 245–248): the test list gains the S3 tests named in §4 Q5 and Q7b; no rule text change. (O1 is not written into UR-21 until decided.)
- **design/18 UR-22** (line 249):
  - "A first send of a device burst carries at least `MIN_FIRST_SEND` = 4 samples per channel" → at least `MIN_OPEN_FIRST_SEND` = 32 for a first send that leaves the device burst open; a closed burst's first send of one to three samples is padded with zeros to four (unchanged);
  - "a first send of four or more needs no pad" → "of 32 or more, or the whole burst when it is shorter; a burst of at most 32 samples, or the part of it up to its end or a cut, is sent whole in one `START_OF_BURST`+`END_OF_BURST` send";
  - "A repeat's first send tiles its waveform to four payload samples" → 32;
  - the close rule: a booking at the exclusive end of such a closed short burst (≤ 32) is decided by its late policy against that burst's padded front plus one sample (the existing text covers only below four);
  - keep: "After a successful close, the next safe start is the closing send's exclusive end, padding included, plus one sample; the held-tail close is unchanged" (the held-tail close is padded to two: the safe start is the payload end + 2).
  - Test line (252): add the Q1, Q3, Q4, Q6 and Q5 test names; remove none.
- **design/18 UR-23** (line 254–255) (P6): add the exception: "A burst whose remaining part up to its end or up to a cut is at most 32 samples closes its device burst with one send (padded to four when shorter); a burst booked at its exclusive end is decided by its late policy against that closed front (UR-22)". Line 254's "the device burst continues" and line 255's "the payload goes without end-of-burst" both get "except a closed short burst (UR-22)". Line 255's "A first send of at most four samples is not held back" → 32.
- **design/18 UR-23** (line 259): "waits for four samples of window space (UR-22), or for its whole length when that is shorter" → 32.
- **design/18 UR-23** test line (262): add Q3, Q4, Q7, Q7b and the extra sequences; keep `ur_23_a_four_sample_first_send_is_not_held`.
- **design/18 UR-33** (line 329, the Transmit paragraph): the undersized rule as it stands (1–3 samples, timed, dropped through its EOB) is kept for closed packets. The open-short rule is added as FakeDevice's model: an open first packet of 1–29 samples at interpolation ≥80, with a later packet of the burst at a later device tick: one Underflow per channel, drop to EOB, one TimeError for each later timed packet and for the first later untimed packet. "No BurstAck" for a dropped burst (INFERRED, G5).
- **design/09 MR-17** (line 143): "A finite burst shorter than four samples that starts a device burst also sets the frontier … to at least its padded device end plus one sample" → finite bursts of at most 32 samples; for a closed one, the frontier contribution is its padded device end plus one sample, which replaces the handed end plus two. O1 adds nothing here until decided.
- **design/09 MR-17 test line** (144): add `mr_17_a_short_finite_burst_up_to_32_sets_its_closed_front`; `mr_17_a_one_sample_burst_uses_the_safe_sample` kept.
- **design/09 MR-25** (line 170): "a burst whose first send, four samples or the whole burst if shorter, does not fit inside the window" → 32.
- **design/09 MR-25 test line** (172): `mr_25_a_repeat_first_send_is_judged_at_four_samples` → `…_at_32_samples`; `mr_25_a_first_send_that_does_not_fit_the_window_is_not_handed_over` kept (changed in its body).
- **Changes rows:** design/18 (after line 720) and design/09 (after line 434), dated 2026-11-06, `note 48 S3`, the wording of each change and its D-decisions. `rules.py --base HEAD` must pass.
- **Code comment:** device.rs 636–637 (the `MIN_FIRST_SEND` doc) names the 1–2 MS/s evidence; update to name both constants and the bench's 30-sample rule.
- **KNOWN_LIMITATIONS.md:** a new entry **KL-32** after KL-31 (before "## Device behaviour the UHD Module already handles", line 283). Draft (wording to be fixed after O1):

  > ### KL-32 A burst booked just after a short burst can be played by one Provider and dropped by the other
  > **Symptom.** A finite burst of at most 32 samples, and a burst booked at its exclusive end or one sample later: when the earlier-booked burst was received before the transmit window reached it, MockRadio may play the later burst while the UHD Module drops it late. (O1 pending: the reverse receipt order inside the window, and a contiguous booking after a burst of more than 32 samples, if option (a) is chosen.)
  > **When.** The earlier-booked burst is received more than one in-flight window before its start.
  > **Workaround.** Book one burst, or start the next one a sample later.
  > **Evidence.** Design note 48 S3, probes `probes/p1-early-n{4,32}.log`.
  > **Status.** #79; note 47 stage 5 (one receipt-time rule in `ezsdr_radio::timeline` on both sides). Not fixed by S3.

- **`plan/maintenance/failure-mechanisms.md`** (#79 row): "a booking within four samples after a short held burst" → "within 32 samples after a short burst" (the S3 widening), plus one line for the O1 cases if option (a) is chosen.

---

## 9. Gaps and checks (not fixed; reported)

- **G1 (D1).** #79 is widened to 4–32 samples (§3.2) and carried by KL-32 and note 47 stage 5. Not fixed by S3 (ruling).
- **G2.** MockRadio has no `ClockEnded` refusal for a cut leaving 1–3 samples. S1 added it on the UHD side only (pre-existing; not S3's). Report, do not fix here.
- **G3 (D4).** 29 open samples at interpolation ≥80 are not measured; the model follows the brief's 30. 29 faults under the model.
- **G4 (D4).** 4 open samples followed by a delayed continuation at interpolation ≥80: no bench run with a gap (the 2026-11-05 "4+2 passed" runs sent the continuation at once). The model faults it when delayed; the Module never sends an open first send under 32 after S3, so only FakeDevice controls see it.
- **G4b (D4).** Closed 1–3-sample timed packets at 4–10 MS/s: not measured; S1's drop applies unchanged.
- **G5.** No BurstAck for a dropped open burst: INFERRED (S1's model), not measured.
- **G6.** The hardware tests (`hardware.rs` 1408, ~2824) are compile-check only; not run here.
- **G7.** `--features uhd` build and clippy: only through `rdev.py`; not run in step 1.
- **G8.** The early-booked case (§3.2, #79) has no parity test: an in-window test cannot show it. KL-32 documents it.
- **G9.** The probe copies are not the tree. `p0` and `p1` and the red copy are throwaway; nothing in them goes into the tree. The probe test code and its `PROBE_LEN` switch are in `probes/p1-probe-edits.diff`, not kept.
- **G10 (new).** `send` has no close term for a clock cut (§2, Q7). S3 needs one; the brief's bullet "a cut leaving 4–31 samples closes the device burst with that send" depends on it.
- **G11 (O1).** B→A receipt order and the contiguous booking after a long burst: pending the orchestrator (§1).
- **G12 (new, INFERRED, not probed).** A short continuation (1–3 samples, closed by its own EOB): the Mock's floor makes its front `start + 5` (HEAD and S3 alike). The Module closes the continuation at its own end, padded to two for one sample, so its safe start is `start + 3` for 1–2 samples and `start + 4` for 3. The Mock is later by 1–2 samples. This is pre-existing and outside S3's rule unless step 2 changes the continuation path.
- **G13.** Unequal channel lengths are unreachable: `provider/control.rs:386–403` validates complete interleaved frames and creates equal-length channel vectors (Sol). No test.
- **G14.** `ur_22_a_first_send_cut_short_keeps_the_reports_paired` and the other §5 retargets change the window only; each keeps its own observation.

---

## 10. Step 2 (after the go)

0. The orchestrator decides O1 (§1). Items 1–5 may start on the go; the B→A known-divergence test and KL-32's final wording wait for O1.
1. Write the new and changed tests in §4–§5 first (red at HEAD + rule where stated; the Q1 fake.rs test must be red before the Module change; the Q6 and Q5 Module tests must be red under the mutants they name).
2. Implement the S3 changes: `MIN_OPEN_FIRST_SEND` (device.rs, 32) and `DEVICE_MIN_OPEN_FIRST_SEND` (Mock, 32); the Module lines 540, 547, 570 and 572 at `<= 32`; the clock-cut close term (G10); the refusal kept at 4; the FakeDevice rule per D2 variant B with the two traps fixed; the Mock floor (lib.rs 294–295), threshold (1377) and the closed-front replacement (the `handed` merge at ~1367).
3. Specs (§8), Changes rows, KL-32, failure-mechanisms row.
4. Mutation rows (§7): anchored, each count = 1; the re-anchored S1 rows checked; the N48S3-13 and -27 rows only after the code exists.
5. `cargo fmt --all`; clippy `-D warnings` workspace and `-p ezsdr-radio-uhd --features uhd --all-targets` via `rdev.py`; workspace tests; `chmod go+r`; `rules.py --base HEAD` → 0.
6. Report to the orchestrator; the gate is the orchestrator's.

---

## 11. Rulings after the Sol review and the O1 and re-check rulings (carried into step 2, 2026-11-06)

This section overrides §§1–10 where they differ. Sources: `rulings-sol-plan.md` (P1–P6), `rulings-o1.md` (O1), `rulings-recheck.md` (N1–N8, the P-small fixes, the simplification).

- **O1 (ruled):** no known-divergence tests in the tree. The B→A receipt order inside the window and the contiguous booking after a long held burst (§1 O1, §3.1) go to #79 as a comment, with the probe outputs; the probe source is kept with the step records. KL-32 covers only the hardware limit D1 rules (a burst booked at the next sample after a burst of at most 32 samples is decided late; workaround: one burst, or start one sample later), and may cite #79 under evidence. The S3 parity tests cover A→B inside the window at 4, 10, 31, 32 (end and end+1), and the Stop case (N1).
- **Simplification (recheck P4, replaces the first-send widening):** the widening (`wanted = wanted.max(...)` under `device_sob`) is removed. The chunk gets a lower bound of `MIN_OPEN_FIRST_SEND` (`clamp(MIN_OPEN_FIRST_SEND, TX_SEND_CAP)`). With the 10 ms window the chunk is under 32 only below 12.8 kS/s, so the widening decided nothing at X3x0 rates; no row covers the lower bound. **Rows deleted:** N48S3-03, N48S3-05, N48S3-06 (N4), N48S3-11. Q6 asserts the repeat's first send carries at least 32 payload samples. `ur_23_a_first_send_is_widened_to_32_when_the_chunk_is_smaller` (§4 Extra) is not written.
- **Close at a cut (recheck P2 small fix):** the close for a clock cut and for a held cut use one expression over `cut = min(held_cut, clock_cut)`: `short_first = device_sob && count <= 32` (one copy of the 32 boundary) and `closes_at_cut = short_first && cut == Some(next + count)`. §9 G10 reads "4–32" (a cut leaving 4–32 samples closes; a cut leaving 3 is refused).
- **N1 (major, adopted):** both Mock Stop sites use a closed short burst's floor in place of `+2`: `stop_tx` (`lib.rs` ~1137, `remember_tx_front(cut + 2)`: the floor replaces it when it is `cut + 1`) and the closing block of `emit_tx_until` (~1765, `stop + 2`, for a block that ends a closed short burst at its waveform end). Tests: A of 10 and of 32 inside the window, Stop, then B at A's end (both drop) and end+1 (both play), in Mock and parity. A failing probe first.
- **N2:** `ur_33_a_late_continuation_without_eob_advances_the_duc_before_radio_drops_the_next_packet` (device.rs:2530) and `ur_33_a_duc_timeline_survives_radio_underflow` (device.rs:2591) take a 32-sample first packet (they test the DUC timeline, not the short rule). `ur_33_a_four_plus_two_split_is_clean` (device.rs:2744) is the kept same-tick control.
- **N3:** the Mock's handed-front skip is derived from `short_burst_floor(..).is_none()` (one copy of the boundary).
- **N5:** add `ur_33_an_open_first_packet_of_29_samples_faults` and the row 30 → 29 (killer: the 29 test).
- **N6:** Q1's red at HEAD is forced by a fixed manual clock and window (`txp_rig(Some(…), …)` as in `ur_23_a_first_send_waits_for_four_window_samples`), recorded in the probe log, not as a test claim.
- **N7:** Q5's Module tests book B before A's first send (before the window reaches A), so both are decided in the same rig.
- **N8:** `ur_22_a_first_send_is_never_undersized` (tx.rs:4903) asserts `eob || n >= 32` for every SOB send. D2's Underflow: one report per channel, once per device burst; the short rule and the generic late-continuation Underflow never both fire for one burst (the rule's fault replaces the generic dry Underflow for that burst).
- **P3 (timed shape, recheck):** SOB(2 samples, open, timed), a middle packet and an EOB, each at a later device tick, at interpolation ≥ 80: one Underflow per channel, then one TimeError per later timed packet; no BurstAck; the continuation's samples are not in `device.transmitted`. Added as `ur_33_a_delayed_timed_open_burst_underflows_with_no_ack`.
- **D2 model state (implementation choice, recorded):** the FakeDevice's `Radio` keeps one burst state (`Clean`, `Armed(tick)`, `Timed`, `Silent`) in place of the bool `dropped`. `Armed` is set only by an accepted open SOB (after its late refusal) of 1–29 samples at interpolation ≥ 80; the next non-SOB packet clears it and faults only at a later tick. A new SOB clears every state. The generic dry Underflow (`active && now > cursor`) does not fire for a faulted burst (`active` is false).
- **Closed-packet controls:** `ur_33_an_undersized_timed_first_packet_is_dropped` and `ur_33_an_undersized_untimed_first_packet_underflows_then_acks` (device.rs:2710, 2727) become closed packets (SOB+EOB); the undersized rule (1–3 samples) applies to closed packets only (`packet.sob && packet.eob`).
