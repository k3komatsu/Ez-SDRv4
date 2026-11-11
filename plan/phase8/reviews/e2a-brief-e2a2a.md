# Brief — E2a-2a: `open` into the book, the send split, the lock-wait stat, the F7 checks (no behaviour change, no trace change)

Repository /home/komatsu/local_work/Ez-SDRv4, branch `work/e2a`. Its head is the E2a-1 commit; the orchestrator names the hash when launching. The step's start commit is `0ccb267`.

Read first:
- /home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/brief.md (the step brief; its rules bind);
- test-plan.md in the same directory: §1, §2.1, §2.2, §2.4 and §3.2, with the "Changes after …" sections;
- plan-adjudication.md and plan-dispositions.md (the orchestrator's decisions);
- sketch.md §1 and fable-design.md (A3, F6, F7, F8, F9) for the shape;
- `git show HEAD` (E2a-1: `TxBook` in crates/ezsdr-radio/src/staging.rs).

## Scope (plan §3.2; D1 accepted)

- `open` moves from `Tx` and the Mock's `TxStream` into the book (`Open{booking, next, first}`).
- New book transitions: `take`, `untake`, `end_open`, `issue(send)` and `complete(outcome) -> Settle`. There is one `inflight: Option<Send>` and no op ids (A3).
- `send` splits into three parts: the plan (one short section under the streams lock, calling `issue`), the device calls with no lock held, and `complete` (one short section).
  - `step`'s other sections stay as they are (D1): `tx_step_guarded`, `e1_q2b`, `e1_q14` and N47S5-57 stay.
  - The new probe `tx_send_planned` sits after the plan, with no lock held.
- The Mock calls `issue`, then `complete(Accepted(len))`, for each block, and takes its `open` from the book.
- `stats.tx_lock_wait_max_ns` is added through `Core::stat_max`. Measure it on the host clock; it is a lazy key.
- The F7 test-build checks (§2.4):
  - Re-entering the same mutex panics.
  - `assert_line_free("emit")` runs in `Core::emit` and `emit_at`.
  - Every existing emit under a lock that this exposes is a finding. Fix it in production in this sub-step and list it in the report.
- Move cursors, padding, hold and close rules verbatim. Device-level code stays in uhd-tx.
- **No behaviour change, and no FakeDevice trace change.**

## Tests

- P6–P9 and R1–R6, with R3a, as the plan states. R4 uses the amended order: H is booked before the Stop and is cancelled; H′ is booked after it and survives.
- Rig tests go in the new file `crates/ezsdr-radio-uhd/src/provider/tx/tests/e2a.rs`.
- Existing tests may change only in accessor spelling. Use the test-only `open_next`, `open_first` and `poke_open` readers on `Tx`. No assertion changes.

## Trace comparison (§2.2 as amended; the main risk)

- Use throw-away copies under /tmp. Patch FakeDevice to log `calls()` plus, at the entry of its send, the attempted stream index, sob, eob, at, n and channel count. Do the patching before the fake forces EOB or drops fields.
- Compare **the step's start `0ccb267`** with this sub-step's tree, over `cargo test -p ezsdr-radio-uhd --lib`.
  - Required: zero differing files on the stable set.
  - Report the stable-set size and the unstable tests. E2a-1 found three that are unstable at base: `ur_22_underflow_close_failure_reopens_before_the_next_payload`, `ur_23_p1_p2_each_owner_raises_its_own_thread_once_before_its_first_send` and `ur_23_p5_p6_the_success_is_counted_per_owner`.
- Copy builds use `CARGO_INCREMENTAL=0` and a target directory outside the tree. Delete it afterwards.

## Mutation rows

- Re-anchor every row your edit breaks (expect ≤ 30, plan §3.2), with the same program change and in the same Provider.
  - Never move a UHD-only or Mock-only row into shared `staging.rs` code.
  - Do not make two rows identical.
  - Check every `new` text compiles against the new code. In E2a-1, four rows still named `.busy` in `new` and failed to compile.
- Add the 10 rows of plan §3.2 and the 2 Mock rows. Each names its killer in `filter`, and that killer must run the mutated path.
- You cannot run rows. The orchestrator runs `remote_mutate.py all` after your report and sends failures back.

## Before reporting

- `cargo fmt --all`.
- `python3 plan/maintenance/tools/remote/rdev.py -- cargo test --workspace`, in the foreground: green.
- `rdev -- cargo clippy --workspace --all-targets`: clean.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 0ccb267` = 0.

## Report

- `git diff --stat`.
- The book API added.
- The send split: which lines run under which lock section.
- Every existing test touched, and what changed in it.
- The F7 findings and their fixes.
- The trace comparison result.
- Every mutation row added or re-anchored, by id, with what changed.
- Anything in the plan that proved wrong against the code, with evidence.

Do not commit, push or `git add`. Do not edit AGENTS.md, handoff.md, plan/phase8/49-*.md, bench-results.md or bench/.
