# Phase 2 spec 10 — The host data path: host memory, the host Link, the `sink` Vocabulary and the capture Sink

| Field | Value |
|---|---|
| Status | Accepted at Gate P (owner, 2026-09-24; `00-overview.md` §11). Normative for `crates/ezsdr-hostmem`, `crates/ezsdr-link-host`, `crates/ezsdr-sink` and `crates/ezsdr-sink-capture`. |
| Scope | The `host` memory domain and a buffer pool (SC-9's Phase 2 helper); the planar-to-interleaved sample layout helpers; the Link Module `ezsdr.link.host` 1.0.0, the first real `DataLink` (MA-28a); the `sink` Vocabulary 1.0.0, which owns the `capture` verb (RS-13a, RS-14); the capture Sink Module `ezsdr.sink.capture` 1.0.0, the first real consumer of a MockRadio stream. |
| Not in scope | SigMF and other artifact formats (Phase 4); non-host memory domains; lock-free rings (Phase 7+); a copy-regression benchmark (Phase 8). |
| Crates | `ezsdr-hostmem` (depends on `ezsdr-kernel`), `ezsdr-link-host` (Module; `ezsdr-kernel`), `ezsdr-sink` (Vocabulary; `ezsdr-kernel`, `serde`, `serde_json`, `schemars`), `ezsdr-sink-capture` (Module; `ezsdr-kernel`, `ezsdr-sink`, `ezsdr-hostmem`, `serde_json`). |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

---

## 1. Purpose

A MockRadio stream is useful only when something consumes it. Vision §30 makes a recorder a Sink fed by a drop-class link, §28 makes its continuity metadata derived from headers, and §50 makes its output an artifact the Manifest references by hash. This document specifies the smallest set of pieces that carries a block from a Provider's port to a hashed file: where host bytes live, the link that moves a `BlockRef`, and a Sink that writes captures.

## 2. Host memory (`ezsdr-hostmem`)

- **HD-1** `ezsdr_hostmem::HOST_MEMORY: MemoryDomainId = MemoryDomainId::local(0)` is the host memory domain. Every Phase 2 Module that produces or consumes host bytes declares and uses this id (SC-6: a domain's kind is Vocabulary content, and this crate is where the kind `host` is defined).
- **HD-2** `HostPool::new(slot_bytes: usize)` holds shared buffers of `slot_bytes` bytes. `HostPool::fill(&mut self, len: usize, write: impl FnOnce(&mut [u8])) -> Arc<[u8]>` returns a buffer whose first `len` bytes `write` has written: when `len ≤ slot_bytes`, a slot no block references (`Arc::strong_count == 1`) is reused through `Arc::get_mut`, or a new slot is added when every slot is referenced; when `len > slot_bytes`, a buffer of exactly `len` bytes is allocated and not pooled. `HostPool::slots(&self) -> usize` reports how many slots exist. A slot is reused only when no block holds it, which is SC-9's rule; the bytes after `len` in a reused slot are unspecified and no consumer reads them (SC-10a's planar length bounds every read). *Ceiling: the pool grows without bound if its consumer never releases blocks; a fixed capacity with refusal is the Phase 7 upgrade for the real-time path.* *Checked by `hd_02_a_released_slot_is_reused_and_a_held_one_is_not`.*
- **HD-3** Sample layout helpers, for SC-4's planar blocks and Vision §51's interleaved files:
  - `write_cf32(buf: &mut [u8], len: usize, channel: usize, index: usize, re: f32, im: f32)` writes sample `index` of `channel` at byte `(channel · len + index) · 8`, `re` then `im`, little-endian;
  - `read_cf32(buf: &[u8], len: usize, channel: usize, index: usize) -> (f32, f32)`;
  - `interleave(block: &SampleBlock, bytes_per_sample: usize, from: usize, to: usize) -> Vec<u8>` returns samples `from … to − 1` of the block's host bytes, for each sample every channel in order (`[s0c0, s0c1, …, s1c0, …]`), each `bytes_per_sample` bytes copied as stored; `None` host bytes, or `from > to` or `to > len`, is a panic-free empty `Vec` and the caller's error.

  *Checked by `hd_03_layout_round_trip`.*

## 3. The host Link Module (`ezsdr.link.host` 1.0.0)

- **HD-4** The Module is `ModuleDescriptor { id: ezsdr.link.host, version: 1.0.0, kernel_api: 4.0.0, roles: [Link], vocabularies: [], deployment: InProcess, impl_hash: Some(ContentHash::of_bytes(b"ezsdr.link.host 1.0.0")) }` and its descriptor is `LinkDescriptor { module: ezsdr.link.host 1.0.0, kind: ezsdr.link.host, connects: [(HOST_MEMORY, HOST_MEMORY)], policies: [block, drop_oldest, drop_newest], cross_process: false }`. `HostLinkModule::new()` implements `Link`; `create(decl)` refuses a `capacity` of 0 with `Rejected` and returns a `HostLink` with the declaration's policy and capacity. *Checked by `hd_04_descriptor_and_create`.*
- **HD-5** `HostLink` implements `DataLink` exactly as SC-20, SC-20a and SC-20b say: under `Block` a publish at capacity returns `Full` and keeps nothing; under `DropOldest` the oldest queued block is evicted, absorbed into the carry and counted; under `DropNewest` the new block is refused, absorbed and counted; `drops()` never drops a count; `take_drop_carry()` returns and clears the carry; `publish` never parks. *This carries MA-28a.* *Checked by `hd_05_policies`, which is Phase 1's `sc_20_*` and `sc_20b_*` suite run against `HostLink`.*

## 4. The `sink` Vocabulary (`sink` 1.0.0)

- **HD-6** The Vocabulary is `VocabularyDescriptor { id: sink, version: 1.0.0, prefix: sink, keys: [KeyDecl { key: sink.capture_samples, kind: int, coercible: false, coercion_default: reject, update_class: Some(block_boundary) }], event_kinds: [EventKindDecl { kind: sink.REQUEST_REJECTED, default: continue, severity: warning }] (HD-14), verbs: [VerbDecl { verb: capture, compiles_to: UpdateParameter { key: sink.capture_samples, class: block_boundary } }], checks: [] }`, returned by `ezsdr_sink::vocabulary()`; `ezsdr_sink::register(registry, checks, kinds)` registers it and its one event kind with owner `Some(sink)`. `ezsdr_sink::CAPTURE_ARTIFACT_KIND = "sink.capture"`. A `capture` action carries `params: { sink.capture_samples: N }` with `N ≥ 1` and optionally `at` (RS-14, RS-19). *Checked by `hd_06_vocabulary`.*

## 5. The capture Sink Module (`ezsdr.sink.capture` 1.0.0)

- **HD-7** The Module is `ModuleDescriptor { id: ezsdr.sink.capture, version: 1.0.0, kernel_api: 4.0.0, roles: [Sink], vocabularies: [{ sink, ^1.0.0 }], deployment: InProcess, impl_hash: Some(ContentHash::of_bytes(b"ezsdr.sink.capture 1.0.0")) }`. Its `SinkDescriptor` is `{ module: ezsdr.sink.capture 1.0.0, kind: ezsdr.sink.capture, memory_domains: [HOST_MEMORY], contracts: [ezsdr.stream.cf32, ezsdr.stream.sc16], artifact_kinds: [sink.capture] }`. *Checked by `hd_07_descriptor`.*
- **HD-8** `CaptureSink::from_binding(binding)` refuses with `Rejected` a `module` other than HD-7's, a `profile`, and a selector other than exactly `{ dir: str }`. `dir` is the directory the Sink writes into; a relative path is taken relative to the process's working directory. A `feed` is accepted and not read (the coordinator builds the link, KC-10).
- **HD-9** `prepare(fragment, ctx)`: `fragment.content` must parse as an `OutputReq` (a Spec Run's output, or a Session's implicit one, SB-22c) whose `params` hold nothing but, optionally, `sink.capture_samples` as an `Int` ≥ 1; `ctx.links` must be exactly one `{ component: fragment.id, port: in, StreamIn }`; `dir` is created with `create_dir_all`. Each failure is `Rejected`. It keeps `ctx.clocks`, `ctx.time`, `ctx.events`, `ctx.actions` and `ctx.run`, and returns `PrepareReport { fragment: fragment.id, effective: {}, coercions: [], warnings: [] }`. The contract of what it receives is read from each block's header (HD-10), because the Sink's fragment carries the feed's port and policy, not its contract. *Checked by `hd_09_prepare_cases`.*
- **HD-10** **Captures.** A capture records the first `N` samples its feed **delivers** at or after its start instant, across gaps and SampleClock changes, into one artifact:
  - the output's **own capture**: when its `params` hold `sink.capture_samples = N`, one capture of `N` samples starting at the first delivered sample; when they do not, there is none;
  - **requests**: each admitted `UpdateParameter { key: sink.capture_samples, value: Int(N), at }` the Sink receives — from a Session's `capture` verb or from a Spec's schedule, alike — is a capture request, with `N ≥ 1` (another value, or a value that is not an `Int`, is refused as HD-14 says); requests are served one at a time, in arrival order, after the own capture: each starts at the first delivered sample whose instant is at or after `max(at, the end of the capture before it)`, comparing in the delivered block's SampleClock with `at` converted into it with `ClockRegistry::convert` and rounded up (an `at` absent means no lower bound; an `at` that cannot be converted, because its root is another, is refused as HD-14 says);
  - with no own capture and no request, the output produces no artifact: there is no "record everything" mode in 1.0.0, because the Sink cannot tell a Spec output from a Session's implicit one and a mode that depended on it would be a guess;
  - the artifact id is the output id for the own capture and `<output id>_<k>` for the `k`-th **accepted** request, `k` counting from 0 (a request HD-14 drops takes no `k`);
  - the bytes per sample are the block header's contract's: 8 for `ezsdr.stream.cf32`, 4 for `ezsdr.stream.sc16` (SC-4); a block of any other contract makes `step` return `Rejected`; a block whose contract differs from the capture's first block's finishes that capture with `partial: true` and the block is processed for the next capture;
  - the file is `<dir>/<run id>_<artifact id>.<ext>`, where every character of the run id outside `[A-Za-z0-9_-]` is replaced by `_` and `ext` is `cf32` or `sc16` after the capture's first block; its bytes are the captured samples interleaved (HD-3), in arrival order;
  - `continuity` is derived by `ContinuityBuilder`s (SC-30), `lossless: false` because a feed is drop-class (SC-21): one builder per SampleClock and channel count; `DomainChanged` or `ChannelsChanged` finishes the current builder with the returned carry and starts a new one (SC-30c); before every block the Sink takes the link's `DropCarry` and passes it with the block's first push (SC-30b) — but only to a capture that was already recording before that block; a carry taken while no capture was recording, including for a block in which a capture starts, is discarded, because the drops it counts precede the capture. The header pushed for a block is the part inside the capture: its first sample time and length are the overlap's, and its `flags` and `lost` are the block's when the overlap starts at the block's first sample and `NONE` otherwise;
  - a capture completes when it has recorded `N` samples: its builders finish, the file is closed, and the artifact is `ArtifactRef { id, kind: sink.capture, uri: "file://<absolute path>", hash: ContentHash::of_bytes(file bytes), size_bytes, partial: false, marks: [], continuity }`.

  *Checked by `hd_10_capture_of_n_samples_across_jittered_blocks`, `hd_10_a_capture_spans_a_gap_and_a_clock_change`, `hd_10_session_requests_are_sequential`, `hd_10_the_carry_attributes_a_dropped_overflow`.*
- **HD-11** `step(until)` first handles every Action from `ctx.actions` — an `UpdateParameter` on `sink.capture_samples` is a request (HD-10); a `Stop` targeting the Sink finishes the recording capture as `stop(orderly)` would; anything else is refused as HD-14 says — then receives every queued block from its link and processes it, and reports `progressed` when it handled an Action or received a block. It never blocks and never calls `wait_until` (MA-20).
- **HD-14** A request or Action the Sink cannot carry out — a capture of a value that is not an `Int` ≥ 1, an `at` that cannot be converted into the stream's SampleClock, an Action other than those of HD-11 — is dropped, and the Sink emits `sink.REQUEST_REJECTED` with `emit_control`, source `sink/<output id>`, time the current instant in the primary root, and payload `serde_json::to_value(RequestRejectedPayload { action: <the Action's kind tag>, reason: "HD-14: <why>" })`; `step` then continues and returns `Ok`. A client's malformed request is the client's error and must not end the Run, which `Err` from `step` would (KC-30). `RequestRejectedPayload { action: String, reason: String }` (`deny_unknown_fields`) is defined in `ezsdr_sink` and its schema is committed as `schemas/sink/request_rejected_payload.v1.json` (PO-7). A block of a contract other than `cf32` or `sc16` still makes `step` return `Rejected`: `plan()` refuses such a feed (SC-3), so reaching it is a defect, not a request. *Checked by `hd_11_an_unexpected_action_is_rejected`, `hd_14_a_bad_capture_value_is_an_event_not_a_failure` and `hd_14_schema_freeze`.*
- **HD-12** The Sink assumes no block length, minimum or alignment (SC-15): it slices any block at any sample. *This discharges SC-15's consumer obligation for the one Phase 2 consumer.*
- **HD-13** `stop(mode)` finishes the capture being recorded, if any, with `partial: true`, since it holds fewer than its `N` samples, under either mode; a request that never started produces no artifact. It returns every finished artifact, completed or partial, in completion order (MA-26). `cleanup()` closes any open file and drops its handles; it is idempotent.

## 6. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| H1 | The pool | Shared `Arc<[u8]>` slots, reused when unreferenced (HD-2) | A ring with a fixed slot count and a refusal (needs a back-pressure story the Simulation class does not have) | Phase 7 |
| H2 | The link | A mutex around a queue (HD-5) | A lock-free SPSC ring (no measured need; the stepping loop is single-threaded) | a later Link Module version |
| H3 | Artifact format | Raw interleaved samples, one file per capture (HD-10) | SigMF now (Phase 4); planar per block (the file would need the block boundaries to be read) | Phase 4's SigMF Sink |
| H4 | What `N` counts | Delivered samples (HD-10) | Sample instants (a capture across a gap would hold fewer than `N` samples, and across a rate change the interval has no single unit) | none |
| H5 | Concurrent Session captures | Sequential, in arrival order (HD-10) | Overlapping captures (two files writing one stream, for no Phase 2 use) | Additive later |
| H6 | Where the `capture` verb lives | A `sink` Vocabulary crate (HD-6) | In the capture Sink crate (a later SigMF Sink would then depend on a Module, MA-3) | none |
| H7 | The artifact's hash | Over the file's bytes, read back at completion (HD-10) | An incremental hasher (a `sha2` dependency outside the Kernel, PO-4) | An incremental hash is an optimisation |
| H8 | What an output with no `sink.capture_samples` records | Nothing until a request arrives (HD-10) | Every delivered sample until `stop` (a Session's implicit output has empty `params` too, so every Session would record its whole stream to disk, and the Sink cannot tell the two apart) | A later additive key such as `sink.capture_all: bool` |

## 7. Tests

`crates/ezsdr-hostmem/tests/hostmem.rs`: `hd_02_a_released_slot_is_reused_and_a_held_one_is_not` (fill, drop the block, fill again: `slots()` stays 1; hold the first: `slots()` becomes 2); `hd_03_layout_round_trip` (write and read every sample of a 3-channel block; `interleave` over `[1, 3)` gives the six samples in `s1c0, s1c1, s1c2, s2c0, …` order).

`crates/ezsdr-link-host/tests/link_host.rs`: `hd_04_descriptor_and_create` (the descriptor registers; `create` with capacity 0 is refused); `hd_05_policies` (Phase 1's link suite against `HostLink`: `Block` full with no drop, `DropOldest`, `DropNewest`, the carry's flags and `lost` and its clearing, `Full` never discards).

`crates/ezsdr-sink/tests/sink_vocabulary.rs`: `hd_14_schema_freeze` (regenerate `RequestRejectedPayload`: byte-equal to `schemas/sink/request_rejected_payload.v1.json`, no other file there); `hd_06_vocabulary` (the descriptor, with its one kind; the Kernel registry accepts it; `session::compile` of a `capture` action with `params { sink.capture_samples: 1000 }` gives one `UpdateParameter` to `sink/<output id>` with class `block_boundary`).

`crates/ezsdr-sink-capture/tests/sink_capture.rs` (a local test link and `ManualTimeAuthority`; blocks built with `ezsdr-hostmem`):

| test | input | expected | rules |
|---|---|---|---|
| `hd_07_descriptor` | `descriptor()` | registers with `sink`; its `SinkDescriptor` as HD-7 says | HD-7 |
| `hd_09_prepare_cases` | no link; two links; a link on port `out`; a selector with an extra key (`from_binding`); `params { sink.capture_samples: 0 }`; `params { other.key: 1 }`; a `dir` under a regular file | each refused | HD-8, HD-9 |
| `hd_10_capture_of_n_samples_across_jittered_blocks` | 10 000 ramp samples in blocks of lengths 1, 7, 1 999, 3, … and a capture of 5 000 from sample 123 | the file holds samples 123…5 122, each equal to the ramp; one map, one segment | HD-10, HD-12 |
| `hd_10_a_capture_spans_a_gap_and_a_clock_change` | an overflow block (`GAP_BEFORE｜RESTARTED`, `lost` 50) then a new SampleClock | two maps; the first has one `OverflowRestart` gap with `lost` 50 | HD-10, SC-30, RS-40 |
| `hd_10_session_requests_are_sequential` | two requests at `at` 100 and 150 of 100 samples each | artifacts `rec_0` samples 100…199, `rec_1` samples 200…299 | HD-10 |
| `hd_10_the_own_capture_comes_first` | `params { sink.capture_samples: 50 }` and one request of 20 at `at` 0 | artifacts `rec` samples 0…49, then `rec_0` samples 50…69 | HD-10 |
| `hd_10_no_capture_no_artifact` | empty `params`, no request, 1 000 samples delivered | `stop` returns no artifact | HD-10, HD-13 |
| `hd_10_the_carry_attributes_a_dropped_overflow` | a `DropOldest` link evicting the block that carried `RESTARTED` | the gap's cause is `OverflowRestart` with its `lost`, and `link_dropped` 1 | HD-10, SC-30b |
| `hd_11_an_unexpected_action_is_rejected` | a `TxBurst` delivered to the Sink | `step` returns `Ok`; one `sink.REQUEST_REJECTED` with `action == "tx_burst"` and a reason beginning `"HD-14"` | HD-11, HD-14 |
| `hd_14_a_bad_capture_value_is_an_event_not_a_failure` | requests with `Int(0)`, `Num(5.0)` and an `at` on an unrelated root, then a valid one | three `sink.REQUEST_REJECTED` events; `step` returns `Ok` each time; the valid request's artifact is produced with id `rec_0` (a dropped request takes no `k`) | HD-14, HD-10 |
| `hd_13_partial_on_abort_and_on_an_unfinished_capture` | stop(abort) mid-capture; stop(orderly) with 4 000 of 5 000; a second request queued behind the first | `partial` true both; the unstarted request gives no artifact | HD-13, MA-26 |

A `Block` feed is not a Sink test: `validate` refuses it (SC-21), so it never reaches the Sink.

## 8. Vision issues found

1. **§30's recorder controls** ("sample every N blocks, decimate, maximum output rate, drop if busy") are not in `ezsdr.sink.capture` 1.0.0: `drop if busy` is the feed's drop-class policy, and the others are later Sink parameters. §30 lists them as controls a recorder *may* offer.

## 9. Deferred

SigMF export (SC-32, Phase 4). Pinned and GPU memory. A fixed-capacity pool. Recorder decimation.
