# Phase 8 lane 2 (idle time before the bench): #118 — Sonnet drives Go DeepSeek

Worktree `<worktree>`, branch `work/n118` (base main `62d947d0`). Work only there; never touch `<main tree>` (another lane). Do not commit, stash, push, reset or switch branch.

**Risk: not risky** (a refusal of a malformed Module report at prepare; no queue, deadline, identity, lifecycle, FFI or fake boundary changes). No test plan review; write the tests with the code. Sol reviews the final code.

## The issue
`gh issue view 118 -R k3komatsu/Ez-SDRv4`. Under `accept`, a Provider's validate-time `coerce` previews `19.5 → 20`; at prepare it returns `effective = 20` with a coercion claiming `19.5 → 40`. The Kernel checks the coercion's `requested` (SB-44, since #115) and compares `effective` with the preview, but not the reported coercion's `applied` with `effective` or the preview, so the false `applied` stays in the successful prepare report and the Manifest despite MA-12's producer obligation.

## What to do (orchestrator)
- SB-44 (design/03-spec-and-binding.md) also refuses a prepare coercion whose `applied` differs from the report's `effective` for that key, and from the validate-time preview's `applied` (find where SB-44's `requested` check from #115 lives in `collect_prepare`, and add the comparison next to it, one place). Decide from the code which comparisons are needed: if `effective` is already checked against the preview, `applied == effective` may be enough — say which and why.
- The refusal has the same shape as SB-44's other refusals (an `ezsdr.effective` violation, its reason carrying the reported value; SB-6's `requested` rule). Check the refusal against EA-16 and the user-facing promises (design/16-easy-api.md): only a lying Module triggers it.
- Every Provider in the tree (MockRadio, UHD Module, test fakes and fixtures) already reports `applied == effective`? Search and say; a fixture that reports otherwise is either a bug to fix or a test that must now expect the refusal.
- Tests: one failing-first test with the issue's sequence (preview 19.5 → 20, prepare `effective = 20`, coercion `19.5 → 40` → refused, reason carries 40); a positive case stays accepted. Mutation rows (Rust token anchors, smallest unique `old`) that the test kills by assertion; run them with `remote_mutate.py` through `simq` (`~/.cache/ezsdr-fixes/tools/` has `simq`; never run mutate.py locally).
- design/ text: SB-44 and, if it states the check, MA-12; a Changes row in spec 03 (and 05 if touched). No issue numbers in design/. `rules.py --base 62d947d0` passes.
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` before reporting.

## Process
Use DeepSeek for implementation or reading sweeps if it saves context (tasks `ds-<n>-task.md`, logs `ds-<n>.log` in `~/.cache/ezsdr-fixes/2026-11-20-n118/`; a task file must not start with `---`; launch `cd <worktree> && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`, real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'`, check `/proc/<PID>/cwd`; **wait with the Bash tool's `timeout` set to 600000**: `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`; tell DeepSeek: shell cuts at 120 s, no commit). Review DeepSeek's diff yourself (at most two send-backs). Never `chmod -R`; chmod 644 named files only. Test temp files on /tmp. Kill processes by PID only.

## Report
Japanese, short: the diff stat; where the check lives and which comparisons; the Provider/fixture search (command and result); tests and rows with their verdicts; design/ text changed; INFERRED items; open questions with your recommendation.


## Fix round 1

# #118 fix round — Sonnet (implement directly or drive Go DeepSeek)

Worktree `<worktree>`, branch `work/n118` at `2241a4d8` (your earlier work, committed by the orchestrator, who also changed the SB-44a violation's check from `ezsdr.effective` to `ezsdr.coercion` to match SB-44's other refusals). Prior brief: `~/.cache/ezsdr-fixes/2026-11-20-n118/brief.md`. Do not commit, stash, push, reset or switch branch. Never `chmod -R`.

Sol's code review: `~/.cache/ezsdr-fixes/2026-11-20-n118/sol-code.md` — read it. All four findings are adopted (orchestrator):

1. **F1 (major) one bad key, one violation.** A key that already has a violation from the stray check, the false-request check (already skipped), SB-6's kind check, or MA-12's effective-vs-preview check gets no SB-44a violation as well. Decide the precedence in one place (e.g. collect the keys already refused for the fragment before the SB-44a comparison, or order the checks so the earlier ones win) and state it in SB-44a's text in one clause. Tests: Sol's two sequences (preview 19.5→20, report `effective = 40` with coercion `applied = 40` under `accept` → exactly one violation, the existing SB-44/MA-12 one; a stray key `test.flag` reported `false → true` with `effective = false` → exactly one violation, the stray).
2. **F2 (major) a coercion with no comparison source.** A prepare coercion whose key has no validate preview and is absent from the report's own `effective` is refused by SB-44a too (a malformed report: a coercion applies a value, so the report's `effective` must carry the key). First confirm with a search that every in-tree Provider that reports a prepare coercion puts its key in `effective` (MockRadio `crates/ezsdr-mock-radio/src/lib.rs` ~2333, UHD `crates/ezsdr-radio-uhd/src/provider/mod.rs` ~558, the Sink, test fakes); if one does not, stop and report instead of changing it. Test: Sol's sequence (request `Eq(20)` satisfied directly, no preview, prepare omits the key from `effective` and reports `20 → 40` → refused).
3. **F3 (minor)** restore the SB-6 test's original report (`effective = 20`, coercion `applied = "twenty"`) and let the kind refusal take precedence (one violation), per F1.
4. **F4 (minor)** share the resource-and-key preview lookup between SB-44a and MA-12's loop (one helper), and make a test exercise it with two resources that both carry a prepare coercion for the same key, so a key-only lookup fails. Fix the SB-44a test row in spec 03 so it says which policy each call uses.

