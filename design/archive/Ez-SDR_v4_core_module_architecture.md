# RETIRED — Ez-SDR v4 Core / Module Architecture Principles

> **Status:** RETIRED on 2026-09-21. This document is no longer maintained. It duplicated the Architecture Vision at principle level and had to be synchronised by hand on every revision; its two passages without a counterpart (the Module communication example and the smart-antenna litmus test) were folded into Vision §7 and §66 before retirement.  
> **Superseded by:** the Architecture Vision — index `Ez-SDR_v4_ARCHITECTURE_VISION.md`, parts under `design/vision/`. Where this text and the Vision differ, the Vision governs.  
> **Why it is kept:** `design/v4-vision-audit.md` and `design/v4-vision-rereview.md` cite this document as `CMA §N`. The text below is the final version those citations refer to.  
> **Section map (CMA § → Vision §):**

| CMA § | Vision § |
|---|---|
| 1 | §1 |
| 2 | §2 |
| 3 | §4 |
| 4 | §5 |
| 5 | §6 |
| 6 | §7 |
| 7 | §8 |
| 8 | §8 |
| 9 | §8, §59 |
| 10 | §12 |
| 11 | §13 |
| 12 | §13 |
| 13 | §13 |
| 14 | §16 |
| 15 | §16, §46 |
| 16 | §14, §15, §17 |
| 17 | §14 |
| 18 | §15 |
| 19 | §19 |
| 20 | §7 (Module communication rule), §39 |
| 21 | §21, §23 |
| 22 | §40 |
| 23 | §41 |
| 24 | §41 |
| 25 | §37, §38 |
| 26 | §39 |
| 27 | §23, §31 |
| 28 | §11 |
| 29 | §11, §32 |
| 30 | §60 |
| 31 | §57 |
| 32 | §59 |
| 33 | §44 |
| 34 | §52 |
| 35 | §36 |
| 36 | §29 |
| 37 | §17 |
| 38 | §50 |
| 39 | §65 |
| 40 | §66 |
| 41 | §67 |
| 42 | §68 |

---

*Archived text follows unchanged.*

---
# Ez-SDR v4 — Core / Module Architecture Principles

> **Status:** Architecture principles / implementation boundary document  
> **Purpose:** Define how the large Ez-SDR v4 vision is decomposed into a small, stable Core and replaceable sub-modules.  
> **Primary goal:** Keep the Core clean enough to evolve safely, while allowing advanced SDR experiments, simulation, AI-agent validation, heterogeneous acceleration, and external laboratory hardware without turning Ez-SDR itself into a monolithic framework.  
> **Revision (2026-09-21):** Aligned with the P0, P1 and P2/P3 findings of `design/v4-vision-audit.md` (see the Architecture Vision §5, §7–§11, §13–§15, §19–§23, §25–§30, §33–§35, §37–§38, §41, §47, §49–§50, §52–§53, §60–§64). Also applies R1, R2, R3, R6, R17, R21, R22 and the editorial items R8 and R11 of `design/v4-vision-rereview.md`. Where this document and the Vision differ, the Vision governs.  
> **Structure note (2026-09-21):** The Architecture Vision is split into eleven part files under `design/vision/`; `Ez-SDR_v4_ARCHITECTURE_VISION.md` is its index, reading guide and revision history. Section numbers (§N) are unchanged. This document remains a single file and summarises the same principles; where it differs from the Vision, the Vision governs.

---

## 1. Motivation

The Ez-SDR v4 vision now includes substantially more than simple remote SDR control:

- high-rate RX/TX,
- UHD / USRP,
- SoapySDR,
- ExperimentSpec,
- reproducible Runs and Artifacts,
- reactive Processor / Reactor execution,
- dynamic TX bursts,
- real-time DSP,
- RFNoC-backed radio capabilities,
- GPU processing,
- future RT-WASM execution,
- Linux TUN/TAP networking,
- smart antennas and other external equipment,
- structured UHD health/error reporting,
- MIMO / coherent operation,
- IBFD,
- ISAC,
- OTFS,
- AI-agent-driven experimentation,
- software-only simulation/emulation,
- future distributed execution.

If all of these are implemented directly in a single runtime, Ez-SDR will become unnecessarily large, tightly coupled, and difficult to verify.

The central architectural response is:

> **Ez-SDR Core should be a small experiment microkernel. Hardware, processing engines, host I/O, simulation models, and laboratory devices should be Modules / Providers that implement stable Core contracts.**

The Core should define **meaning, lifecycle, time, ownership, capabilities, events, bindings, and provenance**.

Modules should define **how concrete functionality is implemented**.

---

# 2. One-sentence architecture

> **Ez-SDR v4 is a small typed Experiment Microkernel surrounded by replaceable capability-providing Modules, with software simulation treated as a first-class execution target equal to physical SDR hardware.**

---

# 3. High-level architecture

