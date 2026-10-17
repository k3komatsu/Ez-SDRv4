# 修正の指示書：Phase 8 段階 1b-2a（レビュー後）

あなたは実装役です．リポジトリ /home/komatsu/local_work/Ez-SDRv4．前回の指示書 `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2a/brief.md` の「守ること」はすべてそのまま有効です（`cargo fmt`/`rustfmt` 禁止，mutation は `remote_mutate.py` だけ，commit/push/`git add` 禁止，2分を超えるコマンドは nohup で背景・終わりは `kill -0 <pid>`，テストを弱めない，版を上げない，新しいファイルは `chmod 644`）．レビューは2本：`~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2a/opus-review.md`（O1–O3）と `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2a/sol-review.md`（1–3）．**両方を全文読んでから**始めること．採否：**全部採用**．以下が決定です（レビューの修正案と食い違うところはこちらが優先）．

## 直すこと

1. **設定全体への判定を1つの公開関数に（O1，major）**：`channels::Description` に公開の `violations(&self, configuration: &BTreeMap<Key, Value>) -> Vec<(Key, String)>` を置き，設定全体（全 channel・全 stream の設定 key）に対して RM-31（全二重）と RM-32（throughput）を判定する．要求に依らず**必ず報告する**：RM-31 は受信 channel の antenna の key，RM-32 は上限を超えた方向で（channel の入っている）最初の stream の rate の key を名乗る（どちらを名乗るかを spec に書く）．`coerce` は defaults ＋ applied にこの関数を呼び，各違反を今のように要求に含まれる key へ写す（RM-31 は antenna か送信 channel の `stream`，RM-32 は rate か `stream`）．要求のどの key にも写らない違反は defaults だけからは生じないので，その場合は理由の中の key で拒否に載せる（黙って捨てない）．RM-31・RM-32 の本文に「Provider は更新の effective instant の設定をこの関数で判定し直す（RM-7 に対する MR-18 の役目）」を足す（Provider の配線は後の段階．本文は規範だけで段階を書かない）．テスト：要求なしで `tx.c0.stream = 0`・`rx.c0.antenna = TX/RX`（同じ UBX）の設定に RM-31 が出る；上限を超える設定に RM-32 が出る；1つの key の更新（`rx.c0.antenna` だけの要求）を，すでに `tx.c0.stream = 0` の設定に重ねて `violations` で判定すると拒否になる．mutation 行を各1つ以上．
2. **throughput のテストの補強（Sol 1，major）**：`c0→s0`・`c1→s1` を両方 200 Msps（合算で超える．stream ごとの判定なら通ってしまう）；rate の違う2本の合算（例 200 と 100 Msps で超える，200 と 25 で… 実際の上限で超える／超えない境の組を選ぶ）；送信側；rate を要求せず membership（`stream`）だけの要求で超える場合．それぞれを壊す mutation 行（stream 0 だけ数える，stream ごとに別々に判定する，送信を判定しない）を足し kill を確かめる．
3. **channel の key の解析を1か所に（O2）**：`channels::concrete` と `lib.rs` の `numbered_channel` を1つにする（`lib.rs` か `keys` に置き，`coerce`・`is_configuration`・`RfEnvelopeCheck` がそれを使う）．
4. **clock の両立表を D8 の位置で引く（O3）**：同じ timekeeper の上の，種類の違う board の全ての組（slot をまたいで）について `boards_compatible` を引き，両立しなければ拒否する（今は常に両立）．同じ slot の送受の衝突の検査とは別の検査にする．RM-28 の文を合わせる．
5. **RM-28 の `*Test:*`／§6 の表を実際の assert に合わせる（Sol 2）**：要素の拒否が position と入力値を名乗ることを，各拒否のケース（未知の board，方向，frontend，timekeeper，重複など）で assert するか，表の文を検証している範囲に狭める．assert を足す方を優先する．
6. **報告の訂正（Sol 3）**：N=257 は coercion として記録され，Spec Run の既定 policy（`reject`）では拒否，Session（SB-45 の既定 `warn`）や明示の `accept`/`warn` では警告付きで格子値に適用される．spec 07 の RM-30 などに「拒否」と書いた所があれば同じく直す．
7. spec 07 の Changes 行（行を増やさない）と §6 の表をこの修正に合わせる．

## mutation

流すのは，この修正で `old` かコードが変わった行と新しい行だけ（変わった行は報告に列挙）：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ids>`（背景で）．全部 killed で，各行がちょうど1回一致すること．

## 確かめること

前回の 1〜5（`cargo test --workspace --no-fail-fast`，host clippy，Docker の `--features uhd` clippy，Python 3.14 suite，`rules.py --base HEAD`）と上の mutation．

## セルフレビュー

前回と同じ形で（新しい読み取り専用レビュー役，最大3回）．チェックリストは前回のものに加え：「O1–O3・Sol 1–3 の各項目が上の決定どおりに直っているか」．

## 報告

`~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2a/fix-report.md`．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．項目ごとに直したこと（ファイルと要点），テストとそれが捕まえる間違った実装，mutation 行と kill したテスト，確かめることの結果（数とログ），セルフレビューの各回，未解決．
