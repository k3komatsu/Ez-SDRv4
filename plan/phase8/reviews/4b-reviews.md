# Stage 4b — reviews

## fable-review

Model: claude-fable-5-2

## Stage 4b design review (AGENTS §9 step 0): the overflow restart gap (RM-17) and #108

Reviewed: `design-draft.md` (revised 2026-11-18) against `decisions.md` Q1–Q4, at `main` ce631495. Read-only. No test was run: the drafter's probe logs already show the rig result (`scratchpad/out/probe_host2.txt:11` `PROBE excused []`; `probe_host_unbounded.txt:11` `[243, 766, 875]`; `probe_host.txt:13,17` is the same run failing only on the old assertion), so a rerun would add nothing.

UHD paths are under `/root/tmp/uhd-4.10.0.0/` in `ezsdr-v4-dev:uhd4.10`. "VERIFIED" below means read in source; nothing here was run on hardware, so every statement about what a USRP does in a sequence is INFERRED from that source unless a bench record is cited.

Q1–Q4 are not argued against. M1 names one harm the owner was not told (it changes the wording of what Q1 promises and the bench test, not the choice). M2 is mechanism inside Q1.

---

### 1. Majors

#### M1. A restart booked while the reader is still reading the backlog starts late; the draft's contract, S9 and S10 miss this window

The bounded wait fixes a booking made **after uhd-rx has received the overflow result**. It does nothing for a booking made between the overrun `f` and that result, and after a stall that window is long.

Mechanism (code VERIFIED):
- After a stall the reader first reads everything buffered before the overrun; UHD returns the overflow only when a zero-timeout read finds nothing left (`host/lib/include/uhdlib/transport/rx_streamer_zero_copy.hpp:209–237`). Call that instant `t_o`.
- Each drained block sets `last_samples` to the host's present time (`crates/ezsdr-radio-uhd/src/provider/rx.rs:476`).
- A silent stream is ended only when `quiet` holds: host time since `last_samples` ≥ `quiet_span` (`rx.rs:521–523`). So the stop cannot be issued before `t_o + span`, whatever the cut.
- The plan's ready instant is `cut + span`, the origin `ready + 10 ms`.

Sequence (10 MS/s, 2 000-sample blocks, span 3.2 ms):
```
stall 500 ms .. 2.5 s; device overruns at f; reader resumes at r
r + 20 ms : Stop + start_rx booked (cut c = r+20 ms, origin = c + 13.2 ms)
t_o = r + 19..37 ms on a quiet host (S4: restart-after-resume 68.7–87.2 ms minus UHD's 50 ms;
      draft §6 table; bench/2026-11-16-stage4/README.md:20), r + 39..89 ms under H4 (139.1 − 50)
rx_stop at t_o + 3.2 ms; rx_start(origin) after it
```
Outcome: whenever `t_o − c` exceeds about 10 ms minus the configure time, `rx_start` goes out with a short or negative lead: `START_LEAD_SHORT`, or a missed start (`LATE_COMMAND`, first block `GAP_BEFORE` with `lost` its index, UR-17). MockRadio starts on time. This is #108's symptom, properly reported this time, but still a Mock/USRP difference.

Consequences for the draft:
1. **Contract** (§1 "A restart booked after an overflow starts on time, on both Providers"; the Q1 record "starts at its RM-25 origin on both Providers"): true only for bookings after `t_o`. From the user's side the gap starts at `f`, and the user sees neither `r` nor `t_o` (the event arrives with the restart block or the tail). The draft's exception ("a host stall that keeps uhd-rx from running past the new origin", "not overflow-specific") does not cover it: the reader is running, and the backlog is specific to an overflow after a stall. The owner was told two caveats (retune margin, STOP order); not this one. **Bring to the owner**: the promise becomes "on time unless the Module is still reading the samples buffered before the overflow (about 20–40 ms after the reader returns on a quiet host, up to about 90 ms under H4; INFERRED from S4), in which case the start is a reported missed start". KL-7's replacement line must name the backlog, not only "a host stall".
2. **S9** books "about 20 ms after the expected resume": that is inside or at the edge of the drain window at H0 and well inside it under H4. Its verdict "`LATE_COMMAND` = fail, which calls for the stop-at-the-cut fallback" would then fire for a cause the fallback cannot fix (stopping at `t_o` instead of `t_o + span` gains only `span`). S9 needs two cells, classified per trial from the trace (the overflow result's time against the Stop's booking), not by a fixed delay: (a) booked after `t_o` → must be on time; (b) booked before `t_o` → a reported missed start is the expected result.
   Once the cut's instant has passed, uhd-rx also reads one packet a call (`one_packet`, `rx.rs:238–246`; `crates/ezsdr-radio-uhd/src/uhd.rs:1018`), each turn taking the chain and streams locks, so the rest of the drain can be slower than S4 measured (INFERRED).
3. **S10 cannot rehearse S9.** The FakeDevice's behind path returns `Overflow` on the first call after the stall and discards the backlog (`crates/ezsdr-radio-uhd/src/device.rs:1768–1780`); it has no drain, so every booking after `r` is in cell (a). AGENTS §9 asks the rehearsal to book the same boundary on both sides. Either the fake delivers (or delays for) the buffered samples before its `Overflow`, or S10 is stated to rehearse cell (a) only and cell (b) is named as hardware-only.
4. **The rig** has no backlog either (data is ready at device time plus latency, `differential.rs:950`), so "all 1 000 seeds clean" proves the post-result window only. That is fine if said.

No cheap Module fix is seen: starting the new segment before the old one's backlog is read needs two streams in uhd-rx. Recommendation: document it and test it as above; do not build that.

#### M2. The Module's STOP can overtake UHD's restart; the draft's margin and its "harmless" case are both wrong

VERIFIED from source:
- UHD posts the restart request **ASYNC** from inside `recv`, then returns the overflow (`host/lib/rfnoc/rfnoc_rx_streamer.cpp:179–186`). The radio's handler reads the device time over the network and posts the START (`host/lib/rfnoc/radio_control_impl.cpp:105–125`); the streamer forwards it to every channel (`rfnoc_rx_streamer.cpp:338–352`). All of it runs on one handler thread from one FIFO queue (`host/lib/rfnoc/graph.cpp:573–603` enqueue, `:625` onward the handler).
- The Module's `rx_stop` and `rx_start` are `uhd_rx_streamer_issue_stream_cmd` (`crates/ezsdr-radio-uhd/src/uhd.rs:608, 994–1004`); that the handle from `uhd_usrp_get_rx_stream` (`uhd.rs:985`) is an `rfnoc_rx_streamer` on an X3x0 is INFERRED (not traced through `multi_usrp_rfnoc`). Its STOP is a SYNC post per channel (`rfnoc_rx_streamer.cpp:131–150`; default mode `host/include/uhd/rfnoc/node.hpp:453–455`). A STOP enqueued before the radio's handler has posted its START is processed first; the START then goes to the radio after it. `_last_stream_cmd_stop` does not stop a cascade already in flight.

The draft says the STOP comes "at least `quiet_span` (≥ 3 ms) after the overflow result" (§3 residual). That holds after a stall (the result follows the last drained block at once). It fails in two cases:
- `quiet` counts from the last samples, and the overflow result does not touch `last_samples` (`rx.rs:479–482`). When the overrun message arrives during a wait, the result comes at the end of that wait, already `span` after the last samples; the next 1 ms wait ends the stream (`rx.rs:455–456, 518–527`). The STOP follows the restart request by **about 1 ms**.
- Under abort it follows at once: `command(Abort)` cuts at `expected` and calls `try_end` (`rx.rs:251–263`).

Sequence A (restart, two channels in one streamer):
```
recv → Overflow (restart request queued)       t
abort, or cut + 1 ms: rx_stop                   t + 0..1 ms   (handler thread not yet through get_time_now)
queue: [restart_req, STOP ch0] → radio posts START behind STOP ch0
STOP ch0 runs; START reaches both radios; STOP ch0's sync returns; STOP ch1 runs
```
Outcome: radio 1 is stopped and cleared; **radio 0 keeps UHD's START at about t + 50 ms and restarts alone**. With one channel the same START survives.

Sequence B (what the surviving START does to the next segment; the Module then issues `rx_start(origin)`, which queues behind it; a continuous command runs until a STOP, `fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v:556–598`):
- UHD's time before the new origin — the draft calls this "harmless". It is not: the stream runs on UHD's grid; packets stamped before the origin are dropped whole (`rx.rs:543–549`), so the first kept block has `k > 0` with no pending report → `GAP_BEFORE | SEQ_DISCONTINUITY`, `lost` up to a packet, and every block of the new segment is off the lattice (`rx.rs:551–554`, KL-8) with the head zeros misplaced.
- UHD's time after the new origin: the segment starts up to about 40 ms late, same flags, no `LATE_COMMAND`.
- After an abort or the Run's end: the device streams to no reader until it overruns again, and leaves samples and a pending overrun flag in the streamer for the next user of it.

Likelihood is INFERRED and low on a quiet host (the window is one handler wake-up plus one register read). It is not low by construction under H4, the host state the 140 ms bound is taken from: the same H4 runs show about 50 ms of extra delay somewhere between resume and restart (139.1 against 87.2 ms). S9's 20 trials per cell cannot bound a rare race (0 of 40 still allows several percent).

**Alternative (recommended, inside Q1): a second `rx_stop` when the stream ends with an overrun reported since its last block.** When the first STOP's SYNC post returns, the queue is empty and nothing is being processed (`graph.cpp:606–622` `sync_actions`, "Wait for the action queue to be empty AND no action is currently being processed"), so UHD's START, if it slipped behind, has already been written to the radio. A second STOP then clears it on every channel (`radio_rx_core.v:171–183`, and `ST_TIME_CHECK` leaves on `cmd_stop`, `:524–527`). That replaces "ordered by timing" with an order read from source, costs one call on a rare path in `try_end` (`rx.rs:267–303`), and covers abort and two channels. The draft keeps this as a fallback "only if the bench shows it"; the bench cannot show a rare race, and the failure (a new segment silently on UHD's grid) is hard to recognise afterwards. The rig and the FakeDevice need a matching scripted case: a START that survives one stop and is cleared by the second (new sequence S13 below).

---

### 2. Minors

**m0. `Stop` in the gap, then an abort before uhd-rx has ended the silent stream: the tail event exists on the Mock and not on the USRP.** (Rated minor: the window is at most `span` and needs an abort.) uhd-rx ends a silent stream only at `cut + span` (5 ms at 2 000-sample blocks, up to 100 ms at 65 536). An abort in that window lowers the cut to `expected`: `request_cut` keeps the minimum (`crates/ezsdr-radio-uhd/src/provider/core.rs:199–201`; `rx.rs:254–260`, and `refresh_intent` `rx.rs:689–696`).

```
overrun at f (k_f = 1 000); Stop at c = 6 ms; abort (Policy, close) at 8 ms
Mock : at 6 ms rx_settle ends the clock at c and (Q2) emits {overrun, 5 000}   (lib.rs:1274–1300)
UHD  : at 8 ms cut := k_f; finish with lost 0 → no event; clock ended at k_f
```
The shared line agrees with the Mock: the Stop already cut the segment, and a later End finds none running (`crates/ezsdr-radio/src/timeline.rs:475–485`). uhd-rx's behaviour is deliberate for delivery (the comment at `rx.rs:254–256`: an abort ends "a segment still being delivered up to an earlier cut" at the first sample not delivered), so the question is only the event: before Q2 the two differed in the clock's end; with Q2 one emits and the other does not, for the same Action log. The Mock cannot take its event back. Decide which is right and pin it in S6: either uhd-rx emits the tail for the Stop's cut although the abort shortened delivery, or the difference goes to KL-16. The same window exists for a device loss followed by the Policy's abort (S7).

**m1. The draft's UHD statement on when the overflow is returned is inexact, and one consequence follows.** "`recv` returns the overflow only once … its wait has timed out" (§6): when the overrun flag is already set, UHD skips the wait (`rx_streamer_zero_copy.hpp:215`), so after a stall the 100 ms wait never entered S4's numbers — good, the measured 68.7–139.1 ms do not depend on the wait that 4b changes. But when the message lands *during* a wait, the restart is requested only when that wait ends: restart ≤ `f` + wait + 50 ms + arrival. With `r = f` ("none is behind") and the no-cut wait at its 100 ms cap (65 536-sample blocks below about 675 kS/s, S12's third row) that is above `r + 140 ms`. Rare and INFERRED; say in KL-15 that the bound was measured at the bench block length, and add the cell to the post-4b sweep or cap the no-cut wait.

**m2. A tail loss with no report stays silent, and a late report lands on the next segment.** The overrun message travels apart from the data. If the stream is ended by silence at `cut + span` before the message has reached the streamer (`recv` returned plain `Timeout`), `pending` is empty: no tail event although `expected, cut)` was never delivered. The flag then stays set in UHD (`rx_streamer_zero_copy.hpp:149–152`, cleared only at `:236`), and the next segment's first idle `recv` returns an overflow that is not its own (`Pending::Overrun` on a stream that never overran). If the message arrived before the STOP, overrun mode is still set (a user START does not clear it), and UHD also requests a restart (`rfnoc_rx_streamer.cpp:179–186`), which queues behind the Module's START and is cleared at the next stop. When the message arrives after the STOP, overrun mode is cleared on arrival (`rfnoc_rx_streamer.cpp:321–324`) and no restart is requested; only the stale report remains. Same for an overrun after the cut but before the STOP reaches the device. Draft principle 2 ("exactly one, never none") should be worded as "every *reported* overrun"; decide whether a segment ended by silence with `expected < cut` and no report, no missed start and no loss gets a `sequence` tail (the block rule flags such a jump `SEQ_DISCONTINUITY`, UR-18, but a tail has no block to flag). Pin with a FakeDevice case: `Timeout`, stop, new start, then `Overflow`.

**m3. The tail event of a segment that published nothing.** RM-11 gives the time in the primary root "before one exists" (`design/07-radio-model.md:121`). uhd-rx registers the clock at the first block (`rx.rs:586–591`); the Mock's publish path registers it before it knows the link refused (`lib.rs:613` in `publish_rx`), while a fault before the first block does not (`lib.rs:724–766`). So "overrun at the origin, Stop in the gap" gives a root-timed event with `lost` in samples of a clock the Manifest never lists, and MR-19 (b) with a refused first block gives a registered clock with no block. Choose one form for both Providers and add the sequence (S14 below); the draft only says "timed per RM-11".

**m4. Emission order of the tail.** uhd-rx must emit before `clocks.end` (`rx.rs:322–326`) and the tail is the only `RX_OVERFLOW` that follows its segment's last block instead of preceding a block. `mark_artifact` marks "every artifact open at that moment" (RS-30, `design/04-run-and-session.md:211`): whether a capture of the cut segment is still open when the hot-path record is drained was not traced (INFERRED risk; the Mock is synchronous, uhd-rx is another thread). The VH-8 tail parity test should run with a capture and compare the artifact marks, and S7 should fix the order of `DEVICE_LOST` (emitted in `device_lost_locked`, `core.rs:1088`) against the tail on both Providers.

**m5. MR-27's `faults` row against the tail event.** The row is "written when it fires and never revised: `lost` is … `k_g − k_f`" (`design/09-mock-radio.md:179`); with a Stop in the gap the event now says `c − k_f`. Two numbers for one loss in one Manifest. Keep the row (it is the injected loss) but say so in MR-27 and in the draft's contract table, which presents the row's value as the loss.

**m6. `Pending::Alignment` at the segment's end** is still dropped with the stream (`rx.rs:632–638` emits it only with a block). "No loss is silent" should either cover it or say it does not.

**m7. MR-19 (b) under abort.** With the abort floor at `a`, a block the link refused and the Mock dropped produces no event at all (today it is emitted at the drop, `lib.rs:582–597`). That follows from RM-16 (the cut is `a`, nothing was lost before it), but SC-20a's sentence "must not discard the block silently" needs the clause; the draft only deletes RM-17's exception.

**m8. RM-17's new text.** `r` is defined for the three listed cases; state two more: a second overrun inside a gap (the Mock adds the losses and the later `f + gap` governs, `lib.rs:739–757`, `v58_06_overflows_inside_a_gap_extend_the_recorded_loss`), and that the Mock's tail `lost` is `cut − k_f`, not the sum of its reports' `lost` (the block path sums them, `lib.rs:686`), or a tail after two faults over-reports. "At the latest" plus RM-35 `LowerBound` (`design/07-radio-model.md:347`) is consistent; nothing else found that still needs `f + gap` beyond MR-21, MR-27 and the layer-1 oracle the draft lists.

**m9. The rig change** hides nothing the rig showed before, and S4c pins it. Two things to state: the rig now can raise UR-29's 1 s silence check where it could not (`rx.rs:524`; no seed does today), and it still has no backlog (M1) and no UHD cascade (M2), so it proves the timing rule, not the hardware order.

### 3. Notes

- CPU: the short wait matters only while nothing arrives. An armed stream far before its origin now turns about 200–330 times a second instead of 10, each turn taking the chain and streams locks (`rx.rs:174–190`); negligible, but it is shared with uhd-control.
- UHD truncates to whole milliseconds (`rx_streamer_impl.hpp:154`); the 1 ms floor holds; `timeout < 1 ms` at `rx.rs:233` is dead code after the clamp.
- A cut booked during a 68.5 ms wait (65 536 at 1 MS/s) is read one turn late: the first `timeout()` still sees no cut (`rx.rs:522`), the next 1 ms wait ends the stream. Within the draft's "+ ≤ 1 ms".
- Two receive streams: each has its own streamer and thread; a STOP's sync also waits for the other stream's cascade. No problem seen.
- A larger alternative, not for 4b: start the stream so that UHD never restarts it (UHD restarts only in continuous mode, `radio_control_impl.cpp:1335–1337`, `rfnoc_rx_streamer.cpp:318–324`) and let the Module issue its own timed restart on the lattice. It would remove KL-8's rounding, the cascade and the unknown restart instant, but it changes the measured value's meaning and needs a new bench. Worth a line in the Phase 8-end review.

### 3a. Attacked, held

Cases the brief names that came out sound, given the draft's own decisions (the tail is emitted where the pending reports are dropped, with `lost = cut − k_f`):
- **A restart packet after the cut, trimmed to nothing:** `len` becomes 0 and the stream ends (`rx.rs:560–577`); `pending` is taken only when a block is published (`rx.rs:595`), so the tail fires once. A packet that straddles the cut publishes a block with the event and empties `pending`: no tail. Exactly one either way.
- **A cut exactly at `k_f`:** `follow` ends the stream at once (`rx.rs:184`); loss 0, no event; the Mock settles with `cut − k_f = 0` (`lib.rs:556–559`). Equal.
- **A sequence error and an overrun before one cut:** both stay pending (`rx.rs:84–88`; `lib.rs:762`) and the shared helper picks `overrun` (`crates/ezsdr-radio/src/lib.rs:648–658`). Equal, provided the tail uses that helper (S8 should have this case).
- **The Run's end against a Stop:** an orderly end books its cut at the stop instant on both (`lib.rs:1217–1222`; `core.rs:648–670`), so it is the Stop case; an abort cuts at the first sample not delivered and loses nothing by RM-16. Sound except m0.
- **A STOP after UHD's START has reached the radio but before its time:** the STOP clears the command FIFO and leaves the time wait (`radio_rx_core.v:171–183, 524–527`). Sound; the open case is M2's order.
- **Device loss in the gap:** both book the end at the loss's instant (`core.rs:1080–1082`; `lib.rs:1217–1220`). Sound; event order in m4.
- **`quiet` on the host clock under load:** host time never runs behind device time on hardware, so `quiet` can only be late through M1's backlog.
- **MR-19 (b), repeated refusals:** each refusal adds its stretch to `rx.lost` (`lib.rs:582–597`), the reports' sum equals the accepted block's jump (`lib.rs:686`), one event at the first refused sample (`lib.rs:688`). A `Block` link is the port's only link (MR-19, `design/09-mock-radio.md:153`), so no other consumer saw the dropped block.
- **A fault report and a back-pressure loss before one block:** `lost_total` carries the fault's loss into the refusal's (`lib.rs:588`); cause `overrun`; one event. Sound under (b) once a refused publish stops consuming reports, as the draft says.
- **Two receive streams:** separate streamers and threads (note above).

### 4. Omissions in the draft's lists

Tests and literals not listed (search: `grep -rn -E "50_000_000|50 \* MS|50 ms" crates … | grep -i -E "overflow|restart|gap|overrun"` and `grep -rn rx_overflow crates/*/tests`):
- `mr_20_faults_fire_at_their_instants` (`crates/ezsdr-mock-radio/tests/mock_radio.rs:2251–2254`): its second fault is "after the 50 ms gap" and falls inside a 140 ms one.
- `v58_06_overflows_inside_a_gap_extend_the_recorded_loss` (`crates/ezsdr-acceptance/tests/v58.rs:247–253`) and `v58_04_mock_events_reach_counters_policy_and_manifest` (`:143–164`), both with the 160 ms horizon; the draft lists only `:184–225`.
- `bench1115_skip_probe_records_a_receive_overflow_and_its_gap` (`crates/ezsdr-radio-uhd/tests/fake.rs:10009–10012`): depends on the fake's 50 ms skip.
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:760–774` (an overflow per stream in a layout).
- `ur_17_a_missed_start_and_overrun_share_the_block_summary` (`rx.rs:1433–1447`): a 50 ms literal; still valid, but Q4 says no gap literals.
- `vh_8_reports_before_one_block…` and `vh_8_repeated_overruns…` (`parity.rs:914–938`): the second fault's time (50) is the gap, not only a horizon.
- `crates/ezsdr-radio/src/x3xx.rs:414` (the value itself); `rm_20_a_timing_document_with_a_removed_key_is_refused` (`crates/ezsdr-radio/tests/radio_model.rs:302`) is a free-standing document and need not change.

Design texts: MR-27 (m5); SC-20a (m7); RM-11 if m3 chooses a registered clock; UR-18 needs the tail and the second stop; `plan/phase8/bench.md` S4/S9 procedure; KL-7's line (M1).

Sequences to add to §9:
- **S13** (M2): overflow result, then stop within 1 ms and under abort, one and two channels, with a scripted START that survives the first stop → no sample of the old stream after the stop, new segment from index 0 on its own lattice.
- **S14** (m3): overrun before the first block, Stop in the gap → one tail event, the same time form and Manifest clocks on both Providers.
- **S15** (m0): Stop in the gap, abort before `cut + span` → the same event and clock end on both.
- **S16** (m2): `Timeout`, stop, new start, then a late `Overflow` → no event on the new segment, its first block index 0.
- **S9a/S9b** (M1) in place of S9; S10 stated for the cell it can reach.
- S2/S5 with a capture open (m4); S11 with a fault report pending before the first refusal and with an abort (m7); S8 with a sequence report *and* an overrun before one cut (cause `overrun`).

Mutation targets to add: the second stop removed (S13); the tail's `lost` taken from the reports' sum (m8); whichever m0 decides (S15).

### 5. Summary

Majors:
1. **M1** — overrun after a stall; Stop + start booked while uhd-rx still reads the backlog (`r` to `t_o`, about 20–40 ms quiet, to 90 ms under H4): the stop cannot go out before `t_o + span`, so the start is short of lead or missed on the USRP and on time on the Mock. The contract, KL-7's line, S9 (booked at +20 ms, inside that window) and S10 (the fake has no backlog) miss it. Owner: reword what Q1 promises.
2. **M2** — overflow result, then `rx_stop` 0–1 ms later (not ≥ 3 ms as drafted): the STOP can be queued ahead of UHD's restart START, which then survives; one of two channels can restart alone; the next segment runs on UHD's grid with `SEQ_DISCONTINUITY` (the draft's "harmless" case) or starts late.

Recommended alternatives:
- A second `rx_stop` when a stream ends with an overrun pending: order from UHD's SYNC post semantics instead of timing; do it in 4b, not after the bench.
- Decide the tail event for a Stop followed by an abort within `span` (m0), and the form of a tail on a segment that published nothing (m3).
- S9 split by trace into "booked after the result" (must be on time) and "booked before it" (reported missed start expected).

Minors: m0–m9 (§2), none blocks the direction.

Omissions: the tests and texts of §4 (eight test sites, five texts); sequences S13–S16 and the S9 split; the measured bound's block length in KL-15 (m1).

Not checked: no hardware run (every USRP sequence is INFERRED from UHD 4.10 source); that the C API's receive streamer on an X3x0 is the `rfnoc_rx_streamer` read here (M2, m2 rest on it); the kernel's drain order against artifact close (m4); UR-17/25/33 texts line by line; `hw_*` tests beyond the draft's list. Cost of this run: not visible to me.

## sol-plan

**Phase 8 stage 4b test-plan review.** I reviewed `work/4b` at `d50073d2`. Before opening the decisions or test plan, I recorded the failure scenarios below from the current code and contracts. I made no changes and ran no builds or tests. UHD 4.10 behavior cited through the design and Fable review is **INFERRED** for this review. [Decisions (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/decisions.md:3`) · Test plan (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:1`)

#### Independent failure scenarios

| Operation sequence | Wrong observable outcome |
|---|---|
| Enter a silent receive without a cut, then book Stop and Start during the wait. Vary the block duration and the host clock’s progress. | Stop is issued too late to start the successor at its planned origin, or the rig advances device time while its silence monitor still sees no elapsed host time. rx.rs (`crates/ezsdr-radio-uhd/src/provider/rx.rs:205`) · differential.rs (`crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:694`) |
| Return an overrun, then Stop before any recovery block and start a successor on the same streamer. Repeat with a consumed report, a sequence-only report, and a failure on either stop. | The old restart survives, a second stop is issued without a pending overrun, or a device call follows published loss. rx.rs (`crates/ezsdr-radio-uhd/src/provider/rx.rs:267`) · UR-29 (`design/18-uhd-radio.md:362`) |
| Report loss, then cut before recovery, at the first undelivered sample, or through the first recovery block. Include a segment with no registered clock, mixed reports, and a device loss. | The tail is missing, duplicated, zero-length, charged beyond the cut, or stamped on the wrong clock. rx.rs (`crates/ezsdr-radio-uhd/src/provider/rx.rs:533`) · MockRadio (`crates/ezsdr-mock-radio/src/lib.rs:1274`) · RM-11 (`design/07-radio-model.md:121`) |
| Stop in a reported gap, then abort before the receive owner finishes. Compare an ordinary abort in the gap and an abort while a receive is in flight. | A booked cut moves, MockRadio uses the restart cursor as its delivered floor, or the receiver publishes data after abort. rx.rs (`crates/ezsdr-radio-uhd/src/provider/rx.rs:251`) · MockRadio (`crates/ezsdr-mock-radio/src/lib.rs:1217`) |
| Begin a successor, read stale metadata, then receive either a legitimate first-block jump or a genuine successor overrun. Separately book a start while the old backlog drains. | Stale metadata creates a successor loss, real metadata disappears, or a missed start shifts the planned clock origin or goes unreported. rx.rs (`crates/ezsdr-radio-uhd/src/provider/rx.rs:397`) · UR-18 (`design/18-uhd-radio.md:241`) |
| A pending fault reaches a refused publish, then another refusal, acceptance, Stop, or Abort. Exercise a fractional sample grid and overlapping faults. | A refusal consumes the report, reports loss early or twice, advances the abort floor, or resumes at the wrong lattice point. MockRadio (`crates/ezsdr-mock-radio/src/lib.rs:579`) · MockRadio fault handling (`crates/ezsdr-mock-radio/src/lib.rs:735`) |

#### Findings

1. **Major — S2a does not exercise the wait that causes #108.** S2a books both actions and advances to `cut + span` before calling `turn()`. `turn()` polls the plan first, so both the old and new implementations take the existing cut-pending path. N108-01 cannot be killed by that sequence. S3 repeats it with a larger block. S2b is closer, but `FakeFault::Overflow(5 ms)` is an absolute device-time threshold tested against the sample cursor, and `Direct::start()` already waits until about two seconds plus 1 ms. Settling for a fixed interval does not establish that the Stop lands inside an outstanding no-cut receive. Use a scripted receive that records its supplied timeout, books Stop and Start during that call, and advances both clocks before returning. Check the planned origin independently, the actual issue margin against the device lead, and the first published block. S12 remains a direct killer for N108-01/03. Its 390,625 S/s case needs an explicit ratio fixture because the usual `rx()` helper fixes 1 MS/s. Also resolve the wording of “each wait”: P1 deliberately retains a cut-pending arm that can exceed one block plus 3 ms, and UHD’s per-packet timeout behavior is **INFERRED** from the cited source. S2/S3 and S12 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:64`) · rx.rs (`crates/ezsdr-radio-uhd/src/provider/rx.rs:205`) · Ratio fixture (`crates/ezsdr-radio-uhd/src/provider/rx.rs:763`) · Direct (`crates/ezsdr-radio-uhd/tests/fake.rs:1476`) · FakeDevice (`crates/ezsdr-radio-uhd/src/device.rs:1720`) · UHD citation (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/design-draft.md:331`)

2. **Major — P5 needs separate checks for retaining the tail and stopping delivery.** Deliver through index 999, report a sequence error, book a Stop at index 6000, then publish the Run’s Abort while a receive is in flight. Have it return `3000,7000)` before uhd-rx reads `Shutdown`. If `refresh_intent` simply preserves 6000, `samples()` can publish `[3000,6000)` after abort and consume the report in a block event. S15 calls `command(Abort)` directly and misses that path. Exercise both shared-intent and command paths and assert delivery, tail, and clock end independently. Also test a Stop already handled off-lattice, followed by Abort before the next sample instant. The Stop has taken effect even though its rounded cut is still numerically ahead of `now`. Keep the loss-free and plain-abort controls. N108-35/35b need to distinguish these paths. [P5 and S15 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:188`) · Binding decision (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/decisions.md:11`) · Provider abort publication (`crates/ezsdr-radio-uhd/src/provider/mod.rs:717`) · Receive completion (`crates/ezsdr-radio-uhd/src/provider/rx.rs:533`) · RM-16 (`design/07-radio-model.md:177`)

3. **Major — P6’s origin check is a partial stale-report filter.** A ends on silence, B starts, A’s stale overflow returns at or after B’s origin, and B’s first kept block has a jump or B ends before a block. P6 can charge A’s loss to B and trigger a second stop on B. That hardware arrival is **INFERRED**, but the resulting software attribution is directly testable. A second hole exists before the origin: a failed non-loss stop leaves the chain `Uncertain`, establishes no `last_stop`, and still allows `begin_result`, so `not_before` is an incomplete proxy for “successor.” Keep the early gate if the residual is explicitly accepted, and test its boundary, the failed-stop path, and the three gated metadata classes. S16a’s existing gapless behavior checks the block and event only. The new diagnostic needs a separate assertion. S16b can kill N108-36. S16c should pin genuine successor reports independently of the fake’s early scripted report. P6/S16 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:196`) · Fable m2 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/fable-review.md:87`) · Stop and successor setup (`crates/ezsdr-radio-uhd/src/provider/rx.rs:284`) · RxChain (`crates/ezsdr-radio-uhd/src/provider/core.rs:160`)

4. **Major — S2c expects a capture mark the coordinator excludes.** Deliver `0,kf)`, report an overrun, and end the Run in the gap. The tail is stamped at `kf`, which is the artifact’s exclusive end. Finalization drains the event and attaches marks only within `[first,end)`, so the Manifest can correctly contain the tail and zero marks on both Providers. Decide the artifact behavior before implementation. A test with the current rule should check the tail and absence of a mark on that artifact. Requiring a mark there changes the coordinator’s attribution rule. Separately, the collector accepts timestamps without consulting the clock registry, and ended clocks remain convertible. N108-23/32 are equivalent for the proposed observations. Drop them or use an event-sink spy if emit-before-end remains a requirement. [S2c and mutation assumptions (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:129`) · Artifact finalization (`crates/ezsdr-kernel/src/coordinator/ending.rs:259`) · Event collector (`crates/ezsdr-kernel/src/event.rs:571`) · Clock conversion (`crates/ezsdr-kernel/src/time/domain.rs:365`)

