# Phase 1 spec 02 — Stream Contract

| Field | Value |
|---|---|
| Status | Draft for Gate A. Normative for `ezsdr-kernel::stream` and `::contract` once accepted. |
| Scope | The DataContract registry and Port; MemoryDomain and BufferRef; SampleBlock, its flags and its construction invariants; DataLink identity and back-pressure policy; the TX burst state machine and late policy; the derivation of ContinuityMap and ValidityMap. |
| Not in scope | Block pools and real link implementations (Phase 2); the MockRadio device model (Phase 2); TimingEnvelope values, which supply `min_lead` (Radio Model, Phase 2); the SigMF writer; `pdu.*` and `tensor.*` contracts (registered later without changing anything here). |
| Vision § covered | §21; §22's burst and late-policy parts; §23 in full, including the RX rules 1–8, the TX rules 1–5, the overflow paragraph and the ContinuityMap paragraph; §28; §30's link policy; §31's MemoryDomain and BufferRef; §34's wire-format note; §46's TX-tap note; §17's fault-equivalence requirement. |
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
                host bytes only through the link's host-mapping interface

TX consumers wrap blocks with BurstTracker  → start/end of burst, contiguity, records
Sinks wrap blocks with ContinuityBuilder    → ContinuityMap per SampleClock → Manifest, SigMF
```

## 4. Types

Language-neutral shapes (normative):

```text
DataContractId   a namespaced identifier matching ^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$   e.g. ezsdr.stream.cf32
Scalar           Int(signed 64-bit) | Float(float64) | Str(string) | Bool
DataContract     { id: DataContractId, attributes: map<string, Scalar>, compatible_from: set<DataContractId> }
Port             { name: string, direction: In | Out, contract: DataContractId }
PortRef          { component: ComponentId, port: string }

MemoryDomainId   { node: NodeId, local: unsigned 32-bit }
MemoryDomain     { id: MemoryDomainId, kind: namespaced string }
                 kinds named at v4.0: ezsdr.mem.host | pinned_host | huge_pages | wasm_linear | gpu | device | remote
BufferRef        { memory_domain: MemoryDomainId, handle: unsigned 64-bit (opaque), len_bytes: unsigned 64-bit }

ChannelMask      unsigned 64-bit; bit c set means channel c is valid in this block
Direction        Rx | Tx                      taken from the producing Port at construction
BlockFlags       unsigned 16-bit:
                 GAP_BEFORE 0x0001 | SEQ_DISCONTINUITY 0x0002 | RESTARTED 0x0004 | LATE 0x0008
                 | PARTIAL_CHANNELS 0x0010 | START_OF_BURST 0x0020 | END_OF_BURST 0x0040
                 | ALIGNMENT 0x0080
                 bits 8–15 reserved and must be zero
