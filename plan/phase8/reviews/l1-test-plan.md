# L1 receive-chain test plan

**Run 1 — plan only.** No implementation, repository edits, test runs, mutation runs, or reviewer calls in this run. Worktree: `/home/komatsu/local_work/Ez-SDRv4-l1`, branch `wip/l1`, base `main` at `3e63ad2`.

**Basis:** accepted design note 44 v6 (§3.1–§3.8, §6 RX 1–9 and 4a, §8); note 39 §6.6–§6.8; note 35; the D9b-2 review record; current provider code and UR-24/UR-25/UR-26, MR-34, MA-1, KL-9. The plan adopts note 44’s rule: uhd-control hands a timed RX frequency only while the chain holding that channel is already `Armed` or `Streaming` the segment covering its instant. Until then it retains the value and stops that release pass. A value with no covering segment continues through the existing `targets`/projection path.

## Test discipline and oracles

- Use `ManualTimeAuthority` and the FakeDevice/LockSpy device log. For concurrency, use explicit gates/channels and registered `HostClock::spawn` participants. Wall-clock timeouts may only bound a stuck test; they must not decide expected device behavior. Do not add sleep-based ordering or wall-clock timing oracles.
- Observe the property named in each row through the relevant combination of: `RxChain` fields/state and `SegmentKey`; `Control.held`; device call order and timed/untimed `apply`; FakeDevice `dsp_pending`/`dsp_log`; `RunRecord.applied`/`rejected`; emitted events; `Streams` phase/holder and planned origins; and the values used by later configure.
- FakeDevice’s DDC sample-consumption log is an assertion about the model, not a claim about physical phase. UHD’s documented past-due command behavior may be relied on only as “executes immediately in queue order.” Physical timed-tune phase when the command arrives late remains **INFERRED** and is not asserted.
- Preserve the raw P4 A/C hardware characterization as observation-only. It bypasses the RX owner and must not be converted into a production-path verdict.

## Run 2 amendments (adopted before implementation)

These amend the rows below and take precedence over any earlier optional wording. Tests use explicit proposed names; one case is one test, except RX 1 may table-drive outcomes for one transition function. Every gate uses a waiter handshake on the chain lock, not simultaneous thread launches. A wall-clock timeout is only a hang guard.

### RX 1 — named transition tests

`rx_chain_adopt_start_transitions`; `rx_chain_begin_transitions`; `rx_chain_restart_transitions`; `rx_chain_first_sample_transition`; `rx_chain_hand_transitions`; `rx_chain_stop_transitions`; `rx_chain_flush_transitions`; `rx_chain_finish_transitions`; `rx_chain_lose_is_terminal`; and `rx_chain_abandon_unowned_records_cleanup_state`. Each table lists its prior state, device result (success, ordinary refusal, or loss where applicable), and exact resulting device/intent/segment/debt/left/latest/restore facts. `rx_chain_finish_transitions` includes `Lost` and proves no device call. Keep the integrated first-segment adoption and unowned-owner cleanup assertions.

### RX 2 — lock publication, both lock orders, and first sample

- `ur_24_a_release_waits_until_begin_publishes_eligibility`: gate begin after the device `rx_start` succeeds while it still holds that stream's chain lock and before both chain eligibility and `Streams::started` are published. Confirm the release thread has queued as a chain-lock waiter; no timed apply or debt can enter before release; after releasing the gate and completing publication, one release pass hands exactly once to that segment.
- `ur_24_a_begin_and_timed_apply_serialize_in_both_chain_lock_orders`: test begin-first, apply-before-restart, and restart-before-apply with explicit waiter handshakes. Control cannot apply until begin/restart has atomically published `Armed`/`Streams::started`; begin/restart cannot start until apply and debt publication complete. Assert call order, chain identity, held/debt, and exactly one device apply.
- `ur_24_a_first_rx_sample_preserves_eligibility_and_segment_identity`: hold the chain lock at the first-sample transition and prove the `Armed` → `Streaming` transition preserves the covering `SegmentKey`; a waiting timed handoff enters after the transition and is accepted once. Retain `ur_24_a_stop_decision_waits_for_a_gated_timed_rx_handover` and `ur_24_a_completed_rx_owner_serializes_a_late_frequency` for stop/completion contention.

