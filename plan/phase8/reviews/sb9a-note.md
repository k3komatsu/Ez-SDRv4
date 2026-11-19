# Design note: `mech:admission-per-origin` after #104 (AGENTS §6)

This is a draft for the orchestrator, written on 2026-11-19 by a Claude Opus subagent working read-only. The base is `main` 7b676bce, and every path:line below is `main`'s. **VERIFIED** means read or run; **INFERRED** means reasoned but not executed.

## 1. Answer

Kernel Actions and the Spec's key positions already pass through one point each; that is what closed #5, #25 and #27's shape half. What is still checked door by door is **SB-9a**, at 12 sites. It exists only because the canonicaliser refuses a non-ASCII key instead of ordering it as RFC 8785 does. Make the canonicaliser order such keys, and the rule, its 12 sites and the "the Run transmitted and then could not seal its Manifest" failure all go.

## 2. The four strikes

All four rows are VERIFIED.

| # | Skipped check, and where | What closed it | Pinned by |
|---|---|---|---|
| #5 | A Module update's class (RS-52), on the Module path | KC-24: one `admit_with` (`coordinator/admission.rs:30`) for arm `pipeline.rs:822`, agenda `stepping.rs:373`, Session `pipeline.rs:1020` and Module `pipeline.rs:1208`; step 5 is at `admission.rs:109-135` | `kc_24_a_module_update_must_state_its_providers_declared_class` (`tests/coordinator.rs:2317`) |
| #25 | KeyDecl owner and kind, in the `needs[].requires` traversal | One iterator, `keyed_entries` (`spec.rs:1154`; `needs` at :1159), read by both checks (`spec.rs:1077`, `plan/validation.rs:382`) | `sb_02_needs_constraints_are_checked_against_key_decl` (`tests/spec_binding.rs:4457`) |
| #27 | `Value` shape and ASCII keys, on the Module path | Shape: the type (spec 26, `spec.rs:146-205`). ASCII: inside the one function, KC-24 step 4 (`admission.rs:62-105`) | `sb_09a_module_action_rejects_a_non_ascii_map_key_before_dispatch` (`tests/coordinator.rs:4211`) |
| #104 | ASCII keys, in Provider reports and registered checks | One more check per door (272bc6ff; rows 4–7 below) | Six `sb_9a_*` tests (`tests/coordinator.rs:362, 392`, `run_session.rs:1001`, `spec_binding.rs:3028, 3050, 3103`) |

## 3. Inventory

`assemble_manifest` (`coordinator/ending.rs:49-174`) lists everything that reaches the Manifest. Its only failure is `seal()` (`manifest.rs:388`; the branch at `ending.rs:169-171`). The only content refusal inside `seal()` is the ASCII assertion at `hash.rs:169-171`.

SB-9 (`deny_unknown_fields`) belongs to each type and runs at every parse, so it is not repeated per door. Rows are VERIFIED unless marked.

