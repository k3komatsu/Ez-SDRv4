# Stage 1 code review: dispositions (orchestrator)

Sources: Sol code review (`sol-s1-code-final.md`, NEEDS CHANGES, 1 P1 + 3 P2) and the orchestrator's own review (`orch-s1-code-review.md`, O-1…O-5).

| # | Source | Severity | Mechanism | Finding | Disposition |
|---|---|---|---|---|---|
| S1 | Sol | major | — (new lookup, not a listed mechanism) | `vocabulary_of` picks the first prefix match, not the Vocabulary whose declaration owns the key; nested prefixes declare the wrong major | adopted: `vocabulary_of` = the prefix-matching Vocabulary whose `keys` declare the key; `key_decl` reuses it (one ownership decision); regression for nested and shared prefixes |
| S2 | Sol | minor | — | `radio=""` (or any falsy) silently targets `"radio"` | adopted, merged with O-1: default only when `radio is None`; a non-str or empty `radio` is refused; `radio=` given with neither `rx` nor `tx` is refused (it would be silently ignored) |
| S3 | Sol | minor | — | raw-wire value keys `"0"`/`"00"`/`"+0"` alias one channel; later one silently wins | adopted: value keys must be canonical (`text == number.to_string()`); raw-server cases |
| S4 | Sol | minor | — | Python and Rust stream-name predicates differ (non-ASCII digits, whitespace set) | adopted: one rule in both, written in EA-10a: non-empty, not only ASCII digits `0-9`, no `:`, `#`, whitespace or control character (Rust `is_whitespace() \|\| is_control()`; Python `isspace()` or category `Cc` — the same set); shared cases (`"１２"`, `"\x1c"`, `"　"`, `"a\u0085"`) in both suites |
| O-2 | orch | note | — | the name rule exists in client and server | accepted as is: the server must check untrusted frames, the client refusal is the owner's ruling; S4 keeps them equal |
| O-3 | orch | note | — | a stream with no channels but a `sample_rate` is accepted | no change: harmless |
| O-4 | orch | note | — | stream names unique across radios, stricter than Q1 | no change |
| O-5 | orch / Sol | note | — | TX open-time two-zero burst | VERIFIED pre-existing (`provider/core.rs:1458-1468`, `:1571-1573`); stage 1 moves it to `connect` for Sessions that ask for TX; reported to the owner |

## Fix round 1 re-review (Sol, `sol-s1-fix-final.md`)

| # | Source | Severity | Mechanism | Finding | Disposition |
|---|---|---|---|---|---|
| S5 | Sol | major | — (pre-existing: Spec prefix check reads Vocabulary ids as prefixes) | shared-prefix initial key: ownership picks `alpha.beta@3`, then `check_key_prefixes` refuses `alpha.two` as not under `alpha.beta` | adopted at the root: `check_key_prefixes` takes the registry and requires the key's owner (`vocabulary_of`) to be declared (SB-2's text); regressions through `implicit_spec → validate` and a Spec Run case |

## Fix round 2 re-review (Sol, `sol-s1-fix2-final.md`)

PASS, no findings. S5 fixed (`check_key_prefixes` judges a declared key by its owner; a key no Vocabulary declares keeps the old prefix test and is refused by `check_keys`, message unchanged).

Count: Sol 2 major (S1, S5), 3 minor (S2–S4); orchestrator 1 minor (O-1, merged into S2), 4 notes. No listed mechanism of `plan/maintenance/failure-mechanisms.md` fits; S5's root cause was pre-existing (Spec prefix check read ids as prefixes).
