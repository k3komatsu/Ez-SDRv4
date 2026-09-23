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
| [04-run-and-session.md](04-run-and-session.md) | `04-run-and-session.md` (`RS-n`) | 59 |
| [05-module-api.md](05-module-api.md) | `05-module-api.md` (`MA-n`) | 51 |

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

A `GAP` or an `UNCERTAIN` assertion is a finding for the exit review, not a defect to fix
in passing: the table's job is to make the coverage visible rule by rule.

## Result (2026-09-23, after D69–D94)

**267 rows for 267 rules — every rule of all six documents has a disposition.** The six new lettered
rules split Run/coordinator, Executor/Link and Link-implementation responsibilities by phase
(D56–D59, D74).

| Disposition | Count |
|---|---|
| `default` (a Kernel item plus at least one Phase 1 test) | 215 |
| `process` (a document, `Cargo.toml` or the test tree) | 20 |
| `producer` (the Module author's obligation) | 13 |
| `forward` (a later phase) | 10 |
| `withdrawn` (OV-1 keeps the number) | 7 |
| `consumer` | 1 |
| `default` derivation + `producer` flag-setting (SC-31a) | 1 |
| **`GAP`** (nothing carries the obligation) | **0** |
| `UNCERTAIN` as the disposition itself | 0 |

A further **37 assertion cells** carry an `UNCERTAIN:` note: the rule has a carrier, but
one or more clauses remain partial or are carried by a test the rule does not cite.

**No rule is left without a disposition or carrier.** The 37 `UNCERTAIN:` notes are the remaining
partial-coverage findings; they are not unassigned rules and remain visible for the acceptance review.

### Former `GAP` rules — resolved by D51–D68

The 15 baseline `GAP`s and OV-3's unassigned disposition have been resolved. The adopted carriers
are recorded in the rows above: schema and API gates for SC-1/SC-7/MA-16, migration provenance for
SB-49, compiled Policy and RunId assertions for RS-1, cleanup reachability for RS-11, and selected
Link descriptor registration/admission for MA-28/MA-39. Module-author obligations are marked
`producer`; coordinator work is marked `forward`; `RS-32a` is withdrawn and `OV-2`/`OV-3` are
`process` obligations. The summary table carries the full disposition counts.

### The `SPEC-DEFECT` rules — all resolved 2026-09-22

Six rules named a checker, test or artefact as carrying them that did **not** do what was
claimed: `TM-1`, `SC-31a`, `SB-44`, `RS-13`, `RS-46`, `MA-44`. (`TM-1` was recorded as
`UNCERTAIN` because `01` was written before this flag existed.) `SB-9a`, `RS-14` and
`MA-45` appear in a text search for the flag only because their cells discuss it; each
says in its own words that it is not one.

Five were **wrong citations**, corrected in the rule text: TM-1 named `kernel_surface`,
which has no field-type scan, where the carrier is `ov_22_schema_freeze` and the seven
committed time schemas; SC-31a named `SampleBlock::new`, which carries no `ALIGNMENT`
logic, where the derivation is checked by a test and **setting** the flag is the
producer's obligation; SB-44 named a test that calls `validate()` only and so cannot
check that a dry run and a real run agree; RS-46 named `rs_45_hash_equal_for_equal_inputs`,
which asserts RS-45's property; MA-44 listed "an event generator" the test double does not
have.

One was a **missing carrier** and got a test: RS-13's four control-op compilations —
`Release`, `Adopt`, `Renew`, `RunChild` — had no test anywhere, and
`rs_13_every_session_action_compiles_to_its_kernel_form` now asserts that each produces
its `ControlOp` and dispatches no Action, while `Stop{target}` is an Action instead.

### Method note

Every row was produced by reading **test bodies**, not by matching rule-ID prefixes: four
adversarial review passes over this crate each found a correctly prefixed test whose
expectation was the implementation rather than the rule, and this sweep found more. The
six tables were written by six subagents, one per document, and independently verified:
rule coverage is exact in all six (no missing, extra or duplicate rows), and every one of
the 360 test names the tables cite resolves to a real `#[test]` function.
