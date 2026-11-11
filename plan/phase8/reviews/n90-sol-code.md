My failure scenarios, before inspecting the implementation:
- Short writes followed by an error: only the accepted prefix may affect hash and size, including when `stop` later records it.
- Empty, truncated, and exact-length captures: neither unselected samples nor a repeated final block may enter the digest.
- Contract changes and overlapping own/request captures: each file must keep its own ordered digest through completion or abort.
- Finalization: flush/close must not append unhashed bytes; metadata and Manifest must describe the same file.
- Hash API: empty input, split updates, cloned state, and SHA-256 block boundaries must preserve the existing wire representation.
- Removed reread: file replacement/modification or tests depending on rereading must not silently invalidate the documented contract.

I’ll also check whether the generic writer is doing necessary error-path testing rather than adding a second production abstraction.
**Verdict: PASS — no major or minor findings.**

What this change does: hashes capture bytes as they are accepted by the file, eliminating the whole-file reread at completion.

- **VERIFIED:** Short writes hash/count only the accepted prefix; failed calls add nothing (`crates/ezsdr-sink-capture/src/lib.rs:86–96`). All capture finalization paths consume that file’s accumulated state (`:328–354`). Data has no header or close-time framing.
- **VERIFIED:** `ContentHasher` adds only the approved API and traits. SHA-256 finalization, lowercase hex and `sha256:` representation preserve the previous implementation (`crates/ezsdr-kernel/src/hash.rs:100–126`).
- **VERIFIED:** The private generic `HashedFile<W>` is justified: it tests real `Write::write_all` behavior under short writes and failure without duplicating production logic.
- **VERIFIED:** All seven mutation anchors match uniquely; killers are appropriate. C90-1-01 and C90-1-07 independently compiled and were killed through `remote_mutate.py` on sim03. The other five replacements are structurally compile-safe (**INFERRED**, not independently built).
- **VERIFIED:** Archived-copy tests on sim03 passed: **65 tests**, including all capture integration tests and hashing tests. HD-10 and H7 describe the implemented path; no reread-dependent test was found in the inspected callers/tests.

Not checked: full workspace gate, multi-GB throughput, or real filesystem failure injection. External modification of an active capture remains a caveat: the digest now describes bytes the Sink wrote, not subsequent edits to the file.
