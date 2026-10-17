# Orchestrator review: Phase 8 stage 1b-1 (Opus)

## O1 (minor, simplification): `key_decl`'s two passes are unobservable
`crates/ezsdr-kernel/src/module_api/registry.rs:276-287`. Registration refuses every overlap, so at most one declaration declares a concrete key, and `declares` of an exact declaration is `d.key == key`. The exact-first order cannot be observed (the report's P23 `equivalent`). Fix: one pass, `.find_map(|v| v.keys.iter().find(|d| d.declares(key)))`; drop SB-2a's sentence "`key_decl` consults the exact declarations before the numbered ones" and the doc comment's "every exact declaration is consulted first"; delete row P23. Design 33 D2's "完全一致の次に" is satisfied trivially (record it in the §8 log, orchestrator).

## O2 (minor): a declaration outside its Vocabulary's prefix is not refused (report open item 1)
`registry.rs` `check_key_declarations`. MA-34 already says every key a Vocabulary declares is `<prefix><path>`, and SB-2a's "position past the prefix's segments" check is only sound when the key starts with the prefix. Fix: refuse at registration any `KeyDecl` (exact or numbered) whose key is not `has_prefix(&v.prefix)`, naming the key and the prefix; check it before the position checks. Test in the registration-refusal test (an exact one and a numbered one); MA-34's `*Test:*` names it; one mutation row.

## O3 (minor, DA-1 "a topic is written once"): MA-32 restates SB-2a's refusals
`design/05-module-api.md` MA-32. Replace the restated list with a citation: "a `VocabularyDescriptor` whose declarations SB-2a (or MA-34) refuses".

## O4 (minor): mutation rows for the overlap branches
- No row for the `(false, true)` arm (`registry.rs`/`spec.rs` `overlaps`: the exact declaration is `self`, the numbered one `other`). Add one (`(false, true) => false`), killed by the cross-order case.
- P35 (`(false, false) => false`) disables almost every overlap (the prefix segment is never numbered), not the exact/exact branch. Replace it with a mutant specific to two exact declarations, e.g. `a.len() == b.len()` → `a.len() == b.len() && (pa.is_some() || pb.is_some())`, killed only by the two-equal-spellings case.

## Sound
Index grammar; `overlaps` (I tried crossed positions, prefix words `c`/`ch`, different lengths: correct); position/prefix/word checks; every `key_decl` caller (8, incl. `matching.rs:100`), no `decl.key` left in a refusal; `policies.coercion` still `get(&c.key)`; the test double's `coerce` refactor preserves `test.grid`; schema `#[serde(default)]` reads an old descriptor; J206/J226/J242 updated mechanically.
