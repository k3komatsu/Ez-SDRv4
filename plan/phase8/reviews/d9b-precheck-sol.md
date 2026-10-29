> plan · gpt-6.1-sol
## D9b pre-check

**Recommendation:** retain owner choice (a): close the old **DUC/device burst**, then resume the Kernel burst at a future position on its original timeline. T1 alone cannot provide this recovery. Strengthen T3’s ordering and R2′’s consumption condition before implementation.

All external source links below reference UHD **v4.10.0.0**.

### 1. Late packets and error policy — VERIFIED

**While `ST_TRANSMIT` remains active, the radio does not check continuation-packet timestamps.** A nominally late continuation is therefore not automatically dropped. After underflow returns the radio to idle, it checks the next packet’s timestamp.

A late packet then generates `ERR_TX_LATE_DATA` and enters policy handling:

- Default **packet policy**: discard through that packet’s end, then reconsider the next packet.
- **Burst policy**: discard through EOB.

The FPGA initializes and resets to packet policy and exposes a writable policy register ([`radio_tx_core.v:100–136`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L100-L136), [`337–409`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L337-L409)).

UHD maps late/underflow/EOB acknowledgment to `TIME_ERROR` (`0x8`), `UNDERFLOW` (`0x2`), and `BURST_ACK` (`0x1`) ([`radio_control_impl.cpp:1291–1318`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/radio_control_impl.cpp#L1291-L1318)). Packet policy can generate one TimeError for **each subsequently checked late packet**; burst-policy dropping does not recheck each packet. A discarded EOB does not take the normal BurstAck path. Error-FIFO overflow can lose reports.

**Host-policy qualification:** generic `underflow_policy=next_packet/next_burst` is documented, but I found no handler in the inspected X300/RFNoC radio, multi-USRP or TX-streamer implementations; do not assume that argument programs this FPGA register.

**Bench:** P6’s Underflow followed by repeated TimeErrors agrees with packet-policy recovery.

### 2. Timed continuation packets — VERIFIED

The radio **neither waits for nor checks lateness of each packet while transmitting**. It waits on timestamps only after entering its idle/time-check path. Thus an early continuation cannot create a scheduled gap inside uninterrupted transmission.

Moreover, the DUC’s `axi_rate_change` takes the **first timestamp**, then generates subsequent timestamps by counting samples; later input timestamps do not reset that timeline ([`axi_rate_change.v:396–433`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_rate_change.v#L396-L433)).

Consequently, “the next host packet has an on-time timestamp” does **not** guarantee playback at that timestamp inside the same DUC burst. T1 preserves `has_time` and enables timed DSP updates; it does not permit skipping missing timeline positions.

**Bench:** P6 switches at approximately e+25 µs; P7 does not switch through e+40 ms. These establish timestamp-continuity effects, not arbitrary mid-burst rescheduling.

### 3. Underflow and resumption — VERIFIED

On a requested sample with no valid data, the radio reports underflow. Between packets it returns directly to idle; inside a packet it discards according to policy. Output becomes the configured idle value, **zero by default**, rather than replaying the last sample ([`radio_tx_core.v:359–426`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L359-L426)).

Under default packet policy, the next acceptable packet can restart transmission. **SOB is not required by this radio interface**—its inputs contain timestamp, `has_time` and EOB, but no SOB.

**Qualification:** the radio restarting does not reset the upstream DUC’s burst timestamp accumulator.

**Bench:** P6 confirms late continuation data disappears and subsequent TX recovery occurs; it does not prove same-DUC-burst recovery at a new timestamp.

### 4. Safe implementation of choice (a) — INFERRED recommendation

**Close and restart the DUC/device burst before skipping forward.** Although the radio can recover without EOB, the DUC otherwise retains its old sample-count timeline.

Recovery should:

1. terminate the old burst through a successfully transferred T2 EOB;
2. choose a still-future cursor on the original Kernel timeline;
3. start a fresh timed device burst there;
4. record the omitted interval as a gap.

A TimeError alone does not inherently require radio-burst termination, but uniform close/restart is the safer Module rule when recovering from lost continuity.

Do not equate a host send’s success with completed playback, or require BurstAck for an EOB the radio may discard. Existing `tx.rs:974–990` also correlates each TimeError as though it terminates a booked burst; repeated continuation TimeErrors invalidate that assumption.

### 5. #70 and the two-sample workaround — VERIFIED mechanism; universal guarantee NOT VERIFIED

For the DUC, `MAX_N=1`. On the first sample, `partial_first_word` is still true. If that sample also ends the burst, the input FSM takes its “partial first word” branch **without publishing the final word count**. Output EOB depends on that count, so it can be lost ([`axi_rate_change.v:225–269`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_rate_change.v#L225-L269), [`327–384`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_rate_change.v#L327-L384)). The timestamp FSM consequently remains in its continuation state and ignores the next burst’s starting time.

An empty EOB normally becomes **one zero sample** in UHD ([`tx_streamer_impl.hpp:266–277`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/transport/tx_streamer_impl.hpp#L266-L277)).

Two successfully transferred samples avoid this first-sample-EOB branch. This is **not proof against every timeout, partial send, transport failure or already-corrupted state**.

**Bench:** D70 verifies the one-sample failure (~150 ms early, no BurstAck) and two-zero-sample control (+34 µs). A one-sample EOB following an already-established longer burst is not necessarily the same defect.

### 6. T3, stale headers and previous commands — VERIFIED / INFERRED

**VERIFIED:** an untimed two-zero-sample burst replaces the effective stale timed-header state. P5 C verifies the workaround on UBX and OBX. It need not literally zero `vita_time_now`; disabling effective `has_time` is what matters ([`axi_tag_time.v:114–134,207–230`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_tag_time.v#L114-L134)).

**It does not drain a pending timed-command FIFO:** timestamp dequeue requires `has_time`. Stale tags can also consume values before valid samples arrive.

Fresh graph construction flushes/resets blocks before controller construction ([`rfnoc_graph.cpp:714–722`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_graph.cpp#L714-L722)); ordinary streamer reopen is not that guarantee.

**INFERRED:** T3 is safe initialization only when queues are known clean and its processing precedes subsequent tuning. “Send T3, then configure” in host-call order alone does not prove that cross-path ordering. Do not describe T3 as applying all previous-process commands before configure.

### 7. #71: depth, overflow and TX R1 — VERIFIED / INFERRED

**VERIFIED:** “depth 32” describes the main FIFOs, not total buffering:

- timestamps: **32 + 2 output-stage entries**;
- frequency values: **32 + 1 staging register**.

See [`axi_tag_time.v:157–189`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_tag_time.v#L157-L189), [`axi_fifo.v:15–25`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/fifo/axi_fifo.v#L15-L25), and [`axi_setting_reg.v:64–87`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/control/axi_setting_reg.v#L64-L87).

The DUC does not backpressure control on fullness. Unaccepted timestamps are lost; writes overwrite a blocked staging value. The unequal buffering can therefore destroy **time/value correspondence**, not merely coalesce updates.

**INFERRED:** TX R1 prevents sampleless accumulation only if “running” means actual continuing sample coverage—not a future booked burst. It misses stopped, starved, refused or subsequently shortened bursts unless released commands are tracked through recovery. Already-consumed samples also cannot execute a newly arriving command at its intended instant.

**Bench:** P4–P7/D70 do not establish DUC overflow depth or recovery.

### 8. R2′ and timed RX stop — VERIFIED, conditional sufficiency

DDC timestamp dequeue and frequency-value consumption require sample transfers ([`axi_tag_time.v:207–211`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_tag_time.v#L207-L211), [`ddc.v:118–142`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/ddc.v#L118-L142)). P4 A/C confirms settings survive early stop/reopen.

**Wall-clock passage beyond the latest instant is insufficient.** Samples must actually traverse the chain, with enough transfers to consume multiple queued commands, including equal-time commands.

X300 explicitly **does not support timed STOP**; STOP bypasses its command FIFO ([`radio_rx_core.v:171–183,524–527`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v#L171-L183)). Therefore defer issuing STOP until consumption is established; keep receiving/discarding past the logical cut. Overflow, finite acquisition ending early and device loss need explicit handling.

## Required changes to note 39 §6

Keep T1–T3, but qualify successful termination, separate radio recovery from DUC timeline reset, and make T3 an ordered initialization—not a FIFO drain. Rewrite R2′ around **actual command consumption**, not merely stop time.

No edits, builds, tests or hardware operations performed. Universal T2 safety and exact deployed-image equivalence remain unverified.