```text
                    Python / CLI / AI Agent
                              │
                              ▼
┌─────────────────────────────────────────────────────────┐
│                     Ez-SDR Core                         │
│                                                         │
│ Experiment / Run / Resource / Capability / Time         │
│ Binding / Lifecycle / Event / Policy / Provenance       │
│ ExecutionPlan composition / Module registry             │
│                                                         │
│                "Experiment Microkernel"                 │
└────────────────────────┬────────────────────────────────┘
                         │ Module Contracts
        ┌────────────────┼─────────────────┬──────────────────┐
        ▼                ▼                 ▼                  ▼
    Radio Module    Processing Module  Host-I/O Module  Peripheral Module
        │                │                 │                  │
   ┌────┼────┐      ┌────┼─────┐      ┌────┼────┐       ┌────┼─────┐
 Mock  UHD  Soapy   Native WASM GPU   TUN TAP UDP    SmartAnt RF-SW ...
  │     │
  │   RFNoC-backed radio capabilities (Replay repeat, DDC/DUC, device FFT)
  ▼
 Simulation Environment
```

The Core coordinates the experiment. It should not contain the implementation details of the devices or execution engines.

---

# 4. What belongs in Core

The Core should remain deliberately small, and "small" is measured on the **Kernel**, the frozen tier. The Core has three tiers (Vision §5):

```text
Kernel       frozen, strict semver     Run/Session lifecycle, Lease, ClockDomain/TimePoint/ClockRelation,
                                       TimeAuthority, SampleBlock/BufferRef/MemoryDomain (Stream Contract),
                                       DataContract registry, Port, DataLink identity, Event/Action set,
                                       Policy, generic Capability matching, composite Resource tree,
                                       Spec/Binding/Plan/Manifest envelopes, Module API, ExecutionClass
Vocabulary   versioned per crate       Radio Model (incl. TimingEnvelope, PerformanceEnvelope, coherence
                                       basis), Calibration, Peripheral model, Endpoint model, standard
                                       DataContracts
Extensions   unstable, namespaced      backend-specific power
```

Modules depend on the Kernel plus specific versions of the Vocabulary crates they use. New experiment types add Vocabulary, never Kernel concepts.

The Kernel should own:

- experiment intent and the compiled plan,
- resource requirements and generic capability matching,
- resource binding and the composite resource tree,
- time and clock-domain semantics, and the Time Authority,
- the Stream Contract and buffer ownership,
- lifecycle and ownership,
- experiment-level failure policy,
- structured events and actions,
- reproducibility metadata,
- transactional prepare / arm / start / stop / cleanup in dependency order,
- provenance of all participating Modules.

The Kernel should **not** own the implementation of 200 Msps stream processing loops, radio-specific capability keys, Processor execution ABIs, placement optimisation, or calibration algorithms.

---

# 5. What must not belong in Core

The Core should not directly contain implementation-specific knowledge such as:

```text
UHD API
USRP model-specific behavior
SoapySDR API
HackRF quirks
RFNoC graph construction
Replay DRAM management
Wasmtime runtime
CUDA API
Linux /dev/net/tun
AF_XDP
DPDK
USB / serial protocols
smart-antenna vendor SDKs
IEEE 802.11 PHY/MAC algorithms
OTFS detector implementations
IBFD canceller algorithms
Processor execution ABIs
automatic placement optimisation
expression or template languages inside ExperimentSpec
a rules engine for failure Policy
RFNoC graph construction for user processing
a metrics framework
a Probe or Taint type
sensor-data ingestion (video, motion capture)
```

These belong in Modules.

Ideally, many of these terms should not even appear in the Core crate.

---

# 6. Model + Provider as a universal pattern

Ez-SDR should use a consistent pattern:

```text
Domain Model
    │
    ├── Provider A
    ├── Provider B
    └── Provider C
```

For example:

```text
Radio Model
    ├── Mock Radio Provider
    ├── UHD Radio Provider
    └── Soapy Radio Provider
```

Likewise:

```text
NetworkEndpoint Model
    ├── Linux TUN Provider
    ├── Linux TAP Provider
    ├── UDP Provider
    ├── PCAP Provider
    └── future AF_XDP Provider
```

And:

```text
Peripheral Model
    ├── Smart Antenna Provider
    ├── RF Switch Provider
    ├── USB Attenuator Provider
    └── Mock Peripheral Provider
```

The model defines **what a resource means**.

The provider defines **how that resource is implemented**.

Terminology follows three orthogonal axes (Vision §7): the **Module** is the unit of versioning and deployment; its **roles** are Provider (Resource Model), Executor (Processing engine), Sink (Artifacts), Link (DataLink) and Authority (Time Authority: a device timekeeper or the Simulation Engine); its **deployment form** is in-process or Plugin (out-of-process, typed protocol). One Module may play several roles.

---

# 7. Experiment intent must be separated from physical binding

This is one of the most important design rules.

An ExperimentSpec should normally state:

> **What capability the experiment requires**

rather than:

> **Which exact hardware implementation must be used**

For example:

```json
{
  "resources": {
    "radio": {
      "kind": "radio",
      "requires": {
        "rx": { "channels": 2, "coherent": true },
        "tx": { "channels": 2 },
        "full_duplex": true,
        "sample_rate_hz": 20000000,
        "hardware_time": true
      }
    }
  }
}
```

