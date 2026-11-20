# #118 — Terra re-checks

## Re-check 1

**Verdict: FAIL.** The fix closes Sol’s specific missing-value example and the duplicate cases tested under `accept`, but other report omissions and duplicate-violation paths remain.

1. **Major — A previewed value can disappear from the effective configuration.** Request `19.5` under `accept`; validation previews `19.5 → 20`. At prepare, return the matching coercion but omit the key from `effective`. SB-44a accepts the coercion because it matches the preview, and MA-12 skips the absent effective entry. `collect_prepare` succeeds, so the configuration served to checks and the Run omits the value the Provider claimed to apply. Omitting both the coercion and effective entry also passes. Require the previewed key in the report and check the corresponding coercion obligation. `prepare.rs:154` · `prepare.rs:257` · `design/03-spec-and-binding.md:314`

2. **Major — Existing refusals still produce two violations for one key.** Leave a `reject`-policy key out of a Provider request and report a coercion for it. The stray check emits SB-44; the loop suppresses SB-44a but continues into policy handling and emits SB-46 for the same key. A wrong-kind coercion under `reject` similarly gets SB-6 and SB-46. Stop processing a coercion once its key has a preceding refusal, and test these combinations. `prepare.rs:136` · `prepare.rs:158` · `prepare.rs:174`

3. **Minor — SB-44a can defer to an MA-12 violation that never occurs.** If one fragment has an SB-6 kind error, the function returns before *any* MA-12 checks. A different fragment whose effective value differs from its preview has already had SB-44a suppressed on the assumption that MA-12 will report it. The stage is refused, but that second key gets no violation, contrary to SB-44a’s stated precedence. `prepare.rs:156` · `prepare.rs:188` · `design/03-spec-and-binding.md:325`

Sol’s findings 2 and 3 are closed for their stated sequences. Finding 1 is closed for its two stated `accept` examples, but not for the combinations above. Finding 4 is partly closed: the shared resource-and-key helper and two-resource test now cover its scope, and the test row identifies the policies. N118-1 still dies on the diagnostic’s source wording when the fallback finds the same expected value; it does not independently prove that the preview governs the decision.

The static trace found no false refusal of the in-tree MockRadio, UHD, capture Sink, or the replaying test Providers: the radio Providers return their applied configuration with the coercions, and the capture Sink returns no coercions. The recorded remote run reports N118-1 through N118-9 killed; its retained summary does not show the individual failing assertions, so I could not independently verify every kill’s assertion from that log. `mut-fix.log`

Verdict: FAIL
## Re-check 2

1. **Major — SB-6 can emit two violations for the same key.** A report gives a declared numeric key a string in both `effective` and a coercion’s `applied` value. The first kind check inserts the key into `refused`, but the second kind check ignores the insertion result and emits another SB-6 violation. The later guards cannot remove it. `prepare.rs:112`

2. **Major — MA-12’s narrowing refusal does not suppress its re-match refusal.** Request `test.count = 2` on a node that admits it, then report `effective = 7` without a coercion. Narrowing refuses the value but does not insert the key into `refused`; re-match then refuses the same key again. An existing test explicitly expects both violations, contrary to the new SB-44a text. `prepare.rs:230` · `prepare.rs:239` · `spec_binding.rs:2653`

3. **Major — Registered checks still see and can refuse an already refused key.** Validate a TX frequency inside the configured RF envelope. At prepare, report an out-of-band effective frequency and a coercion for that key with a false `requested` value. SB-44 refuses the key, but `checks.run` receives the unfiltered effective map; the RF-envelope check adds another violation for that frequency. This contradicts the stated rule that every later check skips the refused key. `prepare.rs:156` · `prepare.rs:195` · `lib.rs:425`

4. **Minor — Multiple stray keys lack a regression test.** The loop now reports each distinct stray key, but the existing prepare test supplies only one. I found no test or caller that assumes only the first of multiple stray keys; the single-key assertion remains valid for its fixture. A two-key test would distinguish the new loop from the former `find`. `prepare.rs:136` · `spec_binding.rs:3827`

The two earlier findings’ stated sequences are closed: stray or wrong-kind coercions no longer fall through to policy, and MA-12 runs for another fragment after an SB-6 refusal. The refusal set is scoped by fragment and key. Wrong-kind declared values are excluded from registered checks and MA-12; removing the early return only exposes MA-12 to report sets already refused for kind, so it cannot change an accepted run. The new cross-fragment test distinguishes that behavior from the old return.

The SB-44a specification promises broader deduplication than the implementation provides, as findings 1–3 show. Rows N118-10 through N118-14 target the described decisions. The logs report N118-10, 11, 13, and 14 killed in the first run; N118-12 survived there and was killed in the follow-up. The retained summaries do not show the individual failing assertions. No build or formatting was run.

**Verdict: FAIL**
## Re-check 3

**Major — J160 was deleted even though a list can reach re-match.** Declare a coercible key of kind `List`, omit that capability from the node, and request `Present`. The constraint has no scalar values to fail the kind check. The Provider’s `coerce` can return the key in `applied` as a list, with no coercion entry: admission accepts it because the key is present in `applied`. At prepare, report the list as `effective`. The kind check accepts it, narrowing skips it because the node has no such capability, and there is no preview, so re-match reaches `is_some_and`. Changing that to `is_none_or` would incorrectly accept the report. Restore J160 and retarget it to a test of this sequence. `matching.rs:75` · `matching.rs:140` · `prepare.rs:224` · `prepare.rs:285`

The other requested checks pass:

- Findings 1, 2 and 4 are closed. The SB-44a text now matches the unfiltered registered-check behavior, subject to the existing wrong-kind guard.
- The four changed tests still exercise their named behavior. The separate re-match test now uses a value inside the declared range, preserving coverage of that rule independently of narrowing.
- N118-15, N118-16 and the re-anchored N118-6 mutate their stated decisions; the supplied log reports each killed.

This was a read-only review; I did not build or format.

**Verdict: FAIL**