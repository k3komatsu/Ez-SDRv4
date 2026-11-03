# 設計メモ 46：受信中・送信中にできることの帳簿

Status: 第 3 版（2026-11-04，orchestrator Opus．F4 を F4a と F4b に分けた）．**owner の決定は §8**（F4a と F4b は (d)，よって F5 もなくなる．F1 は (e)．F4b は受信だけ (e) に改めた）．Sol の事実確認（19 件，すべて取り入れた．§7）を入れた．**このメモは推奨を書かない**：owner が機能ごとに利点と欠点を数えて結論を出す（owner 2026-11-04「受信しながら送信しながらできることを制限しても良い．ただし，それには私が明確にメリットデメリットを勘定して結論を出します」）．

| 項目 | 内容 |
|---|---|
| きっかけ | 設計メモ 45 の実験が二つの条件を二回続けて満たさず，owner が実験を終えた（2026-11-04，[45-report-e4](45-report-e4.md)）．次の手として owner が機能を減らす方向を選んだ |
| 問い | UHD Module の競合の不具合の多くは，流れている stream と並行して動く機能から出る（設計メモ 45 §1，棚卸し §7）．機能を一つ制限すると，その機能だけが使う lock，状態，テスト，spec の文が消える．何を制限し，何を失うか |
| 材料 | spec 18，spec 07（RM-12〜RM-16，RM-25，RM-27，RM-34），spec 09，spec 16 の EA-16，Vision §3・§27・§58・§61，Sol の棚卸し [research/45-concurrency-inventory.md](research/45-concurrency-inventory.md) §7，[failure-mechanisms](../maintenance/failure-mechanisms.md)，設計メモ [39](39-dsp-timed-commands.md)，main `50b5e83` |
| 範囲 | UHD Module と，同じ規則を持つ MockRadio と Radio Vocabulary（spec 07）．Kernel の契約と Stream Contract は変えない |
| 決め方 | 機能ごとに一つずつ．一つの制限が別の機能の扱いを変える（§5） |

## 1. 読み方

各機能（F1〜F9）に同じ欄を置く．

- **できること**：流れている間に，実験する人が何をできるか．
- **根拠**：Vision の要求か，Vocabulary（spec 07）の宣言か，Module（spec 18）だけの選択か．Vision の要求を制限するには Vision の改訂が要る．Vocabulary の宣言は v4.0 の凍結の前なので変えてよい（AGENTS.md §6）．RM の振る舞いを変えれば MockRadio も変える．Module だけの規則（F3，F8）は UHD Module だけで変わる．
- **生んだ不具合**：棚卸し §7 の群，failure-mechanisms の行，issue．一つの不具合が二つの機能にまたがるときは「共有」と書く．機能ごとの件数は足し合わせられない．
- **量**：§4 の表．どれも機能の間で重なり，制限して消える量ではない．
- **制限すると消えるもの**：その機能だけが使う仕組み．ほかの機能も使う仕組みは「残る」と書く．
- **制限すると失うもの**：できなくなること，代わりの手順，その代価．
- **制限の段階**：今のままと全部禁止の間の案．

**Session では，受信は Run の始めから流れている**（RM-21：受信の stream は Run の始めから `Stop` まで on．EA-16 の `rx.stop()`／`rx.start()`）．そのため Session で受信の設定を変えることは，そのまま「受信しながら変える」ことになる．

## 2. 制限しても残るもの

三つに分ける（Sol 17）．

**Vision と契約が求めるもの**

- T0 での時刻つきの開始，指定した時刻・sample index からの capture，repeat の継ぎ目の連続（Vision §61 の v3 の振る舞い，`design/vision/10-implementation-path-and-testing.md:211-216`）．
- Run の中で TxBurst を出すこと（Vision §58 の 9：「Reactive execution can receive an event and generate a dynamic TxBurst」）．流れている burst への preemption までは求めていない（F7）．
- overflow と sequence の gap（§58 の 6），burst を閉じて続けること（§58 の 15）．
- 装置の loss（UR-29），`Provider::stop` と片付け（UR-26 の前半），スレッドの panic と join の上限（UR-16）．

