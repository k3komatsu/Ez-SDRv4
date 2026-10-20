# 1b-4c：修正の回 1〜4 の指示書と報告

回 1・2 は OpenCode の Haiku 5.5 #max（Go の上限の間），回 3・4 は Claude Code の Haiku 5.5 のサブエージェント（セルフレビューは DeepSeek #max）．

## 回 1：指示書


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `4199e51` に 1b-4c の未コミットの変更．あなたが実装したもの）．

### 最初に読むもの

- `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/impl-brief.md`：「守ること」は同じ．
- `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/opus.md`：orchestrator のレビュー．
- `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/sol-review.md`：Sol のコードレビュー．
- 前回の報告 `report.md`．

**注意**：`plan/*/tools/mutations.json` は orchestrator が HEAD の書式（indent 1．maintenance の後半は indent 2）に戻した．行を足したり変えたりするときは，ファイル全体を書き直さない．変える行だけを周りの書式で書く（例えば Python で HEAD の文をそのまま残し，該当の行だけ差し替える）．

### 裁定：次をすべて直す

1. **O1（major．lock）**：`book_cold` と `start_rx` で，instant と seq を取ってから記帳するまで，`streams` の lock を 1 つの critical section で持つ．HEAD と同じで，両方のモードに当てはまる．
   - lock を離してよいのは，from-none の device 呼び出しだけ（owner が idle．HEAD の理由のまま）．
   - layout の `ColdConfig` は lock を取らずに作れる constructor にする．投影した held の更新を最初から渡す．
   - セルフレビューの P2-3（順序 `updates` → `streams`）への対応は，「新しく作った，まだ共有していない `ColdConfig`」には当たらない．そのことを code のコメントに書く．
   - テストを 1 本足す．記帳が instant と同じ critical section で行われることを固定する．spy か unit で．
   - P271 の当て直しを見直す．mutation の行は「lock を離して取り直す」形にする．
2. **Sol 1（major）：TX の report の channel**．`TxReport.channel` は streamer の中の位置で，device の channel 番号ではない（UHD 4.10 `rfnoc_tx_streamer.cpp:152-159,312-319`，FakeDevice `device.rs:1515-1516,1555-1556`）．
   - `unacked` などの pending の report は，`0..channel_count` の位置で持つ．
   - テスト：stream 0 が `{1}` で送る．ACK で pending が退くこと，`TIME_ERROR` の target が正しいこと．
   - mutation の行を足す．
3. **Sol 2（major）：from-none の拒否**．open・rate の照合・configure のどれかが失敗したら，値を保持した refused item として timeline に記帳し，stream を halt させる．owner が拒否したときと同じにする（RM-25）．
   - テスト：全受信 channel が stream に入っていない状態から始める．
     1. antenna の read-back が失敗する channel を join させる．
     2. 拒否と halt を確かめる．
     3. configuration of record が新しい値になっていることを確かめる．
     4. timed の更新を送る．
     5. `start_rx` か，成功する cold 変更で再開する．
   - mutation の行を足す．
4. **Sol 3（major）：equivalent を取り消す**
   - P344・P345 の `equivalent` を消す．
     - `StreamState.idle` の初期値は `false` なので，owner の最初の reopen が方向の最初の open になりうる．
     - owner の最初の poll より先に control が join を取る順序を，テストで作って殺す（manual clock か wrapper で制御する．sleep で競合を作らない）．
   - R26 は，要素が空の方向を持つ既存のテストに当て直して殺す．`equivalent` の欄は消す．
   - P313 の `equivalent` の理由は直す（後の stream に line が無いので差が観測されない）．
5. **O2（minor）：死んだ検査**：spec を設定した後の `have < need` の検査（layout の腕と今の key のモードの両方）を消す．
   - R23 を削除する．
   - UR-5 の該当する文も直す．
6. **Sol 4（minor）：layout のモードの LATE_COMMAND**：床で上がった後に拒否される cold 変更は，MockRadio と同じ順序で報告する（LATE_COMMAND の後に拒否）．
   - 今の key のモードは HEAD のまま．
   - parity にその列を足す．
7. **Sol 5（minor）：parity を完成させる**
   - 床の列：送信の future cold の後に，それより早い受信の cold を送る．instance 全体の床の instant，`LATE_COMMAND`，clock の `ended_at` を比べる．
   - 論理の更新 `(key, value, effective instant)` を取り出して比べる処理．取り出し方は doc comment に書く．
   - M1 のテストに，受信の block が channel 1 の内容であることの観測を足す（FakeDevice の channel ごとの loopback）．
8. **Sol 6（minor）：Manifest の key**
   - rate の行は stream の key（`radio.rx.s0.sample_rate_hz`）にする．周波数・利得・antenna は channel の key にする．
   - テスト：記録する key がすべて，その instance の宣言にあること．
9. **Sol 7（minor）：spec 18**
   - 偽の文を直す：UR-12 の「ten keys」，UR-23 の「channel count changed」，UR-28 の source，UR-30 の `rates_checked`（arm だけ，と書いている），UR-33 の `"FAKE0001"`．
   - 規範の文から「this stage」などの段階の言い方を除く（DA-1）．
   - 方向ごとの stream 0 だけを動かす規則は，規則として書く．

### 確かめること

impl-brief と同じ 1〜4．4 は次の行だけを流す：この回で足した行，変えた行，コードかテストが変わった行（AGENTS.md §7）．COMPILE ERROR も生き残りとして扱う．

### セルフレビュー

impl-brief と同じ（Haiku 5.5 #high，P1 が無くなるか 3 回まで）．チェックリストに次を足す．
- 上の 9 項目がすべて直っているか．
- `equivalent` が残っている行の理由が正しいか．

### 報告

`~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/fix-report.md` に書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．中身：
- 9 項目それぞれの修正
- テストと行
- 確かめることの結果
- セルフレビュー

## 回 1：報告

STATUS: done

## Phase 8 段階 1b-4c：修正の回 1 の報告

前回の run（`fix2.log`）は，最後の 309 行の mutation を背景で流したまま turn を終え，その run は流れていない（§3）．この報告は，その後の記録（`/tmp/opencode/` の remote-fix-*.log，selfrev-*.log，ws-8.log，clippy-9.log と fix2.log の transcript）と，木の読み直しに基づく．コードとテストは変えていない．mutation は流していない．cargo も流していない（この報告で再実行したのは rules.py だけ）．

### 0. 要約

- 9 項目すべてに手を入れた．直ったのは 1（O1），2（Sol 1），5（O2），6（Sol 4），8（Sol 6）．一部だけ直したのは 3（Sol 2），4（Sol 3），7（Sol 5），9（Sol 7）．
- 自己レビューは上限の 3 回で止めた．3 回目の後も P1 が 2 件残った．1 件（start_rx の lock）は直したが再レビューしていない．1 件（Sol 3 の順序テスト）は退けた．
- 依頼どおりに行かなかった，または未解決のもの（orchestrator の判断が要る）：
  1. Sol 3 の順序テストを書いていない．依頼の前提（`StreamState.idle` の初期値 `false` が観測される）は，このコードでは成り立たない（§5）．P344・P345 は殺せるようになったが，殺すのは「前の照合の結果を再適用する」経路だけで，owner が方向の唯一の照合になる経路は残った．
  2. 送信の from-none 拒否は owner の拒否と同じでない．clock を入れる（tx.rs:679–684）．Sol 2 の「owner と同じにする」に合わない（§1-3，§7-2）．
  3. parity の `note` 行は比べていない（round 1 P2-5．直していない）．
  4. P271 の新しい形は流していない．P313 の注記は行番号がずれている（§6）．
  5. 最後の木に対して全 list が流れていない．gate で全 list を流すこと（§3）．

### 1. 9 項目の修正

#### 1. O1（lock）：直した

- `book_cold`：instant（control.rs:686）から記帳（:831）まで `streams` の lock を一度に持つ（lock は :675）．device を呼ぶのは from-none の区間だけで，そこでは離して（:797）取り直す（:824）．
- current keys の configuration は lock の前に作る（:667–670）．layout の `ColdConfig` は lock の中で作る（:785）．その constructor は `layout_with_clock`（:90）で，lock を取らず，held の更新を最初から受け取る．
- `start_rx`：instant・seq・記帳を一つの lock にした（:406–415．configuration は :403，:410–413）．
- ブリーフが求めたコメント（新しく作って，まだ共有していない `ColdConfig` には `updates`→`streams` の順序の対応が要らない）は control.rs:671–674．
- テストと行は §2．

#### 2. Sol 1（TX report の channel）：直した

- pending の report は streamer の位置（`0..count`）で持つ（tx.rs:214–221）．`reported_target` が位置で引く（:225–232）．ACK（:741–742）と TIME_ERROR（:756–757）もそう．
- テスト `ur_28_a_report_is_correlated_by_its_position_in_the_streamer`（tx.rs:1520）．行 P350（killed）．

#### 3. Sol 2（from-none の拒否）：control 側は直した．送信側は未解決

