# Phase 6 exit review — per-rule OV-3 dispositions

One disposition for every rule Phase 6 introduced or amended: spec 16's `EA-1…EA-19`, spec 17's `KF-1…KF-4` and `VD-1` with the rules each amends or adds, and the governance rules `GY-1…GY-7`. It follows the evidence rule of [Phase 2's exit review](../../phase2/exit-review/README.md): a test citation is included only when the test body asserts the obligation, not because the name carries the rule prefix.

| file | document | rules |
|---|---|---:|
| [16.md](16.md) | spec 16 — the server, the protocol and the Python package (`EA-1…EA-19`) | 19 |
| [17.md](17.md) | spec 17 — the amendments (`KF-1…KF-4`, `VD-1` and the rules they amend or add) and the governance rules `GY-1…GY-7` | 26 |

## Dispositions

- `default`: the Phase 6 behaviour is asserted by the cited test body.
- `producer`: a Module-side obligation is carried by that Module's tests.
- `process`: a document, spec text or step procedure is the carrier; the `read from` cell names it.

Python tests are cited as `test_easy_api.py::<name>` (`python/tests/test_easy_api.py`, run under GY-7).

No row is `GAP` or `UNCERTAIN`.
