# Ez-SDR v4 — Architecture Vision · Part 03: Simulation, Mock and Time

> Sections §12–§17 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 02: ExperimentSpec, BindingProfile and Compilation](02-spec-binding-and-compilation.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 04: Execution Model and Data Contracts](04-execution-model-and-data-contracts.md) →

---

# 12. Mock is a first-class execution target

Mock must not be:

```text
MockUHDBackend
```

Instead:

```text
                 Radio Contract
                 /            \
                /              \
       MockRadioProvider    UHDRadioProvider
                                  │
                                 UHD
```

Mock and UHD are peers.

Mock is a software implementation of the Radio model.

Mock is a peer *because* it implements the same contract **and the same constraint envelope** (§13). A Mock that accepts what its emulated hardware would reject is a bug.

---

# 13. Software-only execution must work without SDR hardware

A substantial part of Ez-SDR should work with no SDR attached and ideally without UHD installed.

A software environment may contain:

```text
Simulation Environment
├── MockRadio
├── SimulationChannel (the `sim` Vocabulary's shared medium, not a Module)
├── SimulationEngine (discrete-event Time Authority; virtual clock, §15)
├── MockPeripheral
└── MockNetworkEndpoint
```

The BindingProfile's `sim.faults` environment document is read by each target Provider; it is not a FaultInjector Module. Likewise the SimulationChannel is not a Module: it is the `sim` Vocabulary's shared medium, which the runtime hands to each simulated radio, configured by the `sim.channel` environment document (§16).

An AI agent should be able to:

1. generate or modify an experiment,
2. run it entirely in software,
3. inspect events and artifacts,
4. test failure behavior,
5. benchmark processing components,
6. perform real-time emulation,
7. promote the experiment to HIL,
8. finally bind it to real SDR hardware.

## The Timing Envelope, and why Mock must enforce it

"Change only the BindingProfile" holds only if the constraints of the hardware are (a) visible at `validate()` time and (b) enforced by the Mock. The constraints are concrete: a minimum lead time for timed commands, a start-up latency (PPS synchronisation takes up to two seconds), a tail of buffered samples after stop, coercion of sample rate / gain / frequency to a device grid, a restart gap after an RX overflow (UHD restarts a continuous stream about 50 ms later), a bounded timed-command queue, and alignment constraints for repeat waveforms held in device memory.

A software radio that accepts everything is the OpenAirInterface `rfsimulator` pattern: it implements the same device interface as real hardware but never emulates late, underflow or overflow, so experiments pass in software and fail on the bench.

Therefore the Radio Model requires every Provider to publish, next to its PerformanceEnvelope (§34), timing values as capabilities:

```text
TimingEnvelope capabilities (RM-6)
├── radio.timing.min_timed_command_lead_ns
├── radio.timing.startup_latency_ns
├── radio.timing.stop_tail_ns
├── radio.timing.command_queue_depth
├── radio.timing.overflow_restart_gap_ns
├── radio.timing.restart_lead_ns
└── radio.timing.start_lead_ns
```

The Radio Model Vocabulary defines coercion rules; supported grids, such as the values in `radio.rx.sample_rate_hz` and `radio.tx.sample_rate_hz`, are declared as capabilities. A Provider also exposes its command lead through the generic `ProviderInstance.min_command_lead`. The Kernel uses that field for Action admission and the pre-start `RejectAtPlan` check; it does not read `radio.timing.*` by name (KA-7).

and the following are rules, not options:

1. **MockRadio enforces the envelope of the profile it emulates** (`x310-like`): it refuses `start(T0)` before synchronization ends, rather than starting late (MR-11); a timed command issued with insufficient lead produces the same typed `LATE_COMMAND` / `TIME_ERROR` event as the hardware; requested rates are coerced on the same grid and reported in each fragment's PrepareReport (§11); `stop` delivers the same tail; an injected overflow reproduces the RM-17/MR-21 restart gap and block flags (§23); and a request that exceeds the profile's PerformanceEnvelope (§34), channel count × rate × format beyond the emulated transport, is rejected at `validate()` / `prepare()`, so a four-channel 200 Msps request fails on a 1GbE-like profile in simulation as it would on the bench.
2. **Timing requirements are checked at the layer that owns them.** The generic matcher compares requested timing constraints with the bound Provider's declared TimingEnvelope capabilities at `validate()`; for Actions, the Kernel uses only `ProviderInstance.min_command_lead`, and the target Provider enforces its own timed commands. Mock and hardware follow the same checks.
3. **RF behaviour is not part of the envelope.** Propagation stays in the SimulationChannel (§16); the device's own RF behaviour — gain, LO phase, clipping, path delay — belongs to the radio model (MR-32…MR-36). RF behaviour no bound model covers is reported as unmodelled (§14).

