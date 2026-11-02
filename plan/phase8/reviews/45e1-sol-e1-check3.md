# PASS

The P1 is closed. The drain reset is sound under the adjacent command-admission rules. Both P2s and the assertion note are closed.

Paths below are relative to `crates/ezsdr-radio-uhd/src/brain/`.

| Item | Status | Evidence |
|---|---|---|
| **P1: Terminate before device Stop** | **CLOSED** | `stop_if_shutting_down()` now includes `Stopping` (`core.rs:509–513`). Thus cut publication before delivery of `Armed` no longer skips Stop. `terminates_ready()` also rejects an unstopped active chain and waits for outstanding work (`core.rs:742–749`; `cell.rs:455–463`). The regression observes the actual device state when R takes Terminate (`cases.rs:1055–1075`), rather than actor exit alone. |
| **Extra change: reset Ended at Start** | **CLOSED — sound** | Only `Ended` becomes `Idle`; `Booked` and `Draining` are preserved (`core.rs:573–576`). `StartNext` waits for retirement, pending commands, drains and unfinished stops (`core.rs:828–840`; `cell.rs:455–456`). A late restart requires Armed/Streaming and no outstanding command (`core.rs:853–862`), whereas sending Stop closes acceptance and changes that phase to Stopping (`core.rs:578–581`; `cell.rs:261–265`). Consequently, this reset cannot reopen an already-sent stop for the same segment or erase a pending drain. It correctly removes the retired epoch’s Ended marker before the new segment starts. |
| **`incomplete()` exemptions** | **CLOSED, within the adopted failure contract** | An otherwise healthy exited executor with an unbounded running device stream or outstanding command is now reported (`world.rs:1554–1566`). Loss/F exemptions match the executor’s discard-and-exit path (`world.rs:1035–1044`); brain death deliberately terminates executors without waiting for command settlement (`world.rs:801–808`). Uncertain permits a failed Stop—not proof that the physical stream stopped (`cell.rs:375–378`). Outstanding commands remain checked for an Uncertain cell. These exemptions do not conceal the original normal-shutdown failure. |
| **P2: narrowed case 7b schedules** | **CLOSED** | Original case 7b again has four inputs and declares W7b1–2 (`cases.rs:251–266`). The fifth input and W7b3 belong to the separate `case7b_late_restore` (`:273–282`), with separate micro and sweep tests (`:744–745,815–822`). |
| **P2: missing witness traces** | **CLOSED** | Each witness retains its first seed and trace prefix; reports print both trace and scheduling weights (`cases.rs:602–629`). Prefix lengths identify the first state reaching the witness (`:515–524`). The regression replays a retained trace (`:998–1003`). |
| **Late-command assertion note** | **CLOSED** | Assertions now inspect streamer state after `RCall`, for both restart and end-now (`world.rs:2489–2497`), through the settling loop (`:2468–2475`). |

**No new P1/P2 findings.**

**Note — exhaustive coverage is not enforced by the regression.** `cases.rs:1075` uses `probed()`, whose assertion accepts a bounded search (`:915–922`). A green test alone therefore does not establish the reported exhaustive completion.

**Not checked:** No edits, formatting, builds, tests, mutation execution or state-changing git commands. Execution results, exhaustive-search completion, E2 and hardware behavior were not verified. The holdout directory was not accessed.
