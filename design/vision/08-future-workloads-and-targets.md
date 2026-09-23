# Ez-SDR v4 — Architecture Vision · Part 08: Future Workloads and Targets

> Sections §42–§49 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 07: Peripherals and Host I/O](07-peripherals-and-host-io.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 09: Provenance, Validation and Ownership](09-provenance-validation-and-ownership.md) →

---

# 42. WASM is a future Processor/Reactor implementation target

WASM is attractive for:

- portability,
- sandboxing,
- dynamic deployment,
- AI-generated code,
- SIMD,
- language independence.

Real-time WASM should use a strict profile.

Possible constraints include:

- fixed/bounded memory,
- no memory growth during RUN,
- no arbitrary file/network/process access,
- pre-instantiation,
- bounded calls,
- minimal host calls,
- block-oriented ABI,
- deterministic lifecycle,
- SIMD where available.

The exact WASM runtime must not leak into Core semantics. The WASM Executor owns its block ABI and consumes the same ComponentDescriptor (§19) and schema-first Event/Action types (§10) as every other Executor; its linear memory is one MemoryDomain among others (§23).

---

# 43. AI is outside the hard real-time loop

AI-agent integration is orchestration.

It is not a replacement for deterministic real-time execution.

```text
                    AI Agent
                       │
                  slow control
                       │
                       ▼
                Ez-SDR Runtime
                       │
──────────────────────────────────
          real-time boundary
──────────────────────────────────
                       │
             Processor/Reactor graph
                       │
                  Radio / Hardware
```

AI may:

- inspect resources,
- inspect capabilities,
- generate ExperimentSpecs,
- generate Processing components,
- run software simulations,
- inject faults,
- inspect metrics,
- validate plans,
- launch hardware experiments,
- compare Runs.

It should not be required for microsecond-scale decisions.

Before an agent's Spec reaches hardware it passes `validate()`, including the site's RF safety envelope (§52).

---

# 44. AI-generated real-time components

A long-term workflow is:

```text
AI generates DSP / MAC component
        ↓
compile
        ↓
validate ABI / sandbox
        ↓
unit tests
        ↓
deterministic simulation
        ↓
fault tests
        ↓
performance benchmark
        ↓
real-time emulation
        ↓
HIL
        ↓
physical hardware
```

WASM may provide the default trust boundary for generated code.

AI-generated native shared libraries should not automatically be considered safe.

---

# 45. MIMO and coherent systems must be natural, not special cases

The architecture should support progression from:

```text
single radio
→ 2×2 MIMO
→ multi-USRP coherent MIMO
→ large arrays
→ distributed / cell-free systems
```

Important concepts include:

- CoherentGroup,
- calibration,
- aligned sample sets,
- partial channel validity,
- matrix/tensor ports,
- multi-clock relations,
- memory placement,
- distributed placement.

A simple vector of independent RX streams is insufficient as the only abstraction.

Coherence is declared by the Provider that owns the channels and never inferred by the Core (§25); a request for coherent channels binds to one Provider instance (§8).

---

# 46. IBFD must be a normal supported workload

IBFD may require:

```text
simultaneous RX + TX
shared clocks / LO assumptions
high dynamic range
PA nonlinearity
IQ imbalance
SI channel
analog cancellation
digital cancellation
adaptive coefficient updates
clipping/saturation observability
external attenuator / phase-shifter control
```

The Processor/Reactor + Peripheral + Calibration architecture must support this without special-case Core APIs.

The same IBFD graph should be runnable in:

```text
software SI simulation
→ RF loopback
→ real OTA hardware
```

with appropriate BindingProfiles.

The digital canceller's reference is the timed TX stream itself: TX blocks carry their target TimePoint (§23), so a tap on the TX edge plus the calibrated TX→RX delay (§26) aligns reference and receive samples without a special API. In simulation the self-interference path is the diagonal block of the SimulationChannel coupling matrix (§16).

---

# 47. ISAC must support heterogeneous sensing resources

ISAC may combine:

```text
SDR array
smart antenna
positioner
camera
external trigger
GPU processing
communication PHY
radar/sensing Processor
```

The same Run should correlate:

- IQ time,
- peripheral state,
- external sensor time,
- calibration state,
- sensing results,
- communication results.

This motivates ClockRelation, CalibrationArtifact, Tensor ports, and structured provenance.

## External sensor data is referenced, not ingested

Ez-SDR is not a sensor-data platform. A camera or motion-capture system appears in a Run in one of two ways: as a Peripheral that accepts a hardware trigger (`trigger_input`), or as a Peripheral that emits timestamped `frame_captured` events in its own ClockDomain together with a reference to the file it wrote. The frames themselves are Artifacts **by reference** (path plus hash); the Run never streams video through the data plane. Correlation with IQ time is a `ClockRelation` (§24) plus post-processing. Published ISAC work on USRP X410 arrays correlates motion-capture data by recorded start-time offsets in exactly this way.

---

# 48. OTFS and tensor-heavy processing must not require redesign

OTFS and related research may transform:

```text
IQ SampleStream
    ↓
time-frequency representation
    ↓
delay-Doppler Tensor
    ↓
detector / equalizer
```

The architecture must support:

- Tensor data,
- GPU execution,
- explicit memory placement,
- large intermediate data,
- Probe/Tap observation,
- CPU fallback,

without changing Core semantics.

---

# 49. Distributed execution is a future implementation, but not a forbidden model

Full distributed multi-host execution does not need to exist in v4.0.

However, the domain model must not permanently assume:

```text
one machine = one experiment
```

Future concepts may include:

```text
ComputeNode
ExecutionIsland
NetworkLink
Placement
ClockRelation
```

v4.0 may resolve everything to the local machine.

The architecture should avoid decisions that make future multi-host execution impossible without replacing the public model.

What v4.0 does about it is small and concrete: every identifier that could later cross a host boundary is **node-qualified**.

`ClockDomainId`, `MemoryDomainId`, `IslandId` and `DataLinkId` are `{ node, local }`, and `ResourceId` is `{ node, path }`, because a resource is a composite tree whose channels, GPIO banks and timekeeper must each be addressable (§8). In v4.0 `node` is always the local node, and any document or value naming another node is refused.

Normative: [plan/phase1/00-overview.md](../../plan/phase1/00-overview.md), X7; [design/01-time-model.md](../01-time-model.md), TM-11; [design/02-stream-contract.md](../02-stream-contract.md), SC-6; [design/03-spec-and-binding.md](../03-spec-and-binding.md), rules SB-1, SB-3.

A future NetworkLink is one more DataLink implementation; a future remote Provider is one more Module. Neither requires a ComputeNode scheduler, and Ez-SDR will not grow one. Cross-site coherence is a ClockRelation with uncertainty: sub-nanosecond distribution such as White Rabbit exists, but it is a physical fact the Core records, not one it manufactures.

---

← [Part 07: Peripherals and Host I/O](07-peripherals-and-host-io.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 09: Provenance, Validation and Ownership](09-provenance-validation-and-ownership.md) →