The keys are defined by the Radio Model; the Kernel matcher is generic. A radio is a **composite resource** (device → channels, streams, timekeeper, GPIO banks, replay memory), and a request for coherent channels must bind to one Provider instance that can declare that coherence (Vision §8, §25).

The physical implementation should be supplied separately by a **BindingProfile**.

---

# 8. BindingProfile

The same ExperimentSpec should be executable using different environments.

## Hardware binding

```yaml
bindings:
  radio:
    provider: ezsdr.radio.uhd
    selector:
      model: x310
      serial: "123456"
```

## Software simulation binding

```yaml
bindings:
  radio:
    provider: ezsdr.radio.mock
    profile: x310-like
```

## Environment

A BindingProfile is `bindings` plus `placements` plus `environment`. The `placements` part maps each processing component of the Spec's graph to an Executor and a MemoryDomain (Vision §20); the Spec itself never names a placement, and its radio requirements are stated per direction (`rx`, `tx`). The environment holds what surrounds the experiment without being part of its intent: for simulation the virtual-time class, the SimulationChannel model and the fault schedule; for the laboratory the clock distribution and the RF path. **The ExperimentSpec never contains environment.** A channel emulator that the experiment itself drives is a Peripheral resource in the Spec instead.

```yaml
bindings:
  radio:
    provider: ezsdr.radio.mock
    profile: x310-like
    instances: 2
environment:
  time: { class: simulation }
  channel: { model: awgn, snr_db: 10, delay_samples: 37 }
  faults:
    - { at: "2.5s", inject: rx_overflow, target: "radio[0].rx" }
```

For the laboratory, `environment` also carries the RF safety envelope (allowed bands, maximum gain or power, TX enable per channel) that `validate()` and `prepare()` enforce before any transmission (Vision §52).

The fundamental relationship is:

```text
                  ExperimentSpec
                        │
                ┌───────┴────────┐
                │                │
         BindingProfile     BindingProfile
          simulation         laboratory
                │                │
                ▼                ▼
           Mock Radio         UHD Radio
```

The experiment logic should not change.

---

# 9. Same ExperimentSpec from simulation to hardware

The desired development path is:

```text
ExperimentSpec
      │
      ├── Software Simulation
      ├── Real-time Emulation
      ├── Hardware-in-the-Loop
      └── Physical SDR Hardware
```

Only the binding and environment change.

The following should remain unchanged whenever possible:

- ExperimentSpec,
- Processor graph,
- Reactor logic,
- Packet/PDU flow,
- failure policies,
- timing semantics,
- Python orchestration,
- AI-agent workflow.

This is one of the most important capabilities of Ez-SDR v4.

---

# 10. Mock must be a first-class Provider

The software implementation must **not** be designed as:

```text
UHD Backend
    ↑
MockUHDBackend
```

That architecture makes the mock implementation depend on UHD semantics.

Instead:

```text
                 Radio Contract
                 /            \
                /              \
       MockRadioProvider    UHDRadioProvider
                                  │
                                 UHD
```

The Mock provider and UHD provider are peers.

The Mock provider is an implementation of the abstract Radio model, not a fake UHD implementation, and it enforces the Radio model's TimingEnvelope (§13).

---

# 11. Software-only execution is a primary target

Ez-SDR should be usable with **no SDR hardware installed**.

A complete experiment should be able to execute against:

```text
Mock Radio
Virtual Clock
Simulation Channel
Mock Network Endpoint
Mock Peripheral Devices
Fault Injection
```

The AI agent should be able to:

1. generate or modify an experiment,
2. run it entirely in software,
3. inspect events and artifacts,
4. test failure behavior,
5. benchmark real-time components,
6. only then promote the experiment to HIL or physical SDR hardware.

This is not merely a test feature.

It is a core part of the Ez-SDR development model.

---

# 12. Simulation Environment

The software environment should eventually contain:

```text
Simulation Environment
├── MockRadio
├── SimulationChannel
├── SimulationEngine (discrete-event Time Authority; virtual clock, §18)
├── MockPeripheral
├── MockNetworkEndpoint
└── FaultInjector
```

The Simulation Environment should be deterministic when requested.

---

# 13. Mock Radio

MockRadio is a peer of the hardware Providers because it implements the same contract **and enforces the same TimingEnvelope** (Vision §13). From its first implementation it models:

```text
virtual hardware clock
RX channels
TX channels
sample-rate constraints
frequency tuning
timed RX
timed TX
TxBurst scheduling
FIFO behavior
transport delay
minimum timed-command lead
start-up latency and stop tail
timed-command queue depth
rate / gain / frequency coercion grids
overflow (zero samples, restart gap, block flags)
underflow
clock drift
capabilities and envelopes
```

It should present exactly the same logical Radio contract seen by higher-level components, and it must reject or flag what its emulated hardware would reject: insufficient lead for a timed command, an unsupported sample rate (coerced on the same grid and reported), a throughput beyond the profile's PerformanceEnvelope, a stop without its tail. A software radio that accepts everything, the OpenAirInterface `rfsimulator` pattern, produces experiments that pass in software and fail on the bench. RF behaviour is not MockRadio's job; it belongs to SimulationChannel (§14).

