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
