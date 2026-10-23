# 1b-7b：実装の指示書（`RX_OVERFLOW` の値を語彙の 1 か所で決める：D2）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `7d9b74d`，tree はきれい）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-23-phase8-1b7b/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`.*.orig` など）は読むだけ．書き換えない．orchestrator が動かしている見張りのプロセス（`watchdog_proc.py`，`mutation_watchdog.py`）は止めない．自分で起こした背景の job だけを片付ける．`git stash` を使わない．**

## 1. 背景

設計メモ `plan/phase8/36-radio-report-rules.md` を最初に全部読むこと．この段階はその **決定 4・5**（1b-7b，D2）だけ．決定 1・2（held burst の取り消し）は 1b-7a で済み，`ezsdr_radio::CancelReason` と `cancellation_reports`（`crates/ezsdr-radio/src/lib.rs`）がその形の手本になる．

- **決定 4**：`RxOverflowPayload` から `restart_gap_ns` を削除し，`{ cause, lost }` にする．hot path の形は 17 byte → 9 byte（byte 0 が cause，1〜8 が `lost` の little-endian u64）．旧 17 byte と `restart_gap_ns` を持つ旧 object は拒否する（互換の decoder も版上げも足さない．AGENTS.md §6）．envelope の `radio.timing.overflow_restart_gap_ns`（Mock が overrun の空白を作る model の値）は別物で，残す．
- **決定 5**：report（UHD の device の report，Mock の fault）の後の，最初の飛びのある block ごとに `RX_OVERFLOW` は 1 個．`lost` はその block の飛び全体．`cause` は代表値で，その前の report に overrun が 1 つでもあれば `overrun`，sequence だけなら `sequence`．時刻は今と同じ，最初に失った sample．飛びの無い block が来たら report は event を出さずに消える．report の後に block が来なければ event は出ない．**例外**：Mock の back-pressure の overrun（MR-19）は今のまま，捨てた時点でその時に足した損失を報告し，共通の関数を通らない．UHD は link の `Full` を数えるだけ（UR-17）．
- 「その block の前の report の一覧と block の `lost` → event 0 個か 1 個」の関数を `ezsdr_radio` に 1 つ置き，両 Provider が呼ぶ（決定 2 と同じ形）．
- 変えないもの：report の無い飛び（UHD で `SEQ_DISCONTINUITY` だけ，event なし），`ALIGNMENT_ERROR` とその payload，block の flag（`RESTARTED`，`SEQ_DISCONTINUITY` は今のまま立つ），UHD の missed start だけの飛び（event なし），Mock の `ezsdr.radio.mock.faults` の行ごとの `lost`．

この段階は **risky**（AGENTS.md §9：fake（Mock）と実機の境の報告と hot path の形を変える）．**今回はテスト計画だけを書いて止まる**（§5）．コードは書かない．

## 2. やること（実装の段階で．今回は計画に書くだけ）

1. `crates/ezsdr-radio/src/lib.rs`：`RxOverflowPayload`（`:774` 付近）から `restart_gap_ns` を消し，`RX_OVERFLOW_HOT_BYTES` を 9 に，`to_hot`・`from_hot`・`from_payload` を合わせる．共通の関数 1 つ（入力：その block の前の report の cause の一覧と block の `lost`．出力：`Option<RxOverflowPayload>`）．名前は計画で決める．それ以外の仕組み（trait，Kernel の型，report の型の共有）は足さない．
2. MockRadio（`crates/ezsdr-mock-radio/src/lib.rs`）：`apply_rx_fault`（`:735` 付近）は fault ごとの payload を `overflows` に積む代わりに，report（cause と最初に失った sample）だけを積む．block を出す所（`:695` 付近，`publish_rx` の後）で共通の関数を呼び，0 か 1 個を emit する．back-pressure（`:600-615`，`emit_rx_overflow`）は今のまま（payload から `restart_gap_ns` が消えるだけ）．
3. UHD Module（`crates/ezsdr-radio-uhd/src/provider/rx.rs:530-575` 付近）：pending の report ごとの emit をやめ，block ごとに共通の関数を呼ぶ．飛びの無い block（`lost` が `None`）では event を出さず pending を消す（今は `lost: 0` の event を出しうる）．`Alignment` の `ALIGNMENT_ERROR` は今のまま．同じ種類の report を 1 つにまとめる `report`（`:91`）は残してよい．
4. schema：`schemas/radio/rx_overflow_payload.v1.json` と `schemas/radio/rx_overflow_hot_payload.v1.json` を作り直し，`schema_freeze` を緑にする（版は上げない．`SCHEMA_CHANGELOG.md` に書かない）．
5. spec（DA-1 の形式，AGENTS.md §2）：`design/07-radio-model.md` の RM-11 の payload の表（`:126`），RM-17，RM-18，RM-22（`:208`），RM-24（`:239`）とテスト表（`:357`）；`design/09-mock-radio.md` の MR-19，MR-21，MR-22，MR-37 とテスト表；`design/18-uhd-radio.md` の UR-18，UR-19（違う種類の report の RX_OVERFLOW の部分），UR-20（17 byte）．各 `*Test:*` 行と Changes の 1 行．メモ §3 の一覧を土台に，自分でも `restart_gap_ns`，`17 byte`，`17 bytes`，`each kind` を grep する．
6. テスト：`restart_gap_ns` を使う既存テスト（`crates/ezsdr-radio/tests/radio_model.rs`，`crates/ezsdr-mock-radio/tests/mock_radio.rs`，`crates/ezsdr-acceptance/tests/v58.rs`，`crates/ezsdr-radio-uhd/src/provider/rx.rs` の tests，`ur_19_reports_before_one_block_keep_each_kind` など．自分で grep する）を新しい規則に合わせる．`parity.rs` に両 Provider を突き合わせるテスト．

