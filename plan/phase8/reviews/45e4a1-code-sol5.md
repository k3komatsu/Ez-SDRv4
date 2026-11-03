**PASS — the P2 is closed. No new false alarm found.**

- Only an actual `Reserve` matching the current held head earns credit (`crates/ezsdr-radio-uhd/src/brain/world.rs:782–786`).
- Head changes cannot intervene: `ReserveAll` prepends `Reserve`; another input waits until operations finish (`crates/ezsdr-radio-uhd/src/brain/core.rs:519–523`; `world.rs:512`).
- A refused attempt still earns credit; P6a requires success only when reservable at that event (`world.rs:783–785`; `oracle.rs:433–440`).
- Cancellation leaving no held values creates no obligation (`crates/ezsdr-radio-uhd/src/brain/oracle.rs:453`).
- E4-55 removes the scheduling operation and targets the regression that checks every transition; by inspection, P6b catches it (`plan/maintenance/tools/mutations.json:19727–19734`; `oracle.rs:998–1004`).

Not checked: test/mutation execution, hardware, or excluded holdout material.
