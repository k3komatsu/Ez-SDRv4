# Exit criterion 2 — the per-rule OV-3 disposition table

`00-overview.md` §13 criterion 2 requires, for **every** rule of the six Phase 1
documents, a recorded disposition: for a rule carrying an OV-3 marker, the marker and
the phase or artefact that carries it; for an unmarked rule, the test whose assertion
**is that rule's own obligation**, read from the **test body**.

A rule-ID prefix on a test name is OV-19's navigation convention and is neither
necessary nor sufficient here. Three review passes each found a correctly prefixed test
whose expectation was the implementation rather than the rule, and a fourth found a
rule covered only by a test named for its neighbour. The table is the evidence; the
prefix is not.

One file per document, so the rows stay beside the rules they are about.

| file | document | rules |
|---|---|---|
| [00-overview.md](00-overview.md) | `00-overview.md` (`OV-n`) | 27 |
| [01-time-model.md](01-time-model.md) | `01-time-model.md` (`TM-n`) | 31 |
| [02-stream-contract.md](02-stream-contract.md) | `02-stream-contract.md` (`SC-n`) | 47 |
| [03-spec-and-binding.md](03-spec-and-binding.md) | `03-spec-and-binding.md` (`SB-n`) | 52 |
| [04-run-and-session.md](04-run-and-session.md) | `04-run-and-session.md` (`RS-n`) | 57 |
| [05-module-api.md](05-module-api.md) | `05-module-api.md` (`MA-n`) | 47 |

## Row format

| Rule | Disposition | Carrier | What the carrier asserts |
|---|---|---|---|

- **Disposition** is one of `default`, `producer`, `consumer`, `forward`, `process`,
  `withdrawn`, or **`GAP`** when nothing carries the obligation.
- **Carrier** is a test function name for `default`, or the phase/artefact for a
  marker, or `—` for `withdrawn`.
- **What the carrier asserts** is read from the test **body** and states the
  obligation the assertions actually check, so a reader can compare it with the rule
  without opening the file. `UNCERTAIN: …` where the match is arguable.

A `GAP` or an `UNCERTAIN` row is a finding for the exit review, not a defect to fix in
passing: the table's job is to make the coverage visible rule by rule.

## Result (2026-09-22)

**261 rows for 261 rules — every rule of all six documents has a disposition.**

| Disposition | Count |
|---|---|
| `default` (a Kernel item plus at least one Phase 1 test) | 206 |
| `process` (a document, `Cargo.toml` or the test tree) | 18 |
| `producer` (the Module author's obligation) | 8 |
| `forward` (a later phase) | 6 |
| `withdrawn` (OV-1 keeps the number) | 6 |
| `consumer` | 1 |
| **`GAP`** (nothing carries the obligation) | **15** |
| `UNCERTAIN` as the disposition itself | 1 (OV-3) |

A further **39 assertion cells** carry an `UNCERTAIN:` note: the rule is carried, but
only in part, or by a test the rule does not cite.

**Criterion 2 is not yet met.** The table it asks for now exists and is complete, which
is what makes the rest visible: 15 rules have no carrier at all, and OV-3 — the rule that
defines this table — has no settled disposition of its own, because D38's `process`
marker was written for OV-4…OV-19 and excludes it.

### The 15 `GAP` rules

`OV-2` · `SC-1` · `SC-5` · `SC-6` · `SC-7` · `SB-49` · `RS-1` · `RS-11` · `RS-32a` ·
`MA-3` · `MA-8` · `MA-16` · `MA-19` · `MA-27` · `MA-28`

Two shapes dominate. **A type exists and nothing reads it**: `RS-32a`'s `hot_layout` is
`None` everywhere with no reader in `src/`, and `SB-49`'s `original_version` /
`original_hash` are never set to `Some`, which is the shape D40 withdrew
`constraints_hit` for. **The subject does not exist yet**: `MA-27`/`MA-28` are about Link
Modules and nothing implements `Link`; `SC-5`'s PerformanceEnvelope is Phase 2's. The
second shape is a candidate for a `forward` marker rather than a gap, but only the owner
may add one — the rules as written name Phase 1 behaviour.

### The 8 `SPEC-DEFECT` rules

`SC-31a` · `SB-9a` · `SB-44` · `RS-13` · `RS-14` · `RS-46` · `MA-44` · `MA-45`

Each names a checker, test or artefact as carrying it that does **not** do what is
claimed. `TM-1` is a ninth of the same shape, recorded as `UNCERTAIN` because `01` was
written before this flag existed: its text names `kernel_surface` as its checker and that
file has no float or field-type scan at all.

### Method note

Every row was produced by reading **test bodies**, not by matching rule-ID prefixes: four
adversarial review passes over this crate each found a correctly prefixed test whose
expectation was the implementation rather than the rule, and this sweep found more. The
six tables were written by six subagents, one per document, and independently verified:
rule coverage is exact in all six (no missing, extra or duplicate rows), and every one of
the 360 test names the tables cite resolves to a real `#[test]` function.
