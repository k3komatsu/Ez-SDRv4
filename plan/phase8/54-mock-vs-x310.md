# 54 — Mock ↔ X310: how far the Mock imitates the hardware (draft; final after the 2026-11-27 bench)

Owner, 2026-11-20: sort the Mock's constraints against the X310's — stricter by choice, about the same, not reproduced, unmeasured — instead of only the rule "never looser" (Vision §59). Facts: [54-mock-vs-x310-facts.md](54-mock-vs-x310-facts.md) (87 rows, collected by a Sonnet subagent at `ea7bc5c1`; only the three L rows below were re-verified in code by the orchestrator, the rest carry the collector's own VERIFIED/INFERRED marks). The table below classifies the 21 rows flagged `LOOSER?` there.

**After the bench** (owner, 2026-11-20): fill the U rows from the 2026-11-27 records (placed by `place_measurement.py`), confirm the L rows are closed by #125–#127, then write one user-facing page — what the Mock imitates, where it is stricter, what it does not reproduce (class N) — whose form and place (a section of design/09 or its own page) the owner decides.

Classes: (S) stricter than the X310 by choice; (E) about the same — one shared rule or a parity record; (N) not reproduced — a host, analog or spontaneous effect the Mock does not model; (U) unmeasured — the comparison waits for a measurement; (L) looser by rule — a Spec or binding the Mock accepts and the UHD Module deterministically refuses: a bug (Vision §59).

| Row | Class | Why |
|---|---|---|
| TX5 | **L** | Outside channel mode the Mock never reads a burst's waveform bytes, so absent bytes, a wrong `size_bytes` or a non-finite sample pass (`ezsdr-mock-radio/src/lib.rs:1670`); the UHD Module refuses all three at receipt (`provider/control.rs:437,446`). VERIFIED (code). |
| RX4 | **L** | The Mock prepares one `Block` link on an rx port (`lib.rs:2234`); the UHD Module refuses any `Block` link at prepare (`provider/mod.rs:513`). VERIFIED (code). |
| TM2 (constants) | **L-risk** | The 3 ms delivery allowance and the 2 ms device lead are separate constants in the Mock and the UHD Module, with no test that they agree; equal today, nothing keeps them so. |
| TX9 | S/E decided | The Mock accepts a switch for a repeat that has not started on a device (it has none); UHD refuses (UR-38). Owner decision N2 = (b1), note 49 §12.1; KL-41 documents it. Stays. |
| RG3, FL4 | N | A device configure that reads back differently (refused on UHD) cannot happen on the Mock: the grid is the device's own; documented (KL-16). |
| TM2 (load), TM5, TM6, TX6 | N | Host-stall effects: a late dispatch, a configure ending inside the lead (`START_LEAD_SHORT`), a device dropping a late burst. The Mock's timing is ideal; the admission rules are shared and equal. KL-15, KL-45. |
| TX2, TX3, TX12, RG6 | N | Host-fed throughput: underflow, the repeat's pause and resume, grouped TX channels. The admission bound is shared; the sustained value is KL-5's Ceiling and `tx_bytes_per_s` is NotCompared (owner 2026-11-20). |
| FL2 | N | Loss timing and the radiated tail after a loss (UR-16); the Mock stops at the fault instant. |
| SA2, NS1, CK1 | N | Analog impairments, noise (opt-in through `noise_dbfs`), CFO and clock drift — spec 11 "Not in scope". Later: impairments in SimulationChannel/MockRadio (owner 2026-11-20, 8b D-1 discussion). |
| RG4, LO5 | — | The `ideal` profile claims no hardware equivalence (fidelity `none`); not a parity subject. |
| FE7 | U | Mixed boards on one timekeeper are accepted by both; BP4b never benched (Gate X decision). |
| PD1 | U | `path_delay_samples` is unmeasured in every record; direction depends on use. |

Framework facts: no hardware record is placed in `crates/ezsdr-radio/measurements/` yet (D13's tool places them after 2026-11-27), so RM-35's import test compares fixtures only until then.
