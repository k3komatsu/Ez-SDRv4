# REVISE

Paths below are relative to `crates/ezsdr-radio-uhd/src/brain/`. No P1 findings.

1. **P2 — Completion still need not wake the brain.**  
   `shared.rs:268–270`; `threads.rs:589–605`; `force.rs:216–227,363–367`.  
   Sequence: targeted free run → R executors finish → brain sleeps → S marks itself exited and parks in `SDiscard`. No notification wakes the brain; S cannot actually return until `brain_gone`. Even an R’s `Sched::exited` notifies only the global condition, not the brain’s targeted condition. Shutdown can hang indefinitely. **Previous P2-1 remains open.**

2. **P2 — The exit test can miss the final Lost despite the corrected completion order.**  
   `shared.rs:264–265`; `threads.rs:572–577,716–719`.  
   Sequence: post-step exit test reads inbox empty → R sends Lost and marks exited → test reads all senders gone → brain returns without serving Lost. Observable: queued Lost and an unchanged cell, potentially an unresolved disposition. Establish sender completion **before** checking the inbox. **Previous P2-2 is only partly closed:** moving R’s mark after its send is correct; `world.rs:559–562,572–576` also correctly fixes joins.

3. **P2 — Clock and timer senders are excluded from closure.**  
   `shared.rs:256–265`; `threads.rs:986–999,1023–1028,1039–1049`; `world.rs:553–556,675–679`.  
   Sequence: clock has read time but pauses before sending, or timer removes a due booking and pauses → S/R finish → terminated brain returns → pending sender queues its input. The inbox is then permanently unserved. Neither sender is stopped or accounted for by `senders_gone`; they must finish pending sends or have delivery explicitly closed.

4. **P3 — The regressions do not establish the required wake ordering.**  
   `stop_tests.rs:440–452,458–485`. Hand-built states are appropriate for the otherwise-unreached failure window, and the held-send test checks useful behavior. But both runs use the polling scheduler; the clean test does not force the brain to wait before completion. Neither covers targeted wakes, the empty-before-completion race, or clock/timer sends. Add controlled orderings for those cases.

**I20:** the new inbox check is sound for the model’s stated terminated-brain contract, with the deliberate dead-brain exception (`oracle.rs:517–521,1384–1394`). It detects stranded inputs, but does not prove sender closure or thread-level atomicity.

Not checked: builds, test/mutation reruns, hardware, or either forbidden holdout directory. No files or git state changed.
