# 1b-6b：実装の指示書（UHD Module：owner の段階を遷移の関数だけが変える）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `2af9afd`，tree はきれい）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-22-phase8-1b6b/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`.*.orig` など）は読むだけ．書き換えない．orchestrator が動かしている見張りのプロセス（`watchdog_proc.py`，`mutation_watchdog.py`）は止めない．自分で起こした背景の job だけを片付ける．`git stash` を使わない．**

## 1. 背景

設計メモ `plan/phase8/35-uhd-stream-state.md` を最初に全部読むこと．前の段階 1b-6a（R1）は `2af9afd` で入った（記録 `plan/phase8/reviews/1b6a-opus.md`，`Claims`・`Claim`・遷移の関数の形はそこに倣う）．この段階はメモの **R2** だけ：`crates/ezsdr-radio-uhd/src/provider/core.rs` の `StreamState` の `idle`・`active`・`delivered` を private な `Phase`（メモ §2 R2）にし，`Streams` の遷移の関数だけが変える．`Claims`（R1），device の記録，`completed`，`clocks`，loss，`needed_before` の判定は変えない．

**挙動は変えない**．これは書き方の作り替えで，HEAD で観測できることはすべてそのまま残る．メモの R2 の表と HEAD のコードが食い違う所を見つけたら，HEAD のコードの挙動を正とし，食い違いをテスト計画の「食い違い」の節に書く（勝手に挙動を変えない）．

この段階は **risky**（AGENTS.md §9：lifecycle）．**今回はテスト計画だけを書いて止まる**（§5）．コードは書かない．

## 2. やること（実装の段階で．今回は計画に書くだけ）

1. `core.rs`：`Phase { Idle, Pending, Active { segment: Option<(i64, Config, Option<u64>)>, delivered: Option<i64> } }`（private な field），遷移の関数（例：初期化，`begin` の探索の結果，始めた，ブロックを渡した，終えた．名前は計画で決める），読む関数（`Idle` か，`Active` の segment，`delivered`）．
2. 今 `idle`・`active`・`delivered` を書いている所（`rx.rs` の `Rx::new`・`begin`・ブロックごと・`finish`，`tx.rs` の `plan_section`・`do_switch`，`control.rs` の `start`）と読んでいる所（`claim_release`，from-none の判断，`Streams::book` の `End`，`complete`）を関数の呼び出しにする．field は private にする．
3. spec 18 の UR-17（delivered の文）と UR-25（idle mark の文）を，段階の言葉に書き直す（規則は変えない）．DA-1 の形式（AGENTS.md §2）．
4. テストのうち `.idle`・`.active`・`.delivered` を直接読み書きする所（`grep -rnE "\.(idle|active|delivered)\b" crates/ezsdr-radio-uhd`）を，関数か観測で置き換える．
5. **P472**（`plan/maintenance/tools/mutations.json`）を，thread を使わない `Streams` の unit test に当て直す：受信 stream 0 と 1 を `Active` にする → stream 1 を終える → stream 0 の `delivered` が残る → Abort の `End` が stream 0 をその floor で切る．

## 3. 系列と期待（メモ §4 の R2．どれも Module の中の挙動で VERIFIED：HEAD のコードと spec 18 UR-17・UR-25）

