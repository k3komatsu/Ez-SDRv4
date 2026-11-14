# Maintenance table 3 — design (Opus drafts)

## Direction (draft.md)

# Maintenance table 3 — a burst the X310 finds late: FakeDevice, UR-28, UR-33 (AGENTS §9 step 0 draft)

Design drafter (Opus, effort max), 2026-11-13. Read-only: nothing in the repository changed. Line numbers are HEAD `c40a5f5` (`work/n51-3`). UHD paths are in the image `ezsdr-v4-dev:uhd4.10`, under `/root/tmp/uhd-4.10.0.0/`. Brief: `design-brief.md` next to this file.

## 0. Recommendation

**Row 3's premise is reversed for the shapes the Module actually sends.**

- **The X310 does drop per packet** (`TX_ERR_POLICY_PACKET`; UHD 4.10 never changes it). But the Module times every payload send (UR-22 T1), and the DUC carries the first header's time to the end of the burst. The radio also drains a dropped packet at one sample per tick. So once one timed packet is late, every later packet of its device burst is late too. **The X310 drops the Module's late device burst whole**, through its EOB, with no BurstAck. H1, all-timed cell: 18/18 trials over two records, nothing heard. H1's "untimed tail plays" comes from a shape the Module never sends (UR-22 allows no untimed payload).
- **The fake no longer drops a late burst whole.** That changed in D9b-1a (`80dd2f6`, 2026-10-30).
  - It checks each `tx_send` against its clock at the call and skips a late one instantly (`device.rs:759-765`). It does not move its cursor and applies no flow control (`:1942`).
  - The Module sends a burst in chunks of W/4 (`ezsdr-radio/src/segment.rs:40-45`: 2 500 samples at 1 MS/s). The chunk after a late first chunk carries a DUC time that is still ahead of the fake's clock, so it plays (`device.rs:766-792`), and the EOB earns a BurstAck.
  - **So the fake is looser than the X310 for the Module's shape.** This is read from the code, not run. The step's first test pins it (§8).

**Recommended (alternative A): treat the device burst as the unit of a late drop.**
1. **FakeDevice.** A late packet that the fake drops occupies the radio for its samples, starting when the radio reaches it (`max(now, cursor)`), as the X300's drain does. Every later packet of that DUC burst is then late as well. Each is dropped through the EOB, with one TimeError per channel per send and no BurstAck. A device burst handed behind it whose start falls inside the drain is late too. The report's tick is the instant the radio reaches the packet.
2. **UR-28.** A `TimeError(channel, t)` retires every pending device burst on that channel whose first instant is at or before `t`. This has the BurstAck rule's reach, with `≤` in place of `<`. Each Kernel burst of a retired device burst gets one `TIME_ERROR { late_at_device }`, with `late_by` = `t` − the device burst's first instant. The "exactly one candidate" rule and the `late_reported` set are removed.
3. **UR-22.** When that report retires the open device burst, the Module abandons it exactly as it abandons after an underflow. The predicate is the same (`ezsdr-radio/src/staging.rs:589-594`) and so is the path (`tx.rs:1147-1150`). Every abandon retires its device burst's correlation without an event.
4. **RM-11 and KL.** RM-11 keeps "nothing was transmitted" and now says it holds per burst. A new KL entry states the device behaviour and that MockRadio never shows it.

Nothing else changes: no FPGA policy, no mark in the burst record, no MockRadio change. There is **no new refusal rule, but two existing refusals gain a trigger**: KL-39's region, and the refused switch on an ended repeat (§3). That is the case `mech:refusal-where-late-policy-decides` describes (note 49 §11.2, C6 still open), so O2 and O3 are put to the owner with it. The step is **risky** (it changes the boundary between the fake and the hardware, and the identity a report is matched to), so it needs a test plan and a Sol review.

## 1. Evidence

### 1.1 UHD 4.10 source

