
> plan · gpt-6.1-sol
What this change does: removes the raw queue-depth measurement while preserving coercion, restoration and record handling. The timed `Device` interface remains for β.

**Failure scenarios checked**
- A failed apply or expired deadline allows another call or restoration.
- Panic or failed restoration drops an unsafe device handle.
- Queue deletion removes retained coercion, record, PLL-retry or packet-state coverage.
- Probe registration or record assembly still expects queue results.
- Deleted mutation rows target retained behavior.
- α changes behavior reserved for later sub-steps.

**Findings:** No P1/P2/P3 code findings.

**Review results**
- All 18 deleted tests concern removed queue behavior. The clock-read test is obsolete because the raw group no longer reads time; receiver-stop cases disappear with its streaming trial.
- Retargeted failure, deadline and panic tests retain their observations. Lease/restore, record assembly, PLL retries, restart-lead and P5/D70 coverage remain.
- All 52 deleted mutation rows target removed code, including both P863 anchors. P732’s retarget remains effective against its retained assertion. No remaining mutation filter names a deleted test.
- Remaining queue references are historical records or contracts deliberately retained until later sub-steps. No active queue-measurement caller or environment-variable forwarding remains.
- `design/18-uhd-radio.md:30` and UR-34 match α. No premature β/S2a-ii/S2b implementation found.
- `git diff --check` and shell syntax checking pass.

**Verdict: CODE OK.**

Not checked: independently rerunning builds, tests, hardware or mutation gates. Reported execution results were not treated as independently verified.
