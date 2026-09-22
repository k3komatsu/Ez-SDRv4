# Phase 1 — Kernel semantic model: overview and plan

| Field | Value |
|---|---|
| Status | All five specs drafted; 01–04 have been through adversarial review and 05 is at Gate C. Governance rules `OV-n` become binding when this document is accepted. |
| Phase | Vision §67 Phase 1. Predecessor: Phase 0 (audit + re-review, complete). Successor: Phase 2 (Radio Model + Simulation Engine + MockRadio). |
| Scope | The five normative specs listed below, the decisions that cut across them, and the single Kernel crate that implements them. |
| Not in scope | MockRadio, the Simulation Engine, SimulationChannel, the Radio Model vocabulary, the Python client, UHD. Anything on the Vision §6 list. |
| Language | English. Rule IDs and normative words are quoted from code comments, test names and schema descriptions, so the specs and the code must use one vocabulary. |
| Location | `plan/phase1/` while drafting. On acceptance the five specs move to `design/`; this file stays in `plan/phase1/` as the record of how Phase 1 was run (§12). |

---

## 1. Why Phase 1 exists

The audit's readiness verdict (`design/v4-vision-audit.md` §14.3) is *READY WITH REQUIRED CHANGES*, and its reason is a sequencing argument rather than a design objection:

> the P0 items are all types that appear in every Provider and Processor signature of the first milestone; if they are left undecided while MockRadio is written, the Mock's implementation becomes the contract, and the Mock → X310 parity test of Phase 8 then fails against a contract that cannot be changed without breaking every Module.

Phase 1 therefore produces **decisions, not features**. Its output is five normative documents plus one crate whose tests run with no hardware, no Mock and no simulator. The measure of success is that Phase 2 can write MockRadio without deciding anything about time, blocks, admission, lifecycle or module boundaries.

The three types the audit singles out (§1.2) are settled first: `TimePoint` and the Time Authority (spec 01), `SampleBlock` / `BufferRef` and the Stream Contract (spec 02), and the composite Resource tree (spec 03).

---

## 2. Deliverables

```text
plan/phase1/
  00-overview.md         this file: cross-cutting decisions, crate and schema strategy, tests, traceability, exit criteria
  01-time-model.md       TM-n   ClockDomain, TimePoint, Duration, the two Deadline kinds, ClockRelation, TimeAuthority, SampleClock
  02-stream-contract.md  SC-n   DataContract, Port, MemoryDomain, BufferRef, SampleBlock, DataLink, BurstTracker, ContinuityMap
  03-spec-and-binding.md SB-n   ExperimentSpec, BindingProfile, Constraint and matcher, compile pipeline, PrepareReport, versioning
  04-run-and-session.md  RS-n   Run state machine, cleanup, Session action log, Lease, Policy, the Kernel
                                Action set, Event and counters, Manifest, hashing
  05-module-api.md       MA-n   the five role traits, ModuleDescriptor and registry, ComponentDescriptor, Island, ExecutionClass, fidelity
```

Every spec carries: header (status, scope, Vision § covered, audit §14.1 items covered) → evidence → model overview → types (language-neutral shape plus an illustrative Rust sketch) → normative rules with stable IDs → algorithms → decisions table → Phase 1 test table → Vision coverage table → Vision issues found → deferred items.

- **OV-1** A rule ID, once published in an accepted spec, is never reused or renumbered. A withdrawn rule keeps its number and is marked withdrawn, exactly as the Vision's §N numbers are stable.
- **OV-2** Every normative obligation carries a rule ID. A rule that places obligations on more than one implementer, or in more than one phase, is split into lettered sub-rules (`TM-13a`, `TM-13b`, …), which OV-1 protects like any other ID. A rule whose several clauses are checked by one function and one test stays whole and enumerates them. Text with no rule ID is explanatory and binds nothing.
- **OV-3** A rule with no disposition marker means "checked by a Kernel item and at least one Phase 1 test", and the exit review verifies that by finding a test row **whose expectation is that rule's own obligation**, not merely one citing its ID. The distinction matters: a producer-side rule such as spec 02's SC-13, an "if and only if" about what the producer knows, is cited by three rows that all test the builder's derivation instead, so an ID-matching check would pass it as covered. Every rule that is **not** in that class carries an explicit marker naming what it is — a **producer obligation**, a **consumer obligation**, a **forward obligation** or a **process obligation** — and the phase or artefact whose test covers it. The fourth marker exists because OV-4…OV-19 govern how the documents and the repository are written: their carrier is a document, `Cargo.toml` or the test tree, never a Kernel item, and the first three markers cannot describe them. The default's "a Kernel item" clause binds only where the rule names Kernel behaviour: OV-22, MA-44 and MA-45 are carried by test code, and searching `src/` for them is the wrong search. The default is inverted this way so that the marker is never a judgment call: the first draft said "written in the rule itself where it is not obvious", and the rules whose disposition was least obvious were exactly the ones left unmarked. The exit review produces a **per-rule table**: for an unmarked rule, the test whose assertion is that rule's obligation, **read from the test body**; for a marked rule, the marker and the phase or artefact. The table is the evidence. A rule-ID prefix on a test name is OV-19's navigation convention and is neither necessary nor sufficient here — three review passes each found a correctly prefixed test whose expectation was the implementation rather than the rule.
- **OV-4** A spec never restates a rule owned by another spec; it cites it. Cross-spec citation is by ID (`see TM-13c`), never by copying the text.
- **OV-4a** In a rule, "must" and "must not" are the normative verbs. "Should" does not appear inside a rule; a recommendation that is not binding belongs in explanatory text. Each spec repeats this in its header.

---

## 3. Sequencing and review gates

Dependencies are real: 02 needs 01's `TimePoint`; 03 needs 01's deadlines and 05's `coerce` / `PrepareContext` shapes; 04 needs 01, 02 and 03; 05 needs all four for its signatures. Three gates, not five serial reviews and not one big-bang review — the re-review's R1, R2 and R21 were all introduced by cross-document edits that nobody reviewed as a pair.

| Step | Documents | Gate | Size estimate |
|---|---|---|---|
| 0 | `00-overview.md` first version | — | ~380 lines |
| 1 | `01-time-model.md` + `02-stream-contract.md` | **Gate A** — the two most type-shaping specs | 01: 250–350, 02: 350–450 |
| 2 | `03-spec-and-binding.md` + `04-run-and-session.md` | **Gate B** | 03: 400–500, 04: 350–450 |
| 3 | `05-module-api.md`, then this file finalised (decision log, traceability) | **Gate C** | 05: 350–450 |
| 4 | The crate, in module order `id → time → stream/contract → hash → module_api → spec/binding → plan → event/policy → run/session → manifest → schema` | exit review | src 3.5–4.5k lines, tests ~3k (±50 %) |
| 5 | Accepted specs move to `design/` (§12) | — | — |

- **OV-5** At each gate the owner reads the decisions table first, then the rules, then the test table. Disagreements are recorded in §11's decision log with a verdict, never silently edited into the Vision.
- **OV-6** The Vision, the audit and the re-review are not edited during Phase 1. Everything a spec would want to change about the Vision is collected under "Vision issues found" and applied in one pass at Step 5 (§12), with the owner's approval.
- **OV-7** After each gate, `handoff.md` §2–§4 is updated to the new state. Commits are the owner's; the assistant stops at a clean working tree with a suggested subject.

---

### Where these specs depart from the Vision

Collected here so that the R13 pass (§12) can apply them in one edit. Per-spec issues are in each spec's own "Vision issues found" section.

| Vision | Departure | Why |
|---|---|---|
| §49 writes `ResourceId { node, local }` | X7 writes `ResourceId { node, path }` | A resource is a composite tree (§8): a channel, a GPIO bank or a timekeeper is a sub-resource that must be addressable, and a flat `local` cannot name one. The node qualification §49 asks for is unchanged. Spec 03 fixes the path grammar |
| §14's fidelity value sets stop at `hardware_quirk` | spec 05 will add `real` | §14 says every Run records the vector, and a Hardware Run has no value to record. Open question 1 |
| §3's action log names `StartRepeat` and `Capture` as Session actions | spec 04 RS-13a makes them namespaced Vocabulary verbs | `repeat` is a Radio Model capability in audit §13 and a recorder is a Sink; a Kernel that enumerated them would need a new variant for the first peripheral or calibration verb (invariant 30) |
| §29's list of event kinds reads as a Kernel registry | spec 04 RS-27 keeps only the kinds the Kernel emits or owns the policy for — from §29's code block, `PROCESSOR_DEADLINE_MISS` and, under §35's name, `DEVICE_LOST` — and gives the rest to their Vocabulary | audit §13's Kernel line names the envelope, the counters, `EVENTS_DROPPED` and the Policy mechanism, not concrete kinds |
| §50's envelope lists `random seeds` and an environment capture | spec 04 RS-43 keeps both out of it | the Kernel owns no generator, the environment is already recorded verbatim, and the capture has no Kernel-defined content |

## 4. Cross-cutting decisions

These bind all five specs. Each row is open to reversal at a gate; a reversal is recorded in §11.

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| X1 | Crate layout | One Cargo workspace at the repository root, one library crate `ezsdr-kernel`, boundaries expressed as a module tree (§5) | Mirror Vision §60's five core crates now (five crates for zero consumers, and §60 says "do not create crates merely to mirror this diagram"); one crate per Vision part (the module tree already gives those boundaries) | Split triggers in §5. A Vocabulary crate depending on the whole Kernel crate is **not** a trigger |
| X2 | Schema technology | Rust types are the authoring source; `serde` + `schemars`, pinned in `Cargo.lock` rather than by an exact version requirement (OV-11), generate JSON Schema 2020-12; generated files are committed under `schemas/`; a freeze test makes the committed file the arbiter (§6) | Hand-written JSON Schema with `typify`-generated Rust (heavy dependency tree, lossy for enum representations, two sources of truth the moment a serde attribute is needed); Protobuf (a toolchain and a second IDL for documents humans and agents author); FlatBuffers (solves a real-time parsing problem that invariant 10 says does not exist) | CBOR can be added later as a second *encoding* of the same schemas without touching them |
| X3 | 64-bit integers in documents | JSON `integer` with `format: int64`, written as an exact decimal by the canonicaliser (X5's integer profile); consumers are required to use a 64-bit-safe parser (Rust `i64` exact, Python `int` arbitrary precision) | String-encoding `ticks` (pollutes every TimePoint in every document). The reason to reject it is **not** that no JavaScript consumer exists — it is that the integer profile of X5 already makes the exact value survive canonicalisation | If a browser consumer appears it uses a BigInt-aware parser, or a v2 schema changes the encoding |
| X4 | YAML | The Kernel does not parse YAML. Human-authored YAML BindingProfiles are converted to JSON by the frontend (Phase 6) | `serde_yaml` (deprecated and archived); its forks (maintenance unverified); either way YAML 1.1 type coercion (`no` → false) is the wrong failure mode for a document that gates RF transmission | A Rust CLI that wants YAML adds the dependency in its own crate |
| X5 | Content hashing | `sha256:<64 lowercase hex>` over canonical JSON: **RFC 8785 (JCS) with an integer profile** — every value that came from an integer type is written as an exact decimal instead of through JCS's ECMAScript double rule (§7) | Strict JCS (its number rule routes every number through an IEEE-754 double, so two Manifests whose epoch-to-UTC offsets differ by 1 ns in 1.7 × 10^18 would share a hash); `serde_jcs` / `json-canon` (unverified, and now precluded by the deviation); BLAKE3 (speed is irrelevant for kilobyte documents); hashing the raw bytes (whitespace and key order would give two identities to one intent) | The deviation is why a stock JCS crate **cannot** be dropped in later; it is recorded as a named deviation in `SCHEMA_CHANGELOG.md` and pinned by the test vectors in §7 |
| X6 | Dependencies | Four: `serde`, `serde_json` (never with `preserve_order`), `schemars`, `sha2`. Hand-written: `Rational`, `BlockFlags`, `Version` + caret matching, error `Display`, `RunId` (§8) | `thiserror`, `bitflags`, `semver`, `num-rational`, `uuid`, `proptest` — each replaces under ~50 lines of std code | §8 states the rule for adding one |
| X7 | Identifiers | Node-qualified, `node = LOCAL = 0` in v4.0 (Vision §49): `ClockDomainId`, `MemoryDomainId`, `IslandId`, `DataLinkId` are `{node, local}`; `ResourceId` is `{node, path}` | A flat `u64` or a bare string (Vision §49 requires the qualification now so multi-host does not need a new public model) | v4.0 refuses any id whose `node ≠ LOCAL` |
| X8 | Document versus in-process types | Anything that appears in a Manifest or crosses a frontend or Plugin boundary is a **document**: it has a schema, a version where the Vision requires one, and canonical hashing. Anything carrying samples or a live handle (`SampleBlock`, `BufferRef`, link handles, trait objects) is **in-process only** and has no schema | Schemas for everything (a `SampleBlock` schema invites someone to serialise the real-time path) | §6 lists both sets |
| X9 | Floating point | The only floats in Kernel-owned document fields are `ClockRelation.drift` and `drift_uncertainty` (measurements; neither reaches a stored `TimePoint` unrounded). Floats also appear in `DataContract.attributes` (for example `full_scale`) and in Vocabulary values inside `requires` and `environment`, so canonical float formatting is required anyway | Integer parts-per-billion for drift (does not remove the need for canonical float formatting, so it saves nothing) | X5's number rule covers both |
| X10 | Test style | Plain `#[test]`, one integration-test file per spec, test names prefixed with the rule ID they prove (`tm_04_sibling_exact`). Doubles live in `tests/support/` and never in `src/` (§9) | `proptest` in Phase 1 (boundary tables cover the arithmetic; add it only if a time bug escapes them) | §9 lists the doubles |
| X11 | Kernel-does-not-grow check | A `kernel_surface` test that parses every `src/**.rs` with `syn` (a dev-dependency under OV-18: zero new transitive crates, MSRV 1.71): every public item is on an allow-list naming either a specific audit §13 token or a `NEW:` justification, every public item's doc comment cites a rule ID, and no token from `tests/banned_tokens.txt` appears in `src/` outside a comment citing the ban (OV-23, OV-23a, OV-23b) | `cargo public-api` (nightly rustdoc JSON: an unstable format on a toolchain X12 does not govern, whose failure mode is a gate that will not run — invisible with no CI); a hand-written lexer (eleven demonstrated evasions over five review passes, two of them hiding public items in the shipped crate — a finite spelling list against the language's whole spelling space). Parsing closes the *lexing* class by construction and no more: a later pass demonstrated five **predicate** failures in the parser-based gate, one of them open in the shipped crate, so OV-23's own refusals carry negative tests | The allow-list *is* the review checklist for "Core remains small"; the gate's failure mode must be a red test on the MSRV toolchain, never a silent pass |
| X12 | Toolchain | Edition 2024, `rust-version = "1.85"`. Phase 1 verifies with local `cargo test`; setting up CI is separate work | Edition 2021 (resolver 3 and MSRV-aware resolution come with 2024); building CI inside Phase 1 (scope creep) | MSRV moves only at a Kernel minor and stays ≥ 6 months behind stable |

