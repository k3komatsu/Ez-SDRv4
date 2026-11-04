# Sol re-review: note 47 stage 1, fix round 2

Read-only, same rules as before (`brief-sol-s1-code.md`). Your last finding: `sol-s1-fix-final.md` (P1, shared-prefix ownership through validate). Dispositions: `dispositions-s1-code.md` (S5). The working tree (uncommitted, HEAD `b3d39ab`) holds the fix; plan `test-plan-s1.md` rev 5 §7.

Review:
1. `ExperimentSpec::check_key_prefixes` (crates/ezsdr-kernel/src/spec.rs) now takes the registry and requires a declared key's owner (`vocabulary_of`) to be in `requirements.vocabularies`. The implementer kept the old declared-id prefix test for a key no registered Vocabulary declares, so `check_keys` still gives its existing message (`sb_02_the_ext_owner_is_checked_at_every_key_position`). Is that sound — can any key now pass both checks that SB-2 refuses, or be refused that SB-2 allows? Check every key position (`keyed_entries`), ext keys, component schedule keys, and `plan()` running the structural checks again.
2. Tests `sb_02_a_key_is_owned_by_the_vocabulary_that_declares_it`, `sb_22c_a_session_asking_a_shared_prefix_key_validates_and_declares_its_owner`, the nested case through validate; rows P1360, P1327, P22.
3. SB-2's amended text and its Changes row (design/03-spec-and-binding.md), in the accepted-spec format.

Report in your final message: verdict (PASS / NEEDS CHANGES), findings P1/P2/P3 with path:line, minimal sequence, wrong outcome. At most 40 lines.
