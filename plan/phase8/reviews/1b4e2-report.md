STATUS: done

# Phase 8 1b-4e-2 — 実装・検証報告

作業ツリーは未コミット・未ステージのまま。`git add`、commit、push、`cargo fmt` はしていない。

## 1. 裁定 A1–A10 と D1–D6

### D1–D6

| 決定 | 実装 |
|---|---|
| D1 / A2 | owner が `active` / `completed` と `busy_seq` を記録し、必要のない idle streamer だけを `release_unneeded` で閉じる。source が moved channel を物理的に解放した後も、同じ move の configure/refusal が確定するまで destination は待つ。`crates/ezsdr-radio-uhd/src/provider/core.rs:166-191,376-409,968-1096`、`rx.rs:285-373`、`tx.rs:286-384,704-773`。 |
| D2 / A4 / A6 | conflict wait は `await_release` が loss/stop/refusal/plan change を毎回読み直す。古い `Planned` が最新 plan に無ければ device call をせず中断し、待ち後の accepted Cold で segment が変わった場合も中断する。`core.rs:1000-1096`; 対応 unit は `core.rs:1307-1538`。owner close 中も次 Action を booking できる gate test は `control.rs:1437-1483`。 |
| D3 | from-none open は他 stream の recorded set が moved channel を持つ場合に使わず、owner に委譲する。`crates/ezsdr-radio-uhd/src/provider/control.rs:968-1032`; S0/S3 は `tests/fake.rs:5837,5951`。 |
| D4 / A8 | `Device` に必須の `rx_close` / `tx_close` を加え、FakeDevice と UhdDevice が stream 単位で実装。UhdDevice は open/close を同じ control mutex で直列化し、map 参照を外しても in-flight Arc を保つ。`device.rs:189-219,1125,1431,1683-1693`; `uhd.rs:909-930,1005-1026,1085-1102`; feature tests `uhd.rs:1340-1368`。 |
| D5 / A5 | move に結び付く device refusal は両 line を拒否し、TX clocks を両方再計算する。RX/TX owner は refusal を記帳してから `busy` を解除する。`core.rs:285-304,457-493`; `rx.rs:307-319`; `tx.rs:755-767`; recovery tests `tests/fake.rs:4955-4976`; both-TX-clock test `core.rs:1609-1635`。 |
| D6 / A9 / A10 | `book_cold` が両 projection/configuration を同じ `seq/e` で予約し、`applied` と `timing` に move row を記録する。時刻/floor と不可能な move の拒否は shared `Lines` にある。`control.rs:794-900`; `timeline.rs:482-505,545-630`; parity は `crates/ezsdr-acceptance/tests/parity.rs:1323-1382`。 |

### 裁定 A1–A10

裁定の対応表と実施テスト一覧は [`test-plan-v2.md`](1b4e2-test-plan.md) §§2.1–2.13、特に lines 7–20 と 156–168。

| 裁定 | 対応 |
|---|---|
| A1 | per-stream prune watermark が move の両 item/record を両 line の境界が進むまで保持し、片側 prune 後の source Stop/Start refusal で destination segment origin も更新する。両 prune 完了時の release floor を unpruned Lines と比較。`crates/ezsdr-radio/src/timeline.rs:438-443,512-531,673-711`; tests `tests/timeline.rs:783-842`; `design/07-radio-model.md:354-358`。 |
| A2 | streamer が無くなった時点だけで解放とはせず、source の同 move configure/refusal 完了まで待つ。cut-at-origin TX intermediary は開かず、repeat tail は separate one-shot tail test で検証。`core.rs:404-409,1000-1096`; `tests/fake.rs:4975,5951,5981,6179`。 |
| A3 | successful open 後の UR-13 refusal は同じ caller が直ちに close し、`StreamState` を `Unknown` / `configured=None` にする。`core.rs:431-448,945-963`; `tests/fake.rs:4929`; state assertion `core.rs:1555`。`design/18-uhd-radio.md:207,285-292`。 |
| A4 | wait 後に latest plan を照合し、removed/refused/changed plan は abort（refusal ではない）。Stop による中断は RX `from` / TX `at` を進めず、`COMMAND_REJECTED` を出さない。さらに accepted Cold が wait 中に segment を変える系列を追加。`core.rs:1000-1025,1034-1096`; `rx.rs:285-319`; tests `core.rs:1435-1554`, `rx.rs:750-786`。 |
| A5 | source/destination open・configure refusal を両 line に反映し、TX clock の cut/origin を両側で同期。4 recovery tests は `tests/fake.rs:4955-4976`、clock test は `core.rs:1609-1635` と `timeline.rs:494-505`。 |
| A6 | wait の状態到達を probe/gate で同期。owner close gate 中に control booking が完了すること、loss/stop wait exit の各 thread join を確認。`control.rs:1437-1483`; `test-plan-v2.md:60-80,102-116`。 |
| A7 | `tx_send n=` を sample 数として扱う。old one-shot tail test は cut 前の最後の held sample を EOB call に含むことを確認し、別の repeat test は cut をまたぐ旧 waveform と cut 前に予約した destination-clock burst を確認。`tests/fake.rs:6117-6177`; `6179-6240`; `P509/P518`。S1/S2 apply/sample の訂正も同表の §2.3/§2.5 に記録。 |
| A8 | `UhdDevice` open/close の同一 control mutex と Arc 遅延 drop を feature tests で確認。`uhd.rs:909-930,1005-1026,1085-1102,1340-1368`; feature gate 結果は §4。 |
| A9 | same-stream/direction/channel membership/no-line など拒否理由・projection を確認し、move row は Mock の理由と一致。`timeline.rs:545-630`; `control.rs:794-900,1525-1605`; `tests/fake.rs:5864,1486-1568`。 |
| A10 | move の `applied`/`timing` row と clocks/segments/sequence rank/updates を Mock/UHD parity で比較。move `ready == booked_at` を S3 でも明示 assertion。`control.rs:887`; `tests/fake.rs:5951-5978`; `parity.rs:1323-1382`。 |

