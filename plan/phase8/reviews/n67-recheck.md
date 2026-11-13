# #67 — Opus re-check of fix round 1 (2026-11-13)

Read-only re-check (Claude Opus subagent, effort high) of commit `6e0a460` against Sol's code review ([n67-sol-code.md](n67-sol-code.md)).

## Per finding

- **M1 (a lost stop on open at `arm` lost its `lost` flag) — closed for receive, VERIFIED by code.** `arm` routes the receive open's error through `refuse_open` (`provider/mod.rs`), which calls `device_lost` (sets `lost`, `mark_lost`, one `DEVICE_LOST`) before the `UR-13` refusal; a second call is a no-op. `cleanup` closes streamers only when the device is not lost and `write_sections` makes no device call. The new test's spy logs `rx_open`, `rx_stop`, `rates`, `close_streams` and `mark_lost`, so an extra close would show. The owner's configure path already published a loss (`refuse_segment` → `device_failed`), as does `reopen_streamer`. Non-loss transmit refusals behave as before (pinned by the `ur_13_arm_*` rate-refusal tests).
- **m2 (the fake's orphan stash missed a start behind an unread tail) — closed, VERIFIED.** The tail's queued start is stashed per channel with its origin and rate and cleared like the ordinary one (time zero, receive spec change, untimed stop, origin passed). Nit: an ordinary tail end promotes with the channel's current rate (the Module never changes the rate between `rx_start` and the tail's end, UR-36).
- **m3 (`calls()` hid any later stop) — closed, VERIFIED.** A receive open is always followed directly by its own setup stop under the device gate; the Module's other stops come only after a start. Nit: the helper's comment on a refused open.
- **m4 and the text — true against the code**, with nits: UR-16/UR-13/UR-29 named only the open's stop; a table row said "no `close_streams` after the one at `arm`"; a missing comma in UR-33's test list.
- **Rows:** J327, P372, P476, T42 re-anchored to the same program (applied with `anchor.apply` to old and new trees); N67F-27…30 apply once, deterministic killers.

## New findings

- **Minor:** the implementer also routed the transmit open at `arm` through `refuse_open`. `UhdDevice` marks errors from `uhd_usrp_get_{rx,tx}_stream` as `lost` (`uhd.rs`), so a failed transmit open at `arm` would become `DEVICE_LOST` instead of a `UR-13` refusal — user-visible, and untested.
- **Minor:** that transmit side has no test.
- **Note (before this step):** the same mechanism remains on other paths: `check_rates` turns a lost `rates` read into `failed`; `prepare`'s configure drops the flag in `map_err(|e| rejected(e.message))`.

Dispositions in [n67-dispositions.md](n67-dispositions.md).
