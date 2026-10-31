# Ez-SDR v4 — Handoff (2026-10-31)

次のセッション（人間・AI どちらでも）が最初に読む現状メモ．**今のことだけ**を書く：どこに何があるか，何が未解決か，owner の判断待ち，次に何をするか．終わった作業の記録はその置き場（`plan/phaseN/00-overview.md` か各計画ファイル）に書き，ここには1行の参照だけ残す（AGENTS.md §9．`rules.py` が 300 行・40 000 byte を超えると落ちる）．2026-10-14 までここにあった記録も同じ日に各置き場へ移した．

開発の恒常的なルールは [AGENTS.md](AGENTS.md)．

## 1. 今の状態

- **Phase 1〜7 は Gate X 受理済み**．Phase 7（UHD Provider `ezsdr.radio.uhd`）は 2026-10-01 に X300＋OBX で bench B0–B9 を通してクローズ（[plan/phase7/00-overview.md](plan/phase7/00-overview.md) §11，[bench-results.md](plan/phase7/bench-results.md)）．accepted specs の一覧は AGENTS.md §1・§2．
- **凍結前の改修（項目1〜8）は完了**（[plan/maintenance/24-prefreeze-audit.md](plan/maintenance/24-prefreeze-audit.md)）．最後の項目8（spec 01〜06 を DA-1 形式に）は 2026-10-14 に完了（[計画31](plan/maintenance/31-item8-kernel-specs.md) §10）．その前の保守の記録は各計画ファイル（Phase 7 クローズから spec 20 までは [audits-2026-10.md](plan/maintenance/audits-2026-10.md)）．
- **spec 22 の DA-3 手順2（spec 07・09・18 を DA-1 形式に）は 2026-10-15 に完了**（spec 22 の Status 行）．残る手順3・5 と文書監査の残りは §3．
- **Phase 8（Mock ↔ X310 parity）は Gate P 受理**（owner 2026-10-17）：計画 [plan/phase8/00-overview.md](plan/phase8/00-overview.md)（判断 P1〜P6，段階，bench BP0〜BP5，出口条件），設計 [33-channels-and-streams.md](plan/phase8/33-channels-and-streams.md)（channel と stream，混在 board，`x3xx`，Kernel の変更は `KeyDecl` の番号付き segment だけ）．レビューと調査は `plan/phase8/reviews/`・`research/`．
- **Phase 8 の段階 1（channel と stream，1a〜1b-7b），設計メモ 35〜43，段階 3（測定と実機の準備：3a〜3g，D9a，MA1，43a，TXP，3c-3b，TXS）は完了**（2026-10-17〜29）．各段階の記録は [計画](plan/phase8/00-overview.md) §8 の判断ログ，レビューは `plan/phase8/reviews/`，実機の結果は [bench-results.md](plan/phase8/bench-results.md)．
- 実装：13 crate（`crates/`），schema（`schemas/`），Python package（`python/ezsdr`）．確認の手順は §5．
- open issue は6件：#56・#67（§3），#70・#72（D9b-1a の実機再確認後に閉じる），#71・#75（§2 の (6)）．

## 2. 次にすること