---

## 5. Crate layout

```text
Ez-SDRv4/
├── Cargo.toml                     [workspace] members = ["crates/ezsdr-kernel"]
│                                  [workspace.package] edition = "2024", rust-version = "1.85"
├── Cargo.lock                     committed
├── crates/ezsdr-kernel/
│   ├── Cargo.toml                 version 4.0.0-alpha.1 until the v4.0 freeze
│   ├── src/lib.rs                 re-exports and the crate-level rule index
│   ├── src/id.rs                  NodeId, ResourceId, RunId, ModuleId, the *Id newtypes (X7)
│   ├── src/time/mod.rs            → time/{rational,domain,point,relation,authority}.rs      (01)
│   ├── src/stream/mod.rs          → stream/{block,buffer,link,burst,continuity}.rs          (02)
│   ├── src/contract.rs            DataContract registry, Port                               (02)
│   ├── src/hash.rs                canonical JSON + SHA-256                                  (§7)
│   ├── src/module_api.rs          the five role traits, descriptors, registry, Version       (05)
│   ├── src/spec.rs                ExperimentSpec envelope, Constraint, Value                 (03)
│   ├── src/binding.rs             BindingProfile, matcher, AdmissionCheck registry           (03)
│   ├── src/plan.rs                ExecutionPlan, fragments, dependency DAG, admission        (03)
│   ├── src/event.rs               Event, EventRecord, counters, EVENTS_DROPPED               (04)
│   ├── src/policy.rs              EventKind registry, Policy table                           (04)
│   ├── src/run.rs                 Run state machine, cleanup, Lease                          (04)
│   ├── src/session.rs             SessionAction, action log, admission                        (04)
│   ├── src/manifest.rs            Manifest envelope, ArtifactRef                             (04)
│   ├── src/schema.rs              schema generation entry points                             (§6)
│   └── tests/
│       ├── support/mod.rs         test-double Provider, in-memory DataLink, recording
│       │                          TestExecutor, ManualTimeAuthority wiring, fake HostClock
│       ├── support/doubles.rs     the eleven doubles OV-20 counts
│       ├── time_model.rs  stream_contract.rs  spec_binding.rs  run_session.rs  module_api.rs
│       └── schema_freeze.rs  kernel_surface.rs  hashing.rs  event_hotpath.rs
├── schemas/                       generated JSON Schema, one file per document per major version
│   └── SCHEMA_CHANGELOG.md
└── plan/phase1/                   00–05 (01–05 move to design/ on acceptance)
```

`hash` is its own module because `spec`, `binding` and `manifest` all need it and it must not depend on any of them. The directory is `crates/` rather than §60's `core/` because `core` collides with the Rust `core` crate in conversation and in grep; §60 is directional by its own words.

- **OV-8** Phase 1 creates exactly one crate. The split into Vision §60's five core crates happens when, and only when, one of these appears: (a) a consumer that must not link lifecycle code — the first WASM guest bindings or the first out-of-process Plugin SDK, Phase 9–11 — at which point `ezsdr-types` splits off as `no_std + alloc`; (b) two Kernel modules need incompatible dependency sets; (c) build time or artefact size is a measured problem. A Vocabulary crate depending on `ezsdr-kernel = "^4"` is the coupling Vision §5 prescribes, not a split trigger.
- **OV-9** The Kernel crate is `std`. It uses `String`, `BTreeMap`, `Arc` for block reference counting and `std::time::Instant` for the Lease host clock, and `schemars` derive needs `std`. WASM components consume the *schemas*, not this crate (Vision §42), so no bare-metal target exists for it. OV-8(a) is the upgrade path.

---

## 6. Schema strategy

- **OV-10** Every document type has a JSON Schema generated from its Rust definition and committed under `schemas/<name>.v<major>.json`. The committed file is the contract: non-Rust consumers read it, and `schema_freeze` fails the build when regeneration does not reproduce it byte for byte.
- **OV-11** `schemars` is pinned in the committed `Cargo.lock`, not by an exact `=` version requirement, so regeneration is reproducible inside this workspace without blocking downstream crates. Upgrading it is a deliberate change whose schema diff is reviewed and whose `schema_freeze` failure is the gate.
- **OV-12** After the v4.0 freeze a committed `*.v1.json` is immutable. A change creates `*.v2.json` plus a migration or a refusal (Vision §10). Any schema diff requires an entry in `schemas/SCHEMA_CHANGELOG.md`.
- **OV-13** Serialisation conventions, so that non-Rust consumers get discriminators: data-carrying enums use an internal tag (`{"kind": "...", ...}`), unit-only enums serialise as `snake_case` strings, and namespaced opaque sections are `serde_json::Value` with `additionalProperties: true`. Two enums are carved out and carry no tag, because their values *are* plain JSON: `Value` (SB-4) and `Scalar` (SC-2). A tag on either would make every Spec parameter and every contract attribute a two-field object instead of the scalar, list or map an author wrote. An integer is told from a float by the JSON number's own form, under OV-15.

Is generating the schema still "schema-first" in the Vision's sense? Vision §10's rule is that nothing is *defined only as a Rust type and re-described by hand* elsewhere. Nothing here is re-described by hand: the schema is generated, committed, reviewed and is the artefact every other language reads. The governance in OV-10 to OV-12 is what makes the committed schema, and not the Rust source, the arbiter.

**Documents** (schema, and a `version` field where the Vision requires one): `ExperimentSpec`, `BindingProfile`, `ExecutionPlan` summary, `PrepareReport`, `CoerceReport`, `Manifest`, `Event`, `Action` (including `TxBurst`), action-log entry, `Lease`, `Policy` table, `ExecutionClass`, `Fidelity`, `ModuleDescriptor`, `VocabularyDescriptor`, `ProviderInstance` and `Resource`, `ExecutorDescriptor`, `SinkDescriptor`, `LinkDescriptor`, `AuthorityDescriptor`, `ComponentDescriptor`, `IslandDecl`, the `Fragment` type, `ModuleError`, `StopReason`, `ArtifactRef`, `ContentHash`, `DataContract`, `DataLinkDecl`, `ContinuityMap`, `Gap`, `BurstRecord`, `SampleClockRecord`, and `ClockDomain` / `TimePoint` / `Duration` / `ClockRelation` / the deadline types as shared definitions.

**In-process only, no schema by design**: `SampleBlock`, `BufferRef`, memory pools and their handles (the `MemoryDomainId` *is* a document type), link handles, `EventSink`, `ActionReceiver`, every trait object, `TimeAuthority`.

---

## 7. Content hashing

- **OV-14** A content hash is the string `sha256:` followed by 64 lowercase hex digits. The hash algorithm is named in the value so a later algorithm is an additive change.
- **OV-15** A document is hashed over its canonical form: UTF-8 JSON text, object members sorted by key, no insignificant whitespace, RFC 8785 string escaping, and the **integer profile** — a value serialised from an integer type is written as an exact decimal with no exponent, whatever its magnitude, while a value serialised from a floating-point type takes the ECMAScript `Number::toString` form of RFC 8785. Non-finite numbers are rejected rather than written as `null`.
- **OV-15a** The integer profile is a deliberate, documented deviation from RFC 8785, whose number rule would route `1 700 000 000 000 000 000` through an IEEE-754 double and lose the last bits. It is recorded as a named deviation in `SCHEMA_CHANGELOG.md`, and no implementation that lacks it may be substituted. One consequence is accepted by design: an integer-typed `20` and a float-typed `20.0` share one canonical form and therefore one hash, because they are one value under SB-6's scalar comparison; RS-45's "comparable by construction" is preserved by that, not broken. That coincidence holds while |v| ≤ 2^53 and **not above it**: there a float's canonical text is the shortest decimal that names the `f64` rather than the number's exact decimal, so an `int` and a `num` spelling of one number may hash as two documents, and one canonical text may name two numbers. The hash is therefore **document** identity (RS-45) and SB-6 is **value** identity; neither is the other's definition (finding D50).
- **OV-16** Equal inputs produce equal hashes across processes, platforms and map insertion orders. Two documents that differ only in key order or whitespace have one hash.

Implementation notes for `hash.rs` (about 80 lines). `serde_json::to_string` already emits compact output with RFC 8785-conformant string escaping, and its `Map` is a `BTreeMap` unless the `preserve_order` feature is on. Two things are therefore not free: key order (any crate in the workspace enabling `preserve_order` would switch `Map` to insertion order, so the canonicaliser sorts keys itself) and number formatting (`serde_json` uses shortest round-trip via ryu, for example `2400000000.0`, where RFC 8785 requires `2400000000`). The number rule is: integers via plain decimal; other finite numbers laid out per ECMAScript — plain decimal for `1e-7 ≤ |x| < 1e21`, exponential otherwise, `-0` written as `0`. Keys are sorted by UTF-16 code unit as RFC 8785 requires; all Ez-SDR keys are ASCII by the key-namespace rule, where byte order and UTF-16 order coincide, and the canonicaliser asserts that.

Tests use the RFC 8785 number vectors for the floating-point rule (`1e21 → "1e+21"`, `1e-7 → "1e-7"`, `0.000001 → "0.000001"`, `333333333.33333329 → "333333333.3333333"`, `5e-324`, `1.7976931348623157e+308`) and, for the integer profile, the values `9007199254740992` (2^53), `9007199254740993` (2^53 + 1), `1700000000000000000` (a realistic epoch-to-UTC offset in nanoseconds) and `-9223372036854775808`, each of which must round-trip as its exact decimal and must hash differently from its neighbour. SHA-256 known answers cover the digest itself.

- **OV-17** What is hashed: the ExperimentSpec body after migration; the BindingProfile body including its `environment`; each artifact's bytes; each component's Module-declared implementation hash; each Manifest section individually. The Manifest's own hash is computed over the Manifest without that field and stored beside it, never inside the hashed body.

---

## 8. Dependency policy

| Crate | Version | MSRV | Purpose | Verdict |
|---|---|---|---|---|
| `serde` (derive) | 1.0.229 | 1.56 | document types | use |
| `serde_json` | 1.0.151 | 1.71 | JSON I/O, `Value` for opaque sections | use; never enable `preserve_order` |
| `schemars` (derive) | `1.2.2` | 1.74 | schema generation | use; pinned in `Cargo.lock`, **not** with an exact `=` requirement, which in a published library cannot unify with a downstream `^1.3` and would block the first Vocabulary crate that needs a schemars fix |
| `sha2` | 0.11.0 | 1.85 | SHA-256 | use; never hand-write crypto |
| `syn` (**dev**) | 3.0.6 | 1.71 | the parser `kernel_surface` reads `src/` with (OV-23, X11) | use; OV-18 applied 2026-09-22 — the hand-written alternative was 709 lines and was walked past eleven times over five review passes (the parser-based gate was then walked past five more times, by predicate bugs rather than lexing ones), and it adds **zero** new transitive crates, since `syn`, `proc-macro2`, `quote` and `unicode-ident` are already resolved through `serde_derive` and `schemars_derive` (verified against `Cargo.lock`). A dev-dependency, so X6's four and exit criterion 6 are untouched |
| `thiserror` | — | — | error `Display` | no: about five error enums, hand-written `Display` is ~50 lines |
| `bitflags` | — | — | block flags | no: seven flags, a `BlockFlags(u16)` newtype is ~25 lines |
| `semver` | — | — | Module versions | no: `Version` plus caret matching is ~30 lines. Pre-release tags are out of scope because **Module and Vocabulary versions in a `ModuleDescriptor` are release-only**; the crate's own `4.0.0-alpha.N` is a Cargo version, not a descriptor version, and an alpha crate declares `4.0.0` in its descriptor |
| `num-rational` | — | — | tick rates | no: gcd-normalised `Rational` with 128-bit intermediates is ~40 lines, and it must return errors where `Ratio` panics |
| `uuid` / `rand` | — | — | `RunId` | no: `RunId` is the opaque string `<node>:<pid-hex>:<unix-nanos-hex>-<counter>`. The process id is required: `node` is fixed at `LOCAL` for all of v4.0 and the counter is per-process, so without it two `ezsdr` processes on one host produce identical ids within one nanosecond. Ceiling: unique per host, not across hosts until `NodeId` is real |
| `proptest` | — | — | property tests | no in Phase 1 (X10) |
| any YAML crate | — | — | — | no in the Kernel (X4) |

