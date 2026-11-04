# Phase 2 spec 07 — The Radio Model Vocabulary (`radio` 2.0.0)

| Field | Value |
|---|---|
| Status | Accepted at Gate P (owner, 2026-09-24) and ratified at Gate X (owner, 2026-09-25; [`plan/phase2/00-overview.md`](../plan/phase2/00-overview.md) §11). Normative for `crates/ezsdr-radio` and for every radio Provider (MockRadio, spec 09; the UHD Provider, spec 18). Its amendments are listed in the closing Changes table. |
| Scope | The `radio` Vocabulary: the resource kinds and tree shape a radio Provider exposes; the configuration keys and the capability keys, each with its `KeyDecl`; the TimingEnvelope and PerformanceEnvelope as capabilities; coercion rules; the event kinds with their defaults, severities and payloads; the Session verbs; the `TxBurst` conventions; the `radio.rf_envelope` section and its admission check. |
| Not in scope | Any Provider's values (spec 09's MR-3 and spec 18's UR-9 both build their description with `ezsdr_radio::channels`, design 33 D5). Channel models (spec 11). Receive-side DDC and transmit-side DUC capabilities, device FFT and RFNoC (spec 18's Not in scope; Vision §20, §35). |
| Crate | `crates/ezsdr-radio`, library `ezsdr_radio`. Depends on `ezsdr-kernel`, `serde`, `serde_json`, `schemars`. |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

---

## 1. Purpose

Vision §5 puts "channels, streams, RF parameters, TimingEnvelope, PerformanceEnvelope, coherence basis, repeat / decimation capabilities" in the Radio Model, a Vocabulary: versioned on its own, additive, and never read by the Kernel by name (OV-21). This document is that Vocabulary's first version. Every radio Provider — MockRadio (spec 09) and the UHD Provider (spec 18) — implements the same keys with the same meaning, which is what makes Vision §59's "change only the BindingProfile" testable.

## 2. Evidence

