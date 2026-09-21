# Ez-SDR v4 — Architecture Vision · Part 10: Implementation Path and Testing

> Sections §57–§61 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 09: Provenance, Validation and Ownership](09-provenance-validation-and-ownership.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 11: Boundaries, Invariants and Roadmap](11-boundaries-invariants-and-roadmap.md) →

---

# 57. Minimal implementation path

The first implementation should be much smaller than the full Vision.

Recommended initial stack:

```text
Kernel (tiers, time, Stream Contract, Sessions, composite resources)
+
Radio Model (including the TimingEnvelope)
+
Simulation Engine (discrete-event Time Authority)
+
MockRadio (enforcing the x310-like envelope)
+
SimulationChannel (coupling matrix; loopback / gain / delay / AWGN)
+
RuntimeEvent pipeline
+
Python client (Sessions)
```

No physical SDR is required.

At this stage this should already work:

```python
with ezsdr.connect() as sdr:
    sdr.tx.repeat(x)
    y = sdr.rx.capture(N)
```

but entirely in software, and it must already produce a Session Manifest containing the action log, the waveform hash, the capture's first sample time and validity flags, and the effective configuration.

The initial SimulationChannel may support only:

```text
loopback
gain
delay
AWGN
```

---

# 58. First architecture acceptance tests

Before UHD is implemented, the Core + Mock system should prove:

1. **Binding substitution works.**
2. **Virtual time works faster than wall clock**, driven by the Simulation Engine (§15).
3. **Deterministic runs reproduce with a seed** (Simulation class), including the order of events and Reactor decisions.
4. **RuntimeEvents flow through the same path used by hardware.**
5. **Fault injection triggers cleanup and policy behavior.**
6. **Sample continuity/validity can represent gaps** as Stream Contract flags, and an injected overflow shows the same gap, restart and flags as a UHD overflow (§23).
7. **Runs record ExperimentSpec, BindingProfile (with environment), plan, events, and artifacts.**
8. **Two MockRadios can communicate through a SimulationChannel** declared in the BindingProfile environment, with an ExperimentSpec that contains no channel configuration (§8).
9. **Reactive execution can receive an event and generate a dynamic TxBurst.**
10. **Application logic does not depend on Mock-specific APIs.**
11. **Mock enforces the envelope.** A timed TX issued with less than `min_timed_command_lead` on an `x310-like` Mock produces the same typed `TIME_ERROR` as hardware; a request for 19.5 Msps is coerced to 20 Msps and reported as requested-versus-applied (§13). A request beyond the profile's PerformanceEnvelope is rejected at `validate()`.
12. **Block-size independence.** A Processor keeps working when the Mock's block-length jitter option is enabled (§23).
13. **Sessions leave provenance.** Using only the Easy API produces a Manifest with the action log, waveform hash and effective configuration (§3).
14. **Environment portability.** Re-running with the same Spec and a BindingProfile that differs only in `environment` (channel model, fault schedule) is possible without editing the Spec (§8).
15. **TX bursts are closed and contiguous.** An unclosed burst followed by a timed block yields the same `TIME_ERROR` on Mock and hardware; a `repeat` burst does not underflow at the wrap; a time jump inside a burst is a `TX_DISCONTINUITY` event, never zero padding (§23).
16. **Session Actions are admitted.** A runtime frequency change outside the RF envelope, or a rate outside the profile's PerformanceEnvelope, is rejected and logged as rejected, on Mock and hardware alike (§3, §13, §52).

A useful minimal reactive test is:

```text
Radio A sends PING
        ↓
Radio B receives PING
        ↓
Reactor
        ↓
dynamic timed PONG
        ↓
Radio A receives PONG
```

This tests much of the future packet-radio architecture without implementing IEEE 802.11.

---

# 59. UHD is added only after the Core + Mock model works

The UHD milestone should test a fundamental architectural claim:

> **Existing experiments written for MockRadio should run against a real USRP by changing the BindingProfile rather than changing application logic.**

If large semantic changes are required when moving from Mock to UHD, the abstraction boundary must be reconsidered.

Hardware-specific extensions are allowed.

Hardware-specific rewrites of experiment logic should not be the normal path.

The parity test also measures the hardware's envelope — minimum timed-command lead, coercion grid, stop tail, restart gap, sustainable throughput — and **fails if the Mock profile is more permissive than the measurement**. Profile values are updated from that measurement and the profile version is bumped. A Mock that is stricter than the hardware is acceptable; one that is looser is a defect.

---

# 60. Suggested logical repository structure

This is directional, not a requirement to create every crate immediately.

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

Logical boundaries matter more than crate count.

"Distributed" is not a Module category. It is a deployment topology whose only Module-level footprint is a NetworkLink under `link/` (§49). Artifact writers are Sinks (§7).

Do not create a large number of crates merely to mirror this diagram.

---

# 61. Testing philosophy

The majority of correctness tests should not require hardware.

Test categories should eventually include:

- unit tests,
- state-machine tests,
- ExperimentSpec validation,
- binding tests,
- Module contract tests,
- Mock integration tests,
- deterministic simulation tests,
- fault injection tests,
- ClockRelation tests,
- continuity/validity tests,
- event-storm tests,
- malformed/fuzz tests,
- Processor/Reactor contract tests,
- memory/copy regression tests,
- throughput/latency benchmarks,
- peripheral plugin tests,
- TUN/TAP tests,
- hardware-in-the-loop tests,
- UHD hardware tests,
- v3 behavioral compatibility tests where useful.

v3 compatibility means behaviour, not wire protocol. The v3 message format and command identifiers are not carried forward. The behaviours worth a regression test are:

- a repeating TX stays continuous across the waveform wrap (no underflow at the loop boundary);
- a capture can start at a deterministic, requested position: `capture(n, at: TimePoint | sample_index)` replaces v3's `alignSize` multiples, which users abused to detect one-sample drifts by hand;
- a timed start of TX and RX at a given device time (v3's `onTime`);
- several devices sharing 10 MHz and PPS start aligned, with the PPS source armed first.

Performance is a regression-tested property.

---

← [Part 09: Provenance, Validation and Ownership](09-provenance-validation-and-ownership.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 11: Boundaries, Invariants and Roadmap](11-boundaries-invariants-and-roadmap.md) →