## 2. テスト、S0、parity

系列と assert の完全な表は `test-plan-v2.md:32-145`。代表的な配置：timeline A1 tests `crates/ezsdr-radio/tests/timeline.rs:783-842`; RX/TX move, refusal/recovery, S8/S10, tail/repeat は `crates/ezsdr-radio-uhd/tests/fake.rs:4955-6300`; waits and new failure paths are `crates/ezsdr-radio-uhd/src/provider/core.rs:1257-1635`, `rx.rs:734-750`, `tx.rs:2476-2500`; VH-8 is `crates/ezsdr-acceptance/tests/parity.rs:1323-1382`.

- S0 の3 probe は実装前に失敗した。RX/TX は元 streamer が close されない、idle destination は必要な source streamer が close されないことを確認（`s0-rx.log:7-17`, `s0-tx.log:7-17`, `s0-idle.log:7-17`）。Formal test の idle 形には `start_rx` を加えたため、最終形を元 HEAD で再実行していない（`test-plan-v2.md:49`）。
- 系列の対応：S0 は `ur_34_a_join_after_a_leave_closes_the_source_s_streamer_first`, `ur_34_a_transmit_join_after_a_leave_closes_the_source_s_streamer_first`, `ur_34_an_idle_destination_leaves_its_join_to_the_owner_when_another_streamer_holds_the_channel`（`fake.rs:5782,5809,5837`）。S1–S4 は `fake.rs:5889,5927,5951,5981`。S5 は `fake.rs:6117,6179`。S6 は拒否/回復4系列 `fake.rs:4955-4976` と `core.rs:1609-1635`。S7 の wait/loss/stop 系列は `test-plan-v2.md:102-116`、S8 `fake.rs:6004-6035`、S9/S10 `control.rs:1525,1544` と `fake.rs:6038-6064`、S11 は `parity.rs:1323,1354`。
- VH-8 の move-between-streams / empty-destination は両方 pass。Mock/UHD の divergence なし、`#[ignore]` なし（`parity.rs:1323-1382`）。
- 新規 `ur_34_*`、A1 prune pair、VH-8 move parity は3回連続で pass: per run 22 lib tests, 21 fake tests, 2 timeline tests, 2 parity tests（`repeated-focused-tests.log`）。

## 3. Gate 結果

- `timeout 300 cargo test --workspace --no-fail-fast`: pass, 最終状態で1回。
- `cargo clippy --workspace --all-targets -- -D warnings`: pass。
- UHD 4.10 container: feature clippy pass; `cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`: 246 lib tests + 5 API tests pass。
- `cargo build -p ezsdr-server`: pass; Python 3.14 suite 47/47, Python 3.9 suite 47/47 (`PYTHONDONTWRITEBYTECODE=1`)。
- `PYTHONDONTWRITEBYTECODE=1 timeout 900 python3 plan/maintenance/tools/rules.py --base HEAD --renames /home/komatsu/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e2/rules-renames.txt`: 0 problems; `rules.py --self-test`: pass; `git diff --check`: clean.
- Remote mutations: final selected 29 maintenance rows pass (27 killed, P358/P494 equivalent; explanations in `mutations.json`). P490 also killed in a separate rerun. P521/P522 (lost owner configure unlock) killed. Phase7 U40 was killed in the earlier combined run. The earlier 44-row batch and its row-level log are in `impl.log:719-770`; P371's old equivalence was removed after the corrected fixture and final mutation now kills it. P269/P492 were timeout-killed (not survivors). An initial remote baseline failed from a test compile error (`Option<Planned>` treated as `Result`); fixed before the final mutation batches, which report no baseline failures.
- A local attempt to run the UHD feature test outside the UHD 4.10 Docker image failed as expected because `libuhd`/`UHD_LIB_DIR` was unavailable. The required Docker feature run above passed.