- control.rs:793–840：from-none の open・rate 照合・configure のどれかが失敗すると，その値を持った refused の項目として記帳する（`refused: failure.is_some()`，:831）．segment は `configured = true` で置く（:826）．記帳の後に `device_failed` を出す（:838–840）．stream は halt する（RM-25）．
- テスト `ur_25_a_refused_enable_from_no_stream_halts_and_keeps_its_values`（fake.rs:6201）：拒否，記帳，設定の記録，timed 更新，再開の 5 段．行 P351（失敗を `device_failed` だけにする）と P354（refused を false にする）．どちらも killed．
- 間接的な点：halt は「no channel」の出力で見ている（fake.rs:6252–6257）．configuration of record は restart で適用される値から見ている（:6245–6246）．refused の antenna の claim は観測していない．
- **未解決**：送信側．from-none で拒否された送信区間は `configured = true` で plan に入り，`do_switch` は open も configure も飛ばして clock を入れる（tx.rs:669–673，:679–684）．owner が拒否したときは clock を入れない（tx.rs:686–693）．ブリーフの「owner が拒否したときと同じにする」に合わない．観測の差（送信に出るか）は未確認．§4 の回 3 P2-1 も参照．

#### 4. Sol 3（equivalent の取り消し）：equivalent は消した．順序テストは退けた

- P344・P345 の `equivalent` を消した（`plan/maintenance/tools/mutations.json` の 7235–7253 行）．殺すテスト：P344 は `ur_13_the_reopen_enforces_the_direction_s_rate_comparison`（fake.rs:5953），P345 は `ur_13_a_transmit_switch_is_compared_with_the_stored_outcome`（tx.rs:849）．
- R26：対象を既存の `ur_05_a_direction_without_elements_sets_no_spec`（fake.rs:4176）に当て直し，`equivalent` を消した．名前は殺し方（空の方向で要素 0 に落ちて panic する）に合わせた．
- P313：`equivalent` の理由を書き直した（§6）．
- 順序テストは書いていない（§5）．

#### 5. O2（死んだ検査）：直した

- `have < need` は `crates/` に残っていない（grep 0 件）．R23（phase7）を削除した．UR-5 の文からも消えている（round 3 の読みで確認）．

#### 6. Sol 4（layout の LATE_COMMAND の順序）：直した

- layout の cold 変更は，床で上がった後に拒否されるとき，理由より前に LATE_COMMAND を出す（control.rs:688–693）．current keys は HEAD のまま（:749–751）．
- テスト `vh_8_a_floored_refusal_is_reported_in_the_same_order`（parity.rs:772）．行 P352（LATE を落とす）．killed．

#### 7. Sol 5（parity の完成）：直した．cut は許容差（逸脱）．`note` の穴は残る

- 床の列（送信の future cold の後に，それより早い受信の cold を送る）：テスト `vh_8_the_same_floor_after_a_transmit_change`（parity.rs:706）．床の instant，LATE_COMMAND，clock の cut を比べる．この列には行が無い（ブリーフは Sol 5 に行を求めていない）．
- 論理の更新 `(key, value, effective instant)` を取り出す `updates`（parity.rs:685–703）．取り出し方は doc comment（:677–684）に書いた．
- **逸脱**：clock の cut は `ended_at` の完全一致ではなく，200 tick（1 サンプル）以内で比べる（parity.rs:765–768）．理由は :761–764 の注：両 Provider の格子が一致しない（実測の差は 28 と 159 tick）．
- M1（`ur_25_a_same_size_membership_swap_reopens_the_streamer`，fake.rs:5284）に channel 1 の loopback の観測を足した．行 P317・P318 は impl のもの．
- **穴（round 1 P2-5．直していない）**：`updates` は UHD が書く `note` 行（`note == "no channel"`）を両側から除く．Mock の crate は `"note"` を書かない（0 件）．そのため，UHD だけが書く `note` の差は比べられない．doc comment には除く理由を書いたが，穴は残る．

#### 8. Sol 6（Manifest の key）：直した

- `applied_key`：rate は stream の key（`keys::stream`，`STREAM_LEAF` は `sample_rate_hz`），他は channel の key（mode.rs:245–251）．
- テスト `ur_30_every_applied_key_is_declared_by_the_instance`（fake.rs:5786）．行 P353（killed）．

#### 9. Sol 7（spec 18 の偽の文）：大部分直した．段階の言い方が残る

- 直した（grep と読み）：UR-23 の「channel count changed」（design/ に 0 件）．UR-33 の `"FAKE0001"`（design/ に 0 件）．UR-28・UR-30 の文（round 3 の読み）．stream 0 だけを動かす規則は規則として書いた（design/18:194）．
- 変更記録の文：design/18:595 は「at most once, in any order」に直した．design/09 の「MR-38 states that both Providers read it」は消えた（0 件）．design/07 の rate-set の文（`rate_set_matches` を両 Provider が読むと誤って書いていた）は消えた．
- control.rs:80–83 の doc：`updates` は投影ではなく，caller が作った copy だと直した．
- **残り**：design/18 の 241，313，314，319 行に「Phase 8 measures」（段階の言い方）が残る．直していない．58 行は直した．

### 2. テストと行

#### テスト：足した（この回．HEAD に無く，impl の報告にも無いもの）

| テスト | 場所 | 目的 | 行 |
|---|---|---|---|
| `ur_25_the_cold_booking_books_in_the_section_that_takes_its_instant` | control.rs:1676（lib） | LockSpy で，LATE と REJECT の経路の記帳が instant と同じ section にあることを見る | P349 |
| `ur_25_the_layout_cold_change_is_built_in_its_booking_section` | control.rs:1722（lib） | 受理される layout の変更で，`ColdConfig` を作るとき lock が持たれている | P355 |
| `ur_25_a_cold_change_takes_the_line_lock_once_for_its_instant_and_its_booking` | control.rs:1750（lib） | 受理される cold 変更は lock を 1 回，from-none の enable は 2 回取る | P356 |
| `ur_25_a_start_rx_takes_the_line_lock_once_for_its_instant_and_its_booking` | control.rs:1788（lib） | start_rx の instant と記帳が一つの lock | P357 |
| `ur_28_a_report_is_correlated_by_its_position_in_the_streamer` | tx.rs:1520（lib） | Sol 1：stream 0 が {1} で送る．ACK で pending が退く，TIME_ERROR の target が正しい | P350 |
| `ur_25_a_refused_enable_from_no_stream_halts_and_keeps_its_values` | fake.rs:6201 | Sol 2：5 段（§1-3） | P351，P354 |
| `ur_13_the_reopen_enforces_the_direction_s_rate_comparison` | fake.rs:5953 | Sol 3：前の拒否の後の owner の open が，その結果を再適用する | P344 |
| `ur_13_a_transmit_switch_is_compared_with_the_stored_outcome` | tx.rs:849（lib） | Sol 3：保存された拒否の後の transmit の switch が，その結果で拒否される | P345 |
| `ur_30_every_applied_key_is_declared_by_the_instance` | fake.rs:5786 | Sol 6 | P353 |
| `vh_8_a_floored_refusal_is_reported_in_the_same_order` | parity.rs:772 | Sol 4 | P352 |
| `vh_8_the_same_floor_after_a_transmit_change` | parity.rs:706 | Sol 5（行は無い） | — |

#### テスト：変えた・消した

- 変えた：`ur_25_a_same_size_membership_swap_reopens_the_streamer`（fake.rs:5284）に channel 1 の内容の観測を足した（Sol 5）．
- 変えた（impl の hunk の帰属）：in-src のテスト（control.rs の 8 件，tx.rs の 26 件，rx.rs の 3 件，mod.rs の 1 件）は，impl が入れた API の変更（例：`core.description.timing` を `core.mode.timing()` に）に合わせた機械的な書き換え．読んだ範囲では挙動の変化は無い（round 1 の読み）．この回の変更と impl の変更は hunk ごとには分けていない．
- 消した：なし．

#### 行（maintenance）

- 足した：P349–P357（9 行）．P349 は late 通知の周りで lock を離す．P350 は TX report の位置．P351 は失敗を refused にしない．P352 は LATE の省略．P353 は applied の key．P354 は refused を false にする．P355 は layout の `ColdConfig` を lock の外で作る．P356 は instant と config_at の間で離す．P357 は start_rx の instant と記帳の間で離す．
- 当て直した：P271．impl の形は `drop` を消して lock を持ち越すものだった．この回の形は `drop(streams)` の直後に取り直して device 呼び出しを lock の下にするもので，ブリーフ 1 の「lock を離して取り直す」に当たる．殺すテストは `ur_25_the_line_lock_is_free_inside_every_control_device_call`（control.rs:1656）．**この新しい形は流していない．** 旧形は remote-fix-5 で killed．
- 注記を書き直した：P313（§6）．
- `equivalent` を消した：P344，P345．

#### 行（phase7）

- 当て直した：R26（§1-4）．
- 削除：R23（§1-5）．

#### 残る `equivalent`（maintenance に 4 件）

- G03，J58，J156（HEAD から）：理由は再導出していない．J58 は round 1 の読みで成り立つ（`streams.end` と `lost.store` が lock の中にある）．G03 は未確認．J156 は kernel の行．
- P313（この回で書き直した）：成り立つと読んだ（§6）．phase7 の `equivalent` は 0 件．

### 3. 確かめたこと

