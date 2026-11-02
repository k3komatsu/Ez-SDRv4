# Dispositions: Sol code review of the #76 fix (sol-i76-code.md, REVISE, 5 × P2)

All five adopted (orchestrator, 2026-11-03). They are the same mechanism the step fixes; a fix that leaves a window only lowers the probability (note 37 §3: booking further ahead does not count, and neither does a policy change that leaves a scheduling-dependent window).

1. EOB-recovery tests (`tx.rs` ur_22_a ×2): adopted. `SendAsapAndFlag` alone leaves the Core's 10 ms submission window. Run the post-recovery part on a clock participant (pattern: `clock.spawn("test-control", ..)`, `tx.rs:4827`), then re-hold `Frozen` and book/dispatch with both clocks controlled.
2. `control.rs` successor receive after `drop(frozen)`: adopted. Do the receive on a participant registered before `Frozen` is released.
3. H4 first update can be late: adopted. The real-clock smoke may see zero applied; the timed-apply wiring (P1139/P1142/P1154) gets a deterministic, controlled-timing check.
4. H4 lost end-selection (`tx_end_ticks.min()`) and successful-mapping coverage: adopted. Deterministic fixtures for underflow → later apply → exclusion, continued gain application, and a successful mapping through result assembly; new rows for `min→max` and the mapping-call error.
5. Nearby receive tests (`rx.rs:1182–1184, 2018–2032`, `control.rs:4166–4168`): adopted, same fix as 2.

## Re-review (sol-i76-rereview.md, REVISE): findings 1, 2, 4, 5 closed; `released()`, the extracted H4 functions, P1316/P1317 confirmed
- P2: the new manual-clock H4 test `h4_a_timed_gain_update_on_the_manual_clock_is_applied_and_matched` still has a submission window (the test thread is not a participant; device time can pass `first_e` before the booking). Adopted: protect target selection through booking (hold + `released`), or move the wiring check into an in-crate unit test, or book in the Spec. No larger lead.

## Narrow check (sol-i76-check.md, REVISE): the scheduling P2 closed; P1139/P1142/P1154 kill for their named reasons
- P2: the measurement's lookup wiring (`h4.rs:~331`, `matching_timed_apply(e)` → `(e + 1)`) is unpinned after the Session-driven test was removed. Adopted: a scheduling-independent test through the wrapper with a pre-recorded apply, and a new row for that mutation.
- Narrow check 2 (sol-i76-check2.md): PASS, the lookup P2 closed; P1318 kills for its named reason.
