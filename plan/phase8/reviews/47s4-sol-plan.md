# Note 47 stage 4 — Sol's plan review (2026-11-06, `openai/gpt-6.1-sol`, read-only)

## Independent scenarios

- Stop, loss and Provider shutdown before/at/after the clock origin; cancellation, clock records, reports and Manifest remain correct.
- Receipt orders: value→booking, booking→value, Stop→booking→value; include booking-time moves/drops, repeats, two channels and ideal’s zero window.
- Payload handed over but still on air; configuration waits through the held tail, EOB, padding and recovery.
- Partial configure failure→same-snapshot retry→corrected value; preserve Kernel admission and clear unknown device state.
- First burst after prepare, unchanged bursts, reopen and concurrent Stop/loss during configure.
- Delayed dispatch after a booking expires; retain its booking snapshot and historical radiated values.

Reviewed the plan **with P1–P10 applied**; superseded recommendations are not findings.

## P2 — Must fix

### 1. Shared `Busy` does not ensure equal receipt answers

**Evidence:** plan §3.1, S14; `crates/ezsdr-mock-radio/src/lib.rs:1364–1386,1611–1641`; `crates/ezsdr-radio-uhd/src/provider/control.rs:411–458`.

Minimal sequence, x3xx at 1 MS/s:
1. At `s−5 ms`, book one-sample A at `s`.
2. Book 100-sample B at `s+1 sample`, `DropAndFlag`.
3. At `s+5 samples`, submit a frequency value.

Mock’s booking-time padded front drops B; UHD’s booking accepts B because it checks the ordinary lead, not that front. Under P4, Mock’s `Busy` ends with A, whereas UHD’s includes B until `s+101 samples`. **Mock accepts the value; UHD refuses it.**

This is a new staging consequence of #79, not a request to implement stage-5 overlap refusal. Add this receipt-parity case and obtain an explicit resolution of which bookings feed `Busy`; identical accepted-booking inputs cannot currently be assumed.

### 2. The owner-safety tests omit the cases where its guard actually matters

**Evidence:** plan S1, S3, F1/F14, N47S4-09; P3; `provider/tx.rs:596–615,734–749`.

S1 stages only after A’s booked end. Removing the previous-end guard need not break that test: its receipt timing already places configuration after the nominal end. F14 explicitly excludes short bursts.

Required counterexamples:
- A is moved at dispatch beyond its booked end; accept f2 after the **booked** end; dispatch B while moved A’s samples remain on air. Configuration must wait for A’s actual device end.
- A has one payload sample plus padding; accept f2 at its payload end. Configuration must not occur inside the padded device burst.
- Stop closes a held tail with two samples; configuration waits through that close’s actual end.

For each, hold device time before the relevant end, assert **no apply**, then cross it and assert apply-before-B. P3 requires the first case, but the plan has no concrete amended test schedule or named killer for it.

### 3. S7’s fault cannot reach the intended lifecycle, and does not test partial application

**Evidence:** plan S7 (`test-plan.md:214–215`); `provider/mod.rs:502–514` and following prepare configure; `provider/core.rs:1010–1044`.

`TxApplyFails(n)` fails the **first n transmit applies**. Through the required Kernel/Provider lifecycle, transmit configuration already occurs during prepare. The proposed fault therefore fails preparation rather than burst A.

Also, failing before any channel changes cannot expose a partially applied snapshot.

Specify a post-prepare fault trigger. Add: c0 applies successfully→c1 fails/read-back refuses→no burst sent→mark is `None`→retry configures **both** channels. Otherwise a stale mark or partial device configuration can pass S7.

### 4. “Configure is synchronous” does not justify dropping concurrent lifecycle cases

**Evidence:** plan S10 (`test-plan.md:230–235`), F5; `provider/core.rs:1287–1321,934–946`; `provider/mod.rs:688–701`.

The configure is synchronous **on uhd-tx**, but control/loss/shutdown run elsewhere.

Minimal sequence: configure finishes its last device call→pause before publishing the mark→another thread publishes loss→resume configure completion. An unconditional success publication can leave `configured = Some(g)` after loss, contrary to P5.

Use deterministic probes at completion/publication and between channels. Assert mark disposition, claim release and no subsequent payload/device calls. Include Stop and Provider shutdown while configure is paused. Boundary-only tests do not cover these interleavings; the existing loss path also does not visibly clear `configured`, so that lifecycle must be implemented and pinned, not assumed.

## P3 — Should fix

### 5. B4’s removal inventory is incomplete and its replacement tests observe different facts

**Evidence:** plan §2.1–2.3; `provider/tx.rs:1876–1901,4512,4890,4917,5401–5419`; `provider/core.rs:2063–2080`.

The existing plan-reload test observes an **End booked after the owner’s first read**. A replacement counting clocks does not observe cancellation or reservation release.

Several direct cut users are missing from the inventory, including `ur_23_m14_a_cut_is_not_waited_past_or_crossed`. Rows F36/F40 also contain the lead helper being changed but are absent from the row dispositions.

Add explicit dispositions for every cut/plan user: unreachable synthetic cut→delete with evidence; retained lifecycle assertion→move to a reachable End/Stop/reopen test. Explicitly retain report/Manifest comparisons.

Also clarify end semantics: the current clock-end test expects **no registry end after origin** (`core.rs:2064–2072`). “Body unchanged” does not establish a new claim that every stream end ends its clock.

### 6. Several mutation rows have no demonstrated killer

**Evidence:** plan §4, N47S4-12/19; §3.3 S7/S9.

- **N47S4-19** mutates Mock `handle_update` to skip an **admission** check. That function runs after admission; it cannot disable `radio.rf_envelope`. Anchor the actual admission check.
- Under P5, suppressing failure-time mark clearing can leave old generation 0 while the failed snapshot is generation 1. S7’s second generation-1 burst still configures and fails again, so its proposed observation does not kill N47S4-12. Assert the mark is `None` and test partial application.
- Amend all snapshot-comparison rows to the ruled generation representation and give each a specific named killer.

### 7. Equal-value updates expose a gap between R4 and P5

**Evidence:** R4; P5; plan S9.

Concrete sequence: prepare at f1, gain g1→on an empty stream set **f1 again**→book a burst. P5 increments the generation, so generation comparison requires a configure although the device already holds exactly those values. R4 says no device call for an equal snapshot.

The same occurs for f1→f2→f1 before any burst. Add these cases and explicitly resolve the consequence: either equality is deliberately generation-based and documented, or unchanged final values must avoid a configure. Current S9 only tests reuse without another accepted setter.

### 8. Python validation and remaining specification edits need explicit coverage

**Evidence:** plan S13, §6.2; `python/ezsdr/session.py:318–345`; `design/07-radio-model.md:204`; `design/18-uhd-radio.md:63`.

S13 lacks the transmit equivalent of “validate everything before sending”: valid first value plus invalid second channel/type/waveform must submit **nothing**. Also test an admitted first value followed by a rejected second value: retain the first staging, send no later values or burst.

The text inventory omits RM-20’s exact envelope-member count and spec 18 §3’s “one start lead per direction”/two-lead Ceiling. Both become false when `tx_start_lead_ns` is removed.

## Verdict: **PLAN NEEDS CHANGES**

Read-only review; no edits, formatting, builds or bench transmissions. Counterexamples above were traced, not executed.