| 確かめること | 結果 | 記録 |
|---|---|---|
| `cargo test --workspace --no-fail-fast` | 66 target すべて ok．**1394 passed，0 failed**，終了 0．最後の source の編集（control.rs 07:56:54）の後の 07:57 に build された | `/tmp/opencode/ws-8.log` |
| `cargo clippy --workspace --all-targets -- -D warnings` | 終了 0．警告なし（ezsdr-radio-uhd などを再検査） | `/tmp/opencode/clippy-9.log`（07:58） |
| `rules.py --base HEAD` | **0 problem**（この報告の直前に再実行） | `/tmp/opencode/rules-fixreport.log` |
| 変えた・足した行の `old` の一致 | P271，P313，P344，P345，P349–P357，R26 の 14 行すべて，`old` がファイルに各 1 回一致（python で確認） | — |
| R23 の削除，`have < need` の検査 | 0 件（grep） | — |

#### mutation の run

| run（終了時刻） | 行 | 結果 | 木の状態 |
|---|---|---|---|
| remote-fix-1（06:38） | 223（maintenance 165，phase7 58） | 223/223 killed | 修正の途中 |
| remote-fix-2（07:12） | 225 | 225/225 killed | 〃 |
| remote-fix-3（07:30） | 226（P356 を含む） | 226/226 killed | core.rs・mode.rs の数の統合（07:47）より前 |
| remote-fix-4（07:48） | P357 | 1/1 killed | — |
| 227 行の run（記録は transcript のみ．出力ファイル無し） | 227（maintenance 169，phase7 58） | 227/227 passed．生き残りは equivalent の P313 と J58 | 自己申告では数の統合より前（transcript の 13634 行） |
| remote-fix-5（08:02） | control.rs の lib の 18 行（P271 の旧形と P357 を含む） | 18/18 killed | 最後の control.rs（07:56:54）より後 |
| remote-fix-6（08:04 開始） | 「309」と表示（ID は 307 個） | **結果なし**．ログは見出し 1 行で止まる（77 バイト） | 流れていない |
| P271 の新しい形，P313 の注記 | — | **未実行** | — |

**最後の木に対する coverage**：core.rs と mode.rs は 07:47 に，control.rs は 07:56:54 に最後に変わった．最後の木に対して流れたのは remote-fix-5 の control.rs の 18 行だけで，全 list は流れていない．core.rs と mode.rs の行は，07:47 より後に流れた run が無い（227 行の run は数の統合より前と自己申告）．gate で全 list（maintenance と phase7，必要なら phase3–6）を流すこと．

### 4. セルフレビュー

ブリーフの上限（P1 が無くなるか 3 回まで）で，回 1 から回 3 まで．reviewer は `anthropic/claude-haiku-5-5#high`（`--agent plan`，読み取り専用）．各回の指摘と扱い．

#### 回 0（`selfrev-1.log`，06:40）

判定が出なかった．reviewer が「続けますか」で止まり，確定前の一覧だけが残った．やり直して回 1 とした．

#### 回 1（`selfrev-1b.log`，06:58）

| 指摘 | 扱い |
|---|---|
| P1-1 Sol 3 の順序テストが無い | **退けた**（§5） |
| P1-2 O1 のテストが記帳の critical section を固定していない（受理される cold 変更はイベントを出さない） | **直した**：test_support の LockSpy で保持を記録．`ur_25_the_cold_booking_books_in_the_section_that_takes_its_instant`（control.rs:1676），`ur_25_the_layout_cold_change_is_built_in_its_booking_section`（:1722）．行 P349，P355 |
| P2-1 design/18 の Changes「twice」は誤り | **直した**（design/18:595 は「at most once, in any order」） |
| P2-2 design/09 の Changes「MR-38 states that both Providers read it」は誤り | **直した**（本文に無いので，Changes 行を書き直した．0 件） |
| P2-3 A3 の一覧に device.rs の `rm_26_…` が無い | **記録は既にある**：impl の報告 §6（A3 の 3 件ではない．裁定の構造 2 と計画 T5 が名指した書き換え）．plan の A3 一覧は直していない |
| P2-4 R26 は panic でしか殺さない（名前の「不明な板の検査」と違う） | **直した（名前）**：行の名前を panic の殺し方に合わせた．テストは brief の指定（既存の `ur_05_…`）のまま |
| P2-5 parity の `updates` が UHD の `note` 行を両側で除く（Mock は書かない） | **直していない**（§1-7 の穴） |
| P2-6 in-src のテストの hunk に帰属が無い | **この報告で書いた**（§2） |
| P3-1 「(UR-25, design 33 D16)」の形が normative に入った | **退けた**：HEAD にも同じ形が多い．ただし diff は normative 行に計画番号を増やしている（§7-7） |
| P3-2 Sol 2 の configuration of record：refused antenna の claim は観測していない | **直していない**（限界．§1-3） |
| P3-3 Sol 5 の cut は 200 tick の許容差 | **逸脱として記録**（§1-7） |
| P3-4 tx.rs:680 の `self.channels = to.set()` は open の失敗より前に入る（既存） | 今回の変更ではない．記録のみ |

#### 回 2（`selfrev-2.log`，07:14）

| 指摘 | 扱い |
|---|---|
| P1-1 O1：記帳と instant を同じ区間に固定するテストが無い（witness は LATE と REJECT だけ） | **直した**：`ur_25_a_cold_change_takes_the_line_lock_once_for_its_instant_and_its_booking`（control.rs:1750）．受理される変更は lock を 1 回，from-none の enable は 2 回．行 P356 |
| P1-2 Sol 3：順序テストが無い（`idle` の前提） | **退けた**（§5） |
| P2-1 P271 の旧形（`let _held = streams;`）は自己デッドロックで hang する | **退けた**：作業コピー（`/tmp/opencode/probe-p271`）で旧形を流し，hang ではなく `test_support.rs:168` の assert で落ちることを確かめた．P271 はブリーフの「取り直す」形に当て直した（新形は未実行．§3） |
| P2-2 P313 の同値の理由が今のコードの順序と合わない（decimation を読んでいない） | **直した**：注記に格子と decimation の根拠を書いた．行番号は後の数の統合でずれた（§6） |
| P2-3 from-none の失敗で報告が二重になる可能性 | **退けた**：book が false を返すのは時刻変換の失敗だけ（TimeError は replay と finish だけが返す）．device の失敗とは別の経路（読みによる） |
| P2-4 Sol 2 の halt の確認が間接的 | **直していない**（限界） |

#### 回 3（`selfrev-3.log`，07:37）．結論：P1 が 2 件

| 指摘 | 扱い |
|---|---|
| P1-1 `start_rx` の単一 section が固定されていない | **直した**：`ur_25_a_start_rx_takes_the_line_lock_once_for_its_instant_and_its_booking`（control.rs:1788），行 P357（remote-fix-4 と -5 で killed）．**再レビューはしていない（上限）** |
| P1-2 Sol 3：順序テストが無い | **退けた**（§5） |
| P2-1 送信の from-none 拒否が clock を入れる（owner の拒否と違う） | **退けた**（INFERRED）：refusal の登録結果は同じと読んだ．内部の差だけ．**未解決の疑いとして残す**（§1-3，§7-2） |
| P2-2 判断が二か所にある（数の読み，方向の接頭辞） | **直した**：数の読みは `scalar_number`（mode.rs:285）に統合．方向は `current_key_direction`（mode.rs:295）を `book_timed`（control.rs:529）と `book_cold`（:661）で使う．重複していた helper は削除 |
| P2-3 既存テストの hunk（device.rs の `rm_26`）が A3 に無い | 回 1 の P2-3 と同じ（impl の §6 にある） |
| P3-1 parity の cut は 200 tick の許容差 | **逸脱として記録**（§1-7） |
| P3-2 parity.rs:706 と :825（`vh_8_the_same_logical_updates`）に行が無い | 直していない．ブリーフは Sol 5 に行を求めていない |
| P3-3 design/07 の「both Providers read」（`rate_set_matches`）は誤り | **直した**（文を削除） |
| P3-4 design/18 の「Phase 8 measures」（58，313–319 行） | **一部直した**：58 行は直した．241，313，314，319 行は残る（§1-9） |
| P3-5 control.rs:80 の doc の「projection」 | **直した**（control.rs:80–83） |
| P3-6 configuration of record の確認が間接的 | 限界として記録（fake.rs:6201） |

### 5. P344・P345 と Sol 3 の順序テスト

**結論**：Sol 3 の求めた順序（control が owner の最初の poll より先に join する）のテストは書いていない．理由は二つ．その前提（`idle` の初期値 `false` が観測される）がこのコードでは成り立つか疑わしいこと，そして求めた順序では P344 の行（owner の照合）を通らないこと．P344・P345 は別のテストで殺せるようにした．

- **殺すテスト**
  - P344（rx.rs:290 の照合を外す）→ `ur_13_the_reopen_enforces_the_direction_s_rate_comparison`（fake.rs:5953）．rx の腕では，control の from-none の拒否（最初の open．結果が保存される）の後に，owner の最初の open（`start_rx`，fake.rs:5992）が，保存された結果を再適用することを見る（核心は `check_rates` の保存．core.rs:546–551）．tx の腕は from-none の enable を重ねる．
  - P345（tx.rs:673 の照合を外す）→ `ur_13_a_transmit_switch_is_compared_with_the_stored_outcome`（tx.rs:849）．保存された拒否の後で，`configured = false` の switch（tx.rs:832–846）を `do_switch` に渡すと，照合の結果で拒否されることを見る．
  - 両方とも remote-fix-2 と remote-fix-3 で killed．
