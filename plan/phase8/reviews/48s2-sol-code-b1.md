
> plan · gpt-6.1-sol
What this change does: removes the unused timed-setting queue capabilities and event, and removes only the release-window term from receive-restart timing. **No P1/P2 runtime defect found; two P3 documentation fixes remain.**

### Failure scenarios checked
- Stop → staged configuration → restart: wrong origin, lost delivery allowance, or changed configure warning.
- Restart with fractional sample timing: incorrect lattice rounding.
- Attempted stopped-stream rate change: old and new ratios diverge.
- Old envelope or measurement record: removed keys silently ignored.
- Deleted tests or mutation rows: retained behavior loses coverage.

### Findings

1. **P3 — Document the timing paths that remain** (`design/18-uhd-radio.md:190–196`).
   A2 explicitly requires a sentence distinguishing the receive-command FIFO from the removed timed-setting queue, and TX timestamps travelling with data. No such sentence appears in the accepted specs; the plan assumed an existing receive-FIFO sentence that is absent.
   **Fix:** add the short distinction, including that STOP bypasses and clears the receive-command FIFO, citing note 48’s primary-source evidence. This prevents the removed queue-depth measurement being mistaken for a bound on supported RX starts or TX packets.

2. **P3 — Mark restart-lead observations as pre-B3 evidence** (`KNOWN_LIMITATIONS.md:153`).
   The edited lead row still offers the 1 ms shake-out candidate without explaining that prior restart measurements do not establish the remaining margin after removing the release term. Note 48 B3 expressly requires remeasurement before relying on those results.
   **Fix:** retain the historical observation, but label it pre-B3 and require post-B3 remeasurement before lowering the profile lead. This is the KL-15 table, not KL-16.

### Review conclusions
- **B3 formula matches the ruling** at `crates/ezsdr-radio/src/timeline.rs:521–534`:
  `ceil_to_grid(max(effective, cut + lead, max(item.ready, cut + ceil(call × old_ratio) + allowance) + lead))`.
  UHD and Mock remove only the release-window input. Profile RX/TX leads remain **50 ms**.
- **N9 deviation is valid today.** Rate updates remain classless (`crates/ezsdr-radio/src/lib.rs:178–191`), and staging accepts only frequency, gain and antenna (`crates/ezsdr-radio/src/staging.rs:65–68`). Note 47’s successful hardware probe did not enable runtime rate changes. Old/new ratios therefore cannot diverge in this production path. N9 pins outer lattice rounding; it does **not** distinguish floor versus ceiling inside the fractional call term.
- **N5’s form is acceptable:** the shared literal-origin tests pin the calculation, while the existing UHD and Mock differential tests check provider wiring.
- **The 8 → 5 ms fake delay does not weaken the assertions:** the test still requires LATE, SHORT and successful cases, remaining-lead evidence, and a 10 ms measured result (`tests/common/measurement.rs:4232–4262`).
- **Removing `check_start_lead` is within B3.** It checked only `rx_start_lead_ns >= release_window_ns`, not configure duration or device lead. `START_LEAD_SHORT` and missed-start handling remain (`provider/rx.rs:371–388`).
- **G6 is covered:** unknown envelope fields are rejected; removed measurement keys produce comparison violations. Schema changes contain no version bump or changelog entry.
- **Rows reconcile:** ten deleted, two added, only J06/P626 retargeted. Deleted rows belong to removed behavior. The 13 filter warnings are qualified-name/prefix false positives; the named tests remain.
- KL-34 can remain unchanged as explicitly **Gone** history. No S2b-ii implementation or new maintainability problem was found.

**Verdict: CODE NEEDS CHANGES — documentation findings 1–2 only.**

Not checked independently: builds, tests, mutation execution or hardware. I reviewed the supplied probe logs and ran read-only diff/ledger checks; post-B3 hardware lead margin remains unverified.