SampleBlock      { first_sample_time: TimePoint (in the stream's SampleClock),
                   len: unsigned 32-bit ≥ 1,            samples per channel
                   channels: unsigned 16-bit in 1..=64,
                   direction: Direction,
                   valid: ChannelMask,
                   flags: BlockFlags,
                   lost: optional unsigned 64-bit ≥ 1,
                   contract: DataContractId,
                   buffer: BufferRef }
BlockRef         a shared, immutable reference to a SampleBlock

BackPressure     Block | DropOldest | DropNewest
DataLinkId       { node: NodeId, local: unsigned 32-bit }
DataLinkDecl     { id: DataLinkId, from: PortRef, to: PortRef, contract: DataContractId,
                   policy: BackPressure, capacity: unsigned 32-bit ≥ 1 }
PublishOutcome   Accepted | DroppedOldest | DroppedNewest | Full
DropCarry        { flags: BlockFlags, lost: optional unsigned 64-bit, blocks: unsigned 32-bit }
                 what a drop-class link accumulated from the blocks it discarded; cleared when read

LatePolicy       RejectAtPlan | SendAsapAndFlag | DropAndFlag
LateOutcome      OnTime | SendAsap { late_by: Duration } | Drop { late_by: Duration }
                 | PlanViolation { late_by: Duration }
BurstState       Idle | InBurst { target: TimePoint, expected_next: TimePoint, blocks, samples }
BurstStep        Started | Continued | Ended { record: BurstRecord }
                 | Discontinuity { expected: TimePoint, got: TimePoint, closed: BurstRecord }
BurstEnd         Eob | Stop | Discontinuity
BurstOpen        { waveform_len: optional unsigned 32-bit, late: optional LateOutcome }
                 supplied by the Provider on the block that opens a burst; the tracker cannot know either
BurstRecord      { target: TimePoint, requested_target: optional TimePoint, actual_start: optional TimePoint,
                   blocks, samples, wraps, late_by: optional Duration, end: BurstEnd }

GapCause         Stream | OverflowRestart | SequenceError | Alignment | LinkDrop
                 | Mixed { stream_lost } | Unknown
Gap              { start: TimePoint, len: unsigned 64-bit, lost: optional unsigned 64-bit,
                   cause: GapCause, link_dropped: unsigned 32-bit }
                 cause and lost describe what the stream lost; link_dropped counts the blocks the
                 link discarded inside the same interval. The two are recorded side by side rather
                 than collapsed, so a device overflow behind a lossy link stays a device overflow.
                 link_dropped is a block count, not a sample count: the samples the link lost are
                 jump − lost, and are unrecoverable when lost is absent. That is a real limit of
                 what a dropped header can tell us, not an omission.
ChannelGap       { channel: unsigned 16-bit, start: TimePoint, len: unsigned 64-bit, cause: GapCause }
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
pub struct SampleBlock { /* private fields */ }
impl SampleBlock {
    // bytes_per_sample comes from the producing Port's DataContract, resolved at prepare
    pub fn new(h: BlockHeader, buffer: BufferRef, bytes_per_sample: u32)
        -> Result<SampleBlock, StreamError>;                                  // enforces SC-10
    pub fn first_sample_time(&self) -> TimePoint;
    pub fn end_time(&self) -> Result<TimePoint, TimeError>;   // first + len, checked
    /* getters only: no mutation API exists */
}
pub type BlockRef = std::sync::Arc<SampleBlock>;

// the slice borrows the BLOCK, not the link: a pool may recycle the buffer as soon as the last
// BlockRef is dropped, so a slice tied to the link's lifetime would outlive its own bytes (SC-8)
pub trait HostMemoryAccess { fn map_host<'a>(&self, b: &'a BlockRef) -> Option<&'a [u8]>; }
pub trait DataLink: Send + Sync {
    fn publish(&self, b: BlockRef) -> PublishOutcome;
    fn receive(&self) -> Option<BlockRef>;
    fn drops(&self) -> u64;              // a never-dropping counter, read by the event collector
    fn take_drop_carry(&self) -> DropCarry;   // SC-20a: what the dropped blocks were carrying
    fn policy(&self) -> BackPressure;
}
pub struct BurstTracker { /* domain, state, records */ }
impl BurstTracker {
    pub fn on_block(&mut self, h: &BlockHeader, open: Option<BurstOpen>)
        -> Result<BurstStep, StreamError>;
    pub fn set_actual_start(&mut self, t: TimePoint);   // device feedback, when the Provider has it
    pub fn stop(&mut self) -> Option<BurstRecord>;
}
impl LatePolicy {
    pub fn decide(self, target: TimePoint, now: TimePoint, min_lead: Duration)
        -> Result<LateOutcome, TimeError>;
}
pub struct ContinuityBuilder { /* domain, lossless flag, expected_next, open segments, gaps */ }
impl ContinuityBuilder {
    pub fn push(&mut self, h: &BlockHeader, carry: DropCarry)
        -> Result<(), (StreamError, DropCarry)>;      // SC-30c returns the carry on rejection
    pub fn finish(self, carry: DropCarry) -> ContinuityMap;
}
```

## 5. Normative rules

### Contracts and ports (Vision §21)

- **SC-1** A `Port` is a name, a direction and a `DataContractId`, and nothing more. Per-port parameters the Kernel does not interpret — a tensor shape, a maximum PDU length — live in the `ComponentDescriptor`'s parameters (spec 05), not in the Port.
- **SC-2** A `DataContract` is registered under a namespaced id with a fixed attribute map and a set `compatible_from` of producer contracts it accepts without conversion. Registering an identical definition twice is a no-op; registering a different definition under an existing id fails. "Identical" is exact, and the criterion is OV-15's: an `Int` and a `Float` attribute are one value only when they share **one canonical form**, which holds exactly while the float is integral and its magnitude is at most 2^53. Below that bound every integer is uniquely representable, so the integer profile's exact decimal and the ECMAScript shortest-round-trip decimal agree; at or above it they diverge — `i64::MIN` and −2^63 are the same number, and the canonicaliser writes `-9223372036854775808` and `-9223372036854776000`. Comparing them through an `f64` made 2^53 + 1 equal to 2^53, so a genuinely different definition — with a different canonical form and a different hash — was taken for a re-registration and discarded with no diagnostic, and equality stopped being transitive. OV-15a's accepted consequence is that `20` and `20.0` share one hash, not that two distinct values compare equal. *Checked: `sc_02_scalar_equality_is_exact_across_int_and_float`.*
- **SC-3** A link from a producer contract P to a consumer contract C is admissible if and only if P equals C, or P is a member of `compatible_from` for C. The check is directional, and it is the only contract check the Kernel performs. (Vision §21: "the Kernel does not become a type system".)
- **SC-4** The contracts registered at v4.0, whose definitions belong to the contracts Vocabulary and serve as Phase 1 test fixtures, are `ezsdr.stream.cf32 { bytes_per_sample: 8, full_scale: 1.0, layout: "planar" }`, `ezsdr.stream.sc16 { bytes_per_sample: 4, full_scale: 32767, layout: "planar" }`, `ezsdr.control {}` and `ezsdr.event.<schema-id> { schema: <id> }`. All have an empty `compatible_from`. `full_scale` is the converter full-scale convention of Vision §23 rule 4, and `layout: "planar"` means channel c occupies the byte range `[c · len · bytes_per_sample, (c+1) · len · bytes_per_sample)` of the buffer.
- **SC-5** The host-to-device wire format (`sc16`, `sc8`, `sc12`) is never a Port contract. The Provider converts and reports it in its PerformanceEnvelope. (Vision §34.)

### Buffers and memory domains (Vision §23 rules 5 and 6, §31)

- **SC-6** `MemoryDomainId`s are node-qualified, with `node = LOCAL` in v4.0. A `MemoryDomain` carries a namespaced `kind` string; the Kernel compares ids and never interprets kinds.
- **SC-7** A `BufferRef` is a memory domain, an opaque handle and a length in bytes. The handle is meaningful only to the owner of that memory domain — the producing pool, or the link that carries the domain. No Kernel interface dereferences a handle.
- **SC-8** Host bytes are obtained only through the host-mapping interface implemented by the DataLink, which delegates to the owning pool, and which yields nothing for a domain that is not host-reachable. No Module contract takes or returns a raw host slice. (Vision invariant 34.)
- **SC-9** A block's buffer remains valid while any reference to the block exists, and its owner must not reuse it earlier. Producers allocate blocks from their own Island's pool, and the real-time path performs no allocation. The pool is a Phase 2 shared helper. *Producer obligation; no real-time path exists in Phase 1, so the test is the Phase 2 and Phase 8 copy-regression benchmarks (Vision §61).*

### Blocks (Vision §23 rules 1–4)

- **SC-10** The constructor enforces, and rejects with `InvalidBlock` otherwise: `len ≥ 1`; `1 ≤ channels ≤ 64`; no bit of `valid` set at or above `channels`; reserved flag bits zero; `lost` present only together with `GAP_BEFORE` and at least 1; `RESTARTED` or `SEQ_DISCONTINUITY` implies `GAP_BEFORE`; `START_OF_BURST` and `END_OF_BURST` never together with `GAP_BEFORE`, `RESTARTED` or `SEQ_DISCONTINUITY`; and the direction rules of SC-16. `PARTIAL_CHANNELS` is derived by the constructor from the mask, and passing it as input is an error. *Checked: `SampleBlock::new`.*
- **SC-10a** The constructor also enforces `buffer.len_bytes ≥ channels · len · bytes_per_sample`, where `bytes_per_sample` is passed in from the producing Port's DataContract. Without it the one invariant that prevents an out-of-bounds read is unchecked: SC-4 fixes the planar layout, so a Provider that publishes four channels of 2 000 samples over a buffer sized for two produces a consumer-side panic or a garbage read rather than a Kernel error, and `map_host` hands back a correctly sized slice that hides it. *Checked: `SampleBlock::new`.*
- **SC-11** A block is immutable once published: the type has no mutation interface and is shared by reference. Fan-out to N consumers is N links carrying the same reference; nothing is copied.
- **SC-12** Time is monotonic within one SampleClock. On a lossless path, for consecutive blocks either the next block's first sample time equals the previous block's end, or it is later and `GAP_BEFORE` is set. Earlier than the previous block's end is a contract violation (`TimeOverlap`). A block whose domain differs from its predecessor's is a rate change (spec 01, TM-13c), not a gap. Gaps are never filled, with zeros or with repeated data.
- **SC-13** `GAP_BEFORE` is set if and only if, as far as the producer knows, samples are missing between the previous block's end and this block's first sample. `lost` carries the count when known and is absent when unknown. When known, `lost` equals the time jump on a lossless path and is at most the jump behind a drop-class link, where the difference is link loss. `RESTARTED` qualifies the gap as a stream restart, `SEQ_DISCONTINUITY` as transport sequence loss. Every v4.0 producer is timestamped and therefore always knows `lost`. *Producer obligation: the constructor checks the shape (SC-10) but cannot check that the flag reflects reality; tested against MockRadio's injected faults in Phase 2.*
- **SC-14** Validity is per channel and constant within a block: a change of validity is a block boundary, and the producer splits. `PARTIAL_CHANNELS` is set exactly when `valid` is not the full mask for `channels`.
- **SC-15** Block length is not guaranteed. A consumer must not assume a fixed length, a minimum, or an alignment. *Consumer obligation; the Mock's length-jitter option makes it testable in Phase 2 (Vision §58 #12).*
- **SC-16** A block carries its `direction`, taken from the producing Port. A receive block must not carry `START_OF_BURST` or `END_OF_BURST`; a transmit block must not carry `GAP_BEFORE`, `RESTARTED` or `SEQ_DISCONTINUITY`, because a jump in transmit time is a discontinuity (SC-24) and never a flagged gap. These are constructor checks under SC-10, not conventions: without the field nothing can detect a receive Provider that sets a start-of-burst flag by copying a transmit code path, and the block reaches the capture unremarked. *Checked: `SampleBlock::new`.*
- **SC-16a** On a receive stream `LATE` means that the stream started later than the requested time, and the producer sets it on the first block of the stream only. A stateless constructor cannot check this. *Producer obligation, tested against MockRadio in Phase 2.*
- **SC-17** Flag bit positions are fixed by this document, so that an Executor mapping input blocks to output blocks one-to-one can propagate flags unchanged by default. Propagation for a mapping that is not one-to-one is a documented Processor convention, not a Kernel type. (Vision §28; audit F28.)
- **SC-18** How an overflow appears. A UHD overflow — zero samples returned, the stream restarting about 50 ms later — appears as the next block carrying `GAP_BEFORE` and `RESTARTED`, with `lost` equal to the time jump, and with that jump equal to the restart gap. A sequence error appears as `GAP_BEFORE` and `SEQ_DISCONTINUITY` without `RESTARTED`. Zero-length blocks are never published. An injected fault must produce identical flags and jump. *Producer obligation, tested in Phase 2 against the documented UHD behaviour (Vision §17, §58 #6).*

### Links (Vision §23 rule 8, §30)

- **SC-19** Every DataLink is declared with a policy and a capacity in blocks. There is no default policy.
- **SC-20** Under `Block`, publishing returns `Full` when the capacity is reached, and nothing is ever dropped. Under `DropOldest`, the oldest queued block is evicted and the outcome says so. Under `DropNewest`, the new block is refused and the outcome says so. A drop-class link never returns `Full`. The link's drop count is a never-dropping counter, incremented on every drop and readable by the event collector (Vision §29).
- **SC-20a** `publish` never parks. `Block` names the back-pressure the policy produces, not a blocking call: a step-driven Island cannot afford a parking publish, and a `PublishOutcome` that could never be observed would be dead. A producer that receives `Full` **must not** discard the block silently; it retries on its next turn, or it stops and emits the typed `LINK_BACKPRESSURE` event for the Run's Policy table to act on. This is what makes SC-21 necessary: the stall a drop-class Sink link prevents is not a parked producer but a producer that keeps retrying because its Sink never drains. *Checked: the in-memory link and a producer-side test; the event kind is spec 04's.*
- **SC-20b** A drop-class link accumulates what it discarded into a `DropCarry`: the union of the dropped blocks' `GAP_BEFORE`, `RESTARTED`, `SEQ_DISCONTINUITY` and `ALIGNMENT` flags, the sum of the `lost` counts that were present, and how many blocks were dropped. The carry is cleared when read; what a consumer does with one whose push is rejected is SC-30c's rule. Without the carry, a hardware overflow whose block is then evicted by the lossy link in front of a recorder is re-derived as a plain link drop, and the Manifest attributes a device overflow to host-side loss with the lost count thrown away although it was known — v3's failure (§2) reappearing one layer up. *Checked: the in-memory link.*
- **SC-21** A link whose consumer port belongs to a Sink-role Module must be drop-class; `validate()` rejects `Block` there, because observation must never stall the real-time path. The predicate is a Kernel function, called by the planner (specs 03 and 05). (Vision §30, invariant 21.)
- **SC-22** Fan-out is N links sharing block references (SC-11). A Probe is a drop-class link feeding a recorder Sink, and is not a Kernel concept. (Vision §30; audit F29.)

### The transmit side (Vision §22, §23 rule 7 and TX rules 1–5)

- **SC-23** Every transmit block carries the target time of its first sample in the transmit stream's SampleClock, in continuous mode as well. A tap on the transmit edge therefore yields timed reference samples with no special interface. (Vision §46; audit F21.)
- **SC-23a** A target expressed in another **exactly related** domain is converted at admission with `apply` (TM-4). An `Inexact` result is advanced to the next transmit sample instant, `floor + 1`, and both times are recorded: the `BurstRecord` keeps `requested_target` alongside the applied `target`. Refusal would be wrong, not strict. TM-13b makes each stream's origin its own first sample, so a receive and a transmit SampleClock at the same rate share a sample instant only when their origins are congruent — for a decimation of ten, one origin pair in ten. A Reactor that computes `target = rx_time + turnaround` in the receive domain, which is the natural computation and the one Vision §56 and §58 #9 require, would otherwise be refused on almost every burst with an error it cannot act on. Advancing to the next instant is the physically correct answer, because a radio cannot transmit between samples, and recording requested against applied is the same idiom the PrepareReport uses for coercion (Vision §11). *Checked: the admission path, `sc_23a_*`.*
- **SC-23b** A target in an **unrelated** domain is refused. TM-5 makes such a conversion uncertain, and an uncertain transmit instant is not a transmit instant; the caller supplies a ClockRelation and converts deliberately, accepting the uncertainty in its own code. *Checked: the admission path.*
- **SC-24** A transmit stream is a sequence of bursts. A burst begins with a block carrying `START_OF_BURST` and ends with a block carrying `END_OF_BURST`; within a burst, each block's time equals the previous block's end. The state machine in §6 is normative, and the tracker's return value for a block reports **every** transition that block caused: a block whose time jumped and which also carried `END_OF_BURST` closes the open burst *and* opens and ends another, and says so, because a return value a caller cannot trust is what a normative state machine exists to prevent. A block published while idle without `START_OF_BURST` is `MissingStartOfBurst` and must not be transmitted. A block whose time differs from the expected next time in either direction, or which carries `START_OF_BURST` while a burst is open, is a discontinuity: the tracker closes the open burst and reports it, and the block becomes the first block of a new burst. Nothing is padded. *Checked: `BurstTracker`.*
- **SC-24a** On a discontinuity the Provider emits the typed `TX_DISCONTINUITY` event and evaluates SC-27's late policy for the new burst before transmitting it. The kind is registered by the Radio Model, not the Kernel (RS-27). *Forward obligation: the Provider's half of SC-24, tested against MockRadio in Phase 2.*
- **SC-25** Continuous transmission is one burst, ended by the Run's stop or by an explicit `END_OF_BURST`. An underflow is a typed `TX_UNDERFLOW` event carrying the time at which it occurred; the Provider fabricates no samples, and what the hardware radiated during the gap is recorded in the Provider's Manifest section.
- **SC-26** `repeat` is a burst attribute. A repeated burst of waveform length L is transmitted so that repetition w begins exactly at `target + w · L`, the block sequence stays contiguous across every wrap, and it must not underflow at the wrap — which is the v3 behavioural compatibility test of Vision §61. Whether the Provider uses a host loop or device memory is its choice, and the constraints of that choice (maximum length, word alignment) are Radio Model capabilities. *The tracker checks contiguity across the wrap in Phase 1; that the Provider does not underflow there is a producer obligation tested in Phase 2 (Vision §20, §35, §61).*
- **SC-27** Lateness. `LatePolicy::decide(target, now, min_lead)` yields `OnTime` when `target − now ≥ min_lead`, and otherwise the outcome named by the policy. `target` and `now` are in one domain and their difference is a `Duration` there (TM-7). `min_lead` comes from the bound Provider's TimingEnvelope, a static Vocabulary document that cannot name a domain created at `prepare`, so like a `RelativeBudget` it is declared in `host.monotonic`, and TM-21 compares the two exactly by cross-multiplication with no rescale and no rounding. Exactness matters in both directions: a check looser than the hardware is a defect (Vision §59), and one needlessly stricter rejects bursts the device would have sent. `RejectAtPlan` is legal only for a burst whose target is statically known: `validate()` rejects it on a runtime-decided burst (a rule for specs 03 and 05), and if it is nevertheless reached at runtime, `decide` yields `PlanViolation` and the Provider must not transmit.
- **SC-28** What was transmitted is recorded: one `BurstRecord` per burst, carrying the target, the actual start where the Provider can supply it, the block and sample counts, the number of wraps, any lateness, and how the burst ended. The Provider puts these in its Manifest section; the envelope is spec 04.
- **SC-29** `BurstTracker` is the single implementation of the state in SC-24 to SC-28, and every Provider — Mock and UHD alike — routes its transmit blocks through it, so that burst semantics cannot diverge between them. The tracker owns the state, the contiguity arithmetic, and the fields of a `BurstRecord` it can derive from block headers alone: `target`, `requested_target`, `blocks`, `samples`, `wraps` and `end`. The Provider owns device input and output, including the zero-length end-of-burst send that closes a burst on UHD as v3 did, the `now` and `min_lead` inputs, executing the late policy, and emitting events. *The tracker's half is checked in Phase 1; the Provider's half is a producer obligation tested in Phase 2.*
- **SC-29a** Three `BurstRecord` fields are not derivable from headers, so the Provider supplies them: `waveform_len`, without which `wraps` cannot be counted, and `late`, which comes from `LatePolicy::decide`, are passed in a `BurstOpen` on the block that opens the burst; `actual_start` arrives later as device feedback through `set_actual_start`. A burst a **discontinuity** opened carries no `START_OF_BURST` and therefore no `BurstOpen`, yet SC-24a still requires the late policy to be evaluated for it, so that outcome arrives through `set_late` before the next block. Without the second path `late_by` would be permanently absent on exactly the bursts SC-24a is about. Left unsupplied, `wraps` would ship as a constant zero and the v3 wrap-continuity regression of Vision §61 and §58 #15 would have nothing to assert against. *Checked: `BurstTracker`, `sc_29a_*`.*

### Derivation (Vision §23's closing paragraph, §28)

- **SC-30** `ContinuityMap` and the per-channel validity it contains are derived by the Kernel's continuity builder from block headers alone, and never assembled by hand. There is one map per SampleClock: a block in another domain ends the map with `DomainChanged`, and the Sink starts a new builder. The builder is told whether its path is lossless; on a lossless path a jump without `GAP_BEFORE` is a contract violation, and on a drop-class path it is a link-drop gap.
- **SC-30a** A block whose `channels` differs from the builder's ends the map with `ChannelsChanged`, exactly as a domain change does. Otherwise the per-channel loop runs over the new block's channel count and any channel above it keeps an open segment that `finish()` closes at the last block's end, so the map would claim a channel was valid through an interval in which the stream did not carry it.
- **SC-30b** A consumer behind a drop-class link reads the link's `DropCarry` (SC-20b) before pushing the next header and passes it to the builder, which merges its flags into the block's for the purpose of deriving a cause, sums the `lost` counts that are present, and records the dropped block count in the gap's `link_dropped`. Merging the counts is the point: in the case SC-20b exists for, the evicted block held the `lost` and the delivered block holds none, so a rule that dropped the count when either side lacked one would discard exactly the number it was written to preserve. *Checked: `ContinuityBuilder`.*
- **SC-30c** A carry that no delivered block follows is recorded by `finish` as a zero-extent `Gap` at `expected_next` — its `len` is 0 because the extent of what followed the last delivered block is unknown, not because nothing was lost; `lost` and the block count carry what is known — carrying the cause derived from its flags, its `lost` where present, and its block count; the map's `end` does not move, because no sample after the last delivered one is accounted for. A push rejected by SC-30 or SC-30a returns the carry to the consumer, which gives it to the **outgoing** builder's `finish` rather than to the new one: those samples belong to the domain and channel count that just ended. Handing it to the new builder would lose it outright, since a first push has no previous block and therefore no gap to merge into. *Checked: `ContinuityBuilder`.*
- **SC-31** `GapCause` is a closed, derived set describing what the **stream** lost: `OverflowRestart` from `RESTARTED`; `SequenceError` from `SEQ_DISCONTINUITY`; `Stream` from `GAP_BEFORE` alone; `LinkDrop` from a jump with no gap flag on a lossy path; `Mixed` from `GAP_BEFORE` with `lost` less than the jump on a lossy path and no carry to explain the difference; `Unknown` from `GAP_BEFORE` with `lost` absent; `Alignment` from a per-channel break (SC-31a). What the **link** lost is the separate `link_dropped` count, not a cause, so a device overflow inside an interval that also lost a block to the link is still reported as `OverflowRestart` with its sample count, beside a link-drop count of one. Collapsing the two into a single cause was the first draft's rule and it lost the device's own diagnosis. *Checked: `ContinuityBuilder`.*
- **SC-31a** A channel that loses validity while the stream continues produces a `ChannelGap` with a cause, not merely a hole between two `Segment`s. Vision §28 requires a capture to describe a multi-channel alignment failure, and a hole alone is indistinguishable from a channel that was never enabled over that interval. The cause is `Alignment` when the block that dropped the channel carries the `ALIGNMENT` flag, and `Stream` otherwise. Without that flag the `Alignment` variant would be unreachable, because nothing else in a block header distinguishes an alignment failure from an intentionally disabled channel. The event kind `ALIGNMENT_ERROR` that accompanies it is spec 04's to define (Vision §29). *Checked: `SampleBlock::new` for the flag, `ContinuityBuilder` for the derivation.*
- **SC-31b** A per-channel break caused by a **stream** gap produces no `ChannelGap`. The stream `Gap` already covers those samples for every channel, and emitting both would report one overflow on a four-channel stream as one `Gap` plus four duplicates of it, which a SigMF export (SC-32) would then write as four channel annotations for one gap. *Checked: `ContinuityBuilder`.*
- **SC-31c** A channel that was valid somewhere in the map and is invalid at its end produces a `ChannelGap` running to the map's end. A channel that fails and never returns is the usual outcome of an alignment error, and a builder that emitted a break only when a channel came back would describe that capture as though the channel had simply ended. A channel that was never valid produces nothing, because never enabled is not a gap. *Checked: `ContinuityBuilder`.*
- **SC-31d** A channel's break is held as a pending close and emitted only when its extent is known: when the channel returns, when a stream gap ends it early, or at `finish`. A break that coincides with a stream gap is recorded from the gap's end, not from the channel's last valid sample, because the samples before the gap were lost by the stream and are already in its `Gap`. Emitting at the moment of the break instead would either double-count those samples against the channel or, when the stream gap closed the channel's segment first, lose the break entirely. *Checked: `ContinuityBuilder`.*
- **SC-32** A SigMF export maps each valid segment to a capture with `core:sample_start` and `core:global_index`, and carries per-channel validity in an `ezsdr` extension namespace, since no existing SigMF extension covers validity. *Forward obligation: binds the SigMF Sink; no Phase 1 code, tested in Phase 4.*

## 6. Algorithms

**Burst tracker** (SC-24). `t` is the block's first sample time, `L` its length, `E` the expected next time.

| state | input | next state | step |
|---|---|---|---|
| Idle | start of burst, not end | InBurst { target t, E = t + L } | `Started` |
| Idle | start and end together | Idle | `Started` then `Ended { Eob }` (a single-block burst) |
| Idle | neither | Idle | error `MissingStartOfBurst`; the block is not transmitted |
| InBurst | t = E, no start, no end | InBurst { E += L } | `Continued` |
| InBurst | t = E, end of burst | Idle | `Ended { Eob }` |
| InBurst | t ≠ E, or a start of burst | InBurst { target t, E = t + L }, or Idle if this block also ends | `Discontinuity { expected E, got t, closed record }`; the Provider emits `TX_DISCONTINUITY` and evaluates SC-27 for the new burst |
| InBurst | stop | Idle | `Ended { Stop }` |
| any | the block's domain differs from the tracker's | unchanged | error `DomainMismatch` |

**Continuity derivation** (SC-30).

```text
push(h, carry):                            -- carry is the link's DropCarry, empty on a lossless path
  if h.domain ≠ domain            -> DomainChanged
  if h.channels ≠ channels        -> ChannelsChanged                      -- SC-30a
  flags = h.flags OR carry.flags                                          -- SC-30b
  lost  = sum of whichever of h.lost and carry.lost are present            -- absent only if neither is
  if expected_next is Some(E):
     if h.t < E                   -> TimeOverlap
     if h.t = E and GAP_BEFORE in flags -> GapFlagWithoutJump
     if h.t > E:
        jump = h.t − E                                                    -- checked; overflow -> Time(..)
        if GAP_BEFORE not in flags:
           lossless ? JumpWithoutGapFlag
                    : gaps.push(Gap{ E, jump, none, LinkDrop, carry.blocks })
        else:
           cause = RESTARTED          in flags ? OverflowRestart
                 : SEQ_DISCONTINUITY  in flags ? SequenceError
                 : lost is none                ? Unknown
                 : (lost < jump and carry.blocks = 0 and not lossless) ? Mixed{ lost }
                 : Stream
           gaps.push(Gap{ E, jump, lost, cause, carry.blocks })
        for each channel with a pending close at s with cause k:          -- SC-31d
           channel_gaps.push(ChannelGap{ c, s, E − s, k }); clear it      -- it ends where the stream gap begins
        close every open segment at E
  was_valid = the previous pushed block's mask         (empty before the first push)
  for each channel c in 0..h.channels:
     if valid bit c is set:
        if a pending close at s with cause k exists for c:                -- SC-31a
           channel_gaps.push(ChannelGap{ c, s, h.t − s, k }); clear it
        open a segment at h.t if none is open, then extend it to h.t + len
     else:
        k = ALIGNMENT in h.flags ? Alignment : Stream                     -- the block that DROPS c
        if a segment is open for c: close it and record a pending close at its end with cause k
        else if c was set in was_valid: record a pending close at h.t with cause k   -- SC-31d
        else: nothing — a channel that was never valid has no gap        -- SC-31a
  expected_next = h.t + len

finish(carry):
  if carry.blocks > 0:                                                    -- SC-30c
     gaps.push(Gap{ expected_next, 0, carry.lost,
                    cause-from-carry.flags, carry.blocks })               -- end does not move
  for each channel with a pending close at s with cause k:                -- SC-31c
     channel_gaps.push(ChannelGap{ c, s, end − s, k })
  close every open segment
  return ContinuityMap{ domain, channels, valid, gaps, channel_gaps, first, end }
```

The pending-close record is what makes the two guards work. A channel's break is remembered, never emitted at the moment it happens, so the emission can span the right interval: it is flushed when the channel returns, when a stream gap ends it early, or at `finish`. Without the flush in the stream-gap branch a channel that dropped before an overflow and returned after it would report the whole span, overflow included, as its own fault; without the `was_valid` clause a channel lost *at* an overflow, whose segment the gap already closed, would leave no record at all and its data would appear to have simply ended.

The two guards matter. Without `closed_by_stream_gap` every stream gap would also emit one `ChannelGap` per valid channel over the same samples, so one overflow on a four-channel stream would report one `Gap` and four duplicates of it. Without the `finish` clause a channel that fails and never returns — the usual outcome of a UHD alignment error — would produce no `ChannelGap` at all, which is the case Vision §28 names.

## 7. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| S1 | Per-channel validity | A 64-bit mask, with validity constant within a block so that a mid-block change is unrepresentable and the producer must split | A range list per channel (a variable-size header needing allocation or a fixed cap, carrying the same information as splitting, when block length is unguaranteed anyway) | 64 channels per stream, marked in code as a deliberate ceiling; widening the mask newtype is an in-memory change with no schema impact **only while its inner field is not public** — a public field makes the widening a Kernel major after the freeze (amended 2026-09-22, 00 §11) |
| S2 | The `lost` field | Keep it as an optional count | Removing it (a consumer behind a drop-class link could not separate stream loss from link loss); making it mandatory (forecloses a future producer without timestamps) | none |
| S3 | `PARTIAL_CHANNELS` | Derived by the constructor; supplying it is an error | Producer-set (the flag and the mask would drift apart); removing it (breaks the `flags == 0` fast path that one-to-one propagation relies on) | none |
| S4 | Immutability, reference counting, pooling | Immutable by construction and shared through a reference-counted handle, both Kernel types; the pool is the producing Island's responsibility and a Phase 2 shared helper | A Kernel pool now (nothing real-time exists to use it); a custom reference count (the standard library already does it) | Phase 1 tests allocate per block; the Phase 2 pool recycles slots when the count drops to one |
| S5 | BufferRef and host memory | An opaque handle plus a host-mapping interface reached through the link, whose returned slice borrows the **block**, not the link | An enum with a host variant holding a byte slice (bakes host memory into the contract, against invariant 34); a slice borrowing the link, which was the first draft's signature and outlives the block, so a pool recycling the buffer when the last reference drops would hand back recycled samples through a slice that still type-checks; a Kernel arena | none |
| S6 | MemoryDomain kind | A namespaced string; the Kernel compares ids only | A closed enum (the Kernel is frozen while Vision §31's list will grow); a reachability matrix in the Kernel (the Core does not plan transfers, Vision §63) | none |
| S7 | Port and attributes | Port minimal, attributes fixed at registration, identity is the id | Structural identity over id and attributes (invites shape algebra into the Kernel); per-port attribute overrides | Tensor shapes live in descriptor parameters (spec 05) |
| S8 | Compatibility | A directional `compatible_from` set, empty at v4.0 | Automatic insertion of conversion components (Vision §64 names it as future work); no mechanism at all (Vision §21 requires the check) | none |
| S9 | The link interface | One trait with publish, receive, drop count, drop carry and policy; `publish` never parks; the Phase 1 test link is a mutex around a queue | Split sink and source traits; a parking `publish` (a step-driven Island cannot afford one, and it would make `PublishOutcome::Full` unobservable); asynchronous channels; a lock-free dependency (real links are Phase 2 Link Modules) | The test link is not lock-free and says so; real links replace it |
| S10 | Sink links | Drop-class required, rejected at validate | A warning only | none |
| S11 | Who owns burst semantics | A Kernel `BurstTracker` for the state, the contiguity arithmetic and the derivable record fields; the Provider for device input and output, for the three fields headers cannot carry (SC-29a), for executing the late policy and for events | A state machine per Provider (divergence between Mock and hardware is exactly what Vision §23 forbids); a tracker that also does device input and output; a tracker that owns the whole record (it cannot: `wraps`, `late_by` and `actual_start` are not in the headers it sees) | none |
| S12 | `TX_DISCONTINUITY` versus `TIME_ERROR` | Two layers: the tracker recovers and emits `TX_DISCONTINUITY`, while the Phase 2 Mock device model still raises `TIME_ERROR` for a Provider that bypasses the tracker, as hardware would | Choosing one (the Vision needs both; see §10) | none |
| S13 | A burst target's domain | The transmit stream's SampleClock. A target in another exactly related domain is converted and, when inexact, advanced to the next transmit sample instant, with both times recorded (SC-23a) | Refusing an inexact target, which was the first draft's rule: TM-13b makes disjoint sample grids the default, so it would refuse roughly nine reactive bursts in ten and block Vision §56's litmus; device root ticks (a target off the sample grid would be silently floored, which is the same error without the record); rounding to nearest (advancing is the only direction that cannot transmit in the past) | The applied target can be one sample later than asked, which at 20 Msps is 50 ns and is recorded |
| S14 | Zero-length blocks | Forbidden | A zero-length end-of-burst marker (a special case, when the producer ending a burst knows which block is its last) | Allowing an end-of-burst-only marker later is an additive relaxation |
| S15 | Continuity derivation | A Kernel builder, incremental | Sink-side derivation (every Sink would derive it differently, against Vision §28) | none |
| S16 | `GapCause` | A closed derived set | A free-form string | none |
| S17 | Flag bit positions | Fixed in this document | An ordering that exists only in the Rust source | none |
| S18 | Is `SampleBlock` a document? | No: no schema and no version. The records (`ContinuityMap`, `BurstRecord`, `DataLinkDecl`, `DataContract`) are documents | Versioning the block header (it never leaves the process) | none |
| S19 | What `LATE` means on receive | The first block of a stream that started after its requested time | Leaving it undefined, as the Vision does | none |

## 8. Phase 1 tests

| test | input | expected | rules |
|---|---|---|---|
| `sc_10_block_rejects_invalid_shape` | len 0; channels 0; channels 65; a valid bit at or above `channels`; a reserved flag bit | `InvalidBlock` in each case | SC-10 |
| `sc_10a_block_rejects_undersized_buffer` | 4 channels of 2 000 samples at 8 bytes with a buffer of 32 000 bytes, then 64 000 | `InvalidBlock`, then accepted | SC-10a |
| `sc_16_direction_flags_rejected` | a receive block with a start of burst; a transmit block with `GAP_BEFORE` | `InvalidBlock` in both | SC-16, SC-10 |
| `sc_10_block_partial_channels_derived` | two channels with mask 0b01, then 0b11; then the flag as input | set, clear, then error | SC-10, SC-14 |
| `sc_10_block_flag_implications` | `RESTARTED` without `GAP_BEFORE`; `lost` without `GAP_BEFORE`; a start of burst with `GAP_BEFORE` | error in each case | SC-10, SC-16 |
| `sc_11_block_fanout_shares_reference` | one block reference into two links | both receives yield the same pointer, strong count 3 | SC-11, SC-22 |
| `sc_03_contract_identity_or_compat` | cf32 to cf32; cf32 to sc16; a fixture whose `compatible_from` holds Y, in both directions | accepted; `Incompatible`; accepted; `Incompatible` | SC-3 |
| `sc_02_contract_registry_conflict` | register cf32 twice identically, then with a different full scale | accepted, then error | SC-2 |
| `sc_04_standard_contracts_fixture` | cf32 and sc16 | the attributes of SC-4, empty compatibility | SC-4 |
| `sc_08_buffer_map_host_none_for_gpu_domain` | a buffer in a GPU memory domain | nothing returned | SC-8 |
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
| `sc_31a_channel_gap_carries_a_cause` | 4 channels, channel 2 dropped from the mask on a block carrying `ALIGNMENT`, then restored | one `ChannelGap` for channel 2 with cause `Alignment`; the same sequence without the flag gives cause `Stream` | SC-31a |
| `sc_31b_stream_gap_emits_no_channel_gaps` | 4 valid channels across an overflow gap | one `Gap`, zero `ChannelGap`s | SC-31b |
| `sc_31c_channel_that_never_returns` | channel 2 dropped and never restored before `finish` | one `ChannelGap` for channel 2 running to the map's end | SC-31c |
| `sc_31c_channel_lost_at_the_overflow` | 4 channels to t=200, then t=350 with `GAP_BEFORE｜RESTARTED` and channel 2 cleared, never restored | one `Gap` 200→350, and one `ChannelGap` for channel 2 from 350 to the end | SC-31c, SC-31d |
| `sc_31d_break_across_a_stream_gap_is_split` | channel 2 dropped at 100 on `ALIGNMENT`, a stream gap 200→350, channel 2 restored at 350 | `ChannelGap{2, 100, 100, Alignment}` only; the gap's 150 samples are not charged to channel 2 | SC-31d |
| `sc_31a_never_valid_channel_has_no_gap` | channel 3 never set in any mask | no `ChannelGap` for channel 3 | SC-31a, SC-31c |
| `sc_30_continuity_jump_overflow_is_time_error` | a block whose time makes the jump computation overflow | `StreamError::Time(Overflow)`, not a panic | SC-30 |
| `sc_24_burst_sob_eob_basic` | start at 1000 len 100, then 1100 len 50, then 1150 len 10 with end | `Started`, `Continued`, `Ended` with 160 samples; state idle | SC-24, SC-28 |
| `sc_24_burst_single_block` | start and end on one block | `Started` then `Ended` | SC-24 |
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
| `sc_20b_drop_carry_preserves_attribution` | a `DropOldest` link at capacity; the evicted block carries `GAP_BEFORE｜RESTARTED` with lost 150 | the carry reports both flags and 150; it is cleared on read | SC-20b |
| `sc_30b_carry_merges_into_next_gap` | that carry pushed with a clean following header | one gap: cause `OverflowRestart`, `lost` 150, `link_dropped` 1 — not `LinkDrop` with no count | SC-30b, SC-31 |
| `sc_30b_a_carry_on_a_contiguous_block_is_not_discarded` | two blocks refused, the delivered block contiguous, a later jump | one gap with `link_dropped` 3 — the held count plus the one at the jump | SC-30b |
| `sc_30b_a_carry_held_across_a_contiguous_block_reaches_the_trailing_gap` | the same, with no later jump | a zero-extent trailing gap with the held count, not a map that claims none | SC-30b, SC-30c |
| `sc_30b_a_held_carry_keeps_its_flags_and_lost_count` | a carry with `GAP_BEFORE \| RESTARTED`, `lost` 150 and one block, delivered contiguously | the next gap's cause is `OverflowRestart` with `lost` 150; the contiguous push is **accepted**, because the carried `GAP_BEFORE` is a dropped block's claim and not this block's | SC-30b, SC-13 |
| `sc_30b_a_rejected_push_does_not_destroy_the_held_carry` | a held carry, then a push rejected with `JumpWithoutGapFlag` | the caller gets its own carry back and the builder still holds its own, which reaches `finish` | SC-30b, SC-30c |
| `sc_31_mixed_requires_that_no_carry_explains_the_shortfall` | a held block, then a jump of 400 with `lost` 150 and an empty carry | cause `Stream`, not `Mixed`: a gap reporting `link_dropped` 1 cannot also claim nothing explains the shortfall | SC-31, SC-30b |
| `sc_30c_carry_survives_a_rejected_push` | a carry read, then a push rejected by a channel-count change | the carry is returned with the error and lands in the outgoing map's `finish`, not the new one | SC-30c, SC-30a |
| `sc_30c_trailing_carry_is_zero_extent` | a carry at `finish` with `lost` absent | a `Gap` at `expected_next` of length 0 with the block count; `end` unchanged | SC-30c |
| `sc_21_sink_links_must_be_drop_class` | a Sink consumer with `Block`, then with `DropOldest` | rejected, then accepted | SC-21 |
| `sc_23a_tx_target_advances_to_next_sample` | a target at root tick 70 with ratio 8, then at 40 | advanced to sample 9 with `requested_target` recorded; accepted as sample 5 unchanged | SC-23a, TM-4 |
| `sc_23a_reactive_target_across_disjoint_grids` | receive and transmit SampleClocks whose origins differ by 3 root ticks, a target computed in the receive domain | admitted, advanced by at most one sample, both times in the record | SC-23a, TM-13b |
| `sc_23b_tx_target_unrelated_domain_refused` | a target in a domain with another root | refused | SC-23b, TM-5 |
| `sc_27_min_lead_cross_multiplied` | `min_lead` 1 ms in `host.monotonic` against leads either side of it in a 3 Hz and in a 30.72 Msps SampleClock | the exact verdict in all four cases; a rescale-and-round implementation reports the 3 Hz case 333 times too strict and fails | SC-27, TM-21 |
| `sc_29a_wraps_counted_from_burst_open` | a repeat burst of 1 000 samples over 3 500 samples of blocks | `wraps` is 3; without `BurstOpen` the tracker refuses rather than reporting 0 | SC-29a |
| `sc_29a_set_late_carries_a_discontinuity_opened_burst` | a burst opened by a time jump, whose late outcome arrives through `set_late` | the outcome reaches that burst's record; `OnTime` clears `late_by` rather than recording a zero | SC-29a, SC-24a |

## 9. Vision coverage

| Vision | This spec |
|---|---|
| §21 Port, DataContract registry, explicit format conversion | SC-1…SC-5, decisions S7 and S8 |
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
| §28 "multi-channel alignment failure" as a describable outcome | SC-31a |
| §30 a Probe is a lossy link plus a Sink | SC-21, SC-22 |
| §31 MemoryDomain and BufferRef | SC-6, SC-7 |
| §34 the wire format is not a contract | SC-5 |
| §46 a transmit tap for self-interference cancellation | SC-23 |
| §17 an injected fault equals a hardware fault | SC-18 (the test is Phase 2) |

## 10. Vision issues found

1. **§23 TX rule 1 contradicts §58 #15.** TX rule 1 says the Provider closes the burst, emits a discontinuity and starts a new burst, while §58 #15 says an unclosed burst followed by a timed block yields a `TIME_ERROR` on Mock and hardware alike. Both are wanted, at different layers: the Kernel tracker never lets the device see that sequence (SC-24), and the Mock's device model must still emulate `TIME_ERROR` for a Provider that bypasses the tracker (decision S12). §58 #15's first clause tests the device model, not the Stream Contract path. The Vision wording should say which layer it means.
2. **§23 RX rule 1 leaves an unknown `lost` half-defined.** It allows `lost` to be absent, but does not say what time the next block then carries; a monotonic time cannot be both exact and unknown. Resolved by requiring timestamped producers at v4.0 (SC-13) and reserving the absent case for a future producer. The attribution of loss behind a drop-class link is why the field must travel with the block at all.
3. **§23 lists `LATE` among the flags** with neither a side nor a meaning. Defined in SC-16 and decision S19.
4. **§23 shows one buffer for N channels with no layout**, while UHD delivers one buffer per channel. Resolved by making the layout a contract attribute (SC-4).
5. **§28's `Gap.cause` has no defined value set.** Closed in SC-31.
6. **§22 puts a burst target "in the radio's ClockDomain" while §23 rule 7 puts block times in the stream's SampleClock.** These are different domains, the device root and the sample grid. SC-23 chooses the SampleClock; SC-23a converts an exactly related target with `apply` and advances it to the next sample instant when inexact, and SC-23b refuses an unrelated one on TM-5's grounds.
7. **§23 RX rule 5's "the real-time path performs no allocation" is untestable in Phase 1**, because no real-time path exists yet. Recorded as a producer rule (SC-9) whose test is the Phase 2 and Phase 8 copy-regression benchmarks.
8. **§22's `TxBurst.format` and channel list versus a block's contract and channel count.** Mapping a burst's channels onto a stream's channels is Radio Model vocabulary (Phase 2). Noted, not decided here.
9. **§23 rule 8 names the policy `block` but the Vision never says whether `publish` parks.** A parking publish cannot be called from a step-driven Island (§32), and a non-parking one makes the name a misnomer. SC-20a keeps the Vision's word and fixes the semantics: the policy produces back-pressure through a refusal the producer must honour, not through a parked thread. §23 rule 8's wording should say so.
10. **§28 requires a capture to describe a "multi-channel alignment failure" but §23 gives no way to carry the cause of a per-channel break.** Resolved by `ChannelGap` and `GapCause::Alignment` (SC-31a). The matching event kind `ALIGNMENT_ERROR` is in §29's list but appears in no §23 rule.
11. **Neither §23 nor §30 says what a drop-class link does with the flags of the blocks it drops.** Since §23 rule 8 puts recorders as well as probes behind drop-class links, silently losing a dropped block's `RESTARTED` and its lost count would misattribute a device overflow in the very artifact §50 asks to be honest. Resolved by the `DropCarry` of SC-20b.

## 11. Deferred

Block pools and real Link Modules (a single-producer single-consumer ring, shared memory). The MockRadio device model: `TIME_ERROR` emulation, length jitter, the stop tail, overflow injection. TimingEnvelope values and therefore the source of `min_lead`. The host-loop versus device-memory implementation of `repeat` and its capability constraints. A transmit-as-radiated monitor port for self-interference cancellation. The SigMF writer. The `pdu.*` and `tensor.*` contracts. The payload schemas of the event kinds named here (`RX_OVERFLOW`, `TX_UNDERFLOW`, `TX_DISCONTINUITY`, `TIME_ERROR`, `ALIGNMENT_ERROR`, `LINK_BACKPRESSURE`), which belong to spec 04. The planner hooks that apply SC-21 and SC-27's plan-time rule, which belong to specs 03 and 05. Taint conventions for Processors whose mapping is not one-to-one.

---

## Notes for specs 03, 04 and 05

- Spec 04 must define the Manifest's **namespaced section mechanism** and the `ArtifactRef`, not per-radio envelope slots. The `BurstRecord`s of a transmit stream go in the Provider's own section (SC-28), and a capture's `ContinuityMap` hangs off its `ArtifactRef`; only the `SampleClockRecord` sequence and the Run's clock relation to UTC are envelope fields, because Vision §15 and §50 name them as such. Of the event kinds named here, spec 04 registers only `LINK_BACKPRESSURE`, which is a DataLink policy and therefore the Kernel's; `RX_OVERFLOW`, `TX_DISCONTINUITY`, `TX_UNDERFLOW`, `TIME_ERROR` and `ALIGNMENT_ERROR` are registered by the Radio Model in Phase 2, under RS-27's rule that a kind belongs to whoever emits it.
- Spec 05 must carry per-port parameters (decision S7), the Sink-role predicate that SC-21 needs, the rule that a runtime-decided `TxBurst` cannot use `RejectAtPlan` (SC-27), and an optional target on the Kernel `Stop` action.
- `00-overview.md` §6 lists the records above among the documents; `SampleBlock`, `BufferRef` and the link handles are in-process only.
- Deliberate simplifications to mark in code: the 64-channel mask ceiling (S1), the mutex-based test link (S9), no pool in Phase 1 (S4), and the one-sample advance a transmit target may take (S13).

On the Kernel surface: `ContinuityMap`, `Gap`, `ChannelGap`, `Segment`, `GapCause`, `ContinuityBuilder`, `BurstTracker`, `BurstRecord`, `BurstOpen`, `BurstState`, `BurstStep`, `BurstEnd`, `LatePolicy`, `LateOutcome`, `PublishOutcome`, `DropCarry`, `Direction`, `Scalar` and `PortRef` — nineteen items — are not named individually in audit §13, which ends its data line with the catch-all "Stream Contract (normative)". Under `00-overview.md` OV-23b each goes on the allow-list as `NEW:` with a justification, and the exit review reports the count. The continuity types stay in the Kernel rather than moving to an artifact Vocabulary because Vision §50 makes continuity metadata an envelope field and §28 requires it to be derived rather than assembled, which is the same anti-divergence argument that justifies `BurstTracker`.
