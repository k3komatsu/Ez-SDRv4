# #75 — an unconfirmed transmit recovery flush (T3) still ends recovery: investigation and options

Tree: `/home/komatsu/local_work/Ez-SDRv4-e3`, branch `work/n99` = main `a9123cb`. Read-only: nothing edited, formatted or built in the tree; no probe copy was needed (reading only). FPGA/UHD sources were read from UHD `v4.10.0.0` into `/tmp/n75-src` (deleted at the end). Every claim is **VERIFIED** (read in code, spec, a log or the source named) or **INFERRED**.

## 0. Summary

- The issue's premise holds on the tree. T3 has three non-lost "unconfirmed" outcomes (no ACK in the 100 ms window, a short send, a failed send without loss). All three return `Ok(TxFlushResult { acknowledged: false })`; `reopen_tx_streamer` and `reopen_streamer` turn that into `Reopen::Reopened { front: None }`, `recovery` becomes `Clean`, and the held burst is sent next (§1).
- **The rule is written down as intended**: `design/18-uhd-radio.md:259` says "A missing acknowledgement does not itself stop the stream". The fix reverses a normative sentence, so it is a change to a rule the user reads.
- What the hardware tells us (§2): an ACK proves the EOB passed the DUC and the radio. Only a send that put no packet out *proves* the opposite. A whole send with no ACK is ambiguous, and nothing on the host tells the two apart. A fresh reopen plus T3 resolves the ambiguity either way, so **retry is the right answer to "no ACK" as well as to "short send"**.
- **Recommendation: option 2**: recovery ends only on a confirmed T3. The reopen retries (a fresh streamer and a fresh T3, at most 3 attempts in all) inside the one Reopen operation. After the last attempt it is a failed reopen, which the book and the executor already handle (`Reopen::Failed`: the stream's clock ends, held bursts are cancelled with `COMMAND_REJECTED`). Diff: about 25 production lines in `core.rs`, none in `staging.rs`; UR-22's sentence, a KNOWN_LIMITATIONS entry and the tests.
- Owner decisions (§3.7): (1) the visible end state after the retries (the stream ends for the Run: recommended); (2) that a whole-send/no-ACK T3 counts like a failed one, so a healthy device whose report is lost can lose its transmit stream; (3) whether the open-time flush at `arm` follows the same rule (a Connect refusal).
- The fix is risky (§4). Bench: nothing has provoked a missing recovery ACK, and none of the ~240 flush ACKs I could read at supported rates were missing (§2.3). An optional probe is described in §2.4.

## 1. The current path on the tree

### 1.1 Code

- Entry: `Tx::pass` sees `book.device.needs_reopen()` (`provider/tx.rs:211-213`) and calls `reopen_streamer` (`tx.rs:794-854`): it claims the stream, `issue_reopen()` (`staging.rs:851`), calls `core.reopen_tx_streamer` with `streams` released (`tx.rs:812`), then `complete_reopen(id, outcome)` (`tx.rs:819-833`).
- `reopen_tx_streamer` (`core.rs:1315-1332`) holds the report gate, closes the old streamer, opens a new one through `open_tx_device_locked` (`core.rs:1334-1344`) and checks the rates. `open_tx_device_locked` sends T3 (`tx_flush_locked`, `core.rs:1221-1303`) and returns `Err` only for loss (`core.rs:1337-1339`) or `Provider::stop` (`core.rs:1340-1342`). Everything else is `Ok(flush)`, `acknowledged` included.
- `tx_flush_locked`: one untimed 4-zero SOB+EOB `tx_send` with a send timeout inside one 100 ms device-host-clock deadline (`core.rs:1221-1243`). It then reads `tx_async` until every channel position has a `BurstAck` or the deadline passes (`core.rs:1245-1292`). A problem is only recorded as `rejected { call: "tx_flush" }` (`core.rs:1294-1301`). `acknowledged = acknowledged == expected`, `ack_tick` = the largest ACK tick (`core.rs:1302`).
- `reopen_streamer` builds `flush_front` only `if flush.acknowledged` and an ACK tick (`tx.rs:814-817`), but `outcome = Reopen::Reopened { front: flush_front }` whenever `result.is_ok()` (`tx.rs:821-824`). `complete_reopen` then advances the front when there is one and calls `device.reopen_succeeded()`, which sets `Recovery::Clean` (`staging.rs:859-868`, `staging.rs:675-677`). The next `pass()` dispatches the held burst.

### 1.2 Outcome table (current tree)

| T3 outcome | Core result | Recovery after | Front | Held bookings | User sees |
|---|---|---|---|---|---|
| ACK for every channel in the window | `acknowledged`, `ack_tick` | `Clean` | advances to the flush's exclusive end plus `g` (`tx.rs:816`) | sent after T3 | nothing; the raw `async` row has `tx_flush: true` |
| No ACK in 100 ms (send whole) | `acknowledged: false` | `Clean` | **unchanged** (old front) | sent next | a `rejected` row `tx_flush` ("BurstAck missing on channel positions …"), no event |
| ACK on some channels only | same | `Clean` | unchanged | sent next | same row |
| Short send (`Ok(n<4)`) | `acknowledged: false`, still waits the 100 ms | `Clean` | unchanged | sent next | `rejected` row ("flush send accepted n of 4 samples; BurstAck missing …") |
| Failed send, not lost | `device_failed` calls `command_rejected` (`core.rs:1235`, `core.rs:1078-1084`), `acknowledged: false` | `Clean` | unchanged | sent next | **a `COMMAND_REJECTED` event** (kind `tx_flush`) and a `rejected` row. The run goes on and the burst is sent. |
| Loss in the send or a read | `stopped: true`, `Err(lost)` (`core.rs:1336-1339`) | `Reopen::Interrupted { lost: true }` → `device.lose()` | n/a | cancelled by the loss path | `DEVICE_LOST` |
| `Provider::stop` during T3 | `Err("UR-25: Provider::stop …")` | `Interrupted { lost: false }` → `provider_stop()` → `Clean` | n/a | shutdown path | nothing new |
| Reopen itself (open or rate check) fails, not lost | `Err` | `Reopen::Failed` | n/a | open booking ends, held cancelled, clock ends (`tx.rs:831-846`, `staging.rs:877-883`) | `COMMAND_REJECTED` "tx_reopen" with the error, one `COMMAND_REJECTED` per cancelled burst ("UR-22: cancelled because the TX streamer could not be reopened"), later bursts refused ("UR-25: its transmit clock ended before the burst could start", `tx.rs:298-301`) |

Rows 2 to 5 are the defect (VERIFIED by reading and by the tests below).

Two facts the table depends on:

- After `Failed` the stream has no clock for the rest of the Run. The only writes of `Tx::clock` are `Tx::new` (`mod.rs:663`, once per owner) and the two `= None` sites (`tx.rs:863`, `tx.rs:1042`). The tx owner is not recreated. VERIFIED by `grep "self.clock = "` and `grep "Tx::new("`. No KNOWN_LIMITATIONS entry covers this (grep `reopen` and `tx_reopen` in KNOWN_LIMITATIONS.md found nothing), so today's failed-reopen end state is undocumented to users.
- The same collapse exists at `arm`: `open_streamer` calls `open_tx_device(k, set).map(|_| ())` (`core.rs:1385`) and throws the flush result away, so an unconfirmed open-time flush is only a `rejected` row. KL-12's workaround text states it (`KNOWN_LIMITATIONS.md:123`: "If `tx_flush` is rejected for a missing acknowledgement, the stale-header reset is not confirmed"). Same mechanism, different path; not in #75's text. VERIFIED.

### 1.3 Tests that pin the current behaviour

- `ur_22_owner_keeps_clean_recovery_after_an_unacknowledged_t3` (`tx.rs:1583-1635`): for `DropBurstAck`, `ShortSend(0)`, `FailSend(0)`, and a partial-ACK two-channel case, asserts `Recovery::Clean`, front unchanged, generation replaced, one `tx_flush` rejected row, no flush ACK row, **no payload in the reopen turn but the next `pass()` sends it** (`tx.rs:1609-1614`). The next-pass assertion is the one a fix reverses.
- `e3b.rs:453` and the golden `e3b.rs:553-557`: "failed EOB, flush not acknowledged" ends with `recovery false … clock true` (`e3b.rs:556`); this line flips to a halted clock under options 1 to 4.
- Core flush level, which stay true under every option: `ur_22_t3_missing_ack_is_bounded_and_read_faults_keep_progressing` (`core.rs:1729`), `ur_22_t3_flush_send_faults_record_a_missing_ack` (`core.rs:1754`), `ur_22_t3_flush_requires_each_channel_ack_and_device_loss_stays_distinct` (`core.rs:1769`), `ur_22_t3_flush_reports_after_the_deadline_stay_the_runs` (`core.rs:1873`).
- The open-time sibling: `ur_28_ack_ticks_separate_a_lost_flush_ack_from_later_bursts` (`tx.rs:4408-4430`) runs `tx_flush` after `DropBurstAck` and expects the run to go on.
- Ordering tests that stay: `ur_22_underflow_close_failure_reopens_before_the_next_payload` (`tx.rs:1479-1580`, including `Failed` for `TxOpenFails(1)` and `RatesMismatch(0)`), `ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts` (`tx.rs:1645`). These two already pin everything the Failed outcome does, so options 1 to 4 reuse tested end states.
- Fake model caveat: `FakeFault::ShortSend(k)` takes half of the packet (`device.rs:1818`). For a 4-sample T3 UHD's `send` is one packet per channel, sent whole or not at all (`tx_streamer_impl.hpp:278-290`, `tx_streamer_impl.hpp:431-436`; VERIFIED single channel, INFERRED multi-channel). The faithful fault for T3 is `StalledSend(k)` (returns 0, `device.rs:1820`). `DropBurstAck` is permanent (`device.rs:1865`), so a "no ACK once, then ACK" sequence (any retry option) needs a one-shot ACK drop: a small new `FakeFault`.

## 2. What the hardware does

### 2.1 Source evidence (UHD `v4.10.0.0`, read in full)

1. **The ACK is made at the radio, downstream of the DUC.** `radio_tx_core.v:371-374`: only in `ST_TRANSMIT`, on `s_axis_tlast && s_axis_teob`, `new_error_code <= ERR_TX_EOB_ACK`. So an ACK for the flush means the EOB went through the DUC and out of the radio (VERIFIED). The DUC's burst state ends on its own EOB: `axi_rate_change.v:370`, `:383` (the pop only at EOB) and `:423-424` (`last_word_in_burst` returns `vt_state` to `VT_INIT`), and a header without a time clears the propagated time (`:411-419`). This is the issue's cited mechanism (`l2-sol.md` finding 4); VERIFIED that the lines say this, INFERRED that it explains a mistimed next burst.
2. **Cases where the EOB reached the DUC but no ACK comes** (INFERRED from the same file):
   - An underrun in the flush: `radio_tx_core.v:362-370`. Between packets the radio goes `IDLE`; mid-packet it goes to `ST_POLICY_WAIT` (`:399-409`), which drops the rest of the packet and its EOB with no ACK. The DUC has already passed the EOB. This is the low-rate bench case (§2.3).
   - The error FIFO is full: `radio_tx_core.v:436-438` "If the FIFO fills then new errors will be ignored". It is `axi_fifo_short`, built on `SRLC32E` (32 entries; `axi_fifo_short.v:10`, `:42`). An ACK behind about 32 queued errors is dropped silently. The EOB is consumed.
3. **Cases where no EOB reached the DUC:**
   - The host put no packet out. `tx_streamer_impl.hpp:278-290` and `:431-436`: a send of at most `spp` samples is one packet; `get_send_buffs` failing (flow-control timeout) returns 0 and nothing is sent. VERIFIED for one channel. For a multi-channel streamer a failure after the first channel's buffer was taken may leave the channels apart (INFERRED; the Module's T3 uses every channel of the streamer).
   - A non-lost UHD exception inside `send`: unknown how much left the host (INFERRED, no source read).
   - A packet lost between host and device (10 GbE or PCIe-e): the T3 packet is the last one on that streamer, so UHD's sequence-error detection (which needs a following packet) cannot tell. INFERRED, never seen.