| # | 系列 | 期待 |
|---|---|---|
| P1 | 受信の初期化：`start` が T0 の stream を持つ／持たない | `Active{delivered: 0}`（segment は plan に一致するときだけ）／`Idle` |
| P1b | 受信の初期化で T0 の segment が plan に無い（`start` の途中に別の stream が T0 前の loss を publish） | `Active{segment: None, delivered: Some(0)}`．終えても `completed` に何も載らない |
| P2 | 受信の `begin`：cursor 以降に segment がある／無い／holder が `Free` でない | `Pending`／`Idle`／段階を動かさない |
| P3 | 受信が segment を始め，ブロックを渡し，終える | `Active{segment, 0}` → `delivered` が進む → 終えると `completed` に載り `Pending` |
| P4 | 受信の configure が拒否される／loss | `Pending` のまま |
| P5 | 送信の `plan_section`：clock も switch も無い／clock がある／clock が無く switch がある | `Idle`／今の segment で `Active`／`Pending` |
| P6 | 送信の `do_switch` が拒否される：次の segment が無い／有る | 旧 `Active` を完了し，最後の plan の読み直しで `Idle`／`Pending` か `Active` |
| P7 | 送信の空への switch，長さ 0 の segment の skip，拒否済みの segment の skip | HEAD と同じ段階と `completed` |
| P8 | `release_unneeded` と from-none の判断 | `Idle` の stream だけ（R1 の `claim_release` の条件は段階を読む形に） |
| P9 | `Streams::book` の `End`（loss，`Provider::stop`） | `Active` の `delivered` を，その segment を切る End にだけ使う（HEAD の照合のまま） |
| P10 | 有限の move の鎖，同時の swap，source の拒否の前の destination の待ち | `needed_before` と `completed` の判定が HEAD と同じ（1b-4e-2 の既存テスト） |
| P11 | P472 の系列（§2 の 5） | stream 1 を終えても stream 0 の `delivered` が残り，Abort がそこで切る |

## 4. テスト計画の書き方

`test-plan.md` に，系列ごとに次を書く．

- 今あるテストでその系列を見ているもの（名前と，何を assert しているか）．自分で grep する．候補：`ur_26_an_end_takes_delivered_from_each_segment_s_own_stream`（`core.rs`），`ur_26_two_receive_streams_end_at_their_own_stops`（`tests/fake.rs`），`ur_34_a_release_*`，`ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain`，`ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing`．
- 足りない系列に足す新しいテスト：thread を使わない `Streams` の unit test（遷移の関数の各行）を基本にし，thread の順序が要るものは既存の probe（`core.probe`）と manual clock の形に合わせる．sleep を判定に使わない．
- 各テストが何を assert し，どうやって観測するか，どんな誤った実装で落ちるか．
- 印を直接読み書きしている今のテストの一覧と，それぞれの置き換え方．
- 食い違い：メモ R2 の表と HEAD のコードが合わない所（あれば）．
- mutation の行は計画に書かない（コードと一緒に足す）．ただし今の行のうち `idle`・`active`・`delivered` を `old` か `new` に持つ行を grep して一覧にし，それぞれどう当て直すかを書く．

## 5. 守ること

- **今回はコードを書かない．`test-plan.md` と `report.md` だけを書く．** `report.md` の最初の行は `STATUS: plan`．
- `cargo fmt`・`rustfmt` を流さない．mutation は流さない．`mutate.py` をローカルで流さない．commit・push・`git add`・`git stash` をしない．
- **指示書と設計メモに無い仕組みを足さない．** production に `#[cfg(test)]` だけの道を作らない（probe は既存の仕組み）．
- 前の段階で出た誤り：テストが誤った実装を見抜けない観測（待ちの loop の前の probe だけで待ちを見る，lock の回数だけで順序を見る，踏めない道を review だけにする），照合が空でも通るテスト，到達できる系列のテストを代わりなしで消す，していないことを報告に書く．
- **起きたことを書く．していないことは書かない．**

---

# 1b-6b：計画の修正と実装の指示書

作業 dir `~/.cache/ezsdr-fixes/2026-10-22-phase8-1b6b/`．最初の指示書 `impl-brief.md` の §1・§2・§3・§5 の「守ること」はそのまま効く（ただし今回はコードを書く）．**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`.*.orig`，`.*.snap`）は読むだけ．見張りのプロセスを止めない．`git stash`・commit・push・`git add`・`cargo fmt`・`rustfmt` をしない．指示書と設計メモに無い仕組みを足さない．**

## 1. 計画の修正（Sol の計画レビュー `sol-plan.log` の最後．orchestrator はすべて採用した）

**`test-plan.md` を直してからコードを書く**（前の段階では順序が逆になった）．直した所は `test-plan.md` の末尾に「## 修正（Sol の計画レビュー）」として一覧にする．

