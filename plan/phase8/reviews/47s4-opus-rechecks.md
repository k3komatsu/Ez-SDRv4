# Note 47 stage 4 — Opus re-checks (read-only, effort high, AGENTS §9)

## Note 47 stage 4: re-check of test plan rev 2 (Opus, read-only, HEAD aee03ea)

**Verdict: GO WITH LISTED CHANGES.** Four majors (M1–M4) must land in the plan before part (a) starts. M1, M2 and M3 are in part (b); M4 is in part (a). The minors can be folded in as the implementation goes. Nothing was built or run. Every claim below was read in the tree at the cited line.

## Rulings: closed or not

| Ruling | State | Note |
|---|---|---|
| P1 | closed | §0 Q1, §5; S8 owns the descheduled host |
| P2 | closed | `Busy::stop` = min(e, s+in); RM-16 :167 cited correctly |
| P3 | closed | record unchanged, N47S4-05 dropped; rows 09a–c need concrete text (m2) |
| P4 | closed | units unstated (m5) |
| P5 | **not closed** | M1 (prepare's mark is wiped at arm), M2 (publication condition) |
| P6–P10 | closed | P7 also needs the burst's ActionId (m6); P8's inventory is short (m3); P9: `tx_two()` exists, VERIFIED `fake.rs:5389`; P10: N47S4-35 is not a concrete mutation (m2) |
| Sol 2 | closed | (a) and (b) bite; (c) bites only inside a 1–2-sample window (m9); rows (m2) |
| Sol 3 | **not closed** | M3 |
| Sol 4 | **not closed** | M2 |
| Sol 5 | **not closed** | M4; leftovers of O-3 (below) |
| Sol 6 | partly | 19 is anchored on the real check (`ezsdr-radio/src/lib.rs:437`, VERIFIED); 12 is equivalent in behaviour (m8); 11/12c/35 need concrete text |
| Sol 1, 7, 8 | closed | Sol 1 in §0 and §5 P-6; m3 adds lines to Sol 8's text inventory |

## The three open questions

**O-1: accept the scalar `Busy`, but rewrite its justification.** The plan's counterexample is wrong. The plan says the air is free at 5 while the rule refuses until 13 (VERIFIED, both Providers):
- At s=3 the window reaches 13. UHD hands a held burst once `k < window_cut` (`tx.rs:419, 485`).
- MockRadio counts a held burst as handed once its first send fits the window (`mock-radio/src/lib.rs:1376-1380`).
- RM-16 then closes at the handed cursor (`design/07:167`). So B's [10,13) may already be on air.

13 is therefore the tightest bound knowable at receipt; an exact answer needs the owner's progress (#79's mechanism). RM-21 should say "bounded by one in-flight window after a Stop", not "conservative".

**O-2: accept dropping `version`, but put the read in `pass`, not in `step`.**
- Today `pass` already takes `core.streams` twice per turn (`tx.rs:297` begin_read + plan_section, `:306` end_read). Reading the cut inside the first section costs no extra lock.
- Read `lines.line(key).and_then(|l| l.plan.first()).and_then(|s| s.cut)`, not `streams.planned(key)`. `planned` builds a `Vec` and clones each segment's settings (`core.rs:599-611`).
- I found no lock-order hazard. `step` is entered with no lock held: `pass` drops its guard before `reports` and `step`, and `dispatch_start` holds only `core.held` (`tx.rs:956-970`).
- Today `version` bumps on every `book` and `refuse` of any stream (`core.rs:534, 547, 661`), so a per-turn read also detects every change today's version check detects.
- A field the read refreshes each pass is fine. N47S4-37's killer must then drive `pass()`, not `step()` (§2.4 says `tx.step()`).

**O-3: the claim holds. No reachable sequence gives a transmit cut other than sample 0.** Deleting the cut arithmetic is approved, but on a different argument and with more deletions (below).

The only producer of a transmit cut is `end_at_origin`. It is called from `finish` (`timeline.rs:465-468`), and `cut()` there takes `item.e = origin`, which gives sample 0 (`:480-487`). Path by path:

| Path | What happens (VERIFIED) | Existing test |
|---|---|---|
| Loss | `core.rs:942` → `Streams::end` → `Kind::End` on transmit sets `ended` only (`timeline.rs:446`); a cut only if origin ≥ end | `rm_26_the_timeline_cases` (`ezsdr-radio/tests/timeline.rs` "a loss before a transmit clock's origin…"); `core.rs:2076-2080` |
| `Provider::stop` | `mod.rs:694`; same path | `core.rs:2063-2075` (S = O−4000, O, O+400) |
| Run's end | It is `Provider::stop`. The production `Streams::end` callers are only `mod.rs:694` and `core.rs:942` (grep `\.end(&core`) | as above |
| Stop | `control.rs:276-279` books `Kind::Stop`; the transmit side falls to `Kind::Stop \| Kind::Start => {}` (`timeline.rs:457`). It acts through `TxCmd::Stop` → `stop_now`, not a cut | `rm_26` "a transmit Stop ends no clock" |
| Clock end after the origin | `finish`'s condition is false: no cut and no registry end | `core.rs:2063` (O+400); `rm_26` "a loss ends a running transmit stream's bursts" |
| Abort | `Kind::End { .. }` on transmit ignores `abort` (`timeline.rs:442 vs 446`) | **none** for transmit (m10) |

Every non-zero transmit cut in the tree is set by a test (`tx.rs:3090, 3126, 3332, 3367, 3520, 4512, 4890, 4917, 5406`).

The plan's reason is false, though. §2.2 says "`now ≥ cut` at :421 covers the origin alone". It does not. With `cut_k = Some(0)`, the guards at `tx.rs:440, 445, 460, 463, 482, 499` are what stop the owner from starting a held burst, or continuing an open one, inside the window before a future origin.

They are unreachable in production for a different reason (VERIFIED code, INFERRED timing):
- The origin is `lattice(A)` at arm (`mod.rs:563-565`). `Provider::stop` reads S after `start` (`mod.rs:692`), so S ≤ O only within one lattice step of A.
- In that step no burst can be held: bursts are refused before start-up (`control.rs` "begins before the device's start-up") and the control thread polls every 1 ms. After a loss the owner sends nothing (`tx.rs:272, 402`).

`send`'s `clock_cut` is never `Some` even in rigs: with cut 0, `step` returns at :445 or :482 before it calls `send`.

Consequences of the deletion:
- **`refuse_first_send` (`tx.rs:761-768`) becomes dead and must go.** It reads only `clock_cut` (`tx.rs:532`). So must UR-22's "or, at most 32 samples long, a clock cut" (`design/18:250`).
- **`ur_23_a_cut_before_four_first_samples_is_refused` must be deleted, not retargeted.** The preemption path pads and sends: `ur_23_a_preemption_cut_leaving_n…`, `tx.rs:4896-4909`, sends n samples and closes. The retarget the plan INFERRED would fail.
- The §2.4 replacement test must not step inside the window before its origin.

## Clock-end semantics as stated

F-3 and §2.1 are true. Add:
- UHD ends a transmit registry clock only at `core.rs:668` (an End booked at S ≤ O, ending the clock at O). After the origin the registry never ends it.
- The owner's `end_clock` (`tx.rs:1071`) ends only its own clock.
- The Mock's twin of that rule is `tx_reconcile` (M4).

R1's phrase "the clock ends at the stream's end" is looser than the code; the plan should follow the code.

## Findings

**M1 (major, VERIFIED): prepare's `Some(0)` mark is erased at arm.**
- Prepare configures at `mod.rs:517`. Arm then opens the transmit streamer: `mod.rs:559` → `core.rs:1248` `record_opened` → `record_open` sets `configured = None` (`core.rs:627`).
- Sequence: connect at f1 → arm → start → burst A with no new value (generation 0). Wrong outcome: the mark is `None ≠ Some(0)`, so an `apply tx` comes before A's first `tx_send`. `ur_25_a_burst_whose_snapshot_equals_the_device_s_makes_no_apply` fails, or every Run pays a silent extra configure.
- Fix: publish `Some(0)` in arm after the transmit open and its rate check succeed, under the streams lock. State that the open keeps the radio settings (INFERRED, as F-28).

