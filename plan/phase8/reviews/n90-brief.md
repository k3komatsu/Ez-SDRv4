# Brief — #90: the capture Sink hashes as it writes, never reading the file back

Work tree: **/home/komatsu/local_work/Ez-SDRv4-n90** (a git worktree, branch `work/n90` from `main` `fe86629`). Work only there; the main tree /home/komatsu/local_work/Ez-SDRv4 belongs to another task — do not touch it. Read first: AGENTS.md (§7, §9 "Maintainability", the mutation and build rules), GitHub issue #90 (`gh issue view 90`), `crates/ezsdr-sink-capture/src/lib.rs` (the whole file), `ezsdr_kernel::hash::ContentHash` (how it is computed, so the incremental one is byte-for-byte the same), the capture Sink's spec (search `design/` for HD-10 and the Sink's other rule ids), and its tests.

Risk: not risky (the artifact's bytes, hash and Manifest stay the same; only how the hash is computed changes). No test plan, no self-review; the orchestrator reviews, then the full mutation lists, then Sol's code review.

## Goal

`finish_front` no longer reads the capture file back (`fs::read`, lib.rs:322). The hash is fed as the bytes are written, and the `ContentHash` is built at the end, so a large capture closes in time proportional to nothing more than the final flush (issue #90's sequence: a 4 GB capture made `CaptureSink::stop` exceed RS-8a's 5 s and lost its ArtifactRef and `.sigmf-meta`).

1. Hash exactly the bytes that reach the file, in order, and nothing else: whatever header or framing the file has, every block, and whatever is written at close. If a write can be partial or fail, the hash must still describe the file on disk, or the failure must surface as it does today (say which in the report).
2. The resulting `ContentHash` equals `ContentHash::of_bytes(&fs::read(path))` for every capture the existing tests make. Add a test that pins this equality on a capture of several blocks (including whatever close-time bytes exist), and one that fails if a block is hashed twice or skipped.
3. Keep error handling: a write error still fails as today; keep the HD-10 reason wording where it still applies, or remove the now-unreachable read error and its text.
4. Leftovers: remove `fs::read` and anything only it needed; check `design/` text that says the Sink reads the file back (if any) and correct it; the hash must not change `impl_hash` or the Manifest schema.
5. Search for any other place that reads a whole artifact back to hash it (`grep -rn "fs::read" crates/*/src`), and list what you found; change only the capture Sink unless another is the same defect, then report it instead of fixing.

## Rules (binding)

- Builds and tests only through `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`, run from /home/komatsu/local_work/Ez-SDRv4-n90 (the local PC carries no build load; sim03 because the main task uses sim02). The shell tool stops a command at 120 s unless you pass a longer timeout: give rdev runs an explicit long timeout (e.g. 3600000 ms) and wait in the foreground; never leave a command running in the background when you report.
- Mutation rows: never run mutate.py or remote_mutate.py, and never apply a mutant inside the tree. Add rows for the new behaviour in `plan/maintenance/tools/mutations.json` (Rust token anchors, each `old` unique, a killer named in the row and checked by applying the mutant in a copy outside the repository); re-anchor any row whose anchor you change (same file, no two rows identical). `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base fe86629` must report 0 problems.
- `cargo fmt --all` before reporting, then `cargo fmt --all --check`. Run `rdev --host sim03 -- cargo test --workspace` once at the end, and the Python suite if Python is affected (it should not be).
- Test temp files on /tmp. New files mode 644. Python with PYTHONDONTWRITEBYTECODE=1.
- Do not edit handoff.md, AGENTS.md, KNOWN_LIMITATIONS.md, plan/*/00-overview.md, governance.rs, bench files.
- Do not commit, push or `git add`.

## Report

Write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-11-n90/report.md; your final message is its summary: the change (file:line), how every written byte is hashed (and the partial-write answer), the tests added, the rows and their checked killers, the other whole-file reads found, every behaviour change, and anything in this brief that proved wrong.