| # | Claim | Status | Source |
|---|---|---|---|
| E1 | The RFNoC radio's TX error policy resets to PACKET. UHD's host never writes `REG_TX_ERROR_POLICY`, and the C API cannot: `set_user_register` throws on RFNoC devices | VERIFIED | `fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v:106,116`; `grep -rn REG_TX_ERROR_POLICY host/lib` finds only its definition, `host/lib/include/uhdlib/rfnoc/radio_control_impl.hpp:292` (the wider `ERROR_POLICY` also hits the non-RFNoC `tx_vita_core_3000.cpp`); `host/lib/usrp/multi_usrp_rfnoc.cpp:1119-1123` |
| E2 | Times are checked only from IDLE. A late timed packet raises `ERR_TX_LATE_DATA` and enters `ST_POLICY_WAIT`. Under PACKET policy the radio returns to IDLE at that packet's last word | VERIFIED | `radio_tx_core.v:299-357`, `:399-410` |
| E3 | `ST_POLICY_WAIT` takes at most one word per radio clock (`tready = state==ST_POLICY_WAIT`). An X300 word is one sample: NIPC defaults to 1 and the X300 does not override it. A dropped packet therefore costs at least its length in ticks | VERIFIED; the "+2 clocks per packet" overhead is INFERRED (my reading of the state machine, which fits H1's 1 998-tick spacing) | `radio_tx_core.v:425-426`; `host/include/uhd/rfnoc/blocks/radio.yml:14` (only E310/X4xx set `RADIO_NIPC`) |
| E4 | UHD times every fragment of a timed `send`. The DUC output carries its first header's time until the EOB and ignores later headers | VERIFIED | `host/lib/include/uhdlib/transport/tx_streamer_impl.hpp:310-317`; `fpga/usrp3/lib/rfnoc/axi_rate_change.v:396-433` (cited in `design/18-uhd-radio.md:23`) |
| E5 | Consequence: a timed run whose first packet is late stays late to its EOB. Lateness never shrinks (E3), and the EOB is dropped, so no ACK comes | INFERRED from E2–E4; VERIFIED at 1 MS/s by H1 (§1.2) |
| E6 | Every TX error report carries the radio time at the error. The error FIFO holds 32 entries and ignores new errors when full | VERIFIED | `radio_tx_core.v:436-438, 456, 491, 509` |
| E7 | The Module times every payload send | VERIFIED (code) | `design/18-uhd-radio.md:257` (UR-22); `crates/ezsdr-radio-uhd/src/provider/tx.rs:628-630` |

### 1.2 Bench H1 (raw rows, not the summary)

- **Source.** The observation `hw_h1_late_multi_packet_burst` in `~/ezsdr-bench/2026-11-13/3f1f87f921ce-{114706189933052,122349474797197}/measurement.json` and its `measurement.json.rows/hw_h1_late_multi_packet_burst.jsonl`, on usrp-lnx03; release `2026-11-13-n67-records` (`bench/2026-11-13-n67-records/raw.txt`). Test: `crates/ezsdr-radio-uhd/tests/hardware.rs:1701-1768`.
- **Conditions.**
  - Raw UHD, not the Module.
  - Board #1 only (X300 + OBX), 1 MS/s, a 20 000-sample burst.
  - Nominal lateness of 1, 5 and 10 ms × 3 trials, measured from one time read before each trial (INFERRED).
  - Two records: R1 unloaded, R2 under the `h4` load.
- **All-timed cell** (one timed SOB/EOB send, which UHD splits into timed 1 996-sample packets):
  - 18/18 trials: nothing heard, no BurstAck. TimeError counts were 365–368 in R1 and 313–454 in R2.
  - In R1 the first report came +1.27 to +10.33 ms after the requested start.
  - From the first report to the last: 3 994 002 to 4 003 992 ticks, which is the burst's 4 000 000. The drop ran at real time.
- **The discriminator.** In R1 trials 0–2 the burst was about 1.3 ms late. The second UHD packet starts 1 996 samples (2.0 ms) in. The 1 996 is not in H1's rows: it is `benchmark_rate`'s TX spp with the same 8 000-byte frames (`plan/phase8/bench-results.md:153-157`, at 200 MS/s), and INFERRED at 1 MS/s, since spp follows the frame size. Had the first packet been skipped instantly, which is the fake's rule, the second would have been 0.7 ms early and would have played. Nothing was heard.
- **Report spacing.**
  - In every trial the first ~37 gaps are exactly 1 998 ticks (measured). The reading — one report per drained ~1 996-sample DUC packet plus 2 clocks (E3) — is INFERRED.
  - Later gaps are whole multiples of that: mostly 6× in R1, 4–7× in R2.
  - So about 2 000 packets were dropped per burst, and the count is not a packet count. Most reports were lost, INFERRED to the 32-entry FIFO (E6). **No rule may pin a count.**
- **Timed 500-sample head + untimed 5 000-sample sends:** 18/18 trials.
  - 36–40 TimeErrors over the head, then one BurstAck.
  - 19 544–19 545 samples heard: the 19 500-sample tail plus its edges. They start just after the last report, so the tail played late by the lateness.
- **P6 (2026-10-28/29, both boards).** After a starvation, the timed continuations are dropped with TimeErrors (`plan/phase8/bench-results.md:223, 270, 300`). This is E5 after an underflow.
- **Not measured:**
  - the Module's own multi-send shape (INFERRED to be the same: the DUC ignores continuation headers, E4);
  - other rates, and board #2 (UBX);
  - a burst handed behind a late one (the cascade).

### 1.3 Code today (read, not run)

- **Fake.**
  - The late check runs only for the packet in hand. It changes neither the cursor nor the burst state (`device.rs:759-765`); only the short-guard refusal sets `Burst::Timed` (`:750-757`).
  - A later packet whose DUC time is ahead starts playback (`:766-792`). A dropped packet returns before flow control (`:1942`).
- **Module.**
  - The TimeError arm names a pending burst only if it is the unique candidate, and it neither retires nor abandons (`tx.rs:1152-1173`, `:345-353`).
  - A correlation is taken only at a device SOB (`:695-697`).
  - `abandon` leaves `unacked` untouched (`:877-911`).
- **MockRadio** emits only `drop`, `plan_violation`, `refused` and `send_asap` (`crates/ezsdr-mock-radio/src/lib.rs:1714-1764`). It never emits `late_at_device`.

## 2. Minimal sequences: what goes wrong today

- **S1 (the fake is looser).**
  - Setup: 1 MS/s, W = 10 ms, chunks of 2 500 samples. A booking at T; uhd-tx's first send reaches the device 1 ms after T.
  - Fake: send 1 (DUC time T) gets a TimeError and is dropped. Send 2 (DUC time T + 2 500) is ahead of the fake's clock, so it plays from T + 2.5 ms to the end, and the EOB gets a BurstAck.
  - X310: nothing plays and no BurstAck comes (E5, H1). A test expecting sound after a stall passes on the fake and fails on the device.
- **S2 (a late repeat is fed forever).**
  - A `start_repeat` whose first send is late at the device produces one `late_at_device`. uhd-tx then feeds the repeat until the Stop, because nothing abandons it (`tx.rs:1152-1173`).
  - The X310 drops all of it. `async` gains a row per delivered report: about 18 per ms per channel in H1, INFERRED for a long repeat.
  - The record says the repeat ran until the Stop.
- **S3 (the cascade is silent).**
  - Device burst A is late by δ. Device burst B is handed at A.end + 4, which is before A.end + δ.
  - A gets no ACK (E5) and stays pending. B's reports have `t ≥ B.start`, so there are two candidates and the report stays raw only (`tx.rs:345-353`, pinned by the tests at `:3742` and `:3849`).
  - B is dropped with no event. The hardware side is INFERRED: E2, E3 and the in-order FIFO.
- **S4 (a joined burst is never named).** Kernel bursts A and B are joined into one device burst whose first packet is late. One event names A. B is dropped as well and gets no event, because correlations are taken per device burst (`tx.rs:695-697`).
- **S5 (two events for one burst; INFERRED from the code).**
  - A repeat underflows: TX_UNDERFLOW and an abandon.
  - The handed rest is dropped late (P6), giving `TimeError t ≥ first`. The burst is still pending and the unique candidate, so it gets `late_at_device`.
  - It did transmit up to the underflow, contrary to RM-11's "nothing was transmitted" (`design/07-radio-model.md:134`).

## 3. The user-facing contract (after A)

**USRP X310 (UHD Module):**
- **The late burst.** A burst can be handed in time by the Module's clock and still reach the device late, after a host stall. Such a burst transmits nothing. Neither does any burst joined to it in the same device burst (RM-15's join). Each of these bursts gets one `radio.TIME_ERROR { cause: late, outcome: late_at_device, late_by_ns, target }` from `<id>/tx/s<k>`. `late_by_ns` is the report's instant less the device burst's start.
- **The Module ends that device burst.**
  - Its open Kernel burst ends at the handed cursor: the record shows `end: stop`, and `rejected` gets the note `UR-22: the device dropped the burst late; burst not resumed`.
  - A repeat ends; start it again. A switch on it is then refused, as after an underflow (`tx.rs:6942`).
  - Its region stays (KL-39). Bursts held behind it keep their bookings and are judged by their late policy at dispatch.