Mutation rows: retarget or add rows for the new precedence and the no-source refusal; each dies by assertion. Run all N118-* rows and any re-anchored row through `simq` + `remote_mutate.py` (`REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:2,sim03:2 <ids>`). Before reporting: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `rules.py --base 62d947d0`. chmod 644 any file you create.

Report in Japanese, short: diff stat; each finding fixed or rejected with reason; the F2 Provider search (command and result); tests and row verdicts; spec text changed.


## Fix round 2

# #118 fix round 2 — Sonnet

Worktree `<worktree>`, branch `work/n118` at `2a1c5b46` (your fix round 1, committed). Prior briefs: `brief.md`, `brief-fix.md` in `~/.cache/ezsdr-fixes/2026-11-20-n118/`. Do not commit, stash, push, reset or switch branch. Never `chmod -R`.

Terra's re-check: `~/.cache/ezsdr-fixes/2026-11-20-n118/terra-recheck.md` — read it. Dispositions (orchestrator):

- **Terra 2 (major) and 3 (minor): adopted, with one mechanism.** Replace the per-check suppression with one rule: within `collect_prepare`, the first check that refuses a (fragment, key) wins and every later check skips that key — one `refused` set of (fragment, key), filled by SB-6's kind check, the stray check, the false-request check, SB-44a and SB-46's policy, and read by every later check including MA-12's loop (so MA-12 skips a key SB-44a refused, instead of SB-44a deferring to an MA-12 check that an early return may never run). The coercion loop `continue`s before policy handling for a key already refused (Terra's stray + SB-46 and SB-6 + SB-46 under `reject`). Keep SB-6's early return if it is needed for its own reason (a registered check must not read a wrong-kind value); then SB-44a must not defer to MA-12 at all — just refuse, and MA-12 skips the key. State the rule once in SB-44a's text (or SB-41's, wherever the prepare checks' order is described) in one clause. Tests: Terra's sequences (stray key under `reject` → exactly one violation; wrong-kind coercion under `reject` → exactly one; an SB-6 error in one fragment plus a preview mismatch in another → the mismatch key still gets its violation).
- **Terra 1 (major): rejected for #118, filed separately** (orchestrator): a report that omits a previewed key from `effective` is a different lie (an omission, MA-12's re-match), outside #118's false `applied`; it goes to 8b under the Phase 8 issue cut-off. Do not change it.
- **N118-1 (Terra's note): adopted.** Make its test's fallback value differ from the preview so the row dies on the decision, not on the diagnostic's wording.

Mutation rows: add or retarget rows for the refused-set rule; run all N118-* and re-anchored rows through `simq` + `remote_mutate.py` (`REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:2,sim03:2 <ids>`) and keep the log. Before reporting: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `rules.py --base 62d947d0`. chmod 644 any file you create.

Report in Japanese, short: diff stat; the refused-set rule and where it lives; tests and row verdicts; spec text changed.


## Fix round 3

# #118 fix round 3 — Sonnet (implement directly)

Worktree `<worktree>`, branch `work/n118` at `e49c43b5` (your fix round 2, committed). Prior briefs: `brief.md`, `brief-fix.md`, `brief-fix2.md` in `~/.cache/ezsdr-fixes/2026-11-20-n118/`. Do not commit, stash, push, reset or switch branch. Never `chmod -R`.

Terra's re-check of round 2: `~/.cache/ezsdr-fixes/2026-11-20-n118/terra-recheck2.md` — read it. Dispositions (orchestrator):

1. **Terra 1 (major): adopted.** In `collect_prepare`'s `report_kind` (`crates/ezsdr-kernel/src/plan/prepare.rs:112`), push the SB-6 violation only when `refused.insert` returns true (a key wrong-kind in both `effective` and a coercion's `applied` → one violation). Test: that sequence → exactly one SB-6 violation.
2. **Terra 2 (major): adopted.** MA-12's narrowing refusal inserts (fragment, key) into `refused`, so the re-match loop skips it. Update the existing test that expects both (`spec_binding.rs` ~2653, request `test.count = 2`, report `effective = 7` with no coercion) to expect exactly one (the narrowing one) — say in the report which test and what it asserted before.
3. **Terra 3 (major): rejected as a code change, adopted as a text fix.** Registered checks (`checks.run`, e.g. the RF envelope) keep reading the unfiltered `effective` map: they are a separate stage, and filtering a safety check's input is not worth one fewer message on a refused Run. Narrow SB-44a's text in `design/03-spec-and-binding.md` so the one-violation rule covers `collect_prepare`'s own checks (SB-6 kind, SB-44 stray and false `requested`, SB-44a, SB-46 policy, MA-12's narrowing and re-match), and say in one clause that a registered check may still add its own violation for the key. No test change for this one.
4. **Terra 4 (minor): adopted.** Add two distinct stray keys to a prepare test (or a new small one) and assert two stray violations, one per key.

