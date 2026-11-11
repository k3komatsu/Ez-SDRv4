# #90 — review record and dispositions (2026-11-11)

Issue #90: the capture Sink read the whole file back at completion to hash it (`fs::read`), so closing a large capture (4 GB at 100 MS/s for about 5 s) overran RS-8a's 5 s cleanup cut and lost its ArtifactRef and `.sigmf-meta`, and a capture ending mid-Run stalled the data thread. Risk: not risky (the bytes, hash, size and Manifest are unchanged). Branch `work/n90` from `fe86629` in a worktree (lane 2 of [plan 50](../50-remaining-work.md)), landing on `main` as one squashed commit.

## Process

- Implementer: OpenCode2 `anthropic/claude-sonnet-5-5#high` ([n90-brief.md](n90-brief.md)). The Sink feeds a hasher with exactly the bytes the file accepts (a short write hashes its accepted prefix, a failed call nothing); the hash and `size_bytes` come from it.
- **Owner decision (2026-11-11):** the hasher is the Kernel's new `hash::ContentHasher` (OV-17; `new`, `update`, `finish`), not `sha2` in the Sink: PO-4 keeps `sha2` out of Module crates, and design/10 §8 H7 had named the incremental hasher as the alternative. `ContentHash::of_bytes` goes through it, so the hash is computed in one place. The Kernel surface grows by one item (`kernel_surface_allow.txt`, pin 113 → 114).
- Tests: `ov_17_a_hasher_fed_in_pieces_equals_the_hash_of_the_whole` (with a known SHA-256 answer), `hd_10_the_hash_is_over_every_written_byte_once_in_order`, `hd_10_the_hash_and_size_describe_the_bytes_the_file_accepted` (a writer that takes 7 bytes a call and fails after 100). Rows C90-1-01..07, each killer checked in a copy.
- Full mutation run (sim03 only, beside lane 1): 2121/2122 (`phase7/G09` known).
- Sol's code review ([n90-sol-code.md](n90-sol-code.md)): PASS, no findings.

## Gate

PASS ([n90-gate.json](n90-gate.json)), after a rebase onto `aa478bc` (#96): 2270 tests, 532 UHD, Python 121 × 2, mutation 2121/2122 (`phase7/G09` known; sim03 only).

## Findings by source and severity

| Source | Major | Minor | Note |
|---|---|---|---|
| Sol, code | 0 | 0 | 1 (the hash describes the bytes the Sink wrote, not a later outside edit of the file) |

## Found on the way

- EA-13's `read` loads a whole artifact into the server's memory, and the Python client takes it whole: filed as #97.
