# 2026-11-15 bench probes — Sol's code review (2026-11-13, openai/gpt-6.1-sol, read-only)

## Verdict: FAIL

These probes collect useful traces, but several decision lines can recommend a change without the required evidence. Reviewed `89fd3f7..c959310`, read-only.

### Independent failure model

**VERIFIED, UHD 4.10:** [`set_decim`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/ddc_block_control.cpp) enables `min(v2(D), NUM_HB)` half-bands; the standard DDC defaults to three. Rates can be coerced. ACKs are [posted along the radio’s input graph edge](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/radio_control_impl.cpp#L1312-L1317), not tagged with a streamer generation. The [host ACK queue](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/tx_async_msg_queue.cpp) has no age-based expiry.

**INFERRED hardware risks:** noise/quantisation masquerading as signal, changed level or startup state moving zero counts, unstable control offsets, foreign ACKs, and an ACK never generated being mistaken for one lost during the delayed read.

## Major — must fix

1. **Five decimation cells cannot become complete.**  
   `crates/ezsdr-radio-uhd/tests/common/bench1115.rs:441-465` — **VERIFIED** by arithmetic. For D=3, the 400,000-tick interval contains 133,334 samples, but `covered` demands `400000 / 3 = 133333`. D=6,12,24,60 fail likewise. Align the analysed span to complete samples and the tone-analysis window; add a successful odd-decimation coverage test.

2. **Missing evidence still produces affirmative RXZ decisions.**  
   `crates/ezsdr-radio-uhd/tests/common/bench1115.rs:307-390` — **VERIFIED**. One counted cell per class suffices for “D holds”; one fresh cell suffices for “every level rate”; one level comparison can yield “no KNOWN_LIMITATIONS entry.” Thus finding 1 can silently remove crucial cells while D still passes. Require the complete planned cell set and ≥10 valid observations per cell—not the newly introduced 8-of-10 threshold—or report INCONCLUSIVE.

3. **The proposed bound excludes the level cells.**  
   `crates/ezsdr-radio-uhd/tests/common/bench1115.rs:320-323,362-392` — **VERIFIED**. Example: gain-0 class-three cells give 14 zeros, gain+20 gives 20. The output retains D and proposes bound 14, merely adding a level limitation. That is not a bound over the swept conditions. Compute the candidate maximum over all applicable measured levels/start kinds, separately from the class-structure comparison.

4. **An unstable control can falsely establish the skip-T3 hazard.**  
   `crates/ezsdr-radio-uhd/tests/common/bench1115.rs:744-781,883-886` — **VERIFIED**. Control offsets `[0;5] + [10000;5]` all PASS; their median is 10000. A skipping trial at offset 0 then FAILs, and the decision declares T3 necessary. Reject an unstable control and bracket/interleave controls before attributing the difference to T3.

5. **ACK decisions claim causes the observations do not establish.**  
   `crates/ezsdr-radio-uhd/tests/common/bench1115.rs:951-964,1019-1029,1105-1116`; `plan/phase8/bench.md:1315-1318` — **VERIFIED**.  
   - One ACK with **no tick** passes routing; all PASS then declares ACKs streamer-owned, contradicting the source-level routing model. Require distinguishable timing and say only “no foreign ACK observed.”  
   - A device-call error—or a T3 whose ACK was never generated—becomes “the late ACK was lost; waiting cannot recover it.” Separate transport/setup failure, missing ACK and demonstrated retention. Absence alone cannot prove expiry.

6. **The open-flush decision bypasses the required two-channel evidence.**  
   `crates/ezsdr-radio-uhd/tests/common/bench1115.rs:1282-1291`; `plan/phase8/bench.md:1325-1328` — **VERIFIED**. Thirty successful one-channel trials plus three NOT RUNNABLE cells print “enable (3).” The owner gated (3) on the one- **and two-channel** sweep. Keep the global decision INCONCLUSIVE pending those cells or an explicit owner waiver.

7. **The commands combine three transmitting probes behind one confirmation.**  
   `plan/phase8/bench.md:1265,1272-1273` — **VERIFIED**. Separate the commands and require the owner’s attenuator confirmation before each transmitting probe, as requested.

## Minor — should fix

8. **The onset noise guard disappears when the median is zero.**  
   `crates/ezsdr-radio-uhd/tests/common/bench1115.rs:734-741` — **VERIFIED**: sparse sc16 noise with median zero and positive 90th percentile passes `top >= 8 × median`. Add a quantised-noise case and a positive noise-floor/coherent-tone requirement; flat-noise tests do not cover this.

### Checks and limits

- Actual RX-rate read-back is checked through `iii::incomplete_reason`; the half-band formula is correct.
- Warm-up exclusion does not eliminate the fresh-streamer measurement: every counted fresh trial reopens.
- Whole-life software bounds correctly include creation, unwind and teardown. RF-off after abort/loss remains **INFERRED**, not guaranteed; power-cycle handling remains necessary.
- **B1115-01…33:** inspected mutations and killing assertions; `mut.log` records 33 ordinary `killed` outcomes, none `killed (bound)`. Several assertions nevertheless encode the flawed decisions above.

No edits, formatting, builds, hardware transmissions or remote runs. No probe copy was created.