1. **D9b-1a・D9b-2・L1・L2 は完了**（D9b-1a と D9b-2 は 2026-10-30，L1・L2 は 2026-10-31．[00-overview](plan/phase8/00-overview.md) の各行，設計メモ [39](plan/phase8/39-dsp-timed-commands.md) §6.6〜§6.8，[44](plan/phase8/44-device-chain-state.md) §3・§4）．実機では未確認．L1：時刻つきの受信の周波数は，覆う segment を streamer が流すまで uhd-control が持ち続け（§3.1），受信の装置の状態は stream ごとの `RxChain` の遷移だけで変わる．L2：送信の device burst・front・recovery は streamer ごとの `TxChain` の遷移だけで変わる（動作は変えない）．記録 [reviews/l1-sol.md](plan/phase8/reviews/l1-sol.md)，[reviews/l2-sol.md](plan/phase8/reviews/l2-sol.md)．**次の順番**（owner 2026-10-31）：(1)(2) mutation 行の字句での照合と rustfmt（`max_width = 200`）は済（2026-10-31，AGENTS §9，`.git-blame-ignore-revs`）→ (3) 提案 A は済（2026-10-31：gate を 1 コマンドに〈`gate.py`〉，段階の最小の記録をリポジトリへ，指示の出所を 1 つに〈AGENTS 冒頭の Safety core，実装役は §5 だけ〉と `opencode.json` の禁止，major には反例．記録 [reviews/a-gate-sol.md](plan/phase8/reviews/a-gate-sol.md)）→ (4) **実機**（owner に att の確認．#1 OBX 192.168.40.36 と #2 UBX 192.168.47.37 で D9b-1a の P5〜P7・D70・TX200，D9b-2 の P4，L1 の受け渡しと L2 の送信を production のコードで．送信中に ssh を切って送信が止まるか．#70・#72 は確かめてから閉じる）→ (5) **設計メモ [45](plan/phase8/45-uhd-decision-core.md) の実験**（純粋な判断の核 `uhd-brain` と実行役．受信の範囲で，判定の基準は §7 に固定．他の作業と並行しない，owner）→ (6) 結果で分岐：成功なら送信と M2 を 45 に足し，D9b-1b（送信の R1〈#71〉と R2′，underflow 後の (a)）と #75（確認のない T3 でも recovery が終わる，`mech:failure-collapsed-into-value` の 7 件目）をその中で解く．失敗なら #75 の設計メモ，次に今のコードで D9b-1b → (7) 移行を 8b の前に終える．
2. **その後**：TXP の B3（コマンドを用意して実機），段階 4（profile の値：queue の深さ 7／8，throughput，受信の start lead．送信 1 channel 200 MS/s は TXS で 300 s 通った，`tx_in_flight_ns` 10 ms は INFERRED のまま），H3 は段階 4．**owner に（最初の bench の後）**：`overflow_restart_gap_ns` は `unmeasured` のままで，読み直しから再開までの推定を値とするか（基準は設計メモ 41 §8 の末尾）．**owner に（段階 4 の前後）**：今の UHD Module は流れていない chain にも時刻つきの周波数を渡しうり，DDC/DUC に残った設定が後の構成を上書きする（D9 で確認．設計メモ 39 §3 の推奨は `src/` の仕事，D9b と重なる所は D9b で）．**Phase 8 の終わり（8b の前）にコードとテストの組み立ての振り返り**（owner 2026-10-31．段階 1 の後の crate ごとの冗長の棚卸し〈owner 2026-10-19〉を広げたもの）：UHD の provider はテストが本体の約 5 倍（本体約 6 100 行，内のテスト約 18 700 行，`tests/` 約 11 800 行），手書きの mutation 行は約 2 000 行（Rust の字句で照合）．純粋な遷移のテストと操作の列の生成は設計メモ 45 の実験が答えるので，ここに残るのは mutant の自動生成（cargo-mutants など）と手書きの行の役割分担，テストの準備の共通化．まず過去の段階の増え方（変えた本体 1 行あたりのテスト行，振り直した test と row の数）を測り，設計メモで比べて owner に諮る．道具の選び方は v4.0 の凍結の前に決める．各段階のレビューで見つかった既存の保守の負債（AGENTS.md §9 Maintainability）はここへ集める．
   - **持ち越し**：[1b-4a の修正の報告](plan/phase8/reviews/1b4a-fix-reports.md) §9，[1b4e2-opus](plan/phase8/reviews/1b4e2-opus.md) §1（待ちが origin を越える場合，close の時点で読めない TX の report，`effective != e` の到達性），4d-3 の 2 件（`rx_segment` の timing 行の `channels` は数のまま，plan の e2e は Manifest ファイルまで通さない）．
   - **進め方**：AGENTS.md §9．実装役とセルフレビューは §5．1 段階は本体とテストで約 1,500 行，テストの繰り返しは helper に，テスト計画は順序ごとに assert 1〜2 個．セルフレビューは 1 回，checklist 4 項目，`timeout 2400`，mutation を流さないと明記．指示書には「orchestrator のファイルは読むだけ，到達できる系列のテストを代わりなしで消さない，消した名前を `new` に持つ行と消したテストを `filter` に持つ行を grep する，実時間に頼るテストを足さない」を入れる．`mutation_watchdog.py` は削除の mutant と `new` がよくある語の行で誤報を出す．
