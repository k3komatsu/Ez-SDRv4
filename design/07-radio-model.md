# Phase 2 spec 07 — The Radio Model Vocabulary (`radio` 1.3.0)

| Field | Value |
|---|---|
| Status | Accepted at Gate P (owner, 2026-09-24) and ratified at Gate X (owner, 2026-09-25; [`plan/phase2/00-overview.md`](../plan/phase2/00-overview.md) §11). Normative for `crates/ezsdr-radio` and for every radio Provider (MockRadio in Phase 2, UHD in Phase 7). Amended in Phase 3 by VB-1 and VB-10. Amended in Phase 4 by VC-1 ([`plan/phase4/13-amendments.md`](../plan/phase4/13-amendments.md)). Amended in Phase 7 by VE-1 (RM-1, RM-26), VE-2 (RM-6, RM-14, RM-15, RM-16, RM-25) and VE-3 (RM-10, RM-11, RM-20, RM-22; `radio` 1.3.0; [`plan/phase7/19-amendments.md`](../plan/phase7/19-amendments.md)). |
| Scope | The `radio` Vocabulary: the resource kinds and tree shape a radio Provider exposes; the configuration keys and the capability keys, each with its `KeyDecl`; the TimingEnvelope and PerformanceEnvelope as capabilities; coercion rules; the event kinds with their defaults, severities and payloads; the Session verbs; the `TxBurst` conventions; the `radio.rf_envelope` section and its admission check. |
| Not in scope | Any Provider's values (MockRadio's are spec 09's MR-3; UHD's are Phase 7's). Channel models (Phase 3). Per-channel configuration (RM-5's ceiling). Receive-side DDC and transmit-side DUC capabilities, device FFT and RFNoC (Vision §20, §35; Phase 7+). |
| Crate | `crates/ezsdr-radio`, library `ezsdr_radio`. Depends on `ezsdr-kernel`, `serde`, `serde_json`, `schemars`. |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

---

## 1. Purpose

Vision §5 puts "channels, streams, RF parameters, TimingEnvelope, PerformanceEnvelope, coherence basis, repeat / decimation capabilities" in the Radio Model, a Vocabulary: versioned on its own, additive, and never read by the Kernel by name (OV-21). This document is that Vocabulary's first version. Every radio Provider — MockRadio now, the UHD Provider in Phase 7 — implements the same keys with the same meaning, which is what makes Vision §59's "change only the BindingProfile" testable in Phase 8.

## 2. Evidence

