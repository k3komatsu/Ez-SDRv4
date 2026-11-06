# Design note 48 S3 — Opus re-checks (2026-11-06, summaries)

Re-check reviewer: Claude Opus subagent, effort high, read-only (AGENTS §9). Probes ran in throwaway copies outside the repository; probe sources in `48s3-probes.md`.

## 1. Amended test plan — GO WITH LISTED CHANGES

- Sol plan 1 (Mock end + 1), 2 (boundary `<= 32`, cut tests), 5, 6: closed; 3 closed apart from the timed shape; 4 not closed for N48S3-03/-05/-11: the first-send widening decides nothing at X3x0 rates (the chunk is under 32 only below 12.8 kS/s with the 10 ms window), so the rows were equivalent mutants.
- N1 (major): the Mock's two Stop sites still set cut + 2 where the Module's closed short front is end + 1 (Mock stricter after S3's P1 change).
- N2–N8 (minor): FakeDevice tests killed by the new rule instead of their own path; a second copy of the Mock's 32 boundary; N48S3-06 mutating non-existent code; 29 samples unpinned; Q1's red not deterministic; Q5's booking order; `ur_22_a_first_send_is_never_undersized` to check 32.
- O1's plan changes listed (no known-divergence tests, KL-36 only the hardware limit).

## 2. Fix round 1 — FAIL

- Sol 1 (second Stop), Sol 2 (FakeDevice timing from the packet's own time spec), the #78 regression (fails at HEAD's `tx.rs` through FakeDevice's Underflow and TimeErrors; manual clock): closed.
- R1 (major): `short_burst_floor` applied to every finite burst of ≤ 32 samples, including one continuing a longer device burst. L (40) at s and A (10) at s+40 booked early; B at s+51 inside the window: the Module drops B (front s+52), the Mock plays it. HEAD's Mock dropped it.

## 3. Fix round 2 — FAIL

- R2 (major): the Mock's `continues` flag is set once at booking. A received before L: the Mock plays B at A's end + 1, the Module drops it; also with a Stop and for a chain L→A→A2. New with S3 (HEAD's Mock dropped B in every order).
- Rows N48S3-39..41 (the flag's carry at emission) unobservable in any user flow (zero-lead profiles never consult the front).
- Recommended deriving instead of storing; the orchestrator instead returned the Mock to note 40 D5 (R3), removing the inference.

## 4. Fix round 3 — FAIL (one major, ruled within D1)

- The Mock's diff against HEAD is the 32 hand-over threshold only; inside the window the Mock is never looser (closed short bursts with 0–2 Stops, a 33-sample burst, short repeats, the 32-sample wait, continuations in both receipt orders). Both D5 parity tests check each Provider separately; N48S3-42 meaningful. Specs, removed rows and renames consistent (`rules.py` 0).
- M1 (major): bookings received before the window: (a) B at a closed short burst's end (4, 10, 32 samples), (b) B 4–32 samples into an unhanded 40-sample burst: the Module closes the cut burst and drops B; the Mock plays B. HEAD: both play B. Ruled within the owner's D1 (#79, note 47 stage 5).
- m1: MR-17 omits MR-25's first-send condition for an earlier-received burst; m2: KL-36's "When" omits (b). Fixed by the orchestrator.
