# Stage TXS — review record (2026-10-29)

Implementer GPT-6 Luna fast (#max); self-review Go DeepSeek V4.1 Flash; independent review Sol 6.1 (plan and code); orchestrator Claude Opus.

| Source | Stage | Severity | Finding | Mechanism | Disposition |
|---|---|---|---|---|---|
| Sol | test plan | P2 ×6, P3 ×1 | UR-23 amendment missing; allocation check by address; maximum-offset tiling; late preemption at a larger send; parity record fixture; reuse across failures and reconfiguration; cap justification | test power / stale rule (not a design mechanism) | all adopted before code |
| self-review | code | no P1 | contract assertion, Drop coverage, held-tail EOB coverage, stale names, duplicate citation, accidental TXP plan edit | — | fixed |
| orchestrator | code | minor | `TX_CHANNELS_MAX` 64 unchecked | — | rejected: `Channels` is a `u64` mask (`crates/ezsdr-radio/src/timeline.rs:27`) |
| Sol | code | major | P1116's `new2` panicked on unused fixed-array entries, a false kill | mutation row, not code | fixed as Sol prescribed; P1116 rerun 1/1; no re-review (row only) |
| full gate | code | 4 survivors | P1111, P1112 (held / cold cut on a chunk boundary), U16 (tiling left no send at offset 0), P937 (baseline failed under shard load; RX-only) | test power, not a design mechanism | fix round 1 (Luna): tests moved off the boundary, U16 and P937 re-aimed at deterministic unit tests; full gate 1869/1870 (G09 known) |

The orchestrator dropped the implementer's duplicate `bench-results.md` section (the 2026-10-29 section already records the same runs).

## Sol, code review

VERDICT: FAIL

No production-code defect found. One re-aimed mutation produces a false kill and must be corrected before accepting the gate.

## Scenarios reviewed

- **Sizing:** 20,000-sample cap, quarter-window admission, first-SOB exception, and recomputation after rate/channel changes.
- **Tiling:** original length 997, first send 996, maximum-phase continuation, grouped channels, unchanged storage, original-length records, and no copying for long repeats.
- **Boundaries:** held/cold cuts, Stop, late preemption under both policies, and a target exactly at the handed frontier.
- **Failures/lifetimes:** partial/error sends, held-tail success/failure, channel membership changes, pointer refresh, cached metadata ownership, and streamer replacement.
- **Contract:** cf32 remains fixed throughout this TX path; channel/rate changes do not invalidate the cached identifier.

## Findings

### Major — P1116 is killed by an unrelated bounds panic

**Location:** `plan/maintenance/tools/mutations.json:14354–14355`

**Problem:** The replacement for P1116 restores the old dynamically collected slice vector, but `slices` is now a fixed 64-entry array. Only the active channels contain payload slices; unused entries are empty.

The replacement evaluates:

```rust
slices.iter().map(|ch| &ch[..sending]).collect()
```

For any positive `sending` with fewer than 64 active channels, this panics on an unused empty entry.

**Evidence:** Its selected test, `ur_23_m9_a_waiting_continuation_leaves_the_held_tail_alone`, first sends a one-channel finite burst (`crates/ezsdr-radio-uhd/src/provider/tx.rs:5548–5553`). The mutant therefore panics during that setup, before reaching the waiting continuation whose tail ordering it is supposed to test. A reported kill does not establish that the tail-order assertion caught the intended defect.

**Fix:** Keep the new fixed-array construction unchanged. In `new2`, insert the delayed admission guard immediately before that construction, preserving the existing `sent[..channels]` initialization. Rerun P1116 remotely and confirm failure occurs at the intended waiting-continuation assertion rather than during setup.

## Other review results

- The tiling formula provides a full-cap slice from every original phase and stays below 60,000 samples per channel. Tiling occurs once on receipt, not per send.
- The changed send workspaces use stack arrays; native pointer storage belongs to each streamer. Sample pointers are refreshed before every native send.
- Cached metadata is selected only for untimed/no-SOB/no-EOB sends. Normal destruction and construction-error cleanup own it through `TxStream`; reopening creates new storage. The existing lost-device policy deliberately skips native-handle teardown.
- UHD 4.10’s primary-source C implementation supports unchanged metadata reuse; `make` allocates and `free` deletes.
- The changed mutation match strings each occur exactly once. UR-23’s cited tests resolve. The amendment preserves original phase/accounting, and the bench text correctly labels 20,000 as proposed.
- Scope remains TXS: no changes to RX sizing, `DEFAULT_BLOCK_LEN`, MockRadio/MR-15, timed continuations, or R2′.
- Most new payload tests use frozen manual-clock owners. The parity test involves provider threads and manual-clock scheduling; repeat-run verification remains relevant.

**Not checked:** No files changed; no builds, tests, mutations, or hardware runs performed, as instructed. Sustained 200 MS/s throughput and Stop latency remain bench-dependent.

## Sol, test-plan review

## Verdict: revise the test plan before implementation

No P1 found. Six P2 findings follow. References to “plan” mean `/home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/txs-test-plan.md`.

### P2-1 — The plan forbids a necessary spec amendment

Plan lines 36–42 leave accepted specs untouched, but `design/18-uhd-radio.md:256` explicitly makes the repeat’s wrap a buffer boundary. Tiled sends deliberately cross that boundary.

**Fix:** list an orchestrator-owned UR-23 amendment allowing tiled repeats to cross original-waveform wraps while preserving phase and the whole-`wanted` window rule. Mock’s independent MR-15 block sizing can remain unchanged.

### P2-2 — S7’s stable-address check does not prove allocation reuse

Plan line 31 uses stable slice/pointer-vector addresses to detect per-send allocations. An allocator can repeatedly return the same address after each allocation/free, so the current allocating implementation could pass.

Existing allocations are at `crates/ezsdr-radio-uhd/src/provider/tx.rs:425,460` and `crates/ezsdr-radio-uhd/src/uhd.rs:1114`.

**Fix:** measure allocation/reallocation calls in a narrowly scoped, warmed-up workspace operation, excluding FakeDevice’s own allocations. Alternatively, explicitly make allocation freedom a source-review obligation; retain address checks only for storage identity and pointer correctness.

### P2-3 — The tiling tests do not force the maximum-offset case

S1’s `L=1000`, `C=20000` always returns to phase zero. S5’s unspecified “multiple sends” of `L=997` does not guarantee reaching the phase that distinguishes the proposed padding formula from simply rounding `C` up to whole periods.

The existing first-SOB exception can create arbitrary offsets (`provider/tx.rs:415–420`).

**Fix:** force a first timed send of **996 samples for `L=997`**, then advance the window enough for a full 20,000-sample continuation. Assert its complete payload beginning at phase 996. Also assert:
- tiled storage consists of whole periods and satisfies the stated bound;
- its allocation remains unchanged across sends;
- the original waveform length drives records and phase.

This directly kills insufficient padding, phase reset, and per-send retiling.

### P2-4 — S2a tests a booked boundary, not late preemption

After the first send, the cursor is 520,000; booking at 530,000 exercises `held_cut`. It does not exercise a target already passed by a larger send. That takes a different branch at `provider/tx.rs:323–326,694–753`.

**Fix:** add a high-rate sequence after the first 20,000-sample send:
- book inside the handed interval: `DropAndFlag` drops; `SendAsapAndFlag` moves to the actual frontier and reports once;
- book exactly at the frontier: on-time adjacent continuation.

Assert event target/outcome, new burst position, records, and sample-exact continuity—not just send lengths.

### P2-5 — S8 needs an explicit record fixture and parity synchronization

S8 extends S1 and expects `wraps=60`, but the suggested TXP helper supplies `waveform_len: None` (`provider/tx.rs:996–1004,5001–5005`). Without fixing that fixture, the tracker cannot produce the expected wrap count.

Also, identical Stop operations do **not** automatically produce identical Mock/UHD sample totals: Mock computes its handed frontier from the window (`crates/ezsdr-mock-radio/src/lib.rs:1387–1415`), whereas UHD uses samples actually handed over.

**Fix:** supply `BurstOpen.waveform_len=Some(original_L)`, preferably through one production booking-path test. Define the parity comparison at a common explicit burst end/cut, rather than assuming equal immediate-Stop cursors. Assert `blocks` separately per Provider.

### P2-6 — Buffer reuse lacks failure and reconfiguration sequences

S7 covers successful sends on one unchanged streamer. Reused references/pointers also meet held tails, partial sends, and channel-set changes. Those are meaningful existing paths (`provider/tx.rs:441–486,550–604,779–823`).

**Fix:** name retained or extended tests that actually exercise the new workspace:
- tiled payload short/error send: abandon without replaying the unsent suffix;
- held-tail continuation: tail precedes new payload; failed tail prevents continuation;
- channel/rate switch, including a same-count membership change: next send uses the new ordered buffers and recomputed chunk size.

The existing suite can supply these sequences, but the plan should identify the tests and their observations rather than only promise updated sizes.

### P3 — Correct the cap justification

Plan line 7 says 20,000 matches a measured result; the brief reports **10,000**, not 20,000, in UHD’s zero-underrun run.

**Fix:** describe 20,000 as a proposed constant requiring bench confirmation. It represents 100 µs at 200 MS/s; the quarter-window still limits low-rate sends to 2.5 ms. Neither calculation proves native-call Stop latency or sustained Module throughput.

## UHD metadata facts — VERIFIED

Against UHD **v4.10.0.0**:

- [`metadata_c.cpp:101–120`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/types/metadata_c.cpp#L101-L120): `make` allocates a new object; `free` deletes and nulls it. Calling `make` over a live handle leaks it.
- [`metadata.h:183–224`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/include/uhd/types/metadata.h#L183-L224): public TX metadata functions provide creation, destruction and getters—not field setters.
- [`usrp_c.cpp:170–180`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/usrp_c.cpp#L170-L180) and [`tx_streamer_impl.hpp:165–192`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/transport/tx_streamer_impl.hpp#L165-L192): send passes metadata by const reference and the implementation copies it. **An unchanged handle can be reused**; the limitation is updating its fields through the public C API.

Leaving metadata creation/free unchanged is therefore an allowed fallback under the brief, but “metadata handles cannot be reused” is too broad.

No files changed; no tests, mutations, or hardware runs performed. Sustained throughput and native Stop latency remain bench-dependent.