**装置の制約（X3x0 で送信を安全に続けるため）**

- EOB での閉じ方（UR-22 の基本），開いたときの T3 flush（DUC に残った header の掃除）．
- 同じ C の handle への呼び出しを直列にする（`last_error` の文字列，設計メモ 45 §3.1）．streamer ごとの call mutex（`src/uhd.rs:1124-1150`）はこのため．

**今回は残す Module の方針（帳簿に足すなら owner が足す）**

- 開始に遅れたときの時刻つきの開始の出し直し（UR-17 の LateCommand）．
- 時刻の取り直し（uhd-clock）と reference の監視（UR-27）．
- 宣言した複数の stream を，固定の構成で同時に流すこと（stream ごとの owner スレッド，`provider/mod.rs:92-109`）．F5 を全部禁止しても残る．
- 送信と受信を同時に流すこと（full duplex）．

## 3. 帳簿

### 一覧

| | 機能 | 根拠 | 生んだ不具合 |
|---|---|---|---|
| F1 | 受信中の時刻つき周波数（DDC の受け渡し，debt，drain，`left`） | Vocabulary：RM-27 の `frequency_hz` は `hardware_timed`．Vision §58 の 16（Run の中の周波数の変更を admission で拒む試験）はこの変更を前提にする | D9b-2 の 6 件（受け渡しと停止・完了など．F6 と共有），L1 の M1 と受信の窓，38a の 4 件（終わりの清算．F2・F9 と共有），流れる channel のない値を後の segment へ渡す設計（failure-mechanisms:38，計画 4 件とコード 5 件．F2・F4 共通と共有） |
| F2 | 送信中の時刻つき周波数（DUC） | Vocabulary：RM-27 | #72（untimed の続きの send が TX の周波数を取らない），#71（burst のない間の時刻つきの TX の値がたまる．burst の間に受け付ける場合），38a（共有），D9b-1a の T3 の待ち（時刻つきの TX DSP の値の前に ACK を読む） |
| F3 | 停止の時に受信の周波数を戻す（restore） | Module だけ（UR-26 の末尾） | D9b-2 の「flush の stop が失敗した後も restore する」 |
| F4a | 走っている間に stream が持つ channel の組を変える（`stream` key：channel を足す，外す，入れ替える） | Vocabulary：RM-27 は `cold`，RM-21．Easy API の `stream` の設定（EA-16） | 1b-4d-1a の 3 件（streamer の取り替え．F8 と共有），1b-4c の「owner が持つ装置の写しが古くなる」（device record，F4b と共有）．ほかは F4 共通（下の行） |
| F4b | 走っている間に sample rate と antenna を変える | Vocabulary：RM-27 は `cold`．Vision §27 は「sample rate の変更は常に `cold`」と書き，§58 の 16 は Run の中の rate の admission を試す．Easy API の `sample_rate`・`antenna` の設定（EA-16） | #35（rate の `cold` change の clock のない間の受信の fault）．ほかは F4 共通（下の行） |
| F4 共通 | `cold` change が流れている stream を切り，新しい segment を始めること（F4a と F4b のどちらでも起こる） | RM-25 | spec 22 の refusal の往復 5 件，1b-4c の O1（`cold` の booking と記帳の間で lock を離す）と再レビューの major 3，MockRadio の #45（`cold` change の stream の tail の fault），#46・#47（Stop と `cold` change．F6 と共有），#50（`Provider::stop` の時に残る `cold` change の parity） |
| F5 | stream の間の channel の move，空の stream を開く（from none） | Vocabulary：RM-34（`stream` は `cold`） | 1b-4e-2 の major 2（from none の追い越し），move の destination の開始の条件 5 件（failure-mechanisms:28），続けた move の prune（`reviews/1b4e2-sol.md:24-31`），#58（from none の開始と abort） |
| F6 | 一つの stream だけを途中で止める（`Stop`），止めた受信を開き直す（`start_rx`） | Vocabulary：RM-12，RM-16，MR-29．Easy API の `stop()` と `start()` | D9b-2（受け渡しと Stop．F1 と共有），#46・#47（共有），#67 の一部（遠い未来の開始と停止．#67 は §2 の最初の時刻つきの開始と `Provider::stop` でも起こる） |
| F7 | 送信中に，流れている burst と重なる・続く burst を予約する（preemption，続きの device burst，最後の sample の取り置き） | Vocabulary：RM-14，RM-15．Module の UR-21・UR-23 | Phase 7 K の N-P1-2（preemption の前線が send に追い越される），note 40 の Mock の framing 4 件と UHD の二重の `TIME_ERROR`（failure-mechanisms:48．基本の送信と共有） |
| F8 | 送信の閉じ方が不確かなときの recovery（開き直して T3 で掃除し，続ける） | Module だけ（UR-22） | D9b-1a の M1 と report の照合，recovery の指摘，#70（一 sample の device burst と EOB．§2 の基本の送信と共有），#75（確認のない T3 でも recovery が終わる） |
| F9 | 時刻つきの gain（受信・送信） | Vocabulary：RM-27 の `gain_db` は `hardware_timed`．Vision §27 の例に「TX gain」，§3 の Session の例に「gain を変えて，もう一度 capture」 | 38a（終わりの清算．共有）．gain だけの不具合は見つけていない |

