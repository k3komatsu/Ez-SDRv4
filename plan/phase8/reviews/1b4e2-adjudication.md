# 1b-4e-2：テスト計画の裁定（orchestrator）

入力：`plan-brief.md`（D1〜D6，S0〜S11），`test-plan.md`（DeepSeek），`sol-plan.md`（Sol：FAIL，major 7，minor 2）．Sol の指摘はすべて採る．計画の異論 §5（swap）も採る．以下が計画に加える決定で，指示書の D1〜D6 より優先する．

## A1：per-stream prune と moves（Sol 1，計画 §3.7・§6.1）

計画の「片側の item が消えたら move の記録も消し，floor だけ残す」は**退ける**（move の拒否と release の変化がもう片方へ伝わらなくなる）．決定：

- `Lines::prune_stream(stream, before)` は，**もう片方の line がまだ畳んでいない move の item を畳まない**：その stream の prune の境界を，そういう move の `e` より前に止める（item はそこに残る）．
- move の記録を落とすのは，両方の line の prune がその move を過ぎたときだけで，そのとき今の `Lines::prune` と同じく source の release を destination の release floor に残す．
- `Line::released` の `expect`（`timeline.rs:278`）に届く系列が無くなることをテストで示す．
- テスト（`crates/ezsdr-radio/tests/timeline.rs`）：source 先と destination 先の prune，その後の source の release の変化，その後の move の拒否で両 line が halt，両方の prune が過ぎた後の floor．どれも「prune しない Lines」との plan の比較で確かめる．
- MockRadio は prune を呼ばない（Sol が確認）．Mock の挙動は変わらない．

## A2：swap と連鎖（Sol 2，計画 §5）

計画 §5 を採る：owner は，開く集合が他の stream の streamer と衝突するとき，**自分の古い segment の処理を終えてから**（受信は cut までの sample を届け終え，送信は cut 前の burst と held の尾を出し終えてから）自分の streamer を閉じ，記録を「無し」にして，それから待つ．衝突しない reopen は今の 1 回の open のまま．

- 「他の stream が必要とする」の判定は `cut.is_none()` ではなく，**その stream の owner がまだ処理していない segment**（始まっていない，または始まって cut まで処理し終えていない）が channel を持つか，で書く．有限の cut を持つ中間 segment の連鎖（A→B→C）で解放が起きること．
- テスト：swap（受信と送信），有限の中間 segment を持つ連鎖（受信），cut 直前の sample／burst が close の前に処理されたことの観測，close の後の reopen を device が拒否した後に，次の `cold` 変更か `start_rx` で回復すること．
- swap を穴として残すことはしない．

## A3：記録の「無し」は物理的に streamer が無いこと（Sol 3）

open が成功して rate の照合（UR-13）が失敗すると，今は streamer が残ったまま記録が `Unknown` になる．決定：**照合が失敗した open は，その場で streamer を閉じる**（同じ thread が `rx_close`／`tx_close`）．これで `Unknown` は「物理的な streamer が無い」と一致する．lost の device は閉じない（F4）．UR-36 の文に 1 文．テスト：from-none の open 成功 → 照合の失敗 → その channel を別の stream へ動かす → `reconnect` が出ない．

## A4：待ちの後は plan を読み直す（Sol 4）

- 待ちが解けたら，その segment の有無，拒否，origin と configuration，stop と loss を**読み直す**．変わっていたら古い `next` を使わず，次の turn で plan から読み直す．
- 「中断」と「拒否」を分ける．中断は cursor（受信の `from`，送信の `at`）を進めない．
- 送信の owner が待っている間に届いた `Stop` の扱いを決めてテストする（held burst が取り消されること）．
- from-none（`busy` が立っている間）と，source の owner の解放とが device 呼び出しを重ねないテスト．uhd-control が from-none の間に次の Action を取らないこと（今の直列性）は別のテストで分ける．

