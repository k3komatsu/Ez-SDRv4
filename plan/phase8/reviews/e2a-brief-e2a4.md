# Brief — E2a-4 (#93): a late underflow report does not cut the next device burst

Repository /home/komatsu/local_work/Ez-SDRv4, branch `work/e2a`. Head: the E2a-3 commit (the orchestrator names the hash when launching). The step's start: `0ccb267`. The owner decided on 2026-11-11 that this fix goes into stage E2a. It is a small, behaviour-changing fix in the function E2a-2b changed for A5/F4.

Read first:
- issue #93 (`gh issue view 93 -R k3komatsu/Ez-SDRv4`);
- /home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/brief.md, whose rules bind;
- brief-e2a2b.md, the A5/F4 part;
- `git log -5`;
- `crates/ezsdr-radio-uhd/src/provider/tx.rs`: `report`'s underflow branch, `TxChain`, and `chain.burst_is_open()`;
- note 39 §6.6 (`plan/phase8/39-dsp-timed-commands.md`), which answers this mechanism with channel-and-tick correlation;
- bench-results.md "2026-11-05" and "2026-11-06" (read only). Check what the X300's underflow report tick is: the end of what was sent, or radio-current-time. Cite what you find and mark it VERIFIED or INFERRED. The rule below depends on it.

## The change

- `TxChain` keeps the first tick of the open device burst, that is, the instant of its SOB send.
- An underflow report whose tick is before that first tick belongs to an earlier device burst. Record it only: emit the `TX_UNDERFLOW` event as today and do not abandon.
- A report with no tick keeps today's rule.
- Decide, with evidence, whether "before" is `<` or `<=` from the tick's meaning, and state it.
- Keep it minimal: no new state beyond the one tick. The open-burst gate of A5 stays.

## Tests (red first: run each on the E2a-3 head before the change, and give the failure text)

- A rig test in `e2a.rs` for #93's sequence:
  1. A (finite) ends with EOB.
  2. B's first send is issued within the in-flight window.
  3. Then `report(Underflow { tick: < B's first tick })` arrives.
  - Expected: B plays whole, with no UR-22 note, no two-sample close mid-B and no abandon. The `TX_UNDERFLOW` event is still emitted.
- Its twin: a report whose tick is inside B (≥ B's first tick) abandons B as today.
- A report with no tick abandons as today. Point to the existing test if one already covers this.
- Use the step-F constants and the rig units. No wall clock.

## Mutation rows

- Add rows for: the tick comparison removed (`true`); `<` swapped with `<=`; and the first tick not updated at a new SOB. Each names its killer in `filter`, and that killer must run the path; check this in a /tmp copy.
- Re-anchor anything this breaks.

## Text

- UR-22 (`design/18-uhd-radio.md`): state that an underflow abandons the open device burst only when the report's tick is not before that burst's first sample, and that a report without a tick abandons it. Its `*Test:*` line names the new tests.
- Add the change to the E2a row of design/18's Changes table that E2a-3 added, naming #93.

## Before reporting

Run in the foreground:
- `cargo fmt --all`;
- the rdev workspace tests;
- clippy, both `--workspace --all-targets` and `--features uhd`;
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 0ccb267 --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/renames.txt` = 0.

Report:
- `git diff --stat`;
- the tick's meaning with evidence;
- the red-first results;
- the rows;
- the text changed;
- every behaviour change.

Do not commit, push or `git add`. Do not edit AGENTS.md, handoff.md, KNOWN_LIMITATIONS.md, plan/phase8/49-*.md, bench-results.md or bench/.
