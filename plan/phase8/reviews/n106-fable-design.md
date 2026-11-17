# #106 — adversarial design review (AGENTS §9 step 0)

Reviewer: Fable 5.2 (`claude-fable-5-2`), 2026-11-17, read-only against `main` at 5b8059c7. Inputs: `design-draft.md`, `decisions.md`, issue #106, `bench/2026-11-16-stage4/README.md`, the code and specs cited below. No build or test was run: every "would fail" is read from the code and the assertions. Paths are relative to `/home/komatsu/local_work/Ez-SDRv4`; `tx.rs` is `crates/ezsdr-radio-uhd/src/provider/tx.rs`, `core.rs`, `mod.rs`, `rx.rs`, `control.rs` its siblings, `mt3d.rs` is `crates/ezsdr-radio-uhd/src/provider/tx/tests/mt3d.rs`, `fake.rs` is `crates/ezsdr-radio-uhd/tests/fake.rs`.

I do not argue to keep b′. The direction holds: I found no lifecycle sequence in which removing b′ and its zero case corrupts the book, double-resumes, or breaks the 100/s bound (§3). The majors are in the record bound and in what the texts promise, not in the resume.

## 1. Majors

### M1 — A gap can end up in neither an event nor a row; draft §1, principle 5 and the SC-25 clause say it cannot

The draft promises "Every drop is told by an event whose `time` and `resumes_at` bound its silence" (draft §1 "What a user can rely on") and makes events "the complete log" (principle 5); the SC-25 amendment reads "told by the event and, within the bound, recorded". Q1 was accepted on that footing. Three sequences contradict it, all in today's code and all kept by the design:

1. **Expired resume whose report never comes.**
   ```
   repeat open, host late: timeline_expired -> resume_repeat("expired")   # tx.rs:1005-1012, row tx.rs:1115
   the drop was a late one, and its correlation is cleared (a reopen's generation change, tx.rs:1336-1342); or no report is ever read
   ```
   Outcome: one `tx_resume` row, one split record, **no event** (a `TimeError` makes an event only through a correlation, `tx.rs:1437-1459`). An Underflow read after the correlation was cleared still makes its event (`tx.rs:1427-1428`), but with no `resumes_at`, so it bounds no silence either. Pinned as intended by `mt3d_a_resume_whose_event_never_comes_still_splits_the_record` (`mt3d.rs:451-466`: `assert!(emitted(&rig.events).is_empty())`). The draft's own P-1 row says "none only when an expired-check resume never gets its report", then §1 says "every".
2. **One delayed Underflow that retires two entries.**
   ```
   expired check resumes burst B (E_B.resumes_at = R1)                     # tx.rs:1008-1010
   part at R1 handed (E_P), found expired, resumed again (E_P.resumes_at = R2)
   one Underflow, tick >= R1: retire takes both plans                      # tx.rs:398 `pending.resumes_at.take()`
   event built from reached.last() only                                    # tx.rs:1421-1427
   ```
   Outcome: two gaps, **one** `TX_UNDERFLOW` (carrying R2); R1 is taken and dropped. Pinned by `mt3d.rs:469-503` (`assert_eq!(emitted(..), [("underflow", ..)])`), and it is UR-28's text (`design/18-uhd-radio.md:355`: "the `TX_UNDERFLOW` it makes carries the newest burst's `resumes_at`").
3. **A second channel's, or a stale, Underflow** gets an event with `resumes_at` absent while the repeat does resume (`mt3d.rs:350-367`: `[("underflow", Some(36_000)), ("underflow", None)]`; `mt3d.rs:243-256`). So the draft's "one `radio.TX_UNDERFLOW { cause: starved, resumes_at: R }` per underflow report (one per channel)" is wrong: UR-28 says "once" (`design/18-uhd-radio.md:354`).

Wrong outcome: past N (rows and part records omitted), the gaps of 1 and 2 are in no event, no `timing` row and no burst record — only inside the number `omitted.timing.tx_resume`. A reader who follows the texts and rebuilds the silence from events undercounts it, and the amended SC-25 is false as written.

