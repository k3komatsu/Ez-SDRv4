# Report — #102 (a channel invalid on both sides of a stream gap loses its ChannelGap, SC-31c)

Worktree /home/komatsu/local_work/Ez-SDRv4-n102 (branch work/n102, uncommitted). Design: design.md (in [n102-briefs.md](n102-briefs.md)); no change to it during the work.

## Design in one paragraph
Root cause: `push` re-armed a break with `was_valid` = the previous block's mask; after a stream gap flushes a pending break, a channel already invalid before the gap has its bit clear there, so nothing re-arms. Fix: `was_valid` becomes `ever_valid` (union of every pushed block's mask, same builder, same update place) and the branch is `else if self.pending[i].is_none() && self.ever_valid.is_set(c) { pending[i] = Some(t) }`. The `pending.is_none()` guard is new and needed (otherwise a channel invalid over several contiguous blocks has its break moved forward at each block). Rejected alternative: re-arm every flushed break at the gap (zero-length ChannelGap on an immediate return). Spec: SC-31c reworded ("once valid in the map, each later invalid run is a ChannelGap; a stream gap splits a run; a channel not yet valid produces nothing"), SC-31d gets the split sentence, §6 pseudocode uses `ever_valid`, §8 table and Changes row added. Other readers (Manifest map, SigMF annotations `sigmf.rs:66-70`, Python `session.py:168`) take `channel_gaps` as a list; no other expected output changes (`grep ChannelGap` in other tests: only a gap-free case).

## Diff (`git diff --stat`)
continuity.rs 18 (+/-), stream_contract.rs +88, design/02-stream-contract.md 20, plan/maintenance/tools/mutations.json +40 (4 files, +149 -17). I wrote none of the production code or tests; no touch-ups by me.

## Checks
- `cargo fmt --all --check` ok (I re-ran). DeepSeek ran: `cargo test -p ezsdr-kernel --test stream_contract` 75 passed; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` 834 passed, 0 failed (I did not re-run the workspace suite; I saw 32 "test result: ok" lines in its log).
- Old-code check (mine): production file reverted in a scratch copy under /tmp (deleted), `cargo test ... sc_31c`: `sc_31c_invalid_around_a_stream_gap_keeps_its_gap` and `sc_31c_invalid_across_gaps_in_a_row_keeps_each_gap` FAIL on old code (one ChannelGap instead of two / three), the other four pass. Their second scenarios (B) are not reached on old code because A fails first; by reasoning B fails too.
- `rules.py --base 272bc6ff`: 0 problems (ran in 3 s, not 5 min; run twice).
- Rows (remote_mutate.py maintenance sim02:7,sim03:7 N102-1 ... N102-4, log rows.log): N102-1 killed, N102-2 killed, N102-3 killed, N102-4 killed (4/4). The new tests did not change any existing row's filter; no anchor moved (DeepSeek re-checked all 2716 anchors after fmt, 0 mismatches). I did not run the full lists.

## Trial's record
- DeepSeek run 1: whole task (code, 3 tests, spec edits, 4 rows, exit checks), ~8 min (18:23-18:31), finished with every check run and reported honestly (extra fmt --check, all-anchor re-check). Outcome: right at first pass.
- My review round 1: read the whole diff against the design (production hunk, all test values, spec text vs given text, pseudocode lines, row JSON format), reverted-code check above, rows, rules. Findings: none. Send-backs: 0.
- Total time: about 11 min of my own work from reading the brief to this report (design ~4 min of it), including the DeepSeek wait.

## Deviations / INFERRED
- Row N102-3's mutant `self.ever_valid = self.ever_valid;` is a no-op assignment that compiles (rustc may warn); the tool built and killed it.
- plan/maintenance/32-kernel-rules.json carries a note describing this defect; it is a read-only plan record and I left it.
- The first background launches of rules.py/remote_mutate.py: the first remote_mutate call used a wrong `<lists>` argument (a path) and failed at once; the second, detached run died silently (tool cleanup of the process group, INFERRED), so the rows run was redone in the foreground and is the one logged.
- I did not run the Python suites or the image build (not requested by the brief).

## Fix round 1 (after the Opus review: PASS, two minors adopted)
Changes (uncommitted on top of 5303f6af; `git diff --stat`: continuity.rs 10, 02-stream-contract.md 11, mutations.json 22; 3 files, +14 -29):
1. Maintainability: the `ever_valid` field, its initialiser and its update are gone. The branch is `} else if self.pending[i].is_none() && !self.valid[i].is_empty() {` (at that point no segment is open, and closed segments are drained only in `finish`, so "valid earlier in the map" is exactly a non-empty `valid[i]`; I checked `valid[i]` is moved only at `finish`). Spec §6 says "c has a closed segment and no pending close"; the `ever_valid` lines are removed.
2. Spec format: SC-31c cites SC-31d for a run that spans a stream gap; SC-31d's two overlapping sentences are one ("A stream gap ends a channel's pending break at the gap's start; a channel that is invalid in the block after the gap starts a new break at that block, never at its last valid sample, whether it was valid or invalid before the gap."). The Changes row drops its parenthetical about the builder.
3. Rows: N102-2 retired (it mutated the removed field; no analogue). N102-1, N102-3 (retargeted to `&& false`, name updated, killer `sc_31c_channel_lost_at_the_overflow`) and N102-4 re-anchored on the new condition; all other text and filters unchanged. The tests are unchanged.

Checks: `cargo fmt --all --check` ok; DeepSeek: clippy `-D warnings` clean, `cargo test -p ezsdr-kernel -p ezsdr-sink-capture` 702 passed / 0 failed (17 suites), all 2715 row anchors match exactly once; mine: `rules.py --base 272bc6ff` 0 problems; rows run in the foreground through simq (`rows-fix1.log`): N102-1 killed, N102-3 killed, N102-4 killed (3/3).

Trial record for this round: DeepSeek run 2 (ds-2), about 8 min, right at first pass; my review: read the whole diff against the task text (all of it matched exactly), then the checks above; findings none; send-backs 0; total about 12 min for the round (217 s of it after DeepSeek ended).
