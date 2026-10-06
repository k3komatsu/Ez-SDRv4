# Schema changelog

Every schema in this directory is generated from the Rust type that authors it and
is **the contract**: non-Rust consumers read these files, and `schema_freeze` fails
the build when regeneration does not reproduce one byte for byte
(`plan/phase1/00-overview.md` OV-10).

Regenerate with:

```bash
EZSDR_UPDATE_SCHEMAS=1 cargo test --test schema_freeze
```

Any schema diff requires an entry below (OV-12). After the v4.0 freeze a committed
`*.v1.json` is immutable: a change creates `*.v2.json` plus a migration or a refusal
(Vision §10, OV-12, SB-47, SB-48).

## v1 — spec 20 — the restart and start leads

Still version 1: v4.0 has not frozen. Not additive: `radio/envelope`'s `timing` gains the
required `restart_lead_ns` and `start_lead_ns` (spec 20, VF-6); an envelope written without
them is refused for the missing members, not reinterpreted (invariant 39).

## v1 — spec 20 — the component placement

Still version 1: v4.0 has not frozen. Not additive: `binding_profile`'s `placements` loses
`components`, and an Island's `components` lists `{ component, memory_domain }` entries
instead of names (`island_decl` with it) (spec 20, KH-2); a profile in the old shape is
refused by `deny_unknown_fields` or as a malformed entry, not reinterpreted (invariant 39).

## v1 — spec 20 — the prepare section

Still version 1: v4.0 has not frozen. Not additive: `manifest`'s `PrepareSection` loses
`merged_effective` (spec 20, KH-1); a Manifest that carries it is refused by
`deny_unknown_fields`, not reinterpreted (invariant 39). `server/reply_frame` embeds the
Manifest and changes with it.

## v1 — Phase 7 — what a device reports

Still version 1: v4.0 has not frozen. No Kernel schema changed (`plan/phase7/00-overview.md`
GZ-2); the Radio Model Vocabulary is `radio` 1.3.0.

- `radio/time_error_payload`: `outcome` gains the value `late_at_device`, meaning the Provider
  handed the burst to the device in time by its own clock and the device reported it late, so
  nothing was transmitted (RM-11, RM-22; Phase 7, VE-3). Not additive for a validating reader:
  one validating against the Phase 2 file would refuse the new value, so a validating reader
  regenerates from 1.3.0. A document from before 1.3.0 reads as before.
- New `radio/tx_underflow_payload` (`{ cause: "starved" | "lost" }`),
  `radio/alignment_error_payload` (`{ lost }`) and `radio/clock_lost_payload`
  (`{ reference: "frequency" }`): the payloads of the declared kinds `radio.TX_UNDERFLOW`,
  `radio.ALIGNMENT_ERROR` and `radio.CLOCK_LOST`, which a hardware Provider emits (RM-10,
  RM-11, RM-22; Phase 7, VE-3). `schemas/radio/` now holds eleven files (RM-20).