---

# 14. Simulation Channel

RF propagation and impairment simulation should be separated from MockRadio.

Conceptually:

```text
MockRadio TX
      │
      ▼
SimulationChannel
      │
      ├── AWGN
      ├── multipath
      ├── CFO
      ├── phase noise
      ├── delay
      ├── Doppler
      ├── path loss
      ├── PA nonlinearity
      ├── IQ imbalance
      ├── clipping
      ├── self-interference coupling
      └── MIMO channel
      │
      ▼
MockRadio RX
```

This separation is useful for:

- SISO communication,
- MIMO,
- IBFD,
- ISAC,
- OTFS,
- channel sounding,
- smart-antenna evaluation.

The channel is a coupling matrix over all TX ports × all RX ports, including a device's own RX (self-interference is the diagonal block). Its configuration lives in the BindingProfile environment (§8), not in the ExperimentSpec.

---

# 15. IBFD software testbed

The architecture should allow an IBFD chain such as:

```text
TX Processor
     │
     ▼
 Mock TX
     │
     ├──────────── desired channel ──────────┐
     │                                       │
     └─ PA / IQI / SI-channel model ────────┤
                                             ▼
                                          Mock RX
                                             │
                                             ▼
                                      SIC Processor
```

The same SIC Processor should later be runnable against a real X310 without changing its external contract.

---

# 16. Simulation execution modes

At least three execution modes should be supported conceptually.

## 16.1 Functional Simulation

Simulation runs as fast as possible.

```text
10 seconds simulated RF time
        ↓
0.3 seconds wall-clock execution
```

This mode is ideal for:

- AI-agent validation,
- algorithm tests,
- CI,
- deterministic regression testing.

## 16.2 Real-time Emulation

```text
1 simulated second ≈ 1 wall-clock second
```

This allows testing:

- processing latency,
- Reactor responsiveness,
- queue occupancy,
- deadline behavior,
- packet protocol dynamics.

## 16.3 Fault Simulation

The environment deliberately produces failures such as:

```text
RX overflow
TX underflow
late command
packet loss
sample discontinuity
clock drift
device disconnect
USB timeout
Peripheral failure
TUN/TAP queue overflow
Processor deadline miss
```

The purpose is to verify that the experiment behaves safely under degraded conditions. Injected faults reproduce the hardware's typed event, block flags and timing consequence (an RX overflow is zero samples, a restart gap and a `GAP_BEFORE` flag), and fault schedules live in the BindingProfile environment (§8).

---

# 17. ExecutionClass and simulation fidelity

Simulation must never be presented as equivalent evidence to physical hardware execution.

Every Run should record an execution class such as:

```text
ExecutionClass
├── Simulation
├── RealtimeEmulation
├── HardwareInLoop
└── Hardware
```

Every Run additionally records a **fidelity vector**, one entry per aspect, because "timing modelled, RF not" cannot be said with one label:

```text
Fidelity vector
├── timing        none | envelope | hardware_quirk
├── continuity    none | envelope | hardware_quirk
├── coercion      none | grid
├── rf            none | impairment_model
└── transport     none | model
```

Promotion readiness is judged per aspect.

An AI agent should be able to distinguish:

```text
Simulation passed
```

from:

```text
Hardware timing and UHD transport behavior verified
```

---

# 18. Virtual time belongs in the Core model

Simulation itself is a Module concern, but **time semantics must be Core concepts**.

The Core should define abstract concepts such as:

```text
ClockDomain
TimePoint
ClockRelation
Duration
Deadline
```

A real UHD provider may expose:

```text
ClockDomain = USRP hardware time
```

A simulation provider may expose:

```text
ClockDomain = deterministic virtual time
```

Higher-level components should use the same time abstractions in both cases.

Concretely (Vision §15): `TimePoint` is integer ticks in a `ClockDomain` with a rational tick rate; each stream's SampleClock is an exact rational derivation of its device clock; unrelated domains convert only through a `ClockRelation` with uncertainty. A **Time Authority** owns "now" and "wait until": the device timekeeper in hardware Runs, the **discrete-event Simulation Engine** in Simulation Runs (the same engine, paced to wall clock, in RealtimeEmulation). Reactor timers, Processor deadlines, Peripheral latency models and client waits all wait on the Time Authority; no client API sleeps on wall-clock time for something that happens in runtime time. A sample-rate change is a `cold` update that ends the stream's SampleClock and starts a new ClockDomain; blocks carry the domain id (Vision §15).

---

# 19. Processor and Reactor remain Core concepts, not Core implementations

The Core should define the meaning and contracts of:

```text
Processor
Reactor
Port
Event
Action
Timer
TxBurst
```

But the Core should not define all execution engines.

For example:

```text
Kernel
  └── ComponentDescriptor { kind, ports, params + update classes, timing, impl hash }
      Event / Action vocabulary, two deadline kinds (RelativeBudget, AbsoluteDeadline)

Executors (Modules)
  ├── Host Native Executor      own execution ABI
  ├── WASM Executor             own block ABI
  └── CUDA Executor             own batch / kernel ABI
```

RFNoC is not an Executor: device-side blocks are Radio Model capabilities configured by the UHD Provider (Vision §20). Executors share descriptors, DataContracts and DataLinks, never a call signature. Cycles exist only across Island boundaries through Event/Action edges; `SampleStream` edges never form cycles (Vision §19). The graph structure does not change while a Run is RUNNING; parameters change only through declared update classes (Vision §27).

Thus, execution technology remains replaceable.

---

# 20. Module communication rule

Modules must not call other concrete Modules directly.

Bad:

```text
IEEE80211 Module
      │ direct dependency
      ▼
    UHD Module
```

Good:

```text
IEEE80211 PHY Processor
      │ SampleStream
      ▼
    Radio Port
      │
      ▼
 Core-resolved binding
      │
      ▼
   UHD Provider
```

Similarly:

```text
Beam Reactor
      │ PeripheralCommand
      ▼
Peripheral Port
      │
      ▼
Smart Antenna Provider
```

The rule should be:

> **Modules communicate only through Core-defined Resources, Ports, Events, Actions, Capabilities, and Data contracts.**

This is essential for Mock substitution.

Binding to a sub-resource of another Module's device (for example a USRP GPIO bank used by a smart-antenna plugin) is resolved by the Core through a generic capability and is not a cross-module dependency (§26).

---

# 21. Data types must be generic enough for advanced SDR systems

The runtime should not assume that every edge carries raw IQ. Sample-carrying edges follow the Stream Contract (Vision §23): immutable, reference-counted `SampleBlock`s with per-channel validity, gap flags, burst boundaries (`START_OF_BURST` / `END_OF_BURST`) and a target TimePoint on TX, and `BufferRef` handles tagged with a `MemoryDomain`. A TX stream is a sequence of bursts; `repeat` is a burst attribute, and a time jump inside a burst is a `TX_DISCONTINUITY` event, never zero padding (Vision §23). Ports name entries of an open DataContract registry; the Kernel checks identity and declared compatibility only, and never becomes a type system (Vision §21). Format conversion is explicit: connected ports must name the same DataContract or a declared compatibility, and a `Convert` Processor is placed explicitly; the over-the-wire format is a Provider-internal PerformanceEnvelope dimension (Vision §21, §34).

Port/data categories should eventually include at least:

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
IQ
 ↓
OTFS Processor
 ↓
Delay-Doppler Tensor
```

This is important for:

- MIMO,
- OTFS,
- ISAC,
- packet radios,
- AI processing,
- channel matrices,
- beamforming.

---

# 22. Host I/O is a Module category

Linux TUN/TAP should not be modeled as a Peripheral.

It is a host operating-system I/O boundary.

Conceptually:

```text
Host-I/O Modules
├── Linux TUN
├── Linux TAP
├── UDP
├── PCAP
├── File
├── Shared Memory
└── future AF_XDP / DPDK
```

The Core sees a generic:

```text
NetworkEndpoint
```

or:

```text
HostEndpoint
```

---

# 23. Linux TUN/TAP use case

An IEEE 802.11-like transceiver should be able to look like:

```text
Linux TCP/IP stack
       │
       ▼
      TAP
       │ Packet<EthernetFrame>
       ▼
   MAC Reactor
       │ Packet<MPDU>
       ▼
   PHY Processor
       │ SampleStream<IQ>
       ▼
   Radio Provider
```

The reverse RX path follows the same model.

This allows Linux applications to use the SDR link through ordinary networking tools such as:

```text
ping
iperf
TCP
UDP
IPv4
IPv6
```

without putting Linux networking behavior inside Ez-SDR Core.

---

# 24. Privileged host I/O should be isolated

Linux networking features may require privileges such as `CAP_NET_ADMIN`.

The main Ez-SDR runtime should not receive unnecessary host privileges.

A preferred design is a one-shot helper:

```text
Host-I/O helper (CAP_NET_ADMIN, runs once)
      │ creates a persistent TAP, sets owner/group, brings it up, exits
      ▼
 /dev/net/tun  ◄── Ez-SDR Runtime attaches unprivileged (IFF_MULTI_QUEUE as needed)
```

No packet crosses an IPC boundary through a privileged process. This is consistent with the wider plugin/module isolation philosophy (Vision §41).

---

# 25. Peripheral devices are Modules

Laboratory hardware such as:

- smart antennas,
- RF switches,
- attenuators,
- positioners,
- external amplifiers,
- power meters,
- GPIO devices,
- USB / serial instruments,
- reference clock and time sources (OctoClock, GPSDO, PTP grandmaster) reporting lock state,

should be implemented through Peripheral Providers.

The Core only sees typed:

```text
Peripheral Resource
Capabilities
Commands
Events
Timing Guarantees
Health
```

Vendor APIs and transport details remain outside Core. Timing guarantees are declared per Provider instance and device at `prepare` time, not per Provider kind (Vision §38). Peripheral Plugins run out of process; Radio Providers, Executors, Links and Sinks run in process so that the sample path never crosses an IPC boundary (Vision §62).

---

# 26. USRP GPIO should not create a cross-module dependency

If a smart antenna is controlled through USRP GPIO, the Smart Antenna implementation should not directly depend on the UHD provider.

Prefer:

```text
Smart Antenna
      │
      │ generic GPIO capability
      ▼
  GPIO Resource
      │
      ▼
 UHD GPIO Provider
