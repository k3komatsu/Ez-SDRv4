# 1b-4c の orchestrator のレビュー（AGENTS.md §9 の 6：指示書への忠実さ・変えてはならない振る舞い・設計の意図）

対象：HEAD `278de14` からの未コミットの diff（`mutations.json` は空白の差を戻して読んだ），`report.md`．

## 所見

### O1（major）：`book_cold` と `start_rx` が，`e`・`seq` を取ってから記帳するまでの間に `streams` の lock を離す．今の key のモードでも同じで，#63 の窓が開き直る

- **場所**：`crates/ezsdr-radio-uhd/src/provider/control.rs`．
  - `book_cold`：`let (seq, now, e, late) = { let mut streams = lock(...); … };` の後，`let mut streams = lock(&self.core.streams);` で記帳する．
  - `start_rx`：同じ形．
- **HEAD との違い**
  - HEAD は 1 つの critical section で instant を取って記帳した．コメントは「The instant is taken under the lock that books it, so that nothing pruned meanwhile takes effect after it (#63)」．
  - 間で lock を離していたのは from-none の device 呼び出しだけで，そこは「its owner is idle」が理由だった．
- **報告 §2 の 4 の根拠が成り立たない理由**
  - 報告は「control thread が唯一の booker で，prune の `before` は常に `now` 以前」と言う．
  - しかし booker は control だけではない．uhd-rx と uhd-tx も refusal や End を記帳し，prune もする（`Streams::prune`）．
  - prune の `before` は owner が既に通った item の effective instant で，prune する時刻 `t2` 以前である．`e` を決めたときの `now` 以前とは限らない．
- **失敗の筋書き**
  1. `at` 無しの cold 変更が `e = now₀` を取る．
  2. lock を離している間に，uhd-rx が `now₀ < before ≤ t2` で prune する．
  3. `e < before` の item が，畳まれた line に記帳される．
  - 結果：「後から記帳したものは畳んだ境界より前に効かない」（#63）が破れる．
- **lock を離した理由**：セルフレビューの P2-3 の「lock の順序（`updates` → `streams`）」への対応だった．ただし，記帳の前に作る `ColdConfig` は新しく作るもので，まだ誰とも共有していない．順序の規則が防ぐ deadlock の相手がいない．
- **修正**
  - HEAD と同じく，instant・seq・記帳を 1 つの critical section に戻す（両方のモード）．
  - layout の `ColdConfig` は lock を取らずに作れる形にする．例：投影した held の更新を最初から持つ constructor（`ColdConfig::layout_with_clock(clock, base, updates)`）．その後で `streams` を取る．
  - `config_at(e)` と `coerce` は lock を取らないので，critical section の中で行ってよい．
  - from-none の device 呼び出しだけは，HEAD と同じ理由（owner が idle）で外に出す．
  - テストを 1 本足す．`book_cold` の instant と記帳の間に prune が入る経路を，unit で直接組む．または lock を持ったまま記帳することを spy で固定する．
  - mutation の行は「lock を離して取り直す」形にする．P271 の当て直しを見直すこと．

### O2（minor）：spec を設定した後の `have < need` は死んだ code．消して，R23・R26 を引退させる

- **理由**：spec を layout の要素から設定した後は，device の channel 数は要素数と同じになる．報告 §5 のとおり，`have < need` は発火しない．
- **対象**：layout の腕の「the layout's … names N channels」と，今の key のモードの「profile x3xx needs N channels」の両方．
- **やること**
  - 両方の検査を消す．
  - phase7 の R23（検査を外す）と R26（`0..have`）は，壊す規則が無くなったので削除する．`equivalent` の欄は残さない．
  - 関係する design の文（UR-5 の channel 数の拒否）も消す．`ur_05_the_profile_must_be_the_device_s_front_ends` の 2 つの assert は，A3 の 1 のとおり UHD の lookup の文のまま．
- **効果**：phase7 の道具が `equivalent` を読まない問題（報告 §2 の 12）も，これで消える．

### O3（裁定）：P313・P344・P345 の equivalent は認める．ただし 1b-4d で見直す

- **P313**：stream 0 だけの段階では，`stream_channels(dir, 0)` と `stream_channels(dir, k)` が同じ．
- **P344・P345**：方向の最初の open は `arm` か from-none のどちらかである．
- 1b-4d では stream ごとに thread を持ち，2 本目の stream の最初の open が uhd-rx/uhd-tx の reopen になりうる．そのため，どちらの行も 1b-4d の送り一覧に入れる．

### O4（記録）：床で上がった後に拒否される cold 変更の LATE_COMMAND（報告 §2 の 14）

- MockRadio の layout のモードは `LATE_COMMAND` を出してから拒否する．UHD は今の key のモードでも HEAD から出さない．
- event の報告の規則が 2 か所にある機構（`mech:radio-rule-implemented-twice`，#51 と同じ）．
- この段階では直さない．handoff §3 に足し，段階 2（#51 の設計メモ）で一緒に決める．parity は受理される床の列だけを比べる（今の形のままでよい）．

### O5（記録）：front end の照合が，layout と今の key のモードの 2 つの腕に分かれている

- 実装役は理由をつけて退けた（規則が違う．1b-5 で今の key のモードを消すときに畳む）．
- 認める．1b-5 の送り一覧に入れる．

## 確かめて問題がなかったこと

- **`from_binding` の順序**：selector → serial の登録 → master clock → spec（要素のある方向だけ）→ front end．登録は guard の値で，途中の失敗では drop される．
- **Basic/LF の照合**：front end の番号まで見る（`front_end_matches`）．
- **共有した関数**：`keys` の読み（`ezsdr_radio::channel_keys`），宛先（`ezsdr_radio::addressed`），床（`timeline::layout_instant`），重複の検査（`x3xx::check_repeat`）．
  - MockRadio の書き換えは同じ式の置き換えで，挙動は不変．
- **rate の照合**：`check_rates` は device を lock の外で読み，方向ごとに 1 回だけ比べる．arm・from-none・reopen のどれでも同じ関数を通る．
- **開き直し**：rx・tx・from-none のどれも，集合（`Channels`）で比べる（M1）．
- **channel ごとの設定**：
  - `ColdConfig` の更新は channel を持つ．今の key のモードは channel を `0` にして全 channel に効かせる（`timed.chan.is_none_or`）．
  - keys のモードの `applied` の行は，具体の channel key になっている．
- **宛先**：layout の `Stop` は stream ごと，`start_rx` は `k ≠ 0` なら黙って受ける（MockRadio と同じ）．`TxBurst` は `tx/s0` だけ．

## 判定

O1 を直せば通る．Sol のレビューと合わせて裁定し，修正の回を 1 回にまとめる．

## 裁定（Sol のコードレビューと合わせて）

- O1・Sol 1〜3 は major として修正の回 1 で直す．O2 は採る（R23 削除，R26 当て直し）．O3 は Sol 3 を受けて撤回し，P344・P345 の `equivalent` を消して殺すテストを書かせた．O4 は Sol 4 を受けて撤回し，layout のモードの LATE_COMMAND を Mock と同じ順序にした．O5 は採る（1b-5 で畳む）．Sol 4〜7 の minor はすべて採った．指示書と報告は [1b4c-fix-reports.md](1b4c-fix-reports.md)．
- 修正の回 1 の報告で実装役が退けた Sol 3 の順序テスト（control が owner の最初の poll より先に join する）は，`Control::new` が owner の thread を起動する前に stream 0 の `idle` を決めるので起こらない，という反論を採った．Sol も再レビュー 1 で撤回した．代わりに，owner の最初の open が方向の唯一の照合になる経路のテストで P344 を殺した（修正の回 2）．
- 採らなかったもの：parity が UHD だけの `note` 行を除くこと（UHD の診断の行），parity の clock の cut の 200 tick の許容差（run ごとの格子），spec 18 の「Phase 8 measures」（HEAD からある文）．Sol は再レビュー 1 で異議なし．
- 再レビュー（[1b4c-sol-rereviews.md](1b4c-sol-rereviews.md)）：1 は TX の拒否された switch が開いていない集合を「開いている」と残す major（修正の回 3），2 は RX の owner が uhd-control の開いた別の集合を古い集合と取り違える major（修正の回 4），3 は PASS．3 が見つけた `configured` の近道の欠陥（HEAD `278de14` にもある）は，1b-4d の最初の手順として延期した（下）．

## 指摘の数（source・重さ・機構）

| source | 時点 | major | minor | 機構（`plan/maintenance/failure-mechanisms.md`） |
|---|---|---|---|---|
| orchestrator（計画の M1）＋Sol（計画） | コードの前 | 1＋6 | 2 | 計画の段階で直したので数えない |
| Opus（O1〜O5） | コードの後 | 1（O1） | 1（O2）＋記録 3 | O1：`planned-state-read-as-actual`（instant を取った後に prune が入る，#63 の窓） |
| Sol（コード） | コードの後 | 3 | 4 | 1：`correlation-by-partial-key`（TX の report を device の channel で引く）．2：`radio-rule-implemented-twice`（from-none の拒否を owner の拒否と別の形で扱う，1 Provider の中の 2 経路）．3：テストの不足（数えない） |
| Sol 再レビュー 1・2・3 | 修正の後 | 3（うち 1 は延期） | 0 | 3 件とも `planned-state-read-as-actual`：owner が覚えている device の状態（TX の `channels`，RX の `opened`，segment の `configured`）を，uhd-control が device を変えた後も当てにする |
| セルフレビュー（Haiku・DeepSeek） | 各修正の回 | P1 は回ごとに直すか退けた（報告に記録） | | |

修正の回は 4（回 1・2 は OpenCode の Haiku，回 3・4 は Claude Code の Haiku サブエージェント）．

## three strikes：owner が覚えている device の状態

再レビューの 3 件は同じ機構である．局所の修正（TX の `verified`，RX の `stale`）を 4 度目に重ねず，1b-4d の最初の手順で設計を直す．Sol 再レビュー 3 の答えを設計メモの骨子にする：

- stream ごとに device が実際に持つものを 1 つの記録にする：開いている channel の集合と，最後に成功して当てた設定（版つき）．`StreamState` に置き，line の lock の下で読み書きする．device の呼び出しの間は lock を持たない．
- どの thread が device を操作するかを，lock の中で先に押さえる（control の from-none が lock を離している間に owner が古い segment を始めないように）．
- open の成功で集合を更新して設定の一致を無効にし，全 channel の configure と read-back が通った後にだけ設定を記録する．途中の失敗は不明として残す．
- cold の `seq` だけでは設定の同一性にならない（timed 更新で投影が変わる）．版で比べるか，変わるときに無効にする．timed command の投入を「適用済み」と記録しない．
- 消すのは写しと印（`verified`，`stale`，owner 側の集合，`configured` の近道）で，実行中の segment の情報（RX の block の解釈，TX の channel 数と ACK の対応）と UR-13 の保存済みの照合の拒否は残す．
- 最初の回帰テストは，再レビュー 3 の系列（B を control が開いて configure → 集合を空にして B を正の長さで終える → C を control が from-none で開く → owner が B を始める）．修正の前の状態を実機の parity の根拠にしない．

## 計画を先に書く手順は割に合ったか（4b に続く 2 回目）

- コードの前に major 7 件を直した（M1 と Sol の計画レビューの 6 件）．
- それでもコードの後に major 4 件（O1，Sol 1・2，と Sol 3 の行の判断），修正の後に 3 件が出た．後の 3 件はどれも，修正の回 1 で入れた from-none の拒否の経路と，HEAD からあった owner の写しが交わるところにあった．計画の列は「owner と control のどちらが device を操作したか」を状態として持っていなかった．
- 段階が大きすぎた（+4,900 行，うちテスト約 3,500）．owner の判断（2026-10-19）：段階は本体とテストで約 1,500 行に分け，テストの繰り返しは helper にする．計画の手順の評価は，分けた 4d の 2 段階で続ける．
