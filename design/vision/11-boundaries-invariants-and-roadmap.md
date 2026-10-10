# Ez-SDR v4 — Architecture Vision · Part 11: Boundaries, Invariants and Roadmap

> Sections §62–§68 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 10: Implementation Path and Testing](10-implementation-path-and-testing.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md)

---

# 62. Security and trust boundaries

The architecture should distinguish:

```text
trusted Core
hardware Providers
third-party Peripheral Plugins
user Python
native processing components
sandboxed WASM
AI-generated components
privileged Host-I/O helpers
```

Third-party vendor SDKs should normally run outside the Core process.

TUN/TAP privilege should be isolated.

The RF safety envelope of the BindingProfile (§52) is a trust boundary too: it is what stands between an AI-generated Spec and the antenna.

## Process boundaries in v4.0

```text
in-process (Rust traits)         Radio Providers (UHD, Soapy, Mock), Executors (native, WASM, GPU),
                                 Links, Sinks: the sample path never crosses an IPC boundary
out-of-process (typed protocol)  Peripheral Plugins (vendor SDKs), the privileged host-I/O helper (§41)
out-of-process (typed protocol)  clients: Python, and later CLI and MCP, through ezsdr-server
                                 (design/16-easy-api.md); a client's waveforms and captures cross
                                 as bytes, and the sample path between Modules stays in process
```

Rust has no stable ABI, so a dynamically loaded in-process Module is either compiled with the Runtime or exposed through a C-ABI layer; the default is compiled-in. A DataLink crosses a process boundary only through an implementation that can do so without copying, such as shared memory or DMA. Because buffers are handles (§23), a remote Radio Provider that speaks the same contract over shared memory can be added later without changing the contract (§35).

WASM may become the preferred trust boundary for dynamically generated processing logic.

Full authentication/authorization can be staged later, but the architecture must not require a redesign when added.

---

# 63. Explicit non-goals

Ez-SDR v4 is not intended to become:

- a complete programming language,
- a generic workflow engine,
- a new operating system,
- a Kubernetes-like cluster manager,
- a generic distributed runtime,
- a replacement for Python,
- a replacement for Rust/C++ DSP,
- a universal FPGA language,
- a serialization of every UHD API,
- a full replacement for GNU Radio,
- an automatic placement optimiser, graph fuser or transfer planner,
- a runtime that mutates graph structure while running,
- a rules engine for failure policy,
- a template or expression language inside ExperimentSpec,
- a way to place arbitrary user Processors on RFNoC/FPGA,
- a Probe concept in the Kernel (a Probe is a lossy DataLink plus a Recorder Sink, §30),
- a `Taint` type in the Kernel (flag propagation is an Executor default and a Processor convention, §28),
- a metrics framework in the Kernel (counters and delivered events, exported by Sinks, §29),
- a sensor-data platform (external sensor data is referenced, never ingested, §47),
- a "Distributed" Module category or a ComputeNode scheduler (§49, §60),
- a Core that assembles a CoherentGroup across Providers (§25).

Ez-SDR should not compete with GNU Radio by rebuilding its entire DSP block ecosystem.

Its unique focus is:

> **experiment runtime, reproducibility, resource binding, hardware/software promotion, reactive SDR execution, external-lab integration, and AI-agent validation.**

Where useful, external DSP systems should be integrable as Modules or Execution Islands rather than reimplemented.

---

# 64. Features allowed to remain future work

The architecture should permit these, but the first implementation does not need to deliver them:

```text
RT-WASM
GPU processing
RFNoC-backed radio capabilities (Replay repeat, DDC/DUC rates, device FFT)
full IEEE 802.11 PHY/MAC
GraphEpoch atomic reconfiguration
AF_XDP / DPDK
RDMA
QUIC data plane
distributed multi-host execution
massive-MIMO scale-out
full authentication/authorization
runtime-downloadable arbitrary third-party backend ABI
out-of-process (remote) Radio Providers over shared memory
automatic insertion of format-conversion components
richer RF impairment models (hardware-quirk fidelity)
```

