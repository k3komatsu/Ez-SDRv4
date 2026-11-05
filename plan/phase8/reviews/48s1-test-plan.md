# Test plan: design note 48 S1 (#78; fixes (a)–(c)) — rev 9 (fix round 3 record in §17; rev 8 is `test-plan.v8.md`, rev 7 is `test-plan.v7.md`, rev 6 is `test-plan.v6.md`, rev 5 is `test-plan.v5.md`, rev 4 is `test-plan.v4.md`)

Implementer: `anthropic/claude-haiku-5-5#high` (OpenCode2). Base: `main` HEAD `1f120dc`. Rev 3 was written at `6ccc1c5`. The two commits since (`3b63d5a`: the bench record and the repro program; `1f120dc`: the owner's B1 decision in `plan/phase8/48-simplification-inventory.md`) touch no Rust code. Clean tree, no tree changes. Paths are relative to `~/local_work/Ez-SDRv4`. Rev 2 is `test-plan.v2.md`; rev 3 is `test-plan.v3.md`.

Binding inputs, in order: `rulings-plan2.md` (the owner's B1 drop; rulings N1–N5; minor items), `rulings-plan.md` (R1–R10, Sol's review of rev 2; applied in rev 3 and kept here unless §0 says otherwise), `hardware-facts-cpp.md` (**repro 1–4**: its items 1–4; **A1**: addendum 1; **A2**: addendum 2).

Labels: **VERIFIED** = a path:line, URL, commit, bench log or repro run I read; **INFERRED** = a reasoned step, not measured. Lines read in rev 4 are cited as VERIFIED; **†** = a rev 2 line number not re-read.

---

## 0. Summary and ruling map

A2 (addendum 2) governs: a device burst's **first send carries at least four samples per channel** (`MIN_FIRST_SEND`). A first send of one to three samples is **never held back**: it goes at once as one timed SOB+EOB send padded with zeros to four (N1). Continuations have no minimum, and a one-sample continuation close is padded to two as today (N2). **Part (d) is out of S1** (owner, 2026-11-05): every send stays timed. R1 stands: the two-sample held-tail close and its front arithmetic are unchanged.

| Ruling | Where in rev 4 |
|---|---|
| Owner: B1 dropped from S1 | §6 (replaced by pins), §11 (no B1 rows), §12 (UR-22 T1 kept; reason in its Changes row) |
| N1: no waiting first send; `tx.rs:552` exemption generalised | §2 FS-2, FS-3, case table; §7; §12 |
| N2: continuation one-sample EOB padded to two (`padded_eob` kept) | §2 FS-7; §4.3; §9.1; §11 (P1171 retargeted) |
| N3: short burst then adjacent burst, both orders | §2 FS-9; §7; §9.3; §12 (`KNOWN_LIMITATIONS.md:288`) |
| N4: BurstAck correlation strict; F8-e at ratio 1; UR-28 says so | §1 F-33; §8.4; §11 (TBD-n4); §12 |
| N5: first send's `wanted` raised to four; no refusal for a small window | §2 FS-3, FS-4; §7; §15 Q3 |
| R1 (b), R2 (c), R3 (a), R4 (UR-33), R5 (MR-17/25), R6 (late first send, F8), R7 (rows), R8 (specs), R9 (hardware), R10 (print) | as rev 3, updated in §§2–13 |
| Minor (rulings-plan2) | padding tests named with values and zero positions (§2.4); F4-far and §8.2 wrong-behaviour cells filled (§4.3, §8.2); TBD-b4 continuation killer (§7); TBD-d2 test named (§8.4); P1173 and TBD-a3 merged (§3.4); `tests/common/probe_device.rs` path (§1 F-28, §3.3); F1 read order (§3.2); M1 drops through the EOB (§8.1); §12 decides the Q1 Ceiling; UR-22 one-sample-close text (§12) |

Part status:

| Part | What is wrong (VERIFIED) | Change in rev 4 | Still open |
|---|---|---|---|
| (a) | The two-zero T3 flush draws ~9 Underflows before its ACK (repro 1; hw #1). | Four-zero flush (A1 and repro: four is clean); per-channel attribution of flush-window reports (defence). | none |
| (b) | Hardware test expects B at `e+1` to play; the device and the close say `e+1` is gap 0 (repro 3). | R1: retarget the test to `e+2` plays, `e+1` refused as late. No Module change at the close. | none |
| (c) | `hw_p70`: the one-sample trial's device burst is a padded SOB+EOB of two; the control's first send is 1 sample (split 1+2). A2: first packets under four are lost at 1–2 MS/s. | N1–N5: first send ≥ 4 (FS-1), a short burst padded to four and sent at once (FS-2), no waiting for burst length (FS-3). | Q7 (padded send at booked lead), Q1 (decided, §15) |
| (d) | Every payload send carries `at`. | Out of S1. Every send stays timed; the late-first-send test (F-own-1) pins the timed behaviour. | none in S1 |

---

## 1. Facts this step relies on

Rev 2's F-1 to F-23 stand unless noted; rev 3's F-24 to F-32 stand. Changes and additions in rev 4 are marked *(rev 4)*.

| # | Fact | Label | Source |
|---|---|---|---|
| F-1 | Flush: `tx_flush_locked` sends `[Iq::default(); 2]` untimed, SOB+EOB (`core.rs:1107`). Non-ACK reports go to `stash_tx_report` (`core.rs:1138`, `:1149`) and become Run events through `Tx::reports`. `open_device_locked` clears the stash (`core.rs:1247`†). | VERIFIED | code (`core.rs:1107`, `:1113` read in rev 4) |
| F-2 | Bench #1 `hw_b6_txrx_and_repeat`: async `[BurstAck 672710 tx_flush, Underflow 672320 … 672702 (9×, ~48-tick spacing), BurstAck 404809601]`; Run `TX_UNDERFLOW starved` ×9. | VERIFIED | `hw_b6_txrx_and_repeat.36.log` |
| F-3 | Same nine-then-ACK pattern in `hw_b8_burst_at_a_sent_burst_s_end` and `hw_b8_burst_one_sample_after_a_burst`, and in the 2026-10-28 raw P5. | VERIFIED | those logs; notes.md |
| F-4 | Phase-7 async for those two tests had no Underflow (two `BurstAck`s). | VERIFIED | `plan/phase7/bench-results.md:937–940` |
| F-5 | Phase 7: timed SOB at the previous burst's end: last-buffer EOB → gap 0 late 4/4, gap 1 OK 0/4 late; one-zero EOB → gap 1 late 4/4, gap 2 OK 0/4 late. | VERIFIED | `plan/phase7/bench-results.md:882–889` |
| F-6 | "The X300 drops a burst whose timed start is the tick its previous burst ended." | VERIFIED | `plan/phase7/bench-results.md:550`; `KNOWN_LIMITATIONS.md:288` (read in rev 4) |
| F-7 | Phase-7 `hw_b8_burst_one_sample_after_a_burst` (A = 30 000 samples, B at A's end + 1, 4/4): async `[BurstAck 415556401, BurstAck 415756601]`. A's one-sample tail EOB was acknowledged. B played. | VERIFIED (ack); INFERRED (B played) | `plan/phase7/bench-results.md:937–940` |
| F-8 | Pre-`80dd2f6` close: a one-sample held tail alone; `safe_gap = if tail {1} else {2}`. | VERIFIED | `git show 80dd2f6^:crates/ezsdr-radio-uhd/src/provider/tx.rs`, `end_open` ~601–612 |
| F-9 | Current close: `close_device_burst` always sends two samples (held tail + zero, or two zeros) timed at `first` (`tx.rs:742–780`; `let at = Some(clock.instant(first));` at `tx.rs:757`); the front is `close_start + 3` (`tx.rs:727`). | VERIFIED | code (`tx.rs:757` read in rev 4) |
| F-10 | #70 (phase 8): a one-sample SOB+EOB burst gets no BurstAck; the next timed burst starts ~150 ms early. The control was an untimed two-zero burst followed by a timed one: fine. | VERIFIED | `plan/phase8/bench-results.md:262–264` |
| F-11 | FakeDevice: a one-sample EOB is lost only if `n == 1 && (sob \|\| !was_burst)` (`device.rs:621`). No rule for 2 or 3 samples. | VERIFIED (model) | `device.rs:602–654` |
| F-12 | UHD prints one `L` per device TimeError packet; phase-7 late bursts show 33–35 `L`. | VERIFIED (log pattern) | `plan/phase7/bench-results.md:413–417, 538, 885` |
| F-13 | DUC: a continuation packet carries the DUC's counted time until a packet without one begins. | VERIFIED | `design/18-uhd-radio.md` UR-22/23 context; `device.rs:614–642` (model) |
| F-14 | `hw_p70` 2026-11-05: onset 25 (one-sample) and 25/26 (control) samples from the booked instant in both trials; `TIME_ERROR 1`, `TX_UNDERFLOW 10/11`; the log prints `10 U, 32 L` (trial 1) and `9 U, 32 L, 1 U` (trial 2). | VERIFIED | `hw_p70_…36.log`, `…37.log` |
| F-15 | `hw_b8_burst_at_a_sent_burst_s_end` trial 0 (B at A's end): `TIME_ERROR []`; the test failed only on its `async` Underflow assertion (`hardware.rs:1502`†). | VERIFIED (no TimeError); playback INFERRED | `hw_b8_burst_at_a_sent_burst_s_end.36.log` |
| F-16 | Repro 1–2 (UBX #2, UBX/OBX #1, 1 MS/s, five trials per shape): untimed two-zero SOB+EOB: 8–9 Underflow, then BurstAck (5/5). Timed two-sample SOB+EOB 100 ms ahead: dropped, 32 TimeError, 1 Underflow, no BurstAck (3/3). 4, 8 … 512 samples, timed or untimed: clean, BurstAck at exclusive end + 1 tick (3/3 each). | VERIFIED | `hardware-facts-cpp.md` item 1; `tx_repro.cpp`, `run*.log` |
| F-17 | Repro 2: a 9 999-sample burst (first send timed, continuations untimed) closed by a separate two-sample EOB (held tail + zero) is played; ACK at the close's exclusive end + 1. | VERIFIED | repro item 2 |
| F-18 | Repro 3: after that close, B (1 000 samples, timed) at the close's exclusive end (gap 0) → dropped (≈63 TimeError, no ACK); at end + 1, + 2, + 3, + 1 000 → plays, ACK at its end + 1 (5/5 each). | VERIFIED | repro item 3 |
| F-19 | Repro 4: the #70 shape: a 2-sample burst at t, then 10 000 samples at t + 25 µs: the 2-sample burst is dropped (32 TE, no ACK); the second plays on time, identically whether its continuations are all timed or only the first. | VERIFIED | repro item 4 |
| F-20 | Mechanism of F-16 (1): a very short burst stays in the DUC's interpolation pipeline until later samples push it out; timed → drop, untimed → Underflows. | INFERRED | repro item 1 |
| F-21 | Module shapes (today): a Kernel burst of one sample is not held (`!(device_sob && count == 1)`, `tx.rs:552`, read in rev 4) and goes as one SOB+EOB padded to two (`padded_eob`, `tx.rs:576`). A burst of two or more booked more than `DEVICE_LEAD_NS` ahead is held (`tx.rs:551–552`): the first chunk goes as SOB without EOB, the held sample later with the close. A short open-ended burst therefore reaches the device as a **split** (e.g. 1 + 2). *(rev 4: N1 generalises the exemption to `count <= MIN_FIRST_SEND`.)* | VERIFIED (code) | `tx.rs:529, 551–552, 562, 576–580, 593, 691, 742–780`; `profile.rs:11` |
| F-22 | `hw_p70`'s control (`one_sample_trial(…, 2, …)`, `hardware.rs:2880–2904`†) is a timed two-sample burst booked 40 ms ahead, held: the device sees SOB 1 + EOB 2 (total 3). The one-sample trial's device burst is SOB+EOB 2 (padded). | VERIFIED (code path); packets INFERRED until the §5.4 print | `hardware.rs:2880–2904`†; F-21 |
| F-23 | MockRadio: front after a handed payload end is `end + 2` (`mock-radio/src/lib.rs:1369`); the cut and Stop fronts are `cut + 2` and `stop + 2` (`:1131`, `:1751`); `advance_front` keeps the maximum (`tx.rs:167–170`). | VERIFIED | code |
| F-24 | **A2 (VERIFIED):** device bursts with a first send under 4 samples fail at 1 and 2 MS/s: 1+2, 1+3, 2+2, 2+3, 3+2 (12 trials each): whole-burst drops (TE 32, no ACK), partial TE (6–24), Underflow (1–14), ACK off its end by −3…+53 ticks. **4+2: clean in 12/12**, ACK at end + 1. | VERIFIED | `hardware-facts-cpp.md` A2; `uhd-repro/run5-*.log` |
| F-25 | **A1 (VERIFIED):** a standalone device burst of ≥4 samples is clean at 1, 2, 10, 50, 200 MS/s on both boards. 2 and 3 samples: timed dropped at 1–2 MS/s and clean at 10 MS/s and above; untimed 2–3 give Underflows at 1–10 MS/s. Every timed BurstAck is at end + 1 tick (126/126; the rates are not split). | VERIFIED | `hardware-facts-cpp.md` A1; `run4-*.log` |
| F-26 | Untimed four-sample bursts are clean at 1 and 2 MS/s (repro 2, F-16) and at 1–200 MS/s (A1, ≥4 samples). The four-zero flush itself was not run as such on the bench; the untimed four-sample shape is the measured one. | VERIFIED (what was run) | repro items 1–2; A1 |
| F-27 | The reopen front is `at_or_after(ack_tick) + 2` (`tx.rs:800–803`). The fake ACKs at the last sample's tick (`device.rs:724`, read in rev 4), so today's fake front is end + 1. Hardware ACKs at end + 1 tick, so the same formula gives end + 2 samples on hardware (INFERRED). | VERIFIED (code); INFERRED (consequence) | `tx.rs:800–803`; `device.rs:724`; A1 |
| F-28 | `crates/ezsdr-radio-uhd/tests/common/probe_device.rs:512` (read in rev 4) classes a probe row as the T3 flush by `channel.len() == 2` and all-zero samples. A four-zero flush is not classed unless that changes. `tests/hardware.rs:2032–2033` compares device sample counts of non-flush rows with payload. *(rev 4: the path is `tests/common/`, not `src/common/`.)* | VERIFIED | `tests/common/probe_device.rs:512`; `tests/hardware.rs:2032–2033` |
| F-29 | `design/18-uhd-radio.md` UR-28: "A `BurstAck` marked `tx_flush: true` is recorded but does not retire a device-burst correlation"; and "retires, oldest first, every pending burst on that channel whose first instant is at or before `t`". `remember_burst` is called at device-burst starts only (`tx.rs:613, 632, 665`), not for every Kernel burst. | VERIFIED | `design/18-uhd-radio.md` UR-28; `tx.rs:355–362, 613, 632, 665` |
| F-30 | `Duc::packet` clears `has_time` and `time` at an untimed packet (`device.rs:639–642`). A later untimed packet therefore carries no time. *(rev 4: no S1 rule depends on it, since every send stays timed; kept as the model's fact for the late-first-send pin, §6.3.)* | VERIFIED (model) | `device.rs:639–642` |
| F-31 | The flush's `Other(77)` report is stashed today and tested so (`core.rs:1649–1669`†). UR-28: "`Other` is recorded only." | VERIFIED | `core.rs:1649–1669`†; `design/18-uhd-radio.md` UR-28 |
| F-32 | Repeat bursts (`open.held.repeat`) have no end; `eob` is never set by `ends_waveform` for a repeat (`tx.rs:534`). | VERIFIED | `tx.rs:534` |
| F-33 | *(rev 4, N4)* The BurstAck correlation retires a pending burst when `root_tick <= tick` (`tx.rs:373`, read in rev 4; the ruling's `:370` is this block). At 200 MS/s (ratio 1) an ACK at the previous burst's exclusive end + 1 tick (F-25) equals the next burst's first instant when that burst starts at end + 1 sample, so today's comparison retires the next burst's pending entry. | VERIFIED (code and ratio arithmetic); INFERRED (the bench effect at 200 MS/s: A1 reports ACK ticks only at 1–2 MS/s, the 200 MS/s rows as "clean") | `tx.rs:373`; `tx.rs:355–362`; A1 |
| F-34 | *(rev 4)* The FakeDevice's BurstAck tick today is the last sample's tick (`device.rs:724`: `Some(self.cursor.saturating_sub(packet.ratio))`). M2 (§8.1) moves it to the exclusive end + 1 master tick; that `cursor` is the exclusive end in master ticks is INFERRED, to be checked at step 4. | VERIFIED (code); INFERRED (cursor meaning) | `device.rs:724` |

---

## 2. The first-send rule (R2 with A2; N1, N2, N3, N5)

Terms: a **first send** is the send that starts a device burst (`device_sob`, `tx.rs:528`). **`count`** is the payload a chunk would take before the hold; **`sending`** is the payload after the held tail is withheld (`tx.rs:554`); **pad** is the zeros added to a single closing packet. `MIN_FIRST_SEND` = 4 per channel: a constant with a one-line comment citing the 2026-11-05 measurement; a hardware envelope fact of X3x0 (UR-22 and KNOWN_LIMITATIONS), not a profile key.

### 2.1 Rules

| Rule | Text (the implementation contract) |
|---|---|
| FS-1 | A first send carries at least `MIN_FIRST_SEND` = 4 device samples per channel. Either (i) one packet with EOB, padded with zeros to at least four (FS-2), or (ii) a packet without EOB whose `sending` is at least four. A first packet with fewer device samples is never made. |
| FS-2 | **Never held back (N1).** A first send of 1–3 payload samples goes at once as one timed SOB+EOB packet padded with zeros to four. The zeros are not payload: `header.len`, `tx_samples`, the burst records and the probe's `sample_counts` count payload only. |
| FS-3 | **Hold exemption and window wait (N1, N5).** The exemption at `tx.rs:552`, `!(device_sob && count == 1)`, becomes `!(device_sob && count <= MIN_FIRST_SEND)`: a first send of at most four is not held. An open-ended first send of four goes as SOB 4 without EOB; the device burst stays open. A first send needs `min(count, 4)` samples of `avail`: `wanted` is raised to four when the chunk is smaller (N5), and the first-buffer exception `if avail < wanted && !device_sob` (`tx.rs:529`) no longer lets a first send take fewer than four. The wait is for window space only; it is never for the burst's length, and there is no refusal for a small window (N5). |
| FS-4 | **Cut (retained from rev 3).** If a cut leaves fewer than four samples before the burst's first sample, no first send is made on that clock. The existing UR-25 path cancels the old-clock reservation, a `rejected` note records `"UR-22: the first send needs 4 samples per channel before the clock's cut"`, and no TIME_ERROR is emitted. (The window refusal of rev 3 is removed, N5.) |
| FS-5 | Late first send: `dispatch_start` and the late policy (`tx.rs:905–953`) decide before the first send, as today. Stop and loss while the first send waits cancel, as today. |
| FS-6 | A repeat's first send tiles its waveform from the current offset to at least four samples. The tiled samples are payload: the Kernel advances by their count and `header.len` counts them. No zero padding. |
| FS-7 | **Continuations (N2).** A Kernel burst that continues an open device burst has no minimum. Its one-sample EOB is padded to two, as today (`padded_eob`, `tx.rs:576`; F-7, F-21). The pad term splits by `device_sob`: a first send pads to four (FS-2), a continuation keeps `padded_eob`. |
| FS-8 | The two-sample held-tail close (`close_device_burst`) is unchanged (R1). |
| FS-9 | **Adjacency (N3).** A padded short first send closes its device burst. A Kernel burst that starts at its payload end or any instant before the padded front is not continued: it is decided against the padded front by its own late policy (TIME_ERROR drop or `send_asap`, `tx.rs:905–953`). Both orders are tested (§7): the adjacent burst held before the short burst's dispatch, and booked after it. |

### 2.2 Case table

A finite single Kernel burst, or an open-ended one, that starts a device burst. L = payload samples; `w` = its samples; channel 0 shown.

| L | Kind | Device packets | Front (safe start) | Rule |
|---|---|---|---|---|
| 1 | any | at once: one SOB+EOB of 4, `[w0, 0, 0, 0]` | start + 5 | FS-2 |
| 2 | any | at once: one SOB+EOB of 4, `[w0, w1, 0, 0]` | start + 5 | FS-2 |
| 3 | any | at once: one SOB+EOB of 4, `[w0, w1, w2, 0]` | start + 5 | FS-2 |
| 4 | finite | one SOB+EOB of 4, `[w0 … w3]`, no pad | start + 5 | FS-1 |
| 4 | open-ended | SOB of 4 without EOB, `[w0 … w3]`; the device burst stays open | none until its close | FS-3 |
| 5 | open-ended, or finite booked ahead | SOB of 4 without EOB, `[w0 … w3]`; then `w4` held and closed with the held tail + zero (two samples) | close end + 1 (R1) | FS-1, FS-3, FS-8 |
| 5+ | finite, not booked ahead | one SOB+EOB, whole burst, no pad | end + 1 | FS-1 |

The front column is the device interval's end + 1, computed from the packets as sent, padding included. For a single padded send it is `start + device_count + 1`; for a held close it is `close_start + 3` (R1, F-9, unchanged). In code, `tx.rs:691` (`open.next.saturating_add(if padded_eob { 2 } else { 1 })`, read in rev 4) becomes `open.next + pad + 1`, with `pad` = `device_count − sending`: 0 for the held close, 1 for a one-sample continuation EOB (front +2, as today), `4 − sending` for a first send. A short open-ended burst closed at once (L = 1–3) makes its later samples an adjacent burst (FS-9); this is the literal reading of N1 and is listed in §15 Q5.

*Consequence (INFERRED).* A booked-ahead short burst is sent at once, at its booking, about one device lead (100 ms in repro 1–4) before its start. Repro 1–4 measured dropped 2- and 3-sample bursts at that lead; the padded 4-sample packets were clean at the same lead (repro item 1 and A1: 4-sample bursts are clean; the lead for those runs is INFERRED to be the same). The Module-path bench print checks it (§13).

### 2.3 Tests of the first-send rule

| Seq | Test (file, name) | Asserts | Observed through | Wrong behaviour that fails it |
|---|---|---|---|---|
| FS-T1 | `tx.rs` new `ur_22_a_one_sample_burst_is_padded_to_four` | payload `[w0]`; device `[w0, 0, 0, 0]`, SOB and EOB, timed at its start; front `start + 5` | `calls` (`sample_counts`), `chain.front()` | Two-sample pad (today) or no pad |
| FS-T2 | `tx.rs` new `ur_22_a_two_sample_burst_is_padded_to_four` | `[w0, w1, 0, 0]`; front `start + 5` | as FS-T1 | Two-sample first packet |
| FS-T3 | `tx.rs` new `ur_22_a_three_sample_burst_is_padded_to_four` | `[w0, w1, w2, 0]`; front `start + 5` | as FS-T1 | Three-sample first packet |
| FS-T4 | `tx.rs` new `ur_22_a_padded_send_counts_payload_only` | `header.len` and `tx_samples` = L; the burst record's payload = L | `rec.bursts`, `tx_samples` | Zeros counted as payload |
| FS-T5 | `tx.rs` new `ur_23_a_one_sample_continuation_eob_is_padded_to_two` (N2) | continuation `[w0, 0]` (two packet samples); front `open.next + 2` (= end + 2, as today) | `calls`, `chain.front()` | A continuation padded to four, or no pad |
| FS-T6 | `tx.rs` new `ur_23_a_four_sample_open_ended_first_send_is_not_held` (N1) | one device call: SOB, n = 4, no EOB, timed; the device burst is still open; samples contiguous with the next chunk | `calls`, capture | A held tail splitting 4 into 3 + 1 (the old exemption `count == 1`) |
| FS-T7 | `fake.rs` `ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time` (`fake.rs:3315`†) | unchanged assertion: it plays at its time, with ACK at its padded end + 1 tick. Its `n` changes from 2 to 4 | `calls`, `async` | — (its `n` expectation is updated at step 4) |

Existing tests whose expectations change by N1–N2 (listed for step 4): `tx.rs:2728` (renamed, §4.3); `tx.rs:4014` `ur_22_a_failed_one_sample_padded_eob_is_not_safe_until_flush_recovery` (if its burst is a first send, its pad changes from 2 to 4 and its front to `start + 5`; a continuation keeps two); `fake.rs:3315` (FS-T7). `device.rs:2784` and `device.rs:3274` (the one-sample EOB stale-header and no-ACK rules) are unchanged (M4).

---

## 3. Part (a): the T3 flush's Underflow reports

### 3.1 Rule (R3)

- **Shape.** The open-time and recovery flush sends four untimed zero samples as one SOB+EOB (`at = None`). Replace `[Iq::default(); 2]` (`core.rs:1107`, read in rev 4) with four. The ACK check `Ok(2)` (`core.rs:1113`, read in rev 4) and its message become four.
- **Attribution (defence).** Inside `tx_flush_locked` (`core.rs:1102–1181`), a non-ACK report is the flush's only when **all** hold: (i) its code is `Underflow` or `TimeError`; (ii) its channel is a flush channel not yet acknowledged (per channel: an ACK of channel *c* closes *c*'s window); (iii) it is read before the flush's end: either all flush channels are acknowledged or the deadline `until` (`core.rs:1110`, `:1137`) passes. Such a report is recorded as a raw async row with `tx_flush: true` (`record_tx_async`, `core.rs:1084–1088`). It emits no `radio.TX_UNDERFLOW`, is not stashed, and never reaches `Tx::report`.
- **Everything else keeps today's handling**: other codes (`Other(_)`, F-31); duplicate BurstAcks (the `else` branch, stash); channels outside the flush; reports read after the deadline (stash, `core.rs:1138`); reports read after the channel's ACK and after the flush has ended (stash, so they are Run events).
- **Generation (R3, Q5 decided).** The flush reads only the queue of the streamer its own open just created. The device replaces the queue on open (`tx.rs:3261`, UR-28). No generation argument is added to `tx_flush_locked`; test F3 pins it.
- **Stream.** `tx_async(stream)` reads that stream's queue only (`core.rs:1135`).

### 3.2 Sequences and tests

| Seq | Test (file, name) | Asserts | Observed through | Wrong behaviour that fails it |
|---|---|---|---|---|
| F1-4 | `crates/ezsdr-radio-uhd/tests/fake.rs`, new `ur_22_a_flush_is_four_zeros_and_draws_no_underflow` | The flush `tx_send` has `n = 4`, `at = None`, `sob = true`, `eob = true`, all four samples zero. No `radio.TX_UNDERFLOW`; no `async` Underflow; the flush `BurstAck` row has `tx_flush: true` | `device.calls()`, `events`, `async` | The two-zero flush (`n = 2`) |
| F1 | `crates/ezsdr-radio-uhd/src/provider/core.rs` unit, new `ur_22_a_flush_underflows_are_recorded_as_the_flushs` (scripted source: a test-local `Device` wrapper around the spy of the `ur_22_t3_*` tests) | Fixed read order: (1) channel 0 `Underflow` → one `device_async` row with `tx_flush: true`; (2) channel 1 ACK; (3) channel 1 `Underflow` while channel 0 is still unacknowledged → stashed (a Run event later, no flush row); (4) channel 0 ACK → the flush completes | core state (`device_async`, `take_stashed_tx_report`) | Stashing the pre-ACK report (a Run leak); dropping it (row count); a post-ACK report of channel 1 counted as the flush's (the per-channel conjunct gone) |
| F1′ | `core.rs` unit, new `ur_22_t3_flush_attribution_is_per_channel_and_code_bounded` (grouped channels 0, 1; interleaved reports; a duplicate ACK on 0; an `Other(77)` on 1) | A duplicate ACK on 0 does not complete 1; `Other(77)` stays stashed; the flush completes only on both ACKs | core state | A duplicate ACK completing channel 1; `Other` recorded as a flush row |
| F2 | `fake.rs`, new `ur_22_a_burst_underflow_after_the_flush_is_the_runs` (flush, then a burst whose device burst starves: `FakeFault::TxReport(_, Underflow)` at a tick after the ACK, or a late host send) | Exactly one Run `TX_UNDERFLOW` (`cause: starved`) at the burst's tick; the flush rows unchanged; the burst is abandoned with its `UR-22: the device burst underflowed` note | `events`, `rejected` | Flush rows counted as Run events (2 instead of 1); the Run underflow lost (0) |
| F3 | `fake.rs`, new `ur_22_a_recovery_reopen_flush_has_no_run_underflows` (a failed EOB close → `reopen_streamer`; the reopen's flush gets a scripted Underflow) | No Run event from the new flush; its own report is a flush row; reports queued by the old streamer before the reopen are not in the new flush's rows (the queue is replaced, `tx.rs:3261`) | `events`, `async` | A new-flush report as a Run event; an old-queue report in the new flush's rows |
| O-a1 | `core.rs` unit, `ur_22_t3_flush_reports_after_the_deadline_stay_the_runs` (rev 2 name kept) | A report read after `until` is stashed and returned later by `take_stashed_tx_report` (Run's). The test asserts only that; it does not claim the report's origin | core state | A flush row for an after-deadline report |
| O-a2 | `core.rs` unit, `ur_22_t3_stop_interrupts_an_open_flush_wait` (`core.rs:1741`†), extended | Reports read before the stop are flush rows; nothing is emitted as Run | core state, events | A stop turning flush rows into Run events |
| O-a3 | `core.rs` unit, `ur_22_t3_loss_during_flush_records_its_reports` (new) | Device loss during the wait: flush rows recorded, `DEVICE_LOST` once, no `TX_UNDERFLOW` | events | Losing the rows, or emitting them |

Hardware: §13.

### 3.3 Existing tests that change

- `core.rs` `ur_22_t3_*` tests (`core.rs:1568–1775`†) and the `tx_send` count checks near `core.rs:1395`, `:1640`, `:1644`, `:1684`† assert the two-zero shape. They change to four zeros. Reason: the flush shape changes.
- `ur_13_arm_radiates_nothing` (killer of P1173) checks the flush's sample count. Changes to four.
- `crates/ezsdr-acceptance/tests/uhd.rs:368`: "The only send is the two-zero burst": four zeros (comment and assertion).
- `crates/ezsdr-radio-uhd/tests/common/probe_device.rs:512`: the T3 flush detector (`channel.len() == 2`) changes to four (F-28). Required; without it the hardware flush rows are not classed as flush.
- `crates/ezsdr-radio-uhd/tests/hardware.rs:2032–2033`: the sample counts of non-flush rows are compared with payload; padded first sends add zeros. The comparison uses payload (F-28).
- `fake.rs:2613` (`ur_28_underflows_are_tx_underflow`): unchanged, no flush fault.
- No test asserts a flush-Underflow Run event today, so no test is deleted for (a).

### 3.4 Mutation rows (final source; Rust token anchors; anchors fixed at step 4)

| Row | File | Final-source `old` | `new` (regression) | Killing test |
|---|---|---|---|---|
| TBD-a1 | `core.rs` | the attribution condition's per-channel conjunct, `!acknowledged.contains(&report.channel)` | the conjunct removed | F1 (channel 1's post-ACK Underflow becomes a flush row) |
| TBD-a2 | `core.rs` | the code bound, `TxCode::Underflow \| TxCode::TimeError` | `_` (any code) | F1′ (`Other(77)` becomes a flush row) |
| P1173 (retargeted; merged with rev 3's TBD-a3) | `core.rs` | `let zeros = [Iq::default(); 4];` (the four-zero text, `core.rs:1107`) | `let zeros = [Iq::default(); 2];` | F1-4; `ur_13_arm_radiates_nothing` |
| TBD-a4 | `core.rs` | `acknowledged.insert(report.channel)` in the ACK branch | `acknowledged.insert(report.channel) && false` | F1 (the flush is not acknowledged) |
| TBD-a5 | `core.rs` | the non-ACK `else` branch's record call | stash in place of record | F1 (Run leak) |

P1190 (the deadline stash, `core.rs:1138`) stays and is killed by O-a1.

### 3.5 Spec text (a)

- `design/18-uhd-radio.md` UR-22: the T3 flush is four untimed zero samples (SOB+EOB), not two (§12).
- `design/18-uhd-radio.md` UR-28: flush attribution (§12).
- `design/18-uhd-radio.md` UR-33: no flush fault model (§8.1 M3).
- `KNOWN_LIMITATIONS.md`: none (not a Ceiling).

---

## 4. Part (b): a burst at A's end + 1 sample (R1)

### 4.1 Hardware facts

F-17 and F-18: after a two-sample close (held tail + zero) that ends at `x`, a burst at `x` (gap 0) is dropped; `x + 1` plays. A close after a long burst makes the exclusive end `e + 1` when the payload ends at `e`, so the safe front is `e + 2` (`tx.rs:727` with `close_start = e − 1`). The Module is right; the hardware test predates the two-sample close. F-7 (one-sample tail, phase 7) is the older shape and is consistent with the device rule.

### 4.2 Behaviour table (R1)

Let `e` be A's payload end (exclusive); A's last sample is held and closed with two samples (one held, one zero).

| B's start | A's state | Module | Why |
|---|---|---|---|
| `e`, booked while A's held sample is still held | continuation | plays, one device burst | UR-23: an adjacent burst continues the device burst (F-15) |
| `e`, booked after the held sample went out | closed | late (gap 0): dropped, or `send_asap` | F-18: gap 0 is late |
| `e + 1` | closed | **late, dropped**: one `TIME_ERROR{late}`, nothing sent | F-18: gap 0 (the close's exclusive end is `e + 1`) |
| `e + 2` | closed | plays | F-18: end + 1 |
| `e + 3`, far later | closed | plays | F-18 |

### 4.3 Tests (R1)

| Seq | Test | Asserts | Observed through | Wrong behaviour that fails it |
|---|---|---|---|---|
| F4-e1 | `fake.rs` `ur_23_a_burst_at_the_first_safe_sample_after_a_held_tail_close_is_played` (`fake.rs:3219`†): unchanged | B at `e + 2` handed and played; B at `e + 1` gives one `TIME_ERROR late` and no `tx_send` at `e + 1` | `device.calls()`, `time_errors` | An early send at `e + 1` |
| F4-e2 | `tx.rs` `ur_23_a_burst_at_the_first_safe_sample_after_a_held_tail_close_is_played` (`tx.rs:3677`†): unchanged | front `== close_end + 1` | `chain.front()` | A front one sample early |
| F4-late | `fake.rs` new `ur_23_a_burst_at_e_after_a_closed_burst_is_late` (B booked at `e` after A's held sample went out at the deadline) | `TIME_ERROR late` at B's target, `outcome: drop` or `send_asap`; nothing sent at `e` | events | A continuation sent after the EOB |
| F4-adj | `fake.rs` new `ur_23_a_burst_at_e_while_the_held_sample_waits_continues_the_device_burst` (A ≥4 samples, held; B booked at `e` before the held sample goes) | one device burst, no second SOB at `e`; samples contiguous | `device.calls()`, capture | A second SOB at `e`; a gap |
| F4-far | `fake.rs` `ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time` (`fake.rs:3315`†) and `ur_23_a_burst_ends_at_the_next_bursts_start` (`fake.rs:2372`†): unchanged, except the `n` of FS-T7 | a burst booked well after a close (1 000 samples later) plays at its instant with its own ACK; no TIME_ERROR | `device.calls()`, `time_errors`, `async` | A late drop of a burst booked long after a close |
| F4-pad | `tx.rs` `ur_23_a_one_sample_burst_has_a_one_sample_safe_restart` (`tx.rs:2728`†): **renamed** `ur_23_a_one_sample_burst_restarts_after_its_padded_device_burst`; front `2_005` (was `2_003`, rev 2's value†) | front = padded end + 1 = start + 5 | `chain.front()` | Front `2_003` (no pad); the old name asserts a one-sample restart that no longer exists |

The UR-22 test list names the old `tx.rs:2728` test; its new name goes into the UR-22 Changes row (§12).

### 4.4 Hardware test (R1)

§13: `hw_b8_burst_one_sample_after_a_burst` (`hardware.rs:1560`, A = 30 000 samples): B at `e + 2` plays (ACK at its end + 1); B at `e + 1` is refused by the Module (one `late` TimeError, nothing sent). A comment cites repro 3.

### 4.5 Spec (b)

- UR-22: the closing rule (exclusive end + one sample) is unchanged. The padded first send is covered in §12.
- `KNOWN_LIMITATIONS.md:132` (the safe start after a close): unchanged; `:288–289` is updated (§12).
- MR-17 (`design/09-mock-radio.md:145`): the padded-front floor (§9.1).

---

## 5. Part (c): `hw_p70` — what the `L`s are

### 5.1 Explanation

- **One-sample trial.** A one-sample Kernel burst is not held (`tx.rs:552`, read in rev 4) and goes as one SOB+EOB padded to two (`tx.rs:576`, VERIFIED). Repro 1: a timed two-sample SOB+EOB is lost: 32 TimeError, 1 Underflow, no ACK (F-16). The trial's `L` count (32) matches (INFERRED from the count).
- **Control.** A two-sample burst booked 40 ms ahead is held: SOB 1 + EOB 2 (F-22). A2: a 1+2 split with a timed first send is lost in whole-burst drops, 32 TimeError, intermittently (VERIFIED on the repro's 12 trials). The control's 32 `L` match (INFERRED). The control was never a control: it is also a short first send, so the test compared two dropped bursts. The `diff.abs() <= 50` check compared onsets, not the burst's fate.
- **The next burst plays on time in both trials** (F-14, repro 4). So the dropped burst is A, not B.
- **The nine Underflows come first** in each trial's log and are the open-time flush's (F-2, VERIFIED). The one `U` after the `L`s in trial 2 is A's underflow after the drop (INFERRED).

### 5.2 What (c) can and cannot explain

| Candidate | Can it produce an `L`? | Why |
|---|---|---|
| (a) the flush's Underflows | No | The flush is untimed; its reports are Underflows, not TimeErrors. They explain the leading `U`s only. |
| (b) the safe-start rule | No | B is at a + 25 µs; A ends at a + 1 sample (1 µs). No gap-0 booking. |
| (d) timed continuations | No | Out of S1. Repro 4: continuation timing does not change the next onset; the drop is of the short first packet (A2, repro 1). |
| The short device burst (FS) | Yes | Repro 1 and A2 measure the drop of short first packets; the counts match. |
| Not explained | — | The exact packets the device received in each trial (F-22, INFERRED until the §5.4 print); the 32 `L` of the control (INFERRED). |

### 5.3 Fix (R2, N1)

- **One-sample trial.** FS-2 pads its single SOB+EOB to four at once (`[w0, 0, 0, 0]`). Measured clean at four (repro 1, A1).
- **Control.** A **four-sample finite burst** booked 40 ms ahead: one SOB+EOB of four, no pad, sent at once (FS-1; not held, §2.2). The control's first send actually contains four (Sol 8). The control keeps its name `hw_p70` control; it is no longer a two-sample held burst.
- Expected after the fix: no TIME_ERROR and no `L` in either trial; both onsets within ±50 samples; the one-sample trial's ACK at its padded end + 1 tick.

### 5.4 Diagnostic (R10: optional, print only, small)

Add to `hw_p70_one_sample_burst_then_timed_burst` (`hardware.rs:2949`†). No code change:
1. Per trial, the async rows with `code`, `tick`, `tx_flush`.
2. The probe rows for each `tx_send` of A and B: `n`, `at`, `sob`, `eob`.

It settles F-22 and that A is the dropped burst. It does not gate the code, because A2 and repro 1 already measure the rule. #78 stays open until a Module-path bench confirms the fix (§13).

### 5.5 Test after the fix

`hw_p70` keeps the onset comparison. Add: no `TIME_ERROR`; no `L`; no async `Underflow` (the flush's included, §13). Rename the control's description to "the four-sample control".

---

## 6. Part (d): out of S1 (owner, 2026-11-05)

### 6.1 Rule

Every payload-bearing `Device::tx_send` stays timed (UR-22 T1), including continuations, held tails and closing sends. **No code changes for timing**, so no timing mutation row is written. The reason recorded for T1 changes from #72 to "a late timed send is dropped and reported instead of playing on a shifted timeline" (owner); the UR-22 Changes row carries it (§12).

### 6.2 What stays pinned

The existing T1 assertions already pin this and are not changed: `ur_22_repeat_is_continuous_across_the_wrap` (`fake.rs:2312`, its line 2338 "every payload send is timed"), `ur_23_a_burst_ends_at_the_next_bursts_start` (`fake.rs:2372`†), `ur_23_two_bursts_back_to_back_loop_back_whole` (`fake.rs:3279`†), `ur_23_m4c_…` (`tx.rs:4644`†), and the `at` assertions at `tx.rs:1803`†, `:4023`†, `:4039`†, `:4048`† (the first send after a flush stays timed). `hw_p72_timed_tx_chunks_are_timed_and_flushed` (`hardware.rs:2908`) is **unchanged**: its name and its T1 check stay (the rename is withdrawn with B1).

### 6.3 Late first send (F-own-1, rewritten per R6 with B1 out)

`ur_22_a_late_first_send_is_dropped_and_each_timed_send_is_judged_by_its_own_instant` (`fake.rs`, new; manual clock; `SlowSend` so that S1 returns after its instant). Three timed sends: S1 at t0; S2 at t0 + n1; S3 at t0 + n1 + n2 (planned instants). Assertions, from the fake's packet record and `tx_async` after each send:

1. S1 gives one TimeError per channel, no ACK; none of S1's samples is played.
2. Which reports are available before S2 and before S3 are sent (report availability, R6).
3. S2 and S3 each: a TimeError if its own instant has passed when it is handed, else played at its planned instant (playback positions from the capture).
4. Whether the Kernel's cursor-expiry check (`tx.rs:425–430`, `:444–449`: `clock.instant(next) < now`) refuses S2 before dispatch (`calls()` shows no device call for S2 in that case).

**Expected (INFERRED; the test decides):** a late S1 is dropped; no later send plays on a shifted timeline; a later send is judged by its own instant. **Decision rule:** if a later send plays shifted, that is a defect of the timed rule, not a reason for S1 to change. Stop and report; add no abandon-on-TimeError mechanism (Sol P2: reports are asynchronous and the correlation is ambiguous).

### 6.4 Sequences F5–F8

| Seq | Test (file, name) | Asserts | Observed through | Wrong behaviour that fails it |
|---|---|---|---|---|
| F5-a | `fake.rs` `ur_22_repeat_is_continuous_across_the_wrap` (`fake.rs:2312`): unchanged | per-send `at`; contiguous samples across the wrap | `calls`, capture | An untimed send in a repeat |
| F5-b | `fake.rs` `ur_23_a_burst_ends_at_the_next_bursts_start` (`fake.rs:2372`†): unchanged | one device burst; contiguous; every send timed | `calls`, capture | A continuation without its instant |
| F5-c | `fake.rs` `ur_23_two_bursts_back_to_back_loop_back_whole` (`fake.rs:3279`†): unchanged | one device burst across both; sample map equal to the whole waveform | `tx_sends`, capture | A dropped or repeated sample at the join |
| F5-d | `fake.rs` new `ur_23_a_long_burst_in_many_chunks_plays_contiguously` (3 chunks + held tail; 1 000-sample windows) | every sample at its planned instant (`Duc` count); one device burst | capture at `e` offsets | A gap or shift after a chunk |
| F5-e | `fake.rs` new `ur_23_a_held_tail_is_timed_at_its_instant` | the tail send `at` = its instant | `calls` | An untimed tail |
| F6-a | `tx.rs` `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took` (`tx.rs:3846`†) and `ur_23_abandoned_send_preserves_the_accepted_cursor` (`tx.rs:4053`†): unchanged | recovery cursor and flush unchanged; closing sends carry their instant | `calls`, front | The abandonment cursor shifted |
| F6-c | `fake.rs` new `ur_22_a_first_send_is_never_undersized` (Module path; §8.3) | every `sob` packet has ≥ 4 samples per channel | `calls` | An undersized first packet (FS-1 removed) |
| F7 | `tx.rs` `ur_22_underflow_and_loss_end_the_damaged_burst` (`tx.rs:1695`†) and `ur_23_m4c_…` (`tx.rs:4644`†): unchanged | after an underflow the Module abandons the device burst; no later payload is sent on the shifted timeline (`tx.rs:1131–1139`) | `calls`, `rejected`, capture | A payload send after the underflow |
| F8-a | `tx.rs` `ur_28_per_channel_ack_preserves_the_next_burst` (`tx.rs:3089`†), `ur_28_a_report_is_correlated_by_its_position_in_the_streamer` (`tx.rs:3334`†), `ur_28_late_burst_error_and_later_ack_use_report_ticks` (`tx.rs:3127`†): kept; tick expectations per §8.4 | per-position pending retirement | `tx.rs` `unacked`, `device_async` | The ACK retiring the wrong pending |
| F8-b | `tx.rs` new `ur_28_grouped_channels_retire_per_position` (one stream, positions 0 and 1; ACK on 0 retires only 0's pending; a duplicate ACK on 0 does not retire 1's) | per-channel pending retirement | `unacked` | A duplicate ACK retiring another channel |
| F8-c | `tx.rs` new `ur_28_split_streams_retire_their_own_pending` (two streams, one channel each) | each stream retires its own pending | `unacked` per stream | A cross-stream retirement |
| F8-d | `tx.rs` new `ur_28_a_device_burst_over_two_kernel_bursts_keeps_both_records` (`remember_burst` tracks device-burst starts, F-29: one pending correlation for the device burst; both Kernel records in the burst records) | `rec.bursts` has both Kernel records; one pending entry per device-burst start | `rec.bursts`, `unacked` | A Kernel record dropped; a pending entry per Kernel burst |
| F8-e | `tx.rs` new `ur_28_ack_at_end_plus_one_tick_does_not_retire_a_burst_at_the_next_sample` (N4: the fake at ratio 1, 200 MS/s; A finite, ≥ 4 samples; B booked at A's end + 1 sample; the fake's ACK for A at its end + 1 tick, §8.1 M2) | after A's ACK, B's pending entry remains; B's own ACK later retires it | `unacked` | Today's `<=` at `tx.rs:373`: B's entry retired by A's ACK (F-33) |

---

## 7. Window, cut, hold, adjacency and repeat paths

Rev 3 treated the first-buffer exception as a window case. N1 and N5 change it: the wait is for window space only, and the burst's own length never waits.

| Case | Test (file, name) | Asserts | Wrong behaviour that fails it |
|---|---|---|---|
| Window `avail` 1–3, then 4 | `tx.rs` new `ur_23_a_first_send_waits_for_four_window_samples` (FS-3, N5) | no device call while `avail < 4`; one send of exactly 4 payload samples (values checked) once `avail = 4` | A send at `avail = 2` (the old first-buffer exception) |
| A window of exactly four samples | `tx.rs` new `ur_23_a_four_sample_window_takes_a_first_send_of_four` (N5) | the first send of a long burst is 4 samples; the next chunk waits for the window to empty; nothing refused | A refusal of a small window; a send of 3 |
| Four-sample open-ended first send | `fake.rs` FS-T6 (§2.3) | not held: SOB 4 without EOB | A held tail splitting 4 into 3 + 1 |
| A cut before four first samples | `tx.rs` new `ur_23_a_cut_before_four_first_samples_is_refused` (FS-4) | no send on the old clock; the `rejected` reason; the reservation cancelled; no TIME_ERROR | A sub-four send at the cut |
| Held-start preemption moves the start | `tx.rs` new `ur_23_a_preempted_first_send_restarts_its_count` (`preempt`, `tx.rs:957`) | the moved burst's first send is still ≥ 4, or waits for the window | A sub-four first send after the move |
| Repeat 1–3 samples (FS-6) | `tx.rs` new `ur_22_a_repeat_of_one_to_three_samples_tiles_its_first_send` | payload `w[i mod len]` for 4 samples; the Kernel advances by 4; `header.len = 4`; `tx_samples` counts 4 | A first send of 1–3 samples; wrong tile values |
| Repeat wrap | `fake.rs` `ur_22_repeat_is_continuous_across_the_wrap` (`fake.rs:2312`) | unchanged (§6.2) | — |
| Continuation of two (no minimum; FS-7, TBD-b4 killer) | `tx.rs` new `ur_23_a_continuation_of_two_samples_is_sent_at_once_with_no_minimum` | a 2-sample continuation (not EOB) is sent at once, no wait, no pad | A continuation held for four (the minimum applied to continuations) |
| Adjacent short burst, booked before the short burst's dispatch (FS-9) | `fake.rs` new `ur_23_a_short_burst_then_an_adjacent_burst_held_before_dispatch_follows_the_late_policy` (short burst s of 1–3 samples; B held with its start at the payload end `e`, which is before the padded front) | the short burst is padded to four and closed at once; B gets one `TIME_ERROR late` (drop) or is `send_asap`-moved per its policy; nothing of B is sent at `e`; B at the padded front plays | B continued into the padded device burst (sent at `e`) |
| Adjacent short burst, booked after dispatch (FS-9) | `fake.rs` new `ur_23_a_short_burst_then_an_adjacent_burst_booked_after_dispatch_follows_the_late_policy` | the same verdicts, decided when B is booked; the same nothing-sent-at-`e` | A verdict that depends on booking order |
| Stop while the first send waits (window) | `fake.rs` new `ur_23_stop_while_the_first_send_waits_sends_nothing` | no device call after Stop; a cancel report | A sub-four send after Stop |
| Loss while the first send waits (window) | `fake.rs` new `ur_23_loss_while_the_first_send_waits_sends_nothing` | no device call; `DEVICE_LOST` once | A send after loss |
| Waiting passes the start lead (FS-5) | `fake.rs` new `ur_23_a_wait_past_the_start_lead_follows_the_late_policy` | the late policy decides before dispatch, as today | A burst sent past its lead without the policy |

Rows: `P637`, `P638` (window) are kept. `P1108`, `P1109`, `P1113`–`P1117` (the `!device_sob` exemption and the first-buffer rows) are retargeted at step 4. `P1113` ("the first timed buffer waits for a whole buffer too") and `P1114` ("the first timed buffer is not cut to the window") assert today's first-buffer behaviour; N5 replaces it. Their tests `ur_23_m5_the_first_timed_buffer_takes_what_fits` (`tx.rs:4686`) and `ur_23_m1_a_repeat_stops_short_of_a_window_edge_it_cannot_fill` (`tx.rs:4563`) are retargeted: m5 becomes `ur_23_m5_the_first_timed_buffer_waits_for_four_samples`; m1's repeat case gets the FS-6 tiling.

---

## 8. FakeDevice (UR-33; R4)

### 8.1 Model

- **M1 (undersized first packet).** A packet with `sob = true` and 1 ≤ `n` ≤ 3 per channel is undersized. The device burst is dropped **through its EOB** (M1 drops the device burst through its EOB, no ACK): a timed undersized first packet gets a TimeError per channel (the existing TimeError path, `device.rs:690–696`) and every packet up to the EOB is dropped; no BurstAck follows. Untimed: one or more `Underflow` reports, then the BurstAck (repro 1, A2). **Counts are not pinned** (R4). The one-sample SOB+EOB stale-header rule (`device.rs:621`, a separate regression, R4) is kept.
- **M2 (ACK tick).** A BurstAck's tick is the burst's exclusive end + 1 master-clock tick: `Some(self.cursor + 1)` (today `Some(self.cursor.saturating_sub(packet.ratio))`, `device.rs:724`). Measured 126/126 (A1). The fake's reopen front moves by the same tick (§8.4).
- **M3 (no flush model).** `FakeFault::FlushUnderflows` is not added (R3, Sol P2): the production flush is four samples and draws none (A1). Attribution is tested with a test-local scripted source (§3.2 F1, F1′).
- **M4 (unchanged).** A continuation EOB of one or two samples after a device burst of four or more is played (F-7). The one-sample SOB+EOB stale-header rule (`device.rs:621`; `ur_33_a_one_sample_eob_loses_the_next_timed_header`, `device.rs:2784`; `ur_33_a_one_sample_eob_has_no_burst_ack`, `device.rs:3274`) is kept.

### 8.2 Raw fake tests (`fake.rs`, direct `tx_send`, no Module)

| Test | Asserts | Wrong behaviour that fails it |
|---|---|---|
| `ur_33_an_undersized_timed_first_packet_is_dropped` (shapes 1+2, 1+3, 2+2, 2+3, 3+2: the first packet is SOB non-EOB `n1`, the second EOB `n2`, timed first) | a TimeError per channel on the first packet; every packet through the EOB dropped; no BurstAck after the EOB | A model that plays the split (today's default) |
| `ur_33_an_undersized_untimed_first_packet_underflows_then_acks` (same shapes, untimed) | ≥ 1 `Underflow`, then a BurstAck (no count pinned) | An undersized untimed split that is played cleanly with no Underflow |
| `ur_33_a_four_plus_two_split_is_clean` (control: 4 + 2) | no TimeError, no Underflow; BurstAck at end + 1 tick | A model that drops a valid split |
| `ur_33_burst_ack_is_at_the_exclusive_end_plus_one_tick` | the ACK tick equals `cursor + 1` | The old last-sample tick |

### 8.3 Module path

`fake.rs` new `ur_22_a_first_send_is_never_undersized` (F6-c): over bursts of 1, 2, 3, 4, 5 and 9 samples, open-ended and not, window-limited, and repeat: every device call with `sob = true` has at least four samples per channel. Its killer is the FS-1 gate's removal (§11, the FS-1 row).

### 8.4 Existing tests that change (ACK tick, N4 and reopen front)

- Every test that asserts a BurstAck tick or correlates on the old last-sample tick: the implementer lists them at step 4 (`grep -n "BurstAck" -A3` in `tests/fake.rs` and `src/provider/tx.rs`) and retargets each to end + 1 tick with its reason. Known: `tx.rs:3089`†, `:3127`†, `:3334`†; the `fake.rs` ACK assertions near the `ur_28_*` tests.
- `ur_28_late_burst_error_and_later_ack_use_report_ticks` (`tx.rs:3127`†): the late-error tick and the later ACK tick change by the M2 offset.
- **N4 (BurstAck strict).** `tx.rs:373`: `root_tick <= tick` becomes `root_tick < tick` (F-33). Existing BurstAck correlation tests that rely on an ACK at a burst's own first instant are checked at step 4; none is expected to change, because a burst's ACK is always after its first instant. F8-e (§6.4) is the new killer.
- **Reopen front (TBD-d2, conditional on Q6).** The flush's ACK tick feeds the reopen front (`tx.rs:800–803`, F-27). Under M2 the fake's reopen front moves by the same offset (INFERRED: from end + 1 to end + 3 samples), unless Q6 changes `+2`. The new test that pins the reopen front is `ur_22_a_reopen_front_is_the_flush_end_plus_one`; if Q6 is refused, the test pins today's `+2` and TBD-d2 is retired.

### 8.5 Spec (UR-33, line 328)

The undersized-first-packet model (M1, through the EOB), the ACK tick (M2), no flush fault (M3). §12 has the text.

---

## 9. MockRadio parity (R5, F9; MR-17, MR-25)

### 9.1 Rule M1 (floor)

The Mock's front after a handed payload end stays `end + 2` (`mock-radio/src/lib.rs:1369`). For a **finite** burst that starts a device burst, the front is also at least the padded device end + 1:

`front = max(handed + 2, start + max(4, L) + 1)` (merge by max, as `advance_front` does, `tx.rs:167–170`).

- For L = 1–3: `start + 5` (UHD). Today's Mock gives `start + 3`: the Mock would admit a burst at `start + 3` or `start + 4` that UHD refuses. That is the direction the invariant calls a bug (§3, "Mock enforces the hardware envelope").
- For L = 4: `max(start + 6, start + 5) = start + 6`: unchanged (the Mock is already later than UHD).
- For a held close: `handed = e`; `max(e + 2, …)`: unchanged (R1).
- For a continuation of one sample: padded to two (N2); the Mock's `handed + 2` already equals UHD's `end + 2`: unchanged. For a continuation of two or more: unchanged (no padding).

### 9.2 Stop and cut (MR-25)

The Stop and cut fronts (`lib.rs:1131`, `:1751`) stay `cut + 2` and `stop + 2`: payload cuts (MR-25). A cut of a finite device burst that was already sent padded keeps the padded floor by the max rule. A Stop that finds a device burst open and unsent (a window wait, §7) cancels it: no device burst, no front. The Mock has no window wait, so this case is Module-path only (§7); the Mock parity test for it is dropped (§9.3).

### 9.3 Differential tests (`crates/ezsdr-acceptance/tests/parity.rs`; exact action order; both Module and Mock)

| Name | Action order | Observation (both) | Killing mutation |
|---|---|---|---|
| `s1_parity_short_burst_front_s_s3_s5` | one-sample burst at `s` (sent at once, padded), completed; book at `s + 3`; book at `s + 5` | `s + 3` refused by both; `s + 5` admitted by both | M1 floor removed (TBD-m1: the Mock admits `s + 3`) |
| `s1_parity_stop_during_padded_device_burst` | one-sample burst at `s`, sent padded; Stop at `s + 2`; book at `s + 3`, then `s + 5` | front is the maximum of the burst's padded end and the Stop's `stop + 2` (INFERRED from `advance_front`); verdicts equal | Stop front without the device floor (TBD-m2) |
| `s1_parity_grouped_and_split_fronts` | a two-channel grouped stream and two one-channel streams, each with a short burst at `s`; book on each at `s + 3` | the fronts are per stream; verdicts equal on each | One stream's front applied to another |
| `s1_parity_one_sample_continuation_is_padded_to_two` (N2) | a 4-sample burst, then a 1-sample continuation EOB at its end; book at its end + 1 | the continuation is padded to two; front `end + 2`; verdicts equal | A continuation padded to four (or not at all) |
| `s1_parity_payload_cut_separate_from_padding` | a short burst cut by Stop; its payload record | the record counts payload only; zeros are not counted | Padding counted as payload |
| `s1_parity_short_burst_then_adjacent_burst_both_orders` (N3) | a short burst at `s` (padded, closed); an adjacent burst at `s + 1` booked before its dispatch, and a second run with it booked after | both orders: the adjacent burst gets the same late verdict (TIME_ERROR drop or `send_asap`, per its policy); nothing sent at `s + 1` | The floor missing for the adjacent decision (TBD-m1) |

Six named tests (F9). The rev 3 test `s1_parity_stop_while_the_first_send_waits` is dropped: the Mock has no window wait (§9.2).

### 9.4 Mutation rows (Mock)

- TBD-m1: `crates/ezsdr-mock-radio/src/lib.rs` (final-source text of the front's `handed.map(|sample| sample.saturating_add(2))`, anchor at step 4): mutant removes the device floor. Killers: `s1_parity_short_burst_front_s_s3_s5`, `s1_parity_short_burst_then_adjacent_burst_both_orders`.
- TBD-m2: the Stop front (`lib.rs:1751`, `remember_tx_front(k, domain, stop.saturating_add(2))`): mutant drops the floor. Killer: `s1_parity_stop_during_padded_device_burst`.

### 9.5 Spec (MR-17, MR-25)

- MR-17 (`design/09-mock-radio.md:145`): add the padded floor (M1, §9.1): "the front after a finite device burst's first send is at least its padded device end + 1 (UR-22 `MIN_FIRST_SEND`)".
- MR-25 (`design/09-mock-radio.md:172`): add: "a Stop or a cut counts payload only; a device burst the Stop finds open and unsent is cancelled with no front."

---

## 10. Requested mapping F1–F9 and own scenarios

| Seq | Covered by | New or existing |
|---|---|---|
| F1 (flush reports) | §3.2 F1-4, F1, F1′ | new |
| F2 (Run underflow after the flush) | §3.2 F2 | new |
| F3 (recovery flush) | §3.2 F3 | new |
| F4 (B at e, e+1, e+2, far) | §4.3 F4-e1, F4-e2, F4-late, F4-adj, F4-far, F4-pad; `hardware.rs:1560` | existing, renamed, new |
| F5 (chunks, repeat wrap, adjacent bursts, held tail) | §6.4 F5-a…e (all timed) | existing + new |
| F6 (short or failed send; closing send; first send never undersized) | §6.4 F6-a, F6-c; §7; §8.2–8.3; §2.3 FS-T1…T7 | existing + new |
| F7 (underflow mid-burst) | §6.4 F7 | existing |
| F8 (per-position ACK; device burst over two Kernel bursts; tick) | §6.4 F8-a…e | existing + new |
| F9 (MockRadio parity; named) | §9.3 (six named tests) | new |
| Device lengths 1–4 | §2.2 case table; §2.3 FS-T1…T7; §8.2–8.3 | new |
| Adjacency (N3) | §7 (two tests); §9.3 (one test) | new |
| Own | §3.2 O-a1–a3; §6.3 F-own-1 (rewritten as a timed-behaviour pin); §7 | new |

---

## 11. Mutation rows (final source; anchors at step 4)

Every row is written against the final source. Its anchor is taken from the implemented line at step 4 and must match exactly once through `anchor.py` (Rust token anchors). Rows that keep their direction are listed as kept.

| Row | Status | Change | Killing test |
|---|---|---|---|
| P1171 (`let padded_eob = device_eob && sending == 1;` → `false`) | kept, retargeted (N2) | now the continuation pad only (first sends pad through PAD-1) | FS-T5 `ur_23_a_one_sample_continuation_eob_is_padded_to_two`; F-7 test |
| PAD-1 (new) | new | the first-send pad: `MIN_FIRST_SEND.saturating_sub(sending)` → `0` (first sends only) | FS-T1, FS-T2, FS-T3 |
| PAD-2 (new, front) | new | `tx.rs:691`: `open.next + pad + 1` → `open.next + 1` | FS-T1 front (`start + 5`); FS-T5 (front `end + 2`) |
| FS-1 gate (new) | new | the minimum (no first send under four) removed | §8.3 `ur_22_a_first_send_is_never_undersized` |
| FS-2 / FS-3 hold exemption (new) | new | `count <= MIN_FIRST_SEND` (`tx.rs:552`) → `count == 1` (rev 2's text) | FS-T6 `ur_23_a_four_sample_open_ended_first_send_is_not_held` |
| TBD-b4 (FS gate on continuations) | new | the minimum's `device_sob` condition removed (the minimum also applies to continuations) | `ur_23_a_continuation_of_two_samples_is_sent_at_once_with_no_minimum` |
| FS-3 window wait (new) | new | the first send's `min(count, 4)` requirement on `avail` → `avail` alone (sends short) | `ur_23_a_first_send_waits_for_four_window_samples` |
| FS-6 tiling (new) | new | repeat tiling → no tiling | `ur_22_a_repeat_of_one_to_three_samples_tiles_its_first_send` |
| TBD-d1 (new) | new | the undersized first-packet rule (M1) in FakeDevice: the drop-through-EOB → play | `ur_33_an_undersized_timed_first_packet_is_dropped` |
| TBD-d3 (new; M2) | new | final `Some(self.cursor + 1)` → mutant `Some(self.cursor.saturating_sub(packet.ratio))` (last sample), `device.rs:724` | F8-e; `ur_33_burst_ack_is_at_the_exclusive_end_plus_one_tick` |
| TBD-n4 (new; N4) | new | final `root_tick < tick` → mutant `root_tick <= tick`, `tx.rs:373` (today's text) | F8-e; `ur_28_a_ticked_ack_does_not_retire_a_future_burst` |
| TBD-d2 (new, conditional on Q6) | new | `tx.rs:802`: `+ 2` | `ur_22_a_reopen_front_is_the_flush_end_plus_one` |
| TBD-a1…a5 (§3.4) | new (P1173 merged with TBD-a3) | flush attribution | F1, F1′, F1-4 |
| TBD-m1, TBD-m2 (§9.4) | new | Mock floor | §9.3 |
| `tx.rs:552` exemption | generalised (FS-3) | the row above | — |
| P1113, P1114, P1108, P1109, P1115–P1117 | retargeted (§7) | first-buffer and repeat rows | the retargeted tests in §7 |
| P637, P638 (window) | kept | — | `ur_21_send_does_not_cross_the_in_flight_window` |
| P640, P641, P642, P1186, P657 (`+ 3` fronts for normal closes and abandonment) | kept (R1) | — | their tests |
| P643, P656 (held finite burst front) | kept | — | `ur_21_a_finite_burst_with_a_held_tail_keeps_its_front` |
| P1173 (`let zeros = [Iq::default(); 2];`) | retargeted (§3.4) | four zeros | `ur_13_arm_radiates_nothing` |
| P1190 (deadline stash) | kept | — | O-a1 |
| P1170, P1195, P1196, P1197 (B1 timing rows, rev 3 §6.5) | **removed** (owner: B1 out of S1) | — | — |

Rows under the (d) rule are not written: the timing code does not change (§6.1).

---

## 12. Spec text (R8) and Changes rows

Every changed rule gets a row in the spec's *Changes* table with its S1 decision number (the orchestrator assigns it). Rule text is normative and short, ends in an indented `*Test:*` line, and carries no rationale (AGENTS §2, DA-1). The reasons go into the Changes row. `plan/maintenance/tools/rules.py` must pass (`--base` = the commit before the change). No version bump and no `SCHEMA_CHANGELOG` entry (AGENTS §6).

| Spec | Text change |
|---|---|
| `design/18-uhd-radio.md` UR-22 (line 247), rule | (i) the first send of a device burst carries at least `MIN_FIRST_SEND` = 4 samples per channel; a first send of one to three is padded with zeros to four and sent at once, and a device burst closes with an EOB send of at least two samples (a first send of four or more needs no pad); (ii) a continuation has no minimum; its one-sample EOB is padded to two; (iii) a repeat's first send tiles its waveform to four payload samples; (iv) the T3 flush is four untimed zero samples; (v) T1 is unchanged: every payload-bearing send names its instant (the Changes row carries the reason: a late timed send is dropped and reported instead of playing on a shifted timeline); (vi) the next safe start after a device burst is its exclusive end (padding included) + one sample; the held-tail close is unchanged. |
| UR-22, one-sample-close text | The sentence "a one-sample payload close appends one zero" is replaced by (i)–(ii) above: a one-sample close of a first send is padded to four, a continuation's to two. The sentence "a close with no payload sends two zeros" stays. |
| UR-22, the flush paragraph (line ~247, "T3 sends an untimed two-zero SOB/EOB burst") | "four-zero". The test list names `ur_23_a_one_sample_burst_has_a_one_sample_safe_restart` (`tx.rs:2728`): renamed to `ur_23_a_one_sample_burst_restarts_after_its_padded_device_burst`. |
| `design/18-uhd-radio.md` UR-23 (lines 251–254) | Hold: "a first send of at most four samples is not held back; a device burst's first send is never shorter than four samples on the wire (UR-22)". Adjacency: "a Kernel burst that starts before a padded short device burst's front is decided by its own late policy against that front, and does not continue it." The held tail and continuations untimed → removed (not in S1). Test list: the new FS and N3 tests. |
| `design/18-uhd-radio.md` UR-28 (lines 303–306) | (a) flush attribution: a non-ACK report of an unacknowledged flush channel, code `Underflow` or `TimeError`, read before the flush's end (all channels acknowledged, or its deadline), is recorded with `tx_flush: true` and emits no `radio.TX_UNDERFLOW`; every other report keeps its handling; the flush's generation is the streamer its open created. (b) BurstAck: a `BurstAck` retires every pending burst whose first instant is **before** its tick (was: at or before). *Changes row:* fixes a hardware defect: an ACK at the previous burst's end + 1 tick retired the next burst's pending entry at 200 MS/s (`tx.rs:373`, F-33, INFERRED for the bench effect). |
| `design/18-uhd-radio.md` UR-33 (line 328, FakeDevice list) | the undersized first packet (M1: timed → TimeError per channel, the device burst dropped through its EOB, no ACK; untimed → Underflows then the ACK; counts not pinned); the ACK tick at the exclusive end + 1 master tick (M2); no flush fault (M3). INFERRED where F-20 is. |
| `design/09-mock-radio.md` MR-17 (line 145) | the padded floor (M1, §9.1). |
| `design/09-mock-radio.md` MR-25 (line 172) | Stop and cut count payload; a device burst the Stop finds open and unsent is cancelled with no front (§9.2). |
| `KNOWN_LIMITATIONS.md` line 288 (Ceiling for this case, Q1 decided by N3) | Replace "the Module continues the device burst there instead" by: "…continues the device burst there instead, except after a short burst that was padded and closed: a booking at that burst's payload end or before its padded front is decided by its own late policy (UR-22)." (N3; user-visible; the same commit.) |
| `KNOWN_LIMITATIONS.md` line 289 | Rewrite: "The first send of a device burst carries at least four samples per channel. A burst shorter than four is padded with zeros after its samples and sent at once; the next safe start is the exclusive end of the sent device burst plus one sample. The X300 failures were measured at 1 and 2 MS/s (A2); the floor applies to every X3x0 profile." |

The `KNOWN_LIMITATIONS.md:288–289` entries are user-visible: zeros are transmitted after a short burst, and a booking at its payload end is late after one. The same commit updates them (AGENTS §2).

*Interpretation of the owner's UR-22 instruction ("update UR-22's T1 wording only").* The normative T1 sentence is kept verbatim; the new reason goes into the Changes row, because DA-1 forbids rationale in rule text. If the owner wants the reason in the spec text, it goes into a sentence after T1's `*Test:*` line.

---

## 13. Hardware tests (R9, R10)

| Test | Change |
|---|---|
| `hw_b8_burst_one_sample_after_a_burst` (`hardware.rs:1560`) | Retarget (R1): B at `e + 2` plays (ACK at its end + 1); B at `e + 1` gives one `late` TimeError and nothing is sent at `e + 1`; cite repro 3. Its `async` assertion: `code != "TimeError" && (code != "Underflow" \|\| tx_flush == true)` (R9: non-flush TimeError and Underflow keep being rejected). |
| `hw_b8_burst_at_a_sent_burst_s_end` (`hardware.rs:1473`†) | Same `async` assertion (R9). Plus: no `async` Underflow of any kind, because the four-zero flush draws none (R9: the flush checks require no flush Underflow). |
| `hw_b6_txrx_and_repeat`, `hw_b7_session_loopback`, `hw_b8_*` (on #1 and #2) | Clean-flush check (R9): no `async` Underflow and no `radio.TX_UNDERFLOW`. |
| `hw_p70_one_sample_burst_then_timed_burst` (`hardware.rs:2949`) | §5.4 print (R10: optional, small: async rows and per-send rows); after the fix §5.5. The control is the four-sample control (§5.3). |
| `hw_p72_timed_tx_chunks_are_timed_and_flushed` (`hardware.rs:2908`) | **Unchanged** (B1 out; no rename). |
| New print-only `hw_s1_short_first_sends` (Module path) | 1-, 2- and 3-sample bursts, open-ended and not, each booked 100 ms ahead and once at the device lead; a 4-sample open-ended control; repeat 1–3; each followed by a timed B at its end + 1 + 25 samples; one open-ended 2-sample burst followed by an adjacent B at its padded front (N3). Print ACK, TimeError count, Underflow count, B's onset. Settles Q7. Runs in the same session as `hw_p70` after the code. |
| `hardware.rs:2032–2033` (sample counts) | Compare payload counts (F-28). |
| `tests/common/d9.rs` P6, P7 (raw probes) | Unchanged. |

#78 stays open until the Module-path bench confirms (R10).

---

## 14. Gaps (not planned: reported, not added)

1. **Two channels on one streamer.** A2 did not test them. The plan's grouped-channel tests are fake and Module tests only.
2. **Continuation sends of 1–3 samples that are not closes** (cut-limited). Unmeasured. FS-7 keeps them unpadded; F-7 is the only measured case (one-sample EOB, ACK only).
3. **An open-ended short first send followed by an adjacent booking** (FS-9, N3). The planned tests are fake and Module tests; the device sequence (a padded close followed by a SOB at the next sample) is measured only through the `hw_s1` print, not by repro.
4. **Rates 10–50 MS/s and the X310.** A1 and A2 measured X300 at 1–200 MS/s, and 3-sample failures at 1–2 MS/s only.
5. **The reopen front** (F-27; Q6).
6. **The padded send at the booked lead** (Q7): measured only as a repro shape (4 samples clean at 100 ms) and as the print in §13.

---

## 15. Open questions for the orchestrator (each with my recommendation)

Decided by the rulings (rev 4 applies them; no question is open):
- **Adjacent short burst (rev 3 Q1)**: decided by N3 — no refusal; the adjacent burst is decided by its late policy against the padded front; the Ceiling text is in `KNOWN_LIMITATIONS.md:288` (§12).
- **Repeat tiles count as payload (rev 3 Q2)**: decided by R2 — tiling to four, payload.
- **Flush attribution boundary and generation (rev 3 Q5)**: decided by R3 — per channel, codes `Underflow` and `TimeError`, before the channel's ACK or the flush's end; generation by the open that the flush follows.
- **(d) B1 (rev 3 Q4)**: decided by the owner — out of S1.
- **`MIN_FIRST_SEND` at every rate (rev 3 Q8)**: four at every rate; the 3-sample failures are measured at 1–2 MS/s only.
- **Scope of the fact (rev 3 Q9)**: "X300, measured at 1 and 2 MS/s (A2); applied to every X3x0 profile". X310 is unmeasured.

Open:
1. **Explicit refusal (rev 3 Q3), reduced.** The cut refusal (FS-4) stays. N5 removes the window refusal, which means a first send waits for window space and is never refused for a small window. Recommendation: keep the cut refusal; no window refusal. The profile's window is at least the device lead (`src/profile.rs:34–35`, a test that `tx_in_flight_ns ≥ DEVICE_LEAD_NS`), far above four samples at any rate (INFERRED from that test; not measured), so the window wait should be rare.
2. **Reopen front `+ 2` (F-27, rev 3 Q6).** Recommendation: change `tx.rs:802` to `at_or_after(tick)` with no `+ 2`, so the front is the flush's end + 1 on both the fake and hardware (UR-22). This changes a UR-22 test, so it needs the owner's OK. Until then TBD-d2 is conditional.
3. **Padded first send at the booked lead (rev 3 Q7, rewritten).** With N1 no send waits, so a short booked-ahead burst goes out about one device lead early (100 ms in the repro). Recommendation: the `hw_s1_short_first_sends` print in the same session as the post-fix `hw_p70`. If the device rejects a padded send at that lead, the rule needs a different send time, which is a new decision.
4. **Mechanism labels (`plan/maintenance/failure-mechanisms.md`).** Recommendation: (a) new mechanism: flush-window reports of the flush's own channels; (c) new mechanism: a short first packet not modelled (2 and 3 samples); (b) no defect (R1 corrects a stale test); (d) not in S1. The orchestrator decides; labels are counted only when it agrees.
5. **Open-ended short first send closes the device burst (N1, literal).** An open-ended first send of one to three samples is padded and closed at once, so whatever the Kernel books next is an adjacent burst (FS-9). Recommendation: accept it as N1 says. The `hw_s1` print includes the case (§13); if the device rejects the adjacent booking at the padded front, FS-9 is wrong.
6. **Scope of the wait (N5 read).** I read N5's "no refusal path for small windows" as: the first send waits for window space and never sends short. The alternative (send what the window holds, padded) would violate FS-1. Recommendation: the wait, as written in FS-3.

---

## 16. Unverified claims

- The padded first send's acceptance at the booked lead (about 100 ms, INFERRED from repro 1–4 and A1; the padded 4 at that lead is not run).
- The adjacency verdicts of FS-9 (INFERRED from the late policy, `tx.rs:905–953`; not run).
- F-33's bench effect at 200 MS/s (INFERRED from the code and ratio arithmetic; A1 has no ACK tick at 200 MS/s).
- The FakeDevice's `cursor` meaning in M2 (INFERRED, F-34; checked at step 4).
- F-own-1's behaviour (§6.3) is INFERRED; the test decides.
- Repro 1's mechanism (F-20) is INFERRED.
- The DUC's treatment of a one-sample continuation EOB after a device burst of four or more: F-7 gives the ACK; "B played" is INFERRED.
- The window of a shipped profile is at least `DEVICE_LEAD_NS` (a test, `src/profile.rs:34–35`); its value and the window's size at each rate are not checked here.
- `radio_tx_core.v` was not re-read (rev 2 §10).
- Line numbers marked † are rev 2's and were not re-read in rev 4; those read in rev 4 are cited as VERIFIED in §1 and in the sections (e.g. `tx.rs:373`, `tx.rs:552`, `tx.rs:691`, `tx.rs:757`, `core.rs:1107`, `core.rs:1113`, `device.rs:724`, `KNOWN_LIMITATIONS.md:288–289`, `tests/common/probe_device.rs:512`).
- Nothing was built, tested or run on hardware for rev 4. The tree is unchanged.

---

## 17. Rev 9: fix round 3 (2026-11-05, E1: D1 reverted)

Fix round 3 (`brief-fix3.md`) applies the "Round 3" ruling E1 of `rulings-code.md`: the owner reverted D1 (the receipt decision against earlier-received held bursts that were not yet handed over), because it changed ordinary back-to-back and overlapping bookings that the UHD Module continues or cuts (A = 16 at s, B at s+16 or s+8; A, C at +5, B at +17). The divergence is issue #79 (`mech:radio-rule-implemented-twice`), fixed in note 47 stage 5, which refuses a booking inside a short burst's padded range `[s, s+5)` on both sides. The Mock's receipt decision is now fix round 1 plus D4. Rev 8's §17 (fix round 2) is kept unchanged in `test-plan.v8.md`; this section records the removals, the kept round-2 items and their reasons, and what was verified.

### 17.1 Removed (D1 and everything that exists only for it)

| Where | What was removed |
|---|---|
| `ezsdr-mock-radio/src/lib.rs` | `HeldBurst`'s `target` and `policy` fields and the `LatePolicy` import (the `use ezsdr_kernel::stream` line is HEAD's again); `planned_front`; `tx_handed_front`'s `ahead` argument, its `unhanded` front and the `order < before` branch; the receipt's `redecide_displaced` call (the receipt ends at `Ok(())` as before); `redecide_displaced`; `move_held_segment`; the cancellation-test helper `held()` is HEAD's again. |
| `ezsdr-mock-radio/src/lib.rs` (Stop path) | The `None` argument of `tx_handed_front`, which only D1 added. The round-1 floor (`floor`, `remember_tx_front(k, domain, floor)`) stays. |
| `ezsdr-acceptance/tests/parity.rs` | `enum Wave`, `assert_held_pair`, the tests `s1_parity_held_short_burst_covers_a_later_booking_in_both_orders` and `s1_parity_held_long_burst_covers_a_booking_at_its_end_plus_one_in_both_orders`, the 16-sample `long` waveform and the `long` field of `CancellationPair`. |
| `ezsdr-mock-radio/tests/mock_channel.rs` | The tests `mr_17_a_short_burst_moves_a_later_booking_to_its_padded_front`, `mr_17_a_zero_window_profile_moves_no_held_burst`, `mr_17_a_displaced_repeat_and_burst_are_moved_to_distinct_starts` and `mr_17_a_send_asap_burst_moved_at_its_booking_is_reported_once`; in `mr_25_stop_does_not_radiate_unhanded_window_edge_payload`, the assertion of a `send_asap` TIME_ERROR at A's receipt (it exists only because D1 moved B at A's receipt). |
| `design/09-mock-radio.md` | MR-17's sentence on an earlier-received held burst (handed or not, the re-judgement, the zero window); the four test names above in MR-17's *Test:* line; the D1 part of the 2026-11-05 Changes row (the row keeps D4). |
| `plan/maintenance/tools/mutations.json` | Rows N48S1-19, 20, 21, 22 (the unhanded front, the displacement call, a SendAsap move's target, the Drop's TIME_ERROR), 26 (`planned_front`), 27 (the zero-window guard), 28 (the front after a move) and 30 (the single report). The numbers are not reused. |
| `plan/maintenance/tools/mutations.json` (re-anchored by D1) | N48S1-10 and P672, P673 went back to the fix round 1 front, `merge_front(previous, merge_front(handed.map(\|sample\| sample.saturating_add(2)), device_floor))`, with their round-1 `new` texts. |
| `plan/phase8/reviews/48s1-renames.txt` | Six `name -> -` lines for the dropped tests (the four Mock tests and the two parity tests). They were added in this step, so no base test is dropped. |

Not removed: P635 and P653 keep their anchor `for held in self.tx[k].held.values() {`. Fix round 2 changed it because D4 changed the `include` call, not because of D1; the anchor matches once.

### 17.2 Round-2 items kept, and why

| Item | Kept | Reason |
|---|---|---|
| D2 | yes | The close-path floor copy stays deleted; the Stop path keeps the floor; N48S1-16 anchors on the Stop line; killer `s1_parity_stop_during_padded_device_burst`. Nothing it depends on is D1's. |
| D3 | yes | UR-21's sentence, its Test-line names and its Changes row in `design/18`; the four `ur_23_a_first_send_*_lead_*` tests (`tx.rs`) were never D1's. |
| D4 | yes | The `first` argument of `include`, the window filter, the held-burst first-send expression (N48S1-23, N48S1-25), MR-25's text and `mr_25_a_first_send_that_does_not_fit_the_window_is_not_handed_over`. The filter judges a first send against the window, not against another burst, so it is independent of D1's decision path. |
| D4 self-review item 3 | yes | A repeat's first send is four samples: `mr_25_a_repeat_first_send_is_judged_at_four_samples`, N48S1-29. It is the D4 rule for repeats. |
| D6 | yes | `tx.rs` pads every channel of a first send; `ur_22_a_two_channel_one_sample_burst_is_padded_on_both_channels` and N48S1-24 are UHD-only. |
| D5, D7 | — | As in rev 8: rejected and pre-existing, no change. |
| Self-review item 7 | yes | MR-17's wording "at its receipt and on a Stop" describes the floor D2 kept. |
| Self-review items 1, 2, 5, 6, 8 | no | Each fixed a D1 mechanism (the zero-window guard, the front after a move, the single report of a moved burst, `planned_front`'s row, the D1 parity helper's comment); removed with it. |
| Self-review item 4 | no | The cascade (a SendAsap move after an unhanded long burst) exists only through D1's move. It is now part of #79 (17.6). |

### 17.3 Mutation rows and probes (rev 9)

The probes ran in a throwaway copy, `/tmp/opencode/probe-s1r3` (rsynced with `--no-times`, every file touched before each build, `CARGO_INCREMENTAL=0`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`). Logs in `probes5/`. Each row was applied with `anchor.apply`; each filter's unmutated run passes (`BASE-*` logs). A row is killed when its named test fails.

Every row whose `old` lies in a Mock function this round changed (`tx_handed_front`, `stop_tx`, `handle_tx_burst`) was re-probed:

| Row | Killer | Probe |
|---|---|---|
| N48S1-10 (front without the device floor) | `mr_17_a_one_sample_burst_uses_the_safe_sample` | killed |
| N48S1-16 (Stop keeps no floor) | `s1_parity_stop_during_padded_device_burst` | killed |
| N48S1-23 (window filter) | `mr_25_a_first_send_that_does_not_fit_the_window_is_not_handed_over` | killed |
| N48S1-25 (held first send as one sample) | same | killed |
| N48S1-29 (repeat first send as four) | `mr_25_a_repeat_first_send_is_judged_at_four_samples` | killed |
| P627, P634, P635, P653, P672, P673 (front and window in `tx_handed_front`) | `mr_25_a_stopped_burst_transmits_every_sample_before_the_stop`, `mr_17_x3xx_tx_preemption_front`, `mr_17_x3xx_tx_preemption_boundaries`, `mr_17_tx_front_is_per_stream` | all killed |
| P645, P652, P655, P659, P662, P670, P674, T26 (Stop path, `stop_tx`) | `mr_25_stop_hands_a_future_repeat_through_the_in_flight_window`, `mr_17_stop_preserves_front_until_expiry`, `mr_20_x3xx_loss_cuts_transmit_at_its_instant`, `a_loss_does_not_remember_a_stop_front`, `mr_25_repeated_stop_preserves_handed_samples`, `mr_25_stop_does_not_radiate_unhanded_window_edge_payload`, `ch_09_a_faulted_round_does_not_depend_on_the_stepping_order` | all killed |
| T35, P651 (receipt, `handle_tx_burst`), P146, P147 | `rm_14_the_recorded_requested_target_is_the_requested_at`, `mr_17_x3xx_tx_preemption_front`, `mr_16_each_transmit_stream_is_judged_by_its_own_channel_count` | all killed |

Twenty-three rows, twenty-three kills; every `BASE` run passes. Probe note: rev 8's `probe.py` labelled every failing run COMPILE-ERROR (its check matched cargo's `error: test failed` line). Rev 9's `probe2.py` matches only `error[E…]` and `could not compile`; its verdicts are from the logs, and the logs show `test result: FAILED` for each kill.

Rows removed: N48S1-19, 20, 21, 22, 26, 27, 28, 30 (17.1). The other rows in functions this round did not change (for example `handed_end_before`, `merge_front`, `remember_tx_front`, `stop_at`, `emit_late_burst`, and the UHD rows in `tx.rs`, `core.rs`, `device.rs`) were not re-probed; `rules.py` (below) checks that every row still matches exactly once.

### 17.4 Verification (rev 9, after the last edit)

- Parity: 26 passed (28 at rev 8, minus the two D1 tests). `ezsdr-mock-radio`: `mock_channel` 29 (33 at rev 8, minus four), `mock_layout` 27, `mock_radio` 69, `timeline` 1, lib 8 (cargo's per-file output; rev 8's note listed the last two in the other order).
- `ezsdr-radio-uhd --lib`: 359 passed; `--test fake`: 332 passed (no change to the UHD crate this round).
- Remote workspace, `rdev.py -- cargo test --workspace` after the last code edit: 1817 passed, 0 failed (1823 at rev 8, minus the six D1 tests).
- Clippy `-D warnings`, remote: `--workspace --all-targets` clean; `-p ezsdr-radio-uhd --features uhd --all-targets` clean (cached; the UHD crate does not depend on the Mock).
- `cargo fmt --all --check`: clean.
- Python 3.14 suite (`~/.cache/ezsdr-venv`, `EZSDR_SERVER` = the tree's `ezsdr-server`): 92 tests OK.
- `rules.py --base HEAD --renames plan/phase8/reviews/48s1-renames.txt`: 0 problems. (It is run again after the one-cell fix in design/09 below.)
- `git ls-files -m -o --exclude-standard | xargs chmod go+r`: applied.

### 17.5 Self-review (one round, read-only)

The free tier refused (`probes5/self-review-fix3-free.log`); the fallback `opencode-go/deepseek-v4.1-flash#max` ran (`probes5/self-review-fix3-go.log`). Its checklist: nothing of D1 left, D2/D3/D4/D6 intact, no other behaviour changed. Its findings:

| # | Sev | Finding | Disposition |
|---|---|---|---|
| 1 | P3 | The 2026-11-05 Changes row of `design/09` (fix round 2, D4) had a duplicated link cell, a fifth cell in a four-column table. | Adopted: the duplicate is removed. A doc-only change; `rules.py` runs again. |
| check 1 | — | D1 leftovers: none. The names occur only in `48s1-renames.txt`. | — |
| check 2 | — | D2, D3, D4, D6: intact, with the locations the checklist named. | — |
| check 3 | — | `lib.rs` differs from round 1 only by D4 and the expected Stop/floor lines. | — |
| check 4 | — | The renames file lists every removed test. | — |

### 17.6 Open points and gaps (rev 9)

- **#79 (stage 5).** The divergence is a booking inside a short burst's padded range `[s, s+5)`, which the Mock and UHD decide differently. The D1 parity sequences (a short A and B at `s + 1` or `s + 4`, a 16-sample A and B at A's end + 1, both orders) now belong to #79 and are not in the tree as tests (E2: no ignored or expected-failure test). The cascade (17.2, self-review item 4), the D4 corner (a first send cut by a held burst) and the two-sample A + B case (E4) also go to #79. The D1 residual collision (INFERRED, previously open at 17.6 rev 8) went with D1's `redecide_displaced`.
- **Parity values (rev 8, 17.6):** gone with the pair.
- **`tx_clipped` (rev 7 residual):** unchanged.
- **Stored `late_by` on a second move (D7):** gone with the SendAsap move; pre-existing in UHD (`tx.rs:1020`), unchanged.
- **Bench:** the `hw_b8` late trial and `hw_s1_short_first_sends` are unchanged and compile-checked only (open since rev 7).
- **KNOWN_LIMITATIONS.md:** no entry for the #79 divergence (E3: a defect tracked as a GitHub issue is not listed).
