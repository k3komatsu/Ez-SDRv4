# Ez-SDR v4 — Architecture Vision · Part 04: Execution Model and Data Contracts

> Sections §18–§24 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 03: Simulation, Mock and Time](03-simulation-mock-and-time.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 05: Coherence, Calibration and Runtime Semantics](05-coherence-calibration-and-runtime-semantics.md) →

---

# 18. Fixed and reactive experiments are both first-class

Ez-SDR must not be limited to:

```text
configure → start → wait → stop
```

It must support:

```text
Experiment
├── static configuration
├── scheduled actions
└── reactive execution
```

Reactive operation includes:

- packet-radio protocols,
- ARQ,
- TDD,
- adaptive modulation,
- IBFD adaptation,
- smart-antenna control,
- sensing-triggered responses,
- dynamic TX generation.

---

# 19. Processor and Reactor

The Core should define the semantic contract of two main active components.

## Processor

A Processor is primarily a data transformation component.

```text
input
  ↓
processing
  ↓
output
```

Examples:

- filtering,
- mixing,
- resampling,
- FFT/IFFT,
- synchronization,
- channel estimation,
- equalization,
- modulation/demodulation,
- coding/decoding,
- SIC.

## Reactor

A Reactor is primarily a stateful event-driven decision component.

```text
events / messages
       ↓
state machine
       ↓
zero or more actions
```

Examples:

- IEEE 802.11 MAC,
- ARQ,
- TDD controller,
- adaptive MCS controller,
- IBFD control logic,
- smart-antenna controller.

Typical actions may include:

```text
TxBurst
SetTimer
UpdateParameter
PeripheralCommand
Emit
Stop
Abort
```

The Core defines these concepts.

Concrete execution engines remain Modules.

## What the Kernel owns, and what it does not

The Kernel owns the **descriptor** of a component and the **vocabulary** it speaks, not the way it is called:

```text
ComponentDescriptor
├── kind            Processor | Reactor
├── ports           [(name, direction, DataContractId)]         (§21)
├── params          schema, each with an update class            (§27)
├── timing          budget, preferred batch, stateful, parallelism
└── impl            identity / hash of the implementation
```

The execution ABI — how a native, WASM or GPU component is invoked, how buffers are handed to it — belongs to each Executor (§20). Executors interoperate through DataLinks carrying `SampleBlock`s and through Event/Action queues, never through a shared call signature. This keeps the Kernel out of the block-API business that GNU Radio carries in its core and lets a GPU batch executor and a WASM block executor coexist without a lowest common denominator.

## Cycles

A Reactor that receives a decoded packet and schedules a TxBurst forms a cycle through the radio. Such cycles are allowed **only across Execution Island boundaries and only through Event/Action edges**, which are asynchronous and queued. `SampleStream` edges never form cycles. Adaptive feedback inside a Processor (filter coefficients updated from its own output) is internal state, not graph structure.

## Two kinds of deadline

```text
RelativeBudget   { duration }     a Processor's processing time per block, measured from block arrival
AbsoluteDeadline { time_point }   a TxBurst or PeripheralCommand target in a device ClockDomain
```

They are different types. A missed RelativeBudget is a typed event with a policy; a missed AbsoluteDeadline follows the burst's late policy (§22). Admission checks use budgets; envelope checks use absolute deadlines.

Multi-rate behaviour (a decimator producing fewer samples than it consumes) is the Executor's concern inside an Island; the Kernel only uses declared port rates for validation.

---

# 20. Processing execution is replaceable

Processing Executors include:

```text
Native host CPU
WASM
GPU
```

RFNoC is deliberately absent from this list. RFNoC blocks are device-side functions that the Radio Provider configures (Replay for `tx.repeat`, DDC/DUC for rate conversion, device FFT); placing an arbitrary user Processor there would mean building an FPGA image, which is a non-goal (§63). RFNoC therefore appears as **Radio Model capabilities implemented by the UHD Provider**, with their constraints (memory size, word alignment), not as a Processor placement target (§35). Custom blocks remain reachable through `extensions.uhd.rfnoc.*`.

The same high-level Processor concept may have multiple implementations.

Placement is **explicit, not selected**: the BindingProfile's `placements` section states which Executor and which MemoryDomain each processing component runs in. The Spec states requirements only (`executor_kind`, budget, memory needs) and never names a placement, so the same Spec can be bound to a GPU host, a CPU-only HIL rig or a pure simulation. The Core validates the choice against:

- available capabilities,
- timing requirements and declared budgets,
- DataContract compatibility, including sample format,
- performance envelope,
- MemoryDomain reachability through an available DataLink,
- component implementation identity,
- resource limits,

and rejects what does not fit. It does not search for a better arrangement.

The abstraction must not reduce all execution models to an unusably weak lowest common denominator. Executors keep their own execution ABI (§19); they share descriptors, DataContracts and DataLinks, not a call signature.

Backend-specific extensions remain available when necessary.

---

# 21. Data is not always IQ

The graph model must not assume every edge carries raw IQ.

Ports do not carry a closed enum of data kinds. A Port names a **DataContract**, and DataContracts live in an **open registry**:

```text
Port          { name, direction, contract: DataContractId }
DataContract  { id, attributes, compatibility rules }

registered at v4.0:      ezsdr.stream.cf32 { full_scale: 1.0 }, ezsdr.stream.sc16, ...
                         ezsdr.event.<schema-id>, ezsdr.control
registered later,        ezsdr.pdu.bytes { max_len }, ezsdr.pdu.ethernet
without changing Port:   ezsdr.tensor.cf32 { shape: [...] }, ezsdr.annotation.timed
```

The Kernel checks two things only: that connected contracts have the same identity, or that a declared compatibility (an explicit conversion) exists. Tensor shape algebra, PDU framing rules and similar checks belong to the Executor or to a validation plugin of the Vocabulary that defines the contract. The Kernel does not become a type system.

Contracts that must be expressible without changing the Port model:

```text
SampleStream<T>
Packet / PDU<T>
Tensor<T, Shape>
Event<T>
Control
TimedAnnotation
```

Examples:

```text
Ethernet Frame
      ↓
MAC
      ↓
IEEE 802.11 MPDU
      ↓
PHY
      ↓
IQ SampleStream
```

or:

```text
IQ samples
   ↓
OTFS Processor
   ↓
Delay-Doppler Tensor
```

This is required for:

- packet radios,
- MIMO,
- OTFS,
- ISAC,
- channel matrices,
- beamforming,
- AI processing.

## Format conversion is explicit

Two connected ports must name the same DataContract, or a declared compatibility must exist; otherwise `validate()` rejects the link. There is no automatic insertion of conversion components in v4.0. A standard `Convert` Processor (`sc16 → cf32`, interleaving changes, scaling) is placed explicitly in the graph like any other component; automatic insertion is future work (§64). The over-the-wire format between host and device (`sc16`, `sc8`) is a Provider-internal choice reported in its PerformanceEnvelope (§34), not a graph contract. v3 carried `srvfmt`/`devfmt` per streamer and rejected mismatches at set-up time; v4 keeps the explicitness and moves the check to `validate()`.

---

# 22. Dynamic TxBurst is a first-class primitive

TX must support more than:

```text
continuous
finite
repeat
```

A Processor or Reactor must be able to generate a burst dynamically during RUN.

Conceptually:

```text
TxBurst
├── data / waveform reference
├── format
├── channel(s)
├── repeat (optional)
├── target TimePoint       an AbsoluteDeadline in the radio's ClockDomain (§19)
├── metadata
└── late_policy            (below)
```

This is essential for:

- packet responses,
- ARQ,
- TDD,
- reactive protocols,
- closed-loop experiments.

A TxBurst names a target TimePoint in the radio's ClockDomain. Two rules make it portable between Mock and hardware:

- The target must satisfy the bound Provider's `min_timed_command_lead` (§13). When the lead is statically known (scheduled actions), `validate()` checks it at plan time; when it is decided at run time (Reactor responses), the Provider enforces it and emits a typed event.
- Every burst carries a **late policy**:

```text
LatePolicy
├── reject_at_plan          fail validation if the lead cannot be met
├── send_asap_and_flag      transmit at the earliest possible time, emit LATE, flag the burst
└── drop_and_flag           do not transmit, emit LATE
```

MockRadio applies the same policy with the same envelope, so a Reactor that is too slow for the hardware fails in simulation.

A TxBurst is **one burst** of the TX stream (§23): its blocks carry `START_OF_BURST` on the first and `END_OF_BURST` on the last, and `repeat` is an attribute of the burst, not a stream of re-sent blocks.

---

# 23. Time is first-class and must preserve sample relationships: the Stream Contract

Do not use ordinary floating-point seconds as the fundamental timing representation. Time is integer ticks in a named ClockDomain (§15).

The Runtime must preserve:

- hardware/device time,
- virtual time,
- clock domains,
- timestamps,
- sample-relative offsets,
- PPS-relative time,
- deadlines,
- timers,
- uncertainty where relevant.

A packet detected at sample offset `k` must retain a precise relationship to device time.

## The Stream Contract

Every Radio, Processing, Host-I/O and Simulation contract exchanges sample data as `SampleBlock`s. The contract is normative: it says what a producer guarantees and what it is forbidden to do. v3 had no such contract; its UHD bridge discarded the receive error code and the receive timestamp, so overflows produced silently gapped, untimed arrays.

```text
SampleBlock
├── first_sample_time   TimePoint in the stream's SampleClock
├── len                 samples per channel in this block
├── channels            N aligned channels (one stream, one timestamp)
├── valid               per-channel validity (mask or range list)
├── flags               GAP_BEFORE { lost: optional count }
│                       SEQ_DISCONTINUITY
│                       RESTARTED
│                       LATE
│                       PARTIAL_CHANNELS
│                       START_OF_BURST / END_OF_BURST   (TX side, below)
├── contract            DataContract id (sample format, full-scale convention)
└── buffer              BufferRef { memory_domain, handle, len }
```

Rules for producers and consumers:

