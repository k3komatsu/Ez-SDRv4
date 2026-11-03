You are read-only (edit nothing, never run cargo fmt, no git state changes, no builds needed) in ~/local_work/Ez-SDRv4 at main 50b5e83 plus one new uncommitted file.

Task: fact-check the new design note plan/phase8/46-streaming-feature-ledger.md (Japanese). It is a LEDGER for the owner: for each feature the UHD Module allows while a stream receives or transmits (F1-F8), it lists what the feature lets an experimenter do, its basis (Vision / Radio Vocabulary spec 07 / Module spec 18 / Easy API spec 16), the bugs it produced, what it carries, what vanishes if restricted, what is lost, and intermediate restriction levels. The owner, not you and not the orchestrator, decides which restrictions to take. Do NOT recommend restrictions and do not judge which option is better. Judge only whether the facts are right and complete.

First, before reading the note's §3 in detail, write your own list of the features of crates/ezsdr-radio-uhd that run concurrently with a flowing stream (from design/18-uhd-radio.md §4, design/07-radio-model.md, crates/ezsdr-radio-uhd/src/provider/), so that your check is not anchored to the note's list. Then compare.

Check, with path:line evidence for every claim:
1. The "bugs produced" attribution per feature: plan/phase8/research/45-concurrency-inventory.md §7 groups and the issues in plan/maintenance/failure-mechanisms.md (and the review records it cites). Say which attributions are wrong, missing or overstated.
2. Count per feature, as far as it can be separated: production lines and test functions in crates/ezsdr-radio-uhd, crates/ezsdr-mock-radio and crates/ezsdr-radio. State how you counted (grep patterns / line ranges) so it can be rerun. Mark counts that cannot be separated cleanly.
3. The basis column: is each feature really required by the cited RM/MR/EA/Vision text? Any missing basis (e.g. a Vision §58 acceptance test or a §61 v3 behaviour that depends on the feature)?
4. "What vanishes if restricted": would it really vanish, or does another remaining feature (or §2's always-kept list) still use the same mechanism? Name the mechanism and the user.
5. F3's INFERRED item: if the restore at S were dropped, can a timed value left in the DDC/DUC command queue override the next Run's configuration? Use the UHD 4.10 sources and plan/phase8/39-dsp-timed-commands.md; mark VERIFIED/INFERRED.
6. §2 "kept regardless": is anything there actually a feature that belongs in the ledger, or anything in the ledger that cannot be restricted at all because the hardware requires it?
7. §4 combinations: are the stated dependencies right? Any missing dependency (one restriction forcing or removing another)?
8. Any feature that runs while streaming and is missing from F1-F8.

Output: a list of findings, each with the note's section, what is wrong or missing, the evidence (path:line or URL), and the corrected fact. Then a table of your counts per feature. End with one line: FACTS OK, or FACTS NEED CHANGES with the number of findings.