## 4. セルフレビュー3回と扱い

All three fresh Luna reviews are recorded at `review-1.log`, `review-2.log`, `review-3.log`.

| Round | Findings | Treatment |
|---|---|---|
| 1 | 5 P1, 1 P2 (`review-1.log:206-237`) | P1s fixed: device-loss/open-close gate + test; A1 source release and destination-origin test; P491 finite unfinished segment test; Stop-vs-refusal assertions; held tail assertion plus P509. P2 UR-13 wording now states immediate close and no close on loss (`design/18-uhd-radio.md:207`). |
| 2 | 2 P1, 2 P2 (`review-2.log:6492-6507`) | Stale-plan fallback removed; P449 filter retargeted and killed. Added `StreamState` unknown assertion/P514. Added A6 booking mutation P515 and TX report-before-close mutation P517. Rejected the request for an additional remote mutation of the native UHD mutex: the approved remote mutation image has no libuhd and the mutator has no UHD-feature option; the UHD feature API test remains the verification. |
| 3 | 1 P1, 3 P2, 1 P3 (`review-3.log:532-553`) | P1 fixed after review: RX/TX drop `streams` before `device_failed` (`rx.rs:313-318`, `tx.rs:761-766`); `Core::device_lost` asserts line-lock-free in tests (`core.rs:753-756`); RX/TX lost-owner regressions and P521/P522 added (`rx.rs:734-747`, `tx.rs:2476-2492`). P2 A4 fixed with a non-refused plan-change wait test/P520 (`core.rs:1491-1524`); P2 A7 fixed with cut-crossing repeat and prebooked destination burst/P518 (`fake.rs:6179-6240`); P2 S3 fixed by asserting `ready == booked_at`/P519 (`fake.rs:5951-5978`). P3 design wording narrowed (`design/18-uhd-radio.md:270`). **The post-round-3 P1 fix and its tests were not independently reviewed; no fourth review was run, per the three-review limit.** |

Review findings counted by mechanism: one `mech:planned-state-read-as-actual` (stale `Planned` reused when the latest line has no segment) and two `mech:stream-lifecycle-scattered` (loss vs streamer lifecycle and owner loss error re-entering the stream lock); the existing design note is spec 22. The remaining findings were test, mutation-filter, or wording gaps, not counted as mechanism bugs. No issue was filed.

## 5. 差分サイズと穴

- `git diff --stat HEAD`: 19 files, +2,785/−222 lines. Rust implementation/tests: 14 files, +2,361/−152 lines, about 861 additions above the 1,500-line guide. The increase is chiefly the end-to-end owner/wait/failure sequences, differential parity, and post-review regression tests/mutation rows; they were retained because they pin distinct lifecycle cases.
- No hardware bench: real X300 inter-stream continuity remains INFERRED. The v3 comparison, move `effective != e` reachability, `changed/reconcile` i64 overflow, A10 wait-past-origin and unread-report-on-close behavior remain the holes listed in `test-plan-v2.md:187-196`.
- The native UHD close/open mutex has feature coverage but no remote mutation row for the reason above. The test-only lock handoff and post-loss owner error were covered by P510/P511/P521/P522.
- Moves do not change Python API or Session verbs. No `git add`, commit, push, or formatting was done.

---

STATUS: done

# 1b-4e-2 修正の回 1 — 報告

作業ツリーの既存 1b-4e-2 変更は維持し、今回の修正も未 commit・未 stage。`git add`、commit、push、`cargo fmt`、セルフレビューは行っていない。

## 採用した指摘と修正

