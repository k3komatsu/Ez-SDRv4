# 1b-4e-1：実装の指示書

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `a18aaef`，tree はきれい）．この段階は **risky ではない**（AGENTS.md §9：本体のコードを変えず，`crates/ezsdr-acceptance/tests/parity.rs` に stream 1 以上の比較を足す）．テスト計画は別に書かず，実装に入ります．

## 1. 背景と分け方

計画の 1b-4e（plan 33 の「分け方」）は「moves（RM-34）と複数 stream の VH-8 harness」．orchestrator はこれを 2 つに分けた：

- **1b-4e-1（これ）**：VH-8 の harness（`parity.rs`）に stream の次元を足す．Mock（1b-3b から）と UHD Module（1b-4d-2 から）は，どちらも layout の stream 1 以上を動かす．今の `parity.rs` は stream 0 だけを比べる（冒頭の doc comment：「anything of the stream dimension beyond stream 0 (1b-4d's and 1b-4e's work)」）．
- **1b-4e-2（次）**：UHD Module の moves（今は `UR-25: ... does not move a channel between streams yet` で拒否）．harness が先にあれば，moves の差を Mock との比較で拾える．

## 2. 読むもの

- `crates/ezsdr-acceptance/tests/parity.rs`（全部．`MockRig`，`UhdRig`，各 `vh_8_*`）．
- `design/07-radio-model.md` の RM-25（床と開始の規則），RM-27〜RM-32（layout の key），RM-29（stream の node と port）．`design/09-mock-radio.md` の MR-27（`segments`），MR-38〜MR-40．`design/18-uhd-radio.md` の UR-25，UR-30（`timing.plan` の行：1b-4d-3 で `{ stream, sequence, origin, end, channels, ticks_per_sample }` になった）．
- `plan/phase8/33-channels-and-streams.md` の §6（比較の項目），D3，D4，D15．
- 参考：`crates/ezsdr-mock-radio/tests/mock_layout.rs`，`crates/ezsdr-radio-uhd/tests/fake.rs` の stream 1 以上のテスト（`rx_two()`，`tx_two()`，`layout_selector()`，`attached_reading`）．PO-8 により，`parity.rs` は両方の harness の写しで，共有のコードにしない．

## 3. orchestrator の決定：足す比較

どれも **同じ selector と同じ Spec を Mock と UHD（FakeDevice）に通し**，今の `vh_8_*` と同じく，時刻はそれぞれの T0 からの相対で比べる（2 つの rig の時間軸は違う）．layout は 2 本の受信 stream（＋必要なら 2 本の送信 stream）．

1. **prepare**：2 本ずつの layout で，`PrepareReport.effective`，宣言した SampleClock の node と ratio が stream ごとに同じ（`<id>/rx/s1`，`<id>/tx/s1` を含む）．
2. **stream 1 だけの変更**：stream 1 の `cold` な rate 変更が stream 1 だけを cut し，stream 0 の clock を変えない．両方で，各 stream の clock の行（node，ratio，相対 origin，相対 end）が同じ．
3. **stream 1 への membership の join**：`-1` の channel が stream 1 に入る．gap（RM-25 の ready と lead）が同じ．
4. **channel の無い stream 1**：node だけで SampleClock の無い stream 1 に，最初の channel が入ったときの最初の segment が同じ．
5. **送信 stream 1 の clock**：送信の channel を stream 1 で有効にしたときの clock（node，ratio，相対 origin）が同じ．
6. **instance 全体の床**（RM-25，`timeline::layout_instant`）：stream 0 に未効の `cold` 変更があるとき，stream 1 の変更が同じ床に押される．
7. **stream 1 の拒否**：stream 1 だけの変更を両方が同じ key・同じ理由（の規則番号の prefix）で拒否し，stream 0 は変わらない．
8. **Manifest の plan**：stop の後，UHD の `ezsdr.radio.uhd.timing` の `plan` の行と Mock の `ezsdr.radio.mock.segments` の行が，stream ごとに `stream`，`sequence`，相対 `origin`，`channels` で同じ（1b-4d-3 で名前を揃えた）．方向の表し方の違い（UHD は `rx`・`tx` の配列，Mock は `direction` の列）は比べる側で吸収する．

