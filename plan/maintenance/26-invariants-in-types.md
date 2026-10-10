# Maintenance spec 26 — Invariants in the types (pre-freeze item 1)

| Field | Value |
|---|---|
| Status | **Accepted** by the owner on 2026-10-09 ("いいです．これで進めてください"), including the own-domain test of §2. Written by the orchestrator (Claude Opus) on 2026-10-09. It implements the owner's decision on audit item 1 ([24-prefreeze-audit.md](24-prefreeze-audit.md), Owner decisions, "item 1"; findings F1, F2, F3, F4, F5, F10, F32) and opens with the design note AGENTS.md §6 asks for. Implemented: stage 1a in `8664865`, stage 1b on 2026-10-10 (which also removed the then-unused `hash::serialize_finite_f64`). |
| Mechanism | `invariant-outside-type`, six bugs: #2, #7, #11, #22, #23, #24 ([failure-mechanisms.md](failure-mechanisms.md)). The audit found seven more places of the same shape. |
| Stages | **1a** ids and time, then **1b** `Value`. Each stage is one implementation step: implement, independent review, mutation gate, commit. The tree is green after each. |
| Versions | None bumped, no SCHEMA_CHANGELOG entry (AGENTS.md §6). 1b regenerates the schemas that embed `Value`. |

## 1. Design note (AGENTS.md §6)

**What keeps failing.** A rule of a Kernel value type is enforced at the places where someone remembered to check it, not by the type. The type also offers a way around the check: public fields, a derived `Deserialize`, or an unchecked constructor. Each bug was fixed by adding one more check where it bit:
- #7 and #24: a negative uncertainty in a `ClockRelation` was refused inside `convert`.
- #11: a budget in the wrong domain was refused in `RelativeBudget::new`, but its sign is still checked in MA-37.
- #27 was the same shape on the Module Action path (`check_nesting` missing). It is recorded under `admission-per-origin`.

The audit found the remaining holes:
- A Rust caller can build `NodeId(7)` or `ResourceId { path: "a//b" }`. That is why `check_rid` and `check_local_ids` exist and are repeated, per D91 and D106.
- `TimePoint.ticks` can be read without its domain, which is what #31 did.
- A `ClockRelation` with a negative uncertainty can be sealed into a Manifest.
- A nested `Value` passes a schedule entry, a `ParamDecl.default`, a `CapabilityValue` and a selector.

**Can the concept be removed or merged?**
- **The checks: removed.** Once the type holds the rule, every check at the places it is used goes away. That is most of this spec's deletions.
- **`NodeId`: kept.** It is always `LOCAL` in v4.0. Removing it would change every id in every schema and undo X7, which Vision §49 asks for so that multi-host needs no new schema. A private field gives the same safety for one line.
- **Two scalar types: merged.** `Value`'s scalar variants and `contract::Scalar` carry the same four kinds with two copies of one equality (SB-6). 1b keeps one.
- **`Validity`: kept** as a field of `ClockRelation`, checked by `ClockRelation::new`. It is never used alone.

**Is a rule implemented twice?** Yes:
- X7 and SB-1: once in the deserialisers and again in `check_rid`, `check_local_ids`, `check_ids` and two `register` paths.
- SB-4: in `check_nesting` at five call sites, and missing at others.
- TM-14: inside `convert` only.
- SB-6's equality: in both `Value` and `Scalar`.

**Is the spec text too complex?** Partly. SB-1, SB-22f, RS-15 and X7 carry clauses about "a value handed in as a Rust value". They exist only because the types allow such values. They are deleted, not reworded. SB-4 and SB-5 become "the shape is the type".

**Outcome.** Rejected alternative 1 is how D91 and D106 were fixed. A next slot that holds an id or a value would need its own check again, and after the freeze the public fields could no longer be closed.

| | |
|---|---|
| **Adopted** | Close every type's way around its rule, then delete the checks that compensated. |
| **Rejected 1** | Add the missing checks where values are used. |
| **Rejected 2** | Add schema bounds only. The Rust path stays open. |

## 2. Stage 1a — ids and time

**The rules, by type:**

| Type | Construction | Reading |
|---|---|---|
| `NodeId` | The field becomes private. `NodeId::LOCAL` and the deserialiser are the only values. The `{node, local}` ids keep public fields, since their node is local by construction. | |
| `ResourceId` | The fields become private. `parse`, `child` and `parent` are the only constructors. | `node()`, `path()` |
| `TimePoint`, `Duration` | The fields become private. `new` stays (any domain and tick pair is a valid value). | `domain()`; `ticks_in(domain)` is the only way to read the integer, as TM-6 and T3 already say. Inside `time/` the fields are used directly. |
| `ClockRelation` | The fields become private. `ClockRelation::new(...) -> Result<_, TimeError>` checks all of TM-14's shape (below), and `Deserialize` goes through it, as `RelativeBudget`'s does. | getters |
| `UncertainTimePoint` | Built only inside the Kernel (`pub(crate)`). `convert` is its only producer. | public getters |
| `RelativeBudget` | `new` also refuses `ticks ≤ 0`. | |
| `SampleClockHandle` | Private fields. Only `declare_sample_clock` produces one. | getters |
| `ClockRegistry::register` | Refuses a domain whose `ended_at` is set. `end()` is the only way to end a domain. | |

TM-14's shape, checked by `ClockRelation::new`:
- `measured_at` and both `valid` bounds are in `source`, and `offset` is in `target`;
- `uncertainty` is in `target` and ≥ 0;
- `drift` is finite;
- `drift_uncertainty` is finite and ≥ 0;
- `valid.from ≤ valid.to` when `to` is present.

