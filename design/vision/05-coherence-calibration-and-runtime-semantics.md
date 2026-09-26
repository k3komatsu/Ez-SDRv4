# Ez-SDR v4 — Architecture Vision · Part 05: Coherence, Calibration and Runtime Semantics

> Sections §25–§30 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 04: Execution Model and Data Contracts](04-execution-model-and-data-contracts.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 06: Performance, Execution Islands and Radio Backends](06-performance-islands-and-radio-backends.md) →

---

# 25. Coherent operation is different from mere multi-device operation

A set of channels existing simultaneously does not imply coherence.

Ez-SDR should distinguish concepts such as:

```text
DeviceGroup      channels that merely exist together
CoherentGroup    channels whose samples are aligned and whose phase relation is known
Array            a CoherentGroup with geometry
```

These are Radio Model (Vocabulary) concepts, and a CoherentGroup is **declared by the Provider that owns its channels**:

```text
CoherentGroup (provider-owned)
├── members                 channels of this Provider instance
├── basis                   shared_ref_clock | shared_pps | lo_sharing | timed_tune
├── stream                  the single N-channel stream that delivers aligned SampleBlocks
├── sample alignment state
├── phase calibration state / calibration references
├── channel mapping
└── coherence validity      invalidated by retune, clock loss, ALIGNMENT errors
```

Rules:

- **The Core never infers coherence.** Across Provider instances it composes only `ClockRelation`s (§24); a cross-provider set is called coherent only if a CalibrationArtifact (§26) asserts it, with provenance.
- Aligned multi-channel samples come from **one stream of one Provider instance**. A vector of independently started streams is a DeviceGroup, whatever its clock wiring.
- Coherence has a basis and a validity. A daughterboard whose LO phase is random after each retune is coherent only under timed tuning and only until the next untimed retune; the Provider declares this behaviour and MockRadio emulates it (MR-34).

This is important for:

- MIMO,
- coherent beamforming,
- ISAC arrays,
- distributed sensing,
- IBFD MIMO.

---

# 26. Calibration is a first-class research concept

Calibration must not be treated as an incidental script outside Ez-SDR.

The system should be capable of representing calibration artifacts such as:

```text
CalibrationArtifact
├── calibration_id
├── calibration kind
├── target resources
├── coefficients / data
├── method
├── creation time
├── validity conditions
├── uncertainty / quality
└── provenance
```

Possible calibration kinds include:

- gain/amplitude,
- phase,
- delay,
- IQ imbalance,
- DC offset,
- MIMO phase alignment,
- reciprocity,
- array calibration,
- IBFD SI path,
- sensing range/phase bias.

A Run should record which calibration was used.

Calibration validity may depend on:

```text
frequency
sample rate
gain
temperature
hardware identity
frontend configuration
time since calibration
```

## Where calibration is applied

The Kernel does not apply calibrations. It checks validity conditions when a Run binds a `CalibrationRef`, and it records which CalibrationArtifact was used. Application happens in exactly two places:

- in a **Processor** that takes a `CalibrationRef` parameter (per-channel complex gain, delay correction, IQ-imbalance correction), or
- in a **Provider** (timed tuning for constant phase offsets, device-side delay registers).

An experiment that applies calibration somewhere else (a post-processing script) gets no "calibration used" entry in its Manifest, and the Manifest is honest about that.

## Calibration is a Run

A calibration procedure — transmit a known signal, receive, estimate — is an experiment. It is an ExperimentSpec whose `outputs` include a CalibrationArtifact, executed under the same lifecycle and recorded in its own Manifest. The Kernel needs no calibration machinery beyond Artifact typing and validity checking.

## Delay calibration is not optional, even for SISO