5. **Major — S13’s oracle needs “since the last kept block,” and S18 misses loss between the two calls.** Return Overrun, then Timeout, then reach the cut. A second stop is required, yet S13c’s “last receive result” rule would reject it. Conversely, return Overrun and then publish a gapless or recovered block. An implementation that leaves the second stop armed can pass the listed negative cases. Add both sequences and a coverage assertion that the rig actually produced a survivor. In S13a/b, make the survivor obstruct the successor: the fake’s normal start overwrites an active run unless it has a retained tail, so merely leaving `next` intact can let the functional outcome pass while the call-count assertion kills N108-40. Finally, publish loss after the first stop returns `Ok` and before the second. Without a check or ordering at that boundary, the second call can use a lost device. The first-error, second-error, and already-lost cases in S18 are useful but do not exercise it. S13/S18 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:213`) · Fake start behavior (`crates/ezsdr-radio-uhd/src/device.rs:1311`) · Pending consumption (`crates/ezsdr-radio-uhd/src/provider/rx.rs:593`) · Loss publication (`crates/ezsdr-radio-uhd/src/provider/core.rs:1073`)

6. **Major — S9b can pass without exercising the backlog exception.** Submit while uhd-rx is asleep, well before it resumes. The start can already be late when the reader wakes, without draining a single buffered packet. S9 calls that B and accepts the late start. On the fake the same booking can miss for the same reason, despite the planned on-time S10a expectation. A pre-submit stamp can also precede the overflow result while the actual booking follows it. Classify using the relevant stream’s actual booking and its `out_of_sequence == false` result. For B, require evidence the reader had resumed and was still returning pre-overrun samples, or classify the trial as inconclusive. Keep S10b for the Module’s response and S10a for the boundary the fake can demonstrate. Require a successor block and the expected A tail explicitly: the listed bad-tail check can pass when the tail is absent. N108-62/63/64 distinguish their stated branches once the classifier has the right inputs. The probe’s real timestamps are suitable for ordering two trace observations. Fixed host-time sleeps or host-duration assertions would make that rehearsal scheduler-dependent. S9/S10 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:256`) · Owner’s M1 decision (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/decisions.md:16`) · Control booking (`crates/ezsdr-radio-uhd/src/provider/control.rs:325`) · Probe result fields (`crates/ezsdr-radio-uhd/tests/common/probe_device.rs:311`) · Receive stall (`crates/ezsdr-radio-uhd/src/provider/rx.rs:220`)

7. **Major — S8 and S19 contain fixtures that cannot prove their stated result.** In S8’s parity case, schedule the Mock’s second fault at `G`, then Stop strictly before the first restart at `G`. The second report cannot contribute to that Mock tail. Put both fault instants before the cut and independently confirm both reports reached the pending state. For S19, create a capacity-zero Block link and attempt the first publish. The normal HostLink rejects creation, so the test never reaches a refusal. Use a test `DataLink` that returns `Full` on the first publish. Explicitly retain the Mock exception that registration precedes link acceptance. S5/S5b can distinguish a packet trimmed away from a published jump, S8c distinguishes a clipped tail from the report sum, and S14 distinguishes root from SampleClock stamping. S6p needs a delivered prefix because it asserts on registered clocks. Add a nonzero stream or explicit source assertion and the nine-byte payload check to a tail case, as the existing parity helper does for block events. S5–S8/S14 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:138`) · S19 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:239`) · Mock fault ownership (`crates/ezsdr-mock-radio/src/lib.rs:699`) · HostLink capacity (`crates/ezsdr-link-host/src/lib.rs:66`) · Parity event helper (`crates/ezsdr-acceptance/tests/parity.rs:851`)

8. **Major — S21 can publish a comparable bound from incomplete restart evidence.** Give one rate its required count of `Success` captures, with a missing restart estimate in one of them, and give another rate a valid estimate. The current summary counts the first rate as complete and takes the maximum over the remaining estimates. A row builder that trusts `unmeasured_reason` can emit `Measured`. Test this exact input and removal of the row at the group boundary. Build a record through the hardware-source conversion path with synthetic evidence and check the limiting condition and comparison above the profile. The Fake-source round trip replaces every supplied row with the same `Unmeasured` reason, so it cannot distinguish a correct measured row from an always-unmeasured row. N108-60/61 need distinct values and this missing-estimate case. S21 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:244`) · Overflow summary (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:2700`) · Fake-source conversion (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:176`) · S4 completeness predicate (`crates/ezsdr-radio-uhd/tests/common/s4bench.rs:563`)

9. **Major — The staged mutation gates have two concrete breaks.** In **4b-2b**, call the payload helper with an Overrun and `None`. With P10’s zero filter, D38’s `unwrap_or(0)` produces the same `None`, or its anchor disappears if the helper is rewritten with a filter. Retire D38 when P10 lands. In **4b-3**, connect S21’s value row to the group. P800’s `values: vec![]` anchor disappears. Its failure class remains reachable, because the group could emit an unmeasured row for complete evidence. Retarget it to the actual group row and the measured-path assertion in finding 8. Preserve the no-group fallback reason assertion at `measurement.rs:4590` when replacing the old text, and retain or retarget the summary-test filters on P797–P799. The S4A removals are justified with the exemption removed and S4A-77 retained. Retire F32 with the tail in **2b**, and F45/F46/D10/D34 only with the MR-19 conversion in **2d**. F46’s equivalence depends on that conversion. Preserve the separate-fault and hot-payload checks in S11 so premature consumption, no reported loss, and the wrong event path remain observable after the old helpers go. Staging and retirements (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:35`) · D38 (`plan/phase4/tools/mutations.json:376`) · P797–P800 (`plan/maintenance/tools/mutations.json:8185`) · Fallback-reason test (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:4568`)

10. **Minor — Tighten the remaining timing and MR-19 oracles.** S4c’s `host ≥ device` accepts advancing twice as far, which can make a silence test go green early. Assert the clock increment at the advance seam, where elapsed real time can be excluded. Keep N108-04 as the direct pin and treat N108-02/05 as conditional integration kills. S23’s `>=` admits an arbitrarily long fake restart. Use the exact rounded sample on both paths, with the scripted path’s sample cursor independent of the host call instant. S11a’s 300 ms absence also conflicts with accepting at `a + G`: at 140 ms, that publish and the one at `a + 2G` both occur before 300 ms. Drain between the initial refusal and `a + G` for S11a, and use the longer absence for S11b. Keep the existing fractional-root case. S11c/d then give meaningful kills for N108-50–53 and T37 remains useful. In P3, spell out that `Full` controls the radio’s floor, with `DropNewest` handled as link loss under the current non-`Full` path. S4c/S23 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:71`) · S11 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:234`) · Host clock (`crates/ezsdr-radio-uhd/src/host.rs:292`) · Existing fractional case (`crates/ezsdr-mock-radio/tests/mock_radio.rs:2195`) · Link outcomes (`crates/ezsdr-link-host/src/lib.rs:95`)

