I found a teardown path the new shim test does not check: a failed native stream open drops its temporary streamer before the error reaches `Core::publish_loss`. I’m checking the handle ownership and mid-run ordering to see whether any native calls can precede `mark_lost`.
The temporary-handle concern is not a blocker: UHD 4.10 assigns the native streamer only after `get_*_stream` succeeds, so a failed open frees an unattached wrapper. All 681 library tests, 613 fake integration tests, and both feature-`uhd` classification tests passed.
## Verdict: PASS

What this change does: preserves device loss through prepare/arm refusals and owner recovery, avoids closing after a lost rate read, and limits error-47 classification to the two native stream opens.

**No major or minor findings.**

**VERIFIED**
- Publication remains idempotent; non-lost refusals retain their behavior.
- Lost rate reads preserve the device error, cache nothing, and skip close.
- A booked Stop no longer suppresses a recovery-open loss.
- Both actual native call literals match the classifier; negative streaming-call cases remain non-lost.
- Design changes match the implementation. New and retargeted mutation anchors/killers were inspected statically.
- Tests passed: **681 library**, **613 fake integration**, **2 feature-`uhd` classification** tests.
- Working tree unchanged; diff whitespace check passed.

**INFERRED:** dead-link behavior on physical hardware; no bench run performed.

Not checked: mutation execution or kill effectiveness, per instruction. No formatting or files changed.