- **UHD coerces rate, gain and frequency** to a device grid and documents reading them back (audit Appendix B; spec 03 §2). Hence RM-8's coercion rules and the per-key coercion defaults Vision §11 names: `reject` for sample rate and frequency, `warn` for gain.
- **UHD's overflow**: zero samples returned, `out_of_sequence` distinguishing a host-side sequence error, a continuous stream restarting after `OVERRUN_RESTART_DELAY` = 0.05 s (audit Finding 3, VERIFIED). Hence RM-17 and RM-18.
- **UHD resets the CORDICs on every start of burst and needs a time spec on each** (spec 02 §2). Hence RM-13's one waveform per burst and the Kernel's `BurstTracker` (SC-29).
- **A late timed command** on UHD yields `ERROR_CODE_LATE_COMMAND` on receive and `EVENT_CODE_TIME_ERROR` on transmit (audit §10's UHD row). Spec 04 §9 issue 2 separates the two: `TIME_ERROR` for a burst whose target has passed, `LATE_COMMAND` for a control command issued too late. Hence RM-10.
- **Retune phase** on UBX/CBX/SBX is random unless the retune is timed (audit Finding 15, VERIFIED against UHD's page_sync). Hence RM-9's declared behaviour.

## 3. Model

```text
Provider instance tree (one per radio instance)
└── <device>                kind radio.device       the node a Spec binds (Y5)
    │   ports:  rx  out  ezsdr.stream.cf32          (tx  in  — reserved, not declared in 1.0.0)
    │   capabilities: configuration ranges, envelope values, coherence and repeat constraints
    ├── <device>/rx          kind radio.rx_stream    stream id: SampleClock owner, event source
    └── <device>/tx          kind radio.tx_stream    stream id: SampleClock owner, TxBurst target, event source

configuration  (PrepareReport.effective; per fragment, KA-4)
  radio.{rx,tx}.channels  radio.{rx,tx}.sample_rate_hz  radio.{rx,tx}.frequency_hz
  radio.{rx,tx}.gain_db    radio.{rx,tx}.antenna

envelope (capabilities of <device>; matchable, never read by the Kernel except min_command_lead)
  radio.timing.*   radio.perf.*
```

## 4. Rules

### Identity and registration

- **RM-1** The Vocabulary is `VocabularyDescriptor { id: radio, version: 1.3.0, prefix: radio, keys: RM-4's table, event_kinds: RM-10's table, verbs: RM-12's table, checks: [radio.rf_envelope] }`, returned by `ezsdr_radio::vocabulary()`. `ezsdr_radio::register(registry, checks, kinds)` registers the descriptor, the `RfEnvelopeCheck` of RM-19 and every event kind with owner `Some(radio)`, in that order, and is the only way a runtime assembles the Vocabulary. Version 1.1.0 adds the two path-delay capabilities of RM-4 and RM-23 (Phase 3, VB-1); version 1.2.0 adds RM-24's hot-path form of `RX_OVERFLOW` (Phase 4, VC-1); version 1.3.0 adds RM-25, RM-26, the device lead of RM-14, and RM-22's three new payload types and `TIME_ERROR` outcome `late_at_device` (Phase 7, VE-1…VE-3). *Checked by `rm_01_register_adds_the_descriptor_the_check_and_the_kinds`.*

### Resource tree

- **RM-2** A radio Provider instance's tree has one node of kind `radio.device` per radio device (in Phase 2 the root), and beneath each exactly two children: `<device>/rx` of kind `radio.rx_stream` and `<device>/tx` of kind `radio.tx_stream`, neither with capabilities nor ports. A Spec resource that uses a radio is of kind `radio.device`. The device node declares the port `rx` (direction `out`, contract `ezsdr.stream.cf32`) when the Provider can receive; its blocks are in the host memory domain `MemoryDomainId::local(0)` (HD-1's `HOST_MEMORY`) in version 1.0.0, which is the "Phase 2 value" MA-10 leaves to the Vocabulary; the port name `tx` (direction `in`) is reserved for link-fed transmission and is not declared in version 1.0.0. No node of a radio tree is `shareable`. *Producer obligation; MockRadio's test is `mr_02_the_tree_has_the_radio_model_shape`.*
- **RM-3** The receive stream of a device delivers blocks whose `channels` is the effective `radio.rx.channels` and whose channel `c` is device receive channel `c`; the transmit stream likewise for `radio.tx.channels`. A SampleClock of a stream is declared with `stream` = the stream node's id (`<device>/rx`, `<device>/tx`) (TM-13a, KA-2). A count of 0 means the stream does not exist: no SampleClock, no block, and — for transmit — every `TxBurst` refused (KC-24 finds no clock). *Producer obligation.*

### Keys

- **RM-4** The keys are exactly these, declared with these `KeyDecl`s:

  | Key | kind | coercible | coercion default | update class | Role |
  |---|---|---|---|---|---|
  | `radio.rx.channels` | int | no | reject | cold | configuration: receive stream channel count, ≥ 0 |
  | `radio.tx.channels` | int | no | reject | cold | configuration: transmit stream channel count, ≥ 0 |
  | `radio.rx.sample_rate_hz` | num | yes | reject | cold | configuration |
  | `radio.tx.sample_rate_hz` | num | yes | reject | cold | configuration |
  | `radio.rx.frequency_hz` | num | yes | reject | hardware_timed | configuration: RF centre frequency |
  | `radio.tx.frequency_hz` | num | yes | reject | hardware_timed | configuration |
  | `radio.rx.gain_db` | num | yes | warn | hardware_timed | configuration |
  | `radio.tx.gain_db` | num | yes | warn | hardware_timed | configuration |
  | `radio.rx.antenna` | str | no | reject | — | configuration: antenna port name |
  | `radio.tx.antenna` | str | no | reject | — | configuration |
  | `radio.rx.frequency_step_hz` | num | no | reject | — | capability: tuning resolution |
  | `radio.tx.frequency_step_hz` | num | no | reject | — | capability |
  | `radio.rx.gain_step_db` | num | no | reject | — | capability |
  | `radio.tx.gain_step_db` | num | no | reject | — | capability |
  | `radio.rx.coherent` | bool | no | reject | — | capability: the receive channels are one coherent group (RM-9) |
  | `radio.full_duplex` | bool | no | reject | — | capability |
  | `radio.hardware_time` | bool | no | reject | — | capability: timed commands are executed by the device |
  | `radio.phase_behavior_on_retune` | str | no | reject | — | capability: `deterministic` or `random_unless_timed_tune` (RM-9) |
  | `radio.tx.repeat_max_samples` | int | no | reject | — | capability: longest repeated waveform |
  | `radio.tx.repeat_align_samples` | int | no | reject | — | capability: a repeated waveform's length is a multiple of this |
  | `radio.rx.block_len` | int | no | reject | — | capability: the nominal samples per receive block (informational; SC-15 still holds) |
  | `radio.timing.min_timed_command_lead_ns` | int | no | reject | — | TimingEnvelope (RM-6) |
  | `radio.timing.startup_latency_ns` | int | no | reject | — | TimingEnvelope |
  | `radio.timing.stop_tail_ns` | int | no | reject | — | TimingEnvelope |
  | `radio.timing.command_queue_depth` | int | no | reject | — | TimingEnvelope |
  | `radio.timing.overflow_restart_gap_ns` | int | no | reject | — | TimingEnvelope |
  | `radio.perf.rx_bytes_per_s` | int | no | reject | — | PerformanceEnvelope (RM-7) |
  | `radio.perf.tx_bytes_per_s` | int | no | reject | — | PerformanceEnvelope |
  | `radio.perf.wire_bytes_per_sample` | int | no | reject | — | PerformanceEnvelope: the host–device wire format's bytes per complex sample (SC-5) |
  | `radio.tx.path_delay_samples` | int | no | reject | — | capability: transmit path delay (RM-23) |
  | `radio.rx.path_delay_samples` | int | no | reject | — | capability: receive path delay (RM-23) |

  No other `radio.` key exists in 1.1.0; version 1.0.0 declared the first twenty-nine rows, and a later key is an additive minor version (Phase 3, VB-1). *Checked by `rm_04_the_key_table_is_exactly_the_declared_one`.*
- **RM-5** A radio Provider's `PrepareReport.effective` holds exactly the ten configuration keys of RM-4 for its fragment, with the applied values, whether or not the Spec constrained them; an unconstrained key takes the Provider's default. The capability keys are never in `effective`. All channels of a stream share the stream's frequency, gain and antenna. *Ceiling: per-channel values are an additive key later, for example `radio.rx.gain_db_per_channel` as a list-valued parameter.* *Producer obligation; MockRadio's test is `mr_08_effective_holds_exactly_the_ten_configuration_keys`.*

### Envelopes

- **RM-6** The **TimingEnvelope** is the five `radio.timing.*` capabilities, each declared on the device node as `One(Int(v))`, in nanoseconds or, for the queue depth, commands. A radio Provider sets `ProviderInstance.min_command_lead` to `Some(Duration(host.monotonic, radio.timing.min_timed_command_lead_ns))` when that value is positive and to `None` when it is 0 (KA-7: absent means zero). A Spec expresses a timing requirement as a constraint on these keys — a Reactor whose turnaround is 3 ms writes `radio.timing.min_timed_command_lead_ns: { kind: max, value: 3000000 }` — and the generic matcher refuses a device that cannot meet it at `validate()` (Vision §13 rule 2, audit Finding 4). The value of `radio.timing.min_timed_command_lead_ns` is the lead the Provider needs from the coordinator's dispatch (MA-10), its delivery allowance included, so that a Spec's timing constraint is compared with what the Provider can promise end to end (Phase 7, VE-2). *Producer obligation for the values; the matching is SB-6's.*
- **RM-7** The **PerformanceEnvelope** is the three `radio.perf.*` capabilities. For each direction `d ∈ {rx, tx}`, a configuration is admissible only if `radio.d.channels × radio.d.sample_rate_hz × radio.perf.wire_bytes_per_sample ≤ radio.perf.d_bytes_per_s`. A radio Provider refuses an inadmissible configuration in `coerce` with a `rejected` entry naming `radio.d.sample_rate_hz` when that key is in the request and `radio.d.channels` otherwise (KA-5), in `prepare` with `Rejected`, and, when an `UpdateParameter` it did not refuse at admission takes effect, with `radio.COMMAND_REJECTED` (RM-10; MR-18 checks at that instant, against the configuration then). The Kernel's runtime coerce (KA-6) makes the first of these the one a Session change meets. *Producer obligation; MockRadio's tests are MR-6's and `mr_18_a_scheduled_pair_is_checked_when_it_applies`.*
- **RM-8** **Coercion.** A numeric configuration key is coerced to its declared grid: the sample rates the device node declares as `AnyOf`, the frequency to a multiple of `radio.d.frequency_step_hz` within the declared `Range`, the gain to a multiple of `radio.d.gain_step_db` within the declared `Range`. For an `Eq(v)` constraint the applied value is the grid value nearest `v`, the lower of two equally near; a `Coercion { key, requested: v, applied, reason }` is recorded exactly when `applied ≠ v` numerically (SB-6). A value outside the range is not coerced into it: the key is `rejected`. For `Min`, `Max`, `Range` and `Set` constraints the applied value is, respectively, the smallest grid value at or above the bound, the largest at or below, the smallest inside the range, and the first member of the set on the grid; none existing is a rejection, and none of these is a coercion. `Present` takes the default. *Producer obligation; MockRadio's tests are MR-6's.*

### Coherence

- **RM-9** `radio.rx.coherent: One(true)` declares that the receive channels of the device's one stream are sample-aligned with a known phase relation (Vision §25's CoherentGroup, owned by the Provider; the Kernel never infers it, SB-35). `radio.phase_behavior_on_retune` declares whether that phase relation survives an untimed retune (`deterministic`) or only a `hardware_timed` one (`random_unless_timed_tune`, Vision §26). *Producer obligation; MockRadio emulates both behaviours on the SimulationChannel (MR-34) (Phase 3, VB-1).*

