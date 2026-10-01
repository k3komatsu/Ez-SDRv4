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

## Clauses no test body asserts

| row | clause | implemented in (read) |
|---|---|---|
| UR-7, MA-29 (KG-3) | callbacks run outside the lock `schedule` and `cancel` take; a callback may schedule (at its own instant, fired in the same wakeup) and another thread may schedule while one runs | `crates/ezsdr-radio-uhd/src/authority.rs` `next_wakeup` |
| UR-24, UR-26, UR-30, RM-16 | a timed command already released within the release window is not recalled by a `Stop` and is recorded in `applied` as issued, with its `e` | `crates/ezsdr-radio-uhd/src/provider/control.rs` (the release path's `issued: true` row) |
| UR-21, RM-15 | a preemption whose move lands on another held burst's start refuses the moved burst (`TIME_ERROR { refused }`, `COMMAND_REJECTED`) | `crates/ezsdr-radio-uhd/src/provider/tx.rs` |
| KC-15 | an overflow of `L` or of T0 is `Failed { arm }` with `KC-15: overflow` | `crates/ezsdr-kernel/src/coordinator/pipeline.rs` |
| KC-21a | the wait re-checks at least every 10 ms whether an end was requested or dispatch frozen, which end it; a Module's own submission (MA-14a) is not waited for | `crates/ezsdr-kernel/src/coordinator/paced.rs` (`wait_finished`), `pipeline.rs` (`Submitter`) |
| KC-29 (KG-3) | after an end the data thread requested, `state()` reads `Running` and `events()` already shows the cause until the next call | `crates/ezsdr-kernel/src/coordinator/` |
| KC-36 | the Lease deadline the data thread checks is updated on `Renew`, `Adopt` and `Release` (only `disconnect` is exercised) | `crates/ezsdr-kernel/src/coordinator/` |
| KC-45 | an Authority whose `relations()` panics records no relation and the missing-`utc` failure | `crates/ezsdr-kernel/src/coordinator/ending.rs` |
| KC-46b | under `orderly` the data thread goes on stepping Executors and Sinks after its own RS-6 steps 1–2, and the Run stays `Running` until the control thread's cleanup | `crates/ezsdr-kernel/src/coordinator/paced.rs` |

Two `process` rows rest partly on inference, said in their cells: UR-35 and GZ-5 (that the default build passes on a host without libuhd: every recorded host had libuhd; the `build.rs` early return and `src/uhd.rs`'s `cfg` make it so) and GZ-10 (the device's `pp_string` is summarised in `bench-results.md` "The bench", its text kept in the bench server's logs).
