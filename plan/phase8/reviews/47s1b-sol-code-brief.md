# Sol code review: note 47 stage 1b (channel names in the radio selector)

You are the independent reviewer (read-only). Repository `~/local_work/Ez-SDRv4`, HEAD `b07e032`; the change is the **uncommitted working tree** (`git diff`, plus untracked `crates/ezsdr-radio/tests/fixtures/names.json`). Read `AGENTS.md` §9. Do not edit, format, commit or run mutation tools; if you build, from a copy outside the tree with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`.

- Design: `plan/phase8/47-no-change-while-streaming.md` §2.4 bullet **名前**, §4 stage 1b.
- Brief `brief-s1b.md`, rulings `rulings-s1b-plan.md`, your plan review `sol-s1b-plan-final.md` and its dispositions `dispositions-s1b-plan.md`, the final plan `test-plan-s1b.md` (all in `~/.cache/ezsdr-fixes/2026-11-04-n46/`).
- Out of scope: stages 2–6; names in `start()`/`repeat()` beyond `rx_channels[name]`.

First write your own failure scenarios, then review:
1. **Correctness end to end**: selector `#name` parsing (`crates/ezsdr-radio/src/channels.rs` `split_entry`, `names_of`, `name_error`, `canonical_number`), cross-radio uniqueness (`crates/ezsdr-server/src/catalogue.rs` `channel_tables` in `assemble`), the server's resolution (`crates/ezsdr-server/src/initial.rs`: owner radio of name-only streams, mixed number/name, value keys, refusals before any Run), the `connected.channels` table, Python (`python/ezsdr/session.py`). Look for a name routed to the wrong radio/direction/number, a silently dropped or overwritten value, a refusal after a Run exists, an ambiguous spelling.
2. **A named selector behaves as the unnamed one** in every Provider path: search every reader of selector strings yourself (MockRadio, UHD Module layout, device record, `subdev_spec`, measurement helpers, acceptance helpers, `plan/phase8/tools/run-record.sh` — its attenuator prompt must still name the boards).
3. **Identity ruling**: a name is part of the binding; child Runs must repeat it (RS-25a). Is that consistently implemented and specified?
4. **Tests' power**: each sequence has its test observing what the plan says; rows P1361–P1386 and the retargeted P948, P1334, P1335, P1338, P1345, P1351, P1353–P1355 break what they claim; the shared `names.json` really drives both suites.
5. **Specs** RM-28, RS-25a, MR-2, UR-5, EA-3, EA-10, EA-10a, EA-16 and anything else the change makes false; accepted-spec format.
6. **Maintainability** (AGENTS.md §9): one name rule, one `#` reader, no third copy; leftovers from stage 1's "names need stage 1b" refusals.

Report in your final message: verdict (PASS / NEEDS CHANGES), findings P1/P2/P3 with path:line, a minimal operation sequence and the observable wrong outcome, a suggested fix. At most 100 lines.
