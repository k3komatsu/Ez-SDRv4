# n73 / n97 — adversarial design review (AGENTS §9 step 0)

Reviewer: Fable 5.2 (`claude-fable-5-2`), read-only, 2026-11-20. Checkout `work/bt2` at `500726dd`. Nothing was built or run in the tree; one 5-line Python pipe check ran outside it (3.14 only).

Verdict: the decided direction is the best I see, and I would not reopen its shape. It needs four amendments before a brief can be written (4 majors), all small. Top concern: M1.

Marks: VERIFIED = read at the cited path:line. INFERRED = reasoned, reason given.

## 1. My own scenarios and design, before the draft

Process note: I read issues #73 and #97, AGENTS §2/§3/§6 and the code (`session.py`, `_client.py`, server `lib.rs`, the capture Sink, `paced.rs`) and formed the list below before opening `draft.md` or `decisions.md`, but I wrote it into this file only afterwards. Treat it as unanchored in content, not in proof.

### 1.1 My design

- The Sink takes an in-memory request through a second key (a verb compiles to one key, so a flag on `sink.capture` cannot travel), accumulates into a `Vec`, and announces with the same event and `ArtifactRef`.
- The bytes sit in a store the server creates and hands to the Sink it builds (`crates/ezsdr-server/src/catalogue.rs:115`); the Kernel learns nothing.
- `read` serves the URI and removes the bytes. The URI names the ActionId, never the content hash (two silent simulated captures have equal bytes).
- The record (event, artifact, Manifest) never depends on whether or when the client read. Memory pressure acts on the store only.
- The cap counts bytes actually held. When a growing capture needs room, the oldest completed unread capture goes; if nothing can go, the growing capture is not held.
- #97: stream the file after a header that carries the size; the client allocates once, before it sends `read`.
- The server's per-request cost must not grow with the event log, or "continuous monitoring" is not delivered.

This matches the decided design except in the cap accounting (M1, M2), the event-log cost (M3) and the per-channel validity of `pieces` (M4).

### 1.2 My failure scenarios

| # | Scenario | Outcome against the decided design |
|---|---|---|
| F1 | Store lock held while a 1 GiB body is written to a slow pipe; the data thread blocks on it | Avoided: the draft moves the `Vec` out and writes outside the lock (draft §3 (d), last paragraph) |
| F2 | A large allocation or free on the data thread stalls receive | Open, m3 |
| F3 | A completed capture is evicted between the client's `wait_for` and its `read` | Real, and worse than a race: M1 |
| F4 | Two recorders capture at once; each is under the cap, the sum is not | Undefined: M2 |
| F5 | A raw `submit` of a capture larger than the cap | Undefined under D5 revised: M2 |
| F6 | Abandoned handles pin memory | Solved by D5 revised |
| F7 | Identical bytes collide in a content-addressed store | Avoided (draft §3 (b), URI form) |
| F8 | `finish` during an in-memory capture leaves an unreadable `mem:` record | Accepted and documented (D10) |
| F9 | The streamed file is shorter than the header said | The server exits and writes the Manifest: acceptable, m5 |
| F10 | The client cannot allocate the array after the header arrived | Session lost for no reason: m4 |
| F11 | `pieces` returns a channel's invalid stretch as ordinary samples | Undefined: M4 |
| F12 | Monitoring slows down as the event log grows | Real: M3 |
| F13 | An in-memory verb in a Run with no reader | Solved by D9 |
| F14 | A partial capture comes back as a short array without an error | Pre-existing, n2 |
| F15 | Mixed `capture` / `capture_in_memory` requests are reordered | Not a problem, n1 |

## 2. Findings against the decided design

### M1 (major) — A pipelined capture is evicted before its client could read it