- **Bursts behind it.** A burst handed behind the late one, whose start comes before the device has drained it, is dropped too and gets its own `late_at_device`. This is INFERRED until the bench measures it; its `late_by_ns` is a lower bound.
- **The Manifest.**
  - `bursts` stays the host's view, unmarked (UR-30, `design/18-uhd-radio.md:350`).
  - `async` keeps every raw report. Each incident is now bounded by what was handed: at most about W's worth of DUC packets per channel, of which the device's FIFO delivers a fraction.
  - `rejected` gets the abandon note.
- **After an underflow:** `TX_UNDERFLOW` only, with no `late_at_device` for the same burst.
- **You can rely on:**
  - no late burst is ever partly transmitted;
  - no time shift is ever hidden (no untimed salvage);
  - every burst the device dropped has its event.
- **You cannot rely on:**
  - the number of TimeError rows;
  - `late_by_ns` of a burst dropped behind another;
  - the event for a short last burst, when the device's FIFO dropped every report after its start. The next ACK then retires it silently (INFERRED edge).

**FakeDevice:**
- It gives the same events, records and abandon for the Module's shapes.
- It reports one TimeError per channel per dropped send. It does not model the DUC's packet sizes or the error FIFO.
- After a late timed head, it still takes the first untimed continuation as timed and drops it, whereas the X310 plays it late (H1's second cell). The fake is stricter there, for a shape the Module never sends; UR-33 states this.

**MockRadio:** it never emits `late_at_device`: no host stands between its decision and its device, and its lateness is RM-14's, judged at receipt. A Spec that passes on MockRadio can still meet `late_at_device` on the USRP under host load. A KL entry declares this.

## 4. Alternatives

- **A (recommended)**, as in §0. It names every dropped burst, bounds the reports, reuses the underflow path, and makes the fake as strict as the device.
- **B. Keep the fake and declare the difference.** Rejected. The fake is looser than the X310 in the Module's own shape (S1), against UR-33's "so that a test cannot pass on a looser fake" (`design/18-uhd-radio.md:361`) and the purpose of §59. S2–S5 would stay invisible to tests.
- **C. Model the fake per UHD and DUC packet, with H1's report counts.** Rejected. The counts depend on DUC packetisation and on FIFO loss (§1.2), vary with the rate and were measured only at 1 MS/s. No rule reads them, so this would build a packetiser for no test.
- **D. Set `REG_TX_ERROR_POLICY` to BURST.** Rejected.
  - The C API cannot reach it (E1); it would need C++ outside UR-3's C slice.
  - For all-timed sends it changes nothing that plays (E5). It only cuts the report count, which A already bounds.
- **E. Send continuations untimed, so a late burst's tail plays** (H1's second cell). Rejected.
  - The tail plays late by the lateness (§1.2): a hidden time jump, which the Stream Contract forbids (§23, UR-22 T1, note 39 §6).
  - It would reopen the DUC-state defects #72 and F7.
- **F. Fix the fake and the correlation, but do not abandon.** Rejected.
  - S2 remains: a late repeat stays silent until the Stop, `async` grows without bound, and the Module keeps a dead device burst open.
  - The existing answer for a burst the device has killed is the underflow path: KL-13 says "an underflow ends the burst".
- **G. Push the stream's front past the drain** (front += the reported lateness) to stop the cascade. Rejected. It is a new mechanism for a rare event. The host can only bound the drain's end, not know it, and A already names each burst dropped behind and shortens the drain by abandoning at the first report.
- **H. Mark dropped bursts in `bursts`** (a `dropped_at_device` field). Rejected for now. `BurstRecord` is the Kernel's type (SC-28, SC-29), and the event already names the target.
- **I. Re-send a late `send_asap` burst** (a repeat) at a later safe instant. Rejected for the same reason as G: the drain's end is unknown. The underflow path does not resume either. This is O3 below.

## 5. Principles that decide cases