### Events

- **RM-10** The event kinds are exactly these, registered with owner `radio` (RS-27):

  | Kind | Severity | Default reaction | Emitted when |
  |---|---|---|---|
  | `radio.RX_OVERFLOW` | warning | mark_artifact | a receive overrun or sequence error lost samples (RM-17, RM-18) |
  | `radio.TX_UNDERFLOW` | warning | mark_artifact | a transmit stream ran out of samples inside a burst (SC-25) |
  | `radio.TX_DISCONTINUITY` | warning | mark_artifact | the burst tracker reported a discontinuity (SC-24a) |
  | `radio.TIME_ERROR` | error | mark_artifact | a burst's target was too close or past (RM-14), or the device model saw an unclosed burst followed by a timed block (spec 02 decision S12) |
  | `radio.LATE_COMMAND` | warning | mark_artifact | a timed command or a stream start arrived with too little lead (UC-6, MR-11) |
  | `radio.ALIGNMENT_ERROR` | error | mark_artifact | channels of one stream lost alignment (SC-31a) |
  | `radio.CLOCK_LOST` | fatal | abort | the device lost its reference |
  | `radio.COMMAND_QUEUE_FULL` | error | abort | a timed command found the device's queue full (UC-6); on the X3x0 this needs a full restart (audit Finding 4) |
  | `radio.COMMAND_REJECTED` | error | mark_artifact | the Provider could not carry out an admitted Action: an unsupported Action, a malformed waveform, a target it does not have, a configuration its envelope refuses |

  `radio.TX_UNDERFLOW`, `radio.TX_DISCONTINUITY`, `radio.ALIGNMENT_ERROR` and `radio.CLOCK_LOST` are declared so that a Spec's `policies.failure` may name them (SB-18); MockRadio never emits them; the UHD Provider emits all but `TX_DISCONTINUITY` (spec 18 UR-19, UR-27, UR-28) (Phase 7, VE-3). *Checked by `rm_10_the_kinds_are_registered_under_radio_with_their_defaults`.*