- **前提の確認（コードの場所）**
  1. `StreamState` の `idle` の既定値は `false`（core.rs:98 の `#[derive(Default)]`，フィールドは :107）．しかし `Control::new` が stream 0 の `idle` を `start.rx.is_none()` と `start.tx.is_none()` にする（control.rs:270–271）．これは owner の thread が起動される前（mod.rs:662 の `Control::new`，mod.rs:665–668 の spawn）．owner の最初の poll はまた `idle` を書く（rx.rs:280）．よって owner が最初に観測する値は既定の `false` ではない．
  2. 求めた順序で start.rx が無い（`idle` が true）ときは，join は from-none に入る（control.rs:790：`from_none = !channels.is_empty() && idle && planned`）．control が自分で open と照合をし（:797–803），segment を `configured = true` で記帳する（:826）．owner はそれを `configured` として開かない（rx.rs:285–287）．owner の照合（rx.rs:290，P344 の行）は通らない．つまり，この順序は P344 を殺さない．control 側の from-none の照合は，既存の `ur_13_an_enable_from_no_stream_compares_the_rate_set`（fake.rs:6148）が見ている．
- **残った穴（ブリーフが懸念した経路）**：owner の最初の open が，その方向の唯一の照合になる場合．start.rx が無いまま `start_rx` が来て，owner が先に poll する（rx.rs:275–283 が `idle = false` にし，rx.rs:288–290 が open と照合する）．P344 の行のテストは保存された結果の経路だけを見るので，この経路を殺さない．`ur_13_*` のテスト名と `RatesMisreported` の使い方から読んだ範囲では，この経路を見るテストは無い（INFERRED．実行していない）．TX も同じ（tx.rs:669–674 の照合は `configured = false` のときだけ）．
- **推奨**：owner が poll を終えてから（settle の後に）`start_rx` を送るテストを書く．sleep で競合は作らず，順序は settle で作る．依頼の順序（control が先）とは逆になる．
- 退けた根拠は上の前提 1 と 2．round 1 P1-1，round 2 P1-2，round 3 P1-2 が同じ指摘．

### 6. P313 の注記（equivalent）

- **成り立つと読んだ**（round 1 と round 3 も読んだ）：
  - (a) 後の stream の channel 集合は空．D14 の拒否（control.rs:697–700）が，後の stream に channel を入れる候補を先に refuse するため．元の `stream_channels(dir, k)` は空で，mutant は stream 0 の集合を取る．
  - (b) 後の stream には line が無い（line を入れるのは stream 0 だけ，control.rs:263–271）．`let Some(old) = old else`（:769）で return する．
  - (c) decimation に届くのは rate だけで，rate は coerce 後の値（:712–713）．格子の値は `decimation` が受ける（x3xx.rs:64–74：`mcr / N` を N に戻すと一致する）．よって拒否は起きない．格子は，説明の格子（x3xx から作られる）と同じと読んだ．
  - → 差は観測されない．
- **注記の問題**：
  - 行番号が，数の統合（11 行）の後にずれた．注記の 755（decimation の検査）は現在 744–748．780（`let Some(old) = old else`）は 769．769（`empty_layout` の定義）は 758，その使用は 774．774（state の読み）は 759–768．
  - (a) の D14 の点が注記に無い．足すとよい．
- 既定の rate（key が無いとき）が格子上にあるかは未確認（round 3 の未確認）．ただし P313 が殺す変更は rate の key 自身なので，candidate に key があり，既定値は使わない（読み）．

### 7. 残っている問題（orchestrator の判断）

1. **Sol 3 の順序（退けた）**：owner が方向の唯一の照合になる経路を殺すテストが無い（§5）．推奨：owner が poll を終えた後に `start_rx` を送るテスト．
2. **送信の from-none 拒否（未解決）**：tx.rs:679–684 は clock を入れ，owner の拒否（tx.rs:686–693）は入れない．推奨：ブリーフどおり owner と同じにする（clock を入れない）．観測の差を調べるなら，送信に出るかを見るテストを先に書く．
3. **parity の `note` 行（直していない）**：UHD は `note == "no channel"` の行を書く．`updates`（parity.rs:685–703）はそれを両側で除くので，差は比べられない．推奨：`note` 行を比べる列を足すか，除く理由を仕様に書く（round 1 P2-5）．
4. **P271 の新しい形（未実行）**：gate で流す．旧形は killed（remote-fix-5）．
5. **P313 の注記（直す）**：行番号（§6）と D14 の点．`mutations.json` の 6932 行付近．
6. **design/18 の「Phase 8 measures」（241，313，314，319 行）**：DA-1 の段階の言い方．直していない．
7. **design 33 の参照（DA-1）**：design/ 全体で，HEAD の 65 件が今は 73 件．diff が normative 行に足したもの（例：design/18:125 の UR-5 の「(UR-5, design 33 D5)」）がある．round 1 P3-1 を退けた理由（HEAD にもある）は弱い．直すかどうか決める．
8. **最後の木の全 list（未実行）**：§3 の coverage．gate で全 list を流す．
9. **限界**：Sol 2 の halt と configuration of record は間接（fake.rs:6201，:6245–6257）．R26 は panic で殺す．parity の cut は 200 tick の許容差．G03・J58・J156 の理由は再導出していない．
10. **記録**：227 行の run の出力ファイルは無い（transcript の 13634 行の自己申告）．remote-fix-6 は結果なし．309 と 307 の差の原因は確かめていない．

## 回 2：指示書


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `4199e51` に 1b-4c と修正の回 1 の未コミットの変更）．
守ること：`impl-brief.md` の「守ること」と同じ（cargo fmt・rustfmt 禁止，mutation は remote_mutate.py だけ，commit しない，`mutations.json` は変える行だけを周りの書式で書く）．
前の回の報告：`fix-report.md`（同じディレクトリ）．その §5 と §7 を読むこと．

### 裁定：次を直す

1. **owner の最初の open が方向の唯一の照合になる経路（報告 §5 の推奨を採る）**
   - 受信：start.rx 無しで始め，owner の poll が落ち着いてから `start_rx` を送る．`RatesMisreported` で rate の照合が拒否し，拒否が報告されることを確かめる．このテストが P344 の行を殺すこと．順序は settle で作り，sleep で競合を作らない．
   - 送信：同じ形で，tx.rs の `configured = false` の照合（P345 の行）が方向の唯一の照合になる経路があるか確かめる．あればテストを書いて P345 を殺す．無ければ，最初の送信の open が必ず arm か from-none を通る理由をコードの場所つきで報告に書く．
2. **送信の from-none 拒否（報告 §7-2）**：owner が拒否したときと同じにする（clock を入れない）．
   - まず，今の差が観測できるかを確かめるテストを書く（例：Manifest の clock の記録，後の burst の扱い）．
   - 直した後にそのテストが通り，直す前の形を mutation の行にして殺すこと．
   - 観測できる差がどうしても作れないときは，同じ形にそろえたうえで，その旨を報告に書く．
3. **P313 の注記（報告 §7-5）**：行番号を使わない．関数や式の名前で場所を書く．D14 の拒否で後の stream の channel 集合が空になる点を足す．

採らないもの（直さない）：
- parity の `note` 行：UHD だけが書く診断の行として除いてよい．doc comment に理由があればよい．
- design/18 の「Phase 8 measures」：HEAD からある文で，この段階の範囲ではない．
- design 33 の参照の数：`rules.py` が通っていればよい．

### 確かめること

1. `cargo test --workspace --no-fail-fast` と `cargo clippy --workspace --all-targets -- -D warnings`．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`．
3. 変えた行・足した行がすべて，ファイルにちょうど 1 回一致すること．
4. mutation は次の行だけを流す：この回で足した行・変えた行，P271（新しい形），P344，P345，P313，tx.rs のこの回の変更が関わる行．
   - `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <IDs>`
   - **前景で流し，終わるまで待つ．turn を終えるとき，背景の job を残さない**（前の回は背景の run を残して turn を終え，run ごと消えた）．
   - COMPILE ERROR も生き残りとして扱う．

### セルフレビュー

reviewer は `anthropic/claude-haiku-5-5#high`（`--agent plan`）．P1 が無くなるか 2 回まで．チェックリスト：
- 上の 3 項目が直っているか．テストが言うものを観測しているか．mutation の行がそれを壊すか．
- 修正の回 1 の 3 回目で直した `start_rx` の lock（P357）が正しいか（再レビューしていない）．

### 報告

`~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/fix-report2.md` に日本語で書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．
中身：3 項目の修正，テストと行，確かめることの結果（mutation は run ごとの結果），セルフレビューの指摘と扱い．

## 回 2：報告

STATUS: done

## Phase 8 段階 1b-4c 修正の回 2 の報告

### 0. 要約

- 三つの項目を直した．受信は，owner の最初の open が方向の唯一の照合になるテストを足し，P344 を殺した．送信は，唯一の照合になる経路を探した結果，決定的には作れなかった（競合の窓だけ）．P345 は既存のテストが殺す．その窓の中では，channel 集合が同じだと照合が行われない欠陥が直す前からある．この回では触れていない．経路と欠陥は §1-2，§6．
- 送信の from-none 拒否は，owner の拒否と同じにした（clock を入れない．switch の時刻は進める）．差を観測するテストを先に書き，直す前の形（P358）を殺した．
- 最後の木で `cargo test --workspace --no-fail-fast` は 1396 件すべて通った．clippy は警告 0．`rules.py --base HEAD` は 0 件．変えた行はすべて，ファイルにちょうど一回一致する．mutation は 12 行を一回流し，11 行が killed，P313 は同値（equivalent）として通った．
- セルフレビュー（`anthropic/claude-haiku-5-5#high`，`--agent plan`，read-only）を二回．P1 は両回とも 0．P2・P3 はすべて直すか，理由を書いて退けた（§5）．
- 何もコミット・push・add していない．HEAD は `4199e51` のまま．

