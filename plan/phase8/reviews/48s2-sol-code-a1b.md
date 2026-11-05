
> plan · gpt-6.1-sol
What this change does: removes timed settings and their FakeDevice machinery while retaining untimed configuration, frequency history, and TX packet/header state.

### Failure scenarios checked
- Untimed apply loses delay, coercion, read-back, or refusal.
- A later frequency write changes earlier samples.
- Flow removal breaks receive stop/restart or stale TX headers.
- Flush ACK exclusion or S1 report attribution changes.
- Hardware untimed paths disappear with timed paths.
- Deleted tests or mutation rows leave retained behaviour untested.

### Findings

**P2 — Restore P4 C’s FIFO-independent reopen check**  
`crates/ezsdr-radio-uhd/tests/fake.rs:7276`; retained branch at `crates/ezsdr-radio-uhd/tests/common/d9.rs:225–229`.

The deleted P4 test checked two different things: FIFO consumption **and whether case C actually closes and reopens the streamer**. Only the first became obsolete. Deleting P1224 also removes the mutation check for the retained reopen branch.

Minimal counterexample:
```text
p4_dsp_only(DspOnlyReopen)
→ accidentally skip rx_close/rx_open
→ execute case A’s sequence under case C’s label
→ automated tests still pass
```
The retained hardware probe is explicitly output-only (`tests/hardware.rs:1006–1008`), so it is not a replacement assertion.

**Fix:** retain a small sequence test using an untimed/no-op tune closure; assert the close/reopen calls for C and their absence for A. Retarget P1224 to it. No FIFO simulation is needed.

### Other checks
- Production `apply`/`rx_stop` callers previously passed `None`; untimed UHD behaviour remains intact.
- N1/N2/N10/N11 and retained refusal, packet-state, flush and attribution tests cover the other examined cases.
- Bench-only `tune_dsp`/`tune_auto` retain `at`.
- A4 and S2b remain deferred; the UR-22 ordering sentence can move with A4.
- Independently checked **1,709 mutation rows**: every anchor matches exactly once. `git diff --check` passes.
- No production runtime regression identified.

**Verdict: CODE NEEDS CHANGES** — restore the retained P4 sequence check.

Not checked: no builds, tests, mutations, or hardware runs performed; runtime results were inspected from supplied logs. No files changed.
