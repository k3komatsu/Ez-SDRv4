# Note 48 S1 — dispositions and rulings

---

## `rulings-plan.md`

## Orchestrator rulings on Sol's review of the S1 test plan v2 (2026-11-05)

Source: sol-plan-final.md (PLAN NEEDS CHANGES). Every finding is adopted. Revise the plan to rev 3 (keep v2 as test-plan.v2.md).

R1 (b): keep the tail+zero close; retarget `hw_b8_burst_one_sample_after_a_burst` to e+2 played / e+1 refused as late. No change to the front arithmetic for normal closes.

R2 (c), the device rule (hardware-facts-cpp.md addendum 2 overrides plan §4.5/§6):
- The first send of a device burst carries >= 4 samples per channel, counted after tail withholding. Constant name e.g. `MIN_FIRST_SEND = 4` with a one-line comment citing the 2026-11-05 measurement; it is a hardware envelope fact of X3x0 (spec it in UR-22 and KNOWN_LIMITATIONS, not a profile key unless the plan shows a reason).
- A finite Kernel burst shorter than 4 samples that starts a device burst goes as ONE timed SOB+EOB send padded with zeros to 4 samples. Kernel records and statistics stay payload-only; the safe front is computed from the actual padded device interval (end + 1 sample as today).
- A Kernel burst that continues an established device burst gets no minimum and no padding.
- Open-ended or window-limited first sends: no send until 4 samples per channel can go (window `avail`, `wanted`/chunk, a cut, a held start); if waiting passes the start lead, the existing late-policy rules decide before dispatch; Stop/loss while waiting cancel as today. If the constraints make a legal first send impossible (e.g. a window that cannot hold 4 samples), refuse explicitly with a reason — never spin, never send undersized. Repeat waveforms of 1–3 samples: the first send tiles to >= 4.
- Retire closing-send-only padding (the plan's `padded_eob` replacement) unless the plan shows a remaining case.

R3 (a): four-zero T3 flush; attribution bounded by streamer generation, stream, the flush's channel positions and the flush window (until its ACKs or its existing deadline). Unrelated reports (other channels/streams/generations) keep their normal handling; `Other(77)` handling unchanged. After-deadline reports: keep today's conservative fallback, tested, without claiming their origin. Test the attribution with a test-local scripted report source (no production option to choose the flush shape).

R4 FakeDevice (UR-33): a deterministic adverse model of an undersized FIRST packet of a device burst (< 4 samples per channel; with or without EOB): timed → dropped with TimeError and no ACK; untimed → Underflow reports. Raw fake tests pin failing splits (1+2, 1+3, 2+2, 2+3, 3+2) and a clean 4+2 control. Do not encode exact report counts or spacing. Keep the one-sample stale-header regression separate. Also align the fake's BurstAck tick with the measured hardware: the burst's exclusive end + 1 master-clock tick (126/126 measured), and reconcile every ACK assertion/correlation with it.

R5 Mock parity (MR-17, MR-25): name the differential tests with exact action order, observations and killing mutations: completion and Stop, grouped and split streams, a short burst continuing a device burst, the s / s+3 / s+5 front case. Payload cut separate from padding and the safe-start margin.

R6 Late first send (F-own-1): replace the oracle with explicit assertions over >= 3 sends, playback positions and when reports are available; no new abandon-on-TimeError mechanism; establish whether existing cursor-expiry abandonment covers it. F8: `remember_burst` tracks device-burst starts; assert per-channel pending retirement and that both Kernel records survive.

R7 Mutation rows: write every row against the FINAL source (regressions), reverse/retarget P1170/P1195–P1197 and the held-tail/close rows accordingly; add killing rows for the first-send minimum, the premature window send, tail withholding, padding/front arithmetic, Mock/Stop parity and the fake rule. Fill every blank observation/wrong-behaviour cell; F9 gets named tests.

R8 Specs: UR-22/23 (first-send buffering, atomic short-burst padding, adjacency), UR-28 (flush attribution bounds), UR-33 (undersized first packet, ACK tick), MR-17/MR-25, KNOWN_LIMITATIONS.md:289 (first send >= 4, not "four per device burst"); Changes rows.

R9 Hardware assertions: keep rejecting non-flush TimeError/Underflow; the four-zero flush checks additionally require no flush Underflow.

R10 The `hw_p70_…` print-only diagnostic: optional, keep it small (print async and per-send rows). #78 stays open until a Module-path bench confirms.

---

## `rulings-plan2.md`

## Orchestrator rulings on the Opus re-check of plan rev 3 (2026-11-05) — binding for rev 4

Source: the re-check (summarised here). Owner decisions today: B1 is DROPPED from S1 — every send stays timed (the reason changes from #72 to "a late timed send is dropped and reported instead of playing on a shifted timeline"); update UR-22's T1 wording only. Remove part (d) and everything that depends on it (§6.5 rows, F5 retargets, UR-22 (v), the `hw_p72` rename, F-own-1's B1 consequences); keep F-own-1 only if it pins today's timed behaviour.

N1 (P1): no waiting first send. Generalise the `tx.rs:552` exemption: a device-burst first send of 1–MIN_FIRST_SEND samples is never held back; it goes at once as one timed SOB+EOB send padded with zeros to MIN_FIRST_SEND (4). Remove the wait from FS-3, the case table, §13 Q7 and the KNOWN_LIMITATIONS text. A later booking at its end meets the padded front through the existing late policy.
N2 (P2): keep today's two-sample pad for a one-sample continuation EOB (`padded_eob`, row P1171 retargeted, not retired). Only first sends pad to 4.
N3 (P2): a short (< 4) burst followed by an adjacent burst: the short burst is padded to 4 and closed (no continuation); the adjacent burst is decided against the padded front by its own late policy (TIME_ERROR drop or send_asap) — both orders (B held before A's dispatch, B booked after). Module test, Mock parity test, mutation row, Ceiling text in KNOWN_LIMITATIONS (replace :288's "the Module continues the device burst there instead" for this case).
N4 (P2): BurstAck correlation strict (`<` instead of `<=` at `tx.rs:370`); F8-e runs at ratio 1 (200 MS/s) with the fake's ACK at end + 1 tick; a mutation row on the comparison. This also fixes a hardware defect today (an ACK retiring the next burst's pending entry at 200 MS/s) — say so in UR-28.
N5 (P3): a first send's `wanted` is raised to MIN_FIRST_SEND when the chunk is smaller; no refusal path for small windows.
Minor (R7/R8): name files and assertions for the padding tests (values and zero positions `[w0,0,0,0]`; header/record counts payload-only), fill F4-far and §8.2's wrong behaviour, give TBD-b4 a continuation killer, name TBD-d2's reopen-front test, merge P1173-retarget with TBD-a3, fix the `tests/common/probe_device.rs` path, F1's read order, M1 drops the device burst through its EOB (no ACK), §12 decides the Q1 Ceiling, UR-22's one-sample-close text.

---

## `rulings-code.md`

## Rulings on the S1 code reviews (orchestrator, 2026-11-05)

Sources: Sol's code review (`sol-code.log`, verdict CODE NEEDS CHANGES) and the orchestrator's review (step 6).

| # | Source | Sev | Finding | Ruling |
|---|---|---|---|---|
| C1 | Sol 1 | P2 | Mock drops the padded front on Stop (`ezsdr-mock-radio/src/lib.rs` ~1122–1133, 1359–1360, 1755–1759): after A (one sample at s) is dispatched and Stopped, the Mock keeps `s + 3`, UHD keeps `s + 5`. `parity.rs:1467–1479` queues A and Stop together and asserts the opposite of its comment. | Adopted. The Mock's Stop keeps the floor-raised front apart from the payload cut. The test observes A's dispatch before the Stop and asserts B at `s + 3` dropped, B at `s + 5` played, on both sides. This is TBD-m2's killer: add the row. |
| C2 | Sol 2 | P2 | A waiting first send is rechecked after its start, not after its start lead (`tx.rs:426–449`): window space appearing inside the 2 ms lead sends it with no late verdict. Rev 4 named the test "the wait past the start **lead**". | Adopted. An unsent device-burst start is judged by its late policy once it is inside the lead, with the same lateness test `dispatch_start` uses (reuse it, do not write a second one). Tests: both policies (DropAndFlag, SendAsap) one tick on each side of the lead boundary. |
| C3 | Sol 3 | P2 | `hw_b8_burst_one_sample_after_a_burst` (`hardware.rs:1600`) now accepts flush Underflows; R9 requires the four-zero flush to draw none. | Adopted. Reject every Underflow and TimeError in the raw async section of this bench test (keep `hw_b8_burst_at_a_sent_burst_s_end` as is). Same for any other bench assertion the step loosened this way (search `tx_flush\"] == true` in `hardware.rs`). |
| C4 | Sol 4 | P3 | `design/18` UR-23 "a later buffer takes what fits" is false (continuations wait for `wanted`); the Changes row at ~721 says "four or more" are not held, the exemption is at most four. | Adopted: correct both texts; no code change. |
| C5 | Orch | P3 | Brief F3 (a recovery reopen's flush Underflows are not the Run's) has no test; §17 says it shares an existing test's path. | Name the existing test and the line where the recovery reopen reaches `tx_flush_locked` with the attribution, in §17. If none drives Underflows into a recovery flush, write it in `core.rs` or `tx.rs` with the `ReportFeed` rig. |
| N1 | Sol | note | The 97–98 ms parity probe: an artefact of the parity harness's 1 ms polling of the UHD side's clock (`parity.rs:1041–1049`, `:433–443`), not a Module/Mock divergence. | Recorded in §17; no change. |

### Round 2 (after the Opus re-check of fix round 1)

The re-check found C1 and C2 correct. Rulings on its findings:

| # | Source | Sev | Finding | Ruling |
|---|---|---|---|---|
| D1 | re-check | P2 | MockRadio decides a burst at receipt against the fronts of *handed* bursts only. A short burst A held outside the window (one sample at `s`) and B at `s + 1…s + 4`: UHD dispatches A first (padded to four, front `s + 5`) and drops B by its late policy; the Mock plays B. Either receipt order. The same class likely exists before S1 for a long A and B at A's end + 1 (UHD front end + 2) — INFERRED, check it. | Adopted. The Mock decides a burst against the front of every burst it holds that starts earlier, handed or not (end front and short-burst floor alike). When a burst arrives whose front covers the start of a burst already held (the B-then-A order), that held burst is decided again at this receipt by its own late policy (Drop: one TIME_ERROR and it is removed; SendAsap: moved to the front). Write the failing tests first: parity (`parity.rs`) for short A + B at `s + 1` and at `s + 4` in both receipt orders, and long A + B at A's end + 1 in both orders; keep the verdict counts and the radiated samples equal on both sides (the event instant may differ: the Mock decides at receipt, UHD at dispatch). Spec: MR-17 states the rule in one sentence. Mutation row(s) with their killer. |
| D2 | re-check | — | The Mock's close-path floor copy (`lib.rs:1704`, `1768-1770`) is unobservable; only the Stop path decides. | Adopted: delete the close-path copy; anchor N48S1-16 on the Stop line alone. |
| D3 | re-check | — | UR-21 does not say when a waiting first send is re-decided. | Adopted: append to UR-21's sentence "repeats the device-lead decision against its current instant": "; a first send that waits for window space (UR-23) is decided again once its start is less than the device lead away, before it is handed." Add the `ur_23_a_first_send_*_lead_*` tests to its *Test:* line, and a Changes row. |
| D4 | re-check | P3 | MR-25: the Mock counts a first send as handed when its start is before the window's first sample; UHD waits until four samples (or the whole short burst) fit. | Adopted: the Mock counts a first send as handed only when four samples, or the whole burst if shorter, are inside the window; MR-25 text; one Mock test. |
| D5 | re-check | P3 | A timing with `tx_in_flight_ns` < lead + 4 samples makes a SendAsap first send move forever. | Rejected for S1: the production profile is 10 ms against a 2 ms lead, and `with_measurement_timing` is a bench tool. Recorded in the review record. |
| D6 | re-check | P3 | No grouped (two channels, one stream) padded first send test. | Adopted: one `tx.rs` test, a two-channel stream with a one-sample burst: both channels padded to four, front `s + 5`. |
| D7 | re-check | note | A second SendAsap move overwrites `open.late`'s `late_by`. | Pre-existing (`tx.rs:1020`, window re-dispatch); recorded, no change. |

### Round 3 (after the Opus re-check of fix round 2)

The re-check found D2–D6 correct, and D1 as ruled wrong (P1, reproduced): judging a booking against every earlier held burst made the Mock drop or move ordinary back-to-back and overlapping bookings that the Module continues or cuts (A = 16 at s, B at s+16 or s+8; A, C at +5, B at +17). The ruling, not the implementation, caused it.

| # | Ruling |
|---|---|
| E1 | **Owner (2026-11-05): revert D1 and fix it in note 47 stage 5.** Remove D1's Mock change (the receipt decision against unhanded held bursts, `redecide_displaced`, `planned_front`, and the round-2 self-review fixes that only serve them), its tests (the D1 parity tests and any Mock tests written for D1), its rows (N48S1-19…22 or whichever rows mutate D1 code), and its MR-17 sentence. Keep D2, D3, D4, D6 and every D4 self-review fix. The divergence is issue #79 (`mech:radio-rule-implemented-twice`); stage 5 refuses a booking inside a short burst's padded range `[s, s+5)` on both sides. |
| E2 | Keep one parity test that documents today's boundary without asserting the divergence: none. Do not add an ignored or expected-failure test; #79 holds the sequences. |
| E3 | `KNOWN_LIMITATIONS.md`: no entry (AGENTS §2: defects tracked as GitHub issues are not listed). |
| E4 | Re-check P3s: the D4 corner (first send cut by a held burst) and the two-sample A + B case go to #79 (preemption, stage 5). The duplicated decision code leaves with D1. |