## A5：S6 を直す（Sol 5）

- `Lines::refuse(stream, seq)` は既に move を両側で拒否する．新しく要るのは**両側の clock の再計算**（`changed(from)` と `changed(to)`）．「誤った実装」の説明を直す．
- 両 line の halt は，channel が残る S1 の形で，unit で `halted`／refused の item を見る．受信の再開と，送信の両 clock の終わり（拒否された新しい clock は origin で終わる，古いものは e で終わる）を別に観測する．
- 回復の手順は move の後の所属に沿って書き直す（空になった s0 に rate 変更を送っても open は起きない）．
- source の configure の拒否も足す．

## A6：待ちに入ったことを同期してから突く（Sol 6）

- 待ちの入口に test の probe（`Core::probe` と同じ仕組み）を置き，テストは probe を待ってから stop／loss／拒否を入れる．
- source の解放を gate で止めたまま，次の Action の予約が終わることを直接見る（uhd-control が待たないことの証明）．
- 固定の sleep と 100 ms の実時間上限を正しさの根拠にしない．状態の到達で同期し，上限は余裕のある watchdog だけ．
- `Provider::stop` の join のテストと待ちの出口の unit テストを分け，既知の #67（遠い origin の timed start が queue されたままの stop）を踏まない系列にする．

## A7：assert の訂正（Sol 7）

- S1：2 Msps・2 000 sample の call は 1 ms．stream をまたぐ stop と open の順は要らない：各 owner の stop → 自分の open，source の解放 → destination の open を見る．
- S2：destination s1 の link は `rx1`．
- S5：`tx_send n=` は sample 数．channel 数は wrapper の `samples.len()` か，channel ごとに違う波形を `transmitted` で見る．古い clock と新しい clock の held burst，cut をまたぐ repeat を足す．
- S8：timed apply は device channel を受ける．発行の instant，projection，実効値を見る．
- S10：s1 の最初の clock は move の前からある．新しい segment は `sequence` で選び，cut-at-origin で省かれる受信 segment を考える．

## A8：H4 と close の直列化（Sol 8）

- H4：最後の参照の破棄が graph の edge を外すことは **VERIFIED（UHD 4.10 のソース）**：`rfnoc_rx_streamer.cpp`・`rfnoc_tx_streamer.cpp` の destructor，`multi_usrp_rfnoc.cpp:338-354` の disconnect callback，`graph.cpp:790-806`．X300 で別の streamer へ移した後の sample は INFERRED（bench で確かめる）．
- `UhdDevice` の close は open と同じ `control` の mutex を取る（別の owner の close と open が並ぶため）．native-boundary の unit テストで，最後の Arc が残る場合と close／open の競合を見る．

## A9：S9 と文字列（Sol 9）

- 4 種の拒否が key の経路で届かない結論は採る．ただし前提（membership の projection と `Lines` の一致）を，二重 move，拒否の後の次の move，prune の後の次の move のテストで固める．
- no-line の理由は Mock と同じ `RM-34: no line for move of channel c{channel} from stream s{from} to s{to}`（`ezsdr-mock-radio/src/lib.rs:1912`）．
- design/18 の今の move の拒否の文と古いテスト名（`design/18-uhd-radio.md:268, 273` あたり）を消す／置き換える．
- `effective != e` を拒否するなら，Mock と同じく `Lines` の写しで予約してから採る（成功した `book_move` の後の return は戻しにならない）．

## A10：その他（計画 §6）

- §6.2（move の `applied` 行）：UHD も move を `applied` に 1 行書く（`{ key, claimed, read_back: null, at }`，今の empty_layout の行と同じ形）．S11 は `updates` も比べる．
- §6.5（待ちが origin を越える）：範囲外．報告の穴に残す．
- §6.6（送信の close と未読の report）：close の前に，その streamer の report を読み終える（`reading` と同じ扱い）．読めない分は穴に書く．
