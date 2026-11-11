# #89 test plan — the orchestrator's answers (before Sol's review)

- The plan's corrections 1–7 (A2's timing, the contract table's wording under A2/F3, the Detached refusal before the directory, N1's two extra places, K2/K3's rigs, N2 as text only, the `test_ea_16_errors` assertion) are accepted.
- Q1: accepted. `ssh-drop-tx.sh` gets `EZSDR_DROP_CUT=link|kill`; `kill` mode kills the server container instead of the ssh client and prints an E9 measurement line (time from the kill to the end of the tone on #2's monitor) instead of C1/C2. Records stay machine-made.
- Q2: accepted. After any abandon `close()` waits for the exit, reads the Manifest if it can, else returns `None`; the exception that caused the abandon has already reached the user at the call. `close()`'s own `finish` failing still raises `ProtocolError` as today.
- Q3: accepted. The two dropped tests are covered by today's EOF tests (`ea_15_finish_and_disconnect`, `ea_15_every_exit_writes_the_manifest`) and by X2/X3; say so in the plan.
- Q4: accepted. A bounded wait whose expiry is the failure is allowed; keep each such bound as short as the test allows (a correct run passes in well under a second), and name the rows killed only by the bound in the report.

## After Sol's review (the amended plan's new questions)

- New Q1, accepted narrowly. Add a crate-private `Server::watched(config, Arc<Watch>)` constructor. The counters `len()` and `contended` are `#[cfg(test)]` only. In production the removal path stays a plain `lock()`. The `try_lock` probe that counts contention also lives behind `#[cfg(test)]`, so release builds carry no test scaffolding (AGENTS §9 Maintainability 5).
- New Q2, agreed. Make the order structural with one helper: it removes the signal under the watch's lock, then hands back the live Session or child to finish. Every removal site uses it. Review checks the order; no new Kernel surface is added.
- New Q3, agreed. The inferred entry evidence (the client-side relay and the 1 s gap; otherwise INCONCLUSIVE) is enough. The server gets no production logging just for the bench.