```

The same logical smart-antenna experiment may then use:

```text
USRP GPIO
server GPIO
USB controller
serial device
```

without changing experiment logic.

The USRP GPIO bank is a sub-resource of the radio device, provided by the same Module instance and sharing its timekeeper (Vision §8, §39).

---

# 27. Data plane implementations are Modules too

The Core must not hard-code one buffer/ring implementation. Contracts exchange `BufferRef` handles tagged with a `MemoryDomain`, never raw host slices (Vision §23).

Future execution may involve:

```text
Host RAM
Pinned Host RAM
Huge Pages
GPU Memory
WASM Linear Memory
RFNoC / FPGA Memory
RDMA Memory
```

The Core should model:

```text
MemoryDomain
DataLink
BufferContract
TransferRequirement
```

Placement and MemoryDomain are stated explicitly in the BindingProfile; the ExecutionPlan validates reachability and records the DataLink chosen from what the bound Modules offer. The Core does not optimise placement (Vision §20).

For example:

```text
Host execution
    → SPSC ring

GPU execution
    → pinned memory / GPUDirect

device-internal (UHD Provider, RFNoC)
    → device-local stream

future distributed node
    → RDMA / network transport
```

---

# 28. Core is a transaction coordinator

The Core should not directly perform all execution.

It coordinates Module plan fragments.

Conceptually:

```text
ExperimentSpec
      ↓
 Core Compiler
      ↓
ExecutionPlan
      │
      ├── RadioPlanFragment
      ├── ProcessingPlanFragment
      ├── HostIOPlanFragment
      ├── PeripheralPlanFragment
      └── DataLinkPlanFragments
```

Then the Core coordinates:

```text
prepare
   ↓
arm
   ↓
start
   ↓
running
   ↓
stop
   ↓
cleanup
```

This allows modules to own high-performance execution while the Core owns consistency. Plan fragments carry dependency edges, and the Core prepares and arms them in DAG order (PPS source first). `prepare` returns a PrepareReport (effective configuration, coercions, warnings) that the Kernel checks against the Spec's coercion policy and records in the Manifest (Vision §11). Every public Kernel type is schema-first and versioned; old Specs are migrated or refused, never silently reinterpreted (Vision §10).

---

# 29. Real-time execution stays outside generic Core logic

A 200 Msps streaming loop should not repeatedly cross a generic Core dispatch layer.

The Core prepares the graph and resources before RUN.

The high-rate path should then execute directly between prepared components.

Conceptually:

```text
Core
  │
  │ prepare / arm
  ▼
──────────────────────────
      Real-time island
──────────────────────────
Radio → DSP → DSP → TX
```

The Core remains responsible for:

- lifecycle,
- policy,
- event collection,
- state transitions,
- failure handling,
- Run provenance.

It should not become the per-sample scheduler.

---

# 30. Proposed logical project structure

A directional structure is:

```text
Ez-SDR/
│
├── core/                      Kernel: frozen at v4.0 (§5)
│   ├── ezsdr-types
│   ├── ezsdr-kernel
│   ├── ezsdr-module-api
│   ├── ezsdr-experiment
│   └── ezsdr-time
│
├── vocab/                     Vocabulary: versioned per crate, additive (§5)
│   ├── radio-model
│   ├── calibration
│   ├── peripheral-model
│   ├── endpoint-model
│   └── contracts
│
├── modules/
│   │
│   ├── radio/
│   │   ├── radio-mock
│   │   ├── radio-uhd
│   │   └── radio-soapy
│   │
│   ├── simulation/
│   │   ├── sim-engine         discrete-event Time Authority (§15)
│   │   ├── sim-channel
│   │   ├── sim-rf-impairments
│   │   └── sim-faults
│   │
│   ├── processing/
│   │   ├── processing-host
│   │   ├── processing-wasm
│   │   ├── processing-cuda
│   │   └── processing-rfnoc
│   │
│   ├── hostio/
│   │   ├── tuntap
│   │   ├── udp
│   │   ├── pcap
│   │   └── file
│   │
│   ├── peripheral/
│   │   ├── plugin-host
│   │   ├── clocksource        OctoClock / GPSDO / PTP lock state as ClockRelation evidence
│   │   └── reference-plugins/
│   │
│   ├── link/                  DataLink implementations (SPSC ring, shared memory, pinned copy; RDMA later)
│   │
│   └── sink/                  Artifact writers
│       ├── raw-iq
│       └── sigmf
│
├── frontends/
│   ├── python
│   ├── cli
│   └── mcp
│
└── native/
    └── uhd-bridge