Profiles are versioned. An `x310-like` profile carries a version that the Manifest records in its `mock.*` section, and its envelope values come from measurement on the hardware, not from data sheets (§59). An `ideal` profile without constraints may exist for algorithm work; a Run on it is recorded with `timing: none` and `coercion: none` and is not evidence for promotion.

The first MockRadio implementation must enforce the envelope. It is a few timestamp comparisons, and it is the difference between a validation tool and a source of false confidence.

---

# 14. Simulation execution classes

Every Run must record what kind of evidence it represents.

The ExecutionClass is one of Simulation, RealtimeEmulation, HardwareInLoop and Hardware. It is derived at binding resolution from two axes — whether the Time Authority is the Simulation Engine (free-running or wall-paced) or a device timekeeper (§15), and whether the RF path is simulated, cabled or over the air — and cross-checked against any `ezsdr.time` class the environment declares; a simulated Authority on a real RF path is refused. The class never changes during a Run. Determinism is a property of the Simulation class only, and only with a recorded seed: RealtimeEmulation trades it for real deadlines, and HardwareInLoop and Hardware are never deterministic. A single label cannot express "timing is modelled, RF is not", so every Run records a **fidelity vector** over five aspects — timing (lead times, start-up, stop tail, queue depth), continuity (overflow gaps, restarts, sequence errors), coercion (rate, gain and frequency grids), rf (SimulationChannel models, §16) and transport (packetisation, NIC behaviour) — each the weakest value any bound Provider declares, with `real` at the top of every aspect so that a Hardware Run has something to record. A Simulation Run's virtual root need not have a relation to UTC; transition `host_utc_nanos` values place its lifecycle transitions on the wall clock.

Normative: [design/05-module-api.md](../05-module-api.md), rules MA-41, MA-42; [design/04-run-and-session.md](../04-run-and-session.md), rules RS-41, RS-42.

Promotion readiness is judged per aspect. A Simulation Run with `rf: none` says nothing about RF behaviour, however many events it passed.

Simulation passing must never be reported as equivalent to verified hardware behavior.

---

# 15. Deterministic virtual time

Time semantics belong in Core.

Simulation implementation does not.

The Core should define abstractions such as:

```text
ClockDomain
TimePoint
Duration
RelativeBudget / AbsoluteDeadline
ClockRelation
```

A UHD Module may expose:

```text
ClockDomain = USRP device time
```

A simulation Module may expose:

```text
ClockDomain = deterministic virtual time
```

The same Processor/Reactor and ExperimentSpec semantics should operate over either.

Functional simulation may run faster than wall clock.

Real-time emulation may bind virtual time approximately 1:1 to wall time.

## Representation

Time is integer ticks, never floating-point seconds. A TimePoint is a ClockDomain id plus a signed 64-bit tick count, a Duration names its domain, and every rate is an exact reduced rational. A domain is a Root — one per timekeeper, with an epoch; `utc` and `host.monotonic` are reserved — or a Derived domain naming its root directly, with an exact ratio and an origin. Every stream's **SampleClock** is a Derived domain, so a tick is a sample index. Two domains with one root convert exactly, or report an inexact floor with its remainder; domains with different roots convert only through a ClockRelation, with uncertainty. There is no other path, and TimePoints of different domains do not compare. A sample-rate change is a `cold` update (§27) that ends the SampleClock and allocates a new one, so a consumer detects the change from the block's own domain id. The Manifest records every stream's SampleClock sequence and any relation from the Run's root to UTC; Simulation Runs may omit that relation because their virtual root need not map to wall time, while transition `host_utc_nanos` values place lifecycle transitions on the host clock.

Normative: [design/01-time-model.md](../01-time-model.md), rules TM-1…TM-12, TM-13a, TM-13b, TM-13c, TM-13d, TM-13e, TM-18, TM-19.

## Time Authority

