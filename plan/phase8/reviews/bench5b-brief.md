# Brief: bench additions for Monday — the transmit-rate probe's onset quantization, and the ssh drop

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `808a870`, tree clean. Nobody else edits the files this brief gives you. **Not risky** (bench-only test code, a bench script check, and the runbook; production code does not change). The rules of `~/.cache/ezsdr-fixes/2026-11-07-bench5/brief.md` §「Rules」 apply unchanged (no `src/` change, RF bound, pure-function VERDICTs with unit tests, mutation rows probed in a throwaway copy outside the repository, rdev for `--features uhd`, hardware tests compile-checked only, no background jobs, end checks).

Read: `AGENTS.md` (Safety core, §7, §9), `plan/phase8/bench-results.md` §「2026-11-05」 and §「2026-11-06」 (the (d) transmit-rate and (g) ssh-drop lines), `plan/phase8/bench.md` §「2026-11-06 の手順（note 48 の後）」 blocks (d) and (g) and the last section 「<DATE> の手順（note 47 段階 5 の後）」, `crates/ezsdr-radio-uhd/tests/hardware.rs` `hw_n47_raw_tx_rate_on_an_open_streamer` (around line 3328) and `tone_start` (around line 743), `tests/common/n47.rs` `tx_rate_verdict`, `plan/phase8/tools/ssh-drop-tx.sh` and `ssh-drop-driver.py`.

## T1. The transmit-rate probe: onset quantization (orchestrator's reading; check it)

On 2026-11-06 #2 gave INCONCLUSIVE three times: B's onset was −1 200, −2 200 and +2 200 ticks from the expected `b + delay2` (tolerance 1 000 ticks = 5 samples at 1 MS/s receive). The orchestrator's reading (VERIFIED for the code, INFERRED for the numbers): `tone_start` (`hardware.rs:743-753`) cuts the capture into fixed 25-sample windows from the start of the read (`chunks_exact(25)`) and returns the start of the first of three tone windows, so its resolution is 25 samples = 5 000 ticks, coarser than the tolerance. `control_r2` and `b` fall at unrelated phases of that window grid, so `b_onset − (b + (onset2 − control_r2))` can differ by up to ±25 samples from the true difference.

Confirm or refute this from the code (cite `path:line`). If it holds, fix the test, not the tolerance: place `control_r2` and `b` at the same phase of the 25-sample window grid relative to the read's first sample (say how the grid origin is known — the receive start `rx_start` and the first block's tick; if the first block can start off `rx_start`, take the grid from the samples actually read), so the quantization is equal and cancels in the difference. Do the same for `control_r1` if a verdict uses it. Keep the tolerance at 1 000 ticks and keep the INCONCLUSIVE when the onset is still off. Make the phase alignment a small pure function with a unit test (two instants put on the same phase; a grid origin off `rx_start`) and a mutation row on it. Add a FakeDevice rehearsal only if FakeDevice models enough of the raw calls; say whether it does.

## T2. The ssh drop script still matches the code

`plan/phase8/tools/ssh-drop-tx.sh` and `ssh-drop-driver.py` were written before note 47 stages 4 and 5. Without hardware and without ssh, check that they still match the current code: the Python client API they call (`python/ezsdr/`, connect with transmit channels, the repeat call, close), the profile they build (Module, profile and layout names the server accepts today; compare with the Rust bench helpers and `crates/ezsdr-server`), the server binary and image paths, and the Manifest field `client_disconnect` C1 reads. Run what can run without a device and without ssh (shell `bash -n`, Python byte-compile with `PYTHONDONTWRITEBYTECODE=1 python3 -c "import ast,sys; ast.parse(open(sys.argv[1]).read())" <file>`, the driver's profile construction if it is a pure function, and the server's profile validation against MockRadio or a dry `validate` path if one exists — say which). Fix only what is stale, citing the change that made it stale. The script reaches the server with `ssh localhost`; the PC's sshd listens on port **10022**, not 22: do not hard-code a port in the script; the runbook says the owner sets `Port 10022` for `localhost` in `~/.ssh/config` (the orchestrator arranges it).

