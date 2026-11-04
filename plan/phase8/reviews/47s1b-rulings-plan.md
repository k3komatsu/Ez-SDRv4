# Stage 1b: orchestrator rulings on the implementer's plan questions (2026-11-04)

1. **Default profile names:** the server's default profile stays unnamed in stage 1b. The portability test uses two explicit Mock profiles that put `loop-rx`/`loop-tx` on different numbers. Naming the default profile is a later owner question (reported).
2. **One namespace for rx and tx:** yes. A name says radio, direction and number (note 47: 名前だけでどの radio のどの channel か決まる). A duplex front end listed in both `rx` and `tx` carries two names.
3. **Stream numbers are per owner radio:** a numbered stream key is a number in its owner's radio; a number that radio lacks is refused (RM-4, before any Run). Multi-radio programs use stream names; no cross-radio renumbering.
4. **Hashes:** the Spec hash is equal with and without names; the binding description and plan fragment hashes differ because the profile text is an input. Accepted; the test pins exactly that.
5. **Cross-radio uniqueness in `catalogue::assemble`:** accepted (covers `run_child` and the acceptance rigs too).
6. **`StreamNames` → `DirectionNames`** serving both tables: accepted.
7. **Shared name-rule fixture:** accepted — one `names.json` case list read by the Rust and Python suites.
8. **`plan/phase8/tools/run-record.sh`:** strip `#…` before comparing; hand check recorded in the plan.
9. **Wire rule for name-only streams:** as proposed (the owner radio of the names, refusals for mixed owners, a number on another radio, unknown name, other-direction name; the stream's key moves with it).