### 1. 三つの項目

以下，crate を書いていないコードの場所は，`control.rs`，`core.rs`，`rx.rs`，`tx.rs`，`mode.rs` が `crates/ezsdr-radio-uhd/src/provider/`，`device.rs`，`host.rs` が `crates/ezsdr-radio-uhd/src/`，`timeline.rs`，`channels.rs`，`x3xx.rs` が `crates/ezsdr-radio/src/` にある．

#### 1-1. 受信：owner の最初の open が唯一の照合（直した）

依頼の順序（start.rx 無し → settle → start_rx）では streamer が開かない．コードの場所：

- `Control::new` は start.rx が無いと空の channel 集合を置く（`control.rs:258–260`．`Channels::first(0)` は空，`timeline.rs:32–33`）．
- planner は channel が空なら segment を始めず（`timeline.rs:863`），start は stream が on の間は無視される（`timeline.rs:860–867`）．
- よって owner の `begin` は開くものが無く（`rx.rs:279–283`），照合も起きない．

そこで依頼の順序に一点だけ足した．from-none の enable を入れ，その最初の open を scripted fault `FakeFault::RxOpenFails` で一回だけ落とす（`device.rs:352`，`:1094`）．uhd-control の from-none は open してから照合する（`control.rs:803–806`）ので，open が落ちると照合の前に返る．次に owner が `start_rx` で開き，owner の照合（`rx.rs:290`）が方向の最初で唯一の照合になって，UR-13 で拒否される．

- テスト：`ur_13_the_receive_owner_s_first_open_is_compared_there`（`crates/ezsdr-radio-uhd/tests/fake.rs:6008`）．見ているもの：最初の拒否は UR-13 でない（open の失敗）．二つ目の拒否は `rx` の UR-13（`:6039`）．`rx_open` は二回（`:6040`）．拒否された segment は configure されない（apply 数が変わらない，`:6041`）．`rates_checked.rx` が真（`:6043`）．順序は settle で作り，競合を sleep で作ってはいない．
- 行：P344（`rx.rs:290` の照合を外す）．名前を「the receive owner's open does not compare the rate set (UR-13)」に，filter を新しいテストに変えた．killed．

#### 1-2. 送信：唯一の照合になる経路は決定的には無い（競合の窓だけ）

依頼は「あれば P345 を殺すテスト，無ければ理由をコードの場所つきで」．調べた結果，最初の送信の open が必ず arm か from-none を通る，とは言えない．arm でも from-none でもない経路を探したが，見つかったのは競合の窓だけだった．次の三段になる．

1. from-none の enable の open が uhd-control で落ちる（`control.rs:803–804`）．照合には行かず，segment は refused になる（`control.rs:834`）．計画上は origin で切られる（`timeline.rs:892–896` の `finish` と `end_at_origin`）．
2. その switch は `retarget` が `Switch{cut: MIN}` として立てる（`tx.rs:184–189`）．clock が無いが switch が残るので，tx の `idle` は偽のまま（`tx.rs:179`）．owner の `step` は clock が無く switch があれば，すぐ `do_switch` を実行する（`tx.rs:237–241`）．
3. この switch が実行される前に二つ目の enable が来る．`from_none` は `idle` を見るので取らず（`control.rs:793`），二つ目は `configured = false` の segment として記帳される．owner はそれを，channel 集合が前と違えば開いて照合する（`tx.rs:670–673`）．その照合が方向の最初で唯一の照合になる．

この窓は決定的には作れない．二つ目の enable は，一つ目の refused な open と owner がその switch を実行する（`tx.rs:237–241`）の間に予約されていなければならないが，owner は switch を見つけ次第実行し，テストからその順序を止める手段が harness に無い．tx の open を落とす `FakeFault` も無いが，それを足しても順序は決まらない．よって送信のテストは書いていない．

窓の中には，直す前からの欠陥が一つある（この回では触れていない）．refused の switch は `to.configured` が真なので open も照合もせず（`tx.rs:670–673`），その後 `self.channels = to.set()` を無条件に入れる（`tx.rs:680`）．二つ目の enable の channel 集合が一つ目と同じだと，`to.set() != self.channels` が偽になり，owner は open も照合もせずに configure する（`tx.rs:670`）．一つ目の open は失敗していたので，この方向は一度も開かれず，照合も行われない．窓を閉じる案（次の段落）は，この経路も閉じると見込まれる（未確認）．

P345 の行（`tx.rs:673` の照合を外す）は，既存の `ur_13_a_transmit_switch_is_compared_with_the_stored_outcome`（`tx.rs:853`）が殺す．このテストは，先に保存された拒否を owner の open が再適用することを見る．照合が最初ではない．だから「唯一の照合」の性質は，どのテストでも見ていない．

閉じる案（この回では変えていない．後の回の候補）：refused の switch が残る間も `idle` を真と扱えば（`tx.rs:179`），二つ目の enable は from-none になり，最初の照合は control 側に残る．

#### 1-3. 送信の from-none 拒否（直した）

直す前の差（コード）：

- uhd-control が from-none で拒否した tx segment は `configured = true` なので（`control.rs:829` の `from_none`．open が失敗しても変わらない），`do_switch` は `Ok(())` で通り，clock を入れる（`tx.rs:685–687`）．
- owner の拒否（`tx.rs:689–696`）は clock を入れず，`at += 1` する．
- 差は二つ．次の segment の switch が，拒否された segment の origin まで待つこと．tx の `idle` が origin まで偽のままになること（`tx.rs:179`）．

直し：

- `tx.rs:684` に `Ok(()) if to.refused => self.at += 1,` を足した（clock を入れない．switch の時刻は進める）．timing 行と，unacked の扱い（`reopened` の計算）は変えていない．
- `Planned.refused`（`control.rs:164–166`）を，その command が refuse されたか（uhd-control でも owner でも）を表す欄として足し，`core.rs:243` で計算する．
- 直したあとの `idle` は owner の拒否と同じになる（次の segment が無ければ真）．

テスト：`ur_25_a_transmit_refusal_from_no_stream_starts_no_clock`（`fake.rs:6047`）．

- 先に書いて差を観測した．直す前（実時計）は，二つ目の `tx_switch` が at=446749882 で実行され，拒否された segment の origin 446715600 より後だった．テストは落ちた．
- 見ているもの：二つ目の `tx_switch` の `at` が，最初の refused segment の origin より前（`fake.rs:6080`）．二つ目の拒否が action を持つ，つまり no stream からの enable として受理された（`fake.rs:6081`）．
- 時計は manual にした（`ClockMode::Manual`，`fake.rs:6054`）．実時計では二つ目の switch と origin の差が百数十ミリ秒しかなく，負荷で揺れうるため．manual の時計は device のスレッドが忙しい間は止まる（`host.rs:260`）．

burst（回 1 の F3 の答え）：refused の transmit segment は origin で切られ（`timeline.rs:892–896`），その clock も reconcile で origin で終わる（`core.rs:262–283`，`:270–271`）．`book_burst` は `current` を，cut の無い最後の segment からしか取らない（`control.rs:863–868`）．取れなければ UR-21（`control.rs:889–891`），取れても clock の domain が違えば UR-21（`control.rs:892–893`）．よって refused の clock に乗る burst は admission で拒否され，「later」には入らず，`tx.rs:205` の "never started" にも届かない．これはコードを読んで確かめた．テストは無い（§6）．

殺す行：

- P358：`tx.rs:684` を `Ok(()) if false && to.refused => self.at += 1,` に変えた形．killed．
- P359：`core.rs:243` の refused を `false` にした形．killed．

#### 1-4. P313 の注記（直した）

- `plan/maintenance/tools/mutations.json` の P313 の `equivalent` 注記を，行番号を使わず，関数と式の名前で書き直した．crate パスも付けた．
- 足した点：D14 の拒否（`Mode::channel_in_later_stream`．`book_cold` が `coerce` の前に呼ぶ，`control.rs:701`）で，後の stream の channel 集合は空になる．そのため `Mode::stream_channels` は k≥1 で空（`mode.rs:197–202`）．`coerce` は範囲内の stream 番号を変えない（`channels.rs` の `coerce_stream`）．後の stream への target は，stream 自身の `sample_rate_hz` の key からしか来ない．
- 注記の論証（コードで確かめた）：`Control::new` が line を作るのは stream 0 だけ（`control.rs:270–272`）．`let Some(old) = old else` の return（`control.rs:772`）は `empty_layout`（`:777`）と `Config`（`:785`）より前にある．decimation の `None if channels.is_empty()` の arm（`:749`）は，rate が coerce 後の格子の値なので届かない．格子は `coerce_numeric`（`crates/ezsdr-radio/src/device.rs`）が `x3xx::rates`（`crates/ezsdr-radio/src/x3xx.rs`）から作るもので，`x3xx::decimation` はその値をすべて受ける．

### 2. テストと行

テスト（この回で足した）：

