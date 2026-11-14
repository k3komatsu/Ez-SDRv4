# Maintenance table 3 — Fable 5.2 design reviews

## Brief (direction)
# Fable review brief — maintenance table 3, direction A (AGENTS §9 step 0)

You review a design direction the owner has chosen, adversarially. Read-only: never edit, build in, or change git state in /home/komatsu/local_work/Ez-SDRv4 (another agent works there) or its worktrees. Write your review to /home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/fable-review.md. Begin the review with the exact model ID from your system prompt.

Inputs: draft.md (the Opus design draft, the evidence and alternatives A–I, the owner questions O1–O7), design-brief.md, decisions.md (the owner chose A on 2026-11-14) — all in /home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/. Primary sources: UHD 4.10 source in the docker image `ezsdr-v4-dev:uhd4.10` under /root/tmp/uhd-4.10.0.0/ (e.g. `docker run --rm ezsdr-v4-dev:uhd4.10 sed -n 280,440p /root/tmp/uhd-4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v`), the repo (design/18-uhd-radio.md UR-22/UR-28/UR-33, design/07-radio-model.md RM-11/RM-14/RM-15, KNOWN_LIMITATIONS.md KL-13/KL-16/KL-39, crates/ezsdr-radio-uhd/src/device.rs and provider/tx.rs, crates/ezsdr-mock-radio), the bench raw rows the draft cites (~/ezsdr-bench/2026-11-13/...). Process rules: AGENTS.md §3, §6, §9. Owner decision today: a transmit underflow abandons the burst (KL-13), the plan to resume after it is dropped.

Do:
1. Before reading the draft's conclusions, write your own failure scenarios for "a timed transmit burst reaches the X310 late" from the code and the FPGA source.
2. Attack the premise: is "the X310 drops the Module's late device burst whole" really VERIFIED for the Module's chunked multi-send shape at rates other than 1 MS/s (DUC packetisation, interpolation, E3's one-sample-per-tick drain, header handling of continuation sends)? What evidence would refute it, and what bench cell would settle it?
3. Attack A itself: sequences where A's UR-28 rule (retire every pending device burst with first instant ≤ t) names the wrong burst, misses one, double-reports with an underflow, or races the abandon; the cascade rule for bursts queued behind; the FakeDevice drain model vs the hardware (looser or stricter); interaction with KL-39, repeats, Stop, the open-flush (T3) and #75's recovery.
4. Propose better alternatives if any (simpler, fewer mechanisms, or stricter), with what they make impossible later. "Leave it as declared" is a valid conclusion if justified.
5. For each of O1–O7: agree / disagree with the draft's pick, with the reason.
Mark VERIFIED vs INFERRED; path:line or source line for every claim; a major finding states a minimal operation sequence and the wrong observable outcome. No length cap, but put a 10-line summary first. End with your token usage if you can see it.

## Review (direction)
claude-fable-5-2

# Maintenance table 3, direction A — adversarial review (AGENTS §9 step 0)

## Summary (10 lines)