1. **Sol 1 — 連鎖 move の prune**: `crates/ezsdr-radio/src/timeline.rs:687-720` で各 request から effective boundary を作り、全 move の制約を前ラウンドの snapshot に適用して不動点まで反復する。floor は双方の effective boundary が move を越え、双方の item がまだある場合だけ固定する。B→C の release を読む前に両 item の存在を確かめ、片側だけ畳んで `Line::released` の `expect` に入る経路を作らない。回帰は `crates/ezsdr-radio/tests/timeline.rs:847-895`: A→B `e=10`、B→C `e=20`、B/C だけ `30` で prune、両順序で unpruned `Lines` の全 plan と比較し、2 item/record と source release が残ること、move 拒否後 C が halt することを確認。Normative と2 test名を `design/07-radio-model.md:357,361` に反映。
2. **Sol 2 — from-none の先行 segment 追越し**: `needed_elsewhere` を削除し、`needed_before(stream, set, i64::MAX)` を `release_unneeded` (`crates/ezsdr-radio-uhd/src/provider/core.rs:953-962`) と from-none 判定 (`control.rs:968-971`) で共用。別 stream の未処理 segment が channel を要すれば control は開かず owner に任せる。`control.rs:1439-1537` の probe test は A 解放後・B open 前に C join を予約し、booking 中 device call なし、A close→B open→B close→C open を確認。UR-25 と Test 行を `design/18-uhd-radio.md:265-266,284,286` に反映。
3. **Sol 3 — P358/P493**: P358 のテストを `crates/ezsdr-radio-uhd/src/provider/tx.rs:1127-1166` に拡張し、後続 Cold を予約して refused TX segment が正の長さで plan に残ることを確認してから owner switch を実行。P493 用に `core.rs:1594-1653` を追加。TX End 後、後続 Cold により期待 segment が plan に完全一致で残ることを確認し、wait が止まり streamer を開かないことを検証。両 mutation の `equivalent` を外して新テストに retarget。
4. **Sol 4 — P494**: RX cut-at-origin guard を `core.rs:992` から削除。planner がその segment を plan から除く不変条件に従い、P494 mutation row も削除。既存の RX Stop wait test は UHD suite で pass。
5. **Sol 5 — S8**: `crates/ezsdr-radio-uhd/tests/fake.rs:6004-6045` で、移動後 destination の untimed configure が直前の timed 10 MHz を引き継ぐこと、FakeDevice の返す実効値が later update 前に10 MHz、後に20 MHzであることを追加 assertion。
6. **Opus minor — 重複 helper**: `needed_elsewhere` を削除。2 callers は既存 `needed_before(..., i64::MAX)` に統合（`core.rs:375-397,961-962`, `control.rs:968-971`）。挙動を変える新しい待機機構は追加していない。

TX Stop を timeline に予約する変更、UR-25 の文量整理は裁定どおり行っていない。

## 検証

- `timeout 300 cargo test -p ezsdr-radio`: pass（timeline 53 を含む）。
- `timeout 300 cargo test -p ezsdr-radio-uhd`: pass（lib 240、fake 269）。
- `timeout 300 cargo test -p ezsdr-acceptance --test parity`: 20/20 pass。
- `timeout 300 cargo clippy --workspace --all-targets -- -D warnings`: pass。
- 新規/変更した timeline 2 tests、from-none、TX Stop/P493、refused-TX/P358、S8 の各 test を連続3回 pass（`fix1-focused.log`）。
- `rules.py --base HEAD --renames /home/komatsu/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e2/rules-renames.txt`: 0 problems。`--self-test`: pass。`git diff --check`: clean。
- maintenance mutation pattern 全878行を確認し、各 `old` / `old2` が source にちょうど1回一致。

### Remote mutation — 14/14 killed

実行 command は `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:3,sim03:3 P102 P103 P108 P252 P253 P278 P358 P489 P493 P496 P504 P508 P523 P524`。

| ID | 結果 |
|---|---|
| P102 | killed |
| P103 | killed |
| P108 | killed |
| P252 | killed |
| P253 | killed |
| P278 | killed |
| P358 | killed |
| P489 | killed |
| P493 | killed |
| P496 | killed |
| P504 | killed |
| P508 | killed |
| P523 | killed |
| P524 | killed |

P252/P253/P496 は分割した from-none predicate に、P489/P508 は統合した `needed_before` predicate に pattern/filter を retarget している。

## 途中の失敗と未実施

初回の新規 unit-test build は thread closure の返却型と `Planned` の debug formatting で compile error。修正後通過した。初回 clippy は bool literal 比較3箇所を指摘したため `assert!` に直し、最終 clippy は pass。最初の `rules.py` は UR-25 の追加で1行が1,500文字を越え、古い P252/P253 pattern も不一致だった。仕様文を複数行に折り、mutation を retarget して再実行し pass。

Orchestrator gate の `cargo test --workspace`、UHD feature Docker gate、Python suite、実機 bench はこの修正回では実行していない。セルフレビューは依頼どおり実行せず、独立レビュー待ち。実機上の inter-stream 挙動は未計測。