Fix, lean (recommended, both):
- Word the promise as what holds: "every resume is counted exactly (`tx_resume` kept + omitted); a drop the device reports is told by an event; `resumes_at` is carried once per resume and its absence does not mean the repeat ended".
- Two fixed-key counters in the existing `stats`, written where the row is (`tx.rs:1115`): `tx_resumes` and `tx_gap_samples` (Σ planned `R − H`, a lower bound when `R` moves). They are O(1), never bounded, need no new section, and make the total silence independent of both rows and events. The draft rejected option (iii) because "every gap's span is in its event" (§2.3); sequences 1 and 2 remove that reason.

Not recommended: making the expired check emit its own event. It reopens the "once" rule of UR-28 and the tests around it for a rare case.

### M2 — The bound also cuts healthy, Action-paced rows; the draft's reason ("the Action's outcome stays complete elsewhere") is false for two of them

Draft §2.3 (third row) accepts that "a per-Action diagnostic kind is cut only past 1 000 of that kind, and the Action's outcome stays complete elsewhere: the event, `applied`, `switches`, the booked burst's record, the action log". Principle 1 says a record "grows with what the user asked for, never with how long a fault lasts", and §2.5 keeps `applied` complete because "a 10 000-point sweep has 30 000+ rows that the user wants".

Sequence: a clean Run, no fault, 5 000 timed one-shot bursts (or 1 001 TX `Stop`s, or a 1 001-segment receive sweep).

| Kind (per-kind budget) | Writer | Paced by | Complete elsewhere? |
|---|---|---|---|
| `async` `BurstAck` | `tx.rs:1397-1401` | one per device burst per channel | **No.** No event exists for an ACK (`tx.rs:1407-1409`); the row is the only per-burst evidence that the device finished it, and its `tick` is what readers match (`crates/ezsdr-radio-uhd/tests/common/e3.rs:55`, `n47s5.rs:798-805`) |
| `timing` `tx_stop_applied` | `control.rs:318` | one per TX `Stop` | **No** (INFERRED): the Stop's applied instant and the `bound` its bookings end at have no other writer I could find (`grep -rn "tx_stop_applied" crates/ezsdr-radio-uhd/src/provider/` finds `control.rs:318` and its test at `:808-818`; no event is made there, `control.rs:314-320`) |
| `timing` `rx_segment`, `first_rx_block`, `rx_stop_untimed`, `rx_stop_late` | `rx.rs:415,678,287,468` | one per receive Action | Partly (the SampleClock registry) |
| `rejected` `kind: tx_burst` etc. | `core.rs:1070` (`command_rejected`) | one per refused Action | Yes (the event) |

Wrong outcome: a Run in which nothing went wrong loses 4 000 `BurstAck` rows and their ticks. Today it loses none. The owner's Q2 answer ("bound `async` (by code)") was given on the draft's "complete elsewhere" sentence, so this is a premise to bring back (AGENTS §9, "when a review refutes the premise").

Complication the draft did not state: `BurstAck` is not purely Action-paced. Each resume's two-zero close is a closed device burst, so a stuttering repeat may add ACK rows per resume (the draft's §9 "not checked"). So `BurstAck` cannot simply be exempted.

Alternatives, best first:
1. **Default-bounded with a named opt-out** for the kinds that only an Action can write: `tx_stop_applied`, `rx_segment`, `first_rx_block`, `rx_stop_untimed`, `rx_stop_late` (one `const`). This is not the draft's rejected "explicit list of bounded kinds": a writer added later is bounded unless someone exempts it on purpose, so forgetting is safe. `BurstAck` and `rejected` stay bounded.
2. For `BurstAck`, say it plainly in the KL entry and UR-30: "a Run with more than N device bursts keeps the first N acknowledgements; raise `EZSDR_UHD_RECORD_ROWS_PER_KIND` before a long pulsed Run". That is what the override is for, but the user must know before the Run.
3. Reword principle 1; as written it is false under the chosen rule.

### M3 — The override's read site is undecided, and the repository's precedent makes it silently ineffective on the bench

Decision Q3: "refused when the device opens, never silently replaced by the default". Nothing says where the variable is read.