- **RM-11** Every radio event is emitted with `emit_control`, except `radio.RX_OVERFLOW`, which every radio Provider must emit on the hot path (`EventSink::emit`, RS-32) with RM-24's bytes, because a device reports it from its sample path (Phase 4, VC-1); the source is the stream node for a stream event (`RX_OVERFLOW`, `TIME_ERROR`) and the device node otherwise (`LATE_COMMAND` for a start or a `hardware_timed` update, `COMMAND_QUEUE_FULL`, `COMMAND_REJECTED`), `time` in the stream's SampleClock or, before one exists, the Authority's primary root, and a payload that is, on the control path, a JSON object with exactly these members:

  | Kind | Payload |
  |---|---|
  | `radio.RX_OVERFLOW` | `{ "cause": "overrun" \| "sequence", "lost": int, "restart_gap_ns": int }` (`restart_gap_ns` 0 for `sequence`); on the hot path, RM-24's bytes |
  | `radio.TIME_ERROR` | `{ "cause": "late" \| "unclosed_burst", "outcome": "send_asap" \| "drop" \| "plan_violation" \| "refused" \| "late_at_device", "late_by_ns": int, "target": TimePoint }` |
  | `radio.LATE_COMMAND` | `{ "key": string \| null, "requested": TimePoint, "applied": TimePoint }` |
  | `radio.COMMAND_QUEUE_FULL` | `{ "key": string, "depth": int }` |
  | `radio.COMMAND_REJECTED` | `{ "action": string, "reason": string }` (`action` is the Action's `kind` tag) |
  | `radio.TX_UNDERFLOW` | `{ "cause": "starved" \| "lost" }`: `starved`, the device ran out of samples inside a burst because the host was late; `lost`, samples of a burst were lost between host and device |
  | `radio.ALIGNMENT_ERROR` | `{ "lost": int }`: the samples, on every channel of the stream, that the misalignment removed, which is the next block's time jump |
  | `radio.CLOCK_LOST` | `{ "reference": "frequency" }`: the frequency reference (10 MHz) was lost; no Phase 7 Provider monitors its time reference (PPS), so no other value is declared (Review J, P2-11) |

  `TIME_ERROR`'s outcome `late_at_device` means that the Provider handed the burst to the device in time by its own clock and the device reported that it arrived too late, so nothing was transmitted; `late_by_ns` is the device's reported instant less the target when the device reports one, and 0 otherwise. Its source is the transmit stream (Phase 7, VE-3).

  Each payload is the JSON serialisation of the RM-22 type of its kind, never a hand-built object. *Producer obligation; MockRadio's tests check each payload.*

### Verbs and bursts

- **RM-12** The Session verbs are: `start_repeat` → `CompileRule::TxBurst { repeat: true, late_policy: send_asap_and_flag }`; `send` → `CompileRule::TxBurst { repeat: false, late_policy: drop_and_flag }`. Their target is the transmit stream, `<resource>/tx` (RM-13). *Checked by `rm_12_the_verbs_compile_to_bursts`.*
- **RM-13** A `TxBurst`'s target is a transmit stream node (`<device>/tx`). Its waveform `ArtifactRef` holds complex samples in the transmit stream's contract — `ezsdr.stream.cf32` in 1.0.0 — channel-interleaved (sample `n` of channel `c` at byte offset `(n · channels + c) · 8`), little-endian, with `channels` = the effective `radio.tx.channels`. Its length in samples is `size_bytes / (8 · channels)`, which must be a whole number ≥ 1. The burst transmits on every channel of the stream. A repeated waveform's length must be at most `radio.tx.repeat_max_samples` and a multiple of `radio.tx.repeat_align_samples`. `TxBurst.metadata` is unused in 1.0.0 and must be empty; a burst that breaks any of these is not transmitted and is reported with `radio.COMMAND_REJECTED`. A Provider that reads the waveform's bytes refuses one with a non-finite component the same way (Phase 3, VB-1). *Producer obligation.*
- **RM-14** On receipt of a `TxBurst` a radio Provider evaluates `LatePolicy::decide(registry, at, now, device_lead)` with `now` its current instant in the transmit SampleClock rounded up to a sample — so that a target whose instant has passed is late even when it is the sample the current instant falls in (Phase 3, VB-1) — and `device_lead` its **device lead**: its TimingEnvelope's `radio.timing.min_timed_command_lead_ns` less the delivery allowance it declares (MA-10, KG-8), in `host.monotonic` (SC-27). A stepped Provider's allowance is zero, so for MockRadio the device lead is the TimingEnvelope's lead. `OnTime`: the burst starts at `at`. `SendAsap`: the target moves to the first transmit sample at or after `now + device_lead`; if admitted, `radio.TIME_ERROR { cause: late, outcome: send_asap }` is emitted (Phase 7, VE-2). If MR-16 refuses that moved start, the burst is not transmitted and the Provider emits only `TIME_ERROR { outcome: refused }` plus `COMMAND_REJECTED` instead of a `send_asap` event. `Drop` and `PlanViolation`: the burst is not transmitted, and `radio.TIME_ERROR` with that outcome is emitted. The outcome reaches an admitted burst's `BurstRecord` through `BurstOpen.late` (SC-29a). *Producer obligation.*
- **RM-15** Bursts on one transmit stream are held in target order. A burst ends at the end of its waveform (without `repeat`), at the start of the next burst, at a `Stop` for its stream or device, at a `cold` change of the stream (UC-3), or at the Run's stop, whichever is first; a repeated burst ends only at one of the last four. A burst whose target equals a burst already held is refused with `radio.COMMAND_REJECTED`. Every block a Provider transmits goes through the Kernel's `BurstTracker` (SC-29). A burst never starts before its transmit clock's origin: a target before it is late, and the burst's `LatePolicy` applies (`send_asap` moves it to the origin with `TIME_ERROR { send_asap }`, `drop` drops it) — which a burst admitted right after a `cold` transmit change can meet on a Provider whose restart lead is not zero (Review K, N-P0-1). A Provider that hands a burst's samples to a device ahead of their instants, and cannot recall them, cannot start the next burst before the first sample it has not yet handed over: for a burst that would start while another is open, the Provider decides it again when it takes it to transmit, against that first unhanded sample — which only the part of the Provider that hands samples over advances, so the decision and the hand-over cannot race (Review K, N-P1-2) —, and a target before it is late, with the burst's `LatePolicy` applied as above; the open burst ends at the next burst's start as decided. The Provider's spec states the in-flight window, the most it hands over ahead of the device's time. MockRadio hands nothing over ahead and restarts at once, so its bounds are `now + device_lead` and the origin (Phase 7, VE-2; Review J, P0-2). *Producer obligation.*
- **RM-16** `Provider::stop(mode)` stops the transmit side first (every transmit sample before the stop instant has been transmitted: the burst transmitting at the stop instant, including a held burst whose start was before it, closes at the first sample at or after it with `BurstTracker::stop` and `BurstEnd::Stop`, a burst that had already ended keeps its end, and every held burst that starts at or after the stop instant is cancelled; Phase 3, VB-1) — and the receive side second: under `orderly` the receive stream keeps delivering until `stop instant + radio.timing.stop_tail_ns` and then ends, under `abort` it ends at once. A `Stop` Action for `<device>/tx` does the transmit half; for `<device>/rx` the receive half, with the tail; for `<device>` both. `Provider::stop` and a `Stop` for `<device>` also cancel every pending timed command; a `Stop` for one stream cancels none, because a pending command may be for the other direction (Phase 3, VB-10). A Provider that hands transmit samples to a device ahead of their instants ends a burst stopped at instant `s` at the last sample it had handed over, no later than `s` plus its in-flight window; the burst's record (`BurstEnd::Stop`) says where it ended. `Provider::stop` and a `Stop` for `<device>` cancel every pending timed command the Provider still holds; a command it has already handed to a device that cannot recall it — within the release window its spec states — is not cancelled, and is recorded as issued (Phase 7, VE-2; the cancellation clause after Review J, P0-4 c). MockRadio's in-flight window and release window are zero. *Producer obligation (MA-13, KA-12).*

### Receive faults

- **RM-17** An **overrun** at instant `f` on a receive stream loses every sample whose instant is in `[f, f + radio.timing.overflow_restart_gap_ns)`. The block being accumulated at `f` is delivered with the samples before `f` if it has any. The next block begins at the first sample instant at or after `f + gap`; when at least one sample is lost it carries `GAP_BEFORE | RESTARTED` and `lost` equal to its whole time jump, which includes any loss already pending for it (SC-18); when none is lost it adds neither (flags already pending stay). `radio.RX_OVERFLOW { cause: overrun, lost, restart_gap_ns }` is emitted with time `TimePoint(<receive SampleClock>, k_f)`, the stream's instant of the first lost sample — the first sample at or after `f` (RM-11: a stream event's time is in the stream's SampleClock). *Producer obligation; this is the UHD overflow of Vision §23 and §58 #6.*
- **RM-18** A **sequence error** at `f` loses the samples of one nominal block (`radio.rx.block_len`) starting at the first sample at or after `f`; the next block carries `GAP_BEFORE | SEQ_DISCONTINUITY`, without `RESTARTED`, with `lost` equal to the jump, and `radio.RX_OVERFLOW { cause: sequence, lost, restart_gap_ns: 0 }` is emitted (SC-18). *Producer obligation.*

### The RF safety envelope

- **RM-19** The section `radio.rf_envelope` is this document (`RfEnvelope`, `deny_unknown_fields`):

  ```text
  RfEnvelope { allowed_bands: [{ lo_hz: num, hi_hz: num }],          required; lo_hz ≤ hi_hz
               max_gain_db:   optional num,                          applies to every transmit channel
               tx_enabled:    optional bool | [bool],                absent: every transmit channel enabled
               antenna_ports: optional [string] }                    absent: any antenna
  ```

  `RfEnvelopeCheck` runs at `validate`, `prepare` and `runtime`, and for each fragment `F`, over `effective[F]` overlaid with `proposed[F]` (KA-4), reports one `Violation { check: radio.rf_envelope, key, requested, reason }` — `reason` beginning `"RM-19: <fragment>: "` — for each of:
  1. the section does not parse as `RfEnvelope`, or a band has `lo_hz > hi_hz` (one violation, `key: None`, for the whole check);
  2. with `radio.tx.channels` > 0 (absent counts as 0): `radio.tx.frequency_hz` present and in no `[lo_hz, hi_hz]`, inclusive; `radio.tx.gain_db` present and above `max_gain_db`; `tx_enabled` false, or a list whose entry for some channel `c < radio.tx.channels` is absent or false (key `radio.tx.channels`); `radio.tx.antenna` present and not in `antenna_ports`;
  3. `radio.rx.antenna` present and not in `antenna_ports`.

  Receive frequencies are not limited: the envelope is about emission (Vision §52). A numeric value is read as `Int` or `Num`; one of another kind is a violation of its key. *Checked by `rm_19_rf_envelope_cases`.*

### Schemas

- **RM-20** `RfEnvelope` and `RadioEnvelope` — the document a radio Provider writes into its Manifest section `<module>.envelope`, `{ profile: ProfileRef, timing: { min_timed_command_lead_ns, startup_latency_ns, stop_tail_ns, command_queue_depth, overflow_restart_gap_ns }, performance: { rx_bytes_per_s, tx_bytes_per_s, wire_bytes_per_sample } }` — have committed schemas `schemas/radio/rf_envelope.v1.json` and `schemas/radio/envelope.v1.json` (PO-7); RM-22 adds eight payload schemas to the same directory — five in Phase 2, and `tx_underflow_payload`, `alignment_error_payload` and `clock_lost_payload` in Phase 7 —, and RM-24 the schema of `RX_OVERFLOW`'s hot-path form (Phase 4): eleven files. `time_error_payload.v1.json` is regenerated with the value `late_at_device`, still version 1 before the freeze; a reader validating against the Phase 2 file would refuse that value, so a validating reader regenerates from 1.3.0 (`schemas/SCHEMA_CHANGELOG.md`, "v1 — Phase 7 — what a device reports") (Phase 7, VE-3). *Checked by `rm_20_schema_freeze` (eleven files).*

### Event payload types

- **RM-22** The payloads of RM-11 are these Rust types in `ezsdr_radio::payloads`, each `#[derive(Serialize, Deserialize, JsonSchema)]` with `#[serde(deny_unknown_fields)]`, and a radio Provider builds a control-path event's `payload` as `serde_json::to_value(&the payload)` (a hot-path `RX_OVERFLOW` carries RM-24's bytes instead; Phase 4):

  ```rust
  pub struct RxOverflowPayload     { pub cause: RxOverflowCause, pub lost: u64, pub restart_gap_ns: i64 }
  pub enum   RxOverflowCause       { Overrun, Sequence }                          // snake_case
  pub struct TimeErrorPayload      { pub cause: TimeErrorCause, pub outcome: TimeErrorOutcome, pub late_by_ns: i64, pub target: TimePoint }
  pub enum   TimeErrorCause        { Late, UnclosedBurst }                         // snake_case
  pub enum   TimeErrorOutcome      { SendAsap, Drop, PlanViolation, Refused, LateAtDevice } // snake_case: late_at_device (Phase 7)
  pub struct LateCommandPayload    { pub key: Option<Key>, pub requested: TimePoint, pub applied: TimePoint }
  pub struct CommandQueueFullPayload { pub key: Key, pub depth: i64 }
  pub struct CommandRejectedPayload  { pub action: String, pub reason: String }
  // Phase 7, VE-3:
  pub struct TxUnderflowPayload    { pub cause: TxUnderflowCause }
  pub enum   TxUnderflowCause      { Starved, Lost }                                   // snake_case
  pub struct AlignmentErrorPayload { pub lost: u64 }
  pub struct ClockLostPayload      { pub reference: ClockReference }
  pub enum   ClockReference        { Frequency }                                      // snake_case; `time` is additive when a Provider monitors its PPS
  ```

  Their schemas are committed as `schemas/radio/<name>.v1.json` with the names `rx_overflow_payload`, `time_error_payload`, `late_command_payload`, `command_queue_full_payload`, `command_rejected_payload` and, from 1.3.0, `tx_underflow_payload`, `alignment_error_payload` and `clock_lost_payload` (PO-7; Phase 7, VE-3), which is RS-31's "the payload schemas of non-Kernel kinds are their Vocabulary's". *Checked by `rm_20_schema_freeze` and by `rm_22_payloads_round_trip` (each type serialises to the object RM-11 shows and deserialises back; the Phase 7 types and value included), and by `rm_22_late_at_device_is_snake_case`.*

