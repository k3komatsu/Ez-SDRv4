# Ez-SDR v4 — Architecture Vision · Part 09: Provenance, Validation and Ownership

> Sections §50–§56 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 08: Future Workloads and Targets](08-future-workloads-and-targets.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 10: Implementation Path and Testing](10-implementation-path-and-testing.md) →

---

# 50. Artifacts, Runs, and provenance are first-class

Every execution creates a Run, including an interactive Session (§3), whose Manifest carries its action log.

A Run should record enough information to reproduce or audit the experiment.

The Manifest is a **Kernel envelope with namespaced sections**. The Kernel writes the envelope; each Module writes its own section; an optional environment capture can be switched on per profile. No Provider-specific field ever needs a Kernel change.

```text
Manifest
├── envelope (Kernel)
│   ├── run id, parent Session, ExecutionClass, fidelity vector
│   ├── ExperimentSpec  (hash + body; or the implicit Spec of a Session) and the action log
│   ├── BindingProfile  (hash + body, including its environment part)
│   ├── ExecutionPlan summary; placement as bound
│   ├── PrepareReport   (requested vs applied configuration, coercions, warnings)
│   ├── Module identities and versions; component implementation hashes; waveform hashes
│   ├── epoch ↔ UTC ClockRelation with uncertainty; clock/time configuration
│   ├── calibration artifacts used (by reference and hash)
│   ├── event counters (complete) and sampled RuntimeEvents; EVENTS_DROPPED
│   ├── continuity / validity metadata of every capture
│   ├── random seeds
│   ├── Lease mode and termination reason
│   └── Artifact references with content hashes
├── sections (Modules, namespaced)
│   ├── uhd.*          UHD version, FPGA image, device serials, daughterboards, transport settings
│   ├── mock.*         profile, envelope, injected faults
│   ├── peripheral.*   commands issued, events, baseline restore
│   └── calibration.*  method, uncertainty
└── environment capture (optional, per profile)
    └── host.*         CPU / NIC topology, kernel and RT settings, git commit, performance metrics
```

Everything that can be hashed is content-addressed: two Runs with equal Spec, BindingProfile, component and waveform hashes are comparable by construction.

Large artifacts should normally be returned by reference.

---

# 51. Artifact formats should interoperate with existing ecosystems

Ez-SDR should not invent unnecessary proprietary data formats.

For captured IQ data, SigMF interoperability should be considered a standard feature.

For large tensors and derived measurements, formats should be selected based on:

- efficient access,
- metadata fidelity,
- scientific reproducibility,
- interoperability.

The Run Manifest should reference artifacts rather than embedding large data.

---

# 52. Validation and dry-run are first-class

Before touching real hardware, users and AI agents should be able to request:

```text
validate(spec, binding)
```

and ideally:

```text
plan(spec, binding)
```

Validation may determine:

- schema validity,
- resource availability,
- capability compatibility,
- achievable rates,
- timing feasibility,
- calibration requirements,
- memory requirements,
- Processor placement,
- Host/GPU/device transfer cost,
- Peripheral timing guarantees,
- required extensions,
- selected fallback strategies,
- performance risks.

A failure should happen before RF transmission whenever possible.

## The RF safety envelope

When an AI agent writes the ExperimentSpec, `frequency: 1.575e9, gain: max` is a valid Spec. What makes it unsafe is the site, and the site is described by the BindingProfile. Its laboratory environment therefore carries an RF envelope:

```text
rf_envelope
├── allowed_bands                     [{ lo_hz, hi_hz }]
├── max_gain_db  |  max_power_dbm     per channel
├── tx_enabled                        per channel
└── antenna_ports                     allowed port names
```

`validate()` and `prepare()` enforce it as a Kernel policy, and the Provider re-checks the *applied* values, so a coercion that lands outside the envelope is rejected as well. Session Actions that change RF parameters pass the same check on the control path before dispatch (§3). **Nothing transmits before `validate()` passes, including this check.** MockRadio runs the same check, so an agent learns the site limits in simulation.

## What each step returns

`validate(spec, binding)` returns the admission result: matched capabilities, rejected constraints, envelope violations. `plan(spec, binding)` additionally returns the ExecutionPlan summary with placement as bound and the transfer costs it implies. `prepare()` returns the PrepareReport (§11) with the effective configuration. All three are available to Python and to AI agents before any RF energy is emitted.

