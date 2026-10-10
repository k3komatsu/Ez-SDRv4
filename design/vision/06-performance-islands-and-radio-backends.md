# Ez-SDR v4 — Architecture Vision · Part 06: Performance, Execution Islands and Radio Backends

> Sections §31–§36 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 05: Coherence, Calibration and Runtime Semantics](05-coherence-calibration-and-runtime-semantics.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 07: Peripherals and Host I/O](07-peripherals-and-host-io.md) →

---

# 31. Memory placement matters as much as compute placement

Future execution may use:

```text
Host RAM
Pinned host RAM
Huge pages
WASM linear memory
GPU memory
RFNoC / FPGA memory
Remote / RDMA memory
```

The kinds above are not a Core list. Core holds a node-qualified `MemoryDomain` identity and compares identities; which kinds exist, and what reaching one costs, is Vocabulary content that Executors, Links and Sinks declare. The Core should model enough information to validate an explicitly stated placement:

```text
MemoryDomain
DataLink
BufferContract
TransferRequirement
```

Admission must validate:

```text
compute placement
+
memory placement
+
transfer reachability
```

A GPU Processor that requires repeated host/device copies may be worse than a CPU implementation. The Core neither measures nor uses that cost, and it does not move the Processor for you: the placement is the one the BindingProfile states (§20).

The Core must not hard-code one ring-buffer implementation. Module contracts exchange `BufferRef` handles tagged with a `MemoryDomain` (§23), never raw host slices, so that a ring, a pinned-memory pool, a GPU buffer or WASM linear memory can back the same contract.

---

# 32. Real-time execution should use bounded Execution Islands

General dataflow scheduling is difficult.

Ez-SDR should not casually become a universal dynamic scheduler.

A useful direction is an `ExecutionIsland` concept.

Examples:

```text
RT Island
  detector → sync → decoder
  CPU affinity
  RT policy
  deadline budget

GPU Island
  OTFS equalizer
```

Device-side DDC/DUC/FIR chains are not an Island: they are Radio Model capabilities configured by the Provider (§20, §35).

A Module should be able to declare execution characteristics such as:

```text
expected / maximum processing time
deadline
```

Batch size, parallelism and statefulness join that list when an Executor reads them: a declared field that nothing reads would take its meaning from documents written before it had one (invariant 39).

The Runtime performs admission checks before RUN (§10).

The exact scheduling algorithm is not fixed by this Vision.

## Driving model per ExecutionClass

An Island is declared in the BindingProfile — an Executor binding, the components placed on it, optional affinity and real-time policy — and admission checks it without creating, merging or moving one. Every stepped instance implements `step(until)`, which consumes and emits everything at or before `until`, nothing after, and never blocks. The Kernel coordinator owns the loop on one logical thread: the Authority's `next_wakeup` sets the instant, and `step` runs over every stepped instance in a fixed order (Providers, then Executors, then Sinks, each by instance id) until none progresses; the Simulation Engine decides the instants, and the fixed order keeps determinism independent of assembly order. In the Simulation class every stepped Provider, Executor and Sink is stepped, and each deterministic Link operation applies its declared drop-class policy in virtual time (Phase 2); in RealtimeEmulation stepped Providers are wall-paced while Executors and Sinks run on threads; in HardwareInLoop and Hardware, Islands run on real threads with their declared affinity and RT policy. Without this, "deterministic runs reproduce with a seed" (§58) is a wish: threads and lossy Probe links make event order a coin toss.

Normative: [design/05-module-api.md](../05-module-api.md), rules MA-20, MA-22, MA-30, MA-38…MA-40; [design/06-kernel-coordinator.md](../06-kernel-coordinator.md), KC-46 (the data thread of the device-paced classes).

Within an Island the Executor schedules; between Islands only DataLinks and Event/Action queues exist, and cycles are allowed only through the latter (§19).

---

# 33. Throughput and latency are separate optimization objectives

