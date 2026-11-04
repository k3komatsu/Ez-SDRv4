(no output)
> plan · gpt-6.1-sol
# PLAN NEEDS CHANGES

Read-only review at clean HEAD `b07e032`. Independent scenarios were derived before reading the brief and plan.

References below: `TP` = `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s1b.md`; `R` = adjacent `rulings-s1b-plan.md`.

## Major

1. **Binding identity changes behavior, not merely hashes.** `TP:55`, `R:6`; `crates/ezsdr-kernel/src/coordinator/pipeline.rs:1114–1126`.
   - Sequence: connect a named Mock profile → run a child with the same physical selector but suffixes removed.
   - Wrong outcome: RS-25a rejects “binds an instance its parent does not”; the unchanged named child would pass. `binding_description()` participates in admission, grouping and SB-3 identity.
   - **Plan fix:** add this child-Run sequence and explicitly adjudicate identity semantics. Ruling 4 cannot justify this as a recording/hash exception. Do not introduce radio-specific stripping into Kernel incidentally.

2. **The selector-reader inventory misses a behavior-changing comparison.** `TP:39–48`; `crates/ezsdr-radio-uhd/tests/common/measurement.rs:2115–2120`.
   - Sequence: install `rx=["0:A:ubx:0#in"]`, `tx=["0:A:ubx:0#out"]` → run the measurement record.
   - Wrong outcome: `raw_probes_applicable()` rejects the strings; `run_raw_probes()` records eligible probes as inapplicable without failing the group.
   - **Plan fix:** add this reader; extend `fake.rs:8921` with named/plain UBX/OBX acceptance and named CBX/slot-B refusal, plus a mutation removing suffix stripping. This is not covered by T-H1 or the shell hand check.

3. **Client-side membership checks reject valid mixed representations.** `TP:102`; `python/ezsdr/session.py:787–791`.
   - Sequence: profile names c0 `loop-rx` → `connect(rx={0: Stream([0], gain={"loop-rx": 5})})`; also test `Stream(["loop-rx"], gain={0: 5})`.
   - Wrong outcome under the planned “name not in the stream” client refusal: Python rejects before the server can establish that both references identify c0.
   - **Plan fix:** defer membership decisions requiring name resolution to the server; retain client refusals only when provable locally. Add both positive sequences, true non-member refusals, and number/name duplicate-value refusals. Adjust P1345’s retargeting accordingly.

4. **“Non-numeric value key” is not an unambiguous wire grammar.** `TP:17–21`; `crates/ezsdr-server/src/initial.rs:127–130`.
   - Sequence: name c1 `+0`—legal under the name rule—then connect with `channels=["+0"]`, `gain={"+0": 5}`.
   - Wrong outcome: the existing `parse::<u32>()` recognizes `+0` as zero, causing refusal or incorrect numeric routing instead of selecting named c1.
   - **Plan fix:** define numeric keys by canonical ASCII decimal spelling, not parse success; resolve other legal spellings as names. Test `+0`, `-0`, full-width digits, leading-zero digit strings and overflow. Revisit the existing `protocol.rs:572` expectation explicitly.

## Minor

5. **P1362 is not killed by its named test as specified.** `TP:81,130`.
   - Sequence: mutate `split_once` to `rsplit_once`; parse `0:A:ubx:0#a#b`.
   - Wrong outcome: the test still passes because the malformed physical frontend now causes an error containing direction, position and original entry—the complete stated assertions.
   - **Plan fix:** remove this equivalent acceptance/refusal mutation, or specify and assert a distinct diagnostic/parser contract. Reuse P1334/P1335 rather than duplicating them as P1365/P1368.

6. **Cross-Provider full-tree equality is an invalid oracle; device-record equivalence is missing.** `TP:74–75,117`; `crates/ezsdr-acceptance/tests/x3xx.rs:3–4,36–57`.
   - Sequence: construct both Providers from the same layout.
   - Wrong outcome: full-tree equality fails on intentional Provider differences; conversely, suffix leakage into UHD’s recorded subdev spec can escape the proposed call-trace comparison.
   - **Plan fix:** compare full named/plain trees within each Provider, board-derived surfaces across Providers, and UHD’s recorded `device.subdev_spec` against actual calls. Extend `crates/ezsdr-radio-uhd/tests/fake.rs:6906`. Name the concrete parity test instead of `parity_...` at `TP:77`.

## Notes and coverage

- Rulings 1–3, 5–7 are sound. Ruling 8’s stripping is correct, but `run-record.sh:131` is an **RF attenuator-confirmation prompt**, not merely record labeling. Ruling 9 needs the value-key grammar clarified above.
- Before-Run placement is sound: assembly and expansion precede `coordinator::connect` at `crates/ezsdr-server/src/lib.rs:207–219`. Keep `Refused` plus `assert_no_run` assertions.
- Logical killer mapping is otherwise plausible for P1361, P1363–P1364, P1366–P1385 and retargeted P1334/P1344. P1362 and P1345 need the changes above; exact anchors and executable kills cannot be certified before implementation.

Independent searches run:
```sh
git grep -n -E 'selector|binding_description|provider_content|selector_id|layout_elements|front_end_matches|subdev_spec' -- crates python plan/phase8/tools ':!*.json' ':!*.md'
git grep -n -E '"0:[AB]:|\.selector|\["selector"\]' -- '*.rs' '*.py' '*.sh' ':!v3' ':!spike'
```
Also searched `crates/**/*.rs` for `split(':')`, `split_once(':')`, `rsplit`, `from_selector` and `ideal_from_selector`; Python production selector search found none. The inventory is **not complete** because of finding 2; identity consumers also need behavioral coverage.

No files changed; no tests, mutations or hardware operations run. Mutation conclusions are static plan review, not measured kills.
