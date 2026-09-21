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
├── SimulationChannel
├── SimulationEngine (discrete-event Time Authority; virtual clock, §15)
├── MockPeripheral
├── MockNetworkEndpoint
└── FaultInjector
```

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

Therefore the Radio Model requires every Provider to publish, next to its PerformanceEnvelope (§34):

```text
TimingEnvelope
├── min_timed_command_lead
├── startup_latency_max
├── stop_tail_max
├── timed_command_queue_depth
├── overflow_restart_gap
└── coercion rules        sample-rate grid, gain step, frequency resolution
```

and the following are rules, not options:

1. **MockRadio enforces the envelope of the profile it emulates** (`x310-like`): a timed command issued with insufficient lead produces the same typed `LATE_COMMAND` / `TIME_ERROR` event as the hardware; requested rates are coerced on the same grid and reported in the PrepareReport (§11); `stop` delivers the same tail; an injected overflow reproduces the same restart gap and block flags (§23); and a request that exceeds the profile's PerformanceEnvelope (§34), channel count × rate × format beyond the emulated transport, is rejected at `validate()` / `prepare()`, so a four-channel 200 Msps request fails on a 1GbE-like profile in simulation as it would on the bench.
2. **`validate()` checks reactive timing against the bound Provider's envelope**, for example that a Reactor's turnaround is not shorter than `min_timed_command_lead`. It is the same code path for Mock and hardware.
3. **RF behaviour is not part of the envelope.** It stays in SimulationChannel (§16) and is reported as unmodelled unless a model is bound (§14).

Profiles are versioned. An `x310-like` profile carries a version that the Manifest records in its `mock.*` section, and its envelope values come from measurement on the hardware, not from data sheets (§59). An `ideal` profile without constraints may exist for algorithm work; a Run on it is recorded with `timing: none` and `coercion: none` and is not evidence for promotion.

The first MockRadio implementation must enforce the envelope. It is a few timestamp comparisons, and it is the difference between a validation tool and a source of false confidence.

---

# 14. Simulation execution classes

Every Run must record what kind of evidence it represents.

At minimum:

```text
ExecutionClass
├── Simulation
├── RealtimeEmulation
├── HardwareInLoop
└── Hardware
```

An ExecutionClass is defined along two axes: which Time Authority drives the Run (a device timekeeper, or the discrete-event Simulation Engine; §15), and what the RF path is (over-the-air, cabled, or simulated). HardwareInLoop is a device timekeeper with a cabled or partially simulated RF path.

Determinism is a property of the **Simulation** class only. RealtimeEmulation trades it for real deadlines: the Simulation Engine paces the environment models to wall clock while Islands run on threads, so event order is not reproducible and deadline misses are real. HardwareInLoop and Hardware are never deterministic.

A single fidelity label cannot express "timing is modelled, RF is not". Every Run therefore records a **fidelity vector**, one entry per aspect:

```text
Fidelity vector
├── timing        none | envelope | hardware_quirk     lead times, start-up, stop tail, queue depth
├── continuity    none | envelope | hardware_quirk     overflow gaps, restart, sequence errors
├── coercion      none | grid                          rate / gain / frequency grids
├── rf            none | impairment_model              SimulationChannel models (§16)
└── transport     none | model                         packetisation, NIC behaviour
```

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
Deadline
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

Time is integer ticks, never floating-point seconds:

```text
ClockDomain { id, tick_rate: Rational { num, den }, epoch: EpochRef }
TimePoint   { domain: ClockDomainId, ticks: signed 64-bit integer }
Duration    { ticks: signed 64-bit integer, in a named domain }
```

Each stream has a **SampleClock**: a ClockDomain derived from its device clock by an exact rational (for example 200 MHz / 10). Conversions between domains related by exact rationals are exact. Conversions between unrelated domains go through a `ClockRelation` and yield a value with uncertainty. There is no other path.

A sample-rate change is a `cold` update (§27) that **ends the stream's SampleClock and starts a new ClockDomain** with a new id, related to the device clock by the new rational. Blocks carry their domain id, so a consumer detects the change from the block itself; TimePoints of different SampleClocks are never compared directly. The Manifest records the sequence of SampleClocks of every stream with their start times.

The epoch of a device ClockDomain is arbitrary; it is set at PPS synchronisation. Every Run records the relation `epoch ↔ UTC` with its uncertainty in the Manifest. Without it, correlation with cameras, positioners or other hosts (§24, §47) is impossible.

## Time Authority

Something must be the authority on "now" and on "wait until". In a hardware Run it is the device timekeeper, related to the host monotonic clock. In a Simulation Run it is the **discrete-event Simulation Engine**, which advances virtual time and delivers every waiting party its wake-up in order: Reactor timers, Processor deadlines, Peripheral latency models, SimulationChannel delays, and client waits.

```text
TimeAuthority
├── now(domain)              -> TimePoint
├── wait_until(TimePoint)
└── schedule(TimePoint, Action)

Hardware / HIL run:     device timekeeper (+ ClockRelation to host monotonic)
Simulation run:         discrete-event Simulation Engine, runs faster than wall clock
RealtimeEmulation run:  the same engine, paced to wall clock
```

Three consequences:

- The Simulation Environment (§13) *is* a discrete-event engine, the **Simulation Engine**. MockRadio, SimulationChannel, MockPeripheral and FaultInjector are models scheduled on it. Determinism with a seed follows from delivering events in virtual-time order, not from threads happening to agree.
- **No client API waits on wall-clock time for something that happens in runtime time.** Python has `run.wait_until(t)` and `run.wait_for(event)`; it does not have a device-time `sleep`. A `time.sleep(0.5)` in a script means nothing in a Run that simulates ten seconds in 0.3 seconds.
- Executors implement `step(until: TimePoint)`. In the Simulation class the Simulation Engine step-drives every Island on one logical thread (or behind a deterministic barrier) and drop policies are decided in virtual time; RealtimeEmulation, HIL and Hardware run Islands on real threads (§32).

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
      ├── clipping
      ├── self-interference
      └── MIMO channel
      │
      ▼
MockRadio RX
```

The initial implementation may support only:

```text
loopback
gain
delay
AWGN
```

The architecture must allow richer models later without changing Radio semantics.

The channel is a coupling matrix over **all TX ports × all RX ports, including a device's own RX**. Self-interference is a diagonal-block entry, not a special case; this is what an IBFD graph (§46) is simulated against.

SimulationChannel configuration lives in the BindingProfile `environment` (§8), never in the ExperimentSpec.

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

Injected faults must produce the same typed RuntimeEvent, the same SampleBlock flags and the same timing consequence as the equivalent hardware fault. An injected RX overflow is zero samples, a restart gap and a `GAP_BEFORE` flag (§23, §13). Fault schedules live in the BindingProfile `environment` (§8).

This is essential for AI-agent validation.

---

← [Part 02: ExperimentSpec, BindingProfile and Compilation](02-spec-binding-and-compilation.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 04: Execution Model and Data Contracts](04-execution-model-and-data-contracts.md) →
