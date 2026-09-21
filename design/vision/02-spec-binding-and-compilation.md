# Ez-SDR v4 — Architecture Vision · Part 02: ExperimentSpec, BindingProfile and Compilation

> Sections §8–§11 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 01: Purpose and Core Boundary](01-purpose-and-core-boundary.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 03: Simulation, Mock and Time](03-simulation-mock-and-time.md) →

---

# 8. Experiment intent must be separated from implementation binding

An ExperimentSpec should normally state what the experiment requires.

It should not normally hard-code how that requirement is implemented.

Example:

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

The keys under `requires` are defined by the Radio Model (Vocabulary), not by the Kernel. The Kernel matcher is generic: it compares constraints with declared capabilities and asks the Provider whether a value can be coerced.

## Resources are composite

A physical or simulated radio is not a flat bag of channels. In UHD, coherence, sample alignment, timed commands, GPIO timing and Replay DRAM are all properties of **one device handle**; a `multi_usrp` may span several motherboards. The resource model must reflect that:

```text
Provider instance
└── Device (or DeviceSet)
    ├── channels            RX / TX, RF parameters
    ├── streams             N aligned channels per stream
    ├── timekeeper          a ClockDomain (§15)
    ├── gpio banks          may share the timekeeper (§39)
    ├── replay / DRAM       backs the repeat capability
    └── sensors             temperature, lock status, ...
```

Rules:

- The ExperimentSpec requests capabilities at the device level and **per direction**: `rx: { channels: 4, coherent: true }, tx: { channels: 1 }`. A sensing array needs four coherent RX channels and one TX channel; a channel sounder may need RX only; coherence may be required for one direction and not the other. A single `channels` count cannot say any of this.
- A request for coherent channels must map to **one Provider instance that can declare that coherence**, for example the UHD Provider with `addr0,addr1`. If no single instance can, `validate()` fails. The Core never assembles coherence from independent instances (§25).
- Sub-resources are bound through the Provider instance that owns them. A peripheral that needs a GPIO line binds to a generic GPIO capability, which the Core resolves to a sub-resource of the radio device (§39).
- ExecutionPlan fragments carry dependency edges. The Core prepares and arms in DAG order: the device that sources PPS before the devices that consume it, which was a v3 start-up failure mode.

## BindingProfile = bindings + placements + environment

