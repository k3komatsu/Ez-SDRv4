# Note 51 stage 2a — Sol's test-plan review (2026-11-12, openai/gpt-6.1-sol, read-only)

## Verdict: FAIL

The wrapper approach is sound, but the plan needs these corrections before implementation.

References below use `test-plan.md` in `/home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/`.

### Major findings

**M1 — Do not exempt joins before T0. VERIFIED.**  
`test-plan.md:13,44,84,194`

Minimal sequence: connect with RX → finish before T0 → RX exceeds its actual 1 s join bound → streamers remain open → the proposed wrapper accepts it without a marker.

The probe explicitly records both the timeout **and leaked streamers** (`plan.log:5197–5213`). Production code confirms that consequence (`crates/ezsdr-radio-uhd/src/provider/mod.rs:401–406,737–743`). This is not a healthy shutdown misclassified by the wrapper; it matches open **#67**.

**Recommendation:** retain unconditional join classification. For unrelated tests, advance past T0 before finishing where that preserves their subject. Otherwise use an explicit, narrowly documented declaration pending #67—not a global `now >= t0` exemption. T0 also cannot distinguish a genuine TX wedge from the known RX problem.

**M2 — The method-return tests can pass because `Drop` panics later. VERIFIED by control flow.**  
`test-plan.md:155,173`; prototype `probe/run.rs:248–256`

Minimal sequence:

```text
remove check from advance_to
advance_to ends the Run and returns
test reaches scope exit
Drop finishes/judges and panics "ended unexpectedly"
#[should_panic(expected = "ended unexpectedly")] passes
```

Thus the proposed tests do not establish “before returning control,” and rows 06–09 can survive.

**Required change:** catch the panic around the **individual method call**, with the Run owned outside that closure, and assert that the call itself panicked. Later destruction must not satisfy that assertion.

**M3 — The background-bound scenario is missing. VERIFIED path; INFERRED interleaving, not executed.**  
`test-plan.md:36,56,154–158`

The premise “read-only methods cannot end a Run” misses cleanup already happening elsewhere. The data thread executes `StopProviders` through `stop_early` (`crates/ezsdr-kernel/src/coordinator/paced.rs:133–148`), which can invoke the Provider’s bounded joins.

Minimal sequence: Policy requests termination → background `stop_early` hits a join timeout → test calls only `events()`/`state()` → an assertion fails before a settling call → unwind delegates to raw `RunHandle::Drop`, discarding the Manifest. The failure can remain unmarked.

**Required change:** add this adversarial scenario and resolve the observation boundary. If the public API cannot expose that bound at a read-only return without Kernel changes, explicitly narrow the guarantee; do not claim every return is protected.

### Minor findings

- **Scan specification conflicts with its own constructors.** `test-plan.md:119–122` forbids `connect(` outside `run.rs`, but planned callers use `Run::connect(...)`. Add self-tests distinguishing allowed wrapper calls from raw calls, including `use coordinator::connect as open; open(...)`. Likewise test whitespace and multiline forms.
- **Fixture setup still depends on scheduling.** `test-plan.md:139–140`; `probe/fixtures-4-7.rs:71–74,89–92`. Waiting 700 ms does not prove `ref_locked` has entered its stall. Use an observed-entry handshake with an infrastructure-marked setup bound. Keep fixture 8; it provides missing Cargo-level join coverage.
- **Protocol injection order disagrees.** The sketch injects **before** `server.handle`, while the text says **after** (`test-plan.md:111–113`). Choose after-return consistently and test it.

### Open decisions

- **Join before T0:** product defect, consistent with #67—not grounds to redefine what counts as a bound.
- **`Expect::Any`:** acceptable only for individually named, observation-only bench loops such as `hw_perf_rx_rates` and B8 leads. It must still reject every bound and preserve termination reporting. Do not propagate it through shared helpers indiscriminately. Add FakeDevice checks showing a non-deadline end is reported and the loop continues, whereas a bound aborts. B9 should retain its named DEVICE_LOST expectation.

The single shared wrapper, non-unwinding Drop check, CLOCK_LOST fixture 7, and atomic sub-step 2 are otherwise reasonable. J317/J318 becoming declared bound rows is consistent with their existing killers.

No files changed; no builds or mutations run. Conclusions use source inspection and the supplied probe records; background interleaving and hardware behavior remain unexecuted.
