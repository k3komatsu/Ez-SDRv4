> plan · gpt-6.1-sol
The main fixes are present, but the test plan still conflicts with I15 in case 5: it blocks the next segment on leftover debt, although that segment may be needed to consume it. I also found gaps in the loss-lock test and the receive-recomputation test.
**Draft 4 closes most findings, but R4 remains inconsistent in the test plan.** One major and three smaller corrections remain.

References: **EP** = `45-experiment.md`; **TP** = `test-plan.md`, in the supplied directory.

### Closure

| Item | Assessment |
|---|---|
| **R1 — horizon/deadlines** | Closed in both: real-time epilogue, no early timers, and failing `Incomplete` verdict with regression tests. |
| **R2 — command accounting** | Closed at plan level: call results, `Dropped`, and executor-written `Abandoned` are distinguished. |
| **R3 — deadline progress** | Closed: I16, u04d, E1-59 and W5c observe drain-ended while pinned, independently of native stop completion. |
| **R4 — restore eligibility** | EP’s I15 is correct; TP contains conflicting eligibility rules below. |
| **R5 — loss publication** | Release-before-publication and nonrecursive inline publication are defined. The test incorrectly strengthens the lock restriction below. |
| **Conditional `B.op`** | Closed in both. |
| **Recv identity on recomputation** | Contract closed; the proposed killing test does not distinguish the mutant. |
| **Finite monitor / checks before pruning** | Pre-pruning checks closed; TP’s monitor summary still omits EP’s required identity fields. |
| **Tick boundary** | Closed: first tick produces sample 0. |

### Must fix

**1. Case 5 blocks the operation needed to discharge `left` — major**  
**Evidence:** TP:249–258,451; EP:107,119; note 44 §3.4.

TP:451 applies the debt/`left` prohibition to **both restore and next-segment start**. I15 prohibits only restore.

**Minimal sequence:** deadline 20 ends draining while S holds `L` → at 40 S enqueues v and finishes → R stops before the next sample consumes v → finish reports v as `left` → next-segment start is withheld because `left` remains.

**Observable wrong outcome:** no subsequent sample can discharge v; case 5 reaches `Incomplete` instead of exercising W5d. Conversely, u09a currently requires restore immediately after `Done{Call(Ok)}`, even though that completion can leave consumption debt, contradicting u09e.

**Fix:** separate the conditions:
- Next segment waits for pin clearance and the applicable finish/quiesce conditions, **not empty `left`**.
- Restore additionally waits for channel-wide debt/`left` clearance and the required device state.
- Amend U9’s shared expectation accordingly; explicitly consume the debt before expecting restore.

### Other required corrections

**2. Loss publication must release the publisher’s `L`, not require globally free `L`.**  
TP:380 and E1-56 require `native == None` at `Loss.gate`.

Legal sequence: S releases `L` after loss → another actor acquires `L` → S acquires `G` to publish. The current test excludes this interleaving.

Check that **the publishing actor does not hold `L`**. Preserve case-8 gate contention and inline-publication witnesses.

**3. u07d cannot kill E1-60 as specified.**  
TP:237 changes only the cut; the current epoch remains 1. Replacing the recv epoch with the current epoch therefore changes nothing.

Advance the cell to epoch 2 between snapshot and recomputation, retaining epoch-1 samples and handle. Assert publication identities remain epoch 1 and use its retiring cut. Keep the cut-only test separately if useful.

**4. Carry EP’s complete monitor identities into TP.**  
EP:71 requires value, channel, segment/epoch and matching sample. TP:347 still describes qualifying evidence as a Boolean and application as a tick.

Specify the same identity-bearing summaries in TP’s types, key tests and I4 inputs. Case 9’s restore request and W9c are otherwise appropriate.

No files changed or execution performed. The prohibited holdout directory was not read.

**VERDICT: REVISE.**
