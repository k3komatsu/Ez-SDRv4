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
  "version": 1,
  "requirements": { "vocabularies": [{ "id": "radio", "major": 1 }] },
  "resources": {
    "radio": {
      "kind": "radio.device",
      "requires": {
        "radio.rx.channels":    { "kind": "min", "value": 2 },
        "radio.rx.coherent":    { "kind": "eq",  "value": true },
        "radio.tx.channels":    { "kind": "min", "value": 2 },
        "radio.full_duplex":    { "kind": "eq",  "value": true },
        "radio.rx.sample_rate_hz": { "kind": "eq",  "value": 20000000 },
        "radio.tx.sample_rate_hz": { "kind": "eq",  "value": 20000000 },
        "radio.hardware_time":  { "kind": "eq",  "value": true }
      }
    }
  }
}
```

The keys under `requires` are defined by the Radio Model (Vocabulary), not by the Kernel; the `radio.*` names above stand in for the ones the Radio Model will define. The Kernel matcher is generic: it compares constraints with declared capabilities and asks the Provider whether a value can be coerced.

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

A Spec states requirements per resource under Vocabulary-owned keys, one key per direction (`radio.rx.channels`, `radio.tx.channels`, `radio.rx.coherent`): a sensing array needs four coherent RX channels and one TX channel, and a single `channels` count cannot say so. The generic matcher compares them with the declared capabilities of the node the binding chose, asking the Provider's `coerce` only whether a value can be coerced. A Spec resource binds to exactly one Provider instance, and a node to at most one resource unless its Provider declares it shareable, so coherence is declared by the instance that owns the channels and never assembled across instances (§25). A resource's `needs` are resolved by the Kernel to a sub-resource of a bound instance, possibly another one, which is how a peripheral reaches a radio's GPIO bank with no Module dependency (§39). Fragments are armed in dependency order from each instance's `arm_after` and the `ezsdr.arm_order` section, so the device that sources PPS is armed first — a v3 start-up failure mode.

Normative: [design/03-spec-and-binding.md](../03-spec-and-binding.md), rules SB-2, SB-6…SB-8, SB-12, SB-33…SB-36, SB-39.

## BindingProfile = bindings + placements + environment

A BindingProfile is the closed set `version`, `bindings`, `authority`, `placements`, `environment`. One `bindings` map fills every role slot — a resource's Provider, an output's Sink, an Island's Executor — with an exact Module `{ id, version }`; `authority` names the Time Authority and is mandatory; `placements` assigns components, memory domains and Link Modules (§20); and `environment` holds namespaced sections, the only place for a channel model, a fault schedule, a time class, a clock distribution or a site limit. The ExperimentSpec contains none of them: it states requirements, and a Spec that named an Executor could not be promoted to a host without that Executor.

Normative: [design/03-spec-and-binding.md](../03-spec-and-binding.md), rules SB-13, SB-21…SB-27.

```yaml
# laboratory
version: 1
bindings:
  radio:
    module: { id: ezsdr.radio.uhd, version: { major: 1, minor: 0, patch: 0 } }
    selector:
      addrs: ["192.168.40.2", "192.168.40.3"]   # one instance, two motherboards
authority: radio                         # the device timekeeper, riding on the radio binding
environment:
  ezsdr.rf_path: { path: cabled }        # or: over_the_air
  radio.clock_distribution: octoclock    # 10 MHz + PPS to both
  radio.rf_envelope:                     # enforced by validate()/prepare(), §52
    allowed_bands: [{ lo_hz: 2.400e9, hi_hz: 2.4835e9 }]
    max_gain_db: 20
    tx_enabled: [true, true]
```

```yaml
# simulation
version: 1
bindings:
  radio:
    module: { id: ezsdr.radio.mock, version: { major: 1, minor: 0, patch: 0 } }
    profile: { name: x310-like, version: { major: 1, minor: 0, patch: 0 } }
    selector: { instances: 2 }           # selector content: one Mock instance emulating two motherboards
  sim:
    module: { id: ezsdr.sim-engine, version: { major: 1, minor: 0, patch: 0 } }
authority: sim                           # the Simulation Engine is the Time Authority (§15)
environment:
  ezsdr.time: { class: simulation }      # discrete-event virtual time (§15)
  ezsdr.rf_path: { path: simulated }
  sim.channel:                           # SimulationChannel: a coupling matrix (§16)
    couplings:                           # one path per TX channel → RX channel, explicit ends
      - { tx: radio, tx_channel: 0, rx: radio, rx_channel: 1, gain_db: -30, delay_ns: 37 }
    noise_dbfs: { radio: -60 }           # receiver noise power per receiving fragment, not an SNR
  sim.faults:                            # fault schedule read by its target Provider (§17)
    - at_ns: 2500000000                  # nanoseconds after T0
      fault: rx_overflow
      target: radio                     # target fragment id
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
├── requirements     the Vocabulary majors the Spec is written against; constraints live in each resource's `requires`
├── resources        per-direction radio requests, peripherals, endpoints (§8)
├── inputs           artifacts the Run consumes that no schedule entry carries, such as a Reactor's waveform or a calibration artifact (§26)
├── graph            components, links and their requirements; never a placement (§20)
├── schedule         Action templates at a resource-relative time, resolved to AbsoluteDeadlines at arm (§19, §22)
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