VERIFIED for the Sink's code path and the draft's accounting. INFERRED: that D5 revised evicts at the reservation (`decisions.md` does not say when eviction runs; the draft's accounting is the only one written down).

- The draft reserves `n × frame` at `start_front` and counts the reservation as held (draft §3 (c), "上限の数え方と効かせ方"). D5 revised then evicts the oldest completed captures at that moment.
- In the Sink, a capture that completes mid-block is followed in the same `process_block` loop by the next request's `start_front` (`crates/ezsdr-sink-capture/src/lib.rs:293` then `:245`). Requests are contiguous by rule (HD-10, `design/10-host-data-path.md:44`).

Sequence (cap C, simulated or device-paced):

```
h1 = rx.request(n1)        # n1*8 = 0.6 C
h2 = rx.request(n2)        # n2*8 = 0.6 C; each passes the client's check
rx.result(h1)
```

Outcome: r1 completes; in the same call r2 starts, reserves 0.6 C and evicts r1. `result(h1)` gets `not_held` with "read sooner", although no earlier read was possible. In simulation this happens every time, before the event is even delivered. The draft rejected drop-oldest for a related reason (draft §3 (c): "順に読む client では次の `result()` が次々に失敗する"); D5 revised brought it back without changing the accounting.

Recommendation: count the bytes actually written, not the reservation. Keep the `reserve` (INFERRED: on Linux a large reservation costs nothing until written), and check the cap as the capture grows, one store lock per block. Eviction then happens only when the memory is needed, and the reader has the fill time of the next capture. In the sequence above r1 is evicted only if it is still unread when r2 passes 0.4 C.

Condition: this moves eviction, and M2's buffer drop, from a capture's head to the middle of a capture on the data thread, where a stall becomes a gap inside the capture being taken (m3). Adopt it together with m3's extended bench check, or free evicted buffers off the data thread from the start.

Rejected alternative: the client counts its own unanswered in-memory requests and refuses before sending. It adds client state and brings back pinned abandoned handles, the reason D5 was revised.

### M2 (major) — Overflow with nothing to evict has no rule

VERIFIED that the decisions give none (`decisions.md` D5 revised: "never the one being taken" and nothing more).

Sequence A (two radios, the Phase 8b use case; cap 1 GiB):

```
with sdr.aligned(t):
    a = rx1.request(n)     # 0.6 GiB
    b = rx2.request(n)     # 0.6 GiB; both pass the per-request check
```

Both captures are in progress, nothing completed exists, the sum is 1.2 GiB.

Sequence B: `sdr.submit(...)` raw with `sink.capture_in_memory_samples` above the cap. The client's check does not run; the server must still be bounded (#97's harm otherwise returns: OOM kill, no Manifest, KL-43 (b), `KNOWN_LIMITATIONS.md:316`).

Recommendation: one rule, which M1's accounting already gives. The capture whose growth crosses the cap with nothing left to evict is not held: its buffer is dropped, its hash and continuity continue, its record is unchanged. Its `not_held` message must differ from the eviction message, because "read sooner" is false here: name the captures in progress and `capture_memory`. Document the cap as the sum over the Session's recorders. Also use `try_reserve` and treat a failure as not held; a plain `reserve` aborts the process.

### M3 (major) — The monitoring loop's cost grows with the event log, so #73's need is not met

VERIFIED for the mechanism; the numbers are INFERRED (I assume about 2 µs to clone one event with an `ArtifactRef` payload).

- The clone runs under the `delivered` lock (`stepping.rs:429`), and the data thread takes the same lock in every pass to deliver events (`drain_and_react`, `crates/ezsdr-kernel/src/coordinator/stepping.rs:204`, lock at `:214`; called from `paced.rs:115`). VERIFIED. So on hardware a long clone also stalls the data thread: the cost is receive drops, not only a slow loop (the drop itself is INFERRED, as in m3).

- `count()` clones every delivered event to count them (`crates/ezsdr-server/src/lib.rs:445-449`; `events(0)` clones, `crates/ezsdr-kernel/src/coordinator/stepping.rs:428-430`).
- One `request()` + `result()` calls it at least four times: `status` (`lib.rs:200`, from `python/ezsdr/session.py:422`), `submit` twice (`lib.rs:182`, `:465`), `wait_for` (`lib.rs:551`).
- The control path never drops an event, and each capture adds one.

Sequence: `while True: rx.capture(n)` at 100 captures per second. After about 10 000 captures (under 2 minutes) each capture spends roughly 80 ms in clones and the loop can no longer keep its rate; at 10 per second the same point comes after about 20 minutes. Results are then read late, which under D5 revised turns into evictions.

The draft parks this in KL-48 and "a separate issue" (draft §10). That leaves the issue's stated need, continuous monitoring, unmet by the stage that closes it.

Recommendation: fix it in S3 without touching the Kernel. The server already tracks how far it has scanned (`lib.rs:120`, `:391-404`): the count is `scanned + events(scanned).len()`, which clones only the new tail. KL-48 then covers only the slow memory growth of the log, the events and the Sink's `done` list (`crates/ezsdr-sink-capture/src/lib.rs:345`), at roughly 1–2 KB per capture (INFERRED).