### RX 3 / RX 4 — minimum lead and late delivery

- `ur_24_a_unbegun_successor_does_not_extend_the_old_chain_drain`: exercise L = W through production Control/owner paths, proving no successor debt is added to the old chain and the old drain finishes before successor configure/start; then begin and hand once on a later pass. Preserve the planned origin and minimum lead. Gate the old owner at stop while the release window opens so the test proves the held behavior, then prove the successor starts with L intact.
- `ur_24_a_held_frequency_is_handed_after_begin_inside_release_window` and `ur_24_a_held_frequency_is_handed_after_its_instant`: manually step release and begin, then assert the original `at = e`, no event while held, and one timed apply after eligibility.
- `ur_24_a_late_command_restart_preserves_rx_eligibility_before_release` and `ur_24_a_late_command_restart_preserves_rx_eligibility_after_release`: cover both restart/release orders under the chain lock. `ur_24_a_cut_bearing_late_command_finishes_without_restart` separately asserts the cut-bearing path ends and drains normally.

### RX 4a — loss cases

Keep one test per loss point: `ur_24_a_loss_during_timed_rx_handoff_finishes_without_recovery_calls`, `ur_24_a_loss_during_rx_receive_finishes_without_recovery_calls`, and `ur_24_a_loss_during_rx_flush_finishes_without_recovery_calls`. Each starts with any prior debt specified by the sequence, distinguishes a rejected new handoff from accepted debt, counts one `rx_drain` report per entry, and checks no device call after loss.

### RX 5 — mandatory mixed queue and retarget interleavings

- `ur_24_a_blocked_rx_head_holds_later_rx_and_tx_updates`: production release queue with a held frequency for an unbegun covering RX segment, then a frequency for a second already-running RX stream, then a TX update. Hold the first owner's chain lock before begin and manually advance past all three instants. Assert no later device apply and no premature `issued`/`applied` row. After begin, the next pass applies in `(e, sequence)` order, keeps each row's original `at`, and proves the second stream's owner began on time although its physical update was delayed.
- `ur_24_a_refused_blocked_head_reconsiders_later_commands`: refuse the blocked head, then prove the next release pass reconsiders and releases the following commands in order.
- A separately cancelled blocked head cannot be removed while the Run continues: `Control::cancel_held` is only used at the Run end and calls `settle` for the entire held/released queue (`control.rs:902-943`); there is no selective cancellation path. Replace the unreachable reconsideration sequence with `ur_24_a_stop_settles_blocked_rx_head_and_later_commands`, which proves stop settles both blocked head and suffix at S without any later device apply.
- Test each retarget between two release passes, explicitly stating the manual clock step or gate that places the change between passes and the second-pass result: `ur_24_a_refused_covering_segment_retargets_held_frequency`; `ur_24_a_stop_retargets_held_frequency`; `ur_24_a_cold_change_before_e_retargets_held_frequency`; `ur_24_a_move_retargets_held_frequency_to_new_destination`; and `ur_24_a_removed_destination_retargets_held_frequency_to_no_channel`. Assert the appropriate projection/configure, untimed target, or `no channel` result, and that later held commands do not overtake the head. Keep the existing stop/loss-at-S tests and their exact S-boundary assertions. The move case changes the destination after the first pass and observes it on the second; the no-destination case removes that destination after the first pass and observes `no channel` on the second.
- Replace the removed successor-debt refusal tests with `ur_24_a_configured_successor_configure_refusal_keeps_frequency_held` and `ur_24_a_configured_successor_start_refusal_keeps_frequency_held`: issue the first release while the covering segment is unbegun, prove no device debt, refuse configure or start, then prove the held command takes the existing projection/no-channel path through cleanup and a later open without a stale timed write.