1. The premise holds: with every packet timed, the X310 drops a late device burst whole. VERIFIED in the FPGA source for every DUC rate and in H1's raw rows at 1 MS/s (18/18 all-timed trials over two records); INFERRED for the Module's chunked shape, other rates and board #2.
2. The draft's reading of today's fake (a chunk after a late first chunk plays) matches the code; I read it, I did not run it. Alternative B (leave as declared) is therefore not defensible.
3. **Major M1:** "every abandon retires the correlation without an event" loses the event of a wholly dropped burst whenever uhd-tx's own "timeline expired" abandon runs before the first TimeError is read. Today that burst is named; A would regress it.
4. **Major M2:** A does not remove S5. When the device runs dry while a closing send is in flight, the host has already closed the burst, nothing abandons it, and A's rule calls it `late_at_device` ("nothing was transmitted") after `TX_UNDERFLOW`.
5. One change fixes both (A′): reports alone retire correlations. An `Underflow` at `t` retires silently by the same reach rule; an abandon never touches `unacked`. This is fewer mechanisms than A.
6. **Major M3:** A's fake drain is looser than the X310 at the cascade boundary: the device drains 2 clocks per DUC packet slower than real time (measured 1998 and 502 tick gaps), plus the DUC's between-burst latency. "Only stricter" (O7) is false there.
7. Minor: retirement is per channel and `late_reported` goes, so a two-channel stream needs a stated rule for one event per burst; one channel can be late alone.
8. Minor: the "rely on" list overclaims in three places (a burst joined behind an underflow gets no event; a played burst whose ACK was lost can be named late; an untimed T3 that follows a drain plays the dropped burst's last samples).
9. O1, O2, O4: agree with the aim, with M1/M2's change. O3, O5, O6: agree. O7: agree the code need not wait for the whole-burst rule, disagree for anything that pins the cascade boundary.
10. Bench cells that would settle the rest: Module-shaped chunked timed sends, 10/100/200 MS/s, board #2, the cascade boundary, a two-channel late start, and the held-tail underflow's report order.

Conventions: VERIFIED = I read the source line or the raw row. INFERRED = reasoned from those. UHD paths are under `/root/tmp/uhd-4.10.0.0/` in `ezsdr-v4-dev:uhd4.10`. Repo lines are HEAD `c40a5f5`. I did not build or run anything; every statement about the fake or the Module is from reading.

---

## 1. My own failure scenarios

I had read the draft before this step, so these are not blind. They are derived mechanically from the state machine in `fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v:299-410` and the DUC's header logic in `fpga/usrp3/lib/rfnoc/axi_rate_change.v:193-433`, one per state and input.

| # | State and input | What the device does | Source |
|---|---|---|---|
| R1 | IDLE, a timed packet whose time has passed | `ERR_TX_LATE_DATA`, then `ST_POLICY_WAIT` to that packet's last word, then IDLE. The next DUC packet is stamped first time + samples so far, and is reached at least 2 clocks later than the one before, so it is late again. This repeats to the EOB, which is dropped, so no ACK | `radio_tx_core.v:345-357, 399-410`; `axi_rate_change.v:402-427` |
| R2 | As R1, but an untimed packet reaches the head of the DUC's input | `has_time_clear` is set when that packet is valid at the input, which is ahead of the output by the DUC's pipeline. From the next output packet on, the output is untimed and plays at once, so the last samples before the untimed packet play too | `axi_rate_change.v:411-419` |
| R3 | TRANSMIT, no data, between packets | `ERR_TX_UNDERRUN`, straight to IDLE. The rest of the burst is stamped in the past, so R1 follows: TimeErrors, no ACK | `radio_tx_core.v:360-369` |
| R4 | TRANSMIT, no data, inside a packet | `ERR_TX_UNDERRUN`, `ST_POLICY_WAIT` for the rest of that packet, then R1 | `:368` |
| R5 | A burst B queued behind a drain | B's first packet is time-checked only when the drain ends. It is late if its start is at or before that instant | `:316-357` (checks only from IDLE) |
| R6 | `ST_POLICY_WAIT` or IDLE with no data (the host stops feeding a drain) | It waits. No report is made. Lateness grows | `:399-410` |
| R7 | The 32-entry error FIFO is full | The new report is ignored, whatever its code: a TimeError, an Underflow or an ACK | `:436-456` |
| R8 | Two channels | Each channel has its own radio core and DUC. Each checks its own first packet | `rfnoc_block_radio.v:614`; `rfnoc_block_duc.v:310-318` |
| R9 | A timed packet that arrives less than 2 clocks early | `time_now` needs `radio_time + 2 == timestamp` one clock before. Otherwise `time_past` fires | `:280-285, 337-343` |
| R10 | A timed packet that arrives early | It waits in `ST_TIME_CHECK` and blocks everything behind it, an untimed T3 flush included | `:337-357` |

R1, R3 and R5 are the draft's E5, P6 and cascade. R2, R6, R7 and R8 are where A's contract has holes (§3).

## 2. The premise

**Claim under attack:** "The X310 drops the Module's late device burst whole."

### 2.1 What I verified

- **The drain is never faster than real time.** VERIFIED.
  - `s_axis_tready` in `ST_POLICY_WAIT` is not gated by `radio_tx_stb` (`radio_tx_core.v:425-426`). So the bound rests on one word per clock and on the strobe.
  - On the X300 the strobe is constant: `assign tx_stb[0] = 1'b1` (`fpga/usrp3/top/x300/x300_core.v:743`). One clock is one sample at the master rate.
  - Each dropped DUC packet costs 2 clocks more (IDLE, then TIME_CHECK).
- **That overhead is measured, at two packet sizes.** VERIFIED from the raw rows of both records (`~/ezsdr-bench/2026-11-13/3f1f87f921ce-{114706189933052,122349474797197}/measurement.json.rows/hw_h1_late_multi_packet_burst.jsonl`).
  - All-timed cell: each of the 18 trials begins with a run of 37–39 report gaps of exactly 1998 ticks (1996 + 2), and no gap is shorter.
  - Timed-head cell: each of the 18 trials begins with a run of 32–33 gaps of exactly 502 ticks (500 + 2). The head send was 500 samples.
  - So the DUC's output packet length is the input packet's length in samples. This agrees with `axi_rate_change.v:236-243` (`payload_length_out` from the first header, raised by later ones because `MAXIMIZE_OUTPUT_PKT_LEN` defaults to 1, `:44`, and `rfnoc_block_duc.v:310-317` does not override it).
  - The draft marked the "+2 clocks" as INFERRED. The second packet size makes it VERIFIED at 1 MS/s.
- **Continuation headers.** VERIFIED.
  - `first_header` is set only in `RECV_INIT`, which is entered only after an EOB (`axi_rate_change.v:193-207, 253-262`). The time is latched there (`:402-409`). Later headers' times are ignored.
  - A later header without a time clears the output's time for the rest of the burst (`:411-419`). A later header with a time changes nothing.
  - UHD times every fragment of a timed send (`host/lib/include/uhdlib/transport/tx_streamer_impl.hpp:297-314`).
  - So several timed sends of one device burst give the DUC the same input as H1's single send.
- **H1's all-timed cell.** VERIFIED: 18 trials in R1 and R2, no ACK, no heard interval, in every trial. The first report came 1.27–1.30 ms, 5.3 ms and 10.3 ms after the requested start in R1, and 3.2–13.0 ms after it in R2.
  - Every trial discriminates, not only R1 trials 0–2. In each, the first report to the last spans 3 994 002 to 4 003 992 ticks, which is the burst's 4 000 000. An instant skip would have ended the reports after the lateness.

### 2.2 Verdict on the premise

- **Whole-burst drop for all-timed packets:** VERIFIED in source for every interpolation the X300 DUC allows, since nothing in R1 depends on the rate. VERIFIED on the bench at 1 MS/s on board #1 for one send of 20 000 samples.
- **The Module's chunked shape:** INFERRED. The DUC ignores continuation times, so the only difference is that the host paces the chunks. Slower feeding makes the drain wait (R6), which adds lateness.
- **Other rates and board #2:** INFERRED. The daughterboard is behind the radio core and cannot change this.
- **I found nothing that refutes it.**

### 2.3 Where "whole" is not exact

- **The draft misreads H1's second cell.** It says the 19 544–19 545 heard samples are "the 19 500-sample tail plus its edges".
  - VERIFIED from the rows, all 18 trials: the heard interval starts 87 459–87 664 ticks after the first report, while the head's drain would take 100 400. The last report is 75 300–89 356 ticks after the first. The heard interval is 44–45 samples longer than the tail.
  - My reading (INFERRED) is R2: the untimed packet reached the head of the DUC's input while the head's last samples were still in its pipeline, and those played. The constant offset fits a fixed pipeline depth of some tens of input samples.
  - Consequence: an untimed packet that follows a drain plays the last samples of the "dropped" burst before it. The Module's only untimed send is T3. See N3 in §3.
- **Per channel** (R8). One channel of a two-channel burst can be late while the other plays. See m1.

### 2.4 What would refute it, and the cells that settle it

Refuting evidence in any cell: a heard interval, or a BurstAck, after a late first packet of an all-timed burst.

| Cell | Shape | Settles |
|---|---|---|
| C1 | Raw UHD, timed sends of the Module's chunk size (SOB on the first, a time on each, EOB on the last), 1 ms late | The chunked shape |
| C2 | C1 at 10, 100 and 200 MS/s; board #2 | Rates and boards. At 200 MS/s check the gap is still packet + 2 |
| C3 | C1 with a first send of 32 samples | The small first packet (the DUC's output length starts at 32) |
| C4 | Cascade: B at A's nominal end + gap, the gap swept across A's lateness, ≥ 10 trials per cell near the boundary | The boundary, and M3's margin |
| C5 | Two channels, a late start | m1: whether one channel can play alone |
| C6 | Held-tail underflow under load: the report order per channel | M2: Underflow, then TimeError, no ACK |

## 3. Attacks on A

### M1 (major) — the abandon's silent retire loses the event of a wholly dropped burst

**A's text:** "Every abandon retires its device burst's correlation without an event" (draft §0 item 3).

**Code facts** (read):
- Each turn reads the reports first and then steps (`crates/ezsdr-radio-uhd/src/provider/tx.rs:219-221`).
- `step` abandons an open burst whose next sample is before `now`, with the note "the transmit timeline expired" (`tx.rs:399-403`). It does this before it sends anything.
- A chunk is W/4, capped at 20 000 samples (`crates/ezsdr-radio/src/segment.rs:40-45`): 2.5 ms at 1 MS/s, 1 ms at 20 MS/s, 0.1 ms at 200 MS/s.

**Sequence** (1 MS/s, one channel):
1. A finite burst or a repeat is booked at T. `step` judges its start on time (`tx.rs:393-397`).
2. The host stalls 5 ms inside the first `tx_send`. The 2 500-sample chunk reaches the device at T + 5 ms. The device starts dropping it (R1).
3. Next turn: `reports` finds nothing, because the first TimeError is still on its way. `step` sees `instant(next) = T + 2.5 ms < now` and abandons. Under A the correlation is retired here, silently.
4. The two-zero close goes out, timed at the cursor. It is late too.
5. The TimeErrors arrive. No pending burst is left, so they stay raw.

**Wrong outcome:** the device transmitted nothing of the burst, and no `TIME_ERROR` names it. The record shows `end: stop` and a note. The draft's promise "every burst the device dropped has its event" fails.

**Why it matters:**
- It happens whenever the stall exceeds one chunk. At 200 MS/s that is 0.1 ms.
- Today this burst is named: `abandon` leaves `unacked` alone (`tx.rs:877-911`), and the TimeError finds it as the unique candidate. A would regress it.
- **The fake cannot show it.** The fake queues the TimeError inside the late `tx_send` itself (`device.rs:759-764`), and I saw no transit delay before `tx_async` hands it out (`device.rs:1964-2010`, read in part). So on the fake step 3's `reports` already finds it, the burst is named, and a Session-level test of A passes. Only a `tx.rs` unit that scripts the order (send, step, then the report) pins this.

### M2 (major) — an underflowed burst the host has already closed is still called late

**A's text:** O4, "after an underflow or any other abandon, no `late_at_device` for the same burst", implemented by the abandon's retire.

**Code and source facts:**
- A device burst is closed on the host as soon as its closing send returns: the held last sample with EOB (`tx.rs:476`, `close_device_burst` at `:765-781`), or a last buffer that carries EOB itself. `DeviceEnd::Sent` closes the chain (`crates/ezsdr-radio/src/staging.rs:657-660`).
- `underflowed` is false for a closed chain (`staging.rs:589-594`), so an Underflow read after that abandons nothing.
- `step` abandons an expired timeline before it sends (`tx.rs:399-403` and `:419-423`). So if uhd-tx wakes after the next sample's instant, the abandon runs and the closing send does not. The closing send goes out late only inside a window: `step` passed its check, and the device ran dry before the packet arrived. That is a stall inside `tx_send`, or a wake within the send's transit time of the end.
- On the device this is R3: Underrun, then the closing packet is late, and its EOB is dropped. VERIFIED in source; P6 measured the same order after a starvation.

**Sequence** (the held tail; a last buffer with EOB is the same):
1. Burst P of 20 000 samples is booked at T. Its first 19 999 samples are handed and play.
2. uhd-tx wakes just before P's end. `step` finds the timeline not expired and calls `close_unjoined`. The host is descheduled inside that `tx_send` for 1 ms.
3. The device runs dry at the last sample: `Underflow(u)`, `u ≥ P.first`. The tail packet arrives late: `TimeError(t)`, `t ≥ u`. No ACK. The send returns, and the host's chain is closed.
4. Next turn, `Underflow(u)` is read: `TX_UNDERFLOW` is emitted; `underflowed(u)` is false; no abandon, so no retire.
5. `TimeError(t)` is read. P is pending and `P.first ≤ t`. A's rule retires it with `late_at_device`.

**Wrong outcome:** P transmitted 19 999 of 20 000 samples and gets `TIME_ERROR { late_at_device }`, which RM-11 defines as "nothing was transmitted" (`design/07-radio-model.md:134`). It also gets two events, against O4. This is the draft's S5, and here A leaves it as it is today.

**How often:** I cannot say. If uhd-tx wakes after the end instead, the expired-timeline abandon runs, A retires silently, and the outcome is right. So under A the same device behaviour gives a right or a wrong event depending on which host branch ran. Under A′ it does not depend on the branch.

**The same hole in another place:** an underflow report whose tick is before the open burst's first sample abandons nothing by rule (`tx.rs:1144-1147`, #93). The earlier burst it belongs to stays pending and is named late by its own trailing TimeErrors.

### The fix for M1 and M2 (A′)

Let the device's reports decide, in the order the device made them. Abandons never touch `unacked`.

- **UR-28, one reach rule for every report with a tick `t` on a channel:**
  - `BurstAck`: retires the pending device bursts whose first instant is before `t` (as today).
  - `TimeError`: retires those whose first instant is at or before `t`, with one `late_at_device` each (A's rule).
  - `Underflow`, `UnderflowInPacket`: retires those whose first instant is at or before `t`, with no event beyond `TX_UNDERFLOW`.
- **UR-22:** a TimeError that reaches the open device burst abandons it (A's item 3, first sentence). The second sentence goes.

Why this is right:
- Per channel, the device's reports are in the order it made them. An underflow always precedes the TimeErrors of its own burst's rest (R3, R4). So the underflowed burst is gone before its TimeErrors are read, whether or not the host has closed or abandoned it.
- A wholly late burst has no Underflow, so its first TimeError still finds it, whatever the host did in between (M1).
- It adds nothing to `abandon`, which is called from six places (`tx.rs:400, 422, 611, 655, 669, 1149`), and keeps one key, the first instant against the tick.

What it does not cover (declare, do not build): an Underflow report that the FIFO dropped (R7) or that has no tick, followed by TimeErrors. Sequence errors keep today's handling, since their tick's meaning is unmeasured (`tx.rs:1145`).

### M3 (major for the fake's claim) — A's drain is looser than the X310 at the cascade boundary

**A's text:** a dropped packet "occupies the radio for its samples" (draft §0 item 1); B "plays when k lies outside A's lateness" (§8); "only stricter" (O7).

**Evidence:**
- The device's drain takes samples + 2 clocks per DUC packet (§2.1, VERIFIED at 1 MS/s). A 20 000-sample burst drained in up to 4 003 992 ticks plus its last packet, not 4 000 000. That is 0.1 % at 1996-sample packets.
- The DUC's output packet is as long as the longest input packet so far, starting from the first (`axi_rate_change.v:236-243`). A first send of 32 samples (UR-22's minimum) gives 32-sample packets, and 6 % overhead, until the second input packet is consumed. INFERRED; cell C3.
- After any EOB the DUC waits for its output to finish before it takes the next burst (`axi_rate_change.v:262-266`, "this can be a huge bubble state"). The 2026-11-10 sweep measured up to about 10 µs after a closed short head. Whether the same latency follows a dropped burst is INFERRED, and I see no reason it would not.
- Today's fake arms its guard only after an accepted burst (`crates/ezsdr-radio-uhd/src/device.rs:728-730`), so after a wholly dropped burst it arms none.

**Sequence:** A is 10 ms long and 1 ms late. B is booked at A.end + 1 ms + 5 µs.
- A's fake: the drain ends at A.end + 1 ms. B plays.
- X310 (INFERRED): the drain ends at A.end + 1 ms + about 10 µs, plus the DUC's latency. B is dropped.

**Wrong outcome:** a test that expects B to sound passes on the fake and fails on the device. That is what UR-33 exists to prevent (`design/18-uhd-radio.md:361`).

**Fix (lean):**
- The fake ends a drain at its samples plus a declared pad, and arms the existing short guard `g` from there. Take the pad from C4; until then use something clearly generous (1 % of the drained length, at least `g`).
- UR-33 says the boundary is modelled only as "never earlier than the device's".
- No fake or hardware test places B closer to the boundary than the pad until C4 is measured.

Two more points for the fake's drain, from R6:
- A drain that the host stops feeding makes no Underflow on the device. The fake's `active && now > cursor` rule (`device.rs:745-748`) must not see a drain as active.
- A draining packet blocks an untimed flush behind it (R10). The fake should order them the same way.

### m1 (minor) — two channels: one event per burst, and one channel late alone

- Retirement is per channel (`pending.channels.remove`, `tx.rs:332`). A removes `late_reported`. Then a two-channel late burst gives two retirements. The draft does not say which one makes the event. State it: the first TimeError that retires any channel of a pending device burst makes its events, and the entry remembers that.
- R8: the two channels' first packets are separate CHDR packets. A burst late by about the skew between them can play on one channel and be dropped on the other. INFERRED; cell C5. Then "nothing was transmitted" is false for one channel. The abandon is still the right action. RM-11's sentence should say "on at least one channel" for a multi-channel stream, or the KL entry should.

### m2 (minor) — a played burst whose ACK was lost can be named late

- With `≤ t` retirement, any burst left pending is named by the next TimeError on its channel. Today's unique-candidate rule kept that case raw.
- A burst is left pending only if its ACK was lost: R7 during a flood, for a burst of about 50 µs or less that plays within about 60 µs of a drain's end (the FIFO delivered one report per 8 000–14 000 ticks in H1). It then needs a second late incident before the next ACK.
- UHD's host queue does not drop: it is a `boost::lockfree::queue` built with a capacity and without `fixed_sized` (`host/lib/include/uhdlib/rfnoc/tx_async_msg_queue.hpp:43`). INFERRED that it grows, not checked in Boost.
- This is too rare to build for. Put it in the "cannot rely on" list.

### m3 (minor) — the contract's "rely on" list overclaims

- "Every burst the device dropped has its event." False for a Kernel burst joined behind an underflowed one whose first buffer was handed before the underflow: it transmits nothing and gets only the stream's `TX_UNDERFLOW`. Either say "every burst dropped late", or name such bursts (those of the device burst that start after the underflow's tick). I recommend the wording.
- "A burst handed behind the late one" applies behind an underflowed burst too: its rest drains at real time (R3). The KL entry should cover both.
- "No late burst is ever partly transmitted." See N3 and m1.

### Notes

- **N1 — the front after a late abandon lies inside the drain.** The Module's front is the cursor + 2 + `g` (`tx.rs:899-906`). The device is busy until about the cursor + 2 + the lateness. Every held burst in between is handed on time and dropped, each with its own event. Closely spaced short bursts recover by themselves, because a drain skips the gaps between bursts at once (R5). This is acceptable. The draft's reason for rejecting G is wrong, though: the host does know the drain's end, to within the overhead above, from the first report's tick. G stays rejected on cost and on today's owner decision, not on ignorance.
- **N2 — the close after a late abandon is itself dropped,** so no ACK ever answers it (R1). `close_device_burst` returns `Sent` on the send alone (`tx.rs:780-781`), so nothing waits for one. Under A′ nothing dangles either. The same is true after an underflow today.
- **N3 — T3 during a drain.** If an abandon's close fails or comes up short, the burst is `Uncertain` and the streamer is reopened with an untimed T3 (`design/18-uhd-radio.md:261`). If the DUC still holds the dropped burst's last samples, R2 applies and they play untimed (some tens of samples in H1, §2.3). A T3 queued behind a long drain can also miss its 100 ms deadline (R10), which #75's three attempts absorb up to a few hundred ms. Both need a stall longer than a reopen takes. Whether a reopen empties the DUC is INFERRED (`design/18-uhd-radio.md:23`). Declare it; build nothing.
- **N4 — Stop and shutdown.** A Stop that closes a late burst before its TimeErrors are read leaves it pending. Under A′ the TimeError names it and abandons nothing, since it is closed. That is correct. `shutdown` reads reports for 100 ms (`tx.rs:1183-1198`), so events can follow the Stop's record. This is as today.
- **N5 — the failed first send.** `remember_burst` runs before `abandon` in both error arms (`tx.rs:654-669`). Under A′ the entry stays. If nothing reached the device it is retired silently by the next ACK, or named by a later TimeError (m2's class). An uncertain close clears it with the generation (`tx.rs:1059-1066`).
- **N6 — `≤` against `<`.** `≤` is right for a TimeError: `time_past` is `radio_time ≥ timestamp` (`radio_tx_core.v:285`), so a burst can be late at its own tick, and a burst queued behind a packet dropped at `t` with its start at `t` is late (R5). `<` stays right for an ACK (`tx.rs:321-322`).
- **N7 — KL-39 and repeats.** A late repeat ends and keeps its region up to the cursor plus five. This is the underflow path's existing answer, and I found no new sequence in the region rule beyond N1. I did not trace the switch path (a pending switch cancelled by the abandon, and its `switches` row); see §7.

## 4. Alternatives

- **A′ (recommended): A with reports as the only thing that retires** (§3, the fix for M1 and M2), the fake's padded drain (M3) and the wording changes (m1, m3).
  - It is simpler than A: one reach rule for three report kinds, no hook in the abandon paths, no dedup set.
  - It makes impossible later: the same as A (draft §7). It adds nothing.
- **B (leave as declared): rejected.** I agree with the draft. The fake plays the second chunk of a late burst (`device.rs:759-792`, read), in the Module's own shape.
- **J: drop the correlation altogether.** `unacked` exists only to put `target` into `late_at_device`: the ACK arm discards what it retires (`tx.rs:1135-1137`). The event could carry the report's tick and no burst, like `TX_UNDERFLOW`.
  - For: it removes `mech:correlation-by-partial-key` by construction, which is the question AGENTS §6 asks at three strikes.
  - Against: the user loses which burst died, and for the cascade that is not obvious from the time alone. `target` is a required member of the payload (RM-11).
  - Rejected, but it is the fallback if A′'s rule produces another correlation bug.
- **G (push the front past the drain): stays rejected,** with the corrected reason in N1.
- **D, E, F, H, I:** I agree with the draft's rejections. For D, note also that under BURST policy R3's rest would still be dropped, so it changes no observable for all-timed sends.

## 5. O1–O7

| # | Draft's pick | Verdict | Reason |
|---|---|---|---|
| O1 | `late_at_device` = nothing of this burst was transmitted; one per dropped burst, joined and behind included; `late_by_ns` from the device burst's start | **Agree, with two conditions** | The meaning only holds if an underflowed burst can never get it (M2 → A′). For a multi-channel stream state the one-event rule and the one-channel case (m1). `late_by_ns` from the device burst's start is right: the whole device burst is shifted by that much |
| O2 | The Module ends a device burst reported late, as after an underflow | **Agree** | It matches today's owner decision (an underflow abandons, no resume). It bounds the flood and S2. The abandon must not retire the correlation (M1) |
| O3 | A late `send_asap` repeat is flagged and ended, not re-sent | **Agree** | Re-sending is resuming, which the owner dropped today for underflows. One answer for both |
| O4 | After an underflow or an abandon, no `late_at_device` for the same burst | **Agree with the aim, disagree with the mechanism** | The abandon's retire misses the closed burst (M2) and silences the wholly late one (M1). The Underflow report must retire |
| O5 | No mark in `bursts`; raw reports in `async`; one `rejected` note per abandon | **Agree** | The event's `target` is the join key. A mark would change a Kernel type before the freeze for a rare event |
| O6 | A new KL entry plus lines in KL-13, KL-16, KL-39 | **Agree, add three lines** | The cascade also follows an underflow (m3); one channel can be late alone (m1, INFERRED); a count or a tick of TimeErrors is never a promise |
| O7 | Bench at the next session; the code need not wait, "A is only stricter" | **Partly disagree** | The whole-burst rule is source-verified and can go ahead. "Only stricter" is false at the cascade boundary (M3): no test may pin that boundary before C4. Add C1–C6 of §2.4 to the plan |

## 6. Corrections to the draft's evidence

- §1.2, discriminator: every all-timed trial discriminates, in both records (§2.1).
- §1.2, second cell: the tail is not all that played; the heard interval is 44–45 samples longer and starts before the head's drain would end (§2.3).
- E3: the "+2 clocks per packet" is VERIFIED at 1 MS/s by two packet sizes.
- §3 and §4 G: "the host can only bound the drain's end, not know it" is too weak a statement of what the first report gives (N1).

## 7. Not checked

- Nothing was built or run. The fake's and the Module's behaviour are from reading.
- The tests the draft lists in §8 (`tx.rs:3742` and after, `tests/fake.rs:2533, 3626, 7685`) were not read.
- `boost::lockfree::queue`'s growth was not checked in Boost.
- The repeat-switch path under a late abandon (the cancelled switch, the `switches` row, KL-41's refusal) was not traced.
- The fake's `tx_async` was read only to `device.rs:2010`.
- How often M2's window is hit was not measured; cell C6 would show it.
- MockRadio was not read; I took the draft's statement that it never emits `late_at_device`.
- Token usage: not visible to me in this session.

## Brief (addendum)
# Fable review brief — maintenance table 3, addendum: a repeat resumes (AGENTS §9 step 0)

Review adversarially, read-only (never edit, build in, or change git state in /home/komatsu/local_work/Ez-SDRv4 or its worktrees). Write /home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/fable-addendum-review.md, beginning with your exact model ID.

Inputs in /home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/: addendum.md (the Opus design under review), addendum-brief.md, decisions.md (the owner's decisions; the last entry "REPEAT RESUME" is the direction this designs), draft.md and your own earlier fable-review.md (direction A, A′, M1–M3, cells), test-plan.md (§9 placeholder, Q10). Repo at main (crates/ezsdr-radio-uhd/src/provider/tx.rs, device.rs; design/07-radio-model.md RM-11..RM-16, RM-37; design/18-uhd-radio.md UR-22/UR-28/UR-30/UR-33; design/16-easy-api.md EA-16; the Kernel's burst records SC-25/SC-28/SC-29 in design/05 or wherever they live; KNOWN_LIMITATIONS.md). UHD 4.10 source in docker image `ezsdr-v4-dev:uhd4.10` under /root/tmp/uhd-4.10.0.0/.

Do:
1. Before reading the addendum's conclusions, write your own failure scenarios for "an endless repeat resumes after a late drop or an underflow".
2. Attack the resume point R = first period boundary ≥ max(busy + m, now + device lead, device front) — the addendum's departure from the owner's literal "handed end + reported δ + W": can R be early (a second stall, a lost report, the DUC latency, multi-channel), absurdly late, or off the period lattice? Is m = 10 ms justified?
3. Attack the state change: the repeat's booking open with no device burst open; the four `!open.first` sites; Stop, switch, the next booking, KL-39 regions, the T3 open flush and #75 recovery across the gap; the cap ("a TimeError on the resumed burst ends it, an Underflow resumes again") — can it loop forever, end wrongly, or depend on report order?
4. Attack the user contract: `resumed_at`, one record with a gap (a Kernel change before the freeze) vs split records, switch refusals in the gap (EA-16), MockRadio parity (it never underflows — is that true, and is the parity claim honest?).
5. Propose simpler alternatives if any; say what each makes impossible later.
6. For the addendum's 11 owner decisions: agree / disagree with its pick and why.
VERIFIED vs INFERRED; path:line for every claim; a major gives a minimal operation sequence and the wrong observable outcome. A 10-line summary first.

## Review (addendum)
claude-fable-5-2

# Maintenance table 3, addendum "a repeat resumes" — adversarial review (AGENTS §9 step 0)

## Summary (10 lines)

1. The resume point holds up: `R` from the Module's own `busy` is never early for the shapes the Module sends, short of a transport delay above `m`, and the literal "reported δ" is undefined for an underflow. I agree with the departure, but it changes the owner's stated formula, so it goes back to the owner as one line.
2. **Major M1 (decision 5):** the recommended record puts `GAP_BEFORE`/`lost` on a transmit block. SC-16 and SC-10 forbid that, `block.rs:256-258` rejects it, and AGENTS §3 lists "a time jump inside a burst is `TX_DISCONTINUITY`" as settled. The amendment list names none of them. The split-record alternative needs no Kernel change; take it.
3. **Major M2:** a fifth site is missed. `tx.rs:644` opens the resumed device burst with the repeat's original first instant, so a report of the dropped burst read after the send at R is issued reaches the new burst and drops it again (rare at m = W, usual below it).
4. **Major M3:** the "resuming" state keeps one value (`next = R`) for two facts (handed cursor H, resume point R). When the cap fires in the gap, today's abandon would take R as the cursor: a footprint up to a period too long and a stray close at R.
5. **Major M4:** one cap does not bound a host that cannot keep up. Each part plays, underflows and resumes, without end: a chopped signal, and an event, a note and report rows per cycle. The owner decided for a "momentary stall"; this case needs the owner's word.
6. **Major M5:** the three switch cases are stated for R only. A missed R moves it to R′; a kept switch with `b < R′ + 32` is then refused by a branch meant to be unreachable and stays pending in the record, or reaches join code in an undefined state.
7. Minor: the unread-halt special case looks redundant (it is a Stop in the gap); `busy` does not survive a new owner (`tx.rs:158`); a missed R can repeat with one `send_asap` event each time; `lost = R − H` undercounts the silence by `H − from`; `m` should not be the profile's window value by name.
8. MockRadio: "never underflows" is VERIFIED (no underflow code in the crate; MR-15). The parity claim is honest once the KL line exists; RM-15 should not cite UR-22.
9. Decisions: agree with 1, 2, 3 (less the unread halt), 4 (rename), 8 (deduplicate), 9, 10, 11; disagree with 5; 6 and 7 with changes. One decision is missing: M4.
10. Nothing was built or run. A′ and the fake's drain are plans, not code at `c04fda1`; every claim that rests on them is marked.

Conventions: VERIFIED = I read the line. INFERRED = reasoned from those. Repo lines are the worktree `Ez-SDRv4-e3` at `c04fda1`. "Plan" marks a claim that rests on A′ or on `test-plan.md`, which are not in the tree: at `c04fda1` the unique-candidate rule is still at `tx.rs:345-353` and `:1152-1173`.

---

## 1. My own failure scenarios

These are not blind: I read the addendum before I wrote them. I derived them from the code paths a resume crosses (`tx.rs:360-468`, `:544-708`, `:877-911`, `:1121-1175`; `staging.rs:545-695`, `:737-1040`) and from my earlier R1–R10.

| # | Scenario | Where the addendum stands |
|---|---|---|
| F1 | A report of the dropped burst is read after the resumed burst's first send is issued | Missed for the chain's `first` (M2) |
| F2 | The second drop is found by the expired check while no device burst is open, and the cap's report comes after | Cursor not defined (M3) |
| F3 | The host cannot keep up at all: play, underflow, resume, for hours | Not bounded (M4) |
| F4 | R is missed while a switch waits a few periods ahead | Not defined (M5) |
| F5 | A second stall inside the drain, before the first report is read | Covered: `busy` (§2.2a) |
| F6 | The resumed SOB is handed up to W before R, while the old drain still runs | Covered: it queues behind the drain (R5, R10) and is checked at the drain's end; needs `R ≥ D` only |
| F7 | A Stop in the gap, then a booking after its bound | Covered, and simpler than stated (§3, n1) |
| F8 | An uncertain close, three T3 attempts, R long past | Covered as a missed R; the gap's event is lost with the generation (declared) |
| F9 | Two channels: the reports interleave, one channel's Underflow first | Safe if retirement stays per channel (plan, Q11); R15 should script both orders |
| F10 | A waveform of 10 s: one 1 ms stall costs up to 10 s | Declared as "up to one period"; the owner should see the number (§4) |
| F11 | A new owner (`Tx::new`) right after a drop | `busy` is lost (n2) |

## 2. The resume point

**Verdict: R is sound. I found no sequence that makes it early, other than a transport delay above `m`, which fails safe through the cap.**

- **Early by a second stall:** no. `busy ← max(busy, r, x) + n` takes `r` after the stalled call returns, so a stall before, inside or after `tx_send` only raises it. The device cannot drain faster than one sample per sample period (my earlier §2.1, VERIFIED in source and in H1's rows).
- **Early by a lost report:** no. No report sets R.
- **Early by the DUC:** only by `ε·S + b`, under 1 ms for the worst packet size (32-sample DUC packets, 6 %, over about 13 ms). A repeat's first send is a whole chunk (`tile_repeat`, `tx.rs:64-82`; `segment::extent`, `segment.rs:91-102`), so ε is 0.1 % there. INFERRED until C3, C4, C7.
- **Early by handing ahead:** no. The SOB for R is issued up to W before R (`plan_send` under `window_cut`, `tx.rs:576`). It waits behind the drain and is time-checked when the radio reaches it.
- **Multi-channel:** one `tx_send` carries every channel, so `busy` is common. One channel late alone (m1) is a declared edge.
- **The premise that a dropped EOB still resets the DUC**, so that R's time is latched: INFERRED from source. The DUC is upstream of the radio and sees the EOB whatever the radio drops (`axi_rate_change.v:193-207, 253-262`, my earlier §2.1). C7 is its first measurement.
- **Absurdly late:** only by the period. `repeat_max_samples` is `u32::MAX` on this Module (`profile.rs:21`), so the lattice step has no useful bound. See §4.
- **Off the lattice:** no, as long as a missed R never goes through `dispatch_start` (`tx.rs:975-988`). The addendum says so.
- **`m` = 10 ms:** justified as a generous value before C7, because an early R costs the repeat (the cap). The measured terms sum to under 1 ms plus τ. Do not take it from `radio.timing.tx_in_flight_ns` by name: the window and the drain margin are unrelated, and tuning the window would silently move the margin. Give it its own constant with the same value.

**The literal formula.** The addendum's case (a) needs a second stall longer than about `m − 4 ms`; with m = 10 ms its own numbers do not bite (T + 16 ms against a drain to T + 11 ms). Case (b)'s setting is strained: the error FIFO is not full 60 s into a quiet burst. The conclusion still holds for a simpler reason: an Underflow reports no lateness at all, and a TimeError's `late_by` counts from the device burst's first sample (UR-28), not from the dropped packet. So "reported δ" is undefined for one trigger and meaningless mid-burst for the other.

**Does it go back to the owner?** Yes, as one line. The owner's recorded decision names a formula, and this review supports replacing it. AGENTS §9 asks for that when a premise falls. It is not "internal".

## 3. The state change

### M2 (major) — the resumed device burst opens with the repeat's original first instant

**Code (VERIFIED):** `send` builds `DeviceSend { first: clock.instant(start), … }` with `start = open.booking.k` (`tx.rs:554, 644`). `DeviceChain::send` opens the burst with it (`staging.rs:635-646`), and `underflowed` compares report ticks against it (`staging.rs:589-594`). The addendum's four sites (`:555, :647, :740, :1017`) do not include `:644`. It moves `remember_burst` to R, not the chain's `first`.

**Sequence** (plan: A′ with Q7's predicate, "a report reaches the open device burst when its tick ≥ the chain's first"):
1. A repeat starts at `k`. Minutes later the host stalls inside a send for longer than a chunk.
2. Next pass: no report is read yet. `step` finds the timeline expired (`tx.rs:398`) and resumes: close, R computed.
3. The send at R is issued as soon as R is inside the window, up to W before R; with a short period that is a few passes after the drop. The chain is open again, with `first = instant(k)`.
4. The dropped burst's TimeErrors arrive, ticks `t < R`. The entry rule does not reach the new entry (first instant R). The chain rule does: `t ≥ instant(k)`.

**Wrong outcome:** the healthy resumed burst is dropped a second time: a second gap, a second event, and either another resume or the cap, depending on the report kind. This is #93 again. How often: with `m` = W the send at R is issued no earlier than `busy`, so the stale report has to arrive after the drain's end. That is the error FIFO's tail (about 2 ms in H1) or transit, and only when the lattice rounding adds less than that. It becomes the usual order once `m` is below W, which C7 sweeps. INFERRED.

**Fix:** the device burst's first instant is the instant of the SOB send's first sample (`next`), which equals `start` for every SOB today. One line, no new state. Test: after a resume, a report with a tick in `[k, R)` read after the send at R changes nothing.

### M3 (major) — the "resuming" state keeps R where the handed cursor was

**Code (VERIFIED):** `TxBook::cursor` is `open.next` (`staging.rs:737-739`), documented as "the first sample the device has nothing past". Its readers are the three abandon calls (`tx.rs:400, 422, 1149`). In the gap the addendum sets `open.next = R`.

**Sequence** (plan: the cap, and "who emits"):
1. A resumed part starts at R1. The host stalls inside its first send.
2. Next pass: the expired check resumes again. The state is "resuming" with `next = R2`. Nothing is open.
3. The TimeError of the R1 part arrives. It reaches the `resumed` entry: the cap. "The repeat ends by today's abandon."

**Wrong outcome:** today's abandon reads the cursor. With R2 as the cursor, the repeat's footprint is `[start, R2 + 5)` instead of `[start, H + 5)`: up to `m` plus a period too long, so a booking the user makes after the event is refused (KL-39, wider). If the abandon is also told a device burst is open, a two-zero EOB goes out timed at R2 with no SOB before it. INFERRED: the addendum does not say which cursor the cap takes, and R5 scripts the cap only after the send at R.

**Fix:** the state holds both values (H and R), and `cursor()` keeps returning H. `lost` needs H as well. Add R5b: the cap fires in the gap; assert the footprint at `H + 5` and no send.

### M5 (major) — a missed R is not run through the switch cases again

**Addendum:** a pending switch with `b ≥ R + 32` is kept. A missed R "moves to the next on-time lattice point".

**Sequence:** period 1 000 samples at 1 MS/s. A switch is pending at `b = R + 3 000` and its payload is held. Before the send at R, uhd-tx is not scheduled for 15 ms (a second stall), or the close was uncertain and the first T3 attempt runs to its 100 ms deadline (UR-22). R is missed, and R′ is past `b`.

**Wrong outcome, R′ > b (VERIFIED in today's code, given that state; not run):**
- The held payload starts before the open booking's next sample. `step` takes the `stale` branch (`tx.rs:408-415`): a `debug_assert` fails in a debug build; in a release build the switch gets `TIME_ERROR { refused }` and `COMMAND_REJECTED` ("UR-21: the burst starts before the open burst's next sample").
- `drop_piece` finds no chain at `b` (a pending switch is not a chain), so the record's switch stays pending: `advance` applies only a sent one (`staging.rs:1444-1449`; `drop_piece`, `:1500-1504`). Every later switch is refused as "a switch is pending" until a Stop (UR-38, `18-uhd-radio.md:310`), and the `switches` row never settles before the Manifest.

**Wrong outcome, R′ = b or just below (INFERRED, not run):**
- R′ = `b`: `join_open` (`tx.rs:1014-1022`), once it reads the chain, ends the open booking. The switch is then taken as a new first booking and judged by `dispatch_start` (`tx.rs:946-1012`), which can move it (`move_start`, `staging.rs:1022-1029`). RM-37 says nothing moves a boundary (`07-radio-model.md:359`). With the site not fixed, `continue_device_burst` panics on the real-time thread (`staging.rs:618-621`).
- `b − 32 < R′ < b`: the old waveform's first send has fewer than 32 samples before the cut.

**Fix:** apply the same rule at every move (cancel a pending switch with `b < R′ + 32`). Add R12b.

### What I attacked and found sound

- **Stop, `Provider::stop`, loss in the gap.** `stop_now → end_open(true)` sends nothing once `:740` reads the chain, and the record ends `stop`. `cancel_all` keeps the booking out of the cancelled set because it is not `first` (`staging.rs:899-906`): right.
- **The next booking.** The record refuses anything that meets the open repeat (`staging.rs:1105-1130`), so only a switch can wait. VERIFIED by reading; the addendum's statement holds.
- **KL-39.** No `Bookings::abandon` on a resume keeps the whole region, as for a playing repeat.
- **#75 recovery.** `complete_reopen`'s `Failed` arm ends the open booking whatever its state (`staging.rs:877-882`). The flush's 100 ms deadline is far above a drain of about 13 ms.
- **The cap's rule.** `ERR_TX_UNDERRUN` is raised at one place, inside TRANSMIT (`radio_tx_core.v:366`, VERIFIED by grep in the image). So an Underflow proves the part played, and a TimeError that reaches a part with no Underflow before it proves it did not. Per channel, report order is device order. The rule does not depend on which host branch ran first, given M2's and M3's fixes.
- **Two identities.** The trigger reads the chain (Q7) and the cap reads the entry. They can disagree only after a generation change clears the entries; the result is one extra resume, never a loop. Pin it in R3b.

### Notes

- **n1 — the unread halt.** A halt unread at the drop is read before anything is handed at R (`step` hands only with the list read, `tx.rs:368-381`), and `stop_now` then ends the booking as for a Stop in the gap. I see no sequence where the special row of §1 gives a different record, region or send. INFERRED: I did not trace `halt_played`. If none exists, drop the guard, R10's separate path and its mutation row.
- **n2 — `busy` "across owners".** `Tx::new` replaces the chain with `DeviceChain::default()` (`tx.rs:158-160`). A value kept there does not survive an owner. Say "within an owner", or keep it beside the chain.
- **n3 — a missed R can repeat.** Each move emits `TIME_ERROR { send_asap }`. Today a moved start reports once (`report_send_asap`, `tx.rs:931-936`). Keep one event per gap.

## 4. The user contract

### M1 (major) — the recommended record contradicts the Stream Contract

**Addendum:** "the block at R carries `GAP_BEFORE` with `lost`"; amendments SC-24, SC-25, SC-28; "a small Kernel change".

**VERIFIED:**
- SC-16: "a transmit block must not carry `GAP_BEFORE`" (`02-stream-contract.md:214`). SC-10 lets `lost` exist only with `GAP_BEFORE` (`:200`).
- `SampleBlock::new` rejects it: "A jump in transmit time is a discontinuity (SC-24), never a flagged gap" (`block.rs:256-258`). The test is `sc_16_direction_flags_rejected`.
- AGENTS §3 lists as settled: "a time jump inside a burst is `TX_DISCONTINUITY`".
- uhd-tx hands the tracker a bare `BlockHeader` (`tx.rs:684-706`), so no constructor would see the flag. The rule would be broken silently.

**Sequence:** a repeat resumes once. The Module builds a transmit header with `GAP_BEFORE` and `lost`. **Wrong outcome:** a header that SC-16 says cannot exist, in the one Provider that ships; or, built through the constructor, `InvalidBlock`.

**More against one record:**
- The event's `target` is the join key to `bursts` (O5). A resumed part's event has `target` = the previous R, which matches no record when the repeat is one record with target T.
- `samples` and `wraps` count periods that were handed and dropped as transmitted, and `lost = R − H` leaves out `[from, H)`, which can be as long as `lost` itself. "Each gap is counted in `lost`" overstates it.
- It adds a Kernel concept for one Provider's hardware, before the freeze (AGENTS §3, Kernel stays small).

**Take the alternative:** the block at R carries `START_OF_BURST` with its `BurstOpen`. The tracker already closes the open record as `discontinuity` and opens the next with `wraps` counted (`burst.rs:339-352`), and `Tx::record` already writes it (`tx.rs:716-731`). Each part is a record whose `target` is where it really started, a missed R included. There is no Kernel code and no `BurstRecord` schema change (the payload schemas still regenerate for `resumes_at`).
- SC-24a obliges the Provider to emit `TX_DISCONTINUITY` and to judge the new burst's lateness (`02-stream-contract.md:248`). The second half is the missed-R rule. For the first, a scope line: a resume's discontinuity is told by the drop's event. No payload type exists for the kind (`grep TxDiscontinuity crates/ezsdr-radio/src`: none), and RM-10 says the UHD Provider never emits it (`07-radio-model.md:119`).
- It reads the owner's words directly: "a gap in the burst (Stream Contract time jump)".
- It makes impossible later: one record per `start_repeat`. A reader counts parts.

### M4 (major, the owner's) — one cap does not bound a host that cannot keep up

**Sequence:** a repeat at a rate the host sustains only in part (CPU contention, 200 MS/s). The part at R is handed a lead ahead and plays. The device runs dry: Underflow. Resume. The part plays. Underflow. This never ends, because each part "played".

**Outcome:** the antenna radiates a chopped signal with gaps of at least `m`. Each cycle adds an event, a `rejected` note and its raw rows to `async`, all kept in memory until `cleanup` (`tx.rs:1125`; UR-30, `18-uhd-radio.md:352`). Today the first underflow ends it. The rate of the cycle is INFERRED; that nothing stops it is read from the design.

**Options for the owner:**
- (a) Declare it in KL-13 and KL-45 and build nothing.
- (b) A progress rule with no clock and no new number: a part that underflows before it has played as long as the gap before it ends the repeat. It is decided from report ticks, like the cap.
- (c) The addendum's β (N resumes in a window), which the owner already declined.

I recommend (b), or (a) with the plain sentence "a host that cannot keep up makes a repeat stutter without end".

### Smaller points

- **`resumed_at` is a plan, not a fact:** a missed R, a Stop or the cap can follow it. The statement "silent in `[from, resumed_at)`" stays true as a lower bound. Name it `resumes_at`, and let the next record's `target` be the fact (it is, under split records).
- **The gap's length.** With a 10 s waveform, a 1 ms stall silences up to 10 s. A resume at the next sample on the lattice would bound the gap by `m`, but then a part no longer starts at the waveform's first sample, which a framed waveform needs. The period boundary is the owner's choice and defensible; put the number in KL.
- **The refusal in a gap.** It reuses `started = false`, so the user reads (EA-16, `16-easy-api.md:102`; KL-41, `KNOWN_LIMITATIONS.md:169-171`) "has not started on the device" for a repeat that played for minutes. Give it its own reason, and tell the user to retry after `resumes_at`. Accepting it with the bound raised to `R + 32` is possible (R is in the book under the same lock), but a missed R would then cancel it; the refusal is the leaner answer.
- **A cancelled switch** (`H ≤ b < R + 32`) leaves the old waveform playing after the gap. RM-37's `cancelled` clause speaks only of a Provider that ends the repeat (`07-radio-model.md:360`); it needs this case, and so does the shared oracle.
- **MockRadio.** VERIFIED: no underflow code in `crates/ezsdr-mock-radio/src` (`grep -n -i underflow`: no hit); MR-15 (`09-mock-radio.md:142`); RM-10 (`07-radio-model.md:119`). The parity claim is honest with the KL line. RM-15 is the shared rule, so it should state the permission ("a Provider may pause a repeat and resume it on its lattice, telling the span") and let UR-22 give the mechanism, not cite UR-22 from RM (AGENTS §2).

## 5. Simpler alternatives

| Alternative | Saves | Makes impossible later |
|---|---|---|
| Split records through SC-24's discontinuity (M1) | every Kernel and schema change; the `lost` arithmetic | one record per repeat |
| The device burst's first instant is its SOB's (M2) | a class of stale-report cases | nothing |
| Drop the unread-halt row (n1), if no counter-sequence is found | a guard, a test path, a row | nothing |
| Cancel every pending switch not yet handed, as today's abandon does (UR-38, `18-uhd-radio.md:314`) | the `R + 32` case, M5, one row | a timed switch booked ahead surviving a stall. I would keep the three cases and fix M5 |
| Resume notes as rows in `timing`, beside the receive restarts (UR-30, `:351`) | prose with an instant inside; `rejected` holding non-rejections | nothing |
| Resume at the next lattice sample, not the period boundary | `repeat_boundary`'s `None` row; gaps of a period | every part starting at the waveform's first sample |

I agree with the addendum's rejections of α, β (but see M4), γ, δ′, ε and G. Its reason against ζ ("no discontinuity") does not stand against SC-24.

## 6. The 11 decisions

| # | Addendum's pick | Verdict | Reason |
|---|---|---|---|
| 0 | (listed as internal) R from `busy`, not the reported δ | **Agree; the owner's** | §2. The recorded formula changes |
| 1 | m = 10 ms now, measured after C4 and C7 | **Agree** | Generous is right while an early R costs the repeat. Its own constant, not the window's key |
| 2 | The expired timeline resumes too | **Agree** | Otherwise one stall ends or resumes by host branch. Needs M2 and M3 |
| 3 | What still ends a repeat | **Agree, less one row** | Sequence loss, failed or short sends, a failed reopen, R past the clock: yes. The unread halt is a Stop in the gap (n1) |
| 4 | Optional `resumed_at` on both payloads | **Agree, renamed** | The owner asked for the span. It is a plan: `resumes_at` |
| 5 | One record with a declared gap | **Disagree** | M1. Split records; no Kernel change |
| 6 | One `rejected` note per resume | **Agree with a change** | A row in `timing`; and unbounded until M4 is decided |
| 7 | Switch: applied, cancelled in `[H, R + 32)`, kept, refused in the gap | **Agree, with M5 and a reason of its own** | A moved R re-runs the rule; RM-37 and the oracle gain the case |
| 8 | A missed R moves on with `send_asap`, not the cap | **Agree** | One event per gap (n3) |
| 9 | The cap's edges end the repeat | **Agree** | Both are rare, and ending is today's answer |
| 10 | The KL texts and EA-16's phrase | **Agree, add three lines** | The stutter (M4), the gap of up to a period with its number, the refusal's retry advice |
| 11 | Bench cell C7 | **Agree, add two cells** | A host that cannot keep up (M4), and two channels. `Descheduled` wraps a `FakeDevice` by type (`tests/fake.rs:3466-3468`), so C7 needs it made generic |
| 12 | (missing) What bounds a stuttering repeat | **The owner's** | M4: (a), (b) or (c) |

Internal picks: `busy` in the chain needs n2; the entry marks are right for branch independence; "the four `!first` sites" are five (M2), plus the expired check (`tx.rs:398`) and the late-start check (`:393`), which must both know the state.

## 7. Corrections to the addendum's evidence

- §5's grep `underflow\|stall\|Descheduled` over `parity.rs` is not empty: it matches `install_probe` at `:3157` and `:3168`. Neither is a stall, so the conclusion stands.
- §2.4 decides a drain pad on the fake, `max(g, ⌈S/100⌉)`. `test-plan.md` §1.1 records the opposite decision ("No pad, no guard"), with no test nearer than 1 000 samples to the boundary. One of them has to change before Sol reads the plan; "drop `+ m`, killed by R1 through the pad" depends on it.
- §2.2 (a) and (b): see §2.
- §4 "there is no discontinuity": SC-24 defines exactly this jump as one.

## 8. Not checked

- Nothing was built or run. Every statement about the Module and the tracker is from reading `c04fda1`.
- A′, Q7, Q11 and the fake's drain exist only in `decisions.md` and `test-plan.md`. M2, M3 and the cap analysis rest on them.
- `device.rs` (the fake) was not re-read for this review; `test-plan.md` was read in §1, §7–§10 only.
- `Bookings::halt_at` and `halt_played` were not traced (n1), nor `record.switch` with `started = false`.
- KL-16 was not read; KL-39 and KL-41 were read in their first lines.
- UHD's host path from `tx_send`'s return to the radio (τ) was not read; it is the addendum's INFERRED and C7's to measure.
- The cycle rate in M4 and the outcomes in M5 are INFERRED.
- Token usage is not visible to me in this session.
