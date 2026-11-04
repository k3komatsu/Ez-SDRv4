# NEEDS CHANGES

## P1 — Shared-prefix ownership still fails through validation
**Locations:** `crates/ezsdr-kernel/src/session.rs:508–512`, `crates/ezsdr-kernel/src/spec.rs:1075–1089`; coverage gap: `crates/ezsdr-kernel/tests/run_session.rs:897–915`.

**Minimal sequence:**
1. Register `alpha@1`, prefix `alpha`, declaring `alpha.one`.
2. Register `alpha.beta@3`, also prefix `alpha`, declaring `alpha.two`.
3. Bind a Provider supporting `alpha.two`; connect with only `alpha.two = 1`.

**Wrong outcome:** Ownership correctly selects `alpha.beta@3`, but the implicit Spec then fails Validate with `UnknownKeyPrefix("alpha.two")`. `check_key_prefixes()` treats declared Vocabulary **IDs** as prefixes: `alpha.two` is not under `alpha.beta`. This existing validator assumption breaks the newly added initial-request path.

**Fix:** Validate against registered descriptor prefixes/ownership rather than assuming ID equals prefix. Extend the shared-prefix regression through `implicit_spec → validate`, requesting only the second Vocabulary’s key. The current test checks shared-prefix lookup but builds an implicit Spec only for nested prefixes—and validates neither.

## Other checks
- `vocabulary_of` now decides ownership correctly; `key_decl` preserves its previous results. All eight production call sites retain their declaration, coercion and update-class behavior.
- Undeclared keys under registered prefixes and keys under no registered prefix still refuse with `UnknownKeyPrefix`. Spec Runs retain their existing `key_decl` checks.
- P1327 breaks the new ownership test. P22 breaks the separate indexed-declaration test; it does **not** break the new exact-key ownership test.
- S2–S4 are fixed: explicit invalid `radio` values refuse before connection; value-key aliases refuse; both stream-name predicates agree with EA-10a/EA-16.
- P1344, P1348 and P1353–P1359 have meaningful assertions and matching anchors by inspection. No P2/P3 findings.

**Checks run:** Six mocked Python connection tests passed; `git diff --check` passed.  
**Not rerun:** Rust builds/tests, mutation execution or hardware. No files changed.
