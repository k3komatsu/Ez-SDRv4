# Holdout errors for the design-note-45 experiment (read-only)

You are an independent reviewer. Do not edit any file. Repository: ~/local_work/Ez-SDRv4 (main). Do not cd into v3/.

Design note 45 (`plan/phase8/45-uhd-decision-core.md`) proposes a pure decision core ("判断役", `uhd-brain`), per-stream acceptance cells and executors for the UHD Module, and an experiment (§7) on the receive slice: the receive chain and control's timed DDC handoff, loss, stop, streamer close/open, channel move. One success criterion (§7, "独立に選んだ誤り") is yours:

> Before implementation, Sol selects from the design note (not the code) 20 behavioural errors to inject into the decision core and cell procedures, and freezes them (errors that only disable an oracle are excluded). Without changing externally visible assertions, the DST (deterministic simulation testing) must find 16 or more.

The implementation does not exist yet, and the implementer will NOT see your list (it is a holdout set). Read note 45 (all of it), and for the semantics it builds on, note 44 §3 (`plan/phase8/44-device-chain-state.md`), note 39 §6.6–§6.8 (`plan/phase8/39-dsp-timed-commands.md`) and spec 18 UR-24 (`design/18-uhd-radio.md`). Do NOT read `crates/` (the errors must come from the design, not from today's code).

Pick 20 errors an implementer could plausibly make when writing the receive-slice core from the note. Spread them: procedure 1 (reservation conditions: phase, channel, generation, origin, plan cut, executor progress; take/cancel; result write), the Stopping transition, procedure 2 (Stop shortens the cut against executor progress), procedure 3 (epoch advance, quiescence, retiring-epoch cut for stale buffers, publish+advance in one section), procedure 4 (failure flag order, executor checks, panic guard, handle retention), pinning (§4.4: no restore/move/close/open/next segment while a command is taken or maybe-applied), the five command states and exactly-once settlement, partial-failure-as-maybe-applied, drain deadline and late completion (`left`), native lock (§4.7), bounded in-flight values and held queue, consumption rule (note 39 §6.8: eligibility at acceptance, one value per eligible sample, equal-time values on separate samples). Prefer subtle ones (an off-by-one in a bound, a check done outside the section, a state forgotten in one transition) over gross ones.

For each error give:
- id H01..H20, area (from the list above)
- the wrong behaviour, stated precisely enough to inject into ANY implementation that follows the note (what condition/transition/order changes), not tied to code names
- a minimal operation sequence (pseudocode is fine) and the observable wrong outcome (a safety invariant broken or a progress property violated; name it)
- which of the note's six exhaustive cases (§7 cases 1-6) or the seeded search should expose it, or "none of the six" if you think the cases miss it (that is useful: say which case would be needed)
Exclude errors that only weaken a check/assertion/oracle. Then list, in one paragraph, any area of the slice where you think the six cases as written give weak coverage.
Output in English.