| # | Input | Enters at | SB-9a | Other content checks | Skips a check another door runs? |
|---|---|---|---|---|---|
| 1 | Spec (`start_spec_run`, `run_child`) | `spec.rs:1061-1067`; hashed at entry `coordinator/mod.rs:243-245` | `spec.rs:1065` | Version, SB-9 top level, SB-13; key owner and kind via `keyed_entries` (`validation.rs:382-417`) | No |
| 2 | BindingProfile; also parsed by the server (`ezsdr-server/src/lib.rs:251`, `catalogue.rs:91`) | `binding.rs:124-128` | `binding.rs:127` | Version, SB-21 | No |
| 3 | Session entry (`Request::Submit`, `protocol.rs:133`) | `pipeline.rs:954` | `check_entry` (`session.rs:172-177`) → `check_values` (:131-138), covering `SetParameter` and `Vocabulary`, the only variants with a `Value` (`session.rs:35`); again at `session.rs:362`; again at KC-24 step 4 | KC-28, KC-24 | No; checked three times |
| 4 | Kernel Action, every origin | `admission.rs:30` | Step 4 (`admission.rs:62-105`): `UpdateParameter`, `TxBurst.metadata`, `Command.params`, `Emit.payload` and the class-pending update, i.e. every variant that carries a `Value` or JSON (`event.rs:700`) | Steps 5–13 | No |
| 5 | `CoerceReport` at validate | `plan/matching.rs:105` | `matching.rs:127-163` | SB-7 (106-125), SB-44 (137-155), SB-46 (`validation.rs:110-123`) | **Kind of `applied`: no check** (INFERRED, §5) |
| 6 | `CoerceReport` at runtime (KC-24 step 10) | `admission.rs:213-280` | `session.rs:249-261` | Kind at step 11 (`admission.rs:270-278`); SB-46 | No |
| 7 | `PrepareReport` (Provider, Executor, Sink) | `plan/prepare.rs:58` | `prepare.rs:62-75` | SB-41, SB-44, SB-46, registered checks; MA-12 narrowing (170-180) and re-match (182-237) | **Kind of an `effective` value neither covers: no check** (INFERRED) |
| 8 | Registered check's `Violation.requested` | `binding.rs:360-370` | Recorded as absent | — | No |
| 9 | Control event (`emit_control`) | `event.rs:671` | `event.rs:672` | Source bound to the instance's root (`event.rs:649-658`); no size bound | No |
| 10 | Provider section | `ending.rs:53-71` → `manifest.rs:412` | `manifest.rs:420`, at cleanup; a refusal costs the section only | Own namespace (`manifest.rs:413`, RS-39) | No |
| 11 | Manifest read back | `manifest.rs:380-384` | `manifest.rs:382` | Version | No |
| — | No free object key: implicit Session Spec (`InitialRequests` is `Ident → Key → Scalar`, `session.rs:450`; hashed at `coordinator/mod.rs:263`); ControlOps; hot-path events (bytes, written as a number array, `event.rs:510`; at most 32 bytes, `event.rs:576`); `plan` (built from rows 1–2 only, `plan/compile.rs:72-105`; `DataLinkDecl` holds only a contract id, `stream/link.rs:37`); `instance()` (typed tree, `Scalar` capabilities); `ArtifactRef`, clock records, `StepOutcome`; `ModuleError` (`.detail` is never read: `git grep -n '\.detail\b' main -- crates/ezsdr-kernel/src` finds nothing); Module-registry descriptors (`ParamDecl.schema`/`default`, `module_api.rs:319`), which `assemble_manifest` does not copy (only ids, `impl_hash` and profile, `ending.rs:89-117`) | | — | Typed | — |

The only size bounds are the server's 16 MiB header (`ezsdr-server/src/protocol.rs:18`; the body is unbounded, `transport.rs:119`) and the 32-byte hot-path payload.

## 4. Q2: can the concept be removed? Yes

Evidence:
- **The shortcut.** `hash.rs:24-27` says "All Ez-SDR keys are ASCII by the key-namespace rule … and the canonicaliser asserts that rather than implementing UTF-16 collation"; `plan/phase1/00-overview.md:171` says the same.
- **Its premise is false.** `Value` map keys have "no other grammar" (SB-4, `design/03:152`), and opaque sections are free JSON (SB-9, `03:169`).
- **SB-9a is the compensation.** Its first text (f1b7bc02, 2026-09-22) gives the reason: "OV-15's canonicaliser refuses such a key, because RFC 8785 orders keys by their UTF-16 code units and the integer profile does not implement that ordering". VERIFIED.
- **What RFC 8785 requires.** §3.2.3 sorts property names as arrays of UTF-16 code units (rfc-editor.org, read through a fetch summary).
- **A scratch run.** `encode_utf16().cmp` reproduces the RFC's §3.2.3 example order. Byte order does not: it puts U+FB33 before U+1F600. For ASCII keys the two orders are equal, so **no existing hash changes**. VERIFIED by running it; the ASCII case also follows because each ASCII character is one UTF-16 unit equal to its byte.

**The change.** `hash.rs:169-172` becomes one `sort_unstable_by` over `encode_utf16()`. String escaping already matches §3.2.2.2 (`hash.rs:188-192`).

**Why `seal()` can then never refuse.** A `serde_json::Number` cannot hold a non-finite value, and `1e400` does not parse. This was checked with serde_json 1.0.151; `cargo tree -e features -i serde_json` shows only `default`, `std` and `alloc`, so `arbitrary_precision` is off. With keys accepted, `seal()` has nothing left to refuse, so KC-44's failure branch becomes defensive only.

**What this rules out.** Later code can never rely on ASCII object keys. Doing this before the freeze costs no versioning.