- `crates/ezsdr-radio-uhd/tests/fake.rs`：`ur_13_the_receive_owner_s_first_open_is_compared_there`，`ur_25_a_transmit_refusal_from_no_stream_starts_no_clock`．
- `crates/ezsdr-radio-uhd/src/device.rs`：`FakeFault::RxOpenFails`（一回だけ落とす scripted fault）．

コード（この回で変えた）：`control.rs`（`Planned.refused` と doc），`core.rs`（`planned()` の計算），`tx.rs`（`do_switch` の arm と test helper の `refused: false`），`device.rs`（`RxOpenFails`）．

行（`plan/maintenance/tools/mutations.json`）：

- 変えた：P344（name と filter），P313（注記）．
- 足した：P358，P359，P360．
- この回の run に入れた（変えていない）：P271（新しい形），P345，J16，P249，P255，P272，P318．

### 3. 確かめたこと

| 確認 | 結果 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 1396 件 pass，0 fail（最後の木．`/tmp/opencode/fix2/ws-final.log`） |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0（`/tmp/opencode/fix2/clippy-final.log`） |
| `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` | 0 problem |
| 行の一致（`old`） | 全 909 行（maintenance 743，phase7 166）が，それぞれのファイルにちょうど一回一致（`/tmp/opencode/fix2/check_rows.py`） |
| 新しい二つのテスト（単独） | pass（manual clock の版） |

### 4. mutation（この回の run は一回，最後の木）

`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 P271 P313 P344 P345 P358 P359 P360 J16 P249 P255 P272 P318`（前景で実行し，終わるまで待った．ログ：`/tmp/opencode/fix2/mutate-final.log`）

| 行 | 結果 | 行が作る誤り |
|---|---|---|
| P271 | killed | uhd-control が from-none の device 呼び出しの間も line lock を持つ |
| P313 | equivalent (survived) | stream の rate 変更が，key の指す stream でなく stream 0 の channel を取る（同値．§1-4 の論証．remote_mutate は同値を通過として数える） |
| P344 | killed | 受信 owner の最初の open が rate の集合を照合しない（新しい filter） |
| P345 | killed | tx の reopen が rate の集合を照合しない（既存のテストが殺す．§1-2） |
| P358 | killed | 拒否された tx segment（uhd-control の from-none）が clock を始める |
| P359 | killed | 計画の segment が，その command が拒否されたのに refused にならない |
| P360 | killed | 受信 open の scripted 拒否が起きない（fault の一回性） |
| J16 | killed | uhd-tx が自分の device の拒否を書かない |
| P249 | killed | uhd-tx が拒否を受信 stream の line に書く |
| P255 | killed | switch が owner の segment の index を動かさない |
| P272 | killed | uhd-tx が switch の device 呼び出しの間も line lock を持つ |
| P318 | killed | 送信 streamer を channel 数が変わったときだけ開き直す |

結果：12 行中 12 行が通過（killed 11，equivalent 1）．COMPILE ERROR も NOT FOUND も無い．

### 5. セルフレビュー

reviewer は `anthropic/claude-haiku-5-5#high`，`--agent plan`，read-only（木を変えず，build もしない）．

#### 5-1. 回 1（`/tmp/opencode/fix2/review1.log`）：P1 0 件

| 指摘 | 重さ | 扱い |
|---|---|---|
| F1：P344 の名前が新しいテストと合わない | P2 | 直した（名前を変えた） |
| F2：`Planned.refused` の doc が狭い | P2 | 直した（uhd-control でも owner でも，と書いた） |
| F3：refused の clock に乗る burst の扱い（"never started"） | P2 | コードで確かめた．admission で UR-21 になる（§1-3）．テストは無い（§6） |
| F4：行に mutation run が無い | P2 | 直した（§4） |
| F5：「唯一の照合の経路は無い」に競合の窓の但し書きが要る | P2 | 書いた（§1-2） |
| F6：P313 注記に crate パスが無い | P3 | 直した（§1-4） |
| F7：「device 呼び出しの下で lock を持たない」の書き方 | P3 | 書き直した（§6） |
| F8：`start_rx` が `Streams::book` の結果を見ない（HEAD にもある） | P3 | 直していない（§6） |
| F9：実時計の余裕（百数十ミリ秒） | P3 | manual clock にした（§1-3） |

回 1 の確認項目：P357（`start_rx` の lock）は正しいと判定された．`streams` の lock の下で `updates` を取らず，device 呼び出しもしない（`config_at` と `held_updates` は自分の状態を読むだけ，`device.host()` は getter で `crates/ezsdr-radio-uhd/src/device.rs:221–223`，`:835–837`）．lock は一回だけ取る（`ur_25_a_start_rx_takes_the_line_lock_once_for_its_instant_and_its_booking`）．デッドロックは見つからなかった．

#### 5-2. 回 2（`/tmp/opencode/fix2/review2.log`）：P1 0 件

| 指摘 | 重さ | 扱い |
|---|---|---|
| 1：`ur_25` の注釈「manual clock で時刻が正確になる」は言い過ぎ | P2 | 直した（device の時計は忙しい間は止まる，と書いた） |
| 2：P313 注記の crate パスが曖昧，`x3xx.rs:611` が decimation とある | P2 | 直さず．注記は既に `crates/ezsdr-radio/src/...` と `x3xx::decimation` で書いてある．`x3xx.rs:611` は decimation ではない（decimation は `x3xx.rs:64`）．指摘は古い版の引用 |
| 3：refused clock の burst を試すテストが無い | P2 | テストは足さず，§6 の限界に書いた（コードの確認は §1-3） |
| 4：P313 注記に，後の stream の target の出所と `coerce` 後の候補の説明が無い | P3 | 足した（§1-4） |
| 5：「nothing is compared or stored」は保存を観測していない | P3 | 直した（「open の前には照合しない」） |

回 2 のチェックリスト：

- 変更 (a) から (d)：確認した．(d) は §1-3 と同じ結論．
- 行：P344，P358，P359，P360 が，それぞれの名前のテストを壊すと判定された（run でも killed）．
- P313 の論証：正しいと判定された．
- 注釈：指摘 1 と 5 を直した．
- その他の欠陥：見つからなかった．

### 6. 残る問題と限界

- **送信の唯一の照合**：窓は決定的に作れない（§1-2）．P345 は保存された拒否の再適用で殺している．
- **送信の窓の欠陥（直す前から）**：窓の中で二つ目の enable の channel 集合が一つ目と同じだと，owner は open も照合もせずに configure する（`tx.rs:670`，`:680`）．この回では触れていない．窓を閉じる案（refused の switch が残る間も `idle` を真に扱う．`tx.rs:179`）が，この経路も閉じると見込まれる（未確認）．後の回の最初の候補．
- **refused の clock に乗る burst**：テストは無い．admission で拒否されることはコードで確かめた（§1-3）．
- **F7（lock の書き方）**：`streams` の lock は device 呼び出しの間に持たない（`book_cold` は from-none の device 呼び出しの前に落とす．P271 がそれを守る）．既存の `ColdConfig::configure` は，`updates` の lock を `core.configure`（device 呼び出し）の間ずっと持つ．HEAD の `control.rs:95` と同じで，この回では変えていない．`start_rx` の `streams` の下の `device.host()` は getter で，UHD の呼び出しではない．
- **F8（HEAD にもある）**：`start_rx` は `Streams::book` の結果を見ない（`control.rs:418`，HEAD の `:331`）．失敗は `book` が RM-26 として報告する（`core.rs:173–190`）．残るのは，どの item も指さない `configs` の項目一つで，読まれず，prune（`core.rs:229`）で消える．この回では変えていない．
- parity の `note` 行（回 1 の P2-5）は残る．依頼どおり触れていない．design/18 の「Phase 8 measures」は，HEAD の 5 行のうち 58 行が回 1 で「the bench measures」に直してあり，残る 4 行（241，313，314，319 行）は回 2 では触れていない．design 33 の参照の数も触れていない（`rules.py` は通る）．
- 全 list の mutation は流していない．この回の行だけを流した（最後の gate は orchestrator）．
- 未コミット（HEAD `4199e51`）．コミットは owner が行う．

## 回 3：指示書


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `4199e51` に 1b-4c と修正の回 1・2 の未コミットの変更）．

### 守ること

- `cargo fmt`・`rustfmt` を流さない（tree はわざと rustfmt の形にしていない．mutation の行はソースの文字列にちょうど一致する必要がある）．新しいコードは周りの書き方に手で合わせる．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．
- commit・push・`git add` をしない．
- `plan/*/tools/mutations.json` はファイル全体を書き直さない．変える行・足す行だけを，周りの書式（indent など）のまま書く．
- 背景の job を残さない．長い command は前景で，Bash の timeout を 600000 にして流す．
- Python は `PYTHONDONTWRITEBYTECODE=1` で流す．

### 読むもの（同じディレクトリ `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/`）

- `sol-rereview.md` の末尾（`判定：FAIL` から後）：直す指摘．
- `fix-report2.md` §1-2・§6：同じ欠陥を実装役が見つけた記録．

### 直すこと：拒否された TX の switch が，開いていない channel 集合を「開いている集合」として残す（Sol の再レビュー 1，major）

