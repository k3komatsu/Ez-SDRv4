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

A Reactor's actions are the closed Kernel Action set — `TxBurst`, `SetTimer`, `UpdateParameter`, `PeripheralCommand`, `Emit`, `Stop`, `Abort` — and adding one is a Kernel major ([design/04-run-and-session.md](../04-run-and-session.md), rules RS-48…RS-52).

The Core defines these concepts.

Concrete execution engines remain Modules.

## What the Kernel owns, and what it does not

The Kernel owns the **descriptor** of a component and the **vocabulary** it speaks, not the way it is called:

A ComponentDescriptor declares its kind (Processor or Reactor), its ports (§21), its params each with an update class (§27), its timing (budget, preferred batch, statefulness, parallelism), its requirements — never a placement — and the identity and hash of its implementation ([design/05-module-api.md](../05-module-api.md) §4, rules MA-36, MA-37).

The execution ABI — how a native, WASM or GPU component is invoked, how buffers are handed to it — belongs to each Executor (§20). Executors interoperate through DataLinks carrying `SampleBlock`s and through Event/Action queues, never through a shared call signature. This keeps the Kernel out of the block-API business that GNU Radio carries in its core and lets a GPU batch executor and a WASM block executor coexist without a lowest common denominator.

## Cycles

A Reactor that receives a decoded packet and schedules a TxBurst forms a cycle through the radio. Such cycles are allowed **only across Execution Island boundaries and only through Event/Action edges**, which are asynchronous and queued. `SampleStream` edges never form cycles. Adaptive feedback inside a Processor (filter coefficients updated from its own output) is internal state, not graph structure.

## Two kinds of deadline

There are two kinds of deadline, and they are distinct types. A RelativeBudget is a processing time per block, measured from block arrival; it is always in `host.monotonic`, because a static descriptor cannot name a domain created at `prepare`, and a missed one is the Kernel event `PROCESSOR_DEADLINE_MISS`, handled by the Policy table. An AbsoluteDeadline is a TxBurst or PeripheralCommand target in a device ClockDomain — for a TxBurst, the transmit stream's SampleClock — and a missed one follows the burst's late policy (§22). Admission compares budgets with block periods exactly across domains; envelope checks compare absolute deadlines.

Normative: [design/01-time-model.md](../01-time-model.md), rules TM-15, TM-21; [design/04-run-and-session.md](../04-run-and-session.md), rules RS-27, RS-49, RS-51; [design/02-stream-contract.md](../02-stream-contract.md), SC-23.

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

A Port is a name, a direction and a DataContract id, and nothing more. DataContracts live in an open registry under namespaced ids, each with fixed attributes and a declared `compatible_from` set, and the Kernel's only contract check is directional: a link is admissible when the producer's contract equals the consumer's or is in its `compatible_from`. The contracts available at v4.0 belong to the contracts Vocabulary — `ezsdr.stream.cf32` and `ezsdr.stream.sc16`, planar, each with its full-scale convention; `ezsdr.control`; `ezsdr.event.<schema-id>` — and `pdu.*` and `tensor.*` contracts are added later without changing Port. The host-to-device wire format is never a Port contract.

Normative: [design/02-stream-contract.md](../02-stream-contract.md), rules SC-1…SC-5.

Tensor shape algebra, PDU framing rules and similar checks belong to the Executor or to a validation plugin of the Vocabulary that defines the contract. The Kernel does not become a type system.

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

A TxBurst is a Kernel Action carrying a waveform by reference, a `repeat` attribute, a target `at`, the originally requested target, a late policy and namespaced metadata. `TxBurst.format` is determined by the transmit stream's Stream Contract, and the burst uses every channel of that stream; a Radio Model's channel mapping travels in namespaced metadata, not in a Kernel field (Phase 2).

This is essential for:

- packet responses,
- ARQ,
- TDD,
- reactive protocols,
- closed-loop experiments.

The target is an AbsoluteDeadline in the transmit stream's SampleClock: a target in another exactly related domain is converted, advanced to the next sample instant only when the conversion is inexact, with both times kept, and one in an unrelated domain is refused. Every burst carries a **late policy** — `reject_at_plan`, `send_asap_and_flag` or `drop_and_flag` — decided against the target Provider's `min_command_lead` (§13), declared in `host.monotonic` and compared exactly; `reject_at_plan` is legal only for a statically known target. When the lead is statically known (scheduled actions), the coordinator checks it at arm, once T0 exists and before start; it uses the ProviderInstance's generic `min_command_lead` value, not a Radio Model key. When it is decided at run time (Reactor responses), the Provider enforces it and emits a typed event of the Radio Model's, while the lateness itself is recorded on the burst's record. `LATE` is a block flag (§23), not the name of this event.

Normative: [design/04-run-and-session.md](../04-run-and-session.md), rules RS-49, RS-51; [design/02-stream-contract.md](../02-stream-contract.md), rules SC-23, SC-23a, SC-23b, SC-26…SC-29a; [design/05-module-api.md](../05-module-api.md), MA-14.