### RX 6 / RX 7 / RX 8 — chain reuse, invariant, restoration

- `ur_24_a_left_and_latest_survive_close_open_and_chain_reuse`: production path; hand a value, force normal drain and bounded flush to fail, finish with one `rx_drain` leftover, close/open and begin a new segment, verify the fake's stale value survived, pass qualifying samples to consume it, and assert software removes the `left` blocker without a second `rx_drain` row. Stop and inspect device state before another configure; restoration must use retained `latest`.
- `ur_24_a_left_stays_on_the_old_chain_after_channel_move`: repeat with the channel moved; the leftover remains on the old stream chain, blocks restore on that channel, and is inspected before destination configure can hide it.
- `ur_24_provider_stop_skips_restore_for_left_on_this_chain`: fail the normal drain and bounded flush with `e > S`, then stop and finish the same owner; assert the local `left` blocks the planned restore and inspect the device before any later configure.
- For every §3.6 close/open path, keep a named caller-level test in addition to helper panic checks: `rx_chain_reconfigure_close_requires_idle_empty_debt`; `rx_chain_release_unneeded_close_requires_idle_empty_debt`; `rx_chain_control_open_requires_idle_empty_debt`; `rx_chain_rate_check_failure_close_requires_idle_empty_debt`; `rx_chain_open_failure_close_requires_idle_empty_debt`; `rx_chain_prepare_open_requires_idle_empty_debt`; and `rx_chain_cleanup_close_requires_idle_empty_debt`. `left` is permitted; `abandon_unowned` and detached-owner cleanup remain separate.
- Extend the provider-stop restore test with `latest` surviving consumption and close/open. `ur_26_provider_stop_restores_the_frequency_in_force_at_its_instant` covers same-instant booking order, an initial value on a second channel, and multiple channels; `ur_24_provider_stop_drains_after_s_then_restores_the_frequency_in_force_at_s` covers exclusion of a post-S update and restore after drain; `ur_24_provider_stop_skips_restore_for_left_on_this_chain` and `ur_24_a_left_stays_on_the_old_chain_after_channel_move` cover `left` on this and another chain; `ur_24_provider_stop_records_restore_failure_and_stops_after_loss` covers refusal and loss. Inspect device state before any subsequent configure.
- `ur_24_a_parallel_rx_restore_checks_do_not_nest_chain_locks`: two RX owners restore concurrently at Provider stop, each with its own planned channel. Gate both at the pre-scan probe after releasing their local chain lock; both must complete and apply once, proving that checking another chain never nests chain locks or deadlocks.

### RX 2 / RX 4 — delayed apply refusal

`ur_24_a_delayed_rx_apply_refusal_rolls_back_debt_and_record`: hold a frequency past `e`, begin the covering segment, and refuse the timed apply without loss. Assert provisional debt is removed, `latest` and configuration of record stay unchanged, exactly one `COMMAND_REJECTED` is emitted, no frequency-update `applied` row exists, existing projection behavior is unchanged, and a later held command proceeds on the next release pass.

### RX 9 — raw P4 characterization

Keep `ur_24_a_cut_booked_after_the_release_leaves_the_entry_for_the_next_segment` as a raw-device characterization only. Inject V1 using the raw timed device apply, not `timed_at` or Control; retain stale-overwrite and pending assertions and explicitly state that the sequence bypasses the RX owner and Control eligibility. It is not evidence for production handoff behavior.

## Required sequences

Each row below specifies a test, the assertion, the observation, and the incorrect behavior it must reject.

### RX 1 — `RxChain` transition table and cleanup cases (§3.3)

**Test:** Pure, thread-free unit tests for every transition/result in the table, plus a small `Rx`/`Streams` integration test for adoption of the first segment and cleanup when an owner cannot be woken.

