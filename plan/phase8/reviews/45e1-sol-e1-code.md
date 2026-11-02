> plan · gpt-6.1-sol
# REVISE

The simulation has gaps that can hide protocol bugs or certify behaviour that differs from production. These should be fixed before E3.

## P1 — Major

### 1. Unread `left` values are forgotten, making restore falsely eligible
**Category (b), (d), (e)**  
`crates/ezsdr-radio-uhd/src/brain/world.rs:1219–1223,1275–1285`

**Counterexample:** Apply v with an effective instant beyond the flush window → stop → report v as `Left` → flush expires without consuming v → successful flush stop → request Restore.

`retire_unread` deletes v from the cell even though it remains in the device FIFO. Restore now passes both `decide` and I15. A later sample can apply v after the restore.

Production preserves these entries in `RxChain::left` (`provider/core.rs:367–370`); note 44 §3.4 explicitly preserves them across segments. Tests at `world.rs:2069–2092` and mutation rows E1-107/E1-118 instead enforce their deletion.

**Fix:** Preserve unread leftovers until consumption or loss. Represent “flush already attempted” separately so persistent leftovers do not cause repeated flushes. Retarget the tests and mutation rows.

### 2. Sending Close does not close acceptance
**Category (a), (d)**  
`crates/ezsdr-radio-uhd/src/brain/core.rs:495–507,546–559,767–771`

**Counterexample:**
```text
Streaming, no values
Brain handles Close → records outstanding Close, leaves phase Streaming
Brain handles Release(v) → reservation succeeds
S takes v → chain pinned
R executes Close
```

The handle closes while its chain is pinned. `reserve` ignores the outstanding lifecycle command, and I3 checks Close only when routed—not when it subsequently executes (`oracle.rs:201–211`). The draft-7 outstanding-command mechanism therefore protects later lifecycle decisions, but not later reservations.

This also contradicts note 45 §4.5: closing/replacing a streamer must first close acceptance.

**Fix:** Implement the closing/retiring transition before issuing Close, and require the appropriate stopped/quiesced state. Add the sequence above as a regression test.

### 3. A simulated brain panic does not kill the brain
**Category (a), (b), (d)**  
`crates/ezsdr-radio-uhd/src/brain/world.rs:528–532,640–656,749–761`

**Counterexample:** Panic at step k → execute the four guard operations → deliver an executor completion or Shutdown → `BStep` remains enabled → the brain handles it and writes dispositions, clears outstanding work, or sends termination commands.

Consequently, case 6 can complete using a brain that would no longer exist. This contradicts §10 draft 5’s explicit “dead brain writes no dispositions” decision and weakens the independence of executor cleanup.

**Fix:** Give the brain a terminal dead state after its unwind guard. Stop normal brain processing permanently; define post-panic accounting and executor termination without depending on its inbox being serviced.

### 4. A restart queued behind a stopped tail never starts
**Category (b), (e)**  
`crates/ezsdr-radio-uhd/src/brain/model.rs:150–159,174–201`

**Counterexample:** Start at 0 → produce samples → stop at 4 → start at 8 while the stopped tail remains → receive the entire tail → advance through tick 8.

`start` stores `then = Some(8)`, but neither `recv` nor `tick` ever promotes `then`, clears `end`, or advances the tail cursor. No samples from the new start arrive. Meanwhile `open_flow` has already installed its DSP flow, so DSP application can proceed without corresponding received samples.

FakeDevice performs the tail-to-next-start transition at `device.rs:1627–1640`. U15d and E1-27 check only that `then` is assigned, not that the deferred start subsequently works.

**Fix:** Model the complete stopped-tail transition, including late-start behaviour when applicable. Extend U15d through delivery of the next segment’s samples.

### 5. Consumption identities are reconstructed from intent, not observed device facts
**Category (b), (d)**  
`crates/ezsdr-radio-uhd/src/brain/world.rs:352–370,1136–1142,1316–1331`

