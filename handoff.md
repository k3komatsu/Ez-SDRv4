# Ez-SDR v4 — Handoff (2026-10-19)

次のセッション（人間・AI どちらでも）が最初に読む現状メモ．**今のことだけ**を書く：どこに何があるか，何が未解決か，owner の判断待ち，次に何をするか．終わった作業の記録はその置き場（`plan/phaseN/00-overview.md` か各計画ファイル）に書き，ここには1行の参照だけ残す（AGENTS.md §9．`rules.py` が 300 行・40 000 byte を超えると落ちる）．2026-10-14 までここにあった記録も同じ日に各置き場へ移した．

開発の恒常的なルールは [AGENTS.md](AGENTS.md)．

## 1. 今の状態

- **Phase 1〜7 は Gate X 受理済み**．Phase 7（UHD Provider `ezsdr.radio.uhd`）は 2026-10-01 に X300＋OBX で bench B0–B9 を通してクローズ（[plan/phase7/00-overview.md](plan/phase7/00-overview.md) §11，[bench-results.md](plan/phase7/bench-results.md)）．accepted specs の一覧は AGENTS.md §1・§2．
- **凍結前の改修（項目1〜8）は完了**（[plan/maintenance/24-prefreeze-audit.md](plan/maintenance/24-prefreeze-audit.md)）．最後の項目8（spec 01〜06 を DA-1 形式に）は 2026-10-14 に完了（[計画31](plan/maintenance/31-item8-kernel-specs.md) §10）．その前の保守の記録は各計画ファイル（Phase 7 クローズから spec 20 までは [audits-2026-10.md](plan/maintenance/audits-2026-10.md)）．
- **spec 22 の DA-3 手順2（spec 07・09・18 を DA-1 形式に）は 2026-10-15 に完了**（spec 22 の Status 行）．残る手順3・5 と文書監査の残りは §3．
- **Phase 8（Mock ↔ X310 parity）は Gate P 受理**（owner 2026-10-17）：計画 [plan/phase8/00-overview.md](plan/phase8/00-overview.md)（判断 P1〜P6，段階，bench BP0〜BP5，出口条件），設計 [33-channels-and-streams.md](plan/phase8/33-channels-and-streams.md)（channel と stream，混在 board，`x3xx`，Kernel の変更は `KeyDecl` の番号付き segment だけ）．レビューと調査は `plan/phase8/reviews/`・`research/`．
- **Phase 8 段階1a，1b-1，1b-2a，1b-2b，1b-3a，1b-3b，1b-3c，1b-4a，1b-4b，1b-4c，1b-4d-1a，1b-4d-1b，1b-4d-1c は完了**（2026-10-17〜20）：1a は X3xx の description を `ezsdr_radio::x3xx` に1か所（profile `x3xx` 0.1.0，rate の格子，CBX の `random`，方向別の start lead），1b-1 は `KeyDecl` の番号の付く segment（SB-2a），1b-2a は `radio` 語彙の channel・stream の key と layout の builder（`ezsdr_radio::channels`，spec 07 RM-27〜RM-33），1b-2b は timeline の channel の集合と D3 の移動（`timeline::Lines`，RM-34）と D14，1b-3a は MockRadio の内部を stream ごとの状態と `Lines` に組み直し（挙動は不変），1b-3b は MockRadio が selector の `keys: "channel"` で新しい key を使う，1b-3c はその instance に channel の移動（RM-34）と SimulationChannel の論理 channel 番号・channel ごとの gain/frequency/phase を足した（始まった segment は全 channel の untimed tune，MR-34），1b-4a は UHD Module の device 境界を stream の番号・subdev spec・rate の集合・serial・antenna の read-back に広げた（`UhdRadio` の挙動は不変），1b-4b は `UhdRadio` の内部を stream ごとの状態・`timeline::Lines`・stream の列に組み直した（挙動は不変），1b-4c は UHD Module の `keys: "channel"` のモード（方向ごとに stream 0）と open のときの検査（subdev spec，front end，serial，rate の集合）を入れた，1b-4d-1a は stream ごとに device が実際に持つもの（集合，configure の `(seq, 版)`，streamer の世代）を 1 つの記録にし，owner の写しを消した，1b-4d-1b は stream の device を呼ぶ担当（from-none の間は uhd-control の `busy`，uhd-tx の report の読みの `reading` を from-none が待つ）と loss・stop の出口を入れた，1b-4d-1c は timed command の効く時刻を booking で確定して `e` の順に device へ出し，channel の無い間の更新も pending の segment に届けて，configuration の版を消した．記録は [計画](plan/phase8/00-overview.md) §8．
- 実装：13 crate（`crates/`），schema（`schemas/`），Python package（`python/ezsdr`）．確認の手順は §5．
- open issue は5件：#51，#56，#59，#62，#67（§3）．