## 3. 系列と期待（メモ §4．Module の中の挙動は HEAD のコードで VERIFIED，UHD の report の性質はメモ §1 で UHD 4.10 の source により VERIFIED）

| # | 系列 | 期待 |
|---|---|---|
| T6 | overrun 1 回 | event 1 個，`lost` が block の `lost`，cause `overrun`，hot path 9 byte |
| T7 | sequence error 1 回 | event 1 個，cause `sequence` |
| T8 | 1 block の前に overrun と sequence | event 1 個，cause `overrun`，`lost` が block の `lost`，flag は両方 |
| T9 | 1 block の前に overrun 2 回 | event 1 個，`lost` が block の `lost`（UHD は今のまま 1 個，Mock は 2 個から 1 個に） |
| T10 | report の無い飛び（UHD） | `SEQ_DISCONTINUITY`，event なし（今のまま） |
| T11 | `RxOverflowPayload` の hot path | 9 byte の round trip，別の長さ（17 を含む）と cause の byte の拒否，`restart_gap_ns` を持つ object の拒否 |
| T12 | 等式 | event の `lost` の合計 = report のある飛びを持つ block の `lost` の合計（back-pressure なし，event の ring の drop なしの条件で） |
| T17 | UHD で missed start と overrun が 1 block の前 | event 1 個，cause `overrun`，`lost` が両方の飛び（Mock は start に失敗すると受信を始めないので UHD だけ：MR-11） |
| T18 | report の後に飛びの無い block | event なし，report はそこで消え，後の飛びに付かない（飛びの無い report が実機で来るかは INFERRED．来ないなら fake でだけ固める） |
| T19 | report の後に block が来ない（stop） | event なし |
| T20 | back-pressure：fault の後の block が `Full`，`Full` が続く，`Full` の後に block なし | Mock は捨てた時点ごとにその時の損失を報告し，2 重に数えない |

## 4. テスト計画の書き方

`test-plan.md` に，系列ごとに次を書く．

