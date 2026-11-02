> plan · gpt-6.1-sol
The plans cover many of the right races, but I found approval blockers: `R.recv` has no separate call-entry event, packet-sized ticks hide mid-packet device changes, lifecycle admission omits the retained `device_gate`, and the consumption oracle rejects valid unpublished drain evidence. Some added past-bug scripts also contain contradictory or unreachable states.

The DFS approach itself is suitable for bounded safety checking; its deadline scheduling and cycle claims need tighter definitions.
## Recommendation: REVISE before implementation

What this change does: E1a builds the receive decision core and device model; E1b explores their interleavings. The direction is sound, but the proposed events and oracles can both hide real failures and reject correct behaviour. **I do not approve the granularity and cases as written.**

References below use **EP** for `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md` and **TP** for the adjacent `test-plan.md`.

### Independent scenarios

Before reading EP or TP, I identified:

- A taken settings command finishes after stopping and overwrites a restored or reassigned channel.
- Stop or epoch replacement invalidates a buffer between snapshot and publication.
- Equal-time writes share one consumption sample, or buffered pre-acceptance samples falsely discharge an obligation.
- Loss/panic leaves queued calls, executor loops or late completions active.
- Queue backpressure holds the native lock through the drain deadline.
- Exploration passes because its events omit a race, its state key merges distinct obligations, or its oracle repeats implementation bookkeeping.

The plans address these themes, but the following gaps remain.

## Major findings

### 1. Packet-sized time and instantaneous receive omit relevant interleavings

**Evidence:** EP:42–59; TP:126–134, 338–343.

**Minimal sequence:** a packet covers samples 8–11; a timed value for `e=8` reaches the device at tick 10; the packet returns after sample 11. The correct consumption point can be sample 10. With `Tick` advancing only by four, the model can place acceptance at 8 or 12, but not 10.

Similarly, case 3 requires **receive begun under epoch 1 → epoch advance → receive returns**. There is no receive-entry event: `R.recv` atomically obtains a packet or returns timeout. `R.snap` occurs afterward and cannot establish which epoch owned a call already in progress.

Other unjustified atomic combinations are:

- `ClockRead`: acquire lock, read, release and enqueue are collapsed.
- `S.result`: cell outcome publication and inbox delivery are collapsed.
- `B.step`: potentially several independently locked cell accesses and command deliveries are collapsed.

**Required change:** split receive entry from return, recording its handle, epoch, requested length, timeout and `one_packet`. Permit device effects between relevant sample instants, either with sample-level events or a justified boundary abstraction. Split clock lock acquisition/read/release, and cell publication from message delivery where another actor can intervene. A whole pure brain calculation may remain atomic; accesses to separate shared resources need an explicit justification.

This is necessary for C2/C3, not merely additional realism.

### 2. Lifecycle admission omits the retained `device_gate`

**Evidence:** EP:27–35, 45–62; note 45:136, 148. Existing implementation: `provider/core.rs:1220–1228,1347–1349`.

**Minimal sequence:** R checks `F=false` → loss is published and `F` becomes true → R acquires `L` → R opens a streamer. I1 permits the pre-loss check window, and I9 does not prohibit an **open** after loss. The proposed model therefore admits something the retained lifecycle-admission gate is supposed to prevent.

The native control lock is not that gate: `mark_lost` does not take it.

**Required change:** model the lifecycle gate and its inner loss recheck for open/close/rate admission, including serialization with loss publication. Add a case that actually attempts lifecycle admission across loss. Distinguish this stronger admission rule from the intentionally retained check-to-call window for ordinary calls.

### 3. The consumption oracle rejects legitimate drain evidence

**Evidence:** EP:I4, case 1; TP:470; `plan/phase8/39-dsp-timed-commands.md:134–135`; `provider/rx.rs:350–362,699–704`.

**Minimal sequence:** a taken value remains pending when Stop cuts publication at 8 → a drain or flush sample at 9 consumes it → that sample is deliberately not published. The production contract permits this evidence to discharge the obligation. I4 instead requires consumption by a **published** sample and would reject the correct result.