## 2. 次にすること

1. **実機**：2026-10-20 に BP0 と BP5 を 2 台で済ませた（[bench-results.md](plan/phase8/bench-results.md)：Phase 7 の hardware テスト 29 本が #1＋OBX・#2＋UBX とも通り，悪くなった数値は無い）．今つながっているのは #1＋OBX．組み換えは owner に頼む（2026-10-20 夜は翌朝まで不可）．
2. **Phase 8 の段階 1b-5**（切り替え：server・Python・fixture を新しい key に移し，今の key のモードを消して畳む．[設計 33](plan/phase8/33-channels-and-streams.md) §5）．1b-4e-2（UHD の moves，RM-34）は 2026-10-21 に完了（[00-overview](plan/phase8/00-overview.md) §8，[reviews/1b4e2-opus.md](plan/phase8/reviews/1b4e2-opus.md)）．**owner の指示（2026-10-20）で 1b-4e-2 の gate の後は止めている．1b-5 は owner の再開の指示を待つ．**
   - **#67**：遠い origin の timed start が queue されたまま `Provider::stop` が来ると，uhd-rx は join の 1 s で detach され，cleanup は `close_streams` を呼ばない（1b-4d-2 の Sol の計画レビュー．#56 の bench を待つ）．
   - bench 行き：idle の DDC/DUC の timed frequency と fake の FIFO（1b-4d-1c），2 本の rx streamer と block ごとの queue（1b-4d-2）．今の key の queue の会計は 1b-5 で消える．
   - 1b-4e-2 の実機（2026-10-21，[bench-results](plan/phase8/bench-results.md)）：閉じた streamer の front end を別の stream で開くのは受信・送信とも VERIFIED．**持ったまま 2 本目を開くと，UHD は拒否するが持っている streamer の edge を壊し，プロセスが abort する**．FakeDevice は同じ日から，その open を拒否したうえで fake の drop でテストを失敗させる（UR-33，`take_abort`）．
   - 4d-3 の持ち越し：`rx_segment` の timing 行の `channels` は数のまま，plan の e2e は Session の Manifest ファイルまで通さない．4e-2 の持ち越し：待ちが origin を越える場合，close の時点で読めない TX の report，`effective != e` の到達性（[1b4e2-opus](plan/phase8/reviews/1b4e2-opus.md) §1）．
   - **1b-5 の次の段階**（owner 2026-10-21）：UHD Module の stream ごとの状態を 1 つの明示的な状態にまとめる．`stream-lifecycle-scattered` が 1b-4d-2・1b-4e-2 で 5 件（loss の周り）出たため（[failure-mechanisms](plan/maintenance/failure-mechanisms.md)）．今は `StreamState` の印（`set`，`configured`，`idle`，`busy`，`busy_seq`，`reading`，`active`／`completed`）と `device_gate`・`lost` を各 thread が別々に読み書きしている．設計メモ（印を状態と遷移に置き換える案，消せる印，テストで固める遷移）から始める risky な段階．
   - mutation の見張り（`mutation_watchdog.py`）は，削除の mutant（`new` ⊂ `old`）と `new` がよくある語の行で，実装役の当て直し中に誤報を出す（1b-4e-2 で 4 回）．直すか，書き換えの多い段階では行の検査を外して gate で全行の一致を確かめる．
   - **大きさ**（owner 2026-10-19）：1 段階は本体とテストで約 1,500 行．指示書でテストの繰り返し（Manifest の section を引く式，拒否の理由，device の準備）を helper にさせ，テスト計画は順序ごとに assert を 1〜2 個に絞る．
   - **手順**：AGENTS.md §9．計画を先に書く手順の評価は 4b・4c・4d-1c・4d-2 の記録（[1b4d2-opus](plan/phase8/reviews/1b4d2-opus.md) §4 など）．
   - **持ち越し**：
     - [1b-4a の修正の報告](plan/phase8/reviews/1b4a-fix-reports.md) の §9（修正の回 1・2）．
     - [1b-3c の報告](plan/phase8/reviews/1b3c-report.md) §10．
     - [1b-3c の裁定](plan/phase8/reviews/1b3c-opus.md) の minor 1（RX と TX の同じ origin の引き順のテスト，1b-5）．
     - 1b-4c の O5：front end の照合が 2 つのモードの腕に分かれている（1b-5 で畳む）．
   - **その後**：
     - 2（#51 の設計メモ→#51・D2，#62 と D9 の DDC/DUC の release の規則，#59，A2 のテスト）．
     - 3（測定記録・比較テスト・測定スイート・ホスト設定の script）．
     - 段階 1 が終わったら，crate ごとの冗長の棚卸しを 1 回（ponytail の audit と Opus のレビュー，owner 2026-10-19）．
   - **実装役**（owner 2026-10-19）：判断の多い段階（設計の作り替え，テスト計画，並行処理）は DeepSeek（`opencode-go/deepseek-v4.1-flash#max`），指示の決まった小さな修正の回は Claude Code の Haiku のサブエージェント（`model: haiku`）．回ごとに枠の消費を owner に報告する．セルフレビューは DeepSeek．1b-4d-1a では 5 回とも判定を出さずに終わったが，1b-4d-1b でチェックリストを 4 項目にし，`timeout 2400`，mutation を流さないと明記したら 3 回とも判定が出た．OpenCode2・Claude Code・Codex には ponytail が入っている．
