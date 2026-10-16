# Phase 1 spec 02 — Stream Contract

| Field | Value |
|---|---|
| Status | Accepted 2026-09-23 (Gate A; Phase 1 Step 5). Normative for `ezsdr-kernel::stream` and `::contract`. Its amendments are listed in the closing Changes table. |
| Scope | The DataContract registry and Port; `MemoryDomainId` and BufferRef; SampleBlock, its flags and its construction invariants; DataLink identity and back-pressure policy; the TX burst state machine and late policy; the derivation of ContinuityMap and ValidityMap. |
| Not in scope | Block pools and real link implementations (spec 10); the MockRadio device model (spec 09); TimingEnvelope values, which supply `min_lead` (the Radio Model, spec 07); the SigMF writer; `pdu.*` and `tensor.*` contracts, registered later; their Vocabulary specs define what SC-33 leaves to them. |
| Vision § covered | §21; §22's burst and late-policy parts; §23 in full, including the RX rules 1–8, the TX rules 1–5, the overflow paragraph and the ContinuityMap paragraph; §28; §30's link policy; §31's `MemoryDomainId` identity and BufferRef (domain kinds are Vocabulary content); §34's wire-format note; §46's TX-tap note; §17's fault-equivalence requirement. |
| Audit §14.1 items | 3 in full (F3, F21); 9 in full (F9); 12 in part (link drop counters, F16). Also F28 (taint as a convention) and F29 (Probe is not a Kernel concept). |
| Re-review | R12 (language-neutral shapes). |
| Depends on | Spec 01 for `TimePoint`, `Duration`, `ClockDomainId`, `TimeError`, and TM-21 for comparing durations across domains. |
| Modal verbs | "must" and "must not" are the normative verbs (OV-4a). "Should" does not appear inside a rule. |

---

## 1. Purpose

Vision §23 makes the stream normative rather than conventional, because v3 proved what happens otherwise: its UHD bridge discarded the receive error code and the receive timestamp, so an overflow produced a silently gapped, untimed array and the user was left to detect drift by hand. This document fixes the block header, the flag set, what a producer guarantees, what it is forbidden to do, and the two helpers that must exist in the Kernel so that Mock and hardware cannot diverge: the TX burst tracker and the continuity builder.

The audit's argument for settling this before MockRadio is written (Finding 3) is that the block type appears in every Provider, Executor and Sink signature. A Mock that publishes zero-filled gaps, or a contract that hands out host slices, cannot be corrected later without breaking every Module.

## 2. Evidence

Verified in the audit (Finding 3, Finding 21, Appendix B) unless marked as v3 evidence.

- UHD returns **zero samples** on an error. `OVERFLOW` covers both a device-side buffer overrun and a host-side sequence error, distinguished by `out_of_sequence`. A continuous receive restarts about **50 ms** after an overflow (`OVERRUN_RESTART_DELAY` = 0.05 s), so one overflow means a gap of at least that length. `ALIGNMENT` errors and single-channel loss exist, which is why validity must be per channel.
- Blocks arrive packet-sized: about **2 000 samples** at 10 GbE with `sc16`. Block length is a transport artefact, not a contract.
- UHD resets the CORDICs on **every start of burst**, so every start of burst needs a time specification, and an unclosed burst followed by a timed packet is a `TIME_ERROR`.
- The asynchronous message queue holds **1 000** entries, which is why event bodies are lossy while counters are not (spec 04).
- **v3 evidence.** `v3/cpp/uhd_usrp/multiusrp.cpp:595-596, 613-622, 640-642`: `start_of_burst` is set once, cleared after the first successful send, and `end_of_burst` is sent only as a zero-sample send at stop, so a cyclic transmission is one unbounded burst. `v3/cpp/uhd_usrp/multiusrp.cpp:698-702`: the receive path returns only the sample count. `v3/source/device/package.d:111-126`: the waveform loop sends the tail from the current offset to the end of the buffer and wraps with a modulo, so blocks shrink monotonically toward the wrap and the wrap is a block boundary; `get_max_num_samps` is never consulted anywhere in v3. `v3/source/controller/cyclicrx.d:84-145`: a capture is serviced only when every channel's buffer is full, so captures start at multiples of `alignSize`.

## 3. Model

```text
producer                                                            consumer
  Provider RX / Executor / a TxBurst                                  Executor / Provider TX / Sink
  builds SampleBlock { header, BufferRef } in its own pool
        │  publish(BlockRef)                 receive()
        └────────────────▶ DataLink { policy, capacity } ─────────────▶
  header names: a TimePoint in the stream's SampleClock, len, channels,
                a per-channel validity mask, flags, an optional lost count,
                the DataContract id
  payload:      BufferRef { memory_domain, opaque handle, len_bytes };
                host bytes only through the block's `host_bytes()` (SC-8)

TX consumers wrap blocks with BurstTracker  → start/end of burst, contiguity, records
Sinks wrap blocks with ContinuityBuilder    → ContinuityMap per SampleClock → Manifest, SigMF
```

## 4. Types

Language-neutral shapes (normative):

```text
DataContractId   a namespaced identifier matching ^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$   e.g. ezsdr.stream.cf32
Scalar           spec 03's Scalar (SB-4)
DataContract     { id: DataContractId, attributes: map<string, Scalar>, compatible_from: set<DataContractId> }
Port             { name: Ident, direction: In | Out, contract: DataContractId }
PortRef          { component: Ident, port: Ident }

MemoryDomainId   { node: NodeId, local: unsigned 32-bit }
BufferRef        { memory_domain: MemoryDomainId, handle: unsigned 64-bit (opaque), len_bytes: unsigned 64-bit }

ChannelMask      unsigned 64-bit; bit c set means channel c is valid in this block
Direction        Rx | Tx                      taken from the producing Port at construction
BlockFlags       unsigned 16-bit:
                 GAP_BEFORE 0x0001 | SEQ_DISCONTINUITY 0x0002 | RESTARTED 0x0004 | LATE 0x0008
                 | PARTIAL_CHANNELS 0x0010 | START_OF_BURST 0x0020 | END_OF_BURST 0x0040
                 | ALIGNMENT 0x0080
                 bits 8–15 reserved and must be zero
SampleBlock      { first_sample_time: TimePoint,        in the stream's SampleClock for a sample-stream contract (SC-33)
                   len: unsigned 32-bit ≥ 1,            samples per channel for a sample-stream contract (SC-33)
                   channels: unsigned 16-bit in 1..=64,
                   direction: Direction,
                   valid: ChannelMask,
                   flags: BlockFlags,
                   lost: optional unsigned 64-bit ≥ 1,
                   contract: DataContractId,
                   buffer: BufferRef,
                   host: optional shared bytes }        attached by new_host and read by host_bytes (SC-8)
BlockRef         a shared, immutable reference to a SampleBlock

BackPressure     Block | DropOldest | DropNewest
DataLinkId       { node: NodeId, local: unsigned 32-bit }
DataLinkDecl     { id: DataLinkId, from: PortRef, to: PortRef, contract: DataContractId,
                   policy: BackPressure, capacity: unsigned 32-bit ≥ 1 }
PublishOutcome   Accepted | DroppedOldest | DroppedNewest | Full
DropCarry        { flags: BlockFlags, lost: optional unsigned 64-bit, blocks: unsigned 32-bit }
                 what a drop-class link discarded between two delivered blocks (SC-20b)

LatePolicy       RejectAtPlan | SendAsapAndFlag | DropAndFlag
LateOutcome      OnTime | SendAsap { late_by: Duration } | Drop { late_by: Duration }
                 | PlanViolation { late_by: Duration }
BurstState       Idle | InBurst { target: TimePoint, expected_next: TimePoint, blocks, samples }
BurstStep        Started | Continued | Ended { record: BurstRecord }
                 | Discontinuity { expected: TimePoint, got: TimePoint, closed: BurstRecord,
                                   then_ended: optional BurstRecord }
                 then_ended: the burst this block opened, when the same block also carried END_OF_BURST and so ended it (SC-24)
BurstEnd         Eob | Stop | Discontinuity
BurstOpen        { waveform_len: optional unsigned 32-bit, late: optional LateOutcome,
                   requested_target: optional TimePoint }
                 supplied by the Provider on the block that opens a burst; the tracker cannot know any of them
BurstRecord      { target: TimePoint, requested_target: optional TimePoint, actual_start: optional TimePoint,
                   blocks, samples, wraps, late_by: optional Duration, end: BurstEnd }

GapCause         Stream | OverflowRestart | SequenceError | Alignment | LinkDrop
                 | Mixed { stream_lost } | Unknown
Gap              { start: TimePoint, len: unsigned 64-bit, lost: optional unsigned 64-bit,
                   cause: GapCause, link_dropped: unsigned 32-bit }
                 cause and lost describe what the stream lost; link_dropped counts the blocks the
                 link discarded inside the same interval.
                 link_dropped is a block count, not a sample count: the samples the link lost are
                 jump − lost, and are unrecoverable when lost is absent.
ChannelGap       { channel: unsigned 16-bit, start: TimePoint, len: unsigned 64-bit }
Segment          { start: TimePoint, len: unsigned 64-bit }
ContinuityMap    { domain: ClockDomainId, channels, valid: per channel a list of Segment,
                   gaps: list of Gap, channel_gaps: list of ChannelGap,
                   first: TimePoint, end: TimePoint }
StreamError      Time(TimeError)                 from spec 01; carries DomainMismatch and Overflow
                 | MissingStartOfBurst | TimeOverlap { expected, got }
                 | GapFlagWithoutJump | JumpWithoutGapFlag | InvalidBlock { reason }
                 | DomainChanged { from, to } | ChannelsChanged { from, to } | Incompatible { from, to }
```

Illustrative Rust sketch, not normative:

