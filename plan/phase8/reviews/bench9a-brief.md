# Brief — step A after the 2026-11-09 bench (test and tool changes only; not risky)

Repository: /home/komatsu/local_work/Ez-SDRv4 (branch `main`, HEAD `2bb3d1d`). Read AGENTS.md "Safety core" and §7 first.

## Rules (binding)

- Change only the files named below. Production code (`crates/*/src/**`) is not touched.
- Do not commit, push, or run `git add`. Do not run `mutate.py` anywhere, never apply a mutant to the tree, do not run `remote_mutate.py` (the orchestrator runs mutation rows).
- Builds and tests run remotely: `cd /home/komatsu/local_work/Ez-SDRv4 && python3 plan/maintenance/tools/remote/rdev.py -- <cargo command>` (wait in the foreground until it ends; it may take several minutes — use a long timeout). Do not run cargo locally.
- Before reporting: `cargo fmt --all` (local is fine: it only formats), then `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` must print 0 problems.
- Temp files only under /tmp. Kill processes by PID, never by name.

## Changes

### 1. The repeat switch verdict looks only near the boundary (bench item (e))

`crates/ezsdr-radio-uhd/tests/common/iii.rs`, `switch_verdict` (around line 503–550). Today the "changes elsewhere" scan
`trial.tone.iter().find(|(t, hz)| (*t < b - m && …) || (*t >= b + m && …))` looks at every tone window of the whole capture, so on the bench it caught windows before the first repeat started and after the Stop (LO leakage near 0 Hz, 19–58 ms from the boundary) and failed a correct switch.

- Restrict that scan to windows with `b - 5 * TICKS_PER_MS <= t < b + 5 * TICKS_PER_MS` (the same span the `before` / `after` counts already use). Keep the transition exclusion `[b - m, b + m)` and every other check unchanged.
- Update the doc comment of `switch_verdict` to say the tone is checked within 5 ms on each side of the boundary.
- Add one unit test next to `a_tone_off_its_frequency_away_from_the_boundary_fails` (same `mod tests`, same helpers `tone()`, `power()`, `trial()`): a clean switch plus two extra tone windows outside the span — one at `B - 20 * TICKS_PER_MS` with 491_925.0 Hz and one at `B + 30 * TICKS_PER_MS` with 0.0 Hz — must still be `Verdict::Pass`. Name it `windows_far_from_the_boundary_do_not_fail_the_switch`. The existing test `a_tone_off_its_frequency_away_from_the_boundary_fails` must keep passing (its off window is inside the span).
- Add one mutation row to `plan/maintenance/tools/mutations.json` (append at the end of the array, same shape as the existing row `N48III-01`, `"test_bin": "fake"`, `"crate": "ezsdr-radio-uhd"`), id `N49A-01`, that removes the new span restriction (its `old` is the smallest unique source text of the new restriction, `new` the text without it), with `"filter": "windows_far_from_the_boundary_do_not_fail_the_switch"`. Check that `rules.py` finds its `old` exactly once.

### 2. Print the tone windows of each switch trial

`crates/ezsdr-radio-uhd/tests/common/n47s5.rs`, `repeat_switch_verdict` (around line 227). After the existing `println!("T4 switch: …")`, print one more line: `T4 switch tone windows near the boundary: [(tick − boundary in µs, Hz), …]` for the tone windows within 10 ms of the boundary (boundary = `origin + boundary * per` plus `delay` when both are known; skip the line when either is unknown), and the count of tone windows farther away with their minimum and maximum Hz. Keep it to one line.

### 3. A send trace for bench item (c)1 (issue #81)

`crates/ezsdr-radio-uhd/tests/hardware.rs`, `hw_n47s5_front_after_a_short_burst` (around line 3682), and `common/n47s5.rs` `front_trial` / `FrontTrial` if needed. Wrap the device of each trial in a `ProbeDevice` exactly as `hw_n47s5_repeat_switch` does (`Arc::new(probe_device::ProbeDevice::new(device.clone()))`, pass `probe.clone()` to `tx_session_at`). After each trial's verdict, print one line per transmit data send of that trial (use `n47s5::data_sends(&probe)`):
`T2 send: at <at>, samples <sample_counts>, sob <sob>, eob <eob>, accepted <result>, host_before_ns <…>, host_after_ns <…>` — prefix each line with the trial's claim text so lines are attributable. Keep the verdict logic unchanged.

### 4. `ssh -n` in the ssh-drop script (bench item (i))

`plan/phase8/tools/ssh-drop-tx.sh` line 60: the pre-check `ssh -o BatchMode=yes … localhost true` reads the script's stdin, so the attenuator confirmation `yes` piped to the script is consumed before `read -r ATT_CONFIRM` (line 66). Add `-n` to that `ssh` only. Do NOT add `-n` to the wrapper's `exec ssh -T …` (line 136): the server protocol runs over its stdin. Check whether any other command before line 66 can read stdin and say so in the report.

## Checks to run (remote, foreground)

1. `python3 plan/maintenance/tools/remote/rdev.py -- cargo test -p ezsdr-radio-uhd --test fake switch` (the switch verdict unit tests, including the new one) — then the whole `--test fake` once.
2. `python3 plan/maintenance/tools/remote/rdev.py -- cargo test -p ezsdr-radio-uhd --features uhd --test hardware --no-run` (the hardware tests compile).
3. `bash -n plan/phase8/tools/ssh-drop-tx.sh`.
4. `cargo fmt --all`, then `rules.py --base HEAD` = 0 problems.

## Report (short)

Files changed with a one-line summary each; the exact `old`/`new` of N49A-01; the test results (counts); anything you could not do. Do not paste whole files.
