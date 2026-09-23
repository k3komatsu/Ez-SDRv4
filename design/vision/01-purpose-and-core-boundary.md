# Ez-SDR v4 — Architecture Vision · Part 01: Purpose and Core Boundary

> Sections §1–§7 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 02: ExperimentSpec, BindingProfile and Compilation](02-spec-binding-and-compilation.md) →

---

# 1. Why v4 exists

Ez-SDR v4 is a clean-sheet rewrite in Rust.

The goal is not merely to port Ez-SDR v3 from D to Rust. v4 exists because the original system was designed primarily around a human writing Python code to operate an SDR, while future use requires:

- high-rate SDR execution,
- deterministic real-time behavior,
- reactive packet-radio experiments,
- multiple SDRs and coherent operation,
- reproducible publication-grade experiments,
- external laboratory equipment,
- heterogeneous execution using CPU / GPU / FPGA / WASM,
- software-only simulation and emulation,
- AI-agent-driven experiment generation and validation.

The v3 codebase is therefore a source of:

- behavioral requirements,
- compatibility expectations,
- migration tests,
- historical lessons,

but it must not constrain the v4 internal architecture.

---

# 2. One-sentence vision

> **Ez-SDR v4 is a typed, deterministic Experiment Runtime for humans and AI agents, built around a small experiment microkernel and replaceable Modules, with software simulation treated as a first-class execution target equal to physical SDR hardware.**

The design principle is:

> **Simple by default, powerful when needed.**

And the development principle is:

> **Design once, validate in software, promote to hardware.**

---

# 3. What Ez-SDR should feel like

The system may become sophisticated internally, but basic use must remain simple.

```python
import ezsdr

with ezsdr.connect() as sdr:
    sdr.tx.repeat(x)
    y = sdr.rx.capture(100_000)
```

A slightly more explicit experiment should remain natural:

```python
with ezsdr.connect() as sdr:
    sdr.rx.frequency = 2.45e9
    sdr.rx.sample_rate = 20e6
    sdr.rx.gain = 20

    sdr.tx.repeat(x)
    y = sdr.rx.capture(1_000_000)
```

Advanced users should be able to descend into:

- explicit devices,
- channels,
- groups,
- clocks,
- calibration,
- leases,
- streams,
- timed operations,
- dynamic bursts,
- Processor/Reactor graphs,
- external peripherals,
- expert backend extensions,

without switching to an unrelated conceptual model.

Python remains a first-class client.

However:

> **The Python object model must not become the internal architecture of Ez-SDR.**

Python, CLI, MCP/AI interfaces, and future frontends must all map to the same typed domain model.

## Sessions: the Easy API is a Run

Interactive use is a long-lived context in which the user changes configuration, starts a repeating TX, captures, changes gain, and captures again. This must not become a second command channel beside ExperimentSpec. The v3 controller protocol was exactly that, and it left no provenance.

> **`connect()` opens a Session. A Session is a Run whose ExperimentSpec is implicit and whose Manifest records an action log.**

`connect()` derives the Session's implicit Spec from the BindingProfile, hashes it like any other, compiles it through the whole pipeline and leaves the Run running. Every Easy API call is one entry in a densely numbered action log with its runtime TimePoint (§15) and its outcome, rejected calls included, and nothing bypasses the log. Before anything is dispatched, each Action passes the same check set in the same order that `validate()` applies to a Spec — the registered admission checks, the RF envelope (§52) among them, then the coercion policy (§11) and the parameter's declared update class (§27) — and a rejected Action is logged and never reaches the real-time path (invariant 10). The Kernel's Session verbs are lifecycle verbs only — `SetParameter`, `Stop` with an optional target, `Release`, `Adopt`, `Renew`, `RunChild` — while domain verbs such as repeating a waveform or capturing are namespaced Vocabulary verbs that compile to Kernel Actions as their Vocabulary declares. The TimingEnvelope and PerformanceEnvelope (§13) are to be checked on this path as well; no Phase 1 rule does so yet, because both are opaque until the Radio Model (Phase 2). A Session changes parameters only through declared update classes and never changes structure. `sdr.run(spec)` creates a child Run with its own profile under the Session's Lease; replaying a Session re-applies its admitted entries and refuses a profile with a different hash. The default Lease is attached, so leaving the `with` block stops TX and releases resources; a detached Lease needs a TTL and is recorded in the Manifest (§53).

Normative: [design/04-run-and-session.md](../04-run-and-session.md), rules RS-4, RS-12…RS-25a; the implicit Spec's derivation in [design/03-spec-and-binding.md](../03-spec-and-binding.md), SB-22c.

`sdr.tx.repeat(x); y = sdr.rx.capture(N)` therefore yields a Manifest containing the waveform hash, the capture's first sample time and validity flags, and the effective RF configuration, with no extra code from the user.

---

# 4. Ez-SDR v4 is a microkernel architecture

The growing v4 vision is only manageable if the Core remains small.

The intended architecture is:

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
                         │ stable Module Contracts
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

The Core coordinates the experiment.

The Modules implement concrete execution.

The Core box has two tiers: the **Kernel** (frozen contracts) and the **Vocabulary** crates (versioned, additive). §5 defines both.

---

# 5. What belongs in Core: three stability tiers

"Core" must not mean two different things at once. Earlier drafts used it both for the experiment microkernel and for the whole shared domain vocabulary (about 45 concepts once §19–§49 are counted). Freezing all of them together paralyses evolution; freezing none of them makes "stable Module Contracts" an empty promise.

The Core is therefore split into **three tiers with three stability policies**:

```text
Tier          Stability                        Contents
────────────  ───────────────────────────────  ─────────────────────────────────────────────────
Kernel        frozen at v4.0, strict semver    lifecycle, ownership, time primitives, data-plane
                                               primitives, event/action vocabulary, generic
                                               matching, envelopes of Spec/Binding/Plan/Manifest,
                                               Module API
Vocabulary    versioned per crate,             Radio Model, Calibration, Peripheral model,
              additive changes by default      Endpoint model, standard DataContracts
Extensions    unstable, namespaced             backend-specific power (uhd.*, mock.*, ...)
```

## Kernel (frozen)

```text
Run / Session lifecycle and transaction   prepare → arm → start → stop → cleanup, dependency DAG
Lease
ClockDomain / TimePoint / Duration /      integer ticks at a rational rate (§15, §23)
Deadline / ClockRelation / TimeAuthority
SampleBlock / BufferRef / MemoryDomain    the Stream Contract (§23)
DataContract registry / Port / DataLink   identity and compatibility only
Event envelope / counters / Action set    TxBurst, SetTimer, UpdateParameter, PeripheralCommand,
                                          Emit, Stop, Abort; Policy
Capability / Constraint matching          generic, schema-driven; keys are defined by Vocabulary
Composite Resource tree                   Device → sub-resources (§8)
ExperimentSpec / BindingProfile /         versioned envelopes; contents are namespaced
ExecutionPlan / Manifest envelopes
Module API                                roles, descriptors, registry
ExecutionClass + fidelity vector          (§14)
```

## Vocabulary (versioned separately)

```text
Radio Model          channels, streams, RF parameters, TimingEnvelope, PerformanceEnvelope,
                     coherence basis, repeat / decimation capabilities
Calibration          CalibrationArtifact kinds and validity conditions
Peripheral model     capabilities, timing classes, triggers
Endpoint model       NetworkEndpoint / HostEndpoint
Standard contracts   stream.*, pdu.*, tensor.*, event.*, control
```

A Module depends on the Kernel and on specific versions of the Vocabulary crates it needs. Vocabulary crates evolve additively; a breaking Vocabulary change is a new major version of that crate, not of the Kernel.

## What the Kernel owns

- experiment intent and its compiled plan,
- resource requirements and generic capability matching,
- resource binding and the composite resource tree,
- lifecycle, ownership, and transactional cleanup,
- clock/time semantics and the Time Authority,
- the Stream Contract and buffer ownership,
- structured event and action semantics, and the failure policy,
- provenance, Run/Session identity, and reproducibility.

The Kernel does not own per-sample execution, radio-specific capability keys, Processor execution ABIs, placement optimisation, or calibration algorithms.

> **The invariant "Core remains small" applies to the Kernel.** A new experiment type may add a Vocabulary crate; it must not add Kernel concepts.

The word *Kernel* is used only for this tier. The discrete-event simulator that implements the Time Authority in software is the **Simulation Engine** (§15), never a kernel.

---

# 6. What must not belong in Core

The following are implementation details and should remain outside the Core:

```text
UHD API
USRP model-specific code
SoapySDR API
HackRF-specific behavior
RFNoC graph construction
Replay DRAM management
Wasmtime
CUDA
Linux /dev/net/tun
AF_XDP
DPDK
USB / serial vendor protocols
smart-antenna SDKs
IEEE 802.11 algorithms
OTFS equalizer implementations
IBFD canceller implementations
Processor execution ABIs
automatic placement optimisation
expression or template languages inside ExperimentSpec
a rules engine for failure Policy
RFNoC graph construction for user processing
a metrics framework
a Probe or Taint type
sensor-data ingestion (video, motion capture)
```

Ideally, many of these terms should not appear in Core crates at all.

---

# 7. Model + Provider is the default extension pattern

Ez-SDR should consistently separate:

> **what a resource means**

from:

> **how that resource is implemented**

Examples:

```text
Radio Model
    ├── Mock Radio Provider
    ├── UHD Radio Provider
    └── Soapy Radio Provider
```

```text
NetworkEndpoint Model
    ├── Linux TUN Provider
    ├── Linux TAP Provider
    ├── UDP Provider
    ├── PCAP Provider
    └── future AF_XDP Provider
```

```text
Peripheral Model
    ├── Smart Antenna Provider
    ├── RF Switch Provider
    ├── USB Attenuator Provider
    └── Mock Peripheral Provider
```

Module, role and deployment are three orthogonal axes. There are exactly five roles — Provider (a Resource Model: Radio, Peripheral, Endpoint, Simulation), Executor (a processing engine), Sink (Artifacts), Link (a DataLink) and Authority (the Time Authority, §15) — with one trait each and no shared lifecycle supertrait. A Module declares its roles in its ModuleDescriptor and may hold several: the UHD Module is a Radio Provider, a GPIO Provider and an Authority (its timekeeper); the `sim-engine` Module is an Authority. Deployment is in-process (compiled in) or Plugin; "Plugin" names a deployment form, not a role. Peripheral Modules are Plugins by default because vendor SDKs run outside the Core process (§62); the Plugin variant is fixed in the schemas now and refused at registration until a Plugin host exists.

Normative: [design/05-module-api.md](../05-module-api.md), rules MA-1…MA-3, MA-32, MA-46.

## Module communication rule

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

> **Modules communicate only through Kernel-defined Resources, Ports, Events, Actions, Capabilities and DataContracts.**

This is essential for Mock substitution. Binding to a sub-resource of another Module's device (a USRP GPIO bank used by a smart-antenna Plugin) is resolved by the Core through a generic capability and is not a cross-module dependency (§39).

---

[Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 02: ExperimentSpec, BindingProfile and Compilation](02-spec-binding-and-compilation.md) →