```rust
pub struct SampleBlock { header: BlockHeader, buffer: BufferRef, host: Option<HostBytes> }   // private fields
struct HostBytes(std::sync::Arc<[u8]>);   // host bytes a producer attached; they live as long as the block (SC-8, SC-9)
impl SampleBlock {
    // bytes_per_sample comes from the producing Port's DataContract, resolved at prepare
    pub fn new(h: BlockHeader, buffer: BufferRef, bytes_per_sample: u32)
        -> Result<SampleBlock, StreamError>;                                  // enforces SC-10
    pub fn new_host(h: BlockHeader, memory_domain: MemoryDomainId, bytes: std::sync::Arc<[u8]>,
                    bytes_per_sample: u32) -> Result<SampleBlock, StreamError>;   // SC-8, SC-10a
    pub fn host_bytes(&self) -> Option<&[u8]>;    // the bytes new_host attached; None after new (SC-8)
    pub fn first_sample_time(&self) -> TimePoint;
    pub fn end_time(&self) -> Result<TimePoint, TimeError>;   // first + len, checked
    /* getters only: no mutation API exists */
}
pub type BlockRef = std::sync::Arc<SampleBlock>;

pub trait DataLink: Send + Sync {
    fn publish(&self, b: BlockRef) -> PublishOutcome;
    fn receive(&self) -> Option<(BlockRef, DropCarry)>;   // SC-20b: with what was dropped just before it
    fn drops(&self) -> u64;              // a never-dropping counter, read by the event collector
    fn take_drop_carry(&self) -> DropCarry;   // SC-20b: everything dropped after the last receive
    fn policy(&self) -> BackPressure;
}
pub struct BurstTracker { /* domain, open burst */ }
impl BurstTracker {
    pub fn on_block(&mut self, h: &BlockHeader, open: Option<BurstOpen>)
        -> Result<BurstStep, StreamError>;
    pub fn set_late(&mut self, late: LateOutcome);      // the late outcome of a burst opened by a time jump in a block without START_OF_BURST (SC-29a)
    pub fn set_actual_start(&mut self, t: TimePoint);   // device feedback, when the Provider has it
    pub fn stop(&mut self) -> Option<BurstRecord>;
}
impl LatePolicy {
    pub fn decide(self, registry: &ClockRegistry, target: TimePoint, now: TimePoint, min_lead: Duration)
        -> Result<LateOutcome, TimeError>;
}
pub struct ContinuityBuilder { /* domain, lossless flag, expected_next, open segments, gaps */ }
impl ContinuityBuilder {
    pub fn push(&mut self, h: &BlockHeader, carry: DropCarry)
        -> Result<(), StreamError>;      // SC-30c: a rejected push keeps the carry
    pub fn finish(self, carry: DropCarry) -> ContinuityMap;
}
```

## 5. Normative rules

### Contracts and ports (Vision §21)

- **SC-1** A `Port` is a name, a direction and a `DataContractId`, and nothing more. The name is an `Ident` (spec 03 SB-1), as is each half of a `PortRef`. Per-port parameters the Kernel does not interpret — a tensor shape, a maximum PDU length — live in the `ComponentDescriptor`'s parameters (spec 05), not in the Port.
  *Test:* `sc_01_port_shapes_are_pinned_by_the_schema_freeze` (`Port` has exactly three properties and `PortRef` exactly two); a `PortRef` half that is not an `Ident`, refused at parse by spec 03's `sb_01_every_name_grammar_is_enforced_at_the_document_boundary`.
- **SC-2** A `DataContract` is registered under a namespaced id with a fixed attribute map and a set `compatible_from` of producer contracts it accepts without conversion. Registering an identical definition twice is a no-op; registering a different definition under an existing id fails. "Identical" is exact: an `Int` and a `Num` attribute are one value **iff they are numerically equal**, by SB-6's equality on `Scalar`.
  *Test:* `sc_02_scalar_equality_is_exact_across_int_and_float`, `sc_02_contract_registry_conflict`.
- **SC-3** A link from a producer contract P to a consumer contract C is admissible if and only if P equals C, or P is a member of `compatible_from` for C. The check is directional, and it is the only contract check the Kernel performs. (Vision §21: "the Kernel does not become a type system".)
  *Test:* `sc_03_contract_identity_or_compat`.
- **SC-4** The contracts registered at v4.0, whose definitions belong to the contracts Vocabulary and serve as test fixtures, are `ezsdr.stream.cf32 { bytes_per_sample: 8, full_scale: 1.0, layout: "planar" }`, `ezsdr.stream.sc16 { bytes_per_sample: 4, full_scale: 32767, layout: "planar" }` and `ezsdr.control {}`. All have an empty `compatible_from`. `full_scale` is the converter full-scale convention of Vision §23 rule 4, and `layout: "planar"` means channel c occupies the byte range `[c · len · bytes_per_sample, (c+1) · len · bytes_per_sample)` of the buffer.
  *Test:* `sc_04_standard_contracts_fixture`, for cf32 and sc16; none named for `ezsdr.control`.
- **SC-5** The host-to-device wire format (`sc16`, `sc8`, `sc12`) is never a Port contract. The Provider converts and reports it in its PerformanceEnvelope. Here `sc16` means the device wire format, distinct from SC-4's `ezsdr.stream.sc16` host-memory DataContract. (Vision §34.)
  *Test:* on MockRadio, `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section` (MR-3).
- **SC-33** SC-12, SC-13, SC-18 and SC-23…SC-32, and the receive and transmit meanings SC-16 gives `direction`, apply to the blocks of a sample-stream contract (`ezsdr.stream.*`). For the blocks of another contract, that contract's Vocabulary spec defines the domain of `first_sample_time`, the unit of `len` and the meaning of `direction`, and those blocks carry no gap flag (`GAP_BEFORE` and its qualifiers) and no burst flag (`START_OF_BURST`, `END_OF_BURST`). SC-10, SC-10a, SC-11, SC-14, SC-15 and SC-19…SC-22 apply to every block.
  *Test:* none named.

### Buffers and memory domains (Vision §23 rules 5 and 6, §31)

- **SC-6** `MemoryDomainId`s are node-qualified, with `node = LOCAL` in v4.0; the Kernel compares ids and never interprets a domain kind. A kind such as host, pinned or GPU is Vocabulary content, not a Kernel type.
  *Test:* `sc_06_memory_domain_id_is_node_qualified`.
- **SC-7** A `BufferRef` is a memory domain, an opaque handle and a length in bytes. The handle is meaningful only to the owner of that memory domain — the producing pool, or the link that carries the domain. No Kernel interface dereferences a handle.
  *Test:* `sc_07_unsafe_code_remains_forbidden`: the `kernel_surface` gate asserts `#![forbid(unsafe_code)]` remains present.
- **SC-8** Host bytes are reached only through `SampleBlock::host_bytes()`, which returns the bytes a producer attached with `SampleBlock::new_host` and `None` for a block built with `SampleBlock::new`. The bytes are shared with the block and live while any reference to the block does; a pool recycles a buffer only when no block references it (HD-2). Which memory domains are host-reachable is Vocabulary content (SC-6), so the Kernel cannot refuse `new_host` for a domain that is not; attaching host bytes to such a block is a producer defect. No role-trait method takes or returns a byte slice (MA-6): `host_bytes` is a method of a block, reached through a `BlockRef` a link delivered.
  *Test:* `sc_08_host_bytes_only_for_a_block_that_carries_them`.
- **SC-9** A block's buffer remains valid while any reference to the block exists, and its owner must not reuse it earlier. Producers allocate blocks from their own Island's pool, and the real-time path performs no allocation. The pool is a shared helper (spec 10, HD-2).
  *Test:* spec 10's `hd_02_a_released_slot_is_reused_and_a_held_one_is_not` (HD-2); none named for the allocation rule, whose check is Vision §61's copy-regression benchmark.

### Blocks (Vision §23 rules 1–4)

- **SC-10** The constructor enforces, and rejects with `InvalidBlock` otherwise: `len ≥ 1`; `1 ≤ channels ≤ 64`; no bit of `valid` set at or above `channels`; reserved flag bits zero; `lost` present only together with `GAP_BEFORE` and at least 1; `RESTARTED`, `SEQ_DISCONTINUITY` or `ALIGNMENT` implies `GAP_BEFORE`; `START_OF_BURST` and `END_OF_BURST` never together with `GAP_BEFORE`, `RESTARTED` or `SEQ_DISCONTINUITY`; and the direction rules of SC-16 and SC-16a. `PARTIAL_CHANNELS` is derived by the constructor from the mask, and passing it as input is an error.
  *Test:* `sc_10_block_rejects_invalid_shape`, `sc_10_block_flag_implications`, `sc_10_block_partial_channels_derived`, `sc_16_direction_flags_rejected`; none named for `SEQ_DISCONTINUITY`'s implication of `GAP_BEFORE`, or for an `END_OF_BURST` block with `GAP_BEFORE`, `RESTARTED` or `SEQ_DISCONTINUITY`.
- **SC-10a** The constructor also enforces `buffer.len_bytes ≥ channels · len · bytes_per_sample`, where `bytes_per_sample` is passed in from the producing Port's DataContract.
  *Test:* `sc_10a_block_rejects_undersized_buffer`; through `new_host`, `sc_08_host_bytes_only_for_a_block_that_carries_them`.
- **SC-11** A block is immutable once published: the type has no mutation interface and is shared by reference. Fan-out to N consumers is N links carrying the same reference; nothing is copied.
  *Test:* `sc_11_block_fanout_shares_reference`; immutability by the type definition, which has no mutation method.
