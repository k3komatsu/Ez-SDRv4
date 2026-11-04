# Brief: note 47 stage 1b — channel names in the radio selector

You are the implementer (Claude Sonnet subagent) in `~/local_work/Ez-SDRv4` (branch `main`, HEAD `b07e032` or later). Read `AGENTS.md` first.

**This step is risky** (it adds a second identity for a channel — a name — beside its number, and changes the selector string every Provider and profile check reads). Two phases. **Phase 1 (now): write the test plan only, report, and stop. No code.** Phase 2 starts only after Sol has reviewed the plan and the orchestrator says so.

## Why and what is decided

- Design: `plan/phase8/47-no-change-while-streaming.md` §2.4, the bullet **名前** (owner 2026-11-04) and §4 stage 1b. Read them in full. Stage 1 (just committed, `b07e032`; records `plan/phase8/reviews/47s1-*`) built the path this step extends: Python `connect(rx=, tx=, radio=)` and `ezsdr.Stream` (`python/ezsdr/session.py`), the wire `initial` (`crates/ezsdr-server/src/protocol.rs`, `ChannelRef::Name` already exists and is refused as "needs stage 1b"), the server's `crates/ezsdr-server/src/initial.rs` (`expand`, after assemble and before `coordinator::connect`), `Connected.streams`.
- The decisions (owner; do not relitigate):
  - A channel name is written in the radio's BindingProfile selector, after `#` in a layout entry: `"0:A:ubx:0#loop-rx"` (RM-28 amended). An entry without `#` stays as today and is reachable by number only.
  - A name is unique in the **whole profile** (across radios): a name alone says which radio and which channel. Its characters follow stage 1's stream-name rule (non-empty, not only ASCII digits, no `:`, `#`, whitespace or control character); keep one rule, shared with the stream-name check, not a third copy.
  - Everywhere in the Easy API an `int` is a number and a `str` is a name, mixable. A stream whose channels belong to two radios is refused.
  - Resolution: names in `connect`'s arguments are resolved by the **server** from the profile's selectors (the same place as stage 1's layout-bound refusals, before any Run). After `connect`, the **client** resolves names (e.g. `start(frequency={"loop-rx": …})` in later stages, `sdr.rx_channels["loop-rx"]` now) from a name table the `connected` reply returns.
  - The Kernel, Actions, keys, the action log and records keep **numbers**; the name ↔ number map is readable from the recorded profile. The Kernel learns nothing about names.
  - Every place that compares or reads a selector string strips `#…` first (MockRadio, the UHD Module, profile matching, parity, test helpers). A named selector must behave exactly as the same selector without names.

## Scope of stage 1b (only this)

1. RM-28's selector grammar with the optional `#name`, parsed once in `ezsdr-radio`'s layout code; per-radio refusals there (bad characters, a name used twice in one radio).
2. Cross-radio uniqueness, checked by the server for the whole profile on every `connect` (whether or not names are used in the call).
3. Server resolution of `ChannelRef::Name` in stream channel lists and in value-map keys (`frequency`, `gain`, `antenna`), choosing the owning radio. Decide and state in the plan how the wire carries a stream whose channels are only names (the client cannot know their radio before `connect`): the orchestrator's preference is that the client may send such a stream under any resource key (default `radio`) and the server moves it to the radio that owns the names, refusing a stream mixing radios, and refusing a number whose radio differs from its names' radio. Propose the clearest rule; the orchestrator will rule.
4. The name table in `connected` (per radio, per direction, name → number), the reply schema, and Python: `connect` accepting `str` channels and `str` value keys (client-side checks stay: type, duplicates within a direction by number and by name where it can tell), `rx_channels[...]`/`tx_channels[...]` accepting a name.
5. Every selector reader stripping `#…`; MockRadio and FakeDevice/UHD behaving the same with and without names; parity.
6. Specs: RM-28 (and any RM/MR/UR/EA rule that quotes the selector), EA-10a, EA-16, in the accepted-spec format, with Changes rows. No version bumps, no SCHEMA_CHANGELOG; regenerate changed schemas.

Out of scope: stages 2–6 (membership fixed, staging, TX idle, overlap, cleanup); names in `start()`/`repeat()` beyond what `rx_channels[name]` needs (they come with stages 3–4 using this table); names in the Kernel or records.

## Phase 1 deliverable: the test plan

Write `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s1b.md`. First list **every place** that reads, compares or hashes a radio selector string (search `selector`, `rx`/`tx` layout parsing, `selector_id`, profile matching, parity helpers, `x3xx_selector`/`layout_selector`/`mock_x3xx_selector` in `ezsdr-acceptance`, the UHD Module's layout and device-record code, Python), each `path:line` with what it does with the string, and the exact searches you ran. Then for each sequence: the test (crate, name), what it asserts, how it observes it, and the wrong behaviour that would fail it.

1. A selector with names → assemble → connect with numbers only: the same layout, effective values, coerce, Provider behaviour and Spec hash inputs as without names (except the recorded profile text); MockRadio and FakeDevice alike.
2. Name rule refusals in a selector (each bad character class, digits only, empty after `#`, `#` twice, the same name twice in one radio) → refused with a named error at the right stage, no Run.
3. The same name in two radios → refused at connect, no Run, even when the call uses no names.
4. `connect(rx={0: ["loop-rx"]})` and value keys by name → the right radio, direction, channel number and value; names and numbers mixed in one stream; a name of the other direction (a tx name in `rx=`) refused; an unknown name refused; a stream mixing two radios refused; a number on a different radio than its names refused.
5. The `connected` name table → `sdr.rx_channels["loop-rx"]` is channel n of the right radio; an unknown name raises `KeyError` naming the known ones; the table survives a profile where only some entries are named.
6. Portability: one Python program using only names runs on two profiles that put `loop-rx`/`loop-tx` on different numbers (e.g. a Mock x3xx layout with reordered entries) and gets the loopback on both.
7. Records: the action log, keys and Manifest carry numbers only; the recorded profile carries the names.

Also list: every existing test, helper, fixture and mutation row that reads a selector and how it changes; **mutation rows you plan**, one line each, ids from the next free P id after the current maximum in `plan/maintenance/tools/mutations.json`, token anchors for Rust and exact substrings otherwise, each with its expected killer; for every time read or wait a test adds, why its verdict does not depend on thread timing (`plan/phase8/37-test-thread-timing.md`).

## Rules

- Local cargo allowed; build output in `~/.cache/cargo-target/Ez-SDRv4`; test temp files on `/tmp`. Python with `PYTHONDONTWRITEBYTECODE=1`, venv `~/.cache/ezsdr-venv`.
- Never run `mutate.py` or `remote_mutate.py`; never apply a mutant to the tree. In phase 2, out-of-tree probes count a compile failure as **not killed**, name the killing test for every row, and save output to `/tmp/s1b-*.log`. Append rows to `mutations.json` without re-serializing it.
- Never git commit, push, stash, reset or checkout. One agent edits the tree at a time.
- No step-5 self-review (owner 2026-11-03); Sol reviews the plan and the code.
- In phase 2: `cargo fmt --all` before reporting; `git ls-files -m -o --exclude-standard | xargs chmod go+r`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` passes.

## Report (phase 1, at most 15 lines)

The plan's path; the wire rule you propose for name-only streams; the list of selector readers (count, and any you think risky); anything in the design you think is wrong or unclear; open questions.
