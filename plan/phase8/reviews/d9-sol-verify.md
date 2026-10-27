# D9 の調査の Sol 6.1 による裏取り（2026-10-27）

依頼文は orchestrator の作業 dir の `sol-d9-verify-brief.md`（Opus の調査 [research/d9-production-investigation.md](../research/d9-production-investigation.md) の要点を UHD 4.10 の source で確かめる）．

### 1. F3′ — CONFIRMED, with qualifications

The timestamp-loss mechanism is correct:

- [`axi_rate_change.v:396–433`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_rate_change.v#L396-L433) initializes output time from the first header, latches `has_time_clear` when an untimed input appears, and clears output `has_time` at an output-packet boundary. Subsequent timed inputs do not restore it within that burst.
- [`axi_tag_time.v:207–230`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_tag_time.v#L207-L230) requires `has_time` for timed-command tagging and timestamp dequeue.
- Production timestamps only device SOB (`crates/ezsdr-radio-uhd/src/provider/tx.rs:434`); continuation tails and closing sends are untimed (`:442`, `:579`). UHD preserves timestamps across fragments of a timed send ([`tx_streamer_impl.hpp:297–315`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/transport/tx_streamer_impl.hpp#L297-L315)).

Thus a **pending DSP change** can remain unapplied through the remainder of the burst while radio tuning proceeds independently.

Qualifications: “first ~2 ms” is not an exact boundary; chunk size is computed at `tx.rs:404`, and clearing depends on pipeline/packet boundaries. Also, not every AUTO/AUTO frequency change writes DSP: unchanged cached DSP frequencies can suppress the write.

### 2. F7 — CONFIRMED mechanism; UNCLEAR causal attribution

The stale-header mechanism is real:

- `axi_tag_time.v:114–134` does not reset `vita_time_now`; `:229–230` generates tags without checking sample validity.
- [`axi_fifo_flop.v:21–37`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/fifo/axi_fifo_flop.v#L21-L37) clears validity, not stored header data.
- [`dds_timed.v:349–366`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/dds_timed.v#L349-L366) can consume a timed value from that tag without requiring valid IQ data. Timestamp dequeue, conversely, requires a sample transfer.

Graph initialization flushes/resets blocks and requests time zero ([`rfnoc_graph.cpp:108–109,714–722,985–998`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_graph.cpp#L985-L998)); `noc_shell_duc.v:155–164` propagates reset into CE. Production also explicitly zeros time (`authority.rs:372`).

**This is the strongest source-backed explanation found for P2’s early shift**, consistent with P1’s later timestamps and P2’s untimed tail before its second case. It is not experimentally isolated. Moreover, “−19 ms” is the first inspected window (`tests/hardware.rs:1021`), not a measurement proving the exact switching instant.

### 3. F1 paths A/B and H1/H7 — CONFIRMED, except overflow semantics

Both production paths exist:

- `control.rs:516–523` checks layout membership and `halted`, not RX `on`. Stop changes `on=false` without setting `halted` (`crates/ezsdr-radio/src/timeline.rs:941–944`).
- `control.rs:532–538` targets later planned segments.
- `control.rs:578–585,607–608` expires bookings by wall/device time, not FPGA consumption.

DDC backpressure is explicit: [`rfnoc_block_ddc.v:298–319`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_ddc/rfnoc_block_ddc.v#L298-L319) makes acknowledgment depend on timestamp-FIFO space. Without samples, that can wedge control.

DUC does not use this backpressure (`rfnoc_block_duc.v:224`). **However, “overflow is mostly harmless/latest wins” is not a reliable contract.** Its timestamp and value queues have different buffering; timestamps can be dropped while `axi_setting_reg` overwrites a pending value ([`axi_setting_reg.v:55–87`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/control/axi_setting_reg.v#L55-L87)). Command/value timing correspondence can therefore break.

### 4. H13/R1/R4 — mixed verdict

**H13: CONFIRMED conditionally.** AUTO’s signed RF residual and MANUAL’s clipping match [`multi_usrp_rfnoc.cpp:692–713`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L692-L713). Equivalence requires reproducing the same actual RF and representable DSP value.

**R1 sufficiency: REFUTED.** Its exception for an already-configured future segment still feeds a sampleless DDC. Bookings expire before that segment starts, so successive batches can exceed FPGA capacity while never exceeding 16 outstanding bookings. A 50 ms start lead does not bound occupancy. Configure/start refusal or a later cut also invalidates the promised sample coverage.

**R4: CONFIRMED for timestamp continuity, not yet as an unconditionally safe complete fix.** Timestamp each data-bearing send at its actual cursor, including held tails. Radio ignores continuation timestamps while transmitting, but rechecks after underrun; late data enters error-policy dropping ([`radio_tx_core.v:337–409`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L337-L409)).

Do not blindly timestamp emergency empty EOBs: a late EOB can be dropped without the normal transmit-state BurstAck. Distinguish recovery termination from data-bearing tails.

### 5. Material corrections

- Replace “3 ms is sufficient” with “worked in one 1 MS/s case.” Different rates, buffering and load remain unbounded by that observation.
- Remove R1(b)’s claimed queue-depth guarantee; coordinate deferred updates with actual pre-start configuration.
- Treat DUC overflow as corruption of command/value correspondence, not benign coalescing.
- Make production-failure claims conditional on an actual changed DSP register write; the DSP-only probes deliberately force such writes.
- R4 does not resolve stale-idle tags, accumulated commands, or commands arriving after their samples were processed.

No edits, builds, tests, Docker or hardware operations performed. Exact deployed-image equivalence and F7 attribution remain unverified.