The important requirement is:

> **v4.0 must not make them require replacing the Core model.**

---

# 65. Strong design invariants

The following should be treated as architecture invariants unless strong evidence justifies changing them.

1. **Rust is the primary implementation language.**
2. **Python remains a first-class user interface.**
3. **The typed domain model is the source of truth.**
4. **The Kernel remains a small Experiment Microkernel; Vocabulary crates version separately and change additively.**
5. **Core does not know concrete SDR/backend implementations.**
6. **Experiment intent and implementation binding are separate.**
7. **Mock Providers are first-class peers of hardware Providers.**
8. **The same ExperimentSpec should run in Simulation and Hardware whenever capabilities and envelopes match.**
9. **ExperimentSpec is compiled before RUN.**
10. **The real-time path never parses ExperimentSpec/JSON.**
11. **Core does not own the high-rate per-sample execution loop.**
12. **Modules communicate through Core-defined contracts, not direct concrete dependencies.**
13. **Fixed schedules and reactive execution are both first-class.**
14. **Dynamic timed TxBurst is first-class.**
15. **Time and ClockDomain are first-class.**
16. **Coherence and calibration are distinct from basic device grouping.**
17. **Data ports are not limited to IQ SampleStreams.**
18. **Memory placement and transfer cost are explicit planning concerns.**
19. **Loss, continuity, backpressure, deadlines, and fallback are explicit semantics.**
20. **Backend/plugin health is typed RuntimeEvent and counter data, never only log text.**
21. **Event reporting never blocks the real-time sample path.**
22. **External lab devices are resources implemented through Providers/Plugins.**
23. **Host I/O such as TUN/TAP is separate from Peripheral hardware.**
24. **AI agents remain outside hard real-time loops.**
25. **Simulation supports deterministic virtual time and fault injection.**
26. **Every Run records whether evidence came from Simulation, Emulation, HIL, or Hardware.**
27. **Runs and Artifacts are reproducible and inspectable.**
28. **Backend-specific power remains reachable through explicit namespaced extensions.**
29. **Single-host assumptions must not be unnecessarily burned into public semantics.**
30. **Adding future Modules should not cause Core growth proportional to feature count.**
31. **Stability has three tiers: Kernel (frozen), Vocabulary (versioned, additive), Extensions (unstable).**
32. **Every timestamp names its ClockDomain; sample time is integer ticks at a rational rate, never floating-point seconds.**
33. **Stream semantics are normative: gaps are represented by flags and time jumps and are never silently filled; validity is per channel.**
34. **Buffers are immutable-after-publish, reference-counted handles tagged with a MemoryDomain; no Module contract bakes in host-memory slices.**
35. **A Mock that accepts what its emulated hardware would reject is a bug: Mock implements the declared constraint envelope, not the ideal.**
36. **Coherence is declared by the Provider that owns the channels; the Core never infers coherence across Providers.**
37. **Every interaction with the runtime, including the Easy API, is a Run (a Session) with a Manifest and an action log.**
38. **The ExperimentSpec never describes the environment; channel models, fault schedules and site constraints live in the BindingProfile.**
39. **Kernel public types are schema-first with language-neutral serialization and explicit versions; old documents are migrated or refused, never silently reinterpreted.**
40. **No structural graph mutation during RUN; parameters change only through declared update classes.**
41. **Placement is explicit, lives in the BindingProfile (never in the ExperimentSpec), and is validated, never optimized, by the Core.**
42. **Nothing transmits before validate() passes, including the RF safety envelope of the BindingProfile; every Session Action passes the same admission before it is dispatched.**

---

# 66. Architecture success criteria

The architecture is successful if the same Core naturally supports all of the following.

## Simple human experiment

```python
with ezsdr.connect() as sdr:
    sdr.tx.repeat(x)
    y = sdr.rx.capture(100_000)
```

executed as a Session Run with an action log and a Manifest (§3).