```

This is a **logical architecture**, not a requirement to immediately create one Rust crate per box.

Too many crates too early would itself create unnecessary complexity.

---

# 31. Minimal implementation path

The architecture allows v4 development to start very small.

A useful first system is:

```text
Kernel
+
Radio Model (with TimingEnvelope)
+
Simulation Engine (discrete-event Time Authority)
+
MockRadio (envelope-enforcing)
+
SimulationChannel
+
Python client (Sessions)
```

No USRP is required.

At this point the following should already work:

```python
with ezsdr.connect() as sdr:
    sdr.tx.repeat(x)
    y = sdr.rx.capture(N)
```

but entirely in software, producing a Session Manifest with the action log, waveform hash and effective configuration (Vision §3). Session Actions are admitted on the control path (envelopes, RF envelope, coercion policy, update class) before dispatch (Vision §3).

This validates:

- Python API,
- resource model,
- ExperimentSpec semantics,
- lifecycle,
- timing model,
- buffers,
- events,
- artifacts,
- failure behavior.

Only after that should:

```text
radio-uhd
```

be added.

---

# 32. Desired result when UHD is added

The ideal milestone is:

> **When the UHD Radio Provider is implemented, existing experiments previously running against MockRadio should run against the physical USRP without changing their ExperimentSpec or high-level Python code.**

This is a major architecture test.

If large changes to application logic are required when moving from Mock to UHD, the abstraction boundary is probably wrong.

---

# 33. AI-agent validation workflow

The architecture should directly support an AI-driven development loop.

```text
AI modifies experiment / Processor / Reactor
        │
        ▼
1. unit tests
        │
        ▼
2. compile component
        │
        ▼
3. deterministic simulation
        │
        ▼
4. fault injection
        │
        ▼
5. real-time emulation
        │
        ▼
6. performance benchmark
        │
        ▼
7. HIL / hardware dry-run
        │
        ▼
8. physical SDR run
```

Ez-SDR therefore becomes more than an SDR control library.

It becomes an **SDR experiment CI/CD substrate suitable for AI agents**.

---

# 34. Experiment validation must work without hardware

An AI agent should be able to validate an experiment using only software-provided capabilities.

For example:

```text
Experiment requires:
  2 RX
  2 TX
  full duplex
  timed TX
  20 Msps

MockRadio provides:
  2 RX
  2 TX
  full duplex
  timed TX
  20 Msps

→ capability validation succeeds
```

The same experiment may later be bound to an X310 and validated against its actual capabilities.

---

# 35. Hardware-specific quirks remain Provider concerns

Physical SDRs have unavoidable device-specific issues:

- stream limits,
- transport limits,
- sample-rate combinations,
- channel-count-dependent throughput,
- LO behavior,
- firmware/FPGA dependencies,
- known UHD quirks,
- transport tuning.

These should remain inside Providers and Provider-specific profiles.

The Core should consume the resulting:

```text
Capability
PerformanceEnvelope
Warning
Constraint
RuntimeEvent
```

without learning UHD-specific implementation details.

---

# 36. Structured observability remains a Core contract

Modules must report health through typed events and metrics.

Examples include:

```text
RX_OVERFLOW
TX_UNDERFLOW
TX_DISCONTINUITY
LATE_COMMAND
ALIGNMENT_ERROR
CLOCK_LOST
DEVICE_DISCONNECTED
PROCESSOR_DEADLINE_MISS
TUN_QUEUE_DROP
PERIPHERAL_TIMEOUT
PLUGIN_FAILURE
```

The Core owns common event semantics.

Modules own how hardware-specific events are translated into that model.

Every `(source, kind)` pair has a never-dropping counter; event bodies go through a bounded queue, and any drop is announced by an `EVENTS_DROPPED` meta-event, so Manifests carry true counts (Vision §29). Failure Policy is a closed table from event kind to continue / mark / stop / abort, not a rules engine (Vision §53). Leases are Attached by default; a Detached lease needs an explicit TTL (Vision §53). There is no metrics framework in the Kernel; counters and sampled events are exported by Sink Modules, and a Probe is a lossy DataLink plus a Recorder Sink rather than a Kernel concept (Vision §29, §30).

---

# 37. Simulation must produce the same event model

Fault simulation should use the same event types as hardware execution.

For example:

```text
MockRadio injected RX overflow
        ↓
RuntimeEvent::RxOverflow
```

and:

```text
UHD reports RX overflow
        ↓
