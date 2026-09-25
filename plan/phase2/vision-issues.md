# Phase 2 Vision issues applied at Gate X

Collected from the "Vision issues found" sections of specs 06–10. The owner accepted Gate X on 2026-09-25; all fifteen edits below are applied to the Vision, whose revision history records this Step X pass.

1. Describe the effective configuration in Vision §§11 and 52 per fragment so registered checks can judge each fragment's values. ([spec 06 §13](../../design/06-kernel-coordinator.md#13-vision-issues-found))
2. In Vision §13 distinguish Provider-declared TimingEnvelope capabilities from the Kernel's `min_command_lead`; §22 should identify which layer checks each value. ([spec 06 §13](../../design/06-kernel-coordinator.md#13-vision-issues-found))
3. Vision §22's plan-time `RejectAtPlan` lead check occurs at arm, once T0 exists; change the wording to "before start". ([spec 06 §13](../../design/06-kernel-coordinator.md#13-vision-issues-found))
4. Assign drop-policy decisions in Vision §32 to the deterministic Link operation, rather than to the Simulation Engine. ([spec 06 §13](../../design/06-kernel-coordinator.md#13-vision-issues-found))
5. Add KA-12's mapping from Kernel steps to role-trait calls to Vision §53's cleanup list. ([spec 06 §13](../../design/06-kernel-coordinator.md#13-vision-issues-found))
6. Allow a Simulation Run's virtual root in Vision §14 to have no UTC relation; transition `host_utc_nanos` provides its wall-clock placement. ([spec 06 §13](../../design/06-kernel-coordinator.md#13-vision-issues-found))
7. Use the Radio Model's names in Vision §8's illustrative radio keys, including separate RX and TX sample-rate keys. ([spec 07 §7](../../design/07-radio-model.md#7-vision-issues-found))
8. Place coercion rules in the Radio Model Vocabulary in Vision §13, with the supported grid represented as a capability. ([spec 07 §7](../../design/07-radio-model.md#7-vision-issues-found))
9. State in Vision §22 that the stream contract owns `TxBurst.format` and a burst uses every channel of its stream. ([spec 07 §7](../../design/07-radio-model.md#7-vision-issues-found))
10. Separate Vision §26's declared `phase_behavior_on_retune` capability from its Phase 3 emulation. ([spec 07 §7](../../design/07-radio-model.md#7-vision-issues-found))
11. Describe the `FaultInjector` in Vision §§13 and 17 as an environment document read by the target Provider, not as a Module. ([spec 08 §6](../../design/08-simulation.md#6-vision-issues-found))
12. Use integer `at_ns` from T0 and a target fragment id in Vision §8's `sim.faults` example, instead of floating-point seconds and an indexed resource expression. ([spec 08 §6](../../design/08-simulation.md#6-vision-issues-found))
13. Name RM-17/MR-21 in Vision §13 as the overflow rule that reproduces the specified restart gap. ([spec 09 §7](../../design/09-mock-radio.md#7-vision-issues-found))
14. State in Vision §13 that MockRadio refuses a start earlier than its synchronization end instead of beginning late. ([spec 09 §7](../../design/09-mock-radio.md#7-vision-issues-found))
15. Describe recorder controls in Vision §30 as optional: `drop if busy` belongs to the feed's drop-class policy, while the other controls may be Sink parameters. ([spec 10 §8](../../design/10-host-data-path.md#8-vision-issues-found))
