# Phase 4 spec 13 — Amendments: the error round, the first hot-path layout, SigMF captures

| Field | Value |
|---|---|
| Status | Draft under the owner's delegation (2026-09-26); decided at Gate X ([`00-overview.md`](00-overview.md) §11). |
| Scope | One Kernel amendment (KD-1: MA-30's round after a Module error) and three Vocabulary and Module amendments (VC-1: `radio` 1.2.0 and RM-24; VC-2: `ezsdr.radio.mock` 1.2.0 and MR-37; VC-3: `ezsdr.sink.capture` 1.1.0 and HD-15). |
| Amends | `design/05-module-api.md` (MA-30), `design/07-radio-model.md` (RM-1, RM-11, RM-20, RM-22; new RM-24), `design/09-mock-radio.md` (MR-1, MR-19, MR-28, MR-30, §8; new MR-37), `design/10-host-data-path.md` (HD-7, HD-10, H3, §9; new HD-15), `design/02-stream-contract.md` (SC-32's text and marker), `design/08-simulation.md` (S1, §7), `design/11-simulation-channel.md` (CH-9's ceiling). |
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

> A `step` that returns an error — a panic included, which the coordinator contains as one (KC-30) — marks its instance failed for the rest of the round: it is not stepped again at that `until` (a later instant may step it again, as a cleanup drain does), and the round goes on over the other instances until none reports `progressed`. `step_until_quiescent` then returns the **first** error in stepping order (pass, then role rank, then instance id); a round that reaches the cap after a failure returns that failure too, and raises no `STEP_LIVELOCK`, because the failure is what ends the Run. The coordinator acts on the first failure as before — `DEVICE_LOST` for `DeviceLost`, a failed Run otherwise — and emits `DEVICE_LOST` for every later `DeviceLost` of the same round as well, so a second lost device is reported; a later failure of another kind is not recorded, its Run already ending. The instances a failure did not touch therefore see only what the failed instance published before it failed, wherever it sorts, and no instance's output depends on a failed peer's name (Phase 4, KD-1; the cap and the later failures after the Phase 4 review). *Checked by `ma_30_a_step_error_finishes_the_round`, `ma_30_a_second_failure_does_not_replace_the_first`, `ma_30_a_failure_beats_the_step_livelock_cap`, the coordinator's `kd_01_a_device_lost_is_not_reported_as_a_step_livelock` and `kd_01_every_lost_device_of_a_round_is_reported_and_the_first_failure_decides`, and, end to end, `kd_01_a_faulted_round_does_not_depend_on_fragment_names`.*

**CH-9** loses its ceiling paragraph ("Ceiling (Gate X, Review C P2-1): …"), and **MR-30** loses ", except in a round a Module error ends early, where which blocks were published before the abort can depend on fragment order (CH-9's ceiling; Phase 3, VB-10)"; each gains "(the error round: MA-30, Phase 4 KD-1)".

**Rejected.** Keeping the ceiling (every later determinism claim would carry the exception, and Phase 5's Reactor decisions join §58 #3). Stepping the failed instance again (it has reported that it cannot run; a Provider that lost its device must not be asked to produce). Stopping after the current pass (an instance that consumes in a later pass what a peer published in this one would still depend on order). Letting the cap's `STEP_LIVELOCK` win over a failure found earlier in the round (the Kernel's own event would become the recorded cause and the Module's error would be lost — which returning at the first error never did; found by the Phase 4 review). Reporting only the first failure (KD-1 now steps a second failing instance, and a second lost device would be found and dropped: the coordinator's slot held one error). Letting every failure decide the termination (the coordinator ends a Run once; the first in a fixed order is deterministic).

**Code.** `crates/ezsdr-kernel/src/module_api.rs`: `step_until_quiescent` keeps a failed flag per instance and the first error, skips failed instances, returns the error once a pass makes no progress, and returns it at the cap instead of raising `STEP_LIVELOCK`. `coordinator/stepping.rs`: `guard_step` records every failure in call order (stepping order); `round` acts on the first as before and emits `DEVICE_LOST` for every later `DeviceLost`. No public item changes (GW-2).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `ma_30_a_step_error_finishes_the_round` (`crates/ezsdr-kernel/tests/module_api.rs`) | a Provider that fails on its first step, a Provider that progresses on its first two steps, a Sink; run once with the failing Provider named `a` (first) and once named `z` (last) | both runs return the failing Provider's error; it is stepped exactly once; the other Provider and the Sink are stepped the same number of times in both runs, as many as with no failing instance at all | MA-30, KD-1 |
| `kd_01_a_faulted_round_does_not_depend_on_fragment_names` (acceptance) | Phase 3's probe: a transmitter and a receiver on a loopback channel, `device_lost` on the transmitter at 1 999 001 ns after T0; the transmitter named `a` and the receiver `b`, then `z` and `a`; then `device_lost` on both | both Runs stop on `DEVICE_LOST`; the receiver's `rx_blocks` and `rx_samples` are equal in the two Runs and equal to the order in which the receiver sorts first (1 and 2 000); with both lost, `DEVICE_LOST` from `dev_rx` and `dev_tx` in either order | MA-30, KD-1, CH-9, MR-30 |
| `ma_30_a_failure_beats_the_step_livelock_cap` (Kernel) | a failing Provider and a Sink that never quiesces | the Provider's error is returned; no `STEP_LIVELOCK` is emitted | MA-30, KD-1 |
| `kd_01_a_device_lost_is_not_reported_as_a_step_livelock` (coordinator) | the same through a Spec Run (Review E's probe) | the Run stops on `DEVICE_LOST`; no `STEP_LIVELOCK` delivered | MA-30, KD-1 |
| `kd_01_every_lost_device_of_a_round_is_reported_and_the_first_failure_decides` (coordinator) | Providers `p` and `q` failing in one round: `p` with an ordinary error and `q` losing its device; then both losing it | `Failed { run }` naming `p`, and `q`'s `DEVICE_LOST` delivered; then `DEVICE_LOST` for each | MA-30, KD-1 |

---

## 2. Vocabulary and Module amendments

### VC-1 — `radio` 1.2.0: the hot-path form of `RX_OVERFLOW` (RM-24)

**Problem.** Phase 1 withdrew a Kernel-owned hot-path layout (D51, RS-32a: "The Kernel does not interpret a Vocabulary's hot-path byte layout; a Vocabulary that needs one defines and decodes its own representation"), and left the first layout to MockRadio. None was ever defined, so every radio event travels the control path, including the one a hardware Provider raises on its sample thread (spike finding K10).

**RM-24** (new):

> `radio.RX_OVERFLOW` has a hot-path form of 17 bytes, little-endian: byte 0 the cause (`0` overrun, `1` sequence), bytes 1–8 `lost` as u64, bytes 9–16 `restart_gap_ns` as i64. `RxOverflowPayload::to_hot(&self) -> [u8; 17]` writes it. `RxOverflowPayload::from_hot(bytes: &[u8]) -> Result<RxOverflowPayload, String>` reads it and refuses another length or a cause byte other than 0 or 1. `RxOverflowPayload::from_payload(payload: &serde_json::Value) -> Result<RxOverflowPayload, String>` reads a delivered event's payload in either form — RM-11's object, from the control path, or the array of byte values the Kernel's drain makes of a hot-path record (RS-34) — and refuses anything else, so one call reads a Manifest from before 1.2.0, whose Mock emitted the object, and one from after. `RxOverflowHotPayload`, the array as a Manifest holds it, has the committed schema `schemas/radio/rx_overflow_hot_payload.v1.json` (PO-7), so a reader that validates payloads has a schema for both forms. The Kernel does not decode it (RS-32a, withdrawn: the Vocabulary owns its layout) (Phase 4, VC-1). *Ceilings: a drain delivers the ring's bodies before the control path's (RS-34), so the Manifest's `delivered` list is ordered by path, then by emission, and when two kinds of one drain both stop the Run the hot one is the recorded cause; a hot body the full ring drops is counted and reported in `EVENTS_DROPPED` but marks no artifact, since `mark_artifact` acts on delivered bodies (RS-30).* *Checked by `rm_24_the_hot_form_round_trips` and `rm_24_a_delivered_payload_reads_in_either_form`.*

**RM-11 amended**, its first sentence: "Every radio event is emitted with `emit_control` in Phase 2 (RS-32's hot path is the Phase 7 Provider's concern), …" becomes "Every radio event is emitted with `emit_control`, except `radio.RX_OVERFLOW`, which every radio Provider must emit on the hot path (`EventSink::emit`, RS-32) with RM-24's bytes, because a device reports it from its sample path (Phase 4, VC-1), …", and "a payload that is a JSON object with exactly these members" becomes "a payload that is, on the control path, a JSON object …". The payload table's `RX_OVERFLOW` row gains "; on the hot path, RM-24's bytes". **RM-22** says a Provider builds a *control-path* payload with `to_value`; **RM-20** counts RM-24's hot-form schema among the committed ones.

**RM-1 amended**: `version: 1.2.0`, and "Version 1.2.0 adds RM-24's hot-path form of `RX_OVERFLOW` (Phase 4, VC-1)."

**Rejected.** A Kernel decoder, by declared layout or by callback (D51; `00-overview.md` Q1). Changing RM-22's schema to the byte form (the object stays readable and valid; `from_payload` reads both). 2.0.0 (nothing is removed).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `rm_24_the_hot_form_round_trips` | an overrun of 50 000 samples and 50 ms; a sequence error with `lost` `u64::MAX` and `restart_gap_ns` `i64::MIN`; an overrun with 0 and −1; a 16- and an 18-byte slice; a cause byte of 2 | the first's 17 bytes pinned byte for byte; `from_hot(to_hot(p)) == p` for all three; three refusals | RM-24 |
| `rm_24_a_delivered_payload_reads_in_either_form` | `to_value(p)`; `p.to_hot()` as a JSON array of integers; an array holding 256; a string; the array as `RxOverflowHotPayload`, and a 3-byte array | `p` twice; two refusals; the array parses as the hot-form schema's type and the short one does not | RM-24, RM-11, RM-20 |

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

> Every capture is a SigMF Recording (Vision §51, SC-32). Its Dataset file is HD-10's file, named with the extension `sigmf-data`; its bytes are unchanged, because SigMF's complex types store `I` then `Q` little-endian and interleave several channels sample by sample, as HD-3's `interleave` writes. When a capture finishes — complete or partial — with exactly one ContinuityMap, the Sink records its `ArtifactRef` and then writes beside it `<dir>/<run id>_<artifact id>.sigmf-meta`, the document `sigmf_meta(map, rate, contract, partial)` returns with `rate = ClockRegistry::nominal_rate(map.domain)`: first to a staging file, then renamed into place, so a reader never sees half of it and a failed write cannot cost the capture its record. A capture with more than one map gets no metadata file.
>
> `ezsdr_sink_capture::sigmf_meta(map: &ContinuityMap, rate: Rational, contract: &DataContractId, partial: bool) -> Result<serde_json::Value, String>` returns an object with exactly the members `global`, `captures` and `annotations`. A **file index** counts delivered samples from `map.first`: for a delivered tick `t`, `t − map.first.ticks − Σ g.len` over the gaps `g` with `g.start.ticks + g.len ≤ t`. A map with a file index below 0 or beyond `i64::MAX` — which no `ContinuityBuilder` produces, but a hand-built one can — is an error naming HD-15, never invalid SigMF.
>
> - `global`: `core:datatype` — `cf32_le` for `ezsdr.stream.cf32`, `ci16_le` for `ezsdr.stream.sc16`, an error for any other contract; `core:version` `"1.2.6"`; `core:num_channels` `map.channels`; `core:sample_rate` the rate as a JSON number; `core:recorder` `"ezsdr.sink.capture 1.1.0"`; `core:extensions` `[{ "name": "ezsdr", "version": "1.0.0", "optional": true }]`; `ezsdr:sample_rate` `{ "num", "den" }`, the exact rate; `ezsdr:partial`, whether the capture is partial (HD-13), which a SigMF reader could not otherwise tell; `ezsdr:gaps`, one `{ "sample_start", "global_index", "len", "lost", "cause", "link_dropped" }` per `map.gaps` entry in order, where `sample_start` is the file index of the first sample after the gap (the delivered count when none follows), `global_index` the gap's start tick, `lost` `null` when absent and `cause` the `GapCause` as serialized; `ezsdr:valid`, per channel in order, the list of `{ "sample_start", "sample_count" }` of that channel's valid segments in file indices.
> - `captures`: one segment per maximal run of delivered samples between two stream gaps, in order, `{ "core:sample_start": <file index of its first sample>, "core:global_index": <that sample's tick> }`; a run of no samples has no segment.
> - `annotations`: one per `map.channel_gaps` entry, sorted by file index and then channel (the builder lists a break when it closes, and SigMF requires start order), `{ "core:sample_start", "core:sample_count": <len>, "core:label": "invalid channel", "ezsdr:channel", "ezsdr:cause" }`.
>
> The metadata is a function of the `ArtifactRef`'s continuity and the SampleClock's recorded rate, so the Manifest does not hash it (`00-overview.md` Q7). `crates/ezsdr-sink-capture/ezsdr.sigmf-ext.md` defines the `ezsdr` extension namespace, as SigMF requires of an extension. *Ceiling: no metadata for a capture that spans a SampleClock or channel-count change, since a Recording has one `core:sample_rate` and one `core:num_channels`; no `core:frequency` and no `core:datetime`, which the Sink does not know.* *Checked by `hd_15_a_capture_is_a_sigmf_recording`, `hd_15_channel_validity_and_its_causes`, `hd_15_a_capture_across_a_clock_change_has_no_metadata` (both routes to a second map), `hd_15_the_datatype_follows_the_contract`, `hd_15_gap_fields_carry_the_continuity_causes`, `hd_15_annotations_are_sorted_by_index_then_channel`, `hd_15_a_map_it_cannot_describe_is_refused`, `hd_15_a_partial_capture_has_metadata`, `hd_15_an_sc16_capture_is_ci16_le`, and end to end `v51_an_overflowed_capture_is_a_sigmf_recording` and `v58_13_a_session_stop_of_the_recorder_keeps_a_partial_capture`.*

**HD-7 amended**: `version: 1.1.0`, `impl_hash: Some(ContentHash::of_bytes(b"ezsdr.sink.capture 1.1.0"))` (Phase 4, VC-3); the `SinkDescriptor`'s `module` follows.

**HD-10 amended**: "and `ext` is `cf32` or `sc16` after the capture's first block" becomes "with the extension `sigmf-data` (HD-15; Phase 4, VC-3)".

**H3 amended**: the choice becomes "SigMF Recordings: HD-10's interleaved file as the Dataset, and a metadata file (HD-15)"; the ceiling "a capture spanning a SampleClock change has no metadata (HD-15)". **§9** drops "SigMF export (SC-32, Phase 4)". The header's "Not in scope" drops "SigMF and".

**SC-32 amended** (text and marker): "A SigMF export maps each run of delivered samples between stream gaps — the stream's valid segments — to a capture segment with `core:sample_start` and `core:global_index`, and carries the gaps and per-channel validity in an `ezsdr` extension namespace, since no existing SigMF extension covers validity; a SigMF capture segment belongs to the whole Recording, so a channel's own breaks are annotations, not segments." Its text said "each valid segment", which read per channel contradicts HD-15 and `hd_15_channel_validity_and_its_causes` (found by both reviews, `implementation-notes.md`). Marker: "*Checked in Phase 4 by `hd_15_a_capture_is_a_sigmf_recording` and `hd_15_channel_validity_and_its_causes` (HD-15).*"

**Rejected.** A separate SigMF Sink; a `format` selector key; a post-Run export function; a second `ArtifactRef` for the metadata (`00-overview.md` Q5, Q7). SigMF Collections, one Recording per channel (SigMF recommends them for multi-channel IQ, but a Collection of N files would split one capture's bytes, which HD-10 writes as one file with one hash; `core:num_channels` is valid SigMF).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `hd_15_a_capture_is_a_sigmf_recording` | a 1-channel cf32 capture of 300 samples from tick 100, over a block `[100, 200)` and one `[250, 450)` carrying `GAP_BEFORE｜RESTARTED` with `lost` 50 | files `…_rec.sigmf-data` (the 300 delivered samples) and `…_rec.sigmf-meta`; the meta equals, member for member, a literal: two capture segments `{0, 100}` and `{100, 250}`, one `ezsdr:gaps` entry `{100, 200, 50, 50, { kind: overflow_restart }, 0}`, `ezsdr:valid` `[[{0, 100}, {100, 200}]]`, no annotations | HD-15, HD-10, SC-32 |
| `hd_15_channel_validity_and_its_causes` | a 2-channel capture of four 10-sample blocks; channel 1 invalid on the second, which carries `ALIGNMENT`, and on the fourth, which does not | annotations on channel 1 at `{10, 10}` with cause `{ kind: alignment }` and `{30, 10}` with `{ kind: stream }` (SC-31c: the last runs to the end); `ezsdr:valid` `[[{0, 40}], [{0, 10}, {20, 10}]]` | HD-15, SC-31a, SC-31c, SC-32 |
| `hd_15_a_capture_across_a_clock_change_has_no_metadata` | a capture spanning a new SampleClock | a `.sigmf-data` file and no `.sigmf-meta` | HD-15 |
| `hd_15_the_datatype_follows_the_contract` | `sigmf_meta` of a one-block map at rate 3/2 with `ezsdr.stream.cf32`, `ezsdr.stream.sc16` and another contract | `cf32_le`, `ci16_le`, an error naming HD-15; `core:sample_rate` 1.5 and `ezsdr:sample_rate` `{3, 2}` | HD-15 |
| `hd_15_gap_fields_carry_the_continuity_causes`, `hd_15_annotations_are_sorted_by_index_then_channel`, `hd_15_a_map_it_cannot_describe_is_refused`, `hd_15_a_partial_capture_has_metadata`, `hd_15_an_sc16_capture_is_ci16_le` (after the review) | `design/10-host-data-path.md` §7 | the gap fields and causes, the zero-extent gap, the annotation order, the refusal of hand-built maps, the partial flag and the staging file, sc16 end to end | HD-15 |
| `v51_an_overflowed_capture_is_a_sigmf_recording` (acceptance) | `v58_06`'s overflow Run | the artifact's `uri` ends in `.sigmf-data`; its meta has the capture segments `{0, 0}` and `{1 000, 51 000}`, `core:sample_rate` 1 000 000, and one `ezsdr:gaps` entry at global index 1 000 of 50 000 samples, all lost, cause `overflow_restart` | HD-15 |

`hd_07_descriptor` reads 1.1.0 and its implementation hash (GW-3).

---

## 3. Vision issues found

Applied at Step X with the owner's approval (OV-6, GW-5), recorded in [`vision-issues.md`](vision-issues.md):

1. **§28 and §51** say SigMF interoperability "should be considered". It now exists; each gains a `Normative:` line naming `design/10-host-data-path.md` HD-15 and `design/02-stream-contract.md` SC-32 (re-review R13).
2. **§29** says the hot path "emits a fixed-size record with at most 32 bytes inline", and nothing about who reads those bytes. A sentence: "The bytes are the owning Vocabulary's layout, which it also decodes; the Kernel counts and queues them without interpreting them (`radio.RX_OVERFLOW` is the first, RM-24)."

---

## Appendix A — Mutations (GW-4)

The list is [`tools/mutations.json`](tools/mutations.json), run with `python3 plan/phase4/tools/mutate.py plan/phase4/tools/mutations.json <scratch-dir>` (Phase 3's tool with its phase directory changed): each mutation is applied alone in a scratch copy, the named test is run, and the result is recorded in `implementation-notes.md` as `mutation: <name>: killed`, or the phase stops. Six are run twice, against the unit test and against the end-to-end or coordinator carrier (D01/D02, D07/D08, D12/D13, D18/D19, D21/D22, D23/D24). D21–D34 were added after Reviews D and E, each for a finding (`implementation-notes.md`).

| Id | Change | Mutated crate | Killed by |
|---|---|---|---|
| D01 | KD-1 return at the first error | `ezsdr-kernel` | `ma_30_a_step_error_finishes_the_round` |
| D02 | KD-1 return at the first error, end to end | `ezsdr-kernel` | `kd_01_a_faulted_round_does_not_depend_on_fragment_names` |
| D03 | KD-1 a failed instance is stepped again | `ezsdr-kernel` | `ma_30_a_step_error_finishes_the_round` |
| D04 | KD-1 the last error wins | `ezsdr-kernel` | `ma_30_a_second_failure_does_not_replace_the_first` |
| D05 | RM-24 cause bytes swapped | `ezsdr-radio` | `rm_24_the_hot_form_round_trips` |
| D06 | RM-24 no length check | `ezsdr-radio` | `rm_24_the_hot_form_round_trips` |
| D07 | RM-24 the byte array refused | `ezsdr-radio` | `rm_24_a_delivered_payload_reads_in_either_form` |
| D08 | RM-24 the byte array refused, end to end | `ezsdr-radio` | `v58_04_mock_events_reach_counters_policy_and_manifest` |
| D09 | MR-37 the injected overflow on the control path | `ezsdr-mock-radio` | `mr_37_the_overflow_travels_the_hot_path` |
| D10 | MR-37 the back-pressure overrun on the control path | `ezsdr-mock-radio` | `mr_19_backpressure_is_an_overrun` |
| D11 | MR-1 radio requirement left at 1.1.0 | `ezsdr-mock-radio` | `mr_01_descriptor_registers` |
| D12 | HD-15 global index written as the file index | `ezsdr-sink-capture` | `hd_15_a_capture_is_a_sigmf_recording` |
| D13 | HD-15 global index written as the file index, end to end | `ezsdr-sink-capture` | `v51_an_overflowed_capture_is_a_sigmf_recording` |
| D14 | HD-15 the file index ignores earlier gaps | `ezsdr-sink-capture` | `hd_15_a_capture_is_a_sigmf_recording` |
| D15 | HD-15 metadata for a capture with two maps | `ezsdr-sink-capture` | `hd_15_a_capture_across_a_clock_change_has_no_metadata` |
| D16 | HD-15 sc16 as cf32_le | `ezsdr-sink-capture` | `hd_15_the_datatype_follows_the_contract` |
| D17 | HD-15 no channel annotations | `ezsdr-sink-capture` | `hd_15_channel_validity_and_its_causes` |
| D18 | HD-15 the Dataset keeps the raw extension | `ezsdr-sink-capture` | `hd_15_a_capture_is_a_sigmf_recording` |
| D19 | HD-15 the Dataset keeps the raw extension, end to end | `ezsdr-sink-capture` | `v51_an_overflowed_capture_is_a_sigmf_recording` |
| D20 | HD-7 implementation hash left at 1.0.0 | `ezsdr-sink-capture` | `hd_07_descriptor` |
| D21 | KD-1 the cap's STEP_LIVELOCK beats a failure | `ezsdr-kernel` | `ma_30_a_failure_beats_the_step_livelock_cap` |
| D22 | KD-1 the cap's STEP_LIVELOCK beats a failure, in the coordinator | `ezsdr-kernel` | `kd_01_a_device_lost_is_not_reported_as_a_step_livelock` |
| D23 | KD-1 only the first failure of a round is reported | `ezsdr-kernel` | `kd_01_every_lost_device_of_a_round_is_reported_and_the_first_failure_decides` |
| D24 | KD-1 only the first failure of a round is reported, end to end | `ezsdr-kernel` | `kd_01_a_faulted_round_does_not_depend_on_fragment_names` |
| D25 | KD-1 the last failure decides | `ezsdr-kernel` | `kd_01_every_lost_device_of_a_round_is_reported_and_the_first_failure_decides` |
| D26 | HD-15 annotations not sorted | `ezsdr-sink-capture` | `hd_15_annotations_are_sorted_by_index_then_channel` |
| D27 | HD-15 an empty run gets a capture segment | `ezsdr-sink-capture` | `hd_15_gap_fields_carry_the_continuity_causes` |
| D28 | HD-15 the Sink reports every capture as complete | `ezsdr-sink-capture` | `hd_15_a_partial_capture_has_metadata` |
| D29 | HD-15 a gap's lost written as its len | `ezsdr-sink-capture` | `hd_15_gap_fields_carry_the_continuity_causes` |
| D30 | HD-15 a gap's link_dropped written as 0 | `ezsdr-sink-capture` | `hd_15_gap_fields_carry_the_continuity_causes` |
| D31 | HD-15 a negative file index clamped instead of refused | `ezsdr-sink-capture` | `hd_15_a_map_it_cannot_describe_is_refused` |
| D32 | HD-15 the staged metadata never renamed into place | `ezsdr-sink-capture` | `hd_15_a_partial_capture_has_metadata` |
| D33 | MR-37 the overflow emitted at info severity | `ezsdr-mock-radio` | `mr_37_the_overflow_travels_the_hot_path` |
| D34 | MR-19 the back-pressure overrun reports no loss | `ezsdr-mock-radio` | `mr_19_backpressure_is_an_overrun` |
