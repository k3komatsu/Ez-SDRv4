# Phase 5 — Vision issues

Collected from spec 14 §8 and spec 15's "Vision issues found". Applied at Step X on 2026-09-26, after the owner accepted Gate X as recommended (OV-6, GX-5).

| # | Section | Change | Source | State |
|---|---|---|---|---|
| 1 | §9 | The conceptual shape of an ExperimentSpec gains, after `resources`: `inputs — artifacts the Run consumes that no schedule entry carries, such as a Reactor's waveform or a calibration artifact (§26)` | spec 15 KE-1, SB-20a | **applied** (Step X) |
| 2 | §22 | "MockRadio applies the same policy with the same envelope, so a Reactor that is too slow for the hardware fails in simulation." becomes: "MockRadio applies the same policy with the same envelope, so a Reactor whose response leaves the device too little lead — counted from when the device delivered the samples it reacts to — fails in simulation as on hardware. Simulation charges a component no processing time of its own; RealtimeEmulation and a component's declared budget are where that is exposed." | spec 15; `00-overview.md` R4 | **applied** (Step X) |
| 3 | §19 | After "Concrete execution engines remain Modules.": `Normative: design/14-native-executor.md (the first, ezsdr.exec.native).` (re-review R13) | spec 14 §8 | **applied** (Step X) |
