Before reading the diff, my failure scenarios are:

- Configure completes while Stop, loss, or `Provider::stop` arrives: preserve mark publication, claim release, rejection/note order, and the next-turn cancellation before payload.
- Configure refusal leaves joined pieces behind: preserve KL-38, retry of the same generation, value-rule busy time, and exact refusal text.
- Reopen succeeds or fails during Stop/loss: preserve both loss reads, the retained/cleared mark, ACK-derived front, clock ending, and cancellation order.
- Tokens survive receipt-side pruning, Stop, switches, and loss without being removed or replaced before completion; every production completion must match in both debug and release.
- Send prerequisites must not reject valid continuations or Mock sends, or alter KL-41/42.
- The shape must leave room for C1 resegmentation, C2 sent-based outcomes, C6 footprint shrinking, and E3 shared segmentation without another independent lifecycle state.
The raw trace archive supports the reported comparison: 3,614 groups were stable across four base runs; one differs in two new runs, leaving 3,613 matching groups after the disclosed exclusion. The characterization snapshots were not changed in E2b-2.

I’m checking one concrete gap: R1 expects a panic from a `debug_assert!`, but the test also runs in release builds, where that assertion is normally disabled.
## Verdict: **PASS WITH CHANGES**

Configure/Reopen now use the book’s token protocol. No production behavior regression or reachable token-check panic found by inspection.

### Minor — release test fails **VERIFIED**
`crates/ezsdr-radio-uhd/src/provider/tx/tests/e2b.rs:644–648` expects a panic from the `debug_assert!` at `provider/tx.rs:655`.

In release builds that assertion disappears, so the test fails: **“test did not panic as expected.”** Reproduced on sim03:
```sh
cargo test --release -p ezsdr-radio-uhd --lib e2b_r1_a_first_send
```
**Fix:** gate this debug-only test with `#[cfg(debug_assertions)]`.

### Review checks
- **VERIFIED:** E2b-1 snapshots remain unchanged; they compare complete ordered arrays within the documented exclusions. All 17 E2b tests passed independently in debug.
- **VERIFIED:** raw traces support **3,613 matching stable groups** after the disclosed timing-sensitive exclusion—not unconditional trace equality.
- Critical-section order, loss reads, mark publication, refusal texts, Stop/shutdown notes, KL-38/41/42 and Mock wiring remain consistent with the base.
- New mutation rows target the intended failures; the supplied full-run log confirms **2171/2172**, with only known survivor G09.
- **INFERRED:** no new scheduling-dependent verdict or structural blocker for C1/C2/C6/E3. Deferring Close tokens remains reasonable.

Not rerun: full gate, mutation suite or hardware tests. No files changed.
