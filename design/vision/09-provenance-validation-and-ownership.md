# Ez-SDR v4 — Architecture Vision · Part 09: Provenance, Validation and Ownership

> Sections §50–§56 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 08: Future Workloads and Targets](08-future-workloads-and-targets.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 10: Implementation Path and Testing](10-implementation-path-and-testing.md) →

---

# 50. Artifacts, Runs, and provenance are first-class

Every execution creates a Run, including an interactive Session (§3), whose Manifest carries its action log.

A Run should record enough information to reproduce or audit the experiment.

The Manifest is a **Kernel envelope with namespaced Module sections**, written once at the end of every Run that terminates, a failed Run included, and then sealed. The envelope carries a mandatory `version` and records the Run, its ExecutionClass, fidelity vector and state transitions; the Spec (or a Session's implicit Spec) and the BindingProfile, each as hash plus body, with the environment verbatim; the plan with placement as bound, the admission result and the PrepareReports; Module and Vocabulary versions and component implementation hashes; input artifacts such as waveforms and calibration, by reference; the clock domains and SampleClocks with a relation to UTC and its uncertainty; the complete event counters; the Lease, the action log and the termination; and the produced artifacts with their hashes and continuity. Each Module writes only under its own namespace — `uhd.*` for UHD version, FPGA image and serials, `mock.*` for the profile and injected faults — so no Provider-specific field needs a Kernel change. Random seeds and an environment capture are not envelope fields: a seed is part of the environment, which is already recorded, and the capture is the `ezsdr.capture` section written by the Module that produces it. Everything hashable is content-addressed, so two Runs with equal Spec, BindingProfile, component and input hashes are comparable by construction; large data is always by reference; and the Manifest's own hash is stored beside it, never inside the hashed body.

Normative: [design/04-run-and-session.md](../04-run-and-session.md), rules RS-1, RS-11a, RS-38…RS-47; [design/01-time-model.md](../01-time-model.md), rules TM-13d, TM-18; [design/02-stream-contract.md](../02-stream-contract.md), rules SC-28, SC-30.

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

The list is a scope, not a guarantee; what `validate()` returns is stated below.

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

The Kernel does not interpret the envelope. The Radio Model registers an admission check against its `radio.rf_envelope` section, and the Kernel guarantees that every registered check runs at three points: `validate()` over each fragment's requested configuration; `prepare()` over each Provider fragment's effective configuration, so a coercion that lands outside the envelope is refused; and the admission of every Session Action before dispatch (§3), over each fragment's effective configuration with the proposed value overlaid on its target fragment. Checks judge per-fragment values rather than a merged configuration, so two Providers' values cannot mask one another. A check is pure and needs no hardware, so `validate()` is a true dry run. **Nothing transmits before `validate()` passes, including this check.** MockRadio runs the same check, so an agent learns the site limits in simulation.

## What each step returns

`validate()` returns the matched resources, the rejected constraints, the envelope violations, a coercion preview and warnings; `plan()` adds the fragments, links, dependency edges, the Authority, the derived ExecutionClass and the declared transfer costs; `prepare()` returns a PrepareReport per fragment and the merged effective configuration. All three are available to Python and to AI agents before any RF energy is emitted.

Normative: [design/03-spec-and-binding.md](../03-spec-and-binding.md), rules SB-29…SB-31, SB-38, SB-39, SB-41; [design/04-run-and-session.md](../04-run-and-session.md), RS-17.

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

the Runtime runs one ordered cleanup, however the Run ends: (0) end child Runs; (1) freeze dispatch and cancel pending bursts and timers; (2) stop TX and (3) then RX, each in reverse dependency order; (4) cancel Peripheral operations; (5) restore baseline state; (6) finalise artifacts, marking open ones partial; (7) flush events and collect counters; (8) release the Lease and write the Manifest. The freeze precedes stopping TX, because a queued timed burst would otherwise reopen it. Every step runs under a deadline and is attempted even when an earlier one failed, so a failed Run's Manifest is always written. A Lease is `Attached` by default and ends the Run when its client disconnects; a `Detached` Lease needs a TTL on the host monotonic clock, and only its adoption token reclaims it; a child Run inherits its parent's Lease. The Policy is a closed table from registered event kinds to `continue`, `mark_artifact`, `stop` or `abort`; each kind's default is declared where the kind is registered — the Kernel's `DEVICE_LOST` aborts, and the Radio Model is to declare `RX_OVERFLOW` continue-and-mark — and a kind with no entry falls back by severity.

The Kernel maps these numbered steps to role calls: step 2 calls `Provider::stop(mode)`; in an orderly Simulation Run, step 3 drains the stepped instances and then calls `Executor::stop(mode)` and `Sink::stop(mode)`, collecting returned artifacts; step 5 calls `cleanup()` on every instance that reached `prepare()`. The Provider owns its internal TX-before-RX stop order.

Normative: [design/04-run-and-session.md](../04-run-and-session.md), rules RS-6…RS-11a, RS-21…RS-25a, RS-26…RS-30.

TX must not continue indefinitely merely because a client disappears.

## Lease modes

The default is Attached. A student who wants to set up a repeating transmission, disconnect and walk to a spectrum analyser asks for a Detached lease with a TTL; the Manifest records it, a reconnecting client may adopt it, and on TTL expiry the cleanup above runs. An AI agent that crashes leaves an Attached lease, so its TX stops. Both behaviours are correct; the mode makes the choice visible. A child Run created inside a Session inherits the Session's Lease: ending the Session ends its children, and a Detached Session keeps its children until its TTL expires.

## Policy is a closed table

A Run's failure policy is a declarative table from event kinds to one of four reactions, plus the cleanup sequence above. It is not a rules engine and has no expression language; conditions that need logic belong in a Reactor or in Python orchestration.

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
