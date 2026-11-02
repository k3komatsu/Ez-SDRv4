# Brief: issue #76 — six tests whose verdict depends on when the test thread runs

You are the implementer (AGENTS.md §9). Repository `~/local_work/Ez-SDRv4`, branch `main`. Edit in place: you are the only editor of the tree. Read AGENTS.md first. Do not cd into `v3/`. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`** and do not search for it.

The tree holds uncommitted note-45 E2a work under `crates/ezsdr-radio-uhd/src/brain/`, `src/host.rs` and `plan/maintenance/tools/mutations.json`. Do not touch any of it.

## Why
Issue #76 (`gh issue view 76 --comments`). Six tests fail at load average ~17 (sim02, sim03, this PC), and each failure blocks the gate. A test that fails under load also counts a mutant as killed and hides a survivor (AGENTS.md §7). The mechanism is `mech:wall-clock-test-oracle`. Its design note is `plan/phase8/37-test-thread-timing.md`, and its rule is written in `crates/ezsdr-radio-uhd/tests/README.md`: **a test's verdict never depends on when the test thread runs.** These tests were written after note 37, or were missed by its cleanup. Follow the rule's four options exactly:
1. book in the Spec before the start;
2. an assert that holds however far time moves;
3. `send_asap_and_flag` where a delay must not matter;
4. a window-precise property moves to a controlled-clock unit test, with `Frozen` (`src/provider/test_support.rs`) held across the window.

"Booking further ahead" only lowers the probability, and it does not count.

## The tests
| Test | Where | Seen failing |
|---|---|---|
| `provider::tx::tests::ur_22_a_failed_one_sample_padded_eob_is_not_safe_until_flush_recovery` | `src/provider/tx.rs` (~6138) | "the next burst follows the flush" |
| `provider::tx::tests::ur_22_a_failed_eob_close_recovers_with_flush_before_the_next_timed_burst` | `src/provider/tx.rs` (~6074) | with the one above |
| `provider::control::tests::ur_24_a_unbegun_successor_does_not_extend_the_old_chain_drain` | `src/provider/control.rs` (~7977) | assert ~8116 |
| `provider::rx::tests::ur_24_fake_ddc_distinguishes_host_silence_from_a_stalled_ddc` | `src/provider/rx.rs:1520` | assert 1533, `Silence` case: pending 1, expected 0 |
| `h4_fake_session_records_timed_rx_gain_and_tx_submission_proxies` | `tests/fake.rs` (~9205) | assert ~9227 |
| `h4_counts_a_timed_tx_sample_clock_underflow_and_excludes_later_gain_updates` | `tests/fake.rs` (~9264) | assert ~9271 |

What the issue suspects, INFERRED:
- the `ur_22_a` tests drop the frozen clock before the next booking, so device time passes the burst's instant and `DropAndFlag` drops it;
- `ur_24_fake_ddc` drops `Frozen` before an `rx_recv` that waits 1 ms of wall time;
- the H4 tests run on a Real clock by design.

Confirm or correct each cause from the code before changing it.

## Do
1. For each test, find the window: what the test thread reads or does, and the later operation or assert that time can overtake. Pick one of options (1)–(4) and apply it. Keep what the test proves: its name, its sequence, and the property its assert names. If a property can only be pinned with a controlled clock, move that property into a unit test and keep the integration test for wiring, as stages 37b and 37c did.
2. The H4 tests measure proxies on a Real clock. If their verdict can be made independent of the test thread's timing with option (2), do that. If the Real clock is essential to what they measure, say why, and make the assert one that holds under any scheduling delay (for example, count the entries the proxy must record instead of a timing bound). **Do not delete them.**
3. Find the same pattern near these tests: the same file, the same helper, the same `drop(frozen)` followed by a later wall-clock wait or booking. List each one you find, and fix it the same way, or say why it is safe. Do not sweep other files.
4. Mutation rows whose filter names one of these tests: they must still be killed for the same reason. Find them with `grep -n '<test name>' plan/maintenance/tools/mutations.json`. Rows P1150 and P1158 (the H4 rows) misbehaved under load. List the rows affected. Retarget a row only if a test moved. Never re-serialize the file.

## Checks
- Run the six tests 20 times each under load, with `stress-ng` or 16 busy loops (`for i in $(seq 16); do (while :; do :; done) & done`, then kill those PIDs by number when done). Note 37a ran its check this way. Record the failures, which must be 0, and the load average.
- Run `cargo test -p ezsdr-radio-uhd` (lib and the `fake` test binary), `cargo fmt --all`, and `cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings` clean.
- Then `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD` (0 problems).
- Never run `mutate.py` or `remote_mutate.py`. The orchestrator runs the rows.
- The self-review is waived. Sol reviews the code.
- Never git commit, push, stash, reset or checkout.

## Out of scope
- Production code. If you think a test failure is a production bug, stop and report it with the evidence.
- FakeDevice behaviour.
- `design/`.
- Anything under `src/brain/`.

## Report (at most 20 lines)
- Per test: the window found, the option applied, and what changed.
- The nearby cases from item 3.
- The rows affected.
- The 20× results under load, with the load average.
- Suite results and clippy.
- Lines changed.
- Anything in this brief you think is wrong.
