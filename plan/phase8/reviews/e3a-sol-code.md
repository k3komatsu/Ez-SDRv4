# E3a — Sol's code review (2026-11-12)

## Verdict: PASS WITH CHANGES

No major findings or reachable behavior regression found. One minor documentation correction.

### Minor — VERIFIED

**`Plan::short_first` incorrectly implies that the send closes the device burst.**  
`crates/ezsdr-radio/src/segment.rs:80`


Describe this field only as the first-send size classification; closure is reported by `device_eob`. No code change needed.

### Review conclusions

- **VERIFIED:** UHD and Mock call-site arithmetic preserves valid-booking behavior. Mock wake and emission now share `tx_block`; receipt ordering, previous-cut merging and the Stop floor remain intact. Zero-length Mock waveforms are refused before booking (`crates/ezsdr-mock-radio/src/lib.rs:1581`).
- **VERIFIED:** Characterization tests observe send boundaries, flags, timestamps and cursors—not merely totals. Pure tables and E3A mutation killers cover the extracted decisions; call-site rows protect wiring. No introduced maintainability blocker found.
- Mock’s window-only joined-piece difference is **pre-existing**, not a new hardware-envelope relaxation. Keep the planned note 49 §5.4.8 correction at landing. No additional `design/18-uhd-radio.md` amendment identified.

### Unnamed-thread drop: noise, not a branch regression

**VERIFIED:** All eight retained `v58` runs have identical 85-file content multisets. Both unnamed dumps contain zero RX/TX blocks and no bursts or segments.

**VERIFIED mechanism; INFERRED attribution:** cleanup workers retain `Arc<Shared>`, send completion, then release it without being joined (`crates/ezsdr-kernel/src/run.rs:596–607`; `coordinator/ending.rs:33`). The last owner can therefore disappear on either thread. The affected validation-refusal cases never execute segmentation. The 2/4 versus 0/4 observation does not establish a new race.

**Checks:** 24 targeted tests passed remotely on sim03. All E3A rows are killed in the supplied full mutation log.

Not rerun: full gate, mutations or hardware. No files changed.