### Update classes of radio keys

- **RM-21** For radio keys, UC-3's `cold` applies to the channel counts and the sample rates — a change ends the stream's SampleClock and starts a new one, and a count changed to 0 ends the stream, from 0 starts it — and UC-6's `hardware_timed` to frequency and gain, with the device's command queue of `radio.timing.command_queue_depth` slots. The antennas are not changeable during a Run. *Producer obligation.*

### Path delay

- **RM-23** `radio.tx.path_delay_samples` and `radio.rx.path_delay_samples` are the device's fixed delays between a sample's timestamp and the antenna, in samples of the stream's current rate, declared on the device node as `One(Int(n))` with `n ≥ 0`: a transmit sample stamped `T` is at the antenna `n_tx` transmit samples after `T`, and a receive sample stamped `T` is what was at the antenna `n_rx` receive samples before `T`, so a loopback through a zero-delay path at one rate shows `n_tx + n_rx` samples of delay. This is Vision §26's per-profile delay default; a per-Run override by a CalibrationArtifact is later work (Phase 3, VB-1). *Producer obligation; MockRadio's values are MR-3's and its test is `mr_32_the_transmit_path_delay_shifts_a_loopback`.*

### Hot-path forms

- **RM-24** `radio.RX_OVERFLOW` has a hot-path form of 17 bytes, little-endian: byte 0 the cause (`0` overrun, `1` sequence), bytes 1–8 `lost` as u64, bytes 9–16 `restart_gap_ns` as i64. `RxOverflowPayload::to_hot(&self) -> [u8; 17]` writes it. `RxOverflowPayload::from_hot(bytes: &[u8]) -> Result<RxOverflowPayload, String>` reads it and refuses another length or a cause byte other than 0 or 1. `RxOverflowPayload::from_payload(payload: &serde_json::Value) -> Result<RxOverflowPayload, String>` reads a delivered event's payload in either form — RM-11's object, from the control path, or the array of byte values the Kernel's drain makes of a hot-path record (RS-34) — and refuses anything else, so one call reads a Manifest from before 1.2.0, whose Mock emitted the object, and one from after. `RxOverflowHotPayload`, the array as a Manifest holds it, has the committed schema `schemas/radio/rx_overflow_hot_payload.v1.json` (PO-7), so a reader that validates payloads has a schema for both forms. The Kernel does not decode it (RS-32a, withdrawn: the Vocabulary owns its layout) (Phase 4, VC-1). *Ceilings: a drain delivers the ring's bodies before the control path's (RS-34), so the Manifest's `delivered` list is ordered by path, then by emission, and when two kinds of one drain both stop the Run the hot one is the recorded cause; a hot body the full ring drops is counted and reported in `EVENTS_DROPPED` but marks no artifact, since `mark_artifact` acts on delivered bodies (RS-30).* *Checked by `rm_24_the_hot_form_round_trips` and `rm_24_a_delivered_payload_reads_in_either_form`.*