- 今あるテストでその系列を見ているもの（名前と，何を assert しているか）．自分で grep する（`RX_OVERFLOW`，`RxOverflowPayload`，`restart_gap_ns`，`overflows`，`Pending::`，`RxOverflowCause`）．
- 足りない系列に足す新しいテスト．T6〜T9，T12，T18〜T19 は `parity.rs` で両 Provider に同じ損失を起こして比べるのを基本にする（event の数，`cause`，`lost`，時刻，block の flag と `lost`）．Mock の fault と UHD の FakeDevice の report で同じ場面が作れないもの（T10，T17 は UHD だけ，T20 は Mock だけ）は各 crate のテストで，理由を書く．共通の関数の表は unit test．
- 各テストが何を assert し，どうやって観測するか，どんな誤った実装で落ちるか（例：report ごとに emit する，`lost` を report ごとの分にする，cause を最後の report にする，飛びの無い block の report を次の飛びに持ち越す，back-pressure を共通の関数に通して 2 重に数える）．
- `restart_gap_ns` と 17 byte を前提にしている既存テストの一覧と，それぞれの置き換え方（消さない．新しい規則の観測に移す）．
- 時刻に頼るテストを足さない（#68・#69：実時間の FakeDevice で負荷に揺れる観測は mutation の kill を偽る）．FakeDevice を使うなら手動の時計の rig か，飛びを決定的に作る道を使う．
- mutation の行は計画に書かない（コードと一緒に足す）．ただし今の行のうち `restart_gap_ns`，`RX_OVERFLOW_HOT_BYTES`，`to_hot`，`from_hot`，`overflows`，`emit_overflow`，`emit_rx_overflow`，`Pending::Overrun`，`Pending::Sequence`，`RxOverflowCause` を `old` か `new` に持つ行を grep して一覧にし，それぞれどう当て直すかを書く（`plan/maintenance/tools/mutations.json` と `plan/phase*/tools/mutations.json`）．

## 5. 守ること

- **今回はコードを書かない．`test-plan.md` と `report.md` だけを書く．** `report.md` の最初の行は `STATUS: plan`．
- `cargo fmt`・`rustfmt` を流さない．mutation は流さない．`mutate.py` をローカルで流さない．commit・push・`git add`・`git stash` をしない．
- **指示書と設計メモに無い仕組みを足さない．** production に `#[cfg(test)]` だけの道を作らない．
- 前の段階で出た誤り：テストが誤った実装を見抜けない観測（数だけ見て値を見ない），照合が空でも通るテスト（event が 1 個も無いのに `all(...)` が通る），到達できる系列のテストを代わりなしで消す，differential の操作を縮める，順序を前の event で満たす barrier（1b-7a の Sol の指摘），していないことを報告に書く．
- **起きたことを書く．していないことは書かない．**

# 1b-7b：計画の修正と実装の指示書

作業 dir `~/.cache/ezsdr-fixes/2026-10-23-phase8-1b7b/`．最初の指示書 `impl-brief.md` の §1・§2・§3・§5 の「守ること」はそのまま効く（ただし今回はコードを書く）．**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`.*.orig`）は読むだけ．見張りのプロセスを止めない．`git stash`・commit・push・`git add`・`cargo fmt`・`rustfmt` をしない．指示書と設計メモに無い仕組みを足さない．**

## 1. 計画の修正（Sol の計画レビュー `sol-plan.log` の最後．orchestrator はすべて採用した）

**`test-plan.md` を直してからコードを書く**．直した所は `test-plan.md` の末尾に「## 修正（Sol の計画レビュー）」として一覧にする．

1. **missed start だけの飛び（major）**：T17 の決定的な unit test に `LateCommand → Samples`（overrun なし）の場面を足す．正の飛びを持つ block が実際に届いたこと，`GAP_BEFORE` と正しい `lost`，`LATE_COMMAND` が 1 個，`RX_OVERFLOW` が 0 個を明示して assert する（共通の関数へ渡す変換で `MissedStart` を `Sequence` にする誤りを落とす．今の `rx.rs:574` は `MissedStart` を除いている）．
2. **D33（minor）**：mutation の行の一覧に `plan/phase4/tools/mutations.json` の D33（`to_hot` を含む）を足す．共通の emit の箇所が変わらなければそのまま，変われば当て直す．`mr_37_the_overflow_travels_the_hot_path` の severity の assert（`mock_radio.rs:2130` 付近）は残す．
3. **`mr_21_overrun_shape` の系列（minor）**：このテスト（`mock_radio.rs:1816-1836` 付近）は 1 ms と 10 ms の fault が**同じ再開 block** に合流する系列（fault ごとの損失 50 000 と 9 000，block 全体 59 000）．計画の「別 block にまたがる 2 fault」は誤り．系列はそのまま残し，event だけを `(time = 1 000, cause = overrun, lost = 59 000)` の 1 個に直す．block と fault の行の既存の assert は残す．

## 2. 実装

`impl-brief.md` §2 のとおり．計画のテストをすべて書く．spec と schema は §2 の 4・5．

## 3. 確かめること