Every radio has a fixed TX→RX delay through its digital and analog chain. Any timing claim — a packet arrival time, a radar range, an IBFD reference alignment — needs it. srsRAN carries it as a per-device `time_alignment_calibration` in samples; OpenAirInterface as `tx_sample_advance`. Ez-SDR carries a default per device profile in the Radio Model, as two capabilities in samples, `radio.tx.path_delay_samples` and `radio.rx.path_delay_samples` (RM-23), and allows a per-Run override by a CalibrationArtifact of kind `delay`.

## Retune phase behaviour is a declared capability

Daughterboards such as UBX/CBX/SBX have a random inter-frontend phase after every untimed retune; timed tuning keeps the offset constant. The Radio Model declares this behavior as a device capability:

```text
phase_behavior_on_retune:  deterministic | random_unless_timed_tune
```

declared per device profile as `radio.phase_behavior_on_retune`. Phase 2 records the capability; MockRadio emulates it from Phase 3 (MR-34).

---

# 27. Runtime parameter mutation must have semantics

Real-time experiments need to change parameters during RUN.

Examples:

- SIC coefficients,
- beamforming weights,
- thresholds,
- channel estimates,
- MCS,
- TX gain,
- antenna beam.

Not all updates are equivalent.

Every parameter that may change during a Run declares one update class from a closed set — `cold`, `block_boundary`, `atomic_realtime`, `hardware_timed` — a component parameter in its ComponentDescriptor (§19) and a Provider parameter in its Vocabulary's key declaration; a key with no class cannot change during a Run. An `UpdateParameter` Action carries its class and an optional instant at or after which it takes effect, and an update through an undeclared class is rejected at admission and never reaches a Module. What each class guarantees in timing is the Executor's and the Provider's to implement; the Kernel checks only that the class was declared.

Normative: [design/05-module-api.md](../05-module-api.md) §4 (`UpdateClass`), rules MA-24, MA-36, MA-37; [design/03-spec-and-binding.md](../03-spec-and-binding.md), SB-2; [design/04-run-and-session.md](../04-run-and-session.md), rules RS-4, RS-17, RS-49, RS-52; [design/01-time-model.md](../01-time-model.md), TM-13c.

Where useful, prepared configurations may switch at a barrier or GraphEpoch rather than mutating arbitrary graph state unsafely.

## No structural mutation during RUN

Parameter changes are the only runtime mutation. **The graph structure — its components, ports and links — does not change while a Run is RUNNING.** A change of structure is `stop → re-plan → start`, or, in a later version, an atomic switch to a fully prepared next GraphEpoch. GNU Radio 3.x's `lock()/unlock()` — stop the scheduler, re-flatten everything, restart, with buffer sizes that cannot change and a deadlock if called from the wrong thread — is the failure mode this rule avoids.

Consequences:

- A component whose behaviour must change at run time (an adaptive-MCS demodulator) is one component with a parameter, not a family of components swapped in and out.
- v3's practice of pausing every thread that touches a device before applying a change is the `cold` class, made explicit.
- A sample-rate change is always `cold` (§15).

GraphEpoch is deferred until a concrete experiment needs it; the prohibition is not.

---

# 28. Stream continuity and data validity are first-class

A RuntimeEvent saying that an overflow occurred is not sufficient for publication-grade data.

The resulting data must expose whether samples are valid.

These are derived from the Stream Contract flags (§23), never assembled separately:

```text
ContinuityMap   valid ranges and gaps per channel, derived from GAP_BEFORE / RESTARTED / valid masks
ValidityMap     per-channel validity over the whole artifact
Gap             { start_time, lost: optional count, cause }   cause from a closed set, derived from the flags
```

Propagation of invalidity through Processors ("taint") is a documented convention for Processor authors, not a Kernel type. Executors propagate block flags by default whenever a component maps input blocks to output blocks one-to-one; where the mapping is not one-to-one (an FFT taints its whole output block, a FIR only a tail), the Processor author decides and documents it. There is no `Taint` type in the Kernel.

A capture should be able to describe:

- valid ranges,
- known sample gaps,
- lost sample count where known,
- sequence discontinuity,
- partial channel loss,
- multi-channel alignment failure.