G9’s “applied-but-unpublished may be reported” does not resolve this: it makes reporting permissible, but still forbids recognizing successful unpublished draining.

**Required change:** separate:

1. device-side application;
2. qualifying receive evidence, including drain/flush samples;
3. user-visible publication.

Check consumed obligations against qualifying evidence and device history. Check cut/publication independently. Include both successful unpublished draining and device application for which the host never obtains evidence.

### 4. “Settled exactly once” conflicts with later consumption of `left`

**Evidence:** EP:I4/P2/U11; TP:240–242.

**Minimal sequence:** an obligation is reported when its segment finishes → late completion leaves it in `left` → the next segment consumes it. U11a requires one settlement already; U11b then requires `Settled{Consumed}` for the same value. Appending that outcome violates I4; replacing the earlier outcome risks erasing the historical report.

**Required change:** define separate facts for command-result settlement, once-only leftover reporting, and subsequent physical consumption. State exactly which one P2 counts. Rewrite U11 to observe the later consumption without requiring a second terminal settlement.

Without this separation, a plausible implementation cannot satisfy all the planned assertions.

### 5. Deadline checks lack a scheduler contract

**Evidence:** EP:66–72, P1; TP:341, 480.

**Minimal sequence:** drain work is runnable and `L` is free → the scheduler repeatedly selects `Tick` instead of R or the brain → the deadline passes before the drain executes. P1 rejects this even if the protocol is correct: neither lock availability nor a runnable actor guarantees scheduling by an absolute deadline.

The same issue affects environment windows: `Tick` can cross a mandatory input’s latest time without selecting it. The plan must distinguish an illegal environment schedule from a protocol failure.

**Required change:** fix the scheduling assumptions before coding:

- Define how mandatory input windows constrain time advancement.
- Define actor scheduling bounds, or change deadline claims to protocol progress after deadline observation rather than unconditional physical-time completion.
- Define drain termination separately from successful physical stop when the native lock blocks it.
- Store deadline observations needed by terminal checks in the exploration state.

DFS with exact state equality is appropriate for bounded safety. An on-stack revisit correctly detects a cycle, but rejecting **every** cycle is stronger than fair-scheduler progress. Either explicitly adopt that stronger property and exclude benign polling transitions, or specify the relevant fairness/progress condition.

### 6. Added past-bug scripts contain contradictory or unreachable sequences

**Evidence:** TP:418–435; EP:84; note 44 §3.3.

**Case 7a:** it starts with `plan_cut=16`, then injects a “cut-free” LateCommand. Whether a cut exists is lifecycle state, not a label on the injected error. This can exercise the ending path instead of the restart that exposed L1 M1.

**Case 7b:** the first Stop is explicitly `Provider::stop`, followed by a new segment and another value release. Provider termination is not a reusable segment cut. Also, the planned flush is conditioned on a value remaining **pinned**, whereas flush debt can remain after a settings call has finished and its pin has cleared.

**Minimal wrong outcome:** the experiment passes because the restart or failed-flush-stop path never executes, while its assertions only verify the absence of restore/start commands.

**Required change:**

- Make 7a genuinely cut-free.
- Simplify 7b to a reachable successful-stop → outstanding drain debt → flush → failed final stop → restore request sequence. Test the earlier ordinary stop failure separately.
- Distinguish outstanding consumption debt from an unfinished-call pin.
- Add reachability assertions showing each intended failure path actually occurred.

### 7. Several advertised concurrency and past-bug claims remain unexercised

**Evidence:** EP:49, cases 1–7 and seeded configuration; TP:370–398, 450–456.

Case 3 exercises replacement without values. Case 5 exercises pinning without a scripted restore, move, replacement or next-start request. The seeded plan cancels/restarts a stream but does not specify an actual channel transfer. Consequently, several “no conflicting command while pinned” assertions can pass because the conflicting request never exists.

Also, no exhaustive script explicitly selects `S.effect(Partial)` or a non-loss settings refusal. U10 tests one transition, not its interleaving with stopping and reuse.

**Minimal sequence missing from the cases:** take a value → shorten the cut → request channel transfer or replacement → complete with a partial failure → attempt the deferred lifecycle operation. An early unpin, lost leftover or wrong destination can survive the current exhaustive cases.