- **P1** The X310's unit of a late drop is the device burst. With every packet timed, once one packet is late every later one is too (E5).
- **P2** The radio works in order and never faster than real time. A report at tick `t` therefore means every pending device burst whose first instant is at or before `t` is late: either it is the packet being dropped, or it is queued behind it.
- **P3** Every burst the device dropped is told once, by its own event. A burst whose damage is already told (TX_UNDERFLOW, or the note of an abandon) is not told again as late.
- **P4** A device burst the device has killed is abandoned the same way, whatever killed it: an underflow, a sequence error or a late report.
- **P5** The fake is never looser than the X310 in the Module's shapes. Where it differs it is stricter, and UR-33 says so.
- **P6** MockRadio models no host. §13, §34 and §59 concern the envelope (RM-14's device lead), which does not change.
- **P7** No rule pins a TimeError count.

## 6. Check against existing promises

- **RM-11** (`design/07-radio-model.md:134`), "so nothing was transmitted":
  - today it holds on the X310 for the named burst, but is false on the fake (S1) and after an underflow (S5);
  - after A it holds on both, and the text says it holds per burst.
  - `late_by_ns` is redefined: RM-11's "the reported instant less the target" becomes "less the start of the target's device burst". The old reading would go negative for a joined burst that starts after the report (O1).
- **RM-14 and RM-15:** unchanged; burst decisions stay at receipt and at dispatch. The abandon keeps RM-15's footprint (KL-39).
- **UR-22** (`design/18-uhd-radio.md:257-260`): T1 is unchanged. The list of reports that abandon a burst gains the late report, with the underflow's tick rule.
- **UR-28** (`:335-336`): the TimeError rule is replaced, and the dedup sentence goes.
- **UR-30** (`:350`): unchanged; the record is still unmarked, and the abandon ends it early.
- **UR-33:**
  - `:375`, "a future packet starts playback with or without start-of-burst", now holds only after the DUC burst's EOB; the drain sentence is added there.
  - The undeclared shapes go into `:361`'s list.
- **§23 and SC-25:** nothing is padded or filled; the abandon ends the burst and nothing is sent untimed. No Provider emits `TX_DISCONTINUITY` (RM-10).
- **§13, §34 and §59:** MockRadio is unchanged, and the fake becomes stricter.
- **`mech:refusal-where-late-policy-decides`** (three strikes; note 49 §11.2, C6 open): the ended repeat's KL-39 region and the refused switch on it are existing refusals, which now gain a trigger, a host stall. Ending a `send_asap` repeat instead of moving it (O3) is exactly that mechanism's case. A keeps it bounded to the underflow path's existing answer (KL-13); Alternative I is the answer the mechanism would prefer.
- **KL entries:**
  - KL-13 (`KNOWN_LIMITATIONS.md:127`, "an underflow ends the burst") gains "a late report ends it too".
  - KL-16 (`:216`) gains "MockRadio never shows `late_at_device`".
  - KL-39 (`:155`) adds the late report to its list of abandons.
  - KL-41 (`:169`) already describes the refused switch.
- **EA-16:** no API change; `events()` shows the events.
- **Note 39 §6.6** (`plan/phase8/39-dsp-timed-commands.md:117-126`, 2026-10-29): its rule that a TimeError "names only when unique, without retiring" rested on a later packet of a late burst possibly still playing. E5 and H1 refute that for all-timed sends. Under AGENTS §9 the re-decision goes back to whoever owns it, and O1 and O4 cover its user-visible part.

## 7. What this makes impossible later

- Salvaging the rest of a late burst on the X310. That would need untimed sends, which T1 forbids, and A ends the burst.
- Treating a TimeError as a mere note for a burst that might still be acknowledged. A's rule assumes every payload packet is timed; reintroducing untimed payload means revisiting UR-28.
- A fake that plays a later packet of a late DUC burst. A device whose drain could outrun time would need a new model; there is none in UHD 4.10's X3x0, and X4xx at NIPC > 1 still drains one word per clock.
- A late repeat continuing by itself. Auto-resume would become one decision together with the underflow path (O3).

## 8. Tests that change (AGENTS §9 "Hardware tests and bench values")

- **First test** (it pins the premise and fails on today's fake): a `device.rs` unit test.
  - Setup: a timed SOB send 1 ms late, then timed continuations at the Module's chunk size, with EOB on the last.
  - Expected: every send dropped, at least one TimeError per channel per send, no BurstAck, `transmitted` empty.
- **New fake unit tests:**
  - the cascade: B at A.end + k plays when k lies outside A's lateness and is late inside it;
  - the report tick is the instant the radio reaches the packet;
  - a dropped packet is flow-controlled like a played one.
- **Changed tests:**
  - `device.rs:2664` still passes, with its claim narrowed to "after an EOB".
  - `tx.rs:3742`, `:3761`, `:3815` and `:3849` are inverted. In the last one, B is named because it is queued behind A's drain.
  - `tx.rs:3869` loses the dedup set, and `tx.rs:4374` changes because the abandon retires A.
  - `tests/fake.rs:2533` and `:3626`: the repeat ends, one `late_at_device` comes per Kernel burst, and the record ends at the abandon.
  - `tests/fake.rs:7685`: check the D9 probe's P6 part.
- **New Module tests:** one per sequence S2–S5 (`tx.rs` units on the manual clock, plus one Session-level test on the fake).
- **Hardware:**
  - **H1** (`hardware.rs:1701`) gets a `VERDICT` for the rule UR-33 takes from it, and a rehearsal on the fake. New cells:
    - the Module's chunked timed burst;
    - a cascade cell with B at A.end + gap, the gap below and above the lateness, ≥ 10 trials per cell near the boundary.
  - **A Module-level `hw_` test** stalls the host before the first send, using the `Descheduled` wrapper that `tests/fake.rs:3626` uses. Cases: a finite burst, a repeat, and two joined bursts. Expected: one event per Kernel burst, nothing heard, the record abandoned, and a burst booked past the drain plays. It gets the same rehearsal on the fake.
- **Mutation rows:** the drain line; `≤` versus `<` in the retire rule; the abandon call; the per-Kernel-burst naming; the abandon's retire.

## 9. Decisions

**Owner** (user-visible), each with my recommendation:
- **O1** — the meaning of `late_at_device` (RM-11): "nothing of this burst was transmitted", one event per burst the device dropped, including bursts joined to it and bursts handed behind it within its lateness. `late_by_ns` is measured from the start of the burst's device burst. **Recommend: yes.**
- **O2** — the Module ends a device burst the device reports late, as after an underflow. The record ends at the handed cursor, a repeat ends, a switch on it is refused, and its region stays (KL-39). Two existing refusals thereby gain a trigger, a host stall (`mech:refusal-where-late-policy-decides`). **Recommend: yes.**
- **O3** — a `send_asap` burst (a repeat) that is late at the device is flagged and ended, not re-sent later. This is that mechanism's own case: the declared late policy says "move and flag". **Recommend: flag and end; the user re-issues it.** Re-sending (Alternative I) would be one decision together with resuming after an underflow.
- **O4** — after an underflow or any other abandon, no `late_at_device` for the same burst. **Recommend: yes.**
- **O5** — records: no mark in `bursts` (UR-30 stays), every raw report stays in `async`, and each abandon writes one `rejected` note. **Recommend: as stated.**
- **O6** — KNOWN_LIMITATIONS: a new Device-behaviour entry (the whole device burst plus the bursts behind it, and MockRadio never shows it), plus one line each in KL-13, KL-16 and KL-39. **Recommend: add.**
- **O7** — bench before generalising beyond 1 MS/s on #1: the Module-shaped H1 cell, the cascade cell, 10/100/200 MS/s, both boards. **Recommend: at the next bench. The code need not wait, since A is backed by the source and only stricter.**

**Internal** (orchestrator decides):
- the fake's drain model, its report tick and its flow control;
- what UR-33 lists as not modelled (counts, FIFO loss, the first untimed continuation after a late timed head);
- the correlation's shape: device-burst entries carrying their Kernel targets, per-channel retirement, one event per burst;
- reusing `DeviceChain::underflowed` (renamed) as the abandon predicate;
- the tickless first-pending rule: X310 reports always carry a time (E6), so it serves only scripted fakes. Once an abandon retires A, a tickless ACK of A's close would retire B; keep the rule (and fix that) or drop it;
- the step's risk class, test plan and Sol review, and the mutation rows;
- sub-steps: the fake first, then the Module;
- the record fixes (§10) and the mechanism labels. S3–S5 fit `mech:correlation-by-partial-key`, whose note is 39 §6.6.

## 10. Records to correct (orchestrator)

- `plan/maintenance/audits-2026-10.md:16`, row 3: its premise ("fake drops the whole burst") has been stale since `80dd2f6`, and so is `plan/phase8/bench-results.md:459` ("Fake は burst ごと捨てる").
- `bench/2026-11-13-n67-records/README.md`, H1 row:
  - R2 ran H1 too (18 trials, the same outcome, rows in the raw archive), but the R2 column shows "—".
  - "~366 TIME_ERROR per burst" is a count of delivered reports, not of packets.

**Residual risks:**
- H1 covers one rate on one board.
- The cascade is INFERRED.
- Under a flood the error FIFO can drop a later burst's ACK (E6), leaving that burst pending until the next ACK.

## Repeat resume (addendum.md)

# Maintenance table 3, addendum — a repeat resumes after a late drop or an underflow (AGENTS §9 step 0)

Design drafter (Opus, effort max), 2026-11-14. Read-only: nothing in any tree changed.
- Repo lines are main `c04fda1` (worktree `Ez-SDRv4-e3`).
- Evidence is cited, not repeated: `draft.md` §1 (E1–E7, H1), `fable-review.md` (R1–R10, §2.1, M1–M3, m1–m3, C1–C6), `test-plan.md` §9 (the repeat tests and rows this touches).
- The decision being designed is "REPEAT RESUME" in `decisions.md`.

## 0. Recommendation

1. **Triggers.** A host stall kills a repeat's device burst in three ways: an `Underflow` report reaches it, a `TimeError` retires it (A′), or uhd-tx's own expired-timeline check fires. In each case the Module closes the device burst (the T2 two-zero close at the handed cursor `H`), and the repeat resumes. Every other death still ends it (§1).
2. **Resume point.** `R = repeat_boundary(origin, N, max(busy + m, decision instant + device lead, device front))`. `busy` is the Module's own bound on when the device finishes all it was handed. Each send the device took moves it to `max(busy, the instant the send returned, its first sample) + its samples`. `m` is the owner's W: 10 ms (`radio.timing.tx_in_flight_ns`) until C4 and C7. This is the owner's `H + δ + W` with `δ = busy − (H+2)`, the lateness the Module's own sends prove. That is never below the δ the device reports for a host stall (up to the transport time τ). **It departs from "the reported lateness" on purpose.** The device's δ alone is early after a second stall during the drain, and absurd after a lost `Underflow` (§2.2). So device ticks trigger the resume but never set R.
3. **Across the gap the open booking stays open** with its next sample at `R`, and no device burst is open. The device chain becomes the one source of "device burst open" (four sites, §3). The send at `R` is a timed SOB (T1) of at least 32 samples (tiled), starting at phase 0.
4. **Events and record.** The drop's event (`TIME_ERROR { late_at_device }` or `TX_UNDERFLOW { starved }`) gains an optional `resumed_at: R`. The record stays one burst per Action, with a declared gap (`GAP_BEFORE` and `lost` on the block at R; `BurstRecord.lost`), as SC-25 reads. Owner decision 5, with the alternative.
5. **The cap** is decided by the kind of the first report that reaches the resumed device burst. A `TimeError` means nothing of it played (E5), so the repeat ends by today's abandon. An `Underflow` means it played, so it is a new incident and resumes again. No clock is involved.
6. **Risk.** The step is risky: a deadline, an identity, a lifecycle, and the boundary between the fake and the hardware. It needs test-plan additions (§8) and Sol's review. MockRadio needs nothing built (§5).

## 1. What a repeat's device burst can die of

| Death (where) | Today | Now |
|---|---|---|
| `Underflow`/`UnderflowInPacket`, tick ≥ the open burst's first sample, or no tick (`tx.rs:1130-1150`, `staging.rs:589`) | abandon | **resume** the span starts at the event's time |
| `TimeError` retiring the open device burst (A′'s arm, replacing `tx.rs:1152-1173`) | abandon (A′) | **resume** the span starts at its target |
| Expired timeline with a device burst open (`tx.rs:398-401`); runs before reports are read (Fable M1) | abandon | **resume** the device's later report carries the event (§4) |
| The same check while resuming (R passed, nothing handed) | — | **missed R** (§3): no close, no entry |
| Expired timeline, waiting device burst (`tx.rs:420-423`) | abandon | unreachable: a repeat never leaves one |
| `SeqError`/`SeqErrorInBurst` (`TX_UNDERFLOW { lost }`) | abandon | **ends**: not a host stall, and the tick's meaning is unmeasured |
| Failed or short send or tail (`tx.rs:611, 655, 669`); an uncertain close's failed reopen | abandon / clock ends | as today |
| A halt (Stop, `Provider::stop`, loss) unread in the list at the drop (`staging.rs:346`) | — | **ends**, as today |
| `R` not fitting the clock (`repeat_boundary` → `None`, `staging.rs:1616`) | — | **ends** |
| The cap: a `TimeError` reaches a resumed device burst (§4) | — | **ends** |

One-shot bursts keep today's abandon in every row.

## 2. The resume point

### 2.1 The bound

- **`busy`** is one value per stream, held in `DeviceChain` ("what the executor knows of the device", `staging.rs:545-552`), across owners and reopens. After each send the device took whole (payload, close or T3), `busy ← max(busy, r, x) + n` in root ticks. `r` is `core.now()` when `tx_send` returned. `x` is the first-sample instant (none for the untimed T3). `n` is the sample count, padding included. On an on-time stream, `busy` is the handed cursor. After a late send it is that send's return plus everything queued behind it, which is the drain. It carries into the next device burst, so the cascade (Fable R5) is inside it.
- **R** is computed after the drop's close returns.
- **The lattice** is the open booking's `(k, waveform_len)`: the repeat's start, its dispatch-moved origin (e2a_88), or an applied switch's `b`. RM-37 takes boundaries on the same lattice (`playing`, `staging.rs:915`). The waveform restarts at offset 0 at R (`segment.rs:91`).

### 2.2 Why not the reported δ

Counterexamples at 1 MS/s, with a 10 ms window and 2 500-sample chunks:
- **(a) Early.** Send 1 returns 1 ms late, so the device reports δ = 1 ms. Before that report is read, send 2 stalls 6 ms. The device drains send 1, waits (R6), and drains send 2 until about T + 11 ms. `H + 2 + δ + m` = T + 6 ms + m, inside the drain unless m ≥ 5 ms, so the cap ends the repeat. `busy` = T + 11 ms + 2 is correct.
- **(b) Absurd.** A repeat plays for 60 s, then underflows. Its `Underflow` is lost: the 32-entry FIFO is still full from an earlier drop's flood (R7, E6). A′ retires the repeat on a `TimeError` with δ = t − first ≈ 60 s, so the repeat would be silent for a minute. `busy` is the stalled send's return plus its samples: a few ms.

### 2.3 R cannot be early for the Module's shapes

- **The device follows the same recursion as `busy`, with arrival ≤ `r + τ`.** Every payload send is timed (T1, E7), so a late first packet leaves the whole DUC burst late (E4–E5). The radio takes samples in order: it plays each at its instant, and drops a late one no earlier than its arrival, at one word per clock plus 2 clocks per DUC output packet (E2, E3, Fable §2.1).
- **So the drain ends at `D ≤ busy + τ + ε·S + b`.** `S` is the samples drained: at most a chunk, plus the window, plus the report latency, which is about 13 ms plus latency at 1 MS/s. The expired check (`tx.rs:398`) or the first report stops the hand-over. `ε` is at most 2/32 for 32-sample DUC packets (Fable M3, C3), and 2/1996 VERIFIED at 1 MS/s. `b` is the DUC's between-burst latency after an EOB: ≤ 10 µs after a short closed head (2026-11-10), INFERRED after a drop.
- **The condition.** `R ≥ D + g + 2 clocks` (R9 and the post-EOB gap) holds when `m ≥ τ + ε·S + b + g + 2 clocks`. With m = 10 ms that holds for τ up to about 9 ms. τ covers the send's return to the radio's input plus the host/device clock relation, and is INFERRED to be ≪ 1 ms.
- **Fail-safe.** If R is early anyway, the cap ends the repeat: today's outcome, never a loop.

### 2.4 Values and bench cells (INFERRED until measured)

- **m = 10 ms** costs about 10 ms of extra silence per incident.
- **Existing cells.** C4 (Fable §2.4) gives `b` and `ε` after a dropped burst. C2 and C3 give ε at other rates and after a 32-sample first packet. C6 gives the report order of an underflow.
- **New C7** (Module level, both boards): a `start_repeat` forced late at its start, and one starved mid-play, using the `Descheduled` wrapper (`tests/fake.rs:3466`) on `UhdDevice`. m swept over 10, 1 and 0.2 ms, at least 10 trials per cell. Assert silence in `[from, R)`, the waveform back at R with phase continuous, and no cap. Only C7 measures τ on the Module's own path. The owner then sets m.
- **FakeDevice.** The fake has τ = 0 and an exact drain, so only Fable's drain pad lets a fake test kill a mutant that drops `m`. Decided here: the fake's drain ends at its samples plus `max(g, ⌈S/100⌉)`, and arms the existing guard `g` from there (Fable M3). Tests keep m ≫ the pad.

## 3. Closing the dropped device burst and starting the new one

- **The close** is the abandon's two-zero close at `H` (`tx.rs:765-781, 898-900`): timed, EOB, no tail, since a repeat never holds one. The radio drops it, but its EOB still ends the DUC burst (E4; UR-33 transmit), so the SOB time at R is honoured. `Sent` sets the front to `H + 2 + g`. `Uncertain` runs the reopen and T3 first (UR-22, #75). A flush front past R is a missed R. A failed reopen ends the clock.
- **It does not call `Bookings::abandon`** (`staging.rs:1415`). The record keeps `[start, ∞)` and `until: Open`, with no footprint, and `playing` stays. Only switches change, as below.
- **The one new state, "resuming":** an open booking that is not `first` while no device burst is open. Only the resume enters it. It is left by the first whole send at R, by an end, or by a missed R. Four sites infer "device burst open" from `!open.first` and must read `DeviceChain` instead (AGENTS §9 maintainability 2): `tx.rs:555` (`device_sob`), `:647` (`was_open`), `:740` (`end_open_at`, which would otherwise send a stray close at R on a Stop), and `:1017` (`join_open`, which would otherwise panic in `continue_device_burst`). The block at R carries `GAP_BEFORE` with `lost = R − H`, not `START_OF_BURST` (§4). `remember_burst(R)` marks its entry `resumed`.
- **A missed R.** The send at R is judged as any device burst's start (RM-14: the decision instant plus the device lead). If it is late, R moves to the next on-time lattice point, with `TIME_ERROR { send_asap }` (the repeat's policy, RM-12). This is not the cap: nothing was handed. It never goes through `untake` → `dispatch_start`, whose SendAsap moves the repeat off the lattice (`tx.rs:393-396, 975-988`). A stall inside the send at R is a second drop, so the cap decides it.
- **Switches (RM-37), for a pending switch at `b`.** `b < H` (its boundary send was handed): the open booking is already the switch's. It settles `applied` when device time reaches `b`, as today (UR-30 host view), and its waveform resumes at R on its own lattice. `H ≤ b < R + 32`: cancelled. Its sample at `b` is never sent ("never moved"), and the first send at R needs 32 samples (UR-22). Its payload goes (`TxBook::cancel_switch`, `staging.rs:984`), and the replaced repeat is `playing` again. `b ≥ R + 32`: kept. A switch received in the gap is refused with UR-38's reason. The resume sets `playing.started = false`, and the first issue at R sets it again (`TxBook::issue`, `staging.rs:786-795`).
- **Stop, `Provider::stop` or a loss in the gap.** The open booking ends with no close, since none is open. The record ends `stop`; RM-16's handed cursor is `H`. Nothing is sent at R. The Stop region follows RM-16.
- **A burst booked behind the repeat.** The record refuses any booking that meets an open repeat's `[start, ∞)` (`staging.rs:1105-1130`, `meets` `:1526`). So one exists only as a switch, or after a Stop that the record has booked but uhd-tx has not yet read, which caps the region (e1_q5 at `e1.rs:216`; e2a_80 at `e2a.rs:233, 256`). In that case the unread-halt row of §1 ends the repeat as today, and those tests keep their answers.
- **KL-39.** A resumed repeat keeps its whole region. The cursor-plus-five footprint now comes only from §1's ends.

## 4. The user-facing contract

- **Events.** `TIME_ERROR { late, late_at_device, late_by_ns, target, resumed_at? }`, one per Kernel burst (A′, O1, m1). `TX_UNDERFLOW { starved, resumed_at? }`, one per report. The span `[from, resumed_at)` transmitted nothing. `from` is the target (for a resumed part, the previous `resumed_at`) or, for `TX_UNDERFLOW`, the event's time. The field is absent when the repeat ended, and on one-shot bursts. It is optional and skipped when absent (RM-22, `deny_unknown_fields`), so today's payloads are unchanged. Schemas are regenerated, with no version bump (AGENTS §6).
- **Who emits.** The report that retires the dropped device burst's entry, into which the resume writes `R` (UR-28). So the expired path, which has no report yet, gets the same event later, whichever host branch ran first (Fable M1, M2). A report that retires several entries makes one event per Kernel burst. That happens on the expired path when its own report never came and a later one arrives at t ≥ R. The newest entry decides: a `TimeError` on a resumed entry is the cap, with no `resumed_at`; an `Underflow` resumes again with the new R. This is `mech:correlation-by-partial-key` ground, and R3b pins it.
- **The record.** SC-25: "Continuous transmission is one burst", and the underflow gap is recorded in the Manifest (`02-stream-contract.md:250`). **Recommended:** `BurstTracker` continues a burst across a block that carries `GAP_BEFORE` with `lost` equal to its forward jump, and `BurstRecord` gains `lost`. An undeclared jump still splits it (SC-24). A repeat is then one record: `samples` handed, `lost`, wraps counted, and `end` = `stop` at a Stop, the Run's end or the cap. There is no `TX_DISCONTINUITY`, because there is no discontinuity. Cost: a small Kernel change (SC-24, SC-25, SC-28; `burst.rs:296-358`; the `BurstRecord` schema). MockRadio never sets the flag. **Alternative, with no Kernel code:** the nested-SOB path. The parts end `discontinuity` (`burst.rs:148-156`), the part at R opens with a `BurstOpen`, and SC-24a gains a scope line.
- **Manifest.** `async` keeps every report. `rejected` gets one note per resume: `UR-22: the repeat resumes at <R> after <a late drop | an underflow | an expired timeline>`. `switches` follows §3.
- **The cap** is decided from device reports alone, with no clock. Per channel the reports come in device order (E6). The first report that reaches the `resumed` entry decides: a `TimeError` means nothing played (E5), so the repeat ends; an `Underflow` means the underrun happened in TRANSMIT (R3, R4), so the part played, and the repeat resumes; a `BurstAck` means the part played to its close. Declared edges, INFERRED: a lost `Underflow` of a part that played (R7) and one channel late alone (m1) end the repeat.
- **You can rely on these:** the lattice (waveform sample `k` is at `origin + j·N + k` across every gap); each gap is silent on every channel (INFERRED for m1, and for N3's tens of samples after an uncertain close's T3); each gap is counted in `lost`; a second drop before the part at R plays ends the repeat, with today's answers.
- **You cannot rely on these:** a gap's length (at least the stall, plus `H − from`, plus m, plus up to one period); every gap having its event (a lost report, or a streamer replaced after an uncertain close, loses it, while `lost` keeps it); `resumed_at` being reached when a Stop comes in the gap; the cap under R7 or m1.
- **EA.** The client reads payloads as dicts (`python/ezsdr/session.py:453`), so it needs no change. `tx.switch()` now also raises `Rejected` during a gap: one phrase each in EA-16 (`design/16-easy-api.md:102`) and in the docstring (`session.py:498-507`).

## 5. MockRadio

- **It never resumes, so nothing is built.** It never underflows: "it never underflows" (MR-15, `design/09-mock-radio.md:142`); RM-10 lists no `TX_UNDERFLOW` for it (`design/07-radio-model.md:119`). It emits `TIME_ERROR` only as `send_asap`, `drop`, `plan_violation` or `refused` (`crates/ezsdr-mock-radio/src/lib.rs:1714-1764`). No host stands between its decision and its device. RM-15 is worded so the rule is vacuous for it: "a repeated burst ends only at …; a Provider whose device drops part of it resumes it (UR-22)".
- **Parity is unchanged.** No parity test stalls a host (`grep -n 'underflow\|stall\|Descheduled' crates/ezsdr-acceptance/tests/parity.rs`: none). On MockRadio a repeat never pauses. On the USRP a host stall pauses it once per incident, told by the event and by `lost` (a KL-16 line). §3's "Mock enforces the hardware envelope" concerns the Timing and Performance envelopes, which do not change (draft P6).

## 6. Alternatives

- **α. Resume after the last report.** Rejected: no report marks the drain's end. The dropped EOB is never acked (E5, N2), and under a flood the close's own `TimeError` arrives about one time in six (H1). It would also need a guessed quiet-time timeout.
- **β. End after N resumes in a time window.** Rejected: it needs two unmeasured policy values, and the owner chose one cap, decided by reports.
- **γ. The reported δ alone (the literal formula).** Rejected: early in case (a) of §2.2, absurd in case (b).
- **δ′. Resume at once,** at `now` plus the 2 ms lead (`profile.rs:11`). Rejected: the drain in case L lasts a chunk plus the window, about 12.5 ms, so the cap would end most stalls.
- **ε. Hold the repeat again at R.** Rejected: dispatch's SendAsap moves it off the lattice (e2a_88), a Stop cancels it as a held burst, and the tracker's record is left open.
- **ζ. Mark the gap with `TX_DISCONTINUITY`, or with `tracker.stop()` and a new record.** Rejected: RM-22 has no payload for `TX_DISCONTINUITY`, and it would be a second event per gap. `end: stop` claims a Stop that never happened.
- **G. Front every burst at `busy`.** Not now: it reopens O1's events for bursts dropped behind (§7).

## 7. What this makes impossible later

- **A dropped span is never re-sent:** it is skipped on the lattice, so a repeat no longer promises every sample of every period.
- **A gap's end is announced at the drop** (`resumed_at`): a later, better estimate can move it later (`send_asap`), never earlier.
- **Still possible:** `busy` as every burst's front (G), and the same resume for one-shot bursts (RM-15 would allow a declared gap in a finite burst).

## 8. Amendments, tests, mutation rows

**Amendments.**
- **RM-11** (`:121, :134`): `late_at_device` means "nothing from `from` to `resumed_at`".
- **RM-22** (`:223`): `resumed_at: Option<TimePoint>` on both payloads.
- **RM-15** (`:164-175`): the ends of a repeat; "a repeat that UR-22 abandoned" now covers only §1's ends.
- **RM-37** (`:359-363`): the switch cases, and the refusal in a gap.
- **SC-24, SC-25, SC-28** (`02-stream-contract.md:246, 250, 256`): the declared gap and `lost`. Under the alternative, a scope line in SC-24a (`:248`) instead.
- **UR-22** (`:257-260`): the triggers, close, `busy`, `R`, `m`, a missed R, an unread halt, and the cap.
- **UR-23** (`:270-286`): the open booking across a gap; the device chain decides SOB and continuation.
- **UR-28** (`:335-338`): `resumed_at` and the `resumed` mark on entries; the newest entry decides.
- **UR-30** (`:349-352`): `lost` and the resume note.
- **UR-33** (`:361`): the drain pad, and τ = 0 on the fake (only C7 shows whether m is enough).
- **UR-38** (`:310`): a switch in a gap is refused.
- **EA-16** (`16-easy-api.md:102`).
- **KL-13** (`:127-132`): (c) now holds for finite bursts only; a repeat pauses and resumes. Its Status must say plainly that, for repeats, this reverses `c04fda1`'s "abandoning the burst after an underflow: kept by the owner on 2026-11-14". The shorter-bursts workaround stays for one-shot bursts.
- **KL-16** (`:216`), **KL-39** (`:155`) and **KL-41** (`:169`): one line each.
- **KL-45** (`test-plan.md` §7): pause and resume, and the cap's edges.
- **Records:** `plan/phase8/50-remaining-work.md`: E's (a) is partly revived, and A's O2 staging line is superseded. Note 39 §6.3 (`39-dsp-timed-commands.md:94-96`) points here. `brief-plan.md` S2 and O3, and `test-plan.md` §9 and Q10: a late repeat now resumes. Q10's option (b) fits sub-step D.

**Tests.** Units run on the manual clock with scripted reports (`rig`, `switch_rig`, `tx.report`). Session tests run on the fake (`SlowSend`, `Descheduled`). In each row, the text after `→` is the wrong behaviour the test catches.

| # | What it asserts → what it catches |
|---|---|
| R1 | Late start: `late_at_device { target T, resumed_at R }`; R is exactly the lattice point ≥ busy + m; silence in `[T, R)`, then phase 0 at R; one record with `lost = R − H` and wraps counted; one note. → today's end, R at busy, R off the lattice, a split record |
| R2 | Underflow mid-repeat: `TX_UNDERFLOW { resumed_at: R }`. |
| R3 | Expired timeline first, then the report: same kind, `from` and lattice R. R3b: one report retiring two entries follows the newest. → branch dependence |
| R4 | Double stall (§2.2a, a probe at `tx_send_planned`): the part at R plays. → `busy` without `r` |
| R5 | Cap: a scripted `TimeError` with t ≥ R gives today's abandon answers (note, cursor + 5, switch refused) and no `resumed_at`. → resuming again |
| R6 | An `Underflow` with tick ≥ R resumes again. → the cap firing on a part that played |
| R7, R8 | §3's switch cases; a switch in the gap is refused, then accepted after the first issue at R. |
| R9 | Stop, `Provider::stop` and loss in the gap: no send at or after R, no stray close. |
| R10 | A halt unread at the drop: the repeat ends. |
| R11 | Uncertain close: T3 runs first; R is past the flush front, or moves; a failed reopen ends the clock. |
| R12 | Missed R: the next on-time lattice point, `send_asap`, not the cap. |
| R13 | A one-shot burst and a `SeqError` end. |
| R14 | R's pure function: the rounding, the max, `None` past the clock. |
| R15 | Two channels: one event per Kernel burst; the cap fires on one channel's `TimeError`. |
| HW | C7's body as an `hw_` test, with its FakeDevice rehearsal (AGENTS §9). |

**Tests that change.** `test-plan.md` §9's list, plus:
- `ur_22_underflow_and_loss_end_the_damaged_burst` (`tx.rs:1899`): split by code.
- `ur_22_real_starvation_does_not_extend_the_stop_window` (`tx.rs:1971`).
- `c2_s10` and `c2_s11` (`c2.rs:220, 239`): outcomes per §3.
- The shared oracle `switch_outcome_after_abandon` (`oracle/mod.rs:378`): gains a resume twin.

**Mutation rows.** Anchors are chosen with the code. Each mutant breaks one rule, killed by the test named:

| Mutant | Killed by |
|---|---|
| Lattice → plain bound | R1 |
| Drop `+ m` | R1, through the pad |
| Drop `r` from `busy` | R4 |
| `repeat` guard → every burst | R13 |
| Cap → resume | R5 |
| Underflow → cap | R6 |
| SeqError → resume | R13 |
| `R + 32` → `H` | R7 |
| Drop `started = false` | R8 |
| `device_sob` back to `first` | R1, R9 |
| `resumed_at` → `None` | R1–R3 |
| Drop `GAP_BEFORE`/`lost` | R1 |
| Drop the unread-halt guard | R10 |
| Drop the missed-R move | R12 |

Rows to re-target: `test-plan.md` §9 (`N47S5-136…155`, `O1-M6`, `C2-41`, `E2A2B-*`, `E2A4-*`, `U28`).

## 9. Decisions

**The owner's** (user-visible, beyond `decisions.md`):
1. **m for now:** 10 ms (`tx_in_flight_ns`), replaced by a measured value after C4 and C7. **Recommend: yes.**
2. **The expired timeline resumes as well.** Otherwise one stall ends or resumes the repeat depending on which host branch runs first (Fable M1, M2). **Recommend: yes.**
3. **What still ends a repeat:** sequence loss, a failed or short send, a failed reopen, a halt unread at the drop, and an R that does not fit the clock. **Recommend: yes.**
4. **The payload:** an optional `resumed_at` on `TIME_ERROR` and `TX_UNDERFLOW`, giving the span `[from, resumed_at)`. It is absent when the repeat ended. **Recommend: yes.**
5. **The record:** one burst with a declared gap (`GAP_BEFORE`, `lost`; a small Kernel change), as SC-25's "continuous transmission is one burst" reads. The alternative is parts that end `discontinuity`, plus a scope line in SC-24a. **Recommend: one burst.**
6. **One `rejected` note per resume** (O5 extended). **Recommend: yes.**
7. **Switches:** a switch already handed is applied; one with `b` in `[H, R + 32)` is cancelled; one with a later `b` is kept; one received in the gap is refused (KL-41 gains a trigger; `mech:refusal-where-late-policy-decides`, note 49 C6). **Recommend: yes.**
8. **A missed R** moves to the next on-time lattice point with `send_asap`; it is not the cap. **Recommend: yes.**
9. **The cap's edges** (R7, m1) end the repeat. **Recommend: accept, recorded in KL-45.**
10. **The KL texts** (13, 16, 39, 41, 45) and EA-16's phrase. **Recommend: add.**
11. **Bench cell C7** joins C1–C6 after 11-15. **Recommend: yes.**

**Internal** (the orchestrator confirms):
- **δ from `busy`, not from the reported δ** (§2.2). This refines the owner's literal wording. The only visible effect is fewer early resumes. Bring it back to the owner if you read it as theirs.
- **Where the state lives:** `busy` in `DeviceChain`; `resumed_at` and `resumed` on A′'s entries; the four `!first` sites read the device chain; no `Bookings::abandon` on a resume; `playing` is restored after a cancelled switch; the fake's pad is `max(g, ⌈S/100⌉)`.
- **Sub-step D** runs after `test-plan.md` §8's B, at about 1 500 lines or fewer. D1 covers `busy`, the resume, events, the record and the cap; D2 covers switches, halts, a missed R and the texts.