Something must be the authority on "now" and on "wait until". One Time Authority per Run answers `now`, `wait_until` and `schedule` for the domains it declares: a primary root, that root's derived domains, `host.monotonic`, and, for the Simulation Engine, every root it simulates. In a Hardware or HIL Run it is the device timekeeper, which publishes a relation to host monotonic and to UTC, which the Manifest records ([design/05-module-api.md](../05-module-api.md), MA-29; [design/06-kernel-coordinator.md](../06-kernel-coordinator.md), KC-45). In a Simulation Run it is the **discrete-event Simulation Engine**, which advances virtual time — `host.monotonic` included — and delivers every waiting party its wake-up in order: Reactor timers, Processor deadlines, Peripheral latency models and client waits. The SimulationChannel needs no wake-up: a transmitter's content is known before its instant, so the receiving radio evaluates the channel, delayed paths included, when it publishes a block (CH-4, CH-9). In RealtimeEmulation the same Engine reads the real host clock and paces to it. `schedule` takes a callback at an instant of a governed domain that is at or after `now` and is a tick of that domain's root; callbacks fire in time order, ties in insertion order.

Normative: [design/01-time-model.md](../01-time-model.md), rules TM-16a…TM-17b; [design/05-module-api.md](../05-module-api.md), MA-29.

Three consequences:

- The Simulation Environment (§13) *is* a discrete-event engine, the **Simulation Engine**. MockRadio and MockPeripheral are models scheduled on it, and the SimulationChannel is the medium the radios read (§16); each target Provider reads its applicable entries from the `sim.faults` environment document (§17). Determinism with a seed follows from delivering events in virtual-time order, not from threads happening to agree.
- **No client API waits on wall-clock time for something that happens in runtime time.** Python has `run.wait_until(t)` and `run.wait_for(event)` ([design/06-kernel-coordinator.md](../06-kernel-coordinator.md), KC-29 and KC-29b); it does not have a device-time `sleep`. A `time.sleep(0.5)` in a script means nothing in a Run that simulates ten seconds in 0.3 seconds.
- Every stepped instance implements `step(until: TimePoint)`. The Kernel coordinator runs the stepping loop in a fixed order on one logical thread, and the Authority decides the instants; RealtimeEmulation, HIL and Hardware run Islands on real threads (§32; [design/05-module-api.md](../05-module-api.md), MA-20, MA-30).

---

# 16. SimulationChannel is separate from MockRadio

Radio hardware semantics and RF/channel propagation should be separate.

```text
MockRadio TX
      │
      ▼
SimulationChannel
      │
      ├── gain / path loss
      ├── delay
      ├── AWGN
      ├── multipath
      ├── CFO
      ├── Doppler
      ├── phase noise
      ├── PA nonlinearity
      ├── IQ imbalance
      ├── self-interference
      └── MIMO channel
      │
      ▼
MockRadio RX
```

The initial implementation may support only:

```text
gain
delay
AWGN
```

A loopback is not a separate model: it is a coupling whose two ends are one radio, the diagonal block below. Clipping is the radio's, not the channel's: MockRadio clips at the contract's full scale (§23). The architecture must allow richer models later without changing Radio semantics.

The channel is a coupling matrix over **all TX ports × all RX ports, including a device's own RX**. Self-interference is a diagonal-block entry, not a special case; this is what an IBFD graph (§46) is simulated against.

SimulationChannel configuration lives in the BindingProfile `environment` (§8), never in the ExperimentSpec.

Normative: [design/11-simulation-channel.md](../11-simulation-channel.md), rules CH-1…CH-11; [design/09-mock-radio.md](../09-mock-radio.md), MR-31…MR-36.

---

# 17. Fault injection is a first-class validation tool

Software execution should deliberately inject conditions such as:

```text
RX overflow
TX underflow
late command
sample gap
sequence error
alignment failure
clock drift
device disconnect
Processor deadline miss
Peripheral timeout
TUN/TAP queue overflow
plugin crash
```

Injected faults must produce the same typed RuntimeEvent, the same SampleBlock flags and the same timing consequence as the equivalent hardware fault. An injected RX overflow is zero samples, a restart gap and a `GAP_BEFORE` flag (§23, §13). Fault schedules live in the BindingProfile `environment` (§8) as a document read by each target Provider, not as a Module.

This is essential for AI-agent validation.

---

← [Part 02: ExperimentSpec, BindingProfile and Compilation](02-spec-binding-and-compilation.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 04: Execution Model and Data Contracts](04-execution-model-and-data-contracts.md) →
