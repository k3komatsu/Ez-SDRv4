# Phase 1 — Kernel semantic model: overview and plan

| Field | Value |
|---|---|
| Status | Draft for owner review. Governance rules `OV-n` become binding when this document is accepted. |
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
  04-run-and-session.md  RS-n   Run state machine, cleanup, Session action log, Lease, Policy, Event and counters, Manifest, hashing
  05-module-api.md       MA-n   the five role traits, ModuleDescriptor and registry, ComponentDescriptor, Island, ExecutionClass, fidelity
```

Every spec carries: header (status, scope, Vision § covered, audit §14.1 items covered) → evidence → model overview → types (language-neutral shape plus an illustrative Rust sketch) → normative rules with stable IDs → algorithms → decisions table → Phase 1 test table → Vision coverage table → Vision issues found → deferred items.

- **OV-1** A rule ID, once published in an accepted spec, is never reused or renumbered. A withdrawn rule keeps its number and is marked withdrawn, exactly as the Vision's §N numbers are stable.
- **OV-2** Every normative obligation carries a rule ID. A rule that places obligations on more than one implementer, or in more than one phase, is split into lettered sub-rules (`TM-13a`, `TM-13b`, …), which OV-1 protects like any other ID. A rule whose several clauses are checked by one function and one test stays whole and enumerates them. Text with no rule ID is explanatory and binds nothing.
- **OV-3** A rule with no disposition marker means "checked by a Kernel item and at least one Phase 1 test", and the exit review verifies that by finding a test row **whose expectation is that rule's own obligation**, not merely one citing its ID. The distinction matters: a producer-side rule such as spec 02's SC-13, an "if and only if" about what the producer knows, is cited by three rows that all test the builder's derivation instead, so an ID-matching check would pass it as covered. Every rule that is **not** in that class carries an explicit marker naming what it is — a **producer obligation**, a **consumer obligation** or a **forward obligation** — and the phase whose test covers it. The default is inverted this way so that the marker is never a judgment call: the first draft said "written in the rule itself where it is not obvious", and the rules whose disposition was least obvious were exactly the ones left unmarked. The exit review reports how many rules carry a marker and which phase each names.
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
| X11 | Kernel-does-not-grow check | A `kernel_surface` test (std only): every public item is on an allow-list naming either a specific audit §13 token or a `NEW:` justification, every public item's doc comment cites a rule ID, and no token from `tests/banned_tokens.txt` appears in `src/` outside a comment citing the ban (OV-23, OV-23a, OV-23b) | `cargo public-api` (nightly rustdoc JSON); a `syn`-based scanner (dependency) | The allow-list *is* the review checklist for "Core remains small" |
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
│   ├── src/time.rs                → time/{rational,domain,point,relation,authority}.rs      (01)
│   ├── src/stream.rs              → stream/{block,buffer,link,burst,continuity}.rs          (02)
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
│       ├── time_model.rs  stream_contract.rs  spec_binding.rs  run_session.rs  module_api.rs
│       └── schema_freeze.rs  kernel_surface.rs
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
- **OV-13** Serialisation conventions, so that non-Rust consumers get discriminators: data-carrying enums use an internal tag (`{"kind": "...", ...}`), unit-only enums serialise as `snake_case` strings, and namespaced opaque sections are `serde_json::Value` with `additionalProperties: true`.

Is generating the schema still "schema-first" in the Vision's sense? Vision §10's rule is that nothing is *defined only as a Rust type and re-described by hand* elsewhere. Nothing here is re-described by hand: the schema is generated, committed, reviewed and is the artefact every other language reads. The governance in OV-10 to OV-12 is what makes the committed schema, and not the Rust source, the arbiter.

**Documents** (schema, and a `version` field where the Vision requires one): `ExperimentSpec`, `BindingProfile`, `ExecutionPlan` summary, `PrepareReport`, `CoerceReport`, `Manifest`, `Event`, `Action` (including `TxBurst`), action-log entry, `Lease`, `Policy` table, `ExecutionClass`, `Fidelity`, `ModuleDescriptor`, `VocabularyDescriptor`, `ProviderInstance` and `Resource`, `ExecutorDescriptor`, `SinkDescriptor`, `LinkDescriptor`, `AuthorityDescriptor`, `ComponentDescriptor`, `IslandDecl`, the three fragment types, `ModuleError`, `StopReason`, `ArtifactRef`, `ContentHash`, `DataContract`, `DataLinkDecl`, `ContinuityMap`, `Gap`, `BurstRecord`, `SampleClockRecord`, and `ClockDomain` / `TimePoint` / `Duration` / `ClockRelation` / the deadline types as shared definitions.

**In-process only, no schema by design**: `SampleBlock`, `BufferRef`, memory pools and their handles (the `MemoryDomainId` *is* a document type), link handles, `EventSink`, `ActionReceiver`, every trait object, `TimeAuthority`.

---

## 7. Content hashing

- **OV-14** A content hash is the string `sha256:` followed by 64 lowercase hex digits. The hash algorithm is named in the value so a later algorithm is an additive change.
- **OV-15** A document is hashed over its canonical form: UTF-8 JSON text, object members sorted by key, no insignificant whitespace, RFC 8785 string escaping, and the **integer profile** — a value serialised from an integer type is written as an exact decimal with no exponent, whatever its magnitude, while a value serialised from a floating-point type takes the ECMAScript `Number::toString` form of RFC 8785. Non-finite numbers are rejected rather than written as `null`.
- **OV-15a** The integer profile is a deliberate, documented deviation from RFC 8785, whose number rule would route `1 700 000 000 000 000 000` through an IEEE-754 double and lose the last bits. It is recorded as a named deviation in `SCHEMA_CHANGELOG.md`, and no implementation that lacks it may be substituted.
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
- **OV-20** Test doubles live in `tests/support/` and are never compiled into `src/`. Phase 1 has exactly four: a test-double Provider, an in-memory DataLink implementing all three policies, a recording TestExecutor of about twenty lines, and a fake `HostClock`. The `ManualTimeAuthority` is the exception: spec 01 makes it normative (TM-17a) and Phase 2's Simulation Engine crate will build on it, which a sibling crate's `tests/support/` cannot provide. It therefore ships in `src/` behind a non-default `testing` feature, and OV-23's frozen-surface check excludes feature-gated items through one explicit allow-list line that says so.
- **OV-21** The test-double Provider uses no radio vocabulary. Its keys are `test.count`, `test.grid` and `test.flag`. If a matcher test needs the word "channel" or "rate", the matcher is not generic and the test has found a defect (audit F7).
- **OV-22** `schema_freeze` regenerates every document schema with pinned settings and compares it byte for byte with `schemas/`. `EZSDR_UPDATE_SCHEMAS=1` rewrites the files from inside the test; there is no build script and no `xtask`.
- **OV-23** `kernel_surface` scans `src/**.rs` and fails when: a public item is missing from the allow-list; an allow-list entry names neither a **specific token** from audit §13's Kernel tree nor a `NEW: <one-line justification>`; a public item's doc comment cites no rule ID; or a banned token appears outside a comment that cites the ban. Items behind the `testing` feature are excluded by one allow-list line that names the feature.
- **OV-23a** The ban is a fixed literal token list kept in `tests/banned_tokens.txt`, not "any Vision §6 term": `uhd`, `soapy`, `hackrf`, `rfnoc`, `replay`, `cuda`, `wasmtime`, `dev/net/tun`, `tuntap`, `af_xdp`, `dpdk`, `802.11`, `otfs`, `ibfd`, `taint`, `prometheus`. Vision §6 also bans "Processor execution ABIs", but `Processor` is a Kernel role in audit §13's module-api line, so a literal scan for §6's phrases would fail the crate on its first run and the check would be disabled within a week.
- **OV-23b** An allow-list entry marked `NEW:` is a type the Kernel has that audit §13 does not name. The exit review reports the `NEW:` count and each justification; that number, not the raw public-item count, is the "Core remains small" measurement. Audit §13's `data` line ends with "Stream Contract (normative)", which would otherwise absorb every type spec 02 invents.

**Phase 1 test inventory.** Each spec's own test table is the authority, and this document does not restate it. A second copy is what went stale inside one review round: it kept a refusal SC-23a had reversed and it missed `DropCarry`, `ChannelGap`, SC-10a and SC-20a. The exit review reads the per-spec tables, plus this document's own four: `schema_freeze`, `kernel_surface`, the canonical-JSON vectors of §7 including the integer-profile values, and the SHA-256 known answers.

## 10. Traceability

### Audit §14.1 items → spec, code, tests

| # | Item | Spec | Code | Tests |
|---|---|---|---|---|
| 1 | Three tiers and the freeze unit | 00 §5, 05 MA-1…4 | one crate, the allow-list | `kernel_surface` |
| 2 | TimePoint representation, epoch, TimeAuthority, step-driven simulation | 01 TM-*, 05 MA-15/20/29/30 | `time`, `module_api::Authority` | `time_model`, `module_api` stepping (the Engine itself is Phase 2) |
| 3 | Stream Contract | 02 SC-* | `stream`, `contract` | `stream_contract` |
| 4 | TimingEnvelope, coercion, late policy, fidelity vector | 03 SB-*, 02 SC-27, 05 MA-11/12/42 | `binding`, `plan`, `module_api` | `spec_binding`, `module_api` (envelope *contents* are Phase 2) |
| 5 | Session and Lease | 04 RS-* | `session`, `run` | `run_session` |
| 6 | Composite resource tree, provider-declared coherence, arm-order DAG | 03 SB-*, 05 MA-10/16 | `binding`, `plan` | `spec_binding` (the coherence basis is Vocabulary, Phase 2) |
| 7 | BindingProfile = bindings + placements + environment | 03 SB-*, 05 MA-38/41 | `binding` | `spec_binding`, `module_api` |
| 8 | Generic matcher and PrepareReport | 03 SB-*, 05 MA-11/12/34 | `binding` | `spec_binding`, `module_api` |
| 9 | DataContract registry and Port | 02 SC-1…5, 05 MA-36 | `contract` | `stream_contract` |
| 10 | Schema-first and versioning | 00 §6, 03 SB-* | `schema`, all documents | `schema_freeze`, `spec_binding` |
| 11 | Descriptor versus ABI, cycle rule, two deadline kinds | 05 MA-19…22/36/37, 01 TM-15 | `module_api`, `time` | `module_api` |
| 12 | Event counters, EVENTS_DROPPED, closed Policy | 04 RS-* | `event`, `policy` | `run_session` |
| 13 | Manifest envelope, namespaced sections, hashes | 04 RS-*, 00 §7 | `manifest`, `hash` | `run_session`, hash vectors |
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
| X1–X12 | A/B/C | | |
| Per-spec decision tables | A/B/C | | |

Open questions the owner must settle at a gate:

| # | Question | Recommendation |
|---|---|---|
| 1 | The fidelity vector has no value for a Hardware Run; Vision §14's sets stop at `hardware_quirk`. | Add `real` to every aspect's value set (spec 05). The alternative, omitting the vector on Hardware Runs, contradicts §14's "every Run". |
| 2 | The Kernel Action set (§5, §19) has no RX stream command, so a fixed Spec cannot say "start RX at time T" as an Action. | Express it as `Provider::start(at)` plus a recorder parameter, and do not add a Kernel Action. Confirm this reading, or accept an Action-set addition. |
| 3 | Vision §32 says the Simulation Engine step-drives every Island; the plan puts the stepping loop in the Kernel coordinator with the Authority deciding *when*. | Confirm that reading (the Engine decides the next wake-up, the Kernel calls `step` in a fixed order so determinism does not depend on registration order). |
| 4 | A recording TestExecutor is a fifth double beyond "test-double Provider and in-memory link". | Allow it: twenty lines, and without it the stepping loop has no test until Phase 2. |
| 5 | `schemars` pinned in `Cargo.lock`, not by an exact `=` requirement. | Accept: an exact requirement in a published library cannot unify with a downstream `^1.3`, and the lock plus `schema_freeze` already give reproducibility. Upgrades are deliberate changes that regenerate the schemas. |
| 6 | MSRV 1.85 and crate version `4.0.0-alpha.N` until the freeze. | Accept. |
| 7 | TM-16a1 settles that the Authority drives `host.monotonic` as virtual time in the Simulation class only, and reads the real clock in the other three. This borders on a Phase 2 question about the Simulation Engine. | Settle it now. It changes no type, but `RelativeBudget`'s meaning depends on it and TM-15 has already fixed that domain, so leaving it open would leave deadline semantics undefined in RealtimeEmulation — the one class that exists to expose real deadlines. |

---

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
2. Every normative obligation of every covered Vision section has a rule ID and no coverage table contains a gap, and every rule has an OV-3 disposition: the review reports how many rules are producer, consumer or forward obligations and which phase each names.
3. `cargo test` passes on the pinned MSRV and on stable, with no `#[ignore]` in the inventory, and the whole compile pipeline runs end to end against the test doubles with no Mock.
4. `schemas/` is committed, `schema_freeze` passes, and `SCHEMA_CHANGELOG.md` has its v1 entry.
5. `kernel_surface` passes: every public item is on the allow-list with a specific audit §13 token or a `NEW:` justification, its doc comment cites a rule ID, and no token from `tests/banned_tokens.txt` appears outside a comment citing the ban. The review records the `NEW:` count as the Kernel-growth number.
6. The Kernel crate's **direct** dependencies are exactly the four crates of §8. The resolved tree is larger (`sha2` brings `digest`, `block-buffer`, `crypto-common`, `hybrid-array`, `typenum`; `serde_json` brings `serde_core`, `itoa`, `memchr`, `ryu`; `schemars` brings `dyn-clone` and `ref-cast`), and that is expected.
7. After the move in §12, every link in `design/` and `plan/phase1/` resolves.
