# #96 — fix-round briefs

## Brief — #96 fix round after Sol's code review

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n96`, HEAD `93b4d57` (your #96 change, committed). Uncommitted orchestrator files (handoff.md, plan/phase8/50-remaining-work.md) are not yours: do not touch them. Read /home/komatsu/.cache/ezsdr-fixes/2026-11-11-n96/brief.md (its rules bind, including rdev and the 120 s shell limit — use `rdev.py --host sim02`, sim03 is busy with another task), your report.md, and Sol's review sol-code.md. Fix only these (orchestrator adopted, 2026-11-11):

1. **Major 2 — one receipt for two switches** (`o1_uhd_at` taking an iterator, ~parity.rs:2598–2600, caller ~:2189). Separate `push` calls are not one receipt: a 50 ms test-thread pause between them lets the first switch apply. Hand the batch to the Provider in one receipt (find how a single receipt of several Actions is made — the existing queue/receipt path, a Provider or rig API that takes several Actions at once; do not add production code). If no test-side way exists, say so and stop on this item.
2. **Major 1 — `s15_parity_provider_stop_cancels_a_pending_switch`** (~:2282–2312): a 5 s pause between the switch's receipt and `Provider::stop` reads `applied`. First search the frozen/unit rigs (`crates/ezsdr-radio-uhd/src/provider/tx.rs` and `tx/tests/`, `control.rs` tests, Mock tests) for a test that already pins "a switch still pending at Provider::stop is cancelled and never sent" on each Provider; name it. If both Providers have one: make the paired test judge only when it observed that the Stop came before the switch's boundary (read the device time / the records after the fact, as `s15_parity_s4b_three_bookings_...` skips itself when its precondition is not established), and otherwise return as "not established" with a printed reason. If one is missing, add it on the frozen rig (UHD) or the stepped Mock, then do the same to the paired test.
3. **Minor (pre-existing, Sol)** — the wrong-policy test drains events at ~:2233 and checks a second drain at ~:2236–2237, so its "no TIME_ERROR" assertion cannot fail: check the TIME_ERROR absence on the same drained events.

Verify with the implementer's injection (a sleep after `wait_until` in `submit`, `submit_untimed`, `s14_submit`, `o1_uhd_at`) at 5 ms, 50 ms and 200 ms, plus Sol's two probes: 50 ms between the two switch pushes (now impossible) and 5 s after `o1_uhd_at` in the stop test (now "not established", not a failure). Remove all probes before reporting. Unloaded parity binary and `rdev --host sim02 -- cargo test --workspace` once. `cargo fmt --all` and `--check`; `rules.py --base fe86629` 0 problems; rows touched re-anchored and killers checked in a copy outside the repository. Do not commit.

Append "Fix round after Sol" to report.md; your final message is its summary.

## Brief — #96 last minor fixes (after the Opus re-check)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n96` (HEAD `93b4d57` plus your uncommitted fix round). Uncommitted orchestrator files (handoff.md, plan/phase8/50-remaining-work.md) are not yours. Rules of brief.md bind (rdev `--host sim02` only; long shell timeouts; no commit). Read the re-check /home/komatsu/.cache/ezsdr-fixes/2026-11-11-n96/recheck.md and fix only:

- **M1** `s15_parity_provider_stop_cancels_a_pending_switch` (~parity.rs:2295–2322): judge only when the switch's boundary is past the Stop's *bound* — the stop row's `at` (Manifest timing section) plus the in-flight window plus the end margin, as `Provider::stop` computes it (mod.rs:694–700, core.rs:627–641) — with a stated margin; otherwise "not established". Say in the comment exactly what makes a judged run sound. Use the existing records; no production change.
- **M2** The Mock half is deterministic: run its assertions unconditionally; only the UHD half may be skipped. Fix the comment.

Check: the test still judges in ordinary unloaded runs (5 runs via rdev), and is "not established" with a 5 s sleep after `o1_uhd_at` (probe removed afterwards). `cargo fmt --all --check`, `rules.py --base fe86629` 0 problems. Append "Last minors" to report.md; final message: its summary.