## Pure software experiment

```text
MockRadio A
   ↓
SimulationChannel
   ↓
MockRadio B
```

with no physical SDR.

## Reactive packet radio

```text
continuous RX
→ packet detect
→ decode
→ Reactor
→ dynamic TxBurst
→ timed TX
```

## Linux network link

```text
Linux TAP
→ MAC Reactor
→ PHY
→ Radio
```

## Coherent MIMO

```text
CoherentGroup
→ aligned samples
→ calibration
→ MIMO Processor
```

## IBFD

```text
simultaneous TX/RX
→ SI path
→ adaptive SIC
→ optional analog Peripheral control
```

## ISAC

```text
radio array
+
sensing Processor
+
smart antenna
+
positioner
+
shared Run timing/provenance
```

## Smart antenna

```text
RX measurement
 ↓
Beam Reactor
 ↓
Command
 ↓
Mock Smart Antenna / Real Smart Antenna
```

## OTFS/GPU

```text
IQ
→ GPU Tensor processing
→ DD detector
```

## AI-generated component

```text
AI writes component
→ software simulation
→ fault test
→ benchmark
→ HIL
→ real hardware
```

## Degraded execution

```text
UHD overflow
→ typed event
→ continuity gap
→ policy
→ metrics
→ AI/Python visibility
→ Run provenance
```

If any of these require an unrelated special-purpose API or a second architecture, the Core/module boundary is probably wrong.

---

# 67. Development sequencing

A recommended direction is:

```text
Phase 0
Architecture Audit

Phase 1
Kernel semantic model
(tiers, time representation, Stream Contract, Sessions, composite resources)

Phase 2
Radio Model (with TimingEnvelope) + Simulation Engine (virtual time) + MockRadio (envelope-enforcing)

Phase 3
SimulationChannel + deterministic Runs

Phase 4
Events / failure / continuity / artifacts

Phase 5
Mini Reactive Radio
PING → Reactor → timed PONG

Phase 6
Python Easy API

Phase 7
Native UHD Provider

Phase 8
Mock → X310 parity test

Phase 8b
Multi-device: several USRPs on one 10 MHz + PPS reference, time set at one PPS edge, aligned start

Phase 9
Packet/PDU + TUN/TAP

Phase 10
Processor/Reactor execution engines

Phase 11+
WASM / GPU / RFNoC-backed radio capabilities / advanced peripherals / distributed execution
```

The exact version numbers are not fixed.

Phase 8b follows the parity test because aligning several devices builds on one device's measured timing, and it precedes Phase 9 because it is v3's working multi-device behaviour (§61, behaviour 4), not new function. It brings the Authority that sets every device's time at one PPS edge, the relations of a second device's root (§15), and the coherence a Provider declares (§25).

The architectural order is more important than the release numbering.

---

# 68. Final perspective

Ez-SDR v4 should not merely be a faster rewrite of v3.

Its long-term role is at the intersection of:

- easy remote SDR access,
- deterministic experiment execution,
- reactive real-time radio systems,
- reproducible research infrastructure,
- software simulation/emulation,
- heterogeneous compute,
- external laboratory orchestration,
- structured observability,
- AI-agent experimentation.

The only way to keep that vision implementable is to keep the Core very small.

The defining architectural property is:

```text
Experiment semantics
        ↓
small stable Core
        ↓
replaceable Modules
        ↓
Simulation or Hardware
```

A future contributor or AI agent should be able to add:

```text
new SDR
new accelerator
new network endpoint
new peripheral
new simulation model
```

without redesigning the Core.

And the same experiment should ideally progress through:

```text
deterministic software simulation
        ↓
real-time emulation
        ↓
hardware-in-the-loop
        ↓
real physical SDR
```

without rewriting the experiment itself.

That is the central promise of Ez-SDR v4:

> **Design once, validate in software, promote to hardware.**

---

← [Part 10: Implementation Path and Testing](10-implementation-path-and-testing.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md)