Versions are the ones verified present in the local registry; MSRV values are each crate's own declaration.

- **OV-18** Adding a dependency to the Kernel crate requires all of: the hand-written alternative exceeds about 150 lines or is a standards or cryptography implementation; the crate's MSRV is at or below ours; it adds at most three new non-optional transitive crates; a row is added to the table above with its verification status; `Cargo.lock` is updated in the same change. Dev-dependencies follow the same rule.

---

## 9. Test strategy

- **OV-19** Tests are plain `#[test]`. Integration tests live in `crates/ezsdr-kernel/tests/`, one file per spec, and each test's name begins with the lowercased rule ID it proves. Inline `#[cfg(test)]` modules are for private helpers only (gcd, canonical number layout).
- **OV-20** Test doubles live in `tests/support/` and are never compiled into `src/`. Phase 1's include a test-double Provider, an in-memory DataLink implementing all three policies, a recording TestExecutor of about twenty lines, and a fake `HostClock`; the set is not closed, and a double is added when a rule has no other way to be proved. The `ManualTimeAuthority` is the exception: spec 01 makes it normative (TM-17a) and Phase 2's Simulation Engine crate will build on it, which a sibling crate's `tests/support/` cannot provide. It therefore ships in `src/` behind a non-default `testing` feature, and OV-23's frozen-surface check excludes feature-gated items through one explicit allow-list line that says so.
- **OV-21** The test-double Provider uses no radio vocabulary. Its keys are `test.count`, `test.grid` and `test.flag`. If a matcher test needs the word "channel" or "rate", the matcher is not generic and the test has found a defect (audit F7).
- **OV-22** `schema_freeze` regenerates every document schema with pinned settings and compares it byte for byte with `schemas/`. `EZSDR_UPDATE_SCHEMAS=1` rewrites the files from inside the test; there is no build script and no `xtask`.
- **OV-23** `kernel_surface` **parses** every file under `src/` with `syn::parse_file` and fails when: a public item is missing from the allow-list; an allow-list entry names neither a **specific token** from audit §13's Kernel tree nor a `NEW: <one-line justification>`; a public item's doc comment cites no rule ID; or a banned token appears outside a comment that cites the ban. Items behind the `testing` feature are excluded by one allow-list line that names the feature. A public item is an item of the crate's module tree — inline `mod` bodies included — whose visibility is `pub`; `pub(crate)`, `pub(super)` and `pub(in …)` are not. A `pub use` fails when any branch of its tree — a brace group has one root per branch — begins with a segment other than `crate`, `self`, `super` or a module of this crate, so a dependency's type cannot reach the surface unlisted; a `pub use` carrying a **glob** fails outright, because the scan cannot enumerate what it exports and the allow-list therefore cannot hold it. A doc comment citing a **withdrawn** rule number does not satisfy the citation check, since a withdrawn rule states no obligation (OV-1). The `testing` exclusion is an **exact** match on `feature = "testing"`: a substring test for the word also skipped `#[cfg(not(feature = "testing"))]`, which is the default build.

  Parsing replaces lexing because five review passes demonstrated **eleven** ways past a line-based scan — an unbalanced brace in a comment, a multi-line string, `r"…"`, `br#"…"#`, `union`, a name on the next line, a foreign `pub use`, an attribute before the item, `pub async`, `pub extern` split across lines, and a bare `pub` — and every one was a lexing failure, a spelling the keyword and modifier lists did not cover. Two of them were hiding public items in the shipped crate: four node-qualified id types generated by a macro, and `document_schemas`. A parser closes that class by construction rather than by enumeration, and a file it cannot parse fails the test, so this gate's failure mode is a **red test on the MSRV toolchain** and never a silent pass.

  What no source parser sees is code produced at **expansion** or **inclusion**, and that class is closed by refusing its mechanisms — a closed list, unlike the language's spellings: a `macro_rules!` body carrying the token `pub`, an `extern` block, a `mod` carrying `#[path]` or `#[cfg_attr]`, `include!` (which splices items from a file `rust_sources` never opens, since it collects `.rs` only) and `pub extern crate` (a whole dependency on the surface) are each refused outright. Proc-macros are confined to `serde_derive` and `schemars_derive` by X6 and generate no module-level items of this crate.

  Parsing is not by itself the gate. A cross-family pass demonstrated **five** more evasions against the parser, and they were not lexing failures but **predicate** failures: the `testing` exclusion matched a substring, a brace-group `pub use` named no root, `pub extern crate` fell through the item match, `include!` was unread, and the allow-list key encoded an inline module's depth rather than its name, so two same-named items collapsed onto one entry. One was open in the shipped crate. The three predicates that were wrong carry direct negative tests of their own, because the five were otherwise verified only by transient injection.
- **OV-23a** The ban is a fixed literal token list kept in `tests/banned_tokens.txt`, not "any Vision §6 term": `uhd`, `soapy`, `hackrf`, `rfnoc`, `replay`, `cuda`, `wasmtime`, `dev/net/tun`, `tuntap`, `af_xdp`, `dpdk`, `802.11`, `otfs`, `ibfd`, `taint`, `prometheus`. Vision §6 also bans "Processor execution ABIs", but `Processor` is a Kernel role in audit §13's module-api line, so a literal scan for §6's phrases would fail the crate on its first run and the check would be disabled within a week.
- **OV-23b** An allow-list entry marked `NEW:` is a type the Kernel has that audit §13 does not name. The exit review reports the `NEW:` count and each justification; that number, not the raw public-item count, is the "Core remains small" measurement. Audit §13's `data` line ends with "Stream Contract (normative)", which would otherwise absorb every type spec 02 invents.

**Phase 1 test inventory.** Each spec's own test table is the authority, and this document does not restate it. A second copy is what went stale inside one review round: it kept a refusal SC-23a had reversed and it missed `DropCarry`, `ChannelGap`, SC-10a and SC-20a. The exit review reads the per-spec tables, plus this document's own four: `schema_freeze`, `kernel_surface`, the canonical-JSON vectors of §7 including the integer-profile values, and the SHA-256 known answers.

## 10. Traceability

### Audit §14.1 items → spec, code, tests

| # | Item | Spec | Code | Tests |
|---|---|---|---|---|
| 1 | Three tiers and the freeze unit | 00 §5, 05 MA-1…4 | one crate, the allow-list | `kernel_surface` |
| 2 | TimePoint representation, epoch, TimeAuthority, step-driven simulation | 01 TM-1…TM-21, 05 MA-15/20/29/30 | `time`, `module_api::Authority` | `time_model`, `module_api` stepping (the Engine itself is Phase 2) |
| 3 | Stream Contract | 02 SC-1…SC-32 | `stream`, `contract` | `stream_contract` |
| 4 | TimingEnvelope, coercion, late policy, fidelity vector | 03 SB-44…SB-46, 02 SC-27, 05 MA-11/12/42 | `binding`, `plan`, `module_api` | `spec_binding`, `module_api` (envelope *contents* are Phase 2) |
| 5 | Session and Lease | 04 RS-12…RS-25 | `session`, `run` | `run_session` |
| 6 | Composite resource tree, provider-declared coherence, arm-order DAG | 03 SB-33…SB-36, SB-39, 05 MA-10/16 | `binding`, `plan` | `spec_binding` (the coherence basis is Vocabulary, Phase 2) |
| 7 | BindingProfile = bindings + placements + environment | 03 SB-21…SB-28, 05 MA-38/41 | `binding` | `spec_binding`, `module_api` |
| 8 | Generic matcher and PrepareReport | 03 SB-5…SB-8, SB-41, 05 MA-11/12/34 | `binding` | `spec_binding`, `module_api` |
| 9 | DataContract registry and Port | 02 SC-1…5, 05 MA-36 | `contract` | `stream_contract` |
| 10 | Schema-first and versioning | 00 §6, 03 SB-47…SB-49 | `schema`, all documents | `schema_freeze`, `spec_binding` |
| 11 | Descriptor versus ABI, cycle rule, two deadline kinds | 05 MA-19…22/36/37, 01 TM-15 | `module_api`, `time` | `module_api` |
| 12 | Event counters, EVENTS_DROPPED, closed Policy | 04 RS-26…RS-37, RS-48…RS-52 | `event`, `policy` | `run_session` |
| 13 | Manifest envelope, namespaced sections, hashes | 04 RS-38…RS-47, 00 §7 | `manifest`, `hash` | `run_session`, hash vectors |
| — | *Not an audit item: the Kernel Action set.* Audit §13 pairs it with events on one line and §14.1 gives it no row, which is how it reached Gate B with no owner | 04 RS-48…RS-52 | `event` | `run_session` |
| 14 | RFNoC repositioned | 05 MA-21/46 | — (no executor kind for it) | `kernel_surface` term ban |
| 15 | The three axes | 05 MA-1/2 | `module_api` | `module_api` |

### Vision §58 acceptance tests → phase

| # | Test | Phase 1 status |
|---|---|---|
| 1 | Binding substitution works | partial: two test-double instances bound to one Spec; complete with MockRadio (2) |
| 2 | Virtual time runs faster than wall clock | Phase 2 |
| 3 | Deterministic runs reproduce with a seed | Phase 2–3; Phase 1 fixes the stepping-order rule |
| 4 | Events flow through the hardware path | partial: counters, bounded queue, EVENTS_DROPPED with the double |
| 5 | Fault injection triggers cleanup and policy | partial: the policy table and transactional cleanup, driven by an injected lifecycle failure; injection is Phase 4 |
| 6 | Continuity can represent gaps identically to a UHD overflow | partial in Phase 1 (flags and ContinuityMap over hand-made blocks); the acceptance test itself is **Phase 2**, against the documented UHD behaviour — zero samples, a 50 ms restart, `out_of_sequence` — because §58's preamble puts it before UHD exists. Phase 8 re-measures the profile and may tighten it |
| 7 | Runs record Spec, Binding, plan, events, artifacts | Phase 1 |
| 8 | Two MockRadios through a SimulationChannel | Phase 3 |
| 9 | Reactive dynamic TxBurst | Phase 5 |
| 10 | No Mock-specific APIs in application logic | Phase 2+ |
| 11 | Mock enforces the envelope | partial: the coercion → PrepareReport → policy path; envelope values are Phase 2 |
| 12 | Block-size independence | Phase 2/10 |
| 13 | Sessions leave provenance | partial: action log and effective configuration; waveform hash and Python are Phase 6 |
| 14 | Environment portability | Phase 1 |
| 15 | TX bursts are closed and contiguous | partial: the burst state machine as a pure function; Mock and hardware are Phase 2/8 |
| 16 | Session Actions are admitted | partial: the admission path with a generic admission check; radio keys are Phase 2 |

---

## 11. Decision log

Filled in at the gates. One row per decision the owner confirmed or reversed, so that Phase 2 does not reopen them.

| Decision | Gate | Verdict | Note |
|---|---|---|---|
| Location: draft in `plan/phase1/`, move to `design/` on acceptance | pre-A | confirmed | 2026-09-21 |
| Language: English | pre-A | confirmed | 2026-09-21 |
| Three gates: 01+02, 03+04, 05+00 | pre-A | confirmed | 2026-09-21 |
| Phase 1 includes the crate, not only the specs | pre-A | confirmed | 2026-09-21 |
| X1–X12 | A/B/C | confirmed (12/12), then **X11 reversed in part** | Second-opinion pass, 2026-09-22. No exceptions at the time. **X11 was later reversed in part**: the second-opinion reviewer, asked after five review passes had demonstrated eleven ways past the hand-written scanner, reversed the half of X11 that rejected a `syn`-based scanner and confirmed the half that rejected `cargo public-api`. The deciding reason is the failure mode inside the loop this project runs: the hand lexer fails **silently**, `cargo public-api` fails loudly but **outside** `cargo test` on a nightly toolchain X12 does not govern, and `syn` fails loudly **inside** `cargo test` on the MSRV toolchain. X11 had recorded the `syn` alternative and rejected it as a dependency without applying OV-18, the rule written for that question; applied, `syn` passes every clause — the hand-written alternative is 709 lines, `syn 3.0.6`'s MSRV is 1.71, and it adds **zero** new transitive crates, since `syn`, `proc-macro2`, `quote` and `unicode-ident` are already resolved through `serde_derive` and `schemars_derive`. X6 and exit criterion 6 count `[dependencies]`, which stay at four. X5 carries one Phase 6 note, not an amendment: a Python consumer needs a reference canonicaliser, because `repr(1e-7)` is `1e-07` where RFC 8785 wants `1e-7` |
| Per-spec decision tables | A/B/C | confirmed, with three amendments | Second-opinion pass, 2026-09-22. 01 T1–T13, 02 S2–S19, 03 B1–B3 and B5–B9, 04 R1–R9 and 05 M1–M12 confirmed as written. Amended: **02 S1**, whose promised widening is a Kernel major until the mask newtypes' inner fields stop being public; **03 B4**, whose ceiling holds only once RS-49 carries `UpdateParameter.at` (open question 2); **04 R10**, whose envelope gains the mandatory `version` Vision §10 requires. The three amended cells are rewritten in their own spec tables; those tables carry no Verdict column, because they become normative text at Step 5 |

