Model: claude-fable-5-2 (Fable 5.2), adversarial design review, AGENTS.md §9 step 0.

# Review: withdraw SB-9a, make the canonicaliser total (D1, D2)

Base: `main` 7b676bce. Every `path:line` is `main`'s. **VERIFIED** = read or run; **INFERRED** = reasoned, not run.
Scratch runs used a `git archive main` copy outside the repository, built into `~/.cache/cargo-target/Ez-SDRv4-review` with `CARGO_INCREMENTAL=0`. Nothing in the repository was edited.

## 0. Verdict

**No major finding. D1 and D2 stand.** I found no concrete harm the owner has not weighed.
The direction is right: the canonicaliser's shortcut was the only reason for SB-9a, and removing it removes the failure class instead of guarding it at one more door.

What I ran (all VERIFIED):

| Check | Result |
|---|---|
| `hash.rs:169-172` replaced by `keys.sort_unstable_by(\|a, b\| a.encode_utf16().cmp(b.encode_utf16()))` | compiles; the RFC 8785 §3.2.3 example comes out in the RFC's order (`\r`, `1`, U+0080, `ö`, `€`, U+1F600, U+FB33) |
| Independent oracle: Python, keys sorted by `k.encode("utf-16-be")`, 21 keys (empty key, U+0000, U+007F, U+0080, NFC/NFD `é`, U+D7FF, U+E000, U+FFFF, U+10000, U+10FFFF, prefixes) | byte-identical canonical text and the same sha256; Python's code-point `sort_keys` differs, as expected |
| String escaping, U+0000–U+001F, U+007F, U+2028, U+2029, `/`, `"`, `\`, U+1F600, U+FEFF, in a key and in a value | RFC 8785 §3.2.2.2 form: `\b \t \n \f \r`, other controls `\u00xx` lowercase, everything else literal |
| Whole `ezsdr-kernel` suite, with the sort patched, the prepare door check (`prepare.rs:68`) disabled and a read-back hook after `seal()` | three tests fail, all expected: `ov_15_non_ascii_key_is_refused` (the sort) and the two `sb_9a_*_at_prepare` tests (the disabled door). Every other hash assertion passes, so no existing hash changes |
| A Provider reporting a non-ASCII `Value` map under an undeclared key, prepare door check disabled | the Run reaches `Running`, ends `Stopped { Client }`, and the Manifest seals |
| Every Manifest assembled by the suites of `ezsdr-kernel` (about 370) and of `ezsdr-mock-radio`, `-sim`, `-sim-engine`, `-sink`, `-sink-capture`, `-server`, `-exec-native`, `-acceptance` (208), serialised and read back with `Manifest::from_json` | all read back equal and none unsealed, except my own non-ASCII probe, which the still-present check at `manifest.rs:382` refuses. `ezsdr-radio-uhd` not run |

Hash stability holds for the reason the note gives (`hash.rs:172` and UTF-16 order agree on ASCII), and nothing else hashes a document: the only callers of `ContentHash::of` / `of_value` are `pipeline.rs:1080-1086`, `manifest.rs:161` and `manifest.rs:397`. Python hashes raw bytes only (`python/ezsdr/session.py:128`) and never canonicalises a document; `session.py:161` uses `sort_keys` only to compare typed clock-domain objects. Every `impl_hash` is `ContentHash::of_bytes` of a constant ASCII string (`ezsdr-mock-radio/src/lib.rs:59`, `ezsdr-radio-uhd/src/lib.rs:47`, `ezsdr-sink-capture/src/lib.rs:48` and four more), so key order cannot touch it. The literal hashes in tests are of raw bytes (`tests/hashing.rs:111-112, 128`) or of an ASCII document (one in `tests/spec_binding.rs`); `git grep -c "sha256:[0-9a-f]\{64\}" main -- crates python schemas` finds no fixture or schema that stores a document hash.

## 1. Attacks on the direction

### 1.1 "seal() can never refuse" — holds (VERIFIED)

- `serde_json::to_value(f64::NAN)` is `Ok(Null)` and `json!({"x": f64::INFINITY})` is `{"x": null}` (run). A `serde_json::Value` cannot hold a non-finite number, so `HashError::NonFiniteNumber` (`hash.rs:204, 212`) is unreachable from any `Value`.
- `1e400` does not parse (run).
- Every map key type in the Manifest tree is a string type (`Ident`, `Key`, `Namespace`, `EventKind`, `ContentHash`, `String`; the `BTreeMap` fields of `manifest.rs`, `module_api.rs`, `spec.rs`, `binding.rs`), so `to_value` has no "key must be a string" failure. No serialised type has an `i128`/`u128` field: the only such fields are `ExactConversion`'s (`time/point.rs:296-302`), which derives no `Serialize`, and `Rational` is two `u64`s (`time/rational.rs:17-22`). This matters because `to_value(u128::MAX)` is an error (run).
- After D1, KC-44's unsealed branch (`ending.rs:169-172`) is dead code. It has no test today either (`git grep -n "hash.is_none()" main -- crates` finds nothing).

**Note N1 (pre-existing, adjacent).** A Module that builds opaque JSON from a float (`json!({"snr_db": f64::NAN})` in a section or an `Emit` payload) records `null`, silently. OV-15 says "Non-finite numbers are rejected rather than written as `null`" (`plan/phase1/00-overview.md:167`); that holds only for the typed floats (`Finite`, `spec.rs:146-162`; `ClockRelation`, `tests/hashing.rs:48-86`). KC-44's "possible only for a non-finite number or a non-ASCII key" (`design/06:424`) therefore names two conditions of which, after D1, neither can occur. The rewritten KC-44 should say the branch is defensive, and OV-15's sentence should be scoped to the typed floats. Not caused by D1.

### 1.2 D2: every producer of `Malformed` really goes — holds (VERIFIED by reading)

`git grep -n "Malformed" main -- crates python schemas` minus `MalformedHash` and `TimeError::Malformed`: constructors only at `pipeline.rs:948, 954, 1053, 1075, 1090, 1095, 1103`; one consumer `ezsdr-server/src/lib.rs:496`; tests `tests/coordinator.rs:1388`, `ezsdr-server/tests/protocol.rs:282`.
- `:948, :1053, :1095` are `SessionLog::append`, which fails only through `check_entry` (`session.rs:162-174`), which is only `check_values` (`session.rs:131-138`), which is only the ASCII check.
- `:1075` maps a `HashError` of `of`/`of_value` on a JSON document: total after D1.
- `:1103` is `start_spec` (`coordinator/mod.rs:242-251`): two `from_json` calls on documents `child_refusal` has just parsed (`pipeline.rs:1120-1126`) and `sections` (`mod.rs:271-274`), a hash. Nothing else in it returns `Err`.

Python has no code or test that matches on `malformed` (`git grep -n -i malformed main -- python` is empty); `_raise` (`python/ezsdr/_client.py:40-46`) special-cases only `ended`, `protocol` and `unsupported_protocol`. EA-16 promises nothing about it.

**Minor m1.** After D2, `pipeline.rs:1103` has no error to map to. The lean form is to let `child_refusal` hand back the parsed Spec and profile and start the child from them: the child's documents are parsed three times today (`:1079/1084`, `:1120/1123`, `mod.rs:243-244`), and an `expect` there would put a panic on the control path of a parent Run that may be transmitting. Orchestrator's call.

**User-visible change the note states only in part (owner already decided D1; for the record).** `Session.set(target, key, {"é": 1})` is today `ServerError(kind="malformed")` with no log entry. After D1 it is a logged `Rejected` entry with a sequence number, which the Python client raises as `Rejected` (`session.py:114-118`), and the Manifest's `action_log` transcribes the value. `ea_05_errors` (`ezsdr-server/tests/protocol.rs:281-289`) asserts the opposite of both today.

### 1.3 RFC 8785 beyond key order

- **Unpaired surrogates.** A Rust `String` cannot hold one, and `serde_json` refuses the escape (run: `{"\ud800":1}` and `{"a":"\ud800"}` both fail to parse). So the canonicaliser never sees one. See m2 for what the refusal costs.
- **Surrogate pairs.** `{"😀":1}` parses to U+1F600 and sorts as two code units (run).
- **Normalisation.** `"é"` (U+00E9) and `"e"` followed by U+0301 are two keys with two hashes (run). RFC 8785 requires exactly that. **Note N2:** SB-4's new text should say keys are compared as code-point sequences with no normalisation, so that nobody adds NFC "helpfully" later and changes hashes. Text typed on macOS or pasted from a file name can be NFD.
- **Duplicate keys after decoding.** `{"é":1,"é":2}` becomes `{"é":2}`, last one wins, silently (run). The same already happens for `{"a":1,"a":2}` (run), and for any `BTreeMap` field such as two `resources` entries of one name. RFC 8785 assumes I-JSON input, which forbids duplicate names. **Note N3:** pre-existing, independent of D1; the hash is of the parsed document, so it stays self-consistent.

**Minor m2 (pre-existing, but D1 makes "any Unicode key" a promise).** Sequence: a Python caller passes a `str` holding a lone surrogate in any position of a request, e.g. `session.set("radio", key, {"\ud800": 1})` or as a plain string value. `json.dumps` (default `ensure_ascii=True`, `python/ezsdr/_client.py:79-81`) writes `"\ud800"`. The server's header parse fails (`ezsdr-server/src/transport.rs:100-106`), which is a framing error: the server replies `protocol` and **exits**, ending the Session and whatever it was transmitting (EA-5, `design/16:38`). Each part VERIFIED by reading or running; the end-to-end sequence is INFERRED (no server run). Lean fix on the client: build the header with `ensure_ascii=False` and encode it before the `try` at `_client.py:85`, so the caller gets a `UnicodeEncodeError` and the Session lives. Worth an issue; not part of this step.

### 1.4 Python client and server round trip (VERIFIED by reading)

- Request: `json.dumps` escapes non-ASCII as `\uXXXX` pairs, which `serde_json` decodes. Frame lengths are byte counts of the body only (`_client.py:80, 87`).
- Reply: `serde_json::to_vec` plus `\n` (`transport.rs:154`); Python reads a byte line and `json.loads` it (`_client.py:91-94`). A raw U+2028 or U+0085 in a key does not split a binary `readline`, and a `\n` in a key is escaped.
- Manifest file: written with `to_string_pretty` (`ezsdr-server/src/lib.rs:431`), read with `encoding="utf-8"` (`session.py:869-870`).
- SigMF: `sigmf_meta` builds only fixed keys (`ezsdr-sink-capture/src/sigmf.rs:18-95`); file names come from the Run id, sanitised (`ezsdr-sink-capture/src/lib.rs:314-316`). No user map key reaches a file name.

### 1.5 Identity, path or node name derived from a map key — none found

Free-key positions are exactly `Value::Map` (`spec.rs:202`, one level, scalars) and the `serde_json::Value` fields (extensions `spec.rs:639, 806`; environment `binding.rs:110`; sections `module_api.rs:516`; payloads). Selector keys are `Ident`s (`binding.rs:44`), `params` and `metadata` keys are `Key`s (`event.rs:721, 760`), so their grammar stays ASCII. The string-composed identities I found use selector *values* or fixed names (`ezsdr-mock-radio/src/lib.rs:2214, 2280`; `ezsdr-radio-uhd/src/provider/mode.rs:40`). The one identity built from a whole selector is `binding_description` (`plan/validation.rs:428-431`; used at `pipeline.rs:31, 1128-1129`, `validation.rs:252`): it is `serde_json::to_string` of the selector and the profile, which cannot fail for string keys, is injective, and is compared only inside one process, so its byte key order (not the canonical UTF-16 order) is harmless. Two selectors that differ only in a map key's normalisation form are two descriptions, hence two instances (SB-3); see N2. No byte-index slicing of a free key exists (`git grep -n -E "\.truncate\(|split_at\(|\[\.\.[a-z_0-9]+\]" main -- crates/ezsdr-kernel/src crates/ezsdr-server/src` finds only `id.rs:283`, a grammar-checked path, and byte buffers). Search of runtime-computed keys in the Provider crates was by these patterns only: INFERRED complete.

**Note N4.** An environment-section reader that is lenient about unknown members loses an accidental typo guard for non-ASCII look-alikes (a full-width or Cyrillic letter in a limit's name used to be refused at `from_json`). `start_lead_ns` is such a reader (`binding.rs:148-155`: a misspelled `start_lead_ns` means 0). An ASCII typo has always had the same effect, and the safety sections refuse unknown fields (RM-19, `design/07:405`), so this is no new class.

### 1.6 KC-24's "never transcribes the key" — nothing needs to replace it

The rule existed because the transcribed value itself reached the Manifest and made it unhashable (`session.rs:249-253`, `binding.rs:364-367`, `matching.rs:127-134`), not for terminal or log safety. ASCII never excluded control characters (ESC is ASCII), there was never a bound on key length, and where a reason is built with Rust's `{:?}` (the KC-12 prepare reason is, as my run's output shows) non-printable characters are escaped. What is new is only that a printable non-ASCII key appears verbatim in a reason, as string *values* already do. The clause at `design/06:304` and `admission.rs:296-299` can go with step 4.

## 2. Inventory

I found no door the note missed. `assemble_manifest`'s fields (`ending.rs:129-160`) are all covered by the note's rows or its "typed" list.

**The remaining per-door candidate, now VERIFIED by reading and partly by running.**
- For a key the bound node declares, a wrong-kind `effective` value **is** refused, by MA-12's narrowing, not by a kind check. Run: `test.flag` (declared `Bool`) reported as a `Str` ends the Run `Failed { Prepare }` with "MA-12: `effective` may narrow a declared capability and must not widen one" (`prepare.rs:170-180`).
- The hole is narrower than the note says: a key with a `KeyDecl` that is in **neither** the node's `capabilities` nor the resource's `requires`, and **every** Executor and Sink report, because the loop at `prepare.rs:154-160` runs only over `spec.resources` that have a Provider. At validate, `matching.rs:105-168` has no kind check on a `CoerceReport`'s `applied`; if the node declares the key, prepare's MA-12 catches it late, otherwise nothing does.
- I agree with "probe first". The probe's fixture must use a declared key the node does not list, or it will pass for the MA-12 reason.

## 3. Alternatives

I do not recommend a different direction. Two additions, both small:

**A1. Pin the property, not the doors: "what a Run writes, its own reader accepts".** #104 and the older D91 class (`plan/phase1/00-overview.md:1131`) are this one property. D1 removes one way to break it (the hash); the reader still refuses things a writer could in principle emit (name grammars, X7, `Value` shape, unknown fields). Checking, after `seal()`, that `Manifest::from_json(&to_value(&manifest))` succeeds and equals the Manifest makes every Run-level test of every crate a door test, with no hand-written list of positions (the weakness the note names for its alternative D).
- It holds today: 578 Manifests across nine crates' suites (§0).
- **Mechanism: not a `debug_assert!`.** Users are told to build the server with plain `cargo build` (`python/ezsdr/_client.py:54`), so a debug assertion would panic at cleanup on a bench Run that has transmitted, which is #104's outcome again. Either keep it test-side (the hook I used reads an environment variable that the test runs and `gate.py` would set), or make it always-on as a recorded step-7 `CleanupFailure` that leaves the Manifest as written. The second is user-visible, so it is the owner's; the first is the orchestrator's.

**A2 (optional).** The RFC example already separates UTF-16 order from code-point order (U+FB33 against U+1F600), so it kills both the byte-sort and the `chars()`-sort mutant and the note's one row is enough. If the test asserts the whole canonical string and adds the boundary pair U+FFFF / U+10000 and a prefix pair (`"a"`, `"aa"`), it also pins the comparator's edge cases; my 21-key vector and its Python oracle are in §0.

## 4. Omissions in the note's lists

**Tests (minor m3).** The note's rewrite of `sb_9a_a_provider_effective_map_key_refuses_the_run_at_prepare` "so the Manifest seals" does not test what it says if it keeps the fixture. Run: with the door check removed, `with_effective("test.flag", <map>)` ends `Failed { Prepare }` by "MA-12: a capability's `effective` value is a scalar", the report never enters `manifest.prepare.reports`, and `manifest.hash.is_some()` is true anyway. Use a key the node does not declare (`test.zeta` reached `Running` and sealed in my run), and assert that the key is in the Manifest and that the Manifest reads back.

**Mutation rows (minor m4).** The count of 13 is right (content scan of `mutations.json`, same 13 ids). Two of them pin behaviour that survives and should be retargeted, not deleted (AGENTS §9 maintainability 4):
- **J256** is anchored on the ASCII hunk but its killer is `kc_24_a_module_emit_is_dispatched_to_its_target`: it pins "an `Emit` is admitted and dispatched".
- **J237** pins "the target resolves before anything reads the payload"; after step 4 goes, the order left is step 3 before step 5, which `kc_24_the_target_is_resolved_before_the_update_class` can kill.
`hash.rs` has one row today (C90-1-07); the key sort has none, because removing `sort_unstable()` is an equivalent mutant while `serde_json::Map` is a `BTreeMap`. The new row is therefore the first that pins key order at all.

**Texts made false that the note does not list.**
- `design/06:191` — `Malformed { error: SpecError }` in the `RunHandleError` listing.
- `design/06:343` — KC-28 step 1's `Malformed` refusal; `:348` "`Err` is only step 1's three refusals" and "`check_entry` in step 1 guarantees the append cannot fail" (the note cites `:348` but for KC-28 in general).
- `design/06:403` — KC-37a step 1 "refuses with `Malformed` when a hash cannot be computed or `check_entry` fails".
- `design/04:172` — RS-15's whole clause "one whose value the canonicaliser cannot hash was never an Action … `check_entry` … `append` calls it first" goes, not only its SB-9a citation.
- The *Test:* lines that name removed or renamed tests and that the note does not cite: `design/04:173`, `design/06:362` (the note has `03:182`, `04:217`, `04:299`, `06:328`).
- `plan/phase1/00-overview.md:167` — OV-15 itself says only "sorted by key"; it should now state the order (UTF-16 code units), since the rule text, not only the implementation note at `:171`, is what a second implementation reads.
- `plan/maintenance/failure-mechanisms.md:18` — the mechanism's row must record this note's outcome (AGENTS §6).
- Rule citations in code: once `WITHDRAWN` (`tests/kernel_surface.rs:120`) holds SB-9a, a doc comment that cites it as its rule fails the citation check, e.g. `session.rs:127-130, 153-154`.

**Schemas.** The 16 files whose `Value` description says "its keys are ASCII (SB-9a)" are 13 Kernel schemas plus `schemas/radio/measurement.v1.json`, `schemas/server/request_frame.v1.json` and `schemas/server/reply_frame.v1.json`, so three crates' schema freezes regenerate, not one. `reply_frame.v1.json` changes twice (the description and the `malformed` constant at `:1015`).

**Confirmed absent.** The Vision, KNOWN_LIMITATIONS, README and the Python package say nothing about ASCII keys, JCS or canonical form (`git grep -n -i -E "ascii|8785|jcs|utf-16|canonicali" main -- design/vision Ez-SDR_v4_ARCHITECTURE_VISION.md KNOWN_LIMITATIONS.md README.md python/ezsdr` finds only `session.py:245`, a channel-number check).

## 5. Findings by severity

| # | Severity | Finding |
|---|---|---|
| — | major | none |
| m1 | minor | `pipeline.rs:1103` needs a non-panicking shape once `Malformed` is gone (§1.2) |
| m2 | minor | a lone surrogate in a Python request ends the Session; pre-existing, client-side fix (§1.3) |
| m3 | minor | the rewritten prepare test passes without the key entering the Manifest unless its fixture key changes (§4) |
| m4 | minor | J256 and J237 are retargets, not deletions (§4) |
| m5 | minor | texts missing from the note's list: `design/06:191, 343, 403`, RS-15's clause, OV-15's own sentence, the ledger row, three non-Kernel schema files (§4) |
| N1 | note | a Module's NaN is recorded as `null`; KC-44 and OV-15 wording (§1.1) |
| N2 | note | say "no normalisation" in SB-4 (§1.3) |
| N3 | note | duplicate keys after decoding: last wins, pre-existing (§1.3) |
| N4 | note | lenient section readers and look-alike keys (§1.5) |

## 6. Not checked

- `ezsdr-radio-uhd`, `ezsdr-radio`, `ezsdr-hostmem`, `ezsdr-link-host` and the Python suites with the patch (the other nine crates were run).
- m2 end to end against a running server.
- Runtime-computed map keys inside the UHD and Mock Providers, beyond the pattern searches named in §1.5.
- The wrong-kind probe for an Executor or Sink report and for a declared key outside the node's capabilities (read only, §2).

## 7. Cost

The run's token cost is not visible to me from inside the session. Work done: about 40 tool calls, five scratch builds (the kernel suite and eight other crates' suites, under a minute of tests).