3. **実機**（2026-10-29 の構成，[bench-results.md](plan/phase8/bench-results.md)）：#1 X300＋OBX（192.168.40.36，enp1s0f1np1，att 40 dB），#2 X300＋UBX（slot A）・CBX（slot B）（192.168.47.37，enp1s0f0np0，att 各 30 dB），#1 → #2 に PPS と 10 MHz（UHD の `test_pps_input` は成功．Module の外部 ref の道〈UR-7〉は未試験で 8b）．**送信の前に毎回 owner に att を確かめる**．CBX の 1.2〜6 GHz は raw probe の `SAFE_TX_HZ`（999.5〜1000.5 MHz）の外．残り：混在構成（BP4b），XG（BP4c），8b の 2 台の時刻差を測る RF 配線（1 つの信号を分配して両方の RX へ，減衰器付き）．8c は FPGA image に Replay・DDC/DUC（・FFT）の block があるかを `uhd_usrp_probe` で確かめる．
4. **その後の順番**（owner 2026-10-10，Vision §67，[28-forward-audit.md](plan/maintenance/28-forward-audit.md) 判断7）：8 → 8b（複数台：1 instance・1 root，判断1．`Session.aligned(at)`・`next_pps(k)`・`EpochRef::Pps` は実装済み，U12 は 8b）→ 8c（RFNoC：Replay，DDC/DUC．device FFT は FPGA image に block があるときだけ．UHD の RFNoC API は C++ のみなので UHD Module に薄い C++ 層が要る）→ **Radio Model と UHD Module の汎用性の点検**（owner 2026-10-24．設計メモ：A 別の SDR 系〈HackRF One・Pro，bladeRF 2.0．v3 の `v3/source/device/hackrf.d` が証拠〉，B X4xx・N3xx〈UHD 4.10 のソースを読むだけ〉．その後 HackRF One → bladeRF 2.0 の Module．Phase 8 の後・凍結前．紙の作業は段階 2 の合間に先にしてよい）→ 10（Processor/Reactor の実行エンジン，最小の PDU 経路）→ 全規則の確認 → 凍結レビュー → **v4.0 凍結** → 11+（Packet/PDU + TUN/TAP＝旧 Phase 9，WASM，GPU…）．

## 3. 未解決と owner の判断待ち（一覧はここだけ）