**Assert:** `adopt_start` takes `Off` to `Armed` with the first `SegmentKey`; `begin` succeeds from `Off` and `Uncertain`, fails without falsely changing the prior device fact, and turns loss into terminal `Lost`; `restart` works from `Armed`/`Streaming`, preserving the prior fact on ordinary failure; first sample changes `Armed` to `Streaming`; stop success/failure/loss yields `Off`/`Uncertain`/`Lost`; flush start failure, read outcome, final-stop failure, and loss preserve their specified states; `finish` works from `Off`, `Uncertain`, and `Lost` without a device call from `Lost`; `lose` is terminal/idempotent; `abandon_unowned` records cleanup state without making a device call. Assert `Intent.cut`, `end_now`, and `terminate` remain independent, and finishing clears only the segment/debt facts allowed by the transition.

**Observe:** Exact state before/after each transition, scripted device call log and result, debt/left contents, one-time drain report, and `Streams` phase/holder after integrated finish. Unit tests invoke transitions directly; no thread is needed.

**Wrong behavior caught:** A failed start recorded as stopped; `Uncertain` treated as `Off` or blocked from the next begin; `Lost` followed by another device call; finish omitted after loss; termination erasing a cut; two independent intent facts collapsed; repeated finish/report; or a cleanup path inventing a live segment.

### RX 2 — release eligibility, held order, and begin/release race (§3.1)

**Test:** `ur_24_a_release_waits_until_begin_publishes_eligibility`; `ur_24_a_begin_and_timed_apply_serialize_in_both_chain_lock_orders`; `ur_24_a_first_rx_sample_preserves_eligibility_and_segment_identity`; and the mandatory mixed RX/RX/TX queue test `ur_24_a_blocked_rx_head_holds_later_rx_and_tx_updates`. Also retain the covered-current, uncovered-instant, no-covering-segment, and gain controls as separate named cases or table entries.

**Assert:** The covering current segment receives exactly one timed apply and owns the resulting debt. A planned successor's value stays in `Control.held`, with no timed or untimed RX apply/debt, and blocks entries behind it. Both lock orders serialize; after begin/publication a subsequent pass hands once. Held values retain instant/booking order. With no covering segment, existing target/projection behavior still applies. Gain remains unaffected by the frequency-only rule. The mixed queue must hold the later RX and TX commands, not merely commands the fixture happens to expose.

**Observe:** Timed/untimed device-call log and order, held list before/after each pass, debt and `SegmentKey`, later-value calls, `applied` rows, and chain state at the gate. Use explicit gates; a synchronization timeout is only a hang guard.

**Wrong behavior caught:** A stale shared `stopped` bit authorizes handoff to an unbegun successor; a begun segment fails to receive its held value; double handoff across the race; a later command bypasses a blocked held head; or the new rule accidentally changes gain/no-covering-segment routing.

### RX 3 — minimum lead and no successor debt in the old drain (§5; §6 RX 3)

**Test:** Deterministic FakeDevice/LockSpy regression with `L = W`, an old segment ending at `c`, and a successor frequency `e` whose release window opens while the old owner is finishing. Keep any old-segment debt separate from the successor value. The successor is planned/configured but has not started when the release pass sees it.

**Assert:** The successor value is not added to the old chain’s debt and does not extend its drain deadline. The old segment stops/finishes before successor configure/start; the successor keeps its planned origin and is started with its minimum lead intact. Once the successor is `Armed`/`Streaming`, a later release pass hands the value timed exactly once; FakeDevice records the value on the qualifying sample at/after `e`.

**Observe:** Chain debt and device log before/after handoff, computed drain boundary and successor origin on the manual clock, call order (`rx_stop`/finish before successor configure/start), `Streams` origin/phase, and the fake DDC log.

**Wrong behavior caught:** Release hands the future successor value to the old chain, its drain waits through that successor instant, the successor origin/lead moves, or a held value is lost/doubled after the successor begins. Do not infer real hardware tune phase from this fake test.

