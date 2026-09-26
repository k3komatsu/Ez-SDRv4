# Phase 3 Vision issues collected at Step 7

Eight edits, from the "Vision issues found" sections of
[spec 11 §7](../../design/11-simulation-channel.md#7-vision-issues-found) (four) and
[spec 12 §3](12-amendments.md#3-vision-issues-found) (four). Per GV-5 the
Vision is **not** edited during Phase 3; these are applied at Step X with the
owner's approval, adding one row to the revision history in
`Ez-SDR_v4_ARCHITECTURE_VISION.md` (AGENTS.md §2).

**Applied at Step X (2026-09-26)**, after the owner accepted Gate X, with the
revision-history row of that date. Issue 1 also rewrote §15's "models scheduled on
it" sentence, and issue 8 moved clipping out of §16's channel list.

## Spec 11 — the SimulationChannel

1. **§15** lists "SimulationChannel delays" among the wake-ups the Simulation
   Engine delivers. Phase 3 needs none: a transmitter's content is known before
   its instant, and a receiver computes a delayed path when it publishes a block
   (CH-4, CH-9). §15 should say the channel is evaluated by the receiving model,
   not scheduled.
2. **§8**'s `sim.channel` example (`model: awgn, snr_db: 10, delay_samples: 37`)
   becomes CH-1's `{ couplings: [{ tx, tx_channel, rx, rx_channel, gain_db, delay_ns }], noise_dbfs: { <rx>: dBFS } }`: an integer delay in nanoseconds (C5), a noise power rather than an SNR (C6), and explicit ends, which a coupling matrix needs.
3. **§13**'s Simulation Environment lists SimulationChannel beside MockRadio and
   the Simulation Engine as if it were a Module. It is the `sim` Vocabulary's
   shared medium, handed to each simulated radio by the runtime (C1), as §13
   already says of the fault schedule.
4. **§16** and **§57** list "loopback" as a model beside gain, delay and AWGN. A
   loopback is a coupling whose two ends are one radio (CH-4); §16's own sentence
   that self-interference "is a diagonal-block entry, not a special case" already
   says so.

## Spec 12 — the amendments

5. **§26** says "Ez-SDR carries a default per device profile in the Radio Model";
   RM-23 carries it as two capabilities in samples, as srsRAN and
   OpenAirInterface do. §26 should name them.
6. **§23** says "MockRadio clips at the contract's full scale (Phase 2 Mock
   obligations)"; MR-36 does, in Phase 3. The parenthesis should read "(MR-36)".
7. **§25**'s "the Provider declares this behaviour and MockRadio emulates it" is
   MR-34; §26's closing sentence "emulation of phase changes on retune is Phase 3
   work" should cite it.
8. **§13 rule 3** ("RF behaviour is not part of the envelope. It stays in
   SimulationChannel") and **§16**'s list, which names clipping among the
   channel's models, put all RF behaviour in the channel. Phase 3 puts the
   device's own RF behaviour — gain, LO phase, clipping, path delay — in the radio
   model and only propagation in the channel (M15), as §23 (MockRadio clips) and
   §25 (MockRadio emulates the retune phase) already imply. §13 rule 3 should say
   propagation stays in the SimulationChannel, and §16 should call clipping the
   radio's.

## Not among them

Review C's P2-1 — a round that a Module error ends early does not step the
remaining instances, so which blocks a receiver published before the abort can
depend on fragment order — is **not** a Vision issue. It is a ceiling for CH-9 and
MR-30, and the owner's Gate X verdict, with the measurement in
[`implementation-notes.md`](implementation-notes.md).
