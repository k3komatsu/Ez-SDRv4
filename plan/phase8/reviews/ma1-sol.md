# 段階 MA1：Sol のレビュー

## 計画レビュー 1

> plan · gpt-6.1-sol
## Independent scenarios

Before reading the proposed sequences, I identified these failure cases:

1. RX updates before T0, during Stop/refusal, or between segments incorrectly establish coherence.
2. A covering segment is missed because it belongs to another stream, or coverage uses the wrong channel or inclusive end.
3. UHD decides at release, then a Stop/change/refusal alters the plan before Mock decides at application.
4. An inactive update and a later segment origin consume separate draws; coherence alone cannot establish their number or order.
5. Mixed channels and same-instant events corrupt per-channel phase, aggregate coherence, or reproducibility.
6. TX idle LO timing is incorrectly inferred from the separate DSP command path.

## Comparison

The plan covers most ordinary cases well. The shared half-open coverage predicate, phase-sample observations, boundary table, and preservation of UHD behavior are appropriate. However, it leaves an actual caller mismatch and several claimed assertions unpinned.

### Major 1 — Linkless streams can acquire planned coverage

**Evidence:** `crates/ezsdr-mock-radio/src/lib.rs:2616–2620` initially stops a linkless line, but `start_rx` subsequently books `Kind::Start` on it (`:1586–1596`). The planner can then create a segment. Mock nevertheless never instantiates that receive segment (`:1315–1318`) and skips its origin tune (`:1030`).

D1 passes **all** RX lines to `running_at`. Consequently:

> Linkless stream → `start_rx` → wait beyond its planned origin → frequency update

can be classified as a timed tune although nothing receives. Test 13 only tests the initially stopped line and misses this sequence.

**Required plan change:** Explicitly define caller eligibility: Mock must exclude linkless receive streams from coverage without changing their membership/move bookkeeping. Extend test 13 with `start_rx`, advance beyond the planned origin, and then update. Observe the draw through a later linked-channel sample; also assert that the linkless stream emits no block, clock, or segment row.

### Major 2 — Decision-time disagreement needs a settled contract and regression

Plan §5.1 correctly notices release versus application, but leaves the consequence as an optional Ceiling.

The disagreement is real: UHD calls `reach` during release (`crates/ezsdr-radio-uhd/src/provider/control.rs:647–669,738–748`), while Mock would inspect its current plans at application (`crates/ezsdr-mock-radio/src/lib.rs:2088–2093`).

For example:

> UHD releases a tune for a covering segment → Stop arrives before e → Mock applies the update after the cut.

Sharing the arithmetic does **not** make those decisions identical. This is acceptable as conservative modeling only if explicitly chosen; it is not exact issuance parity.

**Required plan change:** Adopt application-time classification as the Mock contract, document that later plan changes can make it more conservative than UHD’s earlier release decision, and add a deterministic regression for this sequence. Observe UHD’s already-issued timed call and Mock’s additional untimed draw through recovery samples. Do not assert the hardware FIFO’s eventual outcome: D9’s stop-after-issuance behavior still has bench uncertainty.

Also distinguish a refusal already applied from one merely scheduled for the same instant; specify arrival order in that test.

### Major 3 — Several draw/order assertions have no effective observer

- **Test 2:** “retune[1] is not used by anyone” cannot prove that the covered update consumed no draw. A discarded draw passes the listed sample/coherence assertions.
- **Test 7 / mutation 11:** Its updates precede the **first** RX origin. First segments take no origin tune (`lib.rs:1033–1034`), so this scenario cannot detect update-versus-origin draw ordering.
- **Test 10:** It asserts an inactive update consumes a draw but supplies no recovery or sentinel observation. There are no received samples during the halt; unchanged invalid coherence alone is insufficient.

**Required plan change:**

1. After test 2’s covered update, trigger a known untimed tune and assert its exact next RNG phase.
2. Assign mutation 11 to an actual non-first-origin tie test. The existing `mr_34_a_layout_untimed_tune_follows_a_timed_tune_at_its_instant` is the relevant starting point.
3. Complete test 10 with recovery and an exact expected draw index; assert no refused RX segment row survives.

### D1 — Shared predicate and refusal filtering

**Recommendation: accept the free function.** Keep UHD’s destination/configuration machinery private.

The RX planner explicitly excludes refused cold-change segments (`crates/ezsdr-radio/src/timeline.rs:953–964`); refused starts also create none (`:945–950`). UHD derives its `Planned` entries from that same `line.plan` (`crates/ezsdr-radio-uhd/src/provider/core.rs:582–590`).

Thus, caller-side refusal filtering is not currently two independent RX refusal implementations. Document the iterator’s eligibility precondition and test the adapters. No new public wrapper type or optional `Segment::runs()` abstraction is needed for this extraction.

### D2 — Two draws

**Accept two draws as an explicit simulation rule; reject copying UHD’s destination machinery.**

Correct the rationale at plan line 43: for the selected destination, D9a normally chooses **either** deferred configure **or** untimed release because configure already happened—not both (`control.rs:752–760`; existing tests `:6003–6032,6146–6177`).

Mock’s update draw plus origin draw is a deterministic modeling convention, not a hardware retune count. Tests 3/4 appropriately pin it through the final phase.