- **欠陥**：`crates/ezsdr-radio-uhd/src/provider/tx.rs` の `do_switch` は，open も照合も行わない switch（from-none で拒否された segment，`configured = true`）や，open か rate の照合が失敗した switch でも，`self.channels = to.set()` を先に書く．次の segment の channel 集合が同じだと，owner は open も照合もせずに configure し，clock を始める．
- **直し方（裁定）**：`self.channels` は，streamer が実際に開いて rate の照合も通った集合だけを持つ．
  - 拒否（`to.refused`），open の失敗，照合の失敗のときは，開いている集合を無効にする（次の switch が必ず開き直して照合する形）．
  - 照合の拒否は `check_rates` が方向ごとに結果を保存しているので，開き直しの照合で保存済みの拒否が再適用される．そのことをテストで確かめる．
  - 受信の owner（`rx.rs`）に同じ形の欠陥があるかを確かめる．あれば同じように直す．無ければ理由をコードの場所つきで報告に書く．
- **テスト**（tx.rs の既存の unit test のように，`read` と `step` を直接動かして順序を固定する．sleep で競合を作らない）
  1. open の失敗：一つ目の enable が from-none で open に失敗して拒否される → `tx.read(None)` だけを実行 → `step` の前に，同じ channel 集合を使う二つ目の cold 更新を記帳する → switch を進める → 二つ目で open し直して照合することを確かめる．
  2. 照合の拒否：一つ目で open は成功し，rate の照合が拒否する → 同じ順序 → 二つ目で保存済みの拒否が再適用され，clock も burst も始まらないことを確かめる．
- **mutation の行**：直した箇所を元の形に戻す行（例：拒否のときも `self.channels` を書く）を足し，上のテストが殺すこと．

### 確かめること

1. `cargo test --workspace --no-fail-fast` と `cargo clippy --workspace --all-targets -- -D warnings`．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`．
3. 変えた行・足した行がすべて，ファイルにちょうど 1 回一致すること．
4. mutation は次の行だけを流す：この回で足した行と変えた行，`do_switch` の行（J16，P249，P255，P272，P318，P345，P358），受信側を変えたならその箇所の行．
   - `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <IDs>`
   - COMPILE ERROR も生き残りとして扱う．

### セルフレビュー

reviewer は DeepSeek．回ごとに新しく起動する（前の回の推論を引き継がないため）：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 1500 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat <チェックリストのファイル>)" > <ログ> 2>&1
```

P1 が無くなるか 3 回まで．チェックリスト：
- 上の欠陥が直っているか．受信側の扱いは正しいか．
- 2 本のテストが，言っているものを観測しているか．mutation の行がそれを壊すか．
- 修正が前の回の性質を壊していないか：拒否した送信は clock を始めない（P358），from-none の拒否は値を保持して halt する，`streams` の lock は device 呼び出しの間に持たない．
- 範囲の外の変更が無いか．
- 「すべて・だけ」と言っている箇所を grep で確かめたか．

指摘はすべて直すか，理由をつけて退ける．黙って持ち越さない．

### 報告

`~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/fix-report3.md` に日本語で書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．
中身：修正，テストと行，確かめることの結果（mutation は行ごと），セルフレビューの各回の指摘と扱い．

## 回 3：報告

STATUS: done

## 修正の回 3（Phase 8 1b-4c，Sol 再レビューの major）

### 修正

- **送信（uhd-tx，`provider/tx.rs`）**: 拒否・失敗した switch が，開いていない channel 集合を「開いている集合」として残していた．`self.channels` は `Ok(())` かつ `!to.refused` の腕でだけ更新する．新しいフィールド `verified: bool` を足し，`self.verified = false;` を match の前に置く．成功時だけ `verified = true` にする．`verified` が偽なら，同じ集合でも `tx_open` と `check_rates` をやり直す（保存済みの照合結果が再適用される）．
- **受信（uhd-rx，`provider/rx.rs`）**: 同じ欠陥があった．uhd-control の from-none の enable が `rx_open` か `check_rates` で失敗すると，uhd-rx の owner は開いていない集合を「開いている」と扱い続ける．`StreamState` に `pub unopened: bool` を足し（`core.rs`），control が from-none の失敗時にだけ立てる（`control.rs`，`streams` の同じ critical section 内）．`Rx::begin` は lock 内でこのフラグを `take` し，立っていれば `self.opened` を空にして reopen と照合をやり直す（`rx.rs:283-295`）．
- **検討して退けた案**: 「`next` が None の begin では opened を広く無効化する」案．`rm_26_the_timeline_against_uhd_rx` が「同じ channel 数なのに rx_open がもう一度呼ばれた」で 9 seed 落ちたので，退けた．フラグは control が失敗したときだけ立つので，差分試験のモデルには現れない．
- **RX の分析**: 拒否された zero-length の from-none segment は RX の plan に載らない．だから RX owner はそれを見られず，control からのフラグが必要になる．

### テストと mutation 行

新しいテスト（すべて pass）:
- `tx.rs` `ur_25_a_refused_enable_leaves_its_set_unopened_for_the_next_switch`（fake でなく lib）
- `tx.rs` `ur_13_a_refused_comparison_is_repeated_at_the_next_switch`
- `tx.rs` `ur_13_a_refused_switch_to_another_set_leaves_the_old_set_unverified`（回 3 のレビュー後に追加）
- `tx.rs` `ur_13_a_switch_to_the_verified_set_opens_nothing`（回 3 のレビュー後に追加）
- `control.rs` `ur_13_a_failed_open_from_no_stream_leaves_the_receive_streamer_unopened`（停止した受信 stream と idle の owner を作って from-none にする）
- `rx.rs` `ur_13_an_unopened_receive_streamer_is_opened_again`

mutations.json の変更: P318 の old/new を新しい再利用行へ移した（`verified` を含み，`count()` 比較）．新しい行は P361〜P366 の 6 本．P362 と P363 の filter は新しいテストを指す．既存行は変えていない．合計 749 行（743 + 6）．

| 行 | 内容 | 結果 |
|---|---|---|
| P318 | 送信の reopen は channel 数が変わるときだけ（更新） | killed |
| P361 | 再利用が `verified` を無視する | killed |
| P362 | `verified = false` を match の前に置かない | killed（初回は SURVIVED，テスト追加後 killed） |
| P363 | 成功時に `verified = true` を立てない | killed（初回は SURVIVED，テスト追加後 killed） |
| P364 | control の失敗で unopened を立てない | killed |
| P365 | control が unopened を保存しない | killed |
| P366 | uhd-rx が unopened で reopen しない | killed |
| J16, P249, P255, P272, P345, P358 | do_switch の既存行 | killed |
| P344 | rx owner の open が照合しない（rx.rs を変えたため追加） | killed |

### 確かめたこと

- `cargo test --workspace --no-fail-fast`: exit 0，失敗なし（最終 run）．
- `cargo clippy --workspace --all-targets -- -D warnings`: pass．
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problem．
- 変えた行と足した行は，それぞれのファイルにちょうど 1 回一致する（rules.py と remote run の両方で確認）．
- rm_26 の差分試験（lib の `rm_26_the_timeline_against_uhd_rx` と `..._control`）は pass．`unopened` は差分モデルでは立たないので，挙動に影響しない．
- mutation: `REMOTE_SSH="ssh -o BatchMode=yes" remote_mutate.py maintenance,phase7 sim02:8,sim03:8 P318 P361 P362 P363 P364 P365 P366 J16 P249 P255 P272 P344 P345 P358` の最終 run で 14/14 killed．phase7 の該当行はない．

### セルフレビュー

reviewer は `opencode-go/deepseek-v4.1-flash#max`，`--agent plan`，回ごとに新しい `opencode2 run --standalone`．checklist は `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/fix3-review-checklist.txt`．

**回 1**（`fix3-review-round1.log`）
- P1「P363 が生き残る」: **退けた**．reviewer は，成功時の `verified = true` を消しても初期値の true が残ると考えた．しかし `self.verified = false;`（tx.rs:684）は match の前に必ず走るので，P363 では成功後も偽のまま，2 回目の switch が reopen する．実際に，P363 は remote run で `ur_13_a_switch_to_the_verified_set_opens_nothing` により killed（上の表）．
- P2「rx テストの『compares again』を観測していない」: **直した**．rx.rs のテストのコメントから「compares again」を外し，照合は他の `ur_13_*` テストに任せると書いた．assert の対象（rx_open の回数）は変えていない．
- P2「コメントがコードより強い不変条件を書いている」（`verified`，`unopened`）: **直した**．`verified` は空集合で真で始まることと，reopen の理由を書き足した．`unopened` は「開いたが照合が失敗した場合も含む」と書き，reopen で照合が繰り返されると書いた．

**回 2**（`fix3-review-round2.log`）
- P0・P1 はなし．reviewer は，回 1 の P1 が誤りだったことを自分で確認した．
- P2-1「`addressed` の『1 か所』という主張が `control.rs:890` で成り立たない」: **退けた**．この分岐（`node != Some(&self.core.tx_id)`）は 1b-4c の差分で，この回 3 の修正の前からある（回 3 は from-none の block だけを変えた）．挙動は reviewer の提案と同じなので，直すなら 1b-4c 側の作業として扱うべき．回 3 の範囲外としてここに記録する．
- P2-2「`channels` の doc が修正後の意味と合わない」: **直した**．「最後に成功した switch が開いて照合した集合．拒否・失敗した switch は前の集合のまま残し，`verified` を偽にする」と書いた．

**回 3 は不要**: 回 2 で P1 以上がなくなったので，ループは終了．

### 限界

