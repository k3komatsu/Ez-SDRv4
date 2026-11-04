**PASS — no P1/P2/P3 findings.**

- The fallback is sound: a key with no registered declaration cannot pass `check_keys`, regardless of the retained declared-id prefix test. It changes only which refusal message appears.
- Declared keys require their actual owner in `requirements`; nested/shared prefixes and differing IDs no longer cause incorrect acceptance or refusal.
- Both checks cover all five `keyed_entries` positions. Extension keys retain registered-Module ownership checks; component schedule keys retain their `ParamDecl` exception.
- `plan()` reruns the same structural checks with the registry.
- The shared-prefix validation regression and nested negative case cover S5. P1360, P1327 and P22 target the intended decisions by inspection.
- SB-2’s amendment, indented *Test:* line and Changes row follow the accepted-spec format.

`git diff --check` passed. No files changed.
Not rerun: Rust tests, mutation execution or hardware; execution results remain unverified.