1. **P6（major）**：`tx.rs` の既存テスト（1550〜1578 行付近）の最後は 3 個目の segment の `Active` として検査する（`Idle` ではない）．別に「後続の無い configure の拒否」の系列を作り，旧 segment の完了，clock が無いこと，最後が `Idle` であることを検査する．
2. **P4（major）**：`rx.rs` の拒否からの再開は command 2 の `Start` なので，再開後の `active` は `by: Some(2)` を検査する．configure 中の loss の既存テスト（`rx.rs` 935〜940 行付近）にも，戻った直後が `Pending` で active も floor も無いことの assert を足す．
3. **P1b（major）**：T0 の line を作り，T0 前の loss を実際に book して segment を消してから `Rx::new` と finish を通すテストにする．段階，完了の記録が無いこと，loss の `End` と cut が HEAD と変わらないことを検査する．§5 の「その間はどの thread も動かない」は誤り（`Provider::start` は RX を 1 本ずつ作ってすぐ spawn する，`provider/mod.rs` 633〜664 行付近）なので直す．
4. **`delivered` の形（orchestrator の決定）**：**HEAD の tuple `(origin, config, k)` をそのまま残す**．`Active { segment: Option<…>, delivered: Option<(i64, Config, i64)> }`（受信だけ `delivered` を持つ）．`first_undelivered` は tuple をそのまま返す．すべての道で同じ floor になることを示せないので，挙動を変えない方を採る．食い違い 3 は「採らなかった」として残す．
5. **N7（minor）**：Abort を使い，`End` の instant を stream 0 の `delivered` より後に置いて，stream 0 が instant ではなく `delivered` で切れることを assert する．Orderly の場合は残してよい．
6. **N4・N5（minor）**：掲げた誤実装を落とすには，各前の状態から該当の操作を 1 回して不変を確かめる．到達できないなら，落とす誤実装の主張から外す．
7. **配線の mutation**：P470・P472 を `Streams::finished` へ移した後も，`Rx::finish` が `finished` を呼ぶ配線を変異させて殺す行を 1 つ足す（既存の fake のテストか P470 のテストで殺す）．N6 は clock があり segment が無い枝も検査する．

## 2. 実装

`impl-brief.md` §2 のとおり．計画のテストをすべて書く．`design/18-uhd-radio.md` の UR-17（delivered）と UR-25（idle mark）の文と `*Test:*` 行，Changes の 1 行を直す（DA-1 の形式）．

## 3. 確かめること

1. `timeout 300` を付けて：`cargo test -p ezsdr-radio-uhd`，`cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．
2. docker の uhd：`R="docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10"; $R cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings; $R cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（テストの名前を変えたら `--renames` の file を作業 dir に置く）．
4. 新しいテストを，同じ binary で `--test-threads=8` 以上で 20 回続けて流し，1 回も落ちないこと（決定性）．
5. mutation：当て直した行，足した行，`new` に消した名前を持つ行と `filter` に消したテストを持つ行（grep する）を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:3,sim03:3 <IDs>` で流す．全行の `old` が source にちょうど 1 回一致することも確かめる．全リストは orchestrator が流す．

## 4. セルフレビュー

reviewer は実装役と同じモデル．**同時に 1 本だけ**：自分のモデルの呼び出しが無い間（shell で reviewer を待つ間）に流す．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "$(cat ~/.cache/ezsdr-fixes/2026-10-22-phase8-1b6b/checklist.md)" > ~/.cache/ezsdr-fixes/2026-10-22-phase8-1b6b/review-<n>.log 2>&1
```

P1 が無くなるか 3 回まで．**P1 が無くなったら次の回は流さない**．指摘はすべて直すか，理由をつけて退ける．利用の上限で止まったら（OpenCode の free の rate limit など），そこまでの事実を書いて `STATUS: blocked: limit` で終える．

## 5. 報告

`report.md` の末尾に「## 実装」を足す．最初の行を `STATUS: done`（か `STATUS: blocked: <理由>`）に変える．中身：計画の修正の一覧，変えた本体（`path` と行数），テストの一覧（新しい・変えた），mutation の行（当て直し・足した・grep の結果・流した結果），spec 18 の直した所，確かめることの結果（決定性の 20 回を含む），セルフレビューの指摘と扱い，穴．**起きたことを書く．していないことは書かない．**
