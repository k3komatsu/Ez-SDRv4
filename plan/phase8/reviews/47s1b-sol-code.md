My independent failure scenarios:
- A suffix changes physical layout, subdevice selection, device verification, or the bench’s board/attenuator prompt.
- A name resolves to another radio, direction, or logical position; mixed references accidentally reroute numbered channels.
- Number/name aliases overwrite membership or values, or malformed numeric spellings become ambiguous.
- Invalid/duplicate names reach Run creation, or the reply table disagrees with the effective channels.
- A child Run drops or changes a binding name while still passing identity checks.
- Python serialization loses distinct keys or accepts names differently from the server.


## PASS

No P1/P2/P3 findings.

Reviewed selector parsing and readers, cross-radio routing, value-alias rejection before Run creation, reply tables, Python lookup, child-Run identity, specification amendments, and test/mutation coverage. The shared `names.json` drives both language suites; no obsolete stage-1b refusal remains.

Checks run:
- Python `ConnectRequests`: **10 passed**
- `git diff --check`: **passed**

No files changed. Rust builds, mutations and hardware checks were not run; mutation conclusions are static, not independently measured kills.
