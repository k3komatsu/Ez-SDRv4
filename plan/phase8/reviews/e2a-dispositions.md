# E2a — review record and dispositions (note 49 stage E2a, 2026-11-10 … 11-11)

Stage: one transmit book owns operations and results (#80, #84, #85, #86, #88, and #93 added by the owner on 2026-11-11). Risk: risky. Branch `work/e2a` from `0ccb267`, landing on `main` as one squashed commit.

## Process

- **Design (AGENTS §9 step 0).** Opus (effort max) drafted [e2a-sketch.md](e2a-sketch.md). The owner settled the direction and N2 = (b1), #86, the `moved_to` deletion and (d1)–(d5) one decision at a time. The design review was done by **Fable 5.2** instead of Sol, as a trial (owner, 2026-11-10): [e2a-fable-design.md](e2a-fable-design.md), 28 min, $14.18. All of F1–F9 and A1–A5 were adopted.
- **Test plan.** A Sonnet subagent wrote it ([e2a-test-plan.md](e2a-test-plan.md)), with 13 disagreements against the sketch and the review, all accepted ([e2a-plan-adjudication.md](e2a-plan-adjudication.md)).
  - Sol reviewed the plan: FAIL, 2 majors and 3 minors ([e2a-sol-plan.md](e2a-sol-plan.md)). All were adopted ([e2a-plan-dispositions.md](e2a-plan-dispositions.md)).
  - The Opus re-check of the amended plan found one new major (R9's rig could not show the difference) and three minors, all adopted (the plan's "Changes after the re-check").
- **Implementers.** E2a-1 was implemented by a Sonnet subagent and reviewed by a Haiku subagent. From E2a-2a on, the implementer was an Opus subagent at effort medium and each sub-step's review a fresh Opus subagent at effort high (owner, 2026-11-11). Every sub-step got a full mutation run before its local commit.
- **Code review.** Sol reviewed the whole step: PASS WITH MINORS, 3 minors.

## Findings by source and severity

| Source | Major | Minor | Note |
|---|---|---|---|
| Fable 5.2, design | 0 (F1–F9 adopted as design changes) | — | A1–A5 adopted |
| Sol, test plan | 2 | 3 | — |
| Opus, plan re-check | 1 | 3 | — |
| Haiku, E2a-1 | 1 (re-anchored rows moved into shared code) | 4 | — |
| Opus high, E2a-2a | 0 | 5 | 2 |
| Opus high, E2a-2b | 0 | 5 | several |
| Opus high, E2a-3 | 0 | 4 | several |
| Opus high, E2a-4 | 0 | 3 | 5 |
| Sol, code | 0 | 3 | — |

Mutation runs with failures before their fix: E2a-1 had 4 compile errors (a `new` text still naming `.busy`) and 1 survivor (E1-U13). E2a-2a had 1 survivor (P1399: the deferred refusal was not visible while its killer held the lock). E2a-2b, E2a-3 and E2a-4 had none beyond G09.

## Dispositions after the plan

**E2a-1 (Haiku)**
- Major: five re-anchored rows mutated shared `TxBook` code, and two pairs were identical. Fixed: each was re-anchored at its Provider's call site.
- m1, the `playing` clear needing the stream clock: test fixtures only; left, and rewritten in E2a-2b.
- m2: renamed.
- m3: the Mock order restored.
- m4: followed m3.

**E2a-2a (Opus high)**
- The F7 checks exposed two emits under the streams lock: refusals in `Streams`, and the reopen-failure cancellation. Both were fixed in production by emitting after the lock is released. Named change: the order of these events relative to other threads' events.
- R4–R6 probe cycles: fixed with `Weak`, together with four e1 tests.
- `TxBook::take`: changed to `debug_assert!`.
- Doc comment: fixed.
- A test helper now ends and releases the lock.
- B31's reason recorded.
- UR-30 lists `tx_lock_wait_max_ns`.

**E2a-2b (Opus high)**
- m1, a pending switch on a later repeat is still cancelled by an abandon: pre-existing. (b1) makes the state unreachable; the doc comment says so.
- m2, the Stop path and the abandon path give different rows for an applied switch with an unsent open payload: these are different events. Recorded here.
- m3 doc comment and m4 test gaps: fixed. Rows E2A2B-12 and E2A2B-13 added.
- `debug_assert!` added in `cancel_all`.

**E2a-3 (Opus high)**
- m1, after a Stop a switch with b < B reads `applied` though nothing of its waveform is sent (UHD): this is the plan of note 49 §5.4.5 (UHD aligns to the Mock's answer, and the honest outcome comes with C2). It now happens also with the switch unread, so it was added as **KL-42**.
- m2, the boundary computed twice: a UHD call-site row was added (E2A3-17). Sol raised it again, see below.
- m3: row E2A3-18 added (a guard that reads halts only).
- m4: the unreachable pending branch of A1 was deleted, and the drain dependency documented.
- Row descriptions fixed, the UR-36 wording fixed, the redundant `if !switch` gates removed, and the Changes rows added.

**E2a-4 / #93 (Opus high)**
- m1: the tick rule is limited to underflow reports. Sequence errors abandon as before, because their tick's meaning on the X300 is not established.
- m2: row E2A4-04 added.
- m3: the wording changed to "recorded and reported (UR-28) but abandons nothing".
- The residual risk is recorded on #93.

**Sol (code)**
- 1, KL-41 said the predecessor "plays on": fixed. A late repeat can be dropped at dispatch.
- 2, KL-42's Mock range: fixed. It runs up to the MR-25 payload cut, not `B`.
- 3, both Providers ignored `SwitchAccepted.boundary`: fixed. The preview serves only the validation checks, the booking and the row take `accepted.boundary`, and a `debug_assert_eq!` turns the existing killers (E2A3-17, N47S5-33) red on a one-sided change.

## Named behaviour changes (all in the step's Changes rows)

- **#80:** an abandon keeps later bookings, `playing` and the busy end.
- **F3:** after an applied switch, the abandoned repeat's footprint ends at cursor + `STOP_FRONT`, so a SendAsap booking there is refused at receipt.
- An applied switch whose payload is the unsent open booking is cancelled by an abandon.
- **#85:** a halt cancels an open booking that issued nothing.
- **A5/F4:** a late underflow with no open device burst changes nothing.
- **#93:** an underflow report whose tick is before the open device burst's first tick abandons nothing.
- **(b1):** on UHD, a switch on a repeat whose first send has not been issued is refused at receipt (KL-41).
- Switches never move, and `moved_to` is gone.
- **#86:** a timed switch pushed past its own boundary records `requested_target = at`.
- **#88:** `playing` follows a repeat moved at dispatch.
- After a Stop, a switch with b < B reads `applied` (KL-42, until C2).
- The new stat `tx_lock_wait_max_ns`.
- Refusals are emitted after the streams lock is released.

## Mechanisms

None of the step's findings starts a new mechanism count. The issues filed on 2026-11-11 from the Fable 5.2 repository audit (#89–#95) are counted in `plan/maintenance/failure-mechanisms.md`. #93 is fixed here.
