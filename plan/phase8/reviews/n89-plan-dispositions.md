# #89 test plan — dispositions of Sol's review (sol-plan.md, verdict FAIL)

All findings adopted. None refutes an owner decision.

1. **Major: X1 does not show that the repeat was transmitted before the departure.** Before closing the input, X1 waits for a positive-payload `tx_send` in `calls()`. It keeps the EOB → `close_streams` assertions.
2. **Major: N1's thread ownership of the device drop has no concurrency test.** Add a test that keeps `fire` in progress with a blocking test closure, sets the removal barrier, and only then lets the owner drop. Assert that the final drop does not happen on the input thread, or that removal waits for `fire`. Cover the four removal sites: `disconnect`/`finish`, `connect`'s ended path, and the end of `drive`. Add narrow test hooks only where necessary.

Minors, all adopted:
- **Connect.** An integration case for `connect`: registration happens before the T0 advance, and the ended-before-T0 path unregisters. The fire is gated so that it lands at that point.
- **First end.** K5 gains the order "`finish` requests `client` first, then a signal during cleanup", pinned with a cleanup gate.
- **Read errors.** A controlled reader returns an error while a request is active. C89-2-04 names that test.
- **Python.** Add an interruption while the reply body is being read, and a mocked `TimeoutExpired → kill → wait`, which tests the 30 s fallback without waiting.
- **C89-3-06.** Give the final anchor and show a wrong outcome distinct from the existing `stdin.closed` refusal, or drop the row.
- **Bench.** Record evidence that the request was entered (the server is inside `advance` when the input is cut). A run without that evidence is INCONCLUSIVE.
- **EA-15.** Exit 0 applies to BrokenPipe only.
- **EA-16 and KL-43.** Carry the distinction "earlier abandoned call" vs "`close()`'s own `finish` failed", plus the qualifications: missing or partial Manifest, timeout kill, inherited pipes, hung server.
- **Mocks.** No production `abandoned is True` check. The test doubles initialise `abandoned` explicitly.

## The Opus re-check of the amended plan (all adopted; the plan is accepted with these)

- **Major 1 (X1) was not closed.** Every transmit stream open sends a 4-zero untimed flush (`core.rs:1205-1215`), which already satisfies "`n=k>0`" before the repeat sends anything. X1 now polls for a **timed** payload send, `tx_send 0 n=<k> at=<digits>` with k > 0, and requires the `eob=true` send at a strictly later index, with `close_streams` after that.
- **X1 race.** When the input thread fires before the `advance` frame is taken, A2 skips it, and X1 then rehearses bench C1/C2 only some of the time. X1 records which branch ran and retries, bounded, until the advance has been served at least once; otherwise the hardware audit's claim is softened. The implementer chooses one of the two and says which.
- **Major 2 is closed for the watch.**
  - W2: wait for `contended >= 1`; make the gate a channel whose sender drops on unwind; go through the same helper the sites use.
  - **New W5 (re-check finding; it revises my answer to Q2):** the order at `finish` *is* observable without new Kernel surface. Register a gated test signal G before `connect`; `fire` runs on another thread; the owner thread calls `handle(Finish)`; wait for `contended`, then open the gate. Remove-then-finish gives `client_disconnect`; finish-then-remove gives `client`. Add a row that reverses the order at `finish`. The order at `disconnect` and on `connect`'s ended path stays with review, because those paths give the same reason either way.
- **`drive`'s early returns** (`lib.rs:309-316`): add the child's signal only after them.
- **X7:** an `Interrupted` read is retried and is not a departure. Say so in the plan and test it, if that is cheap.
- **P6:** the wrapper spec gains a `read` mode.
- **Bounds:** pick one consistent set (the shortest that works), and fix §0 and the Counts.
- **C89-3-02** is killed by P1's assertion, not by a bound. Correct the list.

## S2 (implementation) — sub-step review and owner decisions

- Opus high review: M1 (X1's 10 s bound covered `connect`) fixed, the bound now applies after `join`; minor 1, `Watched` has a `Drop`, and the orchestrator moved `Live.watched` before `run` because struct fields drop in declaration order (N1 on unwind); minor 2, a failed stdin read is a departure (exit 0), a named change in EA-15; minor 3 (EA-15 wording) fixed; minor 4, the test helpers were unified.
- **PO-11 (owner 2026-11-11, (a))**: the stdio server's input thread (`transport.rs`) is a named exception; the scan forbids `thread::Builder` and exempts `paced.rs` and `transport.rs`; the thread-spawning unit tests moved from `src/` to `crates/ezsdr-server/tests/`.
- **PO-8 (owner 2026-11-11, accepted)**: the server's dev-dependency on itself with the `test-support` feature (the `Watch` test seams), added by the implementer unasked and brought to the owner; the plain builds the gate's Python step and the bench use do not carry it.
- Rows 13, 14, 15 and 19 were removed: 15 and 19 are equivalent under `Drop`, and 13 and 14 are order-only at `disconnect` and on `connect`'s ended path, which the plan leaves to review. Row 21 was added (W6).
- Python 3.14: 110 tests OK on this tree. No scripted-stdin use of the binary exists in the repository.