### The lattice

- **RM-25** A radio Provider registers every SampleClock at an origin on its root's **lattice**: a whole multiple of the numerator, in lowest terms, of the clock's `root_ticks_per_tick`, so that two clocks of one ratio share every sample instant and T0 (KC-15) is a sample instant of every stream. The receive clock's origin is T0 (MR-11, spec 18 UR-15). A transmit clock registered at `arm` takes the first lattice instant at or after the arm instant (MR-9). On a `cold` change with effective instant `e` (UC-1), the old clock, if one runs, ends at `e₁`, the first instant at or after `e` on the old clock's lattice, and the new clock, if the new channel count is > 0, starts at `e₂`, the first instant at or after `e₁` plus the Provider's restart lead (UC-3 as KG-13 amends it; zero for MockRadio) on the new clock's lattice; the samples between `e₁` and `e₂` are neither delivered nor a gap. A change from 0 channels has no old clock and needs no restart lead: the new clock starts at the first lattice instant at or after both `e` and the instant the Provider finished configuring the stream (Phase 7, VE-2; one rule for both Providers after Review J, P0-4 d; the from-zero case after Review K, N-P0-1). *Producer obligation; MockRadio's tests are `mr_09_the_transmit_clock_starts_on_its_lattice` and `mr_18_a_cold_change_starts_its_clock_on_the_lattice`; the UHD Provider's is spec 18's `ur_15_every_clock_starts_on_its_lattice`.*