| 項目 | 中身 | いつ・どこで |
|---|---|---|
| 汎用性の点検の続き | 判断1〜4は決定（owner 2026-10-25）：1 instance ごとの宣言と新しい update class `untimed`，2 HackRF を受け入れ，時刻を持たない装置の模型は設計メモで（故障の時刻は厳密でなくてよい），3 bladeRF の Run の中の rate の変更は許し，変更の後の時刻は同じメモの模型で保証の外，4 UC-6 の command の格子を広げる．どれも Kernel に触れ，凍結前に amendment が要る | 凍結前（Phase 10 の前後，順番は owner と決める）．[33-radio-generality.md](plan/maintenance/33-radio-generality.md) §6 |
| #67 | 遠い origin の timed start が queue されたまま `Provider::stop` が来ると uhd-rx が join の 1 s を越えて detach され，streamer が漏れる（HEAD から） | #56 の bench の後 |
| #56 | X310 で start の e₁〜e₂ 間の Stop と Stop の処理遅れを測る | Phase 8 の bench |
| H1〜H5 | H1：開始が遅れた複数 packet の burst．H2：burst 途中の Underflow（再開時刻，ずれ）．H3：TX の `max_num_samps`．H4：負荷下の uhd-tx・uhd-control の起床遅延．H5：保持した最後のサンプルの 2 ms 期限 | Phase 8 の bench |
| 方向別の start lead | owner 決定（2026-10-08）：`radio.timing.start_lead_ns` を `rx_start_lead_ns` と `tx_start_lead_ns` に分けた（段階1a で実装，両方 50 ms）．値は Phase 8 の bench で測る | Phase 8 の bench |
| 測って決め直す値 | 送信と受信の start lead（今は 50 ms，INFERRED），cold change・再開で受信が途切れる時間（約55 ms〜221 ms），UR-17 の受信要求長の上限（残すと決定済み，実測で見直す） | Phase 8 の bench |
| 保守表の 3・4 | 3：fake は TimeError で burst 全体を捨てるが FPGA の既定は packet 単位（H1 の後に UR-33 と fake を直すか）．4：保持した最後のサンプルの 2 ms 期限（設計の代償として受け入れるか見直すか）．表は [audits-2026-10.md](plan/maintenance/audits-2026-10.md) の「Phase 7 後の保守」（1・2・5 は解決済み） | Phase 8（owner） |
| spec 22 の残り | 手順3（DA-5 (b)：spec 11 を 08 へ統合）と手順5（spec 10・14・16）．受理済み（2026-10-07）で未実施，Phase 8 の後（手順3は取りやめも owner に確認）．手順2（spec 07・09・18 を DA-1 形式に．文書監査の D4・D5・D3 の spec 側・D11 を含む）は 2026-10-14〜15 に済み，記録は spec 22 の Status 行と各 spec の Changes 行 | 手順3の取りやめ（owner） |
| 文書監査の残り（2026-10-14） | D2（RX_OVERFLOW の `restart_gap_ns`・`lost`・件数が Mock と UHD で違う，#51 と同じ機構）・D10（値ごとの「緩い向き」）は Phase 8 の計画で（D12〜D14 は 2026-10-16 に済み：`rules.py` が OV/PO の ID を守る，AGENTS.md §2 に OV/PO の扱い，凍結チェックリストに PO，AGENTS.md と handoff の重複を削った） | Phase 8（D2 は段階2，D10 は P2 の比較テスト．D3 は段階1a で済み） |
| テストのない条件（2026-10-16） | Test 行の網羅調査（[計画32](plan/maintenance/32-test-line-sweep.md)）が `none named` に書いた 130 条件を仕分けた．A1（30 件，Kernel・Sink など）はテストを足し済み（2026-10-16，T01〜T33）．A2（14 件，RM/MR/UR）は 2026-10-25 に済み（T34〜T45，[reviews/a2-opus](plan/phase8/reviews/a2-opus.md)）．B（4 件，実機が要る）は Phase 8 の段階 4．C（82 件）は `none named` のまま | Phase 8 段階4（B） |
| Test 行の sweep の 2 回目（2026-10-25） | 済み：計画32 の `tools/sweep/` で全 spec を確かめ，Sonnet が全件を確かめてから Test 行を直し，Sol がレビュー（[計画32](plan/maintenance/32-test-line-sweep.md) の Second sweep）．spec の本文とコードの食い違い 6 件は owner が決めた（2026-10-25）：MR-7・SE-6・KC-45・OV-21・UR-34 は本文をコードに，SB-34 はコードを spec に合わせた（`needs` も空きの節点を先に取り，排他の節点が埋まっていれば `NodeAlreadyBound`） | なし |
| Provider の id を名乗る写し | 受け取り時の拒否で id を名乗る処理が MockRadio・UHD に約15か所ずつ（MR-18・UR-26 の一部は未固定）．1か所で id を渡す形にまとめると減る | 任意 |
| 規則ごとの確認への記録 | TM-21 の budget/block period 比較は未実装．TM-16c の既存テストは ManualTimeAuthority で paced Authority の規則を固定しない．SC-29 の SOB なし time jump の `requested_target` 不在と SOB 付き time jump の組合せは未固定．Authority の `now/schedule/cancel` の panic の扱いは規則との対応を要確認 | 凍結前の全規則の確認（§4） |
| feature `uhd` の mutation | `uhd.rs` の規則は mutation の gate の外にある．対象は handle の寿命，streamer の map，空の列の channel 0，`serial`・`rates`・antenna の read-back．mutation の image（`plan/maintenance/tools/remote/Dockerfile`）に libuhd と header がなく，行に feature を渡せないため．今は docker の `--features uhd --lib`・`--test uhd_api` だけで確かめている．image と行に feature を足すか（1b-4a の Sol の再レビュー 2） | Phase 8 の中（1b-4c までに判断） |
| 負荷に弱いテスト | maintenance/J293 の無変異テスト `ur_26_the_shutdown_s_last_burst_ack_waits_on_the_device_s_clock` が16 shard の負荷下で1回失敗（2026-10-14）．2026-10-17 に手元の `cargo test --workspace` で4回再発（1b-2a で1回，1b-2b の修正中に3回．単独では毎回通る．どの差分も UHD の挙動を変えていない）．原因（2026-10-17）：テストの thread が `now` を読んでから送るまでに負荷で約 18 ms を超えて遅れ，`send`（DropAndFlag）が burst を遅れとして落としていた（plan 30 §5(c)）．テストの verb を `start_repeat`（SendAsapAndFlag）に替えた（J293 は kill のまま）．同じ形の遅れは `ur_21_a_descheduled_timed_send_…` の manual の分岐にもある（再発したら同じ直し）．この類が続くなら plan 30 の段階 B は owner の判断．同じく phase7/U17（`tx.rs` の repeat の終わり）が 2026-10-14 の gate で負荷下に1回生き残り，単独の再実行で kill（変異を見逃すテストは本当の生き残りを隠しうる）．Python の `test_v54_sleep_is_run_time` の 10 s の上限は 2026-10-24 に時刻の差と §58 の 2 の 2 本に分けて解消（設計メモ 37） | 再発時 |
| リモートからの利用 | owner 決定（2026-10-30）：ネットワークの listener は作らず，ssh で server の標準入出力を運ぶ（`EZSDR_SERVER` を `ssh -T <host> ezsdr-server` の wrapper に向ける．path は server 側のもの）．Phase 6 S9・Phase 7 §11 の「listener・server 所有の profile・認証は後の phase」を置き換える．残り：送信中に ssh が切れたとき送信が止まるかの確認，ファイルを作らない capture（#73，既定をメモリに傾く）と host の `sc16`（#74） | 切断の確認は次の bench．#73・#74 は Phase 8 の後・8b の前（owner 2026-10-30） |
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
- [ ] 将来の設定エディタ（GUI，[34-future-config-editor.md](plan/maintenance/34-future-config-editor.md)）のために Kernel の型を変える必要がないことを確かめる．

