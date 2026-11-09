# Step B test plan: the front after a closed short device burst is its transmitted end + 10


## Orchestrator amendment (revision 3a, 2026-11-09) — binding, overrides the sections below where they differ

From the re-check of revision 3 (`recheck-plan-rev3.md`, FAIL with two majors; both fixes adopted as written):

1. **tx.rs:6307 moves to B2.** `values_turn` reads `previous_end = self.chain.front()` (tx.rs:565), the owner's front, which only `advance_front` moves (`+1` at :800 until B2). In B1 the test stays unchanged and green; its new value (`applied[0] >= instant(21_014)`) and the longer turns go in B2 with the owner's `+10`. It stays N47S4-09a's killer. The §4 line that calls N47S4-09a "unchanged … checked at B2" and the line that calls it "re-designed" are reconciled: re-designed in B2.
2. **N-P5 (i) and N-P6 are pinned per Provider, not by one cross-Provider Stop.** The UHD parity rig's device clock runs up to about 1.2 ms (≈ 1 200 samples at 1 MS/s) ahead of the injected tick (parity.rs:2257), wider than Q1's 10-sample range, so one relative Stop instant cannot hit the window-edge geometry on both Providers. Follow parity.rs:2257-2258 ("the boundary is pinned per Provider"):
   - Mock: the rewritten `mr_25` gains one row per late policy.
   - UHD: the manual-clock control-path rig in the shape of tx.rs:3135 — A at 9_999, `stop_now` at device 0, then D booked: DropAndFlag at 10_004 refused; SendAsapAndFlag at 10_004 moved to 10_013 with `TIME_ERROR{send_asap}`; untimed placed at 10_013; a booking at 10_013 admitted.
   - The parity comparison is the equality of those numbers between the two tests (both Providers give the same relative answers), not one shared Stop.
3. **mock_channel.rs:535:** B and D must not share a start (the Mock's `held` is a `BTreeMap` keyed by start, lib.rs:208/:1761): move B to 2_010_020 (or name the "cancelled by stop" rejected row as N47S5-23's killer explicitly), and update the receive-loopback sample-position assertions (10_044–10_050) to the new geometry.
4. **N-S5e** is built through `remove_head` (a short head of ≤ 32 cannot take a join, staging.rs:244): `accept(100,40)`, `accept(140,10)`, `accept(150,4)`, `drop_piece(100,40)` → chain 140..154, front 164; `halt(141,146)`: 145 Err, 146 Ok. N49B-12's mutant keeps front 164, so 146 is Err.
5. **UR-33 (iv)** generalised: below interpolation 80 (`OPEN_FAULT_MIN_RATIO`, device.rs:664) every abandoned burst of 1–30 accepted + two zeros (3–32 samples) falls under the +10 guard; the model stays stricter than the owner's cursor + 3.
6. §0's cite "the tests' `stop_at(…, 0)`, parity.rs:269" is wrong (that is the Mock harness's Stop helper); the only window-0 halt is tx.rs:997.
7. (re-check of 3a, minor) In item 2, each late-policy row (DropAndFlag, SendAsapAndFlag, untimed, the admitted booking at the front) runs on its own rig — rows booked on one rig interfere (a moved or placed booking occupies 10_013). The same for the Mock's per-policy rows.
8. (re-check of 3a, minor) Item 6 reads: tx.rs:997 is the only fixed window-0 halt call in production code; the Mock's `ideal` profile also runs every Stop at window 0 (channels.rs:352).

