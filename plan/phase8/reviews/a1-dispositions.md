# A1 — review record and dispositions (2026-11-12)

The server binary's fail-stop backstop (`crates/ezsdr-server/src/main.rs` `Backstop`): when its input ends or a read fails (not `Interrupted`), a timer starts; a process still alive `EZSDR_SERVER_FAILSTOP_SECS` seconds later (default 60) exits with status 3, with no cleanup and no Manifest, so a server hung after its client has gone stops transmitting (#89; [n89-fable-design](n89-fable-design.md) A1, F4). EA-15, EA-16 and KL-43 follow; KL-43 (b) carries E9 from the bench of 2026-11-12. Risk: not risky (no queue, deadline or identity of the Run changes; a process-level timer). Owner: "A1は入れてください" (2026-11-12). Branch `work/a1` from `1231462`, rebased onto `2c0493c`, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([a1-briefs.md](a1-briefs.md)); self-review waived.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, code ([a1-sol-code.md](a1-sol-code.md)) | 2 | 1 | — (major 1: a fallible step unchecked; major 2: a premise in the spec text; minor 3: `wall-clock-test-oracle`) |
| Opus, re-check | 0 | 2 | — |

## Dispositions

**Sol, code (FAIL)**
- Major 1, a thread that cannot be created panicked the input thread, so neither the timer nor the disconnect ran and the server lived on: fixed, `Builder::spawn`, and a failure exits with status 3 at once. Not tested (a thread-creation failure is not injected); the Tests row says so.
- Major 2, 60 s is not longer than RS-8a's worst cleanup (RS-8a bounds each step, not the whole): **owner's decision** (2026-11-12, "本番ではほぼ起こらないので受け入れで良いです"): N stays 60 s and is a cutoff, not a bound on cleanup; EA-15, its Changes row and KL-43 say a cleanup still running then is cut and leaves no Manifest.
- Minor 3, the delay assertion was measured when the parent saw the exit, so row A1-3 could survive a descheduled parent: fixed, the test asserts the server was seen alive at least 1 s after its input closed (delay 2 s); A1-3 is killed by that assertion, A1-1 and A1-2 stay `"kill": "bound"`.

**Opus, re-check**
Majors 1 and 2 and minor 3 closed (every end or failure of the input starts the timer before the watch fires; the cutoff wording is the same in EA-15, its Changes row, KL-43 and `main.rs`).
- Minor (INFERRED), `std::process::exit` runs atexit handlers and C++ static destructors (libuhd's) on the timer thread, which could hang while the main thread is stuck in a device call: fixed by the orchestrator, `fail_stop()` calls `_exit(3)` (no cleanup, as EA-15 says). Rows A1-1…3 re-run: killed as before.
- Minor, once `frames` has returned with an error, `forward` stops reading the input, so no end of input and no timer follow: not changed. The Session has already ended and its Manifest is written, so nothing transmits; recorded only.
- Note, the EA-15 Changes row linked only the Fable design record: it now links this record too. The superseded "longer than RS-8a's worst" stays in the historical records [a1-briefs.md](a1-briefs.md) and [n89-fable-design.md](n89-fable-design.md).

## Decided by the orchestrator

- Exit status 3 also for a timer that cannot be created (same meaning: the input ended and the server could not finish).
- `_exit` instead of `exit` for the fail-stop (the re-check's minor), with one `unsafe` FFI declaration in the binary (`main.rs`; the library keeps `forbid(unsafe_code)`).
- No Python suite run in the fix round (the server's behaviour as Python sees it is unchanged); the gate runs them.

## Gate

PASS ([a1-gate.json](a1-gate.json)) on sim02:8 and sim03:8: 2366 tests, 582 UHD, Python 121 × 2, e2e 9/9, mutation 2191/2192 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, N51A-14, N51A-15, `phase7/G16`.