MockRadio applies the same policy with the same envelope, so a Reactor that is too slow for the hardware fails in simulation.

A TxBurst is **one burst** of the TX stream (§23), and `repeat` is an attribute of the burst, not a stream of re-sent blocks.

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

A SampleBlock is an immutable, reference-counted header plus a buffer handle. The header holds the first sample's TimePoint in the stream's SampleClock, a length, a channel count, a per-channel validity mask, a direction, fixed-position flags — `GAP_BEFORE` with an optional lost count, `SEQ_DISCONTINUITY`, `RESTARTED`, `LATE`, `PARTIAL_CHANNELS`, `ALIGNMENT`, `START_OF_BURST`, `END_OF_BURST` — and the DataContract id; the block's constructor refuses an inconsistent header. The handle is a BufferRef naming a memory domain, whose kinds (host, pinned, GPU, …) are Vocabulary content: the Kernel compares domain identities and never dereferences a handle.

Normative: [design/02-stream-contract.md](../02-stream-contract.md), rules SC-6…SC-18 (blocks), SC-19…SC-22 (links), SC-23…SC-29a (transmit), SC-30…SC-32 (derivation).

The numbered rules below are cited by number; each is now stated by the rules it names.

| Rule | Headline | Normative |
|---|---|---|
| RX 1 | Time is monotonic within one SampleClock; a gap is a time jump plus `GAP_BEFORE`, never filled | SC-12, SC-13 |
| RX 2 | Validity is per channel and constant within a block | SC-14, SC-31a |
| RX 3 | Block length is not guaranteed | SC-15 |
| RX 4 | The full-scale convention is a contract attribute | SC-4 |
| RX 5 | Immutable, reference-counted, pool-allocated; fan-out shares the reference; no allocation on the real-time path | SC-9, SC-11, SC-22 |
| RX 6 | Buffers are handles tagged with a memory domain; no raw host slices | SC-6…SC-8 |
| RX 7 | Transmit blocks carry their first sample's target time, in continuous mode too | SC-23 |
| RX 8 | Every link declares a policy and a capacity; `block` is back-pressure by refusal, not a parked thread; Sink links are drop-class | SC-19…SC-21 |

A Mock may deliver any block length and should offer a length-jitter option, so that a Processor assuming a fixed block size fails in simulation, and MockRadio clips at the contract's full scale (Phase 2 Mock obligations).

An overflow on UHD hardware returns zero samples, distinguishes buffer overrun from host-side sequence error, and restarts a continuous stream about 50 ms later; under this contract it appears as one block boundary with `GAP_BEFORE`, `RESTARTED` and the restart gap as its time jump, identically from a UHD Provider and from a MockRadio fault injection (SC-18).

The artifact-level `ContinuityMap` (§28) and a SigMF export are derived from these headers and never assembled by hand (SC-30…SC-32).

## The TX side

A TX stream is a **sequence of bursts**. A burst is a run of blocks whose times are contiguous in the stream's SampleClock; the first block carries `START_OF_BURST`, the last carries `END_OF_BURST`. UHD requires exactly this: every start of burst needs a time specification (the device resets its CORDICs on it), and a burst must be closed with an end-of-burst flag, otherwise the next timed packet is a `TIME_ERROR`.

| Rule | Headline | Normative |
|---|---|---|
| TX 1 | Time is contiguous within a burst; a jump closes the burst, is reported as `TX_DISCONTINUITY` and opens a new one, never padded | SC-24, SC-24a |
| TX 2 | A TxBurst is one burst | SC-23, SC-24 |
| TX 3 | Continuous TX is one burst; an underflow is `TX_UNDERFLOW`, and no samples are fabricated | SC-25 |
| TX 4 | `repeat` is a burst attribute, contiguous across the wrap | SC-26 |
| TX 5 | What was transmitted is recorded | SC-28, SC-29, SC-29a |

The Kernel's burst tracker never lets a device see an unclosed burst followed by a timed block; MockRadio's device model still raises `TIME_ERROR` for a Provider that bypasses it, exactly as the hardware does.

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

A ClockRelation records a measured relation from a source clock to a target clock: `measured_at` in the source, `offset` as that instant's image in the target, a `drift` with a bound on the drift's own error, an uncertainty valid at `measured_at`, a namespaced measurement method, and a validity interval. Converting through it yields a nominal time plus an uncertainty that grows with the time elapsed since `measured_at`, because drift is itself a measurement. The Kernel never chains relations: a caller that needs source → A → target measures or composes that relation explicitly.

Normative: [design/01-time-model.md](../01-time-model.md), rules TM-5, TM-14, TM-18 (the shape: §4, `ClockRelation`).

This is important for:

- ISAC,
- distributed experiments,
- external sensors,
- multi-host systems,
- reproducible timing claims.

---

← [Part 03: Simulation, Mock and Time](03-simulation-mock-and-time.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 05: Coherence, Calibration and Runtime Semantics](05-coherence-calibration-and-runtime-semantics.md) →