1. `timeout 300` を付けて：`cargo test -p ezsdr-radio -p ezsdr-mock-radio -p ezsdr-radio-uhd -p ezsdr-acceptance`，`cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．host が別の job で混んでいることがある．落ちたテストは単独で流し直し，両方を報告に書く．
2. docker の uhd：`R="docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10"; $R cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings; $R cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`．
3. Python：`cargo build -p ezsdr-server` の後，`PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=~/.cache/cargo-target/Ez-SDRv4/debug/ezsdr-server ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests`．
4. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（テストの名前を変えたら `--renames` の file を作業 dir に置く）．
5. 新しい parity test と変えた UHD・Mock のテストを，同じ binary で `--test-threads=8` 以上で 20 回続けて流し，1 回も落ちないこと（決定性）．
6. mutation：当て直した行，足した行，`new` に消した名前（`restart_gap_ns` など）を持つ行と `filter` に消したテストを持つ行（grep する）を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:3,sim03:3 <IDs>` で流す．共通の関数の各分岐（cause を最後の report にする，`lost` を 0 にする，飛びの無い block で event を出す，overrun 優先を外す）と，両 Provider の呼び出し（report ごとに emit する，pending を消さずに持ち越す，back-pressure を共通の関数に通す）を変える行を足す．全行の `old` が source にちょうど 1 回一致することも確かめる．全リストは orchestrator が流す．

## 4. セルフレビュー

reviewer は Go の DeepSeek（owner 2026-10-23）．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat ~/.cache/ezsdr-fixes/2026-10-23-phase8-1b7b/checklist.md)" > ~/.cache/ezsdr-fixes/2026-10-23-phase8-1b7b/review-<n>.log 2>&1
```

P1 が無くなるか 3 回まで．**P1 が無くなったら次の回は流さない**．指摘はすべて直すか，理由をつけて退ける．利用の上限で止まったら，そこまでの事実を書いて `STATUS: blocked: limit` で終える．

## 5. 報告

`report.md` の末尾に「## 実装」を足す．最初の行を `STATUS: done`（か `STATUS: blocked: <理由>`）に変える．中身：計画の修正の一覧，変えた本体（`path` と行数），共通の関数とその呼び出し箇所の一覧，テストの一覧（新しい・変えた），mutation の行（当て直し・足した・grep の結果・流した結果），spec と schema の直した所，確かめることの結果（決定性の 20 回を含む），セルフレビューの指摘と扱い，穴．**起きたことを書く．していないことは書かない．**

You are a read-only reviewer of an uncommitted change in /home/komatsu/local_work/Ez-SDRv4 (Phase 8 stage 1b-7b: `radio.RX_OVERFLOW`'s payload loses `restart_gap_ns` — hot form 9 bytes, old forms refused — and after a loss report each first block with a jump carries at most one event, its `lost` the block's whole jump and its cause overrun if any report was one, decided by one `ezsdr_radio` function both Providers call; MockRadio's back-pressure overrun stays outside it). Read-only: do not edit files; do not run cargo, python, mutate.py or remote_mutate.py. Read `git diff HEAD`, the design note `plan/phase8/36-radio-report-rules.md` (decisions 4 and 5) and the test plan `~/.cache/ezsdr-fixes/2026-10-23-phase8-1b7b/test-plan.md`.

Check only these four, and report each finding as P1 (wrong behaviour, a planned sequence with no test, or a test that cannot fail on the bug it names), P2 or P3, with `path:line`:

1. Every planned test exists and observes what the plan says (event count, cause, lost, time, block flags and lost, the hot bytes); it would fail if the rule it pins were broken (no empty event lists passing `all(...)`, no sums over nothing, no wall-clock timing as the oracle).
2. Both Providers decide the event only through the shared function: no per-report emit left, no pending report carried past a jump-less block, no event at a stop with no block, the back-pressure path not routed through it and not counted twice; `ALIGNMENT_ERROR`, flags and the missed-start-only jump unchanged.
3. Nothing outside decisions 4–5 changed (no new mechanism, no `#[cfg(test)]`-only production path, `radio.timing.overflow_restart_gap_ns` kept, no version bump or SCHEMA_CHANGELOG entry).
4. RM-11, RM-17, RM-18, RM-22, RM-24, MR-19, MR-21, MR-22, MR-37, UR-18, UR-19, UR-20 and both schemas say what the code now does, in the accepted-spec format; every `*Test:*` name exists; mutation rows whose `new` names a removed item or whose `filter` names a removed test were found and fixed.

If you find nothing of P1, say 「P1 なし」. Write in Japanese. The orchestrator's gate (full mutation lists) is not a finding.