- The precedent is the binary: `crates/ezsdr-server/src/main.rs:73-80` reads `EZSDR_PROFILE` and `EZSDR_SERVER_FAILSTOP_SECS`, with the comment "the library reads no environment" (`main.rs:73`; `design/16-easy-api.md:85`: "a value that is not one is a usage error, exit 2").
- The hardware and fake harness does not go through the server: it builds the Provider in-process with `UhdRadio::from_binding` (`crates/ezsdr-radio-uhd/tests/common/mod.rs:406`).

Sequence: the implementer follows the precedent and reads the variable in `main.rs`. On the bench: `EZSDR_UHD_RECORD_ROWS_PER_KIND=100000 cargo test … hw_tx200_hot_path_probe`. Outcome: N stays 1 000 and nothing is refused: exactly the silent default the owner ruled out, in the one place the override is needed. Whether rows are then cut depends on the Run: the measured Module trials stay below N (at most 14 underflows × up to 36 `TimeError` rows on #1, 10 × up to 48 on #2, draft §5: about 500 rows); a Module Run with as many underflows as bare UHD had (19–34) at 48 rows each would pass 1 000 (INFERRED, an extrapolation).

Decide in the design, not in the code review:
- **Read site:** one, in the Module, where both paths meet: `UhdRadio::from_binding` (`mod.rs:206`). Parsing is a pure function over `Option<&str>` so unit tests pass strings; tests that need a small N use a builder (as `with_measurement_timing`, `mod.rs:936`), never `std::env::set_var` (process-global, racy across the parallel test threads).
- **"Absurdly large" needs a number.** N is a memory budget per kind, and there are about 45 kinds (8 codes, about 18 `what`s, about 15 `rejected` kinds, the parts). At about 0.6–1.5 KB a row in memory (draft §2.2, INFERRED) N = 100 000 is up to about 0.1 GB per flooded kind. I suggest `1..=100_000`; the owner decides.
- **Order:** the server opens the device before it builds the Provider (`crates/ezsdr-server/src/catalogue.rs:140-145`), so a bad value would be refused after a real UHD open of several seconds, and once more in the reopen loop. Check the value before `open_uhd`.
- **The refusal's form** must be checked against EA-16 before the brief fixes it (AGENTS §9 step 1): through the catalogue it surfaces as `EA-7: <name>: …` (`catalogue.rs:108`), like the UR-5 selector refusals (`mod.rs:219-228`).
- **Better than an environment variable (for the owner):** a binding selector key, `record_rows_per_kind`, beside `block_len` and `reopen_on_unlock` (`mod.rs:219`). It is per Run, travels in the BindingProfile the Manifest already holds, works when the server is not a child of the user's shell, has the UR-5 refusal path already, and needs no process-global state. The draft rejected "a profile key" because "a profile describes the device" (Q3); a selector key is not a profile value. If the owner keeps the variable (it was his proposal), the points above still apply.

## 2. Minors

### m1 — `rejected`'s `kind` key is too coarse: a flood can hide the row that says why a repeat ended
Principle 3 claims "a one-off row is never dropped". Under §2.4 every row with `kind: "tx_burst"` shares one budget: per-Action refusals (`core.rs:1070`), "the late policy dropped the burst" (`tx.rs:1273`), "cancelled by Stop" (`tx.rs:1305`), and the one-off end notes "…burst not resumed" (`tx.rs:1014,1035,1060,1430,1461`).
Sequence: 1 000 dropped-late or refused bursts in a long Session, then a repeat ends because its resume point does not fit the clock (`tx.rs:1035`, a note with **no event**) → the note is omitted; the Manifest says only `omitted.rejected.tx_burst: 1`.
Fix: for a row with `kind`, the key is `kind` plus the rule id that starts `reason` ("UR-22", "UR-21", "UR-26"; static prefixes, so the cardinality stays small). Do not key on the whole `reason`: `command_rejected`'s reasons carry device text (`core.rs:1112-1115`).

### m2 — `async` `BurstAck` mixes a flush's acknowledgements with bursts'
Flush rows carry `tx_flush: true` (`core.rs:1238-1242`) and share the `BurstAck` budget. A recovery's flush ACK after 1 000 ordinary ACKs is omitted; `flush_evidence` counts exactly those rows as "T3's evidence in a Manifest" (`crates/ezsdr-radio-uhd/tests/common/e3.rs:109-114`; which bench cells call it I did not trace). Fix: the key is the code, plus `tx_flush` when the row is flagged.

### m3 — "The first 1 000 gaps are recorded in full detail" is false (draft §1; KL text)
The budgets are per code. `TimeError` rows are 1–48 per drop, mean 13.3 (draft P-2), so their budget is gone after about 75 drops, as few as 21. At the measured rate (14 resumes per 300 s) that is about 27 minutes, not the worst case. `Underflow` lasts 1 000 drops on one channel, 500 on two. Write "the first N rows of each kind", and give the `TimeError` figure in the KL entry.

### m4 — Key cardinality is not bounded by construction
The bound is N × (number of kinds). `async`'s key is `format!("{:?}", report.code)` (`tx.rs:1399`), and `TxCode::Other(i32)` takes whatever UHD returns (`crates/ezsdr-radio-uhd/src/uhd.rs:1140`). The `call` and `kind` values are literals today (INFERRED: I read the call sites in the grep of `reject_note`, not every `command_rejected` caller), but nothing stops a later writer from putting device text in a key, which brings unbounded growth back, in the rows and in `omitted` itself. Fix: at most 64 kinds per section; the next one is counted and kept under `other`. One line, and it makes the "every" claim true without review.

### m5 — "One helper" does not cover the direct pushes, and one of them is under the lock
Draft §8 routes `Core::timing`, `Core::reject_note`, "the async push" and `Tx::record`. Direct writers today: `rec.rejected.push` inside `Tx::record` while it holds `rec` (`tx.rs:817,827`), `rec.bursts.push` at `tx.rs:819-823,860,974,1178`, `device_async.push` at `tx.rs:1401` and `core.rs:1239`. Calling `Core::reject_note` from `tx.rs:827` would lock `rec` twice on one thread. The fields are `pub` (`core.rs:786-793`) and tests read them in dozens of places.
Fix: make the bounded sections a small type whose only mutator is `push(kind, row)` and that derefs to `[Json]` for readers. Then the compiler finds every writer, "count first, then keep" cannot be bypassed, and the mutation row for "a writer that skips the count" is not needed. `record_bound` must be put in the same `rec` guard as the clones (`mod.rs:420-430`): UR-16 can leave a thread detached and still writing (`mod.rs:409-412`).

### m6 — The first N are per Run, so one stream or one early episode uses up every later one's rows
Draft §8: "Per Run, not per stream". Sequence: stream 0 drops 1 000 times; stream 1 (slot B) then underflows once → its `Underflow`, `tx_resume` and part record are all omitted. Same for a second, unrelated episode an hour into a Python Session. Principle 4 ("the onset is kept") holds only for the Run's first fault. The events stay complete, so this is detail lost, not a wrong count. Option: key by (kind, stream) where the row has `stream` (`tx.rs:1115,1399`); it changes `omitted`'s kinds, which the owner sees. I would leave it per Run and say so in the KL entry.

### m7 — The safety net for a wrong `m` goes with b′'s zero case; say what the loop looks like
`m` = 10 ms is INFERRED until C4 and C7 (`design/18-uhd-radio.md:264`; draft §5 "Not measured"). The bench verified resumes after **underflows** on #1 (`bench/2026-11-16-stage4/README.md`, T200 probe row); no hardware run has resumed after a **late drop**. If a part's start lands inside the drain of the drop before it, every part is dropped late, and the repeat radiates nothing while it keeps resuming, up to 100 times a second, each cycle one `TIME_ERROR { late_at_device }`. With the zero case it ended at the second drop. The fake already shows the shape: `ur_28_the_device_reports_a_late_burst` uses `SlowSend(20 ms)`, which makes every send late (`fake.rs:2576-2597`).
This is a consequence of decision 1, not a reason to keep b′. It needs (a) a KL-13 sentence: "a repeat whose every part the device drops late transmits nothing and keeps resuming; its events are all `late_at_device`", and (b) C7's late-start cell before anyone relies on a late-dropped repeat recovering (INFERRED until then).

### m8 — A stuttering host makes more two-zero closes, so more chances of the one ending that kills the stream
Each resume sends a close (`tx.rs:1089`). A short or failed close is `Uncertain` (`tx.rs:888-896`) → reopen with T3 (`tx.rs:257-259,900-968`); three unconfirmed attempts end the stream's clock for the Run (`design/18-uhd-radio.md:271`; `tx.rs:945-949,972-980`). Under b′ a host that could not keep up ended the repeat after two drops; now it can reach, after many cycles, "every later burst refused". INFERRED and unmeasured (it needs a close that comes up short). Each reopen also clears `unacked` (`tx.rs:1336-1342`), which is M1's sequence 1. One KL sentence; no code.

## 3. Checked and found sound (notes)

- **Late report reaching a part, one channel.** First `TimeError` with tick ≥ R: `retire` removes the entry, `dropped` → `hit` → `resumable` → `resume_repeat` (`tx.rs:1137-1143`). The other 1–47 rows of that drop: no entry, device burst closed → `!hit` → `Nothing`. One resume, one event.
- **Two channels.** The second channel's report finds the burst closed → `Nothing` (`tx.rs:1123,1137`). One resume, in either order. `mt3d.rs:385-389` (the last block) flips to "no second resume, no event".
- **Stale reports and the next part.** They reach the part only with tick ≥ its first instant (`tx.rs:381`; `crates/ezsdr-radio/src/staging.rs:608-613`). Unchanged; rests on `m` (m7).
- **Expired check, then the report.** `Nothing` carries the stored plan (`tx.rs:1425,1449`); no second resume. `mt3d.rs:258-276` becomes "the event carries R2, the repeat stays paused at R2".
- **Sequence loss in the gap** ends the repeat with no `source` guard today (`tx.rs:1125-1127`), so removing `Paused.source` changes nothing there. Sol's M2 guard protected only the b′ branch (`tx.rs:1132`).
- **Stop, Run end, loss in the gap:** `stop_now` → `end_open(true)` sends nothing with no device burst open and closes the record (`tx.rs:839-862,1310-1319`). Untouched.
- **Switch across the gap:** `gap_turn` (`tx.rs:1026-1076`) reads `lattice` and `planned`, never `stall` or `source`. `replace_paused` copies the whole `Paused` (`staging.rs:805-810`), so it compiles unchanged.
- **The 100/s bound holds, also for a tickless Underflow**, but the draft's argument skips a step. `underflowed(None)` is true (`staging.rs:611`) and tickless underflows are a known case (`crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:876-877`), so a report's tick does not pace resumes. What does: R(n+1) ≥ busy + m ≥ R(n) + 32 samples + m (`staging.rs:760-762`, `tx.rs:1101-1102`), **and** a part is handed only once the in-flight window reaches R + 32 (`crates/ezsdr-radio/src/segment.rs:129-136`), so the n-th part opens no earlier than R(n) − window. Consecutive opens are therefore ≥ m apart in device time, whatever the window (10 ms on x3xx, `crates/ezsdr-radio/src/x3xx.rs:411`; 200 ms in the test rig, `tx.rs:1635`). Put that step in UR-22's text or the bound's test.
- **`busy`** only moves forward and follows what was handed, played or not (`staging.rs:760-762`); a late-dropped part makes the next R later, never earlier.
- **`unacked`.** A later report on a channel retires every older entry of that channel (`tx.rs:376-405`). It grows by one per resume only if one channel of a multi-channel stream never reports anything again, not even an ACK (INFERRED rare; `retire` then scans a growing queue on the real-time thread, `tx.rs:375-383`).
- **Nothing rewrites a bounded row.** Only `switches` is rewritten (`core.rs:1178`). No production code reads `rec`'s vectors during the Run (the `rejected.len()` reads at `control.rs:1505,1530` are under `mod tests`, `control.rs:687`).
- **The book and its chains do not grow per resume.** `pause`, `resume_at` and `move_resume_point` write only `open.paused`, `open.next` and the device chain (`staging.rs:778-802`); the chain is a value the owner copies (`tx.rs:437`); `Bookings::issued` and `carried` touch only the one pending switch (`staging.rs:1392-1405`); `chains` gets an entry per booking and is pruned at each receipt (`staging.rs:1310-1316`). A resumed part is the same booking, so it adds none.
- **An event's `target` and an omitted part.** A part's correlation is named by its own start R (`tx.rs:787`), which is its record's `target`. Past N that record is not kept, so the event joins no record, as the draft says; nothing reads the join during the Run (UR-28's correlation is `unacked`, `tx.rs:373-408`). The first record and the closing record of every repeat stay joinable.
- **Other growth:** I found none the draft missed. Receive: `rx_restart`, `rx_error`, `rx_block_refused` (`rx.rs:498,512,671`). Control: `ref_locked_failed`, `time_read_failed` (`control.rs:656,667`). All go through `Core::timing`.
- **Absence and presence checks survive the bound** ("no `TimeError` row", "any `tx_resume` row": `fake.rs:2310,2344`, `tests/common/mod.rs:604`, `tests/common/measurement/h4.rs:284`): a kind that occurs keeps its first row.
- **The part predicate needs one fact the tracker does not hold:** whether the record now open was opened by a resume. `BurstRecord` cannot tell (`crates/ezsdr-kernel/src/stream/burst.rs:339-350` closes any start-while-open as `Discontinuity`). Keep it as one `bool` assigned at every start-of-burst send (`tx.rs:759-790`, `!first && paused.is_some()`) and read only in `record`'s `Discontinuity` arm (`tx.rs:820-825`): no reset at the four `tracker.stop()` sites, because those records are always kept. Decide in the test plan the one unclear case: an intermediate part closed by a **switch** that replaced the paused repeat (`first && paused`). By the predicate's words it is bounded; by "every repeat's closing record" it is kept. Either is safe (switches are Actions); pick one and pin it.