### D3 — TX outside bursts

**Keep TX unchanged. Source verification supports this for the LO path.**

In UHD 4.10:

- [`x300_radio_control.cpp:146–148`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L146-L148) sends SPI register writes with the command time.
- [`bus_int.v:541–556,918–940`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/top/x300/bus_int.v#L918-L940) uses the continuously advancing radio timekeeper for timed daughterboard settings.
- [`ctrlport_to_settings_bus.v`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/utils/ctrlport_to_settings_bus.v) waits for `timestamp >= set_time_reg`, not TX sample traffic.
- UBX `set_tx_freq`/`sync_phase` and OBX [`obx_expert.cpp:132–141,210–211`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/obx/obx_expert.cpp#L132-L141) enable timed LO synchronization independently of bursts.

**VERIFIED:** no TX sample-flow requirement on this timed control path. This does not establish correct DUC behavior or instantaneous phase settling at e; those remain outside MA1.

### Minor corrections

1. **MR-27 amendment:** “a timed update covered by RM-xx” must be RX-specific; otherwise it contradicts retained TX behavior.
2. **Mixed coherence:** coherence belongs to a whole segment. A deterministic channel sharing a segment with CBX does not make that segment `deterministic/valid`. Give test 6 separate streams or assert the aggregate result.
3. **Pre-T0 test:** remove §5.2’s proposed replacement by a stopped-stream test. The harness supports a future T0 and pre-T0 updates (`mock_layout.rs:295–302`); preserve the first-segment regression.
4. **VH-8:** leaving it unchanged is justified: `crates/ezsdr-mock-radio/tests/timeline.rs:672–673` generates timed **gain**, not phase-sensitive frequency updates.
5. **Red phase:** deterministic/TX/no-change regressions should already pass HEAD; do not require every listed test to fail before implementation.

## Verdict — FAIL

Amend the plan before implementation:

- Exclude nonreceiving linkless plans and test `start_rx` on a linkless stream.
- Settle and test release-time versus application-time semantics.
- Add observable RNG sentinels and correct mutation-to-test assignments.
- Correct D2’s hardware rationale and the minor specification/assertion issues above.

No files changed; no agents, builds, tests, mutations, Docker, or hardware were run. Hardware phase settling and post-release Stop/FIFO behavior were not bench-verified.

## 計画レビュー 2

> plan · gpt-6.1-sol
VERDICT: FAIL

## Findings

### Major 1 — The acknowledged flip-flop cannot be accepted as a Ceiling

Plan v2 explicitly allows receipt coverage → no coverage → application coverage to produce Mock coherence even when UHD released untimed (§0 D2; §3 MR-34). This retains the forbidden direction of MA-1. Calling it a Ceiling does not resolve it.

**Required change:** Replace `covered_at_receipt` with a sticky eligibility flag: initialize from coverage at receipt; clear whenever a subsequent relevant plan transition leaves the update’s channel uncovered at its effective instant; never restore it. Recheck at application. Add a flip-flop regression and a mutation removing the intermediate invalidation. Assert the final sample phase and invalid coherence, not merely the flag.

The scope of the resulting guarantee must be stated precisely; see the recommendation below.

### Major 2 — The proposed reverse-direction example ignores segment-start lead

The argument that a cold change’s **effective instant** can precede `e` is insufficient: coverage depends on its **segment origin**.

`crates/ezsdr-radio/src/timeline.rs:1022–1040` floors a new origin at `ready + lead`, and, after an RX cut, includes the receive-call duration and allowance. The X3xx profile has a **3 ms release window** but a **50 ms RX start lead** (`crates/ezsdr-radio/src/x3xx.rs:398–400`).

Consequently, in the cited ordinary Stop → release → newly booked `start_rx` sequence, a booking after release cannot create coverage before `e`: its origin is already at least 50 ms after receipt, whereas release happens within 3 ms of `e`.

**Required change:**

- Withdraw the assertion that `timeline.rs:398–419` alone proves this reverse case.
- Distinguish requested/effective instant from actual segment origin throughout tests 10–15.
- Test 15 can demonstrate conservative receipt-time classification if restart is booked sufficiently early, but it then does **not** establish the claimed post-release hardware divergence.
- Make 15b a required evidence test if this divergence remains the justification. Supply a reachable sequence, actual timing terms, and an observed untimed call—not “untimed／destination” as interchangeable outcomes. `Reach::Deferred` makes no apply call (`control.rs:788–793`).

A refusal that replans already-booked items deserves separate investigation; it is not the newly booked restart example.

### Major 3 — Tests 10b/10c do not establish the claimed same-instant coverage

For an ordinary RX cold change at `e_c`, its successor segment does not begin at `e_c`: the previous cut plus call, allowance and lead delays its origin (`timeline.rs:1029–1038`).

Thus, setting update `e == e_c` does not make S2 cover the update before refusal. The predecessor also ends at that cut. Test 10b’s expected timed result is therefore unsupported. If the update arrives first and coverage depends on a segment booked afterward, v2’s receipt flag independently prevents timed classification.

**Required change:** Separate two concerns:

1. Test update-versus-refusal ordering at a cold effective instant with coverage expectations derived from actual origins.
2. Test refusal removing coverage using an update at or after the successor’s actual planned origin. Apply refusal first, then observe the update through a recovery sentinel.

For 10a, specify recovery booking timing so recovery does not itself restore coverage before the update.

### Minor — MR-34’s proposed coherence sentence overstates `random`

The proposed text says `random_unless_timed_tune` **or `random`** remains incoherent “until its next timed tune.” A `random` channel remains incoherent after that tune too (`crates/ezsdr-mock-radio/src/lib.rs:2170–2177`).

**Required change:** Limit restoration by timed tune to `random_unless_timed_tune`; explicitly retain CBX’s always-invalid coherence.

## Round-1 closure

- **Major 1, linkless eligibility:** fixed in the proposed adapter and test 13. The linked-channel sentinel distinguishes an incorrectly omitted draw.
- **Major 2, decision time:** test 14 and 14b appropriately pin conservative application-time behavior, without asserting FIFO outcomes. However, the added reverse-direction reasoning and flip-flop exception prevent closure.
- **Major 3, observers:** test 2 now has a useful sentinel; mutation 11 is correctly assigned to the existing non-first-origin tie test. Test 10’s observer is improved, but its timing scenarios need the correction above.
- **All five previous minors:** addressed: RX-specific MR-27, separate coherence streams, retained pre-T0 case, unchanged gain-only VH-8, and regression tests exempted from mandatory red.

## Recommendation on continuous coverage

**Prefer the sticky continuous-coverage flag over v2’s endpoint checks.** It closes the endpoint-history hole with little Mock machinery.

Hook rechecks after each successful RX booking (`lib.rs:1345–1349`), atomic RX move publication (`:1988–1992`), and refusal (`:2095`). Scan all eligible RX lines after the complete mutation: `Lines::settle` can change dependent lines (`timeline.rs:497–518`). Do not check transient internal states during an atomic move.

However, **the unconditional cross-provider proof is not established**. UHD’s `Planned` really is derived from its own `line.plan` (`core.rs:582–590`), but Mock and UHD do not share a mutable plan or necessarily the same transition timing. UHD books configuration refusals when its owner encounters them (`rx.rs:306–321`); Mock does so at effective application (`lib.rs:2088–2095`). UHD’s from-none readiness also includes configuration completion (`control.rs:1038–1092`).

Therefore, continuous coverage proves the implication **for a common observed plan-transition sequence**, not automatically for every hardware execution. Amend the plan to test these ordering differences and avoid claiming universal parity from shared planner code alone. Reject the explicitly looser Ceiling.

No files changed; no agents, builds, tests, mutations, Docker, or hardware were run. Cross-provider refusal ordering and hardware FIFO outcomes remain unverified.

## 計画レビュー 3

> plan · gpt-6.1-sol
VERDICT: FAIL

## Major 1 — Test 17’s refusal is unreachable as specified

Test 17 books C2 with `e_c2 ≤ S2.origin`. C2 therefore cuts S2 at sample zero, and the RX planner omits S2 (`crates/ezsdr-radio/src/timeline.rs:989–1003`). Mock’s refusal fault explicitly skips a cold change whose segment is absent (`crates/ezsdr-mock-radio/src/lib.rs:2124–2133`). C1 consequently cannot receive the intended refusal.

**Required plan change:** Set `e_c2 > S2.origin`, leaving S2 at least one sample. Assert that C1 is the rejected command, S2 existed before refusal, and S3’s origin advances afterward. Read the origins in calibration runs, then run the update-bearing scenario separately. Do not allow implementation to silently drop this regression merely because the current construction fails.

This revised construction can exercise the intended mechanism: without refusal, C2 cuts S2 and its successor waits that cut plus call/allowance/lead; after refusal, C2 instead follows S1’s earlier cut (`timeline.rs:953–964,1022–1040`).

## Major 2 — Several mutation rows do not distinguish the claimed wrong implementation

- **Row 15:** Removing refusal-time invalidation does not fail 10a: application-time `rx_covers` still observes no coverage. Test 17 also cannot kill it: its eligibility starts false and never needs clearing.
- **Row 18:** Removing the application-time check does not fail 14 or 10a if booking/refusal hooks already cleared the sticky flag.
- **Rows 16–17:** Their distinguishing scenarios remain conditional implementation-time work, rather than reviewed test sequences.

**Required plan change:** Give refusal invalidation a genuine **covered → uncovered by refusal → covered again before application** regression, observing the final phase and invalid coherence. Specify the atomic-move and dependent-line scenarios before implementation, or establish why the proposed mutation is behaviorally equivalent. For the redundant application guard, use an explicitly justified combined mutant or document equivalence; do not claim that 14/10a kill its deletion alone.

Test 16 does correctly distinguish missing booking-time invalidation: its restart can begin well before `e = T0 + 200 ms`, while a final-only implementation would incorrectly restore timed classification.

## Major 3 — Refusal-order uncertainty cannot be discharged as a timing disclaimer

### Answer to point 2

The orchestrator’s premise needs correction: **a refused cold change still cuts its predecessor**. `Kind::Cold` calls `cut` before inspecting refusal (`crates/ezsdr-radio/src/timeline.rs:953–964`). A refused start does not create a segment (`:945–950`).

For an isolated cold refusal, removing S2’s coverage matters only at or after S2’s origin. Mock normally applies that refusal earlier, at the cold effective instant (`crates/ezsdr-mock-radio/src/lib.rs:2088–2095`). This supports the proposed protection against **UHD-earlier** bookkeeping in the ordinary case; it does not prove the general “only a same-instant tie” conclusion.

The opposite ordering remains relevant:

1. Book C1 and C2, with `e_c2 > o2`, so S2 survives.
2. Mock refuses C1 at `e_c1`, advancing S3 from `o3` to `o3'`.
3. Receive the update **after Mock’s refusal**, choosing `o3' ≤ e < o3`. Mock initializes eligibility true.
4. UHD has not yet processed C1’s failed configuration when control releases the update.

UHD’s old plan does not cover `e`; Mock’s post-refusal plan does, continuously from receipt onward. Sticky eligibility cannot detect a plan that predates receipt.

**VERIFIED:** The owner has no fixed “origin minus configure lead” scheduling rule here: it processes the next segment when its loop reaches `begin` (`crates/ezsdr-radio-uhd/src/provider/rx.rs:227–238,286–321`). Release independently evaluates coverage (`crates/ezsdr-radio-uhd/src/provider/control.rs:738–760`). Therefore the plan’s asserted few-millisecond window and “cannot test deterministically” reasoning are unsupported.

**INFERRED pending regression:** The interleaving above defeats the unconditional coverage implication. An unconfigured destination yields **Deferred**, not an observed untimed apply (`control.rs:788–792`); the test must distinguish those outcomes.

**Required plan change:** Add a manual-clock, explicitly ordered UHD/Mock regression using the corrected refusal construction. Existing tests already drive release and configuration separately (`control.rs:6003–6032`). Either conservatively withhold Mock coherence for refusal-advanced coverage, or prove that a precisely identified existing scheduling Ceiling excludes the sequence. A potentially stronger Mock guarantee is not itself an acceptable Ceiling.

## Round-2 closure and minor correction

- **Ordinary flip-flop:** fixed by sticky eligibility and test 16.
- **Start-lead reasoning:** corrected; tests 14–16 now allow reachable restart origins.
- **10a:** recovery timing and phase sentinel are improved and appropriate.
- **10b:** the off-lattice predecessor coverage is sound: refusal does not undo its cut. Remove the fallback expectation “refusal first ⇒ untimed”; that is not supported by the planner. Use coherence and the sentinel as the decisive observers.
- **CBX wording:** fixed; `random` remains invalid after timed tunes.

No files changed; no agents, builds, tests, mutations, Docker, or hardware were run. Cross-provider refusal ordering remains the blocking unverified behavior.

## 計画レビュー 4

> plan · gpt-6.1-sol
VERDICT: FAIL

## Major 1 — “Begun in Mock ⇒ begun in UHD” is still unproved, and lateness breaks the argument

Plan D2.2–3 treats `origin ≤ r` as evidence that UHD has processed the preceding configuration refusal. It establishes only that **Mock’s planned origin has passed**.

A concrete countersequence extends 17b:

1. Book C1 and C2, retaining S2 for at least one sample.
2. Mock refuses C1 and advances S3 to `o3'`.
3. Receive the update with `o3' ≤ r < e < o3`, leaving the required command lead.
4. Delay UHD’s RX owner before it processes C1’s configuration failure; let control release the update first.

Mock sees the same S3 at receipt and application and declares coherence. UHD still sees the old gap between S2’s cut and `o3`; `reach` returns Deferred for an unconfigured destination, not Timed.

**VERIFIED:** RX configuration/refusal progresses through owner turns, not through device time alone (`crates/ezsdr-radio-uhd/src/provider/rx.rs:227–238,286–321`). Release independently reads the current plan (`control.rs:738–760,788–792`). Late starts are recoverable: UHD retains the origin and schedules another start (`rx.rs:447–460`). “No samples at the planned origin” therefore does not exclude the execution or establish eventual phase coherence.

**Required plan change:** Restore a required manual-clock regression for this ordering, distinguishing Deferred, untimed apply and timed apply. Withdraw the unconditional implication. Either conservatively withhold coherence for the unresolved case, or obtain an explicit contract limiting the guarantee to matched refusals and timely owner progress. Mark that restriction as an assumption, not VERIFIED hardware behavior.

The UHD-only read-back refusal difference is already documented by MR-18 (`design/09-mock-radio.md:154`); that supports a scoped fault-model exception, not universal parity.

## Major 2 — M15 cannot be killed by the named existing tests

M15 removes `!planned.refused` from RX coverage. But RX planning already omits refused segments (`crates/ezsdr-radio/src/timeline.rs:945–964`), and UHD builds its entries from that plan (`crates/ezsdr-radio-uhd/src/provider/core.rs:582–590`).

The existing refusal test explicitly asserts that the RX plan is empty (`control.rs:3846–3866`). Removing the additional filter cannot change its result.

**Required plan change:** Remove M15 as behaviorally redundant under the caller invariant, recording why. Alternatively, identify a genuinely reachable violating caller and its distinguishing assertion; do not manufacture an impossible production state merely to claim a kill.

Also correct the secondary mutation assignments:

- **M5:** Test 18 distinguishes it. Test 15 does not: its restart is booked *after receipt*, so receipt-time `adapter(e)` also finds nothing.
- **M6:** Test 16 distinguishes it. In 17a, `timed_from` is None, so replacing equality with “both Some” still produces untimed.

M1–M14 otherwise have plausible distinguishing observers, subject to implementation and execution.

## Minor 3 — Receipt/effective times still need correction

Test 3 specifies receipt at `T0 + 6.5 ms` and effective instant `T0 + 7 ms`. Mock’s X3xx command lead is **2 ms** (`crates/ezsdr-mock-radio/src/profile.rs:51`), and `handle_update` applies that floor (`lib.rs:1913–1917`). Its effective instant is therefore at least `T0 + 8.5 ms`.

Likewise, existing test 1743 receives its origin-targeted update during `step(T0 + 20 ms)`, not when `push` follows the earlier step (`mock_layout.rs:1770–1774`).

**Required plan change:** Correct these times explicitly; book test 3’s recovery only after the actual update application. Require 17a/17b’s calibrated choices to satisfy `e ≥ r + 2 ms`, not merely `r < e`.

## Minor 4 — MR-34’s replacement omits timed-tune behavior

The proposed paragraph defines which updates count as timed, but removes the existing sentence assigning the timed-phase constant and defining random-channel draws (`design/09-mock-radio.md:213`).

**Required plan change:** Retain explicit outcomes for classified timed tunes: constant/fixed for `random_unless_timed_tune`, one fresh draw/invalid for `random`, and zero/valid without a draw for deterministic channels.

Remove the D9b staging note from normative Ceilings; it is history/future work, not a conservative difference. Move bench uncertainty to the evidence record unless expressed as a precise guarantee exclusion.

## Confirmed

- Same-stream/same-origin comparison handles ordinary stop/restart, moves and cold changes that replace the begun segment. Application checking makes later cuts conservative.
- The revised 17a/17b calibration construction is structurally reachable when C2 leaves S2 samples.
- Rewriting test 1743 to expect `retune[1]` and invalid coherence at origin is correct. Dropping its old rank mutation is justified.
- No additional fixed phase expectation change was identified in the searched frequency-update tests. VH-8 remains unchanged: its timed operations update gain (`crates/ezsdr-mock-radio/tests/timeline.rs:672–673`).

No files changed; no agents, builds, tests, mutations, Docker or hardware were run. Cross-provider late-owner behavior remains the blocking risk.

## 計画レビュー 5

> plan · gpt-6.1-sol
VERDICT: FAIL

## Major 1 — Test 19 uses the wrong terms and cannot prove “no silent window”

The inequality is valid **under the explicit start-issuance assumption**:

`release ≥ e − window ≥ origin + mock_lead − window > origin − start_issue_margin ≥ t_s`

For the stated values, `2 + 2 > 3 ms`. However, the terms are not interchangeable:

- Mock’s advertised/booking lead is **2 ms** (`crates/ezsdr-mock-radio/src/profile.rs:49–53`; `src/lib.rs:1913–1917`).
- UHD’s advertised command lead is **5 ms**, not 2 ms (`crates/ezsdr-radio-uhd/src/profile.rs:11–22`).
- UHD’s timed-update booking actually uses **DEVICE_LEAD_NS = 2 ms** (`crates/ezsdr-radio-uhd/src/provider/control.rs:603–609`).
- RX start issuance has **no enforced origin-minus-2-ms deadline**: the owner calls `rx_start(origin)` when configuration finishes (`crates/ezsdr-radio-uhd/src/provider/rx.rs:304–334`).

Absence of `LATE_COMMAND` does not establish that deadline. FakeDevice marks an unscripted start late only when `at < now`, not when less than 2 ms remain (`crates/ezsdr-radio-uhd/src/device.rs:1395–1402`). Native issuance likewise has no such margin check (`crates/ezsdr-radio-uhd/src/uhd.rs:592–607`).

**Required plan change:** Make test 19 check the actual named terms separately; remove the asserted equality of advertised leads. Keep the 2-ms start-issuance margin explicitly assumed, not inferred from absence of lateness events. M16 usefully protects the **numerical sufficient condition**, but cannot verify owner scheduling or hardware behavior.

Thus the conditional argument is acceptable; “LATE_COMMAND が出ない限り” at plan lines 62/185 is not established.

## Major 2 — Test 17c contradicts its forced ordering

Both cases explicitly hold the owner until **after release**, but case (ii) then claims the owner processed the refusal **before release**. Moving `e` cannot reverse an explicitly forced operation order.

Also, one owner turn cannot both refuse C1 and configure/start S3: the refusal branch returns immediately (`crates/ezsdr-radio-uhd/src/provider/rx.rs:309–322`). A subsequent turn must reread the plan and begin S3.

**Required plan change:** Split the schedules:

1. **Release first:** assert Deferred/no frequency apply; process C1’s refusal; reread the advanced S3; configure/start it; advance beyond `e`; inspect calls and read-back.
2. **Owner first:** process refusal and begin S3 before release; then assert the timed frequency call.

For release-first, case (ii) cannot expect successful delivery merely because it lies outside the assumed 2-ms-margin window. Add explicit start-issuance timestamps and drive receive far enough to observe any late-start report. Do not equate “no apply” alone with Deferred without asserting the destination and unconfigured projection preconditions.

## UHD defect conclusion — supported, with a boundary correction

**VERIFIED code-path failure:** Given Deferred release followed by refusal advancing S3 to **`o3' < e`**, the update is not delivered to that S3:

- Deferred calls no device method (`crates/ezsdr-radio-uhd/src/provider/control.rs:788–792`).
- Configure projects only updates with `effective ≤ origin` (`control.rs:146–152,158–175`).
- Release removes the update from `held`; later folding updates configuration-of-record, not the running device (`control.rs:669–673,702–709,729–733`).
- Initial start and late-start recovery issue start commands only (`crates/ezsdr-radio-uhd/src/provider/rx.rs:331–334,447–465`).

**Minor correction:** Replace the plan’s `e ≥ o3'` loss claim with **`e > o3'`**. At equality, configure includes the update.

**Reachability — INFERRED, structurally supported:** Refusal can advance S3 because the refused S2 is omitted while its predecessor remains cut (`crates/ezsdr-radio/src/timeline.rs:953–964,1022–1040`). The problematic ordering requires delayed refusal processing relative to release, but **does not require a missed start**: release and both owner operations can occur before `o3'`, even satisfying the assumed 2-ms start margin for sufficiently small positive `e − o3'`. It is therefore reachable under an on-time-start schedule, not exclusively a late-start failure. Prompt eager refusal processing avoids this ordering; code inspection does not establish its frequency on real hosts.

## Round-4 closure

M15 removal, M5→18, M6→16, corrected receipt/effective times, and restored MR-34 timed-tune outcomes address the other round-4 findings. The assumption is now explicit, but its event-based interpretation and mandatory regression still need the corrections above.

No files changed; no agents, builds, tests, mutations, Docker or hardware ran. Hardware timing and actual reproduction remain unchecked.

## 計画レビュー 6

> plan · gpt-6.1-sol
VERDICT: PASS

Round-5 findings are addressed within the requested scope.

- **Test 19 / M16:** The plan separates Mock’s 2 ms command lead, UHD’s 5 ms advertised lead, UHD’s 2 ms booking lead, and the 3 ms release window. The 2 ms start-issuance margin is explicitly an assumption. Test 19 and M16 protect the numerical sufficient condition, not owner scheduling or hardware behavior. MR-34’s proposed Ceiling makes that limitation explicit; absence of `LATE_COMMAND` is no longer evidence for the margin.
- **Dropped 17c/17d:** No remaining Mock test depends on those UHD regressions. Tests 17a/17b assert Mock’s receipt/application coverage rule; 17b explicitly carries D2’s assumption. The deferred-value loss is separately recorded, not claimed fixed.
- **Defect note:** The boundary **`e > o3'`** is correct; equality is included by configure. The note correctly distinguishes the verified code-path failure from inferred reachability, including an ordering that does not require a missed start.
- **Consistency:** The test list, MR-34 Test additions, mutation rows **M1–M14 and M16**, and implementation order agree. UHD coverage remains the existing D9a tests.

**Minor clarification:** In `ma1-test-plan.md:157`, replace “Mock の lead と release の窓は `ezsdr_radio::x3xx` の値” with “Mock の lead は Mock の `Profile::terms()`，release の窓は `ezsdr_radio::x3xx::timing()` の値.” The main argument and test 19 already identify these sources correctly.

No files changed; no builds, tests, mutations, or hardware checks ran. The UHD defect remains open outside MA1.

## コードレビュー

> plan · gpt-6.1-sol
VERDICT: FAIL

The implementation matches the reviewed plan. One normative-text correction is required; I found no production-code defect introduced by this diff.

## Independent scenarios checked

- Coverage before/at origin, before/at cut, empty segments, failed cut conversion, wrong channels and multiple streams.
- Updates received before T0, during reception, while stopped, and before planned or refusal-advanced segments.
- Stop/restart and channel moves replacing receipt-time coverage before application.
- Linkless streams, including explicit `start_rx`.
- Timed constants, random draws, deterministic zero phase, draw ordering and segment coherence.
- Shared Session/Spec update handling, unchanged TX behavior and unchanged UHD destination selection.

## Findings

### 1. Major — New Ceilings overstate the receive-only and two-draw rules

**Location:** `design/09-mock-radio.md:217–219`

**Problem:** Line 217 says an “update” received before its segment begins is untimed, without restricting this to **receive frequency updates**. TX frequency updates remain timed—including the pre-T0 case explicitly tested at `crates/ezsdr-mock-radio/tests/mock_layout.rs:3313`.

Line 219 likewise says an untimed update draws once and a following segment origin draws again, without preserving two exceptions:

- Deterministic channels draw nothing (`crates/ezsdr-mock-radio/src/lib.rs:2189–2192`, `:1061–1064`).
- The initial segment takes no entry draw (`:1047`). The pre-T0 regression at `crates/ezsdr-mock-radio/tests/mock_layout.rs:3081` therefore consumes one retune draw, not two.

These are contradictions in accepted normative text, not implementation failures.

**Fix:** Qualify line 217 as “a receive frequency update.” Rewrite line 219 to say that a non-deterministic receive frequency update and a subsequent noninitial segment-entry tune consume separate draws, when that entry tune occurs. Preserve deterministic and initial-segment exceptions.

### 2. Note — UHD’s remaining destination-channel filter needs a separate regression, but does not block MA1

**Location:** `crates/ezsdr-radio-uhd/src/provider/control.rs:743–756`; `plan/maintenance/tools/mutations.json`, P989.

Removing the channel filter from `holds` can select an unrelated configured future segment instead of the target channel’s unconfigured destination. That changes Deferred into an untimed device write.

Retargeted P989 correctly tests the extracted **coverage** filter, but does not test this **destination** filter. The gap predates extraction: the old mutation changed both decisions and was killed through coverage.

**Recommendation:** Accept this gap for MA1, since destination selection is unchanged. Add a focused UHD regression in follow-up: unrelated configured segment first, target-channel unconfigured segment later; assert no release-time write and delivery through the target’s configure. Adding that test would not change UHD behavior.

## Tests, mutations and scope

- The planned scenarios have concrete phase, coherence or draw-index observations. The documented test reshaping preserves their purpose.
- The renamed RX tie test correctly expects two untimed draws. The added TX test preserves P187’s distinguishable command-versus-entry ordering.
- P999–P1013 target real decisions. Static inspection found every mutation source pattern matched exactly once, and every MR-34 cited test exists.
- UHD’s extracted predicate is behaviorally equivalent. No TX branch, profile value or Kernel change was introduced.
- RM-36 and the failure-mechanism paragraph match the code. Changes-table decision links, mutation-ID reconciliation and `KNOWN_LIMITATIONS.md` remain explicitly assigned integration work.

**Not checked:** No files changed; no builds, tests, mutations or hardware checks ran. Execution results are the implementer’s report, not independently reproduced. The conditional start-margin assumption and deferred-value-loss defect remain outside MA1.

## 見直し（MR-34 の文）

> plan · gpt-6.1-sol
VERDICT: PASS

No findings. The corrected Ceilings in `design/09-mock-radio.md:217–219` now match the code:

- The pre-segment untimed rule applies only to receive frequency updates; transmit updates remain timed.
- Only `random` and `random_unless_timed_tune` channels draw; deterministic channels draw nothing.
- The segment `arm`/`start` establishes takes no entry draw; a later segment-entry tune consumes a separate draw.

These qualifications agree with MR-34’s surrounding rules and `crates/ezsdr-mock-radio/src/lib.rs:1038–1081,2183–2213`. I found no remaining contradiction within MR-34.

Not checked: No builds, tests, mutations or hardware checks ran. UHD’s `holds` filter remains the accepted follow-up outside MA1.

## 規則の差し替えの確認

> plan · gpt-6.1-sol
VERDICT: ADOPT WITH CHANGES

Adopt the application-time rule, remove receipt tracking, and revise the tests and specification below. The guarantee remains **conditional on the existing owner-deadline assumption**, not an unconditional hardware guarantee.

## Evidence and countersequences

**VERIFIED — release cannot occur before `e − W`.** In `crates/ezsdr-radio-uhd/src/provider/control.rs:643-669`, every iteration checks `e − now > window` before calling `give`. A held command can only release later; a batch cannot bypass the check. `give` chooses timed/untimed/deferred at `:784-788`.

The terms are distinct:

- `W = 3 ms`: `crates/ezsdr-radio/src/x3xx.rs:392-403`.
- UHD device lead `L = 2 ms`, delivery allowance `A = 3 ms`, advertised lead `L + A = 5 ms`: `crates/ezsdr-radio-uhd/src/profile.rs:9-22`.
- Mock command lead `Lmock = 2 ms`: `crates/ezsdr-mock-radio/src/profile.rs:47-54`.
- Root-tick conversion rounds up: `crates/ezsdr-radio-uhd/src/provider/core.rs:935-938`.

**VERIFIED — the booking floor does not release early.** `timed_instant` sets
`floor = max(now + L, earlier bookings ≤ now + W)` and `e = max(requested, floor)` (`crates/ezsdr-radio/src/timeline.rs:424-428`). It changes the effective instant; it does not bypass release gating. Configure selects only updates due by the segment origin (`control.rs:146-175`), so a qualifying segment with `origin ≤ e − W < e` does not consume this update during its configuration.

**INFERRED — conservative under the stated assumption.** Let `o` be the origin of the segment covering the channel at application. If `o ≤ e − W`, and its successful start is issued by `o − 2 ms`, refusal handling and start issuance precede the earliest release by at least 2 ms. The updated plan therefore exists before release; UHD’s covering predicate selects `Reach::Timed` (`control.rs:738-746`; owner refusal/start path: `crates/ezsdr-radio-uhd/src/provider/rx.rs:304-345`). That deadline is an assumption, not enforced by this code.

Consequences:

- **Refusal-advanced S3:** `o3′ ≤ e − W` qualifies, even when receipt preceded `o3′`. If `e − W < o3′ ≤ e`, Mock remains untimed. The known deferred-value-loss sequence requires release before the relevant replanning; for a qualifying segment that violates the start-deadline assumption.
- **Stop/cut:** no segment covering `e` means untimed; equality with the cut is excluded (`timeline.rs:194-199`). A cut learned after UHD released can make Mock more conservative.
- **Flip-flop/moves:** judge the replacement/destination segment, not receipt identity. It qualifies only if its own origin meets the window boundary. Mock processes earlier work before application (`crates/ezsdr-mock-radio/src/lib.rs:2264-2315`).
- **Linkless streams:** keep excluding them (`lib.rs:1019-1023`).
- **Session `e = r + 2 ms`:** the condition becomes `o ≤ r − 1 ms`; a just-started segment is conservatively untimed.

**VERIFIED — restores Spec Run retunes.** Scheduled timed Actions are dispatched together before the initial round (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:462-476`; KC-17/KC-18, `design/06-kernel-coordinator.md:250-252`). Receipt time must not determine coherence. An initial segment qualifies for `e ≥ T0 + W`, provided it still covers `e`.

## Concrete changes

### Implementation and tests

Delete `PendingUpdate.timed_from` and its plumbing. At application, find the covering segment across linked receive streams and compare its origin with `e − W`, using the description’s window converted to root ticks. No new lifecycle record is needed.

In `crates/ezsdr-mock-radio/tests/mock_layout.rs`:

- **Change expected result to timed:** stopped-at-receipt/later-covering (`:3406`), flip-flop (`:3434`), pre-start receipt with `e = T0 + 5 ms` (`:3462`), refusal-advanced segment with `e = o3′ + 5 ms` (`:3583`). Assert the timed constant, valid coherence, and no update draw.
- **Replace the lead-plus-margin arithmetic test** (`:3481`) with behavioural boundary cases: `o = e − W` qualifies; one root tick later does not. Include pre-T0 receipt and a Session update with 2 ms lead.
- Keep other verdicts, including before-T0 **application**, cut, refused segment, linkless, deterministic/CBX and transmit tests. Correct receipt-based comments, including `:1743`.
- Add a real scheduled Spec Run regression, observing samples and coherence.

### Mutations

In `plan/maintenance/tools/mutations.json:13627-13774`:

- Keep P1022–P1025, P1029–P1035.
- Replace P1026–P1028 with mutations removing the window test, rejecting equality, and omitting application-time coverage.
- Replace P1036’s obsolete Mock-lead mutation with one ignoring/changing the window; ensure the behavioural boundary test kills it.
- Update filters/names for renamed tests.

### Specification

In MR-34 (`design/09-mock-radio.md:214-231`), replace the receipt/same-segment sentence and receipt-based Ceiling with the application-time window rule and its conservative `(e − W, e]` exception. Retain the explicit owner-deadline assumption, phase/draw rules and transmit behaviour; update Test lines and Changes history. MR-27’s reference remains valid.

Update KL-32 (`KNOWN_LIMITATIONS.md:82-87`): remove the stale Mock claim and specify scheduling at least `W` after the relevant segment origin.

Rejected alternatives: receipt tracking breaks scheduled Runs; covering-at-application alone leaves the refusal/replanning hole.

Not checked: no edits, builds, tests or hardware runs. Hardware coherence and deadline compliance remain assumptions, not newly verified measurements.

## 見直し（差し替えの後）

> plan · gpt-6.1-sol
VERDICT: PASS

No major or minor findings.

- **Application-time rule:** `crates/ezsdr-mock-radio/src/lib.rs:1015,2178–2190` checks coverage across linked receive streams and uses the profile’s release window converted to root ticks. Receipt tracking is gone; transmit behavior remains unchanged.
- **Tests:** `crates/ezsdr-mock-radio/tests/mock_layout.rs:3406–3648` contains the revised stopped-receipt, replacement-segment, pre-T0 and refusal-advanced cases. The boundary test checks exactly `origin + W`, one tick earlier, and untimed Session updates with the 2 ms lead.
- **Scheduled regression:** `crates/ezsdr-acceptance/tests/v58.rs:802–838` uses a real Spec Run schedule, through `start_spec_run` and scheduled dispatch—not a direct Provider push. It checks captured phase before and after the retune and the segment’s coherence basis and instant.
- **Mutations:** P1022–P1037 target the intended predicates and phase behavior; their named tests contain distinguishing assertions. P589 matches the receipt-state-free insertion. All **1,762 rows across six mutation lists** have exactly one `old` match.
- **Specifications:** MR-34, RM-36, UR-24, KL-32 and Changes describe the implemented rule. The conservative near-origin exception and conditional owner-deadline assumption remain explicit.
- **Existing regression preserved:** `crates/ezsdr-mock-radio/tests/mock_channel.rs:449–484` still distinguishes receive and transmit timeline reads across the 45-sample interval; moving the update to 3.5 ms preserves its purpose.

Not checked: no edits, builds, tests or mutation runs. Reported mutation kills were not independently rerun; hardware coherence remains conditional on the stated deadline assumption.