### F1 受信中の時刻つき周波数

- **できること**：受信を流したまま，決めた時刻に受信の周波数を変える．位相の保証（MA-1）は，UBX・OBX のように timed tune で位相を保つ board で，期限の前に装置に渡せたときだけ（CBX にはない，spec 18 の profile と `:285`）．
- **抱える仕組み**：uhd-control が値を持ち，release window の中で渡す．覆う segment の chain が `Armed`／`Streaming` になるまで渡さない（eligibility）．渡した値は，受け入れの後の sample が来るまで debt として chain に残る．cut の前に drain し，残れば一度だけ `stream_now` で flush し，残れば `left` にする．`left` は close／open をまたいで残り，restore を止める．
- **制限すると消えるもの**：eligibility，debt，drain，flush，`left`，`RxChain` の状態の多く，uhd-control と uhd-rx の受け渡し（設計メモ 45 の実験の一切れ）．ただし (c) では一部が残る（下）．
- **制限すると失うもの**：受信を止めずに周波数を変えること．代わり：止めて，設定して，開き直す（F6 が残る場合）．Session でも同じ（受信は流れ続けているので）．代価：止めてから次の segment までに，release window，古い rate での受信の呼び出し一回，delivery allowance，受信の start lead（50 ms）がかかる（`crates/ezsdr-radio/src/timeline.rs:1013-1038`）．その間は sample がない．位相の保証もなくなる．
- **制限の段階**：
  - (a) 今のまま．
  - (b) 流れている間は受信の周波数を受け付けない（流れていない channel への値は，次の設定に入る）．
  - (c) 流れている segment の中の値だけ受け付ける．ただし値を渡した後に `Stop` や `Provider::stop` で cut が来ることは防げないので，debt と drain と `left` は残る（Sol 10）．消えるのは「流れる channel のない値を後の segment へ渡す」道．
  - (d) Run の間は変えない（class を外す）．§58 の 16 の周波数の部分は書き直しが要る．

### F2 送信中の時刻つき周波数

- **できること**：送信の burst の途中で，決めた時刻に送信の周波数を変える．
- **抱える仕組み**：DUC は時刻つきの send の最初の sample で値を取る（UR-22 がすべての送信を時刻つきにしたのは #72 のため）．開いたときと recovery の T3 の ACK を読むまで，時刻つきの TX DSP の値を出さない（`tx_dsp_gates`，`provider/core.rs:1540-1558`）．burst の間に渡した値は DUC の FIFO に残り，次の burst の最初の sample が取る（UR-24，現在の限界）．
- **制限すると消えるもの**：(c) なら TX DSP の gate と T3 の待ちの順，#71 と #72 の場合．(b) では残る（Sol 11：burst の間の値も FIFO と T3 の順を要る）．
- **制限すると失うもの**：送信を止めずに周波数を変えること．代わり：burst の間に変える（(b)），または止めて開き直す．
- **制限の段階**：(a) 今のまま，(b) burst の間（流れていない時）の値だけ受け付ける，(c) Run の間は変えない．