---

# 53. Resource ownership and cleanup are transactional

SDR systems can transmit RF energy and maintain state.

Resource ownership must therefore be explicit.

On:

```text
client disconnect
AI-agent failure
timeout
Processor crash
Reactor failure
stream error
plugin crash
device disconnect
runtime abort
```

the Runtime must have deterministic policies for:

- stopping TX,
- stopping RX,
- cancelling pending bursts,
- cancelling peripheral operations where possible,
- restoring baseline state,
- marking partial artifacts,
- recording failure events,
- releasing leases.

TX must not continue indefinitely merely because a client disappears.

## Lease modes

```text
Lease
├── Attached                       ends with the Session or client connection; TX stops, resources release
└── Detached { ttl, renewable }    survives disconnect until the TTL expires; must be explicit
```

The default is Attached. A student who wants to set up a repeating transmission, disconnect and walk to a spectrum analyser asks for a Detached lease with a TTL; the Manifest records it, a reconnecting client may adopt it, and on TTL expiry the cleanup above runs. An AI agent that crashes leaves an Attached lease, so its TX stops. Both behaviours are correct; the mode makes the choice visible. A child Run created inside a Session inherits the Session's Lease: ending the Session ends its children, and a Detached Session keeps its children until its TTL expires.

## Policy is a closed table

```text
Policy
└── on(kind) -> continue | mark_artifact | stop | abort
```

A Run's failure policy is a declarative table from event kinds to one of four reactions, plus the cleanup sequence above. It is not a rules engine and has no expression language; conditions that need logic belong in a Reactor or in Python orchestration. Defaults are conservative: an RX overflow continues and marks the artifact; a lost device aborts.

---

# 54. Python and ExperimentSpec are complementary

Python remains the natural environment for:

- loops,
- parameter sweeps,
- high-level adaptation,
- analysis,
- plotting,
- experiment generation,
- ML workflows.

ExperimentSpec is a safe and reproducible execution unit.

Example:

```python
for snr in snrs:
    spec = build_experiment(snr=snr)
    result = sdr.run(spec)
    ber = analyze(result)
```

The Easy API and Experiment API must map to the same underlying Core semantics. The Easy API does so through Sessions (§3): every call is a typed Action in a Session Run's log, and `sdr.run(spec)` opens a child Run.

The Python client provides `sdr.sleep(d)`, defined as `run.wait_until(now + d)` in the Run's time. On hardware it coincides with a wall-clock sleep; in simulation it is the only sleep that means anything. `time.sleep` is documented as wall-clock-only, and the client never uses it itself.

---

# 55. Time-scale separation is a design guide

A useful guideline is:

| Time scale | Typical execution location |
|---|---|
| ns–µs | FPGA / RFNoC / hardware |
| µs–ms | native/WASM real-time Processor/Reactor |
| ms–s | Python / host control |
| s–min | AI agent / experiment orchestration |

This is not an absolute rule.

It is a placement guide.

---

# 56. Reference architecture litmus test: IEEE 802.11-like transceiver

A strong architectural test is:

> **Can Ez-SDR express and execute a packet-radio transceiver that continuously receives, decodes packets, updates protocol state, dynamically generates a response, and performs hardware-timed TX?**

Conceptually:

```text
Linux TAP
    │
    ▼
 MAC Reactor
    │
    ▼
 PHY TX
    │
    ▼
 Radio TX

 Radio RX
    │
    ▼
 PHY RX
    │
    ▼
 MAC Reactor
    │
    ▼
 Linux TAP
```

A received packet may trigger:

```text
RX packet decoded
      ↓
MAC decision
      ↓
TxBurst generated dynamically
      ↓
timed TX
```

This must be a normal execution model, not an exception.

One honesty clause: SIFS-class turnaround (tens of microseconds) is not achievable with host-generated timed TX on an Ethernet-attached USRP. The litmus test is an 802.11-*like* protocol whose turnaround lies within the bound Provider's TimingEnvelope (§13). `validate()` must report an envelope violation before the Run; hardware must never be the first place it is discovered.

---

← [Part 08: Future Workloads and Targets](08-future-workloads-and-targets.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 10: Implementation Path and Testing](10-implementation-path-and-testing.md) →