3. **実機の残り**：複数 stream の layout（H1，1b-4e と段階 3 の後），CBX，混在構成（BP4b），XG（BP4c）．`DeviceAuthority` の UR-7 のやり直しはまだ一度も働いていない．用意できる機材（2026-10-16）：X300 × 2，OBX，UBX（壊れているかも），CBX，LFTX・LFRX・BasicTX・BasicRX（どれも2枚ずつ可，ただし他の実験から借りるので手間）．揃ったら最初に BP0（probe，UBX が動くか，2台目の FPGA image の版と HG/XG）．各段階で要るもの（根拠：Vision §67〈`design/vision/11-boundaries-invariants-and-roadmap.md:317-337`〉，`plan/phase7/00-overview.md:103`「One device is enough for §59's claim」）：
   - **8**：1台．Phase 7 と同じ X300（X310 の代わりで可）＋OBX 1枚のループバック（TX/RX → 30 dB 以上の減衰器 → RX2），10 GbE の Linux PC．外部 10 MHz/PPS 不要．X300 は 2026-10-01 から未接続なので，つなぎ直して `uhd_usrp_probe` で確認する（§6）．
   - **8b**：USRP 2台（2台目は X310/X300 なら同じ driver と profile．USRP2 は不要〈owner〉），10 MHz/PPS の分配器（OctoClock など．X3xx の背面から配れるかは未確認），2台の時刻差を測る RF 配線（1つの信号を分配して両方の RX へ，減衰器付き．推測）．2台目の 10 GbE は usrp-lnx03 の X710 の空きポート（§6.2）が使える．
   - **8c**：8 と同じ1台．FPGA image に Replay・DDC/DUC（・FFT）の block があるかを `uhd_usrp_probe` の block 一覧で確認．
4. **その後の順番**（owner 2026-10-10，Vision §67，[28-forward-audit.md](plan/maintenance/28-forward-audit.md) 判断7）：8 → 8b（複数台：1 instance・1 root，判断1．`Session.aligned(at)`・`next_pps(k)`・`EpochRef::Pps` は実装済み，U12 は 8b）→ 8c（RFNoC：Replay，DDC/DUC．device FFT は FPGA image に block があるときだけ．UHD の RFNoC API は C++ のみなので UHD Module に薄い C++ 層が要る）→ 10（Processor/Reactor の実行エンジン，最小の PDU 経路）→ 全規則の確認 → 凍結レビュー → **v4.0 凍結** → 11+（Packet/PDU + TUN/TAP＝旧 Phase 9，WASM，GPU…）．