Status: test plan only (AGENTS §9 step 2), revision 3. Revision 3 answers the Opus re-check of revision 2 (`recheck-plan-rev2.md`, FAIL, all items adopted) and the NQ-A / NQ-B rulings (`rulings-sol-plan.md`). Revision 2 answered Sol's PLAN NEEDS CHANGES (`sol-plan.md`). No repository file was edited, built or run. Sub-steps (owner, 2026-11-09): **B1** the record's rule (`ezsdr_radio::staging`, MockRadio, parity and iii tests, RM-15, RM-16, KL-36 text); **B2** the device side (uhd-tx's front, FakeDevice, hardware tests, UR-22/UR-23/UR-33, KL-13, KL-38, KL:308). Each sub-step is one local commit on `work/stepB`; each must leave the tree green.

Verdict labels follow the brief: VERIFIED = measured on #2 UBX slot A (UHD only, 2026-11-09); INFERRED = not measured.

## Revision 3 (2026-11-09): where each re-check item is closed

| Item | Closed in |
|---|---|
| **M1 (P2)** Q1's region extension is reachable at production windows: a standalone short head at the window edge (start in `(H − 11, H)`, the mr_25 geometry). §0 window arithmetic and the §2 note corrected (the earlier "invisible" wording is removed); N-P6 expects `max(stop, own front)`; NQ-A's reachable geometry is this window-edge dropped head, in N-P5 (i) and N-P6, on both Providers. | §0, §2 note, §3.3 N-P5 (i) and N-P6, §6 Q1 and Q4 |
| **M2 (P2)** Missed red tests added with re-designed fixtures, each keeping its original sequence and becoming the sole killer of its row: staging 563 (N47S5-55), staging 666 (N47S5-84), mock_channel 535 (N47S5-23), mock_radio 1264 (value only), tx 6307 (N47S4-09a). Hull test 545: second chain at 70 (front 85), probes 40, 70, 85. The §4 "still killer" claim for N47S5-55 is withdrawn. | §2 red table, §3.1, §3.2, §4 |
| **F5 (P2, re-opened)** N-P5 rewritten after the NQ-A ruling: no "c + 13 admitted" at a non-zero window; the geometry is the window edge (i) and the kept-head agreement (ii). | §3.3 N-P5 |
| **Q1 predicate** decided: a dropped standalone short head extends the region when `start < H`, `H = stop − 3` (the handover horizon). Reason in §6 Q1. | §0 (b), §6 Q1 |
| **m1** S1 inversion: `ab = k <= 13` (B refused when A came first; refused iff k ≤ 13, admitted at 14). | §2 red table (parity 1804) |
| **m2** RM-15 `design/07-radio-model.md:165` text. KL:308 text. | §3.5 (RM-15), §3.9 (KL:308) |
| **m3** halt's boundary and joined-term mutations: N-S5d (33-sample kept head, the length bound) and N-S5e (kept chain with a joined piece); rows N49B-11 and N49B-12. | §3.1, §4 |
| **m4** `cargo test --workspace` added to B1's commands. | §7 |
| **m5** N-H1 includes L = 1 (padded to 4). | §3.8 |
| **(b) abandon exception** (the recheck's spot check): an open first send that is partly accepted and abandoned with its two-zero close can leave a device burst of exactly 32 transmitted samples (30 accepted + 2 zeros). The owner's front is cursor + 3 and the FakeDevice guard is +10. Stated in UR-33 (iv), pinned by N-F8e; the model is stricter. | §0 (owner), §3.6, §3.7 N-F8e, §3.9 |
| **NQ-A ruling** (no production hook): the window-edge geometry uses the existing test helpers only (`stop_at`, `stop_both`). | §3.3, §6 |
| **NQ-B ruling** (yes): B1 is a local commit on `work/stepB`; main receives one squashed commit after B2. | §2 point 6, §6 |

## Revision 2 (2026-11-09): where each Sol finding is closed

| Finding | Closed in |
|---|---|
| **F1 (P1)** Q2 rejected. A standalone closed short head that a Stop keeps (start < t) keeps its own front (end + 10). Counterexample: A start 100, len 32, Stop t 101 / stop 104, B at 104 must be refused, A's front is 142. N-S2 dropped. N-S5a limited to len 1–3 with t inside the payload. The two-zero close (cursor + 3) is for an open device burst only. | §0 (rule), §1 row 5, §3.1 (N-S5a, N-S5c new; N-S2 dropped), §4 (N49B-03, N49B-04, N47S5-14, -62), §6 Q1, Q2 |
| **F2 (P2)** FakeDevice: a refused open timed SOB refuses its whole device burst through its EOB (timed continuations TimeError, nothing transmitted, no BurstAck; first untimed continuation TimeError then Silent; next normal SOB recovers). Length counts the device burst across packets: 30 + 2 = 32 (guard applies), 32 + 2 = 34 (does not). | §0 (facts), §1 row 8, §3.7 (rewritten: the two fields, the guard, N-F8a–d), §4 (N49B-06 to -09) |
| **F3 (P2)** B1 red list completed: parity `s15_parity_untimed_queue_behind_a_booking` (11 → 20); staging `rm_16_repeat_occupies_until_a_stop` (fixture); tx `ur_23_a_one_sample_burst_restarts…` receipt part and tx `ur_21_s1_overlap_refused_at_receipt` (B1); tx `ur_23_stop_does_not_send_unhanded_window_edge_payload` keeps its sequence (B moves to an admitted target, not a refusal); staging hull test 545 and placement tests 922, 931 (Q1); iii Stop test (Q1). The B1 commands run the library tests and the iii Stop test by a filter that runs it. | §2 (red table rewritten), §3.1, §3.2, §3.3, §3.4, §3.6, §7 |
| **F4 (P2)** Configure-refusal exception stated in the B1 safety argument (point 6): pieces already accepted as joins are not re-decided at receipt after `remove_head`. tx `ur_21_s2_joined_pieces_left_behind_a_refused_head_keep_their_regions` (3022) moves to B2 with the expected outcome of each piece. C1 (re-segmenting) is out of scope. | §2 (point 6), §1 row 11, §3.6 |
| **F5 (P2)** N-P1 uses `s + max(n, 4) + k`. N-P5 is required at a non-zero window on both Providers, compared against each Provider's own Stop cursor. The dropped-head policies are separated (DropAndFlag refused; SendAsapAndFlag moved with TIME_ERROR{send_asap}; untimed placed after): new N-P6. Parity pins the target, the actual start and the presence of TIME_ERROR, not only row counts. | §1 rows 1, 5, §3.3, §2 note on windows, §6 Q4, NQ-A |
| **F6 (P2)** Raw gap rows keep the 100-sample closed B as a labelled short-B observation. A long-B regression with split timed sends is added (N-H1). R5-m5's gap argument at `hardware.rs:3787` changes 0 → 10. | §3.8, §4, §6 Q7 |
| **F7 (P2)** Normative and workaround text: UR-23 (`design/18-uhd-radio.md:260`), UR-22 safe-start sentence (`:253`), UR-33 (`:342`, `:351`: the guard, the refusal through EOB, the conservative short-B model, no claim that the hardware refuses a short B at gap 9), KL-38 workaround (`KNOWN_LIMITATIONS.md:151`), KL-13 workaround (`:130`) and status (`:132`, conditional reference to KL-36). The RM-15 region sentence the first plan put under UR-22 is in `design/07-radio-model.md:162`. | §3.5, §3.9 |
| **F8 (P3)** No owner length field. The owner identifies a short closed device burst from its framing: the closing send is also the device burst's first send (SOB and EOB in one send) and its transmitted length (`device_count`, `tx.rs:686`) is at most 32. Justification: an open first send is at least 32 samples and a continuation at least 2, so a burst of at most 32 is one send. N-O7b (32 open + 1-sample continuation keeps + 1) pins the case the SOB flag excludes. | §0, §3.6, §4 (N49B-05, N49B-10), §6 Q5 |

Mutation notes adopted: N47S5-14's mutant keeps the coverage of `c.join = false` (its killer is a 40-sample head, unaffected by F1); N47S5-09 and -67 keep their killers, whose setups change with Q1 (re-anchored where their text changes); N48S1-08 and N47S4-09b: killer clock advances to end + 10 (`tx.rs:6338` stops at 20 006 today).

Counts and commands are in §5 and §7; open questions in §6 (NQ-A and NQ-B are new and need a ruling).

## 0. The rule, as the plan implements it

- **Closed short burst** (device): a device burst that is one send carrying start-of-burst and end-of-burst, whose transmitted samples (padding included, `device_count = sending + pad`, `tx.rs:686`; FakeDevice: the packet's `n`) are 1–32 per channel. Record region `[start, transmitted end + 10)`, transmitted end = `start + max(n, 4)` (1–3 samples are padded to 4, UR-22). One value: 10.
- **Record (RM-15, `front()`):** a standalone head of at most 32 samples (no joined pieces) has front `start + max(n, 4) + 10`. A head longer than 32 or a joined piece: `end + 2` (held tail, unchanged). A repeat never ends.
- **Stop (RM-16, `halt(t, stop)`, `staging.rs:346–368`):**
  - (a) A kept chain (start < t) that is a standalone closed short head keeps its own front. It was handed as one closed send, and RM-16 does not recall handed samples. Predicate: no joined pieces and `end <= start + 32` (finite). Other kept chains are capped at `stop` as today (joined pieces, long heads, repeats). The two-zero close at `cursor + 3` belongs to an open device burst only.
  - (b) A dropped chain (start in [t, stop)): if it is a standalone closed short head that may have been handed (start < H, the handover horizon `H = stop − 3`), the region reaches its own front: `max(stop, own front)` (Q1). Other dropped chains add the region to `stop` as today. A dropped chain with start ≥ stop adds nothing (unchanged). A dropped chain with `H ≤ start < stop` was never handed and extends nothing beyond `stop`.
- **Owner (uhd-tx), F8:** a closing send that is also the device burst's first send (`device_sob`) with `device_count <= 32` gets `open.next + pad + 10` at `tx.rs:800`; every other close keeps `+ 1` (Q5). No new state. The abandon path is the one exception (§3.6).

Facts the plan relies on (read in the tree):

- `crates/ezsdr-radio/src/staging.rs:586–592` `front()`: `Some(len) if joined || len > 32 => … checked_add(2)`; `Some(len) => start.checked_add(len.max(4))?.checked_add(1)` (the 1–32 arm; 1–3 and 4–32 share it).
- `staging.rs:346–368` `halt`: `c.front = c.front.min(stop); c.join = false;` (`:351–352`) for every kept chain; the dropped region `(t, stop)` or the hull with an existing region (`:361–365`). `stop = stop_sample(at, in_flight, …) = at_or_after(at + in_flight) + 3`.
- `staging.rs:498–530` `remove_head`: a surviving head ≤ 32 without pieces is re-fronted with `front(c.start, Some(own), false)` (`:519`); pieces via `front(s, Some(e - s), false)` (`:525–526`). `remove_head` makes no receipt decision for pieces already joined.
- `crates/ezsdr-radio-uhd/src/provider/tx.rs:800` `self.chain.advance_front(clock.domain, open.next.saturating_add(pad as i64 + 1))`: the only `pad as i64 + 1` in the tree; applies to any `device_eob`. `tx.rs:685` `pad`; `tx.rs:686` `device_count`; `tx.rs:552` `tx_close_past_record_front` (device front > record front); `past_front` helper at `tx.rs:4695–4696`.
- `crates/ezsdr-radio-uhd/src/device.rs`: `struct Radio` `:633` (`cursor`, `active`, `burst`, `reports`); `Burst` enum (`Clean`, `Armed(i64)`, `Timed`, `Silent`) ~`:639–650`; constants `:655–664`; `Radio::packet` `:667–736`: `if packet.sob { self.burst = Burst::Clean; }` (`:669`), `undersized` (`:673`), the idle timed refusal `if !self.active && packet.radio_time.is_some_and(|t| t < now || t <= self.cursor)` (`:689`, which returns before the cursor moves), the accepted EOB and BurstAck at `cursor + 1` (`:722–727`); `faulted` `:740–766` (called for non-SOB packets only). The DUC (`TxStreamer`, `:1805–1808`) advances before `radio.packet`, even for refused packets.
- Hardware (VERIFIED, #2 UBX slot A, UHD only, 1 and 2 MS/s, 2–4 trials per cell): after a closed burst A of L transmitted samples, a 10 000-sample timed B at gap g plays only when L 4–8: g ≥ 10; L 10–14: g ≥ 7; L 16: g ≥ 5; L 16–30: g ≥ 5; L 31–32: g ≥ 1. A short B (100 samples, one closed send) plays at gap 1 after any short A. The model is stricter than the hardware for L 16–32 and for a short B (owner choice, Q6). Failing trials: Underflow and/or TimeError 100–340 on B.
- Window arithmetic (INFERRED; rechecked against the tree): the kept-head cap of §0 (a) bites only when a kept head's own front exceeds `stop` (own front at most `t + 41`, `stop = t + W + 3`), i.e. only when the window `W` is below about 38–41 samples. The x3xx in-flight window is 10 ms (`crates/ezsdr-radio/src/x3xx.rs:386`), 10 000 samples at 1 MS/s, so the cap cannot bite at a production window; it is reachable only at window 0 (the tests' `stop_at(…, 0)`, `crates/ezsdr-acceptance/tests/parity.rs:269`, and `crates/ezsdr-radio-uhd/src/provider/tx.rs:997`). The dropped-path extension (b) IS reachable at the window edge: a standalone short head with start in `(H − 11, H)` has own front `start + 14 > stop`, and it may have been handed (geometry: §3.3 N-P5 (i) and N-P6). The 29 ms in-flight value (`crates/ezsdr-radio-uhd/src/provider/mod.rs:885`) is noted and not used here.

## 1. Sequences and where each is pinned

| # | Sequence (brief) | Sub-step | Pinned by (new or changed) | Green/red today |
|---|---|---|---|---|
| 1 | Head n ∈ {1,2,3} at s; B at s + 4 + k: k 0–9 refused at receipt, k = 10 admitted and on time | B1 (record, both Providers); B2 (owner dispatches k = 10 on time) | staging `rm_15_1_to_3_…` (§3.1); mock `mr_17_a_one_sample_burst_uses_the_safe_sample` (§3.2); parity `s1_parity_short_burst_front_s_s3_s5` (§3.3) and new N-P1 | red |
| 2 | Head n ∈ {4, 10, 31, 32} at s; B at s + max(n,4) + k: k 0–9 refused, k = 10 admitted | B1 | staging `rm_15_closed_burst_front_is_end_plus_ten` (§3.1); parity `receipt_order_cases` S2 rows and `s3_parity_booking_after_a_closed_short_burst_follows_d5` (§3.3); N-P1 | red |
| 3 | Open device burst (33+, held tail): e + 1 refused, e + 2 admitted; join at e before J(e) is one device burst | unchanged | staging `rm_15_held_tail_front_is_end_plus_two` (360), `rm_15_contiguous_booking_joins_a_long_head` (373), `rm_15_short_burst_joins_a_long_chain_with_the_chain_front` (382); parity `s3_parity_booking_after_a_device_burst_s_short_tail_follows_d5` (1713) and the `s4`/`s4c` rows of `receipt_order_cases`: green | green |
| 4 | Untimed booking after a short head is placed at its transmitted end + 10, no TIME_ERROR | B1 (placement); B2 (owner's dispatch of the untimed burst on time) | staging 862, 871, 884, 914 (values, §3.1); new N-S4; parity `s15_parity_untimed_queue_behind_a_booking` (2173, 11 → 20, §3.3) | red |
| 5 | #82, Stop at t, cursor c. (a) kept: a 1–3-sample head at c − 1 (transmitted end c + 3, front c + 13): booking at c + 3…c + 12 refused, c + 13 admitted at window 0 (at a window edge, N-P5 (ii) checks agreement only). (b) dropped: a Stop at t = c − 1 cuts the head: region reaches its own front. (c) Sol's counterexample: A start 100 len 32, Stop t 101 / stop 104: B at 104 refused, 142 admitted | B1 (record); B2 (hardware R5-m5) | staging N-S5a (len 1–3), N-S5b (dropped, Q1), N-S5c (A(100,32)); iii `a_booking_after_a_head_the_stop_cuts_waits_for_its_front` (1281, renamed, §3.4); mock N-M5 (window 0); parity N-P5 (window edge: dropped region and kept-head agreement) and N-P6 (dropped, policies) | red |
| 6 | A Stop cutting a long repeat (or long head) at c: two-zero close, booking at c + 3 admitted; the long head's cap at `stop` | unchanged | staging `rm_16_repeat_occupies_until_a_stop` (585, fixture changed only), `rm_16_stop_region_refuses_the_span_the_stop_cut` (527), `rm_16_a_stop_lowers_a_finite_front_to_its_bound` (721, 40-sample head); mock `mr_25_s7_stop_tail_refuses_until_bound`; tx `ur_23_stop_closes_a_running_repeat_at_its_handed_cursor` | green (585 red only by its fixture, §2) |
| 7 | UHD owner's front after a closed short burst = record front (end + 10); `tx_close_past_record_front` stays 0 | B2 | tx `ur_22_a_short_first_send_is_padded_to_four` (5134), `ur_22_a_two_channel_one_sample_burst_is_padded_on_both_channels` (5364), `ur_23_a_one_sample_burst_restarts_after_its_padded_device_burst` (3238, B2 part); new N-O7 | red in B2 |
| 8 | FakeDevice: a timed SOB of a new device burst less than 10 samples after the exclusive end of a closed short burst (≤ 32 transmitted) is refused; a refused open SOB refuses its whole device burst through its EOB; at end + 10 it plays; a closed burst over 32 keeps the gap of one | B2 | new device.rs unit tests N-F8a, N-F8b, N-F8c, N-F8d (§3.7) | new |
| 9 | Parity: both Providers give the same answers for sequences 1, 2, 4, 5 | B1 | §3.3 parity tests | red (changed) and new |
| 10 | Hardware tests compute the front from the record; expected values and comments follow the rule; the next bench checks end + 10 and books R5-m5 at c + 13; the long-B split-send regression | B2 | hardware 3669, 3684 (comments), 3708–3735 (gaps, comment at 3711), 3756–3795 (R5-m5 gap 0 → 10, booking c + 3 → c + 13), new N-H1; `iii::head_front` (iii 1268, B1) | compile-only here (`#[ignore]`) |
| 11 | Configure refusal (tx 3022): A(40) refused; B, C, D were accepted as joins before the refusal. After removal B is a standalone closed short head: C and D are no longer decided at receipt and are decided by their late policy at dispatch | B2 (device outcome); B1 keeps today's device behaviour (§2 point 6) | tx `ur_21_s2_joined_pieces_left_behind_a_refused_head_keep_their_regions` (3022, §3.6) | green in B1, changed in B2 |
| 12 | Owner close of a device burst whose closing send is a continuation (32 open + 1-sample continuation) keeps `+ 1`, even though its transmitted total is ≤ 32 per send | B2 | new tx N-O7b (§3.6); mutation N49B-10 | new |

## 2. Why B1 alone stays green, and which tests go red in it

**Argument.** After B1 the record's front for a closed short burst is `end + 10`. uhd-tx still closes at `end + 1` (`tx.rs:800`), and FakeDevice still accepts a timed SOB from `end + 1`.

1. Every booking uhd-tx dispatches was admitted by the record. A booking admitted by the record starts at or after the record front of every earlier chain it does not join (`accept`, `place`, `region_move`, `move_start` test regions, not the owner's front). So every send uhd-tx makes for a short burst starts at or after `end + 10`, which is at or beyond the owner's own `end + 1`. Outside the configure-refusal case (point 6), the device never receives a burst in `[end + 1, end + 10)` in B1.
2. The owner's late judgement at dispatch compares a target with its own front (`end + 1`). That front is below the record's, so B1 cannot turn a burst late that was not late before. It can only leave the late check looser than the record.
3. `tx_close_past_record_front` (`tx.rs:552`) counts owner closes whose device front is above the record front. In B1 the owner's front is below the record's, so the stat stays 0. Its test `ur_21_stat_counts_owner_closes_past_the_record_front` (`tx.rs:3060`, 40 samples) is unaffected.
4. Owner-only tests (those that drive `txp_give` without the record, e.g. `tx.rs:5289` and `:5364`) assert the owner's own front and dispatch judgement. They stay green in B1.
5. FakeDevice is unchanged in B1, so no device test can become looser.
6. **Configure-refusal exception (F4).** A joined piece accepted before a configure refusal is not re-decided at receipt after `remove_head` (§0, `staging.rs:498–530`). Its device path is therefore not protected by the record's new front in B1. Example (`tx.rs:3022`): A(40), B(10), C(4), D(10) are accepted as one chain; A's configure is refused; B becomes a standalone closed head with front `end + 10`, and D (at B's end + 4) is inside it. In B1 the owner's front is still `end + 1`, so the device receives D as it does today. This is the pre-existing #81 path. B1 does not change it and does not claim to fix it; B2 changes the test (§3.6) so that D is late. **B1 does not close #81 on this path.** Ruled (NQ-B): B1 is a local commit on `work/stepB`, and main receives one squashed commit after B2, so this path is never on main in its B1 state. Nothing in B1's commit message or KL text claims it fixed.

**The record is more conservative than the owner, so B1 alone is safe except on the path in point 6.** The B1 risk the record does cover is a test that books through the record and then sends or dispatches at `end + 1 … end + 9`: such a booking is now refused at receipt. Those tests are the red list below, and each is a receipt assertion, not a device assertion.

**Note on windows (F5, Q4, M1).** The cap of §0 (a) cannot bite at a production window (§0), so its kill is the window-0 unit and Mock tests (N-S5a, N-S5c, N-S5d, N-S5e, N-M5). The dropped-path extension of §0 (b) is reachable at the window edge (M1), so Q1 changes production behaviour there: a booking inside the extended region is refused, or moved past its own front, not admitted at `stop`. N-P5 and N-P6 pin that geometry on both Providers; they do not replace the unit tests.

**Red in B1 (the record's values; changed in §3):**

| Test (file:line) | Why red in B1 | Action |
|---|---|---|
| `crates/ezsdr-radio/tests/staging.rs:337` `rm_15_1_to_3_sample_burst_front_is_start_plus_five` | asserts `accept(5)` Ok; new front start + 14 | rename; 4..=13 refused, 14 Ok (§3.1) |
| `staging.rs:347` `rm_15_closed_burst_front_is_end_plus_one` | asserts `end + 1` Ok | rename; end + 1 … end + 9 refused, end + 10 Ok (§3.1) |
| `staging.rs:395` `rm_15_back_to_back_bookings_are_accepted` | `accept(95, 1)` before a chain at 100: region `[95, 109)` meets 100 | rewrite the pair as a head at 86 (front 100) and a chain at 100 |
| `staging.rs:545` `rm_16_second_stop_that_drops_a_booking_extends_the_region_to_the_hull` | Q1: A(50, 10) keeps its own front 70 once dropped, so the region is `[50, 70)` and the hull probe `accept(60, 5).unwrap()` panics | the second chain moves to 70 (front 85); probes: 40 refused, 70 refused, 85 admitted (§3.1) |
| `staging.rs:585` `rm_16_repeat_occupies_until_a_stop` | fixture: `accept(-10, 1)` has front 4 and meets the repeat at 0 | use `accept(-20, 1)` (front −6); the rest unchanged |
| `staging.rs:706` `rm_16_a_stop_that_cuts_nothing_adds_no_region` (`ended` case) | head at 0 (front 14) with Stop at 10 is kept: `accept(10, 1)` now refused | assert 10 refused, 14 Ok |
| `staging.rs:862` `rm_15_an_untimed_booking_is_placed_after_the_booking_it_meets` | front 111 → 120 | change values |
| `staging.rs:871` `rm_15_place_tests_the_full_front` | `place(100, 40)` Some(131) → Some(140) | change value |
| `staging.rs:884` `rm_15_an_untimed_booking_meeting_a_booking_beyond_the_horizon…` | `place_untimed` Some(131) → Some(140); `accept(131, 30)` now Err | change values |
| `staging.rs:914` `rm_15_an_untimed_repeat_meeting_a_later_booking_starts_after_it` | Ok(111) → Ok(120) | change value |
| `staging.rs:922` `rm_16_an_untimed_booking_meeting_a_stop_region_from_before_the_stop_is_placed_after_it` | Q1: head (15, 3) dropped by Stop at 10 keeps front 29; region `[10, 29)`; placed at 29, not 20 | change value and comment |
| `staging.rs:931` `rm_16_a_timed_send_asap_moves_past_only_the_stop_region` | Q1: region `[10, 29)`; moves to 29, not 20 | change value and comment |
| `staging.rs:940` `rm_16_a_timed_send_asap_meeting_a_chain_is_refused` | the chain of 13 at 0 now fronts at 23: `accept(15, 3).unwrap()` panics | use a chain of 4 at 0 (front 14); rest unchanged |
| `staging.rs:960` `rm_16_a_moved_untimed_head_goes_after_its_joined_pieces` | Some(151) → Some(160) | change value and comment |
| `crates/ezsdr-radio-uhd/tests/common/iii.rs:1268` `the_record_front_is_one_sample_past_the_device_end_of_a_head` | `head_front` values 5 → 14 (n = 1, 3, 4), 11 → 20 (n = 10), 33 → 42 (n = 32); `- padded == 1` → `== 10` | rename; change values |
| `iii.rs:1281` `a_booking_at_the_stop_is_admitted_after_a_head_the_stop_cuts` (uses `halt(c - 1, stop)`, the dropped path) | Q1: `accept(stop, 3)` refused | rename to `a_booking_after_a_head_the_stop_cuts_waits_for_its_front`; c + 3…c + 12 refused, c + 13 Ok |
| `crates/ezsdr-acceptance/tests/parity.rs:1502` `s1_parity_short_burst_front_s_s3_s5` | booking at s + 5 now refused; `manifest_rows == 2` fails | rewrite (§3.3) |
| `parity.rs:1551` `s1_parity_padded_front_survives_normal_close` | booking at s + 5 must be refused | change to s + 14 |
| `parity.rs:1804` `receipt_order_cases` (S1 loop `k <= 4`; S2 rows `B at the end + 1`) | S1 k = 5 flips; S2 `end + 1` flips | S1 over k ∈ {1, 3, 4, 5, 9, 10, 13, 14} with `ab = k <= 13` (B is refused when A came first: refused iff k ≤ 13, admitted at 14); the comment's `s + 5` becomes `s + 14`; S2 rows `end + 9` refused, `end + 10` admitted |
| `parity.rs:1662` `s3_parity_booking_after_a_closed_short_burst_follows_d5` | loops `offset in 0..=2`; `e + 1` now refused | offsets {0, 9, 10} (refused, refused, admitted); comment `[s, e + 1)` → `[s, e + 10)` |
| `parity.rs:2173` `s15_parity_untimed_queue_behind_a_booking` (sequence 4) | expects `(11, 11)`; B is 10 samples at s, front s + 20 | expect `(20, 20)`; comment "s + 11" → "s + 20" |
| `crates/ezsdr-mock-radio/tests/mock_channel.rs:620` `mr_17_a_one_sample_burst_uses_the_safe_sample` | refused 2_004_003, 2_004_004; free 2_004_005 → refused to 2_004_013, free 2_004_014 | change targets; comment "the next four" → "the next thirteen" |
| `crates/ezsdr-mock-radio/tests/mock_radio.rs:1018` `x3xx_move` | `records[1].target == 2_003_005` → 2_003_014 | change value |
| `crates/ezsdr-radio-uhd/src/provider/tx.rs:2095` `ur_21_s1_overlap_refused_at_receipt` | B at 2_001 and 2_005 are refused at receipt | rewrite: 2_001, 2_005 refused; 2_014 admitted (B1, through the control path) |
| `tx.rs:3135` `ur_23_stop_does_not_send_unhanded_window_edge_payload` | B at 10_004 is now refused at receipt, so the Stop-window sequence is lost | keep the sequence: B moves to 10_013 (admitted, not yet handed). Its device assertions move to 10_013 and the clock advances to 10_020 so an uncancelled B would have been sent (§3.6) |
| `tx.rs:3238` `ur_23_a_one_sample_burst_restarts_after_its_padded_device_burst` (receipt part only) | B at 2_005 accepted at receipt; now refused | B1: receipt asserts refusal at 2_005 and admission at 2_014; the play part is B2 |
| `staging.rs:563` `rm_16_a_stop_that_drops_nothing_adds_no_region` | the second chain (30, len 5, own front 45) is dropped by the second Stop and its region `(25, 45)` overlaps the probe `accept(20, 1)` (front 34), which is refused | M2: the second chain moves to 40 and the second Stop to `halt(40, 60)`; the probe `accept(20, 1)` Ok is kept. Sole killer of N47S5-55 |
| `staging.rs:666` `rm_15_a_chain_behind_an_open_join_is_not_yet_handed` | `accept(45, 1)` (front 59) overlaps the chain at 50, so it is `Err(Booking)`, not `Ok` | M2: the second chain moves to 60 (len 40), and every call uses horizon 70; the held case stays Ok and the expired case stays `Err(Handed)`. Sole killer of N47S5-84 |
| `mock_channel.rs:535` `mr_25_stop_does_not_radiate_unhanded_window_edge_payload` | window edge (H = 2_010_000, stop = H + 3): D at H + 4 falls inside the region that Q1 gives A's own front, `[stop, 2_010_013)`, so D is refused at receipt and its on-time record is lost | M2/M1: B and D move to 2_010_013 (A's own front; B stays cancelled by the Stop; D is the first sample after A's region). Sole killer of N47S5-23 |
| `mock_radio.rs:1264` `mr_16_untimed_finite_burst_queues_behind_the_booking` | the 10-sample booking at 2_010_000 fronts at 2_010_020 (end + 10), not 2_010_011 | change the value to 2_010_020 |
| `tx.rs:6307` `ur_23_a_value_waits_for_a_burst_moved_at_dispatch_past_its_booked_end` | the moved 4-sample burst's front is 21_014 (record rule), and `turns_at` stops at 21_006, so the value never applies in the window the test checks | M2: assert `clock.instant(21_014)` and add 21_015 and 21_016 to the turns. Sole killer of N47S4-09a |

**Green in B1 but comment or strength changed:** `staging.rs:436` `rm_15_a_short_head_left_alone_is_closed_and_takes_no_join` (150 stays refused; comment "151" → "160"; add 159 refused, 160 Ok); `staging.rs:462` `rm_15_a_short_piece_left_behind_keeps_its_own_front` (100053 stays refused; comment 100055 → 100064; add 100063 refused, 100064 Ok).

**Green in B1, unchanged:** `parity.rs:1528`, `1578`, `1622`, `1713`, the `s4`/`s4c` rows; `tx.rs:2962` and `2989` (`ur_21_s2_…`, both joins; B1 run confirms, since `txp_record` `expect`s the receipt); `tx.rs:3022` (point 6; its device outcome is today's in B1); the Stop tests listed in §1 row 6 (`staging.rs:527`, `721`).

## 3. The test changes, by sub-step

### 3.1 B1: `crates/ezsdr-radio/tests/staging.rs` (record rule)

Rewritten (keep the name unless noted):
- `rm_15_1_to_3_sample_burst_front_is_start_plus_five` → `rm_15_1_to_3_sample_burst_front_is_transmitted_end_plus_ten`. Loop `len in 1..=3`: `accept(4..=13, Some(1))` each Err, `accept(14, Some(1))` Ok. Wrong behaviour caught: front start + 5 (old), start + 13 (off by one), padding not counted (front start + len + 10).
- `rm_15_closed_burst_front_is_end_plus_one` → `rm_15_closed_burst_front_is_end_plus_ten`. For `len` in 4..=32 (explicit 4, 10, 31, 32): `accept(len..=len+9, Some(1))` Err, `accept(len+10, Some(1))` Ok. Keep the 33-sample check (joins at its end). Wrong behaviour: front `end + 1` (old), `end + 9`, `end + 11`, or 32 treated as a held tail (`end + 2`).
- `rm_15_back_to_back_bookings_are_accepted`: head at 86 (front 100) then chain at 100 accepted. Wrong behaviour: a 1-sample head at 95 refusing a chain at 100 (front 109).
- `rm_16_a_stop_that_cuts_nothing_adds_no_region` (`ended` case): `accept(10, 1)` Err (its own front 14 is kept); `accept(14, 1)` Ok.
- `rm_15_an_untimed_booking_is_placed_after_the_booking_it_meets` (862): front 120; the untimed burst at 103 is placed at 120; `accept(120, 10)` Ok, `accept(110, 10)` Err.
- `rm_15_place_tests_the_full_front` (871): `place(100, 40)` = Some(140).
- `rm_15_an_untimed_booking_meeting_a_booking_beyond_the_horizon_is_placed_after_it` (884): `place_untimed(100, 100, 30)` = Some(140); `accept(131, 30)` Err; `accept(140, 30)` Ok.
- `rm_15_an_untimed_repeat_meeting_a_later_booking_starts_after_it` (914): Ok(120).
- `rm_16_an_untimed_booking_meeting_a_stop_region_from_before_the_stop_is_placed_after_it` (922): Stop at 10, bound 20, head (15, 3) dropped with front 29 (Q1): placed at 29.
- `rm_16_a_timed_send_asap_moves_past_only_the_stop_region` (931): the timed booking at 12 moves to 29 (the region `[10, 29)`).
- `rm_16_a_timed_send_asap_meeting_a_chain_is_refused` (940): chain of 4 at 0 (front 14), B at 15 (3 samples, cancelled by the Stop at 10), `region_move(12, 5)` = 12, `accept(12, 5)` Err.
- `rm_16_a_moved_untimed_head_goes_after_its_joined_pieces` (960): `move_untimed(100, 120, 40)` = 160.
- `rm_16_second_stop_that_drops_a_booking_extends_the_region_to_the_hull` (545, M2): the first part is unchanged. Hull part: `accept(50, 10)`, `halt(50, 60)` (region `[50, 70)`), the second chain at 70 (`accept(70, 5)`, front 85), `halt(55, 80)` drops it; the region is `[50, 85)`. Probes: `accept(40, 1)` Err (its own front 54 reaches the first cut at 50; N47S5-67's `start.min(t)` → `t` would admit it), `accept(70, 1)` Err (N47S5-09), `accept(85, 1)` Ok (the dropped chain's own front).
- `rm_16_a_stop_that_drops_nothing_adds_no_region` (563, M2): the first Stop `halt(10, 20)` drops nothing (no region); the second chain at 40 (len 5, own front 55); `halt(40, 60)` drops it: region `[40, 60)`. Probe `accept(20, 1)` Ok (front 34, below 40). Mutant `if true {` (N47S5-55) hulls the first Stop's `[10, 20)` in and refuses the probe.
- `rm_15_a_chain_behind_an_open_join_is_not_yet_handed` (666, M2): the chain at 0 (len 40) is unchanged; the second chain moves to 60 (len 40); every call uses horizon 70. Held: `accept(45, 1)` at now 0 is Ok. Expired: `accept(45, 1)` at now 141 is `Err(Handed)`. Mutant N47S5-84 admits the held case as handed.
- Comment-only: 436, 462 (see §2).

New staging tests (B1):
- **N-S4** `rm_15_an_untimed_booking_after_a_padded_head_is_placed_at_end_plus_ten` (sequence 4): heads of 1 and 3 samples at 0 (padded, front 14); `book_untimed` at 2 with 5 samples is placed at 14. Observes `book_untimed`'s return value. Wrong behaviour: placed at 5 (old) or 13.
- **N-S5a** `rm_16_a_stop_keeps_an_uncut_short_head_s_front` (sequence 5a, kept, window 0): for `len` in 1, 2, 3: head `accept(c - 1, Some(len))`, `halt(c, c + 3)` (start < t, so kept; the payload is cut by nothing). `accept(c + 3..=c + 12, Some(3))` Err, `accept(c + 13, Some(3))` Ok. Wrong behaviour: front capped at c + 3 (#82, `min(stop)` on an uncut head).
- **N-S5b** `rm_16_a_stop_that_cuts_a_short_head_adds_its_front_to_the_region` (sequence 5b, dropped, Q1): head `accept(c - 1, Some(1))`, `halt(c - 1, c + 3)` (start ≥ t, start < stop): `accept(c - 1..=c + 2, Some(1))` Err, `accept(c + 3..=c + 12, Some(3))` Err, `accept(c + 13, Some(3))` Ok. Wrong behaviour: region `(t, stop)` only (old: c + 3 admitted, the bench failure R5-m5 on this path).
- **N-S5c** `rm_16_a_stop_inside_a_32_sample_head_keeps_its_front` (sequence 5c, F1's counterexample): `accept(100, Some(32))`, `halt(101, 104)`; `accept(104, Some(1))` Err; `accept(142, Some(1))` Ok. Wrong behaviour: a Q2-style payload-end test that caps the front at 104 (A's payload is handed as one closed send, so B at 104 would overlap it). Killed by N49B-03.
- **N-S5d** `rm_16_a_stop_caps_a_33_sample_head_at_the_bound` (window 0; m3, the length bound; N49B-11): head `accept(0, Some(33))` (a long head, front 35, joinable), `halt(1, 20)`: `accept(19, Some(1))` Err, `accept(20, Some(1))` Ok (capped at the bound). Mutant `end <= start + 33` keeps the uncapped front 35 for the 33-sample head, so `accept(20, 1)` is Err.
- **N-S5e** `rm_16_a_stop_caps_a_short_chain_with_joined_pieces` (window 0; m3, the `joined` term; N49B-12): head `accept(0, Some(10))`, a joined piece `accept(10, Some(4))` (the chain's front 16), `halt(1, 12)`: the chain is capped at 12, so `accept(11, Some(1))` Err and `accept(12, Some(1))` Ok. Mutant that drops `joined.is_empty()` treats the chain as a standalone short head and keeps its own front (uncapped), so `accept(12, 1)` is Err. The join needs `ON_TIME`, as in `rm_15_contiguous_booking_joins_a_long_head` (`staging.rs:373`); fixture confirmed at implementation.
- N-S2 (the Q2 payload-end test, first plan) is dropped (F1).

### 3.2 B1: MockRadio (`crates/ezsdr-mock-radio/tests/`)

- `mock_channel.rs:620` `mr_17_a_one_sample_burst_uses_the_safe_sample`: targets `(2_004_003, true), (2_004_013, true), (2_004_014, false)` (`s` = 2_004_000; 1 sample, padded to 4; front s + 14). Comment: "holds its own sample and the next thirteen". Wrong behaviour: front s + 5 (old) or s + 9.
- `mock_radio.rs:1018` `x3xx_move`: `records[1].target` 2_003_005 → 2_003_014.
- New **N-M5** `mr_16_a_stop_keeps_an_uncut_short_head_s_front` (sequence 5a on the Mock's window-0 profile, `ideal`): the same geometry as N-S5a through the Mock's `stop(mode)` (`crates/ezsdr-mock-radio/src/lib.rs:1164–1168` `stop_bound`, `stop_sample`, `busy.halt`). Observed through the receipt answer (`MR-16: burst overlaps an accepted booking`) at c + 3 and c + 13. Wrong behaviour: the Mock admits c + 3 while UHD refuses it (parity break). If the Stop cannot sit at the Mock's cursor c, this test is replaced by N-S5a at the unit level and the parity test N-P5 carries the cross-Provider check (NQ-A).
- `mock_channel.rs:535` `mr_25_stop_does_not_radiate_unhanded_window_edge_payload` (M2, sole killer of N47S5-23; window 10 ms x3xx at 1 MS/s): t = 2_000_000, H = t + W = 2_010_000, stop = H + 3 = 2_010_003. Changes: B moves from 2_010_005 to 2_010_013 (A's own front; B is cancelled by the Stop and never handed); A (2_009_999, handed) stays; C at H + 1 = 2_010_001 stays refused; D moves from H + 4 to 2_010_013 (on time). Asserts: the records are A and D, D's target is 2_010_013, and the rejected list keeps B's "cancelled by stop". N47S5-23's mutant (`held` emptied) would radiate B, a third record.
- `mock_radio.rs:1264` `mr_16_untimed_finite_burst_queues_behind_the_booking` (M2, value only; no row): the 10-sample booking at 2_010_000 fronts at 2_010_020, so the queued untimed burst's expected target changes from 2_010_011 to 2_010_020.
- `tx.rs:6307` `ur_23_a_value_waits_for_a_burst_moved_at_dispatch_past_its_booked_end` (B1, M2, sole killer of N47S4-09a): the 4-sample burst moved to 21_000 has record front 21_014 (`end + 10`); the assertion `clock.instant(21_005)` becomes `clock.instant(21_014)`, and turns 21_015 and 21_016 are added. The mutant (`previous_end = None`) applies the value before the front and is killed.
- Unchanged and green: `mr_25_s7_stop_tail_refuses_until_bound`, `mr_16_s2` (`mock_radio.rs:1087`, long A), `:1112`, `:1183`, `:1286`, `:1405`, `:1448`, `:1472`.

### 3.3 B1: parity (`crates/ezsdr-acceptance/tests/parity.rs`)

Changed:
- `s1_parity_short_burst_front_s_s3_s5` (1502): bookings at s + 3, s + 5, s + 9 refused; s + 14 admitted, on both Providers. Pin the target, the actual start and the absence of TIME_ERROR for the admitted one.
- `s1_parity_padded_front_survives_normal_close` (1551): s + 5 → s + 14.
- `receipt_order_cases` (1804–1841): §2 values.
- `s3_parity_booking_after_a_closed_short_burst_follows_d5` (1662): offsets {0, 9, 10}.
- `s15_parity_untimed_queue_behind_a_booking` (2173): expects (20, 20) (sequence 4; the existing cross-Provider test; no new test).

New parity tests (B1):
- **N-P1** `s1_parity_short_head_region_is_end_plus_ten` (sequences 1, 2): heads n = 1, 2, 3 at s, and n = 4, 10, 31, 32 at s; a booking at `s + max(n, 4) + k` for k 0–9 refused on both Providers (receipt answer, `overlap_refusals` count 1 on each), k = 10 admitted with its `bursts` row, its actual start equal to its target and no `late_drops`. The two Providers' refusal lists are equal. Wrong behaviour: one Provider uses a different front, or the formula is written with `s + 4 + k` (not `max(n, 4)`) so that n = 10 is checked at the wrong sample.
- **N-P5** `s1_parity_window_edge_dropped_region_and_kept_head_agree` (sequence 5; reachable geometry at a window edge, both Providers; NQ-A ruling). Test-side only: the existing helpers `stop_at` and `stop_both` (`parity.rs:269`, `385`, `1093`) place the Stop at the arm instant. No production hook. W = 10 ms (x3xx), H = t + W = stop − 3, stop = H + 3 (tx-domain samples; t = 2_000_000 at 1 MS/s, as `mr_25`).
  - (i) Dropped head at the window edge (Q1, M1): a one-sample head A at H − 1 (start < H: dropped and possibly handed; own front H + 13). Bookings: H + 1 refused at receipt (`COMMAND_REJECTED`, nothing transmitted); H + 4 refused at receipt under DropAndFlag (the region is `max(stop, own front) = H + 13`, not `stop = H + 3`); H + 13 admitted, actual start H + 13, no `TIME_ERROR`. Pins the target and the start on both Providers.
  - (ii) Kept head before the Stop: a one-sample head A' at t − 1 (start < t, kept, own front t + 13). Bookings at t + 1 and t + 12 refused at receipt; t + 13 is late under DropAndFlag (`TIME_ERROR`, nothing transmitted) if it lies before the Provider's late bound. The first admitted start is the same on both Providers (the value each reports is pinned). This part checks agreement only; the cap itself is not visible here (§0).
  - Wrong behaviour: a Provider that admits H + 4 (Q1 not applied at the window edge), a Provider that refuses t + 13 at receipt (a cap-like region), or a Provider that disagrees with the other on any row.
- **N-P6** `s1_parity_window_edge_stop_region_by_late_policy` (sequence 5, dropped path, geometry of N-P5 (i)): a booking at H + 4 (inside `[stop, H + 13)`) under each policy, on both Providers. DropAndFlag: refused at receipt (`COMMAND_REJECTED`, nothing transmitted, no `TIME_ERROR`). SendAsapAndFlag: moved to `max(stop, own front) = H + 13` and sent there with `TIME_ERROR { send_asap }`; the actual start is H + 13. Untimed (no `at`): placed at H + 13, no `TIME_ERROR`. Pins the target, the actual start and the presence of `TIME_ERROR` per Provider. Wrong behaviour: target `stop` (H + 3) instead of `max(stop, own front)` (the M1 error), or a DropAndFlag booking moved instead of refused. Contract: `design/07-radio-model.md:167–168` (RM-16).

### 3.4 B1: `crates/ezsdr-radio-uhd/tests/common/iii.rs`

- Test at 1268 (see §2): `head_front` values; `(n, padded)` loop `- padded == 1` → `== 10` for `(1,4), (2,4), (3,4), (4,4), (10,10), (32,32)`. Renamed `the_record_front_is_ten_samples_past_the_transmitted_end_of_a_head`.
- Test at 1281: renamed (see §2), Q1 probe at c + 13.
- Optional, B1: `a_stop_that_cuts_nothing_of_a_short_head_leaves_its_front` (sequence 5a at the UHD record).
- Run: the full `fake` test binary (iii is compiled into `fake` and `hardware` through `tests/common/mod.rs`). A filter of `the_record_front` does not run the Stop test (F3).

### 3.5 B1: RM-15, RM-16, KL-36 text (design/ and KNOWN_LIMITATIONS.md)

- `design/07-radio-model.md:162` (RM-15): replace "a standalone head of 1–3 samples ends at `start + 5`, of 4–32 samples at its end plus one, and a head longer than 32 or a joined piece at its end plus two" with: a standalone closed device burst of 1–32 transmitted samples (padded to four when shorter) ends its region at its transmitted end plus ten; a head longer than 32 or a joined piece keeps its end plus two. One Changes row (note 49 step B, B1). Any `*Test:*` line added must name an existing test.
- `design/07-radio-model.md:165` (RM-15's Producer obligation, "The next safe start is the closing send's exclusive end plus one sample"): "plus ten for a standalone closed device burst of at most 32 transmitted samples, plus one otherwise" (m2; the same Changes row as RM-15, B1).
- `design/07-radio-model.md:167–168` (RM-16) and `design/09-mock-radio.md:171` (MR-25): the Stop text gains (a) a standalone closed short head that the Stop keeps keeps its own front, and (b) a dropped standalone short head that may have been handed (start < H, the handover horizon `H = stop − 3`) extends the region to its own front (Q1). One Changes row each.
- KL-36 (`KNOWN_LIMITATIONS.md:134–141`):
  - Title: "decided late, not continued" → "refused at receipt, not continued" (the symptom already says so).
  - When (136): "for 4 to 32 samples, its exclusive end; for 1 to 3 samples, up to its start plus four" → "up to its transmitted end plus nine (the padded end for 1 to 3 samples)".
  - Workaround (137): "start the next one at the short burst's padded device end plus one sample: one sample after its end for 4 to 32 samples, five after its start for 1 to 3" → "start the next one ten samples after its transmitted end (padding included)".
  - Evidence (138): add `plan/phase8/bench-results.md` (2026-11-09) and issues #81, #82.
  - Status (139–140): unchanged in kind (Ceiling).
  - Its `Untimed:` test name `rm_15_an_untimed_booking_is_placed_after_the_booking_it_meets` is kept.
- `rules.py` after the edits: `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base <step-A commit>` must be 0. No cited test is renamed in B1 (`s3_parity_…`, `ur_21_s1_overlap_refused_at_receipt`, `rm_15_an_untimed_booking_is_placed_after_the_booking_it_meets` are cited and keep their names).

### 3.6 B2: uhd-tx (`crates/ezsdr-radio-uhd/src/provider/tx.rs`)

Production change (F8, no state): at `tx.rs:800`, in the `device_eob` branch, `open.next + pad + 10` when `device_sob && device_count <= MIN_OPEN_FIRST_SEND` (32), otherwise `open.next + pad + 1` (Q5). `device_sob`, `pad` and `device_count` (`tx.rs:685–686`) are in scope at that point (the implementer confirms).

Why no state is needed: an open first send is at least `MIN_OPEN_FIRST_SEND` = 32 samples (UR-22) and a continuation at least 2 (padded), so a device burst of at most 32 transmitted samples is one send carrying SOB and EOB. The SOB flag is needed: a 1-sample continuation (padded to 2) after a 32-sample open first send has `device_count <= 32` but is not a short burst (N-O7b).

**Exception: the abandon path (rev 3).** An open first send that is partly accepted is abandoned; `abandon` (`tx.rs:976`) closes its device burst with two zeros and `advance_front(cursor + 3)` (`tx.rs:1009`), not the `device_eob` branch at `tx.rs:800`. Such a burst has exactly 32 transmitted samples only when 30 were accepted (30 + 2 zeros). Fewer than 30 is armed by the existing open-short rule (`device.rs:663`, `OPEN_FAULT_BELOW = 30`) and faults at once; 31 or more is longer than 32 with its zeros. So for 30 accepted samples the owner's front is cursor + 3 while the FakeDevice guards +10. The model is stricter than the owner: it refuses a booking the owner dispatches on time, with a visible `TimeError`, not a silent wrong burst. Hardware not measured (INFERRED). Stated in UR-33 (§3.9) and pinned by N-F8e (§3.7).

Changed tests:
- `tx.rs:2389` (`chain.front == 10_004`) → 10_013; the comment "the padded device EOB establishes the safe restart sample" stays.
- `tx.rs:3238` `ur_23_a_one_sample_burst_restarts_after_its_padded_device_burst`: receipt part in B1 (§2); B2 part: the restart is 2_014 in play as well as at receipt.
- `tx.rs:5134` `ur_22_a_short_first_send_is_padded_to_four`: 3_005 → 3_014; comment "the padded end + 1" → "+ 10".
- `tx.rs:5289` `ur_23_a_burst_at_a_closed_short_burst_s_end_is_late_at_dispatch` (owner-only): offsets `[(0, late), (1, false)]` → `[(0, late), (9, late), (10, false)]`.
- `tx.rs:5364` `ur_22_a_two_channel_one_sample_burst_is_padded_on_both_channels`: `start + 5` → `start + 14`.
- `tx.rs:6330` `ur_23_a_value_waits_for_a_padded_one_sample_burst` (N47S4-09b killer, F-note): the turns extend past 20 006 to 20 015 (`tx.rs:6338` stops at 20 006 today), and the value assertion moves to the owner's front `clock.instant(20_014)`. The mutant `+ 1` applies the value at 20 005, below the front, and is killed.
- `tx.rs:3022` `ur_21_s2_joined_pieces_left_behind_a_refused_head_keep_their_regions` (F4, B2; point 6): the expected outcome of each piece is pinned from its trace. A(40): configure refused, `COMMAND_REJECTED` (unchanged). B (10 samples at 40_040): a standalone closed short head with front 40_060; it plays. C (4 at 40_050): late under its LatePolicy (TIME_ERROR, nothing transmitted). D (10 at 40_054): inside B's new front, so late under its LatePolicy at dispatch (TIME_ERROR, nothing transmitted; `send_asap` would move it to 40_060 with `TIME_ERROR { send_asap }`, and the test pins whichever its policy gives). E (40_056) refused at receipt (unchanged). The assertion "D plays as recorded" is replaced by these per-piece assertions. The implementer reads the test's `LatePolicy` before writing them.
- `tx.rs:3135` `ur_23_stop_does_not_send_unhanded_window_edge_payload` (B1 change, §2): B moves from 10_004 to 10_013 (admitted at receipt, not yet handed). A is still one sample at 9_999. The device assertion `transmitted(10_004)` → `transmitted(10_013)`, and the clock advances to 10_020 (B's start must be in the past for an uncancelled B to be sent, so the test is not vacuous). The implementer confirms the test fails when the Stop's cancellation is removed (existing rows, or a new one if none does).
- `tx.rs:5289`, `tx.rs:5364`, `tx.rs:6330`: as above (owner-only).

New tests (B2):
- **N-O7** `ur_23_a_closed_short_burst_s_owner_front_is_ten_past_its_end` (sequence 7): for `len` in 1, 3, 4, 10, 31, 32, a closed burst at `k`: the owner's `chain.front()` equals `k + max(len, 4) + 10` and equals the record's front (`txp_record`); `past_front(&core) == 0`. Wrong behaviour: owner front `end + 1` (old) or `end + 9`.
- **N-O7b** `ur_23_a_continuation_that_closes_a_burst_keeps_the_front_one_past_its_end` (sequence 12): a 32-sample open first send, then a 1-sample continuation (padded to 2) closing the device burst: the owner's front is `end + 1` (the pre-existing `+ 1` for a device burst longer than 32 samples; Q5), not `end + 10`. Documents the Q5 mismatch and pins the SOB condition. Killed by N49B-10.

Mutation rows (B2): §4.

### 3.7 B2: FakeDevice (`crates/ezsdr-radio-uhd/src/device.rs`)

State (device model, not owner state; F2): `Radio` gains
- `burst_samples: i64`: transmitted samples of the current device burst, summed over its accepted packets; reset at each SOB (`:669`);
- `short_until: Option<i64>` (ticks): set at the accepted EOB (`:722`) to `cursor + 10 * ratio` when `burst_samples <= 32`; `None` at any other accepted EOB. Cleared by nothing else (only an accepted EOB of a longer burst clears it; the guard applies only while idle).

Guard (B2): in `Radio::packet`, before the existing idle timed refusal at `:689`: `if !self.active && packet.sob && packet.radio_time.is_some_and(|t| self.short_until.is_some_and(|until| t < until))` → one TimeError per channel for the SOB (existing refusal shape), `if !packet.eob { self.burst = Burst::Timed; }`, `return None`. The refusal does not move the cursor, does not arm or clear the open-short rule (existing `ur_33_a_refused_sob_packet_does_not_arm_the_short_rule` at `:2917` stays green), and sets no BurstAck. A closed refused SOB+EOB (single packet) gets its TimeError and no state change.

Continuations of a refused open SOB: `faulted` (existing; `Timed` state) gives a TimeError per timed continuation, one TimeError for the first untimed one and then `Silent`, and nothing transmitted and no BurstAck; the EOB returns the state to `Clean`, so the next normal SOB recovers. The DUC advances before `radio.packet` even for refused packets (`:1805–1808`), so the continuation's time is the DUC's and is refused regardless.

The refusal of a continuation is the model's rule, not a measured hardware result (INFERRED that the hardware behaves the same way; the bench did not test a refused open SOB).

Sequence 8 exception (Q6): no exception. The hardware plays a short B at gap 1 after a short A; the model refuses it (conservative). Documented in UR-33 (§3.9). Rejected: model the L 1–3 → short-B exception (the evidence is for a following long burst only).

New tests (B2, in `device.rs` unit tests beside `ur_33_an_undersized_timed_first_packet_is_dropped` at `:2785`; manual clock as `ur_33_a_closed_four_sample_packet_is_not_dropped` at `:3092`):
- **N-F8a** `ur_33_a_timed_sob_within_ten_of_a_closed_short_burst_is_refused`: closed 4-sample SOB+EOB at `t0`, then a timed SOB+EOB (4 samples) at `t0 + 4 + g` samples: g 0..=9 → TimeError per channel, `transmitted` None, no BurstAck; g = 10 plays. Also a closed 32-sample burst (guard applies at g = 9 and not at g = 10). Wrong behaviour: gap 9 played (fake looser than the hardware's guard, AGENTS §3), or gap 10 refused.
- **N-F8b** `ur_33_a_closed_burst_over_32_keeps_the_gap_of_one`: a 33-sample closed burst, then a timed SOB at end + 1 plays; a 32-sample open first send with a 2-sample closing continuation (total 34), then a timed SOB at end + 1 plays. Wrong behaviour: the rule applied to a longer burst (killed by N49B-07).
- **N-F8c** `ur_33_a_refused_open_sob_refuses_its_device_burst_through_its_eob`: closed 4-sample burst; timed open SOB (100 samples, no EOB) at end + 9 → TimeError per channel. Then a timed continuation (with EOB) at its time: TimeError per timed continuation, nothing transmitted, no BurstAck. An untimed continuation: one TimeError, then nothing more through EOB. A normal timed SOB at end + 10 (or later) plays clean. Wrong behaviour: the continuation plays (the case Sol found: the continuation passes the timed check and BurstAck is sent), or the recovery fails. Killed by N49B-08.
- **N-F8d** `ur_33_the_short_guard_counts_the_device_burst_not_its_last_packet`: (i) a 30-sample open first packet (not armed: `OPEN_FAULT_BELOW` is 30) and a 2-sample closing continuation (total 32): a timed SOB at end + 9 refused, end + 10 plays. (ii) a 32-sample open first packet and a 2-sample closing continuation (total 34): a timed SOB at end + 1 plays. The owner never sends an open first packet under 32 (UR-22); (i) pins the model's boundary. Wrong behaviour: the count uses the last packet only (`burst_samples = n`): (ii) is refused (killed by N49B-09).
- **N-F8e** `ur_33_an_abandoned_30_sample_open_send_is_guarded_as_a_short_burst` (rev 3, the abandon exception): a first open SOB of 30 samples (not armed: 30 is not below `OPEN_FAULT_BELOW`), then a two-zero EOB packet of 2 samples (total 32). A timed SOB at the EOB's exclusive end + 3 samples gets TimeError per channel and nothing is transmitted; at + 10 it plays. (The owner's front for this close is cursor + 3, `tx.rs:1009`; the implementer checks whether an existing abandon test asserts it, and pins it in B2 if none does.) Wrong behaviour: the guard uses the owner's +3 (a looser model), or the two zeros are not counted. The model stays stricter; UR-33 says so (§3.9). No row of its own: N49B-07 and N49B-09 cover the guard's condition and count.

Changed tests:
- `device.rs` existing tests are not changed (checked: `:2785`, `:2801`, `:2917`, `:2935`, `:2954`, `:3092`, `:3202`).
- `crates/ezsdr-radio-uhd/tests/fake.rs` (Module-level): `n47s5_front_rehearsal_on_the_fake` (`:7776`) and `n47s5_held_tail_and_stop_rehearsal_on_the_fake` (`:7819`) read the record front, so they move with it and stay green (checked: they do not book at end + 1…9). `ur_23_a_short_window_first_send_plays_clean_at_1_mhz` (`:2967`), `ur_23_two_bursts_back_to_back_loop_back_whole` (`:3075`), `ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time` (`:3111`), `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake` (`:7880`): green (no closed short burst followed by a timed SOB inside the window). Re-verified by the B2 run.

### 3.8 B2: hardware tests (`crates/ezsdr-radio-uhd/tests/hardware.rs`, `#[ignore]`; compile check only here)

- `:3669` (comment on the record's values) and `:3684` ("n ≤ 3: start + 5; 4 ≤ n ≤ 32: end + 1") → "n ≤ 32: transmitted end + 10".
- `hw_n47s5_front_after_a_short_burst` (`:3682`): values come from `iii::head_front` and move by themselves; the comment and the claim text follow. Gaps `[0, 1, 5, 9, 10, 11]` for the padded head.
- `hw_n47s5_raw_front_gaps` (`:3708`): the gap list `[0, 1, 2, 3, 4, 5]` → `[0, 1, 4, 5, 6, 7, 9, 10]` (the VERIFIED table of §0); the comment at `:3711` "gap 1 is the front" → "gap 10 is the front". The 100-sample closed B stays as a labelled short-B observation (F6): the comment says so.
- **N-H1** `hw_n47s5_long_b_after_a_short_head_gap_9_and_10` (new, `#[ignore]`; m5): a long B (10 000 samples, split timed sends, the form of the UHD-only reproduction's `sweep` case; the implementer reads `tx_repro`) after a short head of 1 (padded to 4), 4, 10 and 16 samples, at gaps 9 and 10. L = 1 is included because a 100-sample closed B plays at gap 1 after any short head, so only the long B separates the rule. Expected (VERIFIED table, §0): for L 1 and L 4 (L 4–8 needs g ≥ 10) the long B is late at gap 9 and on time at gap 10; for L 10 and L 16 it is on time at gap 10 (L 10–14 needs g ≥ 7; L 16 needs g ≥ 5). Not run here.
- `hw_n47s5_held_tail_and_stop` (`:3756`), row R5-m5 (`:3787`, the call `raw_burst_gap_rows(&*device, 1e6, samples, &[0], 4)`): `&[0]` → `&[10]` (F6). The booking after it is at c + 13 (Q7). The comment at `:3756–3770` about the head "padded to end at c + 3" → "its front is c + 13".
- `raw_burst_gap_rows` (`:1272`): unchanged (the gap list comes from the callers).

### 3.9 B2: design text and KNOWN_LIMITATIONS.md (F7)

- `design/18-uhd-radio.md:253` (UR-22 safe-start sentence: "the next safe start is the closing send's exclusive end, padding included, plus one sample"): becomes "plus ten for a closed device burst of at most 32 transmitted samples, plus one otherwise". A Changes row.
- `design/18-uhd-radio.md:260` (UR-23: "A Kernel burst that starts before a closed short device burst's front (its end, padding included, plus one sample) is decided by its own late policy"): "plus one sample" → "plus ten samples for a closed device burst of at most 32 transmitted samples (UR-22)". A Changes row.
- `design/18-uhd-radio.md:342` and `:351` (UR-33): add (i) the closed-short guard (an idle timed SOB less than ten samples after a closed burst of at most 32 transmitted samples is refused with TimeError per channel); (ii) a refused open SOB refuses its device burst through its EOB; (iii) the conservative short-B treatment (the model refuses a short B that the hardware plays at gap 1; no test claims that the hardware refuses gap 9 for a short B). Tests named: N-F8a–d. A Changes row.
- `design/18-uhd-radio.md:342` and `:351` (UR-33), (iv) the abandon exception (rev 3): a 30-sample open first send abandoned by its two-zero close (total 32) is guarded at +10 while the owner's front is cursor + 3; the model is stricter, with a visible `TimeError` (N-F8e).
- `KNOWN_LIMITATIONS.md:130` (KL-13 workaround): "or leave a gap of 2 samples or more" → add "(ten samples after a closed burst of at most 32 transmitted samples, KL-36)".
- `KNOWN_LIMITATIONS.md:132` (KL-13 status): "the safe start is the closing send's exclusive end plus one sample (UR-22)" → "plus one sample, or plus ten after a closed burst of at most 32 transmitted samples (UR-22, KL-36)".
- `KNOWN_LIMITATIONS.md:151` (KL-38 workaround): "leave a gap of one sample after the short burst's end" → "leave a gap of ten samples after the short burst's transmitted end (KL-36); one sample is no longer a safe start".
- `KNOWN_LIMITATIONS.md:308` (x3xx profile section, the first-send framing "the next safe start is the exclusive end of the sent device burst, padding included, plus one sample"): plus ten for a device burst of at most 32 transmitted samples, plus one otherwise (B2).
- `design/09-mock-radio.md`: MR-17 (`:143`) states no number and needs no text change; a Changes row only if the implementer adds a sentence.

## 4. Mutation rows (`plan/maintenance/tools/mutations.json`, Rust token anchors, filters name the killing test)

Verified by a Python scan of the 1 624 rows (`old`, `new`, `filter`). Re-anchor a row when its `old` text changes; each `old` must match exactly once (`anchor.py`, checked by `rules.py`). Exact new anchors are fixed at implementation, after the code exists.

**Re-anchored or renamed (killing test kept or renamed):**

| Row | Sub-step | Old now | Change | Filter | Note |
|---|---|---|---|---|---|
| N47S5-02 | B1 | `start.checked_add(len.max(4))?.checked_add(1)` | `checked_add(10)` | `rm_15_closed_burst_front_is_end_plus_ten` (renamed) | mutant `Some(start.checked_add(len.max(4))?)` (no gap) kept |
| N47S5-03 | B1 | `Some(len) => start.checked_add(len.max(4))?.checked_add(1),` | the same line with `checked_add(10)` | `rm_15_1_to_3_sample_burst_front_is_transmitted_end_plus_ten` (renamed) | the 1–3 and 4–32 arms are one line, so the row now mutates `len.max(4)` to `len.max(3)` |
| N47S5-14 | B1 | `c.front = c.front.min(stop); c.join = false;` (halt, `:351–352`) | the new kept-path line, keeping `c.join = false;` | `rm_16_stop_truncates_the_front_and_ends_the_join` (40-sample head, unaffected by F1) | mutant `c.join = c.join;` (join not ended) keeps its coverage (Sol) |
| N47S5-62 | B1 | `c.front = c.front.min(stop);` | the new kept-path line (the cap for long and repeat chains) | `rm_16_a_stop_lowers_a_finite_front_to_its_bound` (40-sample head) | the killer uses a long head, so it stays valid |
| N47S5-09 | B1 | `front.max(stop)` | re-anchor only if the Q1 text changes it; mutant `front.min(stop)` kept | `rm_16_second_stop_that_drops_a_booking_extends_the_region_to_the_hull` (545, setup changed) | Sol: the killer's setup changes with Q1; the implementer confirms the mutant still fails it |
| N47S5-67 | B1 | `Some((start, front)) => (start.min(t), front.max(stop)),` | re-anchor if Q1 changes it; mutant `(t, front.max(stop))` kept | as N47S5-09 | as N47S5-09 |
| N47S5-159 | B1 | (unchanged `old`) | (unchanged) | renamed to `the_record_front_is_ten_samples_past_the_transmitted_end_of_a_head` | filter rename only |
| N48S1-08 | B2 | `self.chain.advance_front(clock.domain, open.next.saturating_add(pad as i64 + 1));` | the discriminator line (`device_sob && device_count <= 32` → `+ 10`, else `+ 1`) | `ur_23_a_one_sample_burst_restarts_after_its_padded_device_burst` | mutant `open.next.saturating_add(1)` kept, re-anchored |
| N47S4-09b | B2 | `open.next.saturating_add(pad as i64 + 1)` | the same discriminator expression | `ur_23_a_value_waits_for_a_padded_one_sample_burst` (clock to 20 015) | mutant `open.next.saturating_add(1)` kept |

**Unchanged and still killers:** N47S5-04 (held tail `checked_add(2)`), N47S5-63/-64 (region terms, re-checked for uniqueness after Q1), N47S5-69 (`c.start >= t && c.start < stop`), N47S5-112 (`front(k, len, join.is_some())?`), N47S5-141 (`c.front = front(c.start, Some(own), false)…`), N47S5-145 (`c.front = c.front.max(pieces)`), N47S5-60 (stat), N47S5-29/-30 (halt calls), N47S4-09a (`previous_end` read; checked at B2 whether its test still asserts the old front), N47S4-09c (Stop's `close_start + 3`, unchanged), N48S1-07 (`pad` expression), N48S3-23 (the idle timed refusal line; the guard is added on the line before, the row's `old` is re-checked for uniqueness), N48S3-25.
**Re-designed killers (M2, rev 3; anchors unchanged, each test is the row's only killer):** N47S5-55 → staging 563; N47S5-84 → staging 666; N47S5-23 → mock_channel 535; N47S4-09a → tx 6307 (§3.2). The §4 claim that N47S5-55 stayed killed by 563 is withdrawn: Q1 changed the front the test observes (§2). N47S5-67 is killed by the hull probe at 40 (§3.1).

**New rows (B1):**
- **N49B-01** `checked_add(10)` → `checked_add(9)` on the 1–32 arm; filter `rm_15_closed_burst_front_is_end_plus_ten`.
- **N49B-02** `Some(len) if joined || len > 32` → `… len >= 32` (boundary); filter `rm_15_closed_burst_front_is_end_plus_ten` (the 32 case).
- **N49B-03** the kept-path rule disabled (an uncut short head capped again at `stop`): new = the old `c.front = c.front.min(stop)` on a standalone short head. Filters: `rm_16_a_stop_keeps_an_uncut_short_head_s_front` and `rm_16_a_stop_inside_a_32_sample_head_keeps_its_front` (N-S5c, the counterexample; the mutant gives 104 for B, which is admitted, so it fails).
- **N49B-04** the dropped-path region reaches only `stop` (Q1 off): new = `stop`, old = the new own-front expression. Filter `rm_16_a_stop_that_cuts_a_short_head_adds_its_front_to_the_region`.
- **N49B-11** (B1, m3) the length bound `end <= start + 32` in the kept-path predicate → `end <= start + 33`. Killer: N-S5d (window 0).
- **N49B-12** (B1, m3) the `joined.is_empty()` term dropped from the kept-path predicate. Killer: N-S5e (window 0).

**New rows (B2):**
- **N49B-05** owner discriminator dropped on the short branch (the owner always uses `+ 1`, the pre-step behaviour). Filter `ur_23_a_closed_short_burst_s_owner_front_is_ten_past_its_end`.
- **N49B-06** FakeDevice guard constant `10` → `9` (the `short_until` addition). Filter `ur_33_a_timed_sob_within_ten_of_a_closed_short_burst_is_refused`.
- **N49B-07** FakeDevice: the `burst_samples <= 32` condition dropped (every closed burst sets `short_until`). Filter `ur_33_a_closed_burst_over_32_keeps_the_gap_of_one`.
- **N49B-08** FakeDevice: the refused open SOB does not set `Burst::Timed` (the state line removed). Filter `ur_33_a_refused_open_sob_refuses_its_device_burst_through_its_eob`.
- **N49B-09** FakeDevice: `burst_samples += n` → `burst_samples = n` (last packet only). Filter `ur_33_the_short_guard_counts_the_device_burst_not_its_last_packet`.
- **N49B-10** owner discriminator without the SOB condition (`device_count <= 32` alone). Filter `ur_23_a_continuation_that_closes_a_burst_keeps_the_front_one_past_its_end`.

Counts (rev 3): rows re-anchored or renamed: 9 (N47S5-02, -03, -09, -14, -62, -67, -159, N48S1-08, N47S4-09b; -09 and -67 only where their text changes). Rows whose killer is re-designed with an unchanged anchor: 4 (N47S5-55, -84, -23, N47S4-09a). New rows: 12 (B1: N49B-01–04, -11, -12; B2: N49B-05–10).

## 5. Test counts

- **B1 new tests (10):** staging 6 (N-S4, N-S5a, N-S5b, N-S5c, N-S5d, N-S5e); parity 3 (N-P1, N-P5, N-P6); mock 1 (N-M5). Optional: the iii test of §3.4.
- **B1 changed tests (31):** staging 16 (337, 347, 395, 545, 563, 585, 666, 706, 862, 871, 884, 914, 922, 931, 940, 960); parity 5 (1502, 1551, `receipt_order_cases`, 1662, 2173); mock 4 (mock_channel 620 and 535; mock_radio `x3xx_move` and 1264); iii 2 (1268, 1281); tx 4 (2095, 3135, 3238 receipt part, 6307). Comment-only: 2 (436, 462).
- **B2 new tests (8):** tx 2 (N-O7, N-O7b); device.rs 5 (N-F8a–e); hardware 1 (N-H1, `#[ignore]`).
- **B2 changed tests (10):** tx 7 (2389, 3022, 3238 play part, 5134, 5289, 5364, 6330); hardware 3 (comments 3669/3684; 3708–3735 gaps; 3756–3795 R5-m5 gap 0 → 10); fake.rs 0 (the rehearsals move with the record). The B2 run re-checks staging/parity/mock green.

## 6. Open questions for the orchestrator

- **Q1 (decided, rev 3).** A dropped standalone short head extends the region to its own front only when `start < H`, `H = stop − 3`, the handover horizon. Reason: the handed cursor is at most `stop − 3` (`halt`'s comment), so a booking in `[H, stop)` cannot have been handed, and the record already decides "may be handed" with the same horizon (`handover_horizon`, `staging.rs:615`). `start < stop` would also refuse a booking the Provider never sent, an availability cost with no safety gain. Implementation: `halt` takes its horizon from the margin `stop_sample` and `handover_horizon` use (`staging.rs:604`, `615`); the implementer picks the smaller diff. Reachable at production windows: a short head with start in `(H − 11, H)` (§0; N-P5 (i), N-P6). Rejected: `start < stop` (wider than needed).
- **Q2 (rejected, ruling F1).** Payload-end test not used. A kept standalone short head keeps its own front. N-S2 dropped.
- **Q3 (decided).** The re-front in `remove_head` follows from `front()`; B1 confirms `tx.rs:2962` and `2989`. The configure-refusal device outcome (`tx.rs:3022`) changes in B2 (§2 point 6).
- **Q4 (decided, rev 3).** The kept-path cap is killed at window 0 (N-S5a, N-S5c, N-S5d, N-S5e, N-M5). At production windows it cannot bite (§0). The window-edge parity tests (N-P5, N-P6) pin the reachable dropped-path extension and the kept-head agreement on both Providers.
- **Q5 (decided, F8, with the abandon exception).** The owner's discriminator is the closing send that is also the device burst's first send, with transmitted length at most 32 (`device_sob && device_count <= 32`). No new state. A device burst longer than 32 keeps `+ 1` (the pre-existing mismatch with the record's `+ 2`, documented by N-O7b). The abandon path is the one exception (§3.6, N-F8e). A KL entry only if the owner can send a booking the record refuses; none found.
- **Q6 (decided).** No FakeDevice exception for a short B after a short A (stricter than the hardware). Documented in UR-33 (§3.9) as a conservative model. No test claims that the hardware refuses gap 9 for a short B.
- **Q7 (decided).** R5-m5 books at c + 13 with gap 10 in the raw row. The 100-sample closed B stays a labelled short-B observation; the long-B regression N-H1 is added (F6, m5). The bench rerun is the owner's (confirm the attenuator is in the loopback before any transmit; AGENTS / handoff §2; this plan runs no bench).
- **NQ-A (ruled: no production hook).** The window-edge geometry (N-P5, N-P6) uses the existing test helpers only (`stop_at`, `stop_both`). If a geometry cannot be placed through them on both Providers, the implementer reports which test and stops that test; no production change.
- **NQ-B (ruled: yes).** B1 is a local commit on `work/stepB`; main receives one squashed commit after B2. The configure-refusal joined-piece path is therefore never on main in its B1 state; nothing in B1's commit message or KL text claims it fixed.

## 7. Commands for each sub-step (remote only)

- B1: `python3 plan/maintenance/tools/remote/rdev.py -- cargo test --workspace` (m4); `python3 plan/maintenance/tools/remote/rdev.py -- cargo test -p ezsdr-radio --test staging`; `… -- cargo test -p ezsdr-acceptance --test parity`; `… -- cargo test -p ezsdr-mock-radio --test mock_channel --test mock_radio`; `… -- cargo test -p ezsdr-radio-uhd --test fake` (the full binary, which runs the iii tests including the Stop test; a filter of `the_record_front` misses it); `… -- cargo test -p ezsdr-radio-uhd --lib` (the tx.rs tests, `ur_21_s1_overlap_refused_at_receipt`, `ur_23_stop_does_not_send_unhanded_window_edge_payload`, the receipt part of `ur_23_a_one_sample_burst_restarts…`); then `… -- cargo test -p ezsdr-radio-uhd --features uhd --test hardware --no-run` (compile only).
- B2: `… -- cargo test -p ezsdr-radio-uhd --lib` (tx.rs tests); `… -- cargo test -p ezsdr-radio-uhd --lib device` (FakeDevice unit tests); `… -- cargo test -p ezsdr-radio-uhd --test fake`; `… -- cargo test -p ezsdr-radio-uhd --features uhd --test hardware --no-run`.
- After each sub-step: `cargo fmt --all`, then `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base <step-A commit>` = 0. Mutation rows only through `remote_mutate.py`.

## 8. Greps run for this plan (read-only)

- `grep -rn "checked_add(1)\|len.max(4)" crates/ezsdr-radio/src`: `staging.rs:590` (the only hit in the record's front).
- `grep -n "pad as i64\|advance_front(\|device_count" crates/ezsdr-radio-uhd/src/provider/tx.rs`: `tx.rs:685–686` (`pad`, `device_count`), `:800` (`pad as i64 + 1`), `:835`, `:1009` (`+ 3`), `:910`.
- `grep -n "fn packet\|self.burst = Burst::Clean;\|!self.active && packet.radio_time\|fn faulted\|if packet.eob && !packet.lost_eob" crates/ezsdr-radio-uhd/src/device.rs`: `:667`, `:669`, `:689`, `:722`, `:740`.
- `grep -rn "head_front" crates/`: `iii.rs:677, 1269–1276`; `n47s5.rs:158, 177, 314, 338`; `hardware.rs:3669, 3684, 3711`.
- `grep -rn "tx_close_past_record_front\|past_front(" crates/`: `tx.rs:552, 2865, 2886, 2907, 2934, 3067, 3076, 4695–4696`.
- `grep -n "plus one\|plus five\|end plus\|start plus\|one sample past" design/*.md KNOWN_LIMITATIONS.md`: RM-15 (`design/07:162`), UR-22 (`design/18:253`), UR-23 (`design/18:260`), UR-33 (`design/18:342`, `:351` cited by number only), KL-36 (`:136–137`), KL-13 (`:130`, `:132`), KL-38 (`:151`); MR-17 (no number) and MR-25 (`design/09:171`) state the Stop text. Changes rows in `design/18:735, 751`, `design/09:422, 434, 437` are history and stay as they are.
- `grep -rln "rm_15_… / ur_… / s1_parity_… names" design KNOWN_LIMITATIONS.md plan`: design and KL cite only `ur_21_s1_overlap_refused_at_receipt`, `ur_23_a_one_sample_burst_restarts`, `ur_22_a_short_first_send_is_padded`, `ur_23_stop_does_not_send`, `rm_15_an_untimed_booking_is_placed_after_the_booking_it_meets` (KL-36), `s3_parity_booking_after_a_closed_short_burst_follows_d5` (design/09): none is renamed, so `rules.py` needs no `--renames` entries in B1 or B2.
- mutations.json: Python scans of `old`, `new` and `filter` for the record's front code, the owner's `pad` and `advance_front`, the FakeDevice refusal line, and the test names of §3 (the rows of §4 were read in full).

## 9. Not done in this launch

No repository edits, builds, tests, commits, pushes or `mutate.py` runs. The new test bodies and the anchors of the new and re-anchored rows are fixed at implementation, after the NQ-A and NQ-B rulings and the Q1 predicate (all given in revision 3).
