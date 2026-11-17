# #106 design draft — a repeat resumes after every drop, and the UHD Module's per-Run records are bounded

AGENTS §9 step 0. Draft by an Opus subagent, 2026-11-17, read-only against `main` at 081fedeb. Every code claim carries `path:line`; hardware claims are marked VERIFIED or INFERRED (§5).

## 0. Recommendation, and the brief's premises that do not hold

**Recommendation.**
- Drop b′ whole, its zero case included. A late report that reaches a resumed part resumes it again, as decision 1 says ("every late report that reaches it").
- Bound the Module's own records **per kind**. In each of `async`, `timing` and `rejected`, the first N = 1 000 rows of each kind are kept in write order. The kind is the row's `code`, its `what`, or for `rejected` its `call`, `kind` or identifying field (§2.4). Every later row of that kind is counted, not kept.
- Treat a repeat's intermediate parts (records closed by a later resume, `end: discontinuity`) as one more kind under the same N.
- The counts go to a new, always-written section `ezsdr.radio.uhd.omitted`.
- `applied`, `switches` and the record of every booked burst stay complete: they grow only with the user's Actions.
- The Kernel's event list is not bounded by this step. It still grows by about one event per resume, and that is owner question Q1.

**Premises of the brief found false or imprecise.**

| # | The brief or issue says | What the evidence shows |
|---|---|---|
| P-1 | Decision 2: bound the records "the way the Kernel bounds events (RS-33)", so that a Run whose host cannot keep up does not grow memory or the Manifest without bound. | The Kernel bounds only hot-path bodies **between two drains** (RS-34's ring, `EVENT_RING_DEPTH` = 4096, `crates/ezsdr-kernel/src/coordinator/mod.rs:30`). Control-path events are never dropped (`crates/ezsdr-kernel/src/event.rs:667-674`). Every drained event is appended to `Shared.delivered` for the whole Run (`crates/ezsdr-kernel/src/coordinator/stepping.rs:230`, `state.rs:177`), and each `mark_artifact` event to `marks` (`stepping.rs:219-221`, `state.rs:178`). All of them reach the Manifest (`crates/ezsdr-kernel/src/coordinator/ending.rs:146`). A resume emits about one control-path event per channel (`radio.TX_UNDERFLOW` or `TIME_ERROR`, RM-11; none only when an expired-check resume never gets its report). So **the Module's bound alone does not meet decision 2's goal** (Q1). |
| P-2 | "about 10–30 raw async rows" per resume | The 11-16 hot-path logs show 57 underflows on both boards, followed by 1–48 `TimeError` rows each (median 10, mean 13.3; 756 rows in all). VERIFIED (§5). |
| P-3 | "bare UHD underflows 19–34 times (1996 or 1 M samples per send)" | 1 996 per send: 19–34 on both boards. 1 M per send (board #1 only): 136, 22 and 21. The bare tool sends every packet untimed (`plan/phase8/tools/uhd-repro/tx_repro.cpp:500-503`), so it reports no `TimeError` at all. The Module's 1–48 `TimeError` rows per drop come from UR-22's timed sends (every payload send names its instant). They are the price of decision 3, not noise. |

**Two answers to the brief's lists.** These are findings, not errors in the brief.
- **`applied` and `switches` stay complete, against the owner's list.** Neither grows with a fault.
  - `applied` is written only by `Core::configure` (`core.rs:1186-1224`) and the receive stop (`rx.rs:291`).
  - `Core::configure`'s callers are `prepare` (`mod.rs:528`), a receive segment's begin (`rx.rs:355` → `core.rs:1630` → `control.rs:132-137,70-84`) and uhd-tx's configure turn (`tx.rs:575`).
  - The configure turn is reached only for a **held** booking's dispatch (`tx.rs:491-493`; `values_turn` reads only `held`, `tx.rs:539-548`). A resumed part is the open booking and never reconfigures, even after a recovery's reopen clears the mark (`core.rs:711-716`). A receive restart after `LateCommand` calls only `rx_start` (`rx.rs:487-509`).
  - So every `applied` row follows an Action or the Run's start or stop. (Search: `grep -n "\.configure(\|configure_tx(\|owner_configure(\|configure_turn(" crates/ezsdr-radio-uhd/src/provider/*.rs`.)
  - `switches` gets one row per accepted switch Action and is **rewritten in place** while the Run lasts (`core.rs:1178` → `crates/ezsdr-radio/src/staging.rs:1693-1702`), so a dropped row would lose its settled outcome.
- **More than three tests fall.** Decision 1 also removes b′'s zero case (a late report reaching a part), so seven tests change (§4.3).

## 1. The user-facing contract

### During a Run whose repeat stutters
- **The radio.** Each underflow, late drop (`TimeError` reaching the open part) or expired timeline goes the same way:
  - The device burst is closed at the handed cursor H.
  - The next part starts at R, the first lattice point at or after max(busy + m, now + device lead, device front). This is UR-22 as written today.
  - [H, R) is silence: never transmitted, never sent again, never filled.
  - The gap lasts at least m = 10 ms. At most it is m plus one waveform period plus the host's own lateness (KL-13).
  - Each part starts at phase 0 of the waveform. **The repeat never ends because it stutters.**
- **What still ends a repeat** (unchanged, out of scope): a sequence-loss report (also while the repeat waits to resume), a failed or short send, a failed reopen, a lost device, an R or switch boundary that does not fit the clock, a `Stop`, and the Run's stop. A burst that is not a repeat still ends at its first underflow (decision 3).
- **Rate bound.** Two successive resume points of one stream are more than m + 32 samples apart in device time, so a stream resumes **at most 100 times a second**. Why: a resume needs an open device burst (`resumable()`, `crates/ezsdr-radio-uhd/src/provider/tx.rs:989-1001`; `dropped` does nothing when `!hit`, `tx.rs:1137-1139`). The part's whole first send of at least 32 samples moves `busy` to at least R + 32 (UR-22, `design/18-uhd-radio.md:264`). The next resume point is at least busy + m.
- **Events.** All are control-path events, delivered and never dropped:
  - one `radio.TX_UNDERFLOW { cause: starved, resumes_at: R }` per underflow report (one per channel), from `<radio>/tx/s<k>`, its time the report's tick;
  - one `radio.TIME_ERROR { late_at_device, late_by_ns, target, resumes_at }` per Kernel burst of a device burst dropped late;
  - one `radio.TIME_ERROR { send_asap }` when a part starts after its planned R.
  - The default reaction is `mark_artifact` (RM-10), so the Run continues and open artifacts are marked.
- **Python client.** `Session.wait_for(["radio.TX_UNDERFLOW"], t)` and `Session.events()` return each of them (`python/ezsdr/session.py:721-736`). `repeat()` and `switch()` behave as today, and a switch across a gap is accepted (`session.py:500-511`, EA-16). Two ways to end a stuttering repeat:
  - `tx.stop()` when the first event arrives. This is the only way from a Python Session on the USRP, which runs no child Run (KL-24).
  - In a Spec Run driven from Rust, a Policy entry `radio.TX_UNDERFLOW: stop` (RS-26), which stops the whole Run. INFERRED: not checked on a device-paced Run.

### After the Run (the Manifest)
| Where | What |
|---|---|
| `events.delivered`, `events.counters` | Every drop's event, complete (KC-29a). The counters are exact per `(<radio>/tx/s<k>, kind)` (RS-33). |
| `ezsdr.radio.uhd.timing` | One `tx_resume { stream, cause, from: H, resumes_at: R, at }` per resume, **first 1 000 of the Run**. |
| `ezsdr.radio.uhd.bursts` | The record of every booked burst. The first 1 000 intermediate parts (`end: discontinuity`) of the Run. Every repeat's closing record. |
| `ezsdr.radio.uhd.async` | Raw reports, **first 1 000 of each `code`**. |
| `ezsdr.radio.uhd.rejected` | First 1 000 of each kind. After this change a resume writes none (it never did, UR-22). |
| `ezsdr.radio.uhd.omitted` (new) | `{ "<section>": { "<kind>": count } }` for every kind that lost rows. It is `{}` when every row was kept. |
| `ezsdr.radio.uhd.stats` | Exact counters as today. `tx_bursts` counts first sends only (`tx.rs:791-793`), so it counts no parts. |

### What a user can rely on
- The radio never fabricates or replays samples, and never plays untimed: the gaps are silence.
- Every drop is told by an event whose `time` and `resumes_at` bound its silence.
- The count of every kind is exact: the rows kept plus `omitted`. For example, resumes = `tx_resume` rows + `omitted.timing.tx_resume`.
- The first 1 000 gaps are recorded in full detail.
- One-off rows are never crowded out by a frequent kind: `plan`, `stop`, `arm`, `start`, `device_lost`, `leaked`, a thread's `did not join`.
- The outcome records are complete: `applied`, `switches`, every booked burst.

### What a user cannot rely on
- That the repeat ends by itself.
- Rows and part records past the first 1 000 of their kind. A `late_at_device` event whose `target` names an omitted part joins no record.
- The number of `TimeError` rows the device sends (KL-45, unchanged).
- A bounded event list (Q1).

## 2. The bound

### 2.1 What grows today, per resume (one channel)
| Record | Rows per resume | Writer | Read back during the Run? |
|---|---|---|---|
| `async` | 1 `Underflow` + 1–48 `TimeError` (§5) | `Tx::report`, `tx.rs:1397-1401` (all codes); flush rows `core.rs:1238-1242` | No: UR-28's correlation reads `unacked` (`tx.rs:373-405`), never the rows. The only production read is `write_sections` (`mod.rs:426`). |
| `timing` | 1 `tx_resume` | `tx.rs:1115` → `Core::timing`, `core.rs:1037-1039` | No (`mod.rs:429`) |
| `bursts` | 1 part record | `Tx::record`, `tx.rs:815-828`; also `tx.rs:860`, `:974`, `:1178` | No: SC-24's tracker keeps only the open burst (`crates/ezsdr-kernel/src/stream/burst.rs:268-271`) |
| `rejected` | 0 (more only on a double fault: a failing close, `tx.rs:736,752,889,894`, and its T3 attempts, `core.rs:1266-1272`) | `Core::reject_note`, `core.rs:1041-1043` | No |
| Kernel `delivered` + `marks` | about 1 event + 1 mark per channel | `event.rs:673`; `stepping.rs:219-230` | Yes: KC-29a indices, `wait_for` |

Other kinds that can grow with time and no Action:
- receive side: `rx_error` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:512`, one per failed `recv`), `rx_block_refused` (`rx.rs:671`, one per refused block), `rx_restart` (`rx.rs:498`, one per missed timed start);
- control side: `ref_locked_failed` (`control.rs:656`) and `time_read_failed` (`control.rs:667`), one every 500 ms while the reads fail.

Things that do **not** grow per resume (checked):
- `unacked`: a later underflow on a channel retires every older pending entry of that channel, and an entry leaves once all its channels are retired (`tx.rs:373-405`). On a two-channel stream, an entry stays until both channels have reported;
- the book's chains: per booking, pruned (`crates/ezsdr-radio/src/staging.rs:1131,1312,1351`);
- `Paused`: one per open booking (`staging.rs:383-398`);
- the SampleClock: a part uses the stream's clock (`tx.rs:1082-1116`);
- `stats`: fixed keys (`core.rs:796-803`).

### 2.2 Size, before and after (INFERRED arithmetic; sizes not measured)
The Manifest is written pretty-printed (`crates/ezsdr-server/src/lib.rs:431`). On disk that is about 0.12 KB per async row, 0.2 KB per `tx_resume` row, 0.4 KB per burst record and 0.45 KB per event. As `serde_json::Value` in memory, it is roughly 0.6, 0.8, 1.5 and 0.7 KB.

So one resume (mean 13.3 `TimeError`) costs the Module about 2.3 KB on disk and 11 KB in memory, and the Kernel about 0.5 KB and 0.8 KB. `write_sections` clones every vector (`mod.rs:424-429`) and the server builds the whole text (`lib.rs:431`), so the peak at cleanup is about three times the held size.

| Rate | Module today (disk / memory per hour) | Module bounded | Kernel (unchanged) |
|---|---|---|---|
| measured, 200 MS/s on this host: 14 resumes per 300 s | 0.4 MB / 2 MB | same | 0.08 MB / 0.1 MB |
| worst, 100 resumes/s (§1) | 0.8 GB / 4 GB | under 1 MB / 5 MB in all | 0.16 GB / 0.3 GB |

### 2.3 Alternatives
| Option | Verdict | Reason |
|---|---|---|
| (i) First N rows **per kind** plus an exact count of the rest (RS-33's pattern: count first, keep the body only within the bound) | **Kept** | Simple and deterministic. It keeps the onset of a fault, which is what diagnoses it. A rare kind (a sequence loss, a device loss, the `plan` row) has its own budget, so a flood never hides it. Readers that look for "any row of kind X" still work. |
| Section-wide first N | Rejected | The load-bearing rows are written **last**: `plan` and `stop` (`mod.rs:713,729`), `device_lost` (`core.rs:1098`), `leaked` and `did not join` (`mod.rs:744,411`). A flood of `tx_resume` would push them out. |
| Bound only the kinds that grow with time (an explicit list of the writers in §2.1) | Rejected | A writer added later and not listed would grow without bound again, which is the defect being fixed. Under (i), a per-Action diagnostic kind is cut only past 1 000 of that kind, and the Action's outcome stays complete elsewhere: the event, `applied`, `switches`, the booked burst's record, the action log. |
| (ii) First N plus last M | Rejected | A ring per kind, and a reader must know that a hole lies between the two halves. The end of a Run is already told by one-off rows, which (i) never drops, and by the Kernel's termination. The one record whose end matters, a repeat's closing record, is kept by name (§2.4). |
| (iii) Count-only summaries per gap cause (`resumes`, the summed silence), raw rows up to N | Rejected | The exact counts already come from (i) plus the Kernel counters, and every gap's span is in its event. A summed silence feeds no rule, and it adds an aggregator with new fields. |
| (iv) Stream the rows to disk during the Run | Rejected | Disk still grows without bound. It needs an artifact kind for Provider records, which the Kernel does not have (artifacts come from Sinks). It also puts file I/O on uhd-tx, a real-time thread, under the records lock. |

### 2.4 The recommended rule (substance for UR-30; the wording is the implementing diff's)
- `async`, `timing` and `rejected` keep, in write order, the first N = 1 000 rows of each **kind** in the Run:
  - `async` by `code`;
  - `timing` by `what`;
  - `rejected` by its `call` value, else its `kind` value, else the name of the field that identifies the row: `thread`, `leaked`, `event_not_emitted`, `burst_tracker`, `clock_not_ended` or `clock_not_registered`. A row with none of these is kind `other`.
    - A named field list, not key order: `serde_json`'s map sorts its keys.
    - So `leaked` and a thread's `did not join` never share a budget with a repeatable writer: `event_not_emitted` (`core.rs:1050`, `rx.rs:629`), `burst_tracker` (`tx.rs:827`), `clock_not_*` (`rx.rs:326,590`).

  Every later row of that kind is counted and not kept.
- `bursts` keeps every record except a resumed part's record that a later resume closed (`end: discontinuity`). Of those, the first 1 000 in the Run are kept and the rest counted. Booked bursts' records and every repeat's closing record are always kept.
- `omitted` is written at cleanup with the others: `{ section: { kind: count } }`, `{}` when nothing was left out. The kind of `bursts` is `resumed_part`, as in `{ "bursts": { "resumed_part": 4210 }, "async": { "TimeError": 51000 } }`.
- `applied`, `switches`, `device`, `envelope` and `stats` are complete.
- A row is counted under the records lock **before** the keep decision, so kept + omitted = written, exactly.
- No event announces an omission. The rows are read only from the Manifest, which says so.

### 2.5 What must stay complete, and why
- `switches`: rewritten in place by `action_id` (`core.rs:1178`, `staging.rs:1693-1702`). It is also the one section EA-16's `switch` docstring sends the user to (`session.py:509-511`).
- `applied`: the experiment's own record of what was set. A 10 000-point sweep has 30 000+ rows that the user wants, and nothing but Actions grows it (UR-26).
- Booked burst records: SC-28's record of the user's own bursts. It is read typed as `Vec<BurstRecord>` (`crates/ezsdr-acceptance/tests/uhd.rs:175`), so no marker row can go inside the array.
- Nothing UR-28 or SC-24 reads during the Run is a row (§2.1). A capture's ContinuityMap is the Sink's and is not touched.

### 2.6 The Kernel's own records
For a stuttering repeat:
- The action log does not grow (one Action).
- No SampleClock is added (the same clock).
- No ContinuityMap grows (transmit has no artifact).
- No burst list grows (`burst.rs:268-271`).

What grows is `delivered` (about one event per resume per channel; the logs show `tx_underflow_events` equal to the `Underflow` count in all six trials) and `marks`. Each open artifact also receives the marks in its span at cleanup (KC-42; `crates/ezsdr-kernel/src/manifest.rs:352`).

The receive side has the same property: every `RX_OVERFLOW` body that the ring delivers stays in `delivered`. Bounding this is a Kernel change (RS-34/35, KC-29a's stable index, KC-29b's `wait_for`, KC-31/42), outside #106 (Q1).

MockRadio has no resume rule and no rows that grow with time:
- `grep -n -i "resum\|stall\|stutter\|underflow" crates/ezsdr-mock-radio/src/*.rs` finds only `resumes_at: None` (`lib.rs:980,1905`);
- its rows are per declared fault (`lib.rs:774`), per refusal (`:870`), per burst (`:884`) and per switch (`:1822`);
- the shared oracle has none either (`grep -rn -i "resum\|stall\|stutter\|paused" crates/ezsdr-acceptance/tests/oracle/`: no hit).

So the bound is UHD-only, and parity is unaffected.

## 3. Principles
1. **A record grows with what the user asked for, never with how long a fault lasts.** Outcome rows per Action are complete. Diagnostic rows, and the fault-driven split of one Action's record, are bounded.
2. **Count first, then keep.** Every row is counted before the keep decision, under the same lock. Kept + omitted = written, per kind (RS-33's "a count is never lost even when the body is").
3. **The bound is per kind,** so a frequent kind never hides a rare one, and a one-off row is never dropped (its count is 1, far below N).
4. **The onset is kept, not the end.** The first rows diagnose a fault. The end of a Run is in the one-off rows, every repeat's closing record and the Kernel's termination.
5. **Events are the complete log; rows are the detail.** This step bounds no event. Each gap stays told by its event (time, `resumes_at`) and counted by the Kernel.
6. **Nothing a rule reads while the Run lasts is bounded** (`switches`; and the correlation and tracker state, which are not rows).
7. **A reader learns of an omission in one place,** `omitted`, which is always present. An empty object means every row was kept.

## 4. Check against existing promises

### 4.1 `design/` texts the change makes false
| Text | Why false | Amendment (substance) |
|---|---|---|
| UR-22, `design/18-uhd-radio.md:266`: "A part ends the repeat by the abandon above when a late report reaches it ... or an underflow reaches it after it played less than the stall part ..." and "... applies the stall rule only when ..." | b′ and its zero case go | "An underflow or a late report that reaches a part resumes the repeat again. A sequence loss (also in the gap), a failed or short send, a failed reopen, a lost device, or an R or switch boundary that does not fit the clock ends it. A report read while the repeat waits changes nothing but its event." Keep the M2 sentence about a report's reach. |
| UR-22 `:267`: "The stall part of the gap is unchanged by a move: ... not counted by the stutter rule." | No stutter rule | Delete. |
| UR-22 test lists `:275-281` | Name b′ tests | Rename or remove per §4.4. |
| UR-28 `:352`: "records every asynchronous report in the `async` section" | Bounded | "... each asynchronous report, within UR-30's bound". |
| UR-28 `:355`: "the newest decides the repeat by UR-22's stutter rule" | No stutter rule | "... tells each; the `TX_UNDERFLOW` it makes carries the newest burst's `resumes_at`". |
| UR-30 `:370-374` | Sections complete; no `omitted` | §2.4 as a paragraph, plus `omitted` in the section list and its *Test:* line. |
| SC-28, `design/02-stream-contract.md:256`: "one `BurstRecord` per burst" | Intermediate parts past N not kept | One clause: "a Provider that resumes a repeat may leave out its intermediate parts' records past a bound its spec states, counting them (spec 18 UR-30)". Only if Q2 bounds parts. |
| SC-25, `:250`: "what the hardware radiated during the gap is recorded in the Provider's Manifest section" | Within the bound only | "... told by the event and, within the bound, recorded in ...". |
| RM-37 test list, `design/07-radio-model.md:368` | Names `mt3d_a_switch_part_keeps_the_stutter_bound_of_the_gap_it_replaced` | Rename. |
| `plan/phase8/bench.md:1583`, C7: "cap に達しない" | No cap | Reword. This is a plan text. |

Checked and still true:
- RM-11 (`resumes_at` absent when the repeat ended: rarer now, still right).
- RM-15 (a Provider may pause and resume; `07-radio-model.md:164`).
- RM-37's `cancelled` clause ("a repeat it cannot resume").
- RM-10 and RM-22 (no payload change).
- UR-23, UR-38, UR-33.
- RS-26…RS-36, KC-29a/b.
- EA-16 (no error, refusal or API meaning changes; `switches` stays complete) and EA-17.
- MR-*: MockRadio is unchanged.

Owner decisions of 2026-11-14 that this supersedes (`plan/phase8/reviews/mt3-decisions.md`). The new decision record should name them:
- M4, the b′ stutter bound (line 24);
- the b′ item of "Triggers/endings" (line 26);
- REPEAT RESUME's "One cap: dropped again before any sample plays" (line 12). b′ had already absorbed it as its zero case, and it does not come back.
- "Missed R": "not counted by the b′ rule" becomes moot (line 30);
- O5's "every raw report in async (bounded by what was handed)" (line 9). A repeat that resumes forever hands without bound, so `async` becomes bounded by UR-30.
- "KL and bench": KL-13's b′ sentence and the bench's "a host that cannot keep up (b′ boundary)" cell (line 31).

### 4.2 KNOWN_LIMITATIONS (the owner decides, Q5)
- **KL-13**, `KNOWN_LIMITATIONS.md:132`: replace "A part that plays less than the stall part of the gap before it ends the repeat ..." with "A host that cannot keep up makes the repeat stutter until it is stopped: each drop is a gap of at least 10 ms, at most 100 a second per stream. To end it, stop the stream when the first `TX_UNDERFLOW` arrives."
- **KL-39 note**, `:158`: drop "a part that played less than the stall part of the gap before it".
- **KL-5**, `:45-51`:
  - add that a repeat at 200 MS/s on this host now resumes after each underflow (14 per 300 s on #1, §5) instead of stopping on #2;
  - add the T200 decision (continuous 200 MS/s goes to 8c's Replay, `plan/phase8/bench-results.md:468`).
- **New entry** (records, Ceiling), with two parts:
  - The UHD Module keeps the first 1 000 rows of each kind of `async`, `timing` and `rejected`, and the first 1 000 intermediate parts' records, and counts the rest in `omitted`.
  - The Run's event list keeps every event, about one per resume and channel (Q1).
- **KL-45**: one line pointing to the bound for its `TimeError` rows.

### 4.3 Tests the change makes false
| Test | What changes |
|---|---|
| `mt3d_a_part_that_plays_less_than_the_stall_part_ends_the_repeat` (`src/provider/tx/tests/mt3d.rs:226`) | All four cases now resume, including the `TimeError` one. Rewrite as "an underflow or a late drop of a part resumes it again, with its event's `resumes_at`". |
| `mt3d_a_delayed_underflow_that_retires_two_entries_applies_the_stutter_rule` (`mt3d.rs:469`) | The `ends` cases and the late-drop sub-case now keep the second resume. Each entry is told, the part with its plan R2 (`Dropped::Nothing` carries the stored plan, `tx.rs:1422-1426`, `:1446-1450`). |
| `mt3d_a_second_drop_found_in_the_gap_abandons_at_the_handed_cursor` (`mt3d.rs:258`) | The `TimeError` on the paused part no longer ends the repeat. The "abandon at H, no stray close at R" property is already pinned by `mt3d_a_lost_sequence_while_the_repeat_is_paused_ends_it` (`mt3d.rs:1239`). |
| `mt3d_two_channels_late_resume_once_in_either_order_and_the_part_ends_on_one_channel` (`mt3d.rs:370`, last block `:385-389`) | A late report from one channel resumes the part again. Rename. |
| `mt3d_a_switch_part_keeps_the_stutter_bound_of_the_gap_it_replaced` (`mt3d.rs:1091`) | The P − 1 case resumes. Rewrite as "a switch part resumes like any part", or delete. |
| `mt3d_a_stopped_repeat_s_delayed_report_does_not_end_a_newer_repeat_that_waits_to_resume` (`mt3d.rs:1263`) | Still passes, but the guard it pinned (`Paused.source`, `tx.rs:1132`) is removed. Keep it as a plain regression, or delete it. |
| `txbook_a_pause_remembers_the_first_instant_of_the_device_burst_it_closed` (`crates/ezsdr-radio/tests/staging.rs:3226`) | `Paused.source` goes. Delete. |
| `txbook_a_paused_repeat_keeps_its_handed_cursor_and_its_plan_when_its_resume_point_moves`, `txbook_a_switch_replaces_a_paused_repeat_and_its_boundary_moves_with_the_resume_point` (`staging.rs:3165,3179`; fixture `:3154-3160`) | Their `Paused { stall, source }` literals and `resume_at(50, 100)` change shape. |
| `ur_30_the_sections_are_written` | Gains `omitted`. |

Readers that count rows, which the implementer audits against N (AGENTS §9 step 1, audit table):

| Reader | What it counts or reads |
|---|---|
| `crates/ezsdr-radio-uhd/tests/common/iii.rs:33-38` | `async_faults` count |
| `tests/common/measurement/multichannel.rs:874-912` | `async_failures`, `burst_count` |
| `tests/common/measurement.rs:3219-3221,3686` | evidence from `bursts`, `async`, `rejected` |
| `tests/common/n47s5.rs:177,798-805` | `BurstAck` counts |
| `tests/common/e3.rs:55,111` | `BurstAck` rows |
| `tests/common/mod.rs:600-606` | "any `tx_resume` row" (still true under (i)) |
| `crates/ezsdr-acceptance/tests/parity.rs` | `manifest_rows(..).len()` |
| `crates/ezsdr-acceptance/tests/uhd.rs:175,501` | `bursts` |
| `crates/ezsdr-server/tests/protocol.rs:1627,1651` | one-off `timing` rows |

Today all of these Runs stay far below 1 000 of a kind. A reader that counts must use kept + `omitted`, or assert that `omitted` is empty.

### 4.4 Mutation rows (`plan/maintenance/tools/mutations.json`, `plan/phase7/tools/mutations.json`)
| Action | Rows |
|---|---|
| Retire (b′ only) | MT3D-10, MT3D-11, MT3D-12, MT3D-13, MT3D-27, MT3D-28, MT3D-79, MT3D-80 |
| Re-anchor (their anchor holds the removed `paused.stall` or `resumed: stall`) | P350, MT3D-26, phase7 B25, B26 |
| Retarget | MT3D-37. Its killer test changes. The behaviour is still reachable through a sequence loss in the gap: the killer becomes `mt3d_a_lost_sequence_while_the_repeat_is_paused_ends_it`, whose "no send at R" kills the stray close. |
| Keep | MT3D-21, MT3D-22, U43, R13, B40, provided `Core::timing(row)` and `Core::reject_note(row)` keep their signatures (§8) |
| Re-anchor, if the push lines change | P1209, N48S1-05 |
| New | count-before-keep; the per-kind key; the intermediate-part predicate; the closing record kept; the `omitted` write |

### 4.5 Code that b′'s removal makes obsolete (maintainability item 4)
- The stall block in `dropped` (`tx.rs:1128-1135`) and the `newest` and `late` parameters as decision inputs.
- `PendingReport.resumed` (`tx.rs:101-103`), `Reached.resumed` (`tx.rs:121`) and `remember_part`'s `stall` (`tx.rs:355-363`).
- The stall in `resume_repeat` (`tx.rs:1103-1104`).
- `Paused.stall` and `Paused.source` (`staging.rs:383-398`, `pause` `:778-784`, `resume_at` `:788-792`).
- Do **not** touch `rx_stall` (`provider/mod.rs:189`, `rx.rs:105,220-228`): it is an unrelated bench hook.

## 5. Hardware evidence
**VERIFIED** (logs read for this draft):
- **#1, Module** (`~/.cache/ezsdr-bench/2026-11-16/hw_tx200_hot_path_probe.36.log`). 200 MS/s, 300 s, 20 000-sample repeat, three trials:
  - 14, 13 and 14 underflows, every one resumed;
  - 5.9971–5.9973e10 of 6.0e10 samples;
  - `TimeError` rows per underflow 1–36.
- **#2, Module** (`…hw_tx200_hot_path_probe.37.log`):
  - trial 1 resumed through 10 underflows (5.9979e10);
  - trial 2 stopped at 1.883e10 after 4, trial 3 at 2.68e8 after 2;
  - `TimeError` rows per underflow 3–48.
- **Both boards.** Every trial's `tx_underflow_events` equals its `Underflow` row count. Every trial is classed `DeviceFailure`, because the probe classes any underflow as a failure.
- **Bare UHD, untimed sends.** 1 996 per send: #1 26/19/32, #2 33/34/28, with `time_error 0`. 1 M per send on #1: 136/22/21 (`2026-11-16/tx_repro-t200-H0-*.log`, `2026-11-17/tx_repro-t200-spp1M-36-*.log`).
- **v3 on #1.** 49, 63 and 54 `U` characters in 300 s (`2026-11-17/chain17*.out`).

**INFERRED:**
- That b′ ended #2's trials 2 and 3. The Manifests were not kept; the `rejected` note "UR-22: the device burst underflowed; burst not resumed" (`tx.rs:1430`) would have said so.
- That a host that cannot keep up resumes near the 100/s bound. The bound is design arithmetic (§1). The 256 KB ingress buffer (about 0.33 ms at 200 MS/s, VERIFIED by the orchestrator per the brief) suggests a part runs dry soon after R.
- Every byte and memory size in §2.2.
- That a dropped part's `TimeError` rows never reach the next part: their ticks precede R. Same rule as today (UR-28, M2).

**Not measured:**
- A Run that stutters for minutes on end.
- A two-channel stutter (it doubles the per-channel rows).
- A resume rate above about 0.05/s.
- 100 MS/s resume behaviour (the brief's "2 of 10 trials underflowed" was not re-read here).
- Bench cells C4 and C7 (m is still INFERRED).

## 6. What the choice makes impossible later
- **No automatic end to a stutter.** A Run whose host can never keep up radiates a chopped waveform until the user's `Stop`, a Policy or the Run's end. A "fail fast on a repeat underflow" default would be a new rule.
- **Rows past N are gone.** A later analysis that needs every gap must use the events, so the Kernel's event list becomes the only complete gap log. A later Kernel bound on events (Q1) would need a per-gap summary or would accept the loss.
- **SC-28 becomes "one record per burst, intermediate parts past N counted".** A consumer cannot rebuild a long stutter's timeline from `bursts` alone.
- **A constant N needs a rebuild to change.** A profile key can still be added later without breaking anything.
- **The shape of `omitted` becomes something readers rely on** (pre-freeze, changeable until v4.0).

## 7. Questions for the owner, one at a time

**Q1 — The Run's event list (premise P-1).** The Kernel keeps every event for the whole Run (KC-29a). After the Module is bounded, a stuttering Run still grows by about one event and one mark per resume per channel: about 0.1 MB an hour at the measured rate, up to about 0.16 GB on disk and 0.3 GB in memory an hour at the worst rate (INFERRED).
- **Recommend:** accept it now and say so in the new KL entry, and put "bound a Run's delivered events per (source, kind), counters exact" on the freeze-review list (handoff §4).
- *Rejected:*
  - the Module stops telling resumes after N: this breaks UR-28 and RM-11 and `wait_for`, and the Kernel counters would stop counting them;
  - a Kernel bound now: it touches RS-34/35, KC-29a/b and KC-31/42 and the Python `events(start)` index (`session.py:721-736`), which is Kernel scope, not #106.

**Q2 — Which records are bounded, burst records included.**
- **Recommend:**
  - `async`, `timing` and `rejected` per kind;
  - plus the intermediate parts' records (first 1 000 in the Run; booked bursts' and every repeat's closing record always kept);
  - `applied` and `switches` complete. They grow only with Actions, and `switches` is rewritten in place.
  - SC-28 and SC-25 gain one clause each.
- *Rejected:*
  - leave `bursts` complete: SC is untouched, but `bursts` grows by one record per resume (up to about 0.14 GB an hour on disk at the worst rate);
  - bound every burst record: a long pulsed experiment would lose its own records, and an event's `target` would no longer find an unrelated later burst;
  - bound only an explicit list of time-growing kinds: a writer added later and left off the list would grow again.

**Q3 — N and its form.**
- **Recommend** a constant N = 1 000 rows per kind, not a profile value. N is a memory budget, not a device property (as `EVENT_RING_DEPTH` is a constant).
  - 1 000 is about 70 times the 14 resumes of a 300 s stutter at 200 MS/s on this host.
  - A full kind is about 0.1–0.4 MB on disk and 0.6–1.5 MB in memory.
- *Rejected:*
  - 4 096, the ring's depth: four times the size for no reader that needs it;
  - a profile key `radio.records.rows_per_kind`: a profile describes the device, and the key can be added later without breaking anything.

**Q4 — How the Manifest says what was left out.**
- **Recommend** a new section `ezsdr.radio.uhd.omitted`, always written: `{ "<section>": { "<kind>": count } }`, `{}` when complete. The kinds are those of §2.4, with `bursts` as `resumed_part`. The arrays keep their shape, so no reader breaks.
- *Rejected:*
  - keys `omitted.<section>.<kind>` in `stats`: workable and no new section, but flat, absent means 0, and `stats`'s key type changes;
  - each bounded section becomes `{ rows, omitted }`: every reader breaks, including the typed `Vec<BurstRecord>` read at `crates/ezsdr-acceptance/tests/uhd.rs:175`;
  - a trailing marker row: mixes shapes and breaks the same typed read;
  - an event at the first omission: the rows are read only from the Manifest.

**Q5 — The KNOWN_LIMITATIONS texts.**
- **Recommend** §4.2 as drafted: KL-13 (the stutter without end and how to stop it), KL-39's note, KL-5 (200 MS/s stutters rather than stops), a new records entry, and a line in KL-45.

## 8. Decided here (internal shape; the orchestrator may reverse it)
- **Where the bound lives.** One keep-or-count helper on `Records` (`core.rs:786-803`) with per-(section, kind) counts under the existing `rec` lock.
  - The kind is derived from the row inside `Core::timing`, `Core::reject_note`, the async push and `Tx::record`.
  - So every caller keeps its call, and the anchors of MT3D-21/22, U43, R13 and B40 survive.
- **Scope.** Per Run, not per stream.
- **The part predicate.** A record is an intermediate part when it was opened by a part's start of burst (`paused.is_some() && !first` at `tx.rs:759`) and closed by a later resume (`BurstStep::Discontinuity`'s `closed`, `tx.rs:819-823`). A switch that replaced a paused repeat opens a booked record (`first` is true), which is never bounded.
- **The b′ removal** is as in §4.5. `dropped` keeps:
  - a sequence loss while paused ends the repeat (`tx.rs:1125-1127`);
  - `!hit` → `Nothing`;
  - resumable → resume;
  - else end.
- **Step risk: risky.** It changes a lifecycle (when a repeat ends), so a test plan comes before the code (§9 steps 2–3).
- **Tests to add:**
  - the helper, pure: 1 001 rows of one kind → 1 000 kept, `omitted` 1; two kinds independent; a one-off after a flood kept;
  - wiring on the fake: a stutter past N with a pre-filled count keeps the closing record and the booked record, and counts the parts;
  - `omitted` is `{}` on a clean Run;
  - one test per bounded writer on the receive and control side (`rx_error`, `ref_locked_failed`) through the same helper.

## 9. Not checked
- No build or test was run; every "test changes" call above is from reading the assertions.
- Memory per `serde_json::Value` row was not measured.
- The 100 MS/s figures and the HG buffer figure come from the brief.
- Whether X300 sends a `BurstAck` for a part closed after an underflow (it affects only the async count).
- The hardware probe process ended in SIGSEGV after the test failed (`hw_tx200_hot_path_probe.37.log`, last line). Outside #106; not investigated.
