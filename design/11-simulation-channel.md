# Phase 3 spec 11 — The SimulationChannel (`sim` 1.1.0)

| Field | Value |
|---|---|
| Status | Accepted at Gate P (owner, 2026-09-26) and ratified at Gate X (owner, 2026-09-26; [`plan/phase3/00-overview.md`](../plan/phase3/00-overview.md) §11). Normative for `crates/ezsdr-sim` (module `channel`). |
| Scope | The `sim.channel` environment section and its admission check; the exact instants the channel computes at; the shared medium that the simulated radios of one Run transmit into and receive from; the field at a receiver — couplings, path gain, delay, the frequency gate and additive white Gaussian noise; the contract a transmitter meets; the rule that makes the field independent of the order in which the stepping loop steps the radios. |
| Not in scope | What a radio does with its own samples — gain, LO phase, clipping, path delay (spec 09 as amended by `plan/phase3/12-amendments.md` VB-3…VB-8). Carrier frequency offset, Doppler, phase noise, PA nonlinearity, IQ imbalance, band-limited interpolation, the carrier phase of a propagation delay, antennas, per-device drifting roots, RealtimeEmulation (Vision §16's richer models; `plan/phase3/00-overview.md` §3). |
| Crate | `crates/ezsdr-sim`, module `channel`. No new dependency (PO-4). |
| Depends on | Spec 08 as amended by VB-2 (`sim` 1.1.0); the Kernel's `RunId`, `Rational`, `AdmissionCheck` (unchanged). |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

---

## 1. Purpose

Vision §16 separates radio semantics from propagation: MockRadio implements the Radio Model, and a SimulationChannel between MockRadios carries what one transmits to what another receives. §57 names the first models — loopback, gain, delay, AWGN — and §58 #8 is the acceptance test: two MockRadios communicate through a SimulationChannel declared in the BindingProfile's `environment`, with a Spec that contains no channel configuration.

This document specifies that channel. It is not a Module. A Module cannot reach another Module (MA-3), and no Kernel contract carries RF propagation — on hardware the coupling is a cable or the air, which no Kernel type describes either. The channel is the `sim` Vocabulary's model of that physics: a medium object that the runtime creates once per Run and hands to every simulated radio it constructs (CH-6, decision C1).

## 2. Evidence

- **Vision §16**: the channel is "a coupling matrix over all TX ports × all RX ports, including a device's own RX. Self-interference is a diagonal-block entry, not a special case." Hence CH-4's paths, whose transmitting and receiving fragment may be the same.
- **Vision §8**: "SimulationChannel configuration lives in the BindingProfile `environment`, never in the ExperimentSpec." Hence CH-1's section.
- **Vision §15**: "Determinism with a seed follows from delivering events in virtual-time order, not from threads happening to agree." Hence CH-9: with one logical thread, the only remaining source of disagreement is the order in which the stepping loop visits the radios at one instant, and CH-9 removes it.
- **Vision §13**: the OpenAirInterface `rfsimulator` passes experiments that fail on the bench. A channel that let a receiver hear a transmitter on another centre frequency would be the same false confidence; hence CH-4's frequency gate (decision C4).
- **TM-1**: time in a document is an integer, never floating-point seconds. Hence `delay_ns` (decision C5).

## 3. Model

```text
                 BindingProfile.environment["sim.channel"]            (CH-1, checked by CH-2)
                                  │  read by every simulated radio at prepare
                                  ▼
 ┌───────────────────────── Medium  (one per Run, CH-6) ─────────────────────────┐
 │  paths  (tx, tx_channel) ──amplitude, delay──▶ (rx, rx_channel)       (CH-4)   │
 │  noise  one SimRng stream per (rx, rx_channel)                         (CH-5)   │
 └────▲──────────────────────────────────────────────────────────────┬───────────┘
      │ radiated(channel, at) → value, frequency         (CH-8)       │ field(rx, channel, frequency, at)
 transmitting radio (its bursts, gain, LO phase, path delay)      receiving radio (its gain, LO phase, clipping)
```

Every instant the medium handles is an exact rational number of root ticks (CH-3). A receiver asks for the field only at instants in the past of the current round, and a transmitter changes what it radiates only from the current round's instant on (CH-9), so the field at any instant is already final when it is asked for.

## 4. Rules

### The section

- **CH-1** The environment section `sim.channel`, when present, is this document (`ChannelSpec`), and `ezsdr_sim::channel::read(environment) -> Result<Option<ChannelSpec>, String>` is its one reader:

  ```text
  ChannelSpec { couplings:  [Coupling],            default []
                noise_dbfs: Map<Ident, num> }      default {}; each value in −400 ..= 100
  Coupling    { tx: Ident, tx_channel: u16,
                rx: Ident, rx_channel: u16,
                gain_db:  num,                     in −400 ..= 400
                delay_ns: u64 }                    default 0; in 0 ..= 2^62
  ```

  Both types are `deny_unknown_fields`. The reader returns `Ok(None)` when the section is absent and an error beginning `"CH-1: "` when the section is not a JSON object, when `couplings` holds an element that is not a JSON object (serde would otherwise read a struct from an array), when the document does not deserialise, or when a value is outside its range. `tx` and `rx` name fragments — in practice Spec resource names, as `sim.faults` targets do (SE-4); `noise_dbfs` is keyed by the receiving fragment. `ezsdr_sim::channel::CHANNEL_SECTION = "sim.channel"` and `MAX_DELAY_NS = 2^62`. *Checked by `ch_01_reader`.*

- **CH-2** `ChannelCheck` is the admission check of section `sim.channel` and runs at `validate` only. It reports one violation when CH-1's reader fails, whose reason is `"CH-2: "` followed by the reader's error; otherwise one violation per name, in `named`'s order, that is not a key of the per-fragment configuration it receives (KA-4), with the reason `"CH-2: <name> names no fragment of this Run"`. `ezsdr_sim::channel::named(spec)` returns every fragment id the document names — each coupling's `tx` and `rx` and every `noise_dbfs` key — once each, in `Ident` order. Every violation has `check: sim.channel`, `key: None`, `requested: None`. *Ceiling: the per-fragment configuration holds outputs as well as resources, so a name that is an output id passes; the radio refuses it at `arm`, because no radio joins under it (CH-7, MR-31).* *Checked by `ch_02_check`.*

### Exact instants

- **CH-3** `RootInstant { num: i128, den: i128 }`, `den > 0`, is an instant of `num / den` root ticks of the Authority's primary root, not necessarily reduced. `RootInstant::tick(t)` is `t / 1`. `RootInstant::of_sample(origin, ratio, k)` is sample `k`'s instant on a SampleClock whose origin is root tick `origin` and whose `ratio` is `root_ticks_per_tick` (TM-13a): `(origin · ratio.den + k · ratio.num) / ratio.den`. `minus_ticks(d)` subtracts `d` root ticks exactly. `sample_at_or_before(origin, ratio)` is `⌊(self − origin) / ratio⌋`, the last sample of that clock whose instant is at or before `self`, computed exactly in `i128` (saturating to `i64::MIN` or `i64::MAX` when the quotient does not fit). `is_at_or_after(tick)` is `tick ≤ self`. No floating-point number is ever an instant. *Checked by `ch_03_root_instants_are_exact`.*

### The field

- **CH-4** The **field** at receive channel `c` of fragment `rx` at instant `a`, for a receiver tuned to `f`, is

  ```text
  field(rx, c, f, a) = Σ over the paths p with p.rx = rx and p.rx_channel = c, in document order,
                          of  A_p · s     where  s = radiated(p.tx, p.tx_channel, a − D_p)  is Some
                                                  and s.frequency_hz = f exactly
                        + noise(rx, c)                                                   (CH-5)
  A_p = 10^(gain_db / 20)        D_p = ⌈delay_ns · R / 10^9⌉ root ticks, R the primary root's rate in hertz
  ```

  A path is one `Coupling`. Two couplings with the same ends are two paths and both count — a two-tap multipath channel costs nothing to allow (decision C9). A coupling whose `tx` and `rx` are one fragment is that radio's loopback or self-interference path; Vision §16's "loopback" model is this and nothing else. A path whose transmitter radiates nothing at `a − D_p`, or radiates on another centre frequency, contributes nothing: frequency offsets are not modelled (decision C4). The sum is taken in `f64`, in document order. *Checked by `ch_04_the_field_sums_paths_with_gain_delay_and_the_frequency_gate` and `ch_04_a_delay_rounds_up_to_a_root_tick`.*

- **CH-5** **Noise.** When `noise_dbfs` has an entry `P` for `rx`, every evaluation of the field at receive channel `c` of `rx` adds `σ · (g1, g2)` with `σ = sqrt(10^(P / 10) / 2)` — so the noise power, over both components, is `10^(P / 10)` of full scale — where `(g1, g2) = gaussian_pair(rng)` and `rng` is the stream `SimRng::new(seed, "sim.channel/<rx>/<c>")`, created at the first evaluation for that `(rx, c)` and kept for the Run. `seed` is the Run's `sim.seed` (SE-2). `ezsdr_sim::channel::gaussian_pair(rng)` is the Box–Muller transform of two draws: `u1 = (⌊x1 / 2^11⌋ + 1) · 2^−53`, `u2 = ⌊x2 / 2^11⌋ · 2^−53`, `r = sqrt(−2 · ln u1)`, and `(r · cos 2πu2, r · sin 2πu2)`. Each `(rx, c)` has its own stream, so a second receive channel never changes the first one's noise (SE-6's rule). With no entry for `rx` nothing is drawn. The noise is referred to the receiver's input: the receiving radio's gain scales it with the signal (MR-35). *Ceiling: a stream is indexed by evaluation, not by sample instant, so a sample the receiver never evaluates — one lost in an overflow's gap — shifts every later draw of that channel; a Run with the same faults reproduces, but a noise value is not a function of its sample's instant alone.* *Checked by `ch_05_noise_has_its_power_its_seed_and_one_stream_per_receive_channel` and `ch_05_gaussian_pair_follows_its_formula`.*

### The medium

- **CH-6** `Medium` is the shared object of one Run's SimulationChannel. `Medium::new() -> Arc<Medium>` creates an empty one; the runtime that assembles a Run creates one and hands a clone to every simulated radio it constructs (MockRadio: `MockRadio::with_medium`, MR-31). `join(run, fragment, spec, seed, root_rate_hz, transmitter)` registers `fragment` of Run `run` with its transmitter (CH-8). The first join fixes the Run, the document, the seed and the root rate, and precomputes each path's `A_p` and `D_p` (CH-4); it refuses a root rate of 0. A later join is refused when its Run differs ("CH-6: this medium serves Run <joined>, not <run>"), when its document, seed or root rate differs from the first join's ("CH-6: a join must carry the Run's one document, seed and root rate"), or when `fragment` has already joined ("CH-6: <fragment> has already joined"); the first join is also refused when a path's delay in root ticks does not fit an `i64` ("CH-6: a delay overflows the root"), which `delay_ns` up to 2^62 can reach only at a root faster than about 2 GHz. A refusal changes nothing. *Checked by `ch_06_join_rules_and_missing_fragments`.*

- **CH-7** `Medium::missing()` returns, in order, every name of `named(spec)` (CH-2) that has not joined; before any join it is empty. A receiving radio calls it at `arm`, after every fragment of the Run has been prepared (KC-12, KC-14), and refuses to arm when it is not empty (MR-31): a coupling to a fragment that never joined — an output id, a misspelling the check could not see, or a radio the runtime built without the shared medium — would otherwise be silently deaf. *Checked by `ch_06_join_rules_and_missing_fragments` and, for the refusal, `mr_31_channel_mode_needs_a_medium_a_zero_pattern_and_valid_channels`.*

- **CH-8** A **transmitter** implements `ezsdr_sim::channel::Transmitter`: `radiated(channel, at) -> Option<Radiated>` returns what transmit channel `channel` radiates at instant `at` — the complex baseband value `re, im` of the sample it holds at `at` (a zero-order hold: a sample is radiated from its own instant until the next sample's), and the carrier `frequency_hz` it radiates it on — or `None` when that channel radiates nothing at `at`. It is read under the transmitter's own lock and must not call the medium, so that a radio whose receive side evaluates its own loopback path (CH-4) cannot deadlock. The medium calls it without holding its own lock. *MockRadio's implementation is MR-32; its tests are MR-32's.*

- **CH-9** **Finality.** During a stepping round at instant `now`:
  1. a transmitter changes what it radiates only at instants at or after `now` — a burst starts at or after `now`; a stop or a cold change first transmits every burst that started before `now`, including one whose first sample lies between the previous round and `now` and which has therefore not been opened yet, and then cuts every burst at its first sample at or after `now`; a gain or frequency change takes effect at or after `now` (MR-17, MR-25, MR-32, MR-33);
  2. a receiver evaluates the field only at instants before `now` (MR-14).

  Both rest on the stepping loop stepping every radio at every instant any radio scheduled (KC-20 advances to the Authority's next wakeup; MA-30): a round that jumps past a transmitter's scheduled change applies it late, after a receiver may already have evaluated instants it covers. A test harness that steps radios itself must therefore step at every due wakeup, as `mock_channel.rs`'s `run_to` does.

  **Ceiling (Gate X, Review C P2-1):** a round that a Module error ends early does not step the remaining instances — `step_until_quiescent` returns at the first error — so which blocks a receiver published before the abort can depend on fragment order; the published values do not. Measured through the acceptance rig: a receiver that sorts before a transmitter reporting `device_lost` published one block, one that sorts after it none. A Run still reproduces from its own documents, fragment names included; what happens in the rest of a round after a failure is Phase 4's failure work.

  Every path's delay is at least 0, so every radiated value a receiver reads at round `now` belongs to an instant before `now`, which no step at `now` can change. The field, and so every received sample, is therefore independent of the order in which the stepping loop steps the radios (MA-30). Without (1) and (2), a burst starting at exactly `now` would be heard or missed depending on whether its transmitter or its receiver was stepped first, and a burst whose first sample lies between two rounds would be heard by one receiver and withdrawn for another. *Checked by `ch_09_the_receive_output_does_not_depend_on_the_stepping_order` (the order swapped in the round in which a burst starts at the instant of the receiver's last block sample), `ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order` (a burst whose first sample, at 666 333⅓ ns, precedes the round of a stop) and, end to end, `v58_08_a_session_hears_a_burst_from_its_first_sample_in_either_instance_order`.*

- **CH-10** `ChannelSpec` has the committed schema `schemas/sim/channel.v1.json` (PO-7), generated by `ezsdr_sim::document_schemas()` and frozen by `se_12_schema_freeze` in both directions. *Checked by `se_12_schema_freeze`.*

- **CH-11** **Determinism.** The medium draws random numbers only from its CH-5 streams, iterates ordered collections only (PO-11), and its output is a function of the first join's document, seed and root rate and of the calls made to it. Two Runs with one Spec, one BindingProfile and one set of inputs therefore produce identical fields, on one build of the program on one platform. *Ceiling: `ln`, `cos`, `sin` and `powf` come from the platform's math library, whose last-bit results Rust does not specify across platforms; two platforms may differ in the last bits of a noisy or scaled sample. A portable math library is the upgrade, when a cross-platform bit-exact reproduction is asked for.* *Checked by `v58_03_channel_noise_reproduces_with_its_seed` and `v58_03_a_run_reproduces_from_its_own_manifest`.*

## 5. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| C1 | Where the channel lives | A shared `Medium` in the `sim` Vocabulary crate, created by the runtime once per Run and handed to each simulated radio (CH-6) | A channel Module (no role fits, and MA-3 forbids it reaching a radio); Kernel Links between radios (a link is a Spec graph edge, so the channel would be in the Spec, which Vision §8 and §58 #8 forbid); inside MockRadio (Vision §16 separates the two, and a second kind of simulated device could not share it); a process-global registry keyed by `RunId` (hidden coupling, and every MockRadio test harness uses one `RunId`) | In-process only: a Plugin-deployed radio would need a medium protocol (MA-46 names none) |
| C2 | Who versions the model | The `sim` Vocabulary: CH-4 and CH-5 are normative, so a change of the model is a new `sim` version, which the Manifest records | An implementation-defined model (a changed noise generator would change recorded Runs with no recorded version change) | none |
| C3 | Sampling between grids | Zero-order hold: a transmit sample is held until the next (CH-8) | Band-limited interpolation (a filter to choose and to cost, and no Phase 3 test needs it); refusing unequal rates or a delay off the grid (a runtime `cold` change could not be refused cleanly) | Exact only on a shared grid with a delay of whole transmit samples; a later `sim` version may add an interpolating model |
| C4 | Centre frequency | A path counts only between exactly equal frequencies (CH-4) | Ignoring frequency (a receiver would hear a transmitter on another band: looser than hardware, Vision §59); rotating by the offset (CFO is a model Vision §16 lists separately, and without a band limit it aliases) | A 1 Hz offset is silence, not a slowly rotating signal: stricter than hardware |
| C5 | Delay unit | Integer nanoseconds, rounded up to a root tick (CH-4) | Samples (a path between two rates has no one sample unit; Vision §8's `delay_samples` is illustrative); float seconds (TM-1) | none |
| C6 | Noise | Per receiving fragment, input-referred, as a power in dBFS (CH-5) | An SNR (undefined for a bursty signal); noise per path (noise belongs to a receiver) | Per-channel noise powers are an additive field later |
| C7 | Gaussian draws | Box–Muller over `SimRng` (CH-5) | A sum of uniforms (not Gaussian in the tails); a ziggurat (tables for no Phase 3 need); `rand_distr` (a dependency, PO-4) | CH-11's platform ceiling |
| C8 | How the order of stepping stops mattering | Finality: transmitters change only from `now`, receivers read only before `now` (CH-9) | Forbidding a zero command lead (the `ideal` profile needs it); stepping transmitters before receivers (a Kernel concept for a Vocabulary need, OV-21); computing received samples from emitted transmit blocks (a block is emitted only when complete, so the past samples of the block in progress would be missing) | none |
| C9 | Several paths between one pair of ends | Allowed and summed (CH-4) | Refusing a duplicate `(tx, tx_channel, rx, rx_channel)` (a two-tap channel is useful and costs nothing) | none |
| C10 | The carrier phase of a delay | Not modelled: a delayed path is a delayed copy with a real gain | `e^{−j2π f D}` per path (needs the carrier frequency in the medium and gives nothing Phase 3 tests) | A later `sim` version with complex path gains |
| C11 | Evaluation granularity | One field evaluation per receive sample (CH-4) | Block-wise evaluation (faster, but the noise streams, the gate and the timelines would have to be vectorised identically, for no Phase 3 need) | Measured on the planning machine: a channel-coupled receiver at 1 Msps costs about 1 s of wall time per virtual second in a debug build and about 0.15 s in a release build. Vision §58 #2's faster-than-wall-clock claim stays carried by a Run without a channel; block-wise evaluation is the upgrade |

## 6. Tests

`crates/ezsdr-sim/tests/sim_channel.rs` (new):

| test | input | expected | rules |
|---|---|---|---|
| `ch_01_reader` | absent; `{}`; a two-coupling document with noise; `{ couplings: [], noise_dbfs: {} }`; eleven malformed documents (an array; an extra member; a coupling without `gain_db`; one with an extra member; `tx_channel` −1 and 70 000; `delay_ns` −1 and 2^62 + 1; `tx` `"A b"`; a string noise; an array-form coupling); gains ±400.5; noises 100.5 and −400.5; `delay_ns` exactly 2^62 | `None`; the default; the values read back; `Some`; each refused with an error beginning `"CH-1: "`; the edge value accepted | CH-1 |
| `ch_02_check` | fragments `a`, `b`; a valid document; one naming `x`, `y` (twice) and `z`; `{ couplings: 3 }` | no violation, stage `validate` only, section `sim.channel`; exactly three violations `"CH-2: x …"`, `"CH-2: y …"`, `"CH-2: z …"`, each `key: None`; one violation beginning `"CH-2: CH-1: "` | CH-2 |
| `ch_03_root_instants_are_exact` | ratio 1000/3 at origin 10 | sample 7 is `7030/3`; `sample_at_or_before` gives 7, 6 one tick earlier, −1 before the origin, 0 at the origin and at 343, 1 at 344; `is_at_or_after` true at 2343, false at 2344 | CH-3 |
| `ch_04_the_field_sums_paths_with_gain_delay_and_the_frequency_gate` | four transmitters (`a` on 1 GHz with two channels, a silent `b`, `c` on 2 GHz, `d` on 1 GHz + 1 Hz); paths `a/0→b/0` at −20 dB and 100 ns, `a/1→b/0` at 0 dB, `c/0→b/0` at 0 dB, `a/0→b/1` at +6 dB, `d/0→b/0` at 0 dB | the field at `b/0`, 1 GHz, tick 1000 is `0.1 × a₀(900) + a₁(1000)`, and `c` and `d` are asked but not counted (the gate is exact equality, C4); at `b/1` it is `10^0.3 × a₀(50)`; tuned to 2 GHz only `c` counts; before `a₀` starts only `a₁` counts; a fragment with no path into it has field 0 | CH-4 |
| `ch_04_a_delay_rounds_up_to_a_root_tick` | root 250 MHz, a 1 ns loopback | the transmitter is asked at tick 9 for the field at tick 10 | CH-4 |
| `ch_05_noise_has_its_power_its_seed_and_one_stream_per_receive_channel` | −20 dBFS on `b`; 100 000 draws; seeds 7 and 8; channels 0 and 0+1 | mean power within 0.0005 of 0.01 and mean within 0.002 of 0; one seed reproduces, two seeds differ; channel 0's draws are the same whether or not channel 1 draws; channels 0 and 1 differ; the first draw is `σ · gaussian_pair(SimRng::new(7, "sim.channel/b/0"))` | CH-5, SE-6 |
| `ch_05_gaussian_pair_follows_its_formula` | `SimRng::new(0, "")` | the pair equals CH-5's formula applied to the stream's first two values, bit for bit; the stream then continues at its third value | CH-5 |
| `ch_06_join_rules_and_missing_fragments` | a document naming `a`, `b`, `c`; joins in turn | empty `missing()` before any join; `[b, c]` after `a`; the five refusals (a second `a`, another Run, another document, another seed, another root rate) each begin `"CH-6: "`; empty after all three; a root rate of 0 is refused; a 2^62 ns delay is refused at a 2 GHz root ("CH-6: a delay overflows the root") and accepted at 1 GHz | CH-6, CH-7 |

`crates/ezsdr-sim/tests/sim_vocabulary.rs` (amended by VB-2): `se_01_register_adds_the_descriptor_and_the_three_checks` (replaces `se_01_register_adds_the_descriptor_and_both_checks`); `se_12_schema_freeze` now also generates `channel`.

The rules whose carriers are MockRadio's (CH-7's refusal, CH-8, CH-9) are tested in `crates/ezsdr-mock-radio/tests/mock_channel.rs` (`plan/phase3/12-amendments.md` VB-7).

## 7. Vision issues found

1. **§15 lists "SimulationChannel delays" among the wake-ups the Simulation Engine delivers.** Phase 3 needs none: a transmitter's content is known before its instant, and a receiver computes a delayed path when it publishes a block (CH-4, CH-9). §15 should say the channel is evaluated by the receiving model, not scheduled.
2. **§8's `sim.channel` example** (`model: awgn, snr_db: 10, delay_samples: 37`) becomes CH-1's `{ couplings: [{ tx, tx_channel, rx, rx_channel, gain_db, delay_ns }], noise_dbfs: { <rx>: dBFS } }`: an integer delay in nanoseconds (C5), a noise power rather than an SNR (C6), and explicit ends, which a coupling matrix needs.
3. **§13's Simulation Environment lists SimulationChannel beside MockRadio and the Simulation Engine** as if it were a Module. It is the `sim` Vocabulary's shared medium, handed to each simulated radio by the runtime (C1), as §13 already says of the fault schedule.
4. **§16 and §57 list "loopback" as a model beside gain, delay and AWGN.** A loopback is a coupling whose two ends are one radio (CH-4); §16's own sentence that self-interference "is a diagonal-block entry, not a special case" already says so.

## 8. Deferred

The richer models of Vision §16 (multipath beyond summed couplings, CFO, Doppler, phase noise, PA nonlinearity, IQ imbalance, a MIMO channel beyond independent paths). Band-limited interpolation (C3). Complex path gains (C10). A medium protocol for a Plugin-deployed radio (C1). Per-device drifting roots (`plan/phase3/00-overview.md` Z2).