## T3. The runbook

In the last section of `plan/phase8/bench.md` (「<DATE> の手順（note 47 段階 5 の後）」), add after (g):
- **(h) 送信の rate**: `hw_n47_raw_tx_rate_on_an_open_streamer`, #2 then #1, with the attenuator line; expected VERDICT; the acceptance of the transmit rate on an open streamer closes only on PASS on both boards; note that accepting transmit rate changes still needs its own design stage (note 47 §4 item 4: the clock on a refused setting) and the time and booking inventory may remove the feature.
- **(i) 送信中の ssh 切断**: as the 2026-11-06 (g) block, updated for T2; preconditions: `ssh -o BatchMode=yes localhost true` succeeds (sshd on port 10022 through `~/.ssh/config`), the owner names the independent receiver for C2 or C2 is NOT DONE; the commit placeholder; the expected three VERDICT lines.
Update the time table and the「流した後」list.

## Self-review (one round)

`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --agent plan --auto "<checklist>" < /dev/null`, foreground; checklist: (1) T1's reading confirmed or refuted with evidence, the fix aligns the phase without widening the tolerance, the pure function is unit-tested and its row breaks the test; (2) T2's checks cover the API, profile, paths and C1's field, each stale item fixed with its cause; (3) the runbook's commands name tests and scripts that exist and every transmit block has the attenuator line; (4) no check weakened; (5) maintainability. On a transport error retry once, then report and stop.

## Report (at most 20 lines)

T1: the reading (confirmed/refuted, evidence), the change (`path:line`), its test and row, probe outcome. T2: each check run and its result, each stale item and fix. T3: the blocks added. Suites and `rules.py`. Self-review. Deviations.


---

# Brief: the ssh drop's independent receiver — a receive monitor on #2

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `808a870`, with the uncommitted changes of the previous run (`~/.cache/ezsdr-fixes/2026-11-08-bench5b/brief.md`; its report is at the end of `impl.log` in the same directory) and the orchestrator's edits to `plan/phase8/bench.md` and `bench-results.md` (the "two boards not at once" rule removed; keep them). Nobody else edits the files this brief gives you. **Not risky.** The Rules of `~/.cache/ezsdr-fixes/2026-11-07-bench5/brief.md` §「Rules」 apply unchanged.

## Why

The ssh drop (`plan/phase8/tools/ssh-drop-tx.sh`, runbook (i) in the last section of `bench.md`) needs an independent receiver for C2: a command (`EZSDR_DROP_RX_CHECK`) whose last output line is `tone` or `quiet`, run at most 10 s per call. The owner's set-up (2026-11-08): #1 (OBX, `addr=192.168.40.36`, `enp1s0f1np1`) transmits through the ssh path; its transmit output goes through a **splitter** to #1's own receive (so C3 stays as it is) and, attenuated (≥ 30 dB), to #2's receive (UBX slot A, `addr=192.168.47.37`, `enp1s0f0np0`). #2 runs directly, not through ssh, and both boards are connected at once. Opening a device on every call would not fit the 10 s budget, so #2 receives continuously and the check only reads its latest state.

## Tasks

**M1. The monitor: `hw_rx_tone_monitor`** (an ignored hardware test in `hardware.rs`, raw device like `hw_noise_floor_receive`; `pin_slot_a`). Receives slot A at 1 MS/s at the ssh drop's carrier (read the frequency and the tone offset from `ssh-drop-driver.py`, cite them; `EZSDR_MONITOR_HZ` may override). Every 200 ms of samples it decides `tone` or `quiet` for the drop's tone with one pure function (reuse the coherence/power judgement `hw_noise_floor_receive` or `tone.rs` uses; the noise floor is taken from the first second, which the runbook keeps quiet — say how a tone already present at start is handled: refuse to start, with a message). It appends one line `<unix ms> <tone|quiet>` per decision to the file `EZSDR_MONITOR_OUT` and flushes; it runs for `EZSDR_MONITOR_SECONDS` (default 180) and stops the stream cleanly. An overflow or gap inside a 200 ms block makes that line `unknown`. The pure decision function has unit tests (tone, quiet, a block with a gap) and mutation rows on its decisive comparisons, probed in a throwaway copy as the Rules say.