- 受信側の「open は成功して configure だけ失敗した場合」は，mark が立たない．そのとき owner の `opened` が古い集合と一致しても，reopen されない経路が無いかをコード上で確認した（owner の失敗は `self.opened = Channels::first(0)` で無効化済み，次の enable は control の from-none が必ず開く）．専用のテストは無い．
- P2-1（control.rs:890 の `node` 直接参照と `ezsdr_radio::addressed` の doc）は 1b-4c 側の残りで，この回では直していない．
- コミットと push はしていない．`git add` もしていない．変更はすべて作業ツリー上．
- gate の mutation は `remote_mutate.py` だけで行い，main tree には mutant を当てていない．ただし回 2 の reviewer は（その log によれば）外部の debug copy で mutant を当てて試験していた．brief の規則では gate の結果にならない．その結果は参考であり，gate は上の remote run の 14/14 である．

## 回 4：指示書


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `4199e51` に 1b-4c と修正の回 1〜3 の未コミットの変更）．
「守ること」「セルフレビュー」の形は `fix3-brief.md`（同じディレクトリ）と同じ（DeepSeek の reviewer，P1 が無くなるか 3 回まで）．

### 読むもの（`~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/`）

- `sol-rereview2.md` の末尾（`判定：FAIL` から後）：直す指摘．
- `fix-report3.md`：前の回の報告．

### 直すこと：RX の owner が，uhd-control が開いた別の集合を古い集合と取り違える（Sol 再レビュー 2 の 1，major）

- **欠陥**：`unopened` の印は，uhd-control の from-none の open か rate の照合が失敗したときだけ立つ（`control.rs` の `unopened = opened.is_err() && dir == Dir::Rx`）．open が成功して configure だけが失敗したとき，または全部成功しても後の cold 更新がその segment を長さ 0 にして owner が読まないとき，device の streamer は新しい集合なのに，owner の `opened` は古い集合のままになる．同じ古い集合の segment が来ると，`begin` は開き直さずに始める．
- **直し方（裁定，Sol の案）**：uhd-control が受信の from-none で open を試みたら，成否にかかわらず印を立てる（`unopened = dir == Dir::Rx`）．印の名前と doc comment は意味に合わせて直す（例：「uhd-control が streamer を開いたかもしれない．owner の記録は当てにならない」）．
  - 次の segment が uhd-control の configure した成功の segment なら，既存の経路で owner の集合が更新されることを確かめる．余計な reopen を増やさないこと（増えるなら報告に書く）．
- **テスト**（owner を止めた決定的な形．`read`/`begin` などを直接動かす．sleep で競合を作らない）
  1. 集合 A=`{0}` で受信を止め owner を idle にする → uhd-control が集合 B=`{0,1}` を開き照合は通り configure だけ失敗 → 同じ実効時刻で A への cold 更新 → owner が `begin` で開き直すこと．
  2. B の configure まで成功し，owner が読む前に A への cold 更新が B の segment を長さ 0 にする → owner が開き直すこと．
  - unit で組みにくければ，印の立ち方（成功・configure 失敗・open 失敗の 3 つで立つ）と `begin` の消費を分けて固定してよい．その場合は分けた理由を報告に書く．
- **mutation の行**：元の条件（`opened.is_err() &&`）に戻す行を足し，上のテストが殺すこと．

### 確かめること

1. `cargo test --workspace --no-fail-fast` と `cargo clippy --workspace --all-targets -- -D warnings`．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`．
3. 変えた行・足した行がすべて，ファイルにちょうど 1 回一致すること．
4. mutation は，この回で足した行・変えた行と，`unopened` に関わる既存の行（P361〜P366 のうち受信のもの）だけを流す．前景で流す．

### 報告

`fix-report4.md`（同じディレクトリ）に日本語で書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．
中身：修正，テストと行，確かめることの結果（mutation は行ごと），セルフレビューの各回の指摘と扱い．

## 回 4：報告

STATUS: done

## 修正の回 4（Phase 8 1b-4c，Sol 再レビュー 2 の major）

### 修正

- **欠陥の直し方**: 受信の印を `unopened` から `stale` に改名し，意味を変えた．uhd-control は受信の from-none の enable で streamer を開こうとしたら，open・照合・configure の成否にかかわらず印を立てる（`provider/control.rs`，`stale = dir == Dir::Rx;`．従来は `opened.is_err() && dir == Dir::Rx`）．uhd-rx の `begin` は lock 内で印を take し，立っていれば `opened` を空にして開き直す（`provider/rx.rs`）．次の segment が uhd-control の configure した成功 segment なら，既存の経路（`next.configured` で open を飛ばし，`opened = channels`）で owner の集合が更新される．余計な `rx_open` は増えない．
- **裁定の変更点**: 印の名前と doc comment を実際の意味に合わせた（`provider/core.rs` の `StreamState.stale`）．
- **テスト用の fake**: configure の失敗を作るため `FakeFault::RxApplyFails`（`device.rs`，受信の最初の apply を一度だけ拒否）を足した．テスト専用．
- **brief からの逸脱（理由つき）**: brief のテスト 1・2（owner を止めた順序のテスト）は，印の立ち方と `begin` の消費を分けて固定した．理由: `rx()` の helper が一回 `rx_open` を呼んで `opened` を記録するため，owner を止めた完全な順序の再現は別の helper が要る．代わりに (a) control の単体テストで，四つの outcome（無故障・open 失敗・configure 失敗・照合失敗）のすべてで印が立つことを見る，(b) rx の単体テストで，印が立った begin が同じ集合でも一回開き直すことを見る，とした．

### テストと mutation 行

新規・改名したテスト（lib，`cargo test -p ezsdr-radio-uhd --lib -- ur_13` は 6 本 pass）:
- `control.rs` `ur_13_an_enable_from_no_stream_marks_the_receive_streamer_stale_whatever_it_gives`（旧 `ur_13_a_failed_open_from_no_stream_leaves_the_receive_streamer_unopened` を改名し，四つの outcome の loop にした）
- `rx.rs` `ur_13_a_stale_receive_streamer_is_opened_again`（旧 `ur_13_an_unopened_receive_streamer_is_opened_again`）．開き直しの回数は `begin` の前後で数える．`rx()` helper の一回の開きを除くため．

mutations.json（行ごと，すべて `old` がファイルにちょうど 1 回一致）:

| 行 | 内容 | 結果 |
|---|---|---|
| P364 | enable from no stream が印を立てない（`stale = false;`） | killed |
| P365 | 印を保存しない（`if false {`） | killed |
| P366 | uhd-rx が印で開き直さない（`&& false`） | killed |
| P367（新規） | 印を open 失敗時のみ立てる（元の条件 `opened.is_err() && …`） | killed |

mutation は `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 P364 P365 P366 P367` を前景で流し，4/4 killed（`fix4-mutate.log`）．

### 確かめたこと

- `cargo test --workspace --no-fail-fast`: exit 0，失敗なし（`fix4-test.log`）．
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0（`fix4-clippy.log`）．
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problem．
- 変えた行・足した行はすべて，それぞれのファイルにちょうど 1 回一致（P318 と P344〜P367 を含む確認スクリプトで確認）．
- 後から行ったのは doc comment と test comment の文言の変更だけ（P2 の指摘への対応）．その後 `ur_13` の 6 本と rules.py を再実行して pass．mutation 行の `old` は変わっていない．

### セルフレビュー

reviewer は `opencode-go/deepseek-v4.1-flash#max`，`--agent plan`，`opencode2 run --standalone`．チェックリストは `fix4-review-checklist.txt`．

- **回 1 の試行 1**（`fix4-review-round1-attempt1-failed.log`）: 途中で Transport error と OpenAI stream の終了が出て，判定が無いまま 1500 s の timeout（終了コード 124）で止まった．完了していないので数えない．
- **回 1 の試行 2**（`fix4-review-round1.log`）: 判定「指摘なし（P1 以上 0 件．P2 を 3 件付記）」．P1 が無くなったので，ループはここで終わり．
  - **P2-1（採用・直した）**: `stale` の doc，`control.rs` と `rx.rs` の comment が「次の begin で必ず開き直す」と読めるが，`configured` の segment では開き直さない．doc と comment を「開き直すのは，uhd-control が configure した segment でない限り」に直した．
  - **P2-2（採用・直した）**: テストの comment が「rate comparison」を outcome として挙げていたが，その outcome は四つの fault に無い．comment から「rate comparison」を外した．テストの assert は変えていない．
  - **P2-3（退けた，理由つき）**: `configured` の近道で，後続の from-none が device の集合を置き換えた後の古い configured segment を owner が開き直さずに走らせ得る．これは fix 3 の状態と HEAD にもある既存の経路で，この回の混入ではない（reviewer も INFERRED と明記）．直すには，device の集合を `StreamState` に持つか，印が立った begin で `configured` の近道を使わないかの設計判断が要る．後者は余分な `rx_open` を増やし，`ur_25_a_refused_enable_from_no_stream_halts_and_keeps_its_values` の回数の pin と Sol 再レビュー 2 の「増やさない」の条件に衝突する．この回の範囲外として，段階 1b-4d の stream 一般化で扱うことを推奨する（orchestrator の判断事項）．

### 限界・未確認

- P2-3 の系列は決定的テストで確かめていない（INFERRED）．
- 実機（BP0）と Python は実行していない．
- コミットと push はしていない．`git add` もしていない．変更はすべて作業ツリー上．
