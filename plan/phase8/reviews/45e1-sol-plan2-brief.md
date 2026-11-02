# Sol: re-review of the experiment plan after your REVISE (read-only)

Read-only; do not edit files. Repository ~/local_work/Ez-SDRv4 (main); not v3/; never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

Your previous review (VERDICT REVISE, majors 1–7, minors 8–11) is `~/.cache/ezsdr-fixes/2026-11-01-n45/sol-plan.md`. The orchestrator adopted all of it. The experiment plan is now draft 3a: `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md` (the draft you reviewed is saved as `45-experiment-d2.md` beside it, for a diff). The implementer's test plan was amended to draft 3: `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md`; its "stage notes" 1–6 were then resolved in the experiment plan's draft 3a (Changes row), so where the test plan still differs on case 7(c), horizon-end timers, the G→L lock order, the monitor key summary or window notation, the experiment plan wins and the implementer adapts at code time.

Check, for each of your findings 1–11, whether the change closes it, and whether the changes introduced new problems (in particular: the per-sample tick and the input-window rule; B.op atomicity; recv_begin/recv_end; the G/L order and I14; the disposition / command result / leftover consumption split; the restated P1; witnesses; cases 3, 5, 7, 8, 9; exact keys with a finite monitor summary; the stronger no-cycle property). Majors need a minimal sequence and the observable wrong outcome. Then: is the event granularity and the case list approved for implementation? Also say which cases you expect to exceed the 10-minute budget and what the implementer should report rather than do if they do.

End with VERDICT: PASS (approved as written), PASS WITH CHANGES (list them; no re-review needed), or REVISE. Output in English, concise.
