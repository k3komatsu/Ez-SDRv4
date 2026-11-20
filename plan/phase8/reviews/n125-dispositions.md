# #125 and #127 (n125) — dispositions

Findings: Terra 1 major + 2 minors; Opus 0 major, 5 minors, 7 notes; Terra re-check 1 major. Mechanism: the two majors are `radio-rule-implemented-twice` (the burst refusal order was decided separately in each Provider), counted with the issues in `plan/maintenance/failure-mechanisms.md`.

All adopted, decided by the orchestrator (fix list `fixes.md`, outside the repository):
1. Terra major / Opus m1 — a burst with several defects got a different first refusal on each Provider: the UHD Module now checks metadata, then the repeat length, then the bytes (MockRadio's MR-16/MR-32 order); a competing-defect test per Provider (`rm_13_the_metadata_refusal_comes_before_the_waveform_bytes`) and row N125-3 (UHD's old order).
2. Opus m2 — MockRadio's `cleanup` clears the kept input store.
3. Opus m3 — design/09's `mr_32_waveform_refusals` row names RM-13's reasons.
4. Opus m4/N4 — *Changes* rows in design/07, 09 and 18; design/09's allowance source is `ezsdr_radio::x3xx`.
5. Terra minor / Opus N5 — RM-13's rule text without a function path or "mode"; MR-32's *Test:* line; a test name's grammar.
6. Terra minor — a stale comment in `mock_channel.rs`.
7. Opus m5 / Terra minor — test scaffolding: a unit store over the shared `ZEROS` in `mock_radio.rs`; a comment on the fallback in `mock_layout.rs`. Per-harness stores rejected: inputs are content-addressed, every helper waveform is zeros, and the absence test uses a hash no helper makes.
8. Terra re-check major — the promise "the same first refusal on both Providers" is false where the Providers' own repeat limits differ (X3xx Mock alignment 2 and maximum 2^28, UHD 1 and 2^32−1, both documented): RM-13 and its *Changes* row now say the bytes are checked after the metadata and repeat-length checks, whose limits are each Provider's own (orchestrator's text fix; the Mock stays the stricter one, Vision §59).

Decided by the orchestrator, no change: #127's `DEVICE_LEAD_NS` stays a 2 ms constant beside the shared 3 ms allowance — `ur_09_the_declared_lead_is_the_device_lead_plus_the_delivery_allowance` and `the_test_side_literals_equal_the_profile` (default suite) fail if the command lead changes without it; deriving it needs a `const` command lead (Opus N2). The bytes are read twice (check, then decode) on both Providers (Opus N1): unchanged.