## 3. 未解決と owner の判断待ち（一覧はここだけ）

| 項目 | 中身 | いつ・どこで |
|---|---|---|
| #67 | 遠い origin の timed start が queue されたまま `Provider::stop` が来ると uhd-rx が join の 1 s を越えて detach され，streamer が漏れる（HEAD から） | #56 の bench の後 |
| #62 | MockRadio が UHD の device queue の順番どおりの release を模擬しない（Mock が実機より緩い）．VH-8 harness は `timed` の25系列を除外中 | Phase 8 段階2（P4） |
| #59 | MockRadio に「cold change の read-back 拒否」の fault がない | Phase 8 段階2（P4） |
| #51 | 送信 Stop で held burst を捨てるとき UHD は `COMMAND_REJECTED` を出し MockRadio は黙る．先に「イベントの報告規則も1か所で実装するか」を決める（`mech:radio-rule-implemented-twice`） | Phase 8 段階2（P4．先に設計メモ） |
| #56 | X310 で start の e₁〜e₂ 間の Stop と Stop の処理遅れを測る | Phase 8 の bench |
| H1〜H5 | H1：開始が遅れた複数 packet の burst．H2：burst 途中の Underflow（再開時刻，ずれ）．H3：TX の `max_num_samps`．H4：負荷下の uhd-tx・uhd-control の起床遅延．H5：保持した最後のサンプルの 2 ms 期限 | Phase 8 の bench |
| 方向別の start lead | owner 決定（2026-10-08）：`radio.timing.start_lead_ns` を `rx_start_lead_ns` と `tx_start_lead_ns` に分けた（段階1a で実装，両方 50 ms）．値は Phase 8 の bench で測る | Phase 8 の bench |
| 測って決め直す値 | 送信と受信の start lead（今は 50 ms，INFERRED），cold change・再開で受信が途切れる時間（約55 ms〜221 ms），UR-17 の受信要求長の上限（残すと決定済み，実測で見直す） | Phase 8 の bench |
| 保守表の 3・4 | 3：fake は TimeError で burst 全体を捨てるが FPGA の既定は packet 単位（H1 の後に UR-33 と fake を直すか）．4：保持した最後のサンプルの 2 ms 期限（設計の代償として受け入れるか見直すか）．表は [audits-2026-10.md](plan/maintenance/audits-2026-10.md) の「Phase 7 後の保守」（1・2・5 は解決済み） | Phase 8（owner） |
| spec 22 の残り | 手順3（DA-5 (b)：spec 11 を 08 へ統合）と手順5（spec 10・14・16）．受理済み（2026-10-07）で未実施，Phase 8 の後（手順3は取りやめも owner に確認）．手順2（spec 07・09・18 を DA-1 形式に．文書監査の D4・D5・D3 の spec 側・D11 を含む）は 2026-10-14〜15 に済み，記録は spec 22 の Status 行と各 spec の Changes 行 | 手順3の取りやめ（owner） |
| 文書監査の残り（2026-10-14） | D2（RX_OVERFLOW の `restart_gap_ns`・`lost`・件数が Mock と UHD で違う，#51 と同じ機構）・D10（値ごとの「緩い向き」）は Phase 8 の計画で（D12〜D14 は 2026-10-16 に済み：`rules.py` が OV/PO の ID を守る，AGENTS.md §2 に OV/PO の扱い，凍結チェックリストに PO，AGENTS.md と handoff の重複を削った） | Phase 8（D2 は段階2，D10 は P2 の比較テスト．D3 は段階1a で済み） |
| テストのない条件（2026-10-16） | Test 行の網羅調査（[計画32](plan/maintenance/32-test-line-sweep.md)）が `none named` に書いた 130 条件を仕分けた．A1（30 件，Kernel・Sink など）はテストを足し済み（2026-10-16，T01〜T33）．A2（14 件，RM/MR/UR の overflow・再開まわり．D2・#51 と同じ所）と B（4 件，実機が要る）は Phase 8 の計画に入れる．C（82 件）は `none named` のまま | Phase 8（A2 は段階2，B は段階4） |
| Provider の id を名乗る写し | 受け取り時の拒否で id を名乗る処理が MockRadio・UHD に約15か所ずつ（MR-18・UR-26 の一部は未固定）．1か所で id を渡す形にまとめると減る | 任意 |
| 規則ごとの確認への記録 | TM-21 の budget/block period 比較は未実装．TM-16c の既存テストは ManualTimeAuthority で paced Authority の規則を固定しない．SC-29 の SOB なし time jump の `requested_target` 不在と SOB 付き time jump の組合せは未固定．Authority の `now/schedule/cancel` の panic の扱いは規則との対応を要確認 | 凍結前の全規則の確認（§4） |
| feature `uhd` の mutation | `uhd.rs` の規則は mutation の gate の外にある．対象は handle の寿命，streamer の map，空の列の channel 0，`serial`・`rates`・antenna の read-back．mutation の image（`plan/maintenance/tools/remote/Dockerfile`）に libuhd と header がなく，行に feature を渡せないため．今は docker の `--features uhd --lib`・`--test uhd_api` だけで確かめている．image と行に feature を足すか（1b-4a の Sol の再レビュー 2） | Phase 8 の中（1b-4c までに判断） |
| 負荷に弱いテスト | maintenance/J293 の無変異テスト `ur_26_the_shutdown_s_last_burst_ack_waits_on_the_device_s_clock` が16 shard の負荷下で1回失敗（2026-10-14）．2026-10-17 に手元の `cargo test --workspace` で4回再発（1b-2a で1回，1b-2b の修正中に3回．単独では毎回通る．どの差分も UHD の挙動を変えていない）．原因（2026-10-17）：テストの thread が `now` を読んでから送るまでに負荷で約 18 ms を超えて遅れ，`send`（DropAndFlag）が burst を遅れとして落としていた（plan 30 §5(c)）．テストの verb を `start_repeat`（SendAsapAndFlag）に替えた（J293 は kill のまま）．同じ形の遅れは `ur_21_a_descheduled_timed_send_…` の manual の分岐にもある（再発したら同じ直し）．この類が続くなら plan 30 の段階 B は owner の判断．同じく phase7/U17（`tx.rs` の repeat の終わり）が 2026-10-14 の gate で負荷下に1回生き残り，単独の再実行で kill（変異を見逃すテストは本当の生き残りを隠しうる）．Python の `test_v54_sleep_is_run_time` が Mac で 10 s の上限をわずかに超える記録（2026-10-07，その後は未確認） | 再発時 |
| server の小さな文言 | `Ran.path` の doc と schema の説明に「書けなかったときは無い」を足せる（第6監査） | 任意 |
| 後の phase へ | Session replay と artifact store（frontend）．Phase 10 へ（Phase 5 から）：event edge，component parameter を適用する Executor（UC-2〜UC-6，MA-24），component の budget を仮想時間で課す，MA-30 の Action latency．Phase 2 の test ceilings は [implementation-notes.md](plan/phase2/implementation-notes.md) | 各 phase |
| GitHub Pages | https://k3komatsu.github.io/Ez-SDRv4/ の状態表示は 2026-10-11 時点の実装と 2026-10-01 の実機確認まで反映済み．更新は owner の依頼があるときだけ（AGENTS.md §1） | owner |
| v3 → v4 の残り | (1) ghcr の image 名 `ezsdr:latest` を v3 と取り合う：v4 の名前を変えるか v3 利用者を固定 tag へ．(2) v3 readme の `k3kaimu` と現 owner `k3komatsu` の不一致（未確認）．(3) v3 の readme に `Ez-SDRv4` への1行 | v4 リリース前（owner） |