- **UHD coerces rate, gain and frequency** to a device grid and documents reading them back (audit Appendix B; spec 03 §2). Hence RM-8's coercion rules and the per-key coercion defaults Vision §11 names: `reject` for sample rate and frequency, `warn` for gain.
- **UHD's overflow**: zero samples returned, `out_of_sequence` distinguishing a host-side sequence error, a continuous stream restarting after `OVERRUN_RESTART_DELAY` = 0.05 s (audit Finding 3, VERIFIED). Hence RM-17 and RM-18.
- **UHD resets the CORDICs on every start of burst and needs a time spec on each** (spec 02 §2). Hence RM-13's one waveform per burst and the Kernel's `BurstTracker` (SC-29).
- **A late timed command** on UHD yields `ERROR_CODE_LATE_COMMAND` on receive and `EVENT_CODE_TIME_ERROR` on transmit (audit §10's UHD row). Spec 04 §9 issue 2 separates the two: `TIME_ERROR` for a burst whose target has passed, `LATE_COMMAND` for a control command issued too late. Hence RM-10.
- **Retune phase** on a UBX or an OBX is random unless the retune is timed (audit Finding 15, VERIFIED against UHD's `page_sync`), and a CBX has no timed LO phase synchronisation at all (`sync_phase` is the UBX's and the OBX's; design 33 D13), so its phase is random on every retune. Hence RM-9's three values.

## 3. Model

```text
Provider instance tree (one per radio instance)
└── <device>                kind radio.device       the node a Spec binds (Y5)
    │   ports:  rx<k>  out  ezsdr.stream.cf32       one per receive channel
    │   capabilities: per-channel, per-stream and instance-wide values
    ├── <device>/rx/s<k>     kind radio.rx_stream    stream id: SampleClock owner, event source
    └── <device>/tx/s<k>     kind radio.tx_stream    stream id: SampleClock owner, TxBurst target

configuration (PrepareReport.effective; per fragment, SB-30)
  radio.{rx,tx}.c<n>.{frequency_hz, gain_db, antenna, stream}
  radio.{rx,tx}.s<k>.sample_rate_hz

instance-wide capabilities
  radio.full_duplex   radio.hardware_time   radio.tx.repeat_*
  radio.rx.block_len   radio.timing.*   radio.perf.*
```

## 4. Rules

### Identity and registration

- **RM-1** The Vocabulary is `VocabularyDescriptor { id: radio, version: 2.0.0, prefix: radio, keys: RM-4's and RM-27's tables, event_kinds: RM-10's table, verbs: RM-12's table, checks: [radio.rf_envelope] }`, returned by `ezsdr_radio::vocabulary()`. `ezsdr_radio::register(registry, checks, kinds)` registers the descriptor, the `RfEnvelopeCheck` of RM-19 and every event kind with owner `Some(radio)`, in that order, and is the only way a runtime assembles the Vocabulary.
  *Test:* `rm_01_register_adds_the_descriptor_the_check_and_the_kinds`.

### Resource tree

- **RM-2** A radio Provider instance exposes one `radio.device` root with `<device>/rx/s<k>` and `<device>/tx/s<k>` children of kind `radio.rx_stream` and `radio.tx_stream`; stream children have no capabilities, ports or memory domains. The root declares one `rx<k>` output port of contract `ezsdr.stream.cf32` per receive channel and its host memory domain. The port name `tx` (direction `in`) is reserved for link-fed transmission and is not declared. A Spec resource that uses a radio is of kind `radio.device`. No node of a radio tree is `shareable`.
  *Producer obligation.*
  *Test:* `mr_02_the_tree_has_the_radio_model_shape`, `rm_29_the_layout_description_builds_the_stream_tree` and `d3_the_layout_is_a_node_id_of_neither_provider`.
- **RM-3** A stream SampleClock is declared on `<device>/rx/s<k>` or `<device>/tx/s<k>` (TM-13a). Its block's `channels` is the size of its channel set, and block channel `i` is that set's `i`-th smallest logical channel (RM-30, SC-10). A stream that holds no channel has no SampleClock or block; every `TxBurst` targeting it is refused (KC-24).
  *Producer obligation.*

### Keys

- **RM-4** The instance-wide keys are exactly these, declared with these `KeyDecl`s; RM-27 declares the channel and stream keys:

  | Key | kind | coercible | coercion default | update class | Role |
  |---|---|---|---|---|---|
  | `radio.full_duplex` | bool | no | reject | — | capability |
  | `radio.hardware_time` | bool | no | reject | — | capability: timed commands are executed by the device |
  | `radio.tx.repeat_max_samples` | int | no | reject | — | capability: longest repeated waveform |
  | `radio.tx.repeat_align_samples` | int | no | reject | — | capability: a repeated waveform's length is a multiple of this |
  | `radio.rx.block_len` | int | no | reject | — | capability: the nominal samples per receive block (informational; SC-15 still holds) |
  | `radio.timing.min_timed_command_lead_ns` | int | no | reject | — | TimingEnvelope (RM-6) |
  | `radio.timing.startup_latency_ns` | int | no | reject | — | TimingEnvelope |
  | `radio.timing.command_queue_depth` | int | no | reject | — | TimingEnvelope |
  | `radio.timing.release_window_ns` | int | no | reject | — | TimingEnvelope (RM-6): the window after a receive cut that the next start's readiness counts (RM-25) |
  | `radio.timing.overflow_restart_gap_ns` | int | no | reject | — | TimingEnvelope |
  | `radio.timing.rx_start_lead_ns` | int | no | reject | — | TimingEnvelope (RM-6) |
  | `radio.timing.tx_start_lead_ns` | int | no | reject | — | TimingEnvelope |
  | `radio.timing.tx_in_flight_ns` | int | no | reject | — | TimingEnvelope (RM-6, RM-15) |
  | `radio.perf.rx_bytes_per_s` | int | no | reject | — | PerformanceEnvelope (RM-7) |
  | `radio.perf.tx_bytes_per_s` | int | no | reject | — | PerformanceEnvelope |
  | `radio.perf.wire_bytes_per_sample` | int | no | reject | — | PerformanceEnvelope: the host–device wire format's bytes per complex sample (SC-5) |
  `radio.timing.rx_start_lead_ns` and `radio.timing.tx_start_lead_ns` are, for their own direction, a stream's lead from its previous cut and from the instant its Provider is ready for it to a new segment's first sample (RM-25); neither is the Kernel's `ezsdr.time.start_lead_ns`, the lead before T0.
  *Test:* `rm_04_the_key_table_is_exactly_the_declared_one`.
- **RM-5** A radio Provider's `PrepareReport.effective` holds exactly RM-27's configuration keys for its fragment, with the applied values, whether or not the Spec constrained them; an unconstrained key takes the Provider's default. Capability-only keys are not in `effective`.
  *Producer obligation.*
  *Test:* `mr_08_effective_holds_one_channel_and_its_stream_rate`, `mr_38_the_tree_and_capabilities_are_the_layout_s` (the stream rate's applied value and the exact key set of three channels) and `mr_05_a_layout_without_receive_channels_opens_no_receive_stream` (a constrained key's applied value).

### Envelopes

- **RM-6** The **TimingEnvelope** is the eight `radio.timing.*` capabilities, each declared on the device node as `One(Int(v))`, in nanoseconds or, for the queue depth, commands: the minimum timed-command lead, the startup latency, the queue depth, the release window, the overflow restart gap, one start lead per direction and the transmit in-flight window. A radio Provider sets `ProviderInstance.min_command_lead` to `Some(Duration(host.monotonic, radio.timing.min_timed_command_lead_ns))` when that value is positive and to `None` when it is 0 (MA-10). A Spec expresses a timing requirement as a constraint on these keys; the generic matcher refuses a Provider that cannot meet it at `validate()` (Vision §13 rule 2). The `min_timed_command_lead_ns` is the end-to-end lead the Provider needs from the coordinator's dispatch, including its delivery allowance. A Description's `radio.timing.rx_start_lead_ns` is at least its `radio.timing.release_window_ns`; a Provider builds no Description, and a measurement instance takes no candidate timing, that breaks this.
  *Producer obligation.*
  *Test:* `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`, `ur_09_profile_values_reach_the_capabilities_and_the_envelope_section` and `sb_06_constraint_match_table`; `ProviderInstance.min_command_lead` (`Some` for a positive lead, `None` for 0) by `mr_04_instance` and `ur_10_instance`; the start lead against the release window by `rm_6_every_published_description_keeps_the_start_lead_at_or_above_the_release_window`, `rm_6_the_start_lead_check_cases`, `rm_6_building_a_description_checks_the_start_lead`, `rm_6_building_a_description_refuses_a_lead_below_the_window_x3xx`, `rm_6_building_a_description_refuses_a_lead_one_nanosecond_below_the_window_x3xx`, `rm_6_building_a_description_refuses_a_lead_one_nanosecond_below_the_window_ideal`, `rm_6_a_measurement_timing_keeps_the_start_lead_at_or_above_the_window`, `rm_6_a_measurement_timing_with_a_start_lead_below_the_window_is_refused`, `rm_6_a_measurement_timing_with_a_start_lead_one_nanosecond_below_the_window_is_refused` and `measurement_plan_from_env_uses_defaults_and_rejects_invalid_counts` (the receive candidates).
- **RM-7** The **PerformanceEnvelope** is the three `radio.perf.*` capabilities. A layout configuration is admissible only when the sum, over streams in each direction, of channel count × stream rate × wire bytes per sample does not exceed that direction's throughput limit (RM-32). A radio Provider refuses an inadmissible configuration in `coerce`, and in `prepare`.
  *Producer obligation.*
  *Test:* `mr_06_coerce_cases`; the `prepare` refusal by `mr_07_prepare_cases`.
- **RM-8** **Coercion.** A numeric configuration key is coerced to its declared grid: the sample rates the device node declares as `AnyOf`, and each channel's frequency and gain to multiples of that channel's `radio.d.c<n>.frequency_step_hz` and `radio.d.c<n>.gain_step_db` within its declared ranges (RM-27). For an `Eq(v)` constraint the applied value is the grid value nearest `v`, the lower of two equally near; a `Coercion { key, requested: v, applied, reason }` is recorded exactly when `applied ≠ v` numerically (SB-6). A value outside the range is not coerced into it: the key is `rejected`. For `Min`, `Max`, `Range` and `Set` constraints the applied value is, respectively, the smallest grid value at or above the bound, the largest at or below, the smallest inside the range, and the first member of the set on the grid; none existing is a rejection, and none of these is a coercion. `Present` takes the default.
  A layout description coerces its channel and stream keys by the same rule (RM-30).
  *Producer obligation.*
  *Test:* `mr_06_coerce_cases` and `rm_30_coerce_cases`; none named for the `Max` and `Range` applied values and for a `Min`/`Max`/`Range`/`Set` that finds no grid value.

### Coherence

- **RM-9** A layout description declares `radio.d.c<n>.phase_behavior_on_retune` per channel as `deterministic`, `random_unless_timed_tune` or `random` (RM-27, Vision §26). It declares no direction-wide coherence key.
  *Producer obligation.*
  *Test:* `d3_a_cbx_layout_is_not_coherent_on_either_provider`; the `deterministic` value by `rm_29_the_ideal_layout_builds_its_tree_and_defaults` and `mr_34_the_deterministic_profile_keeps_every_phase_zero`.

### Events

- **RM-10** The event kinds are exactly these, registered with owner `radio` (RS-27):

  | Kind | Severity | Default reaction | Emitted when |
  |---|---|---|---|
  | `radio.RX_OVERFLOW` | warning | mark_artifact | a receive overrun or sequence error lost samples (RM-17, RM-18) |
  | `radio.TX_UNDERFLOW` | warning | mark_artifact | a transmit stream ran out of samples inside a burst (SC-25) |
  | `radio.TX_DISCONTINUITY` | warning | mark_artifact | the burst tracker reported a discontinuity (SC-24a) |
  | `radio.TIME_ERROR` | error | mark_artifact | a burst's target was too close or past (RM-14), or the device model saw an unclosed burst followed by a timed block (spec 02 decision S12) |
  | `radio.LATE_COMMAND` | warning | mark_artifact | a stream start arrived with too little lead (MR-11) |
  | `radio.ALIGNMENT_ERROR` | error | mark_artifact | channels of one stream lost alignment (SC-13, SC-31) |
  | `radio.CLOCK_LOST` | fatal | abort | the device lost its reference |
  | `radio.COMMAND_QUEUE_FULL` | error | abort | reserved: no key has an update class (RM-21), so no Provider emits it |
  | `radio.COMMAND_REJECTED` | error | mark_artifact | the Provider could not carry out an admitted Action: an unsupported Action, a malformed waveform, a target it does not have, a configuration its envelope refuses |

  No Provider emits `radio.COMMAND_QUEUE_FULL`. MockRadio never emits `radio.TX_UNDERFLOW`, `radio.TX_DISCONTINUITY`, `radio.ALIGNMENT_ERROR` or `radio.CLOCK_LOST`; the UHD Provider emits all but `radio.TX_DISCONTINUITY` (spec 18 UR-19, UR-27, UR-28).
  *Test:* `rm_10_the_kinds_are_registered_under_radio_with_their_defaults`; none named for MockRadio's four never-emitted kinds and for the UHD Provider's never emitting `radio.TX_DISCONTINUITY`.
- **RM-11** Every radio event is emitted with `emit_control`, except `radio.RX_OVERFLOW`, which every radio Provider must emit on the hot path (`EventSink::emit`, RS-32) with RM-24's bytes. The source node is the stream's own `<device>/rx/s<k>` or `<device>/tx/s<k>` node (RM-3) for `radio.RX_OVERFLOW`, `radio.ALIGNMENT_ERROR`, `radio.TX_UNDERFLOW` and `radio.TIME_ERROR`; the device node for `radio.COMMAND_QUEUE_FULL`, `radio.COMMAND_REJECTED` and `radio.CLOCK_LOST`; and for `radio.LATE_COMMAND` the device node when it answers a start, and the receive stream's node when it reports that stream's missed timed start (spec 18 UR-17). No Provider has a transmit port fed by a link yet, so none emits `radio.TX_DISCONTINUITY`; SC-24a's Producer obligation is the first such Provider. `time` is in the stream's SampleClock or, before one exists, the Authority's primary root, and the payload is, on the control path, a JSON object with exactly these members:

  | Kind | Payload |
  |---|---|
  | `radio.RX_OVERFLOW` | `{ "cause": "overrun" \| "sequence", "lost": int }`; on the hot path, RM-24's bytes |
  | `radio.TIME_ERROR` | `{ "cause": "late" \| "unclosed_burst", "outcome": "send_asap" \| "drop" \| "plan_violation" \| "refused" \| "late_at_device", "late_by_ns": int, "target": TimePoint }` |
  | `radio.LATE_COMMAND` | `{ "key": string \| null, "requested": TimePoint, "applied": TimePoint }` |
  | `radio.COMMAND_QUEUE_FULL` | `{ "key": string, "depth": int }` |
  | `radio.COMMAND_REJECTED` | `{ "action"?: ActionId, "kind": string, "reason": string }` (`kind` is the Action's `kind` tag; `action` is the id it was dispatched with (`Dispatched.id`, MA-14) when the Provider refuses it on receipt, and is absent when something it booked earlier is refused as it applies, when that id is no longer at hand) |
  | `radio.TX_UNDERFLOW` | `{ "cause": "starved" \| "lost" }`: `starved`, the device ran out of samples inside a burst because the host was late; `lost`, samples of a burst were lost between host and device |
  | `radio.ALIGNMENT_ERROR` | `{ "lost": int }`: the samples, on every channel of the stream, that the misalignment removed, which is the next block's time jump |
  | `radio.CLOCK_LOST` | `{ "reference": "frequency" }`: the frequency reference (10 MHz) was lost; no Provider monitors its time reference (PPS), so no other value is declared |

  `TIME_ERROR`'s outcome `late_at_device` means that the Provider handed the burst to the device in time by its own clock and the device reported that it arrived too late, so nothing was transmitted; `late_by_ns` is the device's reported instant less the target when the device reports one, and 0 otherwise.

  Each payload is the JSON serialisation of the RM-22 type of its kind, never a hand-built object.
  *Producer obligation.*
  *Test:* `rm_22_payloads_round_trip`, `mr_29_other_actions_are_command_rejected`, `mr_10_events_carry_the_source_the_context_gave`, `mr_16_burst_refusals`, `mr_37_the_overflow_travels_the_hot_path`, `mr_11_start_cases`, `ur_19_reports_before_one_block_keep_each_kind`, `mr_28_a_layout_instance_s_events_name_their_stream_node`, `ur_27_a_lost_reference_is_clock_lost` (the device node as `CLOCK_LOST`'s source) and `ur_28_a_layout_transmit_report_names_its_stream` (the stream node as `TX_UNDERFLOW`'s); none named for the receive stream's node as the source of `LATE_COMMAND` on a missed timed start.

### Verbs and bursts

- **RM-12** The Session verbs are: `start_repeat` → `CompileRule::TxBurst { repeat: true, late_policy: send_asap_and_flag }`; `send` → `CompileRule::TxBurst { repeat: false, late_policy: drop_and_flag }`. Their target is the transmit stream node `<resource>/tx/s<k>` (RM-13). `start_rx` → `CompileRule::Command {}`: its target is the receive stream node `<resource>/rx/s<k>`, it carries no params, and it turns the stream on again after a `Stop` as RM-21 says; its `at` is the instant at or after which it takes effect, never late. A stream's node may also be the target of a `Stop` (RM-13, RM-16): `ezsdr_radio::stream_target` reads `"<device>/rx/s<k>"` and `"<device>/tx/s<k>"` — `k` in SB-2a's spelling, without a leading zero — and no other id.
  *Test:* `rm_12_the_verbs_compile_as_declared` (the verb names, the `CompileRule`s and the registration); `mr_29_start_rx_is_the_one_command` (the target, the empty params and the restart after a `Stop`); `mr_29_start_rx_takes_effect_at_its_at`, `mr_29_a_receive_link_never_begins_before_t0` and `mr_29_start_rx_is_never_late` (the `at`, the T0 floor a receive link keeps, and the never-late instant); `rm_12_a_stream_node_names_its_direction_and_number` (which ids name a stream).
- **RM-13** A `TxBurst` targets `<device>/tx/s<k>` and its `ArtifactRef` holds complex samples in that stream's `ezsdr.stream.cf32` contract, channel-interleaved (sample `n` of channel `c` at byte offset `(n · channels + c) · 8`), little-endian; `channels` is the number of channels the stream holds and channel `i` is that set's `i`-th smallest logical channel (RM-3, RM-30). Its length is `size_bytes / (8 · channels)`, a whole number ≥ 1. The burst transmits on every channel of the stream. A repeated waveform's length must be at most `radio.tx.repeat_max_samples` and a multiple of `radio.tx.repeat_align_samples`. `TxBurst.metadata` is unused and must be empty. A start equal to a held burst's is refused by RM-15.
  A radio Provider refuses a `TxBurst` with `radio.COMMAND_REJECTED`, and transmits nothing, when:
  - its target is not the transmit stream node;
  - the transmit stream holds no channel (RM-21);
  - `size_bytes` is not a positive multiple of `8 · channels`;
  - a sample component is not finite, where the Provider reads the waveform's bytes;
  - it repeats and its length is above `radio.tx.repeat_max_samples` or not a multiple of `radio.tx.repeat_align_samples`;
  - `metadata` is not empty; or
  - its start is before the Provider's synchronisation end (the instant MR-9 and UR-13 state).

  A burst whose waveform does not have the contract, the channel interleaving or the byte order above is not transmitted and is reported with `radio.COMMAND_REJECTED`.
  *Producer obligation.*
  *Test:* `mr_16_burst_refusals`, `mr_32_waveform_refusals`, `ur_21_txburst_refusals` and `rm_12_a_stream_node_names_its_direction_and_number`; the channel interleaving and the little-endian byte order by `mr_32_a_two_channel_waveform_is_channel_interleaved`; none named for a waveform refused for not having the `ezsdr.stream.cf32` contract.
- **RM-14** On receipt of a `TxBurst` a radio Provider evaluates `LatePolicy::decide(registry, at, now, device_lead)`, where `device_lead` is its **device lead** — its TimingEnvelope's `radio.timing.min_timed_command_lead_ns` less the delivery allowance it declares (MA-10), in `host.monotonic` (SC-27) — and `now` is its **decision instant**: the first transmit sample at or after the later of its current instant and its transmit clock's origin less that device lead, so that a target whose instant has passed is late even when it is the sample the current instant falls in, and a burst timed before its transmit clock's origin is late (RM-15). A stepped Provider's allowance is zero, so for MockRadio the device lead is the TimingEnvelope's lead. `OnTime`: the burst starts at `at`. `SendAsap`: the target moves to the first transmit sample at or after the decision instant plus the device lead; if admitted, `radio.TIME_ERROR { cause: late, outcome: send_asap }` is emitted.
  Under `SendAsap`, `BurstOpen.requested_target` is the burst's original `at`, or SC-23a's `requested_at` when admission already advanced it. If its Provider refuses that moved start, the burst is not transmitted and the Provider emits only `TIME_ERROR { outcome: refused }` plus `COMMAND_REJECTED` instead of a `send_asap` event. `Drop` and `PlanViolation`: the burst is not transmitted, and `radio.TIME_ERROR` with that outcome is emitted. The outcome reaches an admitted burst's `BurstRecord` through `BurstOpen.late` (SC-29a).
  *Producer obligation.*
  *Test:* `mr_17_a_target_whose_instant_has_passed_is_late_even_on_the_floor_sample`, `ur_21_a_burst_before_its_clock_s_origin_is_late`, `mr_16_burst_refusals`, `ur_21_first_dispatch_rechecks_all_late_policies`, `v58_09_a_pong_short_of_the_lead_is_late_as_on_hardware`, `rm_14_the_recorded_requested_target_is_the_requested_at` and `rm_14_the_handed_burst_keeps_its_requested_at` (SC-23a's `requested_at` kept as `requested_target` on time and after a `SendAsap` move, MockRadio and the UHD Module).
- **RM-15** Bursts on one transmit stream are held in target order. A burst ends at the end of its waveform (without `repeat`), at the start of the next burst, at a `Stop` for its stream or device, or at the Run's stop, whichever is first; a repeated burst ends only at one of the last three. A burst whose start equals a burst already held — the start its Provider's `LatePolicy` decided (RM-14) — is refused with `radio.COMMAND_REJECTED`. Every block a Provider transmits goes through the Kernel's `BurstTracker` (SC-29).
  A burst never starts before its transmit clock's origin: a target before it is late, and the burst's `LatePolicy` applies — `send_asap` moves it to the origin with `TIME_ERROR { send_asap }`, `drop` drops it. A burst timed before its transmit clock's origin meets this on a Provider whose start lead is not zero (a Session burst with no `at` does not: RS-19 admits it no earlier than that origin); its Provider decides such a burst as if at its clock's origin less its device lead.
  A Provider that hands a burst's samples to a device ahead of their instants, and cannot recall them, cannot start the next burst before the first sample it has not yet handed over. Its late bound is the later of RM-14's bound and that same-clock front; equality is on time. The Provider's spec states the in-flight window `radio.timing.tx_in_flight_ns`. When that window is nonzero, the device burst closes with an EOB send of at least two samples per channel: a one-sample payload close adds one zero, and a close with no payload sends two zeros. The next safe start is the closing send's exclusive end plus one sample. A zero window preserves the ordinary behavior.
  *Producer obligation.*
- **RM-16** A receive segment ends at its **cut**: the first sample at or after the instant of what ends it — a `Stop` of `<device>/rx/s<k>` or `<device>`, `Provider::stop`, or a device loss (RM-21) — and never before its origin or the first sample not yet delivered; under `abort`, the first sample not yet delivered. Nothing at or after the cut is delivered. A `Stop` takes effect at the instant the Provider handles it.
  On the transmit side a `Stop` of `<device>/tx/s<k>` or `<device>`, and `Provider::stop`, close the open burst at the first sample at or after the instant (`BurstEnd::Stop`), transmit every earlier sample, and cancel held bursts not handed to the device; a later burst is transmitted as usual (RM-15). A Provider that hands samples ahead closes at its current handed cursor, no later than `s + radio.timing.tx_in_flight_ns`; its closing EOB follows RM-15. A device loss closes and cancels the same way.
  A held `TxBurst` cancelled before its start by `Stop`, `Provider::stop` or loss is recorded in the Provider's `rejected` section and emits no `radio.COMMAND_REJECTED`.
  *Producer obligation.*
  *Test:* `rm_26_the_timeline_cases`, `mr_25_orderly_stop_ends_at_its_instant_abort_at_once`, `mr_25_a_stopped_stream_keeps_its_end`, `ur_26_a_stopped_stream_keeps_its_end`, `mr_25_a_stop_cuts_at_its_instant`, `rm_26_the_timeline_against_uhd_rx`, `mr_25_a_stopped_burst_transmits_every_sample_before_the_stop`, `mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter`, `mr_25_a_stop_transmits_every_held_burst_before_it`, `rm_12_a_stream_node_names_its_direction_and_number`, `cancellation_reports_only_cold_change`, `ur_26_shutdown_after_loss_cancels_held_bursts_record_only`, `vh_8_tx_stream_stop_cancels_held_bursts`, `vh_8_device_stop_cancels_held_bursts`, `vh_8_provider_stop_cancels_held_bursts`, `vh_8_loss_cancels_held_bursts_without_rejection`, `vh_8_rx_stop_does_not_cancel_held_tx`, `vh_8_tx_stop_allows_a_later_burst` and `vh_8_tx_stream_stop_leaves_the_other_stream_held`;
  a stopped burst's end within the profile in-flight window, its agreement with the cursor actually handed over, and the two-sample close and safe boundary by `ur_23_stop_preserves_handed_cursor_after_stop` and `ur_23_stop_closes_a_running_repeat_at_its_handed_cursor`.

### Receive faults

- **RM-17** An **overrun** at `f` loses every receive sample in `[f, f + radio.timing.overflow_restart_gap_ns)`. The block in progress ends at `k_f`, the first sample at or after `f`, and is delivered if it holds a sample. The next block begins at the first sample at or after `f + gap`; if a sample was lost it carries `GAP_BEFORE | RESTARTED` and `lost` equal to its whole time jump, any loss already pending included (SC-18). For a block with a jump, the reports since the preceding block produce at most one `radio.RX_OVERFLOW`, with `lost` the block's whole jump less any loss an earlier event already reported (the dropped-block overrun below), `cause: overrun` if any report was an overrun and otherwise `cause: sequence`, timed at the first sample of that loss in the receive `SampleClock`. A block without a jump consumes its pending reports without an event; a report with no following block produces no event. A jump with no report produces no event. An overrun the Provider causes by dropping a block its link refused is emitted when it drops it, outside this rule (SC-20a).
  *Producer obligation (Vision §23, §58 #6).*
  *Test:* `rm_17_reported_losses_make_one_payload_for_the_block_jump`, `mr_20a_the_overflow_comes_with_the_block_that_carries_it`, `mr_19_backpressure_is_an_overrun`, `vh_8_a_reported_overrun_matches_the_block_jump`, `vh_8_reports_before_one_block_make_one_overrun_summary`, `vh_8_repeated_overruns_before_one_block_make_one_summary`, `vh_8_a_report_without_a_following_block_emits_no_overflow`, `ur_17_a_missed_start_and_overrun_share_the_block_summary` and `ur_18_a_report_on_a_gapless_block_is_consumed_without_an_event`.
- **RM-18** A **sequence error** at `f` loses `radio.rx.block_len` samples from `k_f`; the next block carries `GAP_BEFORE | SEQ_DISCONTINUITY`, not `RESTARTED`, with `lost` its time jump. A sequence report contributes `cause: sequence` under RM-17 unless an overrun report precedes the block.
  *Producer obligation.*
  *Test:* `mr_22_sequence_error_shape`, `v58_06_sequence_error_is_seq_discontinuity`, `vh_8_a_reported_sequence_error_matches_the_block_jump` and `vh_8_reports_before_one_block_make_one_overrun_summary`.

### The RF safety envelope

- **RM-19** The section `radio.rf_envelope` is this document (`RfEnvelope`, `deny_unknown_fields`):

  ```text
  RfEnvelope { allowed_bands: [{ lo_hz: num, hi_hz: num }],          required; lo_hz ≤ hi_hz
               max_gain_db:   optional num,                          applies to every transmit channel
               tx_enabled:    optional bool | [bool],                absent: every transmit channel enabled
               antenna_ports: optional [string] }                    absent: any antenna
  ```

  `RfEnvelopeCheck` runs at `validate`, `prepare` and `runtime`, and for each fragment `F`, over `effective[F]` overlaid with `proposed[F]` (SB-30), reports one `Violation { check: radio.rf_envelope, key, requested, reason }` — `reason` beginning `"RM-19: <fragment>: "` — for each violation RM-33 defines.
  1. the section does not parse as `RfEnvelope`, or a band has `lo_hz > hi_hz` (one violation, `key: None`, for the whole check);
  2. a layout configuration violates a transmit-channel, receive-antenna or transmit-enable limit (RM-33).

  Receive frequencies are not limited. A numeric value is read as `Int` or `Num`; one of another kind is a violation of its key.

  No radio key has an update class (RM-21), so a Run-time change of a radio value never reaches the `runtime` point: the Kernel refuses it first (RS-52), and Vision §58's sixteenth acceptance test runs as that refusal. The `runtime` refusal of a value outside the envelope returns with the update classes of a stopped receive stream and an idle transmit stream (note 47 stages 3 and 4).
  *Test:* `rm_19_rf_envelope_cases`; the value outside the envelope refused at connect and the Run-time change refused for want of a class by `v58_16_a_value_outside_the_rf_envelope_is_refused_at_connect_and_a_runtime_change_has_no_class`.

### Schemas

- **RM-20** `RfEnvelope` and `RadioEnvelope` — the document a radio Provider writes into its Manifest section `<module>.envelope`, `{ profile: ProfileRef, timing: { min_timed_command_lead_ns, startup_latency_ns, command_queue_depth, release_window_ns, overflow_restart_gap_ns, rx_start_lead_ns, tx_start_lead_ns, tx_in_flight_ns }, performance: { rx_bytes_per_s, tx_bytes_per_s, wire_bytes_per_sample } }` — have committed schemas `schemas/radio/rf_envelope.v1.json` and `schemas/radio/envelope.v1.json` (PO-7); RM-22 adds eight payload schemas, RM-24 adds the schema of `RX_OVERFLOW`'s hot-path form, and RM-35 adds the measurement-record schema: twelve files. `envelope.v1.json`'s `timing` holds exactly these eight members; an envelope with another member, `stop_tail_ns`, `restart_lead_ns` or a single `start_lead_ns` among them, is refused, not reinterpreted.
  *Test:* `rm_20_schema_freeze` (the committed schemas, twelve files); none named for the refusal of an unknown member.

### Event payload types

- **RM-22** The payloads of RM-11 are these Rust types in `ezsdr_radio::payloads`, each `#[derive(Serialize, Deserialize, JsonSchema)]` with `#[serde(deny_unknown_fields)]`, and a radio Provider builds a control-path event's `payload` as `serde_json::to_value(&the payload)` (a hot-path `RX_OVERFLOW` carries RM-24's bytes instead):

  ```rust
  pub struct RxOverflowPayload     { pub cause: RxOverflowCause, pub lost: u64 }
  pub enum   RxOverflowCause       { Overrun, Sequence }                          // snake_case
  pub struct TimeErrorPayload      { pub cause: TimeErrorCause, pub outcome: TimeErrorOutcome, pub late_by_ns: i64, pub target: TimePoint }
  pub enum   TimeErrorCause        { Late, UnclosedBurst }                         // snake_case
  pub enum   TimeErrorOutcome      { SendAsap, Drop, PlanViolation, Refused, LateAtDevice } // snake_case: late_at_device
  pub struct LateCommandPayload    { pub key: Option<Key>, pub requested: TimePoint, pub applied: TimePoint }
  pub struct CommandQueueFullPayload { pub key: Key, pub depth: i64 }
  pub struct CommandRejectedPayload  { pub action: Option<ActionId>, pub kind: String, pub reason: String } // action skipped when None
  pub struct TxUnderflowPayload    { pub cause: TxUnderflowCause }
  pub enum   TxUnderflowCause      { Starved, Lost }                                   // snake_case
  pub struct AlignmentErrorPayload { pub lost: u64 }
  pub struct ClockLostPayload      { pub reference: ClockReference }
  pub enum   ClockReference        { Frequency }                                      // snake_case; `time` is additive when a Provider monitors its PPS
  ```

  Their schemas are committed as `schemas/radio/<name>.v1.json` with the names `rx_overflow_payload`, `time_error_payload`, `late_command_payload`, `command_queue_full_payload`, `command_rejected_payload`, `tx_underflow_payload`, `alignment_error_payload` and `clock_lost_payload` (PO-7), which is RS-31's "the payload schemas of non-Kernel kinds are their Vocabulary's".
  *Test:* `rm_20_schema_freeze` and `rm_22_payloads_round_trip` (each type serialises to the object RM-11 shows and deserialises back), and `rm_22_late_at_device_is_snake_case`.

### Update classes of radio keys

- **RM-21** No radio key has an update class (RM-27): the Kernel refuses a Session's `SetParameter` and `UpdateParameter` of one and a Spec schedule of one (RS-52, SB-16), so a channel's membership, antenna, frequency and gain and a stream's sample rate are set at `connect` or in the Spec's `requires` and are the same for the whole Run. A receive stream is **on** from the Run's start until a `Stop` of `<device>/rx/s<k>` or `<device>`, and again from a `start_rx` (RM-12); it **runs** while it is on, holds a channel and has a link, and is not halted by a refusal (RM-25). A stream holds the set of channels whose `radio.d.c<n>.stream` names it (RM-27); it holds a channel exactly when that set is not empty. A `Stop`, a `start_rx` or a loss changes whether a receive stream runs; its segment ends (RM-16) and, if it runs afterwards, the next one starts (RM-25). A transmit stream has a SampleClock while it holds a channel; a `Stop` ends its bursts, not its clock.
  *Producer obligation. Test:* `rm_26_the_timeline_cases`; the keys' having no update class by `rm_04_the_key_table_is_exactly_the_declared_one`.

### Path delay

- **RM-23** `radio.d.c<n>.path_delay_samples` is a channel's fixed path delay, in samples of its stream's current rate, declared as `One(Int(n))` with `n ≥ 0`: a transmit sample stamped `T` is at the antenna `n_tx` transmit samples after `T`, and a receive sample stamped `T` is what was at the antenna `n_rx` receive samples before `T`. A per-Run override by a CalibrationArtifact is later work.
  *Producer obligation.*
  *Test:* `mr_32_the_transmit_path_delay_shifts_a_loopback`; none named for the receive delay.

### Hot-path forms

- **RM-24** `radio.RX_OVERFLOW` has a hot-path form of 9 bytes, little-endian: byte 0 the cause (`0` overrun, `1` sequence), bytes 1–8 `lost` as u64. `RxOverflowPayload::to_hot(&self) -> [u8; 9]` writes it. `RxOverflowPayload::from_hot(bytes: &[u8]) -> Result<RxOverflowPayload, String>` reads it and refuses another length or a cause byte other than 0 or 1. `RxOverflowPayload::from_payload(payload: &serde_json::Value) -> Result<RxOverflowPayload, String>` reads a delivered event's payload in either form — RM-11's `{ cause, lost }` object or the array of byte values the Kernel's drain makes of a hot-path record (RS-34) — and refuses anything else, including the former object with `restart_gap_ns` and the former 17-byte array. `RxOverflowHotPayload`, the array as a Manifest holds it, has the committed schema `schemas/radio/rx_overflow_hot_payload.v1.json` (PO-7), so a reader that validates payloads has a schema for both forms. The Kernel does not decode it (RS-32a, withdrawn).
  *Ceilings: a drain delivers the ring's bodies before the control path's (RS-34), so the Manifest's `delivered` list is ordered by path, then by emission, and when two kinds of one drain both stop the Run the hot one is the recorded cause; a hot body the full ring drops is counted and reported in `EVENTS_DROPPED` but marks no artifact, since `mark_artifact` acts on delivered bodies (RS-30).*
  *Test:* `rm_24_the_hot_form_round_trips` and `rm_24_a_delivered_payload_reads_in_either_form`.

### The lattice

- **RM-25** Every SampleClock's origin is on its root's lattice — a whole multiple of the numerator, in lowest terms, of its `root_ticks_per_tick` — so clocks of one ratio share every sample instant and T0 is a sample instant of every stream. The first receive segment starts at T0 (MR-11, UR-15), the first transmit clock at the first lattice instant at or after `arm` (MR-9). A receive segment whose device misses its timed start keeps its origin: its first block carries `GAP_BEFORE` with `lost` its index (SC-13), so an index still names `origin + index · root_ticks_per_tick`. A later receive segment, which only a `start_rx` after a `Stop` begins, starts at the first lattice instant at or after its effective instant, the stream's previous cut plus `radio.timing.rx_start_lead_ns` and the instant the Provider is ready for it plus that lead; the Provider's spec states its ready instant, and a Mock profile emulating a device states that device's. After a cut, readiness includes the release window, one receive call at the old rate and the delivery allowance; the start lead follows those terms.
  Commands and faults of one stream take effect in the order of their effective instants and, at one instant, in the order received, faults — a device loss among them — counting as received at the Run's start; `Provider::stop` is a command and counts in the order it was received. A `start_rx` takes effect at its `at` or its receipt and never before the stream's start (RM-12). Between a cut and the next origin nothing is delivered and nothing is a gap.
  A stream's configuration, the set of channels it holds and its rate, is fixed for the Run (RM-21), and a stream whose set is empty begins no segment.
  A receive SampleClock is registered with the first block its segment publishes and ended at its cut, so a segment that publishes none — one cut at or before its origin, or one whose samples were all lost before a loss or an abort — has none; a transmit clock is registered at `arm`, so that a burst admitted after it can target it (KC-21a).
  A planned event refused or pre-empted at its instant — a configuration the device does not apply, a loss, `Provider::stop` — is removed; cuts made and clocks ended stand, and only the events after that instant are recomputed, a later one the new plan refuses being refused then (`COMMAND_REJECTED`). A refused `start_rx` — one whose configuration the device does not apply — **halts** the stream: it has no segment until the next `start_rx`, which starts it by the rule above; the refused value stays the configuration of record (KC-27). MockRadio refuses no `start_rx`.
  A receive segment that a refused `start_rx` would have begun counts, for planning, as running until the next `start_rx`, `Stop` or end cuts it, whatever `delivered` holds; that cut is the previous cut of the start rule above. The segment is not in the plan: its owner does not begin it, and it has no clock and no record. The stream halts as before.
  Both Providers use `ezsdr_radio::timeline` for stream planning; a layout instance's lines use `timeline::Lines`.
  A line may prune items before a boundary only when no later booking can take effect before it and no later refusal can reach a pruned item. It folds pruned items into its starting state, and pruning leaves the line's plan unchanged.
  *Producer obligation.*
  *Test:* `rm_26_the_timeline_cases`, `rm_25_a_refusal_does_not_move_a_later_origin`, `rm_25_a_refused_receive_segment_is_not_in_the_plan`, `rm_25_a_start_after_a_refusal_begins_from_the_refused_segment_s_cut`, `rm_25_a_virtual_segment_ignores_a_stale_delivered_index`, `rm_16_a_real_segment_keeps_its_delivered_floor`, `rm_25_a_pruned_line_with_a_refused_segment_plans_as_one_that_keeps_every_item`, `rm_25_a_booking_adds_no_coverage_before_its_start_lead`, `rm_26_each_stream_takes_its_own_directions_start_lead`, `rm_26_a_line_pruned_at_an_item_s_instant_keeps_it`, `rm_26_a_pruned_line_keeps_few_items`, `rm_26_a_pruned_line_plans_as_one_that_keeps_every_item`, `rm_26_a_refused_booking_leaves_the_line`,
   `rm_26_generated_sequences_keep_the_timeline_invariants`, `ur_15_every_clock_starts_on_its_lattice`, `ur_17_a_missed_start_restarts_with_a_gap`, `rm_25_a_receive_successor_adds_release_window_before_call_and_lead`, `rm_26_the_timeline_against_uhd_rx`;
  the first transmit clock at the first lattice instant at or after `arm` by `mr_09_the_transmit_clock_starts_on_its_lattice`.

- **RM-26** Withdrawn; timeline behavior is RM-25, and radio Providers use RM-29's layout description.

### The layout description

- **RM-27** A description built from a layout (RM-28) declares these keys beside RM-4's, each with its numbered segment at position 2, so that the declaration `radio.rx.c.frequency_hz` declares `radio.rx.c0.frequency_hz` and `radio.rx.c12.frequency_hz`, for `d ∈ {rx, tx}`:

  | Key | kind | coercible | coercion default | update class |
  |---|---|---|---|---|
  | `radio.d.c.frequency_hz` | num | yes | reject | — |
  | `radio.d.c.gain_db` | num | yes | warn | — |
  | `radio.d.c.antenna` | str | no | reject | — |
  | `radio.d.c.stream` | int | no | reject | — |
  | `radio.d.c.frequency_step_hz` | num | no | reject | — |
  | `radio.d.c.gain_step_db` | num | no | reject | — |
  | `radio.d.c.phase_behavior_on_retune` | str | no | reject | — |
  | `radio.d.c.path_delay_samples` | int | no | reject | — |
  | `radio.d.s.sample_rate_hz` | num | yes | reject | — |

  The concrete spellings are `radio.d.c<n>.<leaf>` and `radio.d.s<k>.sample_rate_hz`; the declaration's own spelling is not a key (SB-2a). No direction-wide coherence key is declared. No `radio.{rx,tx}.c<n>.{frequency_hz,gain_db,antenna,stream}` and no `radio.{rx,tx}.s<k>.sample_rate_hz` has an update class: a value is set at `connect` or in the Spec's `requires`, and the Kernel refuses a change during a Run (RS-52) and a schedule of one (SB-16).
  *Test:* `rm_27_the_layout_builder_keys_are_declared`; the two key sets' separation by `rm_30_coerce_cases` (RM-4's former unnumbered spellings, `radio.rx.channels` and the like, are no radio keys of a layout description).

- **RM-28** A layout description's selector reads `master_clock_hz`, `image`, `links`, `link_gbps`, `rx` and `tx`. Each of the first four is a list of one entry, one per timekeeper: `master_clock_hz` 200000000, `image` `hg` or `xg`, `links` 1 or 2, `link_gbps` 1 or 10; absent takes 200000000, `hg`, 1, and `link_gbps` the image's own speed, 1 for `hg` and 10 for `xg`. `rx` and `tx` are required, and are lists of `"<timekeeper>:<slot>:<board>:<frontend>"` with an optional `#<name>` (below), one entry per logical channel; an entry's position is the channel's number. The timekeeper is 0. The boards are `ubx`, `obx`, `cbx`, `basic_rx`, `lf_rx`, `basic_tx` and `lf_tx`; the first three carry both directions and the others one (`basic_rx` and `lf_rx` receive, `basic_tx` and `lf_tx` transmit). The front ends a board has are `0`, and `0` and `1` for `basic_rx` and `lf_rx`.

  The analysis refuses a layout when a board name is not one of the seven, an element is in a direction its board does not carry, a front end number is not one of the board's, a timekeeper is not 0, one (timekeeper, slot, direction) names two board kinds, one (timekeeper, slot, board, frontend) appears twice in a direction, a slot that holds a duplex board in one direction names another kind in the other, a shared list does not have one entry, or both lists are empty. An element's refusal names its position and its value: the selector's own spelling of the element where its parse refuses it, and the element's board, front end and slot where the layout's checks refuse it; the refusal of a duplex board's slot names the slot and both boards, and a refusal of a pair of boards names both. A daughterboard's clock is shared per direction across both slots, so the analysis consults the boards' clock-compatibility table for every pair of different board kinds on one timekeeper, across slots, and refuses the layout when the table does not admit a pair; the table admits every pair of boards this Vocabulary describes.

  An entry may end in `#<name>`, which names its channel: `"0:A:ubx:0#loop-rx"`. A name is non-empty, not only the digits 0-9, and holds no `:`, `#`, white space or control character; it is used once in a radio, across `rx` and `tx` together, and the refusal names the entry's position and its whole spelling. The layout, the description and everything a Provider builds from it are those of the entry without its `#<name>` — a name reaches no key, node or device call — and an entry without one is reached by its number only. The shared reader gives each logical channel's name as `channels::names_of`. A name is part of the binding (RS-25a), and a profile's names are unique across its radios (EA-10).
  *Test:* `rm_28_a_selector_name_does_not_change_the_layout`, `rm_28_the_name_rule_cases`, `rm_28_a_name_is_used_once_in_a_radio`, `rm_28_a_number_is_a_canonical_ascii_decimal`.

  The `ideal` layout has no motherboard: its `rx` and `tx` are integers 0…64, its channel counts, 1 and 0 when absent; it names no channel.
  *Test:* `rm_28_the_layout_analysis_cases`.

- **RM-29** The description built from a layout has one stream node per logical channel in each direction: the root is `radio.device`, with a child `<device>/rx/s<k>` of kind `radio.rx_stream` for each receive channel and `<device>/tx/s<k>` of kind `radio.tx_stream` for each transmit channel, and the root declares the port `rx<k>` (direction `out`, contract `ezsdr.stream.cf32`) for each receive channel. A stream is the set of channels whose `stream` is `k` (RM-30), which is not the node's count. It builds no channel node and no timekeeper node. The root's capabilities are, for each channel `n` of direction `d`, `radio.d.c<n>.frequency_hz` as a `Range`, `gain_db` as a `Range`, `antenna` as an `AnyOf`, `stream` as a `Range` of −1…=channel count − 1, `frequency_step_hz` and `gain_step_db` as `One` of the front end's step, `phase_behavior_on_retune` as `One` and `path_delay_samples` as `One`; and, for each stream `k`, `radio.d.s<k>.sample_rate_hz` as RM-8's rate grid. The instance-wide capabilities are `radio.full_duplex`, `radio.hardware_time`, `radio.tx.repeat_max_samples`, `radio.tx.repeat_align_samples`, `radio.rx.block_len`, the eight `radio.timing.*` and the three `radio.perf.*`, with the Provider's values; `ideal` declares `radio.timing.tx_in_flight_ns` as 0.

  The front ends' values are:

  | board | tuning | gain | antennas (receive; transmit) | default antenna | phase | default frequency |
  |---|---|---|---|---|---|---|
  | `ubx` | 10 MHz…6 GHz, step 1 Hz | 0…31.5 dB, step 0.5 dB | `RX2`, `TX/RX`; `TX/RX` | `RX2`; `TX/RX` | `random_unless_timed_tune` | 1 GHz |
  | `obx` | 10 MHz…8.4 GHz, step 1 Hz | 0…31.5 dB, step 0.5 dB | `RX2`, `TX/RX`; `TX/RX` | `RX2`; `TX/RX` | `random_unless_timed_tune` | 1 GHz |
  | `cbx` | 1.2…6 GHz, step 1 Hz | 0…31.5 dB, step 0.5 dB | `RX2`, `TX/RX`; `TX/RX` | `RX2`; `TX/RX` | `random` | 2.45 GHz |
  | `basic_rx`, `basic_tx` | 0…`master_clock_hz / 2`, step 1 Hz | `{0}` | `A`, `B`, `AB`, `BA` | `AB` | `random_unless_timed_tune` | 0 Hz |
  | `lf_rx`, `lf_tx` | 0…32 MHz, step 1 Hz | `{0}` | `A`, `B`, `AB`, `BA` | `AB` | `random_unless_timed_tune` | 0 Hz |

  The path delay is 45 transmit samples and 0 receive samples for every board.

  The `ideal` description has no motherboard: its channels' frequency is a `Range` 0…10¹² Hz with step 0, its gain a range −200…200 dB with step 0, its rate grid every integer hertz in 1…10⁹, its receive antennas `RX2` and `TX/RX`, its transmit antennas `TX/RX`, its phase behaviour `deterministic`, its path delay 0, its timing values 0 except a command-queue depth of 2³²−1, its repeat bounds 2³²−1 samples and 1 sample, its throughput 2⁶² bytes per second per direction with 8 wire bytes per sample, and its profile `ideal` 2.0.0.
  *Test:* `rm_29_the_layout_description_builds_the_stream_tree`, `rm_29_the_ideal_layout_builds_its_tree_and_defaults`, `d3_the_two_providers_describe_one_layout_alike`, `d3_the_layout_decides_the_channels_on_both_providers` and `d3_a_layout_without_receive_channels_requests_none_on_both_providers`; the rate grid by `x3xx_the_decimation_set_is_the_ddc_s`.

- **RM-30** A layout description's `coerce` is pure (MA-11). Its `effective` holds exactly RM-27's configuration keys: for every channel its `frequency_hz`, `gain_db`, `antenna` and `stream`, and for every stream its `sample_rate_hz`. The defaults are every channel of both directions in no stream (a Session puts a channel in a stream by asking at `connect`, RS-12), each channel's frequency and antenna as its front end's default and its gain 0, and every stream 1 Msps. A key that names a channel or stream the layout does not have is refused as `RM-4: no such channel c<n>` or `RM-4: no such stream s<k>`; a key that is neither RM-27's nor RM-29's instance capability key is refused as `RM-4: not a radio key`. A numeric value is coerced to its channel's or stream's grid by RM-8; an antenna is one of its channel's names by RM-8; `stream` is an integer in −1…=channel count − 1. RM-29's capability-only keys are matched against their declaration. `Description::violations(&self, configuration: &BTreeMap<Key, Value>) -> Vec<(Key, String)>` judges a whole configuration on RM-31 and RM-32 and reports every violation with the key that violation names, whether or not a request names that key; `coerce` judges the configuration the request leaves with it on that function and charges each violation to a key of the request (RM-31, RM-32), so that the instance-wide constraints hold of a one-key update as well.
  *Test:* `rm_30_coerce_cases`, `rm_30_default_antennas_meet_rm_31_on_every_board`, `rm_31_full_duplex_cases`, `rm_32_throughput_cases`.

- **RM-31** While a transmit channel of a duplex board is in a stream, every receive channel of that board — the same timekeeper and slot — has antenna `RX2`. `violations` reports such a configuration naming the receive channel's `radio.rx.c<n>.antenna` key, with no request needed; `coerce` refuses the request naming that key or, when the request does not name it, the transmit channel's `radio.tx.c<n>.stream` key, and the reason names the board and the transmit channel.
  *Test:* `rm_31_full_duplex_cases`.

- **RM-32** A layout description's configuration is admissible, in each direction `d`, only when `Σ_k (m_k · rate_k) · radio.perf.wire_bytes_per_sample ≤ radio.perf.d_bytes_per_s`, where `m_k` is the number of channels whose `stream` is `k` and `rate_k` is `radio.d.s<k>.sample_rate_hz`. The limit is one link's bytes per second whatever `links` names. `violations` reports a direction that exceeds, naming the `radio.d.s<k>.sample_rate_hz` key of that direction's lowest-numbered stream that holds a channel, with no request needed; `coerce` refuses the request on the key `violations` named when the request names it, otherwise on the sample-rate key of the lowest-numbered stream of that direction it names, otherwise on the `stream` key of the lowest-numbered channel of that direction it names, and otherwise on the key `violations` named. The reason names the direction, the need and the limit.
  *Test:* `rm_32_throughput_cases`; none named for the reason's limit and for the transmit direction in it.

- **RM-33** A configuration that holds an RM-27 channel key is judged by RM-19 as follows. For each transmit channel `c<n>` whose `stream` is 0 or more: `radio.tx.c<n>.frequency_hz`, when present, is in an allowed band; `radio.tx.c<n>.gain_db`, when present, is at most `max_gain_db`; `tx_enabled` is true for the channel's logical number `n`, or, for a list, its entry `n` is present and true; and `radio.tx.c<n>.antenna`, when present, is in `antenna_ports`. For each receive channel whose `stream` is 0 or more, `radio.rx.c<n>.antenna`, when present, is in `antenna_ports`. Each violation names that channel's key, except the enable violation, which names `radio.tx.c<n>.stream`; a `frequency_hz` or `gain_db` value of another kind, and an `antenna` that is not a string, are violations of their keys. A channel whose `stream` is negative is not judged. `RfEnvelope`'s shape is unchanged.
  *Test:* `rm_33_rm_19_per_channel_cases`; none named for a `gain_db` value of another kind and for an `antenna` that is not a string.

- **RM-34** Withdrawn (note 47 stage 2): moving a channel between streams in one operation of `timeline::Lines`, and its release.

- **RM-35** A measurement record is `ezsdr_radio::measurement::Record` with schema identifier `ezsdr.radio.measurement.v1`; deserialization refuses another identifier. It carries the binding selector, profile reference, device and host identity, commit, load, operating conditions, comparable capability values and non-capability observations.
  `rule(key)` resolves numbered channel and stream keys at their concrete indices and assigns each capability this comparison:

  | Rule | Relation (`p`: profile, `m`: measured) | Keys |
  |---|---|---|
  | `LowerBound` | `p ≥ m` | `radio.timing.{min_timed_command_lead_ns,startup_latency_ns,overflow_restart_gap_ns,rx_start_lead_ns,tx_start_lead_ns,tx_in_flight_ns,release_window_ns}`; `radio.{rx,tx}.c<n>.{frequency_step_hz,gain_step_db}` |
  | `UpperBound` | `p ≤ m` | `radio.timing.command_queue_depth`; `radio.perf.{rx,tx}_bytes_per_s`; `radio.tx.repeat_max_samples` |
  | `Subset` | the profile's supported set is a subset of the measured set | `radio.{rx,tx}.c<n>.{frequency_hz,gain_db,antenna}`; `radio.{rx,tx}.s<k>.sample_rate_hz` |
  | `Multiple` | `p` is a positive multiple of `m` | `radio.tx.repeat_align_samples` |
  | `Guarantee` | `p` is no stronger than `m` (`random` < `random_unless_timed_tune` < `deterministic`) | `radio.{rx,tx}.c<n>.phase_behavior_on_retune` |
  | `TrueImplies` | a true `p` requires a true `m` | `radio.full_duplex`, `radio.hardware_time` |
  | `Equal` | `p = m` | `radio.perf.wire_bytes_per_sample`; `radio.{rx,tx}.c<n>.path_delay_samples` |
  | `NotCompared` | excluded from `values` | `radio.{rx,tx}.c<n>.stream`; `radio.rx.block_len` |

  Numeric comparisons and grid membership reject values with absolute magnitude at or above 2^53 rather than comparing rounded `f64` values. `compare(record, description)` checks source and profile, exact coverage of comparable keys, and each measured value under its rule. Condition ids and `(direction, stream)` pairs are unique; stream and channel numbers are within the direction's layout, no channel appears more than once per direction, rates belong to the profile grid, `concurrent` references declared streams, and value and observation condition ids are declared. Measured values and observations require nonempty test names and positive trial counts; observations also require a name. An unmeasured value has a reason. The acceptance comparison checks every record against MockRadio and the UHD Module with each Provider's terms, using MockRadio's `BLOCK_LEN` and UHD's `DEFAULT_BLOCK_LEN`.
  *Test:* `rm_35_every_profile_capability_has_one_rule_and_one_value`, `rm_35_numbered_channel_and_stream_keys_reach_the_second_layout_entry`, `rm_35_measurement_fixture_round_trips_and_accepts_unmeasured_values`, `rm_35_each_value_rule_accepts_equal_or_stricter_claims_and_rejects_looser_ones`, `rm_35_invalid_comparison_fixture_is_rejected_for_each_rule`, `rm_35_stream_rate_grid_containment_is_checked`, `rm_35_step_and_values_grids_compare_by_all_members`, `rm_35_singleton_step_grid_is_contained_without_a_step_ratio`, `rm_35_integer_profile_grid_compares_values_integer_and_step_measurements`, `rm_35_rejects_numeric_values_at_the_exact_f64_limit`, `rm_35_integer_grid_containment_rejects_inexact_endpoints`, `rm_35_values_must_cover_each_comparable_capability_once`, `rm_35_schema_source_and_profile_are_checked`, `rm_35_accepts_observation_only_records_with_unmeasured_values`, `rm_35_value_and_observation_metadata_is_validated`, `rm_35_conditions_and_references_match_the_layout`, `rm_35_condition_validation_keeps_direction_in_stream_channel_and_concurrent_checks`, `rm_35_a_channel_cannot_belong_to_two_streams_in_one_direction`, `rm_35_each_provider_is_compared_with_its_own_description`, `rm_35_rule_labels_match_the_declared_comparison_classes`, `measurement_files_compare_against_mock_and_uhd_descriptions`, `measurement_fixture_files_enforce_provider_specific_rejections`, `measurement_between_the_provider_leads_passes_only_uhd` and `rm_20_schema_freeze`.

- **RM-36** Withdrawn (note 47 stage 2): `timeline::covering`.

## 5. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| R1 | Where per-channel configuration lives | Per-channel keys on the instance root; `radio.d.c<n>.stream` assigns a channel and `radio.d.s<k>.sample_rate_hz` configures its stream (RM-27) | A fragment per channel (membership could not join fragments); list-valued keys (SB-5, KC-24 step 8) | none |
| R2 | Channel membership by default | No channel, receive or transmit, belongs to a stream; a Session asks for the channels it uses at `connect` (RM-30, RS-12) | A default receive channel 0 and, for symmetry, a default transmit channel (every Run would configure and open a transmitter nobody asked for) | A Session names its streams at `connect` |
| R3 | Envelopes | Capabilities (Y6) | A document the Kernel parses (OV-21) | A new envelope value is an additive capability key |
| R4 | Coercion ties | Toward the lower grid value (RM-8) | Toward the higher (would round a rate up past a transport limit) | none |
| R5 | Out-of-range values | Rejected, never clamped | Clamping to the range edge (silently changes a frequency by gigahertz) | none |
| R6 | Late burst events | `TIME_ERROR` for bursts, `LATE_COMMAND` for commands (spec 04 §9 issue 2) | One `LATE` event (Vision §22's earlier wording; `LATE` is a block flag) | none |
| R7 | Where RX restart behaviour is stated | In the Vocabulary (RM-17, RM-18), so MockRadio and the UHD Provider share it | Per Provider (Mock and hardware would diverge, which Vision §17 forbids) | Phase 8 measures the gap |
| R8 | The RF check's scope | Transmission, plus antenna ports (RM-19) | Receive frequencies too (Vision §52 is about emission; a receive-only experiment outside a licensed band is legal) | A site that needs receive limits adds a key |
| R9 | TX port | Reserved, not declared (RM-2) | Declaring it now (nothing in Phase 2 could feed it, and a declared port no Provider serves is a rule with no carrier) | Phase 10 declares it |
| R10 | Where a channel's and a stream's configuration lives | On the instance's root, under a numbered key (`radio.d.c<n>.*`, `radio.d.s<k>.sample_rate_hz`), so one pure `coerce` judges the whole instance (RM-27, RM-30) | A fragment per channel (the stream's membership could not join them); a list-valued key (SB-5, KC-24 step 8) | none |
| R11 | The description's source | Build every Provider description from its binding layout (`ezsdr_radio::channels`) | Keep a second, direction-wide description path | none |
| R12 | Full duplex on a duplex board | The receive antenna is `RX2` while a transmit channel of that board is in a stream (RM-31) | Leaving the constraint to the device (a Mock looser than the hardware, Vision §59) | none |
| R13 | The throughput limit with two links | One link's bytes per second, whatever `links` says (RM-32) | `links × one link` (UHD picks a link by allocation count, not by load) | Per-link assignment when the UHD Module can name a link |
| R14 | Basic/LF phase behaviour before a measurement | `random_unless_timed_tune`, the weaker guarantee (RM-29) | `deterministic` (inferred from the DDC's tune alone) | A timed-tune measurement may strengthen it |

## 6. Tests

In `crates/ezsdr-radio/tests/radio_model.rs`.

| test | input | expected | rules |
|---|---|---|---|
| `rm_01_register_adds_the_descriptor_the_check_and_the_kinds` | empty registries, then `register` | the Vocabulary is `radio 2.0.0`; `checks.run` on a present `radio.rf_envelope` section runs the check; each RM-10 kind is registered with its default | RM-1, VF-4, VF-6, VG-1 |
| `rm_01_register_twice_is_refused` | `register` called twice | the second returns an error; nothing is overwritten | RM-1, MA-32, RS-27 |
| `rm_04_the_key_table_is_exactly_the_declared_one` | `vocabulary().keys` | exactly the fifteen RM-4 instance-wide rows and eighteen RM-27 layout rows, each field as the table says | RM-4, RM-27, RM-23, VF-6 |
| `rm_10_the_kinds_are_registered_under_radio_with_their_defaults` | `vocabulary().event_kinds` | exactly RM-10's nine, severities and defaults as the table says, each under `radio.` | RM-10 |
| `rm_12_the_verbs_compile_as_declared` | `vocabulary().verbs` | `start_repeat` → repeat, `send_asap_and_flag`; `send` → no repeat, `drop_and_flag`; `start_rx` → `Command {}`; the Kernel registry accepts all three (no `RejectAtPlan`) | RM-12 |
| `rm_19_rf_envelope_cases` | a band `[2.4e9, 2.5e9]`, `max_gain_db: 20`, `tx_enabled: [true, false]`, `antenna_ports: ["TX/RX", "RX2"]`; layouts with one or two transmit channels, an out-of-band frequency, excessive gain, an unlisted antenna, an unstreamed channel with out-of-range settings, an invalid receive antenna, and two fragments with only one violating | no violation for an in-band layout; one per out-of-band frequency, gain, disabled transmit channel and antenna; no violation for the unstreamed channel's settings; one on an invalid RX antenna; the violation names only the violating fragment | RM-19, SB-30 |
| `rm_19_a_malformed_section_is_one_violation` | `{ allowed_bands: "x" }`; a band with `lo > hi`; an unknown field | one violation each, `key: None` | RM-19 |
| `rm_19_the_proposed_value_is_judged` | at `runtime`, `effective` in band and `proposed` frequency out of band | one violation carrying the proposed value | RM-19, SB-30 |
| `rm_20_schema_freeze` | regenerate `RfEnvelope`, `RadioEnvelope`, the eight RM-22 payloads, RM-24's hot form and RM-35's measurement record | byte-equal to `schemas/radio/*.v1.json` (twelve files); no other file there | RM-20, RM-22, RM-24, RM-35, PO-7 |
| `rm_22_payloads_round_trip` | one value of each RM-22 type | `to_value` gives exactly RM-11's members with snake_case enum strings; `from_value` gives the value back; an extra member is refused | RM-22, RM-11 |
| `rm_22_late_at_device_is_snake_case` | `TimeErrorOutcome::LateAtDevice` | serialises as `"late_at_device"` | RM-22, RM-11 |
| `rm_17_reported_losses_make_one_payload_for_the_block_jump` | no reports with a jump; reports without a jump; sequence reports; mixed and repeated reports with a block jump | no payload for no report or no jump; one payload whose `lost` is the whole jump and whose cause is overrun if present, otherwise sequence | RM-17 |
| `rm_24_the_hot_form_round_trips` | an overrun with `lost` 50 000; a sequence error with `lost` `u64::MAX`; an overrun with `lost` 0; 8-, 10- and 17-byte slices; a cause byte of 2 | the first's 9 bytes pinned byte for byte; `from_hot(to_hot(p)) == p`; four refusals naming RM-24 | RM-24 |
| `rm_24_a_delivered_payload_reads_in_either_form` | the `{ cause, lost }` object; `p.to_hot()` as a JSON array of integers; that array with a 256; the former object with `restart_gap_ns`; a string | `p` in object and array form; three refusals | RM-24, RM-11 |

In `crates/ezsdr-radio/tests/channels.rs`:

| test | input | expected | rules |
|---|---|---|---|
| `rm_27_the_layout_builder_keys_are_declared` | the registered vocabulary; `key_decl` of `radio.rx.c3.frequency_hz`, `radio.rx.c.frequency_hz` and `radio.rx.c03.frequency_hz`; RM-27's nine rows in both directions | the numbered declaration and its fields; the declaration's own spelling and a leading-zero index are no keys; the concrete-spelling helpers | RM-27 |
| `rm_28_the_layout_analysis_cases` | the owner's mixed layout; a slot-B-only layout; a reordered layout; each refusal (an unknown board, a wrong direction, `basic_tx:1`, `ubx:1`, two kinds in one slot and direction, a duplex board's slot, a repeated front end, timekeeper 1, slot C, an empty layout, a two-entry shared list); the `ideal` counts | the parsed elements; each element's refusal names its position and its value and a duplex board's slot its slot and both boards; the mixed layout is admitted through the clock-compatibility table, which admits OBX, CBX and the Basic/LF boards; the shared lists must have one entry; `ideal` 3 and 1, the defaults 1 and 0, 65 and −1 refused | RM-28 |
| `rm_29_the_layout_description_builds_the_stream_tree` | the owner's layout, and an OBX and a CBX layout | `rx/s0…s2` and `tx/s0…s1`; ports `rx0…rx2`; the UBX, BasicRX and LFTX capability values; per-channel `stream`, phase and path delay; the DDC rate grid; no channel or timekeeper node, no direction-wide configuration or coherence capability; the OBX's frequency range and default, and the CBX's ranges, phase and default (the OBX's phase: `ur_09_the_obx_is_one_channel_from_10_mhz_to_8_4_ghz_defaulting_to_1_ghz`) | RM-29 |
| `rm_29_the_ideal_layout_builds_its_tree_and_defaults` | `ideal` with 3 receive and 1 transmit channel | the tree and ports; no grid; the integer-hertz rate range; `deterministic`; RM-30's defaults for every configuration key | RM-29, RM-30 |
| `rm_30_coerce_cases` | `Present` of every configuration key; `c3` and `s3`; BasicRX 10 MHz and 200 MHz; BasicRX 10 dB; N = 257, 258 and 2040; `stream` −1, 2 and 3; antenna `A` and `RX2`; path delay 45 and 46; RM-4's former unnumbered spellings | the layout's defaults; `RM-4: no such channel c3` and `no such stream s3`; 10 MHz applied and 200 MHz refused; 10 dB refused without a coercion; two exact rates and a coercion for N = 257; the membership and antenna refusals; `RM-4: not a radio key` | RM-30, RM-8 |
| `rm_31_full_duplex_cases` | a UBX in slot A with a BasicRX in slot B: a transmit channel in a stream with receive antenna `TX/RX` and with `RX2`; the transmit channel out of every stream; a UBX in another slot; BasicRX and LFTX in one slot; `violations` of a whole configuration whose transmit channel is in a stream and whose receive antenna is `TX/RX`, the same one key at a time, and the same without `TX/RX` or without the membership | one refusal naming the board and channel c0 (that the reason says `transmit channel c0` is asserted on the runtime path by §§; none named for `coerce`); acceptance in the other cases; the one violation names the receive antenna key and the other configurations give none | RM-31 |
| `rm_32_throughput_cases` | one and two channels at 200 Msps on a two-link selector; two streams of 200 Msps, of 200 + 100 and of 200 + 50 Msps; two transmit streams of 200 Msps; one channel at 200 Msps and at 20 Msps on a 100 MB/s link; two channels at 20 Msps; a membership-only change on a 4 B/s link; a request that names no key of the violation; `violations` of the 200 + 200 and 200 + 50 configurations | 800 MB/s admitted, 1.6 GB/s refused on the rate key; the 200 + 100 sum refused and the 200 + 50 sum admitted, on the direction's first stream's rate key; the transmit side refused; the 1 GbE cases as the sum decides; the membership-only change refused on its membership key and the nameless violation on its own key; the 200 + 200 configuration's one violation and the 200 + 50 one none | RM-32 |
| `rm_33_rm_19_per_channel_cases` | `tx_enabled` `[true, false]` and `[false, true]` with only `c1` in a stream; a channel out of band, over gain and on an unlisted antenna; an idle channel out of band; receive antennas in and out of a stream; a string frequency | the logical channel's enable state; one violation per channel key; nothing for an idle channel; the receive antenna key; the kind violation | RM-33, RM-19 |

In `crates/ezsdr-radio/tests/timeline.rs` (RM-12's, RM-21's and RM-25's tests):

| test | input | expected | rules |
|---|---|---|---|
| `rm_26_the_timeline_cases` | Appendix C's cases of spec 22 that need no Run-time change: the ready term, the transmit side, a refused `start_rx`, the delivered floor, a loss | each plan's segments, `(origin, ended)` | RM-16, RM-21, RM-25 |
| `rm_26_each_stream_takes_its_own_directions_start_lead` | a TimingEnvelope with 50 ms receive and 20 ms transmit leads, and a restart after a cut on each direction | `timeline::start_lead_ns` gives each direction its own, and each new segment's origin is its own direction's lead after the cut | RM-25 |
| `rm_26_generated_sequences_keep_the_timeline_invariants` | VH-8's 1 000 seeded sequences on both streams, every fifth start refused | RM-16's and RM-25's invariants; the plan does not depend on the order of its items | RM-16, RM-25 |
| `rm_12_a_stream_node_names_its_direction_and_number` | `<device>/rx/s0` and `<device>/tx/s12`; `rx`, `rx/s`, `rx/s01`, `rx/c0`, a deeper id, another device | the direction and number, and nothing for each refusal | RM-12, RM-13, RM-16 |

In `crates/ezsdr-radio/tests/device.rs`:

| test | input | expected | rules |
|---|---|---|---|
| `x3xx_the_decimation_set_is_the_ddc_s` | `decimations()`, `rates()`, `decimation` and `clip_decimation` | 639 values with `1`, 255 and 2040 in them and 257, 2041 and 4080 out; an ascending rate grid `200e6 / N`; an exact `decimation`; UHD's `coerce_decim` clipping | RM-29 |
| `ur_13_the_rate_set_comparison_expands_ranges_and_ignores_order` | device rate ranges and a `Grid` | the finite rates compare as sets, independently of order | RM-29, UR-13 |

The deleted generic-description tests are replaced on the layout path by `rm_28_the_layout_analysis_cases`, `rm_29_the_layout_description_builds_the_stream_tree`, `rm_29_the_ideal_layout_builds_its_tree_and_defaults`, `rm_30_coerce_cases`, `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`, `ur_09_profile_values_reach_the_capabilities_and_the_envelope_section`, and the acceptance tests that exercise both Providers.

## 7. Vision issues found

1. **Resolved:** Vision §8's example uses per-channel membership and phase-behavior keys and per-stream rate keys (RM-27).
2. **Resolved:** Vision §13 says the Radio Model Vocabulary defines coercion rules and that supported stream-rate grids are declared as capabilities, not as an envelope member (RM-8).
3. **Resolved:** Vision §22 now says `TxBurst.format` is determined by the transmit stream's Stream Contract and that the burst uses every channel of that stream (RM-13).
4. **Resolved:** Vision §26 says phase behavior is declared per logical channel and MockRadio emulates it (RM-9, MR-34).

## 8. Deferred

A transmit port and link-fed transmission (Phase 10). DDC/DUC and decimation capabilities, device FFT (spec 18's Not in scope). A per-Run delay override by a CalibrationArtifact (Vision §26). A BasicRX/BasicTX tuning upper bound is the DDC's range, `master_clock_hz / 2`; UHD's overall range reaches 350 MHz at 200 MHz, which the DDC's ±100 MHz does not confirm (INFERRED; a bench measurement decides). An LFRX/LFTX tunes within 0…32 MHz, the board's own range; UHD's overall range is wider (0…64 MHz for the two-input modes), so this bound can refuse a tuning the device may accept (INFERRED; the board's range is what the bench confirms). Basic/LF phase behaviour `random_unless_timed_tune` stands in for the inferred `deterministic` until a timed-tune measurement confirms the stronger value (INFERRED).

## Changes

| date | rules | change | record |
|---|---|---|---|
| 2026-10-07 | RM-1, RM-4, RM-6, RM-12, RM-16, RM-17, RM-18, RM-20, RM-21, RM-25, RM-26 | `radio` 2.0.0: one stream timeline, `ezsdr_radio::timeline`; no stop tail and one start lead (`stop_tail_ns` and `restart_lead_ns` removed); `start_rx`; a fault recorded when it fires | [spec 22](../plan/maintenance/22-timing-simplification.md) |
| 2026-10-08 | RM-25 | Spec 22 step 3: when a receive SampleClock is registered is stated per Provider — MockRadio when the segment's first sample instant has passed, the UHD Module with its first block — so a segment whose samples were all lost before a loss or an abort may have none | [spec 22](../plan/maintenance/22-timing-simplification.md) |
| 2026-10-08 | RM-15 | a Session burst with no `at` is admitted no earlier than its clock's origin (RS-19), so only a burst timed before the origin is late there | owner decision, 2026-10-08 |
| 2026-10-08 | RM-25, RM-26 | One receive-clock registration rule on both Providers: a receive SampleClock is registered with the first block its segment publishes; a timeline's items may be pruned without changing its plan (#63) | [note 23](../plan/maintenance/23-one-line-per-stream.md) |
| 2026-10-09 | RM-25 | A stream halted by a refusal keeps the refused value as its configuration of record and has no channel for a `hardware_timed` update until it starts again (#64); only the UHD Module exercises the rule for now, since MockRadio cannot refuse yet (#59) | owner decision, 2026-10-09 |
| 2026-10-10 | RM-25 | a receive segment whose device misses its timed start keeps its origin, its first block carrying `GAP_BEFORE` with `lost` its index (moved from TM-13b) | [audit item 6](../plan/maintenance/24-prefreeze-audit.md), owner decision 2026-10-09 |
| 2026-10-10 | RM-12 | `CompileRule::PeripheralCommand` is `CompileRule::Command` (#55); no change of rule | [audit item 7](../plan/maintenance/24-prefreeze-audit.md), owner decision 2026-10-09 |
| 2026-10-11 | RM-11, RM-22 | `radio.COMMAND_REJECTED` names the refused Action's id as `action` when it is refused on receipt, and its kind tag as `kind`; MockRadio and UHD alike | [spec 27](../plan/maintenance/27-role-typed-identities.md) §3 |
| 2026-10-12 | RM-2, RM-26 | the device node declares its host memory domain as `memory_domains` (MA-10), its portless stream children none; `DeviceDescription::tree` takes the domain from its caller | [spec 29](../plan/maintenance/29-amendments.md), pre-freeze item 3 |
| 2026-10-14 | RM-26, §6 | RM-26 names the current Providers (MockRadio 2.0.0, `ezsdr.radio.uhd` 0.4.0), and the `rm_01_…` row says the Vocabulary is `radio` 2.0.0 (was 1.5.0) (documentation fixes D1/D7/D8/D9, 2026-10-14) | documentation fixes, 2026-10-14 |
| 2026-10-14 | all; RM-1, RM-2, RM-3, RM-4, RM-5, RM-6, RM-7, RM-8, RM-9, RM-10, RM-11, RM-12, RM-13, RM-14, RM-15, RM-16, RM-19, RM-20, RM-21, RM-22, RM-23, RM-24, RM-25, RM-26, §3, §6, §7, §8, the header | rewritten to spec 22's DA-1 format: rationale, history, phase labels, review ids and finding ids leave the rules, each rule's test line moves to its own line under it, and the full text as it last stood is at commit `6ae88f3`; RM-1's per-version amendment list and RM-6's, RM-9's, RM-20's and RM-26's phase and review records leave the rules; RM-13 becomes the one home of the `TxBurst` refusals both Providers state alike (an outside target, a transmit channel count of 0, a `size_bytes` that is not a positive multiple of `8 · channels`, a non-finite component, the repeat constraints, non-empty `metadata`, a start before the Provider's synchronisation end; a start equal to a held burst's stays RM-15; the waveform-format obligation is inherited from the base text and keeps its not-transmitted / `COMMAND_REJECTED` meaning, since neither Provider has a branch that detects it, both read the bytes as this format) and names `mr_16_burst_refusals` and `ur_21_txburst_refusals`; RM-14 states the decision instant once (the first transmit sample at or after the later of the current instant and the clock's origin less the device lead), which MR-17's `now_up` and UR-21's `now'` are, and names the tests that pin it; RM-11 names the source node of every kind RM-10 registers, says `radio.TX_DISCONTINUITY` has no emitting Provider yet (SC-24a's obligation), and names the tests that pin three sources; RM-15 says a held burst's duplicate is judged on the start its `LatePolicy` decided, not its target, as MR-16 and UR-21 do; RM-2's device node is the root of MockRadio's and the UHD Provider's trees; RM-13's, RM-2's and the model's `in 1.0.0` labels drop (the conditions hold today); the withdrawn `KA` citations point at the rules that now hold them — RM-3's `KA-2` and RM-16's `KA-12` at TM-13a and RS-6, RM-6's `KA-7` at MA-10, RM-7's `KA-5` and `KA-6` at SB-7 and KC-26, RM-19's and §3's `KA-4`, and §6's `rm_19_…` row, at SB-30; `ezsdr_radio::timeline::decided_at`'s doc comment cites RM-14 and RM-15; RM-20's schema-regeneration and `SCHEMA_CHANGELOG` sentences move here; §8's phase pointers name spec 18 or the phase still planned, and §8's deferred hot-path payload item is deleted, since RM-24 defines `RX_OVERFLOW`'s form and RM-11 limits the hot path to that kind; §1 and the header's Status and Not-in-scope rows name specs 09 and 18 instead of phases, and §7's Vision issues 1–4 are stubs naming the Vision section that now says each; RM-12's test line names `mr_29_start_rx_is_the_one_command` where it pins the target, the empty params and the restart after a `Stop`, and RM-20's names the committed schemas and no test for the refusal of an unknown member; no other change of behaviour | [spec 22](../plan/maintenance/22-timing-simplification.md) § DA-3 — The rollout plan (step 2), documentation audit D5/D11, 2026-10-14, this stage's brief |
| 2026-10-14 | RM-14 | RM-14 gains the `BurstOpen.requested_target` rule MR-17 (spec 09) held: under `SendAsap` it is the burst's original `at`, or SC-23a's `requested_at` when admission already advanced it; no change of behaviour | [spec 22](../plan/maintenance/22-timing-simplification.md) § DA-3 — The rollout plan (step 2), 2026-10-14, this stage's brief |
| 2026-10-16 | RM-8, RM-11, RM-14, RM-16, RM-18, RM-21, RM-23, §6 | Test lines and §6 rows state only what their tests assert (the Test-line sweep): RM-11's payload claim narrowed to the kinds MockRadio and the UHD Provider each check, and its source claim given the two further MockRadio tests and the UHD one that assert a source; the `refused` outcome, the transmit side and the update classes and queue named where a test asserts them; RM-8's `Max`/`Range` applied values and no-grid-value rejection and RM-23's receive delay under `none named`, and RM-14's `requested_target` carried by the two tests that assert it (only SC-23a's `requested_at` inheritance left under `none named`); the RM-24 row's payloads corrected to the test's three inputs; no change of behaviour | Test-line sweep, 2026-10-16 |
| 2026-10-17 | RM-4, RM-6, RM-9, RM-20, RM-25, RM-26, §2, §6 | the phase value `random` (every applied tune redraws the phase, so a board with it is aligned but not coherent and declares `radio.rx.coherent: One(false)`); `radio.timing.start_lead_ns` becomes `radio.timing.rx_start_lead_ns` and `radio.timing.tx_start_lead_ns`, and each stream takes its own direction's; `DeviceDescription.max_channels` becomes `rx_max_channels` and `tx_max_channels`, the binding's layout lengths, and its receive channel default is 0 when the layout has no receive front end (`envelope.v1.json` regenerated); RM-9's Test line and §2's evidence say all three phase values; the description-tree test is renamed, since the description is the Vocabulary's now | [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) §5 stage 1a. The three renames are listed in the stage's mutation-run record |
| 2026-10-17 | RM-1, RM-2, RM-3, RM-4, RM-5, RM-7, RM-8, RM-9, RM-19, RM-21, RM-27…RM-33, §3, §5, §6, §8 | the layout builder: the channel- and stream-numbered keys (RM-27), the mixed-board layout (RM-28), its description and tree with the Basic/LF front end rows (RM-29), its coercion (RM-30), the full-duplex constraint (RM-31), the one-link throughput limit (RM-32) and the per-channel RF envelope (RM-33); RM-2, RM-3, RM-5, RM-7, RM-8, RM-9 and RM-19 name the current builder where their text is its rule. RM-30, RM-31 and RM-32 gain `Description::violations`, the one function that judges a whole configuration on the full-duplex and throughput constraints with no request, which a Provider uses to re-judge a configuration at an update's effective instant and which `coerce` calls on the configuration its request leaves; RM-28 consults the clock-compatibility table for every pair of board kinds on one timekeeper and states what each refusal names | [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) D1, D3–D5, D7–D9, D12, D13, D16, §5 stage 1b-2 |
| 2026-10-17 | RM-3, RM-12, RM-13, RM-16, RM-21, RM-25, RM-26, RM-34, §6 | a stream's channels are a set (`ezsdr_radio::timeline::Channels`) and its configuration is that set with its rate, so it holds a channel when the set is not empty and a `cold` change to another set of the same size is another configuration (RM-21, RM-25); RM-34 is new — one channel's move between two streams, one instance's lines held as a `timeline::Lines` by a Provider that moves a channel between two of its streams, through which every booking, refusal and pruning goes, which books a move on both lines at one instant and refuses it on both, keeps the release of every move on the items of the stream it moved into — the instant the source's plan stops holding the moved channel, the segments it runs with the receive ones RM-25 leaves out included, after every booking, refusal or pruning or a move — apart from each item's ready, which a transmit replay reads (VH-2), keeps a pruned move's release as the greatest release floor of its destination line, drops the record of a move whose two items a pruning folds away, refuses a move the lines cannot make (a number that is no channel, a channel the source lacks or the destination holds, one stream twice, one move between two directions) and refuses a move's `seq` on both its lines whichever of them names it, and reads the configuration a line leaves in effect (`Lines::config_at`); a layout description's transmit waveform's channel count is its target stream's set's size and its channel `i` that set's `i`-th smallest logical channel, and its receive block's `channels` is likewise the number of channels its stream holds, RM-3 saying so for both; RM-13's `TxBurst` and RM-16's `Stop` name `<device>/tx/s<k>` and `<device>/rx/s<k>` with a layout description, and RM-12 says so for the verbs and an `UpdateParameter`, read by `ezsdr_radio::stream_target` | [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) D3, D14, §5 stage 1b-2 |
| 2026-10-18 | RM-11 | a radio event's source node is the stream's own node (RM-3): `<device>/rx` or `<device>/tx` with RM-26's description, `<device>/rx/s<k>` or `<device>/tx/s<k>` with a layout description (RM-29); no other change | [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) §5 stage 1b-3b |
| 2026-10-18 | RM-25, §6 | with a layout description a `cold` change is floored at every `cold` change of the same instance received earlier, whatever direction, stream or line it belongs to, since the change a layout instance can book on one of them is not the only one it holds (RM-27, RM-29) | [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) §5 stage 1b-3b, fix round 1 |
| 2026-10-18 | RM-25, RM-34 | A layout Provider uses the instance-wide pending floor before projecting a move, and `book_move` enforces the two-line floor at the instant it receives | [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) D3, §5 stage 1b-3c |
| 2026-10-19 | RM-25, RM-26, §6 | the layout instance's floor is one function, `timeline::layout_instant(at, now, start, earlier)`, whose `earlier` is the instance's still-pending `cold` changes and whose `start` is a receive start, called by both Providers; `timeline`'s one computation states that input; the rate-set comparison (`device::rate_set_matches`, which only the UHD Provider calls) and the one `keys` judgement (`channel_keys`, which both read) are named | [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) §5 stage 1b-4c (design 33 D5; the Mock's floor leaves `lib.rs` for the shared function) |
| 2026-10-22 | RM-2…RM-9, RM-11, RM-13, RM-19, RM-21, RM-23, RM-26, §3, §5, §6, §7 | Phase 8 stage 1b-5c-2 retires `DeviceDescription`, `x3xx::description`, the replaced key declarations and selector `keys`; the Providers use layout keys only, and selector `keys` is rejected as unknown. RM-26 is withdrawn; legacy-only tests are replaced by or mapped to layout-path coverage, and the MR-30 untimed RX-before-TX tune order is pinned through the drawn phase. Versions are unchanged. | [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) D12, §5 stage 1b-5c-2 |
| 2026-10-20 | RM-34 | A per-stream prune retains both items and their move record until both lines pass the move instant, then folds its source release into the destination floor | [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) §5 stage 1b-4e-2 |
| 2026-10-22 | RM-8, RM-25, §6 | RM-8's frequency and gain grids use numbered channel keys; RM-25 states the general pruning boundary, state fold and unchanged-plan contract with its retained test. | [note 23](../plan/maintenance/23-one-line-per-stream.md), [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) D12–D13, §5 stage 1b-5c-2 fix 1 |
| 2026-10-22 | RM-16 | An admitted held transmit burst cancelled before its start by Stop, Provider::stop, loss or a cold change is recorded; only a cold-change cancellation emits COMMAND_REJECTED. Versions are unchanged. | [Phase 8 report rules](../plan/phase8/36-radio-report-rules.md) decision 1, Phase 8 stage 1b-7a |
| 2026-10-23 | RM-11, RM-17, RM-18, RM-22, RM-24 | `RX_OVERFLOW` is `{ cause, lost }` with a 9-byte hot form; one shared Radio Model rule summarizes each report-bearing block's whole jump, with overrun priority, while gapless reports and back-pressure keep their specified paths. | [Phase 8 report rules](../plan/phase8/36-radio-report-rules.md) decisions 4–5, Phase 8 stage 1b-7b |
| 2026-10-24 | RM-4, RM-6, RM-16, RM-20, RM-21 | `radio.timing.release_window_ns` joins the TimingEnvelope (x3xx 3 ms, ideal 0; `envelope.v1.json` regenerated); an omitted or past `at` takes a floor of the instant of a timed update booked earlier inside that window, computed by `timeline::timed_instant`, and the queue counts updates whose instant is still after the receipt, after the late judgement; `Provider::stop` and a loss settle at the end's own instant `S`, and a given command is handed over by `S` when the device's time read just before its call began was at or before `S` (a stop) or before it (a loss) | [design 38](../plan/phase8/38-timed-release-order.md) D1, D2, stage 38a |
| 2026-10-24 | RM-25 | MockRadio raises the read-back refusal through spec 08's `cold_change_refused` fault, so the one halt rule is exercised by both Providers and the 2026-10-09 row's "only the UHD Module exercises the rule for now" no longer holds; no other change | [spec 22](../plan/maintenance/22-timing-simplification.md) follow-up 4 (#59) |
| 2026-10-25 | RM-11, RM-12, RM-14 | Test lines name the A2 tests of the conditions they left under `none named`: `CLOCK_LOST`'s device node and `TX_UNDERFLOW`'s stream node (RM-11), `start_rx`'s `at`, its receive link's T0 floor and its never-late instant (RM-12), and SC-23a's `requested_at` kept as `requested_target` on time and after a `SendAsap` move, in MockRadio and the UHD Module (RM-14); no change of behaviour | [plan 32](../plan/maintenance/32-test-line-sweep.md) A2 |
| 2026-10-25 | RM-4, RM-5, RM-6, RM-7, RM-9, RM-10, RM-11, RM-13, RM-16, RM-21, RM-25, RM-27, RM-29, RM-31, RM-32, RM-33, RM-34 | Test lines narrowed to what their tests assert, tests that pin a condition named, and conditions no test asserts listed under none named (second test-line sweep); no change of behaviour | [plan 32](../plan/maintenance/32-test-line-sweep.md) |
| 2026-10-25 | RM-4, RM-6, RM-15, RM-16, RM-20, RM-29 | `radio.timing.tx_in_flight_ns` joins the TimingEnvelope (x3xx 10 ms, ideal 0; `envelope.v1.json` regenerated); preemption uses the later of the ordinary bound and a same-clock hand-over front, with Mock's conservative receipt-order rule; Stop and the empty-EOB restart boundary are stated | [design note 40](../plan/phase8/40-preemption-bound.md) |
| 2026-10-26 | RM-20, RM-35 | Add the versioned measurement record, per-key comparison rules, validation and committed schema | [measurement records](../plan/phase8/41-measurement-records.md) §2–§4 |
| 2026-10-28 | RM-36 | `ezsdr_radio::timeline::covering` is the one rule for "a segment's samples pass at `t`"; the UHD Module's `reach` calls it and MockRadio's receive tune uses it (MR-34) | [plan/phase8/reviews/ma1-opus.md](../plan/phase8/reviews/ma1-opus.md) |
| 2026-10-28 | RM-6, RM-25, RM-34, §6 | A refused receive change or `start_rx` runs for planning until the next item cuts it (no plan row, no clock, no record; RM-34 reads its cut as a release), a transmit change after a refusal begins as if the change had not been refused, and a Description's receive start lead is at least its release window | [design note 43](../plan/phase8/43-refusal-keeps-origins.md), Phase 8 stage 43a |
| 2026-10-30 | RM-15, RM-16 | A nonzero in-flight window closes a device burst with at least two EOB samples; the safe restart is one sample after the closing send's exclusive end, including a no-payload two-zero close. RM-16 refers to RM-15's close rule, and its tests name the two-sample close and safe boundary. | [design note 39](../plan/phase8/39-dsp-timed-commands.md) §6.2, D9b-1a fix round 1 |
| 2026-10-30 | RM-25 | After a receive cut, every successor's readiness includes the release window, one old-rate receive call and delivery allowance before its RX start lead, whether or not a DDC value is pending | [design note 39](../plan/phase8/39-dsp-timed-commands.md) §6.7, owner decision (A) |
| 2026-11-04 | RM-30, §6 R2 | no channel is in a stream by default, receive channel `c0` included; a Session asks at `connect` for the channels it uses (RS-12); the §6 row R2 follows | [Phase 8 note 47](../plan/phase8/47-no-change-while-streaming.md) §2.4, §6 O1, stage 1 |
| 2026-11-04 | RM-28 | a layout entry may end in `#<name>`: the name rule, uniqueness in a radio, no effect on the description | [Phase 8 note 47](../plan/phase8/47-no-change-while-streaming.md) stage 1b |
| 2026-11-04 | RM-4, RM-7, RM-11, RM-12, RM-14, RM-15, RM-16, RM-19, RM-21, RM-25, RM-27, RM-31, RM-32, RM-34, RM-36, §6 | no radio key has an update class: the Run-time machinery is withdrawn (timed release, queue and floor, moves, restore, `cold` changes, `timeline::covering`); RM-34 and RM-36 are stubs; a `start_rx` is the only later receive segment; §58 #16 is tested as a classless refusal until stages 3 and 4 return the envelope refusal | [Phase 8 note 47](../plan/phase8/47-no-change-while-streaming.md) stage 2 |