### F3 停止の時に受信の周波数を戻す

- **できること**：`Provider::stop` の後，drain と stop が済んでから，S より後に効く値か S より後に受け入れられた値を渡していた channel を，S の時点の値に戻す．未消費の値が残る channel は戻さない．
- **何のためか**：S より後の値は記録の上では取り消されるが，装置では効いているかもしれない．restore は装置の周波数を記録に合わせる（Sol 8）．FIFO に残った値を消すものではない．
- **制限すると消えるもの**：restore と，その前の走査．
- **制限すると失うもの**：Run の後の装置の周波数が，記録の S の値と違うことがある．次の Run への影響は graph の扱いで分かれる（VERIFIED，Sol 8）：同じ graph を使い続ければ，FIFO に残った時刻つきの値は後の sample で新しい untimed の設定を上書きしうる（[ddc.v:114-142](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/ddc.v#L114-L142)）．新しく graph を作れば block は flush される（[rfnoc_graph.cpp:714-722, 985-998](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_graph.cpp#L714-L722)）が，生きている graph は再利用される（同 `:1100-1121`）．ただし FIFO に残る値は restore の有無と関係しない（restore は未消費の channel を戻さない）．
- **制限の段階**：(a) 今のまま，(b) restore をやめ，記録に「装置は S の後の値のままかもしれない」と書く，(c) F1 を (b) か (d) にすれば，戻す値がなくなる．「S の後に効く値を S の前に渡さない」は代わりにならない：S は前もって分からないこと（Session の終わり，abort）があり，すでに渡した値は取り消せない（Sol 9）．

### F4 走っている間の `cold` change（F4a と F4b に分けて決める）

owner 2026-11-04：streamer を開き直すのは channel の組が変わる時だけなので，二つに分けて決める．

**F4 共通（どちらを残しても残るもの）**

- 一つでも残せば，`cold` change は流れている stream を切って新しい segment を始める（RM-25）．stream ごとの timeline の segment，cut，origin，instance 全体の `cold` の floor，refusal が stream を止める（halt）と refusal の往復は残る．
- 両方を (d) にすれば，これらが消える．ただし F6 が残れば，`start_rx` が新しい segment を作り，configure と refusal の道が残る（Sol 12）．
- 「F4 共通」の行の不具合は，片方だけを制限しても消えない．

#### F4a 流れている間に channel の組を変える

- **できること**：Run を止めずに，stream が持つ channel を足す，外す，同じ数のまま入れ替える．Easy API の `rx_channels[n].stream = k` など（EA-16）．
- **抱える仕組み**：組が変われば streamer を閉じて開き直す（UHD の streamer は作る時に channel の並びが決まる）．device record の開いた組（UR-36），open generation による report の照合，開き直しの間の受信の止まり．F5 が残れば，開け閉めの順の待ち合わせ（`held_elsewhere`，`needed_before`，`core.rs:1751-1775`）も．
- **制限すると消えるもの**：流れている間の streamer の開け閉め（F5 と F8 も制限すれば，開くのは `arm`，閉じるのは片付けの時だけになる）．device record の開いた組の比べ直し．**残るもの**：F8 が残れば streamer の取り替えと generation．F4b が残れば F4 共通．
- **制限すると失うもの**：Run の中で channel の組を変えること．代わり：Run を分ける（Session では接続し直す）．代価：`arm` と start up の時間，記録が二つに分かれること．
- **制限の段階**：
  - (a) 今のまま．
  - (b) 止めた stream にだけ受け付ける（受信は `Stop` → 変更 → `start_rx`．送信の「止めた」は F4b (b) と同じく定義が要る）．開け閉めは残るが，流れている stream を切る判断はなくなる．
  - (d) Run の間は変えない（`stream` の class を外す）．F5 もなくなる（move と from none は組の変更だから）．

#### F4b 流れている間に sample rate と antenna を変える

- **できること**：Run を止めずに，stream の sample rate と channel の antenna を変える．Easy API の `rx.sample_rate = …`，`rx_channels[n].antenna = …`（EA-16）．
- **抱える仕組み**：channel の組が同じなら streamer は開き直さない．設定し直して，時刻つきで開始する（`owner_step` が `Configure`，`core.rs:899-911`）．rate の変更は新しい SampleClock を始める（Vision §15）．antenna の read-back が拒めば stream を止める（`ur_25_a_refused_antenna_read_back_halts_the_stream`）．
- **制限すると消えるもの**：F4 共通の一部（F4a も制限した場合に全部）．rate の切り替えに伴う clock の終わりと始まり．**残るもの**：F4a が残れば F4 共通．
- **制限すると失うもの**：Run の中で rate や antenna を変えること．代わりと代価は F4a と同じ．§58 の 16 の rate の部分は書き直しが要る．
- **制限の段階**：
  - (a) 今のまま．
  - (b) 止めた stream にだけ受け付ける．受信は `Stop` → 変更 → `start_rx`．送信は「止めた」の意味を決める必要がある：送信の `Stop` は burst を終えて clock を残し，後の burst は送れる．「Stop した」「host が送っていない」「装置が無音」「後の予約がない」は同じでない（Sol 14）．
  - (c) v3 の形：instance のすべての stream を止め，変えて，揃えて開き直す（Vision §27 の箇条）．ただし F6 が残れば stream ごとの計画は残る．全体の止め方の予約の順，止まったことの確かめ方，開き直し方を決めるまでは，line と floor が要らなくなるとは言えない（Sol 14）．
  - (d) Run の間は変えない（class を外す）．

### F5 stream の間の channel の move，空の stream を開く

- **できること**：一つの channel を Run の中で別の stream へ移す．channel を持たない stream に channel を入れて開く（from none）．
- **抱える仕組み**：二本の line に一つの move を book し，source の release を待って destination を開く．holder の `Release` と `FromNone`，uhd-control が自分で streamer を開く道，move の release の計算．
- **制限すると消えるもの**：holder の `Release` と `FromNone` とその競合の扱い，from none の待ち，move の release，UR-25 の Ownership の段落の多く．**残るもの**（Sol 13）：`Free` と `Owner` と report の読みの排他（`reading`）．F8 の開き直しも owner の claim を使う．
- **制限すると失うもの**：Run の中で channel を別の stream へ動かすこと，空の stream を後から開くこと．代わり：Run を分ける．
- **制限の段階**：(a) 今のまま，(b) 止めた stream の間だけ（F4b (b) と同じ「止めた」の定義が要る），(c) Run の間は channel の組を固定（F4a (d) と同じ）．

### F6 一つの stream だけを途中で止める，止めた受信を開き直す

- **できること**：Run の途中で一つの stream を止める（受信は cut，送信は burst を終えて clock を残す）．止めた受信を `start_rx` で開き直す．Easy API の `rx.stop()`，`rx.start()`，`tx.stop()`．
- **抱える仕組み**：受信の cut と，cut をまたぐ時刻つきの値の drain（F1 と共有），止めた後の segment，開き直しの start lead．
- **制限すると消えるもの**：一つの stream の再開，F1 と組み合わせた受け渡しと Stop の競合（D9b-2）．#67 は一部しか消えない（`Provider::stop` でも起こる）．
- **制限すると失うもの**：一つの stream だけの停止と再開．代わり：Run 全体を止める．
- **制限の段階**：(a) 今のまま，(b) Stop は残し，`start_rx` での再開をなくす（なくなるのは明示の Stop の後の再開．F4a か F4b が残れば `cold` change による停止と再開と複数の SampleClock は残る，Sol 15），(c) Stop は Run 全体だけ．

### F7 送信中に，流れている burst と重なる・続く burst を予約する

- **できること**：流れている burst と時刻が重なる burst を予約して今の burst を切る（preemption）．今の burst のすぐ後に続く burst を予約して一つの device burst として続ける．
- **抱える仕組み**：preemption の前線（uhd-tx だけが進める），最後の sample の取り置き（続きが来るかもしれないので），続きの device burst，閉じ方の規則．予約済みの burst も，渡す時に preemption を判断する（`provider/tx.rs:498-524`）．続きの判断は別の分岐（`:517-537`）．
- **制限すると消えるもの**：段階による（下）．
- **制限すると失うもの**：流れている burst を後から割り込んで切ること（(b) 以上），隙間なく続けること（(c)）．
- **制限の段階**（受け付ける時の制限と，予約の中身の制限は別．Sol 16）：
  - (a) 今のまま．
  - (b) 流れている，または予約済みの burst と時刻が重なる予約を拒む．隙間のある後の burst と，ちょうど続く burst は受け付ける．preemption が消える．
  - (c) ちょうど続く burst も拒む．続きの device burst と取り置きが消える．repeat の継ぎ目の連続（§2）は repeat の中の話で，これとは別．
  - (d) 流れている間の予約をすべて拒む．流れ始める前に重なる・続く予約を受け付けるなら，(b)(c) の判断は残る．

### F8 送信の閉じ方が不確かなときの recovery

- **できること**：EOB が短かった，underflow が来た，などで device burst の閉じ方が不確かになったとき，streamer を開き直し，T3 で掃除して，次の burst を送り続ける．
- **抱える仕組み**：recovery の状態，開き直しと T3 の ACK の待ち，report の照合（generation），`Stop` が recovery を消さない規則．
- **制限すると消えるもの**：recovery の状態と道，#75 の場合，D9b-1b の多く．**残るもの**：開いたときの T3（§2）．#70 は基本の送信の閉じ方でも起こる．
- **制限すると失うもの**：不確かな閉じ方の後も，その stream で送信を続けること．代わり：その stream を止める（halt），または Run を止める．代価：underflow が一度出ると，その stream の残りの burst が送れない．
- **制限の段階**：(a) 今のまま，(b) recovery をせず stream を止めて記録する，(c) Run を止める．
- 注：Module だけの規則なので，MockRadio と spec 07 は変わらない．

### F9 時刻つきの gain

- **できること**：受信・送信の gain を，決めた時刻に変える．
- **抱える仕組み**：gain は DSP（DDC・DUC）を通らず，radio の command queue だけで効く（設計メモ 39 `:15-17`，UHD の [set_rx_gain](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L1583-L1587)・[set_tx_gain](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L2117-L2121)）．受け渡しも debt もない．流れる channel がなくても装置の queue が持つ（UR-24）．
- **制限すると消えるもの**：小さい．uhd-control の時刻つきの release の道は，F1 か F2 か F9 のどれかが残る限り残る．
- **制限すると失うもの**：Run の中で gain を変えること．Vision §27 の例と §3 の Session の例に gain の変更がある．
- **制限の段階**：(a) 今のまま，(b) 時刻なし（untimed）だけ，(c) Run の間は変えない．

## 4. 量

数え方（Sol）：各機能に関わる本体の区間の物理行数と，`#[test]` の付いた関数のうち名前が一致したものの数．`src/brain/` は除く．**機能の間で重なり，§2 の共有の実装も配っていない**．制限して消える量や，足し合わせた削減量には使えない．区間と名前の正規表現は [reviews/46-facts-sol.md](reviews/46-facts-sol.md) にあり，再現できる．

| 機能 | `ezsdr-radio-uhd` 行／テスト | `ezsdr-mock-radio` 行／テスト | `ezsdr-radio` 行／テスト |
|---|---:|---:|---:|
| F1 | 904 / 73 | 324 / 25 | 9 / 0 |
| F2 | 489 / 13 | 315 / 10 | 6 / 0 |
| F3 | 107 / 11 | 0 / 0 | 0 / 0 |
| F4a と F4b の合計（分けて数えていない） | 944 / 57 | 532 / 38 | 376 / 17 |
| F5 | 498 / 125 | 137 / 19 | 349 / 48 |
| F6 | 233 / 136 | 244 / 36 | 137 / 4 |
| F7 | 526 / 32 | 307 / 3 | 19 / 2 |
| F8 | 314 / 47 | 0 / 0 | 0 / 0 |
| F9 | （F1・F2 に含まれる） | | |

spec 18 の規則の大きさ：UR-24（時刻つきの値，F1・F2・F9）19.4 KB で名のあるテスト 141，UR-25（計画と ownership，F4a・F4b・F5・F6）16.3 KB で 132，UR-23（送信の計画と device burst，F7 ほか）9.6 KB で 67，UR-36（device record，F4a・F4b・F8）4.1 KB で 24．

## 5. 組み合わせ

- F4a を (d) にすると F5 もなくなる．F4a (b)，F4b (b)，F5 (b) は F6 を要り，送信の「止めた」の定義を要る．
- F4 共通（segment，cut，floor，refusal の往復）は，F4a と F4b の両方を (d) にしたときだけ消える．
- F4a と F4b をなくしても，F6 が残れば新しい segment と configure の道が，F8 が残れば streamer の取り替えと generation が残る．
- F4a，F5，F8 をすべて制限すると，streamer を開くのは `arm`，閉じるのは片付けの時だけになる．F4b は残しても開け閉めを生まない．
- F1 を (b) か (d) にすると，F3 の戻す値がなくなり，D9b-2 の受け渡しと Stop の競合もなくなる．
- F1・F2・F9 をすべて (d) にすると，uhd-control の時刻つきの release がなくなる．一つでも残れば release の道は残る．
- F1・F4a・F4b・F5 を制限しても，loss，終わり，届けた sample の床，clock の登録と終わり，phase，記録の共有は残る（F6 と §2，`provider/core.rs:725-870`）（Sol 18）．
- RM の振る舞いを変える制限（F1・F2・F4a・F4b・F5・F6・F7・F9）は MockRadio と spec 07 も変える．parity の比べる範囲も縮む．F3 と F8 は UHD Module だけ．
- 設計メモ 45 の判断役の形と今の形のどちらが向くかは，この帳簿の結論で変わりうる（報告 E4 §6 の 2・3 をこの後に置いた理由）．

## 6. owner に決めてもらうこと（一つずつ）

F1〜F9（F4 は F4a と F4b）のそれぞれを §3 の段階のどれにするか．順は F4a → F4b → F5 → F1 → F3 → F6 → F2 → F9 → F7 → F8（owner 2026-11-04 に F6 を F1・F3 の後へ移した：F6 を残す代価の大半は，cut をまたぐ受信の時刻つきの周波数，つまり F1 にあるため）．

v3 との比べ（orchestrator が `v3/` を読んだ，master の worktree）：v3 は controller ごとに受信・送信のループを何度でも開始・終了できた（`v3/source/controller/cyclicrx.d:282-301`，`cyclictx.d:219-228`，停止は時刻なしの `STREAM_MODE_STOP_CONTINUOUS`，`v3/cpp/uhd_usrp/multiusrp.cpp:653-681`）．単位は controller で，stream ごとではない．cut を sample で決めて記録すること，再開を新しい clock にすること，cut をまたぐ時刻つきの値を扱うことはしない．周波数と gain は設定ファイルで開始の前に与え（`multiusrp.cpp:127-133,236-255`），流れている間に変える `setParam` の周波数・gain の分は comment にされている（`multiusrp.cpp:525-560`）．

## 7. Sol の事実確認

2026-11-04，`openai/gpt-6.1-sol`，読むだけ．19 件（FACTS NEED CHANGES）．すべて取り入れた．主なもの：#70・#71・#72 の帰属（1），F3 の帰属と restore の目的（2，8，9），抜けていた履歴（3，4），#67 は F6 だけのものではない（5），Vision の根拠（§58 の 9・16，§27，§3，6），F9 と固定の複数の stream の分類（7），F1 (c) でも drain は残る（10），gain は DSP を通らない（11），F4・F5 で残る仕組み（12，13），送信の「止めた」の定義（14），F6 (b) の言い方（15），F7 の段階の分け方（16），§2 の三分（17），§5 で残る共有（18），量の数え方と F1 の位相の条件（19）．記録 [reviews/46-facts-sol.md](reviews/46-facts-sol.md)（答え），[reviews/46-facts-sol-brief.md](reviews/46-facts-sol-brief.md)（ブリーフ）．ログは `~/.cache/ezsdr-fixes/2026-11-04-n46/`．

## 8. owner の決定

| 日付 | 機能 | 決定 | owner の理由 | 組み合わせで決まったもの |
|---|---|---|---|---|
| 2026-11-04 | F4a | (d) Run の間は channel の組を変えない（`radio.<d>.c<n>.stream` の class を外す） | v3 も同じ．用途があまり思いつかない | F5 もなくなる（move と from none は組の変更）．F5 は決めることがない |
| 2026-11-04 | F4b | (d) Run の間は sample rate と antenna を変えない（`radio.<d>.s<k>.sample_rate_hz` と `radio.<d>.c<n>.antenna` の class を外す） | rate はユーザーのプログラムで resampling すればよい．antenna の変更も要らない | F4a と合わせて radio の key に `cold` がなくなる．F4 共通（`cold` change が stream を切って新しい segment を始めること，instance 全体の floor，refusal の往復と halt）は，F6 の `start_rx` の分を除いてなくなる |
| 2026-11-04 | F1 | (e)（帳簿の外の段階．owner の案）：受信の周波数は，止めた受信を開き直す `start_rx` に parameter として付けてだけ変えられる（Easy API なら `rx.start(at, frequency=…)`）．流れている間の `UpdateParameter` では変えない | 止めた stream を開き直すときだけ許せばよい．値と再開を一つの命令にすれば，別々に届いた値と再開の時刻の前後の規則も要らない | 受け渡し，debt，drain，flush，`left`，eligibility，F3 の戻す値がなくなる．設計で決めること：key の class（Vision §27・§65 の 40 により要る．`cold` が合う），`start_rx` の parameter への RF envelope の確かめ（§58 の 16 の試験の書き直し），止めている間の tune の位相（時刻なしなら保たない），MockRadio と RM-12・MR-29 の変更 |
| 2026-11-04 | F4b（改め） | 受信の sample rate と antenna も F1 と同じく `start_rx` の parameter としてだけ変えられる（(d) を改めた）．送信の rate と antenna は F2 と一緒に決める | 技術的に問題がなければ変えられるとよい（owner）．channel の組が変わらないので streamer は開き直さず（`owner_step` の `Configure`），`start_rx` はもともと新しい SampleClock を始める．流れている stream を切る判断と instance 全体の floor は戻らない | 足すもの：再開の時の profile と throughput（RM-32）の確かめ，duplex の board の antenna の規則（RM-31）の確かめ．**条件**：開いたままの streamer で，止めている間に rate を変えて開始し直すことを実機で一度確かめる（FakeDevice でしか通っていない．棚卸し §4 も UHD の保証は見つからないとする） |

## Changes

| 日付 | 変更 |
|---|---|
| 2026-11-04 | 初版（orchestrator Opus） |
| 2026-11-04 | 第 3 版：owner の指示で F4 を F4a（channel の組，streamer を開き直す）と F4b（rate と antenna，開き直さない）に分けた．両方に共通の仕組みと不具合は「F4 共通」に置いた |
| 2026-11-04 | 第 2 版：Sol の事実確認 19 件を入れた．F9（時刻つきの gain）を分けた．§2 を三つに分けた．§4 に量を入れた |
