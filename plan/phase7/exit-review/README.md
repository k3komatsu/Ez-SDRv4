# Phase 7 exit review — per-rule OV-3 dispositions

One disposition for every rule Phase 7 introduced or amended: spec 18's `UR-1…UR-35`, spec 19's `KG-1…KG-14` and `VE-1…VE-6` with the rules each amends or adds, and the governance rules `GZ-1…GZ-10` with the two Phase 2 rules they amend (exit criterion 2, [`00-overview.md`](../00-overview.md) §10). It follows the evidence rule of [Phase 2's exit review](../../phase2/exit-review/README.md), as [Phase 6's](../../phase6/exit-review/README.md) does: a test citation is included only when the test body asserts the obligation, not because the name carries the rule prefix. The tables were written after Gate X (2026-10-01), which accepted Phase 7 with them missing (`00-overview.md` §11), from the test bodies on `main` at `19a74e4`; no test was run for them (the Gate X record: workspace 885 passed, `--features uhd` 171).

| file | document | rows |
|---|---|---:|
| [18.md](18.md) | spec 18 — the UHD Radio Module `ezsdr.radio.uhd` (`UR-1…UR-35`; UR-31 in two rows) | 36 |
| [19.md](19.md) | spec 19 — the amendments (`KG-1…KG-14`, `VE-1…VE-6` and the rules they amend or add) and the governance rules `GZ-1…GZ-10` with PO-2 and PO-11 | 88 |
| **Total** | | **124** |

## Dispositions

- `default`: the Phase 7 behaviour is asserted by the cited test body.
- `producer`: a Module-side obligation is carried by that Module's tests (MockRadio's, the UHD Module's, or the Kernel's `ThreadedProvider` double for its Kernel half).
- `process`: a document, spec text, build script or step procedure is the carrier; the `read from` cell names it.
- `bench`: the obligation lies in `UhdDevice` (`crates/ezsdr-radio-uhd/src/uhd.rs`), which has no fake; the carrier is the hardware test in `crates/ezsdr-radio-uhd/tests/hardware.rs` and the record of its run is the named section of [`bench-results.md`](../bench-results.md). It appears beside `default` where the Provider's side of the same rule is asserted on `FakeDevice`.
- `forward`: the obligation is not met in Phase 7 and is assigned by its own accepted text to a named later phase (Phase 2's disposition of that name).
- `GAP`: no test body and no record carries the obligation; the row says what would.

Python tests are cited as `test_easy_api.py::<name>` (`python/tests/test_easy_api.py`, run under GY-7).

**Granularity.** A row is one rule, as in Phases 1–6, and its tests are the ones the spec's `Checked by` sentence and test table assign, each read to confirm that its body asserts what the table says. Where a clause of a rule is a separate obligation that none of those bodies, nor any other test or bench record found, asserts, the row says **Not asserted** and names the code that implements it, read; the rule keeps its disposition for the rest. Those clauses are listed below. The review read every test the specs name and searched the suites for others; it did not prove that no further sentence of the longer rules (UR-23, UR-25, UR-29 above all) is unasserted — INFERRED complete beyond the clauses listed.

## Rows that are not met

- **UR-31 (`UhdDevice`) — closed after the table was first written:** it was GAP (no test asserted `UhdDevice`'s real fidelity); `hw_b3_receive_at_t0` now asserts `manifest.run.fidelity` real on every aspect, and passed on the X300 on 2026-10-01 (bench-results.md Session 2 part 12). Its row is `bench`.
- **UR-32 — forward.** Spec 18 records it "Not met in 0.1.0" (`UhdDevice::rx_recv` allocates per call) and defers it to Phase 8's copy-regression benchmark by name; Gate X accepted that text.

No row is `UNCERTAIN`.

## Clauses no test body asserted

Each was asserted after the review was first written (the owner: "じゃあそれですすめて", 2026-10-01), by a test that fails when the clause's code is disabled (mutations C01–C10, `plan/phase7/tools/mutations.json`, all killed). One part stays unasserted: KC-36's update on `Release`, which has no observable effect.

| row | clause | test | mutation |
|---|---|---|---|
| UR-7, MA-29 (KG-3) | callbacks run outside the lock `schedule` and `cancel` take | `ur_07_callbacks_run_outside_the_schedule_lock` (`ezsdr-radio-uhd` `fake.rs`) | C01 |
| UR-24, UR-26, UR-30, RM-16 | a released timed command is not recalled by `Stop`, recorded as issued | `ur_24_a_released_command_is_not_recalled_by_stop` (`fake.rs`) | C02 |
| UR-21, RM-15 | a move onto another held burst's start is refused | `ur_21_a_moved_start_on_a_held_burst_s_is_refused` (a unit test in `src/provider/tx.rs`: through the public API the two commands land in one batch only by a microsecond race) | C03 |
| KC-15 | an overflow of `L` or of T0 is `Failed { arm }` with `KC-15: overflow` | `kc_15_an_overflow_of_l_or_of_t0_fails_arm` (`ezsdr-kernel` `device_paced.rs`) | C04 |
| KC-21a | the 10 ms re-check; a Module's own submission is not waited for | `kc_21a_an_end_requested_during_the_wait_ends_it`, `kc_21a_a_module_s_own_submission_is_not_waited_for` (the freeze is only ever set with an end request, so the two conditions are tested together) | C05, C10 |
| KC-29 (KG-3) | `state()` reads `Running` and `events()` shows the cause until the next call | `kc_29_an_end_the_data_thread_requested_is_cleaned_up_at_the_next_call` | C06 |
| KC-36 | the deadline updated on `Renew` and `Adopt` (`Release`: not observable) | `kc_36_renew_moves_the_deadline_the_data_thread_checks`, `kc_36_adopt_clears_the_deadline_the_data_thread_checks` | C07 |
| KC-45 | a panicking `relations()` records none and the missing-`utc` failure | `kc_45_an_authority_whose_relations_panic_records_none_and_says_so` | C08 |
| KC-46b | under `orderly` the data thread steps on; the Run stays `Running` | `kc_46b_under_orderly_the_data_thread_steps_on_after_its_stop` | C09 |

Two `process` rows rest partly on inference, said in their cells: UR-35 and GZ-5 (that the default build passes on a host without libuhd: every recorded host had libuhd; the `build.rs` early return and `src/uhd.rs`'s `cfg` make it so) and GZ-10 (the device's `pp_string` is summarised in `bench-results.md` "The bench", its text kept in the bench server's logs).
