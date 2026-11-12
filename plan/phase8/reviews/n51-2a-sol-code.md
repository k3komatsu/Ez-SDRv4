# Note 51 stage 2a — Sol's code review (2026-11-13, openai/gpt-6.1-sol, read-only)

## Verdict: FAIL

The wrapper addresses the earlier method-return and unwind gaps, but one bound-detection hole remains.

### Major

**1. A KC-21a bound recorded in `termination.also` is invisible.**  
`crates/ezsdr-radio-uhd/tests/common/run.rs:193–196`

The wrapper judges only `termination.reason`. The Kernel preserves later failures in `also` (`crates/ezsdr-kernel/src/coordinator/ending.rs:204–225`).

Minimal sequence:
1. `wait_finished` checks that no end exists.
2. A background Policy requests termination.
3. The waiting thread requests KC-21a after its deadline.
4. Cleanup joins successfully.

KC-21a is now in `also`, while the primary reason remains the declared Policy. The wrapper accepts the Run without an infrastructure marker.

**Evidence:** VERIFIED control flow (`coordinator/paced.rs:186–198`); INFERRED interleaving, not executed.  
**Fix:** inspect primary and secondary terminations for bound reasons before accepting any declaration. Add a test with a declared Policy primary reason and KC-21a in `also`.

### Minor

**2. Marker capture also suppresses unexpected infrastructure failures.**  
`crates/ezsdr-radio-uhd/tests/common/run.rs:91–105`; `crates/ezsdr-radio-uhd/tests/fake.rs:8708–8714`

The Policy-return tests expect **no bound**, yet suppress every wrapper marker. If the call instead meets KC-21a or an incidental join timeout, its marker is captured; the subsequent message assertion fails without emitting that marker. The output consequently looks like an ordinary assertion failure, which the mutation runner treats as a kill rather than infrastructure failure.

**Evidence:** VERIFIED suppression and assertion path; INFERRED scheduling failure.  
**Fix:** do not suppress markers in zero-bound tests. For deliberately bounded tests, capture only the expected bound and emit unrelated ones normally.

**3. The scan misses ordinary function-value aliases.**  
`crates/ezsdr-acceptance/tests/governance.rs:510–520`

This bypass produces no violation:

```rust
let create = ezsdr_kernel::coordinator::connect;
let run = create(&profile, &initial, assembly, lease).unwrap();
```

`connect` is not followed by `(` or `as`, and the inferred handle needs no `RunHandle` token. A new test can therefore create an unchecked Run despite the stated construction guarantee.

**Evidence:** VERIFIED scanner control flow; no existing bypass claimed.  
**Fix:** reject raw constructor references, not only calls, outside the wrapper; add this self-test.

The shared delay reader avoids duplication. The scan’s size alone is not a finding.

**Not checked:** no builds, probes, hardware tests, or mutation runs executed. Repository unchanged.