**Required change:** add bounded scripts combining pending settings work with actual conflicting lifecycle requests, partial completion and refusal. For the past bugs that the architecture makes impossible, provide the structural argument and its admission test rather than claiming an equivalent historical replay.

Keep the existing six required cases; do not substitute these additions for them.

## Minor findings

### 8. Some tests and mutation rows do not observe their claimed behaviour

**Evidence:** TP:179, 275, 314, 523, 535.

- **E1-42** removes `Done{Dropped}` from `dst.rs`, but `u03` manually supplies that completion. The proposed killing test bypasses the mutated producer.
- **E1-30** describes skipping expansion of an already visited state. That is normal graph DFS behaviour. The reconvergence test must require all outgoing edges of each distinct reachable state to be expanded **once**, not re-expanded on every incoming path.
- **U15d** requires `next == None` behind a stopped tail. `FakeDevice::start_rx` sets `then` but preserves the existing tail cursor (`device.rs:1196–1199`). The test contradicts the cited implementation.
- **U12d** is useful as a scratch-adapter check, not independent evidence for the consumption rule: both sides call `RxChain::consume`.

Fix these pairings before code. The other mutation intentions are generally reasonable, but unique anchors, reachability and actual kills cannot yet be approved without implementation.

### 9. The snapshot/writeback protocol needs an explicit merge rule

**Evidence:** EP:55–56; TP:63–70, 108, 140.

Epoch and cut are not the only facts that can change during outside-cell consumption calculation: reservation, take and settings completion also change obligations. Specify whether writeback applies identity-based deltas or validates an obligation revision. It must not overwrite newly inserted obligations or discharge an entry using stale acceptance information.

Add one interleaving test around settings completion between snapshot and publication. Do not add a second independently maintained lifecycle flag merely to solve this.

### 10. Search identity and self-tests need tighter boundaries

**Evidence:** EP:66; TP:315–325, 348.

Exact serialized keys avoid hash-collision concerns. They must also include pending deadlines, receive-entry identity and oracle-monitor facts that affect future verdicts—not just production state.

Conversely, unbounded histories and diagnostic counters can prevent state reconvergence and cycle detection. Add a toy cycle that changes a diagnostic counter, and document which counters are excluded or represented finitely without weakening assertions.

The budget-overrun rule is sound: incomplete exploration must not pass.

### 11. Size estimates are optimistic, not an executable budget argument

**Evidence:** EP stage table; TP:14–18, 346–348.

Roughly a hundred planned test functions, a resumable device, actors, graph search, replay/shrink and independent monitors make 1,500 lines per stage tight. Splitting files does not reduce stage size. The proposed one-microsecond transition and 300-byte exact-key estimates are unmeasured and particularly optimistic with buffered packets, inboxes and histories.

Keep the limits as stop-and-report checkpoints. Measure a representative reduced case before committing to the full search configuration; do not compress code or weaken cases to fit.

## Overall assessment

- **Device model:** head-of-line DSP consumption and FakeDevice’s effective-time rule are appropriate. Host buffering needs specified partial-packet/tail semantics and later E2 conformance. It is not yet established as faithful enough.
- **Oracles:** I2/I5–I9/I11–I13 are useful when exercised. I1 needs lifecycle admission distinguished from ordinary call checks. I3 needs competing requests. I4/P2 need the accounting repair above. I10 is correctly not counted as a runtime oracle, though signatures alone do not prove real lock scope.
- **Maintainability:** deriving pins rather than storing another flag is good. Reusing production consumption is preferable to copying it, provided the scratch adapter preserves identity and acceptance times. Keep protocol state, observation history and device truth separate.
- **C1–C3:** E1 can establish bounded protocol evidence, not latency or native-device fidelity. Those remain E2/E3 obligations.

No files changed; no builds, tests, mutations or hardware operations ran. The prohibited holdout directory was not read.

**VERDICT: REVISE — resolve majors 1–7 and fix the identified test/mutation contradictions before implementation; granularity and cases are not approved.**
