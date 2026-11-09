# O1 test plan (second version, after Sol's review): a spec oracle for transmit, passing on today's code

Status: plan only, no code, nothing run (the tree is under a gate). Base: `25b9b15` (N0 landed). Facts are VERIFIED (read in the file named) or INFERRED (settled by the first run; the implementation phase reports each it settles). Section 11 maps every Sol finding to its change.

## 0. Decisions in one paragraph

One std-only oracle, `crates/ezsdr-acceptance/tests/oracle/mod.rs`, written from the spec text and naming no Kernel, Radio, Mock or UHD type. Three drivers, in three sub-steps (section 9): **O1a** the UHD frozen single-threaded rig (`tx.rs` `switch_rig` family) with the exact-timing tables; **O1b** the Mock's own stepped harness in `crates/ezsdr-mock-radio/tests/mock_channel.rs`, observing samples through the existing SimulationChannel loopback; **O1c** the paired parity rig, restricted to assertions that hold under any thread scheduling. Every exact-timing scenario (receipt instant, admission, switch boundary, dispatch lateness, placement) runs on a frozen rig; no verdict depends on when a test thread runs (design note 37). Cases are compact tables, not one scenario each. Sequence 6 is split, sequence 7 models three separate quantities, today's applied-switch/Stop rule is pinned and marked for C2, #84 is an expected-divergence assertion beside an independent `Cancelled`.

## 1. Where the oracle lives, and its interface

### 1.1 Placement (Sol answers 1, 3: yes, `#[path]`)

- Oracle: `crates/ezsdr-acceptance/tests/oracle/mod.rs` (directory module, not a test target, as `tests/support/mod.rs`), `#![allow(dead_code)]`, `std` only. It cannot name `ezsdr_radio::staging`, so "does not call it" is structural.
- O1a: `#[cfg(test)] #[path = "../../../ezsdr-acceptance/tests/oracle/mod.rs"] mod oracle;` at the top of `crates/ezsdr-radio-uhd/src/provider/tx.rs` (a non-inline `mod` with `#[path]` in a non-mod-rs file is relative to `provider/`; INFERRED, settled by compile); tests in `tx.rs`'s `mod tests` beside `ur_21_s8_*`. The only `crates/*/src` edit, `#[cfg(test)]`-gated; the tests assert on the Provider's observable output (`FakeDevice::transmitted`, `calls()`, `switches` rows, `rejected`, events), never on private state.
- O1b: `#[path = "../../ezsdr-acceptance/tests/oracle/mod.rs"] mod oracle;` in `crates/ezsdr-mock-radio/tests/mock_channel.rs` (the file already includes `../../ezsdr-acceptance/src/layout.rs` the same way, VERIFIED line 24).
- O1c: `mod oracle;` in `crates/ezsdr-acceptance/tests/parity.rs` (PO-8), tests appended in a closing section.
- O1a also carries the text-only amendment of section 10 D (`design/07-radio-model.md` RM-16 and, if it states the same, `design/09-mock-radio.md` MR-25), with a Changes row, `rules.py` green.
- Oracle self-tests: a separate target `crates/ezsdr-acceptance/tests/o1_oracle.rs` (`mod oracle;`, pure hand-computed numbers from the spec text), so they run once, not once per driver.

### 1.2 Interface

```rust
pub struct Clock { pub origin: i64, pub ticks_per_sample: i64 }           // root ticks; both rigs have an integer ratio (UHD 200, Mock 1000 at 1 MS/s)
pub struct Params { pub clock: Clock, pub window: i64, pub lead: i64, pub allowance: i64 /*0 on the Mock*/, pub w0: bool /*window == 0*/ }  // ticks; built from the rig's profile values, never from Provider output
pub fn ramp(tag: u8, len: usize) -> Vec<[f32; 2]>                         // [(i+1)/8192, tag/64]: exact in f32, distinct per index and per waveform, never zero
pub enum Late { Drop, SendAsap }
pub struct Book { id, tag, len, repeat, at: Option<i64> /*sample*/, late, switch: bool, recv: i64 /*root tick*/, answer: Option<Answer> }
pub enum Answer { Accepted, Refused, Moved(i64) }
pub enum Ev { Book(Book), Stop { recv: i64 }, Fail { id, attempt: (i64 /*a*/, i64 /*n*/), accepted: i64 } /*UHD: FailSend*/ }
pub fn admit(p: &Params, evs: &[Ev], b: &Book) -> Verdict               // exact admission where the spec fixes it (1.3)
pub fn place(p: &Params, evs: &[Ev], b: &Book) -> i64                    // RM-14/RM-15 start of a late SendAsap or untimed booking
pub fn fronts(p: &Params, evs: &[Ev]) -> Vec<Footprint>                  // RM-15 regions
pub fn switch_boundary(p: &Params, origin: i64, period: i64, recv: i64, at: Option<i64>) -> i64   // RM-37
pub fn stop_bound(p: &Params, t: i64) -> (i64 /*t_k*/, i64 /*B*/)        // RM-16
pub fn mock_cut(p: &Params, evs: &[Ev], stop: usize) -> i64              // MR-25, full formula (2)
pub fn expect(p: &Params, evs: &[Ev]) -> Expect                          // slots (open ranges), burst records, switch outcomes
impl Slots { pub fn check(&self, observed: &dyn Fn(i64) -> Option<[f32; 2]>, from: i64, to: i64) -> Result<(), String> }
```

`check` identifies each observed sample by lookup against `ramp` (tag and index): a one-sample shift, wrong waveform or wrong repeat phase is named in the message. Silence is `None` or `[0,0]` (pad and EOB zeros, UR-22; their count is not stated).