## 4. Omissions in the draft's lists

**Tests (§4.3):**
- `ur_28_the_device_reports_a_late_burst` (`fake.rs:2576-2597`) is missing, and it is the b′ zero case as an integration test: `late.len() == 2`, `late[1].resumes_at == None`, `records.len() == 2`, one `tx_resume`. After the change the repeat cycles for the whole 100 ms wait, and how many cycles fit depends on the wall clock. Rewrite it without counting cycles (first event's `resumes_at` = second record's `target`; every late event but possibly the last has `resumes_at`), or it becomes a `wall-clock-test-oracle` case (`design/18-uhd-radio.md:775` lists this test among those whose verdict must not depend on the test thread).
- Row-counting readers not in the audit table: `crates/ezsdr-radio-uhd/tests/hardware.rs:2090-2121` (`hw_h5_held_last_sample`: `burst_record_count` per case and `count_report_codes`, `tests/common/observations.rs:129-136`), and a second call site of the listed `evidence_from_sections` (`tests/common/measurement.rs:3372-3376`). The other `async` readers I checked are in the table already or only print the section (`hardware.rs:1382,2226`). Cheaper than auditing each: the shared `section()` helper in `tests/common/mod.rs` fails when `record_bound.omitted` is not `{}`, with an opt-out for the bound's own tests.
- `ur_30_the_sections_are_written` (`fake.rs:2862-2866`) gains `record_bound`, not `omitted` (decision Q4 renamed it).