Any failure is reported with its cause, not as `Overflow`.

**No reading the domain from the value itself.** `x.ticks_in(x.domain())` would make TM-6 a ritual. Production code passes the domain it expects: the stream's SampleClock, the root, `host.monotonic`. One small test fails if this pattern appears in any crate's `src/` outside `time/`, apart from a line marked `// own domain: <why>`. #31 is exactly the bug this stops.

**Deleted:**
- `check_rid` and `check_local_ids`;
- the id branch of `check_ids` (session.rs);
- in `check_bindings`, the checks of the memory domain, `governs`, `arm_after` and the Provider tree;
- the node checks in `register` and `declare_sample_clock`;
- the checks in `convert` that `ClockRelation::new` now makes;
- MA-37's budget check;
- the tests that build forged ids.

**Spec text:**
- design/03: SB-1 and SB-22f, the Rust-value clauses deleted.
- design/04: RS-15, the same.
- plan/phase1/00-overview.md: X7, the same, plus a line that D91's rejected alternative is now adopted.
- design/01:
  - TM-14: `ClockRelation::new` is its shape check;
  - TM-15: "positive";
  - the T3 sketch without `pub` fields;
  - a sentence in TM-11 that `register` refuses an ended domain.
- design/05: MA-37 without the budget.
- Each spec touched gets a Changes row.
- `kernel_surface_allow.txt` is updated.

There is no schema change: the shapes are unchanged, only Rust's way in is closed.

**Tests**, one per rule that is not a compile error:
- `ClockRelation` deserialised with each malformed field is refused, naming the field;
- `RelativeBudget` of 0 ticks and of −1 tick is refused, by `new` and by deserialising;
- `register` of an ended domain is refused;
- the own-domain test above.

The compiler checks the privacy itself, so it needs no test.

**Mutation rows**, in `plan/maintenance/tools/mutations.json`: each condition of `ClockRelation::new`, the budget's sign, and `register`'s ended check.

**Size and risk.**
- About 90 raw `.ticks` reads in production code and about 300 in tests. The compiler finds every one. A test that knows its domain uses `ticks_in(d).unwrap()`.
- About 16 `ResourceId` literals, 38 `.path` reads and 12 `NodeId(n)` calls, almost all in tests.
- 7 `ClockRelation` literals, including the UHD Authority's builder.
- The test edits are mechanical and may go to a cheaper implementer. The production edits and the review stay with Opus (AGENTS.md §8).

## 3. Stage 1b — `Value`

**The shape:**
- `Value = Scalar(Scalar) | List(Vec<Scalar>) | Map(BTreeMap<String, Scalar>)`, untagged. Depth one is the type, and the JSON is unchanged for every valid document.
- `Scalar = Bool | Int(i64) | Num(Finite) | Str`. This is the one scalar type: `contract::Scalar` becomes it, `Float` is renamed `Num`, and SB-6's equality is defined once, on `Scalar`.
- `Finite` is an `f64` with a private field. `Finite::new(x) -> Option<Finite>`, and its `Deserialize` goes through `new`. A non-finite float therefore cannot exist inside a `Value`, and `serialize_finite_f64` stops being its guard.
- `Constraint` (`Eq`, `Range`, `Set`, `Min`, `Max`) and `CapabilityValue` hold `Scalar`. SB-5 becomes the type.
- Convenience constructors, so that call sites stay short:
  - `From<i64 | bool | &str | String>` for `Scalar` and `Value`;
  - `TryFrom<f64>` for both;
  - `Value::num(x) -> Option<Value>`.
- Pattern matches go through `Value::as_scalar()` and the `Scalar` variants.

**Deleted:**
- `check_nesting`, `is_scalar`, and the scalar half of `check_shapes`.

The ASCII rule on map keys remains: it is the one check the type does not hold (`check_ascii_keys`, SB-9a).

**Selector serialisation.** The `unwrap_or_default` in `binding_description` and the `unwrap_or(Null)` in `compile.rs` become `?`, so that a failure can no longer become an identity (F2 d). With `Finite`, they cannot fail on a float any more.

**Spec text:**
- SB-4 and SB-5: "the shape is the type".
- SB-6: equality defined on `Scalar`.
- SC-2: attributes are `Scalar`.
- Changes rows.

**Schemas.** `Value` stops being recursive in the 16 schemas that embed it. Regenerate them and keep `schema_freeze` green. The old recursive shape is refused, not reinterpreted (invariant 39). There are no users, so there is nothing to migrate.

**Tests:**
- a nested list or map is refused by every deserialiser that carries a `Value`: Spec (schedule entry included), Action, ParamDecl, BindingProfile, Manifest;
- `Finite::new` refuses NaN and both infinities;
- SB-6's equality tests run unchanged on `Scalar`;
- the Python client's parameter round trip stays green.

**Mutation rows:** `Finite::new`'s check, and the selector's `?`.

**Size and risk.**
- About 650 `Value::Int` and `Value::Num` uses, most of them test literals that become `Value::from(..)`.
- About 80 `Constraint::Eq` and 90 `CapabilityValue` uses.
- Fixtures that nest are refused and have to be rewritten. The implementer greps for them first (not yet checked).
- The Python package sends only scalars and flat maps (to be checked in 1b).

## 4. Not in this spec

- **RunId parsing:** optional in F1, and left out.
- **Typed identities:** item 2, spec 27.
- **The admission path:** item 3.
- **Schema bounds:** none are added, such as `minimum: 0` on `clock_relation`. The type is the rule, and a bound would be a second statement of it (`fact-stated-twice-in-spec`).