## 4. v4.0 凍結のチェックリスト（残り，Phase 10 の後）

済んだ項目（項目1〜8，F19・F21）と根拠は [24-prefreeze-audit.md](plan/maintenance/24-prefreeze-audit.md)．凍結レビューは次の全項目が済んでから：

- [ ] 全 accepted spec の規則と OV・PO 規則を DeepSeek で1つずつ確認：3つの決まった問い，JSON，根拠必須．Claude は印の付いた行を検証し，残りを抜き取り（[28-forward-audit.md](plan/maintenance/28-forward-audit.md) 末尾）．Test 行の確認は [計画32](plan/maintenance/32-test-line-sweep.md) の `tools/sweep/` で流し直す．§3 の「規則ごとの確認への記録」もここで．
- [ ] OV・PO 規則の置き場：今は `plan/phase1/00-overview.md`・`plan/phase2/00-overview.md` の経緯の中にあり，`rules.py` は ID だけを守る．`design/` へ移すかを凍結レビューで決める．
- [ ] MA-16a の公開 API 許可リスト（`kernel_surface_allow.txt`）の見直し．
- [ ] 文書の種類ごとに受け付ける版を設定する（今は共通の `SUPPORTED_VERSIONS`．項目5で凍結時に延期，[24-prefreeze-audit.md](plan/maintenance/24-prefreeze-audit.md) の 2026-10-09 item 5 の行）．
- [ ] Kernel API を 4.0.0 に，`SCHEMA_CHANGELOG.md` の見出しを凍結後の形式に．凍結までは版を上げず changelog も書かない（AGENTS.md §6）．
- [ ] 凍結後の OV-12 を道具で守る：`EZSDR_UPDATE_SCHEMAS=1` は形の変わった `*.v1.json` を書き換えず拒否する（説明文の修正だけ許す）．

