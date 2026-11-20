# sync_check — dispositions

Two bench programs for 2026-11-27 (no USRP transmit): `plan/phase8/tools/sync_check/sync_check.c` runs 8b step 0's checks of D-1/D-4 (design note in `~/.cache/ezsdr-fixes/2026-11-20-8b0/draft.md` §8, to be placed in the tree with the note), and `tx_packet_probe.c` reads the transmit streamer's `max_num_samps` with the UHD Module's stream args (Phase 8's H3, never recorded). Risk: not risky (outside the crates). A Sonnet subagent implemented; GPT-6.1 Terra reviewed and re-checked twice ([sync-reviews.md](sync-reviews.md)). Brief: [sync-brief.md](sync-brief.md). Gate: `sync-gate.json`.

## Counts

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Terra, review | 4 | 2 | none |
| Terra, re-check 1 | 4 | 0 | none |
| Terra, re-check 2 | 2 | 0 | none |

## Dispositions (all adopted, decided by the orchestrator)
- Review: an uninitialized sensor handle; a time setting that could pass without taking effect; unbracketed verification reads; increments read before a follower's edge; S5a's counter race; cleanup and a watchdog — fixed.
- Re-check 1: a nonce time (current whole seconds + 1000 s) proves the set took effect; all-void attempts are `INCONCLUSIVE` (exit 3), never `REFUSED`; per-handle increments from 1 ms polling of each handle's own edges; one verdict line per run (an atomic claim between the main path and the watchdog) — fixed.
- Re-check 2: an attempt whose verification brackets were all discarded is void; each follower's own edge is awaited before the verification reads — fixed. Closed without another review: the orchestrator rebuilt both programs in `ezsdr-v4-dev:uhd4.10` with `-Werror`, ran `selftest` (it covers the inconclusive/refused split) and read the `set_time` loop.
- Safety: `grep -n tx_ sync_check.c` and `grep -n 'send\|tx_metadata' tx_packet_probe.c` are empty; Terra found no transmit operation in either program in all three rounds.
- Not exercised: no device-using mode has run (no hardware here); the bench is their first run.