- `server/reply_frame`: the `status` reply gains the required member `root_rate`
  (`Rational`, the primary root's nominal rate in ticks per second), so that a client can
  name an instant ahead of the Run's time (EA-4, EA-12; Phase 7, VE-6). Additive for a
  reader of replies; `ezsdr-server` 0.2.0 always sends it.

## v1 — Phase 6 — `sink.CAPTURE_WRITTEN` and the protocol frames

Still version 1: v4.0 has not frozen. Additive; no Kernel schema changes.

- New `sink/capture_written_payload` (Phase 6, VD-1; HD-16): the payload of the `sink` 1.1.0
  event kind `sink.CAPTURE_WRITTEN`, `{ artifact: ArtifactRef }`, emitted by the capture Sink
  1.2.0 when it records a capture, so a client learns during the Session that the capture is
  written and where. After Phase 6 Review H (P0-2) it also carries `request`, the capture
  request's number, and `sink/request_rejected_payload` gains the same optional member
  (absent for a refused Action that is no capture request, so Phase 2's payloads read as
  before).
- New `server/request_frame` and `server/reply_frame` (Phase 6, spec 16 EA-6): the frames of
  `ezsdr.protocol` 1 between a client and `ezsdr-server`, which embed the Kernel's documents
  and add no Kernel type. After Review H, `wait_for` takes `within_ns` or `until` and its
  reply carries `horizon`. After Review I, `finished`'s `path` is optional: absent when the
  Manifest could not be written.

## v1 — Phase 5 — a Spec's inputs

Still version 1: v4.0 has not frozen. Additive.

- `experiment_spec`: adds the optional `inputs`, an array of `ArtifactRef`: artifacts the Run
  consumes that no schedule entry carries, such as the waveform a Reactor transmits. KC-9
  verifies and stores them as it does a scheduled waveform (SB-20a; Phase 5, KE-1). A document
  without it parses as before; absent means empty.
- `component_descriptor`, `experiment_spec`: the description of `ComponentKind`'s `reactor`
  value, which said a Reactor "reacts to samples"; Vision §19 says events and messages (Phase 5
  Review F, P1-2). Description only; no document changes meaning.

## v1 — Phase 4 — the hot-path form of `RX_OVERFLOW`

Still version 1: v4.0 has not frozen. Additive. No Kernel schema changed.

- New `radio/rx_overflow_hot_payload`: `RX_OVERFLOW`'s payload as a Manifest holds it when a
  Provider emitted it on the hot path — the Kernel's drain makes the record's 17 bytes a JSON
  array (RS-34); RM-24 defines the layout and `RxOverflowPayload::from_payload` reads both
  forms (Phase 4, VC-1). `radio/rx_overflow_payload` is unchanged and still describes the
  control-path object.

## v1 — Phase 3 — the SimulationChannel

Still version 1: v4.0 has not frozen. Additive. No Kernel schema changed.

- New `sim/channel`: the `sim.channel` environment section, `ChannelSpec { couplings, noise_dbfs }`
  (Phase 3, CH-1, CH-10).

## v1 — 2026-09-24 — pre-freeze revision, Phase 2 KA-7 and KA-10

Still version 1: v4.0 has not frozen. Additive.

- `provider_instance`: adds the optional `min_command_lead: Duration` (KA-7), the one
  envelope value the Kernel reads. A document without it parses as before; absent means
  zero.
- `action`, `action_template`, `component_descriptor`, `experiment_spec` and
  `vocabulary_descriptor`: the descriptions of the four `UpdateClass` variants say what
  each class means (KA-10, UC-3…UC-6). No field, type or constraint changed.
- New `sim/fault_entry` and `sim/seed` (Phase 2, SE-12).
- New `radio/rf_envelope`, `radio/envelope` and the five `radio/*_payload` schemas (Phase 2, RM-20, RM-22).
- New `sink/request_rejected_payload` (Phase 2, HD-14); `action` is the rejected
  Action's kind tag, including `update_parameter` for malformed capture requests.

## v1 — 2026-09-23 — descriptions only, Phase 1 Step 5

Still version 1. No field, type or constraint changed.

- `binding_profile`, `log_entry` and `manifest`: three descriptions quoted Vision text that
  Step 5 (re-review R13) rewrote — §8's `provider:`, §3's `StartRepeat` and `Capture`, and
  §50's "random seeds". They now say what the Vision wrote before Step 5.

## v1 — 2026-09-23 — pre-freeze revision, adopted D104

Still version 1: v4.0 has not frozen. Additive.

- New `stop_mode` and `step_outcome`: `Provider::stop` and `Provider::step` carry them,
  and MA-46 fixes the schema of every document type a role signature carries (D104).
  `stop_cause` is unchanged; it stays on `Action::Abort`.

## v1 — 2026-09-23 — pre-freeze revision, adopted D95–D103 (the binding and role model)

Still version 1: v4.0 has not frozen.

- `binding_profile`: **`authority` is required** (D97). The Authority is named and never
  inferred. **Breaking for existing documents:** a profile without the field no longer
  parses; add `authority` naming the resource the Authority rides on, or a binding of
  its own.
- `authority_descriptor`: adds the required `module: ModuleRef` (D98), compared with the
  binding it is supplied under, as the Executor, Sink and Link descriptors' already are.
- `log_entry` and `manifest` (its action log): `RunChild` gains the required
  `binding_hash` (D103), since a child Run's profile
  cannot be its parent Session's (RS-25a).
- `binding_profile`, `component_descriptor`, `data_link_decl`, `execution_plan`,
  `experiment_spec`, `manifest`, `provider_instance` and `resource`: `Port.name`, `PortRef.component` and
  `PortRef.port` reference `Ident` instead of a bare string (D95). **Narrows the accepted
  set:** a port name outside SB-1's grammar parsed before and could never be handed to
  a Module, whose `PrepareContext.links[].port` is an `Ident`.
- `component_descriptor` and `experiment_spec`: `ComponentRequires.executor_kind`
  references `Namespace` instead of a bare string (D95): it is a kind the Kernel reads and
  compares, and `any` is itself a `Namespace`. **Narrows the accepted set** to SB-1's
  grammar; a malformed kind other than `any` was refused later by MA-39 anyway.
- Descriptions of `Key`, `ModuleId`, `EventKind`, `ContentHash`, `DataContractId` and
  `ResourceId` now state their grammars (D95). Stale `MA-19` citations are corrected:
  `ModuleId` and `ModuleRef.id` cite SB-1 and MA-31, `NodeId` cites MA-38 for the Island
  ids it qualifies, and `ComponentImpl.id` cites MA-19b. No `pattern` is added: a grammar may still be loosened, and loosening
  would then be a v2 of every schema embedding the type (D94's reasoning). The Kernel
  enforces the grammars at deserialisation instead.

## v1 — 2026-09-23 — pre-freeze revision, adopted D51–D68

Still version 1: v4.0 has not frozen, so this is a pre-freeze revision of the
published v1 schemas.

- `vocabulary_descriptor`: withdraws `EventKindDecl.hot_layout` and its `HotLayout`
  type (D51). The Kernel no longer interprets a Vocabulary's hot-path byte layout.
- `action`: corrects `Action::Emit`'s withdrawn `RS-32a` citation to `RS-31`.
- `manifest`: adds the resolved `policy` table (D59), recording the compiled
  Policy named by RS-1: the Spec's `policies.failure` overrides resolved against
  the registered defaults. It is optional (D69): absent for a Run that failed
  before its Policy compiled, as `plan` is absent before `plan()`.
- `binding_profile`: `placements.links` selects a Link Module per graph link (D67;
  its `link-{i}` key grammar, the zero-based index in `ExperimentSpec.graph.links`,
  is superseded by D76 below and never froze). **Breaking for
  existing documents:** `LinkPlacement.link` changes type from an unversioned
  `ModuleId` string to a versioned `ModuleRef` object `{ id, version }`, and
  admission resolves the descriptor for that exact Module version. The previously
  unused `memory_domain` field was removed.

The same revision then adopted D69–D80 (re-review of D51–D68):

- **all object schemas** gain `additionalProperties: false` (D73): an unknown key at
  any level of a Kernel document is refused, not dropped (SB-9, SB-21, §65 #39).
  Extension content lives only in the namespaced maps the Kernel names. This
  includes the four id objects (`ClockDomainId`, `MemoryDomainId`, `IslandId`,
  `DataLinkId`) and `Version`, whose multi-line derives the first pass missed, so a
  pinned `{ major, minor, patch, pre }` is refused rather than read as the release.
  Every field-less variant of an internally tagged enum is now an empty struct
  variant (`Completed {}`), because serde does not apply `deny_unknown_fields` to a
  unit variant; the schemas and the serialised form are unchanged, and
  `{"kind":"completed","stage":"validate"}` is refused instead of read as `Completed`.
- `binding_profile`: **breaking** — `placements.links` changes from a map keyed
  `link-{i}` to an array of `{ link, from, to }`, each naming its data link by its
  two ends, and covers output feeds as well as graph links (D75, D76; reverses the
  key grammar of D67). `Binding.module` changes from an unversioned `ModuleId` to a
  versioned `ModuleRef` (D78).
- `fragment`, `execution_plan` and `manifest` (which embed it): `Fragment.instance` changes
  from `ModuleId` to `ModuleRef` (D78), and `links` now also carries the output
  feeds, numbered after the graph's links (D75).
- `link_descriptor` and `vocabulary_descriptor`: description text only.
- `binding_profile`, `execution_plan`, `manifest`: description text for
  `Placements` and `ExecutionPlan.links` now names output feeds (D75).

And then D81–D84 (second re-review):

- `sink_descriptor`: adds `memory_domains`, the domains a Sink reads from; MA-39
  checks an output feed against it (D81).
- `executor_descriptor`, `sink_descriptor`, `link_descriptor`: add `module`, the
  exact `ModuleRef` the instance is. Admission refuses an Executor (at `plan()`) or a
  Sink (at `validate` and `plan()`) whose
  `module` differs from its binding, and a Link descriptor is registered under its
  own `module` (D82).
- `binding_profile`: description text of `Binding.module` names the per-role
  comparison (D82).
- `executor_descriptor`, `link_descriptor`: description text (D82).

And then D88–D91 (fourth re-review), none of which changes a schema:

- Every document refuses a `node` other than 0 (`LOCAL`) at any depth: `NodeId`'s
  deserialiser enforces X7 (D91). The schemas keep `uint32`, so lifting X7 later is
  a code loosening rather than a schema change.

## Named deviation: the integer profile

Content hashes are taken over **RFC 8785 (JCS) with an integer profile**: a value
serialised from an integer type is written as an exact decimal with no exponent,
whatever its magnitude, while a value serialised from a floating-point type takes
RFC 8785's ECMAScript `Number::toString` form. Non-finite numbers are rejected
rather than written as `null`.

Strict JCS routes every number through an IEEE-754 double, so two Manifests whose
epoch-to-UTC offsets differ by 1 ns in 1.7 × 10^18 would share a hash. **No
implementation that lacks the integer profile may be substituted**, which is why a
stock JCS crate cannot be dropped in later (OV-15, OV-15a).

64-bit integers travel in documents as JSON `integer` with `format: int64`, written
as an exact decimal by the canonicaliser; consumers are required to use a 64-bit-safe
parser (X3).

## v1 — 2026-09-22 — pre-freeze revision, exit-criterion-2 sweep

Still **version 1**, for the reason the sections below give.

- `binding_profile`: `ComponentPlacement`'s `description` no longer cites **SB-25a**,
  which is withdrawn (OV-1 keeps its number). A description is part of the committed
  contract a non-Rust consumer reads, so it was pointing them at a rule that states no
  obligation. No field, type or constraint changed.

## v1 — 2026-09-22 — pre-freeze revision, adopted the D45–D50 verdicts

Still **version 1**, for the reason the sections below give.

- `vocabulary_descriptor`: `CompileRule::TxBurst` gains a mandatory `late_policy`.
  Which of SC-27's behaviours a Session verb means is the Vocabulary's decision, and
  `compile` was supplying `SendAsapAndFlag` for every verb — a Vocabulary meaning
  living in Core (OV-21). `RejectAtPlan` is refused at registration, because a Session
  burst's target is resolved at `compile` and never passes that stage (finding D46).

## v1 — 2026-09-22 — pre-freeze revision, adopted the Fable second-opinion findings

Still **version 1**, for the reason the sections below give.

- `admission_result`, `manifest`: `coercions_preview` becomes a list of
  `PreviewedCoercion { resource, coercion }` rather than a bare `Coercion` list. A
  `Coercion` names a key and two values, because a Provider answers about the request
  it was handed; the Kernel's own preview has to say which Spec resource it was
  computed for. Keyed on the key alone, `prepare`'s SB-44 check charged one
  resource's coercion to every other resource constraining that key and refused the
  ordinary two-channel Spec whenever one channel coerced (finding F1).

## v1 — 2026-09-22 — pre-freeze revision, adopted review findings (second round)

Still version 1, for the reason the entry below gives.

- `prepare_report`, `manifest`: `PrepareReport` loses `constraints_hit`. No rule consumed
  it, and neither the Vision's sketch nor audit §13 states what it means; `warnings`
  carries what a Provider would have said with it (decision D40).

## v1 — 2026-09-22 — pre-freeze revision, adopted second-opinion findings

Still **version 1**: the Kernel is `4.0.0-alpha.1` and OV-12's immutability begins at
the v4.0 freeze, so these are revisions of v1 rather than a v2 with migrations. They
are listed because a consumer that read an earlier `v1.json` will see them.

- `action`, `action_template`: `UpdateParameter` gains an optional `at`
  (`AbsoluteDeadline`). Without it `hardware_timed` was unimplementable and RS-19's
  admitted instant was computed and then discarded (open question 2).
- `manifest`: gains the mandatory `version` Vision §10 requires of it by name. The
  Manifest outlives every Run, and without the field SB-47's migrate-or-refuse had
  nothing to read (decision R10).
- `data_contract`: `Scalar` is now untagged, so an attribute value is the scalar an
  author wrote rather than a `{kind, value}` wrapper. OV-13 carves it out together
  with `Value` (finding D23).
- `experiment_spec`: `OutputReq.source` / `OutputSource` are replaced by
  `feed: SinkFeed { port, policy, capacity }`. An output *is* the declaration of the
  link that feeds its Sink, so SC-19's mandatory policy and capacity live there
  (finding D17).
- `binding_profile`: `Binding.provider` is renamed `module`, because one map now
  binds Providers, Sinks and Executors; `Binding` gains an optional `feed` for a
  Session's Sink binding; `ComponentPlacement` loses `module`, withdrawn with SB-25a
  (findings D17, D18).
- `resource`, `provider_instance`: `Resource` gains `shareable` (default `false`),
  so exclusivity is the Provider's declaration and never the Kernel's knowledge, and
  `ports`, so a Spec can link a component to a bound resource's stream port
  (findings D27, D31).
- `vocabulary_descriptor`: `KeyDecl` gains an optional `update_class`, which is
  where a Provider parameter's class is declared (finding D33).

## v1 — 2026-09-22 — Phase 1

First publication. 45 document schemas generated at JSON Schema 2020-12 from
`ezsdr-kernel` 4.0.0-alpha.1, covering the types `00-overview.md` §6 lists as
documents:

- time: `clock_domain`, `clock_relation`, `time_point`, `duration`,
  `relative_budget`, `absolute_deadline`, `sample_clock_record`
- stream: `data_contract`, `data_link_decl`, `continuity_map`, `gap`, `burst_record`
- spec and binding: `experiment_spec`, `binding_profile`, `admission_result`,
  `violation`, `coercion`, `execution_plan`, `fragment`, `prepare_report`,
  `coerce_report`, `requested`
- module API: `module_descriptor`, `vocabulary_descriptor`, `provider_instance`,
  `resource`, `executor_descriptor`, `sink_descriptor`, `link_descriptor`,
  `authority_descriptor`, `component_descriptor`, `island_decl`, `execution_class`,
  `fidelity`, `module_error`
- run, session and manifest: `action`, `action_template`, `event`, `log_entry`,
  `lease`, `policy`, `stop_cause`, `artifact_ref`, `content_hash`, `manifest`

`SampleBlock`, `BufferRef`, `EventRecord`, the memory pools and their handles, the
link handles, `EventSink`, `ActionReceiver`, `ActionSubmitter` and `TimeAuthority`
have **no schema by design**: they are in-process only (X8).
