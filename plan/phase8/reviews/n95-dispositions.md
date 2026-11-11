# #95 and #94 — review record and dispositions (2026-11-12)

#95: MockRadio failed the whole Run for a transmit time that overflowed its clock, where the UHD Module refuses only that Action. #94: `repeat_boundary` overflowed for a switch near `i64::MAX` (a panic in a debug build, a wrapped boundary in release). Risk: not risky (no queue, lifecycle or ownership change; the Mock is brought to the UHD's answer). Branch `work/n95` from `aa478bc` in a worktree (lane 2 of [plan 50](../50-remaining-work.md), started while lane 1 waited on the test-timing design), landing on `main` as one squashed commit.

## Process

- Implementer: OpenCode2 `anthropic/claude-sonnet-5-5#high` ([n95-brief.md](n95-brief.md)). A transmit Action whose time does not fit its clock is refused at receipt with its id on both Providers and the Run goes on; `repeat_boundary` is computed in `i128` and returns `Option`, `switch_boundary` gives `SwitchRefusal::TimeOverflow`. Paired tests pin the same answer and that the next booking is served.
- Full mutation run before review (sim02 only): 2125/2126 (`phase7/G09` known).
- Sol's code review ([n95-sol-code.md](n95-sol-code.md)): FAIL, one major and three minors, all adopted ([n95-brief-fix.md](n95-brief-fix.md)).
- Opus re-check of the fix round: PASS, two minors, fixed by the orchestrator (a missing comma in RM-37's Test line; the oracle's `recv + window + lead` checked).

## Gate

PASS ([n95-gate.json](n95-gate.json)), after a rebase onto `5658996` (#90; `mutations.json` merged by hand, both row sets kept): 2284 tests, mutation 2141/2142 (`phase7/G09` known).

## Findings by source and severity

| Source | Major | Minor | Note |
|---|---|---|---|
| Sol, code | 1 | 3 | — |
| Opus, re-check | 0 | 2 | 3 |

## Dispositions

**Sol, code (FAIL)**
- Major, a Stop whose `now + window` overflowed still failed the Mock Run (`MR-25: hand-over horizon overflow`): fixed; both Providers check a Stop with the shared `staging::stop_fits` before touching a transmit stream and refuse it with its id; what is not received (`Provider::stop`, a loss) saturates `stop_reach` toward `i64::MAX`, which cancels more, never less. The UHD side is pinned on a synthetic clock (no rig can reach that instant), so the two agree by sharing one function rather than by a paired end-to-end test.
- Minor 1, the oracle kept #94's `i64` grid: fixed (`i128`, `Option`, pinned independently).
- Minor 2, the paired tests accepted any id and rule: fixed (each Provider's expected id and rule asserted).
- Minor 3, the `TimeOverflow` arms called unreachable are reachable at X3xx 200 MS/s (one root tick per sample): kept, tested on both Providers, rows N95-13/14.

**Opus re-check (PASS)**
- M1, M2: fixed (above).
- Notes left: a fourth copy of the window conversion in the Mock's `handle_tx_burst` (pre-existing); `Core::now()` returns `i64::MIN` when the time authority fails, so the UHD burst path can still panic in that case (pre-existing, needs a failed authority); uhd-tx's own turn `expect`s a representable time (not a receipt path). A targetless `Stop {}` is a device Stop on the Mock and ignored on the UHD (pre-existing; the Kernel never sends one to a Provider).