**両方が違うことが分かったら**：本体のコードは直さない．その比較のテストは `#[ignore = "divergence: <短い説明>"]` にして，違いの中身（どちらが spec に合うか，`path:line` と spec の rule）を報告の §「違い」に書く．orchestrator が次の段階で扱う．moves（stream から stream への移動）は比べない（UHD は 1b-4e-2 まで拒否する）．

冒頭の doc comment の「beyond stream 0」の文は，この段階で比べるようになったものに合わせて直す．

## 4. やること・守ること

- 足すのは `parity.rs` だけ．rig（`MockRig`，`UhdRig`）は stream 1 以上の link と記録の読み出しが要るなら広げる．本体（`crates/*/src`）は変えない．
- 繰り返しは helper にする（rig の起動と T0 の合わせ方，clock の行の相対化，Manifest の行の読み出し）．既存の `vh_8_*` の意味は変えない．
- mutation の行を 3〜5 行（P485 以降）：**新しい比較が拾う本体の誤り**（例：Mock か UHD のどちらかで stream 1 の node を stream 0 にする，stream 1 の変更が stream 0 の clock も切る，床を stream ごとにする）．`test_bin` は `parity`，`crate` は `ezsdr-acceptance`．流して killed を確かめる．**触ったコードに `old` がある既存の行も流す**（今回は本体を変えないので，`parity.rs` を kill 役に持つ既存の行）．
- **指示書に無い仕組みを足さない**．足りないと思ったら，足さずに報告の「穴」に書く．
- `cargo fmt`・`rustfmt` を流さない．新しいコードは周りの書き方に手で合わせる．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．sim02・sim03 は他の計算で混んでいることがある：`REMOTE_CPUS=8` と shard を少なめ（`sim02:2,sim03:2`）にし，`BASELINE FAILED` が出たらその行だけ流し直して報告に書く．
- commit・push・`git add` をしない．`plan/*/tools/mutations.json` はファイル全体を書き直さない．
- **1 本のテストや 1 つの test binary には `timeout 300` を付ける**．背景の job を残さない．
- UHD の rig は実際の thread と wall clock の `settle` を使う．負荷で揺れる assert（gap の上限など）は，今の `vh_8_*` と同じ幅に揃え，狭めない．
- Python は `PYTHONDONTWRITEBYTECODE=1`．
- 前の段階で出た誤り：到達できる系列を「到達できない」と言って消す．していない実装を報告に書く．記録を手で書き換えて，実際の経路を通さずに確かめる．

## 5. 確かめること

1. `cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．`parity` は 3 回続けて流し，揺れないことを見る．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`．
3. mutation：`REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:2,sim03:2 <IDs>`．COMPILE ERROR も生き残りとして扱う．

## 6. セルフレビュー

reviewer は DeepSeek．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e1/checklist.md)" > ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e1/review-<n>.log 2>&1
```

チェックリスト（`checklist.md` に書く）．読む範囲は `git diff HEAD`．reviewer には「read-only．cargo・mutation・mutate.py・remote_mutate.py を流さない．指摘を P1〜P3 で，`path:line` つきで．無ければ『P1 なし』」と書く．項目：
1. §3 の 8 つの比較それぞれにテストがあり，両方の Provider の実際の経路を通し，同じ値を比べているか（片方だけを見ていないか，相対化で差が消えていないか）．
2. 範囲：§3 に対して足りないもの，余計なもの．本体のコードを変えていないか．
3. `#[ignore = "divergence: ..."]` にしたテストの理由が報告と合っているか．
4. 冒頭の doc comment が，比べるもの・比べないものを正しく書いているか．

P1 が無くなるか 3 回まで．**P1 が無くなったら次の回は流さない**．指摘はすべて直すか，理由をつけて退ける．

## 7. 報告

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e1/report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．中身：足したテストと `path:line`（§3 の番号と対応させる），「違い」（見つけた divergence と ignore にしたテスト），確かめることの結果（mutation は行ごと），セルフレビューの指摘と扱い，テストの行数，穴．**起きたことも書く．していないことは書かない**．