### RX 4 — owner delay, late delivery, and LateCommand (§3.8; §6 RX 4)

**Test:** `ur_24_a_held_frequency_is_handed_after_begin_inside_release_window`; `ur_24_a_held_frequency_is_handed_after_its_instant`; `ur_24_a_late_command_restart_preserves_rx_eligibility_before_release`; `ur_24_a_late_command_restart_preserves_rx_eligibility_after_release`; and `ur_24_a_cut_bearing_late_command_finishes_without_restart`.

**Assert:** Until begin, the value remains held. After begin, the next release pass makes one timed apply even if `e` has passed; its `applied` row has `at = e` and is written only when the value is actually handed. Holding emits no new event. The cut-free restart preserves the chain’s eligibility before or after restart; a cut-bearing LateCommand ends the segment and does not restart it, and its debt follows normal drain/finish rules. Existing reservation/start `LATE_COMMAND` behavior remains unchanged.

**Observe:** `Control.held`, device apply log and timestamp, `RunRecord.applied` and event list, `RxChain` device/intent state, and FakeDevice `dsp_log` at the first qualifying sample. Do not assert a physical late-tune phase.

**Wrong behavior caught:** A value is handed before begin, remains held forever after begin, gets an untimed substitute, creates an event merely for being held, records `at` as delivery time instead of `e`, or a cut-bearing LateCommand restarts.

### RX 4a — loss while handing, receiving, or flushing (§6 RX 4a)

**Test:** Three scripted loss points: timed `apply` during handoff, ordinary RX receive, and bounded flush. For receive/flush, start with at least one existing debt entry; during handoff, distinguish the failed new entry from any pre-existing debt.

**Assert:** Loss sets `device = Lost`; the failed new handoff is not left as accepted debt; outstanding debt is reported at most once; `finish` completes the segment/`Streams` transition without any post-loss device call. Loss publication/event occurs once, and subsequent cleanup/finish does not duplicate reports or device operations.

**Observe:** Scripted call order, `Lost`/segment/debt/left state, per-entry `rx_drain` rejection rows, `Streams` completion/prune/holder, and device-loss event count.

**Wrong behavior caught:** Receive/flush continues after loss, the dead device is stopped/restored, a new failed handoff is treated as accepted, the segment/claim is stranded, or an entry is omitted/reported twice.

### RX 5 — covering segment disappears; target fallback, settle, and head-of-line (§3.1, §3.8; §6 RX 5)

**Test:** `ur_24_a_refused_covering_segment_retargets_held_frequency`; `ur_24_a_stop_retargets_held_frequency`; `ur_24_a_cold_change_before_e_retargets_held_frequency`; `ur_24_a_move_retargets_held_frequency_to_new_destination`; `ur_24_a_removed_destination_retargets_held_frequency_to_no_channel`; `ur_24_a_refused_blocked_head_reconsiders_later_commands`; and `ur_24_a_stop_settles_blocked_rx_head_and_later_commands`. Put each reachable retarget between two release passes as specified above. The last case is stop settlement, not a release pass after cancellation, because stop settles the Run. Keep stop/loss at and around `S` as named cases, with exact S-boundary assertions.

**Assert:** Each pass rereads the plan. When the covering segment disappears, the value takes the existing `targets` outcome: projection/configure carries it, a configured later segment gets the existing untimed release, or it is recorded `no channel`/cancelled. No stale timed handoff/debt is made. Values behind the held head do not pass early; after the head is rerouted/settled, later values proceed in original order. Stop/loss preserve existing `settle` boundaries, including the `S`-instant rule: values effective by `S` settle as before; later values are cancelled as before. Check later configure values, not just a row’s `at`.

**Observe:** Each release’s device calls, held/released entries, projection/configure value, applied/rejected rows and events, and `Streams` membership/origin. Cover both a changed destination and no remaining destination.