- **SC-12** Time is monotonic within one SampleClock. On a lossless path, for consecutive blocks either the next block's first sample time equals the previous block's end, or it is later and `GAP_BEFORE` is set. Earlier than the previous block's end is a contract violation (`TimeOverlap`). A block whose domain differs from its predecessor's is a rate change (spec 01, TM-13c), not a gap. Gaps are never filled, with zeros or with repeated data.
  *Test:* `sc_30_continuity_contiguous`, `sc_13_continuity_gap_before_known_lost`, `sc_12_continuity_overlap_is_error`; the domain clause by `sc_13_tm_13c_rate_change_is_a_new_domain_not_a_gap`.
- **SC-13** `GAP_BEFORE` is set if and only if, as far as the producer knows, samples are missing between the previous block's end and this block's first sample — on a stream's first block, between the stream's origin and its first sample, as a late receive start has it (RM-25). `lost` carries the count when known and is absent when unknown. When known, `lost` equals the time jump on a lossless path, on a first block its first sample's index, and is at most the jump behind a drop-class link, where the difference is link loss. `RESTARTED` qualifies the gap as a stream restart, `SEQ_DISCONTINUITY` as transport sequence loss, `ALIGNMENT` as samples the device discarded to keep its channels aligned. Every v4.0 producer is timestamped and therefore always knows `lost`.
  *Test:* on MockRadio, `mr_21_overrun_shape` (MR-21) and `mr_22_sequence_error_shape` (MR-22); on the UHD Provider, spec 18's `ur_17_a_missed_start_restarts_with_a_gap` and `ur_19_reports_before_one_block_keep_each_kind`; in the continuity builder, `sc_13_a_first_block_gap_is_recorded`, `sc_13_continuity_gap_before_known_lost`, `sc_13_continuity_gap_flag_without_jump`, `sc_31_continuity_mixed_on_lossy`, `sc_31_alignment_qualifies_a_stream_gap`, `sc_30b_a_held_carry_keeps_its_flags_and_lost_count`, `sc_30b_a_first_block_s_carry_is_recorded_at_it`.
- **SC-14** Validity is per channel and constant within a block: a change of validity is a block boundary, and the producer splits. `PARTIAL_CHANNELS` is set exactly when `valid` is not the full mask for `channels`.
  *Test:* `sc_10_block_partial_channels_derived`, `sc_14_continuity_per_channel_segments`.
- **SC-15** Block length is not guaranteed. A consumer must not assume a fixed length, a minimum, or an alignment.
  *Test:* on MockRadio, `mr_12_block_lengths` (MR-12); in the capture Sink, spec 10's `hd_10_capture_of_n_samples_across_jittered_blocks` (HD-12).
- **SC-16** A block carries its `direction`, taken from the producing Port. A receive block must not carry `START_OF_BURST` or `END_OF_BURST`; a transmit block must not carry `GAP_BEFORE`, `RESTARTED` or `SEQ_DISCONTINUITY`. These are constructor checks under SC-10.
  *Test:* `sc_16_direction_flags_rejected`, `sc_10_block_flag_implications`; none named for a receive block carrying `END_OF_BURST` (nor for a transmit block carrying `RESTARTED` or `SEQ_DISCONTINUITY`).
- **SC-16a** `LATE` is a transmit flag: a receive block carrying it is refused under SC-10. A receive stream that starts late carries SC-13's first-block gap.
  *Test:* `sc_16_direction_flags_rejected`; the late start, on the UHD Provider, spec 18's `ur_17_a_missed_start_restarts_with_a_gap`.
- **SC-17** Flag bit positions are fixed by this document; how an Executor propagates them is MA-23's. (Vision §28.)
  *Test:* `sc_17_flag_bit_positions_are_fixed_by_the_document`.
- **SC-18** How an overflow appears. A UHD overflow — zero samples returned, the stream restarting about 50 ms later — appears as the next block carrying `GAP_BEFORE` and `RESTARTED`, with `lost` equal to the time jump, and with that jump equal to the restart gap. A sequence error appears as `GAP_BEFORE` and `SEQ_DISCONTINUITY` without `RESTARTED`. Zero-length blocks are never published. An injected fault must produce identical flags and jump.
  *Test:* on MockRadio, `mr_21_overrun_shape` (MR-21) and `mr_22_sequence_error_shape` (MR-22); in the acceptance suite, `v58_06_injected_overflow_is_a_uhd_overflow` (MR-21); the zero-length prohibition by SC-10's `sc_10_block_rejects_invalid_shape`.

### Links (Vision §23 rule 8, §30)

- **SC-19** Every DataLink is declared with a policy and a capacity in blocks. There is no default policy.
  *Test:* `sc_19_link_declares_a_policy_and_a_capacity`.
- **SC-20** Under `Block`, publishing returns `Full` when the capacity is reached, and nothing is ever dropped. Under `DropOldest`, the oldest queued block is evicted and the outcome says so. Under `DropNewest`, the new block is refused and the outcome says so. A drop-class link never returns `Full`. The link's drop count is a never-dropping counter, incremented on every drop and readable by the event collector (Vision §29). The coordinator reads every created link's `drops()` at cleanup step 6 (`flush_events`), before any link is dropped, and records a typed `{ link, drops }` per link in the Manifest's `links`, whose endpoints are the link's entry in `plan.links` (KC-45).
  *Test:* `sc_20_link_block_policy_full`, `sc_20_link_drop_oldest`, `sc_20_link_drop_newest`; the Manifest's record by spec 06's `kc_45_manifest_fields` (link id, the `from` endpoint and zero count), `ka_18_nonzero_link_drops_reach_the_manifest` (nonzero propagation) and `ka_18_a_failed_link_drop_snapshot_is_unknown_not_zero` (unknown snapshot); none named for a Run whose Plan has more than one link.
- **SC-20a** `publish` never parks. `Block` names the back-pressure the policy produces, not a blocking call. A producer that receives `Full` **must not** discard the block silently; it retries on its next turn, or it stops and emits the typed `LINK_BACKPRESSURE` event for the Run's Policy table to act on.
  *Test:* `sc_20a_full_is_not_a_silent_drop`, on the in-memory link with a producer double; none named for `LINK_BACKPRESSURE`, whose kind is spec 04's.
- **SC-20b** A drop-class link accumulates what it discards between two delivered blocks into a `DropCarry`: the union of the dropped blocks' `GAP_BEFORE`, `RESTARTED`, `SEQ_DISCONTINUITY` and `ALIGNMENT` flags, the sum of the `lost` counts that were present, and how many blocks were dropped. `receive` returns each block with the carry of what was dropped immediately before it in stream order; `take_drop_carry` returns, and clears from the link, everything dropped after the last block `receive` returned, queued blocks' carries included.
  *Test:* `sc_20b_drop_carry_preserves_attribution` (the test link) and spec 10's `hd_05_a_carry_belongs_to_the_block_after_the_drop` (the HostLink).
- **SC-21** A link whose consumer port belongs to a Sink-role Module must be drop-class; `validate()` rejects `Block` there. The predicate is a Kernel function, called by the planner (specs 03 and 05). (Vision §30, invariant 21.)
  *Test:* `sc_21_sink_links_must_be_drop_class`.
- **SC-22** Fan-out is N links sharing block references (SC-11). A Probe is a drop-class link feeding a recorder Sink, and is not a Kernel concept. (Vision §30.)
  *Test:* `sc_11_block_fanout_shares_reference`; none named for the absence of a Kernel `Probe`, which the allow-list gate `ov_23_every_public_item_is_on_the_allow_list` does not assert (it only forces every new public item to be justified).

### The transmit side (Vision §22, §23 rule 7 and TX rules 1–5)

- **SC-23** Every transmit block carries the target time of its first sample in the transmit stream's SampleClock, in continuous mode as well. A tap on the transmit edge therefore yields timed reference samples with no special interface. (Vision §46.)
  *Test:* the type definition (`BlockHeader.first_sample_time` is not optional).
- **SC-23a** A target expressed in another **exactly related** domain is converted at admission with `apply` (TM-4). An `Inexact` result is advanced to the next transmit sample instant, `floor + 1`, and both times are recorded: the `BurstRecord` keeps `requested_target` alongside the applied `target`.
  *Test:* `sc_23a_tx_target_advances_to_next_sample`, `sc_23a_reactive_target_across_disjoint_grids`, `sc_23a_requested_target_reaches_the_record`.
- **SC-23b** A target in an **unrelated** domain is refused. The caller supplies a ClockRelation and converts deliberately, accepting the uncertainty in its own code.
  *Test:* `sc_23b_tx_target_unrelated_domain_refused`.
- **SC-24** A transmit stream is a sequence of bursts. A burst begins with a block carrying `START_OF_BURST` and ends with a block carrying `END_OF_BURST`; within a burst, each block's time equals the previous block's end. The state machine in §6 is normative, and the tracker's return value for a block reports **every** transition that block caused: a block whose time jumped and which also carried `END_OF_BURST` closes the open burst *and* opens and ends another, and says so. A block published while idle without `START_OF_BURST` is `MissingStartOfBurst` and must not be transmitted. A block whose time differs from the expected next time in either direction, or which carries `START_OF_BURST` while a burst is open, is a discontinuity: the tracker closes the open burst and reports it, and the block becomes the first block of a new burst. Nothing is padded.
  *Test:* `sc_24_burst_sob_eob_basic`, `sc_24_burst_single_block`, `sc_24_burst_missing_sob`, `sc_24_burst_forward_jump_is_discontinuity`, `sc_24_burst_backward_time_is_discontinuity`, `sc_24_burst_sob_inside_burst`, `sc_24_burst_domain_mismatch`, `sc_24_a_discontinuity_that_also_ends_reports_both_records`.
- **SC-24a** On a discontinuity the Provider emits the typed `TX_DISCONTINUITY` event and evaluates SC-27's late policy for the new burst before transmitting it. The kind is registered by the Radio Model, not the Kernel (RS-27).
  *Producer obligation:* the Provider's half of SC-24, tested against the first Provider with a transmit port fed by a link, which none has yet (MockRadio's transmit takes `TxBurst` Actions only, MR-16).
