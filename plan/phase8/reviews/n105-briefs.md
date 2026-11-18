# #105 / #104 — briefs (2026-11-18)

## Implementation brief (Go DeepSeek, self-review Haiku 5.5)

## Brief — #105 and #104: small Kernel drifts from the rule sweep

You are the implementer. Worktree `/home/komatsu/local_work/Ez-SDRv4-n105`, branch `work/n105` (from main ce631495). Work ONLY in that worktree. Never touch `/home/komatsu/local_work/Ez-SDRv4` (another lane works there) or `/home/komatsu/local_work/Ez-SDRv4-bench`. You edit files; you do **not** commit, stash, push or switch branches. Not a risky step: no test plan. Read `AGENTS.md` (§3, §6, §7, §9) in the worktree first.

Read both issues: `gh issue view 105 -R k3komatsu/Ez-SDRv4 --comments`, `gh issue view 104 -R k3komatsu/Ez-SDRv4`.

### What to do (all decided; do exactly this, add no other mechanism)

1. **SB-9 (#105 item 1).** `EventSource::Kernel` and `EventSource::Unforeseen` (`crates/ezsdr-kernel/src/event.rs:106-127`) become empty struct variants `Kernel {}` / `Unforeseen {}`, as `Termination::Completed {}` already is, so `deny_unknown_fields` refuses `{"kind":"kernel","extra":1}`. Fix every constructor and match. Regenerate schemas if they change (`EZSDR_UPDATE_SCHEMAS=1 cargo test -p ezsdr-kernel --test schema_freeze`), then the plain `schema_freeze` test must pass. Test: `Manifest::from_json` (or the narrowest door that parses an `EventSource`) refuses an extra field on both variants and still accepts the plain form.
2. **OV-23 (#105 item 2).** `WITHDRAWN` in `crates/ezsdr-kernel/tests/kernel_surface.rs` must list every rule id the specs mark withdrawn. Recount from the text: search `design/*.md`, `plan/phase1/00-overview.md` and `plan/phase2/00-overview.md` for rule stubs that say "Withdrawn" (e.g. `grep -nE '\*\*[A-Z]+-[0-9]+[a-z]?\*\* +Withdrawn' design/*.md plan/phase1/00-overview.md plan/phase2/00-overview.md`, and check other spellings). Put the exact search and the resulting list in your report. Only ids whose rule is the Kernel's concern if the existing list is filtered that way — read how `WITHDRAWN` is used first and keep its meaning.
3. **MA-20 (#105 item 3).** `StepOutcome`'s and `Executor::step`'s doc comments say a step that pushed an Action also counts (MA-20, NX-5: read their text in `design/`); regenerate `schemas/step_outcome.v1.json` from the doc comments as above.
4. **MA-29 (#105 item 4).** `AuthorityDescriptor`'s doc comment (`crates/ezsdr-kernel/src/module_api.rs:~663`) says what MA-29 now says (`governs` names the primary root, `host.monotonic` and every simulated root; the `TimeAuthority` it provides governs the derived domains); regenerate `schemas/authority_descriptor.v1.json`.
5. **OV-23b (#105 item 5).** `ov_23b_kernel_growth_is_the_new_count` builds `new` without the `FEATURE:testing` lines, as `total` already excludes them, so it prints the count the rule states (113, not 114). Read the rule text first and make the test assert the rule's count, not print it.
6. **SB-9a (#104).** A Provider's `PrepareReport.effective` and a `CoerceReport`'s coercions are checked for non-ASCII map keys where they enter:
   - in `collect_prepare` (`crates/ezsdr-kernel/src/plan/prepare.rs:32`), a report whose `effective` carries a non-ASCII key refuses the prepare with a `Violation` in the same form as the other refusals there (`check: ezsdr.effective`, `key: None`, `reason` starting `SB-9a:`), never transcribing the offending key (see the comment at `crates/ezsdr-kernel/src/coordinator/admission.rs:~65` for why);
   - where a `CoerceReport`'s coercions enter the admission result (find it: `crates/ezsdr-kernel/src/session.rs:~131/~241` and the validate path), a non-ASCII key refuses at `validate` in the same way.
   - Tests: a test Provider (reuse an existing test Provider/fixture in `crates/ezsdr-kernel` that already returns a custom `PrepareReport` / `CoerceReport`; do not build a new framework) whose `effective` has the key `"radio.ü"` → the Run is refused at prepare, nothing arms or transmits; one whose coercion preview carries such a key → `validate` refuses. Name the tests after the rule (`sb_9a_…`) and add them to SB-9a's `*Test:*` line in `design/`.
7. Add the new tests' names to the `*Test:*` lines of SB-9, OV-23/OV-23b, MA-20 if those lines name tests; add a row to each touched spec's *Changes* table only if `rules.py` or the spec's format asks for it (no history text inside rules; no version bumps; no `SCHEMA_CHANGELOG.md` entry — AGENTS §6).
8. **Mutation rows**: one row for each new refusal in item 6 (and item 1 if a code branch, not just a type, carries it), Rust token anchors, the smallest uniquely matching text, each naming the test that kills it. Put them in the mutation list that already holds rows for the file (search `plan/maintenance/tools/` and `plan/phase*/` for the list files that `remote_mutate.py` takes). Check that existing rows whose anchors you moved still match exactly once (`rules.py` does this).

### Exit checks (all in the worktree, foreground; your shell tool cuts commands at 120 s by default, so give each long command a timeout of at least 1800 s)
- `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base ce631495` (about 5 minutes).
- Rows: your new rows and every existing row whose filter selects a test you changed, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` (it waits its turn in a queue; wait for it in the foreground); log to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-n105/rows.log`. Every new row must be `killed`.

### Self-review (one round, after the exit checks pass)
Run a fresh read-only reviewer from your shell, foreground, timeout 2400 s:
`timeout 2400 opencode2 run --standalone -m "anthropic/claude-haiku-5-5#high" --agent plan --auto "<checklist>"` from the worktree, with this checklist in the prompt: (1) every item 1–6 of /home/komatsu/.cache/ezsdr-fixes/2026-11-18-n105/brief.md has its test and the test observes the refusal, and each mutation row breaks its test; (2) scope against the brief, missing and extra; (3) for OV-23's list and SB-9a's 'every door' claim, search the code and specs for withdrawn ids and for every place a Provider's or Module's `Value` enters the Manifest, and list any missed; (4) every `design/` text the change makes false. Do not run mutation, do not edit, report findings with path:line. Fix each finding or reject it with its reason in your report.

### Rules
- Mutation rows only through `remote_mutate.py`; never run `mutate.py` locally; never apply a mutant to the tree.
- Kill processes by PID, never by name pattern. Temp files on /tmp. No build output in the tree.
- `~/.cache/ezsdr-fixes/` (except your rows.log and report), `handoff.md`, `AGENTS.md`, plan notes: read only.
- No wall-clock tests. No mechanisms beyond the above. Do not delete a test of a reachable sequence without a replacement.
- After `cargo fmt`, `rules.py` must still find every row exactly once.

### Report
Write `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-n105/report.md`: `git diff --stat`, each check's counts, each row's outcome, the OV-23 search and list, the self-review findings and what you did with each, deviations, anything INFERRED. Your final message is that report's summary.

## Fix round 1 brief (after Sol's code review)

## Fix round 1 — #105/#104 (after Sol's code review)

You are the implementer. Worktree `/home/komatsu/local_work/Ez-SDRv4-n105`, branch `work/n105`; the step so far is commit 73b424ac on base ce631495 (brief: /home/komatsu/.cache/ezsdr-fixes/2026-11-18-n105/brief.md, your report: report.md, Sol's review: sol-code.md, same directory — read all three). Same rules as the first brief (its "Rules" section applies unchanged): work only in that worktree, edit only, no commit/stash/push/branch switch. No self-review this round.

### Adopted findings (decided by the orchestrator; do exactly this, add nothing else)

1. **Sol finding 1 (major).** In `crates/ezsdr-kernel/src/plan/matching.rs:~133-166` the SB-9a check runs only on the non-stray coercions, and a stray's unchecked `requested` is copied into `Violation.requested` (:138), so a stray carrying a non-ASCII key leaves the Manifest unsealable. Move the SB-9a check before the SB-44 partition so it covers every coercion the report returns: a coercion failing it produces the SB-9a violation (as now: `key: None`, `requested: None`, reason never naming the key) and is neither a stray's violation nor previewed. Test: a stray coercion (key the request does not name) whose `requested` is a map keyed `"radio.ü"` → `validate` refuses with the SB-9a violation and no violation's `requested` holds a non-ASCII key; name it `sb_9a_a_stray_coercion_carrying_a_map_key_that_is_not_ascii_is_refused_without_transcribing_it`. Mutation row: the check skipped for strays (the smallest token-anchored change that puts the check back after the partition or makes it miss strays), killed by that test.
2. **Sol's inventory, pre-existing open door.** A registered admission check's `Violation.requested: Option<Value>` is kept unchecked (`crates/ezsdr-kernel/src/binding.rs:~360-361`, retained through `plan/validation.rs:~101` into the Manifest). Where `binding.rs` collects a registered check's violations, a violation whose `requested` fails `check_ascii_keys` keeps the violation but has `requested` set to `None` (the refusal still happens; only the value it could not record is dropped), with a one-line comment citing SB-9a and KC-24. Test: a registered check (reuse an existing test check in crates/ezsdr-kernel tests; do not build a framework) returning a violation whose `requested` is a map keyed `"radio.ü"` → the violation is reported with `requested: None` and the refused Run's Manifest seals. Mutation row killed by it.
3. **Sol finding 2 (minor).** `ov_01_citing_a_withdrawn_rule_satisfies_nothing` (`crates/ezsdr-kernel/tests/kernel_surface.rs:~107-115`) iterates `WITHDRAWN`, so it cannot catch an incomplete list. Add explicit assertions that `cites_a_rule` refuses each of the five ids added in 73b424ac (SC-31a, SB-22h, SB-48, SB-49, KA-14), written out literally, not taken from `WITHDRAWN`.
4. Add the new test names to SB-9a's `*Test:*` line in `design/03-spec-and-binding.md`.

### Exit checks (foreground, tool timeout ≥ 1800 s)
`cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base ce631495`; rows: the two new rows plus N105-2 and any row whose anchor you moved, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:7,sim03:7 <ids>`, appended to rows.log. Every new row `killed`.

### Report
Append a "Fix round 1" section to report.md (diff stat of the uncommitted change, checks, rows, deviations). Final message: its summary.