Alternatives, with their cost in code, OV-23 surface and schemas:
- **A. Total canonicaliser, withdraw SB-9a (recommended).**
  - Code: about 5 lines in `hash.rs`, and the 12 sites are deleted.
  - Tests: 14 exist only for SB-9a, three more use it as their example (`kc_28_a_malformed_action_takes_no_sequence_number`, `ea_05_errors`, `kc_24_the_target_is_resolved_before_the_payload_is_read`), and the three doubles at `tests/support/doubles.rs:171, 464, 471` go.
  - Mutation rows: 13 found by a content scan of `mutations.json` go (J230, J231, J237, J243, J250, J251, J256, J259, N105-1…5); one is added.
  - Surface: no module-level public item goes (methods and variants are not OV-23 items, and the JSON walker at `spec.rs:983` is `pub(crate)`), so OV-23b's 113 of 293 stands. `WITHDRAWN` (`tests/kernel_surface.rs:120`) gains SB-9a, with a literal assertion. `HashError::NonAsciiKey` and `RunError::{PayloadKeyNotAscii, SectionKeyNotAscii}` (`run.rs:176, 195`) go.
  - Schemas: the 16 Kernel schemas that embed `Value` change only in a description (`spec.rs:201`).
- **B. Hold SB-9a in the type** (`AsciiKey` map keys, plus a validated newtype for the 13 `serde_json::Value` fields). It keeps the rule and the shortcut. It adds two public types, a wrapper and an error at every Module payload and section, and it moves the Session refusal from `malformed` to a `protocol` decode error.
- **C. One `admit()` for every input** (the ledger's candidate). The inputs need different checks: KC-24's 13 steps, envelope plus serde, SB-7/SB-44/MA-12, root binding, namespace. One function would be a `match` over today's code that merges nothing, and no type forces a new path through it.
- **D. Keep the doors and add a governance test that poisons every Module output.** The positions are listed by hand, which is how #104 happened: n105's self-review missed two doors that Sol's inventory found (`plan/phase8/reviews/n105-sol-code.md`). The 12 sites still need upkeep.

## 5. Q3: rule implemented twice, text too complex

**Duplication.**
- SB-9a is checked 12 times: three times on the Session path (row 3) and twice on the Spec path (`from_json`, then the hash at entry).
- KC-24's clause "sets `requested` only from step 4 on" exists only for SB-9a (`design/06:304`; `admission.rs:296-299`).
- A child Spec with an unknown field is a logged `Rejected` entry (`child_refusal`, `pipeline.rs:1091`), but one with a non-ASCII key is the unlogged `malformed`, because its hash fails first (`pipeline.rs:1075-1088`).

**Texts that change under A** (no version bump or changelog entry):
- `design/03`: SB-4 :152, :58, :144, :261; SB-9a :181-182 becomes a withdrawn stub.
- `design/04`: RS-15 :172, RS-32 :216-217, RS-39 :298-299, `RunError` :99, row :417.
- `design/06`: KC-24 step 4 becomes a stub that keeps the step numbers SB-6 cites (:291, 304, 313, 328); KC-28 :348, :355; KC-44 :424; row :519.
- OV-15: `plan/phase1/00-overview.md:167, 171`.
- EA-5 (`design/16:38, 152`) only if D2.
- RS-11a stays, because it defers to KC-44. The Vision has no such text (`ascii|8785|canonical` in `design/vision/` finds nothing).

**Still open (INFERRED, not executed).** KC-24 step 11 checks a runtime coercion's kind (`admission.rs:270-278`). Nothing checks it at rows 5 and 7.
- The search: `git grep -n "kind_accepts\|check_value_kind" main -- crates/ezsdr-kernel/src` finds only `binding.rs:241, 253-254`, `admission.rs:144, 275` and `validation.rs:417`.
- What it would allow: a Provider bug's wrong-kind value would enter the configuration that registered checks read (KC-27).
- No rule text is broken, because SB-6 names only KC-24 steps 6 and 11.
- The report family is therefore **not closed**. Probe it with a test first. If the test fails, that is this mechanism's fifth bug, and `check_value_kind` (`binding.rs:253`) fixes it at those two doors.
- No single merged report judge is proposed, because the stages judge different requests: SB-7 the whole map, KC-26 one key, MA-12 the report.

## 6. Q4: outcome

The outcome is an amendment: a `plan/maintenance` decision record, with the `design/` texts above reviewed as the implementing diff.

The test that would have caught #104 is any Run-level test that a Provider reporting a non-ASCII key still leaves `manifest.hash.is_some()`. Under A, two tests pin this:
- The RFC 8785 §3.2.3 example in `tests/hashing.rs`, which replaces `ov_15_non_ascii_key_is_refused` (:98). It covers every door at once, because `seal()` is the only reader of key order.
- `sb_9a_a_provider_effective_map_key_refuses_the_run_at_prepare` (`tests/coordinator.rs:362`), rewritten so the Manifest seals.

The new mutation row is the byte-order sort, and the RFC example kills it.

## 7. Q5: what a user would see (owner)

Under A, these refusals disappear:
1. **A document with a non-ASCII object key**, in an `extensions` note, an environment section, a selector or a `Value` map. Today it is refused as `UnknownField { path: "<key>: OV-15 canonicalises only ASCII object keys" }` (`spec.rs:983-997`): EA-5 `refused` at `connect` and for a child profile, `malformed` for a child Spec. After A it is accepted.
2. **`Session.set` or `submit` with such a map value.** Today it is `malformed` with no sequence number (`ea_05_errors`, `ezsdr-server/tests/protocol.rs:281-282`). After A, KC-24 judges it: a Provider key still refuses a map, as a logged `Rejected` entry (step 6 or step 8), and a component or Vocabulary verb may take it.
3. **Module-made content.** Today it is refused or dropped (rows 4–10). After A it is recorded and the Manifest seals. No in-tree Module writes such a key: `git grep -P '"[^"\x00-\x7F]*[^\x00-\x7F][^"]*"\s*:' main -- 'crates/*/src'` finds nothing. Keys computed at runtime were not searched.
4. **A stored Manifest with such a key** is read instead of refused.
5. **EA-5's `malformed` kind has no producer left.** All seven sites die:
   - `pipeline.rs:954` and `:1090` call `check_entry`, which goes.
   - `:948`, `:1053` and `:1095` are `append`, which fails only through `check_entry`.
   - `:1075` is a hash that can no longer fail.
   - `:1103` runs `start_spec` after `child_refusal` (`:1091`) has parsed both documents, and its hash can no longer fail.

Channel and stream names are not affected: "a name reaches no key, node or device call" (RM-28, `design/07:305`); names live in selector strings and server tables (`catalogue.rs:61-69`). EA-16 and KNOWN_LIMITATIONS say nothing about ASCII keys.

## 8. Closing

**Recommendation.**
1. Make OV-15's canonicaliser total. It sorts object keys by UTF-16 code units, as RFC 8785 §3.2.3 does, which is one line in `hash.rs`, and no existing hash changes.
2. Withdraw SB-9a. Delete its 12 door checks, KC-24 step 4 and the seal-failure condition they guarded. KC-24 and `keyed_entries` remain the single points for Actions and Spec keys.
3. Probe the remaining candidate, the kind of a value reported at validate or prepare, with a test before calling it a bug.

**Rejected.**
- B, a typed `AsciiKey` or JSON newtype: it keeps the rule and the shortcut, and puts two public types and a wrapper on every Module payload.
- C, one `admit()` for every input: the checks differ by kind, so a dispatcher merges nothing and cannot force a new path through it.
- D, the doors plus a poison test: the positions are listed by hand, which is how #104 happened, and 12 sites of upkeep remain.

**Owner decisions.**
- **D1.** Accept non-ASCII object keys in documents and Session values, which withdraws SB-9a; this is recommended. The alternative keeps refusing them only at the user's doors (Spec and profile `from_json`, the Session entry). Under that alternative the Module-content doors still go, and `Manifest::from_json` (`manifest.rs:382`) stops refusing, so that a Manifest holding a Module's key can be read back. Either way the failure class is gone.
- **D2.** Remove EA-5's `malformed` kind and `RunHandleError::Malformed`, which have no trigger left; this is recommended. Removing it is a structural change to `schemas/server/reply_frame.v1.json:1015`. The alternative keeps the kind reserved.

**Staging.**
- §9 step 0 applies, because this changes a Kernel contract and refusals a user sees. This note can serve as its draft, and Fable reviews the direction after D1 and D2.
- The step is not risky under §9 step 1, so it needs no test plan.
- One step in two sub-steps, each leaving the tree green:
  - (a) The canonicaliser and its RFC test. This alone turns a child Spec's non-ASCII key from `malformed` into a logged `Rejected`, so it also waits for the owner.
  - (b) The withdrawal, per D1 and D2: the doors, texts, `WITHDRAWN`, schemas, tests and rows.
- Size: about 200 lines of code and comments and about 350 test lines removed, 13 mutation rows out and 1 in (INFERRED). No bench.
