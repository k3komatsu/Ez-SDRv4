# Orchestrator review of stage 1 code (step 6)

Read: Kernel (coordinator connect, implicit_spec, registry vocabulary_of), server initial.rs/protocol/lib, radio channels.rs defaults, UHD test harness edits, Python session.py/__init__.py, spec diffs of 03/07/16.

Brief fidelity: matches O1 revised (no default stream), signature (B), Q1–Q4. Kernel learns no radio keys (generic `InitialRequests`). Refusal of layout bounds is in the server before `coordinator::connect` (no Run).

Findings (mine, kept apart from Sol's):
- O-1 (minor): `connect(radio="x")` without rx/tx is silently ignored (session.py connect). Either refuse or document.
- O-2 (note): stream-name validity is checked in both Python `_stream_name` and server `number_streams` — client/server boundary, acceptable (server must check untrusted frames), but the two rules must change together.
- O-3 (note): a stream with `channels: []` but a `sample_rate` is accepted; harmless (sets a rate on an empty stream).
- O-4 (note): `taken_names` is shared across resources, so names are unique across radios too — stricter than Q1, fine.
- O-5 (note for owner): TX open-time two-zero burst; pre-existing? to confirm with Sol.
