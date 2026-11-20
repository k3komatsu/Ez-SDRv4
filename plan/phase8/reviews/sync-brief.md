# sync_check — brief

## sync_check — the 2026-11-27 bench program for 8b's D-1/D-4 checks (lane 2) — Sonnet, implement directly

Worktree `/home/komatsu/local_work/Ez-SDRv4-n111`, branch `work/sync` (already created from `main` ea7bc5c1). Do not commit, stash, push, reset or switch branch. Never touch `/home/komatsu/local_work/Ez-SDRv4` (another lane). Work dir: `/home/komatsu/.cache/ezsdr-fixes/2026-11-20-sync/`.

**Risk: not risky** (a bench program outside the crates; no production code). The one safety rule: **it never transmits** — it must not reference any transmit API (`grep -n 'tx_' plan/phase8/tools/sync_check/sync_check.c` returns nothing).

## Spec
`/home/komatsu/.cache/ezsdr-fixes/2026-11-20-8b0/draft.md` §8.2 (the program) and §8.1 (how the bench runs it), §3.7 (the time-setting procedure `set_time` copies). Write `plan/phase8/tools/sync_check/sync_check.c` (C11, UHD C API `uhd.h` only) and a short `plan/phase8/tools/sync_check/README.md` (build command, each MODE's command line as §8.1 uses it, output line format, exit status). chmod 644.

## Build and checks (no hardware here)
- Build inside the `ezsdr-v4-dev:uhd4.10` image, output outside the tree (e.g. `/tmp/sync_check/`): first confirm the image has a C compiler and the UHD headers/lib (`docker run --rm ezsdr-v4-dev:uhd4.10 sh -c 'command -v cc gcc; ls /usr/local/include/uhd.h /usr/include/uhd.h 2>/dev/null; ldconfig -p | grep libuhd'`; the image is local or on sim02 — `docker image ls`), then `cc -std=c11 -Wall -Wextra -Werror -O2 sync_check.c -luhd -o /tmp/sync_check/sync_check` (add include/lib paths as the image needs). Report the exact command that worked; if the image cannot build it, say what is missing.
- Run without a device: `sync_check probe addr=192.0.2.1` (TEST-NET, no device) must end with one `S0 … verdict=ERROR error=…` line and exit 2 within UHD's find timeout; `sync_check` with a bad MODE or missing args prints usage and exits 2.
- Pure parts testable without UHD (tick conversion `full·200e6 + round(frac·200e6)`, the bracket check of §3.7, the cross-correlation lag search of `align`) get a small self-test mode (`sync_check selftest`, exit 0) — keep it small.
- `grep -n 'tx_' sync_check.c` is empty.

## Report
Japanese, short: files and line counts; the build command; the no-device run's output; selftest; anything in §8.1/§8.2 you could not implement as written (and why).

Follow-ups sent as messages: (1) add `tx_packet_probe.c` for H3 (the TX streamer's `max_num_samps` with the Module's stream args; never sends); (2–4) the fix rounds below.