**Wrong behavior caught:** Control uses a stale segment identity, silently drops a held command, hands it to a chain that does not own it, allows a later RX/TX value to overtake it, or changes stop/loss settlement.

### RX 6 — current-segment debt is consumed or reported before release (§3.4–§3.5; §6 RX 6)

**Test:** Owner-path cases for cut, abort, `Provider::stop`, cold change, loss, LateCommand, and a quiet stream. Use actual `RxChain` handoff/sample paths, plus existing bounded-drain/flush tests for the final stop and leftover cases.

**Assert:** Only the current segment’s entries are debt. Before its claim is returned, its streamer is closed, or the next segment begins, each entry is either proven consumed or transferred to `left`/reported exactly once by `finish`; no per-exit successor settlement is needed. Preserve the current drain deadline (latest debt instant + one receive call + allowance, with quiet-span floor), the single bounded flush when time remains, and existing loss/failure semantics. Flush stop failure leaves `Uncertain` and does not restore.

**Observe:** Per-entry device sample evidence, chain debt/left/latest, rejection rows, ordered `rx_stop`/flush/close/open/configure calls, `Streams` holder/phase, and subsequent segment start. The cut remains the logical publication boundary; drain samples at/after it are not published.

**Wrong behavior caught:** Claim/streamer is released with unowned debt; a successor starts before prior debt is resolved; a debt entry is duplicated or silently abandoned; loss causes more device calls; or drain/flush consumes the successor’s minimum lead.

### RX 7 — close/open invariant (§3.6; §6 RX 7)

**Test:** Test-build invariant checks at every stream close/open path: owner reconfigure; `release_unneeded`; control from-none open; rate-check failure close; open failure cleanup; prepare; and cleanup `close_streams`. Add negative assertion tests that deliberately invoke a guarded close/open with an invalid chain state/debt. Separately cover `abandon_unowned` cleanup and the detached-owner rule.

**Assert:** Every ordinary close/open call is reached only when that chain has `device ∈ {Off, Uncertain, Lost}` and empty debt; `left` may remain. Invalid state/debt panics in test builds. `abandon_unowned` is the documented cleanup exception, and cleanup does not close under a detached live owner.

**Observe:** Guard state at the device-call boundary, call log and panic result, `left` state, and whether the owner was detached. Exercise all listed callers rather than testing only the helper.

**Wrong behavior caught:** A streamer is reopened while the old segment is active or debt remains; a failure path bypasses the invariant; `left` is incorrectly treated as debt; or cleanup closes resources still owned by a detached thread.

### RX 8 — restore from latest only after safe finish (§3.7; §6 RX 8)

**Test:** `ur_24_a_left_and_latest_survive_close_open_and_chain_reuse`; `ur_24_a_left_stays_on_the_old_chain_after_channel_move`; `ur_24_provider_stop_skips_restore_for_left_on_this_chain`; `ur_26_provider_stop_restores_the_frequency_in_force_at_its_instant`; `ur_24_provider_stop_drains_after_s_then_restores_the_frequency_in_force_at_s`; `ur_24_provider_stop_skips_restore_when_flush_stop_fails`; `ur_24_provider_stop_records_restore_failure_and_stops_after_loss`; and `ur_24_a_parallel_rx_restore_checks_do_not_nest_chain_locks`.

**Assert:** Restore reflects the value in force at `S`, not the latest post-`S` value; `latest` remains after debt consumption. Restore occurs only after successful stop and when that channel has neither debt nor `left` on any chain and this chain is `Off`. A channel with `left` is not restored. Restore is ordered after drain/stop and before another Run’s prepare/configure can hide the result. Refusal/loss is recorded; after loss no later channel restore is attempted.

**Observe:** Per-channel fake device state and apply order before a new configure, `latest`/restore state, debt/left on every chain, `applied` restore rows and `rx_restore` rejections, and loss events.