**Mutation rows (§4.4):**
- **MT3D-81** (`let source = self.device.first();`, killer `txbook_a_pause_remembers…`): retire. Not listed.
- **MT3D-15** (`TxBook::cursor`'s `paused.handed`) and **MT3D-38** (the "dropped the burst late; burst not resumed" note and its abandon): their killer is `mt3d_a_second_drop_found_in_the_gap…`, which no longer ends the repeat. Retarget (MT3D-15 to the sequence-loss-in-the-gap test; MT3D-38 to a one-shot burst's late drop, `mt3d.rs:279-296`).
- **MT3D-55** (`replace_paused`'s `paused: Some(paused)`): its killer is `mt3d_a_switch_part_keeps_the_stutter_bound…`. Retarget before that test is deleted.
- MT3D-58 and MT3D-59 keep their killer (`txbook_a_switch_replaces_a_paused_repeat…`); only its fixture changes shape.

**Texts (§4.1, §4.2):**
- KL-13's **Workaround** also says it: "a host that cannot keep up ends it" (`KNOWN_LIMITATIONS.md:130`). The draft changes only the Status sentence (`:132`).
- KL-5's Workaround: "(two close together can still end it: KL-13, issue #106)" (`KNOWN_LIMITATIONS.md:48`).
- UR-30: "`rejected` (every `COMMAND_REJECTED` reason, …)" and "the `bursts` section then holds one record per part" (`design/18-uhd-radio.md:371-372`) are both false past N; name them in the amendment.
- UR-28's "once" for `resumes_at` and "tells each" (`design/18-uhd-radio.md:354-355`): align with M1's wording.
- RM-22's comment "resumes_at: absent unless a repeat resumes" (`design/07-radio-model.md:229,235`) stays true, but KL-13 should add "absence does not mean the repeat ended": after b′ a second channel's event lacks it on every drop.
- The draft's own KL-13 text says "stutter" (§4.2), which decision Q5 rules out.
- Doc comments: `staging.rs:383-384,389,786`; `tx.rs:102,115,1103,1119-1121`; `crates/ezsdr-radio/tests/staging.rs:3152`.

**Obsolete code (§4.5):** `remember_burst` (`tx.rs:349-352`, `#[cfg(test)]`) becomes identical to `remember_part` once `stall` goes: merge them. `Dropped::Nothing`'s doc and `dropped`'s `newest` parameter (`tx.rs:1122`) go with the block. `DeviceChain::first()` stays (the debug assert at `tx.rs:611,722`).

## 5. Not checked

- No build, no test, no UHD source read: that the X300 stamps every `Underflow` and how it treats a start of burst inside a drain are INFERRED from the repository's own notes.
- Whether any single hardware Run today exceeds 1 000 rows of one kind (the measured T200 Module trials do not, M3).
- Which bench cells call `flush_evidence` (m2), and whether the X300 acknowledges a resume's two-zero close (M2).
- `wait_for`'s cost over a growing `delivered` list (Q1's scope).
- Every caller of `command_rejected` for a non-literal `kind` (m4).
- The Python client reads no Provider section (`grep -rn "radio.uhd\|\.section\|sections\[" python/` finds only a PPS string at `python/tests/test_easy_api.py:1398`).