- **SC-25** Continuous transmission is one burst, ended by the Run's stop or by an explicit `END_OF_BURST`. An underflow is a typed `TX_UNDERFLOW` event carrying the time at which it occurred; the Provider fabricates no samples, and what the hardware radiated during the gap is recorded in the Provider's Manifest section.
  *Test:* `sc_25_burst_continuous_until_stop`; on the UHD Provider, spec 18's `ur_28_underflows_are_tx_underflow` (the underflow's `cause` and raw code) and `ur_22_underflow_and_loss_end_the_damaged_burst` (no further payload after an underflow: no samples fabricated); none named for the underflow's time or the Manifest's record of what was radiated.
- **SC-26** `repeat` is a burst attribute. A repeated burst of waveform length L is transmitted so that repetition w begins exactly at `target + w · L`, the block sequence stays contiguous across every wrap, and it must not underflow at the wrap. Whether the Provider uses a host loop or device memory is its choice, and the constraints of that choice (maximum length, word alignment) are Radio Model capabilities.
  *Test:* the tracker's contiguity across the wrap by `sc_26_burst_repeat_wrap_contiguous` and `sc_26_burst_repeat_wrap_off_by_one`; on MockRadio, `mr_16_repeat_is_contiguous_across_wraps` (MR-16); on the UHD Provider's FakeDevice, `ur_22_repeat_is_continuous_across_the_wrap` (no `TX_UNDERFLOW` at the wrap); on hardware, `hw_b6_txrx_and_repeat` (ignored, needs a USRP: the repeated burst is back to back through the loopback and records no `TX_UNDERFLOW`, the parity bench of Vision §59).
- **SC-27** Lateness. `LatePolicy::decide(registry, target, now, min_lead)`, where `registry` is the `ClockRegistry` that holds their domains, yields `OnTime` when `target − now ≥ min_lead`, and otherwise the outcome named by the policy. `target` and `now` are in one domain and their difference is a `Duration` there (TM-7). `min_lead` comes from the bound Provider's TimingEnvelope and is declared in `host.monotonic`, and TM-21 compares the two exactly by cross-multiplication with no rescale and no rounding. `RejectAtPlan` is legal only for a burst whose target is statically known: a Module's burst carrying it is refused at admission (spec 06 KC-24 step 7), a Session verb declaring it is refused at Vocabulary registration (spec 04 RS-14), and a scheduled burst carrying it is refused before `start` when its lead is shorter than its target's `min_command_lead` (spec 06 KC-19); if it is nevertheless reached at runtime, `decide` yields `PlanViolation` and the Provider must not transmit.
  *Test:* `sc_27_late_policy_decisions`, `sc_27_late_policy_domain_check`, `sc_27_min_lead_cross_multiplied`, `sc_27_min_lead_domain_is_checked_before_the_verdict`; the refusals of `RejectAtPlan`: a Module's burst by `kc_24_a_module_reject_at_plan_burst_is_refused`, a Session verb by `rs_14_a_reject_at_plan_session_verb_is_refused_at_registration`, and a scheduled burst with a short lead by `kc_19_a_reject_at_plan_burst_with_a_short_lead_is_refused`; the Provider not transmitting at runtime, by `mr_17_late_policy_outcomes` (a `RejectAtPlan` burst leaves no record and one rejection) and on the UHD Provider spec 18's `ur_21_first_dispatch_rechecks_all_late_policies` (no transmit call).
- **SC-28** What was transmitted is recorded: one `BurstRecord` per burst, carrying the target, the actual start where the Provider can supply it, the block and sample counts, the number of wraps, any lateness, and how the burst ended. The Provider puts these in its Manifest section; the envelope is spec 04.
  *Test:* `sc_24_burst_sob_eob_basic`, `sc_29a_wraps_counted_from_burst_open`, `sc_29a_late_and_actual_start_reach_the_record`; on MockRadio, `mr_16_repeat_is_contiguous_across_wraps` (MR-23).
- **SC-29** `BurstTracker` is the single implementation of the state in SC-24 to SC-28, and every Provider — Mock and UHD alike — routes its transmit blocks through it. The tracker owns the state, the contiguity arithmetic, and the fields of a `BurstRecord` it can derive from block headers alone: `target`, `blocks`, `samples` and `end`, and `wraps` from `samples` and SC-29a's `waveform_len`. It copies `requested_target` from the `BurstOpen` passed with a block carrying `START_OF_BURST` (SC-29a), so a burst opened by a block without `START_OF_BURST` has none. The Provider owns device input and output, including the zero-length end-of-burst send that closes a burst on UHD, the `now` and `min_lead` inputs, executing the late policy, and emitting events.
  *Test:* the tracker's half by the tests of SC-24…SC-29a; on MockRadio, `mr_16_repeat_is_contiguous_across_wraps` (MR-16, MR-23), `mr_17_late_policy_outcomes` (MR-17) and `mr_24_a_bypassing_provider_gets_time_error` (MR-24).
- **SC-29a** Four `BurstRecord` fields are not derivable from headers, so the Provider supplies them: `waveform_len`, without which `wraps` cannot be counted, `late`, which comes from `LatePolicy::decide`, and `requested_target`, what the caller asked for when the applied target differs (SC-23a), are passed in a `BurstOpen` on the block that opens the burst; `actual_start` arrives later as device feedback through `set_actual_start`. A burst opened by a time jump in a block without `START_OF_BURST` carries no `BurstOpen`, yet SC-24a still requires the late policy to be evaluated for it, so that outcome arrives through `set_late` before the next block.
  *Test:* `sc_29a_wraps_counted_from_burst_open`, `sc_29a_late_and_actual_start_reach_the_record`, `sc_29a_set_late_carries_a_discontinuity_opened_burst`, `sc_23a_requested_target_reaches_the_record`.

### Derivation (Vision §23's closing paragraph, §28)

- **SC-30** `ContinuityMap` and the per-channel validity it contains are derived by the Kernel's continuity builder from block headers alone, and never assembled by hand. There is one map per SampleClock: a block in another domain ends the map with `DomainChanged`, and the Sink starts a new builder. The builder is told whether its path is lossless; on a lossless path a jump without `GAP_BEFORE` is a contract violation, and on a drop-class path it is a link-drop gap. A first block carrying `GAP_BEFORE` is recorded with a `Gap` from `lost` samples before it (zero-extent at the block when `lost` is absent, or when a carry came with it, SC-30b), and the map's `first` is that gap's start (SC-13).
  *Test:* `sc_30_continuity_contiguous`, `sc_30_continuity_jump_without_flag`, `sc_30_continuity_domain_change_ends_map`, `sc_30_continuity_jump_overflow_is_time_error`, `sc_14_continuity_per_channel_segments`, `sc_13_a_first_block_gap_is_recorded`.
- **SC-30a** A block whose `channels` differs from the builder's ends the map with `ChannelsChanged`, exactly as a domain change does.
  *Test:* `sc_30a_channel_count_change_ends_map`, `sc_30c_carry_survives_a_rejected_push`.
- **SC-30b** A consumer behind a drop-class link passes the `DropCarry` that `receive` returned with a block (SC-20b) to the builder with that block's header; the builder merges its flags into the block's for the purpose of deriving a cause, sums the `lost` counts that are present, and records the dropped block count in the gap's `link_dropped`. A carry pushed with a stream's first block is recorded at that block, never in a later gap: in a zero-extent `Gap` at its first sample, whether or not the block carries `GAP_BEFORE`; like a trailing carry's (SC-30c), the gap keeps the summed `lost` and the block count.
  *Test:* `sc_30b_carry_merges_into_next_gap`, `sc_30b_a_carry_on_a_contiguous_block_is_not_discarded`, `sc_30b_a_carry_held_across_a_contiguous_block_reaches_the_trailing_gap`, `sc_30b_a_held_carry_keeps_its_flags_and_lost_count`, `sc_30b_a_first_block_s_carry_is_recorded_at_it`, `sc_30c_a_rejected_push_keeps_both_carries`, `sc_31_mixed_requires_that_no_carry_explains_the_shortfall`.
- **SC-30c** A carry that no delivered block follows is recorded by `finish` as a zero-extent `Gap` at `expected_next` — its `len` is 0 because the extent of what followed the last delivered block is unknown, not because nothing was lost; `lost` and the block count carry what is known — carrying the cause derived from its flags, its `lost` where present, and its block count; the map's `end` does not move, because no sample after the last delivered one is accounted for. A consumer whose recording ends passes `take_drop_carry` to `finish`. A rejected push keeps its carry in the builder, whose `finish` records it: after a rejection by SC-30 or SC-30a those samples belong to the domain and channel count that just ended, and the consumer finishes that builder with an empty carry.
  *Test:* `sc_30c_trailing_carry_is_zero_extent`, `sc_30c_trailing_carry_with_a_known_lost_stays_zero_extent`, `sc_30b_a_carry_held_across_a_contiguous_block_reaches_the_trailing_gap`, `sc_30b_a_first_block_s_carry_is_recorded_at_it`, `sc_30c_carry_survives_a_rejected_push`, `sc_30c_a_rejected_push_keeps_both_carries`.
