# 修正の指示書：Phase 8 段階 1b-3b（第1回）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4．今の作業ツリーはあなたが段階 1b-3b で作ったもの（指示書 `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3b/brief.md`，報告 `report.md`）．orchestrator のレビュー（`opus-review.md`）と Sol 6.1 の独立レビュー（`sol-review.md`）を両方読み，下の裁定どおりに直してください．元の指示書の「守ること」はすべてそのまま有効です（`cargo fmt` 禁止，mutation は `remote_mutate.py` だけ，commit・`git add` 禁止，今のモードのテストを変えない，2分を超えるコマンドは nohup と `kill -0`，など）．

## 裁定（採るもの全部）

### F1（major．Opus O1，Sol 1・2・3 をまとめて）— `keys` instance の `cold` 変更の順序

規則（spec 07 RM-25 に，RM-27 の key の instance の規則として英文で書く．今のモードは変えない）：**`keys` instance の `cold` 変更は，同じ instance で先に受けた `cold` 変更（方向・stream・line の有無を問わない）のどれの effective instant よりも早く効かない．** 床は `self.updates` に残っている `cold` の更新の effective instant の最大（適用済みのものは今より前なので床にならない）と，今の RM-25 の床（`at`，`now`，受信の stream の開始）の最大．床で後ろにずれたら今どおり `LATE_COMMAND`．

実装：`handle_update` で，`keys` instance の `cold` は (1) まず上の床で `tick` を決め，(2) `projected(tick, order)` とその変更を入れた候補から `cold_streams` で対象の stream を選び（移動の拒否もここ），(3) その候補で `coerce`（`violations` を含む）を通し，(4) 対象の stream の line に記帳する．`asked` で選ぶ今の形はやめる．先に受けた `cold` はすべて `tick` 以前なので投影に入り，membership も throughput も全二重の判定も，先に受けたものと矛盾しない（後から挿入された更新が先の予約を無効にする道がなくなる）．spec 09 MR-18 の3つ目の *Ceiling* を消し，MR-18 を新しい規則に合わせる．

テスト（`tests/mock_layout.rs`）と mutation の行（床を掛けない／方向ごとにしか掛けない／`asked` で選ぶ，の各誤り）：
- Sol 1 の反例：`c1.stream=1` から，`c1.stream=-1 @T+5 ms`，続けて `at` なしで `c1.stream=1` → 2つ目は `T+5 ms` 以降に効き（`LATE_COMMAND`），最終状態は stream 1 に c1 があり，設定と line が一致．
- Sol 2 の membership 反例（`c1→s1 @T+20 ms`，続けて `c1→s0 @T+10 ms` → 2つ目は `T+20 ms` に床付けされ，その時の c1 は s1 なので移動として拒否）と throughput 反例（1 GbE で s0=5 Msps @+10，c1→s1 @+20，s0=20 Msps @+15 → 3つ目は +20 に床付けされ throughput で拒否）．
- 方向をまたぐ形：受信 c0 の antenna を `TX/RX` に @+20，続けて同じ board の送信 channel を stream に入れる @+10 → 2つ目は +20 に床付けされ，RM-31（全二重で受信 antenna は RX2）で拒否．
- Sol 3：link のない受信 stream の rate 変更を @+10，続けて @+5 → 2つ目は +10 以降に効き，最終の rate は2つ目の値．

### F2（minor．Opus O2，Sol 4）— 今のモードの `TxBurst` の拒否の文面

今のモード（`keys` なし）は HEAD と同じ経路に戻す：`handle_tx_burst(0, …)` の中で clock を先に，宛先を後に調べる（送信 channel がなく宛先も誤りのとき `MR-16: no transmit channel is configured`）．`keys` instance は今の振り分けのまま．

### F3（minor．Opus O3）— 「全 stream」の道

`keys` instance で `<device>` 宛ての `Stop` が全 stream（受信2本と送信2本）を止めること，`device_lost` が全 stream を止めることのテストと mutation の行．`start` 失敗時の巻き戻しは今のまま（報告の未解決に残す）．

### F4（minor．Sol 5）— stream ごとの送信 channel 数

channel 数の違う2本の送信 stream（例：s0 に2 channel，s1 に1 channel）で，同じ byte 数の waveform がそれぞれの channel 数で解釈され（長さが違う），割り切れない byte 数が片方でだけ拒否されることのテストと，channel 数を stream 0 や 1 に固定する誤りの mutation の行．

### F5（minor．Sol 6）— spec 09 の文

`design/09-mock-radio.md:40` の selector の既定値（`link_gbps` ほか）をモード別に正しく書く（`keys` instance は RM-28 のとおり `hg` で 1）．`:84` 付近の規範本文がテスト名に node・port の定義を委ねている箇所は，本文は RM-29 を引き，テスト名は `*Test:*` 行へ移す．

## 確かめること

元の指示書の 1〜7 をすべて流し直す（workspace test，clippy，Docker clippy，Python，`rules.py --base HEAD`，**足した行と，コードかテストが変わった既存の行**の mutation〈`remote_mutate.py`〉，今のモードの3つのテストファイルの diff が空）．

## セルフレビュー

元の指示書のチェックリストに「F1〜F5 がすべて直ったか，F1 の規則の『どれよりも』を全経路（`hardware_timed` は対象外であることを含む）で grep したか」を足して，新しい読み取り専用のレビュー役で最大3回．

## 報告

`~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3b/fix-report.md`．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．F1〜F5 それぞれの直し方・テスト・mutation の行（id，誤りの形，kill したテスト），確かめることの結果（数とログのパス），セルフレビューの各回，残ったこと．
