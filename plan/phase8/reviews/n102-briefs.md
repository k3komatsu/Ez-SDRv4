# #102 — brief and the implementer's design (2026-11-18)

## Brief — #102: a channel invalid on both sides of a stream gap loses its ChannelGap (SC-31c)

**This step is a trial of a new way of working (owner 2026-11-18).** You, a Claude Sonnet subagent, own the step end to end and are responsible for its quality: you design the fix, Go DeepSeek writes the code, you review DeepSeek's work and send it back until it is right, and only then you report to the orchestrator. You do not hand-write production code or tests yourself (a one-line touch-up after review is fine; say so in the report). This replaces the cheap self-review; an independent Opus review follows your report, so do not stop at "looks fine" — the trial is judged by what that review still finds.

Worktree `/home/komatsu/local_work/Ez-SDRv4-n102`, branch `work/n102` (base 272bc6ff). Work only there. Never touch `/home/komatsu/local_work/Ez-SDRv4` (another lane) or other worktrees. No commit, stash, push or branch switch (you and DeepSeek). Not a risky step: no test plan document. Read `AGENTS.md` §3, §6, §7, §9 in the worktree first.

### The defect
`gh issue view 102 -R k3komatsu/Ez-SDRv4`. Rule SC-31c / SC-31d (`design/02-stream-contract.md:~278-345`, the pseudocode's `else: nothing` branch at ~:335, and the test table ~:400-407); code `crates/ezsdr-kernel/src/stream/continuity.rs:~220-280` (`ContinuityBuilder::push`: `flush_pending_at` and `close_open_segments` at a stream gap, then the per-channel loop that reads `was_valid`).

### Your design (write it down before DeepSeek starts: `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-n102/design.md`, short)
- The rule as it should read (SC-31c/SC-31d and the pseudocode), for every case around a stream gap: channel valid before and after; invalid before (break pending), invalid after; invalid before, valid after (`sc_31d_break_across_a_stream_gap_is_split`); valid before, invalid after (`sc_31c_channel_lost_at_the_overflow`); never valid (`sc_31c_never_valid_channel_has_no_gap`); several gaps in a row with the channel invalid throughout; a channel that was valid only in an earlier map segment before two gaps; the map's first block after a `GAP_BEFORE` origin.
- The smallest code change (state kept with its owner: if "was ever valid" or "a break was flushed at the gap" is needed, say where it lives and why `was_valid` alone cannot carry it), the spec text change, and the tests (one per case above that is not already pinned; the issue's sequence first, expected `ChannelGap{2,350,100}`).
- Who else reads continuity: the Manifest's continuity map and the capture Sink's SigMF per-channel annotations (`crates/ezsdr-sink-capture`), the Python client; whether any expected output elsewhere changes (search the tests for `ChannelGap`).
- Mutation rows for the new behaviour (Rust token anchors, smallest uniquely matching text, each naming its killing test), in the list that already holds rows for `continuity.rs`.

### Driving DeepSeek
- Launch it detached, so it survives whatever happens to your turn, and note its PID: `cd /home/komatsu/local_work/Ez-SDRv4-n102 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > /home/komatsu/.cache/ezsdr-fixes/2026-11-18-n102/ds-<n>.log 2>&1 < /dev/null & echo $!` (write each task to `ds-<n>-task.md` in the step directory first). Give it a precise task: files, the exact behaviour, the tests with their names and expected values, the rows, the exit checks, and what it must not do (add mechanisms you did not ask for, touch other files, run `mutate.py` locally, apply a mutant to the tree, commit). Its shell tool cuts commands at 120 s, so tell it to give long commands a timeout of at least 1800 s.
- A run's length cannot be predicted (tens of minutes is normal). Wait for it with a foreground command that does not depend on its length: `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`, repeated until it prints `done`; between waits, glance at the log's size and last lines. Do not end your turn while it runs. Split the work only where it splits naturally (e.g. code + tests, then rows), not by time. One DeepSeek run at a time; do not edit the tree while it runs. If the log has not grown for 15 minutes, check `~/.local/share/opencode/log/opencode.log` before judging it hung; kill a hung run by PID only (its process group: `kill -- -<PID>`).
- Review each run yourself: read the whole diff against your design, run the exit checks, and check the tests fail on the old code (revert the production hunk in a scratch copy outside the repo, or reason from the mutation rows). Send it back (a fresh `opencode2 run --standalone` whose task names the previous task, the diff and your findings) **at most twice**; if it is still wrong after that, stop and report what is left.

### Exit checks (in the worktree, foreground or waited for)
- `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 272bc6ff` (about 5 minutes).
- Rows: the new rows and every existing row whose filter selects a test that changed or whose anchor moved, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` (it queues; wait for it); log to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-n102/rows.log`. Every new row `killed`.

### Rules
Mutation rows only through `remote_mutate.py`; never `mutate.py` locally; never a mutant in the tree. Kill by PID. Temp files on /tmp. No build output in the tree. `~/.cache/ezsdr-fixes/` (except this step's directory), `handoff.md`, `AGENTS.md`, plan notes: read only; `design/02-stream-contract.md` may be edited. No wall-clock tests. No version bumps or SCHEMA_CHANGELOG entries.

### Report (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-n102/report.md`; your final message is its summary)
- the design (link design.md) and any change to it during the work;
- `git diff --stat`; each check's counts; each row's outcome;
- **the trial's record**: each DeepSeek run (task in one line, duration, outcome), each of your review rounds (findings, what DeepSeek did with each, what you fixed yourself if anything), the number of send-backs, the total time;
- deviations, and anything INFERRED.

## The implementer's design (Sonnet)

### Design — #102 (SC-31c / SC-31d across a stream gap)

#### Root cause
`push` re-arms a break at a block with `else if was_valid.is_set(c)`, where `was_valid` is the **previous pushed block's mask**.
A break already pending at a stream gap is flushed (ChannelGap to the gap's start) and cleared. If the channel was already invalid in the
block before the gap, `was_valid` has the bit clear, so the block after the gap re-arms nothing: those samples are in no Segment and no ChannelGap.
A one-block memory cannot say "this channel was valid earlier in the map"; that is what SC-31c's "never valid produces nothing" needs.

#### The rule (cases around a stream gap; "ever valid" = valid in some earlier block of the map)
| case | result |
|---|---|
| valid before and after | Segments split by the gap, no ChannelGap (SC-31b; `sc_31b_...`) |
| valid, then invalid, gap, invalid after (the issue) | `ChannelGap{c, break start, to gap start}` and a second `ChannelGap{c, first block after the gap, ...}` to the return or the map's end |
| invalid before, valid after | first only (`sc_31d_break_across_a_stream_gap_is_split`) |
| valid before the gap, invalid after | one from the block after the gap (`sc_31c_channel_lost_at_the_overflow`) |
| never valid | nothing (`sc_31c_never_valid_channel_has_no_gap`) |
| several gaps in a row, invalid throughout (ever valid) | one ChannelGap per run, each from its block to the next gap's start / the end |
| valid only in an earlier segment before two gaps | same |
| first block after a `GAP_BEFORE` origin invalid, valid later | nothing: not yet valid is not a gap (new test) |

#### Code
Replace `was_valid` (previous mask) with `ever_valid: ChannelMask` — the union of every pushed block's mask — in the same builder, updated at the same place
(`self.was_valid = h.valid` becomes a union). The third branch becomes `else if self.pending[i].is_none() && self.ever_valid.is_set(c) { self.pending[i] = Some(t) }`.
The `pending[i].is_none()` guard is new: with a union, a channel invalid over several contiguous blocks would otherwise have its pending break moved forward at each block
(before, `was_valid` was clear after the first invalid block, so the branch ran only after a stream gap closed the segment). Reached with no open segment and no
pending break, the branch can only run for a channel whose segment/break a stream gap just closed. No new field besides the rename; no zero-length ChannelGap is possible
(a break starts at a block's `t` and ends at a later block's `t`, a later gap's start, or the end, each after a non-empty block).
Alternative rejected: at the gap re-arm every flushed break at the block after it — it yields a zero-length ChannelGap when the channel returns at once, and needs a guard for it.

#### Spec text (design/02-stream-contract.md)
SC-31c reads: once a channel has been valid in the map, every later run of delivered samples where it is invalid is a ChannelGap, ending where it returns or at the map's end; a stream gap splits a run (SC-31d); a channel not yet valid produces nothing.
SC-31d: the split sentence. Pseudocode: `was_valid` line becomes `ever_valid`, the else branch uses it, last line updates it. Tests table and rule Test lines get the new tests; Changes row.

#### Other readers
Manifest `ContinuityMap` and capture SigMF annotations (`sigmf.rs:66-70`) take `channel_gaps` as a list; the added entries are valid input (sorted by index/channel). The Python client only tests for presence (`session.py:168`). `grep ChannelGap` in the other crates' tests: only `sink_capture.rs:1061` (`hd_15_annotations_are_sorted...`, a stream without gaps: unchanged). Expect `cargo test --workspace` to show no other change.

#### Tests (crates/ezsdr-kernel/tests/stream_contract.rs)
1. `sc_31c_invalid_around_a_stream_gap_keeps_its_gap`: A the issue (finish); B same, then a valid block returning at 450.
2. `sc_31c_invalid_across_gaps_in_a_row_keeps_each_gap`: A invalid over two contiguous blocks then two gaps; B valid only in the first block before two gaps.
3. `sc_31c_a_channel_not_yet_valid_has_no_gap`: GAP_BEFORE origin with the channel invalid, valid later.

#### Rows (plan/maintenance/tools/mutations.json, N102-1..4)
see task.