RuntimeEvent::RxOverflow
```

Higher-level experiment logic should not need to care which provider produced the event.

This is critical for realistic AI-agent validation.

---

# 38. Reproducibility across Modules

Every Run should record:

- ExperimentSpec,
- BindingProfile,
- ExecutionClass,
- Module identities and versions,
- Provider configuration,
- software versions,
- selected ExecutionPlan,
- hardware identities where applicable,
- simulation model identities,
- random seed where applicable,
- timing/clock configuration,
- events and metrics,
- artifacts.

Thus a result can be classified as:

```text
deterministic simulation
real-time emulation
HIL
real hardware
```

with complete provenance.

The Manifest is a Kernel envelope plus namespaced Module sections (`uhd.*`, `mock.*`, `peripheral.*`, `calibration.*`) and an optional environment capture; everything hashable is content-addressed (Vision §50). Calibration is applied only in Processors or Providers that take a `CalibrationRef`, calibration procedures are Runs, and the TX→RX delay calibration is part of every device profile (Vision §26).

---

# 39. Core invariants

The following should be treated as strong design invariants.

1. **Core knows no specific SDR hardware/backend implementation.**
2. **Experiment intent and physical implementation binding are separate.**
3. **Hardware and service implementations connect through Modules / Providers.**
4. **Mock Providers are first-class peers of hardware Providers.**
5. **The same ExperimentSpec should run in Simulation and Hardware whenever capabilities match.**
6. **Modules communicate only through Core-defined contracts.**
7. **Core does not own the high-rate per-sample execution loop.**
8. **Core owns lifecycle, time semantics, capabilities, binding, events, policy, and provenance.**
9. **Simulation supports deterministic virtual time and fault injection.**
10. **Every Run records whether it used Simulation, Emulation, HIL, or Hardware.**
11. **Moving from Mock to hardware should primarily change the BindingProfile, not application logic.**
12. **Simulation errors and hardware errors use the same typed Runtime Event model whenever semantically equivalent.**
13. **Large-data transfer and memory placement are delegated to DataPlane/Provider implementations.**
14. **A Module must not directly depend on another concrete Module implementation.**
15. **The Core remains small even as Ez-SDR gains new hardware, accelerators, and experiment types.**
16. **Stability has three tiers: Kernel (frozen), Vocabulary (versioned, additive), Extensions (unstable); "small" applies to the Kernel.**
17. **Time is integer ticks in a named ClockDomain; a Time Authority (device timekeeper or Simulation Engine) owns "now" and "wait until".**
18. **Sample data follows the Stream Contract: immutable reference-counted blocks, per-channel validity, gaps never filled.**
19. **MockRadio enforces the TimingEnvelope of the profile it emulates.**
20. **Coherence is declared by the owning Provider; the environment lives in the BindingProfile; every interaction is a Run with a Manifest.**
21. **Kernel public types are schema-first and versioned; documents are migrated or refused, never silently reinterpreted.**
22. **No structural graph mutation during RUN; parameters change only through declared update classes.**
23. **Placement is explicit, lives in the BindingProfile (never in the Spec), and is validated, never optimized, by the Core; the Kernel owns component descriptors, Executors own execution ABIs.**
24. **Nothing transmits before validate() passes, including the RF safety envelope of the BindingProfile.**

---

# 40. Architectural litmus tests

The architecture should be considered healthy if all of the following can use the same Core without special-case redesign.

## Simple SDR

```python
sdr.tx.repeat(x)
y = sdr.rx.capture(N)
```

## Pure software execution

```text
MockRadio → SimulationChannel → MockRadio
```

with no physical SDR present.

## IEEE 802.11-like link

```text
TAP
 ↓
MAC Reactor
 ↓
PHY
 ↓
Radio
```

running first with MockRadio and later with USRP.

## IBFD

```text
TX
 ↓
simulated / real SI path
 ↓
RX
 ↓
SIC Processor
```

## Smart antenna

```text
RX measurement
 ↓
Beam Reactor
 ↓
PeripheralCommand
 ↓
Mock Smart Antenna / Real Smart Antenna
```

## AI-generated DSP

```text
AI generates Processor
 ↓
software simulation
 ↓
fault tests
 ↓
benchmark
 ↓
USRP hardware
```

If these require separate architecture paths, the Core/module boundary should be reconsidered.

---

# 41. Development philosophy

The project should be developed from the inside out:

```text
small semantic Core
        ↓
Mock Providers
        ↓
deterministic simulation
        ↓
frontends and tests
        ↓
real SDR Provider
        ↓
reactive runtime
        ↓
advanced processing Providers
        ↓
external peripherals
        ↓
distributed / heterogeneous execution
```

This order is deliberate.

It allows most of Ez-SDR to be developed, tested, and reasoned about without physical SDR hardware.

---

# 42. Final perspective

The growing Ez-SDR v4 vision is manageable only if the architecture sharply separates:

> **what an experiment means**

from:

> **how a particular machine executes it.**

The Core should therefore remain a small Experiment Microkernel.

UHD, SoapySDR, MockRadio, simulation channels, WASM, CUDA, RFNoC, TUN/TAP, smart antennas, external laboratory devices, artifact formats, and future distributed execution should all surround that Core as replaceable Modules.

The software-only path is not secondary.

It should be possible for an AI agent to execute almost the entire experiment stack using deterministic simulation, including timing, dataflow, reactive state machines, networking boundaries, failure handling, and observability.

When hardware is introduced, the system should preserve the same experiment model and merely replace software Providers with physical ones.

That property is central to Ez-SDR v4:

> **Design once, validate in software, promote to hardware.**
