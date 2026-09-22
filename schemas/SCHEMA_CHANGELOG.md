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