**M2 (major, VERIFIED): the configure's lifecycle under Stop, loss and `Provider::stop` is unspecified, and two expectations are wrong.**
- `Holder` has only `Free` and `Owner` (`core.rs:324-330`), and only the owner claims (`core.rs:417-424`). "The owner still holds its claim" is always true, so that half of the publication check (and of N47S4-12b) can kill nothing.
- `TxCmd::Stop` is read only at the top of the run loop (`tx.rs:265-271`).
- Sequence: a step configures B (paused at a probe) → control books a Stop and queues `TxCmd::Stop` → the configure resumes and publishes `Some(g)` → the same step dispatches B → `tx_send`.
- Wrong outcome: B's payload goes out after the Stop, so `ur_22_a_stop_during_the_configure_leaves_no_payload_after_it` fails, or the implementer invents a mechanism.
- The plan's "mark `None` (partial)" after a Stop is also wrong. Nothing aborts the configure, so it completes and the device holds g.
- Fix:
  - Make the configure a turn of its own. On success, publish `Some(g)` only if `!is_lost()`, under the lock, and return `true` with B still held. The next turn drains commands (Stop → `stop_now`, `CancelReason::Stop`) and re-decides B with `dispatch_start`, which R4 asks for anyway.
  - Loss is then covered by `tx.rs:272/402`. `Provider::stop`: UR-25 already allows a start the owner read before the end (`design/18:279`). Assert the mark, the claim's release and Shutdown's `ProviderStop` cancellation, not "no payload". Asserting "no payload" would need a `streams.stop` check at dispatch, which is out of scope.
  - Reuse the existing `apply` probe between channels (`core.rs:1012`) instead of adding new probe names.

**M3 (major, VERIFIED design): S7's one-shot fault cannot produce "refused again".**
- `arm_tx_apply_refusal(0)` fails only the next apply (§3.3). A' with the same snapshot then configures successfully and is sent, so the S7 test fails as written.
- Fix: a refusal that persists, keyed to a value on a channel (refuse every transmit apply with `freq == X` on c, until disarmed). That also models a read-back refusal. The partial case uses it on c1. N47S4-16 anchors it.

**M4 (major, VERIFIED): B4 deletes MockRadio's transmit clock end.**
- §2.2 describes `tx_reconcile` as reconciling the front. Its live job is different: `lib.rs:1410-1422` ends the registry transmit clock at the cut and writes the layout row's `ended_at` (`end_layout_segment`, `lib.rs:1072-1080`). It is called from the stop and loss path (`lib.rs:1189`).
- It is the Mock twin of `core.rs:666-672`, which R1 says to keep.
- The Mock's origin is `lattice(arm now)` (`lib.rs:2133`), so a stop or loss at the arm's virtual instant reaches it.
- No mutation row anchors it: grep `clock.ended` and `tx_reconcile` in `plan/maintenance/tools/mutations.json` finds none. I found no Mock test asserting a transmit `ended_at` at its origin. So the deletion would survive the gate.
- Sequence: MockRadio arm at t (on its lattice) → `Provider::stop` at t. Wrong outcome: today the transmit SampleClock record and the layout row both end at t. After the plan they are null, while UHD (P1398) still records t. Mock and UHD Manifests differ.
- Fix: keep the clock end as a one-clock function. Add a Mock test that mirrors `core.rs:2063` and a row that kills it.

**Minors**

- m1 (VERIFIED): O-2's placement (above): the read goes in `pass`, without `planned()`, and N47S4-37's killer calls `pass`.
- m2: Rows 09a/09b/09c, 11, 12c and 35 describe alternative designs, not text substitutions in the planned code. `apply_due` reads `chain.front()`, and `configure` always iterates every channel (`core.rs:1011`). Concrete alternatives:
  - `previous_end` → `None`, killed by §3.5 (a), (b) and (c);
  - `pad as i64 + 1` → `1` (`tx.rs:712`), killed by (b);
  - `close_start.saturating_add(3)` → `close_start` (`tx.rs:748`), killed by (c);
  - or name the existing rows on those lines with the new killers.
- m3 (VERIFIED): §6.3 misses the receive callers of `timeline::start_lead_ns`: `rx.rs:491, 776, 1277` and `rx/differential.rs:95`. It also misses `design/07-radio-model.md:445`. The Kernel's `binding::start_lead_ns` (`ezsdr-kernel/src/binding.rs:148`) is a different key (`ezsdr.time`) and stays.
- m4 (INFERRED risk): `ColdConfig` sets the rate (`control.rs:73`), and UHD's `apply` calls `uhd_usrp_set_tx_rate` whenever the rate is `Some` (`uhd.rs:895-899`). The bench test T5 covered frequency and gain only (`bench-results.md:293`). Either do the between-burst transmit configure with `rate: None`, keeping the read-back check, or add the rate to the next bench item.
- m5: `Busy`'s units are unstated. Use root ticks, with end = `clock.instant(start_k + len)` and `stop(at, ticks(in_flight_ns))`, the same on both Providers.
- m6 (VERIFIED): `Held` carries no ActionId (`control.rs:458`). R5's refusal "with the burst's id" needs it, through the snapshot as `ColdConfig::of(snapshot, id)` does.
- m7: Changing the apply log format (§3.3) touches about 29 `apply rx`/`apply tx` assertions (grep); observe the order in `calls()` and the manual clock's time instead. m8: N47S4-12 is equivalent in behaviour (generations never repeat, so a kept old mark still differs from g); its killer is the accessor alone; record that.
- m9: (c)'s guard window is only [e, e+2) samples (the close is `close_start+3`, `tx.rs:748`). The test must receive the value exactly at e and hold device time there, or it passes without the guard. m10: add a transmit abort case to `rm_26_the_timeline_cases` (one line).