## 5. 進め方と道具

- **分担と方針**は AGENTS.md §9（分担・順番）と §6（owner の方針）．Sol の残り枠は `python3 plan/maintenance/tools/codex_limits.py`．
- **実装役への指示書に毎回書くこと**：mutation は `remote_mutate.py` だけ（手元の `mutate.py` も作業ツリーへの mutant も禁止，実験はリポジトリ外のコピーで）．`cargo fmt`/`rustfmt` 禁止．手元（usrp-lnx03）で負荷をかけない．セルフレビューはフォアグラウンドで終わるまで待つ．DeepSeek のツールは1コマンド120秒で打ち切るので，長い処理（`rules.py --sentences` 約5分など）は時間を延ばすよう書く．前の段階で出た誤りの型は禁止として列挙する（8g で効いた）．
- **gate**（段階の最後）：
  - mutation 全リスト：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8`（約8分）．既知の生き残りは phase7/G09．結果が欠けた行は ID を並べて再実行（zsh では `$ids` が分割されない）．phase7 G16 と phase6 G21 は timeout で kill が正常．shard × 4 CPU は1ホスト32コア以内．
  - 手元：`cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`，`cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`，Python `PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=~/.cache/cargo-target/Ez-SDRv4/debug/ezsdr-server ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests`．GY-7 の Python 3.9 は container で：`docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -e PYTHONDONTWRITEBYTECODE=1 -e EZSDR_SERVER=/srv/ezsdr-server -v "$PWD:/work:ro" -v ~/.cache/cargo-target/Ez-SDRv4/debug/ezsdr-server:/srv/ezsdr-server:ro -w /work python:3.9-slim sh -c 'pip -q install --user numpy && python -m unittest discover -s python/tests'`．
  - 文書：`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base <変更前の commit>`（`--self-test` で道具自体を確認）．
- **見張りと機械チェック**（モデルなし）：
  - `plan/maintenance/tools/mutation_watchdog.py`：Mac の `mutate.py`・`cargo fmt`/`rustfmt`・作業ツリーに当たった mutant・親のいない／20分を超える test を見つけたら終了して知らせる．実装役を動かす間バックグラウンドで流し，gate を閉じたら止める．判定は実行中のスクリプト名で（文字列一致は誤検出した）．
  - 段階ごとの道具（`~/.cache/ezsdr-fixes/2026-10-13-item8/` の `check05.py`・`drive04.sh`・`watchdog-04.sh` が雛形）：指示書の機械で確かめられる項目を検査し，未達を新しい実装役セッションへ最大3回差し戻す．**起動前に `--calibrate` などで誤検出がないことを確かめる**（8f では誤検出3件で差し戻しが空回りした）．
  - セルフレビューのレビュー役が API 待ちで固まることがある（8g で約20分，CPU 0%，ログが進まない）：ログの更新時刻を見て，レビュー役だけを止める．
- **sim02/sim03**：`ssh sim02`．usrp-lnx03 からは直接つながる．
- **外部モデルの運用で分かったこと**：DeepSeek は有料の `opencode-go/deepseek-v4.1-flash`（無料版は並行で数分のうちに 429）．Sol は同時に起動すると全部 0 byte で固まる：1本ずつ，前のものが書き始めてから起動する．出力が約4分 0 byte なら止めてやり直す．DeepSeek は `Session interrupted: inactivity` で途中終了し古い報告の STATUS を残すことがある：報告だけでなくログを見る．

## 6. 実機環境と開発 PC（Phase 8 から．2026-10-16 に移行を決定）

手順の本体は [plan/phase7/bench.md](plan/phase7/bench.md)（B0–B9，RF 安全，合格条件）．結果は [bench-results.md](plan/phase7/bench-results.md)．

### 6.1 Mac からの移行（2026-10-16，残り2項目）

済み（2026-10-16）：`restore.sh`（メモリ・`.cargo/config.toml`・post-commit hook・`v3/` worktree．`v3/` の引用はすべて解決），venv（Python 3.14 + numpy），opencode2 の3モデル（Sol は応答も確認），`codex_limits.py`，ssh sim02/sim03，image `ezsdr-v4-dev:uhd4.10` の再 build．usrp-lnx03 で gate が通った：`cargo test --workspace`（1139 passed），clippy（host と image 内の `--features uhd`），Python suite（3.14 と 3.9 で 47 passed），`remote_mutate.py all` は 767/768（生き残りは既知の phase7/G09）．ホストの UHD に pkg-config が無いのは問題ない（`--features uhd` は image の中だけ）．

- [x] X300 をつなぎ，`uhd_usrp_probe` で見えることを確かめる（2026-10-20，2 台とも．[bench-results.md](plan/phase8/bench-results.md)）．
- [ ] ホスト側の NIC と sysctl の設定（下の 6.3）．`sudo` にパスワードが要るので owner が流す．2026-10-20 は tree の外の `~/ezsdr-bench/usrp-net.sh`（`set_addr.sh` を縮めたもの）で当てた．再起動で戻る．段階 3 でこのリポジトリの script にする．
- [ ] 2つとも済んだら，この 6.1 を消して1行の記録にする．

### 6.2 usrp-lnx03

| 項目 | 値 |
|---|---|
| サーバー | `usrp-lnx03`（学内に常設．`ssh sim02`・`ssh sim03`・`ssh ydev` は直接つながる．踏み台の設定は要らない） |
| Rust / UHD | ホストの Rust で普段の開発をする．標準の build は libuhd を使わない（UR-35）．ホストの UHD は 4.9 で，X300 の FPGA は UHD 4.10 の image（書き換えない）．互換番号が合わず，ホストの `uhd_usrp_probe` は弾かれる（別プロジェクトで確認済み）．そのため `--features uhd` の build・clippy と実機テストは，すべて Docker image `ezsdr-v4-dev:uhd4.10`（`docker/uhd4.10/Dockerfile`）の中で行う．build 出力は volume `ezsdr-v4-cargo-target` |
| NIC | 10 GbE × 4（X710，driver i40e）：`enp1s0f0np0`・`enp1s0f1np1`・`enp1s0f2np2`・`enp1s0f3np3`．X300 との対応は配線で決まり，NIC 名からは分からない．ホスト側の IP は `ip` で X300 の subnet に合わせる |
| 装置 | #1：USRP X300（serial 347D545）．FPGA は UHD 4.10 の HG image．slot A に OBX，slot B は空．`ARGS=addr=192.168.40.36`，`enp1s0f0np0`．#2：USRP X300（serial 347D541），`addr=192.168.40.37`，`enp1s0f1np1`，UHD 4.10 の HG image（2026-10-20 に書き換え），slot A に UBX-160 v2，slot B に CBX，UBX の TX/RX → 30 dB → RX2．2台とも 192.168.40.x なので，同時につなぐときはホストに `/32` の経路を足す |
| RF | OBX の TX/RX → 30 dB 減衰器 → RX2．利得 0 dB，振幅 ≤ 0.5，1 GHz（envelope 999–1001 MHz） |
| 既知の癖 | ref PLL の lock 失敗が open の約4%（UR-7 の1回の開き直しで救える）．X300 は受信の timed stop を無視する（F1，対処済み）．抜線すると UHD が static teardown で abort する（UR-29） |

### 6.3 200 Msps を落とさないホスト設定

ホスト側の設定はコンテナからは変えられない．別プロジェクト（v3 で X300 3台を 200 Msps）の `ibfd_sefdm/set_addr.sh` が実測で決めた．このスクリプトは Phase 8 の計画でこのリポジトリに移す（X300 のアドレスは `EZSDR_UHD_ARGS` か引数から読む）．どの設定も再起動で戻る．

- **NIC の PCIe ASPM を切る**．同じ物理リンクの全ファンクションで切る．これが受信の取りこぼしの本当の原因だった．2026-10-13 にパケット損失率が 4.0 % から 0.00 % に下がった．深い C-state ではなかった．
- **MTU 9000**．X300 の ICMP には FPGA の ZPU が答えるので，大きい ping は返らなくて正常．ジャンボが効いているかは UHD のログ `[X300] Maximum frame size: NNNN bytes.` で見る．8000 前後なら効いている．
- **sysctl**：`net.core.wmem_max` を 32 MiB，`rmem_max` を 50 MB，`netdev_max_backlog` を 30000 にする．恒久化は `/etc/sysctl.d/`．
- **NIC のリング**を上限まで上げる（`ethtool -G`）．
- **NetworkManager の管理から外す**．外さないと DHCP のつなぎ直しが手で付けた IP を消す．

実行の形（`-w /work` を忘れない）：

```sh
docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" \
  -v "$PWD:/work" -v "$HOME/ezsdr-bench:/bench" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
  -e EZSDR_UHD_ARGS=addr=192.168.40.36 ezsdr-v4-dev:uhd4.10 \
  cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware <test> -- --ignored --nocapture \
  > ~/ezsdr-bench/<test>.log 2>&1