This information should propagate into Artifacts and Run provenance.

SigMF interoperability should be considered for captured IQ datasets.

---

# 29. Observability is structured, typed, and non-blocking

Backend warnings and failures must never exist only as console text.

Examples:

```text
RX_OVERFLOW
TX_UNDERFLOW
TX_DISCONTINUITY
LATE_COMMAND
ALIGNMENT_ERROR
CLOCK_LOST
DEVICE_LOST
PROCESSOR_DEADLINE_MISS
PERIPHERAL_TIMEOUT
TUN_QUEUE_DROP
PLUGIN_FAILURE
CALIBRATION_INVALID
```

Each kind is registered by whoever emits it; only a handful are the Kernel's (below).

The model is:

```text
hot-path condition
      ↓
bounded event path
      ↓
collector
      ↓
policy / metrics
      ↓
Python / AI / Run record
```

Event reporting must never block the real-time sample path.

Event storms and event-queue loss must themselves be observable.

## Event envelope, counters and storms

An Event carries a node-qualified source, a TimePoint in a named domain, a severity, a kind and a schema-versioned payload. The hot path emits a fixed-size record with at most 32 bytes inline and allocates nothing. Every `(source, kind)` pair reachable in the plan has a **never-dropping counter**, incremented before the body is queued. Bodies travel through a bounded ring and are dropped, not sampled, when it is full; each drain emits, outside the ring, one `EVENTS_DROPPED { kind, count }` per kind that dropped, so every counter equals the delivered bodies plus the reported drops, and a ten-minute Run with thirty thousand overflows through a queue of four thousand records thirty thousand, not four thousand. A kind whose reaction is `stop` or `abort` also raises an escalation flag that survives a dropped body. The Kernel registers only the kinds it emits or owns the policy for — `EVENTS_DROPPED`, `LINK_BACKPRESSURE`, `PROCESSOR_DEADLINE_MISS`, `DEVICE_LOST`, `STEP_LIVELOCK` — and every other kind is registered by its Vocabulary or Module.

Normative: [design/04-run-and-session.md](../04-run-and-session.md), rules RS-27…RS-36.

UHD's own asynchronous message queue is bounded (a thousand entries), and its console prints `O`, `D`, `U` and `L` are documented as "generally harmless". For publication-grade data they are not harmless, and Ez-SDR counts them.

## No metrics framework in the Kernel

Queue occupancy, deadline-miss rates and overflow counts are counters and delivered events on this same path. The Kernel has no metrics registry; exporting counters to Prometheus, CSV or a dashboard is a Sink Module's job.

---

# 30. Probe / Tap is important for research instrumentation

Publication experiments often need intermediate results:

- CFO estimates,
- channel estimates,
- equalized constellations,
- detector metrics,
- SIC residuals,
- beamforming weights,
- delay-Doppler maps,
- queue latency,
- Processor timing.

The graph should support non-invasive observation.

Conceptually:

```text
Main data path ───────────────→ next Processor
       │
       └→ bounded lossy Probe
                 ↓
              recorder
```

A Probe must not impose backpressure on the real-time path.

## Probe is not a Kernel concept

A Probe is a **lossy DataLink** (`drop_oldest` or `drop_newest`, §23) feeding a **Recorder Sink**. Nothing else is needed: fan-out shares block references, so tapping costs no copy, and intermediate values such as CFO estimates, channel estimates or SIC residuals are ordinary additional output ports of the Processor with `event.*` or `tensor.*` contracts (§21). The Kernel defines link policies and DataContracts; it does not define "Probe".

Recorder controls are optional. The feed's drop-class policy owns `drop if busy`; a Recorder may expose the other controls as Sink parameters:

```text
sample every N blocks
decimate
maximum output rate
drop if busy
```

---

← [Part 04: Execution Model and Data Contracts](04-execution-model-and-data-contracts.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 06: Performance, Execution Islands and Radio Backends](06-performance-islands-and-radio-backends.md) →