1. **Time is monotonic within a stream** (within one SampleClock; a sample-rate change starts a new SampleClock, §15). A gap is represented by a time jump plus `GAP_BEFORE`; the number of lost samples is given when known. **Gaps are never filled**, not with zeros, not with repeated data.
2. **Validity is per channel.** A multi-channel alignment failure or a partial loss marks the affected channels, not the whole block.
3. **Block length is not guaranteed.** Hardware delivers packet-sized blocks (about 2000 samples at 10 GbE with `sc16`); a Mock may deliver any length and *should* offer a length-jitter option so that Processors that assume a fixed block size fail in simulation.
4. **The full-scale convention is part of the DataContract.** For `cf32`, ±1.0 is ADC/DAC full scale; MockRadio clips at the same level.
5. **Blocks are immutable after publish, reference-counted, and allocated from the producing island's pool.** Fan-out (a Probe, a second consumer) shares the reference; it never copies. The real-time path performs no allocation.
6. **Buffers are handles tagged with a MemoryDomain** (host, pinned host, GPU, WASM linear memory, device memory). No Module contract takes or returns raw host slices; an Executor that needs host memory asks the DataLink for it.
7. **TX blocks carry the target TimePoint of their first sample, also in continuous mode.** The Provider enforces the lead (§13, §22). A tap on the TX edge therefore yields a timed reference for digital self-interference cancellation (§46) without any special API.
8. **Every DataLink declares a back-pressure policy**: `block`, `drop_oldest` or `drop_newest`. Links feeding Probes and recorders are always drop-class, so observation can never stall the real-time path (§30).

An overflow on UHD hardware returns zero samples, distinguishes buffer overrun from host-side sequence error, and restarts a continuous stream about 50 ms later. Under this contract it appears as one block boundary with `GAP_BEFORE`, `RESTARTED` and a time jump of the restart gap, identically from a UHD Provider and from a MockRadio fault injection.

The artifact-level `ContinuityMap` (§28) is derived from these flags; it is never assembled by hand. For SigMF export, gaps become capture segments with `core:sample_start` and `core:global_index`, and per-channel validity is carried in an `ezsdr` extension namespace, since no existing SigMF extension covers validity.

## The TX side

A TX stream is a **sequence of bursts**. A burst is a run of blocks whose times are contiguous in the stream's SampleClock; the first block carries `START_OF_BURST`, the last carries `END_OF_BURST`. UHD requires exactly this: every start of burst needs a time specification (the device resets its CORDICs on it), and a burst must be closed with an end-of-burst flag, otherwise the next timed packet is a `TIME_ERROR`.

Rules:

1. **Within a burst, time is contiguous.** A block whose time does not follow the previous block's end, without an intervening `END_OF_BURST`, is a `TX_DISCONTINUITY`: the Provider closes the burst, emits the typed event, and treats the block as the start of a new burst under that burst's late policy (§22). It never pads the gap with zeros.
2. **A TxBurst (§22) is one burst.** Its target TimePoint is the time of its first block; its blocks arrive with `START_OF_BURST` on the first and `END_OF_BURST` on the last. A dynamically generated response is one or more bursts, never a "gap in a continuous stream".
3. **Continuous TX is one long burst** that ends with the Run's stop or an explicit `END_OF_BURST`. Underflow (the host did not deliver the next block in time) is a typed `TX_UNDERFLOW` event carrying the time at which it occurred; the Provider does not fabricate samples, and what the hardware radiated during the gap (silence or a device-specific hold) is recorded in the Provider's Manifest section.
4. **`repeat` is a burst attribute**, not a stream of re-sent blocks. A burst marked `repeat` is transmitted cyclically until stopped; the Provider implements it with a host loop or with device memory (Replay), and the capability carries the implementation's constraints (maximum length, word alignment; §20, §35). A host-loop implementation must not underflow at the wrap, which is a v3 behavioural compatibility test (§61).
5. **What was transmitted is recorded.** For every burst the Manifest records the target time, the actual start time where the Provider can know it (burst acknowledgement), and any `LATE`, `TX_UNDERFLOW` or `TX_DISCONTINUITY` events. A tap on the TX edge (§46) sees blocks with these flags and times.

MockRadio applies the same rules: on an `x310-like` profile an unclosed burst followed by a timed block is a `TIME_ERROR`, exactly as on the hardware.

---

# 24. ClockRelation is necessary for heterogeneous experiments

Modern experiments may involve:

```text
USRP clock
host monotonic clock
PTP clock
camera clock
smart-antenna controller clock
positioner clock
external trigger clock
distributed host clocks
virtual simulation clock
```

The system should be able to represent relationships such as:

```text
ClockRelation
├── source clock
├── target clock
├── offset
├── drift
├── uncertainty
├── measurement method
├── measured_at
└── validity interval
```

This is important for:

- ISAC,
- distributed experiments,
- external sensors,
- multi-host systems,
- reproducible timing claims.

---

← [Part 03: Simulation, Mock and Time](03-simulation-mock-and-time.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 05: Coherence, Calibration and Runtime Semantics](05-coherence-calibration-and-runtime-semantics.md) →