Open questions the owner must settle at a gate:

| # | Question | Recommendation | Verdict |
|---|---|---|---|
| 1 | The fidelity vector has no value for a Hardware Run; Vision §14's sets stop at `hardware_quirk`. | Add `real` to every aspect's value set. Applied in spec 05 as MA-42; the alternative, omitting the vector on Hardware Runs, contradicts §14's "every Run". Confirm or reverse. | confirmed — `real` stays. The §14 value-set edit is a Step 5 Vision item (05 §9 #2) |
| 2 | The Kernel Action set (§5, §19) has no RX stream command, so a fixed Spec cannot say "start RX at time T" as an Action. | Express it as `Provider::start(at)` plus a recorder parameter, and do not add a Kernel Action. Spec 04's RS-48 fixes the set at seven members on that reading. Confirm, or accept an eighth member. | amended — no eighth Action, but RS-49's `UpdateParameter` gains an optional `at: AbsoluteDeadline` ("the instant at or after which the update takes effect under its class; absent means the first instant the class permits"), RS-49a resolves it, RS-14/RS-19 route a capture's `at` into it, and `compile` stops dropping it. `at` is optional for every class, including `hardware_timed`, or a bare `sdr.rx.frequency = x` would be refused (§3). Without the field `hardware_timed` is unimplementable and §61's `capture(n, at:)` loses its time |
| 3 | Vision §32 says the Simulation Engine step-drives every Island; the plan puts the stepping loop in the Kernel coordinator with the Authority deciding *when*. | Confirm that reading. Applied in spec 05 as MA-30: the Engine decides the instants through `next_wakeup`, and the Kernel calls `step` in a fixed role-and-id order, so determinism does not depend on the order a runtime assembled its Modules. | confirmed — the coordinator issues `step(until)` and the Engine's `next_wakeup` decides when; an Engine-owned loop would hold other Modules' instances (MA-16). The §32/§15 wording is a Step 5 Vision item (05 §9 #4) |
| 4 | A recording TestExecutor is a fifth double beyond "test-double Provider and in-memory link". | Allow it: twenty lines, and without it the stepping loop has no test until Phase 2. | not a decision — allowed, and OV-20 drops its count: ten doubles live in `tests/support/` |
| 5 | `schemars` pinned in `Cargo.lock`, not by an exact `=` requirement. | Accept: an exact requirement in a published library cannot unify with a downstream `^1.3`, and the lock plus `schema_freeze` already give reproducibility. Upgrades are deliberate changes that regenerate the schemas. | confirmed |
| 6 | MSRV 1.85 and crate version `4.0.0-alpha.N` until the freeze. | Accept. | confirmed |
| 7 | TM-16a1 settles that the Authority drives `host.monotonic` as virtual time in the Simulation class only, and reads the real clock in the other three. This borders on a Phase 2 question about the Simulation Engine. | Settle it now. It changes no type, but `RelativeBudget`'s meaning depends on it and TM-15 has already fixed that domain, so leaving it open would leave deadline semantics undefined in RealtimeEmulation — the one class that exists to expose real deadlines. | confirmed — settled now. Phase 2 note: in the Simulation class a budget miss fires only if the Engine models processing time, which it may, because it drives `host.monotonic` |


### Findings from Step 4 (the crate), for the owner's verdict

Raised rather than applied: none of these edits the Vision, the audit or the
re-review (OV-6), and none changes a spec's text. Each is a place where writing the
code met something the spec did not settle, or settled differently from what serde,
Rust or the measurement allows. `plan/phase1/` is unchanged apart from this table.

| # | Where | What the implementation had to do | Why | Verdict |
|---|---|---|---|---|
| D1 | 03 §4 `Constraint`, `CapabilityValue`; `OutputSource` | Struct variants (`Eq { value }`, `Set { values }`, `One { value }`, `AnyOf { values }`, `Port { port }`, `Resource { resource }`) instead of the newtype variants the shape table writes | OV-13 requires an internal tag for a data-carrying enum, and serde cannot internally tag a newtype variant whose payload is not a map. An encoding consequence, not a semantic one | not a decision — an encoding consequence of OV-13; struct variants stand |
| D2 | 04 §4 `RunError` | Added `IllegalTransition { from, to }` | The listed error set names no error for RS-2's own rule, so an attempt to skip or reverse a state had nothing to return | confirm — RS-2 states a rule with no error; `IllegalTransition` is that rule's own |
| D3 | 04 §4 versus RS-27, RS-28 | Registered **five** Kernel event kinds. **Spec corrected** | §4 says "the Kernel registers the four of RS-27" while RS-27 lists five and RS-28 says "the Kernel's five are". Followed RS-28; §4's count looks stale | confirm — the applied edit verified: RS-27/RS-28 list five, and of §29's twelve kinds the Kernel keeps two |
| D4 | 04 §4 `EventKind` | Grammar is dotted segments of `[A-Za-z][A-Za-z0-9_]*` | §4 says "a Namespace", whose SB-1 grammar is lowercase, but every kind the spec names is either `EVENTS_DROPPED` or `test.custom`. The implemented grammar admits both | confirm — both `EVENTS_DROPPED` and `ns.name` must parse; Vocabulary kinds carry their owner's prefix (RS-27), which the Radio Model spells out in Phase 2 |
| D5 | 01 TM-21 | Cross-multiplication is checked and may return `Overflow`. **Spec corrected** | The rule's "TM-3's caps bound them at 2^124" holds only when both nominal rates have terms ≤ 2^31; a `Derived` domain's nominal rate can reach 2^62/1 under those same caps. The rule's own "checked 128-bit intermediates" clause covers it, and `tm_21_duration_cmp_overflow_is_error` proves it. Prose only | confirm — the applied edit verified: a derived rate's terms reach 2^62 and the cross-product 2^186 under §6's tick bound, so the checked 128-bit arithmetic is the bound, not the caps |
| D6 | 02 §4 sketch, SC-27 | `LatePolicy::decide` takes a `&ClockRegistry` | SC-27 requires TM-21's cross-multiplication, which needs both domains' nominal rates. The §4 sketch is marked illustrative | not a decision — the §4 sketch is illustrative |
| D7 | 02 SC-27 | `LateOutcome.late_by` is reported in `host.monotonic` | SC-27 fixes no domain for it. Chosen as the domain `min_lead` is declared in; the lead is rescaled by TM-9 and floored, so `late_by` errs towards more lateness, never less | confirm — the floor errs toward more lateness, the safe direction (§59) |
| D8 | 02 SC-29a | A `BurstOpen` accompanies exactly the block carrying `START_OF_BURST` | SC-29a puts the three fields "on the block that opens a burst" and its test requires a refusal when absent. A burst opened by a **discontinuity** carries no `START_OF_BURST`, so it takes no `BurstOpen` and its `wraps` is 0 — a ceiling of the error path | amend — keep the rule; add `BurstTracker::set_late` so a discontinuity-opened burst can carry the late outcome SC-24a requires, and name it in SC-29a beside `set_actual_start` |
| D9 | 00 OV-23, OV-23a | The ban check reads "outside a comment citing the ban" literally, per line, with `OV-23a` as the exemption marker | Seven UHD evidence citations in doc comments carry the marker; four prose uses of `replay` and `taint` were reworded instead, because there the banned word was not the load-bearing one | confirm |
| D10 | 00 OV-23, OV-23b | Both the allow-list and the rule-ID citation check govern **module-level** public items; a method on a public type is checked by neither | OV-23b's "a type the Kernel has that audit §13 does not name" reads as a type-surface measurement, and a method is not a Kernel concept of its own. "Module level" is tracked by brace depth, so an item inside an inline `mod` — public or private — is scanned and an item inside an `impl` is not | confirm — a method is not a Kernel concept (OV-23b) |
| D11 | 04 RS-32 | `EventCollector::new` locks and releases its ring once at construction | On some platforms `Mutex` boxes its OS primitive at the first lock, so the first `emit` allocated exactly once and RS-32's counting-allocator test failed by one. The allocation now happens at `prepare`, where RS-33 already sizes the table | not a decision — a platform quirk moved to `prepare` |
| D12 | 00 OV-20 | `cargo test` enables the `testing` feature through a self dev-dependency | So that OV-20's `ManualTimeAuthority` is exercised with no flag while staying out of the default shipped surface | confirm |
| D13 | 00 X12, open question 6 | Eight edition-2024 let-chains rewritten as nested `if let` | Let-chains need 1.88; the declared MSRV is 1.85. The suite now passes on **1.85.0, 1.89.0 and stable (1.98.1)**, so X12 stands unchanged | confirm — X12 stands |
| D14 | 02 SB-7, OV-21 | The test double declares `test.grid` as `AnyOf`, not a continuous `Range` | SB-7 consults `coerce` only when the declared capability does not satisfy the constraint directly, and a continuous range cannot express "multiples of 20". A fixture note, not a rule change | not a decision — a fixture note |


### Findings from the Step 4 code review, for the owner's verdict

Two adversarial review passes over the crate (Opus, per AGENTS.md §8) found 3 P0
defects, 24 P1/P2 defects and a further set of places where a spec did not settle
something the code had to. The defects are fixed; the spec questions are raised here.
D3 and D5 were confirmed as spec errors by the second review and **applied** with
the owner's standing authorisation:

- `01-time-model.md` TM-21: "TM-3's caps bound them at 2^124" replaced. A `Derived`
  domain's nominal rate is its root's rate divided by `root_ticks_per_tick`, so each
  term is a product of two capped terms and reaches 2^62 and the cross-product
  reaches 2^186 — which is what the neighbouring "193 bits" paragraph already said
  about TM-4. The checked 128-bit arithmetic is the bound, not the caps.
- `04-run-and-session.md` §4: "the four of RS-27" → "the five of RS-27", and the
  `EventKind` grammar corrected in the same line (D4). The same stale count appeared
  in 04 §9 item 9 and in this file's departure table, where it also mis-stated Vision
  §29: of §29's twelve kinds, RS-27 keeps **two** — `PROCESSOR_DEADLINE_MISS`, and
  §29's `DEVICE_DISCONNECTED` under §35's name `DEVICE_LOST`. Both corrected.

Everything else below is raised, not applied.

| # | Where | What the implementation had to do | Why | Verdict |
|---|---|---|---|---|
| D15 | 02 SC-30c versus §6's `finish` pseudocode | A trailing carry's `Gap` has `len: 0`, whatever `lost` says | SC-30c says "a **zero-extent** `Gap` … the map's `end` does not move, because no sample after the last delivered one is accounted for", while §6 writes `carry.lost or 0` for that length. The two conflict when `lost` is present: the pseudocode's version claims `lost` samples beyond the map's own end, contradicting the rule's own justification. The rule carries the ID, so the code follows the rule. §6's line should change | amend — the rule is right; §6's `finish` writes `0`, not `carry.lost or 0`, and SC-30c gains one clause: `len` is 0 because what followed the last delivered block is unknown, not because nothing was lost |
| D16 | 01 TM-13b's parenthetical | Nothing refuses a block naming an unregistered domain | TM-13b says "*the Kernel checks only that the domain is registered (TM-12)*", but `SampleBlock::new`, `BurstTracker::on_block` and `ContinuityBuilder::push` all take no registry, deliberately. TM-12's block clause is a producer obligation; the parenthetical overstates it | amend — TM-13b's parenthetical becomes the registry-as-seam statement (an unregistered domain has no rate and no conversion; the block constructor takes no registry by design), and the `tm_13b` test row no longer promises a refused block |
| D17 | 05 MA-25, 03 SB-25a, 05 §4 `ComponentDescriptor` | `plan::sink_components` is always empty on a Spec Run, so a Spec Run that declares an output is refused unless the runtime supplies the set directly through `CompileInputs::sink_components` | MA-25 resolves the Sink role "for a Session … from the placement's `module` field"; SB-25a says a Spec Run leaves that field unset; `ComponentKind` is only `Processor \| Reactor`. Nothing left says which component of a Spec's graph is a Sink, so SC-21's drop-class rule, SB-15's `Block`-into-Sink refusal and SB-17's capture check are unenforceable on the Spec path — the path a publication Run uses. The specs need one of: a `Sink` member on `ComponentKind`, a `role` on `ComponentDescriptor`, or `module` set on every placement | amend — as revised below under "Findings from the second-opinion pass": an output is **bound, not placed**, and none of this row's three candidates is taken. The sub-choice there is settled as `Binding.feed` |
| D18 | 05 MA-38 | `plan()` takes the Executor's Module id from the runtime and refuses when it is absent | MA-38 names an Executor **instance**; no document names the Module that supplies it. The first draft invented `ezsdr.test.executor`, which put a Module name in a frozen Kernel crate and would have made every Manifest record a test double | amend — as revised below: MA-38's `executor` names a binding whose Module holds the Executor role, and the runtime supplies the descriptor only |
| D19 | 03 SB-13, decision B3 | The depth scan skips a parameter's `schema` | A parameter's `schema` is opaque Vocabulary content (MA-36). Scanning it refused a component whose parameter object merely *describes* a property called `island`. B3 weighed the false negative (smuggling into `extensions`) and not this false positive | confirm — a false positive B3 never weighed |
| D20 | 04 RS-21 | `Lease::validate()` exists and is tested, and nothing calls it | RS-21's refusal lives in the constructor, which a Lease read back from a document or built field by field bypasses. Phase 1 has no Run-admission seam to call the predicate from, because the coordinator is Phase 2 | confirm — a forward obligation: the Phase 2 coordinator, and the JSON boundary when a Lease is read back |
| D21 | 04 RS-12 | A Session resource's `kind` is the bound instance's own root kind | RS-12 says "one resource per binding with empty `requires`" and fixes no `kind`. The first draft invented `ezsdr.session`, which the matcher (SB-34) then could not bind to, so every Session failed `validate`. Taking the kind from the bound instance keeps the Kernel from owning a vocabulary word | confirm — it keeps a Vocabulary word out of the Kernel |
| D22 | 03 SB-30 versus SB-38, SB-41 | The `prepare`-stage checks run inside `collect_prepare` | SB-30 says the Kernel runs the checks "at three points", but SB-38 names only `validate` and SB-41 names `prepare` without saying it runs them. Putting the call inside `collect_prepare` is what stops the second point being forgotten; the spec should name the call site | amend — SB-41 names the call site: `prepare` runs SB-30's second point over the merged effective configuration before returning, and a violation fails `prepare` (SB-42) |
| D23 | 00 OV-13 | `spec::Value` is `#[serde(untagged)]` | OV-13 says data-carrying enums use an internal tag, with no exception. A Spec's values are plain JSON scalars and cannot carry a tag. OV-13 should carve it out explicitly | amend — OV-13 carves out `Value` and `Scalar` as plain JSON, and `Scalar` becomes untagged too, so one document family has one scalar encoding |
| D24 | 00 OV-15 versus 04 RS-45 | `Int(20)` and `Num(20.0)` hash identically | ECMAScript `Number::toString(20.0)` is `"20"`, so RS-45's "two Runs with equal hashes are comparable by construction" conflates a `kind: int` and a `kind: num` value. This follows from OV-15 as written, not from a coding error; worth one sentence in OV-15a naming it as accepted | amend — accepted as a consequence, with one OV-15a sentence: `20` and `20.0` are one value under SB-6, so one canonical form and one hash |
| D25 | 05 MA-13's Rust sketch | `stop` takes a `StopMode { Orderly \| Abort }` | The sketch writes `stop(reason: StopCause)`, but RS-9 says an abort differs from an orderly stop "only in the **mode** passed to each Provider". `StopCause` survives on `Action::Abort` and in MA-46's frozen list. The sketch is what a Plugin author reads | amend — MA-13 and its sketch read `stop(mode: StopMode)`; `StopCause` stays on `Abort` |
| D26 | 05 MA-37 | Two of the four structural checks were removed | "update classes from the closed set" and "an `impl.hash` present" cannot fail in Rust: `UpdateClass` is a closed enum and `ContentHash` only exists parsed. Both now arrive as a deserialisation refusal at the JSON boundary, which is where a non-Rust producer sends them, and are tested there | amend — MA-37 names the two checks the schema enforces at the JSON boundary instead of listing them as Kernel checks |


### Findings from the verification pass

A third pass verified the fixes above: 20 of 28 clean, 5 partial, none regressed. The
partials and one new P0 were fixed in turn; these are what they left behind.

| # | Where | What the implementation had to do | Why | Verdict |
|---|---|---|---|---|
| D27 | 03 SB-34 | Two Spec resources prefer **different** nodes of one instance but may share one when that is all there is | SB-34 says two resources "**may** bind to different sub-resources" and never says a node may not be shared. Requiring exclusivity refused a satisfiable binding (two resources both wanting the device root); allowing it freely handed one physical channel to two resources with no diagnostic. The implementation prefers disjoint and shares only as a last resort. SB-34 should say which it means | **reverse** — no sharing as a fallback. `Resource.shareable` (Provider-declared, default false); an exclusive node bound twice is refused with `NodeAlreadyBound` naming both resources; `needs` resolutions consume nodes under the same rule. Which kinds are shareable is the Radio Model's declaration, never the Kernel's knowledge |
| D28 | 04 RS-25 | Nothing checks that a child Run has no Lease of its own | RS-25's first half — the parent's expiry ends its children first — is RS-6 step 0 and is tested. The second half has no Kernel seam in Phase 1, because there is no child-Run type until the coordinator exists (Phase 2) | confirm — a forward obligation, Phase 2 coordinator |
| D29 | 03 SB-17 | An output reaching a Sink through **one** link is accepted; a longer chain is refused | SB-17 says "a capture whose source has no placed Sink is refused" and does not settle chain depth. One hop covers the Spec shapes Phase 1 can express | amend — it dissolves under D17: the output names the port its link starts from, so there is no chain to search |
| D30 | 02 SC-30c, 03 SB-13 | The `schema` key is skipped at any depth, like `extensions` | D19's fix widened SB-13's false-negative surface: a Spec carrying `{"schema": {"executor": …}}` anywhere now escapes the scan. B3 already accepts that shape of residual risk for `extensions`; this extends it | confirm — B3's accepted residual. Optional tightening, not required: skip `schema` only under a `params` array |


### Findings from the second-opinion pass, for the owner's verdict

A fourth pass reviewed §4's X1–X12, §11's open questions 1–7, each spec's Decisions
table and D1–D30 above — 112 items — against the Vision and §67's phase order. It was
run by **Fable 5.1**, under the one exception `AGENTS.md` §8 carries for a second
opinion on a judgment Opus has already made; the code fact each row below rests on was
re-verified independently of it. Verdicts: 91 confirm, 15 amend, 1 reverse (D27), and 5
the pass calls **not a decision** — OQ4, D1, D6, D11 and D14 are encoding consequences
and fixture notes, and need no verdict. D3's and D5's applied spec edits were recomputed
and stand.

**Applied 2026-09-22.** Every verdict above is recorded, and the amendments each one
calls for are applied: the self-contained ones (D15, D24, S1, B4, open question 4) and
then the rule-text ones (open question 2, R10, D8, D16, D22, D23, D25, D26, D27, D32,
D33) and this cluster. What is *not* applied is the Vision editing that the confirmed
verdicts imply, which OV-6 defers to §12 step 3, and two values that need the Radio
Model: the memory domain a resource port delivers from, and whether a radio stream
declares one port per sample format or lets `effective` narrow one port's contract at
`prepare`. SB-25a is **withdrawn** and keeps its number (OV-1). `OutputSource` is gone,
so D1's verdict now stands for `Constraint` and `CapabilityValue` only.

**One defect in five places.** A Spec plus a BindingProfile does not carry everything
`plan()` needs, so the runtime supplies the remainder at assembly time — through
`CompileInputs::sink_components`, `CompileInputs::executors` and a `Fragment.content`
that holds only the binding's selector. None of that is recorded in any document, so
RS-38's Manifest cannot reproduce the plan it describes, which is the provenance Vision
§50 asks of it. D17 and D18 above are two of the five; the second opinion changes what
their rows recommend, so the revision is recorded here rather than by rewriting them
(OV-1). D31–D33 are new. Phase 2's MockRadio is the first real Provider and meets all
five.

**D17, revised.** None of the three fixes that row names is right; all three keep the
category error underneath it. The Vision makes Sink a Module **role** (§7 axis 2, audit
§13, MA-2, MA-25, MA-30), parallel to Provider and Executor, with its own trait and its
own rank in the stepping order. RS-12 and SB-25a instead put a Sink into
`graph.components` as a `ComponentDescriptor` — with `impl`, `requires.executor_kind` and
`timing`, none of which mean anything for a Sink — and SB-25 then requires it placed in
an Executor's Island (MA-39). A `Sink` member on `ComponentKind` or a `role` field on
`ComponentDescriptor` keeps a Sink an Executor-loaded component; `module` on every
placement is a second source of truth for what the Spec's descriptor already says, which
SB-25a itself refuses. The recommendation instead: **an output is bound, not placed.**
`bindings` maps each `outputs[]` id to a Module holding the Sink role (SB-22); `OutputReq`
carries the source port and the drop-class `policy` and `capacity` of the link that feeds
it (SC-19, SC-21); `ComponentPlacement.module`, SB-25a and `plan::sink_components` all go
away, so the Kernel surface nets smaller. It settles D18 with the same mechanism and
dissolves D29 — the output names the port its link starts from, so there is no chain to
search — and it gives RS-12's implicit Spec the link its recorder currently lacks, which
is why a Session's capture records nothing today. One sub-choice the Vision does not
settle: where a **Session** recorder's source comes from — an optional `Binding.source`
read only for a Sink binding on a Session profile (one Kernel field), or inferring it
when the profile has exactly one Provider binding with one stream port (no field, but the
first multi-device Session in Phase 6 breaks).

**The sub-choice, settled: the source is declared on the Sink binding.** Inference
breaks at the first multi-device Session and would then need this same field. One
consequence reshapes it: SC-19 forbids a default policy and capacity, so a source port
alone is too thin, and one shape serves both the Spec's output and the Session's
binding —

```text
SinkFeed  { port: PortRef, policy: BackPressure (drop-class, SC-21), capacity: u32 >= 1 }
OutputReq { id, kind, feed: SinkFeed, params }          -- replaces source + OutputSource
Binding   { module: ModuleId, selector, profile, feed: optional SinkFeed }
```

SB-22 gains: a Sink binding on a Session profile carries `feed`, RS-12 makes it the
implicit Spec's output, and a binding that carries `feed` on a Spec Run — whose
`outputs[]` already declare their feeds — is refused. Three further obligations come
with it, none of them in D17's row: `bindings` is keyed by resource names, output ids
and Island executor names together, so SB-22 must refuse a Spec whose three namespaces
collide; a bound Sink needs an address, because `Action::UpdateParameter`, `Stop` and
`Event.source` all take a `ResourceId` and `SinkDescriptor` has no id, so it is
addressable as `ResourceId { node: LOCAL, path: <output id> }` and RS-14's
`sink.capture` retargets to it instead of to the radio; and renaming `Binding.provider`
to `module`, which a Binding naming a Sink or an Executor needs, departs from the
illustrative YAML in Vision §8 and so owes a line in 03 §9's Vision issues for §12.

**D18, revised.** The fix is not for the runtime to keep supplying the Executor's Module
id: MA-38 should name a binding. `bindings[placements.islands[].executor]` resolves to a
Module holding the Executor role (SB-22), and the runtime supplies only the descriptor.
Otherwise `plan.fragments[].instance` and the Manifest's `modules` rest on assembly-time
input that no document records.

| # | Where | What the specs leave unsettled | Why it matters | Verdict |
|---|---|---|---|---|
| D31 | 05 §4 `Resource`, MA-10; 03 SB-15 | A Spec cannot connect a bound resource's stream to a component | `PortRef.component` may hold a resource ident, but `Resource` declares no Ports, so `validate` has no contract to check against a non-component endpoint and `plan()` refuses the link twice — `check_cycles` sees an unknown node and reports it as an MA-22 cycle, then MA-39 refuses it as "touches an unplaced component". Vision §7's own correct diagram is `PHY Processor → SampleStream → Radio Port` and §21 defines `Port`, so the shape is intended. Phase 2's MockRadio has nowhere to send a block on a Spec Run. The specs need `Resource` to declare its Ports (MA-10), SB-15 to admit a resource endpoint, and a resource endpoint to be a node outside every Island | amend — as the row says: `Resource.ports`, SB-15 admits a resource endpoint, a resource is a node outside every Island, and SC-3 runs against the declared port contract. The producer-side memory domain is a Phase 2 value |
| D32 | 03 SB-39, SB-44; 05 MA-12, §4 `Requested` | `Provider::prepare` never receives the request the matcher matched | `plan()` writes a Provider fragment's `content` as the binding's selector alone and is not given `validate()`'s result, so neither the matched node nor the Spec's constraints reach any Provider. 05 §4 says `Requested` is "what the matcher offers a Provider's `coerce`; prepare sees the same input", and nothing delivers it, so MA-12 and SB-44 — `prepare` reports the same coercions `coerce` did for the same request — cannot be honoured. The test that proves MA-12 hand-builds the fragment, which hides it. A Provider would have no rate or frequency to configure. SB-39 should take the admission result and define the fragment's content as `{ selector, requested }` | amend — as the row says: `plan()` takes the `AdmissionResult`, and a Provider fragment's `content` is `{ selector, requested }` with `requested.resource = matched[name]` |
| D33 | 03 §4 `KeyDecl`, SB-2; 05 MA-35; 04 RS-17 | Where a Provider parameter's update class is declared | `KeyDecl` carries `kind`, `coercible` and `coercion_default` and no update class, so RS-17 has nothing to consult for a Provider key and `Admitter`'s class map is supplied by the caller. §27's own examples of runtime mutation — TX gain, antenna beam, MCS — and §3's `sdr.rx.gain = 20` all target a Provider, whose parameters are Vocabulary keys and not `ComponentDescriptor.params`, so the Easy API's first parameter change is refused as undeclared. One optional `update_class` on `KeyDecl` (absent: not changeable during a Run) settles it before the freeze; adding it in Phase 2 is a Kernel document change | amend — as the row says: `KeyDecl.update_class` optional (absent: not changeable during a Run), read by `Admitter` beside `ComponentDescriptor.params` |

The same pass raised further items which are **raised and not applied**, each smaller and
self-contained: a missing mandatory `version` on the Manifest (Vision §10), a
non-ASCII object key making `seal()` fail after a Run has already transmitted
(RS-11), public inner fields on `ChannelMask` and `BlockFlags` turning S1's promised
minor widening into a major, an `UpdateParameter` that carries no `at`, so RS-19's
admitted instant is computed and then dropped and `hardware_timed` is
unimplementable, no way to attach a discontinuity-opened burst's late outcome
(SC-24a), `ComponentDescriptor::validate` and `check_effective_narrows` with no
caller, and an `ezsdr.time` section SB-26 declares the Kernel reads and nothing
reads.


### Findings from the fourth code-review pass, for the owner's verdict

A fourth adversarial pass (Opus, per `AGENTS.md` §8) reviewed the crate at `e8d5caa`,
concentrating on that commit's own diff, and found **1 P0, 11 P1 and 12 P2**. Every
P0 and P1 was executed against a scratch copy rather than inferred. All of them are
**applied**, each with a regression test whose expectation is the rule's own
obligation (OV-3), and three of the P2s are raised below instead.

The pass confirmed the crate's defect class for the fourth time. Five of its findings
were a rule whose enforcement existed as a correct function nothing called:
`AdmissionResult::into_result` (the P0 — `validate()` reported an RF-envelope
violation and `plan()` produced a complete, armable plan from it, so SB-30's first
point refused nothing), `ComponentDescriptor::validate`, `check_effective_narrows`,
and the un-read `ezsdr.time` section of SB-26. Each rule now names its call site in
its own text, which is the only mechanism that has held: SB-30 names all three
points, SB-41 names MA-12's two obligations, MA-37 names `validate()`, and MA-41
names the comparison.

Two of its findings were the `e8d5caa` cluster's own: a resource endpoint's port was
resolved against the **whole** instance tree rather than the bound node, so SC-3
checked a different node's contract — and the test written for it asserted that
behaviour, an OV-3 failure; and `Binding.feed` was never refused on a Spec Run, so
its `Block` policy went unseen. Both are fixed, the test rewritten to discriminate the
bound node by giving the double's root and its lines the same port name with different
contracts.

`kernel_surface` had **four** working evasions, all demonstrated: an unbalanced `{`
in a comment blinded the scan to every later item in the file, `union` was not an item
keyword, a name on the next line was invisible, and `pub use serde_json::Value as X;`
put a dependency's type on the surface unlisted. OV-23 now states the closures, and
all four were reproduced against the fixed gate.

| # | Where | What the specs leave unsettled | Why it matters | Verdict |
|---|---|---|---|---|
| D34 | 00 OV-13, OV-15a; 03 SB-6 | Whether `Value` and `Scalar`, the two OV-13 carve-outs, share one equality | `Scalar` now crosses `Int` and `Float` exactly, because SC-2's "identical re-registration" depends on it. `Value` derives `PartialEq` and does not, so `Value::Int(20) != Value::Num(20.0)` while SB-6's comparison calls them equal and OV-15 gives them one hash — three notions of equality for one document family. Nothing on the live path compares two `Value`s with `==`, so there is no Phase 1 consequence; the first replay or Manifest diff that does will find one | amend — `Value` takes `Scalar`'s exact cross-kind equality (one canonical form under OV-15: the float integral, its magnitude at most 2^53, equal to the integer) and `partial_cmp_scalar` compares `Int` with `Num` in 128-bit arithmetic instead of `as f64`, the cast SC-2 already removed. SB-6's `Eq`, `Range`, `Min` and `Max` all pass through it, so a capability match above 2^53 is inexact today. OV-15 already gives the family one notion of "same value"; a second is a defect, not a choice. No schema change |
| D35 | 01 TM-16c | Whether an Authority may schedule on a domain that ticks finer than its root | `to_root` converts with a floor, so `schedule` and `wait_until` on a `Derived` domain whose `root_ticks_per_tick` has a denominator above 1 queue the callback at a root tick at or **before** the requested instant, and two distinct derived instants can collapse onto one. TM-16c requires the callback at `t`. No Phase 1 fixture builds such a domain and TM-3 permits one, so this is a latent hole rather than a live defect: either TM-16c admits the floor and says so, or TM-13a refuses a non-integral ratio for a domain an Authority governs | amend TM-16c — `to_root` converts with `try_exact`, so `schedule`, `wait_until` and `advance_to` refuse an instant that is not a root tick (`Inexact { floor }`, an existing variant) while `now` keeps the floor: a reading may round, a firing may not, because a callback fired at the floor observes `now()` earlier than `t`. Rejected: admitting the floor (fires early, collapses distinct instants), a ceiling (the same defect mirrored), and TM-13a refusing non-integral ratios (TM-3 admits any capped rational, so a legal rate would fail at `prepare`) |
| D36 | 02 SC-24, §4 `BurstStep` | How a discontinuity whose opening block also carries `END_OF_BURST` is reported | `on_block` pushes **two** records and returns `Discontinuity { closed }` naming only the first, so a caller tracking state from the return value believes a burst is open while `state()` says `Idle`. The records are right; the return type cannot express "a discontinuity, and then it ended". Either `BurstStep` gains that shape or SC-24 says the caller reads `state()` after a `Discontinuity` | amend — `BurstStep::Discontinuity` gains `then_ended: Option<BurstRecord>`, `Some` when the reopening block also carries `END_OF_BURST`, so a block's return value states every transition it caused and agrees with `state()`. A runtime value, not a document: no schema change. Rejected: sending the caller to `state()` (a return value that may lie is what a normative state machine exists to prevent) and a fourth variant |


### Findings from the fifth pass (verification of the fourth), for the owner's verdict

A fifth pass re-ran the fourth's four probes against the fixed tree and then attacked
the fixes. **Every P0 and P1 of the fourth pass is confirmed fixed.** The fifth found
**1 P0, 5 P1 and 11 P2**, all of them executed; all are applied.

The P0 was a **regression of the fourth pass's own fix**: MA-12's new re-match read
`merged.effective`, which SB-41 says in as many words is lossy for a key two fragments
both name and which the Kernel does not interpret. Two channels each asking their own
line's declared count refused each other, so the ordinary two-channel Spec — the first
thing Phase 2 will write — was unrunnable. Fail-closed, and caught only because the
reviewer was asked to attack the fix rather than re-check the defect. Each resource is
now judged by its own `PrepareReport`.

Two further findings were a fix that was necessary but not sufficient. `plan()`'s new
SB-30 guard accepted `AdmissionResult::default()`, because "admitted" means only that
nothing was rejected — so a stale result planned with no `requested` on any fragment.
And the `sink/` prefix narrowed the Sink-address collision rather than closing it,
because `sink/<anything>` is a legal Provider node path; the segment is now reserved.
The `Scalar` equality fix survived at one value, `i64::MIN`, the boundary the
hand-written vector table omitted — its test now asserts the **criterion** (equal iff
`ContentHash::of` agrees) instead of a list of cases.

`kernel_surface` had two evasions left, and one was **already load-bearing in the
shipped crate**: `src/id.rs` generated `ClockDomainId`, `MemoryDomainId`, `IslandId` and
`DataLinkId` from a macro, and none of the four Kernel document types was on the
allow-list or in the OV-23b count. The macro now carries only impls, the four types are
declared and listed, and the gate refuses a macro that declares a type.

| # | Where | What the specs leave unsettled | Why it matters | Verdict |
|---|---|---|---|---|
| D37 | 03 SB-16, SB-43; 04 RS-49a, RS-51; 02 SC-27 | What Phase 1 owes for `spec.schedule`, given that there is no Kernel `arm` | Nothing read `spec.schedule` at all, so a `SpecTime` whose `clock` named no resource passed every stage. `validate()` now refuses that, which is the half a document can be checked against. Three halves remain: SB-43's resolution of a `SpecTime` to an `AbsoluteDeadline` needs the coordinator (Phase 2); RS-51's and SC-27's "`RejectAtPlan` is legal only on a statically known target" has no plan-time check, and in Phase 1 the schedule is the only place a Spec declares a burst, so every scheduled target *is* static and the rule bites only on a Reactor-emitted burst, which Phase 1 cannot express. Marked forward here; the alternative is to say Phase 1 refuses `RejectAtPlan` outside the schedule, which is a rule about a shape that does not exist yet | confirm the forward markings — SB-43's resolution is Phase 2's, and Phase 1 has no shape for a Reactor-emitted burst, so a `RejectAtPlan` rule outside the schedule would be a rule about nothing. **amend** SB-16 and `validate`: an entry whose `action.target()` names nothing the Spec declares is refused too, and SB-16 states the namespace that check needs — **a Spec target is Spec-relative**, its first segment a Spec resource name or `sink/<output id>`, rewritten at `arm` through `admission.matched` as `SpecTime.clock` is. INFERRED from §8's portability and §59/§61; naming Provider node paths instead would make Phase 8 parity depend on the Mock mirroring the X310's node names |
| D38 | 00 OV-3, OV-19 | Whether the OV-3 disposition review is satisfied by a rule-ID-prefixed test, or by any named carrier | The pass measured the sweep exit criterion 2 asks for: of 260 rule bullets at the time (the pass wrote 259; recounted 2026-09-22, and D47 has since added SB-15a for 261), 6 are withdrawn, 8 forward, 10 producer and 1 consumer obligations, leaving **234** that default to "checked by a Kernel item and at least one Phase 1 test". **69 of those have no test whose name begins with their rule ID**, which is OV-19's convention, and **14 are named nowhere in `src/`** — `OV-4`, `OV-4a`, `OV-5`, `OV-6`, `OV-7`, `OV-8`, `OV-9`, `OV-18`, `OV-19`, `OV-22`, `TM-16b1`, `MA-17`, `MA-44`, `MA-45`. Most of the 69 are covered by a test named for a neighbouring rule and say so in their own text, and the governance rules have no Kernel item by nature. The exit review owes a verdict per rule either way: add OV-19's marker, or name the test that carries the obligation. The four rules of D37 are what the sweep finds when it is done rule by rule rather than by grep | amend OV-3 and exit criterion 2 — the prefix count measures OV-19, not OV-3, and criterion 2 restates only the marker tally, so the 234 unmarked rules have no exit check at all. OV-3 gains a fourth marker, **process obligation**, for OV-4…OV-19, whose carrier is a document or the test tree and never a Kernel item; its default reads "at least one Phase 1 test, driving a Kernel item where the rule names Kernel behaviour", which covers OV-22, MA-44 and MA-45; TM-16b1 cites TM-14's test and MA-17 becomes a Phase 2 producer marker. Criterion 2 becomes a **per-rule carrier table read from test bodies** — the table is the evidence, a rule-ID prefix is not |


### Findings from the sixth pass, for the owner's verdict

A sixth pass, by a reviewer given no context from the fifth, re-ran the fifth's probes
and then attacked the fix set. **Every P0 and P1 of the fifth pass is confirmed
fixed.** It found **1 P0, 7 P1 and 12 P2**, all executed; all are applied.

The P0 was **the second regression in a row inside `collect_prepare`**, and the same
kind: MA-12's constraint re-match was applied to every key, and a coercion is by
definition a key whose applied value does not satisfy the requested constraint — so
every coercion was refused at `prepare` and SB-46's `accept` and `warn` branches became
unreachable. Spec 03 §2's own headline evidence, the UHD 19.5 → 20 Msps coercion, was
unrunnable. SB-46 and MA-12 now state the division: a declared change is SB-46's, an
undeclared one is MA-12's.

Two process facts are worth recording, because they are why a P0 on the crate's most
cited behaviour survived two passes and 287 green tests. First, a test helper had been
half-fixed: its comment said it passed the caller's Spec and the next line still
substituted `minimal_spec()`, so no caller's constraints were ever re-matched. Second,
when the coercion refusal first appeared it was **attributed to a neighbouring rule and
the fixture was moved away from it** — SB-30's second point is *about* a coerced value,
so removing the coercion from the fixture removed the rule from the test. Both were
authored while applying the fifth pass's findings.

The gate's third round of evasions — a raw string, a macro emitting a `pub fn`, and
`pub extern "C" fn` — changed its design rather than its state: it now **refuses the
construct**. `src/schema.rs` then immediately gave up a second hidden public item,
`document_schemas`, by the same rule that had exposed the four id types.

| # | Where | What the specs leave unsettled | Why it matters | Verdict |
|---|---|---|---|---|
| D39 | 00 OV-3, OV-19; 03 SB-16 | Whether a rule-ID-prefixed test name is evidence of OV-3 compliance | Three review passes each found a test that carried the right rule-ID prefix and asserted the implementation rather than the rule — `sb_15_…`, then `sb_16_…`, then `ov_15_…`, which named the three call sites its rule is about and then called the predicate directly, none of the three being the call site. So D38's count of missing prefixes is a lower bound on the work and not a measure of it: a present prefix is not evidence either. The exit review's per-rule verdict has to read the test, not its name. The sixth pass also proposes one mechanical step for it that has now produced **six** findings across three passes: grep for a predicate with no caller in `src/`. Also: D37 called the schedule's `clock` "the half a document can be checked against", which is wrong — a schedule entry's `action.target` is equally checkable and a dangling one still validates | confirm — carried by D38's criterion-2 rewrite (read the body) and D37's target check. The mechanical step becomes **step 0 of §12**: list every predicate in `src/` with no caller in `src/`; each is wired or its rule re-marked. Six findings across three passes came from it |


### Findings from the seventh pass, for the owner's verdict

A seventh pass was given two targets, because the sixth had named exactly two places
where anything was left: the coercion exclusion the sixth pass's own fix introduced,
and the surface gate. It found **1 P0, 2 P1 and 9 P2**, all executed; all but the four
rule questions below are applied. It confirms **all nine of the sixth pass's fixes
hold.**

The P0 was the **third** defect in `collect_prepare` and the third of the same shape,
but in the opposite direction from the previous two: the exclusion was keyed on the
**report's own** `coercions` list, which is Provider-declared, and nothing compared the
declared `applied` with what reached `effective`. So naming a key in `coercions` was a
self-issued exemption from MA-12's re-match, and a Provider could run a Spec that asked
for `Eq(20.0)` at 100 with a Manifest recording the substitution as a legitimate
coercion — spec 03 §2's v3 failure restored, with the Manifest now stating a value
other than the one applied. The exclusion is keyed on `admission.coercions_preview`,
which the Kernel computed itself, and what is checked for such a key instead is SB-44's
own obligation. After it, every input to the `prepare`-stage checks is either a parsed
document or a value the Kernel computed at `validate`; no Provider-supplied datum
decides an admission any more.

The gate's fourth round of evasions produced its **design** rather than another patch.
Two of the four refusals the sixth pass added had been written as line prefixes and were
evaded by a line break, so every refusal is now matched as a token; and above them sits
the check that closes the brace class over its **cause** — the scan asserts that its own
frame stack ended balanced, since every one of the four brace-class evasions had that
one signature. All eight evasions the four passes demonstrated are now caught, verified
against a scratch copy.

| # | Where | What the specs leave unsettled | Why it matters | Verdict |
|---|---|---|---|---|
| D40 | 03 SB-41, §4 `PrepareReport` | Whether `constraints_hit` stays | It has **no reader** anywhere — ten construction sites in tests, the field declaration, and nothing that reads it. Its stated meaning, "keys whose declared bound the request reached", is neither a superset nor a subset of "keys that were coerced", so it is not the datum the coercion exclusion needed either. Either a rule names a reader for it or the field goes, which is a pre-freeze schema change (OV-12) | amend — **remove `constraints_hit`.** The prior question is not who reads it but what it means: the Vision's PrepareReport sketch and audit §13 both list the name without semantics, and SB-41's gloss was written by this spec, not taken from them. What a Provider would say with it is `warnings` already, and the Kernel computing "hit the bound" would be Core deciding what is interesting (§63). Pre-freeze schema change plus a changelog line (OV-12), ten test sites, and one line under 03 §9 for the Vision's own sketch |
| D41 | 03 SB-44, MA-11 | The granularity of "the same request" | `match_constraints` calls `coerce` with a **single-key** `Requested`, once per key, while a fragment carries the resource's whole constraint map and a Provider replays `coerce` over all of it. For a Provider whose keys interact — rate × decimation — the two answers differ legitimately, so SB-44's equality is not literally satisfiable and the Kernel cannot check more than it does now (that the applied value equals the previewed one, per key). Either SB-44 says per-key, or `coerce` is called once with the whole map at `validate` too | amend SB-7 and SB-44 — **one call, whole map.** `validate` calls `coerce` at most once per bound node with the resource's whole `requires` map, after SB-7's fail-fast, and judges each key from `applied`. "The same request" then literally *is* the fragment's `requested`, so MA-11's determinism makes the `collect_prepare` check exact rather than approximate and a rate-times-decimation Provider is no longer refused for being consistent. Rejected: SB-44 saying per-key, which outlaws interacting keys at the Kernel's contract — Phase 2's first real Provider |
| D42 | 03 SB-46, SB-45 | Whether a coercion on a key the Spec never constrained is meaningful | At `validate` it cannot arise: `coerce` is called only for a constrained, coercible, not-directly-satisfied key. At `prepare` the policy loop applies that key's default to whatever the Provider declares, so a Provider that reports its own applied gain as a coercion fails `prepare` under a `reject` default for a key nobody asked about. Either SB-46 restricts the loop to the Spec's own keys, or it says a Provider may not declare one | amend SB-44 and SB-46 — a `Coercion` whose key is not in the fragment's `requested` map is a **malformed report** and fails the fragment (SB-42): nothing was requested, so nothing was coerced, and a Provider choosing its own default is MA-12's narrowing case, which belongs in `effective` alone. SB-46's loop then runs over requested keys by construction. Rejected: restricting the loop but keeping the record, which still reaches the Manifest through the report |
| D43 | 04 RS-15, RS-11; 03 SB-9a | What a refused Action's value does to the log | RS-15 logs every Action, admitted or not, and `SessionLog::append` is public and takes the value as given. A value carrying a non-ASCII key therefore makes the log — and the Manifest holding it — unhashable, which is what `compile`'s new check prevents on the compile path but not on a direct `append`. Either RS-15 says the log holds only what passed SB-4's shape check, and a value that failed it is logged by reference or by description, or `append` refuses and RS-15 loses "every Action" | amend RS-15 and `SessionLog::append` — `append` runs the same shape check `compile` runs (SB-4's nesting, OV-15's ASCII keys) and returns `Result`. RS-15's "every Action" survives once RS-15 says what an Action is: a well-formed document. A **rejected** Action still takes a sequence number; an unhashable one never was an Action, and a log the canonicaliser cannot hash breaks RS-11 for the whole Run. The log is the choke point because `append` has no caller in `src/` — the compile path alone protects nothing. Rejected: logging by reference or description, a second representation of one Action |

| D44 | 03 SB-3, SB-22; 05 MA-10 | Where an instance's identity comes from | `CompileInputs` holds it only as a `&dyn Provider`, so the SB-3 path check distinguishes instances by address — the only discriminator available, and unsound in two cases: a **zero-sized** Provider has no address of its own (every `Box` over one is the same dangling address), and a field's address can equal its container's. A size guard closes the first; the second needs an identity the Kernel does not have. Either `ProviderInstance.id` is required to be unique across bound instances and checked, or the Phase 2 coordinator supplies an instance identity and `CompileInputs` carries it | amend SB-3 and `check_binding_names` — **instance identity is the binding description.** Two bindings with equal `(module, selector, profile)` are one instance, per D18's document-not-assembly principle and SB-23's reading of `instances: 2`; the Kernel walks it once and refuses if their Providers report different `instance().id`s. Two descriptions are two instances: distinct ids, disjoint node paths. No pointer is compared and the size guard goes. Rejected: address identity (unsound both ways) and a coordinator-supplied id (assembly-time input again). Consequence: fixture churn, because `profile_binding` gives every binding an empty selector |

The pass also asks the owner one question that is not a rule: **whether `cargo public-api`
earns its place.** X11 rejected it because it needs nightly rustdoc JSON, and four passes
of evasions are the cost of that choice. The gate is now total for the constructs that can
hide a module-level public item, and its own balance check fails loudly if it is ever
mis-lexed again — but it is still a line-based scanner in a test file, and the answer to
this question decides whether it survives Phase 1.

---

### Findings from the eighth pass (a cross-family second opinion), for the owner's verdict

The eighth pass was a **second opinion by a different model family** on the whole crate,
under the exception `AGENTS.md` §8 now carries: the first five passes were one family's,
and the two defect classes they kept producing are the kind a reviewer inherits from the
author. It found **1 P0, 6 P1 and 10 P2**, every one executed against a scratch copy
rather than inferred. Nine were verified independently before anything was changed.

The P0 was the **fourth** defect in `collect_prepare`, and it appeared *inside the third
one's fix*. The sixth pass had moved MA-12's re-match to each resource's own report; the
seventh had keyed the coercion exclusion on the Kernel's own preview. Neither noticed
that `coercions_preview` is a flat `Vec<Coercion>` and that a `Coercion` names a key and
two values and no resource — so no lookup over it can be per-resource. Keyed on the key
alone, one resource's coercion was charged to every other resource constraining that
key: two channels asking their own line's grid, one of which coerces, and the one that
was **satisfied directly** — never passed to `coerce` at all — is refused for not
applying a value nobody computed for it. That is the ordinary two-channel Spec, which is
the first thing Phase 2 writes. The structural fix is that the Kernel's own record now
says which resource it was computed for (`PreviewedCoercion`), which is what
`RejectedConstraint` beside it already did.

Two P1s were of the "pasted, not replaced" kind, both introduced by the **previous**
commit: `check_binding_names` carried D44's binding-description block *and* the pointer
comparison D44 says was removed, plus the SB-22 `sink` check twice. The stale block
refuses what D44 permits — two Provider objects reporting one `instance().id` under one
binding description, which is the runtime handing the Kernel a fresh handle per name.

The other three P1s were rules enforced differently from their text. `derive_class` read
`ezsdr.time.class` through `and_then(as_str)`, so of MA-41's three named refusals —
absent, non-string, unrecognised — only the third fired, and the two the clause names
*first* planned as `Simulation`, the class that may claim determinism. `declared_links`
resolved a link's contract through `graph.components` alone and fell back to
`ezsdr.control`, so every link whose consumer is a resource port — Vision §7's own `PHY
Processor → Radio Port` — reached the Manifest and a Link Module's `create` carrying a
contract the Kernel had already checked as something else. A malformed `ezsdr.arm_order`
entry was skipped rather than refused, so a misspelled `before` left the PPS source
armed second: the v3 start-up failure SB-39 exists to prevent, with no diagnostic.

The gate produced **five** more executed evasions, and this time one of them was live in
the shipped crate rather than a demonstration: `is_testing_gated` matched any `cfg` whose
tokens *contain* `testing`, so `#[cfg(not(feature = "testing"))]` — the **default** build
— was skipped whole. The others were `pub use {…}` (a brace group named no root), `pub
extern crate` (the item fell through the match), `include!` of a non-`.rs` file, and two
inline modules with same-named items collapsing onto one allow-list key, because the key
encoded depth rather than the module's name. All five are now caught, verified by
injection.

**Applied** without a rule question: the P0; both stale blocks; MA-41's three refusals;
`declared_links` through `source_contract`; `arm_order`'s refusals; `Value`/`Scalar`
equality and the matcher's `Eq` (first through the canonicaliser, then — D50 — on exact
numeric comparison, since the canonical form equates different numbers above 2^53);
SB-11's `major`; RS-52 on a
scheduled `UpdateParameter`; SB-24's Authority role read off the registry as MA-25 reads
a Sink's; SC-30b's carry held for the next Gap; a coercion of an unrequested key refused
at `validate` as D42 refused it at `prepare`; the five gate holes; and three tests whose
assertions were vacuous or were the implementation rather than the rule.

