# Sol: narrow re-check of four test-plan fixes (read-only)

Read-only; do not edit files. Repository ~/local_work/Ez-SDRv4; not v3/; never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

Your third review (`~/.cache/ezsdr-fixes/2026-11-01-n45/sol-plan3.md`) found one major and three corrections, all in the test plan. The implementer amended `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md` (the version you reviewed is `/tmp/claude-1001/-home-komatsu-local-work-Ez-SDRv4/5923dcee-8280-4eec-b6f1-5313de56f782/scratchpad/test-plan-draft4-backup.md`, for a diff). The experiment plan `45-experiment.md` is unchanged since your third review. The orchestrator confirms the implementer's note 11: a channel move waits only for pins (left/debt follow I15 at restore time, across streams).

Check only: (1) the next segment no longer waits for empty left/debt, restore alone waits for I15, U9/u09a/u09g and case 5's W5d are consistent and reachable; (2) the loss-publication test checks that the publisher does not hold L; (3) u07d kills E1-60 (epoch advanced between snapshot and recompute); (4) the monitor identities match the experiment plan. Do not reopen other points without a new counterexample. End with VERDICT: PASS (event granularity and cases approved for implementation), PASS WITH CHANGES (list; applied without review), or REVISE. Concise, English.
