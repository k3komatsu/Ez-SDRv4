# Brief for Sol: check the E2b measurement brief before any code, read-only

You are the independent reviewer of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, build nothing, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.** Another agent is editing `crates/ezsdr-radio-uhd/src/brain/` right now: read it as work in progress.

## What to check
The orchestrator's brief for the speed measurement is `~/.cache/ezsdr-fixes/2026-11-01-n45/brief-e2b.md`. Its basis:
- the facts in `~/.cache/ezsdr-fixes/2026-11-01-n45/e2b-speed-facts.md`;
- note 45 `plan/phase8/45-uhd-decision-core.md` §4.5, §4.8 and §7's "速さ" criterion;
- the plan `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md`, its §2 and its §10 last rows.

First write, from note 45 §7 and the code, the ways this comparison could produce a misleading verdict: an unfair harness, a definition that hides lateness, updates that drop out, load bias, a FakeDevice that cannot keep up, a probe that perturbs one owner more than the other, timer resolution, and anything else. Only then read the brief, and say for each way whether the brief prevents it.

Check against the code:
- **The delay definition:** apply entry minus the host instant of `e − window`, the same for both owners. Does it hide anything §7 asks for?
- **"Applied":** the FakeDevice DSP log at tick `e`, from `device.rs` `drain`/`dsp_log`.
- **The update table:** spaced, and bursty (4 updates 100 µs apart every 10 ms). Is it within the 16-command queue, the 32-entry DSP FIFO, the booking floor and the lead (`provider/control.rs:624-674`)? Can today's owner refuse or move some updates by design (a late instant, a full queue), so that the verdict fails for a reason the comparison does not mean?
- **The rate trial and the fallback** ("the 200 MS/s block rate" with smaller blocks).
- **Where the new runtime's release comes from** (K's timer at `e − window`): is it equivalent to today's owner holding updates and releasing them at its window?
- **Interleaving and the host choice.**
- **Statistics:** at least 10,000 updates per condition, and the p99.9 index.

## Output
- Verdict: PASS or REVISE.
- Findings P1/P2/P3. Each has `path:line` and a concrete way the verdict would mislead.
- At most 70 lines.
- End with "Not checked:".
