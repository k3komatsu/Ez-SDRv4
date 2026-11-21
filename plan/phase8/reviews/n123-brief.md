# Lane 1 (idle time before the bench): #123 — Sonnet writes production, Go DeepSeek writes the tests

Worktree `<worktree>`, branch `work/n123` (base main `1ad06f5b`). Work only there; never touch `<main tree>` (main tree) or `<lane 2 worktree>` (lane 2). Do not commit, stash, push, reset or switch branch. Work dir `~/.cache/ezsdr-fixes/2026-11-21-n123/`.

**Risk: not risky** (a refusal of a malformed Module report at prepare; no queue, deadline, identity, lifecycle, FFI or fake boundary change). No test-plan review; write the tests with the code. Terra + Opus review the code afterwards.

## The issue
`gh issue view 123 -R k3komatsu/Ez-SDRv4`. A Module's `prepare` report that omits a key from `effective` although validation previewed a coercion for it passes `collect_prepare` (crates/ezsdr-kernel/src/plan/prepare.rs): request 19.5 under `accept`, `coerce` previews 19.5 → 20; the report carries the matching coercion (or none) and no `effective` entry for the key; SB-44a accepts the coercion (it matches the preview) and MA-12's re-match skips the absent entry. The sibling #118 (landed; records plan/phase8/reviews/n118-*, read n118-brief.md and n118-dispositions.md first) built the one-violation-per-(fragment, key) `refused` set and SB-44a's "a report with no `effective` value for a key without a preview is malformed".

## What to do
- Refuse, in `collect_prepare`, a report that omits from `effective` a key that has a validate preview (a coercion `coerce` returned) or a prepare coercion. One place, inside the existing `refused`-set order (state where it sits in SB-44a's order clause). Decide from the code whether this belongs in SB-44a (malformed coercion report) or MA-12 (re-match) and say why; the violation shape follows the rule it joins (SB-6's `requested` rule; never the reported value as `requested`).
- First search every in-tree Provider/Sink/test fake that returns a `PrepareReport` (MockRadio, UHD Module, the Sink, fixtures): does each already put every previewed or coerced key in `effective`? Give the command and result. If an in-tree producer omits one, stop and report instead of changing it.
- Check the refusal against EA-16 / design/16: only a Module that lies in its report triggers it.
- design/: SB-44a and/or MA-12 text (one clause), the test table row(s), a Changes row in spec 03 (and 05 if touched). No issue numbers in design/. `rules.py --base 1ad06f5b` passes.

## Division of work (owner 2026-11-20)
You write the production change and the spec text yourself. Then Go DeepSeek writes the tests (failing first: the issue's sequence → exactly one violation; with a prepare coercion and without; a positive case where the key is present stays accepted; a key already refused earlier gets no second violation) and the mutation rows (Rust token anchors, smallest unique `old`, killed by assertion), and runs the checks. Review DeepSeek's tests with `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md` (≤ 2 send-backs). Never both editing the tree at once.

DeepSeek launch: task file `ds-<n>-task.md` (must not start with `---`), log `ds-<n>.log` in the work dir; `cd <worktree> && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`; real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'` and check `/proc/<PID>/cwd` is this worktree (lane 2 may run its own DeepSeek — never touch it); wait with the Bash tool's `timeout` set to 600000: `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`. Tell DeepSeek: its shell cuts at 120 s, no commit, no new mechanisms beyond the task.

## Checks
`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (locally), `rules.py --base 1ad06f5b`. Rows via `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ids>` (foreground, wait; also run any N118-* row the change re-anchors or whose killer test changes). Never mutate.py locally. Kill by PID only, never `pkill -f`. Temp files on /tmp. chmod 644 named new files only (never -R). Do not touch failure-mechanisms.md, KNOWN_LIMITATIONS.md, handoff.md.

## Report
Japanese, short: diff stat (prod / test / spec lines; your lines vs DeepSeek's); where the check lives and which rule; the producer search (command, result); tests and rows with verdicts; design/ text changed; DeepSeek rounds/send-backs; open questions with your recommendation.