### One implementation of the device rules

- **RM-26** `ezsdr_radio::device` implements, once, RM-2's tree with the capabilities in MR-4's form, RM-5's defaults, RM-7's refusal and RM-8's coercion, over a `DeviceDescription`:

  ```rust
  pub enum Grid { Values(Vec<f64>), Integer { lo: i64, hi: i64 }, Step { lo: f64, hi: f64, step: f64 } }
  pub struct DeviceDescription {
      pub profile: ProfileRef,
      pub rates: Grid,                  // both directions; `Values` → an `AnyOf` capability, otherwise a `Range`
      pub whole_hertz_rates: bool,      // an `Eq` rate that is not a whole number of hertz is refused
      pub frequency: Grid,              // `Step`; step 0 means no grid
      pub gain: Grid,                   // `Step`
      pub max_channels: i64,            // per direction
      pub rx_antennas: Vec<String>, pub tx_antennas: Vec<String>,
      pub coherent: bool, pub full_duplex: bool, pub hardware_time: bool,
      pub phase_behavior_on_retune: String,
      pub repeat_max_samples: u64, pub repeat_align_samples: u64,
      pub block_len: u32,
      pub tx_path_delay_samples: i64, pub rx_path_delay_samples: i64,
      pub timing: TimingEnvelope, pub performance: PerformanceEnvelope,
      pub defaults: BTreeMap<Key, Value>,   // RM-5: exactly the ten configuration keys
  }
  impl DeviceDescription {
      pub fn tree(&self, device: &ResourceId) -> Resource;                                           // RM-2
      pub fn coerce(&self, device: &ResourceId, request: &Requested) -> Result<CoerceReport, ModuleError>; // RM-5, RM-7, RM-8
      pub fn envelope(&self) -> RadioEnvelope;                                                      // RM-20
  }
  ```

  `coerce` is pure (MA-11). Its refusal reasons begin `"RM-4: "` (a key that is not a radio key, or a value of the wrong kind), `"RM-5: "` (a `Present` with no default), `"RM-7: "` and `"RM-8: "`; a request for a resource other than `device` is refused with a reason beginning `"RM-26: "`. A radio Provider may use it; MockRadio 1.3.0 and `ezsdr.radio.uhd` 0.1.0 do. A Provider that does meets RM-7 and RM-8 at `coerce` by construction and keeps RM-7's obligations at `prepare` and at runtime (Phase 7, VE-1). *Checked by `rm_26_the_description_builds_the_tree_mockradio_built` and `rm_26_coerce_cases` (`crates/ezsdr-radio/tests/device.rs`) and, for equivalence, MockRadio's unchanged `mr_06_*` and `mr_04_instance`.*

