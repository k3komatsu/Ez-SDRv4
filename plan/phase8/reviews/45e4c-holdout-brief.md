# Holdout errors for the second round (E4) of the design-note-45 experiment (read-only)

You are an independent reviewer. Do not edit any file. Repository: /home/komatsu/local_work/Ez-SDRv4 (main). Do not cd into v3/.

Design note 45 (`plan/phase8/45-uhd-decision-core.md`) proposes a pure decision core ("判断役"), per-stream acceptance cells and executors for the UHD Module, and an experiment (§7) on the receive slice. One success criterion (§7, "独立に選んだ誤り"):

> Before implementation, Sol selects from the design note (not the code) 20 behavioural errors to inject into the decision core and cell procedures, and freezes them (errors that only disable an oracle are excluded). Without changing externally visible assertions, the DST must find 16 or more.

The first round used a holdout you chose on 2026-11-01; it is spent and published in `plan/phase8/reviews/45e3-holdout.md`. This is the second round (E4). Its design changes, made since then and reviewed by you, are in note 45 (the paragraph 「上限は二つ」 at the end of §4.5, the paragraph 「実行役の生成の失敗」 in §4.9, and the last entries of §11). E4's model will also cover loss recording, native handle destruction, the shutdown join timeout and an executor spawn failure.

**Read:** note 45 (all of it), note 44 §3 (`plan/phase8/44-device-chain-state.md`), note 39 §6.6–§6.8 (`plan/phase8/39-dsp-timed-commands.md`), spec 18 UR-15, UR-24, UR-29 (`design/18-uhd-radio.md`), and `plan/phase8/reviews/45e3-holdout.md` **only to avoid repeating its 20 errors**.
**Do NOT read:** `crates/` (the errors must come from the design, not from code), `plan/phase8/45-report.md`, `plan/phase8/reviews/45e2*`, `plan/phase8/reviews/45e3-*` other than `45e3-holdout.md`, `plan/phase8/reviews/45e4-*`, and anything under `~/.cache/ezsdr-fixes/` (they describe what the DST found and missed; the new set must not be chosen against that).

Pick 20 **new** errors (none the same as a first-round error; a different error in the same area is fine) that an implementer could plausibly make when writing the receive-slice core from the note as it now stands. Spread them over the whole slice at your own judgement: procedure 1 (reservation conditions, take/cancel, result write), the Stopping transition, procedure 2, procedure 3, procedure 4 (failure flag and loss recording order, executor checks, panic guard, handle retention and destruction), pinning, the five command states and exactly-once settlement, partial failure, drain deadline and late completion (`left`), the native lock (§4.7), the two bounds K and D and the held queue, the consumption rule (note 39 §6.8), lifecycle (§4.9: stop, join timeout, detach, spawn failure). Prefer subtle ones (an off-by-one, a check outside its section, a state forgotten in one transition, an event used as the wrong evidence) over gross ones.

For each error give:
- id N01..N20, area;
- the wrong behaviour, stated precisely enough to inject into ANY implementation that follows the note (what condition, transition or order changes), not tied to code names;
- a minimal operation sequence (pseudocode is fine) and the observable wrong outcome (a broken safety invariant or progress property; name it).

Exclude errors that only weaken a check, an assertion or an oracle. Do not say which test or case should find each error, and add no coverage commentary: the list goes to the evaluator only. Output in English, nothing but the list and a one-line count by area.
