# Phase 7 — Vision issues

Collected from specs 18 and 19. To be applied at Step X, after the owner accepts Gate X (OV-6, GZ-7). None changes what the Vision asks for; each points a Vision section at the rule that now carries it (re-review R13) or records a reading the design had to settle.

| # | Section | Change | Source | State |
|---|---|---|---|---|
| 1 | §35 ("UHD") | After "USRP support should use a narrow native C++ bridge to UHD unless a mature Rust UHD interface becomes clearly superior.": "UHD's own C API (`uhd.h`) is such a bridge: every entry point catches the exceptions UHD can throw and returns a status, and only POD types and opaque handles cross it." At the end of the subsection: `Normative: design/18-uhd-radio.md.` | spec 18 §9 issue 1; overview T2 | **applied** at Step X (2026-10-01), refined: "catches the exceptions UHD throws to it", and a clause on the throw out of UHD's own destructors that no entry point can catch (bench F4, design-notes §17) |
| 2 | §15 ("Time Authority") | "In a Hardware or HIL Run it is the device timekeeper, which publishes a relation to host monotonic." gains "and to UTC, which the Manifest records (design/05-module-api.md MA-29, design/06-kernel-coordinator.md KC-45)" | spec 19 KG-11 | **applied** at Step X (2026-10-01), the paths as links |
| 3 | §32 ("Driving model per ExecutionClass") | The `Normative:` line gains `; design/06-kernel-coordinator.md, KC-46 (the data thread of the device-paced classes)` | spec 19 KG-2 | **applied** at Step X (2026-10-01) |
