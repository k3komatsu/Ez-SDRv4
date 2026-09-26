# Phase 4 — Vision issues

Collected from spec 13 §3. Applied at Step X with the owner's approval (OV-6, GW-5); until then the Vision is unchanged.

| # | Section | Change | Source | State |
|---|---|---|---|---|
| 1 | §28, §51 | Each gains `Normative: design/10-host-data-path.md, rule HD-15; design/02-stream-contract.md, rule SC-32.` SigMF interoperability now exists, so "should be considered" is backed by a rule (re-review R13) | spec 13 VC-3 | pending Gate X |
| 2 | §29 | After "The hot path emits a fixed-size record with at most 32 bytes inline and allocates nothing.": "The bytes are the owning Vocabulary's layout, which it also decodes; the Kernel counts and queues them without interpreting them (`radio.RX_OVERFLOW` is the first, RM-24)." | spec 13 VC-1 | pending Gate X |