- **SC-31** `GapCause` is a closed, derived set describing what the **stream** lost: `OverflowRestart` from `RESTARTED`; `SequenceError` from `SEQ_DISCONTINUITY`; `Stream` from `GAP_BEFORE` alone; `LinkDrop` from a jump with no gap flag on a lossy path; `Mixed` from `GAP_BEFORE` with `lost` less than the jump on a lossy path and no carry to explain the difference; `Unknown` from `GAP_BEFORE` with `lost` absent; `Alignment` from `ALIGNMENT`. Of the qualifiers, `RESTARTED` takes precedence over `SEQ_DISCONTINUITY`, which takes precedence over `ALIGNMENT`. What the **link** lost is the separate `link_dropped` count, not a cause, so a device overflow inside an interval that also lost a block to the link is still reported as `OverflowRestart` with its sample count, beside a link-drop count of one.
  *Test:* `sc_31_continuity_causes`, `sc_31_alignment_qualifies_a_stream_gap`, `sc_31_continuity_mixed_on_lossy`, `sc_31_mixed_requires_that_no_carry_explains_the_shortfall`, `sc_30b_carry_merges_into_next_gap`; `LinkDrop` by `sc_30_continuity_jump_without_flag`; none named for `RESTARTED` together with `SEQ_DISCONTINUITY`, the one precedence pair with no case of its own.
- **SC-31a** *Withdrawn.* A `ChannelGap` carries no cause: an alignment failure discards samples on every channel, a stream gap qualified by `ALIGNMENT` (SC-13, SC-31; spec 18 UR-19).
- **SC-31b** A per-channel break caused by a **stream** gap produces no `ChannelGap`.
  *Test:* `sc_31b_stream_gap_emits_no_channel_gaps`.
- **SC-31c** A channel that was valid somewhere in the map and is invalid at its end produces a `ChannelGap` running to the map's end. A channel that was never valid produces nothing.
  *Test:* `sc_31c_channel_that_never_returns`, `sc_31c_channel_lost_at_the_overflow`, `sc_31c_never_valid_channel_has_no_gap`.
- **SC-31d** A channel that loses validity while the stream continues produces a `ChannelGap`, not merely a hole between two `Segment`s. A channel's break is held as a pending close and emitted only when its extent is known: when the channel returns, when a stream gap ends it early, or at `finish`. A break that coincides with a stream gap is recorded from the gap's end, not from the channel's last valid sample.
  *Test:* `sc_31d_a_channel_break_is_a_channel_gap`, `sc_31d_break_across_a_stream_gap_is_split`, `sc_31c_channel_lost_at_the_overflow`.
- **SC-32** A SigMF export maps each run of delivered samples between stream gaps — the stream's valid segments — to a capture segment with `core:sample_start` and `core:global_index`, and carries the gaps and per-channel validity in an `ezsdr` extension namespace; a SigMF capture segment belongs to the whole Recording, so a channel's own breaks are annotations, not segments.
  *Test:* spec 10's `hd_15_a_capture_is_a_sigmf_recording` (two capture segments around a stream gap, the `ezsdr:gaps` entry and the per-channel `ezsdr:valid`) and `hd_15_channel_validity` (a channel's own breaks as annotations beside one capture segment, `ezsdr:valid` per channel); none named here for a capture with more than one ContinuityMap, which spec 10's `hd_15_a_capture_across_a_clock_change_has_no_metadata` records as a `.sigmf-data` Recording without metadata (HD-15).

## 6. Algorithms

**Burst tracker** (SC-24). `t` is the block's first sample time, `L` its length, `E` the expected next time.

| state | input | next state | step |
|---|---|---|---|
| Idle | start of burst, not end | InBurst { target t, E = t + L } | `Started` |
| Idle | start and end together | Idle | `Ended { Eob }` (a single-block burst) |
| Idle | neither | Idle | error `MissingStartOfBurst`; the block is not transmitted |
| InBurst | t = E, no start, no end | InBurst { E += L } | `Continued` |
| InBurst | t = E, end of burst | Idle | `Ended { Eob }` |
| InBurst | t ≠ E, or a start of burst | InBurst { target t, E = t + L }, or Idle if this block also ends | `Discontinuity { expected E, got t, closed record, then_ended }`, `then_ended` being the new burst's record (end `Eob`) if this block also ends, and absent otherwise; the Provider emits `TX_DISCONTINUITY` and evaluates SC-27 for the new burst |
| InBurst | stop | Idle | `Ended { Stop }` |
| any | the block's domain differs from the tracker's | unchanged | error `DomainMismatch` |

**Continuity derivation** (SC-30).

```text
push(h, carry):                            -- carry is what receive returned with h, empty on a lossless path
  pending_carry += carry                                                  -- SC-30c: kept if rejected
  if h.domain ≠ domain            -> DomainChanged
  if h.channels ≠ channels        -> ChannelsChanged                      -- SC-30a
  flags = h.flags OR pending_carry.flags                                  -- SC-30b
  lost  = sum of whichever of h.lost and pending_carry.lost are present   -- absent only if neither is
  E = expected_next, or on the first block h.t − (h.lost or 0) if GAP_BEFORE in h.flags and pending_carry is empty, else h.t  -- SC-13, SC-30b
  if h.t < E                      -> TimeOverlap
  if h.t = E and GAP_BEFORE in h.flags and not the first block -> GapFlagWithoutJump
  if h.t > E, or h.t = E and (GAP_BEFORE in h.flags or (first block and pending_carry not empty)):  -- SC-13, SC-30b
     jump = h.t − E                                                       -- checked; overflow -> Time(..)
     if GAP_BEFORE not in flags and lossless -> JumpWithoutGapFlag
     cause = GAP_BEFORE not in flags ? LinkDrop
           : RESTARTED          in flags ? OverflowRestart
           : SEQ_DISCONTINUITY  in flags ? SequenceError
           : ALIGNMENT          in flags ? Alignment
           : lost is none                ? Unknown
           : (lost < jump and pending_carry.blocks = 0 and not lossless) ? Mixed{ lost }
           : Stream
     gaps.push(Gap{ E, jump, lost, cause, pending_carry.blocks }); pending_carry = empty
     for each channel with a pending close at s:                          -- SC-31d
        channel_gaps.push(ChannelGap{ c, s, E − s }); clear it            -- it ends where the stream gap begins
     close every open segment at E
  was_valid = the previous pushed block's mask         (empty before the first push)
  for each channel c in 0..h.channels:
     if valid bit c is set:
        if a pending close at s exists for c:                             -- SC-31d
           channel_gaps.push(ChannelGap{ c, s, h.t − s }); clear it
        open a segment at h.t if none is open, then extend it to h.t + len
     else:
        if a segment is open for c: close it and record a pending close at its end
        else if c was set in was_valid: record a pending close at h.t   -- SC-31d
        else: nothing — a channel that was never valid has no gap        -- SC-31c
  first = E on the first block; expected_next = h.t + len

finish(carry):
  carry += pending_carry
  if carry is not empty:                                                  -- SC-30c
     gaps.push(Gap{ expected_next, 0, carry.lost,
                    cause-from-carry.flags, carry.blocks })               -- end does not move
  for each channel with a pending close at s:                             -- SC-31c
     channel_gaps.push(ChannelGap{ c, s, end − s })
  close every open segment
  return ContinuityMap{ domain, channels, valid, gaps, channel_gaps, first, end }
```