**M2. The scripts** in `plan/phase8/tools/`:
- `rx-monitor.sh`: starts M1 in the image (`ezsdr-v4-dev:uhd4.10`, the same `docker run` options as the runbook's `hw()`, plus a mount of the output directory so the state file is visible on the host), in the background, with a container name and a pidfile; prints the state file's path; refuses to start if the state file already exists. A `stop` sub-command stops the container by its name.
- `rx-monitor-state.sh <file>`: prints the last state (`tone` or `quiet`) and exits 0 only when the file's last line is at most 1 s old; otherwise prints `unavailable` and exits non-zero. Shell only, no device.
Check both with `bash -n`, and the state script with a small self-check against fabricated files (fresh tone, fresh quiet, stale, `unknown`, empty).

**M3. The runbook (i)** in the last section of `bench.md`: the splitter wiring (#1 TX → splitter → #1 RX and, ≥ 30 dB, #2 RX), the attenuator line, the order: build first (`hw hw_rx_tone_monitor` compile-only is not possible, so say how the image's target is warmed — e.g. `cargo test --no-run` in the image), start the monitor on #2 with #1 silent, wait until `rx-monitor-state.sh` prints `quiet` (fresh), run `ssh-drop-tx.sh` with `EZSDR_DROP_RX_CHECK='plan/phase8/tools/rx-monitor-state.sh <file>'`, then stop the monitor. Expected C1, C2, C3 lines. Remove the "C2 NOT DONE without a receiver" fallback as the plan (keep it as the rule if the monitor fails to start).

## Self-review (one round)

`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --agent plan --auto "<checklist>" < /dev/null`, foreground; checklist: (1) M1's decision is a unit-tested pure function and its rows break its tests; the floor and the start-with-tone case are handled; (2) M2's freshness rule and exit codes match `ssh-drop-tx.sh`'s use of `EZSDR_DROP_RX_CHECK`; (3) the runbook's order keeps #1 silent until the monitor reads quiet, every transmit block has the attenuator line; (4) no transmit added; (5) maintainability: helpers reused. On a transport error retry once, then report and stop.

## Report (at most 15 lines)

M1–M3: files, functions, tests, rows and probes; the carrier and offset with their source; suites and `rules.py`; self-review; deviations. Your own process is the `opencode` run on this brief; the orchestrator's watchdog runs beside it.


---

# Brief: bench additions for Monday — fix round 1 (Sol code review)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `808a870`, with the uncommitted changes of the previous runs (briefs `~/.cache/ezsdr-fixes/2026-11-08-bench5b/brief.md` and `brief-t4.md`, reports at the ends of `impl.log` and `impl-t4.log`). The Rules of `~/.cache/ezsdr-fixes/2026-11-07-bench5/brief.md` §「Rules」 apply unchanged. Sol's code review is at the end of `~/.cache/ezsdr-fixes/2026-11-08-bench5b/sol-code.log`: read it. All five findings are adopted:

1. **Quiet noise can refuse the monitor's start** (`tone.rs:350–367`). The start refusal must mean a sustained signal, not sparse coherent noise windows: e.g. refuse only when a large share of the first second's windows carry the drop tone (coherent **and** at the drop tone's frequency within a tolerance) — choose the rule, state it, and test it against ordinary Gaussian noise and sc16-quantized noise near 0.5 LSB (a deterministic seeded generator in the test), and against a real tone.
2. **C3 can fail a correctly stopped transmitter** (`hardware.rs:3595–3602`, `hw_noise_floor_receive`). Use a noise-qualified tone decision: count a window as tone only when it is coherent **and** at the drop tone's frequency (the ssh drop's 100 kHz; read it where the monitor reads it) — reuse the monitor's pure decision rather than a second copy. Keep the 1 % bar unless the tests show it must change; test ordinary and quantized noise (pass) and a tone (fail).
3. **Fresh output can describe old samples** (`hardware.rs:3629–3649`, `rx-monitor-state.sh:18–23`). Each 200 ms decision compares its last sample's device time with the device's time when the line is written; if the lag exceeds a bound (state it, e.g. 300 ms) the line is `unknown`. Pure function, unit-tested (on time, delayed).
4. **The stop bypasses the clean receive shutdown** (`rx-monitor.sh:44–47,56–58`; `hardware.rs:3628,3652–3653`). Add a cooperative stop: the monitor checks for a stop file (`EZSDR_MONITOR_STOP`, a path next to the state file) every block, then `rx_stop`/close and exits; `rx-monitor.sh stop` creates it, waits for the container to exit (bounded, e.g. 10 s), and only then stops the container by name, saying so. The runbook's stop step uses it.
5. **The splitter drawing omits the attenuation on #1's receive branch** (`bench.md:992`). Draw ≥ 30 dB on both receive paths (or before the splitter), consistent with the attenuator line and `ssh-drop-tx.sh`'s confirmation.

Rows: add rows for the new decisive comparisons (sustained-signal refusal, frequency qualification, lag bound, stop check if it is a pure decision), retarget changed ones, probe each in a throwaway copy outside the repository. Self-review: one round as before (Go DeepSeek, plan agent, foreground, `< /dev/null`), checklist narrowed to these five plus "no check weakened" and "maintainability"; on a transport error retry once, then report and stop.

Report (at most 15 lines): per finding, the change (`path:line`) and its test; rows and probes; suites and `rules.py`; self-review; deviations.


---

# Brief: bench additions for Monday — fix round 2 (Opus re-check)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `808a870`, with the uncommitted changes of the previous runs (briefs `brief.md`, `brief-t4.md`, `brief-fix1.md` in `~/.cache/ezsdr-fixes/2026-11-08-bench5b/`; your last report is at the end of `impl-fix1.log`). The Rules of `~/.cache/ezsdr-fixes/2026-11-07-bench5/brief.md` §「Rules」 apply unchanged. The re-check report is `~/.cache/ezsdr-fixes/2026-11-08-bench5b/opus-recheck1.md`: read it whole. All findings are adopted with its recommendations:

1. **C3 (major):** in `hw_noise_floor_receive` only, a window counts as tone only when `is_drop_tone_window(w)` **and** its mean power exceeds an absolute floor of amplitude 4 LSB (`(4.0/32768.0).powi(2)` in fc32; name the constant and cite the reasoning in one line: the loopback tone is about −49 dBFS, roughly 30 dB above it, INFERRED from `bench-results.md:27-28`). Make the decision a pure function; unit tests at σ 0.3 and 0.4 LSB quantized noise (seeded, 800 windows: they fail before the fix — confirm that — and pass after) and a tone at the loopback level (counted). Print the largest window amplitude in C3's verdict line. Add one line in the runbook (i) that C3 relies on the Session's default 0 dB receive gain.
2. **Minor:** the frequency bound in `is_drop_tone_window` (`tone.rs:348`) becomes `<= 20e3` so the 125 kHz quantization point falls outside; correct the comment at `tone.rs:341-344` (coherent noise windows pile up at 125 kHz, not at random frequencies). Re-check the monitor's tests still hold.
3. **Minor:** in `hw_rx_tone_monitor`, create the state file before `raw_rx`, and turn the write `unwrap`s (`hardware.rs:3657-3658`) into a path that still runs `rx_stop`/`rx_close` before failing.
4. **Minor:** runbook `EZSDR_MONITOR_SECONDS=3600` (the cooperative stop makes a long run free), and the guard at `bench.md:1027` prints `monitor not quiet: (i) not run` when it refuses.

Rows: one on the amplitude floor (killed by the σ 0.3/0.4 tests), retarget the frequency-bound row if its text changed; probe each in a throwaway copy outside the repository. No self-review this round (the orchestrator's re-check follows). End checks as in `brief.md`. Report (at most 12 lines): per item, the change (`path:line`) and its test; the before/after of the σ 0.3/0.4 tests; rows and probes; suites and `rules.py`.