## 6. Summary

Majors:
1. **M1** — an expired resume whose late-drop report is never correlated, or one delayed Underflow retiring two entries (`tx.rs:398,1421-1427`; `mt3d.rs:451,469`) → a gap in no event, and past N in no row: draft §1, principle 5 and the SC-25 clause are false. Fix the wording; add `stats.tx_resumes` and `stats.tx_gap_samples`.
2. **M2** — a clean Run of 5 000 bursts (or 1 001 Stops, or 1 001 receive segments) loses rows that exist nowhere else (`BurstAck` ticks, `tx_stop_applied`): the Q2 premise "complete elsewhere" is false. Opt out the Action-only `timing` kinds by name; state the `BurstAck` cut in the KL entry.
3. **M3** — the override read in `main.rs` by precedent (`main.rs:73`) never reaches the in-process bench harness (`tests/common/mod.rs:406`): N silently stays 1 000. Fix the read site (`from_binding`), the range, the order before the device open and the EA-16 check; a selector key is the better form.

Alternatives recommended: the two `stats` counters; a push-only bounded type with at most 64 kinds per section; finer keys (`rejected` by kind + rule id, `BurstAck` with `tx_flush` apart); one `bool` set at each start of burst for the part predicate; the omission check in the tests' `section()` helper.

Omissions: `ur_28_the_device_reports_a_late_burst` (`fake.rs:2576`); mutation rows MT3D-81 (retire), MT3D-15, -38, -55 (retarget); KL-13's Workaround, KL-5 line 48, UR-30's two "every/one per part" clauses, UR-28's "once"; the `hw_h5_held_last_sample` row counts (`hardware.rs:2090-2121`); `remember_burst`.
