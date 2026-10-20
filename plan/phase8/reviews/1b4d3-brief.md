# 1b-4d-3：実装の指示書

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `34dde0f`（履歴の書き直しで `d58da80`．tree は同じ），tree はきれい）．この段階は **risky ではない**（AGENTS.md §9：queue・期限・lifecycle・所有・FFI に触れず，Manifest の記録の形とテストを変える）．だからテスト計画を別に書かず，実装に入ります．

## 1. 読むもの

- `plan/phase8/33-channels-and-streams.md` の 1b-4 行「1b-4c で 1b-4d へ送るもの」の (4)，(8)，(9) と「分け方」の 1b-4d-3．D3 の 6（segment ごとの channel の並びを Provider の Manifest section に残す）．
- `plan/phase8/reviews/1b4d2-opus.md`（前の段階），`plan/phase8/reviews/1b4b-opus.md` の Opus minor（`ur_25_a_refusal_leaves_the_transmit_stream_alone` が空の tx line と比べる）．
- `design/18-uhd-radio.md` の UR-30（Manifest の sections，`timing` の plan の行）．`design/09-mock-radio.md` の MR-27（Mock の `segments` の行：`{ direction, stream, sequence, origin, channels, phase_behavior, coherence, ended_at }`）．
- コード：`crates/ezsdr-radio-uhd/src/provider/core.rs` の `Streams::record`（plan の行），`tx.rs` の `report`（`device_async` の行），`rx.rs:1490` の `ur_25_a_refusal_leaves_the_transmit_stream_alone`，`tests/fake.rs` の plan の行を読むテスト．

## 2. orchestrator の決定

1. **plan の行に stream と channel の列を持たせる**（(8)，D3 の 6）：`Streams::record` の各行を `{ stream, sequence, origin, end, channels, ticks_per_sample }` にする．`stream` は stream の番号 `k`，`sequence` はその segment を始めた command（`segment.by`．無ければ `null`），`channels` は**論理 channel の列**（block の順，`Channels::iter()`）．今の `channels`（数）は列に置き換える（数は列の長さ）．方向の分け方（`rx`・`tx` の配列）は今のまま．名前は Mock の MR-27 の `segments` の行に合わせる（`stream`，`sequence`，`channels`）．
   - 退けた案：Mock と同じ別の `segments` section を足す（UHD では segment が `timing` の plan と 2 か所に書かれる）．Mock の `phase_behavior`・`coherence` は D13 の Mock のモデルなので UHD には足さない．
2. **`device_async` の行に `stream`**：`tx.rs` の `report` の行に `stream: k` を足す（`channel` は streamer の中の番号なので，2 本の送信 stream では stream が要る）．
3. **(9) 拒否の隔離のテストを作り直す**：`ur_25_a_refusal_leaves_the_transmit_stream_alone` は空の tx line と比べている．tx line に item（予約された cold 変更か burst の clock の segment）がある状態で RX の拒否を起こし，tx line の plan・items・clock が変わらないことを見る．layout の 2 本の受信 stream でも同じ形を 1 本（stream 0 の拒否が stream 1 の item のある line を変えない）．
4. **(4) 複数 stream の `Provider::stop`**：2 本の受信 stream と 2 本の送信 stream の e2e で，`Provider::stop` の後の Manifest の plan の行が，各 stream（`stream` の値）について最後の segment の `end` を持つこと（RX は stop の instant の sample，TX は今の規則のとおり）を 1 本で見る．VH-8 の harness の stream の次元は 1b-4e なので触らない．
5. **テストの helper**（plan 33 の「テストの繰り返しは helper にする」）：`tests/fake.rs` で Manifest の section を引く式，plan の行を stream で選ぶ式，拒否の理由の取り出しが繰り返されていれば helper にする．既存のテストの意味は変えない．

## 3. design/18

UR-30 の `timing` の plan の行の文を，2.1 の形（`stream`，`sequence`，論理 channel の列）に直す．`device_async` の行に `stream`．§6 の表に新しいテスト，Changes に 1 行（DA-1）．

## 4. やること・守ること

- 新しい規則ごとに mutation の行を 1 行（P480 以降）：plan の行の `stream`，`channels` の列，`sequence`，`device_async` の `stream`．plan の行を読む既存のテストと mutation の行で，`channels` を数として読むものは直す．**触ったコードに `old` がある既存の行も流す**（前の段階で，名前を挙げた行の外の 2 行が当て先を失っていた）．
- **指示書に無い仕組みを足さない**．足りないと思ったら，足さずに報告の「穴」に書く．
- `cargo fmt`・`rustfmt` を流さない．新しいコードは周りの書き方に手で合わせる．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．
- commit・push・`git add` をしない．`plan/*/tools/mutations.json` はファイル全体を書き直さない．
- **1 本のテストや 1 つの test binary には `timeout 300` を付ける**．背景の job を残さない．長い command は前景で，timeout を延ばして（600000 ms）流す．
- Python は `PYTHONDONTWRITEBYTECODE=1`．
- 前の段階で出た誤り：到達できる系列を「到達できない」と言って消す．していない実装を報告に書く．記録を手で書き換えて，実際の経路を通さずに確かめる．

## 5. 確かめること

1. `cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（名前を変えたテストがあれば `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d3/renames.txt` を `--renames` で）．
3. mutation：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <IDs>` で，新しい行，変えた行，触ったコードやテストに当たる既存の行．COMPILE ERROR も生き残りとして扱う．

## 6. セルフレビュー

reviewer は DeepSeek．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat <チェックリストのファイル>)" > <ログ> 2>&1
```

チェックリスト（ファイルに書く）．読む範囲は `git diff HEAD`．reviewer には「read-only．cargo・mutation・mutate.py・remote_mutate.py を流さない．指摘を P1〜P3 で，`path:line` つきで．無ければ『P1 なし』」と書く．項目：
1. §2 の各決定にテストがあり，テストがそれを実際の経路で観測し，mutation の行がそれを壊すか．
2. 範囲：§2 に対して足りないもの，余計なもの．
3. plan の行の `channels` を数として読む所（テスト，mutation の行，design の文，Python の client：`python/` を grep）が残っていないか．
4. この変更で偽になる `design/18` の文が残っていないか．

P1 が無くなるか 3 回まで．指摘はすべて直すか，理由をつけて退ける．

## 7. 報告

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d3/report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．中身：直しと `path:line`，足した・直した・消したテストと行，確かめることの結果（mutation は行ごと），セルフレビューの指摘と扱い，本体とテストの行数，穴．**起きたことも書く．していないことは書かない**．
