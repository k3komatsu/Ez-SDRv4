You are the independent read-only reviewer (AGENTS.md §9 step 7) for the Ez-SDR repository at ~/local_work/Ez-SDRv4. Do not edit any file, do not format, do not commit. Never read ~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/.

Step: fix GitHub issue #76 (`gh issue view 76 --comments`): six tests whose verdict depended on when the test thread runs (mechanism `mech:wall-clock-test-oracle`). The rule they must now satisfy is design note 37 (`plan/phase8/37-test-thread-timing.md` §3) as written in `crates/ezsdr-radio-uhd/tests/README.md`: a test's verdict never depends on when the test thread runs; options (1) book in the Spec, (2) an assert that holds however far time moves, (3) `send_asap_and_flag` where a delay must not matter, (4) the window-precise property in a controlled-clock unit test with `Frozen`. Booking further ahead does not count.

The change is the uncommitted diff in `crates/ezsdr-radio-uhd/src/provider/{tx,control,rx}.rs`, `crates/ezsdr-radio-uhd/tests/fake.rs`, `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs`, and one `filter` line of row P1151 in `plan/maintenance/tools/mutations.json` (`git diff -- <those paths>`). IGNORE the other uncommitted changes (`src/brain/`, `src/host.rs`, the E2* rows): they are a different step. The brief: ~/.cache/ezsdr-fixes/2026-11-01-n45/brief-i76.md.

Write your own failure scenarios first, then check:
1. For each changed test (and `ur_23_m14e_a_waiting_owner_switches...`, also changed): is the verdict now independent of test-thread scheduling? Name any remaining window (a read or action of the test thread, then a later operation or assert that device time can overtake), with a concrete delay that flips the verdict.
2. Did any change lose what the test proved? Compare each test's name and its old asserts with the new ones. Where a property moved (e.g. a pinned `at` replaced by a looser check after `SendAsapAndFlag`; H4 asserts made conditional or replaced by `assert_h4_accounting`), is the property still pinned somewhere (another test, a unit test)? A lost property is a finding; name the mutation that would now survive.
3. The new pure `tx_overlap` function and its unit test in h4.rs: behaviour identical to the old inline match? Boundaries right?
4. Mutation rows whose filter names these tests (P1133–P1158, P1172, P1191, P1221, P1265): still killed for the reason their names give, not by an incidental assert? (All 23 were run remotely after the change: 22 killed, P1158 equivalent as recorded.)
5. Same pattern left nearby in the same files, not fixed.
6. Maintainability (AGENTS.md §9 "Maintainability"): duplication, leftovers, the new helper `assert_h4_accounting`.

Report: verdict PASS / REVISE, then findings as P1/P2/P3 with path:line, a minimal counterexample (operation sequence and the wrong observable outcome) for any major, and a fix suggestion. At most 60 lines.