The Runtime must not assume that maximum throughput and minimum latency use the same settings.

Profiles may include:

```text
auto
balanced
throughput
latency
custom
```

Examples:

- high-rate recording may use larger buffers,
- reactive packet radio may prefer low-latency bounded buffers.

Expert overrides remain possible.

Profiles are hints carried in the BindingProfile and interpreted by Providers and Executors (buffer depths, batch sizes, thread policy); they are not Kernel semantics.

---

# 34. Performance must be described as an envelope

Do not describe hardware simply as:

```text
supports 200 Msps
```

Actual performance depends on:

```text
channel count
RX/TX directions
sample format
sample rate
transport topology
NIC configuration
processing graph
host architecture
```

The architecture should allow a Provider to expose a `PerformanceEnvelope` rather than a single headline rate.

The over-the-wire sample format (`sc16`, `sc8`, `sc12`) is one dimension of that envelope and stays inside the Provider; it is not the DataContract seen by the graph (§21). MockRadio enforces the PerformanceEnvelope of the profile it emulates (§13); a Mock that streams faster than its emulated transport is as misleading as one that ignores lead times.

---

# 35. Radio backend architecture

The Radio model should be implementation-independent.

Providers may include:

```text
Mock
Native UHD
SoapySDR
```

## UHD

USRP support should use a narrow native C++ bridge to UHD unless a mature Rust UHD interface becomes clearly superior. UHD's own C API (`uhd.h`) is such a bridge: every entry point catches the exceptions UHD throws to it and returns a status, and only POD types and opaque handles cross it — though an exception thrown out of one of UHD's own destructors still ends the process, which the Provider avoids by never freeing a device it has lost.

The bridge should:

- expose typed POD-like data,
- translate exceptions into explicit status,
- prevent STL/C++ implementation types from crossing ABI boundaries,
- convert UHD metadata and asynchronous errors into typed RuntimeEvents,
- hide most RFNoC/Replay/device-specific details behind Provider planning,
- implement Radio Model capabilities such as `tx.repeat` (Replay block, with DRAM size and word-alignment constraints), `rx.decimation` / `tx.interpolation` (DDC/DUC) and device-side FFT through RFNoC graphs, exposing them as capabilities with constraints rather than as Processor placement targets (§20).

Expert UHD extensions remain available when needed. Custom RFNoC blocks are reached through `extensions.uhd.rfnoc.*`.

The UHD Provider runs **in-process** in v4.0: the sample path must not cross an IPC boundary. UHD can throw or abort; the bridge translates every exception into a typed status, and a device that disappears becomes a `DEVICE_LOST` event that the Run's Policy table maps to `abort` with full cleanup (§53). v3 handled the same situation by restarting the whole server behind a `--retry` flag; v4 makes it a Kernel policy. An out-of-process (remote) Radio Provider that implements the same Radio contract over shared memory remains possible later, because buffers are handles (§23), not slices (§62).

Normative: [design/18-uhd-radio.md](../18-uhd-radio.md).

## SoapySDR

SoapySDR may provide broad support for devices where extreme UHD-specific timing/performance is not required.

---

# 36. Device profiles and quirks remain Provider concerns

Physical SDRs inevitably have:

- sample-rate constraints,
- channel-count constraints,
- transport limitations,
- firmware dependencies,
- FPGA image dependencies,
- LO behavior,
- stream startup quirks,
- device-specific tuning requirements.

These belong in Provider-specific profiles.

The Core consumes generic outputs such as:

```text
Capability
Constraint
TimingEnvelope
PerformanceEnvelope
Warning
RuntimeEvent
```

The exact Provider profile/version used should be recorded in the Run.

---

← [Part 05: Coherence, Calibration and Runtime Semantics](05-coherence-calibration-and-runtime-semantics.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 07: Peripherals and Host I/O](07-peripherals-and-host-io.md) →