## 5. 進め方と道具

- **分担と方針**は AGENTS.md §9（分担・順番）と §6（owner の方針）．
- **実装役**（AGENTS §9 が名前を書くのはここだけ）：Haiku 5.5 `anthropic/claude-haiku-5-5#high`（OpenCode2，owner 2026-10-31．`#max` は割に合わない）．セルフレビューは free の DeepSeek `opencode/deepseek-v4.1-flash-free#max`，free 枠が切れたら Go の `opencode-go/deepseek-v4.1-flash#max`．独立レビューは Sol `openai/gpt-6.1-sol --agent plan`．Sol の残り枠は `python3 plan/maintenance/tools/codex_limits.py`．
- **実装役への指示書に毎回書くこと**：mutation は `remote_mutate.py` だけ（手元の `mutate.py` も作業ツリーへの mutant も禁止，実験はリポジトリ外のコピーで）．報告の前に `cargo fmt --all`（rustfmt.toml，2026-10-31 から）．手元（usrp-lnx03）で負荷をかけない．セルフレビューも gate や遠隔の mutation もフォアグラウンドで終わるまで待つ（裏に回したまま報告を終えると実行ごと止まる．2026-10-31 に Luna と Haiku で2回）．DeepSeek のツールは1コマンド120秒で打ち切るので，長い処理（`rules.py --sentences` 約5分など）は時間を延ばすよう書く．前の段階で出た誤りの型は禁止として列挙する（8g で効いた）．
- **gate**（段階の最後）：`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/gate.py --out ~/.cache/ezsdr-fixes/<日付>-<作業>/gate --mutation all [--base <変更前の commit>]`（手元約 4 分＋遠隔約 20 分．1 shard だけが長く残る）．判定は `gate.json`（source の同一性，道具の版，各段の数，mutation の全行の勘定）．既知の生き残りは `plan/maintenance/tools/known-survivors.txt`（今は phase7/G09）．fix round は `--only` と `--mutation <lists> --ids <IDs>`．phase7 G16 と phase6 G21 は timeout で kill が正常．shard × 4 CPU は1ホスト32コア以内．`rules.py --self-test` も gate に入っている．docker の Cargo target は checkout ごとの volume（`ezsdr-v4-cargo-target-<path の sha256 の先頭12桁>`）なので，worktree を消すときは `docker volume rm` も．
- **見張りと機械チェック**（モデルなし）：
  - `plan/maintenance/tools/mutation_watchdog.py`：Mac の `mutate.py`・作業ツリーに当たった mutant・親のいない／20分を超える test を見つけたら終了して知らせる．実装役を動かす間バックグラウンドで流し，gate を閉じたら止める．判定は実行中のスクリプト名で（文字列一致は誤検出した）．
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
  - 段階の最小の記録（指示書，テスト計画，指摘の扱い，`gate.json`）：`plan/phaseN/reviews/<段階>-*`（AGENTS §9，2026-10-31 から）．生のログと実装役の記録は `~/.cache/ezsdr-fixes/<日付>-<作業>/`（リポジトリ外，消えうる）．
  - Phase 7 クローズ（2026-10-01）から spec 20 までの保守と監査（#1〜#38）：[audits-2026-10.md](plan/maintenance/audits-2026-10.md)．
  - 2026-10-14 までの handoff の全文は git の履歴（`git show c5bb58e:plan/handoff-archive.md`）．そこから移した記録は，上の各ファイル（と Phase 7 の bench 手順 `plan/phase7/bench.md`）の末尾の節「handoff からの記録（2026-10-14 に移動）」にある（`grep -rl "handoff からの記録" plan`）．
