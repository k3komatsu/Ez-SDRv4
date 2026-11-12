# A1 — briefs (2026-11-12)

## Implementation brief

## Brief — A1: the server's fail-stop backstop (owner 2026-11-12)

Work tree /home/komatsu/local_work/Ez-SDRv4-a1, branch `work/a1` from `main`. Implementer: OpenCode Sonnet 5.5 #high. Risk: not risky (a small process-exit backstop; no test plan); Sol reviews the code.

Read first: plan/phase8/reviews/n89-fable-design.md "A1" (~:157-166) and F4 (~:95-106), n89-sketch.md "Owner decisions" 8, n89-dispositions.md; crates/ezsdr-server/src/main.rs, watch.rs, lib.rs (`serve_watched`, the watch that fires when input ends); design/16-easy-api.md EA-15, EA-16; KNOWN_LIMITATIONS.md KL-43; plan/phase8/bench.md "#89 の後". The bench of 2026-11-12 measured E9: after the server container's death (`docker kill` or the emergency stop), the tone on an independent receiver ended within 81–608 ms and 54–582 ms (1 MS/s, 100 kHz tone, #1 OBX, one run each): a dead server stops the device by starvation within a second.

### What to build

- In `main.rs` only (not in the library's `serve_watched`): once the server's input has ended (the existing watch fires), start a timer; if the process is still alive N seconds later, it exits at once (`std::process::exit` with a distinct non-zero status), without cleanup. N = 60 s (longer than RS-8a's worst cleanup), overridable for tests by an environment variable (e.g. `EZSDR_SERVER_FAILSTOP_SECS`); state the variable where the server's other variables are documented.
- A test that runs the real `ezsdr-server` binary with the override set to a short value and a server kept from finishing after its input ends (find the smallest existing way to hold it — a stdout nobody reads, or a test-only hook if one exists; do not add a production hook only for the test), and asserts the process exits by itself with that status within a bound. The test's wall bound must write an infra marker on expiry (see crates/ezsdr-radio-uhd/tests/common/infra.rs for the format) or be avoided; it must not depend on when a thread runs beyond the stated bound.
- Text: EA-15 / EA-16 (design/16) if they describe what a server does after its input ends; KL-43: remove what A1 resolves ((c)'s hung server not killed by Ctrl-C, a server behind ssh left running) and keep the rest; (b) gains the measured E9 with its conditions, and its "INFERRED … to be measured" becomes the measurement; the Status line drops "A server-side backstop … is to be decided". Changes rows where the spec format requires them.
- A mutation row for the backstop (the timer not started, or N ignored), killed by the test; run it through `remote_mutate.py <list> sim03:4 <id>`.

`cargo fmt --all`; `rdev -- cargo test --workspace` green; clippy clean; Python suites if the server's behaviour seen by Python changes (`PYTHONDONTWRITEBYTECODE=1`, the venv in AGENTS §7); `rules.py --base <the branch's start>` = 0. Builds via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`. No commit.

Report: `git diff --stat`, the test and the row's outcome, the text changed.

## Fix round 1 brief

## Brief — A1, fix round 1: Sol's code review

Work tree /home/komatsu/local_work/Ez-SDRv4-a1, branch `work/a1` (local commit `bb911fa`). Implementer: OpenCode Sonnet 5.5 #high; brief.md (same directory) binds. Read sol-code.md there. Decisions:
1. Major 1 (decided by the orchestrator): the backstop's thread is created fallibly (`std::thread::Builder::spawn`); if it cannot be created, the server fail-stops at once (the same exit status 3, or a distinct one if that reads better — say which) instead of panicking, so neither a failed spawn of the timer nor of the input thread leaves a server alive after its input ended. Check `transport.rs`'s input thread (`forward()` → `watch.fire()`) for the same panic path and treat it the same way if it is in the binary's control.
2. Major 2 (owner, 2026-11-12: "本番ではほぼ起こらないので受け入れ"): N stays 60 s, and it is **a cutoff, not a bound on cleanup**: state in EA-15 (and the binary's doc) that a cleanup still running 60 s after the input ended is cut, with no Manifest, and that RS-8a bounds each cleanup step, not the whole cleanup; KL-43 says the same in one line (when it can happen: many slow Provider steps; the transmit still stops by starvation, E9).
3. Minor 3 (decided by the orchestrator): pin the delay independently of when the parent observes the exit — e.g. the child records its own exit instant (stderr line or a file the test reads) or the test compares the exit instant reported by the process against the close of its stdin — so A1-3 (no sleep) dies on an assertion that does not depend on scheduling; keep the subprocess test for the status and the upper bound (its bound writes the infra marker).
Run the touched rows through `remote_mutate.py maintenance sim03:4 <ids>` (if the other lane runs the full lists, wait); `cargo fmt --all`; `rdev -- cargo test --workspace` green; clippy clean; `rules.py --base <branch start>` = 0. No commit. Report per finding.