## 7. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| S1 | Per-channel validity | A 64-bit mask, with validity constant within a block so that a mid-block change is unrepresentable and the producer must split | A range list per channel (a variable-size header needing allocation or a fixed cap, carrying the same information as splitting, when block length is unguaranteed anyway) | 64 channels per stream, marked in code as a deliberate ceiling; widening the mask newtype is an in-memory change with no schema impact **only while its inner field is not public** — a public field makes the widening a Kernel major after the freeze (amended 2026-09-22, 00 §11) |
| S2 | The `lost` field | Keep it as an optional count | Removing it (a consumer behind a drop-class link could not separate stream loss from link loss); making it mandatory (forecloses a future producer without timestamps) | none |
| S3 | `PARTIAL_CHANNELS` | Derived by the constructor; supplying it is an error | Producer-set (the flag and the mask would drift apart); removing it (breaks the `flags == 0` fast path that one-to-one propagation relies on) | none |
| S4 | Immutability, reference counting, pooling | Immutable by construction and shared through a reference-counted handle, both Kernel types; the pool is the producing Island's responsibility and a Phase 2 shared helper | A Kernel pool now (nothing real-time exists to use it); a custom reference count (the standard library already does it) | Phase 1 tests allocate per block; the Phase 2 pool recycles slots when the count drops to one |
| S5 | BufferRef and host memory | An opaque handle, plus optional shared host bytes owned by the block | An enum with a host variant holding a byte slice (bakes host memory into the contract, against invariant 34); a slice borrowing the link, which was the first draft's signature and outlives the block, so a pool recycling the buffer when the last reference drops would hand back recycled samples through a slice that still type-checks; a Kernel arena; a host-mapping trait on the link (its slice cannot be produced in safe code from a recycling pool, and the consumer's endpoint type does not expose it) | none |
| S6 | Memory-domain vocabulary | Keep `MemoryDomainId` as the Kernel identity; do not define a `MemoryDomain` kind type in Core | A closed enum (Vision §31's list will grow); a reachability matrix in the Kernel (the Core does not plan transfers, Vision §63) | A Vocabulary may define and interpret its own domain kinds |
| S7 | Port and attributes | Port minimal, attributes fixed at registration, identity is the id | Structural identity over id and attributes (invites shape algebra into the Kernel); per-port attribute overrides | Tensor shapes live in descriptor parameters (spec 05) |
| S8 | Compatibility | A directional `compatible_from` set, empty at v4.0 | Automatic insertion of conversion components (Vision §64 names it as future work); no mechanism at all (Vision §21 requires the check) | none |
| S9 | The link interface | One trait with publish, receive, drop count, drop carry and policy; `publish` never parks; the Phase 1 test link is a mutex around a queue | Split sink and source traits; a parking `publish` (a step-driven Island cannot afford one, and it would make `PublishOutcome::Full` unobservable); asynchronous channels; a lock-free dependency (real links are Phase 2 Link Modules) | The test link is not lock-free and says so; real links replace it |
| S10 | Sink links | Drop-class required, rejected at validate | A warning only | none |
| S11 | Who owns burst semantics | A Kernel `BurstTracker` for the state, the contiguity arithmetic and the derivable record fields; the Provider for device input and output, for the three fields headers cannot carry (SC-29a; corrected 2026-10-13: four, `requested_target` included), for executing the late policy and for events | A state machine per Provider (divergence between Mock and hardware is exactly what Vision §23 forbids); a tracker that also does device input and output; a tracker that owns the whole record (it cannot: `wraps`, `late_by` and `actual_start` are not in the headers it sees) | none |
| S12 | `TX_DISCONTINUITY` versus `TIME_ERROR` | Two layers: the tracker recovers and emits `TX_DISCONTINUITY`, while the Phase 2 Mock device model still raises `TIME_ERROR` for a Provider that bypasses the tracker, as hardware would | Choosing one (the Vision needs both; see §10) | none |
| S13 | A burst target's domain | The transmit stream's SampleClock. A target in another exactly related domain is converted and, when inexact, advanced to the next transmit sample instant, with both times recorded (SC-23a) | Refusing an inexact target, which was the first draft's rule: TM-13b makes disjoint sample grids the default (corrected 2026-10-13: no longer — since KC-15 and RM-25, radio clocks of one ratio share every sample instant; a target from a clock of another ratio can still fall between transmit samples), so it would refuse roughly nine reactive bursts in ten and block Vision §56's litmus; device root ticks (a target off the sample grid would be silently floored, which is the same error without the record); rounding to nearest (advancing is the only direction that cannot transmit in the past) | The applied target can be one sample later than asked, which at 20 Msps is 50 ns and is recorded |
| S14 | Zero-length blocks | Forbidden | A zero-length end-of-burst marker (a special case, when the producer ending a burst knows which block is its last) | Allowing an end-of-burst-only marker later is an additive relaxation |
| S15 | Continuity derivation | A Kernel builder, incremental | Sink-side derivation (every Sink would derive it differently, against Vision §28) | none |
| S16 | `GapCause` | A closed derived set | A free-form string | none |
| S17 | Flag bit positions | Fixed in this document | An ordering that exists only in the Rust source | none |
| S18 | Is `SampleBlock` a document? | No: no schema and no version. The records (`ContinuityMap`, `BurstRecord`, `DataLinkDecl`, `DataContract`) are documents | Versioning the block header (it never leaves the process) | none |
| S19 | What `LATE` means on receive | Nothing: a receive block carrying it is refused (SC-16a), and a late start is SC-13's first-block gap | The first block of a stream that started after its requested time (a second encoding of what SC-13 already carries, with no producer) | none |

## 8. Tests

| test | input | expected | rules |
|---|---|---|---|
| `sc_10_block_rejects_invalid_shape` | len 0; channels 0; channels 65; a valid bit at or above `channels`; a reserved flag bit | `InvalidBlock` in each case | SC-10 |
| `sc_01_port_shapes_are_pinned_by_the_schema_freeze` | generated schemas for `Port` and `PortRef` | exactly `name`, `direction`, `contract`; exactly `component`, `port` | SC-1, OV-22 |
| `sc_06_memory_domain_id_is_node_qualified` | `MemoryDomainId::local(7)` | node is `NodeId::LOCAL`; local ordinal is 7 | SC-6, X7 |
| `sc_07_unsafe_code_remains_forbidden` | removal of `#![forbid(unsafe_code)]` from `src/lib.rs` | `kernel_surface` fails | SC-7, OV-23 |
| `sc_10a_block_rejects_undersized_buffer` | 4 channels of 2 000 samples at 8 bytes with a buffer of 32 000 bytes, then 64 000 | `InvalidBlock`, then accepted | SC-10a |
| `sc_16_direction_flags_rejected` | a receive block with a start of burst; a transmit block with `GAP_BEFORE`; a transmit, then a receive block with `LATE` | `InvalidBlock`; `InvalidBlock`; accepted, then `InvalidBlock` | SC-16, SC-16a, SC-10 |
| `sc_10_block_partial_channels_derived` | two channels with mask 0b01, then 0b11; then the flag as input | set, clear, then error | SC-10, SC-14 |
| `sc_10_block_flag_implications` | `RESTARTED` without `GAP_BEFORE`; `ALIGNMENT` without, then with `GAP_BEFORE`; `lost` without `GAP_BEFORE`; a start of burst with `GAP_BEFORE` | error in each case but `ALIGNMENT` with `GAP_BEFORE` | SC-10, SC-16 |
| `sc_11_block_fanout_shares_reference` | one block reference into two links | both receives yield the same pointer, strong count 3 | SC-11, SC-22 |
| `sc_03_contract_identity_or_compat` | cf32 to cf32; cf32 to sc16; a fixture whose `compatible_from` holds Y, in both directions | accepted; `Incompatible`; accepted; `Incompatible` | SC-3 |
| `sc_02_contract_registry_conflict` | register cf32 twice identically, then with a different full scale | accepted, then error | SC-2 |
| `sc_04_standard_contracts_fixture` | cf32 and sc16 | cf32's `bytes_per_sample`, `full_scale` and `layout`, sc16's `bytes_per_sample` and `full_scale`, and both contracts' empty `compatible_from` | SC-4 |
| `sc_08_host_bytes_only_for_a_block_that_carries_them` | a block built with `new`, a 32 000-byte `new_host` block, and a `new_host` block with too few bytes | `new` returns no host bytes; `new_host` returns the attached bytes; the short block is refused by SC-10a | SC-8, SC-10a, KA-3 |
| `sc_30_continuity_contiguous` | t = 0 len 100, then t = 100 len 100 | one segment covering 0 to 200, no gaps | SC-12, SC-30 |
| `sc_13_continuity_gap_before_known_lost` | t = 0 len 100, then t = 250 with `GAP_BEFORE` and lost 150 | a gap at 100 of length 150, cause `Stream` | SC-12, SC-13 |
| `sc_13_continuity_gap_flag_without_jump` | a contiguous block carrying `GAP_BEFORE` | `GapFlagWithoutJump` | SC-13 |
| `sc_30_continuity_jump_without_flag` | an unflagged jump of 400, on a lossless then a lossy builder | `JumpWithoutGapFlag`, then a gap with cause `LinkDrop` | SC-30 |
| `sc_12_continuity_overlap_is_error` | t = 150 after a block ending at 200 | `TimeOverlap` | SC-12 |
| `sc_31_continuity_causes` | `GAP_BEFORE` with `RESTARTED`; with `SEQ_DISCONTINUITY`; with `lost` absent | `OverflowRestart`; `SequenceError`; `Unknown` | SC-31 |
| `sc_31_continuity_mixed_on_lossy` | a lossy path, jump 400, lost 150 | a gap of length 400 with cause `Mixed { 150 }` | SC-13, SC-31 |
| `sc_14_continuity_per_channel_segments` | two channels, masks 0b11, 0b01, 0b11 | channel 0 one segment, channel 1 two segments split at the change | SC-14, SC-30 |
| `sc_30_continuity_domain_change_ends_map` | a block in SampleClock B after one in A | `DomainChanged` | SC-30, TM-13c |
| `sc_30a_channel_count_change_ends_map` | a 2-channel block after a 4-channel one | `ChannelsChanged`, and no segment left open for channels 2 and 3 | SC-30a |
| `sc_31d_a_channel_break_is_a_channel_gap` | 4 channels, channel 2 dropped from the mask, then restored | one `ChannelGap{2, 100, 100}` | SC-31d |
| `sc_31_alignment_qualifies_a_stream_gap` | a jump of 150 with `GAP_BEFORE｜ALIGNMENT` and lost 150 on 2 channels; `ALIGNMENT` with `SEQ_DISCONTINUITY`, then with `RESTARTED` | one `Gap` at 100 of length 150, cause `Alignment`; no `ChannelGap`; `SequenceError`; `OverflowRestart` | SC-31, SC-13 |
| `sc_13_a_first_block_gap_is_recorded` | a first block at 40 with `GAP_BEFORE` and lost 40; then with `lost` absent | a `Gap` 0→40, cause `Stream`, `first` 0; then a zero-extent `Gap` at 40, cause `Unknown` | SC-13, SC-30 |
| `sc_31b_stream_gap_emits_no_channel_gaps` | 4 valid channels across an overflow gap | one `Gap`, zero `ChannelGap`s | SC-31b |
| `sc_31c_channel_that_never_returns` | channel 2 dropped and never restored before `finish` | one `ChannelGap` for channel 2 running to the map's end | SC-31c |
| `sc_31c_channel_lost_at_the_overflow` | 4 channels to t=200, then t=350 with `GAP_BEFORE｜RESTARTED` and channel 2 cleared, never restored | one `Gap` 200→350, and one `ChannelGap` for channel 2 from 350 to the end | SC-31c, SC-31d |
| `sc_31d_break_across_a_stream_gap_is_split` | channel 2 dropped at 100, a stream gap 200→350, channel 2 restored at 350 | `ChannelGap{2, 100, 100}` only; the gap's 150 samples are not charged to channel 2 | SC-31d |
| `sc_31c_never_valid_channel_has_no_gap` | channel 3 never set in any mask | no `ChannelGap` for channel 3 | SC-31c |
| `sc_30_continuity_jump_overflow_is_time_error` | a block whose time makes the jump computation overflow | `StreamError::Time(Overflow)`, not a panic | SC-30 |
| `sc_24_burst_sob_eob_basic` | start at 1000 len 100, then 1100 len 50, then 1150 len 10 with end | `Started`, `Continued`, `Ended` with 160 samples; state idle | SC-24, SC-28 |
| `sc_24_burst_single_block` | start and end on one block | `Ended` | SC-24 |
| `sc_24_burst_missing_sob` | idle, a block without a start | `MissingStartOfBurst` | SC-24 |
| `sc_24_burst_forward_jump_is_discontinuity` | expected 1100, a block at 1300 without an end | a discontinuity, then a new burst targeted at 1300 | SC-24 |
| `sc_24_burst_backward_time_is_discontinuity` | expected 1100, a block at 1050 | a discontinuity | SC-24 |
| `sc_24_burst_sob_inside_burst` | a block carrying a start at the expected time | a discontinuity closing the record, then a new burst | SC-24 |
| `sc_25_burst_continuous_until_stop` | a start then a thousand contiguous blocks, then stop | all `Continued`, then `Ended { Stop }` | SC-25 |
| `sc_26_burst_repeat_wrap_contiguous` | a waveform of 1000 sent as 300, 300, 300, 100, then 300 again (the v3 tail pattern) | `Continued` at every block including the wrap | SC-26 |
| `sc_26_burst_repeat_wrap_off_by_one` | the wrapping block placed at target + 1001 | a discontinuity | SC-26 |
| `sc_24_burst_domain_mismatch` | a block from another SampleClock | `DomainMismatch` | SC-24 |
| `sc_27_late_policy_decisions` | a 1 ms lead, with the target 2 ms and 0.5 ms away, under each policy | `OnTime`; send-as-soon-as-possible with 0.5 ms; drop; plan violation | SC-27 |
| `sc_27_late_policy_domain_check` | target and now in different domains | `DomainMismatch` | SC-27, TM-6 |
| `sc_20_link_block_policy_full` | capacity 2, three publishes, then a receive and another publish | accepted, accepted, `Full`, drop count 0, then accepted | SC-20 |
| `sc_20_link_drop_oldest` | capacity 2, three publishes | receives the second and third, drop count 1 | SC-20 |
| `sc_20_link_drop_newest` | capacity 2, three publishes | receives the first and second, drop count 1 | SC-20 |
| `sc_20a_full_is_not_a_silent_drop` | a producer double that receives `Full`, then retries | the block is delivered on the retry; nothing is lost; a producer that drops instead is caught by the double's assertion | SC-20a |
| `sc_20b_drop_carry_preserves_attribution` | a `DropOldest` link at capacity; the evicted block carries `GAP_BEFORE｜RESTARTED` with lost 150 | the next received block's carry reports both flags and 150, and none trails it; two further drops carry the union of their flags and lost 12 | SC-20b |
| `sc_30b_carry_merges_into_next_gap` | that carry pushed with a clean following header | one gap: cause `OverflowRestart`, `lost` 150, `link_dropped` 1 — not `LinkDrop` with no count | SC-30b, SC-31 |
| `sc_30b_a_carry_on_a_contiguous_block_is_not_discarded` | two blocks refused, the delivered block contiguous, a later jump | one gap with `link_dropped` 3 — the held count plus the one at the jump | SC-30b |
| `sc_30b_a_carry_held_across_a_contiguous_block_reaches_the_trailing_gap` | the same, with no later jump | a zero-extent trailing gap with the held count, not a map that claims none | SC-30b, SC-30c |
| `sc_30b_a_held_carry_keeps_its_flags_and_lost_count` | a carry with `GAP_BEFORE \| RESTARTED`, `lost` 150 and one block, delivered contiguously | the next gap's cause is `OverflowRestart` with `lost` 150; the contiguous push is **accepted**, because the carried `GAP_BEFORE` is a dropped block's claim and not this block's | SC-30b, SC-13 |
| `sc_30b_a_first_block_s_carry_is_recorded_at_it` | a first block at 100 with a carry of 3 blocks, then a jump with `GAP_BEFORE｜RESTARTED`; a first block at 40 with `GAP_BEFORE`, lost 10 and a carry of 2 blocks with lost 5; a first block with a carry of lost 5 | a zero-extent `LinkDrop` gap at 100 with `link_dropped` 3, and the later gap with `link_dropped` 0; one zero-extent gap at 40 with `lost` 15 and `link_dropped` 2, `first` 40; a first block without `GAP_BEFORE` and a carry with lost 5: a zero-extent gap at it with `lost` 5 | SC-30b, SC-13, SC-30c |
| `sc_30c_a_rejected_push_keeps_both_carries` | a held carry of 2 blocks, then a push carrying 1 block rejected with `JumpWithoutGapFlag`; then a push carrying 1 block rejected with `TimeOverlap` | `finish` records one gap with `link_dropped` 3; then one gap with `link_dropped` 1 | SC-30b, SC-30c |
| `sc_31_mixed_requires_that_no_carry_explains_the_shortfall` | a held block, then a jump of 400 with `lost` 150 and an empty carry | cause `Stream`, not `Mixed`: a gap reporting `link_dropped` 1 cannot also claim nothing explains the shortfall | SC-31, SC-30b |
| `sc_30c_carry_survives_a_rejected_push` | a carry pushed with a block rejected by a channel-count change | the builder keeps it, and the outgoing map's `finish` records it | SC-30c, SC-30a |
| `sc_30c_trailing_carry_is_zero_extent` | a carry at `finish` with `lost` absent | a `Gap` at `expected_next` of length 0 with the block count; `end` unchanged | SC-30c |
| `sc_21_sink_links_must_be_drop_class` | a Sink consumer with `Block`, then with `DropOldest` | rejected, then accepted | SC-21 |
| `sc_23a_tx_target_advances_to_next_sample` | a target at root tick 70 with ratio 8, then at 40 | advanced to sample 9 with `requested_target` recorded; accepted as sample 5 unchanged | SC-23a, TM-4 |
| `sc_23a_reactive_target_across_disjoint_grids` | receive and transmit SampleClocks whose origins differ by 3 root ticks, a target computed in the receive domain | admitted, advanced by at most one sample, both times in the record | SC-23a, TM-13b |
| `sc_23b_tx_target_unrelated_domain_refused` | a target in a domain with another root | refused | SC-23b, TM-5 |
| `sc_27_min_lead_cross_multiplied` | `min_lead` 1 ms in `host.monotonic` against leads either side of it in a 3 Hz and in a 30.72 Msps SampleClock | the exact verdict in all four cases; a rescale-and-round implementation reports the 3 Hz case 333 times too strict and fails | SC-27, TM-21 |
| `sc_29a_wraps_counted_from_burst_open` | a repeat burst of 1 000 samples over 3 500 samples of blocks | `wraps` is 3; without `BurstOpen` the tracker refuses rather than reporting 0 | SC-29a |
| `sc_29a_set_late_carries_a_discontinuity_opened_burst` | a burst opened by a time jump in a block without `START_OF_BURST`, whose late outcome arrives through `set_late` | the outcome reaches that burst's record; `OnTime` clears `late_by` rather than recording a zero | SC-29a |

## 9. Vision coverage

| Vision | This spec |
|---|---|
| §21 Port, DataContract registry, explicit format conversion | SC-1…SC-5, SC-33, decisions S7 and S8 |
| §22 a TxBurst is one burst, `repeat` is an attribute, late policy | SC-23, SC-26, SC-27 |
| §23 the SampleBlock shape | §4, SC-10 |
| §23 RX rule 1: monotonic time, gaps never filled | SC-12, SC-13 |
| §23 RX rule 2: per-channel validity | SC-14 |
| §23 RX rule 3: length not guaranteed | SC-15 |
| §23 RX rule 4: the full-scale convention | SC-4 |
| §23 RX rule 5: immutable, reference-counted, pooled, fan-out shares | SC-9, SC-11, SC-22 (the pool is Phase 2) |
| §23 RX rule 6: BufferRef with a MemoryDomain, no raw slices | SC-6…SC-8 |
| §23 RX rule 7: transmit blocks carry a target time | SC-23 |
| §23 RX rule 8: link policy, observation links always drop-class | SC-19…SC-21 |
| §23 the overflow paragraph | SC-18 |
| §23 the ContinuityMap and SigMF paragraph | SC-30…SC-32 |
| §23 TX rule 1: contiguity and discontinuity | SC-24 and the §6 table |
| §23 TX rule 2: a TxBurst is one burst | SC-23, SC-24 |
| §23 TX rule 3: continuous transmission is one burst, underflow | SC-25 |
| §23 TX rule 4: `repeat` | SC-26 |
| §23 TX rule 5: record what was transmitted | SC-28 |
| §28 ContinuityMap, validity, Gap, taint as a convention | SC-17, SC-30, SC-30a, SC-30b, SC-31 |
| §28 "multi-channel alignment failure" as a describable outcome | SC-13, SC-31 (`Alignment`), SC-31d |
| §30 a Probe is a lossy link plus a Sink | SC-21, SC-22 |
| §31 MemoryDomain and BufferRef | SC-6, SC-7 |
| §34 the wire format is not a contract | SC-5 |
| §46 a transmit tap for self-interference cancellation | SC-23 |
| §17 an injected fault equals a hardware fault | SC-18 (the test is Phase 2) |

## 10. Vision issues found

1. **§23 TX rule 1 contradicts §58 #15.** TX rule 1 says the Provider closes the burst, emits a discontinuity and starts a new burst, while §58 #15 says an unclosed burst followed by a timed block yields a `TIME_ERROR` on Mock and hardware alike. Both are wanted, at different layers: the Kernel tracker never lets the device see that sequence (SC-24), and the Mock's device model must still emulate `TIME_ERROR` for a Provider that bypasses the tracker (decision S12). §58 #15's first clause tests the device model, not the Stream Contract path. The Vision wording should say which layer it means.
2. **§23 RX rule 1 leaves an unknown `lost` half-defined.** It allows `lost` to be absent, but does not say what time the next block then carries; a monotonic time cannot be both exact and unknown. Resolved by requiring timestamped producers at v4.0 (SC-13) and reserving the absent case for a future producer. The attribution of loss behind a drop-class link is why the field must travel with the block at all.
3. **§23 lists `LATE` among the flags** with neither a side nor a meaning. Transmit-only (SC-16a, decision S19).
4. **§23 shows one buffer for N channels with no layout**, while UHD delivers one buffer per channel. Resolved by making the layout a contract attribute (SC-4).
5. **§28's `Gap.cause` has no defined value set.** Closed in SC-31.
6. **§22 puts a burst target "in the radio's ClockDomain" while §23 rule 7 puts block times in the stream's SampleClock.** These are different domains, the device root and the sample grid. SC-23 chooses the SampleClock; SC-23a converts an exactly related target with `apply` and advances it to the next sample instant when inexact, and SC-23b refuses an unrelated one on TM-5's grounds.
7. **§23 RX rule 5's "the real-time path performs no allocation" is untestable in Phase 1**, because no real-time path exists yet. Recorded as a producer rule (SC-9) whose test is the Phase 2 and Phase 8 copy-regression benchmarks.
8. **§22's `TxBurst.format` and channel list versus a block's contract and channel count.** Mapping a burst's channels onto a stream's channels is Radio Model vocabulary (Phase 2). Noted, not decided here.
9. **§23 rule 8 names the policy `block` but the Vision never says whether `publish` parks.** A parking publish cannot be called from a step-driven Island (§32), and a non-parking one makes the name a misnomer. SC-20a keeps the Vision's word and fixes the semantics: the policy produces back-pressure through a refusal the producer must honour, not through a parked thread. §23 rule 8's wording should say so.
10. **§28 requires a capture to describe a "multi-channel alignment failure" but §23 gives no way to carry the cause of a per-channel break.** Resolved by `ChannelGap` (SC-31d) and the stream-gap qualifier `ALIGNMENT`, which yields `GapCause::Alignment` (SC-31). The matching event kind `ALIGNMENT_ERROR` is in §29's list but appears in no §23 rule.
11. **Neither §23 nor §30 says what a drop-class link does with the flags of the blocks it drops.** Since §23 rule 8 puts recorders as well as probes behind drop-class links, silently losing a dropped block's `RESTARTED` and its lost count would misattribute a device overflow in the very artifact §50 asks to be honest. Resolved by the `DropCarry` of SC-20b.
12. **§5 and §31 model `MemoryDomain` as a Core concept with a growing set of kinds.** SC-6 keeps the node-qualified `MemoryDomainId` in Core and leaves host, pinned, GPU and future kinds to Vocabularies; Core compares identities and does not interpret kinds or plan transfers (S6, Vision §63). Correct both Vision sections together at Step 5 (§12).

## 11. Deferred

Link Modules beyond spec 10's pool (SC-9, HD-2) and `HostLink` (HD-5): a single-producer single-consumer ring and shared memory. The host-loop versus device-memory implementation of `repeat` and its capability constraints. A transmit-as-radiated monitor port for self-interference cancellation. The `pdu.*` and `tensor.*` contracts. The payload schema of `LINK_BACKPRESSURE`, a Kernel kind (spec 04 RS-27); the Radio Model's kinds' payloads are spec 07's (RM-22). Taint conventions for Processors whose mapping is not one-to-one.

---

## Changes

| date | rules | change | record |
|---|---|---|---|
| 2026-09-23 | SC-1…SC-32 | accepted at Gate A (Phase 1 Step 5), with the Phase 1 findings D50 (SC-2's "identical" is exact numeric equality) and D95 (a port's name and both halves of a `PortRef` are `Ident`s) | [plan/phase1/00-overview.md](../plan/phase1/00-overview.md) §11, §12 |
| 2026-09-25 | SC-5, SC-8, SC-9, SC-13, SC-15, SC-16a, SC-18, SC-20, SC-24a, SC-26, SC-28, SC-29, S5 | KA-3: host bytes travel in the block (`SampleBlock::new_host`, `host_bytes`), replacing the link's `HostMemoryAccess::map_host`, and S5 amended; KA-18: the coordinator records every created link's drop count in the Manifest; KA-19: SC-24a's marker, and every marker that named Phase 2 as a rule's carrier names its test | [spec 06](06-kernel-coordinator.md) §2, [plan/phase2/00-overview.md](../plan/phase2/00-overview.md) |
| 2026-09-26 | SC-32 | VC-3: every capture is a SigMF Recording (HD-15); SC-32 maps the stream's valid segments to capture segments and a channel's own breaks to annotations, where it had said "each valid segment" | [spec 13](../plan/phase4/13-amendments.md) |
| 2026-10-08 | §11 | MockRadio's stop tail leaves the deferred items: spec 22 removed the tail | [spec 22](../plan/maintenance/22-timing-simplification.md), commit `87bb265` |
| 2026-10-09 | SC-20 | the drop counts are recorded as the Manifest's typed `links` (`{ link, drops }`, endpoints in `plan.links`), read at step 6 (`flush_events`), instead of the section `ezsdr.links` (checked by `kc_45_manifest_fields` and `ka_18_nonzero_link_drops_reach_the_manifest`) | [audit item 4](../plan/maintenance/24-prefreeze-audit.md), owner decision 2026-10-09 |
| 2026-10-10 | SC-10, SC-13, SC-16a, SC-20b, SC-30, SC-30b, SC-30c, SC-31, SC-31a, SC-31c, SC-31d | `receive` returns each block with the carry dropped just before it, and `take_drop_carry` everything dropped after the last receive; a rejected push keeps its carry; a first block's carry is recorded at that block; a first block's `GAP_BEFORE` is recorded as a gap from the stream's origin; `LATE` is transmit-only; `ALIGNMENT` qualifies a stream gap as `Alignment`, and `ChannelGap` has no cause (SC-31a withdrawn) | [audit item 6](../plan/maintenance/24-prefreeze-audit.md), owner decision 2026-10-09 |
| 2026-10-10 | SC-2, §4 types | an attribute is spec 03's `Scalar`, whose float variant is `Num` and finite; SC-2's "identical" is SB-6's equality on it, defined once | [spec 26](../plan/maintenance/26-invariants-in-types.md) §3, audit item 1 |
| 2026-10-13 | all; SC-2, SC-4, SC-17, SC-27, SC-29, SC-29a, SC-33, §3, §4, §6, S11, S13, §8, §9, §11, the notes | rewritten to spec 22's DA-1 format: rationale, history, phase labels, review ids and finding ids leave the rules, and each rule ends in its test line (the full text as it last stood is at commit `398fd66`); new SC-33: SC-12, SC-13, SC-18, SC-23…SC-32 and SC-16's receive and transmit meanings hold for sample-stream contracts (`ezsdr.stream.*`), another contract's Vocabulary spec defines its blocks' time domain, length unit and direction, and those blocks carry no gap or burst flag; the Not-in-scope row and §4's `SampleBlock` notes follow it; SC-4 no longer lists `ezsdr.event.<schema-id>`, which nothing registers; SC-2 cites SB-6's equality and SC-17 MA-23's propagation instead of restating them; §3 and §4's sketch show `host_bytes()` and `new_host` instead of the deleted `HostMemoryAccess`, and the sketch shows `set_late`; §6 loses its notes, one of which named `closed_by_stream_gap`; S13's premise is corrected (KC-15, RM-25); §8's heading drops its phase; §11 drops the items specs 06, 07, 09 and 10 settled; the notes to specs 03–05 leave, their content held by SC-28, RS-27, spec 04 §4 and S18; the doc comments of `BlockHeader.first_sample_time` and `len` follow SC-33, and `admit_burst_target`'s drops S13's old premise; by the orchestrator's rulings on stage 8d, §4's shapes match the code (`BurstStep::Discontinuity`'s `then_ended`, which §6's table now names, `BurstOpen`'s `requested_target` and `SampleBlock`'s host bytes), SC-29 takes `requested_target` from `BurstOpen` as the code does and SC-29a and S11 follow, SC-27 names `decide`'s `registry` argument and the rules that refuse `RejectAtPlan` (KC-19, KC-24 step 7, RS-14), as `LatePolicy::RejectAtPlan`'s doc now does, Vision §21's `Normative:` line names SC-33, and six code comments name the right rule or the current text, the schemas copying them regenerated; by its rulings on the second report, SC-29 counts `wraps` from `samples` and `waveform_len` rather than from headers alone, SC-29a's burst without a `BurstOpen` is one opened by a time jump in a block without `START_OF_BURST`, as the doc comments of `on_block` and `set_late` and §8's row for `sc_29a_set_late_carries_a_discontinuity_opened_burst` now say, and §6 and §8 give a single-block burst's step as `Ended`; after Sol 6.1's review, SC-27's Test line names the tests of its three refusals, and §11 defers only a single-producer single-consumer ring and shared-memory Link Modules; no change of behaviour | [plan 31](../plan/maintenance/31-item8-kernel-specs.md), [audit 28](../plan/maintenance/28-forward-audit.md) decision 4, carried items C3 (stage 8b) and C10 (stage 8c), orchestrator rulings on stage 8d |
| 2026-10-13 | §11 | the deferred payload schemas are `LINK_BACKPRESSURE`'s only; the Radio Model's are spec 07 RM-22's, and spec 04 no longer defers them | [plan 31](../plan/maintenance/31-item8-kernel-specs.md) stage 8f |
| 2026-10-16 | SC-4, SC-10, SC-16, SC-18, SC-20, SC-22, SC-25, SC-26, SC-27, SC-31, SC-32, and §6's SC-4 and SC-24a rows | Test lines and §6 rows state only what their tests assert (the Test-line sweep): overclaims narrowed to the assertions read, the tests that pin a dropped condition named, and the conditions no test covers marked `none named`; fix round 1: SC-26's hardware case names `hw_b6_txrx_and_repeat`; no change of behaviour | Test-line sweep, 2026-10-16 |