Mutation rows: add rows for 1 and 2 (smallest unique Rust token anchors; each must die by assertion), and run them plus any row whose anchor or killing test you change (N118-*, re-anchored ones) through `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:2,sim03:2 <ids>`; log to `mut-fix3.log` in the work dir. Before reporting: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (save the full output to `/tmp/n118-test-fix3.log`; if a test fails, name it), `python3 plan/maintenance/tools/rules.py --base 62d947d0`. Test temp files on /tmp. chmod 644 any file you create.

Report in Japanese, short: diff stat; each item done; the changed test (before/after); row verdicts; spec text changed.


## Fix round 4

# #118 fix round 4 — Sonnet (implement directly, tests and one mutation row only)

Worktree `<worktree>`, branch `work/n118` at `64b548d1` (your round 3, committed). Do not commit, stash, push, reset or switch branch. Never `chmod -R`. No production code change.

Terra's re-check: `~/.cache/ezsdr-fixes/2026-11-20-n118/terra-recheck3.md`. Adopted (orchestrator): restore mutation row J160 (the re-match's `is_some_and` → `is_none_or` in `collect_prepare`, `crates/ezsdr-kernel/src/plan/prepare.rs` ~285) and retarget it to a new test of Terra's sequence: a coercible key of kind `List` that the node does **not** declare as a capability, requested `Present`; `coerce` returns it in `applied` with no coercion entry; prepare reports a list `effective` for it → refused by MA-12's re-match (assert the violation's check, key and the `MA-12 (re-match)` reason). If the sequence cannot be built with the existing doubles (`crates/ezsdr-kernel/tests/support/doubles.rs`), or admission refuses it before prepare, stop and report why instead of forcing it. Restore the row with its original fields (`git show e49c43b5:plan/maintenance/tools/mutations.json`), changing only `name` and `filter`.

Run J160 through `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:1 J160`; write the log to `~/.cache/ezsdr-fixes/2026-11-20-n118/mut-fix4.log` (never inside the tree). Before reporting: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p ezsdr-kernel --test spec_binding`, `python3 plan/maintenance/tools/rules.py --base 62d947d0`. The host is under a deliberate load from another lane; do not run the full workspace tests. chmod 644 any file you create.

Report in Japanese, short: the test, the row verdict, checks.