A BindingProfile has three parts: `bindings` (which Provider instance satisfies each resource), `placements` (which Executor and MemoryDomain run each processing component of the Spec's graph, §20), and `environment` (what surrounds the experiment). The ExperimentSpec contains none of them: it states requirements, and a Spec that named an Executor could not be promoted to a host without that Executor.

```yaml
# laboratory
bindings:
  radio:
    provider: ezsdr.radio.uhd
    selector:
      addrs: ["192.168.40.2", "192.168.40.3"]   # one instance, two motherboards
environment:
  clock_distribution: octoclock          # 10 MHz + PPS to both
  rf_path: cabled                        # or: ota
  rf_envelope:                           # enforced by validate()/prepare(), §52
    allowed_bands: [{ lo_hz: 2.400e9, hi_hz: 2.4835e9 }]
    max_gain_db: 20
    tx_enabled: [true, true]
```

```yaml
# simulation
bindings:
  radio:
    provider: ezsdr.radio.mock
    profile: x310-like
    instances: 2
environment:
  time: { class: simulation }            # discrete-event virtual time (§15)
  channel:                               # SimulationChannel (§16)
    model: awgn
    snr_db: 10
    delay_samples: 37
  faults:                                # FaultInjector schedule (§17)
    - at: "2.5s"
      inject: rx_overflow
      target: radio[0].rx
```

> **The ExperimentSpec never describes the environment.** Channel models, fault schedules, virtual-time settings, clock distribution and site constraints live in `environment`. A channel emulator or attenuator that the *experiment itself controls* is a Peripheral resource in the Spec; a channel that merely *exists* around the experiment is environment.

Thus:

```text
                  ExperimentSpec   (no environment inside)
                        │
                ┌───────┴────────┐
                │                │
         BindingProfile     BindingProfile
           simulation        laboratory
                │                │
                ▼                ▼
           Mock Radio         UHD Radio
       bindings + env      bindings + env
```

The Manifest records the environment part of the BindingProfile verbatim.

Backend-specific requirements remain possible through explicit namespaced extensions.

---

# 9. ExperimentSpec is declarative intent, not a programming language

Ez-SDR should support a typed ExperimentSpec.

Its purpose is:

- validation before execution,
- reproducibility,
- AI-agent operation,
- remote execution,
- Mock-to-hardware promotion,
- transactional cleanup,
- experiment composition,
- dry-run and plan inspection.

A conceptual shape is:

```text
ExperimentSpec
├── version          mandatory; migrated or refused, never reinterpreted (§10)
├── requirements
├── resources        per-direction radio requests, peripherals, endpoints (§8)
├── graph            components, links and their requirements; never a placement (§20)
├── schedule         Actions with AbsoluteDeadlines (§19, §22)
├── outputs          Artifacts to produce, including CalibrationArtifacts (§26)
├── policies         the closed Policy table (§53) and the coercion policy (§11)
└── extensions       namespaced backend-specific requirements
```

ExperimentSpec must not become a generic programming language.

Do not add arbitrary:

```text
if
while
variables
general expressions
embedded Python
general function definitions
```

Complex adaptive algorithms belong in:

- Python orchestration,
- Processor implementations,
- Reactor implementations,
- Rust/C++,
- WASM,
- GPU/FPGA code.

## Parametrisation lives in the builder, not in the Spec

The pressure to add expressions is real: v3 grew `CONSTANTS` and `!COMPUTE(...)` in its configuration JSON because users wanted parameter sweeps without duplicating files. The answer in v4 is a **Spec builder on the Python side**, `ezsdr.spec.build(...)`, whose output is a fully expanded ExperimentSpec. The Manifest records the hash of the generating code next to the Spec hash. The Spec itself has no variables, constants, templates or expressions, and never will.

---

# 10. ExperimentSpec is compiled before RUN

The real-time path must never parse or interpret ExperimentSpec.

Conceptually:

```text
ExperimentSpec + BindingProfile
      │
      ▼
Schema validation            (version checked: migrate or refuse, never reinterpret)
      │
      ▼
Semantic validation
      │
      ▼
Resource resolution          (composite resource tree, §8)
      │
      ▼
Binding resolution           (explicit: bindings and placements come from the BindingProfile)
      │
      ▼
Capability matching          (against the bound instances; the Provider answers "coercible?")
      │
      ▼
Plan construction            (fragments + DataLinks + dependency DAG)
      │
      ▼
Admission checks             (contract compatibility, MemoryDomain reachability,
      │                       envelope constraints, deadline feasibility, RF envelope)
      ▼
ExecutionPlan
      │
      ▼
Prepare  → PrepareReport     (effective configuration, coercions, warnings; §11)
      │
      ▼
Arm
──────────────────────────────────
          real-time boundary
──────────────────────────────────
      │
      ▼
Run
```

Three principles follow:

> **ExperimentSpec expresses intent, not implementation.**

> **ExecutionPlan expresses implementation, not intent.**

> **The real-time path never interprets ExperimentSpec.**

## The compiler is a validator, not an optimiser

Placement, MemoryDomains and DataLink choices are stated explicitly in the BindingProfile's `placements` section. The ExperimentSpec states only requirements (`requires: { executor_kind: gpu | any, budget, memory }`); it never names an Executor or a MemoryDomain, because a Spec that did could not be promoted to a host without that Executor (invariants 6, 8, 29). The Core checks that the stated arrangement is feasible and rejects what is not. It never searches for a better arrangement. GNU Radio 4 places components in explicit port domains with explicit conversion blocks, and NVIDIA Aerial's GPU pipeline is laid out by hand; neither runs an optimiser, and Ez-SDR will not either until a concrete experiment proves one necessary (§63).

## Schema-first and versioned

Every public Kernel type — ExperimentSpec, BindingProfile, Manifest, Event, Action, TxBurst, PrepareReport — has a **language-neutral schema with a version**. The same definition serves the Rust runtime, the Python client, out-of-process Plugins and WASM components; nothing is defined only as a Rust type and re-described by hand elsewhere (v3 had one hand-written binary protocol per controller).

ExperimentSpec, BindingProfile and Manifest carry a mandatory `version`. The Kernel either **migrates** a document from the previous major version or **refuses** it with a message naming the version. It never silently interprets an old document with new defaults. v3 accumulated three configuration formats and a chain of converters; v4 states the policy before the first schema exists.

The concrete serialisation technology is not fixed by this Vision.

---

# 11. Core is a transaction coordinator, not the sample scheduler

The Core composes Module plan fragments.

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

The Core coordinates:

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

Plan fragments carry dependency edges. The Core executes prepare and arm in dependency order (for example, the device that sources PPS before the devices that consume it) and fails the transaction as a whole if any fragment fails.

`prepare` returns a **PrepareReport** for every fragment:

```text
PrepareReport
├── effective        the configuration actually applied
├── coercions        [{ key, requested, applied, reason }]
├── warnings
└── constraints_hit
```

The Kernel applies the coercion policy — `accept`, `warn` or `reject`, settable per key in the Spec's `policies` — to that report, records it in the Manifest, and exposes it as `run.effective()` to Python and Reactors. Defaults differ by Run kind: a Session warns; a Spec Run rejects coercions of sample rate and frequency and warns on gain, because a publication Run must not silently change its waveform timing. A Spec that asks for 19.5 Msps on a device whose grid gives 20 Msps sees the coercion before RUN, not in a plot afterwards. MockRadio produces the same report from the same coercion rules as the profile it emulates (§13).

It should not repeatedly dispatch every sample block through a generic Core layer.

High-rate execution must occur directly between prepared components.

---

← [Part 01: Purpose and Core Boundary](01-purpose-and-core-boundary.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 03: Simulation, Mock and Time](03-simulation-mock-and-time.md) →
