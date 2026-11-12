独立に先に確認する失敗シナリオ：
- assertion がなくても、signal・出力欠落・build/test timeout・別テストの marker を kill に数える。
- timing 再実行が、同じ run の本物の失敗や再実行中の別の失敗を消す／本物の kill を ERROR に変える。
- 結果欄の名前衝突、欠落・重複した行、既知 survivor の扱いで未決着の行を gate が通す。
- e2e が fixture 未選択・環境変数未伝達でも通る、または判定だけを見て marker・signal・panic 回数を検証しない。
- 生成時の上限が Running assert より前に分類されず、cleanup/join の情報も取り逃す。

この観点を固定してから diff と報告を読みます。
遠隔の文字列 probe で、gate が「timing failure → 再実行で本物の failure → 次で pass」を通すことを確認しました。また workspace の別 binary が signal で死んでも、timing テストの再実行成功で失敗理由を全部消します。既存 self-test はどちらも検出しません。
## Verdict: FAIL

The fixed-field result parser fixes the `": killed"` ambiguity. The ordinary §6.3 verdict cases look correct. Three major gaps remain.

### Major

1. **Gate retries a genuine failure until it passes** — **VERIFIED**  
   `plan/maintenance/tools/gate.py:409–416`  
   Sequence: timing-marked failure → serial rerun fails **without** a timing marker → second rerun passes. The gate clears all failure reasons and passes. Only another timing-inconclusive result should permit another attempt; a conclusive failure must stop immediately. Confirmed by a remote string probe.

2. **Retry eligibility can erase unrelated failures** — **VERIFIED**  
   `plan/maintenance/tools/gate.py:404–416`; `plan/maintenance/tools/mutate.py:205–208`  
   Both compare failure **counts**, not failed-test identities. A marker naming test A plus an assertion failure in B can therefore rerun only A; A passes → gate passes / mutant becomes `SURVIVED`.  
   Separately, workspace output containing one timing failure **and another binary’s SIGKILL** passes after the timing test passes: `excuse_timing` discards the original signal failure too. Both counterexamples reproduced remotely. Match markers to actual failures and retain independent infra/signal failures.

3. **`spec_run` omits creation-time cleanup/join classification required in M** — **VERIFIED omission; INFERRED runtime sequence**  
   `crates/ezsdr-radio-uhd/tests/common/mod.rs:473–480`  
   It always supplies `false, &[]` to `creation_limit`, ignoring the Manifest. Sequence: creation fails at arm → cleanup exceeds RS-8a → helper returns the ended Run → caller reads a missing section and panics → mutant is counted killed without the required marker. Kernel records this timeout separately from the termination reason (`crates/ezsdr-kernel/src/run.rs:584–588`).  
   The brief explicitly includes `spec_run` in M; this is not an authorized stage-2a deferral. Preserve expected-ended callers while inspecting creation’s Manifest, or obtain an explicit scope change.

### Minor

4. **Mutation rerun accepts a pass still carrying a timing marker** — **VERIFIED**  
   `plan/maintenance/tools/mutate.py:215–217`  
   `returncode == 0` plus a positive test count yields `SURVIVED`, even with `EZSDR-TIMING:` present. Q16 requires a marker-free pass. Reproduced remotely.

5. **Rerun diagnostics lose the actual error** — **VERIFIED**  
   `plan/maintenance/tools/mutate.py:183–190,264–266`  
   Any rerun ERROR becomes `"why": "re-run unsettled"`, including a signal, build timeout or infra marker. Return the decisive rerun’s reason rather than diagnosing from the original invocation.

### Other checks

- G21, C06, P1119 and E2A2A-01 preserve their intended assertions; their changed killers are appropriate.
- Marker writes use one direct `write_all`; real-clock reporting is env-gated.
- E2E exercises the mutation-image path, but not gate’s actual retry path. Fixture 3 also checks marker/panic counts without requiring KC-12a or `FAILED` (`mutate.py:277,285–292`).

Not checked: full gate or mutation-image E2E rerun. Remote string probes and both tooling self-tests ran; no repository edits or builds.
