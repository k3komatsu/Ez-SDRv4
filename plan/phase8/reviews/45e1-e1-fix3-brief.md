# Brief: E1 fix round 3 — the restore precondition, pending settings, and case 8's second stream

You are the implementer (AGENTS.md §9 step 8). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Context
- The experiment code is uncommitted in `crates/ezsdr-radio-uhd/src/brain/`. The rows in `plan/maintenance/tools/mutations.json` run up to E1-178.
- The plan is `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md`, and its §10 drafts 5–10 hold every earlier decision. Read the draft 7–8 mechanism: the brain decided from cell state while work it had handed out, or held itself, would change that state.
- The previous round's report (fix round 2) found a new counterexample: `counterexamples::ce_restore_issued_streaming_applied_after_faulted_stop` in `cases.rs` (case 7b, seed 43409, I13). The brain issues a Restore while the chain is idle in Streaming; S takes it late; R's Stop faults and the chain becomes Uncertain; S then applies the Restore.

## Orchestrator's decisions (implement exactly)
1. **Restore precondition.** Production restores only after Provider::stop has stopped and flushed the chain, and the RX thread runs stop, flush and restore in sequence (provider/rx.rs:300-335, 434-460). So a Restore of a channel is decided only when **every** stream whose channel set contains it is `Off` and not busy, in addition to the existing conditions: pins and queued values, debt and left, left after an attempted flush → `Never`, `Uncertain`/`Lost` → `Never`. A chain in `Armed`, `Streaming`, `Stopping` or `Flushing` makes it wait. Un-ignore the counterexample as a default test (or a shorter scripted equivalent) that passes after the fix. Add a row.
2. **Pending settings commands block lifecycle on their channel.** Once the brain has sent a Restore or Move for a channel, and until its `Done` arrives, StartNext, Open and Close of any stream whose channel set contains that channel wait. A second Restore or Move of that channel waits too. The brain owns this fact, because it sends the command and receives the `Done`: keep one map from command id to channel, written at send and cleared at `Done`. Use it in `decide`; write no second copy of the condition. Test: a Restore sent, then StartNext of the owning stream waits until the Restore's `Done`. Add a row.
3. **Case 8, W8a/W8b.** Under the current rules, Close and Open wait for the case's value to be done, so S cannot be publishing a loss while R0 waits for G. As scripted, the witnesses are unreachable (a plan gap; 0 of 300,000 seeds). Add a **second stream** to case 8 (full, reduced and micro): stream 1, channel 1, streaming from 0, with no values and no cut beyond the horizon. Its R can then see the loss in a recv or a stop and publish it holding G while R0's Close or Open waits for G (W8a), or is refused at its recheck (W8b). Keep everything else in the script.

## After
- Run the default brain suite in debug, with its wall time, and clippy `--all-targets -- -D warnings`.
- In release, run the micro `_full` variants (2 × 10^6 states, 180 s) and record their missing witnesses.
- Run the sweeps with the swarm scheduler: every case at 2,000 seeds, the seeded configuration at horizon 96 at 2,000 seeds, and case 7b and case 8 at 50,000 seeds. Record failures and unreached witnesses.
- Shrink and classify any new failure: (a) world or harness, (b) core, (c) plan gap. Fix (a) with a test and a row. For (b)/(c), add an ignored counterexample and report it, without changing core decisions beyond the points above.

## Build and test (owner: never put build or test load on this PC)
- Every cargo command runs through `python3 plan/maintenance/tools/remote/rdev.py -- …` on sim02, one at a time, in the foreground, each under 10 minutes. Long sweeps may use `--host sim03` for a second command run in sequence, never in parallel. Never run cargo build, test or clippy locally. The only local cargo command is `cargo fmt --all`.
- Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD`. Append rows from E1-179 on, without re-serializing the file. Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout.

## Report (at most 25 lines)
- Each decision: its test, the fix `path:line` and its row.
- The micro `_full` table.
- The sweeps: failures and unreached witnesses with seed counts.
- New counterexamples with their classification.
- The default suite's count and wall time, and the clippy result.