**Counterexample:** A command reserved for channel 0 is mistakenly applied to channel 1 → both channels produce sample 8 → R0 records consumption of v at sample 8.

`sync` iterates device DSP logs but discards their actual channel. Both application and receive evidence obtain their channel, segment and epoch from the same reservation through `ident`. I4 can therefore see matching identities despite the write reaching the wrong physical channel.

Channel moves compound this: `move_channel` changes cell ownership only (`world.rs:893–898`), leaving device streamer channel bindings unchanged; consumption nevertheless uses the new cell channel set.

**Fix:** Preserve actual channel and receive-buffer provenance in device/application evidence. Compare those observations against reservation intent. Model channel movement through the necessary streamer transitions rather than changing only cell channel sets.

### 6. Full case 7b cannot exercise its required failure—and the sweep still succeeds
**Category (e)**  
`crates/ezsdr-radio-uhd/src/brain/cases.rs:264–267,567–588`

**Counterexample:** Run `case7b_flush_final_stop_restore_walks` with the full configuration.

`call_faults` is enabled only for `Variant::Micro`, so the full case cannot execute its failed final flush stop. W7b2 and W7b3 are unreachable. Nevertheless, `sweep` merely prints witness coverage and asserts that no walked execution failed.

The owner approved bounded searches and random walks, not removal of the failure branch defining this case.

**Fix:** Enable the required fault in every relevant variant. Require the declared witness union across the designated coverage runs, independently of whether exploration is exhaustive. Missing coverage must be reported as unmet—not a successful case sweep.

## P2 — Minor

### 7. The key merges states whose future witness results differ
**Category (c), (e)**  
`crates/ezsdr-radio-uhd/src/brain/world.rs:1409–1433`

The dropped fields are not all passive history. Future witness generation reads `shortened` (`world.rs:1133`) and previous witness bits (`world.rs:651,861`).

**Counterexample:** Reserve v with e=4 and cut=16. Compare:
```text
A: S.take(v) → Stop(6)
B: Stop(6) → S.take(v)
```
After the corresponding brain operations, the protocol state and revision can agree, but only A records `shortened[v]=6`. Later consumption at sample 6 produces W1c only from A. Pruning A after visiting B loses that witness; observing witnesses before pruning cannot recover a witness that has not happened yet.

`harness_key_ignores_history` explicitly asserts equality after changing this field (`world.rs:1781–1793`).

**Fix:** Include witness-automaton state that affects future witness detection in the key, or propagate it with a separately justified coverage algorithm.

### 8. Epilogue expiry fabricates early deadline observations
**Category (b), (d), (e)**  
`crates/ezsdr-radio-uhd/src/brain/world.rs:527,631–638`

At epilogue end, a timer scheduled for 100 can be delivered at tick 16; `harness_timer_beyond_end_is_observed_at_end` explicitly requires this (`world.rs:1925–1934`).

**Counterexample:** End the simulation at 16 with an active drain deadline of 100 → deliver that timer → process `TimerFired` → stop and complete. The reported deadline-driven progress occurred on an input that production could not yet receive.

I do not consider this part of draft 5 a faithful implementation of bounded observation.

**Fix:** Keep deadline delivery tied to virtual time. Extend time without generating samples if needed, or classify the remaining wait as incomplete; do not count an early synthetic timer as a real deadline observation.

## Notes on §10 and maintainability

- The two owner decisions permit explicitly incomplete exploration. The bounded-search reporting follows that distinction; it does not establish completeness.
- Tracking outstanding lifecycle work is justified, but finding 2 shows that the protection is incomplete.
- Several mutation rows now reinforce model simplifications rather than production semantics; their kills cannot establish fidelity.
- `world.rs:1623–1639,2260–2287` contains additional handwritten DFS loops outside the generic explorer. These bypass its transition/progress checks. Prefer the existing explorer with focused checks rather than maintaining another search path.

**Not checked:** No edits, formatting, builds, tests or mutation runs were performed. Runtime results, E2 conformance and performance were not verified. The holdout directory was not accessed.
rc=0