**Wrong behavior caught:** `latest` disappears when consumed; post-`S` value wins; initial value is omitted; same-instant booking order is wrong; restore races ahead of drain/stop or another stream’s `left`; or code continues restoring after loss.

### RX 9 — regression envelope and timing boundaries (§6 RX 9)

**Test:** Preserve the D9b-2 unit/integration and differential coverage, adapting state observations to `RxChain` as needed. Keep the far-future-cut and equal-time-consumption cases, current-segment drain/flush tests, and the raw P4 A/C capture as observation-only.

**Assert:** A far-future cut does not make pre-cut receives one-packet; equal-time entries still need distinct qualifying samples after acceptance; multi-channel validity/overflow/drain samples remain off-link as before; host silence is distinct from stalled DDC flow; the deterministic RX timeline differential harness retains its behavioral verdict. No raw hardware observation is promoted to an automated phase verdict.

**Observe:** RX request sizes and `one_packet` flags, sample/debt logs, published block times/lengths/flags, FakeDevice DDC state, and differential comparison output. Hardware P4 remains a separately recorded observation.

**Wrong behavior caught:** A far-future cut changes normal packet reads, one sample consumes multiple equal-time entries, drain tails leak to the link, host silence is mistaken for DDC stall (or vice versa), or the refactor changes established timeline behavior.

## Existing-test disposition

### Keep behavior; migrate state setup/assertions to `RxChain` where they directly touch `RxDsp`

- `rx.rs::ur_24_equal_time_rx_entries_need_distinct_samples_after_acceptance`; `ur_24_multichannel_abort_keeps_overflow_and_drain_samples_off_the_link`; `ur_24_fake_ddc_distinguishes_host_silence_from_a_stalled_ddc`; `ur_24_drain_deadline_runs_one_bounded_flush_and_consumes_or_reports`; `ur_24_a_deadline_receive_uses_one_short_packet_and_preserves_successor_lead`; and `ur_24_a_far_future_cut_keeps_pre_cut_receives_multi_packet` remain behavior controls for RX 6/9. Direct `pending` construction/assertions become chain debt/left/sample-evidence assertions.
- `rx.rs::ur_24_a_cut_after_timed_rx_release_drains_before_its_successor` remains a current-segment drain test; route its setup through a genuinely streaming chain and retain the logical-cut, no-published-tail, consumption, and minimum-lead assertions.
- `rx.rs::ur_24_a_stop_decision_waits_for_a_gated_timed_rx_handover` and `ur_24_a_completed_rx_owner_serializes_a_late_frequency` remain the two lock-order controls. Gate/assert the chain lock and segment completion rather than `RxDsp` state; preserve the no-stop-before-accepted-handoff and no-apply-after-finish outcomes.
- `rx.rs::ur_24_provider_stop_drains_after_s_then_restores_the_frequency_in_force_at_s`, `ur_24_nonloss_rx_stop_failure_reports_pending_and_skips_restore`, `ur_24_provider_stop_skips_restore_when_flush_stop_fails`, `ur_24_a_loss_during_rx_flush_finishes_without_recovery_calls`, `ur_24_a_loss_during_rx_receive_finishes_without_recovery_calls`, and `ur_24_provider_stop_records_restore_failure_and_stops_after_loss` remain current-segment drain/restore coverage. Replace `stopped`/`pending`/`restore_ready` assertions with the specified chain state, debt/left, and `Restore` state.
- In `control.rs`, adapt `ur_24_a_frequency_inside_a_running_segment_is_given_timed`, `ur_24_a_stopped_rx_segment_does_not_receive_a_late_timed_frequency`, and `ur_24_a_frequency_of_a_channel_the_running_segment_does_not_hold_is_deferred` so “running/stopped” is represented by the real chain state, not only a planned/active `Streams` record or a manually set `stopped` bit.
- Adapt `control.rs::ur_26_provider_stop_restores_the_frequency_in_force_at_its_instant` to seed/read per-chain `latest`/`Restore`; keep its same-instant booking-order and initial-value assertions. Existing gain, transmit, schema, and timeline tests are not relaxed by this RX-frequency-only change.

