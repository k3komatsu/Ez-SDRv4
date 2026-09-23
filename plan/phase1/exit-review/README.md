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
| [03-spec-and-binding.md](03-spec-and-binding.md) | `03-spec-and-binding.md` (`SB-n`) | 60 |
| [04-run-and-session.md](04-run-and-session.md) | `04-run-and-session.md` (`RS-n`) | 60 |
| [05-module-api.md](05-module-api.md) | `05-module-api.md` (`MA-n`) | 52 |

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

## Result (2026-09-23, after D108)

**277 rows for 277 rules — every rule of all six documents has a disposition, and no
assertion cell is `UNCERTAIN`.** D96 split SB-22 into SB-22 and SB-22a…SB-22h, one test
each; D100 added MA-16a (process) and D103 RS-25a (forward). The six lettered rules before
them split Run/coordinator, Executor/Link and Link-implementation responsibilities by phase
(D56–D59, D74).

| Disposition | Count |
|---|---|
| `default` (a Kernel item plus at least one Phase 1 test) | 207 |
| `process` (a document, `Cargo.toml` or the test tree) | 22 |
| `producer` (the Module author's obligation) | 13 |
| `forward` (a later phase) | 11 |
| `withdrawn` (OV-1 keeps the number) | 7 |
| `consumer` | 1 |
| `default` + `forward` (the Phase 1 half tested, a call site in a later phase) | 12 |
| `default` + `producer` (SC-31a, SB-23, MA-13) | 3 |
| `forward` + `producer` (MA-24) | 1 |
| **`GAP`** (nothing carries the obligation) | **0** |
| `UNCERTAIN` | **0** |

D108 resolved the 37 `UNCERTAIN:` notes the acceptance review had been left:

- **A test was added, or a case added to one**, for OV-1, TM-13a, TM-16c, SC-20, SC-20b, SB-17, SB-20, RS-3, RS-27, MA-2 and MA-38. TM-7's and TM-15's notes were stale: their tests already had the calls they said were missing.
- **The Phase 1 half is tested and a call site is later's**, marked in the rule text, for TM-13a, SC-20, SB-30, SB-41, RS-5, RS-10, RS-23, RS-25, MA-7, MA-9, MA-26 and MA-29. It is a producer obligation for SB-23 and MA-13, and both kinds for MA-24.
- **The type is the carrier** for SB-1, SB-12, SC-23 and MA-36. The allow-list is the carrier for OV-20 and SC-22.
- **The rule text was corrected** for SB-8 and OV-21 (the double's fourth key, `test.gain`, and the matcher scope) and OV-17 (a per-section hash no field stores). The carriers were already named elsewhere for OV-15a, RS-14 and RS-27.

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
