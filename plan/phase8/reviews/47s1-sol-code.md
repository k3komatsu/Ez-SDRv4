# NEEDS CHANGES

Reviewed the uncommitted diff and `crates/ezsdr-server/src/initial.rs` against HEAD `b3d39ab`. No repository files changed. Plan mode prevents saving the requested cache report.

## Independent failure scenarios

Before comparing with the test plan, I checked:
- Mixed names and explicit numbers reserving the wrong stream.
- Omitted channels/directions retaining unintended membership.
- Invalid requests reaching Run creation.
- Initial settings being dropped or applied after streamer opening.
- Requests targeting the wrong radio or declaring the wrong Vocabulary.
- Empty defaults leaving sample consumers silently idle.

## Findings

### 1. P1 — Major: initial requests can declare the wrong Vocabulary

**Location:** `crates/ezsdr-kernel/src/module_api/registry.rs:191–193`; `crates/ezsdr-kernel/src/session.rs:508–513`.

`vocabulary_of` returns the first matching **prefix**, not the Vocabulary whose declaration owns the key. Registration permits overlapping prefixes when their key declarations do not overlap; existing coverage explicitly permits this (`crates/ezsdr-kernel/tests/module_api.rs:364–384`).

**Minimal sequence:**
```text
Register test, prefix test, major 1, declaring test.count.
Register test.deep, prefix test.deep, major 2, declaring test.deep.count.
Connect with initial {resource: {test.deep.count: 2}}.
```
Both registrations are valid. Lookup selects `test`, so the implicit Spec declares `test@1`, not `test.deep@2`. Prefix validation can pass through `test`, while `key_decl` resolves the nested declaration. The recorded identity therefore omits the actual owner’s major, violating RS-12/SB-11.

**Fix:** Resolve ownership through the uniquely matching `KeyDecl`, reusing the lookup decision already used by `key_decl`. Longest-prefix matching alone would not handle different Vocabularies sharing one prefix. Add nested-prefix and shared-prefix regression cases.

### 2. P2 — Minor: an explicitly empty radio name silently targets `"radio"`

**Location:** `python/ezsdr/session.py:820–822`.

**Minimal sequence:** `connect(profile, radio="", tx={0: [0]})`, where the profile contains resource `"radio"`.

Instead of refusing the nonexistent resource name, `radio or "radio"` assigns its transmitter. `False` and `0` also fall back silently. A connection-only probe confirmed the emitted request targets `"radio"`.

**Fix:** Apply the default only when `radio is None`; reject invalid names/types before starting `Connection`. Test that an explicit empty name never configures the default resource.

### 3. P2 — Minor: raw-wire value aliases silently overwrite settings

**Location:** `crates/ezsdr-server/src/initial.rs:47–53,127–129`.

**Minimal sequence:**
```json
{"number":0,"channels":[0],
 "frequency":{"0":1000000000,"00":1000500000}}
```
`member` parses both keys as channel 0. Both become the same concrete key, and the later insertion silently replaces the first frequency. The accepted Spec records only the replacement.

**Fix:** Require canonical numeric value keys (`text == number.to_string()`), or reject duplicate resolved channels within each value map. Add raw-server cases for `"0"` alongside `"00"` and `"+0"`.

### 4. P2 — Minor: Python and Rust disagree about invalid stream names

**Location:** `python/ezsdr/session.py:743–746`; `crates/ezsdr-server/src/initial.rs:75`.

**Minimal sequence:** Give a stream the name `"１２"`.

Python rejects it as all digits; the raw server accepts it because Rust checks only ASCII digit bytes. Thus the duplicated validation implements different versions of EA-10a’s rule. Some whitespace characters also differ between Python’s and Rust’s predicates.

**Fix:** Specify one character policy and implement matching predicates in both languages. Add shared non-ASCII digit and whitespace cases, rather than testing only `"3"` and `"a b"`.

## Other review results

- The revised no-membership default is implemented in both `defaults()` and `Present` coercion. The main request path inserts `Eq` requirements before hashing and validation.
- Named-stream allocation reserves explicit numbers before assigning names. Resource targeting and logical sample ordering are covered.
- The final plan addresses the earlier review findings. The revised default, initial configuration ordering, RM-31/RM-32, RF bands and `tx_enabled` have meaningful observations.
- P1319–P1352 and re-anchored P49/P137 were inspected; their intended killers are credible by source inspection. Mutation execution was not repeated.
- Removing P75 and its throughput case is justified: with default membership empty, a throughput violation requires requested membership, providing an attributable request key.
- The Spec Run entry and runtime `.stream` setter remain present. Migrated fixtures/examples request membership explicitly.
- The named spec amendments are present. Finding 1 makes the new RS-12 ownership guarantee false.
- Pre-Run expansion refusals create no Manifest. An empty Session directory still exists because `lib.rs:195–198` allocates it first; that behavior is pre-existing.

## Open-time zero flush

**VERIFIED, source:** The two-zero untimed SOB/EOB send predates this change: HEAD already contains it at `crates/ezsdr-radio-uhd/src/provider/core.rs:1458–1468`. Streamer opening invokes it (`:1571–1573`); arm opens assigned TX streams (`provider/mod.rs:562–566`).

Stage 1 changes **when it happens for Sessions using the new initial TX path**: during `connect`, rather than during the later runtime membership assignment. An omitted/empty TX direction opens nothing and sends no flush. Spec Runs with initial TX membership already had this behavior.

Actual analog radiation or LO leakage is **INFERRED**, not established by the FakeDevice trace.

**Not checked:** Builds, full suites, mutation kills or hardware; only a small mocked Python probe and `git diff --check` were executed.
