# Phase 4 spec 13 — Amendments: the error round, the first hot-path layout, SigMF captures

| Field | Value |
|---|---|
| Status | Draft under the owner's delegation (2026-09-26); decided at Gate X ([`00-overview.md`](00-overview.md) §11). |
| Scope | One Kernel amendment (KD-1: MA-30's round after a Module error) and three Vocabulary and Module amendments (VC-1: `radio` 1.2.0 and RM-24; VC-2: `ezsdr.radio.mock` 1.2.0 and MR-37; VC-3: `ezsdr.sink.capture` 1.1.0 and HD-15). |
| Amends | `design/05-module-api.md` (MA-30), `design/07-radio-model.md` (RM-1, RM-11; new RM-24), `design/09-mock-radio.md` (MR-1, MR-19, MR-28, MR-30, §8; new MR-37), `design/10-host-data-path.md` (HD-7, HD-10, H3, §9; new HD-15), `design/02-stream-contract.md` (SC-32's marker), `design/08-simulation.md` (S1, §7), `design/11-simulation-channel.md` (CH-9's ceiling). |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

Each amendment gives the problem, the rule text as it reads after the amendment, the rejected alternatives and the tests. Appendix A lists the mutations that show each test guards its rule (GW-4).

---

## 1. Kernel amendment

### KD-1 — A Module error does not cut the round short

**Problem.** `step_until_quiescent` steps the sorted instances and returns at the first `?`. An instance that reports an error — `DeviceLost` from an injected `device_lost`, say — leaves every instance later in the order unstepped at that instant, and the order is `(role rank, fragment id)`, so a fragment's **name** decides what a receiver published before the fault. Phase 3 measured it through the acceptance rig (receiver `rx_blocks` 0 when the faulted transmitter sorts first, 1 when the receiver does; `plan/phase3/implementation-notes.md`, "P2-1, measured after B1") and accepted a ceiling on CH-9 and MR-30 that names Phase 4's failure work as the owner.

**MA-30 amended** (the loop, and one paragraph after it):

```text
until = authority.next_wakeup()        None ends a Spec Run (KC-33); a Session waits for its client
repeat: step(until) over every stepped instance not failed at this until, in a fixed order
        role rank Provider < Executor < Sink, then instance id
until no instance reports progressed; a cap of 1 000 iterations raises STEP_LIVELOCK and aborts
```

> A `step` that returns an error — a panic included, which the coordinator contains as one (KC-30) — marks its instance failed for the rest of the round: it is not stepped again at that `until`, and the round goes on over the other instances until none reports `progressed`. `step_until_quiescent` then returns the **first** error in stepping order (pass, then role rank, then instance id), and the coordinator acts on that one as before — `DEVICE_LOST` for `DeviceLost`, a failed Run otherwise. The instances a failure did not touch are therefore stepped as they would be if the failed instance had produced nothing at that instant, wherever it sorts, and no instance's output depends on a failed peer's name (Phase 4, KD-1). *Checked by `ma_30_a_step_error_finishes_the_round` and, end to end, `kd_01_a_faulted_round_does_not_depend_on_fragment_names`.*

**CH-9** loses its ceiling paragraph ("Ceiling (Gate X, Review C P2-1): …"), and **MR-30** loses ", except in a round a Module error ends early, where which blocks were published before the abort can depend on fragment order (CH-9's ceiling; Phase 3, VB-10)"; each gains "(the error round: MA-30, Phase 4 KD-1)".

**Rejected.** Keeping the ceiling (every later determinism claim would carry the exception, and Phase 5's Reactor decisions join §58 #3). Stepping the failed instance again (it has reported that it cannot run; a Provider that lost its device must not be asked to produce). Stopping after the current pass (an instance that consumes in a later pass what a peer published in this one would still depend on order). Collecting every error and reporting all (the coordinator acts on one termination; the first in a fixed order is deterministic, and the others' instances are recorded by their own events).

**Code.** `crates/ezsdr-kernel/src/module_api.rs`: `step_until_quiescent` keeps a failed flag per instance and the first error, skips failed instances, and returns the error once a pass makes no progress. `coordinator/stepping.rs` is unchanged: its `guard_step` already records the first failure in call order, which is the same one. No public item changes (GW-2).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `ma_30_a_step_error_finishes_the_round` (`crates/ezsdr-kernel/tests/module_api.rs`) | a Provider that fails on its first step, a Provider that progresses on its first two steps, a Sink; run once with the failing Provider named `a` (first) and once named `z` (last) | both runs return the failing Provider's error; it is stepped exactly once; the other Provider and the Sink are stepped the same number of times in both runs, as many as with no failing instance at all | MA-30, KD-1 |
| `kd_01_a_faulted_round_does_not_depend_on_fragment_names` (acceptance) | Phase 3's probe: a transmitter and a receiver on a loopback channel, `device_lost` on the transmitter at 1 999 001 ns after T0; the transmitter named `a` and the receiver `b`, then `z` and `a` | both Runs stop on `DEVICE_LOST`; the receiver's `rx_blocks` and `rx_samples` are equal in the two Runs and equal to the order in which the receiver sorts first (1 and 2 000) | MA-30, KD-1, CH-9, MR-30 |

---

## 2. Vocabulary and Module amendments

### VC-1 — `radio` 1.2.0: the hot-path form of `RX_OVERFLOW` (RM-24)

**Problem.** Phase 1 withdrew a Kernel-owned hot-path layout (D51, RS-32a: "The Kernel does not interpret a Vocabulary's hot-path byte layout; a Vocabulary that needs one defines and decodes its own representation"), and left the first layout to MockRadio. None was ever defined, so every radio event travels the control path, including the one a hardware Provider raises on its sample thread (spike finding K10).

**RM-24** (new):

> `radio.RX_OVERFLOW` has a hot-path form of 17 bytes, little-endian: byte 0 the cause (`0` overrun, `1` sequence), bytes 1–8 `lost` as u64, bytes 9–16 `restart_gap_ns` as i64. `RxOverflowPayload::to_hot(&self) -> [u8; 17]` writes it. `RxOverflowPayload::from_hot(bytes: &[u8]) -> Result<RxOverflowPayload, String>` reads it and refuses another length or a cause byte other than 0 or 1. `RxOverflowPayload::from_payload(payload: &serde_json::Value) -> Result<RxOverflowPayload, String>` reads a delivered event's payload in either form — RM-11's object, from the control path, or the array of byte values the Kernel's drain makes of a hot-path record (RS-34) — and refuses anything else, so a reader of a Manifest needs one call whichever path a Provider used. The Kernel does not decode it (RS-32a, withdrawn). *Checked by `rm_24_the_hot_form_round_trips` and `rm_24_a_delivered_payload_reads_in_either_form`.*

**RM-11 amended**, its first sentence: "Every radio event is emitted with `emit_control` in Phase 2 (RS-32's hot path is the Phase 7 Provider's concern), …" becomes "Every radio event is emitted with `emit_control`, except `radio.RX_OVERFLOW`, which is emitted on the hot path (`EventSink::emit`, RS-32) with RM-24's bytes, because a device reports it from its sample path (Phase 4, VC-1), …". The payload table's `RX_OVERFLOW` row gains "; on the hot path, RM-24's bytes".

**RM-1 amended**: `version: 1.2.0`, and "Version 1.2.0 adds RM-24's hot-path form of `RX_OVERFLOW` (Phase 4, VC-1)."

**Rejected.** A Kernel decoder, by declared layout or by callback (D51; `00-overview.md` Q1). Changing RM-22's schema to the byte form (the object stays readable and valid; `from_payload` reads both). 2.0.0 (nothing is removed).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `rm_24_the_hot_form_round_trips` | both causes with `lost` `u64::MAX` and `restart_gap_ns` `i64::MIN`; a 16- and an 18-byte slice; a cause byte of 2 | the exact 17 bytes of RM-24 for one value (pinned byte for byte); `from_hot(to_hot(p)) == p`; three refusals | RM-24 |
| `rm_24_a_delivered_payload_reads_in_either_form` | `to_value(p)`; `p.to_hot()` as a JSON array of integers; an array holding 256; a string | `p` twice; two refusals | RM-24, RM-11 |

### VC-2 — `ezsdr.radio.mock` 1.2.0: `RX_OVERFLOW` on the hot path (MR-37)

**MR-37** (new):

> MockRadio emits `radio.RX_OVERFLOW` on the hot path. In `prepare` it resolves the handle for its receive stream node `<device>/rx` and the kind with `ctx.events.resolve` (RS-33); every overflow — MR-19's back-pressure overrun, MR-21's overrun and MR-22's sequence error — then calls `emit(handle, time, warning, &payload.to_hot())` with the time and payload that RM-17, RM-18 and MR-19 give it. An error from `emit` is `ModuleError { kind: Internal }`, as one from `emit_control` is (MR-10). Every other event stays on `emit_control`. *Checked by `mr_37_the_overflow_travels_the_hot_path`: with an `EventCollector` whose ring holds one body, two overflows between two drains are counted twice, delivered once — RM-24's bytes, decoding to the first overflow — and reported once in `EVENTS_DROPPED`; the control path never drops, so a Mock that still used it would deliver both. `mr_19_backpressure_is_an_overrun` asserts that the back-pressure overrun's payload is the byte form.*

**MR-1 amended**: `version: 1.2.0`, `vocabularies: [{ radio, ^1.2.0 }, { sim, ^1.1.0 }]`, `impl_hash: Some(ContentHash::of_bytes(b"ezsdr.radio.mock 1.2.0"))` (Phase 4, VC-2). The profiles stay `x310-like 1.1.0` and `ideal 1.1.0`: their content does not change.

**MR-19 amended**: "`radio.RX_OVERFLOW { cause: overrun }` is emitted" gains "on the hot path (MR-37)" (MR-21 and MR-22 name the samples, and RM-17 and RM-18 the event). **MR-28 amended**: "through `emit_control`" gains "— except `RX_OVERFLOW`, which MR-37 puts on the hot path —".

**§8 Deferred amended**: "A transmit port and `TX_UNDERFLOW`; the hot-path event layout; `ALIGNMENT` injection (Phase 4); a nonzero receive path delay (MR-35)." becomes "A transmit port and `TX_UNDERFLOW` (Phase 10); `ALIGNMENT` injection (Phase 7, with the UHD Provider's alignment behaviour); a nonzero receive path delay (MR-35)."

**Rejected.** Moving `TIME_ERROR`, `LATE_COMMAND`, `COMMAND_REJECTED` too (control-path work; `TIME_ERROR` carries a `TimePoint`, `00-overview.md` §3). Resolving the handle on every emission (`resolve` is control-path only, RS-33).

**Tests.** `mr_37_the_overflow_travels_the_hot_path` (new); `mr_19_backpressure_is_an_overrun` asserts the byte form and `mr_20_faults_fire_at_their_instants` reads `lost` with `from_payload` (GW-3); `mr_01_descriptor_registers` pins 1.2.0, both vocabulary requirements and the implementation hash.

### VC-3 — `ezsdr.sink.capture` 1.1.0: every capture is a SigMF Recording (HD-15)

**Problem.** SC-32 is the only Phase 1 rule still marked "tested in Phase 4"; spec 10 H3 put SigMF in "Phase 4's SigMF Sink"; Vision §51 calls SigMF interoperability "a standard feature".

**HD-15** (new):

> Every capture is a SigMF Recording (Vision §51, SC-32). Its Dataset file is HD-10's file, named with the extension `sigmf-data`; its bytes are unchanged, because SigMF's complex types store `I` then `Q` little-endian and interleave several channels sample by sample, as HD-3's `interleave` writes. When a capture finishes — complete or partial — with exactly one ContinuityMap, the Sink writes beside it `<dir>/<run id>_<artifact id>.sigmf-meta`, the document `sigmf_meta(map, rate, contract)` returns with `rate = ClockRegistry::nominal_rate(map.domain)`. A capture with more than one map gets no metadata file.
>
> `ezsdr_sink_capture::sigmf_meta(map: &ContinuityMap, rate: Rational, contract: &DataContractId) -> Result<serde_json::Value, String>` returns an object with exactly the members `global`, `captures` and `annotations`. A **file index** counts delivered samples from `map.first`: for a delivered tick `t`, `t − map.first.ticks − Σ g.len` over the gaps `g` with `g.start.ticks + g.len ≤ t`.
>
> - `global`: `core:datatype` — `cf32_le` for `ezsdr.stream.cf32`, `ci16_le` for `ezsdr.stream.sc16`, an error for any other contract; `core:version` `"1.2.6"`; `core:num_channels` `map.channels`; `core:sample_rate` the rate as a JSON number; `core:recorder` `"ezsdr.sink.capture 1.1.0"`; `core:extensions` `[{ "name": "ezsdr", "version": "1.0.0", "optional": true }]`; `ezsdr:sample_rate` `{ "num", "den" }`, the exact rate; `ezsdr:gaps`, one `{ "sample_start", "global_index", "len", "lost", "cause", "link_dropped" }` per `map.gaps` entry in order, where `sample_start` is the file index of the first sample after the gap (the delivered count when none follows), `global_index` the gap's start tick, `lost` `null` when absent and `cause` the `GapCause` as serialized; `ezsdr:valid`, per channel in order, the list of `{ "sample_start", "sample_count" }` of that channel's valid segments in file indices.
> - `captures`: one segment per maximal run of delivered samples between two stream gaps, in order, `{ "core:sample_start": <file index of its first sample>, "core:global_index": <that sample's tick> }`; a run of no samples has no segment.
> - `annotations`: one per `map.channel_gaps` entry, sorted by file index and then channel, `{ "core:sample_start", "core:sample_count": <len>, "core:label": "invalid channel", "ezsdr:channel", "ezsdr:cause" }`.
>
> The metadata is a function of the `ArtifactRef`'s continuity and the SampleClock's recorded rate, so the Manifest does not hash it (`00-overview.md` Q7). `crates/ezsdr-sink-capture/ezsdr.sigmf-ext.md` defines the `ezsdr` extension namespace, as SigMF requires of an extension. *Ceiling: no metadata for a capture that spans a SampleClock or channel-count change, since a Recording has one `core:sample_rate` and one `core:num_channels`; no `core:frequency` and no `core:datetime`, which the Sink does not know.* *Checked by `hd_15_a_capture_is_a_sigmf_recording`, `hd_15_channel_validity_and_its_causes`, `hd_15_a_capture_across_a_clock_change_has_no_metadata`, `hd_15_the_datatype_follows_the_contract`, and end to end `v51_an_overflowed_capture_is_a_sigmf_recording`.*

**HD-7 amended**: `version: 1.1.0`, `impl_hash: Some(ContentHash::of_bytes(b"ezsdr.sink.capture 1.1.0"))` (Phase 4, VC-3); the `SinkDescriptor`'s `module` follows.

**HD-10 amended**: "and `ext` is `cf32` or `sc16` after the capture's first block" becomes "with the extension `sigmf-data` (HD-15; Phase 4, VC-3)".

**H3 amended**: the choice becomes "SigMF Recordings: HD-10's interleaved file as the Dataset, and a metadata file (HD-15)"; the ceiling "a capture spanning a SampleClock change has no metadata (HD-15)". **§9** drops "SigMF export (SC-32, Phase 4)". The header's "Not in scope" drops "SigMF and".

**SC-32 marker** becomes: "*Checked in Phase 4 by `hd_15_a_capture_is_a_sigmf_recording` and `hd_15_channel_validity_and_its_causes` (HD-15).*"

**Rejected.** A separate SigMF Sink; a `format` selector key; a post-Run export function; a second `ArtifactRef` for the metadata (`00-overview.md` Q5, Q7). SigMF Collections, one Recording per channel (SigMF recommends them for multi-channel IQ, but a Collection of N files would split one capture's bytes, which HD-10 writes as one file with one hash; `core:num_channels` is valid SigMF).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `hd_15_a_capture_is_a_sigmf_recording` | a 1-channel cf32 capture of 300 samples from tick 100, over blocks with an overflow of 50 lost samples at tick 200 | files `…_rec.sigmf-data` (the samples) and `…_rec.sigmf-meta`; the meta equals, member for member, a literal: two capture segments `{0, 100}` and `{100, 250}`, one `ezsdr:gaps` entry `{100, 200, 50, 50, "overflow_restart", 0}`, `ezsdr:valid` `[[{0,100},{100,200}]]`, no annotations | HD-15, HD-10, SC-32 |
| `hd_15_channel_validity_and_its_causes` | a 2-channel capture where channel 1 is invalid on one block carrying `ALIGNMENT` and later on one that does not | two annotations on channel 1, causes `alignment` then `stream`, at their file indices and lengths; `ezsdr:valid` for channel 1 has the three segments between them | HD-15, SC-31a, SC-32 |
| `hd_15_a_capture_across_a_clock_change_has_no_metadata` | a capture spanning a new SampleClock | a `.sigmf-data` file and no `.sigmf-meta` | HD-15 |
| `hd_15_the_datatype_follows_the_contract` | `sigmf_meta` with `ezsdr.stream.cf32`, `ezsdr.stream.sc16` and another contract | `cf32_le`, `ci16_le`, an error | HD-15 |
| `v51_an_overflowed_capture_is_a_sigmf_recording` (acceptance) | `v58_06`'s overflow Run | the artifact's `uri` ends in `.sigmf-data`; its meta has two capture segments whose `core:global_index` differ by the first run's length plus the gap's 50 000 samples, `core:sample_rate` 1 000 000, and one `ezsdr:gaps` entry with cause `overflow_restart` | HD-15 |

`hd_07_descriptor` reads 1.1.0; the tests that name a capture file by its extension read `sigmf-data` (GW-3).

---

## 3. Vision issues found

Applied at Step X with the owner's approval (OV-6, GW-5), recorded in [`vision-issues.md`](vision-issues.md):

1. **§28 and §51** say SigMF interoperability "should be considered". It now exists; each gains a `Normative:` line naming `design/10-host-data-path.md` HD-15 and `design/02-stream-contract.md` SC-32 (re-review R13).
2. **§29** says the hot path "emits a fixed-size record with at most 32 bytes inline", and nothing about who reads those bytes. A sentence: "The bytes are the owning Vocabulary's layout, which it also decodes; the Kernel counts and queues them without interpreting them (`radio.RX_OVERFLOW` is the first, RM-24)."

---

## Appendix A — Mutations (GW-4)

Each mutation is applied alone to the implemented tree, the named test is run, and the result is recorded in `implementation-notes.md` as `mutation: <name>: killed` or the phase stops.

| Name | Change | Killed by |
|---|---|---|
| `kd1-return-at-first-error` | `step_until_quiescent` returns the error as soon as a step fails | `ma_30_a_step_error_finishes_the_round`, `kd_01_a_faulted_round_does_not_depend_on_fragment_names` |
| `kd1-step-the-failed-again` | a failed instance is stepped in later passes | `ma_30_a_step_error_finishes_the_round` |
| `kd1-last-error-wins` | the last error replaces the first | `ma_30_a_step_error_finishes_the_round` |
| `rm24-cause-bytes-swapped` | `to_hot` writes 1 for overrun and 0 for sequence | `rm_24_the_hot_form_round_trips` |
| `rm24-no-length-check` | `from_hot` reads the first 17 bytes of a longer slice | `rm_24_the_hot_form_round_trips` |
| `rm24-array-refused` | `from_payload` reads only the object form | `rm_24_a_delivered_payload_reads_in_either_form`, `v58_04_mock_events_reach_counters_policy_and_manifest` |
| `mr37-control-path` | the injected overflow is emitted with `emit_control` | `mr_37_the_overflow_travels_the_hot_path` |
| `mr37-backpressure-control-path` | MR-19's overrun is emitted with `emit_control` | `mr_19_backpressure_is_an_overrun` |
| `hd15-global-index-is-file-index` | `core:global_index` written as the file index | `hd_15_a_capture_is_a_sigmf_recording`, `v51_an_overflowed_capture_is_a_sigmf_recording` |
| `hd15-gaps-not-subtracted` | the file index ignores earlier gaps | `hd_15_a_capture_is_a_sigmf_recording` |
| `hd15-meta-for-every-capture` | the metadata is written for a capture with two maps (from its first) | `hd_15_a_capture_across_a_clock_change_has_no_metadata` |
| `hd15-sc16-as-cf32` | `sc16` maps to `cf32_le` | `hd_15_the_datatype_follows_the_contract` |
| `hd15-no-channel-annotations` | channel gaps are not annotated | `hd_15_channel_validity_and_its_causes` |
| `hd15-raw-extension` | the Dataset keeps the `cf32` extension | `hd_15_a_capture_is_a_sigmf_recording`, `v51_an_overflowed_capture_is_a_sigmf_recording` |
