# Stage 1b plan review: dispositions (orchestrator, 2026-11-04)

Sol: PLAN NEEDS CHANGES (`sol-s1b-plan-final.md`), 4 major, 2 minor. All adopted.

| # | Severity | Finding | Disposition |
|---|---|---|---|
| 1 | major | binding identity: the Kernel's `binding_description` (opaque selector text) differs with names, so RS-25a refuses a child whose profile drops the names | adopted as a **ruling on identity**: a name is part of the binding (the Kernel keeps the selector opaque; no radio-specific stripping enters the Kernel). A child Run must repeat its parent's selector exactly, names included. Plan adds the child-Run sequence (same named selector → admitted; names stripped or changed → RS-25a refusal) and RM-28/RS-25a text says a name is part of the binding. "Behaves exactly the same" applies to the Provider's behaviour (layout, coerce, effective, capabilities, device calls), not to the Kernel's binding identity |
| 2 | major | `crates/ezsdr-radio-uhd/tests/common/measurement.rs:2115-2120` `raw_probes_applicable()` compares raw strings | adopted: add to the reader inventory; strip `#…`; extend `fake.rs:8921` with named/plain UBX/OBX accepted and named CBX/slot-B refused; mutation row removing the stripping |
| 3 | major | client membership checks reject valid mixed number/name references | adopted: the client refuses only what it can prove locally; membership needing name resolution is the server's; positive sequences (`Stream([0], gain={"loop-rx": 5})`, `Stream(["loop-rx"], gain={0: 5})`), true non-member refusals, number/name duplicate-value refusals; P1345 retargeted accordingly |
| 4 | major | value-key / channel grammar: `parse::<u32>()` reads `+0` as 0 | adopted: a key or channel is a number only when spelled as canonical ASCII decimal; any other legal spelling is a name; an illegal name (e.g. `00`, digits only) is refused. Cases `+0`, `-0`, full-width digits, leading zeros, overflow; revisit `protocol.rs:572`'s expectation |
| 5 | minor | P1362 equivalent | adopted: drop it; reuse P1334/P1335 instead of P1365/P1368 duplicates |
| 6 | minor | cross-Provider full-tree equality is a wrong oracle; device record missing | adopted: named vs plain within each Provider; board surfaces across Providers; UHD's recorded `device.subdev_spec` against actual calls (`fake.rs:6906`); name the concrete parity test |
| note | — | `run-record.sh:131` is the RF attenuator-confirmation prompt | the stripping must keep the attenuator prompt working; the hand check covers the prompt |