### M4 (major) — `sdr.pieces` has no rule for per-channel validity

VERIFIED that the record distinguishes the two cases and the decision does not (`decisions.md` D7: "splitting by continuity").

- A stream gap is a time jump with no bytes (`Gap`, `crates/ezsdr-kernel/src/stream/continuity.rs:46-61`).
- A channel gap is a stretch where one channel is invalid while the stream continues, so its bytes are in the artifact (INFERRED from the type's doc comment and HD-10's "samples its feed delivers"; `ChannelGap`, `continuity.rs:63-73`; per-channel `valid`, `:94-95`).
- Today's refusal covers both, deliberately (`python/ezsdr/session.py:168-182`; EA-17, `design/16-easy-api.md:142`: "more than one valid stretch on a channel … refused").

Sequence: a two-channel capture in which channel 1 is invalid for 1 000 samples. `result()` raises `CaptureGap`; the user calls `sdr.pieces(e.artifact)`. Read as "split at gaps", it returns one `(2, n)` array in which channel 1's invalid stretch looks like samples. That is what KL-21 exists to prevent, and it breaks "validity is per channel" (AGENTS §3).

Recommendation (owner's point, R2 below): `pieces` splits at stream gaps, SampleClock changes and channel-count changes, and raises `Error` naming `Session.read` when any channel has an invalid stretch. Like `result()` under D7, it decides every refusal from the artifact's continuity before it sends `read`; otherwise it would consume the read-once bytes and then point the user at a `read` that answers `not_held`. It is the lean option and hides nothing.

Also fix in the text: pieces of different maps can have different channel counts and different TimePoint domains; a zero-extent trailing gap (SC-30c) yields one piece. Piece offsets are computable: `Gap.len` is always present, `lost` is not needed.

### m1 (minor) — Eviction between `CaptureGap` and `pieces`

INFERRED. Gaps come when the host is behind, which is when more captures are queued, so the window is correlated with the failure it serves. The owner accepted "subject to eviction" (D7). With M1's accounting the window becomes real time instead of zero. No further change; the `not_held` message already tells the user what happened.

### m2 (minor) — The server's peak is about twice the cap

INFERRED from the draft's own mechanism: a `Vec` moved out for a `read` is no longer counted while it is written to the pipe, and the store can refill meanwhile. Either state "up to 2 × capture_memory" in EA-10 and KL-48, or count the outgoing body until it is written. I recommend the text.

### m3 (minor, INFERRED) — Freeing or first-touching about 1 GiB on the data thread

- In a device-paced Session the Sink runs on the data thread (`crates/ezsdr-kernel/src/coordinator/paced.rs:37`, `:125-128`). Its feed is `drop_oldest` with 64 blocks (`crates/ezsdr-server/src/catalogue.rs:222`).
- Dropping an evicted buffer there unmaps up to the cap; I estimate tens of milliseconds for 1 GiB of touched pages (not measured). A stall longer than the feed's depth becomes a `link_drop` gap in the capture being taken.
- The draft's Q11b measures only the `reserve`.

Recommendation: extend Q11b to a run that evicts (three pipelined 0.4 C captures, never read, at the highest rate). Only if gaps appear, free evicted buffers off the data thread.

### m4 (minor) — The client should allocate before it sends `read`

VERIFIED: a failure inside an exchange abandons the Session (`python/ezsdr/_client.py:100-102`).

Sequence: `capture(n, keep=True)` with a file larger than free memory. If the client allocates after the reply header, `MemoryError` lands mid-exchange and ends the Session. `artifact["size_bytes"]` is known from the event, so allocating first costs nothing and the Session survives. For a `mem:` URI it also avoids losing read-once bytes.

Related:
- The draft's claim that the client holds the body twice (`_client.py:135-143`) did not reproduce: `BufferedReader.read(n)` returned all 100 MB in one object and the single-item join returned that object (checked on Python 3.14; 3.9 not run). Today's peak is the body plus `samples()`'s copy (`session.py:185`). D8 is still right.
- `Session.read` is documented to return `bytes` (`session.py:817-818`). Keep that for the public method and use `readinto` only on the `result()` path, or EA-16 must say `bytearray`.

### m5 (minor) — A stream that fails mid-frame ends the Session

VERIFIED path: a failed `write_reply` leaves `frames` by `?` (`crates/ezsdr-server/src/transport.rs:144`, `:152-159`) and `Drop for Server` writes the Manifest (`lib.rs:437-443`). The client sees its output end inside a body. I see no better option within one frame; padding would be silent corruption. EA-13 must say it. For a `mem:` read the bytes are gone as well, which is fine because the Session is over.

### Notes

- n1. Mixed order is safe. Every Action for an instance goes through one FIFO queue (`crates/ezsdr-kernel/src/coordinator/state.rs:42-49`), and the Sink pushes requests onto one queue whatever the key (`crates/ezsdr-sink-capture/src/lib.rs:181-183`). Admission turns a pending update into an `UpdateParameter` that keeps its `at` (`crates/ezsdr-kernel/src/coordinator/admission.rs:82-87`), and the Sink does the timing itself (`lib.rs:231`, `lower_tick`). VERIFIED for these; that no later stage holds a timed update back until `at` is INFERRED from today's pipelined timed requests arriving in order (I did not read the dispatch after step 6).
- n2. A partial capture (a targeted `Stop`, HD-11) is returned as a short array with no error: `samples()` never looks at `partial` (`python/ezsdr/session.py:154-185`). VERIFIED, pre-existing. D7's check before the read is the natural place to decide it; with read-once bytes it matters more.
- n3. URI: `Ident` is `[a-z][a-z0-9_]*` (`crates/ezsdr-kernel/src/spec.rs:48`), so `mem:<output>/<artifact id>` cannot collide with an input's `mem:sha256:<hex>`. VERIFIED. It is unique within a Run only; two Sessions' Manifests can both hold `mem:rec/capture_5`. Acceptable, since the bytes do not outlive the Run. Listed as an input of another Spec, it passes the scheme check (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:380`) and is then resolved by hash; KL-27 needs one line.
- n4. Replay and invariant 27: with eviction in the store only, nothing in the event log, the artifacts or the Manifest depends on read timing. VERIFIED against the design text. What depends on it is only whether the client gets bytes. On hardware a data-thread stall (m3) could add a gap, but hardware Runs are not replayable anyway.
- n5. The server's `readable` set grows by one URI per capture (`lib.rs:118`, `:400`). To tell `not_found` from `not_held` without remembering every URI, answer `not_held` for any well-formed `mem:` URI of this Session that the store lacks.
- n6. An abandoned (timed-out) request is still served first and, when it runs, pushes out useful captures. Inherent to HD-10; the remedy that exists is a targeted `Stop` (HD-11, `design/10-host-data-path.md:54`). One line in KL-48.
- n7. With no disk in the path, the hash on the data thread becomes the throughput bound. INFERRED, not measured; the KL-5 rewrite should not promise more than the bench shows.
- n8. Python 3.9: nothing in the decided API needs newer syntax (keyword-only arguments, a new exception class, `readinto`, `np.frombuffer` on a `bytearray`). Not run on 3.9.

## 3. Alternatives

I looked for something better on each decided point and found none that changes the shape.

| Point | Alternative | Verdict |
|---|---|---|
| D2 verb | A flag on `sink.capture` | Needs a Kernel compile-rule change (`crates/ezsdr-kernel/src/session.rs:367-384`); the decided verb is right |
| D3 record | Events only, to stop Manifest growth | Loses the final partial capture's record (HD-16, `design/10-host-data-path.md:67`); the decided record is right |
| D5 revised | Drop-newest | Abandoned handles pin the cap; eviction is right once M1 and M2 are fixed |
| D5 revised | Client-side accounting of outstanding requests | Rejected under M1 |
| D6 | A discard op | Not needed with eviction |
| D7 | `CaptureGap` carries the pieces already read | Loses nothing to eviction, but hangs up to 1 GiB on an exception and reads data the user may not want; the decided form is acceptable |
| D8 | Range read | Not needed yet; additive later |

## 4. Spec text the design makes false or must add

The draft's §5 table is stale against the decisions: it still says `sink.take`, `Error.data` and `CaptureTimeout.request`. Correct it with the rows below and keep the rows it has right and I do not repeat: EA-6 (schema regeneration, no version change, `design/16-easy-api.md:39`), EA-15 (`:82`), HD-8 (`design/10-host-data-path.md:40`), HD-13 (`:57`), RS-20, RS-44, KC-9.

| Rule | path:line | Change |
|---|---|---|
| EA-4 | `design/16-easy-api.md:23` | `read` serves `mem:`; `connect` takes and `connected` returns `capture_memory_bytes` |
| EA-5 | `design/16-easy-api.md:38` | Kind `not_held`, with its two causes (evicted or already read; never held, M2) |
| EA-10 | `design/16-easy-api.md:69` | The cap, its default, that it sums over recorders, the peak (m2) |
| EA-13 | `design/16-easy-api.md:80` | "a `file://` URI" becomes false; read once; streamed; a mid-frame failure ends the Session (m5) |
| EA-16 | `design/16-easy-api.md:107`, `:119` | `keep=`; `CaptureGap`; `pieces`; `connect(capture_memory=)`; the refusal before sending |
| EA-17 | `design/16-easy-api.md:142` | "submits `sink.capture`" is false by default; "refused with `Error`" becomes `CaptureGap`, raised before the read; `pieces` and its rule for channel validity (M4) |
| EA-18 | `design/16-easy-api.md:143` | The new verb and key names |
| HD-6 | `design/10-host-data-path.md:35` | The second key and verb |
| HD-10 | `design/10-host-data-path.md:44`, `:48`, `:50` | Requests by either key; no file for an in-memory capture; the `mem:` URI; held bytes and the cap rule (M1, M2) |
| HD-11 | `design/10-host-data-path.md:54` | "an `UpdateParameter` on `sink.capture_samples` is a request" covers both keys |
| HD-14 | `design/10-host-data-path.md:55` | The refusal when the Run has no store (D9) |
| HD-15 | `design/10-host-data-path.md:58` | "Every capture is a SigMF Recording" becomes false |
| HD-16 | `design/10-host-data-path.md:67` | "after the data file is closed and its metadata written" becomes false for `mem:` |
| KL-5 | `KNOWN_LIMITATIONS.md:45-51` | Disk applies to `keep=True` only (n7) |
| KL-21 | `KNOWN_LIMITATIONS.md:280-285` | Symptom is `CaptureGap`; workaround is `pieces`; the channel-validity case (M4) |
| KL-22 | `KNOWN_LIMITATIONS.md:287-292` | Unchanged in substance; its workaround still holds for `mem:` |
| KL-27 | `KNOWN_LIMITATIONS.md:338-343` | One line on a `mem:` capture as an input (n3) |
| KL-48 | new | Memory growth per capture, the cap and its sum over recorders, eviction, read once, loss at Session end, abandoned requests (n6) |

Specs 10 and 16 still carry inline history (AGENTS §2); `rules.py` requires only that their markers and long lines do not grow.

Promises checked: EA-16's "refused before sending, nothing logged" holds for the single-capture check. HD-10's "the Sink cannot tell a Spec output from a Session's" is kept by the separate verb. Invariant 27 holds (n4).

## 5. Points for the owner

Only user-visible points; one recommendation each.

- R1 (D5 revised, from M1 and M2). Keep oldest-first eviction. Add: the cap counts bytes actually held, summed over the Session's recorders; a capture that cannot fit even after eviction is not held and says so with its own message. Recommendation: adopt. Without it "read sooner" is advice the user cannot follow.
- R2 (D7, from M4). Decide what `pieces` does when one channel is invalid while another is valid. Recommendation: raise `Error` naming `Session.read`. The alternative, marking the stretch in the array, alters samples.
- R3 (D10, from M3). KL-48 should not list the per-request slowdown as a limitation; the stage should remove it. Recommendation: fix `count()` in S3. The fix itself is internal and the orchestrator's to decide; the KL text is the owner's.
- R4 (D7, from n2, optional). Whether a partial capture should raise rather than return a short array. Recommendation: raise `CaptureGap` for it too, since the check now runs before the read anyway.

Not to reopen: D1, D2, D3, D4, D6, D8, D9.

## 6. Not checked

- No test or build was run; every timing or cost figure (M3's 2 µs, m3's unmap time, n7) is an estimate.
- The dispatch path after admission's step 6 was not read (n1).
- Python 3.9 was not run; the pipe check ran on 3.14 only.
- The draft's claim that no test depends on `file://` for the default path was not re-checked.
- UHD block lengths were not looked up, so m3 gives no time for the feed's 64 blocks.
