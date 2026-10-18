# 実装の指示書：Phase 8 段階 1b-4b（UHD Module の内部を stream ごとに組み直す．挙動は変えない）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `9816d78` に 1b-4a の未 commit の変更．gate を通った最終の tree で，間もなく commit される）．

**この段階は 2 回に分けて起動する（AGENTS.md §9 の新しい手順．リスクのある段階）：**
- **1 回目（今回）**：テスト計画だけを書く．コードと tree には触れない．
- **2 回目**：Sol のレビューで直した計画をもとに実装する（別の指示で起動する）．

## 最初に読むもの

- `AGENTS.md` を全部．特に §2 の DA-1，§3，§6，§7，§9（新しい手順）．
- 設計 `plan/phase8/33-channels-and-streams.md` の §3，§4 D3，D15，§5 の 1b-3 と 1b-4 の行．
- `design/18-uhd-radio.md` の UR-15，UR-16，UR-24〜UR-29．
- 参考：MockRadio で同じ組み直しをした commit `b8ec8c3`（1b-3a．`git show b8ec8c3`）．
- コード：
  - `crates/ezsdr-radio-uhd/src/provider/`（`core.rs` の `Streams`，`mod.rs` の `start`/`stop`/`spawn`/`join`，`control.rs`，`rx.rs`，`tx.rs`）
  - `crates/ezsdr-radio/src/timeline.rs` の `Lines`

## この段階でやること（2 回目に実装する中身）

`UhdRadio` の中の状態を stream ごとにする．**挙動は 1 つも変えない**（Manifest，event，thread の名前，テストの結果は HEAD と同じ）．
- `Streams.lines: [Option<Line>; 2]` を `timeline::Lines`（key は `(Direction, k)`）にする．
- stream に属する状態は，stream ごとの型に移す．
  - `configs`，`delivered`，`idle`，送信の `tx_clocks` など
  - instance 全体の状態はそのまま：`seq`，`ended`，`stop`，`version`
- `Rx` と `Tx` は自分の stream の番号 `k` を持つ．
  - device の呼び出しは，決め打ちの `0` ではなく `k` を使う．
  - line の参照は `(dir, k)` にする．
- thread は，受信 stream ごとに 1 本，送信 stream ごとに 1 本（D15）．
  - 今の key では各方向に stream は 1 本だけなので，数は今と同じ．
  - thread の名前は今の `uhd-rx`・`uhd-tx` のまま．stream ごとの名前は 1b-4c で決める．
  - `stop` の順序は今と同じにする：control → 全送信 → 全受信（RM-16）．
- uhd-control が受けた command は，宛先の stream の line に記帳する．今は方向ごとに stream 0．
- lock の規則は変えない：
  - 順序：`updates` → `streams` → `rec`
  - device の呼び出しの間は line の lock を持たない（UR-25）．
- テストは変えない．
  - 変えてよいのは，コンパイルを通すための内部の型名や field 名の書き換えだけ．
  - 既存の mutation の行で `old` が一致しなくなったものは，同じ規則を同じ形で壊すように当て直す．

## 守ること

`~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4a/brief.md` の「守ること」と同じ．要点：
- `cargo fmt` を使わない．
- mutation は `remote_mutate.py` だけで流す．
- commit・push・`git add` をしない．
- 2 分を超えるコマンドは nohup で流し，`kill -0` で待つ．
- テストを弱めない．
- 版を上げない．

## 1 回目の成果物：テスト計画（項目ごとに分けて起動する）

1 回目は，項目ごとに別の実行として起動する．順序 1〜11 に 1 本ずつ，「計画の最後に書くこと」に 1 本の，計 12 本．あなたが担当するのは，起動の指示が名指しした**1 項目だけ**．
- 読むだけにする（read-only で起動される）．ファイルは書かず，最後の回答を計画そのものにする（`plan-<番号>.md` に保存される）．
- 書式（見出しをこのとおりに）：
  `## <番号>. <順序の名前>` / `### (a) 既存のテスト` / `### (b) assert と観測` / `### (c) 壊れうる形` / `### (d) 捕まえるか・足す案` / `### 関わる他の順序`
- 根拠は必ず `path:line` で書く．「見つからない」と書くときは，実行した検索（grep のコマンドそのまま）を添える．
- 最後の「関わる他の順序」には，この順序と同時に起きうる他の順序の番号と，そのとき何が問題になりうるかを 1 行ずつ書く．

orchestrator が 12 本をまとめて `test-plan.md` にする．

### 次の操作の順序ごとに，書くこと

各順序について，次の 4 つを書く．
- (a) その振る舞いを今固定している**既存の**テストの名前（`path:line`）
- (b) そのテストが何を assert しているか．どう観測しているか（event，Manifest，device の calls など）
- (c) 組み直しで壊れうる形．例：stream ごとの型に移した値を，別の stream や instance 全体と取り違える．
- (d) (c) を既存のテストが捕まえるか．捕まえないなら，足すテストの案と mutation の行の案．

順序：
1. `start`：line は thread を起こす前に入る．受信の link がないときは line がない．uhd-rx と uhd-tx は stream がなくても起きる（UR-15）．
2. `stop`（Drain と Abort）：全 line に `End` が 1 回だけ記帳される．join の順序は control → 送信 → 受信．
3. stream の thread の panic：その thread の名前で `rejected` に記録され，device の loss として全 stream が終わる．
4. ある stream の thread が device の loss を見つける：全 stream が 1 回だけ終わる（`ended` は instance 全体）．
5. ある stream の device が設定を拒否する（RM-25）：その stream の line だけが halt する．他の方向の stream はそのまま．
6. stream のない方向の enable（channel 数 0 → n．`configs`・`idle`・UR-25）と，0 への disable（`tx_open(0, &[])` の経路）．
7. 受信の cut と delivered の床（RM-16．`delivered` が stream ごとになる）．
8. 送信の clock の登録と終わり（`tx_clocks`），`version` による再読み込み．
9. 同じ instant の `cold` 変更と timed 変更の順序（UC-2，RM-25 の床は instance 全体の `seq`）．
10. lock の順序と，device の呼び出しの間に line の lock を持たないこと．
11. thread の名前と，Manifest の `rejected` の `thread`．

### 計画の最後に書くこと

- provider の中で，stream の番号 `0` を決め打ちしている箇所の一覧（`grep` の結果．path:line）．どれを `k` に置き換えるか．
- 当て直しが要りそうな既存の mutation の行の見込み（リストと id）．
- 多数の stream（2 本以上）でしか出ない誤りは，この段階では観測できない．その一覧を書き，1b-4c に送る．

## 1 回目のセルフレビュー

**なし．** 計画は Sol がレビューする（AGENTS.md §9 の 3）．