### Change to test the new eligibility rule

- `control.rs::ur_24_a_successor_frequency_after_its_origin_stays_timed_during_restart`: release while configured but before begin must now observe held/no apply/no debt; begin the successor, run the next release pass, then assert one timed apply and sample consumption.
- `control.rs::ur_24_a_frequency_just_after_an_origin_is_given_timed_and_follows_the_configure`: currently configures but does not start the segment. Start through the owner, assert held before it is Armed, then timed handoff after begin; retain configure-before-timed ordering and `at = e`.
- `control.rs::ur_24_a_frequency_due_at_a_segment_origin_is_given_timed_and_the_configure_carries_it`: cover the origin boundary with a real owner begin. The configure may still carry the projected setting; the timed handoff must wait until the chain is Armed and then occur once.

### Remove as unreachable under L1; replace with the named sequence coverage

- `rx.rs::ur_24_a_shutdown_without_an_rx_stream_reports_pending_ddc` manually injects debt without a segment/owner. No production handoff can create that state under L1. Replace with RX 1/7 cleanup coverage for `abandon_unowned` and no device recovery call.
- `rx.rs::ur_24_a_pending_ddc_is_reported_when_provider_stop_cancels_await_release` and `ur_24_a_pending_ddc_is_reported_when_source_refusal_removes_move_destination` inject an orphan DDC entry while the destination has no running segment. Remove these settlement tests; RX 5 must instead use a real held control value and prove stop/refusal changes its target without creating debt, with stop/loss settlement still covered.
- `control.rs::ur_24_a_refused_successor_configuration_reports_pending_ddc_through_shutdown_and_reopen` and `ur_24_a_refused_successor_rx_start_drains_handed_ddc_before_shutdown` require a value to have been handed to a successor before it starts. Replace with configured-successor configure/start refusal cases asserting the command was still held, no DDC debt was created, and refusal follows RX 5’s current projection/no-channel path through shutdown/reopen.
- `rx.rs::ur_24_a_new_successor_obligation_is_settled_after_an_earlier_empty_settlement` depends on an unbegun successor owning debt and per-exit settlement state. Remove the obsolete settlement-marker case; RX 2’s two gated begin/release orders must prove the held value is neither lost nor handed twice.

Keep `control.rs::ur_24_a_cut_booked_after_the_release_leaves_the_entry_for_the_next_segment` as raw DDC characterization only: it manually calls the device, bypasses the RX owner, and records P4 A/C behavior. Do not use it as evidence that production uhd-control hands to an unbegun segment.

## Non-test implementation touchpoints (for the later code run)

The test-plan-only run does not edit these. The implementation must pair the behavior change with the note 44 §3.8 amendments to UR-24 in `design/18-uhd-radio.md` (Holding and release, recording, RX frequency/DDC, Consumption), MR-34 Ceiling in `design/09-mock-radio.md`, and the MA-1 Ceiling in UR-24 in `design/18-uhd-radio.md`, and the corresponding KNOWN_LIMITATIONS entry for owner-delay/head-of-line behavior. Keep accepted-spec DA-1 format and Changes rows; later run `rules.py --base main`. Do not add version bumps or changelog entries before the freeze.

## Plan summary

The highest-value new coverage is the controlled release-before-begin / begin-before-release race and the minimum-lead regression: together they prove a future segment cannot acquire old-chain debt and that the hold is released exactly once when its own chain becomes eligible. Keep current-segment drain, flush, loss, restore, equal-time, and differential coverage; remove only tests whose orphan successor debt becomes unreachable, replacing them with held-value retarget/cleanup tests. Use manual time and gates throughout, and make no assertion about physical late-tune phase. This file is the only deliverable for Run 1; stop for orchestrator review.