| # | Where | What the specs leave unsettled | Why it matters | Verdict |
|---|---|---|---|---|
| D45 | 04 RS-17, 03 SB-30 | Whether `Admitter::admit` is the implementation or a fourth copy of it | RS-17 names two Phase 1 call sites — "`validate()` calls it with a Spec's requested configuration, `prepare()` with the applied one" — and **neither does**; `admit` has no caller in `src/` at all. The three copies of the sequence already differ: `validate` accumulates every coercion violation and `admit` returns on the first, and the update-class step exists only in `admit`. Routing both through it is not mechanical, because `validate` accumulates by SB-38's design and `collect_prepare` runs the prepare-stage checks once over the **merged** configuration, which per-fragment admission would change into an over-refusal | **adopted** (Fable 5.1 recommendation, 2026-09-22) — amend RS-17. The three *steps* are already single functions (`AdmissionCheckRegistry::run`, `coercion_policy`+`apply_coercion`, `declared_class`); what differs is aggregation, and each difference is mandated elsewhere: report-all (SB-38) at `validate`, fail-the-transaction (SB-41, SB-42) at `prepare`, refuse-before-dispatch (RS-16) for an Action. `admit`'s step 3 is already gated to `Runtime`, so routing the other two through it would skip it. RS-17 now states the shared check set and its order with its three named sites; the allow-list entry for `session::Admitter` names the runtime point. Rejected: a fourth restructuring of the function that produced four P0s, to make true a sentence whose binding half already was. Text only |
| D46 | 04 RS-14, RS-51, OV-21 | Whether the Kernel may supply the meaning of a Session verb | `session::compile` hard-codes the value a `capture` verb sets (`Bool(true)`) and `LatePolicy::SendAsapAndFlag` for every Session-compiled `TxBurst`. `CompileRule` carries neither, so two Vocabulary-level decisions live in Core. Either `CompileRule` carries them, or a rule says the Kernel's default is normative | **adopted** — two different fixes. The `UpdateParameter` value now comes from the action's `params` under the rule's key and an action carrying none is refused: a Kernel default is a Vocabulary meaning in Core (OV-21) and is invisible to RS-20, which reproduces a Run from the logged `SessionAction`. `CompileRule::TxBurst` gains a mandatory `late_policy` beside `repeat`, and `RejectAtPlan` is refused at Vocabulary registration since a Session burst's target is resolved at `compile`. Pre-freeze schema change to `vocabulary_descriptor` plus a changelog line; three capture fixtures now carry their value. Rejected: a rule making the Kernel default normative (RS-28's own reasoning applies verbatim), and carrying the policy in `params` (the Kernel would read a Vocabulary key) |
| D47 | 02 SC-1, 03 SB-15 | Whether a link's port **directions** are checked | `Port.direction` is carried through the whole compile path and never read: a link `a.in → b.in` validates and plans. No rule states the obligation, so this is a rule that should exist rather than a rule unenforced — and a Phase 2 Link Module would be asked to carry it | **adopted** — new rule **SB-15a**, checked at `validate()` **before** SC-3, because a producer-to-consumer contract check is well posed only once which end is which has been established. A link's `from` names an `out` port and its `to` an `in` port, on a component or a bound node alike; an output's `feed.port` names an `out` port. `source_contract` became `endpoint_port` and returns the direction with the contract. One fixture linked `in → in` and was only ever legal because nothing checked. Rejected: leaving it to Phase 2's Link Module — every Link would repeat it and Mock and hardware could diverge. Vision §21 names two checks; a wording note goes to §12 step 3 |
| D48 | 04 RS-4, RS-18 | Whether two Run predicates are Phase 1's or Phase 2's | `RunStateMachine::check_running` (RS-18) and `check_structural_mutation` (RS-4) are correct predicates with no caller in `src/` and no `D`-row marking them forward, unlike `Lease::validate` (D20). The dispatch loop that would call them is Phase 2's. Either they get a forward marker, or the rules name a Phase 1 carrier | **adopted** — confirm (forward, Phase 2 coordinator). RS-4 and RS-18 gain D20-style markers naming the coordinator as the call site: RS-18's rejection must be logged at the Run's own `TimePoint` (RS-15), which only the coordinator holds, and no Phase 1 API mutates a plan, so RS-4 cannot yet be violated. The predicates stay. Rejected: wiring them into `compile` or `SessionLog::append` — a pure function and a log with no Run state — which would be a seam invented for a caller that does not exist |
| D49 | 03 SB-39 | Whether a malformed `ezsdr.arm_order` entry is an error | Applied as a refusal on the document's own evidence — SB-39 exists because a silently wrong start-up order is the v3 failure — but SB-39's text says only that edges "come from" the section. MA-41 states the clause explicitly for the sections the Kernel reads; SB-39 should carry the same sentence, or the refusal should be withdrawn | **adopted** — amend SB-39. The section's shape (`{ before, after }`, an array) and the refusal are written: a non-array section, a missing or non-string field, and an edge naming a fragment the plan does not have are each refused rather than skipped, as MA-41 refuses a malformed `ezsdr.time`. Also written: a Provider's `arm_after` naming an instance this profile does not bind contributes no edge, which the code did and no rule said. Code and test already existed. Rejected: withdrawing the refusal — it turns a misspelling into agreement |
| D50 | 03 SB-6, 02 SC-2, OV-15a | What "one value" means across `int` and `num` | Both rules define it as **one canonical form**, then gloss it as holding "exactly while \|v\| ≤ 2^53". The gloss is not an iff, and implementing it was wrong in both directions. **But the criterion is wrong too**, and worse: above 2^53 a float's canonical text is the shortest decimal that *names the `f64`*, not the number's exact decimal, so `Int(1152921504606847000)` and `Num(2^60)` share one form and are **two different numbers** — `ContractRegistry::register` took one for an identical re-registration of the other, SC-2's own named harm, moved from 2^53 to 2^60. Meanwhile `Int(2^60)` and `Num(2^60)` are one number with two forms | **adopted** — amend: **value equality is exact numeric equality**, the `Equal` case of the ordering the crate already has, and canonical form is *document* identity. `Eq`, `Set`, `Range`, `Min`, `Max`, `PartialEq` and SC-2's "identical" all decide by the one relation, so none can disagree; OV-15a's "one value, one hash" holds while \|v\| ≤ 2^53 and is stated as that coincidence rather than as the definition. **The code half is applied**, because equating two different numbers is a defect under either candidate rule text; SB-6, SC-2 and OV-15a now carry the sentences |

A verification pass over the applied fix set found **2 P0, 1 P1 and 2 P2 — both P0s in
the fixes themselves**, which is the third time in this crate that applying a verdict
introduced the defect it was applying.

The first was the RS-52 check above: it resolved a scheduled `UpdateParameter`'s class
from `graph.components[].params` before the key's `KeyDecl`, and SB-2 says in as many
words that the `KeyDecl` is "the only place it can be … a Provider's parameters are
Vocabulary keys and never a `ComponentDescriptor`'s `params`". Since a schedule target
is a Spec resource by SB-16 — checked five lines below in the same loop — the first
source consulted was the one the rule excludes. It both admitted a key with no declared
class, if any component happened to name it, and refused a key that states exactly what
its Vocabulary declares. The second was SC-30b's carried count: it was drained only at a
later jump, so a stream whose last drop is followed by a contiguous block and then ends
lost it at `finish` — the same loss, moved from mid-stream to end-of-stream.

The P1 is pre-existing and was the reason to look: `collect_prepare` did not refuse an
`AdmissionResult` that is not this Spec's, although `plan` does and although
`collect_prepare`'s own comment says it is reachable without `plan`. With
`AdmissionResult::default()` — which `is_admitted()` accepts — every `matched` lookup
missed and the whole MA-12/SB-44 block, including the P0 fix above, was skipped in
silence. Guarded now, as SB-39 guards the stage before.

A second verification pass over those five found **1 P1**, again in the fix: SC-30b gives
the carry three jobs — its flags derive the cause, its `lost` counts sum, its block count
reaches `link_dropped` — and a contiguous push was holding only the third. The trailing
Gap the fix had just started emitting therefore attributed a device overflow
(`RESTARTED`, `lost: 150`) to host-side link loss with the count discarded, which is the
failure SC-20b exists to prevent. The whole carry is held now. In the same place, the
`GapFlagWithoutJump` check judged the **merged** flags, so a carried `GAP_BEFORE` refused
a delivered block that was contiguous and correct; it judges the delivered block's own
flags, which is what the rule is about.

A third verification pass, narrowed to `continuity.rs` because that rework had been
reviewed by nobody and contained a **relaxation** made on this author's own reading —
`GapFlagWithoutJump` now judges the delivered block's own `GAP_BEFORE` rather than the
merged set — returned **no P0 and no P1**, and confirmed the relaxation against SC-13's
definition of the flag as a claim about *this* block, `StreamError::GapFlagWithoutJump`'s
own wording, and spec 02's test table. It found two P2s, both applied: the held carry was
cleared before one of `push`'s seven error returns, where the other six leave it for
`finish`; and the `Mixed` guard's change to read the carried block count had no
discriminating test, since a Gap that reports `link_dropped` cannot also claim that
nothing explains the shortfall. Five rows were added to spec 02's test table for the new
`sc_30b_*` and `sc_31_*` tests.

Also applied: a stray coercion is no longer recorded in `coercions_preview` at all, only
refused — leaving it there kept both harms the refusal exists to prevent; a glob `pub
use` is refused by the surface gate, because the root check short-circuits at the first
segment and `pub use crate::internal::*;` therefore named a module of this crate and put
everything behind it on the surface with no allow-list line; and the gate's three
predicates gained direct negative tests, since the five evasions had been verified only
by transient injection.

**Recommendations for D45–D50 (Fable 5.1, 2026-09-22, under AGENTS.md §8's exception;
the first pass on these rows was Opus's).** Each row's verdict cell carries the
recommendation and the alternative it rejects. All six are
**adopted** (owner, 2026-09-22) and applied: RS-17, RS-4, RS-18, RS-14, SB-39, SB-6, SC-2
and OV-15a carry their new text, SB-15a is a new rule, `CompileRule::TxBurst` gained
`late_policy` with a pre-freeze schema change, and D50's exact-equality criterion was
already in the code because equating two different numbers is a defect under either
candidate rule text.

The freeze ordering Fable gave, now moot because all six are applied, is kept for the
record. Settled **before** the v4.0 schema freeze rather than after: **D46**,
because `CompileRule` is on MA-46's frozen list and adding a field afterwards is a v2
with a migration; **D50**, because it changes what `Eq` means in a v1 Spec and what
"identical" means in SC-2, which after the freeze is a reinterpretation of frozen
documents that Vision §10 and SB-47 forbid; **D47**, because it refuses v1 documents that
validate today, and a tightening afterwards needs a migrate-or-refuse story; **D49**,
because it is Kernel-read document content whose shape is written nowhere. **D45** and
**D48** are rule text about code structure and test carriers, change no document, hash or
refusal, and can wait for Step 5.

One observation outside the six, recorded here so it is not lost: on the Session path
nothing calls `Provider::coerce` for a `SetParameter`, so RS-17's coercion step sees only
RS-19's "as soon as possible" record and a Session `sdr.rx.rate = 19.5e6` is dispatched
uncoerced. Phase 2's MockRadio meets it first; it may deserve a row of its own.

Four more OV-3 dispositions for exit criterion 2's table, found by reading test bodies:
`rs_10_abort_during_orderly_escalates` asserts on a fixture it built, because nothing in
`src/` records the escalation cause; `rs_03_failure_at_each_stage_reaches_cleanup` and
`rs_11_manifest_for_every_terminal_run` assert on `manifest_fixture(...)`, and RS-11's "a
Manifest is written" is not Kernel behaviour in Phase 1; `ma_39_island_admission` never
reaches the within-Island memory-domain branch. Each needs a marker or a carrier, not a
prefix.

## 12. What happens at acceptance (procedure only)

1. `git mv plan/phase1/0{1,2,3,4,5}-*.md design/` — accepted normative text lives in `design/` per `AGENTS.md` §2. This file stays in `plan/phase1/`.
2. Add one line to `AGENTS.md` §2: `plan/` holds design work in progress, `design/` holds accepted text.
3. The re-review's R13 edit of the Vision, **with separate owner approval**: for each section listed in a spec's coverage table, replace the normative block with a three-to-six-line summary plus `Normative: design/0N-….md, rules XX-n…m`, keeping the section number, its title and the part's header and footer navigation. Sections affected: §3 → 04; §7 → 05; §8, §10, §11 → 03 (schema technology → this file); §14 → 05; §15, §24, §49 → 01; §19 → 05 with the deadline kinds in 01; §21, §23 → 02; §22 → 02 and 04; §27 → 04 and 05; §29, §50, §53 → 04; §32, §38 → 05; §52 → 03.
4. In the same pass, apply the "Vision issues found" items the owner accepted, and add one revision-history row to the Vision index.
5. Update `handoff.md` §2 and §4; re-run the `v3/` path check from `handoff.md` §1 and a link check over `design/` and `plan/`.
6. The audit and the re-review are not edited. Their `§N` and `CMA §N` citations keep resolving, which is why step 3 keeps every section number.

---

## 13. Exit criteria

1. Six documents accepted across three gates, and every decision-table row has a verdict in §11.
2. Every normative obligation of every covered Vision section has a rule ID and no coverage table contains a gap, and every rule has an OV-3 disposition **recorded in the per-rule table OV-3 requires** — for an unmarked rule the test whose assertion is that rule's obligation, read from the test body; for a marked rule the marker and the phase or artefact. Counting rule-ID prefixes is not this criterion: a present prefix is neither necessary nor sufficient. Measured 2026-09-22 so the review knows its size: of 315 `#[test]` functions, **78** appear in no spec's test table — 19 of those are the `hashing`, `kernel_surface` and `schema_freeze` files this document names as groups rather than by test. The opposite direction is closed: every test name any spec cites now exists (`ma_42_fidelity_is_the_weakest` and `rs_49a_scheduled_action_is_a_template` were table rows with no function behind them and have been written).
3. `cargo test` passes on the pinned MSRV and on stable, with no `#[ignore]` among the tests the five specs' test tables name, and the whole compile pipeline runs end to end against the test doubles with no Mock.
4. `schemas/` is committed, `schema_freeze` passes, and `SCHEMA_CHANGELOG.md` has its v1 entry.
5. `kernel_surface` passes: `src/` parses, every public item is on the allow-list with a specific audit §13 token or a `NEW:` justification, its doc comment cites a rule ID, and no token from `tests/banned_tokens.txt` appears outside a comment citing the ban. The review records the `NEW:` count as the Kernel-growth number.
6. The Kernel crate's **direct** dependencies are exactly the four crates of §8. The resolved tree is larger (`sha2` brings `digest`, `block-buffer`, `crypto-common`, `hybrid-array`, `typenum`; `serde_json` brings `serde_core`, `itoa`, `memchr`, `ryu`; `schemars` brings `dyn-clone` and `ref-cast`), and that is expected. The dev-dependencies are the `testing` self-dependency and `syn`, the latter added under OV-18 for OV-23's parser and adding no new transitive crate; a dev-dependency is not a direct dependency of the Kernel and does not enter what a consumer builds.
7. After the move in §12, every link in `design/` and `plan/phase1/` resolves.
