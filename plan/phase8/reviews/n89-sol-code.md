My failure scenarios, before reading the plans or prior reviews:

- **Kernel:** a signal upgrades its Weak reference while the owner drops the Run; disconnect arrives during dispatch or cleanup; a device-paced wait misses the wake; or normal finish and disconnect overwrite each other’s termination.
- **Server watch:** firing blocks removal while the last Run reference drops on the input thread; unwind drops fields in the wrong order; late registration misses an already-ended input; or an old Session/child signal survives replacement.
- **Transport:** EOF or read failure sits behind queued data; `Interrupted` is treated as departure; a queued request runs after departure; or a reply failure exits without writing the Manifest. Also check whether unbounded buffering introduces a resource-exhaustion path.
- **RF shutdown:** departure during an advance, event wait, capture/result wait, or active repeat requests termination but never reaches Provider cleanup/EOB. Detached refusal must precede directory creation.
- **Python:** Ctrl-C reaches the server despite isolation; an exception during write/header/body reading leaves either pipe open; later calls reuse a desynchronised connection; or `close()`/failed `connect()` waits or reads the Manifest in the wrong order.
- **Bench:** an early exit strands the transmitter; emergency cleanup is mistaken for natural shutdown; E9 reports a duration without a confirmed death/tone observation; receiver errors become “quiet”; or the relay changes framing/lifetime.
- **Contracts/governance:** text promises synchronous shutdown when only termination is requested, or the input-thread exception weakens unrelated governance checks.

I’ll now trace these against the connected code, then compare them with the supplied plans and mutation tests.
The Kernel and server shutdown paths look consistent with the chosen design: the signal requests termination, wakes device-paced waits, and leaves cleanup on the control thread; watch removal protects the Run’s final drop.

I found two suspect E9 paths in the bench script: emergency-stop failure is not checked before reporting a death-based measurement, and an early tone followed by unreadable receiver samples can be reported as “still on 60 s after.” I’m checking both with device-free probes.
## Verdict: FAIL

The disconnect fix looks sound by code inspection across Kernel, server, and Python. Two **major bench-evidence defects** remain; they can mislead the E9/A1 safety decision.

### 1. Major — E9 can measure “after death” while the server is still alive
**VERIFIED — device-free probe.**

- **Sequence:** old server remains transmitting → emergency `docker stop` fails → container still runs → E9 polling starts anyway.
- **Wrong outcome:** the script prints a measurement counted from a supposed server death that never occurred.
- **Evidence:** `plan/phase8/tools/ssh-drop-tx.sh:428–434` ignores the stop failure. `stopped_ns():264–271` substitutes “now” even when inspection says `running=true`.
- **Fix:** require confirmed container termination before either E9 path; otherwise print `INCONCLUSIVE`, retain the live-server warning, and do not describe subsequent readings as post-death measurements.

### 2. Major — An early tone becomes “still on 60 s after”
**VERIFIED — deterministic, device-free probe.**

- **Sequence:** confirmed server death → receiver reports tone at +1 s → every subsequent reading is unavailable through +60 s.
- **Wrong outcome:** E9 prints **“was still on 60 s after”**, despite having no observation after +1 s.
- **Evidence:** `plan/phase8/tools/ssh-drop-tx.sh:237–254`; any nonempty `last_tone` selects that conclusion.
- **Fix:** report the last confirmed tone time and classify the unreadable tail as `INCONCLUSIVE`. Add device-free tests for this and finding 1.

These defects do **not** refute the owner’s decisions, but current E9 output cannot safely support the deferred A1 decision. The corresponding measurement guarantees in `plan/phase8/bench.md:1166–1168,1182,1209` are consequently too strong.

### 3. Minor — A failed stdin read can still exit 2
**VERIFIED code path; execution INFERRED.**

- **Sequence:** connected Session → partial next header/body arrives → stdin returns EIO → stdout remains writable.
- The forwarding thread turns the error into EOF (`transport.rs:55–59`). Framing then returns `BadFrame` (`transport.rs:91–95,121–125`), mapped to exit 2 (`main.rs:32`).
- Cleanup still writes the Manifest, but this contradicts the unconditional failed-read → exit-0 promise in `design/16-easy-api.md:83`.
- Preserve the terminal read-error cause and test truncation followed by a read failure, not only failure during an advance.

### Other review results

- No additional major found in Weak ownership, watch locking/drop order, late registration, first-end-wins, Provider shutdown, Detached refusal, or Python abandon/close paths.
- Compared the briefs, owner decisions, amended test plan and dispositions. The missing scenarios above are bench failure paths and truncated-input read errors.
- All **38 C89 mutation killers** resolve; their paths and re-anchored P14 were inspected. No additional maintainability finding.
- Passed: five Python close tests, relay/driver self-checks, extractor check, and shell syntax check.

**Not checked:** Rust suites, mutation execution, or real RF. No repository files changed; E9 remains unmeasured on hardware.