### 1.3 Takes as input vs decides (Sol answer 4: assert admission where exact)

- Input: the receipt answer where the spec does not fix it (a late booking's move is the Provider's observed `Moved(to)`; the oracle then **computes** `place(...)` independently and requires `to == place`, not merely "later", so a placement two samples too far fails; same for untimed: first free sample from the oracle's own regions).
- Asserted exactly (answer produced by the oracle, compared with the Provider's): booking at a front accepted and at front-1 refused (RM-15); a join at J(e) accepted and one root tick after refused; a booking inside a finite abandoned region refused (KL-39); a `DropAndFlag` booking inside a Stop region refused, `SendAsap` moved to its end, untimed placed at it (RM-16).
- Time is root ticks: receipts are frozen ticks (`device_at(core, .., ticks)` on UHD, `step(world, .., tick)` on the Mock), so equality cases (`J(e)`, `J(e)+1 tick`, `bound` exactly on a grid point) are expressible without a thread.

### 1.4 Ranges and exact points

- UHD Stop (RM-16): cut `c` in `[t_k, B]`, `B = at_or_after(t + W) + 3`; samples before `c` correct, from `c` silent. `c` is read from the observation and range-checked; the record's `samples` and `end` are checked against the attempted/successful sends (7), not against `c` alone.
- Mock Stop (MR-25), the full formula: `cut = max(t_k, payload, previous_cut)`, where `window_end = at_or_after(t + W)`; a started (open) burst contributes `min(its exclusive end, window_end)`, a repeat `window_end`; a burst not yet started counts only if it starts before `window_end` **and its first send fits** (`start + first <= window_end`, `first = min(len, 32)` for a finite burst, 32 for a repeat); a burst received after the Stop does not count (receipt order); `previous_cut` is the cut of an earlier Stop on the same clock (never retracted). With `W = 0` (`ideal`) `window_end = t_k`, so `cut = t_k` (RM-15 "a zero window preserves the ordinary behavior"). MR-25's own text, `design/09-mock-radio.md:171`.
- Switch boundary (RM-37): exact at receipt, the first `origin + j*N >= max(at, at_or_after(recv + W + lead))` (a Mock never moves it, MR-42). A UHD move is computed from the **observed** owner cursor (read from the `calls()` trace, independent of the record): `to` = the first grid point at or after the cursor, exactly (UR-38, Sol minor 7).

## 2. The rules the oracle encodes (rule id, sentence)

| Oracle rule | Spec sentence | Used by |
|---|---|---|
| sample `n` of a burst carries waveform index `n`; a repeat `(k - origin) mod N`; after an applied switch the new waveform's own length is the period from `b` | RM-13; RM-15 "ends at the end of its waveform"; RM-37 | tables T1-T4 |
| a burst ends at its waveform end, at the next burst's start, at a Stop | RM-15 first paragraph | T1, T2, T6 |
| region/front: a standalone closed device burst of 1-32 transmitted samples (1-3 padded to 4) fronts at padded end + 10; a head > 32 samples or a joined piece at end + 2; a repeat holds until its Stop | RM-15 | T1 (lengths 1, 3, 4, 31, 32, 33) |
| join at the exact end of a chain > 32 accepted when received no later than `start - (lead + allowance)`; later refused; equality is on time | RM-15; RM-14 "equality is on time" | T2 |
| lateness before footprint; `SendAsap` moves to the first sample at/after decision instant + lead that no accepted region holds; untimed takes the first free sample, no `TIME_ERROR` | RM-14, RM-15 | T5 |
| Stop closes at the first sample at/after t; UHD cursor never later than `B`; cancels held bookings not handed; a cancelled booking that starts in `[t, B)` leaves region `[t, B)`; a cancelled booking starting at or after `B` leaves none; later burst as usual | RM-16 (two paragraphs); `Bookings::halt` is not read, the sentence is the source | T6a-T6d |
| switch: refused with no repeat playing or one pending; `b` on the grid; pending until reached; Stop with `B <= b` cancels; Stop with `b < B` leaves it applied (RM-37 text; C2 will change it); an abandon before handing `b` cancels; a finite abandon leaves a pending switch of an unstarted repeat pending; a repeat dropped at dispatch cancels it | RM-37 | T3, T4, T7, T8 |
| owner moves a switch whose cursor passed `b` to the first grid point at/after the cursor, `TIME_ERROR{send_asap}` | UR-38; RM-37 | T4b |
| Kernel record: `target` applied start, `requested_target` iff moved, `samples` = payload of completed sends (a withheld tail counted with its buffer, a failed send not counted, note 49 §5.2.2), `wraps = samples / len`, `late_by` iff the policy moved it | RM-15 last sentence; UR-22; MR-15 | all |
| abandoned finite booking keeps its region; an abandoned repeat keeps it to cursor + 3 where cursor is the **attempted** end | RM-15; UR-22 | T7 |

Not modelled: UR-22 send framing (32/4/2), `window_cut`, `Bookings`, `Held`, configure/reopen turns.

## 3. Observation points and limits

| Provider | Samples | Records |
|---|---|---|
| UHD on FakeDevice | `FakeDevice::transmitted(from, n)` (channel 0, `device.rs:1021`); `calls()` for send ranges (`tx_send <stream> n=.. at=.. sob=.. eob=..`) | `bursts`, `switches`, `rejected`, `timing` (`tx_stop_applied`: instant and bound), events |
| Mock (O1b) | **The SimulationChannel loopback**: `Radio::received(0)` in `mock_channel.rs`, a coupled rx stream that hears the transmitter (`design/09-mock-radio.md` MR-32; `channel.rs:82-109`). On the `ideal` profile with `gain_db 0` and no noise, `received` equals the transmitted sample exactly (existing `mr_32_a_loopback_receives_what_it_transmits`, `mock_channel.rs:346`); on `x3xx` (window 10 ms, delay 45 samples, LO phase from `sim.seed`) the oracle's expected sample is rotated by the existing `phases(seed, "mock")` helper and shifted by the profile's `path_delay_samples`, compared to 1e-6 as `mr_32_the_transmit_path_delay_shifts_a_loopback` (`:911`) does | `bursts`, `switches`, `rejected`, `stats`, events |

No production accessor, no new fault. Limits: UHD `transmitted` is channel 0 only (all O1 streams are one channel; multi-channel is not covered). The Mock's loopback shows the radiated plan (position, phase), not the Mock's block headers; headers (block lengths) are covered through the burst record counts. Noise is off for the O1 tables (a seeded noise case is not an oracle question).

## 4. Determinism (Sol 1)

- **Frozen rigs** carry every exact-timing case: UHD `switch_rig` (`Frozen::of(&device)`, `device_at`, `drain_tx`, `Tx::step`, `book_with`; no thread, VERIFIED `tx.rs:6695-6750, 6204-6214`) and the Mock's `step(world, radios, tick)` (deterministic, VERIFIED `mock_channel.rs`). A receipt is "at the tick the test sets", never "when the test thread runs". One rig per table row (cheap), so a case does not inherit another's state.
- **O1c (parity) keeps only scheduling-invariant assertions**, with actual observations as oracle inputs: (i) waveform identity and phase relative to the burst's **observed** `target` (from the `bursts` record): `R[(k - target) mod N]`; (ii) sample-to-record correspondence: the samples the device played equal the record's `samples` (held-tail convention, 2); (iii) a Stop's cut lies in `[t_k, B]` where `t_k` and `B` are read from the `tx_stop_applied` timing row (`control.rs:317,757`) and, for the Mock, from its own record; (iv) a switch's new waveform begins exactly at the recorded `boundary` and that boundary lies on the repeat's grid (origin from the record). No receipt-instant, admission, placement or lateness assertion is made there. The 20-repeat justification is dropped.

## 5. Tables and tests

Waveforms are `ramp(tag, len)`; one rig per case. UHD time: origin 2000, 1 sample = 1 us (`switch_rig` rate 1e6; 1 ms = 1000 samples, VERIFIED `tx.rs:6724, 6773`); the send chunk is `window/4` = 2500 samples (VERIFIED `tx.rs` `send`).

### T1 short and long heads, fronts (seq 1) - O1a `o1_uhd_t1_*`, O1b `o1_mock_t1_*`

Table over `L in {1, 3, 4, 31, 32, 33}` (finite A of length L at `s`, on time) plus `L = 3000`. Oracle: payload `L`; front = `s + max(L,4) + 10` for `L <= 32`, `s + L + 2` for `L > 32`; the pad/EOB is not asserted. Per row, two runs: a booking B at `front` (accepted, plays exactly from `front`, its samples exact) and at `front - 1` (refused, `Refused` matches the oracle). UHD: `Slots.check` over `[s-2, front+L_B)`; both: `bursts` records `(s, L)`, `(front, L_B)`, no `TIME_ERROR`. The `L = 3000` row spans a second send (offset 2500) and asserts, from `calls()`, that at least two payload sends exist and that one starts at a sample greater than `s` (**the nonzero-offset precondition for M1**). Fails on: padding counted as payload, a shifted waveform, 32/33 front mixed up, B admitted one sample inside the front.

### T2 join (seq 2) - O1a `o1_uhd_t2_*`, O1b `o1_mock_t2_*`

Table over receipt instant of B (start `s + 3000`, exactly A's end, A = ramp 3000): `J(e) - 1 tick`, `J(e)` (inclusive: accepted), `J(e) + 1 tick` (refused), `J(e) = start - (lead + allowance)` per rig (UHD 5 ms, Mock 2 ms). Accepted rows: samples continuous (A index 2999 at `e-1`, B index 0 at `e`, no silent sample), **two** Kernel records `(s, 3000)` and `(e, L_B)` with the held tail counted once (note 49 §5.4.2: the record counts the tail with A's last buffer). Fails on: strict `<` for the deadline, a gap/overlap at the join, a merged record, tail sounded twice or lost.

### T3 repeat wraps and Stop (seq 3) - O1a `o1_uhd_t3_*`, O1b `o1_mock_t3_*`

R = ramp(7, 1000) at `s`, run to 2.5 wraps, Stop at frozen `t`. UHD: samples `R[(k-s) mod 1000]` to the cut `c` in `[t_k, B]`, silence from `c`; record `samples`/`wraps`/`end` consistent with the observed sends. Repeats shorter than `TX_SEND_CAP` = 20 000 samples are tiled (`tile_repeat`, `tx.rs:52-58`), so the 1000-sample repeat is sent as 2500-sample chunks that run across wraps, not as one send per wrap. The case asserts from `calls()` (`at=` instants) that some payload send starts at a sample `>= s + 1000` and not on a multiple of 1000 from `s` (a send beginning after a wrap, mid-period: **the precondition for M2**, whose phase error shows at the send's first sample). **T3b (M5's case)**: sends are 2500-sample chunks starting at `s`, so send boundaries are `s + 2500*i`. The oracle picks the Stop tick `t` whose `window_cut = at_or_after(t + W)` is `s + 5000 - 4`, with the owner stepped at that tick and the Stop after it, and with the prior chunk ending at `s + 2500`: correct code cannot hand the chunk `[s+2500, s+5000)` (`avail = window_cut - next = 2496 < wanted = 2500`), the +4 mutant hands it, so the cut is `s + 5000 = B + 1` (`B = window_cut + 3 = s + 4999`); the test asserts `c <= B`. The alignment is asserted by the test itself from `calls()` (the last handed chunk ends at `s + 2500` in the correct run). Mock: the loopback shows `R[(k-s) mod 1000]` to `mock_cut` exactly, record `samples = cut - s`. Mock cut table (T3m): see T9.

### T4 switch (seq 4) - O1a, O1b, O1c(partly)

- **T4a threshold table** (`o1_uhd_t4a_*`, `o1_mock_t4a_*`): R = ramp(8, 700) (a period that divides neither the window nor the lead, origin `s`), switch to ramp(9, 400); receipt ticks chosen so `bound = at_or_after(recv + W + lead)` is `g_j - 1`, `g_j`, `g_j + 1` for a grid point `g_j = s + 700*j`, plus one row with an explicit `at` between grid points. Oracle `b = switch_boundary(...)` independent of the observed boundary. Observed: UHD samples old `R[(k-s) mod 700]` below `b`, `ramp(9)[(k-b) mod 400]` from `b` across 2.5 new periods; Mock the same through the loopback; both: `switches == [(id, applied, b)]`, `bursts == [(s, b - s), (b, ..)]`, no `TIME_ERROR`. Fails on: a threshold rounded down, a boundary one grid point off, new period not its own length, new waveform from index 1.
- **T4b moved switch** (UHD only, `o1_uhd_t4b_*`): `book_switch` then step the owner **without `drain_tx`** until its cursor passes `b` (no state poked; INFERRED to be possible: `Tx::step` hands without draining, the same mechanism `ur_21_s4_a_switch_the_host_dispatched_late` uses with `device_at`), then `drain_tx`. The oracle reads the cursor from the `calls()` trace (the last handed repeat sample + 1), computes `to = first grid point >= cursor`, and requires `moved_to == to`, `TIME_ERROR{send_asap}`, old samples to `to - 1`, new from `to`. E2a deletes this path (note 49 §5.4.5); the test's comment names E2a.
- **T4c Stop before the boundary** (O1a, O1b): `B <= b` (oracle checks, else builds nothing): switch `cancelled`, no `ramp(9)` sample anywhere, one record.
- **T4d / T4e pin today's applied rule** (Sol 6, O1a and O1b): both have `b < B` and, on the UHD, **the owner has drained the switch before the Stop** (the frozen turn that books the switch runs `drain_tx`; if the Stop reached `halt` first, today's UHD answers `cancelled`, note 49 §5.4.5, an ordering difference that only the frozen rig fixes, listed in section 10), the oracle's expected row is `applied` (RM-37 text: only `B <= b` cancels). Each carries `// C2 (note 49 §11.10) changes this outcome; update this expectation there`.
  - T4d: the Stop's window edge leaves the new waveform's first piece unhanded (UHD cursor `<= b`, achieved with `window_cut = b + 600` and a 1000-sample new waveform, `wanted = 1000` > `avail`): row `applied`, **no** `ramp(9)` sample on the UHD; the Mock radiates new samples `[b, mock_cut)` (the Mock's cut is `window_end >= b`), checked through the loopback. The two Providers' emission differs and each is the oracle's own prediction (UHD range, Mock point); the row is the same.
  - T4e: the Stop leaves the cursor past `b + 1000` (the new waveform has sounded): row `applied`, `ramp(9)` samples `[b, c)` on the UHD, to `mock_cut` on the Mock. Separate from T4d: emission and row are asserted by separate assertions (a premature C2 `cancelled` fails T4d's row assertion, not its sample assertion).

### T5 late `SendAsap` and untimed (seq 5) - O1a `o1_uhd_t5_*`, O1b `o1_mock_t5_*`

Frozen receipts. A = ramp(10, 3000) at `s` (front `s + 3002`). Rows: (a) late `SendAsap` B at `at` inside A's region, received so that `at < decision + lead` (late): `place()` = `max(ordinary, A.front)`; the Provider's `Moved(to)` must equal it exactly, `requested_target = at`, `late_by` set, one `TIME_ERROR{send_asap}`; (b) the same with `ordinary > A.front` (receipt late enough that the lead dominates): `place = ordinary`; (c) `DropAndFlag` in the same place: refused (RM-14 "not late and meets a footprint" is false for a late one: it drops, `TIME_ERROR{drop}`; the oracle returns `Drop` and the test asserts the drop notes, not a refusal); (d) untimed C received while A plays: `place = A.front` (`ordinary < A.front`): no `TIME_ERROR`, `late_by` null; (e) untimed with `ordinary > A.front` (A ended): `place = ordinary`. Fails on: placement off by one in either direction, B inside A, untimed given `late_by`.

### T6 Stop cancellation, split (seq 6) - O1a `o1_uhd_t6*`, O1b `o1_mock_t6*`

Cancellations are `rejected` rows (kind `tx_burst`, "cancelled by Stop"), not `COMMAND_REJECTED` events: `ezsdr_radio::cancellation_reports(Stop)` is false (VERIFIED `ezsdr-radio/src/lib.rs:124`, `tx.rs:1143-1148`). Lengths explicit; footprints non-overlapping.
- **T6a** cancellation of an unhanded booking, **no Stop region** (a pin of the amended RM-16 sentence "a `Stop` that cancels a booking starting before `B` leaves the region `[t, B)`", section 10 D): A = ramp 100 at `s`, finished and past its front long before `t`; H = ramp 300 booked far ahead (start `t + 30 ms`, at or after `B`); `Stop` at `t`. H: no sample, one cancellation row; A untouched (no Stop in effect on it: assert its record `end` is waveform end, not stop); D (`DropAndFlag`, ramp 200) received after the Stop with start `t + 6 ms` (on time, before `B = t + 10 ms + 3`): **accepted** (no region), plays from its start.
- **T6b** a cancellation that **does** make `[t, B)`: same A; H2 = ramp 300 booked before the Stop with start `window_end - 16` (`window_end = at_or_after(t + W)`), i.e. in `(window_end - 32, B)`: its first 32 samples do not fit inside the window, so MR-25 does not count it handed (Mock `lib.rs:1402-1411`) and the UHD owner cannot have handed it, provided **the owner's last step is at or before `t`** (the frozen rig takes no turn between the receipt of H2 and the Stop beyond `t`); it starts in `[t, B)`, so it is cancelled and creates `[t, B)`. H2 cancelled (row) on both Providers. Then in fresh rigs: D1 `DropAndFlag` start `t + 9 ms` refused; D2 `SendAsap` start `t + 9 ms` moved to exactly `B` with `TIME_ERROR{send_asap}` and `requested_target`; D3 untimed placed at `B`; D4 `DropAndFlag` at `B` accepted on time. Every row's answer is the oracle's (1.3). A is not asked to end by Stop.
- **T6c** A still playing: A = ramp 12 000 at `s` (longer than `W` = 10 000, so the Stop really cuts it; a shorter A is fully handed and today's UHD record would count A's EOB past the cut), Stop at `t = s + 1 ms`: cut as T3 (UHD range, Mock exact), record `end = stop`, later booking E (ramp 200) received after the Stop with start `>= B` plays on time and exactly; the held H3 booked with start `>= B + 20` is cancelled.
- **T6d** (retained-map gap, small): `A -> Stop -> B` where B is booked after the Stop and A's close send then fails (`FailSend` on the close, existing fault): B's reservation and start survive. Included if it stays under budget; otherwise reported as an uncovered case in section 7.

### T7 abandon (seq 7), UHD only - O1a `o1_uhd_t7*`

`switch_rig_with(waves, vec![FakeFault::FailSend(n)])`: the n-th payload-carrying `tx_send` fails and the fake takes half (`device.rs:1794-1820`, VERIFIED). Three quantities, modelled separately; `Fail { attempt: (a, n_att), accepted: n_att/2 }`. The fake halves the count before it logs (`device.rs:1820` before `:1840`), so `calls()` shows the **accepted half**, not the attempt: the failing entry is the one whose `n=` is half the attempt, `n_att = 2 * logged n` and every failing attempt is chosen **of even length** (so the half is exact); `a` is its `at=`. `accepted = n_att / 2` is the fake's documented behaviour, not read from the Provider. `FailSend(k)` counts every `tx_send` with `n >= 1` (`device.rs:1797-1801`), a held-tail send and a close send included, so `k` is computed per case from the planned send sequence (e.g. a finite burst's first send, its tail, its close are three counted sends) and the test asserts from `calls()` that the failing entry is the intended one:
- *emitted samples*: payload `[s, a + accepted)` sounds, silence from there (UHD slots).
- *abandon cursor* `c = a + n_att` (the attempted end, production's conservative cursor, `tx.rs:735-742,983-999`): a repeat keeps its region to `c + 3` (booking at `c + 3` accepted, `c + 2` refused); the pending switch with `b >= c` is cancelled, no new-waveform sample ever.
- *recorded payload*: derived from `calls()`: the sum of `n` over the sends the device completed before the failing one, a withheld tail counted with its buffer (note 49 §5.4.2), the failed send not counted.
Rows: **T7a** a repeat, failing send ends before `b` (switch `cancelled`); **T7a'** the failing send attempted range ends exactly at `b` (`c = b`, still `cancelled`); **T7c** the send *carrying* `b` (new waveform's first send) fails: today's row is `applied`, pinned with `// C2 (note 49 §5.4.5, 'uncertain') changes this`; **T7b** a finite F (ramp 3000, failing send is its second buffer, so recorded `2500`) keeps its whole region (a booking at `F.start + 2900` refused, at F's front accepted; KL-39). The "finite abandon leaves the switch of a repeat it has not started pending" half stays with `ur_21_s4_a_finite_abandon_keeps_the_switch_of_the_repeat_it_has_not_started` (`tx.rs:7022`), not duplicated here.
Fails on: an abandon cursor taken from the fake's accepted prefix, a record that counts the failed send, a switch not cancelled, a region shrunk to what sounded.

### T8 #84, UHD only - O1a `o1_uhd_t8_*`

Recipe of `ur_21_s4_a_switch_whose_repeat_the_host_dispatched_late_is_refused_without_a_cursor` (`tx.rs:6967`). The oracle independently returns `Outcome::Cancelled` (RM-37 "one that drops the repeat at dispatch"), asserted with its own `assert_eq!(oracle_outcome, Outcome::Cancelled)`. Then:

```rust
// KNOWN DIVERGENCE #84: today's code settles refused_past_boundary. E2a deletes only this block.
assert_eq!(today, "refused_past_boundary", "#84 fixed? delete this block and assert today == spec");
assert_ne!(today, spec);
```

Normal assertions stay: no `ramp(NEW)` sample, one `switches` row, the repeat never played. Not `#[ignore]` (an ignored test stops protecting the rest of the case).

### T9 Mock Stop cut table (MR-25) - O1b `o1_mock_t9_*`

Frozen `step`s, loopback samples and record `samples = cut - start`, `cut = mock_cut(...)`: (a) open repeat: `window_end`; (b) repeat not started, `start = window_end - 32` (first send fits) -> `window_end`, `start = window_end - 31` -> not handed, cut `t_k`; (c) finite len 20 not started at `window_end - 20` (fits, contributes `window_end`), at `window_end - 19` (does not fit); (d) a burst received after the Stop does not count; (e) previous same-clock cut: a Stop whose instant is before the first Stop's cut (the second Stop's own cut computes lower) leaves the retained cut where it was; (f) `ideal` profile (`W = 0`): Stop at `t`, A(100) at `t - 10`: cut = `t_k`, the loopback shows exactly `t_k - start` samples (MR-25's "Stop sample as a lower bound"). Fails on: counting an unfit burst as handed, retracting an earlier cut, a W = 0 cut anywhere but `t_k`.

### O1c parity (paired rig), scheduling-invariant only

`CancellationPair::new_with(.., extra ramps)` (the existing `new` delegates; same behaviour). Tests `o1_parity_*`: a repeat observed across wraps (identity and phase from the observed `target`); a long finite burst's sample-to-record correspondence on UHD, record on both; a Stop with the cut in `[t_k, B]` from the `tx_stop_applied` row, Mock record against its own cut; a switch whose new waveform starts at the recorded `boundary` on the observed grid. **No cross-Provider switch-row clause**: in the paired rig a Stop with `b < B` can reach `halt` before uhd-tx has drained the switch (UHD `installed` false: `cancelled`) while the Mock installs at receipt (`applied`, note 49 §5.4.5), so the row depends on scheduling; T4c-T4e on the frozen rigs own the rows. Nothing about receipt-time admission.

## 6. Mutation rows (final text; for `remote_mutate.py`, not run by me)

Anchors are Rust token sequences (AGENTS §7); each `old` below was grepped once in the tree and matches once (VERIFIED by `grep -n`, line numbers given; `anchor.py` and `rules.py` settle it formally). `crate`/`test_bin`: UHD `ezsdr-radio-uhd`/`lib`; Mock `ezsdr-mock-radio`/`mock_channel`; parity `ezsdr-acceptance`/`parity`.

| Row | file:line | old | new | filter | crate, test_bin |
|---|---|---|---|---|---|
| O1-M1 payload shifted one sample in every send after the first | `tx.rs:645` | `*slice = &channel[offset as usize..(offset + count) as usize];` | `{ let o = if offset > 0 { offset - 1 } else { offset }; *slice = &channel[o as usize..(o + count) as usize]; }` | `o1_uhd_t1_` (the L = 3000 row, which asserts a nonzero-offset send exists) | `ezsdr-radio-uhd`, `lib` |
| O1-M2 repeat phase drifts one sample per wrap | `tx.rs:626` | `let offset = if open.held.repeat { (open.next - start).rem_euclid(len) } else { open.next - start };` | `let offset = if open.held.repeat { (open.next - start - (open.next - start) / len).rem_euclid(len) } else { open.next - start };` | `o1_uhd_t3_` (asserts a send starting after the first wrap) | `ezsdr-radio-uhd`, `lib` |
| O1-M3a UHD switch boundary one grid point late | `control.rs:546` | `start = ezsdr_radio::staging::repeat_boundary(playing.origin, playing.period, bound.max(start));` | `start = ezsdr_radio::staging::repeat_boundary(playing.origin, playing.period, bound.max(start) + playing.period);` | `o1_uhd_t4a_` | `ezsdr-radio-uhd`, `lib` |
| O1-M3b Mock switch boundary one grid point late | `ezsdr-mock-radio/src/lib.rs:1702` | `start = repeat_boundary(playing.origin, playing.period, bound.max(target));` | `start = repeat_boundary(playing.origin, playing.period, bound.max(target) + playing.period);` | `o1_mock_t4a_` | `ezsdr-mock-radio`, `mock_channel` |
| O1-M4 Mock Stop cut one sample late | `ezsdr-mock-radio/src/lib.rs:1178` | `let cut = timeline::later_bound(at_stop, merge_front(payload, previous));` | `let cut = timeline::later_bound(at_stop, merge_front(payload, previous)) + 1;` (`later_bound` returns `i64`, VERIFIED `timeline.rs:339`) | `o1_mock_t9_` | `ezsdr-mock-radio`, `mock_channel` |
| O1-M5 UHD hands past the Stop bound | `tx.rs:454` | `let window_cut = clock.at_or_after(now.saturating_add(window));` | `let window_cut = clock.at_or_after(now.saturating_add(window)) + 4;` | `o1_uhd_t3b_` (window edge four samples short of the send boundary `s + 5000`: the mutant hands the chunk ending at `B + 1`; the test asserts `c <= B`) | `ezsdr-radio-uhd`, `lib` |
| O1-M6 a held switch survives its repeat's abandon (replaces the dropped `cancel_switch_from` row) | `tx.rs:1004` | `self.cancel_held_switches(cursor);` | `let _ = cursor;` | `o1_uhd_t7a_` (the switch is taken and held at `b >= c`; the mutant leaves it in `held`, it dispatches at `b`, and `ramp(NEW)` sounds) | `ezsdr-radio-uhd`, `lib` |
| O1-M7a Mock radiates the wrong sample position | `ezsdr-mock-radio/src/channel.rs:100` | `let n = usize::try_from((j - segment.start) % segment.len).ok()?;` | `let n = usize::try_from((j - segment.start + 1) % segment.len).ok()?;` | `o1_mock_t1_` | `ezsdr-mock-radio`, `mock_channel` |
| O1-M7b Mock repeat phase drifts per wrap | `channel.rs:100` | same | `let n = usize::try_from((j - segment.start - (j - segment.start) / segment.len) % segment.len).ok()?;` | `o1_mock_t3_` | `ezsdr-mock-radio`, `mock_channel` |

Why the old M6 (`stream.busy.cancel_switch_from(cursor)` removed) is dropped: the next statement, `stream.stop_at(clock.instant(cursor), 0)`, calls `halt`, which cancels a pending switch under the same condition (`!(installed && boundary < stop)` with `stop = cursor` is `boundary >= cursor`, the condition `cancel_switch_from` tests), so the mutant is INFERRED equivalent (`staging.rs:380-405` vs `:456-463`). Whether M1 and M2 survive the existing tests is not claimed; the implementation phase runs the existing tests on each mutant and reports. Parity rows: none (O1c asserts no exact timing).

## 7. Retained-test map (Sol 8): what keeps the excluded areas protected through E1-E3

Names read from the tree (path::name), VERIFIED to exist; the implementation phase runs them to confirm they pass.

| Area | Existing tests | Gap |
|---|---|---|
| configure completion/refusal, then Stop or dispatch re-evaluation | `tx.rs`: `ur_22_a_stop_during_the_configure_leaves_no_payload_after_it`, `ur_22_a_provider_stop_during_the_configure_keeps_the_mark_and_cancels_the_burst`, `ur_21_a_refused_configure_refuses_the_burst_and_the_fixed_value_plays_on_the_same_clock`, `ur_21_a_partial_configure_refusal_sends_nothing_and_the_retry_configures_both`, `ur_25_a_slow_configure_makes_a_burst_late_by_its_policy`, `ur_23_a_burst_at_a_held_tail_with_a_new_generation_is_configured_before_it` | none |
| reopen before payload | `ur_22_underflow_close_failure_reopens_before_the_next_payload`, `ur_22_uncertain_reopen_waits_for_owner_claim_and_stops_on_stop_or_loss`, `ur_22_a_failed_eob_close_recovers_with_flush_before_the_next_timed_burst`, `ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts` | none |
| reverse target order | `tx.rs`: `ur_23_bursts_go_out_in_target_order`, `ur_21_s5_b_handed_before_an_earlier_a_is_refused_at_receipt`; `parity.rs`: `s5_parity_b_received_before_a_refuses_a`, `s15_parity_booking_rule_both_receipt_orders` | receipt-order of an untimed booking into a gap: `s15_parity_untimed_queue_behind_a_booking`, `s15_parity_untimed_booking_after_stop` (note: the latter's own header says it depends on thread timing; a frozen twin is T5(d)/T6b D3) |
| `A -> Stop -> B` ordering | `parity.rs`: `vh_8_tx_stop_allows_a_later_burst`; `staging.rs`: `rm_16_a_later_booking_after_a_stop_on_a_long_burst_is_accepted`; `mock_radio.rs`: `mr_17_a_target_after_a_stopped_burst_is_on_time` | "A's later failure must not erase B's reservation or switch state": none; smallest case T6d |
| held-tail Stop | `parity.rs`: `s1_parity_stop_during_padded_device_burst`; `tx.rs`: `ur_23_a_value_waits_for_a_stop_s_held_tail`, `ur_21_m1_held_tail_waits_for_a_join_received_before_its_txcmd_is_drained` | sample/tail accounting is in O1: T2, T3, T6c |
| terminal and per-stream cancellation | `parity.rs`: `vh_8_loss_cancels_held_bursts_without_rejection`, `vh_8_provider_stop_cancels_held_bursts`, `vh_8_tx_stream_stop_leaves_the_other_stream_held`, `vh_8_device_stop_cancels_held_bursts`; `tx.rs`: `ur_23_a_clock_end_cancels_every_held_burst` | none (per-stream settlement of the *switch* by a terminal stop is T4c on one stream only: gap noted) |

Sample-to-record correspondence and tail accounting are O1's (T1, T2, T3, T6c, T7).

## 8. Left unchecked by O1

Multi-channel streams; UR-22 send framing (32/4/2, FakeDevice's job); configure/reopen turns (map above); the `uncertain` outcome and the honest switch outcomes (C2); noise; the Mock's block headers except through record counts; a two-stream switch settlement by a terminal stop.

## 9. Size and the sub-step split (AGENTS §9, one local commit per sub-step, each leaves the tree green)

Estimated test code (INFERRED): oracle 350, oracle self-tests 100, O1a tables 600, O1b 450, O1c 200: about 1,700, beyond 1,500, so three sub-steps, each reviewed on its own diff by the cheap round the brief allows, Sol once at the end:
- **O1a**: oracle + self-tests + the UHD frozen driver (T1-T8, M1-M3a, M5, M6). Leaves green: production unchanged, UHD tests pass on today's code.
- **O1b**: Mock channel driver (T1-T5, T6, T9, M3b, M4, M7a, M7b). Needs only the oracle; independent of O1a's tests.
- **O1c**: `new_with` and the parity tests (scheduling-invariant). Depends on the oracle only.
Order a, b, c. If the oracle is found to need a change while writing b or c, the change returns to a's commit by the orchestrator's rule (one agent edits at a time).

## 10. Spec/code disagreements found by reading, beyond #84

**D (found, decided by the orchestrator): the Stop region.** RM-16 (`design/07-radio-model.md:170`, "A `Stop` that cancels a booking leaves the region `[t, B)`") and MR-25 (`design/09-mock-radio.md:171`, "A Stop that cancels a booking also refuses later bookings whose region meets `[t, B)`") state it unconditionally; `Bookings::halt` adds the region only for a cancelled chain that starts before `B` (`staging.rs:384`). Decision: the code is right (a booking starting at or after `B` was never handed, so there is no device state to wait for); the spec text follows the code. O1a amends both sentences to "a `Stop` that cancels a booking starting before `B` leaves ..." (text only, a Changes row each, `rules.py` green), and T6a pins the amended sentence.

**Known difference covered by the frozen rig: switch row under a Stop with `b < B`.** Today's UHD answers `cancelled` when the Stop reaches `halt` before uhd-tx drained the switch (installed false) and `applied` after; the Mock installs at receipt and answers `applied` (note 49 §5.4.5). The paired rig cannot fix the order, so T4d/T4e on the frozen rigs pin it (the owner drains the switch first) and O1c asserts no row.

Otherwise none. One correction to my first plan, mine not the code's: Stop cancellation is a `rejected` note, not a `COMMAND_REJECTED` event (`cancellation_reports(Stop) == false`). `FailSend` takes `n/2` yet production's abandon cursor advances by the full attempt: this is conservative by design (`tx.rs:735-742`), not a disagreement, and the oracle encodes it (T7). Reading is not a run.

## 11. Changes after Sol's review (finding -> change)

1. **Driver A not deterministic (major)**: exact-timing cases moved to frozen rigs (UHD `switch_rig` family, Mock `step`); O1c keeps only scheduling-invariant assertions fed by observed `target`, `boundary`, `tx_stop_applied`; the 20-repeat justification dropped (sections 4, 5 O1c).
2. **Seq 6 (major)**: split into T6a (cancellation with no region, D accepted), T6b (region `[t, B)` from a cancelled chain starting in it, D1/D2/D3/D4 outcomes), T6c (A still playing, later booking survives), T6d (A -> Stop -> B with a failing close); explicit lengths; cancellations are `rejected` notes; A is required to be cut only in T6c.
3. **Seq 7 (major)**: attempted range, fake-accepted prefix and recorded payload modelled separately from `calls()` and the fault; cursor = attempted end; held-tail accounting per note 49 §5.4.2 (T7).
4. **Mock samples (major)**: the SimulationChannel loopback in `mock_channel.rs` (ideal profile exact, x3xx with the existing `phases` helper and delay), no accessor; mutation rows M7a (position) and M7b (phase).
5. **Boundary discrimination (major)**: tables T1 (lengths 1, 3, 4, 31, 32, 33, 3000), T2 (`J(e)`, `J(e) ± 1 tick`), T4a (period 700, bound at `g-1`, `g`, `g+1`, explicit `at`), T9 (MR-25 full formula: first-send fit, previous cut, `W = 0`); the MR-25 formula in 1.4 corrected; run on frozen rigs against the Providers, no scheduler rebuilt.
6. **Applied-switch/Stop baseline (major)**: T4d (`b < B`, new payload not handed: row `applied`, no new sample) and T4e (new waveform sounded) pinned, row and emission asserted separately, comment names C2; Sol's answer 2 followed (no omission).
7. **Moved boundary (minor)**: T4b computes the first grid point at or after the observed cursor and requires equality; sequence 5 placement is `place()` equality (1.3).
8. **Retained-test map (minor)**: section 7.
- **#84**: the oracle independently returns `Cancelled` (asserted); E2a deletes only the divergence block (T8).
- **Mutation rows**: M1 and M2 assert the exercised-send precondition; M5 uses a window-edge case that exposes the extra payload (T3b); M6 replaced by a distinguishing row on `cancel_held_switches` and the equivalent mutant explained; M3b, M4, M5 and M7 given final `old`/`new` text, all anchored once by grep.
- **Sub-steps**: section 9 (O1a, O1b, O1c).

### Changes after the Opus re-check (base `25b9b15`)

- **A (finding 1)**: the O1c clause "switch row applied/cancelled consistent between the two Providers" removed; rows are owned by T4c-T4e on frozen rigs (section 5 O1c).
- **B (finding 2)**: T6b's H2 starts at `window_end - 16`, inside `(window_end - 32, B)`, so it is unhanded on both Providers and `[t, B)` is still created; the UHD owner's last step is at or before `t` (T6b).
- **C (finding 3)**: T7 uses `n_att = 2 * logged n`, even attempts, and counts `FailSend(k)` over every send with `n >= 1` (tail and close sends included), asserting the failing entry from `calls()` (T7).
- **D (unreported disagreement)**: added to section 10; O1a may amend RM-16 and MR-25 text (before-`B` wording) with a Changes row; T6a cites the amended sentence.
- **Minors**: T6c's A is 12 000 samples; T4d states the owner drains the switch before the Stop and section 10 lists the ordering difference; T3/M5 drop "the piece ending at the wrap" (repeats under 20 000 samples are tiled into 2500-sample chunks) and M5's case aligns `window_cut + 4` with the send boundary `s + 5000`; T7b drops the pending-switch half (`ur_21_s4_a_finite_abandon_keeps_the_switch_of_the_repeat_it_has_not_started`) and derives "recorded" from `calls()`; T9(e) reworded to a Stop whose instant is before the first Stop's cut.
