# Phase 6 — Vision issues

Collected from specs 16 and 17. To be applied at Step X, after the owner accepts Gate X (OV-6, GY-5); none is applied yet.

| # | Section | Change | Source | State |
|---|---|---|---|---|
| 1 | §3 ("Sessions: the Easy API is a Run") | The `Normative:` line gains: `; the Python client, its server and the protocol between them in design/16-easy-api.md; child Runs in design/06-kernel-coordinator.md, KC-37a.` (re-review R13) | spec 16; spec 17 KF-3 | pending |
| 2 | §15 (the consequence "No client API waits on wall-clock time …") | After "Python has `run.wait_until(t)` and `run.wait_for(event)`;": `(design/06-kernel-coordinator.md, KC-29 and KC-29b)` | spec 17 KF-2 | pending |
| 3 | §62 ("Process boundaries in v4.0") | The table gains a row: `out-of-process (typed protocol)   clients: Python, and later CLI and MCP, through ezsdr-server (design/16-easy-api.md); a client's waveforms and captures cross as bytes, and the sample path between Modules stays in process` | `00-overview.md` S1 | pending |
| 4 | §9 ("Parametrisation lives in the builder, not in the Spec") | **Only if the owner withdraws the field at Gate X** (`00-overview.md` §11, KF-4): "The Manifest records the hash of the generating code next to the Spec hash." is removed. If the field is to be added before the freeze, §9 stands and nothing changes here | spec 17 KF-4 | pending the owner's decision |