11. **Minor — S7’s direct tests cannot prove Policy or Manifest ordering.** Keep S7/S7c for exact tail count, stamp, payload, and clock end, and use a Run assertion for the Policy termination. `EventCollector` drains hot bodies before control bodies within a drain, and the coordinator appends separate drains in delivery order. A stamp comparison cannot establish delivery order. P12 can leave cross-Provider order unspecified while still asserting one `DEVICE_LOST`, the exact tail, and abort termination. The existing Mock sequence-loss-then-loss test is a useful fixture to extend. S16d/S17 are appropriate negative controls. For N108-22, S16d is the discriminating control because it explicitly leaves a positive distance to the cut with no report. S7/P12 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:160`) · Collector order (`crates/ezsdr-kernel/src/event.rs:500`) · Policy processing (`crates/ezsdr-kernel/src/coordinator/stepping.rs:201`) · Mock loss fixture (`crates/ezsdr-mock-radio/tests/mock_radio.rs:2549`)

12. **Minor — Complete the hardware audit and amendment list.** S1/S24’s profile-derived expectations and the single literal profile pin are sound in principle. For B5, the hardware test does assert event/continuity consistency through `rehearse_overflow`; the named unit rehearsal exercises synthetic metadata and continuity, and the plan’s additional fake B5 integration test was removed. S4’s FakeDevice rehearsal checks that the bound line exists, while S22’s pure tests check the verdict. Add a sweep-cell exceedance to the latter: `s4_decisions` currently excludes `cell.sweep` from its bound, but Q4 sends any above-bound cell back to the owner. The raw UR-17 duration and n47 buffer-stall tests can stay observations. In the texts, make Vision §23’s fixed-jump sentence an amendment, add MR-25’s refused-first-publish registration exception if P4 stands, and update UR-34’s measurement test references when S21 changes the value row. Reconcile the proposed capture-mark assertion with RS-30 and finalization as in finding 4. Preserve both distinct quantities in the prose: UHD’s cited internal 50 ms timer and the profile’s provisional reader-resume bound. The hardware timing explanation remains **INFERRED** here. Audit and texts (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md:381`) · B5 helper (`crates/ezsdr-radio-uhd/tests/common/mod.rs:919`) · Removed Fake B5 copy (`crates/ezsdr-radio-uhd/tests/fake.rs:3204`) · S4 bound scope (`crates/ezsdr-radio-uhd/tests/common/s4bench.rs:660`) · Q4 (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/decisions.md:7`) · Vision §23 (`design/vision/04-execution-model-and-data-contracts.md:286`) · UR-34 (`design/18-uhd-radio.md:427`)

**Verdict: FAIL at §9 step 3. Revise the plan and recheck the major findings before implementation.**
## review-4b1

## Sub-step review — 4b-1 (commit 4b579533 against a4ad013a)

Reviewer: Claude Opus (fresh, read-only). Read: the brief, `test-plan-v3.md` (§0 P1/P2/P11, §1, §3 4b-1, §4.1–4.3, §5, §6, §7), `decisions.md`, `brief-4b1.md`, `report-4b1.md`, `rows-4b1.log`, `full-4b1.log`, and the whole diff `git diff a4ad013a 4b579533` with the connected code (`rx.rs` turn/receive/timeout/quiet_span, `test_support.rs` LockSpy, `differential.rs` World/Scripted/run, `host.rs` manual clock, `device.rs` FakeDevice rx_recv, `uhd.rs` rx_recv, fake.rs `Direct`). I built nothing and ran no tests. I ran `cargo fmt --all --check` in the main tree (clean, HEAD 4b579533). Test and row outcomes are taken from `rows-4b1.log` (22/22 killed) and the report.

**Verdict: PASS.** No major. 6 minor, 8 notes.

### Checklist

**1. Sequences → tests, rows.**
- S12 → `ur_17_the_receive_wait_without_a_cut_is_one_block_plus_the_allowance` (`rx.rs:2135`). It uses literals 5 ms, 68.536 ms and 100 ms, with a 512-ratio fixture through `rx_with_plan`, and reads `min_recv_timeout` after one `turn()`. That is as planned, and the literals are independent of the code under test. N108-01 and N108-03 are killed.
- S2a/S3 → `ur_17_a_stop_and_start_booked_during_a_no_cut_receive_start_at_their_origin` (`rx.rs:2165`). The checks are:
  - the third call's timeout against the literal (`wait_ns`);
  - lateness: `issued ∈ [e, e + span + 1 ms]` (`rx.rs:2223`);
  - the origin computed independently (`e + call + 3 ms + 10 ms`, literals);
  - the issue margin `origin − at ≥ DEVICE_LEAD_NS`;
  - the `rx_start {origin}` call;
  - the first block at index 0 with `NONE` flags;
  - no `LATE_COMMAND`, and `rx_before_origin` 0.

  I traced it by hand on both codes. On the old code the third call's hook advances to `t_call + 100 ms`, so the stop goes out at about `e + 99 ms`, past `e + span + 1 ms` for both block lengths. That fails on lateness alone, as the report says. On the new code the 65 536 case gives a 1 ms floor wait and the stop at `e + 68.536 ms`, inside the bound. N108-01b is killed.
- S4c → `rm_26_the_rig_s_host_clock_moves_with_its_device_time` (`differential.rs:480`). The `Instant` bracket is right (`real_before` taken before the `before` read, `elapsed` after the `after` read), and `given == delta` is kept as a secondary check. The integration check is `…_follows_set_now` (`:498`). See m2 for the rows.
- S4/S4b: the exemption, `gap_excuses`, `excused_blocks`, the `excused` return and the three tests are deleted, and `DEVICE_LEAD_NS` is no longer imported. `LATE_START` stays, because `Scripted::rx_start` uses it. N108-02 and N108-05 are kept and killed.
- S20: `Reach` is asserted (`differential.rs:150–151`), with `gap()` from `timing()` and the horizon floor `+ (gap() − 50 ms)` (`:89`). See m3.
- S2b → `fake.rs:6299`, smoke only (see m4). The Mock counterpart → `mock_radio.rs:2767` (see m5).
- Retired rows: the twelve deleted ids (S4A-48, 49, 51, 52, 53, 54, 55, 64, 65, 66, 67, 78) are **exactly §4.2's twelve**. This answers the orchestrator's bracketed question: S4A-55 and 64–67 are in §4.2's second and third rows. No row in any of the six lists still filters on the three deleted tests (grep over `plan/*/tools/mutations.json`). Every surviving row with the `rm_26_the_timeline_against_uhd_rx` filter was re-run and killed: J52, P1058, N67F-07, U32, U40, R13, R14, and B12 as well. The row count goes 2 715 → 2 709 against `a4ad013a`; the report's 2 712 → 2 706 is the same delta against `272bc6ff` (see n8).

**2. Scope.**
- Production change: `Rx::recv_timeout` only (`rx.rs:450–460`). The cut arm is the same arithmetic as before. The no-stream case still returns `RECV_TIMEOUT`. The single clamp is kept, so B12 still matches once. A stream with a cut pending is unchanged.
- The host-clock advance is confined to the rig. `RigHost` and `World::set_now` sit in `differential.rs`, which is `#[cfg(test)] mod differential` (`rx.rs:708–709`). `FakeDevice`, `host.rs` and every other path are untouched. The rig's `Scripted` delegates only `apply` to the inner fake, so moving the inner fake's clock has no other effect there (`differential.rs:786–797`; `time_now` is the world's).
- On the real path (`uhd.rs:1006`), the timeout only shapes UHD's wait. A silent stream (before its origin, or in an overflow gap) now loops every block + 3 ms instead of every 100 ms. `receive(Timeout)` only calls `timeout()`, with no rows or stats (`rx.rs:462–467, 518–531`), so nothing else changes.
- Missing from scope: none. The `rx_stop` hook belongs with S18 in 4b-2c. Extra: none.

**3. "every / each" claims.** UR-17's new "each is at least 1 ms and at most 100 ms" is true: one clamp covers both arms. Its purpose clause is not (m1). UR-25's "starts at its origin, except …": see n5.

**4. Texts.** UR-17's wait wording, UR-25's deleted sentence replaced by the M1 sentence, and KL-7's "Also" bullet replaced by the owner's M1 line (it matches decisions.md M1, the backlog included). The §6 rows were added in spec 18 (3) and spec 09 (1). Issues: m1, m6, n4, n5, n6.

**5. LockSpy hooks.**
- The hook is popped in a `let` statement (`test_support.rs:568`), so the `rx_hooks` guard is dropped before `hook(timeout)` runs.
- `call` holds no LockSpy mutex while `f` runs (`enter`/`leave` release theirs).
- `a_one_shot_rx_recv_hook_runs_outside_its_queue_lock` (`:1146`) would deadlock on a hook run under the guard.
- Existing tests see no behavioural change: an extra push to `rx_timeouts`, and the inner call when no hook is queued.

**6. Maintainability.** There is no second copy of the wait or gap computation in the tests, beyond literals by design. `gap()` is one helper, used by `blocks`, `Scripted` and `Reach`. No leftovers of the exemption remain (grep `excus|gap_excuses|243, 766` over crates, design and KL finds none). Small items: n1, n2, n3.

### Findings

**m1 (minor): UR-17's purpose clause covers both arms, and "within one block" is short by the allowance.**
- `design/18-uhd-radio.md:232`: "…with a cut pending it is the time to the cut plus the same span; each is at least 1 ms and at most 100 ms, so that a `Stop` and a `start_rx` booked while a call waits are seen within one block."
- Plan P1 limits the claim to the no-cut wait. With a cut pending ahead of now (UR-25's e1/e2 switch, an off-lattice cut), a booking made during the call is seen only after `(cut − now) + span`, up to 100 ms.
- Even for the no-cut arm, the wait is one block **plus** 3 ms, and one 1 ms floor turn follows (S2a's own bound is `cut + span + 1 ms`).
- The same "within a block" is in the doc comment at `rx.rs:447–449`.
- A "so that" clause is also rationale inside a rule (AGENTS §2, DA-1).
- Fix: state the no-cut bound only ("a `Stop` and a `start_rx` booked during a no-cut wait are read within one block plus the allowance"), or drop the clause.

**m2 (minor): no row pins S4c's seam, and N108-04's "direct pin" is the integration check.**
- N108-04 mutates `self.host.advance(t - self.now)` in `World::set_now` (`differential.rs:654`).
- Its filter `rm_26_the_rig_s_host_clock_` also selects the seam test. That test calls `RigHost::advance` directly (`:483–486`) and never reaches `set_now`, so only `rm_26_the_rig_s_host_clock_follows_set_now` (`:498`, `host ≥ device`) can kill it.
- No row mutates the seam body (`RigHost::advance`, `:600–604`: the `ticks as u64 * 5`, or the `advance_for_test` call). The `Instant` bracket, the plan's answer to N2, is therefore pinned by no row.
- An over-advance (`* 10`) would be caught by the seam test, but nothing runs that mutant. The integration check's `>=` cannot catch it.
- Fix: add a row such as `Wall::from_nanos(ticks as u64 * 5)` → `* 10`, killed by the seam test. In the record, name `…_follows_set_now` as N108-04's killer.
- INFERRED: N108-04's kill relies on seed 1's run taking well under its about 2 s of device time in real time. Otherwise the manual clock's real-time creep alone could satisfy `host ≥ device`.

**m3 (minor): S20's `cut_in_gap` counts terminal cuts.**
- `differential.rs` (the `Reach` block in `compare`) tests `any(segment)` for a cut inside an overrun's gap, the last segment included.
- The #108 shape needs a successor. The deleted `gap_excuses` looked at `hits[index − 1]` for a segment `index > 0`.
- As a result, `seen["a cut inside an overrun's gap"] >= 3` can be met by sequences whose cut in a gap is a final `Stop` or `Provider::stop` with no following `start_rx`. The coverage assertion would then not prove the #108 shape is still generated.
- Fix: count only a segment followed by another (`segments.windows(2)` / `zip(segments.iter().skip(1))`).
- INFERRED: how many of the 24 reported hits have a successor is not checked.

**m4 (minor): S2b's name and §6 row claim a restart "in an overflow's gap" that the test does not set up or check.**
- `fake.rs:6299`: `Direct::start` waits to `t0 + 1 ms` (`fake.rs:1477–1481`). The scripted `Overflow(5 ms)` fires at the first receive call, because the cursor is at about `t0` (`device.rs:1709–1720`), and it skips 50 ms. `settle(100 ms)` then books the Stop at about `t0 + 101 ms`, after the gap has ended (INFERRED from those lines).
- The test asserts no overflow either.
- The comment calls it a smoke test, which is honest. But the name and spec 18's §6 row (`design/18-uhd-radio.md:558`: "a receive Run on the fake with a scripted overflow, then a Stop and start_rx") read as the #108 shape.
- Fix: rename the test or its row to what it pins, or add one assertion that an `RX_OVERFLOW`/overflow report occurred before the booking.

**m5 (minor): the Mock counterpart does not pin its premise.**
- `mock_radio.rs:2767` asserts the successor's origin against `cut + 15 ms`, with `cut` read back from the Mock's own clock record.
- It asserts neither that the overrun fired (fault row `applied: true`) nor that the cut falls inside the gap (`cut` = T0 + 10 ms, after the fault at 1 ms). Whether the actions apply at 2.010 s or at the step's start is the harness's semantics; the report says it was confirmed only in a scratch copy.
- A harness change that moved the cut before the fault would leave the test green with nothing tested.
- Fix: `assert_eq!(fault_rows(&harness, "rx_overflow"), [(true, …)])`, and the cut equals the literal `2_010_000_000` in the root.
- Its comment's "50 ms restart gap" literal (`:2769`) goes stale at 4b-2a, for 4b-2a's audit.

**m6 (minor): the new spec 09 §6 row has four cells in a three-column table.**
- `design/09-mock-radio.md:314` is written in spec 18's four-column shape (`test | setup | expected | rules`). The table header at `:289` is `| test | expected | rules |`, and its neighbours at `:313` and `:315` have three cells. The row renders with its last cell dropped.
- Fix: merge the setup into the "expected" cell.

### Notes

- **n1:** the dead arm `timeout < Wall::from_millis(1)` at `rx.rs:233` stays (P1 allowed its removal, since the clamp floors at 1 ms). It is harmless and can go in a later sub-step.
- **n2:** S12 builds its three shapes on three rigs (`rx.rs:2140–2153`): `rig_with` on the default (real) clock for the first, `rig_on` with a manual fake for the second, and `rx_setup` + `rx_with_plan` for the third. One `rig_on` manual path with `rx`/`rx_with_plan` by ratio would read as one fixture. It is not a wall-clock judgement: there is no time assertion.
- **n3:** S2a's independent origin uses `spy.rx_packet_samples(0).max(block_len)` (`rx.rs:2225`), where n5's formula is `block * 200` (`differential.rs:165`). It is equal today; the literal `block_len` would couple the oracle to nothing in the fake.
- **n4:** UR-17 and UR-25 gain no Changes-table row in spec 18 or spec 09, and the new tests appear only in §6 rows, not in UR-17's or MR-21's `*Test:*` lines. That is consistent with how `ur_25_a_stream_silent_before_e1_switches_before_e2` is listed. Presumably both are added once at the 4b gate (`rules.py` passes); for the orchestrator.
- **n5:** UR-25's new sentence and KL-7's line say a restart booked after an overflow "starts at its origin" without marking the USRP case INFERRED. Q1's caveats (a retune restart keeps about 0–2.5 ms of margin; the stop-after-UHD's-restart order holds by timing only) and M2's second stop arrive at 4b-2c. This is fine for a squashed landing; check that the owner's wording still reads right after 4b-2c and bench S9.
- **n6:** KL-7's "a host stall that keeps uhd-rx from running past an origin" (`KNOWN_LIMITATIONS.md:66`) reads as "keeps it from running beyond"; the meaning is "keeps uhd-rx from running until past an origin".
- **n7:** KL-7 dropped "no `RX_OVERFLOW` reaches the user for that overrun", but the code still drops a pending report when a cut trims the last packet, until 4b-2b's tail. This is interim only, and KL-8 gains the tail text at 4b-2b.
- **n8:** the implementer's checks and row counts are against `272bc6ff`. `a4ad013a` (#102) sits between that base and this commit. `full-4b1.log` shows the orchestrator's full `remote_mutate.py all` started at 19:41, with no result yet; that run covers the combined tree. Plan §4.2 calls S4A-64 "identical" to S4A-77, but they differ (+3 ms against +1 ms on the same anchor). S4A-77's killer `ur_26_the_stop_s_device_lead_guard_is_exact` pins the guard to the tick, so it kills both (INFERRED from the test's assertions), and the retirement stands.

INFERRED, not blocking: the rig's manual host clock also runs at real speed between advances (`host.rs:121–131`), so `quiet` in `rm_26` can turn true earlier than device time warrants. This was worse before, with no advance at all. `ended` also requires device time `≥ cut + span`, so no verdict should depend on it; not verified per seed. S2a's bounds are in device time and are immune.

## review-4b2a

## Sub-step review — stage 4b, sub-step 4b-2a (Opus, read-only)

Commit `f6f706b9` against `4b579533` (20 files, +351 −150). Inputs read: the brief, `test-plan-v3.md` §1 (4b-2a row), §3 (S23, S24, S1), §4, §5, §6, §7; `decisions.md`; `brief-4b2a.md`; `report-4b2a.md`; `rows-4b2a.log`, `rows-4b2a-b.log`; `review-4b1.md`. Code read: the whole diff, plus the code around it (`device.rs` fake `rx_recv`, the Mock's fault path `lib.rs:725–766`, the layer-1 oracle `timeline.rs:262–439`, the rig's `compare`/`Scripted`/`World` in `differential.rs`, `rx.rs` `turn`/`receive`/`try_end`/`timeout`).

**I built and ran nothing.** Test outcomes come from the implementer's report and from the row logs, which list every row as `killed`. The orchestrator's full-list run (`full-4b2a.log`) was still in progress when I wrote this.

**Verdict: PASS.** 0 major, 4 minor, 7 notes.

---

### Findings

#### Minor

**m1. The rig's new skip rule decides "fired" from the plan's cut instant. It ignores the chain and the sample grid, so it can report a failure that is not there. It never hides a defect.**
The rule is at `differential.rs:262–275`, with `planned_cut` at `:253–259`. It keeps a fault `f` on segment `i` when some overrun `f1` already in `hits[i]` satisfies `f1 ≤ f < f1 + gap()` and `f1 < instant(planned cut)`. The Scripted device works differently (`differential.rs:897–921`). At every call it fires the first unfired fault with `ceil(f) ≤ k` on the front run, where `k` is that run's sample cursor. Wall time plays no part, and the call does not depend on whether `f1` was itself placed by the skip rule. uhd-rx always makes that next call. After an `Overflow`, `expected` stays below the cut (`rx.rs:482–485` only records the report), so `recv_len > 0` and `turn` calls `rx_recv` again (`rx.rs:231–247`). Three shapes make the oracle and the device disagree:
- **(i) A chain of overruns.** Take 1 MS/s, 2 000-sample blocks and a 140 ms gap. A has an overrun at f1 = T0+10 ms. A Stop is booked and cuts A at c = T0+30 ms, so B's origin is about T0+45 ms. A second overrun at f2 = T0+40 ms falls inside f1's gap. The skip rule correctly puts it on A, because f1 < c. A third overrun at f3 = T0+160 ms lies after f1 + gap = 150 ms and before f2 + gap = 180 ms.
  - Oracle: `find` reaches f2, then `f2 < instant(c)` is false, so the rule falls back to `rposition` and puts f3 on B.
  - Device: after f2 the cursor is at 180 ms. The next call on A has `kf = max(160, 180) = k`, so f3 fires on A.
  - Result: the oracle predicts a jump on B that never comes. The test fails although uhd-rx is correct.
- **(ii) The sub-sample window.** Suppose f1 falls strictly between `instant(cut−1)` and `instant(cut)`. Then `ceil(f1)` equals the cut, and uhd-rx stops on `len == 0` before any call could fire f1 (`rx.rs:231–236`). f1 never fires on A. After the pop it never fires on B either (`*f >= origin`, `:906`). The oracle still counts f1 as "fired", so a later f2 in its gap at or after B's origin goes to A. The device fires that f2 on B.
- **(iii) The floored cut,** which the implementer named. An overrun in `[plan cut, floored cut)` does fire on A, but the rule falls back for faults in its gap.

In all three cases the oracle's error makes the test fail; none of them lets a defect pass. Masking would need the oracle to predict a wrong observation that a buggy uhd-rx also produces. The rule only moves faults onto a segment after its cut, where nothing can be observed, so it cannot do that. According to the report, none of the 1 000 seeds reaches these shapes at 140 ms (INFERRED: I did not run them). 4b-2b and 4b-2c re-run the 1 000 seeds with the tail and the second stop, so a false failure is cheaper to remove now.
*Suggested fix:* record with each hit whether it fired on its segment. A hit fired if the skip rule placed it, or if `segment.sample_at_or_after(f1) < planned cut`; test that flag instead of `f1 < instant(cut)`. That fixes (i) and (ii). (iii) needs the floored cut, which `compare` computes only after `hits`. Leaving (iii) as a stated limitation in the comment is acceptable.

**m2. The new "after the host has read the samples buffered" clause is labelled VERIFIED but no text cites the source it was verified against.**
The changed lines are spec 07 §2 (`07-radio-model.md:20`, "(audit Finding 3, VERIFIED)"), spec 18 §2 (`18-uhd-radio.md:27`), spec 02 §2 (`02-stream-contract.md:26`) and KL-8's Status (`KNOWN_LIMITATIONS.md:76`).
- Audit Finding 3 verified the 0.05 s constant, not when the delay starts.
- The design draft directs the change "with §6's source lines" (`design-draft.md:222`). Those lines are `rx_streamer_zero_copy.hpp:209–245, 306–332` and `radio_control_impl.cpp:105–125`, and none of the edited texts carries them.

KL-8 has a further problem. Its "When" line measures "68.7–87.2 ms after the reader returns", meaning after the reader resumes from its stall (the bench's `restart_after_resume_ns`). Its Status line then says `OVERRUN_RESTART_DELAY` "runs from the reader's return". Read with the same meaning, that says the 50 ms runs from the resume. That contradicts spec 07 §2 and the source, which have it run from the overflow result after the drain; that is why 50 ms reads as 69–87 ms. KL-7's line (`:66`) already keeps "resumes" and "reading the buffered samples" apart.
*Suggested fix:* in KL-8's Status, write "runs from the overflow result, which `recv` returns after the host has read the buffered samples". Add the UHD file:line references to the §2 lines, or to KL-8's Evidence.

**m3. KL-8's Evidence line does not cite the records behind its new figures.**
`KNOWN_LIMITATIONS.md:75` still lists only `plan/phase7/bench-results.md` and `plan/phase8/bench-results.md:127`. The KL header requires bench records to be "cited by file and line". `plan/phase8/bench-results.md:127` is the shake-out row ("推定最大75.0 ms（1 MS/s，INFERRED，高めの推定）", the row still `unmeasured`). It supports neither the new range nor its VERIFIED status. The S4 summary in that file is at `:466` ("再開は resume から 69〜87 ms").
- I checked the quiet-host figures against the records. `bench/2026-11-16-stage4/results/trials.jsonl` has 200 S4 rows, 2 boards × 5 rates × 20, all with `restart_after_resume_ns`. They range from 68.654 ms (board 37, 25 MS/s) to 87.197 ms (board 36, 1 MS/s), so "68.7–87.2" is correct.
- The 139.1 ms figure comes from `plan/phase8/reviews/stage4-proposal.md:96,101`: R2 maximum 139.111 ms, loaded, 1/10/25/50 MS/s, 5 trials a rate.

Both figures are therefore supported, but neither source is cited. KL-15's row (`:229`) also drops "#1" from the H4 condition, which KL-8 keeps. The loaded maximum was measured on one board only.
*Suggested fix:* in KL-8's Evidence, replace `plan/phase8/bench-results.md:127` with `:466` and add `bench/2026-11-16-stage4/` (README `:20`, `results/trials.jsonl`) and `plan/phase8/reviews/stage4-proposal.md:96`. Add "one board" to KL-15's H4 clause.

**m4. Vision §23 and the Vision's §13 list now contradict the edited SC-18 and spec 07 §2.**
`design/vision/04-execution-model-and-data-contracts.md:286` still says "restarts a continuous stream about 50 ms later … the restart gap as its time jump, identically". `design/vision/03-simulation-mock-and-time.md:65` still says "(UHD restarts a continuous stream about 50 ms later)". 4b-2a rewrote SC-18 to "a jump RM-17 bounds" and changed the value to 140 ms, so these sentences are false now. Plan §7 makes the Vision §23 amendment mandatory (`test-plan-v3.md` §7, "[v2]: an amendment, not optional") but assigns it to no sub-step, and 4b-2a's text list omits it.
*Suggested fix:* the orchestrator puts both Vision lines, with the index's revision-history row, into the 4b-2b brief, or the last sub-step's, so the stage does not land with them stale.

#### Notes

- **n1. The oracle change (a), `next = kg` (seed 707), is correct and hides nothing.**
  - The Mock sets `rx.next = kg` unclamped and takes `kf = max(k(f), rx.next)` (`mock-radio/src/lib.rs:736,747–749`).
  - The layer-1 oracle applies a fault to a segment only when that segment's cut is not yet booked at the fault's (e, seq) (`active_rx`, `timeline.rs:268–280`). So the cut the old oracle clamped to was one the Mock could not have known when the fault fired.
  - MR-27 ("written when it fires and never revised", `09-mock-radio.md:179`) and spec 09's §6 row ("fault rows `140 000` and `9 000`", `:313`, that is `kg2 − kg1`) both pin the Mock's unclamped reading.
  - Minimal sequence, at a 140 ms gap: overrun f1 = 1 ms gives kg1 = 141 000. Overrun f2 = 60 ms. A Stop is booked after 60 ms that cuts at 80 ms.
  - The Mock records f2's row as 200 000 − 141 000 = 59 000.
  - The old clamped oracle set next = min(141 000, 80 000) = 80 000, so kf2 = 80 000 and the row came out 120 000. It counted [80 000, 141 000) twice.
  - At a 50 ms gap the window for this shape is narrower, which is why it never showed.
- **n2. The oracle change (b), the skip rule (seed 833), models the Scripted device correctly in the shape it was written for.**
  - **The seed-833 shape, minimal.** A has an overrun f1 before its cut c, and a second scripted overrun f2 lies in [B.origin, f1 + gap), with B.origin > c.
    - After f1's `Overflow`, A's cursor is at `ceil(f1 + gap)`. uhd-rx's `expected` is still below the cut, so its next turn calls `rx_recv` on A (`rx.rs:231–247`, `:482–485`).
    - The Scripted device then finds f2 with `ceil(f2) ≤ k` and fires it on A at that call (`differential.rs:906–921`). It does so before A's stop is issued, so A's `end` is still `None`.
    - The old oracle used "the segment begun last at f2" (`rposition`), which is B, and expected a jump on B. B is observed clean, so the test failed although uhd-rx was right.
    - The new rule puts f2 on A, which matches the device.
  - This is also the FakeDevice's own rule (UR-33). A scripted fault fires at the first call whose sample cursor is at or past its instant: `if next < at … continue` (`device.rs:1733`). The Scripted device mirrors it, so the rig models the device double, not UHD.
  - The Mock would not put f2 on A. MR-20/MR-21 apply a fault to the stream running at f2, and A has been cut by then.
  - uhd-rx charges A with an `Overflow` it received during A's call, which is right. The old rule's prediction of a jump on B was the oracle's error, not a defect in uhd-rx.
  - The premise is an artifact of the rig: the Scripted device "pre-fires" a fault by sample cursor before its device time has come. Under RM-17's "the segment running at f", and in the Mock, f2 after A's cut would land on B or on nothing.
  - So the rig now deliberately differs from layer 1 for this shape. That is acceptable, because layer 3 models its own device, but the comment at `differential.rs:261–263` should call it the scripted device's behaviour, not UHD's.
  - The new rule also catches a defect the old one could have matched: a stale overflow of A charged to B (S16's class).
  - The edge cases are in m1.
- **n3. The FakeDevice's two overflow paths (S23).**
  - **Scripted path.** It now uses `restart_gap_ticks()`, and `ur_33` asserts the exact `cursor + ceil(G/200)·200` (`device.rs:2750`).
  - **Late-reader path.** Before this change it restarted at the first grid sample strictly after `now`, with no gap. It now restarts at the first grid sample at or after `now + G` (`device.rs:1779–1781`), asserted exactly on the manual clock (`:2745–2748`).
  - That change in the late-reader path is the behaviour change plan S23 asks for ("behind: the restart is the first sample at or after now + G"), and the UR-33 text says it (`18-uhd-radio.md:410`).
  - **Late-reader dependants.** The full suite passes (the report says 3 039). The tests that stall a reader are `ur_34_the_receive_stall...`, which takes only the first `RX_OVERFLOW`, and the fake S4 run, which checks only that the bound line exists (`test-plan-v3.md` §6).
  - **4b-3 will depend on it.** S9's rehearsal and the fake S4 measurement will measure about 140 ms plus the drain on the fake, so the fake's D5 line can read "above" by up to one sample.
  - **The profile coupling.** `restart_gap_ticks` reads `x3xx::timing()` directly, not the device's description. Tests that use `with_measurement_timing` with a candidate gap of `17_000_004` (`fake.rs:557`, `provider/mod.rs:981`) keep the fake at 140 ms. That is consistent with the fake playing the hardware, and none of those tests scripts an overflow, but the coupling is worth knowing.
- **n4. `ur_33`'s scripted half cannot see the rounding direction at 1 MS/s.** `G_ticks` = 28 000 000 is a multiple of the ratio 200, so `skip` rounded down gives the same value. A mutant that rounds down would survive. It is harmless at this rate, and the `rx.rs` and Mock tests cover other ratios only indirectly.
- **n5. N108-12's mutant leaves `let gap_ticks` unused.**
  - It compiled with only a warning and was killed for the right reason: `restart >= now + gap_ticks` fails.
  - If a mutation run ever compiles with `-D warnings`, the row would turn into a compile error.
  - Adding `old2` to drop the `let gap_ticks` line would make the mutant exactly the old program.
- **n6. The test-side gap helpers are repeated.** There are six small copies: `gap_ns`/`gap_samples` in `v58.rs`, `s4a.rs`, `mock_layout.rs` and `mock_radio.rs`, `overflow_gap` in `parity.rs`, and `restart_gap` in `timeline.rs`. Each is two or three lines. Most sit in separate test binaries or crates, so sharing them would need a support module.
  - The s4bench tests deliberately compute `bound_ms` from `timing()` rather than through `s4_bound_ns()`, which is what lets N108-16 kill. Keep that.
  - The `device.rs` unit test hard-codes the 200 MHz master clock (`:2713`) where `device.restart_gap_ticks()` exists. Computing it independently is the better oracle, so keep that too.
  - This is not a maintainability finding.
- **n7. Unrequested and coverage items.**
  - **KL header.** The line saying "stages 4a (2026-11-17) and 4b (2026-11-18)" (`KNOWN_LIMITATIONS.md:13`) was edited unasked. It is correct if 4b lands today; otherwise the orchestrator fixes the date at landing.
  - **S20 coverage margin.** S20's `cut_in_gap` count fell from 33 to 5 against a threshold of 3 (`differential.rs:150`). It is correct (m3 of 4b-1), but the margin is thin, and 4b-2b and 4b-2c can move it.

---

### Checklist

#### 1. The value
- **The pin.** `x3xx.rs:414` is 140 ms. The one literal is the pin `channels.rs:655`, killed by N108-10.
- **Leftover literals.** `git grep` at `f6f706b9` over `crates` and `python` for `50_000_000|50_000|51_000|52_000|53_000|59_000|101_000|140_000|50 ms|0.05 s`, filtered to gap, overflow, restart and lost terms, finds no gap literal left.
  - What remains is not the gap: `measurement.rs:4156/4158` (synthetic and S21's material, 4b-3), `radio_model.rs:302,436` (free-standing, plan §5 "keep"), `sink_capture.rs:1027` (free-standing), `protocol.rs:1525` (a start lead), and Python `0.05` (a lead).
  - The hardware comment `hardware.rs:132` is fixed.
  - S4's bound reads the profile through `s4_bound_ns()` (`s4bench.rs:514`).
- **The §6 audit sites for 4b-2a.** All are fixed as planned:
  - `ur_17_a_missed_start_and_overrun...` (`rx.rs:1444`)
  - `ur_33` (S23)
  - layer 1 (`timeline.rs:201,364`)
  - `mr_03` (`mock_radio.rs:550`)
  - `ur_09` ×3 (`fake.rs:187,533,545`)
  - `mr_20`, `mr_20a`, `mr_21_overrun_shape` (all five parts), `mr_37`, `mr_21_a_fault_cuts...` (`mock_layout.rs:768–806`), `mr_12`, `mr_19_backpressure` (including the fractional case), `mr_19_a_fault_overflow...`
  - `s4a` ×2
  - `v58_04`, `v58_06` ×2, `v51`
  - `vh_8` ×4
  - `s4_decisions` and `d5s_bound`

  Left as planned: `fake.rs:779/815` (S21, 4b-3), `mr_19_*`'s behaviour (4b-2d), `vh_8_a_report_without...` (S14, 4b-2b), and `bench1115_skip_probe` (cannot tell; the comment is fixed, and the suite passing settles it).
- **Untouched overflow sites.** `git grep -n -E '"rx_overflow"|FakeFault::Overflow|with_rx_stall|rx_overflow"' f6f706b9 -- crates python` lists every site that injects or reads an overflow. Removing the diff's sites and the §6 table's leaves only gap-agnostic ones:
  - unapplied faults in `mr_20_*` (`mock_radio.rs:2302–2366, 2614–2621, 2747`);
  - `mr_14` (`:931`), which asserts only the first block;
  - `ur_33_scripted_faults_fire` (`device.rs:2467`), which waits for the `Overflow` result;
  - observation fields in `iii.rs` and `measurement.rs`;
  - `rehearse_overflow` (`common/mod.rs:924`, hardware, audited).

  None has a horizon that must hold 1 ms + 140 ms.
- **Horizons.** Every horizon sees the restart, and each test asserts the restarted block before reading its fields:
  - `.expect("the restarted block")` / `.expect("a restarted block")` in `mr_21` (×2), `s4a` (×2) and `mock_layout` (×2);
  - `blocks.len() == 3` in `mr_12`; `events.len()` checks in `mr_19_a_fault...` and the blocked case;
  - the counter count 1 in `v58_04`; `gaps.len() == 1` in `v58_06_overlap`; `link.receive().expect` in `rx.rs:1449`.
  - `v58_04`'s policy Run reaches `Stopped{Policy RX_OVERFLOW}` only through the restart block's event before 4b-2b, so it too proves the restart. The report says it "would pass if the restart were never reached", which is too pessimistic. After 4b-2b the tail may emit at the Run's end, so 4b-2b should re-check this case.

#### 2. The fake's paths
See n3. Both restarts are exact; the late-reader gap was asked for; the tests pass, and the nearest dependants are named in n3.

#### 3. The two oracle changes
(a) is correct (n1). (b) is a correct model of the Scripted device in the shape it was written for (n2). Its remaining approximations (m1) can only make the test fail, never let a defect pass. That includes the one the implementer named: the plan's cut instead of the floored cut.

#### 4. The 4b-1 minors
- **m1.** UR-17 (`18-uhd-radio.md`) and the `rx.rs:447–450` doc comment now say "the no-cut wait … read within one block plus the allowance; each clamped 1 ms..100 ms". Done.
- **m2.** N108-06 (`ticks as u64 * 5` → `* 10`) is unique (`differential.rs:619`; `:522`'s `observed.end as u64 * 5` is a different token sequence). It is killed deterministically by `assert_eq!(host.given, delta)` (`:512`), not only by the bracket. Done.
- **m3.** `take(segments.len().saturating_sub(1))` (`:473`) is correct: `segments` is sorted by origin, and cancelled segments are filtered out (`:326–327`). Done.
- **m4.**
  - The Stop is booked `gap_ms / 2` after the build (`fake.rs:6309`).
  - `rx_overflows == 1` is asserted (`:6318`).
  - `start <= cut < start + gap` is asserted (`:6341`). `start` is the first `rx_start` and `cut` is the applied stop's `at`; both are device ticks.
  - The test passes, so the domains agree. If they differed by the 5× root/MCR ratio, `cut < start + gap` would fail.
  - A Stop booked after the gap fails the window assertion. The report says it checked `gap_ms + 20`.
  - The assertion that the first segment has no clock proves that no sample was delivered before the overflow. Done.
- **m5.** `fault_rows` has length 1 with `(true, gap_samples)`, which proves the fault fired. The test also asserts `cut == 2_010_000_000` and `cut < 2_001_000_000 + gap` (`mock_radio.rs:2799–2806`). The second assertion follows from the first while the gap is 140 ms, but it guards the window if the value shrinks. Done.
- **m6.** The spec 09 row (`:314`) has three cells. Done.
- **Notes.** KL-7 now says "until past" (`:66`), and the dead `timeout < Wall::from_millis(1)` operand is gone (`rx.rs:233`). `recv_timeout` clamps to at least 1 ms (`B12`'s clamp), so the operand was never true. Done.

#### 5. Texts
- SC-18, spec 02 §2, spec 07 §2 and its pin row, spec 09's §6 rows and MR-37's Test line, spec 18 §2, UR-17, UR-33 (`:410`, `:414`), and the two §6 rows (`:558`, `:646`) all match the code.
- KL-8's figures and KL-15's row are supported by the records (m3), but their citations are missing.
- "VERIFIED against UHD's source" is **INFERRED** in this review. It rests on the design draft's §6 source lines (`design-draft.md:319–336`). This host has no UHD source tree (a `find /` for UHD `radio_control*` files returned nothing), so I could not re-read them. The texts do not cite those lines either, and KL-8's wording is off (m2).
- The KL header edit is in n7. Vision §23 is in m4.

#### 6. Mutation rows
- **N108-10.** The pin only. Everything else follows the profile, so this is the right reason.
- **N108-11.** It is killed by `ur_33`'s exact scripted assertion. The mock-side tests are not its filter.
- **N108-15.** The same mutant, filtered to `vh_8`. The fake loses 50 000 samples where the Mock loses 140 000, so the right reason.
- **N108-12.** The mutant restores the old late-reader restart, with no gap. `restart >= now + gap_ticks` (`device.rs:2746`) kills it. It compiles with an unused-variable warning (n5).
- **N108-16.** `s4_bound_ns()` returns 10 ms. The test's `bound_ms`, computed independently, expects "D5's 140 ms" (`s4bench.rs:946`), so the row is killed for the right reason.
- **N108-06.** See m2 in §4 above.
- **S4B-04.** Before: `max <= S4_D5_VALUE_NS` with the constant 140 000 000. After: `max <= bound` with `bound = s4_bound_ns()` = 140 000 000 (`s4bench.rs:669–670`). The old and new programs, and their mutants, are the same. The anchor matches once (`grep -c` = 1).
- **What "killed for the right reason" rests on.** `rows-4b2a.log` records only each row's outcome line (`EZSDR-ROW {...}` and the name), not the failing assertion. So "killed for the right reason" above is my reasoning from the mutant and the test, not an observed panic message. Since stage M a row counts as `killed` only on a test failure (AGENTS §7), so N108-12's mutant did compile. The unused-variable warning is inferred from the source.
- **Run coverage.** Every row whose filter names a changed test ran: J08, F30, F43, F44, D09, D33, the `vh_8` rows, the `mr_20` rows, `s4a`, `mr_21_a_fault_cuts`, `mr_03`, and the three layer-1 `rm_26` rows. The logs record 454 `killed` and 2 `killed (bound)` (R15, J317, as declared), with no ERROR or survivor.

#### 7. Adversarial checks
| Test | Smallest wrong change | Caught? |
|---|---|---|
| pin `s4a_x3xx_values...` | gap value changed | yes (N108-10) |
| `ur_33` scripted | 50 ms skip; gap added twice | yes (exact equality); the rounding direction is not caught at 1 MS/s (n4) |
| `ur_33` late reader | no gap; gap off the grid; gap added twice | yes (the `>=`, `<` + 200 and modulo-grid assertions). The verdict does not depend on timing. The body runs on a spawned participant, registered and busy (`host.rs:393–401`, `:424–429`), and a manual clock stands still while a participant is busy (`now_locked`, `host.rs:124–125`). So `time_now()` before `rx_recv` and the fake's `now` inside it read the same instant |
| S2b (m4) | Stop booked after the gap; no overflow fired | yes (window assertion; `rx_overflows == 1`) |
| `mr_21_a_restart...` (m5) | overflow unapplied; cut moved | yes (fault row; exact cut) |
| `vh_8_*` | one Provider keeps the old gap | yes (N108-15; Mock against fake) |
| `v58`/`v51` | horizon too short | yes (gap and capture assertions need the restart) |
| layer 1 | Mock clamps `rx.next` to the cut | yes, by the comparison (seed 707's shape); no row, which is acceptable |
| rig skip rule | a stale overflow of A charged to B | yes, now stricter than before (n2) |
| `s4_decisions` | bound read from another key; bound exclusive | yes (N108-16, S4B-04) |

#### 8. Maintainability
- **Duplication.** n6: small, acceptable.
- **State.** The fake's gap has one source, `restart_gap_ticks`, used by both paths. No competing flags were added.
- **Tests.** The pure decisions (`s4_decisions`) are tested directly, and the end-to-end tests keep their wiring roles.
- **Leftovers.** `S4_D5_VALUE_NS` is removed with no references left. The dead `timeout < 1 ms` operand is removed. The rig's old comment "One past that segment's cut leaves its blocks as they are" is replaced.
- **Mutation rows.** Each anchor is minimal and unique (`rules.py` 0 problems, per the report).
- The rig's skip-rule closure (`differential.rs:270–273`) is dense. With m1's flag it would read more simply.
- None of these is a finding.

## review-4b2b

## Sub-step review — 4b-2b (commit 0fd5c539 against f6f706b9), Opus, read-only

**Verdict: PASS.** 0 major, 6 minor, 11 notes.

I found no wrong production logic. The tail, the Mock's abort floor, P5 and P6 behave as the plan, Q2, M2/m0/m2/m3 and F3 decide. They agree between the Mock and uhd-rx in every sequence I traced (§1).

The minors fall in five groups:
- one Manifest-row and UR-17 inconsistency in P5's window (deviation 1);
- one wall-clock bound left in an acceptance test;
- one unreached rig clause;
- one interim Mock double report (a fault inside a back-pressure gap, then a `Stop`), which 4b-2d must pin;
- two text gaps.

What I ran (read-only):
- `cargo fmt --all --check`: clean.
- `rules.py --base a4ad013a --renames …/renames.txt`: 0 problems.
- `grep EZSDR-ROW rows-4b2b.log`: 94 outcome lines (31 + 63). The only one not `killed` is the maintenance G03, `equivalent (survived)`.
- The UHD 4.10 source, read in the image `ezsdr-v4-dev:uhd4.10` (§6).

I did not build or run any tests. Verdicts on tests and rows come from reading the code, the rows and `rows-4b2b.log`, whose outcome lines carry no panic text. So "killed for the right reason" below is reasoned (INFERRED).

---

### 1. The tail on both Providers (checklist 1)

**Production.**
- uhd-rx:
  - `finish_locked` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:323–345`) takes the stream.
  - It computes `loss_causes(&stream.pending)` (`:737–747`, now shared with the block path at `:658`).
  - While a loss report is pending it ends at `planned_end` (`:348–352`: the plan's segment cut, floored at `expected`, the same floor as `refresh_intent`'s `cut.max(stream.expected)` at `:723`). Otherwise it ends at the chain's cut.
  - `emit_tail` (`:355–363`) then emits through the shared payload helper and the shared `emit_overflow` (`:750–754`).
- Mock:
  - `rx_tail` (`crates/ezsdr-mock-radio/src/lib.rs:1307–1318`) runs in `rx_settle`'s cut arm (`:1287`), before the clock ends.
  - `undelivered` is set in one place, the accepted-block branch (`:602`), and starts at 0 (`:1372`).

| Case | uhd-rx | Mock | Same? |
|---|---|---|---|
| restart packet past the cut, trimmed (S5) | `expected` not moved by the trimmed packet; tail `c − kf` (`ur_18_a_restart_packet_past_the_cut…`) | n/a (no packets); a cut in the gap settles at once because `cut <= next = kg` (`lib.rs:559–561`) | yes |
| packet across the cut (S5b/S5m) | block carries the event, `pending` consumed, no tail | restart block carries it (`publish_rx` takes `reports`, `:687`), `rx_tail` finds none | yes |
| cut exactly at `kf` | `follow`'s `done` → `lost` 0 → filtered (P10) | `mr_20_a_fault_at_a_loss_s_instant_removes_nothing` | yes (uhd: see n10) |
| sequence + overrun before one cut, either order (S8b) | shared `rx_overflow_payload` → `overrun`; `lost = c − expected` | same helper; `lost = c − undelivered` (`kf` of the first report) | yes |
| second overrun inside a gap (S8c) | `expected` unmoved, so `c − kf` | `undelivered` unmoved, so `c − kf₁`, not the rows' sum | yes |
| abort with no Stop, report pending (S6u/S6m) | the End carries `first_undelivered` (`core.rs:609–615`) → plan cut `kf` → 0 | End `delivered = undelivered = kf` → 0 | yes |
| Stop cut, then abort, report pending (P5, S15) | `planned_end` → the Stop's cut; delivery still follows the lowered chain cut | the timeline ignores the End after the Stop's cut (`timeline.rs:474–476`: `running.take()` is `None`) → the Stop's cut | yes |
| device loss in the gap (S7) | the loss's End → plan cut `c` → tail `c − kf` | `stop_at` with `Loss` → End at `f`, not floored → tail `c − kf` | yes |
| loss then Policy abort (S7c) | P5 keeps the loss's cut | the End after the loss's End is ignored | yes |
| no block published (S14) | `handle` not taken → primary root at `origin + expected·n` | `domain` `None` → `v_of(origin, ratio, undelivered)` | yes (form compared in `vh_8_an_overrun_cut_before_its_restart…`) |
| the Run's end vs a Stop | both are plan cuts; same rule | same | yes |

**Edge cases I checked and cleared:**
- A Mock tail emitted before the last block up to the cut is published, which would over-report. It cannot happen. `v_after(c−1) = floor(instant(c−1)) + 1 ≤ e` (`mock-radio/src/time.rs:30–32`), so the block that ends at the cut is publishable by the step that handles the Stop, before `rx_settle(u)` (`lib.rs:2085–2092`).
- A pruned segment (deviation 6a). Both Providers cut at 0 (`refresh_intent`'s `map_or(Some(0))`; `rx_settle`'s `None` arm), so the loss is 0 on both.
- Lock order in `planned_end`: chain, then streams, as in `try_end` and `follow`.

**Emission order.**
- The tail comes after the segment's last block on both Providers. In uhd-rx it is emitted in `finish_locked` after any block; in the Mock, in `rx_settle` after `emit_rx_until`.
- Its order against `DEVICE_LOST` is unspecified (P12). S7 asserts that both exist.
- No capture mark is pinned by:
  - `uhd_59_a_cut_overflow_reaches_the_manifest_and_marks_no_capture_alike`;
  - `v58_05_a_loss_after_an_overrun_stops_the_run_and_keeps_the_tail`;
  - `uhd_59_a_loss_after_an_overrun_stops_the_run_and_keeps_the_tail`;
  - a control (`assert_one_mark`) shows that a mark does appear when a block carries the event.

**Divergence:** none found in the decisions. The two Providers' instants differ only through device lag (uhd's quiet-span end), never through a different rule.

### 2. P6's origin gate (checklist 2)

- **The gate** is `stream.successor && now < stream.clock.origin` (`rx.rs:507`). It covers `Overflow`, `BadPacket` and `Alignment` (`:509`). It counts `rx_overflows` for all but `Alignment` (`:510–512`) and writes one `timing` row (`:513`).
- **The flag** is set only when `finish_locked` took a stream (`:327–329`) and is copied in `begin` (`:426`). A stop that fails without losing the device still takes the stream, so `successor` is set:
  - S16f (`ur_18_a_failed_stop_does_not_stop_the_gate`) proves this with a `LockSpy` refusal and the chain `Uncertain`;
  - N108-68 (the `not_before` proxy) is killed through the helper's `assert!(stream.successor)` (`rx.rs` `successor()`), which is the property itself.
- **Each class:** S16g covers all three classes, before and at the origin. S16e covers the boundary (origin − 1 and origin). S16c(2) covers the first stream, S16h the empty finish.
- **The residual** is stated in UR-18 (`design/18-uhd-radio.md`, UR-18's fourth line, "INFERRED from UHD's source; not pinned by a test") and in KL-8 (`KNOWN_LIMITATIONS.md:74`). It is not hidden. See m4 for KL-8's incomplete consequence.
- **The second residual** (a late overrun after A's single stop, gated on B, with UHD's queued restart possibly surviving) is not stated yet. The report puts it in 4b-2c (n9).

### 3. The Mock's abort floor and `undelivered` (checklist 3)

`stop_at` reads `rx.undelivered` (`lib.rs:1223`). `apply_rx_fault` and the `Full` branch never move it (`:750–767`, `:589–598`). This is correct per P3 and Q2.

**Deviation 7.** For now, an abort inside a back-pressure gap ends the Mock's clock at the refused block's first sample, after MR-19's direct event already reported the gap past that end. This is harmless:
- it is Mock-only, on `Block` links (UR-12);
- it lands on `main` only squashed with 4b-2d;
- 4b-2d's S11d (`test-plan-v3.md`, 4b-2d: `mr_19_an_abort_after_a_refusal_loses_nothing_before_its_cut`) pins the converted behaviour, with N108-52 on `undelivered` at a refused publish.

The 4b-2d brief should name deviation 7 so that S11d is read as its closing test.

An orderly `Stop` inside a back-pressure gap, with a fault reported in that gap, double-reports in the same interim (m6). The same 4b-2d conversion closes it.

**Hardware and bench sites that read `RX_OVERFLOW`** (the report's audit, re-read):
- **`rehearse_overflow`** (`crates/ezsdr-radio-uhd/tests/common/mod.rs:920–937`): right.
  - It reads only `overflows[0]` and the first `overflow_restart` gap.
  - The Run ends `after + stall + 500 ms` past T0, well after the restart, so the first event is carried by a block.
  - A tail could only be a later element of the list.
- **`extract_overflow_trial`** (`common/measurement.rs:2608–2686`): right. A trial is `Success` only with one complete (not partial) capture, exactly one `RX_OVERFLOW` and one `overflow_restart` gap whose `lost` equals the payload's (`:2668–2676`). A tail-only or second-tail trial is therefore `Incomplete`, never a false Success. How often the bench produces that shape is not known (INFERRED, as the report says).
- **`bench1115_skip_probe_records_a_receive_overflow_and_its_gap`** (`tests/fake.rs:10058`; the plan's "cannot tell" site): device-level. It calls `skip_probe` on the device directly, and the Module's tail is not in that path, so the test is unaffected.
- **Not re-read:** `hardware.rs:156, 1196`, `s4bench.rs:226`, `multichannel.rs:855–941`, the other `fake.rs` sites. For those I rely on the report's audit paragraph ("print/count only", "first event block-carried, Runs end after the restart").

### 4. The implementer's deviations

| # | Verdict | Disposition |
|---|---|---|
| 1 | Real inconsistency, narrow (m1) | carry to 4b-2c, which reworks `try_end`'s stop for P7 |
| 2 | Right: the Mock unit harness has no coordinator; `v58_05_a_loss_after_an_overrun…` counts one `DEVICE_LOST` | adopt |
| 3 | The probe/margin construction is sound: within the margin the fault's call is fixed by the fake's position-based check, and a miss is routed to `EZSDR-TIMING`. One plain wall-clock bound remains (m2). Its root cause is #107's (T0's real-time component, host.rs M5) | adopt; fix m2 now; n2 for the long-term fix |
| 4 | The rig clause is unreached (m3) | carry to 4b-2c: reach it or delete it |
| 5 | Right: RM-17 is the tail's rule; no new id is needed | adopt |
| 6 | (a) Both Providers cut a pruned segment at 0, so the loss is 0. (b) MR-19's direct emit has `next_sample > next`, so `lost > 0` (`lib.rs:589–598`) | adopt |
| 7 | Harmless interim (§3) | carry to 4b-2d (S11d) |
| 8 | Process only | note |
| 9 | INFERRED items are stated where they belong (UR-18, KL-8, KL-15) | adopt |

**Wall-clock check (deviation 3), test by test:**
- `vh_8_an_abort_in_a_gap_cuts_both_providers…` (`parity.rs:1060–1078`). T0 drift ≥ 5 ms is routed to `infra::timing` before the plain assert. Within the margin, the fault falls inside the first 10 000-sample call, and `FakeFault::Overflow` fires on the stream position at the second call's start. Deterministic.
- `uhd_59_a_cut_overflow…`. Drift ≥ 10 ms is routed (`uhd.rs:136–145`). However, `assert_cut_overflow(…, exact: false)` asserts `horizon ≤ end < horizon + 10 000` samples (`uhd.rs:627`). The end is "the horizon plus the coordinator's own work in real time". That is a wall-clock verdict, and it is not routed (m2).
- `uhd_59_a_loss_after_an_overrun…` has only a lower bound (`sample >= 30_000`) and `lost` taken from the recorded end. Deterministic apart from the routed drift.

### 5. Tests and rows (checklist 5)

For each test: the smallest wrong change it must catch → whether it fails on the named assertion, and what shows it reached its shape.

**uhd-rx unit tests**
- S5 `ur_18_a_restart_packet_past_the_cut…`
  - Catches: no tail (N108-20) → fails on `got.len() == 1`.
  - Shape: `rx_overflows == 1`; prefix `[0, 1 000)`; clock end at 6 000.
- S5b `…across_the_cut_carries_the_event_and_no_tail_follows`
  - Catches: a tail from a consumed `pending` → two events, fails on `got.len() == 1`.
  - Shape: the second block `(5 000, 1 000)` with `RESTARTED`.
- S6u `ur_24_an_abort_in_the_gap…` (3 variants)
  - Catches: the zero filter removed (N108-37) → `got.is_empty()` fails.
  - Shape: the prefix only, one `applied` row. The "booked, then a packet" variant reaches the trimmed-packet path.
- S6o `ur_18_an_orderly_stop_in_the_gap…`: kills N108-21 and N108-26 on `assert_tail` (`lost`, domain).
- S7 and S7c `ur_29_*`
  - Catch: no tail; in S7c, also the chain cut used (N108-35 would fail its `clocks` assertion: reasoned).
  - Shape: one `DEVICE_LOST`.
- S8a, S8b, S8c
  - Kill N108-24 (first-kind cause) and a moved `expected`.
  - Shape: `pending.len() == 2`; `expected == 1 000` after two overflows.
- Root-time test: kills N108-25 on the event's domain. Shape: `clocks(&core).is_empty()`.
- S16d silent end: kills N108-22. Shape: `cut > kf`, `rx_overflows == 0`.
- S17 alignment: kills N108-38.
- P5 table (6 cases)
  - Kills: N108-35 (clock and tail), N108-35c (the in-flight case delivers `[3 000, 6 000)`), N108-35d (the off-lattice rows).
  - Shape: the plan's cut is 6 000; the chain cut is `instant(6 000)`; `now` is on the correct side of the cut instant in each row.
- P5 controls: kill N108-35b in (i).
- S16a
  - Kills N108-36 and N108-78 through `pending.is_empty()` and the timing row.
  - Its title claim ("no event") also holds on the old code: n4.
- S16b, S16c, S16e, S16f, S16g, S16h: kill N108-36, 66, 67, 68, 69/77 and 76, each on a flag or count named in the row.
- S2a extension: tail `{overrun, 20 000}` and clock end at 21 000 for both block lengths.

**Mock tests**
- `mr_21_overrun_shape` (cut case): kills N108-30 and N108-27. J08's row assertion is kept (`gap`).
- S5m: one event, carried by the `(kg, 500)` block.
- S6m (two cases): kill N108-33 and N108-34 on `ended_at == 2 001 000 000`. The fault row asserts that the fault fired.
- S8b (two orders): N108-48 and N108-48b, one order each.
- S8c: N108-31.
- S7 and S7c Mock tests: asserted on stamp, payload, source and clock.
- S15e: the Stop settles before the abort, which then finds the segment cut (§1).

**Acceptance tests**
- `vh_8_an_overrun_cut_before_its_restart…` (S14): kills N108-28 on `mock_at == t0`.
- S6p: kills N108-33.
- S2c and the two Policy tests: as in §4.

I found no test whose kill would come from a timeout or a harness wait. The P5 table's `assert!(rx.stream.is_some())` and the shape assertions come before the abort, so a mutant fails on the outcome assertions, not on setup.

**Rows**
- **31 new or changed rows** (29 new, plus N108-12 and U26): all `killed` (`rows-4b2b.log`).
- **63 existing rows re-run**:
  - G03 (maintenance) is equivalent, as before.
  - The re-run list covers the anchors near the edits: P559, J13, U24/25/47, U32, G60, J55, R08, M26, D09/D33/D35–37, F43/F44/F46, T-rows not touched.
- **F32 retirement is justified.** `rx_tail` takes `rx.reports` before its payload check (`lib.rs:1309`), and the cut arm is the only way out of a cut segment. So after `rx_settle` the mutant's loop never has a report to emit (reasoned).
- **D38 retirement is justified.** Its anchor `let lost = lost?;` is gone, and N108-37 restores that exact text as its mutant.
- **U26's re-anchor** moves `emit` to `emit_control` inside the shared `emit_overflow`. It now mutates both the block path and the tail; its filter `ur_19_reports_before_one_block_keep_each_kind` still kills it.
- **N108-12's `old2`** (`let gap_ticks = self.restart_gap_ticks();` dropped) removes the unused binding, as review 4b-2a n5 asked.
- **Uniqueness:** `rules.py` passes. The tail helpers repeat none of U48's, P559's or D09's token sequences (§4.3 of the plan).

### 6. Texts (checklist 6)

**The UHD source citation is VERIFIED** (read in the image `ezsdr-v4-dev:uhd4.10`):
- `radio_control_impl.cpp:48`: `OVERRUN_RESTART_DELAY = 0.05`.
- `:105–125`: the `ACTION_KEY_RX_RESTART_REQ` handler schedules `get_time_now() + OVERRUN_RESTART_DELAY`.
- `rx_streamer_zero_copy.hpp:209–245`: the overflow is returned only after "the user has read all the packets that were buffered prior to the overrun".
- `:306–332`: `_handle_overrun` flushes, then calls the handler.
- `rfnoc_rx_streamer.cpp:179–186`: posts `ACTION_KEY_RX_RESTART_REQ` unless `_last_stream_cmd_stop`.

KL-8's "runs from the overflow result" now reads correctly (review 4b-2a m2).

**Evidence citations** (review 4b-2a m3) checked:
- `plan/phase8/bench-results.md:466`: "再開は resume から 69〜87 ms";
- `plan/phase8/reviews/stage4-proposal.md:96`: 139.111;
- `bench/2026-11-16-stage4/README.md:20`.

KL-15 gained "on board #1 (one board)".

**Other texts:**
- RM-17 (`design/07-radio-model.md:190–195`) carries Fable m8's two clauses ("A second overrun inside the gap adds its loss, and the later restart governs"; "not the reports' sum"). It also has the tail's stamp and RM-11 root, the loss-of-0 clause and the alignment clause. Wording gap: m5.
- UR-18 has:
  - the owner's sentence (capture's exclusive end, no mark, RS-30), the P5 rule, the gate and its residual (INFERRED);
  - the two quantities (UHD's timer vs the profile's bound).
- UR-19 has the alignment clause. KL-8's "Also" line has the owner's line.
- The Vision lines (part 03 §13 `:65`, part 04 §23 `:286`) are updated, with an index revision row.
- KL-7 no longer has the "unreported" sentence.
- **Still false or incomplete:** m4 (KL-8's residual consequence) and m5 (RM-17's `k_f`).
- **Deferred correctly:**
  - RM-17's back-pressure sentence, SC-20a and MR-37's direct emit go to 4b-2d;
  - the Changes-table rows of specs 02 and 07 wait for the last sub-step that touches them (4b-2d touches SC-20a and RM-17 again).

### 7. Maintainability (checklist 7)

- **Rule implemented twice.** The tail's `lost` and stamp rule is written in each Provider (`rx.rs:355–363`, `lib.rs:1307–1318`). The cause decision is shared (`rx_overflow_payload`). This was decided in P10 and is pinned across the Providers by S14, S6p and S2c (n5).
- **Lifecycle facts.** `undelivered` has one writer (`lib.rs:602`), as P3 asks. `successor` lives in two fields (n1).
- **Within uhd-rx**, the block path and the tail now share `loss_causes` and `emit_overflow`, which reduces duplication.
- **Leftovers:** the unreached rig clause and its `reports` plumbing (m3).
- **Test repetition:** the quiet-end sequence (n3); the T0-probe logic (n2).

---

### Findings

#### Minor

**m1. Under P5, the `applied` `rx_stop` row records a different cut from the clock's end and the tail (deviation 1).**
- `try_end` pushes `"at": self.core.at(cut)` with the chain's cut (`rx.rs:297`). The chain's cut is lowered to `kf` by the abort (`:244–246`, `:725–727`).
- `finish_locked` then ends the clock and measures the tail at `planned_end` (`:331–332`).

Minimal sequence:
1. Deliver `[0, 1 000)`.
2. `Overflow{true}`.
3. `Stop` booked at 6 ms (plan cut 6 000).
4. `command(Abort)` at 7 ms.

Manifest: `applied` `{rx_stop, at: instant(1 000)}`; the clock's `ended_at = instant(6 000)`; `RX_OVERFLOW {sequence, 5 000}` at 1 000. Two cuts are recorded for one end. Nothing reads that row in this window today: the rig's `stops` (`differential.rs:1155`) is compared only on seeds that never reach P5 (m3), and the S14 parity test has no abort.

*Recommendation:* carry it to 4b-2c, which reworks `try_end` for the second stop and keeps "one `applied` row, `at` = the cut".
- Compute the end cut once, in one helper (the chain's cut, or `planned_end` while a loss report is pending).
- Use that helper for the row, the clock's end and the tail.

*Rejected:* documenting the split in UR-18. It keeps two values for one fact in a Manifest row.

INFERRED that this is internal (the row's stated meaning, "the cut", is unchanged). The orchestrator may judge it a Manifest-content question for the owner.

**Text part (fix now).** UR-17 (`design/18-uhd-radio.md:236`) still says: "once `Provider::stop(abort)` has begun, uhd-rx ends a segment still being delivered up to an earlier cut there too … and the end takes that sample". Under UR-18's new P5 sentence, with a loss report pending, delivery ends there but the clock does not. UR-17 needs a clause such as "(its clock, while a loss report is pending, at the plan's cut: UR-18)" so that the two rules do not contradict each other.

**m6. In this interim, the Mock double-reports a back-pressure gap when a fault fires inside it and a `Stop` then cuts the segment.**

Mock-only, on a `Block` link (UR-12). The sequence, at 1 MS/s on `x3xx`:
1. A capacity-1 `Block` link is left full.
2. The block at `a` is refused. The `Full` branch (`crates/ezsdr-mock-radio/src/lib.rs:589–598`) emits MR-19's direct `{overrun, next_sample − a}` at `a`, sets `next = next_sample`, and leaves `undelivered = a`.
3. `rx_overflow` fires at `f`, inside `[a, next_sample)`. `apply_rx_fault` takes `kf = max(k(f), next) = next_sample` (`:739`) and pushes a report (`:765`).
4. A `Stop` is handled with cut `c`, where `a < c`. Its cut is `≤ next = kg`, so `rx_settle` runs `rx_tail` (`:1287`, `:1307–1318`), which emits `{overrun, c − a}` at `a`.

Observable: two `RX_OVERFLOW` events at `a`. Samples `[a, min(c, next_sample))` are counted in both. RM-17 as it stands at this sub-step keeps MR-19's direct event "outside this rule" (`design/07-radio-model.md:193`). By its tail clause, this fault report's own `k_f` is `next_sample`, so it should add `max(c − next_sample, 0)` (0 when `c < next_sample`), not `c − a`.

The block path avoids the double count because MR-19's event takes the reports first, and the block's `lost` is the reported sum (`:687–689`). `rx_tail` measures from `undelivered` instead.

This is in the same family as deviation 7: it disappears with 4b-2d's conversion. There the refusal becomes a report at `a`, the direct emit goes, and the tail `c − a` is reported once.

*Disposition:* carry it to 4b-2d. Its brief must pin this exact sequence: a fault inside a refusal's gap, then a `Stop`, gives one event `{overrun, c − a}`. Plan S11c has the reverse order (fault before refusal) and S11d has no fault, so neither reaches it. No existing test reaches it either: `mr_21_overrun_shape`'s blocked case has no `Stop` in the gap. It cannot reach `main` if 4b-2d lands squashed with 4b-2b.

**m2. `assert_cut_overflow`'s UHD branch asserts a wall-clock upper bound, and the bound is not routed through `EZSDR-TIMING`.**
- `uhd.rs:627`: `assert!((horizon..horizon + 10_000).contains(&sample), …)`.
- Its own doc says that end "follows the horizon by the coordinator's own work in real time, which the manual clock counts (host.rs M5)" (`uhd.rs:595–601`).
- On a loaded host, more than 10 ms of coordinator work fails `uhd_59_a_cut_overflow_reaches_the_manifest_and_marks_no_capture_alike` plainly, as a FAIL rather than a timing re-run. That is the #107 pattern.

The property the bound guards ("inside the gap, not at the restart") is already pinned without wall time:
- `capture.size_bytes == kf * 8`;
- `capture.partial`;
- `marks.is_empty()`;
- one `RX_OVERFLOW` with `lost == sample − kf`.

A Run end past the restart would publish the restart block and break all four.

*Fix (now):* keep `sample >= horizon` and replace the upper bound with `sample < kf + gap_ns()/1 000` (the restart sample), or drop it.

**m3. The rig's P5 clause is reached by 0 of 1 000 seeds (deviation 4).**
- The clause is at `differential.rs:384–395`; `before_stop` at `:239–245`.
- The `reports` plumbing is at `:186`, `:675`, `:947` and `:1162`.

The `seen` entry is printed, not asserted (`:139`). So the oracle branch, and the `Observed.reports` and `World.reports` fields it alone reads, is code that no run exercises: a wrong oracle there could never be noticed. It cannot mask a defect, because an unreached branch predicts nothing. P5 is pinned only by the unit table, as the report says.

*Recommendation:* carry it to 4b-2c, which rewrites `compare` for the survivor model. Either make a seed family reach it (an abort within one quiet span after a `Stop`, with a report pending; then assert the reach is positive, as P11 does for `cut_in_gap`), or delete the clause and its `reports` plumbing (AGENTS §9, maintainability check 4).

**m4. KL-8's residual names only one of its two consequences.**
- `KNOWN_LIMITATIONS.md:74`: "charged to the next stream: a jump block flagged `RESTARTED` with an event".
- Plan P6 and §7: "B's first kept block with a jump gets `RESTARTED` and an event, **or B ending before any block gets a tail**".

The tail case is 4b-2b behaviour: a report of A that arrives at or after B's origin is pending on B, so B cut before a block emits a tail. The tail case should be stated now. The "and a second stop" part can wait for 4b-2c, together with P6's second residual (n9).

UR-18 also omits P12's sentence ("the order against `DEVICE_LOST` is not specified"; plan §7, UR-18). One clause would close both gaps.

**m5. RM-17's tail clause says "`lost` the cut's sample less `k_f`", but `k_f` is defined for one overrun at `f` (`design/07-radio-model.md:190, 192`).**

With two reports the tail is measured from the **first** report's `k_f`, the first sample not delivered. S8b on the Mock shows this: the second report's `k_f` is 3 000, and the tail counts from 1 000. The plan's §7 wording was "the cut's sample minus the first sample not delivered".

*Fix:* "`lost` the cut's sample less the first sample not delivered (the first report's `k_f`)".

#### Notes

- **n1.** `Stream::successor` (`rx.rs:68`) copies `Rx::successor` (`:115`) at `begin` (`:426`). Because the owner flag is set only after its stream is taken and is never reset, the two are equal throughout every stream's life, so one field would do. The duplication is plan-directed (P6 v3 "begin() copies it") and harmless today. Watch it under `mech:stream-lifecycle-scattered` if `Rx::successor` ever gains a reset.
- **n2.** The "probe T0 + margin + `infra::timing`" logic exists twice: inline in `parity.rs:1060–1078`, and in `uhd.rs:113–145` (`spec_t0`, `started_within`, `device_ns`). Each copy has its own M5 comment.
  - Its root cause is #107's: a fault in the fake's absolute device time against a T0 with a real-time component.
  - A `FakeFault` placed relative to the stream's start would remove the probe by construction. That belongs to note 51 / #107, not 4b.
  - If a third copy appears, move the helper to `tests/support`.
- **n3.** The quiet-end sequence (`time.advance_to(ms(50)); rx.last_samples = …; rx.host.sleep(10 ms); rx.receive(Timeout)`) appears 8 times in `rx.rs` (e.g. `:2017–2021`, `:2161–2165`, `:2254–2258`), 7 of them new. A helper such as `end_quietly(&mut rx, &time, &core)` would keep the sequence visible.
- **n4.** S16a's name claim ("no event") also holds on the old code: a report on a gapless block is consumed without an event. Its kill comes from `pending.is_empty()` and the timing row, as the plan's S16a says. S16b is the one whose block and event assertions discriminate.
- **n5.** The duplicated tail rule (§7) is the plan's accepted shape (P10). Its parity pins are S14 (root form), S6p (abort floor) and S2c (Manifest).
- **n6.** For deviation 7, see §3: the 4b-2d brief should name S11d as its close.
- **n7.** The rig's `fired` flag (`differential.rs:262–288`) implements review 4b-2a m1's fix for shapes (i) and (ii), and states (iii) as a limitation. The n2 comment wording is in place.
- **n8.** S20's `cut_in_gap` count stays at 5 against the threshold of 3 (report). Re-check it after 4b-2c's rig changes (review 4b-2a n7).
- **n9.** P6's second residual (plan P6 [v3]: a late overrun after A's single stop is gated on B, and UHD's queued restart may survive it; INFERRED, Fable m2) is not in UR-18 or KL-8 yet. The 4b-2c brief must carry it, beside S9's third verdict. Note that `rfnoc_rx_streamer.cpp:181` requests the restart only when `!_last_stream_cmd_stop`. That bears on whether the restart survives (INFERRED; UHD's async action queue ordering is not read here).
- **n10.** uhd-rx has no unit test for a plain `Stop` exactly at `kf` with a report pending. The path (`follow`'s `done`, then `lost` 0, then filtered) is covered indirectly by S6u and the zero-filter rows (N108-37/37b).
- **n11.** Deviation 8 (files written outside the allowed list) is a process note for the trial's record.

### Summary of dispositions I recommend

- **Fix now:** m2, m4, m5, and m1's UR-17 text clause.
- **Carry to 4b-2c:** m1's row and helper, m3, n9.
- **Carry to 4b-2d:** deviation 7 (S11d) and m6 (a new S11 case).
- **No change:** n1, n4, n5, n7, n10, n11.
- **Optional:** n2, n3.

## review-4b2c

## Sub-step review — 4b-2c (the second `rx_stop`; 4b-2b's items)

Reviewer: Claude Opus (fresh, read-only). Commit `d890fef5` against `0fd5c539`, branch `work/4b`. Tree clean before and after (`git status --short` empty). Nothing was built or run except `rules.py` (`--base a4ad013a --renames renames.txt`: 0 problems). The tests and rows are judged by reading them and the row logs (`rows-4b2c.log` holds 63 rows killed and `rows-4b2c-b.log` 3; none survived or ERROR). UHD claims were read in `ezsdr-v4-dev:uhd4.10` at `/root/tmp/uhd-4.10.0.0`.

### Verdict: FAIL — one major at plan level; the implementation otherwise matches the plan

The production change is small and correct for every path the plan names. The second stop lives in one place (`rx.rs:293-302`), and the chain makes one transition. There is one `applied` row and one `rx_stop_overrun_clear` row, the loss is re-checked between the two calls, and `let result = self.core.device.rx_stop(self.k);` still occurs once, so P270 and R13 hold.

`end_cut` now feeds all three callers of `finish_locked`. `Rx::successor` has one owner. All 66 rows were killed.

The one major does not say the code breaks the plan. It says a premise of the plan's accepted residual (P6, second residual) is false: uhd-rx does read the report it gates. The same race M2 fixes can therefore happen on the successor, and the cheap fix sits inside this sub-step's mechanism. If the orchestrator re-decides to keep the residual and only corrects its text (m1), the verdict becomes PASS.

---

### Major

#### M1 — An overrun report the successor gate swallows never arms the second stop, so the M2 race is left open on the successor. The residual's "uhd-rx cannot clear it" is false.

**Evidence**
- `try_end` reads the condition only from `stream.pending` (`rx.rs:274`).
- The gate counts a successor's pre-origin `Overflow` and writes a `rx_report_before_origin` row, but does not attach it (`rx.rs:519-526`). uhd-rx has read the report; it simply forgets it.

**UHD (VERIFIED from the source, timing INFERRED)**
- The restart request is posted inside the `recv` that returns the overflow, whenever `_overrun_handling_mode && !_last_stream_cmd_stop` holds (`rfnoc_rx_streamer.cpp:179-186`).
- By the time the successor B's reader gets a stale overflow, the last stream command is B's START, so `_last_stream_cmd_stop` is false (`rfnoc_rx_streamer.cpp:140`).
- The cascade is RESTART_REQ, then the radio's START (`radio_control_impl.cpp:105-125`), then the streamer, then the radios. It runs asynchronously on the graph thread (`graph.cpp:573-603`).
- Suppose B is stopped before that START is written: an abort, or a Stop booked before B's origin. B's single STOP is processed first and clears B's own queued START (`radio_rx_core.v:171-183`, STOP takes `clear_fifo`). The restart START then lands on an idle radio and runs. This is exactly M2's survivor, with the same window as the race S13 exists for.
- uhd-rx makes one stop, because `pending` is empty.

**Minimal sequence on the FakeDevice (reasoned from `device.rs:1755-1774`, not executed)**
1. Faults: `[Overflow(t), RestartOvertakesStop]`, with `t` between A's cut and B's origin. A is stopped before its cursor reaches `t`, so the fault stays unfired.
2. A's single stop. B begins: `next` = B's origin, which is ≥ `t`.
3. B's first `rx_recv` runs before its origin. The fault fires because `next >= at` (`device.rs:1761`), and the fake sets `restart` (`:1771`).
4. uhd-rx gates the report (`rx.rs:519`): `rx_report_before_origin`, nothing pending.
5. `rx.command(Shutdown(Abort))`, or a Stop of B before its origin.

**Observable wrong outcome:** the log holds `rx_stop 0 now` then `rx_restart_survived 0`, and no second stop. The fake keeps streaming from `origin + skip`, and the next `rx_start` is queued behind the survivor (`device.rs:1348`). This is the "successor never begins on its grid" outcome S13a pins for A.

**The rig can reach this shape in principle, but its seeds do not.** The rig's Scripted device fires faults on `runs.front()`. That can be a stopped run's tail (a fault with `f < end`, `differential.rs:961-967`), read after B's `rx_start` and before B's origin. The fault then fires (`:970-978`), uhd-rx gates it, and the knob would fire at B's stop if B returned no sample first. All 1 000 seeds are clean, so no seed combines a tail fault, the knob on and B stopped before its first sample. That is INFERRED from the clean run; I did not count the shape.

The oracle would also have to change with the fix. `unanswered` assigns that hit to A's segment (`:279-280`, `:405-411`), so a second stop of B would be flagged "no stream to stop".

**Recommendation** (internal, so the orchestrator decides):
- Let a gated `Overflow { out_of_sequence: false }` arm the second stop until B publishes a block, without entering `pending` and without a tail. A second stop is harmless when nothing survived.
- Add S13d as a unit test: a successor's gated report, then an abort before the origin, with two stops and no survivor.
- Let `compare` permit B's second stop after a gated report, read from the device's log or the `rx_report_before_origin` row.
- Reword UR-18 and KL-8's second residual to match.

The alternative is to keep the residual and only correct its premise and its consequence (m1). That is the orchestrator's re-decision. The KL-8 wording is the owner's.

---

### Minor

#### m1 — Deviation 1: the new UR-18/KL-8 sentence is right about the cause and wrong about the consequence

Texts: `design/18-uhd-radio.md:247`, `KNOWN_LIMITATIONS.md:74`.

**Right (VERIFIED):**
- At A's stop no restart is queued for a later overflow. `_last_stream_cmd_stop` suppresses it (`rfnoc_rx_streamer.cpp:181`).
- If the overrun event arrives after A's stop, `_overrun_handling_mode` is cleared too (`:321-327`).
- Only an overflow the reader takes after B's START was issued requests a restart.
- So the plan's "UHD's queued restart for A may survive A's stop" was wrong, and deviation 1 is better than the decided sentence.

**Wrong: "so the restart can run on UHD's grid" / "so the next stream can run on UHD's grid"**
- B's START is written before that restart. Its post is SYNC by default (`node.hpp:453-455`, `graph.cpp:597-603`), and uhd-rx issues B's start and its `rx_recv` from the same Rx thread.
- So the restart's START is queued in the radio's command FIFO behind B's START.
- A continuous command leaves `ST_RUNNING` only on `cmd_stop` or an overrun (`radio_rx_core.v:556-575`). The FIFO head is popped only in `ST_STOP` (`:343`, `:430`), and B's STOP clears the FIFO (`:171-183`).
- The stale restart therefore runs only in two cases:
  - B's START is itself late: the late error goes through `ST_REPORT_ERR`, then `ST_STOP`, then the next command (`:533-539`, `:616-620`). That is a reported missed start.
  - B's stop overtakes the cascade (M1).

**What a user needs:** a when and a symptom, not a mechanism chain. Proposed meaning: "rarely, after an overflow just before a stop and restart, the next stream may run on UHD's restart grid when its own start is late or it is stopped at once; its first block then shows `GAP_BEFORE | SEQ_DISCONTINUITY` (INFERRED)". Settle this together with M1.

#### m2 — The knob's second arming site and its close reset have no test and no row

- The knob's doc says it is armed by "a scripted `Overflow` or a behind overflow" (`device.rs:273-279`).
- N108-80 removes only the scripted site (`device.rs:1771`). The behind-overflow site (`device.rs:1812`) has no row.
- Every knob test uses `Overflow(Duration::ZERO)`, and the device test and S13a/b use the scripted path, so the behind-overflow site is never reached with the knob armed.
- The `rx_close` reset (`device.rs:1483-1484`) is likewise unpinned.
- Fix: one case in `ur_33_a_restart_that_overtakes_the_first_stop_survives_it_and_the_second_clears_it` with a stall-made behind overflow, and its row. Or drop "or a behind overflow" from the doc and the arm, if no test needs it.

---

### Notes

#### n1 — The knob against UHD (checklist 2)

**Ordering: VERIFIED.**
- The restart request is posted ASYNC inside `recv` before the overflow is returned (`rx_streamer_zero_copy.hpp:228-236`, `rfnoc_rx_streamer.cpp:179-186`).
- A user stop is a SYNC post (`rfnoc_rx_streamer.cpp:131-150`; default `SYNC` at `node.hpp:453-455`).
- `sync_actions` returns only on an empty, idle queue (`graph.cpp:606-622`, `:664`), and `_process_action_queue` drains appended actions in the same loop (`:672-680`).
- So a START that survives the first stop is written before the first stop returns, and the second stop's `cmd_stop` and `clear_fifo` end it in `IDLE`, `TIME_CHECK` and `RUNNING` alike (`radio_rx_core.v:483-490`, `:524-528`, `:570`).
- A start issued while a continuous START runs waits in the FIFO and is cleared by the next STOP (`:556-575`, `:171-183`). The fake's `then` queued behind the survivor and cleared by the stop (`device.rs:1348`, `:1690`) matches this.

**The window is a superset (INFERRED, and safe):**
- UHD's window closes when the cascade's START is written (graph-thread latency, typically far below 1 ms).
- The knob's window closes only when a restart sample is returned, about 140 ms later.
- The Module's rule reads `pending` and not the window, so it is not built around the wider window; the window only decides which cases require a second stop.
- The directions in which the knob and `pending` disagree:
  - A restart sample trimmed to nothing, or dropped before the origin: `pending` stays and the knob is spent. The Module makes a second stop that is harmless.
  - A gated report: the knob is armed and `pending` is not. This is M1.

**Two channels (INFERRED, the Module does not depend on it):**
- `issue_stream_cmd` posts one SYNC STOP per channel (`rfnoc_rx_streamer.cpp:145-149`).
- Channel 0's post returns only after the whole cascade, including channel 1's START, has been written. Channel 1's STOP then follows its START and clears it.
- So on UHD probably only the first-posted channel survives one stop.
- The fake's "both channels of one streamer survive one stop" (device test case 5, S13b) is therefore a stronger model than UHD's. It is harmless, because the second stop clears both either way. The UR-33 text could say "the streamer's channels follow it" without claiming UHD does.

#### n2 — Checklist 1, "never otherwise"

`grep -n "\.rx_stop(" crates/ezsdr-radio-uhd/src/provider/` finds these production calls:

| Call | What it is | Overrun involved? |
|---|---|---|
| `rx.rs:294`, `:299` | The pair | Yes |
| `rx.rs:447` | `begin()`'s clock-declare failure, on a stream just started | No |
| `core.rs:1489` | UR-16's stop at open | No |

`mod.rs:1386` and `test_support.rs` are test wrappers. The stop decision exists once.

Each case of checklist 1, with the test that pins it:

| Case | Behaviour | Pinned by |
|---|---|---|
| Loss before the stop | `rx.rs:281-285` returns with no call | S18f |
| Loss between the two calls | `!self.core.is_lost()` (`:297`) | S18i |
| First stop fails | The `Ok(())` guard means no second call | S18b/c |
| Second stop fails | It is the stop's result (`:303-310`) | S18d/e |
| Abort | Same path as a Stop | S13b |
| Two channels | One pair per streamer | S13b |

S18f's state is reached (asserted `pending == [Overrun]`). An overrun reported after the cut, during the quiet wait (the stop is issued a span after the cut), also gets the second stop, which is correct: UHD has requested a restart for it.

#### n3 — Checklist 4 (4b-2b's items)

**m1:** all three callers pass `end_cut`:
- `rx.rs:283` (lost)
- `:315` (after the stop)
- `:335` (`finish`)

`end` is now computed before the stop call. A loss published by the stop (`device_lost`) books an End at `now`, which is ≥ the cut (`core.rs:1080-1082`). It cannot lower an earlier plan cut (INFERRED from the Line's End semantics), and S18d/S18i pin the clock at 6 000. `stop_result`, `lose` and `finish` never touch the cut (`core.rs:211-233`). Row N108-79 pins the row's `at`.

**m2:** what the dropped upper bound no longer catches is only a device-paced end that drifts inside the gap. An end at or past the restart publishes the restart's block, which fails `size_bytes == kf*8`, the one-run continuity and `marks.is_empty()` (`uhd.rs:606-612`). The doc comment says so (`uhd.rs:598-604`).

**m3:** deleted (deviation 2). The rig now models every abort's clock end per RM-16 (`differential.rs:378-382`). A seed that reaches P5 would fail loudly: that is a false positive, not a blind spot. The P5 unit table (`rx.rs:3926`) is the pin.

**m4, m5, n9:**
- UR-18's DEVICE_LOST sentence (`design/18:243`) is fine.
- RM-17's "the first sample not delivered (the first report's `k_f`)" (`design/07:191`) is fine.
- KL-8's tail clause is fine.
- n9: see m1.

**n1:** `Rx::successor` is the sole owner. It is set only in `finish_locked` when a stream was taken (`rx.rs:340-343`) and never reset. That is right across restarts:
- Before 4b-2c, `Stream::successor` copied the same never-reset flag at `begin`, so behaviour is unchanged.
- A reopened streamer cannot produce a genuine pre-origin report, because a timed start runs nothing before its origin (INFERRED).
- Rows N108-36, 66, 67, 68 (68 retargeted to `stream.not_before != i64::MIN`, the same proxy as before) were killed.

**n3:** `end_quietly` (`rx.rs:2046`) is the same four lines (`core.ticks(50_000_000)` == `ms(50)`). It is a refactor only.

#### n4 — Deviation 3: what slips through the rig

- `unanswered` takes the segment's **last** overrun hit, fired or not (`differential.rs:405-411`). A second stop is therefore permitted on a segment with a planned but unfired overrun hit and no overrun pending.
- Wrong rules that the rig no longer catches on such segments:
  - a second stop on a pending sequence error, alignment or missed start;
  - a second stop for any segment with an overrun hit.
- These are pinned at unit level:
  - S18a: "sequence only", "alignment only", "missed start only", "no report", both "carried" cases;
  - N108-41 and N108-41c.
- The "required" half reads the device's own log (`:452-458`), so a missing second stop after a survivor cannot slip.
- One false-positive edge: a segment uhd-rx began but that the final plan dropped is absent from `segments`, so `unanswered` returns false and a legitimate second stop would be flagged. It is loud, not silent.
- The seed-67 cause is that the layer-3 `fired` model knows only an overrun's skip, not a sequence error's (`:268-289`). It still drives the `hits` assignment elsewhere: the same latent false-positive class, with no failure in 1 000 seeds.

#### n5 — Deviation 7's negative search missed readers, but all stay right

`grep -n "\"rx_stop\"\|rx_stop_untimed" crates/ezsdr-radio-uhd/tests/hardware.rs` also finds:
- `hw_56_stop_between_starts`: `hardware.rs:2193` takes the first `rx_stop` after the submit, so the pair's first call.
- `hw_56_stop_lag`: `:2250-2259` takes `nth(stop_baseline)` with the baseline re-counted each trial, so the first new call.
- `:2224` and `:2397`: they record `rx_stop_untimed`, which is still one row per stop.

None asserts a count after an overflow, so no hardware expectation changes. Every one of these is measurement recording.

#### n6 — S18f has no row, and the plan's killer for N108-42 is unreachable from it

The early-loss return precedes the stop, so N108-42's mutant never runs in S18f. N67F-10's mutant (`if false` on the lost branch, `rx.rs:281`) would make S18f call `rx_stop`, but N67F-10's filter names a different test.

Cheap fix: add a row with N67F-10's mutant filtered to `ur_29_a_loss_found_before_the_stop_of_an_overrun_stream_makes_no_stop_call`. Also, S18f's `if rx.stream.is_some() { end_quietly(..) }` (`rx.rs:2268-2270`) hides which path ended the stream: assert the path instead.

#### n7 — The added stop's latency is not in the successor's origin

The successor's origin is cut + one call + allowance + lead (`rx.rs:2868`, n5). The second SYNC stop adds one control round trip per channel before `begin()` on hardware, which eats some of the ~0–2.5 ms margin M1 accepted (INFERRED small). S9 cell A, which counts `START_LEAD_SHORT`, is where it would show. The point is mentioned so that S9's brief knows.

#### n8 — Maintainability

- **A stored value nobody reads:** `RxStreamer::restart: Option<i64>` keeps a tick that nothing reads; only `is_some()` is used (`device.rs:494`, `:1681`). It is a `bool` in disguise; make it one, or document why the tick is kept. The rig's `World::restart: Option<i64>` (an origin) is the same (`differential.rs:915`, `:975`).
- **The survivor flag is consistent:** `survivor` is set only by the knob and cleared by every other stop and by close. I found no sequence where it and `then`/`end` disagree; a further overflow while it survives falls through to an ordinary stop.
- **The restart tick is computed three times:** `gap_ticks`/`restart` appears in `device.rs` (the knob test), `rx.rs:2856-2858` (S13a) and `:2938-2940` (S13b). A one-line helper would keep them together.
- **The decision is not copied:** apart from the rig's oracle and the two fakes' models, which are models and not copies, there is no second copy of the stop decision.
- **An inverted message:** the knob test's case 4 message "the knob needs no sample of the restart returned" (`device.rs:2892`) states the opposite of what case 4 shows. Reword it.

#### n9 — Process (for the orchestrator)

- DeepSeek's scratch builds used per-copy target directories, against AGENTS §7 (one shared `Ez-SDRv4-review`). They were deleted afterwards.
- `trial-4b2c.log` was written into the read-only folder.
- The full run had one failure, `mt3_c4_rehearsal_…`, reported as a load flake. Watch it at the gate.

---

### Checklist 7 — tests and rows (one line each; reasoned from the code and the row logs, not executed by me)

**Tests**

| Test | Smallest wrong change | Assertion it fails on | Proof the shape is reached |
|---|---|---|---|
| S13a `ur_17_a_restart_that_overtakes_the_stop_is_cleared_by_the_second_stop` | No second stop (N108-40) | The call window `["rx_stop 0 now","rx_restart_survived 0","rx_stop 0 now"]` | The survivor marker; `origin < restart` assertion |
| S13b (abort ×1/×2, orderly ×2) | N108-40c | The same window; then `rx_recv` → `Timeout` | The two-channel orderly case checks no successor block (deviation 4; acceptable, since the device-clear check pins it) |
| S18a/g/h | Unconditional (41) and "ever counted" (41c): `seen == 1` fails with 2. Timeout clears `pending` (46): the first case, 1 ≠ 2. Two `applied` rows (44): row count. Call under the line lock (44b): the spy panics | As listed | `pending` asserted per case |
| S18b/c | N108-42 | `seen == 1` fails with 2 | — |
| S18d/e | N108-43 | `applied` 0 fails with 1 | — |
| S18i | N108-45 | `seen == 1` fails with 2 | `core.is_lost()` and `DEVICE_LOST == 1` |
| `a_one_shot_rx_stop_hook_runs_outside_its_queue_lock` (`test_support.rs:1189`) | The hook run under the `rx_stop_hooks` guard | A hang (a self-deadlock on the `Mutex`), not a named assertion; no row. It is test support, so this is acceptable, but a row would end in a timeout (`ERROR`), not a kill | — |
| S13c `rm_26_the_timeline_against_uhd_rx`, the N1 check | — | — | Re-check N1 holds: the rig's knob fires only at a stop issued while no sample at or after `kg` was returned (`world.restart` is set at the report, `differential.rs:975`; cleared by any returned samples, `:1012`, and by every stop, `:915-921`). The oracle permits a second stop (`:432`) and requires one only after a logged survivor (`:452-458`). N108-40b, 41b and 84 were killed |

The hook runs in the same thread inside the call. `device_lost` takes only `device_gate` and `streams` (`core.rs:1067-1089`), not `rx_chain`, so there is no deadlock and no schedule dependence.

**Rows**

| Rows | Judgement |
|---|---|
| N108-80 to 83 | The intended programs; killed |
| N108-84 | Disables the rig's knob, killed by the `>= 4` coverage assertion (`differential.rs:155`) |
| N108-44 | Its mutant uses `cut` and `now`, which are in scope; compiles; the intended program |
| P216 | Re-anchored onto the knob's `streamer` binding: the same "stop on stream 0" program, since the whole stop path uses that binding |
| P1397, N67F-10 | Re-anchored for the `end` parameter: the same programs |
| N67F-28, N108-20, 35, 35b | Re-anchored: the same programs |
| N108-68 | Re-aimed at the gate's operand: the same "`not_before` as successor proxy" mutant as before |

### Deviations 2–9

| Deviation | Disposition |
|---|---|
| 2 | Accept (n3) |
| 3 | Accept, with n4 recorded |
| 4 | Accept: device test case 5 shows both channels on the fake, and the Module is channel-agnostic here |
| 5 | Accept: N108-44b covers the second call under the line lock |
| 6 | Accept |
| 7 | Correct the record (n5); no code change |
| 8 | Process note (n9) |
| 9 | The INFERRED list should add M1's premise and n1's two-channel model |

## recheck-4b2c

## Re-check — 4b-2c fix round 1 (`d890fef5..05647cdc`)

Reviewer: Claude Opus, read-only. Main tree untouched. In a scratch copy (`/tmp/recheck-4b2c-*`, deleted afterwards; shared `Ez-SDRv4-review` target, `CARGO_INCREMENTAL=0`) I ran two things:
- `cargo fmt --all --check`: clean.
- `cargo test -p ezsdr-radio-uhd --lib -- ur_17_a_ ur_29_a_loss_found ur_33_a_restart rm_26_the_timeline_against_uhd_rx`: 18 passed, 0 failed. This includes both new S13d tests, S18f, the knob test and the 1 000-seed rig.

No mutant was applied. Row verdicts come from `rows-4b2c-fix1.log` and the failing assertions recorded in `ds-4b2c-fix1-2.log:3425-3466`. UHD claims were read in `ezsdr-v4-dev:uhd4.10` at `/root/tmp/uhd-4.10.0.0`.

### Verdict: PASS with minors

- **M1 is closed in code.** No sequence I could build leaves a survivor or a wrong Manifest row.
- **The new UR-18/KL-8 consequence sentence is wrong in two ways (m1′):**
  - Its "when" names a trigger the gate condition all but excludes, and misses the one that can happen.
  - Its symptom flags are wrong for the trigger it does name.
- **KL-8's wording is not ready for the owner.** A proposed text is below.
- **One item needs an owner decision: n-c.** A stale restart that sits behind a running successor can surface as a spurious `radio.LATE_COMMAND` after that stream's own overrun (INFERRED from UHD and FPGA source, not executed). It is pre-existing UHD ordering, not something this diff introduced, so it does not change the verdict. KL-8's wording depends on whether it is documented.

---

### Q1 — M1

**Closed (VERIFIED from code):**

| Fact | Where |
|---|---|
| One read site | `rx.rs:277`, `pending.contains(&Overrun) \|\| stale_overrun` |
| Set only at the gate, and only for `Overflow { out_of_sequence: false }` | `rx.rs:528-530` |
| Cleared only where a block is published, in the same `len > 0` branch that registers the clock | `rx.rs:654` |
| A fresh `Stream` per begin, so nothing carries over from a stream that was replaced | `rx.rs:80` |
| Every non-loss end goes through `try_end` | `rx.rs:195, 241, 268, 517, 562, 585, 633, 734` |

`finish()` (`rx.rs:215, 219, 249, 413`) is reached only on a lost device or with no stream, so the flag cannot be skipped by an end that stops the device.

**Fails on d890fef5 for the review's reason.**
- The new test cannot compile on `d890fef5`, because it reads the new field.
- The equivalent check is N108-85, whose mutant is `d890fef5`'s `try_end` line verbatim. Its killer fails at `rx.rs:3018` with the stop list `["rx_stop 0 now","rx_restart_survived 0"]`: no second stop, and a survivor. That is M1's counterexample exactly (`ds-4b2c-fix1-2.log:3436`).
- The test also asserts the shape before the stop: one `rx_report_before_origin` row, `pending` empty, no event. After the stop it checks that the device is clear: a receive at `restart + 1 ms` returns `Timeout` (`rx.rs:3027`).

**The neighbouring sequences (reasoned from code unless a test is named):**

| Sequence | Result | Pinned by |
|---|---|---|
| Gated overrun, then a block, then a stop | One stop, no clear row | `ur_17_a_block_ends_the_successor_s_gated_arming`, N108-88 |
| Two gated overruns | Idempotent bool: still one extra stop | — |
| Gated overrun, then a post-origin report (a sequence error) | `pending=[Sequence]` plus the flag: two stops, with the tail naming the sequence error. Correct, because UHD's restart is still queued | — |
| Gated overrun, then a device loss before the stop | `try_end`'s loss branch, no call | — |
| Gated overrun, then a device loss between the two calls | `!is_lost()` guard (`rx.rs:300`) | The guard is shared with S18i/N108-45; this combination has no test of its own (note) |
| Gated overrun, then `LateCommand` with a cut pending | `rx.rs:513-517` reaches `try_end`, and the flag gives two stops. Correct | Not tested (note) |
| A gated stream replaced before its origin | The flag lives on that `Stream`. `try_end` stops it twice, and the next stream starts with `false`. This is right, because UHD requests the restart inside the `recv` that returns the report (`rfnoc_rx_streamer.cpp:179-186`, read at that time), so the stream that read the report owns the queued restart | The "plan drops the successor" case of S13d |
| A report the earlier stream never read, then read by a later successor C | It arms C, which is correct for the same reason | — |
| A gated sequence error | Arms nothing | Control case, N108-87 |

**Where `stale_overrun` and `pending` can disagree:** only in the harmless direction.
- A restart sample trimmed to nothing, or dropped as before origin or `not_before`, leaves the flag set (`rx.rs:600-607, 630-635` return before `:654`).
- The result is one more SYNC stop on an idle streamer.
- What becomes visible: one more `rx_stop_overrun_clear` timing row with no `RX_OVERFLOW` for that stream. The `rx_report_before_origin` row explains it. There is no event, no tail and no second `applied` row.
- No user-visible inconsistency.

### Q2 — the rig's `compare` change

**What `gated` reads.**
- `gated(origin)` (`differential.rs:418`) holds when a `rx_report_before_origin` row names that stream's origin (`:1194`) and no registered SampleClock has that origin.
- Clocks are registered at the first published block (`rx.rs:644-649`, `differential.rs:1174-1185`). So "no clock" is exactly "no block", which is the production clearing point.
- The rule permits and never requires. The required half still reads the device's survivor log, so a missing second stop after a survivor cannot slip.

**What can now slip through the rig:** a second stop at the end of a block-less successor after a gated report of any class: sequence error, `BadPacket` or `Alignment`. The timing row has no class.

**Pinned at unit level:**
- The sequence-error class is pinned (S13d control, N108-87).
- `BadPacket` and `Alignment` are not. A mutant `matches!(result, Overflow { out_of_sequence: false } | RxRecv::Alignment)` at `rx.rs:528` would survive both the unit tests and the rig.
- `ur_18_the_origin_gate_covers_every_report_class` (`rx.rs:4301`) feeds every class through the gate, but it publishes a block before the stream ends, which clears the flag, and it never counts stops. It does not kill that mutant.
- **n-a (note):** the wrong outcome is only a harmless extra stop. The cheap pin is one more control case (`RxRecv::Alignment` or `BadPacket`) in S13d's loop, or adding the class to the timing row.

**The rig shape is reached by seed 215 alone** (`ds-4b2c-fix1-2.log:3443`). A generator change could drop it. N108-93 would then fail loudly, so it is acknowledged and acceptable (n-b).

### Q3 — m1's texts

**UR-17** (`design/18-uhd-radio.md`, the continuation line after UR-17): correct against `rx.rs:277, 528, 654`. "Or with one the origin gate dropped (UR-18) and no block published since" is exactly the code.

**UR-33's FakeConfig clause:** correct.
- "A reader more than 100 ms behind" is `OVERRUN_BEHIND` (`device.rs:1026, 1801-1810`).
- §6's row says "a time reset spends it". The reset site is `set_time_zero` (`device.rs:1457, 1479-1486`), not `rx_close` as the first review said. The report is right.

**UR-18 and KL-8, the consequence sentence: m1′ (minor).**

The cause half is right, and so is the second-stop half. Two parts are wrong.

**(a) "It runs only when that start is itself late" names the wrong "when".**

*The late case is nearly excluded by the gate's own condition (INFERRED):*
- uhd-rx issues B's start, SYNC, and then its `rx_recv`, from the same thread.
- The report is gated only when `now < origin`, measured after that `recv` returned (`rx.rs:522`).
- So B's START reached the radio before uhd-rx's own `now` reached the origin. A late START would need the host's device-time estimate to lag by more than the START's write delay.

*The case that can happen is B's own overrun (VERIFIED from the FPGA; the outcome INFERRED):*
1. In `ST_RUNNING`, an overrun goes `ST_REPORT_ERR`, then `ST_REPORT_ERR_WAIT`, then `ST_STOP`. `cmd_done` pops B's START there, and the stale restart START becomes the head (`radio_rx_core.v`, the `ST_RUNNING` overflow branch and `assign cmd_done = (state == ST_STOP)`).
2. `ST_IDLE` then `ST_TIME_CHECK` run before the host's overrun-handler STOP arrives (µs to ms later, `rfnoc_rx_streamer.cpp:300-312`).
3. If the overrun is more than `OVERRUN_RESTART_DELAY` (0.05 s, `radio_control_impl.cpp:48, 117`) after the gated report, `time_past` holds. The radio reports `ERR_RX_LATE_CMD` (`radio_control_impl.cpp:1344-1349`).
4. The streamer returns the overflow first and `LATE_COMMAND` on the next timed-out `recv` (`rx_streamer_zero_copy.hpp:226-242`). UHD's own restart starts 50 ms later, so the late report can surface before it. That holds when uhd-rx's wait (one block plus the allowance, clamped to 1–100 ms, `rx.rs` `recv_timeout`) is under 50 ms; otherwise it surfaces at a later timeout. INFERRED.
5. uhd-rx maps the report to `RxRecv::LateCommand` (`uhd.rs:1051`). With no cut pending it emits `radio.LATE_COMMAND` with `requested` set to the stream's original start, writes an `rx_restart` row, and issues its own `rx_start` (`rx.rs:546-566`).
6. That start queues behind UHD's restart, which is continuous, so it never runs. It stays in the FIFO, and the stream's next overrun pops it late again. INFERRED: each later overrun of that stream repeats the spurious `LATE_COMMAND`.
7. The stream's end still clears the FIFO with one stop, so nothing survives.

*Minimal sequence:*
1. A overruns just before its stop.
2. B starts, and B's first `recv` before its origin returns A's overflow. The gate drops it.
3. B publishes blocks, which clears the flag.
4. More than 50 ms later, B overruns.

*Observable:* B's `RX_OVERFLOW` followed by a `radio.LATE_COMMAND` (Warning) for a start that was not missed.

This is pre-existing (the stale START has sat behind B's START since the gate existed), and the round neither causes nor can cheaply cure it. It is a text finding plus the residual note n-c.

**(b) The symptom flags are wrong for the case the text names.**
- When B's START is late, the reader receives `LateCommand`, which is not gated (`rx.rs:524` covers only Overflow, BadPacket and Alignment), so `MissedStart` is pending.
- `samples()` adds `SEQ_DISCONTINUITY` only when `pending` is empty, and `MissedStart` adds no flag (`rx.rs:656-671`). The first block is therefore `GAP_BEFORE` alone, not `GAP_BEFORE | SEQ_DISCONTINUITY`.
- Only the first block carries `GAP_BEFORE`, not "its blocks".
- The `LATE_COMMAND` event's `applied` names uhd-rx's own restart, which is queued behind UHD's and never runs, so the first block's time differs from it.
- Off-lattice times show in `stats.rx_off_lattice` (`rx.rs:609-611`).

**Proposed KL-8 "Also" tail** (the owner's to approve; it replaces from "an overrun of the previous stream…"):

> an overrun of the previous stream that UHD hands to the reader after the next stream's start was issued makes UHD queue a restart of its own behind that start; the Module stops the next stream twice unless a block came first, so the restart never outlives the stream. While the stream runs the queued restart waits behind it, and if the stream itself overflows more than 50 ms after that report, a `radio.LATE_COMMAND` naming the stream's original start can follow its `RX_OVERFLOW`, and again at each later overflow of that stream; the samples are reported as for any overflow (INFERRED from UHD's source; not measured).

UR-18's last sentence should say the same thing in normative form.
- Replace "runs only when that start is itself late … `GAP_BEFORE | SEQ_DISCONTINUITY`" with: the restart waits behind the later stream's own START until that stream stops or overruns; after an overrun more than `OVERRUN_RESTART_DELAY` after the gated report, UHD reports it late and uhd-rx's `LateCommand` path emits `LATE_COMMAND` with `requested` the stream's start (INFERRED).
- The late-start case can stay as a clause, with the correct symptom: the first block is `GAP_BEFORE` with `LATE_COMMAND`, its time is not the event's `applied`, and the stream is off-lattice.
- The "so a stop that overtakes … does not leave it running" clause is rationale. Under spec 22 DA-1 it could move to the Changes table (note).

### Q4 — m2, the notes, and the rows

- **m2: done.**
  - Knob case 6 covers the behind site, which an overflow arms (`device.rs:1801-1810`).
  - Case 7a: `set_time_zero` spends a pending restart.
  - Case 7b: `set_time_zero` clears the survivor, so a new start is not queued behind it.
  - Rows N108-90, 91 and 92 were killed on the named assertions: `device.rs:2930` 0 vs 1, `:2948` a survivor marker, and `:2965` "the new start's samples: Timeout".
- **n6: done.**
  - S18f now asserts `rx.stream.is_none()` directly after `poll()` (`rx.rs:2274-2275`), and `end_quietly` is gone.
  - N108-89 (N67F-10's mutant filtered to S18f) fails on `seen("rx_stop")` 1 vs 0 (`rx.rs:2276` at HEAD; the log's numbering says `:2272`). That is the intended reason.
- **n8: done** (as adopted in `decisions.md`).
  - `RxStreamer::restart` is a `bool` (`device.rs:494`).
  - Case 4's two messages now read correctly (`device.rs:2898-2899`).
  - The rig's `World::restart: Option<i64>` (`differential.rs:695`) is unchanged; only `is_some()` is read (`:920`). The first review named it, but the disposition adopted only the fake's field, so this is a note, not a leftover.
- **Rows N108-85 to N108-93:** all killed (`rows-4b2c-fix1.log`).
  - N108-85 is killed on the stop list, which is the behaviour.
  - N108-86 and N108-87 are killed first on the white-box `stale_overrun` assertion (`rx.rs:3005`). Both would also fail on the stop list, which comes after it, and N108-85 pins the behavioural read. Acceptable (n-d).
  - N108-88 is killed on "a published block ends the arming". Its behaviour, a second stop after the block, would follow at `rx.rs:3053`.
  - N108-93 is killed by seed 215: "no stream to stop".
- **Re-anchors**, each producing the intended program:
  - P216: the same "stop on stream 0" program over the `bool` take; killer `device.rs:2563`.
  - N108-41c: its `old` is now the whole new line, so the mutant also drops `|| stale_overrun`. That is the same "ever counted" program; killer `rx.rs:2159`, 2 vs 1.
  - N108-80: the scripted arm only, unique through `next + skip`.
  - N108-81: spend on returned samples.
  - All four were killed remotely.

### Q5 — anything new that is wrong or left over

- **n-e (note): two helpers for one formula.** The review asked for one helper. The round added `restart_skip_ticks()` in the `rx.rs` tests (`rx.rs:2840`, with 1 MS/s on a 200 MHz root hard-coded) and `restart_from()` in the `device.rs` tests (`device.rs:2803`). Each copies the fake's production formula (`device.rs:1768, 1808`). The two live in different test modules, so this is acceptable; a `pub(crate)` test helper beside the fake would make it one.
- **n-c (note, for the orchestrator): the residual in Q3(a) is a real but rare user-visible effect,** a spurious `LATE_COMMAND`. It is pre-existing, not a regression of this round. A cure would need a new mechanism, such as stop-and-reissue of the successor's start on a gated overrun before its origin. My recommendation is to document it, as above, and file it as an issue if the bench ever shows it, rather than build that mechanism now.
- Nothing else. `stale_overrun` adds no `pending` entry, tail or flag. `rules.py` was reported at 0 problems, and I did not re-run it.

### Findings by severity

| ID | Severity | What |
|---|---|---|
| m1′ | minor | UR-18/KL-8 consequence: wrong "when" (late start, nearly excluded; misses the successor's own overrun after more than 50 ms) and wrong symptom (`GAP_BEFORE` with `LATE_COMMAND`, not `\| SEQ_DISCONTINUITY`). KL-8 is not ready; proposed text above |
| n-g | note | The rig's `World::restart: Option<i64>` is still a bool in disguise (outside the adopted disposition) |
| n-a | note | `Alignment` and `BadPacket` gated reports: a mutant arming on them survives the unit tests and the rig |
| n-b | note | The rig's gated shape rests on seed 215 alone |
| n-c | note | The pre-existing residual: a stale restart behind a running successor surfaces as a spurious `LATE_COMMAND` after its later overrun (INFERRED) |
| n-d | note | N108-86 and N108-87 are killed on a field assertion before the behavioural one |
| n-e | note | The restart formula appears in two test helpers |
| n-f | note | "Gated overrun, then a loss between the calls" and "gated overrun, then `LateCommand` with a cut" have no test of their own; each shares a guard that is pinned elsewhere |

## review-4b2d

## Sub-step review — 4b-2d (MR-19 (b)), commit 9397eb18 against 05647cdc

Reviewer: Claude Opus 5.5 (fresh, read-only). Main tree untouched; probes ran in a scratch copy `/tmp/rv4b2d` (`CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`). `rules.py --base a4ad013a --renames …` re-run: 0 problems. Rows: `rows-4b2d.log` (223/223 killed; `full-4b2d.log` holds only the start of the full run, not cited).

**Verdict: FAIL on one major (M1), which is small to fix.** The MR-19 (b) conversion is correct: one report per refusal, nothing consumed at a refused publish, one event per stretch, every emission through `rx_overflow_payload`. M1 is an RM-25 hole on the S19 × S11d crossing: an abort after a segment's refused first block leaves the registered SampleClock open. It entered the stage with 4b-2b's abort floor, not with 4b-2d. It belongs to 4b-2d's S19 and MR-25 text, and 4b-2d also removed the one event that path still had. Four minors (texts) and notes.

---

### Major

#### M1. An abort after a segment's refused first block leaves its SampleClock open (RM-25); the `segments` row and the clock disagree

**Where it breaks.**
- `publish_rx` registers the clock before the link answers (`crates/ezsdr-mock-radio/src/lib.rs:616`).
- Under abort the cut is `undelivered` (`lib.rs:1220`). For a segment whose first block was refused, `undelivered` is the origin.
- `rx_settle` then finds `line.made(...) == None` (a cut at the origin) and takes the `None =>` arm (`lib.rs:1278-1282`). That arm drops the segment, writes its `segments` row at `t`, and never calls `clocks.end`. Only the `Some(Segment { cut: Some(cut), .. })` arm ends the clock (`lib.rs:1283-1291`).

**Minimal sequence on a real (capacity-1) link.** VERIFIED by a probe in the scratch copy, built from the test file's own harness:
```
Harness x3xx, one Block link capacity 1, arm_start(2.000 s)
step(2.001999001 s)                         // block 0 accepted, link full, never drained
Stop(mock/rx/s0) + start_rx(mock/rx/s0)     // handled at 2.010 s; new origin 2.025 s
step(2.030 s)                               // segment 2's first block [0, 2 000) published -> Full: clock registered
mock.stop(Abort)                            // cut = undelivered = 0 = segment 2's origin
```

**Observed.**
- The receive clock records are `[(2.000 s, ended 2.010 s), (2.025 s, ended_at: None)]`.
- The second `segments` row says `ended_at: 2.030 s`.
- There is no `RX_OVERFLOW`.

The same holds with capacity 0 and an abort right after the first refused block (`ended_at: None`).

**What it breaks.**
- RM-25 (`design/07-radio-model.md:274`): "A receive SampleClock is registered with the first block its segment publishes and ended at its cut, so a segment that publishes none … has none". Here a clock exists (MR-25's new exception, `design/09-mock-radio.md:174`) and is never ended.
- The `rx_settle` doc (`lib.rs:1271-1273`, "a segment cut at its origin has none") is false for this segment.
- The Manifest's clock record and its `segments` row give two different ends for one segment.

**Attribution.** VERIFIED by the same probe with older `lib.rs`:
- At the stage base a4ad013a the clock ended at `2.140 s` (the old floor `k_g`).
- On 05647cdc it is already open, with a direct event `{overrun, 140 000}`.
- On 9397eb18 it is open with no event.

So the open clock entered with 4b-2b's abort floor. 4b-2d's S19 and MR-25 sentence describe this segment shape without the abort case, and 4b-2d removed its event.

**Fix (smallest), decided by whoever adjudicates:**
- In the `None =>` arm of `rx_settle`, end the registered clock at the origin when `domain` is `Some`, and write the `segments` row at the origin. Alternatively, say in RM-25 or MR-25 that such a clock stays open; that is the worse option, because it leaves a Manifest record with no end.
- Pin it with an S19 variant: T6's fixture, or the sequence above, with `stop(Abort)`, asserting `ended_at == origin` and no event.
- Add a row that drops the new `clocks.end` call.

---

### Minor

**m1. MR-27 now says the same thing twice, and the two sentences disagree** (`design/09-mock-radio.md:180`).
- 4b-2d added to the first paragraph: "a tail event after a `Stop` in the gap says the cut less the first sample not delivered".
- The second paragraph (added in 4b-2b) still says "the `radio.RX_OVERFLOW` after a cut inside the gap says the cut's sample less `k_f`".
- They differ in T5 (`mock_radio.rs:2371`). The fault's `k_f` is `a + G`, but the tail is measured from `a` (28 000). The "less `k_f`" sentence would give a negative number.
- Fix: keep one sentence, the "first sample not delivered" form, in one place (AGENTS §2: a topic is written once).

**m2. RM-17's bound with `r = a` contradicts MR-19's `k_g = max(b, a + gap)` when the gap is shorter than a block** (`design/07-radio-model.md:190-191`).
- RM-17 says the stream restarts "at the latest `overflow_restart_gap_ns` after `r`", with `r = a` for a Mock `Block` link, and that the next block begins at the first sample at or after the restart.
- On `ideal` (gap 0) that is `a`. MR-19 and `mr_19_a_zero_gap_still_loses_the_dropped_block` (`mock_radio.rs:2225`) pin `b`.
- Fix: one clause, either "for a Mock `Block` link `r = a`, and at least the refused block is lost (MR-19)", or a pointer to MR-19's `k_g`.

**m3. MR-19's `*Test:*` line keeps a stale "none named"** (`design/09-mock-radio.md:154`).
- It ends "none named for the jump block of a dropped block that carried flags or a `lost`".
- `mr_19_a_fault_report_survives_a_refused_publish` now pins exactly that case: the refused restart block carried the fault's `GAP_BEFORE | RESTARTED` and `lost` G, and the accepted block's flags and `lost` 2G are asserted. The blocked half of `mr_21_overrun_shape` pins it too.
- Fix: name T3 there and drop the clause.

**m4. Spec 09's Changes row for stage 4b is incomplete** (`design/09-mock-radio.md:472`).
- It lists MR-19, MR-21, MR-25, MR-27, MR-37 and §6.
- Over a4ad013a..9397eb18 the `*Test:*` lines of MR-20 (`:159`, the two `mr_20_a_loss…` tail tests) and MR-22 (`:163`, `mr_22_a_cut_inside_a_sequence_error_s_skip_emits_the_tail`) also changed.
- Earlier rows list rule IDs for Test-line-only changes (2026-11-15, and RM-37 on 2026-11-17).
- Specs 02 (SC-18, SC-20a, §2) and 07 (RM-17, §2, §6) are complete for the whole of 4b.

---

### Notes

**n1. The "Catches:" comments in T4 and T4b name each other's row mutant.**
- T4 (`mock_radio.rs:2319`) says it catches `undelivered = stop`, but its row N108-53 mutates to `= next_sample`.
- T4b (`:2349`) says `= next_sample`, but its row N108-52 mutates to `= stop`.
- By reading the code, each test fails on both mutants, so this is a reader's trap, not a gap. Swap the comments, or the rows' filters.

**n2. T4b's reach** (checklist 4): acceptable without a new assertion.
- N108-52's mutant sits only inside the `if full` branch (`lib.rs:596`), and T4b kills it (`rows-4b2d.log`). That is direct evidence that T4b's abort follows a real refusal.
- Its prefix is identical to T1 and T4, which assert the refusal.
- No observable inside T4b could separate "refused" from "not yet attempted": both leave `undelivered = 2 000`.

**n3. Paired rows.** N108-50/98 and N108-51/97 are identical mutants under different filters (deviation 4, each test its own kill). Each pair is defensible. If row count matters, one row per mutant, filtered on the stronger test, is enough.

**n4. A stalled consumer delays a Policy on `radio.RX_OVERFLOW`** (follows from owner Q3 (b); for the decision record, not a defect).
- SB-18 lets a Run map `radio.RX_OVERFLOW` to `stop`; `v58_04` (`crates/ezsdr-acceptance/tests/v58.rs:170`) does.
- On a Mock `Block` link that Policy now fires only when the consumer reads again or the segment ends.
- A consumer that never reads produces no event until the Run's end. Before 4b-2d it produced one at the first refusal.
- MR-19's text already says when the event comes, so no text change is needed. The orchestrator may want one line in the decision record that the owner's (b) includes this.

**n5. Wording.** In RM-17, RM-25 and MR-37, "published" has to mean "publish called, accepted or refused" for T6's tail in the SampleClock to be right. MR-37's last sentence still begins "A fault's overflow …" though a refused block's report now takes the same path. Fold this into M1's text fix if one is made.

**n6. Captures, RS-30 and the Manifest** (checklist 1).
- No capture can sit on a `Block`-linked port: SC-21 (`design/02-stream-contract.md:233`, a Sink's link is drop-class) and MR-7 step 7 (a `Block` link is its port's only link; enforced at `lib.rs:2208`).
- So capture marks cannot change.
- The Manifest's event log changes as intended: fewer events, one per stretch, each timed at `a` or at the first fault's `k_f`. The `faults` rows are unchanged, and link `drops` stay 0 under `Block`.
- Apart from M1, nothing else recorded changes.

**n7. Zero filter and bypasses** (checklist 2): confirmed.
- Both Mock emit sites take `rx_overflow_payload`'s result: `lib.rs:691` (block) and `lib.rs:1309` (tail).
- `RxOverflowPayload {` appears in production only in `ezsdr-radio/src/lib.rs:624, 657`.
- `emit_rx_overflow` is gone everywhere: grep over crates, `*.json` and `*.py` finds no hit.
- uhd-rx's emit sites are unchanged.

**n8. KL-8 and UR-18 against the code** (checklist 3). KL-8's text is for the owner.
- UR-18's rewritten sentences (`design/18-uhd-radio.md:248`) match the code:
  - the gate (`rx.rs:522-531`: Overflow, BadPacket and Alignment gated; only a non-sequence Overflow arms `stale_overrun`);
  - the `LateCommand` path (`rx.rs:546-566`: `LATE_COMMAND` with `requested = stream.start`, `MissedStart` pending, a new `rx_start`);
  - the block that ends the arming.
- KL-8's tail matches the re-check's proposed text word for word.
- KL-8's "so the restart never outlives the stream" relies on a single stop clearing the FIFO once a block came first. That is the re-check's step 7, INFERRED, and covered by the closing INFERRED tag.

**n9. The 4b-2c re-check items are done as specified.**
- Gated `Alignment` and `BadPacket` arm nothing: rows added at `rx.rs:2985-2987`, with N108-94/95 killed on `stale_overrun == armed`.
- `World::restart` is now a `bool`, with an equivalent program (`differential.rs:695, 920, 926, 980, 1017`). N108-84 is re-anchored to the same mutant.
- The test oracle `restart_from` is one helper (`device.rs:2221-2227`). The FakeDevice's own formula (`device.rs:1768, 1809`) stays separate, which is right for an oracle.

---

### Checklist answers

**1. MR-19 (b) on the Mock.** VERIFIED by reading `lib.rs:582-610, 615-697, 1274-1316`, the tests, and the two probes.

| Case | What happens | Pinned by |
|---|---|---|
| One refusal | A report `(a, overrun, k_g − a)` is pushed (`lib.rs:598`); `undelivered` stays at `a`; no event | T1 |
| Accepted block | Takes every report and emits one event with `lost` = the header's `lost` = the whole jump, at min `k_f` = `undelivered` | T1 |
| Refused publish | Consumes nothing (`if !full`, `lib.rs:687`) | T3 |
| Repeated refusals | Accumulate | T2 |
| A fault inside a refusal's gap | `k_f` is pushed to `a + G` (`lib.rs:736`); the tail still counts `[a, c)` once | T5 (m6) |
| Stop before acceptance | Tail `c − a` at `a` | T4 |
| Abort before acceptance | Cut at `a`, no event | T4b |
| Refused first block, Stop | Tail at 0 in the registered clock | T6 |

Further cases:
- A fault pending before a refusal, then an abort, cuts at `k_f` with no event (probe 2, VERIFIED).
- `lost == Σ reports` holds: the only writers of `rx.lost` and `rx.reports` move together (`lib.rs:596-598, 605, 761-762`). F46's retirement is therefore sound.
- I found no double count, lost count or wrong `at` beyond M1, which is a missing clock end, not a count.

**2.** See n7.

**3.** See m1–m4, n5 and n8. The texts are otherwise accurate:
- SC-20a carries m7's clause and names T4b.
- RM-17's exception is gone (`07-radio-model.md:193`).
- MR-19, MR-21 (`r = f`), MR-25 (the abort sentence and the registration exception) and MR-37 (one path) match the code.
- The §6 rows match the tests' values: 561 000 = 4G + 1 000 checked by hand; 28 000; 20 000.

**4. Deviations 1–8.**
1. F43 re-anchor: right; same mutant over `lost`.
2. F44 unchanged: right; it is still unique and still killed.
3. No S6m-with-Block: accepted; T4b covers N108-52.
4. Extra rows: fine (n3).
5. MemLink capacity 0: right; the test harness's `MemLink` refuses at capacity 0, while HostLink would reject that capacity at creation.
6. MR-27 placement: produces m1.
7. RM-17's wrapped `*Test:*` line: fine.
8. Only the rig bool changed: confirmed.

T4b's reach: see n2.

**5. Tests and rows.**

| Test | Smallest wrong change | Fails on the named assertion? | Reach proved by |
|---|---|---|---|
| T1 `mr_19_backpressure_is_an_overrun` | Direct emit at the refusal (N108-51) | Yes, "the refused block's report waits…" | `queued()==1` and the later `lost == G` |
| T2 `…repeated_refusals…` | `if true` (N108-98) | Yes, the first refusal's event leaves at the second | `lost == 2G` |
| T3 `…a_fault_report_survives…` | `if true` (N108-50) | Yes, "a refused publish consumes no report" | the fault row and `lost == 2G`; a silent take would move `at` to 141 000 |
| T4 `…a_stop_before_acceptance…` | `undelivered = next_sample` (N108-53) | Yes, `rx_tails` is empty | tail at 2 000 |
| T4b `…an_abort_after_a_refusal…` | `undelivered = stop` (N108-52) | Yes, the clock end | n2 |
| T5 `…a_fault_inside_a_refusal_s_gap…` | Direct emit (N108-97) | Yes, "no event at the refusal" | fault row 8 000 ≠ G |
| T6 `…a_refused_first_block_registers_the_clock…` | Tail stamped in the root (N108-96) | Yes, on the `TimePoint` | the clock record asserted before the Stop |
| `mr_21_overrun_shape` (blocked half) | Direct emit | Yes, "the refusals report nothing at once" | the fault rows |
| S13d alignment / bad-packet rows | Widen the gate (N108-94/95) | Yes, `stale_overrun == armed` | timing row `rx_report_before_origin` |

- The re-anchors are right: F43 is the same program over `lost`, and N108-84 is the same program over the bool take.
- The retirements are justified:
  - F45 mutated the line that no longer exists.
  - D10 and D34 mutated `emit_rx_overflow`, now deleted. Its hot path is the shared `emit_overflow`, pinned by D09 and D33 through `mr_37_the_overflow_travels_the_hot_path`, and its `lost` by T1's `rx_tails`.
  - F46 is equivalent now that `lost == Σ reports`.
- None of the new tests uses threads, hooks or gates.

**6. Maintainability.**
- No leftover of the old back-pressure path: `emit_rx_overflow` is deleted, the redundant `rx_register` in the `if full` branch is gone, and comments are updated (`lib.rs:137, 145, 597, 686-688`).
- No new state representation was added: the refusal reuses `reports`.
- Findings: n1 (comment/row mismatch), n3 (paired rows) and m1 (duplicate rule text).

---

Not checked: I did not rerun the workspace suite, the 1 000-seed rig, clippy or fmt. The report states them green, and `rules.py` is green on re-run. I did not check uhd-rx's paths beyond the gate and `LateCommand` lines. Probe code lives only in `/tmp/rv4b2d`.

## recheck-4b2d

## Re-check — 4b-2d fix round 1 (2d9ed796 against 9397eb18)

Reviewer: Claude Opus 5.5 (re-check, read-only). The main tree was not touched; only `cargo fmt --all --check` ran there (clean). Probes ran in a scratch copy `/tmp/rc4b2d` (`git archive HEAD`, fresh timestamps, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`). No mutant was applied. "9397eb18's production" means that commit's `lib.rs` with HEAD's tests.

**Verdict: PASS with minors.** M1 is closed and pinned, and every neighbouring sequence behaves correctly. Two text minors remain: m2 is only partly closed (a), and RM-25 contradicts the new MR-25 sentence (b).

---

### 1. M1 and its neighbours

The fix is at `crates/ezsdr-mock-radio/src/lib.rs:1279-1292`.
- In the `None` arm, `registered = rx.domain`.
- When the clock is registered, it is ended at `TimePoint(root, origin)` and the `segments` row is written at `origin`.
- Otherwise the row is written at `t`, as before.

The doc comment (`lib.rs:1271-1274`) matches the code.

**Which paths reach the `None` arm with a registered clock.** Only an abort of a segment not yet settled does. VERIFIED by reading, and by the probes of an abort after a settling `Stop` below (Q4: the new `clocks.end` never runs twice):
- `publish_rx` registers the clock (`lib.rs:616`) only after a block's end instant has passed (`lib.rs:577`).
- An orderly Stop or a loss then cuts at or after that block's end, so its cut is above 0 and it takes the `Some` arm (`lib.rs:1293`).
- An abort cuts at `undelivered` (`lib.rs:1220`). That is 0 while no block has been accepted, so it takes the `None` arm.

Probe results (scratch tests at HEAD, x3xx at 1 Msps, origin 2.000 s):

| Sequence | Clock record | `segments` row `ended_at` | Events | Verdict |
|---|---|---|---|---|
| Capacity 0, one refused first block, abort (the new test's case 1) | ended at 2.000 s | 2.000 s | none | right |
| Second segment, refused first block on a full capacity-1 link, abort (the new test's case 2) | ended at 2.025 s, its origin | 2.025 s | only the earlier Stop's 8 000 tail | right |
| Capacity 0, many refusals (stepped to 2.300 s), abort | ended at 2.000 s | 2.000 s | none | right |
| Capacity 0, refused, an `rx_overflow` fault in the gap, abort | ended at 2.000 s | 2.000 s | none | right |
| Capacity 0, refused, `stop(Orderly)` (the tail) | ended at 2.002 s | 2.002 s | `{overrun, 2 000}` at sample 0 in the segment's clock | right; the `Some` arm, unchanged |
| Capacity 0, refused, `device_lost` at 5 ms | ended at 2.005 s | 2.005 s | `{overrun, 5 000}` at sample 0 in the segment's clock | right; the `Some` arm |
| Nothing published (stepped to 2.001 s), abort | no clock | 2.001 s (= `t`) | none | unchanged from before |
| Capacity 0, refused, `Stop` at 20 ms (T6), then `stop(Abort)` | stays ended at 2.020 s | stays 2.020 s | the Stop's `{overrun, 20 000}` at 0, none at the abort; `stop` returns `Ok` | right; no second `clocks.end` |
| Capacity 1, refused at 2 000, `Stop` at 20 ms, then `stop(Abort)` | stays 2.020 s | stays 2.020 s | the Stop's `{overrun, 18 000}` at 2 000 only; `Ok` | right |
| Capacity 0, refused, `Stop` at 2.5 ms, then `stop(Abort)` | stays 2.0025 s | stays 2.0025 s | the Stop's `{overrun, 2 500}` only; `Ok` | right |

**uhd-rx has no `None`-arm shape.** VERIFIED by reading:
- uhd-rx ties a clock's end to its registration: `finish_locked` ends the clock iff `stream.handle` is `None`, at `end` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:350-354`).
- A clock is registered only with a block of `len > 0`, which moves `stream.expected` past the origin (`rx.rs:644-668`).
- An abort's cut is `instant(expected)` (`rx.rs:265`), so a registered uhd-rx clock is never cut at its origin.
- A link refusal on uhd-rx is only a stat; `expected` still moves (`rx.rs:712-716`).

INFERRED: `end` is `Some` on every path into `finish_locked` with a registered clock. `refresh_intent` (`rx.rs:739-747`) requests a cut whenever the plan has one, and a loss books an end before it sets `lost` (`core.rs:1081-1083`). This is unchanged by the round and is not a finding.

### 2. The new test, and N108-99

- **On 9397eb18's production.** `mr_19_an_abort_at_a_refused_first_block_ends_its_clock_at_the_origin` fails at `mock_radio.rs:2434` with "the clock ends at the segment's origin", `left: None`, `right: …ticks: 2000000000`.
- **Case 2 alone.** With case 1 cut from a scratch copy of the test, case 2 fails at the same assertion with `right: …ticks: 2025000000`. Both cases fail for M1's reason. VERIFIED by running them.
- **At HEAD.** `cargo test -p ezsdr-mock-radio` gives 235 passed.
- **N108-99** (`plan/maintenance/tools/mutations.json`, last row) replaces only the `if let` scrutinee. The row's end stays at `origin` under the mutant. `check`'s first assertion is therefore the clock one (`mock_radio.rs:2434`), and it is the one that fires. INFERRED by reading, since the brief forbids applying a mutant. It is consistent with the 9397eb18 run, whose first failing assertion is the same, and with `rows-4b2d-fix1.log:99` (killed).
- The anchor matches once in `lib.rs`, and `ClockDomainId` is in scope (`lib.rs:16`), so the mutant compiles.

### 3. Texts

**Closed:**
- **m1:** MR-27 now states the tail once, in its second paragraph (`design/09-mock-radio.md:181`), as "the cut's sample less the first sample not delivered (RM-17)". The duplicate sentence in the first paragraph is gone.
- **m3:** MR-19's `*Test:*` line (`09:154`) drops "none named" and describes T3 correctly.
- **m4:** the Changes row now lists MR-20 and MR-22.
- **n1:** VERIFIED against the mutants.
  - T4 (`mock_radio.rs:2319-2320`): `= next_sample` gives no tail, because `undelivered` is 142 000, which is above the cut of 30 000; `= stop` gives 26 000 at 4 000.
  - T4b (`:2350-2351`): the clock ends at 2.004 s and at 2.142 s.
- **n5:** MR-37 (`09:188`) says "a fault's or a refused block's", "accepted", and "published a block, accepted or refused (MR-25)", which matches `rx_tail` (`lib.rs:1313-1316`).
- **The new MR-25 sentence** (`09:174`) matches the code.
- **SC-20a** is unchanged and consistent.

#### Minor

**(a) m2 is only partly closed: RM-17's new clause says something different from MR-19 at a gap of 0** (`design/07-radio-model.md:190`).
- The clause reads "whose next block begins at the first block boundary at or after `a + gap`, so that at least the refused block is lost (MR-19)".
- At a gap of 0, the first block boundary at or after `a` is `a` itself, the refused block's own start. So the clause's first half contradicts its second half, and MR-19's `k_g = max(b, a + gap)` (`09:153`) and `mr_19_a_zero_gap_still_loses_the_dropped_block`, which pin `b`.
- The Mock has no block grid: the block after a refusal begins exactly at `k_g` (`lib.rs:589-591`), and `block_len_jitter` makes block lengths irregular (`lib.rs:543-546`). So "block boundary" names nothing the code has.
- The same happens with a gap that is not zero. The code resumes at exactly `max(b, a + gap)` (`lib.rs:588-589`), so when `a + gap` does not fall on the block length from `a`, the next block does not begin at a "block boundary" in any grid sense. The tests do not show this only because `a` and the 140 000-sample gap are multiples of the 2 000-sample block. `decisions.md` chose "at a block boundary on the Mock" deliberately. The intent, `b`, the refused block's end, as the floor, is right; the wording is not.
- "(MR-19)" also appears twice in the one parenthesis.
- Fix: "whose next block begins at `max(b, a + gap)`, `b` the refused block's end (MR-19's `k_g`)".

**(b) RM-25 contradicts the new MR-25 sentence for exactly M1's sequence** (`design/07-radio-model.md:274` against `design/09-mock-radio.md:174`).
- RM-25 reads: "A receive SampleClock is registered with the first block its segment publishes …, so a segment that publishes none — one cut at or before its origin, or one whose samples were all lost before a loss or an abort — has none".
- A refused first block followed by an abort is a segment cut at its origin (the abort's cut is `undelivered = 0`), so RM-25's first case, "one cut at or before its origin", applies literally. It is also one "whose samples were all lost before … an abort". By either case RM-25 says it has no clock. MR-25 says it has one, ended at its origin, and the code agrees with MR-25.
- n5 named RM-25 among the texts whose "published" needs the refused-publish reading, and the round fixed only MR-37. RM-25 holds for both Providers; uhd-rx satisfies it either way.
- Fix, one clause in RM-25: "(a block a Mock `Block` link refused counts as published, MR-25)", or narrow the "all lost" case to samples the device never returned.

#### Notes

- **n-a.** "First sample not delivered" (RM-17, MR-27), "first sample not accepted" (MR-19) and "first sample no block has been accepted for" (MR-25) name the same index, `undelivered`. They are three phrasings rather than three rules, and none contradicts another. Unify them if any of these texts is touched again.
- **n-b.** Pre-existing, not introduced by the round: a segment's row in the `None` arm without a clock ends at `t`, not at its cut (the origin).
  - The no-publish abort above writes 2.001 s.
  - A `stop` before the origin writes 0 for a segment whose origin is 2.000 s. The probe at HEAD gave `ended_at` ticks 0 for both `stop(Abort)` and `stop(Orderly)` right after `arm_start`.
  - MR-27 calls `ended_at` the segment's exclusive end, so an end before the origin is odd.
  - The round now gives the same cut (the origin) two different row ends, depending on whether a refused publish happened. Writing `origin` in every `None` case would be uniform. That is INFERRED as right, and it would need its own test, since the row-at-`t` case may be pinned elsewhere. It belongs in the deferred list, not this round.
- **n-c.** No row pins the row-end half (`if registered.is_some() { origin } else { t }`, `lib.rs:1289`). The new test's row assertion (`mock_radio.rs:2437`) would catch `t` in case 1, where `t` is about 2.001999 s and the origin is 2.000 s. That is INFERRED by reading. A row is optional.

### 4. Anything new

- No leftovers.
- Other diff checks:
  - The §6 row (`design/09-mock-radio.md:315`), read in full, matches the tests' values: T4b's clock end at sample 2 000, and the new test's "the clock and row end at the origin, no event". So does the Changes row (`:472`).
- The test's helper closure keeps the sequence visible.
  - The case 2 comment's arithmetic (2.025 s, 2.026 999 s) matches the run.
  - The drained 8 000 tail is correct: cut 10 000 less the first sample not delivered, 2 000.
- `cargo fmt --all --check` is clean on the main tree.

Not checked: I did not rerun clippy, the workspace suite or `rules.py`, which the report states green. I did not execute N108-99 itself (forbidden), only read it plus the 9397eb18 run. The scratch copy `/tmp/rc4b2d` remains (probe tests only, never the main tree).

## review-4b3

## Sub-step review — stage 4b, sub-step 4b-3 (Opus, read-only)

Commit `1cf0b296` against `2d9ed796`. **Verdict: FAIL.** Three majors, all in the S9 bench tool (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs`). Each would make the next bench report a verdict the hardware did not earn: two false FAILs under conditions the bench plan runs, and one false PASS of the retune cell. S21, S22, S10b and the texts are sound apart from minors.

What I did not do: no build and no test run. Mutation outcomes come from `rows-4b3.log` (61/61 killed). `full-4b3.log` was still running when I read it. UHD 4.10 source was read in the `ezsdr-v4-dev:local` image (`/root/tmp/uhd-4.10.0.0`). The other UHD facts are cited from `design-draft.md` §VERIFIED.

### Majors

#### M1 — A second overflow in an S9 trial FAILs correct behaviour, and the bench runs exactly that condition (H4, 100 MS/s)
- **The code.** `s9_extract` pushes every `RX_OVERFLOW{cause: overrun}` the Run delivered into `tails` (`s9bench.rs:346-353`). Its doc assumes "the Run has one stall, so these are the old segment's report" (`:133`). `s9_verdict` then FAILs on `tails.len() > 1` with "more than one tail" (`:167-169`). A sequence-cause event anywhere in the Run FAILs as "the cascade race" (`:175-177`).
- **Evidence that the condition occurs.** Receive at 100 MS/s overflows under the bench's H4 load: "`h4` の負荷の下では受信が 50 MS/s まで（100 と 200 MS/s は overflow）" (`plan/phase8/bench-results.md:458`, record suite 2026-11-13, #1 OBX). KL-4 says the same (`KNOWN_LIMITATIONS.md:40`). The S9 bench plan runs both rate cells "H0 のあと H4" (`plan/phase8/bench.md:1735, 1743`).
- **S4 treats the same condition the opposite way.** S4's evidence makes a trial with `overflow_rows.len() != 1` `Incomplete`, so INCONCLUSIVE (`measurement.rs:2675-2685`). S9 makes it FAIL.
- **Minimal sequence.** H4 host, 100 MS/s, target A:
  1. stall overflow, result row;
  2. booking (cell A);
  3. successor starts on time (index 0);
  4. during the 100 ms `wait` (`s9bench.rs:322`) the loaded host overruns again;
  5. the Module correctly reports a second `RX_OVERFLOW{overrun}`, with UHD's restart block, on the successor's clock;
  6. `tails = [t_stall, t_load]`, so the verdict is FAIL "more than one tail".
  - An overrun in the 200 ms before the stall gives the same result.
  - A load-induced sequence error gives FAIL "the cascade race", which points the owner at the stop-at-the-cut fallback.
- **Likelihood.** Per trial it is INFERRED: each trial streams about 0.3–0.4 s outside the stall, and the overflow rate under H4 at 100 MS/s is not recorded. The condition itself is VERIFIED to occur.
- **Smallest fix.** In `s9_classify`, return `Inconclusive("an overflow besides the stall's")` when the `rx_recv` trace has more than one `overflow` row with `out_of_sequence == false`. This matches S4's rule.
  - Keep the sequence-cause FAIL only if the orchestrator wants the cascade proxy to stay strict. Otherwise scope it to the successor's clock domain: the event's `time.domain()` is the successor's SampleClock, and it is timed at its first sample.
  - This needs one pure case and one row.

#### M2 — The "retune across the LO boundary" cell never crosses the boundary
- **The code.** `s9_trial_with` sets only the rate in the initial requests (`s9bench.rs:287`). A retune trial books `RX_START_LEAD_RETUNE_HZ.1` = 1 000.5 MHz (`:313`). Every trial is a fresh Session, so every retune is from the channel's default.
- **The default.** The default is 1 GHz for UBX and OBX (`crates/ezsdr-radio/src/x3xx.rs:201-206`; `channel_hz`, `tests/common/mod.rs:188-197`). That the Session tunes there at connect is INFERRED from KL-3, "every tune is untimed: at connect".
- **UHD 4.10 source (VERIFIED).** 1 000 MHz and 1 000.5 MHz fall in the same band on both boards:
  - UBX: `freq >= 1000 * fMHz && freq < 1500 * fMHz` (`host/lib/usrp/dboard/db_ubx.cpp`, the branch right after the 800–1000 MHz one, about line 1163).
  - OBX: `else if (freq < 1500e6)` after `freq < 1000e6` (`host/lib/usrp/dboard/obx/obx_expert.cpp:~297`).
- **How S5 avoids this.** S5 does a setup restart to `.0` = 999.5 MHz first and then alternates (`measurement.rs:2448-2460`). Its constant's doc says the endpoints are "on opposite sides and never 1 GHz itself" (`measurement.rs:2164-2167`).
- **Observable wrong outcome.** The retune cells measure an in-band retune, which is cheaper than the band switch. Their PASS would then be read as confirming Q1's caveat that a retune restart in the gap keeps about 0–2.5 ms of margin (`decisions.md` Q1; `bench.md:1739` "LO をまたぐ retune（1 GHz をまたぐ周波数）"; `hardware.rs:4248` "retuned across 1 GHz"). That is a false PASS of the condition the owner was told the bench checks.
- **Smallest fix.** Book `RX_START_LEAD_RETUNE_HZ.0` (999.5 MHz, below the 1 000 MHz edge on both boards) instead of `.1`. The alternative is to put `.1` into the initial requests for retune plans.
  - Add one assertion to `s9_cells_are_the_two_rates_by_retune_by_target`, or a pure check that the retune target and the default straddle 1 GHz.
  - Re-anchor no row; N108-102 still covers the cell list.

#### M3 — A booking that ends close to UHD's own restart FAILs correct behaviour (the brief's open item: real, narrow, fixable by INCONCLUSIVE)
**Facts.**
- UHD queues its restart `START_CONTINUOUS` at `get_time_now() + 0.05 s` (`radio_control_impl.cpp:48, 105-125`), on the cascade that `_handle_overrun()` starts before `recv` returns the overflow (`rx_streamer_zero_copy.hpp:209-245`, `design-draft.md` §VERIFIED).
- So the restart `r` is at least about 50 ms after the result row's `host_before_ns`.
- The Module stops at the earlier of two moments:
  - at `cut + span` once quiet, with `span` = one block plus 3 ms, about 3 ms (`rx.rs` `timeout`, `:567-575`);
  - at once, on reading a packet at or past the cut (`samples`, `:613-616`).

**Two sub-windows, both needing the booking to end about 47 ms or more after the result row (a delayed test thread or slow submits; plausible only under H4):**
- **(i) Cut `c` in (r − span, r].** The Module's stop comes after UHD's START has run. Restart packets at or past the cut end the old segment correctly (tail = c − k_f = `expected_tail`).
  - The packets in flight after the Module's `rx_stop` reach the successor's stream with `first_tick < origin`. They are counted in `stats.rx_before_origin` (`rx.rs:597-604`).
  - Result: **FAIL "a report reached the successor before its origin (the cascade race)"**.
- **(ii) Cut `c` in (r, first restart row's `before`).** The old segment resumes at `r` before its cut, so its loss is an ordinary RM-17 report with `lost = r − k_f`, carried by a block.
  - `expected_tail` is `c − k_f`, so the verdict is **FAIL "the tail's lost is wrong"**, and (i)'s in-flight count usually adds the cascade FAIL as well.

**Why `S9_RESUMED` does not close it.**
- The rule only sees a `samples` row that began before the post-stamp (`s9bench.rs:81`).
- In (ii) the restart's first packet reaches the host one transport latency after `r`. In (i) no row may have begun before the stop. In both cases the booking can complete before that row begins.
- On the manual-clock fake the window is closed: M5 means device time trails real time, and a reader busy in `recv` stops the clock (`host.rs:17-23`). Only hardware latency opens it. That is why S10a never saw it.

**Severity.** Major under the brief's rule (a false FAIL on hardware, and its text blames the cascade race). The probability is low on H0 and INFERRED on H4.

**Smallest fix (INCONCLUSIVE, not a new classification).**
- Give `s9_classify` the restart floor `restart_after_ns` and the stream's `span_ns`. Return `Inconclusive("booked within a receive wait of the device's own restart")` unless `post_stamp_ns + span_ns + 1 ms < result.before + restart_after_ns`.
  - On the hardware path `restart_after_ns` is 50 ms (UHD's `OVERRUN_RESTART_DELAY`, VERIFIED).
  - On the fake it is its own `G` after the call.
- `S9_RESUMED` can stay as a second guard.
- Also add the result row's `before`/`after` to `trial_row` (`s9bench.rs:461-469`): today a person cannot audit an A/B classification from the printed row.
- One pure case each side of the bound, and one row.

### Minors

- **m1 — KL-15 promises a cell the tool cannot run.**
  - KL-15 says the next bench runs "a 65 536-sample block at a low rate; two channels if the tool allows" (`KNOWN_LIMITATIONS.md:230`, the `overflow_restart_gap_ns` row). Only the two-channel clause is qualified.
  - `bench.md:1772` says the 65 536 cell "今の tool では流せない（4b では作らない）". Decision m1 (Fable, adopted) says "the post-4b bench adds a 65 536-sample low-rate cell".
  - The user-visible text therefore overpromises.
  - `hw_s4_overflow_gap` installs no selector (`hardware.rs:4219-4241`). `bench_profile` does honour the thread's selector override (`tests/common/mod.rs:242-246`), and `overflow_manifest` builds its profile through `bench_profile` (`measurement.rs:2799`). So wrapping the body in `with_selector(json!({"block_len": N}), …)` from an env value would very likely let the tool run that cell. That the override reaches `overflow_trial`'s Run is INFERRED from that path; I did not run it.
  - Recommendation: add the knob (a few lines, an existing helper; fulfils m1). Otherwise qualify KL-15's clause like the two-channel one; that is owner-visible text.
- **m2 — Cell B passes a shifted first block on `START_LEAD_SHORT` alone (`s9bench.rs:190-193`). This is a question for the orchestrator, not an implementer defect.**
  - A start that reports `START_LEAD_SHORT` keeps its origin (RM-25; `design/07` `START_LEAD_SHORT` row "the start keeps its origin"). So `first_index > 0` with no `LATE_COMMAND` is an unreported miss, and passing it is a false PASS.
  - The plan and M1's wording ("LATE_COMMAND or START_LEAD_SHORT") allow it literally.
  - Suggested rule: `first_index > 0` requires `late_command > 0`; `START_LEAD_SHORT` is accepted with index 0. Reachability on hardware is INFERRED to be low.
- **m3 — The cascade verdict's text names the wrong thing.**
  - `rx_before_origin` counts old-stream *samples* dropped at the successor's origin (`rx.rs:597-604`), not reports. Reports before the origin go to `rx_overflows` and the `rx_report_before_origin` timing row (`rx.rs:522-531`).
  - The text "a report reached the successor before its origin" (`s9bench.rs:179`) and `bench.md`'s judgement section should say "old-stream samples reached the successor before its origin".
- **m4 — `S9_RESUMED` also counts the successor's own samples.**
  - Any `samples` row after the result with `before < post_stamp` counts (`s9bench.rs:81`), the successor's first rows included.
  - The successor's first row begins about `lead − span` ≈ 7 ms after the start is booked, so a test thread delayed about 7 ms between `submit` and the post-stamp (`:315-318`) turns a correct A trial INCONCLUSIVE. That costs yield only (H4).
  - Fix: limit the check to rows before the first `rx_start` row issued after the pre-stamp.
- **m5 — Two before/after choices in the classifier are unpinned.**
  - `resumed` reads `row.before` and `backlog` reads `row.after`. No case puts a stamp between a row's `before` and `after`.
  - So `before`→`after` in `:81`, or `after`→`before` in `:84`, survives every case: cases 15–17 use post-stamps of 1 350, 1 300 and 1 410 against the row 1 300..1 310.
  - The `after` mutant on `:81` is the unsafe direction: a call begun before the booking completed would be missed. Add case 18 (post 1 305 → RESUMED) and a B case with the pre-stamp inside a backlog row; one row each.
- **m6 — The bench entry path has no default-suite rehearsal.**
  - The hardware test enters via `s9_cells → s9_run → s9_decisions`, with `trial_row`, the latch/`stopped` suffix and target B's "not reached" exit. Only `s9_trial` (target A) runs in the default suite (`fake.rs:10172`); `s9_run`, `s9_decisions` and the target-B wait are never executed.
  - S4 has such a run (`fake.rs:10444-10451`, `s4_run` with a latch).
  - Add the same: one A cell with two trials through `s9_run`, and a target-B plan reaching `Inconclusive("the booking state was not reached")` on the fake. Assert the decision lines and the stop suffix. AGENTS §9: compile-only is not a check.
- **m7 — Maintainability (duplication).**
  - `s9_run` (`s9bench.rs:411-436`) repeats `s4_run`'s runner (`s4bench.rs:614-640`): stop on `Err`, the latch check after each trial, the "no later trial runs on this device: power-cycle…" line. A change to the latch or stop policy must be made twice.
  - A shared `run_cells(cells, trial_fn, row_fn, latched, label)` would hold it once. Neither copy is wrong today.

### Notes

- **n1. Deviation 5 (always one row, `Unmeasured { reason }`) is forced, not optional.** `fake.rs:653-666` asserts that each group supplies exactly its declared keys ("the keys a failed group marks unmeasured are the ones it supplies"). `merge_values` keeps the supplied reason on `Source::Hardware` (`measurement.rs:176-198`). Correct.
- **n2. Deviation 1's premise holds on the X310 (VERIFIED from the source).** After the overflow result no old packet comes until UHD's restart: the flush happens in `_handle_overrun` before `recv` returns, and an overrun ends the radio's command (`radio_rx_core.v:586-597`). So "a post-result `samples` row means the stream resumed" is right; its only gap is M3. The deadlock reason for dropping `pause_reader` is consistent with `host.rs` M2/M3.
- **n3. Deviation 3's pre-stamp guard (`s9bench.rs:338`) is redundant with `start_index`,** which counts the `rx_start` rows before the booking. N108-103 kills `start_index = 0`, not the guard. Harmless.
- **n4. Deviation 6's proxy is acceptable as decided (INFERRED),** subject to M1's scoping.
- **n5. `expected_tail` is a consistency check, not an independent oracle.** It re-derives `lost` from the Manifest clock's `ended_at` and the event's tick, so it catches the tail and the clock disagreeing on the cut (4b-2b m1's class). The two agree on lattice: with a loss pending, the end is `planned_end` = `clock.instant(sample)` (`rx.rs:322-326, 360-365`), so the floor in `expected_tail` (`s9bench.rs:380`) matches `at_or_after`.
- **n6. The stamp-skip in `recv_row` (`s9bench.rs:51-52`) is unreachable.** The probe always writes u64 stamps on `rx_recv` (`probe_device.rs:435`). No row is needed. If kept, prefer INCONCLUSIVE on a stamp-less `rx_recv` row: a skipped `samples` row could turn `S9_RESUMED` into A.
- **n7. S10a's `is_resume` compares real-ns gaps against `stall_ns` on the manual clock.** That is deterministic in the safe direction: the manual clock never runs faster than real time (`host.rs:20`, M5). The S9_RESUMED-only retry and the overfill case 3 are schedule-independent as reported.
- **n8. S10b names.** N108-54's name says "ends … while the backlog still arrives", but the mutant stops 1 ms after the overflow result (`timeout()` runs only on `Timeout`), as the report itself says (36 vs 40 ms). The assertion "no stop while the backlog arrives" (`rx.rs` test, after the backlog loop) cannot fail for the same reason. Rename the row and the assertion message; the test itself is right.
- **n9. The hardware record now compares the overflow value with the profile** (S21 (3), intended). An `h4` record run (139.1 ms seen before) can now fail `hw_measurements_record`'s comparison. Say so in the bench brief.
- **n10. The fallback reason for the overflow key is now stale.** It still says the key "stays unmeasured pending the first hardware bench reading" (`measurement.rs:228-230`), though it appears only in records built without the group (kept by the plan).
- **n11. `v58_05_a_loss_after_an_overrun_stops_the_run_and_keeps_the_tail` (4b-2b, Mock) is named in no `design/` `*Test:*` line.** It belongs to spec 07/09, outside 4b-3's spec-18 task. Every other test 4b added that pins a rule is named: I searched `design/` for each `fn` the diff `a4ad013a..1cf0b296` adds.

### Checklist results

1. **S9 on hardware.**
   - The deviation 1 rule is right on an X310 (n2) but leaves M3.
   - Deviation 3 is redundant but harmless (n3).
   - Deviation 6 is fine as a proxy, but it FAILs on load events (M1).
   - The retune cell is not what it claims (M2).
2. **S21 and S22 are correct.**
   - `overflow_trial_complete` has one copy, called by `S4Trial::complete` and `summarize_overflow_gaps` (`s4bench.rs:566`, `measurement.rs:2723`).
   - The value row is the maximum restart-after-resume, the condition is its rate, and `trials` is that rate's complete count.
   - `merge_values` keeps it on Hardware and replaces it on Fake.
   - The `compare` test covers at-or-below and above.
   - The S22 sweep line uses the same `bound_line`, and S4B-29 is re-anchored on the shared `all(`.
3. **S10a and S10b.**
   - S10a retries only `S9_RESUMED`, three times; case 3 is deterministic; the schedule analysis is right.
   - It rehearses cell A through the same `s9_trial` the hardware path calls. The `s9_run` and decision path is not rehearsed (m6).
   - S10b's issued-instant assertion (`issued ∈ [t_o + span, t_o + span + 1 ms]`) is exact on manual clocks.
4. **Texts.**
   - RM-17's `max(b, a + gap)` matches MR-19's `k_g` (`design/09:153`).
   - RM-25's refused-block clause is correct (`design/07:274`; MR-19 and MR-37 say the same).
   - Spec 18's Changes row and its `*Test:*` names resolve: every named test exists.
   - `bench.md` S9 states the classifier, but it carries M2's false "LO をまたぐ" and m3's wording.
   - KL-15 overpromises (m1).
5. **Rows.**
   - Every new row has a killing case as reported. N108-60's retarget is fine (P798 kills the minimum).
   - Gaps: m5's two directions, M1's and M3's new rules (to add), and the stamp-skip (n6).
6. **Maintainability.**
   - Duplication: m7.
   - The `pub` widenings are needed and minimal: `advance_until`, `DEVICE_WAIT_MS`, `RX_START_LEAD_RETUNE_HZ`, `book_receive_stop`. `book_receive_stop`'s extraction removes a copy rather than adding one.
   - Leftovers: n10 only.
   - `s9bench.rs` keeps the pure decisions apart from the trial body.

## recheck-4b3

## Re-check — 4b-3 fix round 1 (Opus, read-only; AGENTS §9 step 8)

Commit `61f53f27` against `7640eeb4`, branch `work/4b`. Line numbers are at `61f53f27`. UHD 4.10.0.0 source was read in `ezsdr-v4-dev:uhd4.10` (`/root/tmp/uhd-4.10.0.0`).

**Verdict: M1's adopted fix is CLOSED, but brief item 1 is NOT CLOSED: the sequence-cause FAIL, which the round left untouched, still gives a false FAIL on correct behaviour (one major, reachability INFERRED). M2 is CLOSED. M3 is CLOSED apart from a narrow residual, rated minor with its reason in §3. m1, m2, m4, m5 and m6 are CLOSED. The round adds no regression; there are two minors and several notes.**

### 1. M1: adopted fix CLOSED; brief item 1 NOT CLOSED (the sequence-cause clause)

**Closed.**
- More than one non-sequence overflow row anywhere in the trial's trace now makes the trial INCONCLUSIVE (`s9bench.rs:125-127`).
- The count is right:
  - Each trial has its own `ProbeDevice` (`:347`), so the trace is the trial's own rows only.
  - `is_result` excludes `out_of_sequence` rows (`:95-97`).
  - The count covers rows before the stall, in the backlog and on the successor. A second overrun anywhere is therefore open, as S4 treats it (`measurement.rs` `overflow_rows.len() != 1`).
- Case 23 has two forms, an overflow after the result and one before the stall. With `> 1` changed to `> 2`, or the check removed, the base trace classifies A and both forms fail at `case 23` (N108-104 killed).
- Case 24 pins that a sequence row is not counted.

**The kept `tails.len() > 1` FAIL cannot fire on correct behaviour (VERIFIED from the code).**
- In uhd-rx an `RX_OVERFLOW` whose cause is overrun needs a `Pending::Overrun`. Only a non-sequence overflow row at or after the origin sets one (`rx.rs:536-539`).
- A report that arrives before the origin only sets `stale_overrun` (`:525-531`), which drives the second stop and emits no event.
- The pending report is consumed once, at one of two sites:
  - by the next block, as one event (`:653, 677-681`);
  - or by the tail, through `emit_tail` (`:349, 370-377`).
- So with at most one such row, at most one overrun-cause event is possible. That is exactly what the classifier now guarantees before the verdict runs.

**Major (reachability INFERRED): the untouched sequence-cause "cascade race" FAIL counts sequence errors from the whole Run, not the successor's first block.**
- **The code.** `s9_extract` adds every `RX_OVERFLOW{cause: sequence}` the Run delivered (`s9bench.rs:429`). `s9_verdict` FAILs on any of them (`:235-237`). Nothing restricts the count to the successor's clock or to its first block.
- **What the documents say.**
  - `bench.md` defines the third verdict as "successor の最初の block が `GAP_BEFORE | SEQ_DISCONTINUITY` を持つ＝sequence の `RX_OVERFLOW`".
  - The orchestrator left the verdict alone on the premise "a SEQ-cause overflow at the successor is the race it names" (`decisions.md`, 4b-3 fix-round line).
  - The code does not implement that premise.
- **Minimal sequence.** H4 host, 100 MS/s, target A:
  1. During the 100 ms before the stall, the loaded host drops one packet.
  2. UHD returns an `out_of_sequence` overflow row, and uhd-rx reports `Pending::Sequence` (`rx.rs:541-543`).
  3. The next published block carries it as one `RX_OVERFLOW{sequence}` on the old segment's clock (`:677-681`).
  4. The stall, the booking and the successor all behave correctly.
  5. The classifier gives A, because case 24's rule ignores sequence rows.
  6. The verdict is **FAIL "a sequence-error overflow in the Run (the cascade race)"**. That points the owner at the stop-at-the-cut fallback for a correctly reported host drop.
  - The same happens for a drop in the backlog that is followed by more backlog blocks: the next block consumes `Pending::Sequence` and emits a sequence-cause event (`rx.rs:653, 677-681`).
  - It also happens for a drop on the successor after its first block, within the 100 ms wait (`s9bench.rs:396-397`).
  - Only a sequence row with no published block between it and the overflow result merges into the tail. The tail then names the overrun over the sequence error (RM-17; `loss_causes` precedence), so no sequence-cause event is emitted.
- **Reachability (INFERRED).**
  - Overflow under H4 at 100 MS/s is VERIFIED (`plan/phase8/bench-results.md:458`).
  - Whether sequence errors occur there is not recorded. The committed bench rows count overflow events without their cause:
    - `grep -rhoi '"cause": *"[a-z]*"' bench/` gives 0 hits;
    - `grep -n -i "out_of_sequence\|sequence error\|seq_discontinuity" plan/phase8/bench-results.md` gives 0 hits.
- **Smallest fix (recommended: (a)).**
  - (a) In `s9_extract`, count only sequence events in the successor's clock domain at its first sample: the event's `time.domain()` differs from the tail's domain, and its tick is 0. That is `bench.md`'s own rule.
    - The first block's event is stamped at `expected` (`rx.rs:675`). A stream starts with `expected: 0` (`rx.rs:78`), and an on-time first block is index 0 from the origin (`:613`).
    - A first block without a jump consumes its reports without an event (RM-17). So the race's event is that of a first block with a jump, stamped at tick 0.
    - (a) removes all three load cases above.
  - (b) In `s9_classify`, return INCONCLUSIVE when an `out_of_sequence` row precedes the successor's `rx_start` row (the `successor_start` lookup at `:136` finds that row). This is not equivalent: it removes the pre-stall and backlog drops, but a drop on the successor after its first block still FAILs.
  - Either needs one pure case and one row.
  - This decision is the orchestrator's (AGENTS §9: a premise refuted, re-decide).

**Note (M1's rule can hide, not only clear).**
- A real cascade defect that also produces a second non-sequence overflow, such as a surviving restart that overruns, is now INCONCLUSIVE ("an overflow besides the stall's") instead of FAIL.
- On H0 such trials should not happen. The bench brief should say that INCONCLUSIVE for this reason on an H0 run is a finding to bring to the owner, not lost yield.
- The bench.md sentence that P6's second residual "appears in the third verdict" stays true: that residual involves the stall's own single row.

### 2. M2: CLOSED

- The retune books `S9_RETUNE_HZ = RX_START_LEAD_RETUNE_HZ.0` = 999.5 MHz (`s9bench.rs:64, 388`).
- **The Session's tuned frequency.**
  - The S9 Session's initial requests are only `radio.rx.c0.stream` and the sample rate (`s9bench.rs:353`; `tests/common/mod.rs:512-516`). No frequency comes from the selector override either: `pin_to_slot_a` installs only `rx`/`tx` (`tests/common/iii.rs:377`).
  - The configured frequency is therefore RM-5's default, 1 GHz for UBX and OBX (`ezsdr-radio/src/x3xx.rs:201-206`, `:343-348`).
  - uhd-rx's configuration starts from `description.defaults()` (`provider/mod.rs:485`), which holds each channel's `frequency_hz` at its front end's default (`ezsdr-radio/src/channels.rs:473-479`).
  - A start's config snapshot holds that default after a `Stop` and a start: `ur_26_a_value_for_a_streaming_stream_is_refused_without_a_device_call` asserts it (`provider/control.rs:1610-1632`).
  - So `Settings.freq` is `Some(1e9)` (`provider/mode.rs:54`), and the tune at `uhd.rs:905` runs. This is VERIFIED from the code. That the bridge then tunes at connect and at every `start_rx` is KL-3's statement ("Every tune is untimed: at connect, and again for each receive segment a `start_rx` begins").
  - It tunes through `uhd_usrp_set_rx_freq` with an AUTO policy on the exact target (`uhd.rs:905-916`).
  - Both boards set `use_lo_offset` false on RX (`db_ubx.cpp:586`, `db_obx.cpp:327`). So `multi_usrp.cpp:149-163` adds no offset, and the front end sees exactly 1.000 GHz, then 999.5 MHz.
- **The RX band edges (VERIFIED).**
  - UBX: `freq >= 800 && freq < 1000 MHz` against `freq >= 1000 && freq < 1500 MHz` (`db_ubx.cpp:1153, 1163`).
  - OBX: `freq < 1000e6` against `freq < 1500e6` (`obx/obx_expert.cpp:293, 297`).
  - So 1 GHz lies in the upper band and 999.5 MHz in the lower one on both boards: every retune trial crosses the edge. A fresh Session per trial retunes to 1 GHz at connect, so this holds for every trial, not only the first.
  - The TX edges are `<= 1000e6` (`obx_expert.cpp:158`, `db_ubx.cpp:984`), where 1 GHz falls in the lower band. That does not matter, because S9 is receive only.
- **Test.** `s9_retune_target_crosses_the_one_gigahertz_edge_from_the_default` fails at its first assertion when the constant is `.1` (N108-118).
- **Note.**
  - Nothing observes the actual pair of tunes. The test compares constants, and the probe records no `apply` rows (`probe_device.rs:363`). So a frequency later added to `s9_trial_with`'s initial requests would pass unnoticed.
  - The closure therefore rests on reading the code above, not on a test.
  - A one-line guard is cheap: assert that `initial` has no `frequency_hz` key.

### 3. M3: CLOSED (one minor residual)

**The derivation (VERIFIED).**
- UHD calls `_handle_overrun()` inside the `recv` that returns the overflow, after the zero-timeout poll and before setting `ERROR_CODE_OVERFLOW` (`include/uhdlib/transport/rx_streamer_zero_copy.hpp:225-236`).
- The handler posts `RX_RESTART_REQ` (`rfnoc/rfnoc_rx_streamer.cpp:179-184`).
- The radio's handler fixes the restart's time as `get_time_now() + 0.05` (`rfnoc/radio_control_impl.cpp:48, 105-117`). It then posts the START asynchronously, but its time is already fixed.
- `get_time_now()` runs after the result call began. So the restart `r` is at least `result.before + 50 ms` in host terms; the host/device offset cancels. The constant's citation is right, and it matches `design/18-uhd-radio.md:27`.

**The other side of the inequality (VERIFIED from the code).**
- The cut of an untimed Stop is the instant the Provider handles it (RM-16, `design/07-radio-model.md:177`).
- On a Hardware-class Run, `submit` waits for the dispatched targets to finish (`ezsdr-kernel/src/coordinator/pipeline.rs:1046-1050`, `state.rs:349-351`). So the cut `c` is at or before the post-stamp.
- uhd-rx stops once quiet and `now >= cut + span` (`rx.rs:576-585`).

**Span and window.**
- The span comes from the trial's own `block_len` (`s9bench.rs:350-351`), read where uhd-rx reads it (the selector, `provider/mod.rs:253-257`). That matches `quiet_span` (`rx.rs:752-754`).
- The fake's `restart_after_ns` is the profile's gap (140 ms). On the stall path the fake restarts at `now + gap` of the result call (`src/device.rs:1808-1812`), in device time.
- The probe's stamps are real time (`probe_device.rs:128-130`, `Instant`). Since the manual clock never runs ahead of real time, the real restart is at or after `result.before + gap`, which is the safe direction.
- The per-trial stamps are printed (`s9bench.rs:536-547`; pinned by N108-117 and N108-119).
- With a quiet window of about 46 ms and A bookings made about 1 ms after the result (the watch polls every 1 ms), real defects are still judged. Only a booking delayed by more than about 45 ms becomes INCONCLUSIVE.

**Minor — the 1 ms margin is used up by uhd-rx's own receive-wait rounding.**
- `recv_timeout` clamps each wait to at least 1 ms (`rx.rs:503`), and UHD truncates the timeout to whole milliseconds (`rx_streamer_impl.hpp:154`).
- So the `timeout()` that ends the stream returns in `cut + span, cut + span + 1 ms)`. The `rx_stop` pair then follows after a thread wake-up and a control-port round trip.
- The slack left is only `post − c` (the `start_rx` submit, plus the retune sets) and `get_time_now()`'s read latency, a few hundred µs on H0 (INFERRED).
- A booking that ends in the last fraction of a millisecond before `result.before + window`, met by an unlucky phase of the wait or a late reader thread under H4, can still hit shape (i): a false FAIL from in-flight samples before the origin.
- **Why minor although it is a false FAIL.** A false FAIL is major under the brief's rule. This one, however, needs a booking that already ended about 44.5 ms or more after the result call began (only a host delayed that long under H4) and that also falls in a sub-millisecond sliver at the bound with an adverse phase of the wait. Its fix is one constant. If the rule is applied literally, it is a narrow major with the same fix.
- **Fix:** a 2 ms margin (1 ms for the clamp plus 1 ms of slack). Change `S9_RESTART_MARGIN_NS` and the two figures in `s9_window_and_span_…`.

### 4. Minors

**m1 (S4 block length): CLOSED.**
- `EZSDR_S4_BLOCK_LEN` is read through `env_values` and passed by `hw()` (`bench.md` `-e EZSDR_S4_BLOCK_LEN`).
- It is merged after `slot_a_guarded()`, which installs the `rx`/`tx` pin in the same thread (`hardware.rs:4234-4238`; `iii.rs:377`).
- `s4_run → overflow_trial → overflow_manifest → bench_profile` stays on that thread (`measurement.rs:2797`; no spawn in `s4_run` or `overflow_trial`).
- Wrong changes (N108-123, N108-124):
  - dropping the assignment fails `with_block_len_keeps_the_layout_pin`;
  - `contains_key → true` fails `s4_block_len_is_optional_and_positive` at `Ok(None)`.
- **Note:**
  - The `set_selector_override` line in `hw_s4_overflow_gap` is compile-only. `s4_block_len_reaches_the_bench_profile` tests the `with_selector` path, not that line, and has no row.
  - A 65 536-sample `overflow_trial` has never run, even on the fake. KL-15's unchanged wording now rests on it (INFERRED), as the report says.
  - The S4 restart-after-resume estimate is read from `rx_recv` rows. With 65 ms blocks at 1 MS/s its resolution is about a block. The bench should compare the 65 536 cells with that in mind (INFERRED; I did not read the estimator).

**m2 (cell B): CLOSED.**
- B's `first_index > 0` now needs `late_command > 0` (`s9bench.rs:251-253`).
- Restoring `late_command + start_lead_short == 0` fails at `short_b` (N108-111).
- Dropping the `late_command` conjunct fails at `late_b`.

**m4 (successor cap): CLOSED.**
- `resumed_before = min(successor rx_start before, post)`, where a qualifying start is one at or after the pre-stamp (`:136-139`).
- Case 20 fails without the cap (N108-112), and case 21 fails without the `>= pre_stamp` filter (N108-113).
- **Note:**
  - No case has the successor's start after the post-stamp, so changing `start.min(post_stamp_ns)` to `start` survives.
  - On hardware uhd-rx probably issues the successor's device start after the old stream's stops, so the start is usually after the post-stamp. This is INFERRED from #67 (a stop cancels timed starts queued before it); I did not read it in the code.
  - The mutant would widen the resumed window over old-stream rows begun after the booking. With the window rule these cannot carry restart data. The mutant is near-equivalent, so no row is needed.

**m5: CLOSED.**
- Case 18 (post 1 305 inside the 1 300..1 310 row) kills `before→after` (N108-114).
- Case 19 (pre 1 025 inside the 1 020..1 030 backlog row) kills `after→before` (N108-115).

**m6: CLOSED.**
- The new rehearsal runs `s9_run`, `s9_decisions`, the latch suffix and target B's not-reached exit on the fake.
- Each mutant fails at its named assertion:
  - latch ignored → `run.stopped` (N108-120);
  - suffix dropped → `ends_with` (N108-121);
  - not reached reported as A → the cell assertion (N108-122).
- Part 1 asserts no class counts, so it does not depend on scheduling.

**n6, n8, n10 and n11** are as reported. `v58_05_a_loss_after_an_overrun_stops_the_run_and_keeps_the_tail` exists at `crates/ezsdr-acceptance/tests/v58.rs:181`.

### 5. What the round introduced

- **Regressions:** none found. S10a's retry set now includes `S9_NEAR_RESTART`. On the fake the window is about 134 ms, so a retry needs a stall of more than 100 ms in the test thread. No mask results.
- **Leftovers:**
  - `S9Trial::tails`' doc still says "the Run has one stall, so these are the old segment's report" (`s9bench.rs:193`). That is now true because of the classifier, but the doc does not say so. Note.
  - `S9_RESUMED` is now nearly redundant with the window for target A, as the review predicted. It is kept as a second guard; no change.
- **Maintainability:**
  - `is_result` is now shared by the classifier, the watch and `locate`, which is good.
  - m7's rejection stands: about six lines of policy with different prefixes. No new duplication.

### Not done

- I did not run `rules.py` or the mutation rows. The row outcomes are taken from `rows-4b3f1.log` (61/61 killed), which was run before the rebase onto `6cc8026d`.
- I ran `cargo fmt --all --check` on a `git archive 61f53f27` copy: clean.
- I ran the S9, S4 and block-length tests of the `fake` test binary on that copy; see the closing line below.
- I did not run clippy, the `uhd`-feature build or the full workspace suite.
- Result on the `61f53f27` copy (`CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`, command `cargo test -q -p ezsdr-radio-uhd --test fake -- s9 s4_block_len with_block_len s4bench`): 35 passed, 0 failed, 661 filtered out. The 35 cover the S9 pure tests, both S9 rehearsals and the S4 tests. Each ran once.

## sol-code

I reviewed `6cc8026d..74104080` on `work/4b`. I recorded the failure scenarios before opening the stage records, then checked them against the owner’s decisions and the fix rounds. This was a read-only review. Mutation results below come from the existing logs. X310 behavior derived from UHD source citations is marked **INFERRED**.

1. **Major — A multi-packet receive can still make #108 miss the successor’s origin.** At 1 MS/s with 65,536-sample blocks, let uhd-rx receive an overrun and enter a no-cut `recv` at 0 ms. Book Stop and `start_rx` at 1 ms. **INFERRED on X310:** UHD restarts near 50 ms, inside the 68.536 ms timeout. The first packet arrives within that timeout, then the same call can keep collecting packets until about 115.5 ms. The successor was due at about 79.5 ms. uhd-rx trims the old block to nothing and issues the successor start after its origin, even though the host ran throughout and the booking followed the overflow result. P1 already cites the per-packet timeout. The large-block test scripts a single `Timeout` and therefore misses the case. Give the overflow-silence receive a call-wide deadline and test a packet arriving near the end of the first wait. [`rx.rs:236` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:236`) · `rx.rs:495` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:495`) · `rx.rs:3113` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:3113`) · `uhd.rs:1018` (`crates/ezsdr-radio-uhd/src/uhd.rs:1018`) · `test-plan-v3.md:564` (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan-v3.md:564`) · `UR-17` (`design/18-uhd-radio.md:232`)

2. **Major — The second stop can enter UHD after `DEVICE_LOST`.** Receive an overrun, cut the stream, and let the first stop succeed. Pause uhd-rx after the second-stop branch reads `!is_lost`, while it records `rx_stop_overrun_clear` or waits for the UHD control lock. The TX async path publishes and latches `DEVICE_LOST`. Resume uhd-rx. `stream_cmd` enters `uhd_rx_streamer_issue_stream_cmd` without checking the latch, so shutdown can make an extra device call and wait on a dead control path. The existing test publishes loss *inside* the first stop, before the vulnerable check. Order the second command’s admission with loss publication and pin this interleaving. `rx.rs:295` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:295`) · `core.rs:1021` (`crates/ezsdr-radio-uhd/src/provider/core.rs:1021`) · `core.rs:1074` (`crates/ezsdr-radio-uhd/src/provider/core.rs:1074`) · `tx.rs:1387` (`crates/ezsdr-radio-uhd/src/provider/tx.rs:1387`) · `uhd.rs:593` (`crates/ezsdr-radio-uhd/src/uhd.rs:593`) · `rx.rs:2283` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:2283`)

3. **Major — S9 can PASS a successor whose first published block is late and discontinuous.** In cell A at 100 MS/s on the 200 MHz root, give the successor a first raw packet at `origin + 1` root tick, with no sequence report. **INFERRED on X310:** an surviving UHD restart can supply an off-lattice packet. S9 floors `1 / 2` to index 0. uhd-rx rounds it to index 1 and publishes `GAP_BEFORE | SEQ_DISCONTINUITY`. The Provider deliberately emits no overflow event for an unreported jump, so the first-block sequence-event check added in fix round 2 also stays clear. With a valid old tail and no `LATE_COMMAND`, S9 prints PASS. Read the first **published** header on the successor clock and compare it with the booked origin. `s9bench.rs:419` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:419`) · `s9bench.rs:239` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:239`) · `rx.rs:609` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:609`) · `rx.rs:651` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:651`) · `bench.md:1759` (`plan/phase8/bench.md:1759`)

4. **Major — S9 can PASS a tail attributed to the wrong segment.** Let the only overrun be on A, lose A’s tail, and emit one overrun event on B at tick 0 with `lost` equal to B’s final sample index. B otherwise begins cleanly. `expected_tail` selects B’s clock from the event under test, computes the same `lost`, and S9 passes. A wrong cut and a correspondingly shortened tail on A also pass. Select A and B independently, then compare the event’s domain and timestamp with A’s first undelivered sample and its loss with A’s booked cut. `s9bench.rs:430` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:430`) · `s9bench.rs:454` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:454`) · `s9bench.rs:483` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:483`) · `RM-17` (`design/07-radio-model.md:192`)

5. **Major — S9 can report a false cell-A FAIL when the physical stop misses the restart window.** At 100 MS/s, finish booking 44 ms after the result call began, inside the 44.98 ms window. **INFERRED on X310:** a control-lock delay or an H4 scheduling delay lets the queued restart run before the stop pair clears it. An old packet reaches the successor and its origin guard correctly drops it. Since that receive began after the post-stamp, the classifier still calls this A and the verdict reports a cascade FAIL. The extra synchronous stop consumes time even when the first stop already cleared everything. Use the probe’s actual stop stamps and old-stream packet evidence to establish the cell-A premise. The 2 ms margin alone does not establish it. `s9bench.rs:47` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:47`) · `s9bench.rs:135` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:135`) · `probe_device.rs:401` (`crates/ezsdr-radio-uhd/tests/common/probe_device.rs:401`) · `rx.rs:297` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:297`) · `rx.rs:601` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:601`)

6. **Major — S4 promotes a length match to a hardware measurement.** Give an X310 trial one complete capture with an `overflow_restart` gap and one overrun event with the same `lost`, but stamp the event at a different sample or on a different clock. The trial counts as complete. If it supplies the last required trial, the record emits `Measured` and the sweep line can print “at or below” 140 ms. The evidence test itself accepts an event with no timestamp. Require the event and gap to agree on the stream, clock, and first lost sample before either output consumes the trial. The extractor predates 4b. The new value row and sweep bound now use its result to make decisions. `measurement.rs:2613` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:2613`) · `measurement.rs:2673` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:2673`) · `measurement.rs:2766` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:2766`) · `measurement.rs:4174` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:4174`) · `s4bench.rs:663` (`crates/ezsdr-radio-uhd/tests/common/s4bench.rs:663`)

7. **Minor — The S4 sweep bound omits the shared 100 MS/s, 500 ms cell.** If only that cell exceeds 140 ms, the rate line says “above” and the sweep line can say “at or below.” A sweep requesting only 500 ms produces no sweep bound line. The run correctly deduplicates the trial, but the documented sweep summary needs to include the reused result. The S4 block-length knob and both S9 knobs are forwarded by `hw()`. `s4bench.rs:528` (`crates/ezsdr-radio-uhd/tests/common/s4bench.rs:528`) · `s4bench.rs:676` (`crates/ezsdr-radio-uhd/tests/common/s4bench.rs:676`) · `bench.md:1426` (`plan/phase8/bench.md:1426`) · `bench.md:1511` (`plan/phase8/bench.md:1511`)

8. **Minor — Some coverage assertions can succeed without observing their named state.** VH-8’s `cut_in_gap` and `block_after_gap` counters come from predicted faults and blocks. Count an observed report with a cut and successor, and a published `RESTARTED` block. The large-block S4 rehearsal asserts a complete trial, which would also pass at the default block length. Separately, three late wall-clock attempts in the S9 rehearsal cause an ordinary panic before the verdict assertions. The logs record N108-131/132 as killed, but this path can also fail their test before it reaches the mutated verdict. The pure N108-125–128 checks exercise their predicates directly. `differential.rs:507` (`crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:507`) · `fake.rs:10521` (`crates/ezsdr-radio-uhd/tests/fake.rs:10521`) · `fake.rs:10171` (`crates/ezsdr-radio-uhd/tests/fake.rs:10171`) · `mutations.json:28584` (`plan/maintenance/tools/mutations.json:28584`) · `rows-4b3f2.log:19` (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/rows-4b3f2.log:19`)

9. **Minor — Mock back-pressure now retains a growing list for one pending event.** Leave a capacity-zero `Block` link full. Every refusal appends a report until the segment ends, although the observable result is one event. The report lengths are unused by the two emit paths. The pending cause and `undelivered` already suffice to identify and time the stretch. The old direct emit and the #108 exemption were removed. `mock-radio/src/lib.rs:584` (`crates/ezsdr-mock-radio/src/lib.rs:584`) · `mock-radio/src/lib.rs:686` (`crates/ezsdr-mock-radio/src/lib.rs:686`) · `mock-radio/src/lib.rs:1313` (`crates/ezsdr-mock-radio/src/lib.rs:1313`) · `report-4b1.md:22` (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/report-4b1.md:22`)

10. **Minor — KL-8 overstates the second-stop guarantee, and KL-7/KL-15 carry stale lead evidence.** KL-8 says the queued restart “never outlives the stream.” A non-loss failure of either stop can leave the chain `Uncertain`, and the code then finishes the segment. Qualify the guarantee by successful stop completion. The tail wording about a capture ending exactly at the tail stamp agrees with UR-18. KL-7 and KL-15 still say the 100 MS/s start lead is unmeasured, although the November 18 bench recorded the 10 ms rung clean on both boards. The overflow-gap row separately identifies the 140 ms value as provisional and lists the outstanding cells. `KNOWN_LIMITATIONS.md:68` (`KNOWN_LIMITATIONS.md:68`) · `KNOWN_LIMITATIONS.md:74` (`KNOWN_LIMITATIONS.md:74`) · `KNOWN_LIMITATIONS.md:227` (`KNOWN_LIMITATIONS.md:227`) · `core.rs:211` (`crates/ezsdr-radio-uhd/src/provider/core.rs:211`) · `UR-18` (`design/18-uhd-radio.md:245`) · `November 18 bench` (`bench/2026-11-18-mt3/README.md:7`)

**Verdict: FAIL.**
## recheck-solfix

## Re-check: stage 4b, the fix round after Sol's code review (`74104080..ec404e26`)

Reviewer: Opus re-check, read-only. Nothing built or run. The test status comes from `final-test.log`, `rows-solfix*.log` and `final-*clippy.log` in this folder.

### 0. My own failure scenarios (written before reading sol-code.md, decisions.md and the report)

(a) One-packet receive after an overrun report
- a1. The packet decision is a no-op while no cut is pending. At the default `block_len` 2 000 a call is already about one packet. At 65 536 a one-packet call still allocates and zeroes `n` samples per channel (`uhd.rs:1013`). That is about 512 KB for one packet of about 2 000 samples, which costs something only while the report stays pending.
- a2. The report stays pending while the packets returned are dropped. `rx_before_origin` and a fully overlapping packet (`rx.rs:601`, `rx.rs:637`) both clear nothing. Calls then stay one-packet. At 200 MS/s with 65 536-sample blocks the per-call cost could exceed a packet's 10 µs. This needs a stream that carries a pending overrun and receives packets before its origin. That happens only to a successor charged with the previous stream's report, and UHD hands that report over after its backlog, so this is unlikely.
- a3. The lattice and block boundaries do not change. The first block after the restart is about 2 000 samples instead of `block_len`. Blocks already vary in length at a cut, so this is not new.
- a4. The VH-8 rig's scripted device ignores `one_packet` (`differential.rs:961`). The rig cannot see the decision, and only the unit tests pin it.
- a5. The call bound: one-packet receive in UHD is a single `_recv_one_packet` with the timeout (INFERRED from UHD 4.x `rx_streamer_impl.hpp`, cited in Sol's P1). So the call ends by its wait, which is at most `span` after a call begun before `e`. The successor's origin is `e + span + lead`, so the start keeps its full lead.

(b) The second stop under `device_gate`
- b1. Deadlock. `try_end` holds `rx_chain[k]` and then takes `device_gate`. A deadlock needs a gate holder that takes `rx_chain`. The production gate holders are `device_lost`, which takes `streams` and calls `mark_lost` and `emit`; `check_rates`; `open_device_locked`; and `close_device`. None takes `rx_chain`. Only the `#[cfg(test)]` `assert_rx_chain_reusable` does, and it releases it before the gate. I found no cycle with the TX async path or uhd-control.
- b2. While the second `rx_stop` is in UHD, a loss publication waits for it. On a dead link that could be UHD's control timeout. `open` and `close` already do the same, so the behaviour is consistent.
- b3. The first stop (`rx.rs:293`) and the restart `rx_start` after `LATE_COMMAND` (`rx.rs:560`) are still admitted only by an `is_lost()` read outside the gate. Section 2 covers this.

(c) S9
- c1. A successor whose first returned row is before its origin: `receive_sample_follows_start` skips it, so the first row at or after the origin is the first published block. Agrees.
- c2. The tail's expected loss uses `lattice_index` (round). The Provider's `emit_tail` uses `at_or_after` (ceil). The two agree only because `end` lies on the old lattice (`planned_end` or `clock.instant(expected)`). Note.
- c3. `applied_cut` and the old clock's end both come from the same `end` in `try_end` and `finish_locked` (`rx.rs:288–304`, `rx.rs:353–354`). The check therefore cannot see a cut placed at the wrong sample. It only catches a clock that was not ended (`end == None`). Section 2 covers this.
- c4. A stop pair that UHD refused (a non-null `result` on the probe row) still counts as the physical stop pair for cell A. The applied row is then missing, and that only skips the cut check. Section 2 covers this.
- c5. A missing `rx_stop` row with complete rows and a successor start is a Module defect, but it reads INCONCLUSIVE. That is not a false PASS, because the cell's target count stays 0. Note.

(d) S4
- d1. The event's `time` and `source` must parse from the Manifest's own serialization. The fake end-to-end test `s4_overflow_trial_with_a_65536_block_reaches_a_complete_trial_on_the_fake` passes, so the real shape is accepted.
- d2. The stream name is hard-coded as `usrp/rx/s0`. That is the bench profile's id, so this is fine.

(e) Mock `PendingReport`: `rx_overflow_payload` only tests membership (`ezsdr-radio/src/lib.rs:648`), and the minimum first sample is kept. The change preserves behaviour.

(f) The KL texts are checked against `bench/2026-11-18-mt3/README.md` in section 3.

### 1. Sol's findings

| # | Verdict | Why |
|---|---|---|
| S1 (major) | **CLOSED** | `rx.rs:249`: while `Pending::Overrun` is pending, the call is one-packet. UHD gives one packet one timeout (INFERRED from source, as Sol's own P1), so the call ends by `span` after it began. A call begun at or before `e` ends by `e + span`, and the successor's origin is `e + span + lead`, so the start keeps its full lead. Sol's "packet near the end of the wait" is bounded by the same construction. The test's packet is at 72 ms, not near the end, but it does not need to be. Smallest wrong change (drop the `||` clause): `ur_17_an_overrun_silence_receive_returns_at_its_first_packet` fails at `rx_packet_modes == [false,false,true]`. Making every pending report one-packet: `ur_17_only_an_overrun_report…` fails in the "sequence" case. |
| S2 (major) | **CLOSED** for the second stop | `core.rs:1544` takes the gate, re-checks `is_lost()`, then calls. No lock cycle (b1). Smallest wrong changes: no gate fails `ur_29_a_loss_publication_waits…` at `assert_eq!(event, "waiting")` (the event is "done"). No re-check under the gate fails `ur_29_a_loss_published_just_before…` at `seen("rx_stop") == 1`. The same window still exists at the first stop (section 2, N2). |
| S3 (major) | **CLOSED** (INFERRED derivation) | `lattice_index` (`lib.rs:39`) is now shared by `rx.rs:615` and `s9_first_index`. Origin + 1 tick at n = 2 gives index 1, which FAILs cell A as "not on time". The successor row is the first `samples` row with `first_tick >= at` (`probe_device.rs:66–73`), which is the first block the Provider publishes unless one is dropped for `not_before` or overlap. The Manifest does not carry the published header, so this stays a derivation, as `bench.md` says. Tests: `s9_first_index_is_the_lattice_index…` and `s9_a_first_block_off_the_lattice_by_a_tick_fails_cell_a`. Floor division fails the first assertion. |
| S4 (major) | **CLOSED** for the wrong segment, **partly** for the wrong cut | The old clock A is now the clock with the smallest origin, chosen independently of the event. A tail on B, or off A's first undelivered sample, is a named fault, so Sol's sequence now FAILs. Sol's second half ("a wrong cut and a correspondingly shortened tail on A also pass … compare its loss with A's booked cut") is not closed in substance: the comparison is with the Provider's own `applied` row, not the booking (N3). |
| S5 (major) | **CLOSED** | Cell A now needs the last `rx_stop` row after the pre-stamp, and before the successor's start, to end before `result.before + 50 ms − 2 ms`. The resumed check reads up to `max(post, stop_end)`. A late stop pair is INCONCLUSIVE (`NEAR_RESTART`), not a cascade FAIL. Tests: the boundary at 11 099 and 11 100, the missing or early or unstamped stop, the resumed row before the stop end, and the successor's own rows. One leftover: the third assertion of `s9_a_stop_pair_that_ends_after_the_restart…` repeats its first (`s9bench.rs:909`). |
| S6 (major) | **CLOSED** (one clause unguarded) | `overflow_event_agrees` (`measurement.rs:2616`) requires the gap's domain, a parseable event time in that domain at the gap's first sample, a clock of `usrp/rx/s0`, and the event source equal to the clock's stream. Wrong domain, wrong tick, missing or bad time, and wrong stream each fail the test. `source_agrees` alone has no failing case and no row (N4). |
| S7 (minor) | CLOSED | The sweep line covers every cell at the sweep rate. The 80 → 141 expectation in the old test follows from that. |
| S8 (minor) | CLOSED | `observed_shapes` reads the delivered overrun events and the published `RESTARTED` blocks. Its pure test covers each clause. The report's coverage counts (13 / 5 / 10) are unchanged. The 65 536 rehearsal asserts the request length. The S9 rehearsal's exhaustion panic carries `EZSDR-INFRA:`. |
| S9 (minor) | CLOSED | `Option<PendingReport>` preserves behaviour (e). `mr_37_…` pins the merge, and F43, F44, N108-153 to 155 kill the mutants. |
| S10 (minor) | CLOSED | See section 3. |

### 2. New findings

- **N1, minor (a2): the one-packet call has an unbounded case.** The condition is `pending.contains(Overrun)` until a block is published. Packets the Provider drops do not clear it: `rx_before_origin` (`rx.rs:601–607`) and a fully overlapping packet (`rx.rs:637–643`). Sequence: a successor is charged with a previous stream's overrun read at or after its origin (`Pending::Overrun`), then the receive returns a run of packets before the origin. Every call is then one-packet. With `block_len` 65 536, each call allocates and zeroes about 512 KB (`uhd.rs:1013`) for about 2 000 samples. At 200 MS/s that may not keep up and may cause a second overrun. INFERRED and unlikely, because UHD hands the report over after its backlog. Default `block_len` 2 000 is unaffected. Two options: cap it (clear the one-packet mode after the first returned `Samples`, published or not), or state the case in UR-17.
- **N2, minor (pre-existing, same mechanism as S2): the first stop and the `LATE_COMMAND` restart can still follow a published loss.** Sequence: `try_end` reads `!is_lost()` (`rx.rs:285`). The TX async path then publishes `DEVICE_LOST` and calls `mark_lost` (`core.rs:1074–1088`). uhd-rx then calls `device.rx_stop` (`rx.rs:293`): a UHD call after the loss, which UR-29 says must not happen ("stop using the device"). The same applies to `rx_start` at `rx.rs:560`, which is checked under `rx_chain` and not under the gate. This round did not introduce it. The fix gates the second stop and now names the rule in UR-17 ("so that no call follows a published loss") for that call only. Routing the first stop through the gate would make the pair uniform. On a loss under the gate it would make no call and write no `applied` row.
- **N3, minor: the cut check compares the Provider with itself.** `s9_tail_judgement`'s "old clock's end is not the applied stop's cut" (`s9bench.rs:166–168`) compares two values written from one variable: `applied.at = cut` (`rx.rs:300`) and `clocks.end(…, end)` (`rx.rs:354`), where `cut = end.unwrap_or(expected)`. It can only fail when the clock was not ended. Sequence: the Module places A's cut at a wrong sample (for example `end_cut` returns the chain's cut while a loss is pending). The applied row, the clock end and the tail's `lost` all move together, so S9 still PASSes. Sol asked for a comparison with the booked cut. The tool does not know the untimed `Stop`'s effective instant. Either read it from the Manifest's admitted `Stop` if it is recorded there, or keep the check (it does catch an unended clock) and say in `bench.md` that S9 does not verify where the cut is placed: the unit tests and RM-16 do. The implementer also decided "skip the cut check when there is no applied row". That is acceptable once N5 is fixed. Without N5, a refused stop pair skips this check and still classifies A.
- **N4, minor: `source_agrees` in `overflow_event_agrees` is unguarded** (`measurement.rs:2624`). No test case fails on it alone: in `other_stream` the clock's stream is also `other/rx/s0`, so `stream_agrees` fails first. A mutant `source_agrees = true` survives. The report says so. Add one case: a `usrp/rx/s0` clock with an event source of another node.
- **N5, minor (false PASS path): a refused stop pair is accepted as cell A's physical premise.** `s9_classify` takes the last `rx_stop` row's `host_after_ns` and ignores its `result`. Sequence: the bench's first or second `rx_stop` returns a non-lost error. The Provider records no `applied` row, emits `COMMAND_REJECTED` and leaves the chain `Uncertain` (KL-8), while the probe row still has stamps. Cell A then holds on stamps alone, the cut check is skipped (N3), and if the successor happens to start cleanly the trial PASSes without the stop pair having taken effect. Cell A should require `result` null on the pair's rows, and a missing `applied` `rx_stop` row should be INCONCLUSIVE.
- **N6, note (c2):** `s9_tail_judgement` derives the old end index with `lattice_index` (round), while `emit_tail` uses `at_or_after` (ceil). They agree only because the end lies on the lattice. Using `div_ceil`, or the clock's own rule, would remove the dependence.
- **N7, note (c5):** in cell A with complete rows and a successor `rx_start`, a missing `rx_stop` is a Module defect but reads INCONCLUSIVE (`S9_NO_STOP`). This is not a false PASS, because the decision line's target count stays 0. It could be a FAIL.
- **N8, note:** `PendingReport::causes()` builds a `Vec` only for `rx_overflow_payload` to reduce it to one cause. Storing the merged cause (overrun wins) would be shorter. This is maintainability only.
- **N9, note:** `t200_bare_uhd_script_judges_a_run_and_bounds_it` failed in `final-test.log`. It is a signal-trap self-check that fails with an `EZSDR-INFRA:` marker under load, passed 3 of 3 alone per the report, and this round did not touch it. It must still pass at the gate.

### 3. KL texts against `bench/2026-11-18-mt3/README.md` and the code

- **KL-7 Status:** "1, 10 and 100 MS/s, one channel … 100 trials a rung on an OBX and a UBX, the same-configuration restart on time from 4–5 ms and the restart with a retune from 7.5 ms at 10 and 100 MS/s, 10 ms clean in 4 of 4 conditions on both boards; 200 MS/s and two channels are not measured". This matches the README's S5 row: #1 restart-same 4 ms at 10 MS/s and 5 ms at 100 MS/s, restart-retune 7.5 ms at both rates; #2 the same rungs; 10 ms clean in 4 of 4. **Correct.** Note: KL-7's **Evidence** bullet still lists only `plan/phase7/bench-results.md`. The new session is cited inline in Status, so adding `bench/2026-11-18-mt3/` to Evidence would be tidier.
- **KL-15 `rx_start_lead_ns` row:** "100 MS/s one channel, 10 ms clean in 4 of 4 conditions on both boards (… restart-same from 5 ms, restart-retune from 7.5 ms, 100 trials a rung); 200 MS/s not measured: INFERRED". **Correct.** The README's conditions (1 GHz, slot A, gain 0, 30 dB loopback) are not restated, which matches the other rows. KL-15's Evidence bullet does not list the new session (same note as KL-7).
- **KL-8:** "does not outlive the stream when both stops complete (a failure of either stop that is not the device's loss leaves the receive state uncertain and the segment ends, without that guarantee)". This matches `try_end`: a first-stop `Err` makes no second call, `stop_result(Err)` goes to `device_failed`, and the chain ends `Uncertain`. A loss under the gate makes no second call. **Correct.**

### 4. Regressions and leftovers introduced by the round

- **No regression found.** The lock order is sound (b1). The Mock refactor preserves behaviour (e). The VH-8 coverage counts are unchanged.
- **Leftover:** the duplicate assertion in `s9_a_stop_pair_that_ends_after_the_restart_is_inconclusive_not_a_cascade_fail` (`s9bench.rs:909`).
- **Process:** `final-test.log` records the one unrelated `EZSDR-INFRA` failure (N9). The 95 rows are killed after the F44 fix. Clippy (workspace and UHD feature) and the hardware build exited 0.

**Verdict:** S1–S10 CLOSED, except S4's wrong-cut half (N3, minor). Nothing major was introduced. N2, N3 and N5 are the items worth acting on before the next X310 S9 run: N5 is a false-PASS path, and N3 together with N5 makes the cut check skippable.

## terra-recheck-solfix

1. **Major — S3 is NOT CLOSED: S9 still does not inspect the first published block or its flags.** In cell A, suppose the first raw packet rounds to index 0, but a defect publishes that block with `GAP_BEFORE` or `SEQ_DISCONTINUITY` and no corresponding sequence event. S9 derives index 0 from the probe’s raw packet and can report PASS. The adopted fix required the published header and both flags. The shared lattice calculation closes the original off-lattice example, but does not establish the published result. The same distinction matters when a raw packet is trimmed or discarded. `s9bench.rs:495` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:495`) · `probe_device.rs:66` (`crates/ezsdr-radio-uhd/tests/common/probe_device.rs:66`) · `rx.rs:653` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:653`)

2. **Major — S4 is NOT CLOSED: missing applied-stop evidence can produce PASS.** If the old clock ends early, its tail agrees with that shortened end, and the applied `rx_stop` row is absent, the cut comparison is skipped. Provided the other evidence is present, S9 can pass the shortened capture. The row’s absence should leave the cut unverified and the verdict INCONCLUSIVE; it cannot establish the adopted requirement that the end equals the applied cut. The independent clock selection and tail-domain checks otherwise close the original wrong-segment example. `s9bench.rs:151` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:151`) · `s9bench.rs:340` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:340`) · `s9bench.rs:540` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:540`)

3. **Major — S5 is NOT CLOSED: a failed stop is accepted as proof of the cell-A premise.** If the last physical `rx_stop` row ends before the deadline but its `result` records an error, the classifier still admits cell A. **INFERRED on X310:** the queued restart can then survive, and its effects can be reported as a cascade FAIL even though the required successful stop was never established. Require successful completion of the relevant stop pair, otherwise INCONCLUSIVE. Missing and late stop rows are now handled. `s9bench.rs:197` (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:197`) · `probe_device.rs:401` (`crates/ezsdr-radio-uhd/tests/common/probe_device.rs:401`)

4. **Minor — S6’s source check lacks the claimed independent test.** The implementation checks that the event source equals the clock’s stream, but the new wrong-stream case changes both the clock and event source. Removing `source_agrees` alone therefore leaves that case rejected by the stream check. Add a case changing only the event source. The clock, timestamp and first-lost-sample checks otherwise close S6. `measurement.rs:2616` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:2616`) · `measurement.rs:4267` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:4267`)

5. **S1 CLOSED, with a hardware coverage limit.** One-packet mode is enabled only while an overrun report is pending and clears when a block carries it; the first returned packet can form a partial block, after which ordinary multi-packet reads resume. The existing lattice and cut handling still apply. **INFERRED:** UHD’s per-packet timeout makes this bound the silence call. The VH-8 scripted device ignores the mode, and neither it nor this read-only review establishes post-overflow throughput at 200 MS/s. `rx.rs:236` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:236`) · `rx.rs:3277` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:3277`)

6. **S2 CLOSED.** The second stop and loss publication are ordered by the same gate, with the loss rechecked under it. A loss discovered by the call is published after the gate is released; the inspected control and TX paths did not show a reverse lock order that creates a cycle. `core.rs:1067` (`crates/ezsdr-radio-uhd/src/provider/core.rs:1067`) · `core.rs:1543` (`crates/ezsdr-radio-uhd/src/provider/core.rs:1543`)

7. **S7–S10:** S7 **CLOSED**: the shared cell participates in the sweep bound; the rate line covers a run with only that cell. S8 **CLOSED** for the adopted coverage and rehearsal changes, subject to the VH-8 mode limitation above. S9 **CLOSED**: pending causes and the earliest report now occupy bounded space. S10 **CLOSED**: KL-7 and KL-15 agree with the November 18 one-channel results, and KL-8 qualifies the stop guarantee. UHD restart behavior remains **INFERRED** here. `s4bench.rs:675` (`crates/ezsdr-radio-uhd/tests/common/s4bench.rs:675`) · `KNOWN_LIMITATIONS.md:68` (`KNOWN_LIMITATIONS.md:68`) · bench record (`bench/2026-11-18-mt3/README.md:7`)

8. **New-test assertion audit, by test:**
   - S1 silence test: reverting the mode fails the packet-mode assertion; the pure mode test fails its overrun case when that condition is removed.
   - S2 loss-during-stop test: removing the gate changes the expected `waiting` ordering; the pre-gate-loss test fails the stop-call count without the recheck.
   - S3 lattice test: replacing rounding with floor fails the `origin + 1` assertion; the cell-A test fails its expected verdict if that index is treated as zero. Neither tests published flags.
   - S4 tail test: changing the domain, first-undelivered tick, or end/cut comparison fails its respective fault assertion; the row-reader test fails its expected last-delivery index; the verdict test fails if a tail fault is ignored. None requires an applied-stop row.
   - S5 deadline, missing-stop, resumed-samples and successor-samples tests each fail their named classification assertion when its corresponding predicate is removed; none supplies a failed stop result.
   - S6 evidence test fails its domain, tick and timestamp assertions for the corresponding removed checks; the source-only omission survives as described in finding 4.
   - S7 shared-cell test fails its sweep-bound assertion if that cell is excluded.
   - S8 observed-shapes test fails its respective assertions if either observed predicate is forced true; the large-block rehearsal fails its request-size assertion if the requested size is not propagated.
   - S9 pending-report test fails its event assertions if the earliest sample or either cause is lost; it intentionally also passes on the old list implementation.

No builds, tests, mutations, formatting, or writes were run for this review. The test outcomes above are assertion-level analysis, not newly observed executions.

**Verdict: NOT CLOSED overall.**
## recheck-solfix2

## Re-check: stage 4b, Sol fix round 2 (`ec404e26..d8142906`)

Reviewer: Opus re-check, read-only. I built nothing and ran no tests. The test and row status comes from `report-4b-solfix2.md` and `rows-solfix2.log`, which shows 121/121 killed.

### 0. My own failure scenarios (written before reading the reviews and the report)

(a) The successor's first published block, read through `FirstBlockLink`
- a1. **Does the wrapper change timing or back-pressure? No.** `DataLink` has five methods and no default methods (`ezsdr-kernel/src/stream/link.rs:118-133`). `Recording` forwards all five. `Link` has two (`module_api.rs:1154-1159`), and both are forwarded. Every link the Assembly builds is a `HostLinkModule` (`tests/common/mod.rs:413-416`), so replacing each one with a wrapped `HostLinkModule` changes nothing. A publish adds one uncontended mutex lock and a BTreeMap entry on a map of one or two keys. At 100 MS/s with 2 000-sample blocks that is about 50 k per second, which is negligible. VERIFIED by reading.
- a2. **A successor packet dropped before its origin.** A packet with `first_tick < origin` or `< not_before` is dropped whole (`rx.rs:612`). With an on-time timed start, the next packet begins at the origin, so the first published block is index 0. A packet that straddles the origin exists only if the device was already streaming at the origin, which is the stale-restart cascade. Then the first published index is > 0, or `rx_before_origin` > 0, and both FAIL. That FAIL is correct.
- a3. **Could `not_before` (stop + 2 ms) exceed the origin and drop on-time samples (a false FAIL)?** The planner places the origin at or after the cut plus one call at the old rate, plus the 3 ms allowance, plus the 10 ms lead (`timeline.rs:505-519`; X3xx `rx_start_lead_ns` = 10 ms, `x3xx.rs:414`). The stop comes about one quiet span (block + 3 ms) after the cut. The margin is about 8 ms, so this needs a host stall of about 8 ms. Not a finding.
- a4. **Is the successor's clock the right one?** It is found by the `rx_start` row's `at` equal to the Manifest clock's origin on `usrp/rx/s0`. A LATE_COMMAND restart's `rx_start` has `at != origin`, but `nth(start_index)` is the begin's start. If the lookup misses, `first_block` is `None`, which gives INCONCLUSIVE, never PASS.
- a5. **The Kernel's `now` and the Provider's `now` read the same Authority, and it is monotonic.** `DeviceTime::extrapolate` uses `fetch_max` (`authority.rs:139-145`), and `Shared::now` reads the primary root (`coordinator/state.rs:273-275`). The action log's `time` is therefore on the SampleClocks' root, and `ticks_in(record.root)` succeeds.

(b) The cut against the booked Stop
- b1. **The lower bound holds by construction.** The Kernel logs `now` read at the start of `submit` (`pipeline.rs:953`, `:1052`) before it dispatches. uhd-control books the untimed Stop at its own later `now` (`control.rs:316`). The plan's cut rounds up to the lattice (`timeline.rs:154-157` `sample_at_or_after`). So in correct operation the end is at or after `booked_at`, and the check cannot raise a false FAIL.
- b2. **A late cut is not bounded except by the cell-A window.** `bench.md` says so, and I accept it.
- b3. `applied_cut == end` still compares two values the Module writes from one variable. The independent evidence is `booked_at`.

(c) A failed stop in the pair
- c1. The probe writes `result: null` for `Ok` (`probe_device.rs:406`), so an `Ok` pair is not misread. Every row of the pair is checked, not only the last one. A failed stop outside the pair window is ignored.

(d) One-packet mode
- d1. The mode is set in `Stream::report` on `Overrun` (`rx.rs:91`) and cleared on any `Samples` (`rx.rs:543`), so a dropped packet also clears it. A second overrun sets it again. A backlog packet that arrives after the report could clear the mode before UHD's restart, but UHD reports the overflow after its backlog, so INFERRED unreachable.

(e) The gate on the first stop, the second stop and the restart
- e1. **Lock order.** `try_end` and the restart hold `rx_chain[k]` and then take `device_gate`. The holders of `device_gate` are `device_lost_locked` (which takes `streams`, `mark_lost` and `emit`), `check_rates`, `open_device_locked`, `close_device` and `admitted`. None of them takes `rx_chain`: the test-only `assert_rx_chain_reusable` releases it before the gate (`core.rs:1477-1481`, `:1530-1533`). uhd-control takes no `rx_chain` (grep `rx_chain\[` over `control*.rs` and `tx*.rs`: no production hit). The TX order is `tx_report_gate` then `device_gate`, and nothing takes them in reverse. There is no cycle.
- e2. **Self-deadlock.** A `device_lost` called on the same thread inside a gated call would deadlock on the std `Mutex`. Production `Device` implementations do not call back into `Core`, and probes are no-ops outside `cfg(test)`. This is why the deleted test had to go.
- e3. A dead link: a TX-path loss publication waits for an in-progress `rx_stop` or `rx_start` until UHD's control timeout. `open` and `close` already behave the same way. Note.
- e4. **Calls that are still ungated.** These are `begin_result`'s `rx_start` (`rx.rs:439`), the failure-path stop (`rx.rs:459`) and `mod.rs:661`. They are filed as #117 (open, `area:stop-escalation`).

(f) The Mock's `PendingReport { first, cause }`
- f1. `rx_overflow_payload` picks `Overrun` when present, otherwise `Sequence` (`ezsdr-radio/src/lib.rs:648-658`). "Overrun wins" in `note` is the same rule, and `first` still keeps the minimum. Behaviour is preserved.

### 1. Verdicts

| Item | Verdict | Evidence |
|---|---|---|
| **T1** (the published first block) | **CLOSED** | Cell A FAILs on index ≠ 0, `GAP_BEFORE` or `SEQ_DISCONTINUITY` (`s9bench.rs:433-447`). A sample seen with no recorded block is INCONCLUSIVE (`:459-461`). The fake rehearsal asserts `FirstBlock{0,false,false}` for both the same and the retuned successor (`fake.rs:10203`, `:10217`). The wrapper is behaviour-neutral (a1). |
| **T2** (an unverified cut → INCONCLUSIVE), **with N3** (against the booked Stop) | **CLOSED** | `cut_verified` needs `applied_cut == end` and `end >= booked_at` (`s9bench.rs:251`). A missing applied row or booked instant gives INCONCLUSIVE (`:465-467`). An end before the booked instant is a named fault, so FAIL (`:264-270`). `booked_at` comes from the Manifest's action log (`:683`), independent of the Module's own writes. It is a lower bound only (b1, b2). The fake rehearsal pins the extraction (`cut_verified` true in both plans). |
| **T3** (a failed stop is not the premise) | **CLOSED** | Every pair row's `result` is checked (`s9bench.rs:299-310`). The test covers failed then Ok, Ok then failed, a lone failure with no end stamp, and failures outside the pair. |
| **T4** (a source-only case) | **CLOSED** | `measurement.rs:4274-4277` changes only the event's source and keeps the `usrp/rx/s0` clock. Row N108-170 is killed. |
| **N1** (one packet ends the mode) | **CLOSED** | `rx.rs:91`, `:252`, `:543`. UR-17 now says "until a packet comes back … published or not". |
| **N2** (the first stop and the restart under the gate) | **CLOSED** for the three calls | `core.rs:1543-1558` adds `admitted`, used at `rx.rs:304`, `:309` and `:570`. A loss at the first gate makes no call and writes no `applied` row (`None => {}`, `rx.rs:320`). A loss at the second gate keeps the first stop's `Ok` (`.or(Some(Ok(())))`). A loss at the restart gate ends the stream (`try_end`). The residual ungated calls are tracked as #117. |

### 2. Each new or changed test under the smallest wrong change

- `ur_17_the_one_packet_receive_ends_after_one_packet_returned_published_or_not`: dropping `stream.one_packet = false;` (N108-156) leaves the modes `[false,true,true]` and fails the last assertion. Dropping `self.one_packet |= …` gives `[false,false]` and fails the second assertion. That second change has no row of its own; N108-133's use removal is the equivalent.
- `ur_29_a_loss_publication_waits_for_the_first_stop_in_the_device`: taking the gate out of `admitted` (N108-135) or bypassing the first stop (N108-157) makes the hook receive "done", and `assert_eq!(event, "waiting")` fails.
- `ur_29_a_loss_published_just_before_the_first_stop_s_gate_makes_no_call`: `Some(call())` (N108-136) makes the stop call, and `seen("rx_stop") == 0` fails. The bypass (N108-158) removes the probe, so the test fails earlier, at `core.is_lost()` (weaker, as the report says).
- `ur_29_a_loss_published_just_before_the_restart_start_s_gate_makes_no_call`: the bypass (N108-159) or `Some(call())` makes `seen("rx_start")` 2, and the first, named, assertion fails.
- `ur_29_a_loss_published_just_before_the_second_stop_s_gate_makes_no_call` (extended): N108-136 gives `seen("rx_stop") == 2`. Dropping `.or(Some(Ok(())))` gives an applied count of 0 and fails that assertion, but no row covers it (M1).
- `s9_a_published_first_block_must_be_on_time_and_gapless_in_cell_a`: N108-162, 163 and 164 each change the exact FAIL text.
- `s9_a_successor_block_the_link_never_saw_is_inconclusive_not_a_pass`: N108-165 turns INCONCLUSIVE into PASS.
- `s9_the_link_records_the_first_published_block_of_each_clock`: overwriting instead of `or_insert` (N108-166) makes domain 1 hold `{100, gap}`.
- `s9_successor_origin_is_the_booked_rx_starts_at`: dropping the pre-stamp filter (N108-171) returns `Some(100)` for index 0.
- `s9_a_failed_stop_of_the_pair_is_not_the_cell_a_premise`: no check (N108-160) fails every case. A check of the last row only (N108-161) fails "failed then Ok".
- `s9_tail_judgement_checks_the_cut_against_the_booked_stop`: no fault (N108-168) fails the end-before-booked case. Changing `>=` to `>` (N108-169) fails the equal case. Ignoring the applied cut (N108-172) and `true` (N108-173) fail the unverified cases.
- `s9_a_trial_with_an_unverified_cut_is_inconclusive_not_a_pass`: N108-167 turns INCONCLUSIVE into PASS.
- The source-only case in `overflow_evidence_requires_the_event_and_the_gap_to_agree`: N108-170 turns Incomplete into Success.

### 3. New findings

No major findings.

- **M1, minor:** there is no mutation row for "a loss at the second gate leaves the first stop's result as the pair's" (`rx.rs:309`, `.or(Some(Ok(())))`). UR-17 states this rule. The extended second-stop test would kill a mutation that drops it, because the `applied` `rx_stop` count drops from 1 to 0 (`rx.rs:2377-2378`), but no row does. The old `return Ok(())` branch had no row either, so this is not new. Fix: one row, `.or(Some(Ok(())))` → `` (nothing), filter `ur_29_a_loss_published_just_before_the_second_stop_s_gate_makes_no_call`.
- **Note:** the names of rows N108-135 and N108-136 still say "the second stop". Both now mutate the shared `admitted` helper (`core.rs:1544-1548`), which gates all three calls. Rename them.
- **Note:** `design/18-uhd-radio.md:444` cites `s9_first_index_is_the_lattice_index_the_provider_publishes` as evidence for "the successor's first block as published to the link". `s9_first_index` now derives only the old stream's first undelivered sample (`s9bench.rs:202-209`). The citation and the test's name are stale; move the test to the tail-evidence list.
- **Note:** at the restart, `self.core.is_lost() ||` (`rx.rs:570`) duplicates the check `admitted` makes under the gate. Only the `chain.device() == Lost` half still adds anything. It is harmless.
- **Note:** the restart's gate (as opposed to its re-check) has no concurrency test of its own. It shares `admitted` with the stops, whose gate N108-135 kills through the first and second stop's wait tests. Only a restart-specific change that bypasses the gate but keeps the probe and the check would survive, which is not a realistic mutation.
- **Note:** `rx_stop_admitted(k, before_gate: &'static str)` passes a test-probe name through a production signature. This matches how probes are used elsewhere in `Core`, so it is maintainability only.

### 4. Regressions and leftovers

- **No regression found.** The lock order has no cycle (e1), and the Mock refactor preserves behaviour (f1).
- **The deleted test's coverage is accounted for.** `ur_29_a_loss_published_between_the_two_stops_makes_no_second_call` asserted one call, one applied row, the clock's end at 6 000, no clear row, one `DEVICE_LOST` and the tail (1 000, 5 000). The second-stop pre-gate test now asserts the clock end and the tail as well (`rx.rs:2374`, `:2381-2384`). The loss during the first stop's call is covered by the new wait test. The deleted test's "no clear row" is replaced by "the clear decision was made", because the loss now comes after the decision. Retiring N108-45 is correct.
- **No leftovers** beyond the notes above. The duplicate `11 100` assertion is removed.

**Verdict:** T1, T2 (with N3), T3, T4, N1 and N2 are all CLOSED. No major findings. One minor (M1, a missing row) and five notes.

## terra-recheck-solfix2

1. **T1 — CLOSED.** The test-only link records the first header published for each SampleClock, and cell A checks its index and both flags. A packet dropped before the successor’s origin is not mistaken for a published block; the next published header is checked. s9bench.rs:64 (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:64`), rx.rs:610 (`crates/ezsdr-radio-uhd/src/provider/rx.rs:610`).

2. **T2, including N3 — CLOSED for the adopted lower-bound check.** The end must equal the applied cut and be no earlier than the logged Stop arrival; missing either value leaves the cut unverified. This does not independently establish the exact Provider booking instant or detect every late cut. The ordering of the logged arrival and Provider’s later `now` is **INFERRED** from the control path. s9bench.rs:246 (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:246`), control.rs:327 (`crates/ezsdr-radio-uhd/src/provider/control.rs:327`).

3. **T3 — CLOSED.** An error in either observed stop row makes cell A inconclusive. s9bench.rs:299 (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:299`).

4. **T4 — CLOSED.** The new case changes only the event source and asserts an incomplete trial. measurement.rs:4275 (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:4275`).

5. **N1 — CLOSED.** Any returned `Samples` clears one-packet mode before publication or rejection; a later overrun arms it again. UHD’s one-packet timeout and restart behavior remain **INFERRED**, since UHD 4.10 source and hardware were unavailable. rx.rs:540 (`crates/ezsdr-radio-uhd/src/provider/rx.rs:540`).

6. **N2 — CLOSED for the three specified calls.** Each uses the same gate as loss publication and rechecks loss while holding it. I found no production lock-order cycle with the inspected TX and control paths. A synchronous test hook that publishes loss *inside* a gated device call would self-deadlock, which explains the deleted test; its cut, applied-row, event and tail assertions are retained in the pre-gate second-stop test. The other ungated calls named in the report remain a separately filed, pre-existing issue. core.rs:1067 (`crates/ezsdr-radio-uhd/src/provider/core.rs:1067`), core.rs:1544 (`crates/ezsdr-radio-uhd/src/provider/core.rs:1544`), rx.rs:2353 (`crates/ezsdr-radio-uhd/src/provider/rx.rs:2353`).

7. **Note — hardware measurement effect.** The recording wrapper takes a mutex and does a map lookup on *every* published block before forwarding to the unchanged host link. It preserves the link policy, but its effect on X310 scheduling, back-pressure and loss has not been measured. The wrapper unit test establishes queue order, not timing equivalence. s9bench.rs:64 (`crates/ezsdr-radio-uhd/tests/common/s9bench.rs:64`).

8. **Note — Mock and maintainability.** `PendingReport` retains the earliest lost sample and gives overrun precedence, matching the payload reducer’s behavior; I found no behavioral regression from the simplification. lib.rs:160 (`crates/ezsdr-mock-radio/src/lib.rs:160`).

9. **Assertion-level assessment of the new tests**—these are static analyses, not executions:

   - `s9_successor_origin…`: removing the pre-stamp filter fails the earlier-start `None` assertion.
   - `s9_a_first_block_off_the_lattice…`: flooring the lattice index fails the index-1 assertion.
   - `s9_a_published_first_block…`: removing each index or flag check fails its respective exact-verdict assertion.
   - `s9_a_successor_block_the_link_never_saw…`: allowing a missing header to pass fails the INCONCLUSIVE assertion.
   - `s9_the_link_records…`: overwriting the map entry fails the first-block map assertion.
   - `s9_a_failed_stop…`: ignoring a failed row, or checking only the last, fails the failed-first assertion.
   - `s9_tail_judgement_checks…`: removing either required cut value or changing the lower-bound comparison fails its corresponding `TailJudgement` assertion.
   - `s9_a_trial_with_an_unverified_cut…`: ignoring `cut_verified` fails the INCONCLUSIVE assertion.
   - The S4 source-only case fails its incomplete-trial assertion if `source_agrees` is removed.
   - `ur_17_the_one_packet…`: removing the clear fails the final packet-mode assertion.
   - `ur_29_a_loss_publication_waits_for_the_first_stop…`: bypassing the gate fails the `"waiting"` ordering assertion.
   - The first-stop, second-stop and restart pre-gate tests fail their call-count assertions if the **under-gate loss recheck** is bypassed while retaining the hook. A complete bypass also removes the hook; the report correctly identifies that as a weaker mutation proof, since an earlier assertion can fail because the hook did not fire.
   - The updated fake rehearsal asserts the first header and verified cut for both retune cases; it supplies integration coverage for their collection.

No builds, tests, mutations, formatting, or writes were performed in this review. The reported passing runs were not independently reproduced.

**Verdict: CLOSED overall**, with the X310 timing effect and exact booked-cut limitation noted above.