## 5. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| R1 | Keys per direction or per stream node | Direction-prefixed keys on the device node (RM-4) | Unprefixed keys on stream nodes (a Session binds the device, SB-T2); per-channel keys | Additive per-channel keys (RM-5) |
| R2 | A transmit stream by default | Absent: `radio.tx.channels` defaults to 0 at the Provider (MR-5) | Default 1 (every Run would configure a transmitter nobody asked for, and a TX-disabled site would refuse a receive-only Run) | A Session enables TX with a `cold` `SetParameter` |
| R3 | Envelopes | Capabilities (Y6) | A document the Kernel parses (OV-21) | A new envelope value is an additive capability key |
| R4 | Coercion ties | Toward the lower grid value (RM-8) | Toward the higher (would round a rate up past a transport limit) | none |
| R5 | Out-of-range values | Rejected, never clamped | Clamping to the range edge (silently changes a frequency by gigahertz) | none |
| R6 | Late burst events | `TIME_ERROR` for bursts, `LATE_COMMAND` for commands (spec 04 §9 issue 2) | One `LATE` event (Vision §22's earlier wording; `LATE` is a block flag) | none |
| R7 | Where RX restart behaviour is stated | In the Vocabulary (RM-17, RM-18), so MockRadio and the UHD Provider share it | Per Provider (Mock and hardware would diverge, which Vision §17 forbids) | Phase 8 measures the gap |
| R8 | The RF check's scope | Transmission, plus antenna ports (RM-19) | Receive frequencies too (Vision §52 is about emission; a receive-only experiment outside a licensed band is legal) | A site that needs receive limits adds a key |
| R9 | TX port | Reserved, not declared (RM-2) | Declaring it now (nothing in Phase 2 could feed it, and a declared port no Provider serves is a rule with no carrier) | Phase 10 declares it |

## 6. Tests

In `crates/ezsdr-radio/tests/radio_model.rs`.

| test | input | expected | rules |
|---|---|---|---|
| `rm_01_register_adds_the_descriptor_the_check_and_the_kinds` | empty registries, then `register` | the Vocabulary is `radio 1.3.0`; `checks.run` on a present `radio.rf_envelope` section runs the check; each RM-10 kind is registered with its default | RM-1 |
| `rm_01_register_twice_is_refused` | `register` called twice | the second returns an error; nothing is overwritten | RM-1, MA-32, RS-27 |
| `rm_04_the_key_table_is_exactly_the_declared_one` | `vocabulary().keys` | exactly RM-4's thirty-one rows, each field as the table says | RM-4, RM-23 |
| `rm_10_the_kinds_are_registered_under_radio_with_their_defaults` | `vocabulary().event_kinds` | exactly RM-10's nine, severities and defaults as the table says, each under `radio.` | RM-10 |
| `rm_12_the_verbs_compile_to_bursts` | `vocabulary().verbs` | `start_repeat` → repeat, `send_asap_and_flag`; `send` → no repeat, `drop_and_flag`; the Kernel registry accepts both (no `RejectAtPlan`) | RM-12 |
| `rm_19_rf_envelope_cases` | a band `[2.4e9, 2.5e9]`, `max_gain_db: 20`, `tx_enabled: [true, false]`, `antenna_ports: ["TX/RX", "RX2"]`; configurations: TX at 2.45e9 gain 10 one channel; TX at 2.6e9; gain 25; two channels; antenna `J1`; TX count 0 with frequency 2.6e9; RX antenna `J1`; the same two fragments with only one violating | no violation; one each on frequency, gain, channels and antenna; none with count 0; one on the RX antenna; the violation names only the violating fragment | RM-19, KA-4 |
| `rm_19_a_malformed_section_is_one_violation` | `{ allowed_bands: "x" }`; a band with `lo > hi`; an unknown field | one violation each, `key: None` | RM-19 |
| `rm_19_the_proposed_value_is_judged` | at `runtime`, `effective` in band and `proposed` frequency out of band | one violation carrying the proposed value | RM-19, SB-30 |
| `rm_20_schema_freeze` | regenerate `RfEnvelope`, `RadioEnvelope`, the eight RM-22 payloads and RM-24's hot form | byte-equal to `schemas/radio/*.v1.json` (eleven files, Phase 7); no other file there | RM-20, RM-22, RM-24, PO-7 |
| `rm_22_payloads_round_trip` | one value of each RM-22 type | `to_value` gives exactly RM-11's members with snake_case enum strings; `from_value` gives the value back; an extra member is refused | RM-22, RM-11 |
| `rm_22_late_at_device_is_snake_case` | `TimeErrorOutcome::LateAtDevice` | serialises as `"late_at_device"` (Phase 7, VE-3) | RM-22, RM-11 |
| `rm_24_the_hot_form_round_trips` | an overrun of 50 000 samples and 50 ms; both causes with `lost` `u64::MAX` and `restart_gap_ns` `i64::MIN`; a 16- and an 18-byte slice; a cause byte of 2 | the first's 17 bytes pinned byte for byte; `from_hot(to_hot(p)) == p`; three refusals naming RM-24 | RM-24 |
| `rm_24_a_delivered_payload_reads_in_either_form` | `to_value(p)`; `p.to_hot()` as a JSON array of integers; that array with a 256; a string | `p` twice; two refusals | RM-24, RM-11 |

In `crates/ezsdr-radio/tests/device.rs` (Phase 7, VE-1):

| test | input | expected | rules |
|---|---|---|---|
| `rm_26_the_description_builds_the_tree_mockradio_built` | the `x310-like` values as a description | a tree equal, node for node, to the one MockRadio 1.2.0 builds (the expected tree is written out in the test) | RM-26, RM-2 |
| `rm_26_coerce_cases` | MockRadio's `mr_06_*` requests against the description: 19.5 Msps, 7 GHz, gain 31.7, four channels at 200 Msps on 1 GB/s, a `Set` of antennas, `Present` of every key | 20 Msps with a coercion; RM-8 rejection; 31.5 with a coercion; RM-7 rejection naming the rate key; the first available antenna; the defaults | RM-26, RM-5, RM-7, RM-8 |

## 7. Vision issues found

1. **§8's example keys** (`radio.rx.channels`, `radio.sample_rate_hz`, …) are placeholders the Vision says the Radio Model will define. RM-4 defines them; the example's `radio.sample_rate_hz` becomes `radio.rx.sample_rate_hz` and `radio.tx.sample_rate_hz`, and its `radio.rx.coherent` keeps its name.
2. **§13's TimingEnvelope tree** lists "coercion rules" as an envelope member; RM-8 makes the grid the declared capability itself and the rule the Vocabulary's.
3. **§22's `TxBurst.format` and channel list**: the format is the stream's contract and the burst uses every channel (RM-13), as spec 02 §10 issue 8 asked the Radio Model to settle.
4. **§26's `phase_behavior_on_retune`** is a declared capability now (RM-9); its emulation is Phase 3's.

## 8. Deferred

A transmit port and link-fed transmission (Phase 10). Per-channel configuration. DDC/DUC and decimation capabilities, device FFT (Phase 7+). A hot-path payload layout for radio events (Phase 7). A per-Run delay override by a CalibrationArtifact (Vision §26).
