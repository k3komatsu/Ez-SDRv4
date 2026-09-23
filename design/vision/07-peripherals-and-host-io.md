# Ez-SDR v4 — Architecture Vision · Part 07: Peripherals and Host I/O

> Sections §37–§41 of the Ez-SDR v4 Architecture Vision. Section numbers are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. Status, reading guide and revision history: [Ez-SDR_v4_ARCHITECTURE_VISION.md](../../Ez-SDR_v4_ARCHITECTURE_VISION.md).  
> ← [Part 06: Performance, Execution Islands and Radio Backends](06-performance-islands-and-radio-backends.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 08: Future Workloads and Targets](08-future-workloads-and-targets.md) →

---

# 37. External laboratory devices are first-class resources

Experiments may include:

- smart antennas,
- RF switches,
- attenuators,
- amplifiers,
- bias controllers,
- power meters,
- positioners,
- channel emulators,
- sensors,
- GPIO devices,
- custom microcontrollers,
- USB/serial devices,
- reference clock and time sources (OctoClock, GPSDO, PTP grandmasters), which report lock state and provide ClockRelation evidence (§24).

These are experiment resources, but vendor support must not live in Core.

Use a Peripheral Provider / Plugin architecture.

The Core sees:

```text
Peripheral
├── capabilities
├── parameters / commands
├── timing capabilities
├── state / health
└── event sources
```

---

# 38. Peripheral timing guarantees must be explicit

A USB smart-antenna controller and FPGA GPIO do not offer the same timing behavior.

Possible timing classes may include:

```text
best_effort_control
bounded_latency_control
hardware_triggered
hardware_timed
```

Experiment validation must not pretend that a best-effort USB command is sample-accurate.

The class is a property of a **Provider instance on a specific device**, not of a Provider kind. It is known when the instance is created and appears among the instance's declared capabilities, so `validate()` can match it; `prepare` may only narrow it ([design/05-module-api.md](../05-module-api.md), MA-17, MA-12). The same "USRP GPIO" provider is `hardware_timed` on an X3x0, whose GPIO bank sits behind the radio's timed command interface, and is not on an X4x0, where only ATR is hardware-controlled. MockPeripheral declares a class too; when it declares `best_effort_control` it emulates a latency distribution rather than acting instantly.

---

# 39. USRP GPIO must not create cross-module coupling

A logical peripheral should not depend directly on UHD just because one implementation uses USRP GPIO.

Prefer:

```text
Smart Antenna
      │
      │ generic GPIO capability
      ▼
  GPIO Resource
      │
      ├── UHD GPIO Provider
      ├── server GPIO Provider
      └── USB GPIO Provider
```

This preserves Mock substitution and experiment portability.

On a USRP the GPIO bank is a **sub-resource of the radio device** (§8): it shares the device's timekeeper, and whether it can be timed depends on the device generation. It is therefore provided by the same Module instance that provides the radio, and the Core resolves a peripheral's generic GPIO requirement to that sub-resource. Binding to a sub-resource of another Module's device is not a cross-module dependency; the peripheral still knows only the GPIO capability.

---

# 40. Host I/O is distinct from Peripheral I/O

Linux TUN/TAP is not a laboratory peripheral.

It is an operating-system I/O boundary.

Host-I/O Providers may include:

```text
Linux TUN
Linux TAP
UDP
PCAP
File
Shared Memory
future AF_XDP
future DPDK
```

The Core sees a generic HostEndpoint / NetworkEndpoint resource.

---

# 41. Linux TUN/TAP is an explicit target

An IEEE 802.11-like link should be able to appear as an ordinary Linux network interface.

```text
Linux TCP/IP
      │
      ▼
     TAP
      │ Packet<EthernetFrame>
      ▼
 MAC Reactor
      │ Packet<MPDU>
      ▼
 PHY Processor
      │ SampleStream<IQ>
      ▼
 Radio
```

This enables ordinary tools such as:

```text
ping
iperf
TCP
UDP
IPv4
IPv6
```

over the SDR link.

TUN/TAP-specific privileges should be isolated in a small host-I/O helper rather than granting unnecessary privileges to the main Runtime.

## The helper is a one-shot setup, not a resident proxy

Linux requires `CAP_NET_ADMIN` to *create* a network device or to attach to one the caller does not own; attaching to an existing persistent device that the caller owns needs no capability. The helper therefore creates a persistent TAP, sets its owner and group, brings it up, assigns addresses, and exits. The Runtime then opens `/dev/net/tun` unprivileged and attaches to that device, with `IFF_MULTI_QUEUE` when several queues are wanted. No packet ever crosses an IPC boundary through a privileged process.

Kernel-side drops (a full transmit queue) are read from the interface statistics and surfaced as `TUN_QUEUE_DROP` events (§29). Packet timestamps are host-clock timestamps; relating them to device time is a `ClockRelation` (§24).

---

← [Part 06: Performance, Execution Islands and Radio Backends](06-performance-islands-and-radio-backends.md) · [Index](../../Ez-SDR_v4_ARCHITECTURE_VISION.md) · [Part 08: Future Workloads and Targets](08-future-workloads-and-targets.md) →
