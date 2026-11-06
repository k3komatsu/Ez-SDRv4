# Design note 48 S3 — Sol's test-plan review (2026-11-06, verbatim)

## Verdict: PLAN NEEDS CHANGES

Reviewed against `f1d502e`, with D1–D4 binding. The main blocker is that changing the Mock’s short-burst floor alone cannot deliver the required inside-window parity.

### Independent scenarios

Before comparing with Q1–Q10, I identified:
- A long burst first sees 28, 31, then 32 samples of window space.
- Tail holding must not reduce a safe first send to 31.
- A clock cut or later booking truncates an unhanded burst to 3, 10, 31 or 32 samples.
- A short repeat must tile its first send without losing phase.
- After abandon/reopen, the next device SOB must obey the minimum again.
- Grouped channels must all supply the same sample count.
- Fake controls: immediate continuation, delayed continuation, interpolation ≤50, closed ≥4, and a fresh SOB after failure.
- Adjacency at end/end+1, before/after dispatch, and A→B/B→A receipt order.

### Findings

**1. P1 — The proposed Mock floor still drops the required end+1 booking.**  
References: test plan Q5/Q10, §8 MR-17; `crates/ezsdr-mock-radio/src/lib.rs:1363–1384`.

- Minimal sequence: inside the window, receive A of 10 samples at `s`, then B at `s+11`, with DropAndFlag.
- The proposed floor is `s+11`, but A also contributes handed payload end `s+10`; line 1384 takes the maximum with **handed end +2**, yielding `s+12`.
- Observable outcome: Mock drops B; UHD closes A at `s+10`, sets its safe front to `s+11`, and plays B. The same conflict exists for lengths 4–32.
- Amend the plan’s treatment of the competing frontier contributions and its normative text. This is inside-window parity required by D1, not the deferred early-booking #79 case.
- Also make receipt order explicit. The existing test named `…both_orders` tests **before/after dispatch**, not B→A receipt order (`parity.rs:1557–1590`). Reverse receipt order must be examined and its outcome recorded, not assumed equivalent.

**2. P2 — No test covers the first-send preemption cut, including its 32 boundary.**  
References: test plan Q5/Q7 and “Extra sequences”; `provider/tx.rs:542–572`.

- Minimal sequence: hold long A at `s` and B at `s+10` before either is handed over. A’s first send must close with 10 samples; B then meets the closed front.
- An implementation that raises the window minimum but misses short closure at `held_cut` can leave A open with 10 samples. None of the named tests explicitly requires this **long-A truncated-before-first-send** sequence.
- Add cases leaving 10/31 samples, plus **32**: the proposed `count < 32` close term leaves an exactly-32 preemption cut open, contrary to the brief’s “up to a cut, at most 32” rule.
- Name tests and mutations separately from the clock-cut case. Add a window-limited new SOB after underflow/abandon or reopen; current coverage does not pin the 31-wait/32-send rule on that lifecycle path.

**3. P2 — One planned existing-test rewrite contradicts the adopted FakeDevice model.**  
References: test plan §5, line 239; D2 model at line 46; `device.rs:2727–2739`.

- Minimal sequence: untimed two-sample open SOB, advance device time, then untimed EOB.
- The proposed rewrite expects “Underflow then ACK unchanged.” D2 instead fires on that delayed continuation, drops through EOB, and specifies **no BurstAck**.
- This expectation cannot pass without weakening or bypassing the adopted model.
- Preserve the old untimed-closed control using an appropriate closed packet, and separately test delayed open-burst failure. Assert the complete report sequence, absent ACK, and missing continuation samples—not only the Underflow count.

**4. P2 — Several named mutation killers do not distinguish their mutations.**  
References: test plan §7, N48S3-03/05/09; `provider/tx.rs:524–540`, `4260–4287`, `4730–4741`.

- **N48S3-03:** changing the widening minimum 32→31 cannot be killed by a 31-sample waveform: both versions cap that minimum at 31.
- **N48S3-05:** removing `wanted.max(...)` does not affect the stated 1 MHz long-burst window scenario when the normal chunk already exceeds 32. Waiting for window space still determines the first send.
- **N48S3-09:** the table names the end+1 test, while Q5 names the end-of-31 test. These observe different outcomes.
- Q6 also says the default repeat fixture sends exactly 32; its ordinary chunk can be 2,500. Require **at least 32**, or explicitly constrain the window.
- Specify discriminating setups, remove equivalent rows, and give each row one unambiguous named killer. Final token anchors may wait for code, but these logical mismatches cannot.

**5. P2 — The existing-test change list misses a restart test that will stall.**  
References: test plan §5; `provider/tx.rs:3831–3867`.

- In `ur_22_a_first_send_cut_short_keeps_the_reports_paired`, the FailSend branch places B at sample 41,002 and advances the owner’s window horizon to 41,006: **four samples fit**.
- After S3, B must wait for 32, so line 3867’s expected send does not occur.
- Retarget the owner-clock advance while preserving the intended device-late report-correlation observation. Include this test and its mutation rows in the change inventory.

**6. P2 — The spec amendment list leaves an unconditional continuation rule false.**  
References: test plan §8; `design/18-uhd-radio.md:254–255`; D1 ruling.

- UR-23 line 254 unconditionally says a held adjacent burst continues the device burst; line 255 likewise says a reserved continuation makes the payload go without EOB.
- Under D1, A of 10 samples followed at its end closes and B is late. Merely changing “four” to “32” in the tail sentence does not reconcile those earlier rules.
- Add the explicit short-first-send exception, including truncation by a held/clock cut. Keep the owner-required KNOWN_LIMITATIONS entry, without implementing the rejected receipt-time mechanism.

### Other checks

- D2 variant B and D3 are reasonable **bounded models**, not proofs of all hardware shapes. The plan correctly labels the unmeasured 29-sample boundary and no-ACK inference.
- Existing S1 parity cases use one-/two-sample waveforms inside the window; I found no existing early-booked 4–32 adjacency case that newly exposes #79. The supplied early-booking probes substantiate the deferred divergence.
- Unequal Provider waveform lengths are unreachable through normal booking: `provider/control.rs:386–403` validates complete interleaved frames and creates equal-length channel vectors. Record that evidence rather than adding an artificial malformed-`Held` scenario.

Not checked by execution: builds, tests, mutations or hardware. No files changed.