Not checked: whether UHD applies the tx rate again without a glitch (m4); the Python and acceptance wiring; the Mock's B6 history code in detail.

## Note 47 stage 4 — Opus re-check of fix round 1 (2026-11-07)

Read-only. I built nothing and ran no tests. The one command I ran was `rules.py --base HEAD --renames renames.txt`, which reported 0 problems. Kill claims come from `probes-fix1/summary.txt`; I did not rerun them.

## Verdict: PASS. Sol's three findings are closed. No new major finding; four minor findings and two notes are below.

## Sol's findings

**F1 (P2, fractional burst start): closed. VERIFIED by code reading.**
- `channel::Timeline` now keys on `RootInstant` (`channel.rs:27-34`), and `open_held` keys the values at `RootInstant::of_sample(start)` (`lib.rs:971-977`).
  - The burst's first sample reads the new values: `from <= t` holds at equality.
  - The previous burst's last sample, at `t - q < t`, keeps the old values. This follows from the exact comparison, whatever tick the burst opens at.
- The opening tick decides only whether the value is pushed before any sample at or after `t` is read.
  - Rounds emit strictly before their tick, and `advance` emits every stream before it opens the held burst (`lib.rs:1880-1887`).
  - `floor(t) <= t` therefore suffices for the new value.
- D-48's second condition: the previous burst's last block must be emitted before `open_held` overwrites `open`.
  - The block goes out when `v_after(s-1) = floor(t-q)+1 <= floor(t)` (`lib.rs:1749-1751`).
  - That holds for every q ≥ 1.
- The limit (root ticks per sample < 1) is unreachable in production:
  - `prepare` accepts only the Simulation class (MR-7 step 2).
  - The only Simulation Authority is the Simulation Engine, at `VIRTUAL_TICK_RATE_HZ = 1e9` (`ezsdr-sim/src/lib.rs:18`, `ezsdr-sim-engine/src/lib.rs:135`).
  - `ideal` rates are at most 1e9 (`ezsdr-radio/src/channels.rs:376`), so q ≥ 1.
  - On `x3xx`, q = 5N, an integer, so a fractional instant occurs only on `ideal`.
  - The Mock's own 200 MHz-root unit rig (`lib.rs:2390`) runs at q = 200.
- History and stepping order: `open_held` is the only writer of the transmit timelines; bursts open in start order with strictly increasing floor ticks (q ≥ 1), so `rev().find` holds. The opening tick depends on the instant only; `ch_09_*` (both orders) stays green.
- Other readers of "open vs held" in the one-tick window `[floor(t), ceil(t))`:
  - `Busy` (`lib.rs:1728`) is unchanged; actions precede `advance` (`lib.rs:2356-2361`), so an action at `u = floor(t)` sees the burst held; `stop_tx` cancels open-without-record and held alike (`lib.rs:1185-1199`); `transmit_until_cut` works by sample index.
  - `tx_handed_front` (open counts after 1 sample, held after 32, `lib.rs:1409-1414`) decides nothing: the `x3xx` window is 10 ms ≫ 32 samples at the slowest rate (≈0.33 ms); `ideal` has no front. The one observable difference is m4.

**F2 (P3, fail → fix → resend through the Kernel): closed. VERIFIED.**
- `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` (`tests/fake.rs:1871-1914`) submits through a Session, and `admitted()` checks each submission. It asserts:
  - ActionId: `rejected == [(tx_burst, first_id), (update_parameter, early_id)]`; a value at ≈12 ms inside the 10–20 ms booking is refused; the corrected value at ≈21.5 ms is accepted (the list is exactly two);
  - the next burst is `admitted`, on the same clock (`clocks_after == clocks`, non-empty since `tx_at` unwraps the record); `apply tx 0 rate=- freq=3000000000` precedes an experiment `tx_send`; `sent_before == 0`. Manual clock (`common/mod.rs:447`).