4. **Consequence of a missing EOB:** the DUC keeps a burst open, so a later header time is not taken (`axi_rate_change.v:401-408` takes a time only in `VT_INIT`). INFERRED as mistimed or dropped payload, as in the issue. **Related VERIFIED bench fact:** a stale timed DUC header survives a streamer close and reopen, and an untimed burst with EOB clears it. `plan/phase8/bench-results.md:221` (P5 C, X300 #2 UBX, 2026-10-28: a 2-sample untimed SOB+EOB after the reopen restores the transition to `e + 25 µs`) against P5 A (a frequency applied about 19 ms early; `bench-results.md:180`, `:270`; KL-12 evidence, `KNOWN_LIMITATIONS.md:124`). So a skipped flush does leave a stale DUC state on the hardware in the Module's reopen scenario. This is stronger than the issue's "INFERRED"; what remains INFERRED is the next burst's mistiming.
5. **ACK latency and rate:** a timed burst's ACK comes 1 tick after its end, 126 of 126 (`bench-results.md:331`). At supported rates the 4-zero untimed flush finishes in microseconds, so the Module's 100 ms deadline is very loose. A late-but-alive ACK is therefore not the likely reason for a missing one.

### 2.2 Can the host tell "EOB did not reach the DUC" from "ACK report lost"?

Partly, and only in one direction.

| Host observation | What it proves |
|---|---|
| a flush `BurstAck` (any attempt, any channel position) | the EOB passed the DUC and the radio (VERIFIED, §2.1 item 1) |
| a flush-attributed `Underflow` or `TimeError` and no ACK | the packet reached the radio (so it passed the DUC); the radio dropped the EOB (INFERRED, §2.1 item 2). `core.rs:1267-1269` already records these. |
| `tx_send` returned 0 on every channel, or a non-lost error | nothing left the host (0: VERIFIED; error: unknown) |
| whole send, nothing back | **indistinguishable**: packet lost, error FIFO overflow, report lost on the host path, or a very slow device |

A second T3 on a fresh streamer is the discriminator, and it costs one open and 100 ms: if the cause was a lost report or a dropped EOB the second ACK arrives; if the device or the link is unhealthy it does not, twice. A late ACK of the first attempt cannot be mistaken for the second's: a reopen replaces the streamer and the report queue with it (`design/18-uhd-radio.md:329`, UR-28), so each attempt reads only its own generation's reports.

### 2.3 Bench evidence

- No recovery reopen was ever run on hardware: grep of `~/.cache/ezsdr-bench` for `tx_reopen`, `reopen_needed`, `ReopenNeeded` finds no row. The hardware tests reopen raw streamers (`hw_d9_p5_a_stale_timed_header_after_a_reopen`, `hardware.rs:986`) and never fail an EOB close.
- Open-time T3 at supported rates: 241 `tx_flush: true` rows in the bench logs, no `BurstAck missing` text in any log (`grep -rl "BurstAck missing" ~/.cache/ezsdr-bench` finds nothing). These are the Module's tests of 2026-11-09 to 12 on both X300s at 1 MS/s and up. A grep of what the logs print, not a table of every Run's manifest, so INFERRED for "never missing".
- Low rates: a 2-zero untimed flush was not acknowledged within 1 s at 0.25 MS/s and below (33 underflows), and was acknowledged at 0.4 MS/s (`bench-results.md:426`, `bench/2026-11-10-front-sweep`; X300 #1 and #2, 30 dB loopback, 3 trials a cell). Those rates are refused by KL-40 (`KNOWN_LIMITATIONS.md:165-168`), which is the one place a missing ACK is a measured fact, and is out of the Module's range. A flush rule that demands the ACK matches that floor.
- At 1 and 2 MS/s an untimed 2-zero burst underflows (8 to 9 and 4 underflows) and is a different shape from T3's four zeros (`bench-results.md:327-331`; four to 512 samples were clean). T3 uses four (`design/18-uhd-radio.md:255`, UR-22).

### 2.4 Could a bench probe provoke it?

- *EOB that never reaches the DUC*: yes, cheaply. In `plan/phase8/tools/uhd-repro/tx_repro.cpp` (a plain UHD probe, not the Module) add a case that leaves a timed burst A open, reopens the streamer, **skips** T3, sends a timed burst B, and compares B's arrival and ACK tick with a control that sends T3. Skipping it is the hardware-side equivalent of a short T3 that put nothing out. It settles the INFERRED consequence of §2.1 item 4 and tells us whether the stale state costs a burst or only a frequency. Transmits: the owner must confirm the 30 dB attenuators in the loopback first (handoff §2, AGENTS.md safety core). Needed trials: at least 10 per cell near any boundary (§9), here one boundary (next burst timed or not), at 1 and 200 MS/s.
- *Lost ACK report*: not provokable on demand. The error FIFO overflow would need over 32 queued errors with the ACK behind them (INFERRED), a rare tail; the probe above does not reproduce it. A cheap partial answer: send T3, delay `recv_async_msg` by 150 ms, and see the ACK arrive late, which shows a missed 100 ms deadline is not a lost report.
- The rehearsal rule of AGENTS.md §9 ("hardware tests and bench values") applies if a new `#[ignore]` hardware test carries an expectation: a FakeDevice rehearsal in the default suite books the same boundary.

## 3. Options

Common to every option: the book's Reopen outcomes (`Reopened`, `Failed`, `Interrupted`) are enough for options 1 to 3 (no change in `staging.rs`); the Mock has no T3, no reopen and no transmit fault (note 49 M5, `plan/phase8/49-time-and-booking-inventory.md:84`), so every option is a UHD-only rule and parity (VH-4) is untouched. MockRadio therefore never shows the new refusals; each option adds one UHD-only outcome to KNOWN_LIMITATIONS.

### 3.1 Option 1: fail at once

*Rule.* Recovery ends only on a T3 that was whole and fully acknowledged. Any unconfirmed T3 (short, failed, no ACK, partial ACK) returns `Err` from `open_tx_device_locked` → `Reopen::Failed`.

*User sees.* The stream's clock ends for the Run: `COMMAND_REJECTED` "tx_reopen" with the text, one per held and open burst ("UR-22: cancelled because the TX streamer could not be reopened"), later bursts refused ("UR-25: its transmit clock ended before the burst could start"). Default policy `mark_artifact`: the Run goes on (`design/07-radio-model.md:117`); a Spec that needs that stream's data is spoiled but still finishes.

*Short/failed vs missing ACK.* Same.

*Fit.* UR-22 line 259's "does not itself stop the stream" is reversed; UR-25's clock end and the Book's `Failed` are reused as they stand.

*Cost.* About 10 lines (an `unconfirmed → Err` in `open_tx_device_locked`, and closing the new streamer). Tests: the four existing unconfirmed cases flip to the `Failed` expectations; the e3b golden line flips.

*Makes impossible.* Nothing structural; but a healthy device whose single ACK report was lost loses its transmit stream for the Run at the first occurrence, with no second chance.

### 3.2 Option 2 (recommended): bounded retry in the reopen, then fail

*Rule.* Recovery ends only on a confirmed T3. `reopen_tx_streamer` makes up to `T3_ATTEMPTS` = 3 attempts, each a close, an open, a rate check and a T3 on a fresh streamer, under the one report gate and the one Reopen operation (one claim). An attempt is confirmed when every channel position has its flush `BurstAck`. The first confirmed attempt returns `Ok` with its `ack_tick`, so the front comes from it. When the last attempt is unconfirmed, the result is `Err` (`DeviceError::failed("UR-22: the recovery flush was not acknowledged after 3 attempts: <last problem>")`, new streamer closed), which `reopen_streamer` already maps to `Reopen::Failed`. Loss and `Provider::stop` interrupt as they do now: the stop and loss checks run inside every attempt (`core.rs:1246-1248`) and between attempts. Each unconfirmed attempt keeps its own `rejected { call: "tx_flush" }` row.

*User sees.* One unconfirmed attempt: nothing but a `rejected` row and a delay of at most about 100 ms per attempt (a held burst that gets late is judged by its own late policy: `drop` flags `TIME_ERROR`, as after any recovery). After the third: option 1's end state with the message above. A Run that finishes with a recovered stream shows two or three `tx_flush` rows.

*Short/failed vs missing ACK.* One rule: confirmed or not. For the *certain* case (send returned 0 or an error) the attempt need not wait for an ACK that cannot come; an attempt that put nothing out ends at once and retries, a refinement that costs a few lines and is not required.

*Fit.* UR-22: the sentence at line 259 changes to "recovery ends only on an acknowledged T3, tried at most three times; then the reopen fails (UR-25)"; line 258's "T3 completes" gets a defined meaning; the Changes table gets a row. The book is unchanged: the retry is inside one Reopen op, so `Recovery` stays two-valued and holds no attempt counter. That keeps the lifecycle fact in one owner (AGENTS.md §9 "State"). The front is exactly what UR-33 and the L2 front rule say, from the confirmed ACK.

*Cost.* About 25 production lines in `core.rs` (the loop and the close on failure), a one-shot ACK-drop fault in `FakeDevice`, a one-line simplification at `tx.rs:816` (an `Ok` flush is always acknowledged), UR-22 text, KL entry, tests (§4). About 150 test lines. One or two mutation rows (the attempt bound, the confirmed-only `Ok`).

*Makes impossible.* Nothing the book needs later. It fixes N=3 as a constant; a later per-profile number is a one-line change. A stream cannot recover after the third failure until the "restart a halted transmit stream" question (out of scope; today's Failed has the same property) is answered.

### 3.3 Option 3: split by evidence

*Rule.* A T3 that provably did not leave the host (send returned 0 on every channel, or failed without loss) goes through option 2's bounded retry and then fails. A whole send with no ACK (and no flush-attributed Underflow/TimeError) is **explicitly** accepted: recovery ends as today, one `rejected` row remains, and UR-22's sentence is rewritten to say so.

*User sees.* The certain failures look as in option 2. A missing ACK after a whole send looks as today: nothing but a row.

*Short/failed vs missing ACK.* Different, on the §2.2 evidence.

*Fit.* Keeps line 259's sentence for the missing-ACK case and adds the exception. Honest about the hardware's ambiguity.

*Cost.* Option 2's plus a classification (`TxFlushResult` grows a `delivered` fact), two test families, and two rows in the spec where option 2 has one. In the ambiguous case (the packet lost on the link, or the reports gone) the hazard of the issue stays and can only be observed when payload is already wrong. The owner's 2026-10-31 wording ("the missing-ACK policy is decided explicitly") is met, but by choosing the weaker of the two available behaviours when a retry costs 100 ms.

*Makes impossible.* Later strictness (option 2) would be a behaviour change for users who relied on the carried-on stream.

### 3.4 Option 4: option 2, and the Run ends after the last attempt

*Rule.* As option 2, but the final failure is a device fault (`device_lost`-like: the Provider reports `DEVICE_LOST` with the text "UR-22: the recovery flush was not acknowledged…") instead of a halted stream.

*User sees.* The Run ends under the default Policy (UR-16, UR-29); under a Policy mapping `DEVICE_LOST` to continue, the Run goes on with no working Provider.

*Fit.* Wrong category: the device answers other calls, so this is the same collapse in the other direction (a possibly-fine device is declared gone) and conflicts with UR-29's definition of loss as "the device is gone". `mech:failure-collapsed-into-value` again.

*Cost.* Option 2's plus a new loss path to test. *Makes impossible:* partial recovery (other streams of the radio would stop too).

### 3.5 Option 5: stay in recovery, refuse bursts, retry until `Stop`

*Rule.* An unconfirmed T3 leaves `Recovery::ReopenNeeded`; each owner turn retries (backed off) until confirmed. While pending, held and booked bursts wait behind it and are judged by their late policy.

*User sees.* No halt and no refusal at first; bursts become late and are dropped (`TIME_ERROR`) until a retry succeeds or `Stop`/`Provider::stop` ends it. A Run with a dead device shows only `rejected` rows and late drops, never a statement that the stream is dead.

*Fit.* Never ends, so the "failure" is never represented. The owner's wording "or the segment halts or is refused" does not match.

*Cost.* A retry counter or timer in the chain, an unbounded loop on possibly-dying hardware, a backoff test on the manual clock, a hung-Run diagnosis. *Rejected* (§6: "unbounded").

### 3.6 Rejected up front

- **Probation by the next payload** (send the held burst, read its ACK tick, call recovery failed if it is off): payload would already have been transmitted on the wrong timeline, the exact thing the owner decided against (2026-10-31).
- **Model the T3 in the Mock** to compare: the Mock has no transmit fault model (M5); a model that only exists to answer a UHD-only rule is "looser than the hardware" in exactly the way AGENTS.md §3 forbids, and adds a second implementation of the rule.

### 3.7 Recommendation and owner decisions

**Option 2**, because one rule ("recovery ends on a confirmed T3") needs no assumption about why the ACK is missing, resolves the ambiguity with the hardware itself, reuses the tested `Failed` end state, leaves the book unchanged, and the constant is cheap to move.

Owner decides (user-visible, AGENTS.md §9 "What the owner decides"):

1. **The end state after the third unconfirmed attempt**: stream ends for the Run with `COMMAND_REJECTED` (recommended; today's failed-reopen state, now written down in KNOWN_LIMITATIONS), or the Run ends (option 4), or the stream stays pending (option 5).
2. **That a whole-send/no-ACK T3 is as bad as a failed one** (options 1 and 2), against keeping today's behaviour for that case alone (option 3). The cost of option 2 is that a lost *report* on a healthy device, three times running, costs the transmit stream.
3. **The open-time flush at `arm`**: the same rule (an unconfirmed flush after three attempts refuses Connect/`arm` with a `UR-13`/`UR-22` text) or leave it as KL-12 documents it. The same mechanism and the same hazard (a stale header from the previous process, KL-12 evidence, `bench-results.md:221`), but a Connect refusal is a new user-visible failure; recommend yes, filed as a separate step if the owner wants #75 kept small.

The orchestrator decides (not user-visible): N=3, the retry living inside `reopen_tx_streamer` (not in the book), the mutation rows, the `FakeFault` shape, the early-out for a send that put nothing out, and the attempt's closing of its streamer.

## 4. Risk class and tests

**Risky** (AGENTS.md §9 step 1): it changes a lifecycle (the reopen's end), the boundary between the fake and the hardware (the fake's `ShortSend` takes half of a 4-sample packet, which UHD does not), and what recovery means across the book and the executor. Steps 0 to 3 are not needed for a design (the direction follows the owner's 2026-10-31 decision and this note); steps 1 to 3 (brief, test plan, Sol's plan review) are.

Sequences (S) and expected results, under the recommended option. All run on the manual clock with a registered participant that sleeps past the deadlines (the pattern of `tx.rs:1597-1599`).

| S | Sequence | Expected |
|---|---|---|
| S1 | failed close; reopen; T3 ACK on attempt 1 (existing `ur_22_a_failed_eob_close_recovers_with_flush_before_the_next_timed_burst`) | `Clean`, front from the ACK, one `tx_open`, payload after T3 |
| S2 | attempt 1 no ACK (one-shot drop), attempt 2 ACK | `Clean`; two `tx_open`s; the generation changed twice; one `tx_flush` row; front from attempt 2's tick; the held burst's send is after the second T3 and never before |
| S3 | attempt 1 `StalledSend` (returns 0), attempt 2 confirmed | as S2; the first attempt put no packet out |
| S4 | all three unconfirmed (`DropBurstAck`) | `Reopen::Failed`: clock `None`, held cancelled, "could not be reopened" rows and `COMMAND_REJECTED`, no payload sent, three `tx_flush` rows, the new streamer closed; a later burst refused with the UR-25 text; the front unchanged |
| S5 | all three failed sends (`FailSend`) | as S4 plus the `COMMAND_REJECTED` of each failed send's `device_failed` (decide whether that is wanted: three events, or one) |
| S6 | partial ACK on every attempt (two channels, one ACK) | as S4 (partial is unconfirmed) |
| S7 | `Provider::stop` or loss between or during an attempt (gated at the T3 read, as in `ur_22_stop_and_loss_during_owner_recovery_release_the_claim`) | `Interrupted`: claim returned, no `Failed`, no cancellation, no payload, no new device call after observed loss |
| S8 | a flush-attributed `Underflow` with no ACK | unconfirmed (strict: only the ACK counts), logged as the flush's |
| S9 | the stale ACK of an earlier attempt read in the next attempt | impossible by the generation change; assert the old generation's report queue is gone |
| S10 | a held burst whose start passes during the attempts | judged by its late policy at dispatch, not before |
| S11 | ordinary `Stop` with the close failing, then an unconfirmed T3 | `Stop` keeps recovery pending (UR-26), and the new rule applies to the reopen it triggers |
| S12 | the open-time flush at `arm` (if decision 3 is yes) | after three attempts `arm` refuses; the streamer is closed; the record says "no open" |

Existing tests that change: `ur_22_owner_keeps_clean_recovery_after_an_unacknowledged_t3` (rewritten as S2 to S6), the e3b golden line, the Provider-level expectations of the flush faults. Tests that stay: the four core flush tests of §1.3, and the whole reopen/ordering family.

Hardware-test rehearsal (§9): if a `hw_` test of the retry is written, a default-suite FakeDevice test books S2 and S4 on both sides of the boundary. Mutation rows (via `remote_mutate.py` only): the attempt bound (3 to 4 or 2), the confirmed-only `Ok`, and the closing of the failed attempt's streamer. A spec text row for UR-22 in `rules.py`'s checks if the sentence is a tested rule (its `*Test:*` line names S2, S4).

Other required changes: `design/18-uhd-radio.md` UR-22 (lines 258-259) and the Changes table; `KNOWN_LIMITATIONS.md` (a new entry: after a failed recovery reopen the transmit stream ends for the Run; no KL entry exists today); `plan/maintenance/failure-mechanisms.md` (the note #75's mechanism row, with this document as the design note the rule asked for); `handoff.md` one line.

## 5. Not verified

- The consequence of a missing EOB on the next payload (§2.1 item 4) is a source reading, not a bench result; §2.4 describes the cheap probe.
- Multi-channel partial sends (§2.1 item 3) and a failed UHD exception's partial effects: unread.
- The claim "no missing recovery ACK on hardware" is a grep of printed logs (§2.3), not of every Run's manifest.
- `FakeFault` shape for a one-shot ACK drop: not designed here.

## Owner's decisions (2026-11-13 ~09:45): all three as recommended
- Direction: option 2 — bounded retry inside one reopen (fresh streamer and fresh T3, 3 attempts); recovery ends only on a confirmed T3.
- (1) After the last unconfirmed attempt the transmit stream ends for the Run with `COMMAND_REJECTED`; receive and other streams go on.
- (2) A whole send with no ACK counts like a failed T3 (one rule; cost: a healthy device whose ACK report is lost three times running loses its transmit stream).
- (3) The open-time flush at `arm` follows the same rule: an unconfirmed flush refuses the Connect.
- (F2, owner 2026-11-13 ~10:10) One `COMMAND_REJECTED` only when the last attempt fails; each attempt is one row in the Provider's record (a recovered stream leaves no rejection).
- ((3) timing, owner 2026-11-13 ~10:15) Recovery retry, (1), (2) implemented now; (3) implemented in the same form but enabled only after the 2026-11-15 open-flush sweep (1, 2, 10 MS/s × one- and two-channel streamers, ≥ 10 trials each, open + T3 wall time logged); if the sweep shows a missed ACK, ask the owner again.