Compilation runs in a fixed order: schema validation, semantic validation, resource resolution, binding resolution, capability matching against the bound instances, plan construction, admission checks, `prepare`, `arm`. Matching follows binding, because the capabilities matched are those of the instance the binding chose. `validate()` reports what it finds; `plan()` runs the same structural and endpoint checks again through the same functions rather than presuming `validate()` ran, and refuses anything not admitted. `prepare` returns a PrepareReport per fragment, and `arm` fixes what only it can: each transmit SampleClock's origin and each scheduled time. Everything after `arm` is the real-time side, which receives an ExecutionPlan and never a Spec.

Normative: [design/03-spec-and-binding.md](../03-spec-and-binding.md), rules SB-37…SB-43 (which stage checks what: table SB-T4).

Three principles follow:

> **ExperimentSpec expresses intent, not implementation.**

> **ExecutionPlan expresses implementation, not intent.**

> **The real-time path never interprets ExperimentSpec.**

## The compiler is a validator, not an optimiser

Placement, MemoryDomains and DataLink choices are stated explicitly in the BindingProfile's `placements` section. The ExperimentSpec states only requirements (`requires: { executor_kind, memory_bytes }`, with the budget in `timing`); it never names an Executor or a MemoryDomain, because a Spec that did could not be promoted to a host without that Executor (invariants 6, 8, 29). The Core checks that the stated arrangement is feasible and rejects what is not. It never searches for a better arrangement. GNU Radio 4 places components in explicit port domains with explicit conversion blocks, and NVIDIA Aerial's GPU pipeline is laid out by hand; neither runs an optimiser, and Ez-SDR will not either until a concrete experiment proves one necessary (§63).

## Schema-first and versioned

Every Kernel document type has a JSON Schema generated from its Rust definition and committed; the committed file, not the Rust source, is the contract, and a freeze test fails on any drift. The same schema serves the Rust runtime, the Python client, out-of-process Plugins and WASM components (v3 had one hand-written binary protocol per controller). ExperimentSpec, BindingProfile and Manifest carry a mandatory integer `version`: an unsupported one is refused with a message naming the supported versions and never read under newer defaults, a migration is a registered function from one major to the next, and the Manifest records the original version and hash. v3 accumulated three configuration formats and a chain of converters; v4 states the policy before the first schema exists.

Normative: [design/03-spec-and-binding.md](../03-spec-and-binding.md), rules SB-9, SB-10, SB-21, SB-47…SB-49; schema technology in [plan/phase1/00-overview.md](../../plan/phase1/00-overview.md) X2 and OV-10…OV-17.

---

# 11. Core is a transaction coordinator, not the sample scheduler

The Core composes Module plan fragments.

The ExecutionPlan is a set of fragments, one per role slot: a Provider fragment per bound resource, one per Island, one per bound output's Sink, and the Authority's when it stands alone; the Kernel builds a Provider fragment's content from the selector and the matched request and reads no other content, so no fragment type names a resource model. Each fragment carries its dependency edges. The Core drives `prepare → arm → start → running → stop → cleanup` as one transaction: `prepare` and `arm` run in dependency order (the device that sources PPS before the devices that consume it), any fragment's failure fails the whole Run, and cleanup runs in reverse order. `prepare` returns one PrepareReport per fragment, holding that fragment's effective configuration, coercions and warnings. `run.effective()` exposes their effective configurations, per fragment, to Python and Reactors, and the Manifest records the reports. There is no merged view: registered checks judge the configuration per fragment, and a key two fragments name keeps both values. Each coercion is judged by its key's policy, `accept`, `warn` or `reject`: the Spec's `policies.coercion` first, then `warn` for a Session, then the Vocabulary's declared default, which the Radio Model is to set to `reject` for sample rate and frequency and `warn` for gain, because a publication Run must not silently change its waveform timing.

Normative: [design/03-spec-and-binding.md](../03-spec-and-binding.md), rules SB-22b, SB-24, SB-39, SB-41, SB-42, SB-44…SB-46 (the fragments: table SB-T1); [design/04-run-and-session.md](../04-run-and-session.md), rules RS-2, RS-3, RS-6; [design/05-module-api.md](../05-module-api.md), MA-7; [design/06-kernel-coordinator.md](../06-kernel-coordinator.md), KC-27.

A Spec that asks for 19.5 Msps on a device whose grid gives 20 Msps sees the coercion before RUN, not in a plot afterwards. MockRadio produces the same report from the same coercion rules as the profile it emulates (§13).

It should not repeatedly dispatch every sample block through a generic Core layer.

High-rate execution must occur directly between prepared components.

---

← [Part 01: Purpose and Core Boundary](01-purpose-and-core-boundary.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 03: Simulation, Mock and Time](03-simulation-mock-and-time.md) →