- Widening FakeDevice: `FakeFault` was already public with `RxApplyFails`, and the new variant is inert unless a test lists it. Its misuse risk is minor m3.
- S7 and F9 are kept for what the Session test does not pin: S7 the mark (`None`, then `Some(2)`), the retry's second apply and the partial case; F9 the exact booked end (refused at 19 ms, taken at `instant(20_004)`).

**F3 (P3, UR-12 and UR-26): closed, with minor m1.**
- UR-26 (`design/18:294-295`) matches `apply_due` (`tx.rs:233`), `values_turn` and `configure_turn` (`tx.rs:507-559`) on:
  mark ≠ wanted; no device burst open; device time at or after `chain.front()`; `!needs_reopen`; a turn of its own, then `dispatch_start`; the mark published only if not lost (`None` on error); the refusal carries `held.settings.action()`, no halt.
- RM-21 (`design/07:232`) already keeps a refused burst counted to its booked end. The `apply_due` reference is gone; the Changes row (`design/18:738`) is present.

## Mutation rows

| Row | Anchor | Killer and assessment |
|---|---|---|
| N47S4-54 | `v_floor` → `v_of` at `lib.rs:1875`, unique | The fractional test: a ceil-tick open lets sample 1999 be read before the push. KILLED. Right. |
| N47S4-55 | Key rounded through `v_of` | Same killer. KILLED. Right. |
| N47S4-56 | `device_failed(held.settings.action(), …)` → `None` | Session test. KILLED. Distinct from -13b (Busy) and -14 (loss), which share the anchor with other `new` texts. |
| N47S4-16 | `Dir::Tx` → `Rx` in `device.rs:1351` | S7. KILLED. The Session test would also kill it. |
| N47S4-18 | `set_at`'s push → clear + push | The unit test has two changes (100, 200), so clearing loses the change at 100. KILLED. |
| N47S4-46 | Frequency `set_at` removed | `mr_33_…_next_burst_s_start`. KILLED. |
| M24 | `v_after` body → `v_of` | `mutate.py:97` runs the filter as a substring, so `mr_14` runs in the gate; the implementer's `--exact` probe was the only problem. KILLED. |
| M30, M54 | Comparison → `true`, and `.rev()` dropped | The unit test catches both. KILLED. |

No row's behaviour lost its killer: every retargeted row mutates the same decision as before.

## New findings (all minor)

- **m1 (minor, VERIFIED).** UR-12 (`design/18:210`) says a transmit burst's configure sends "frequency, gain and antenna without a rate", and that "for the full configuration the read-back rate must be exactly the claimed one".
  - `configure_tx` passes the clock's rate as the claim (`control.rs:98`). `Core::configure` checks `applied.rate != rate` and the decimation on every configure, the burst's included (`core.rs:1049, 1062`), and writes a `sample_rate_hz` applied row (`core.rs:1036`).
  - With `rate: None` the device keeps its rate, so the refusal is INFERRED unreachable.
  - Fix: say the burst's configure reads the rate back and checks it like the others.
- **m2 (minor, VERIFIED).** The S7 comment at `tx.rs:5753-5754` still says "until it is disarmed". There is no disarm any more; the retry uses F3.
- **m3 (minor, VERIFIED).** The Session test leaves two gaps:
  - It never asserts that the refused value reached the device: there is no assertion that `apply tx 0 rate=- freq=2000000000` comes before the rejection.
  - It does not check the rejection's reason.

  The ActionId match and N47S4-56 carry it. `TxApplyRefused` matches `f64` exactly, so a test whose frequency is coerced would never fire the fault. Optional.
- **m4 (minor, INFERRED).** Two steps at the same `u = floor(t)`, for example a drain round:
  1. The first opens a burst held at fractional `t`.
  2. In the second, a burst booked at the same start is refused with MR-16 "burst begins at or before the open burst's next sample".
  3. Before this fix the reason was "a held burst already has this start".

  The id and the refusal are unchanged; only the reason text differs.
- **Note.** The held-burst wake moved from `v_of` to `v_floor` (`lib.rs:860`) with no row. Within one Mock this is INFERRED equivalent, because `advance(u)` takes a candidate at `floor(t) <= u` either way.
- **Note.** The shortcut at `lib.rs:1874` could instead be a `prepare` refusal of a ratio below 1, since MR-7 step 5 accepts any integer root. It is unreachable today (F1), so no change is needed.