```

装置の確認は `docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.40.36` で行う．

- 長い出力はファイルに書いてから読む．
- リンクが上がった直後の `uhd_find_devices` は空振りすることがある．
- 実機で失敗したら，その場でコードを直さない．記録する → FakeDevice で再現するテストを書く → 修正する，の順にする．

## 7. リポジトリと記録の索引

- branch と remote は AGENTS.md §1．`origin` = `k3komatsu/Ez-SDRv4`（`main` に push）．v3 は `k3komatsu/Ez-SDR`．**`git push --tags`/`--all` をしない**．
- ローカル：`~/work/Ez-SDRv4`（`main`），`v3/`（`master` の git worktree，消さない．設計文書が `v3/...` を22件引用），`/Users/komatsu/orca/workspaces/Ez-SDRv4/gh-pages`（`gh-pages` の worktree）．`v3/` を消したら `git worktree prune && git worktree add v3 master`，リンクが切れたら `git worktree repair v3`．引用の確認：

  ```bash
  grep -rhoE 'v3/[A-Za-z0-9_./-]+' design plan Ez-SDR_v4_ARCHITECTURE_VISION.md | sort -u | while read p; do test -e "$p" || echo "MISSING $p"; done
  ```

- 記録の置き場：
  - 各 Phase：`plan/phase1/` 〜 `plan/phase7/` の `00-overview.md`（決定ログと governance）．
  - 保守：`plan/maintenance/20-*.md` 〜 `31-*.md`（spec 20〜22 の修正，設計メモ 23・25，凍結前監査 24，spec 26・27，前向き監査 28，項目3 の 29，#60 の 30，項目8 の 31）．失敗機構の一覧は [failure-mechanisms.md](plan/maintenance/failure-mechanisms.md)．
  - UHD spike（branch `spike/uhd`，merge しない）：[plan/spikes/2026-09-26-uhd.md](plan/spikes/2026-09-26-uhd.md)．
  - 実装役の報告・レビュー・gate のログ：`~/.cache/ezsdr-fixes/<日付>-<作業>/`（リポジトリ外，消えうる）．
  - Phase 7 クローズ（2026-10-01）から spec 20 までの保守と監査（#1〜#38）：[audits-2026-10.md](plan/maintenance/audits-2026-10.md)．
  - 2026-10-14 までの handoff の全文は git の履歴（`git show c5bb58e:plan/handoff-archive.md`）．そこから移した記録は，上の各ファイル（と Phase 7 の bench 手順 `plan/phase7/bench.md`）の末尾の節「handoff からの記録（2026-10-14 に移動）」にある（`grep -rl "handoff からの記録" plan`）．
