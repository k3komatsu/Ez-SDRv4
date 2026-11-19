# 50 — Phase 8 の残りの仕分けと並行の進め方（2026-11-11，2026-11-18 に §2 を足した）

| 項目 | 内容 |
|---|---|
| 状態 | 進行中の計画．**今の順は §2（段階 4b から Gate X まで，2026-11-18）**．§3 は 2026-11-11 の仕分けの記録（owner は並行の進め方〈2 本の流れ〉と #90 から始めることを承認した．A・B・C の各項目は，owner が見直すまで推奨のまま） |
| handoff | handoff §2 の 1 の「次」がこの表を指す．項目が済んだら行を消すか「済」にし，記録はそれぞれの置き場へ |

## 1. 並行の進め方（owner 2026-11-11）

- **2 本の流れまで．** 3 本以上は orchestrator の確認が粗くなる．
- **流れ 1（中心）**：main の作業ツリー（`~/local_work/Ez-SDRv4`）．`ezsdr-radio-uhd`，試験の rig，変異の道具に触る項目．
- **流れ 2（周辺）**：git worktree（`~/local_work/Ez-SDRv4-n<issue>`，自分の `.cargo/config.toml` で build の出力先を分ける）．`ezsdr-sink-capture`，`ezsdr-mock-radio`，`ezsdr-radio` の時刻の計算など，流れ 1 とファイルが重ならない項目．
- **重い計算は重ねない．** 変異行の全件と gate は 2 台とも空いていれば 2 台で（`sim02:7,sim03:7`：28 コア．各台 4 コアをもう一方の流れの rdev に空ける），どちらかが使用中なら終わるまで待つ（owner 2026-11-12．1 台ずつに分けると両方が約 2 倍遅くなるだけで，計算量の合計は同じ）．負荷に弱い試験が落ちると変異の判定が偽になる（`mech:wall-clock-test-oracle`）ので，二つの重い計算を同時に流さない．
- **Sol は 1 本ずつ**（同時に起動すると止まる）．
- 各流れは自分の作業 branch．main に入れる前に main の最新に載せ直し，gate を流す．
- 設計（読むだけの作業：Fable，Opus）は随時どちらの流れとも並べてよい．
- **owner の答え待ちで止まった流れ**（owner 2026-11-11）：約 30 分答えがなければ，触るファイルが違って安全に並べられる次の項目を新しい流れ（別の worktree）で始めてよい．止まった流れは動いている数に数えない（動いて計算する流れは 2 本まで）．止まった流れの問いを orchestrator が勝手に決めて先へ進めない．安全に分けられるものがなければ待つ．

## 2. 段階 4b から Gate X まで（2026-11-18，orchestrator の推奨．owner に説明済み）

11-18 の bench（[bench/2026-11-18-mt3](../../bench/2026-11-18-mt3/README.md)）までで §3 の A の 1〜8・11・14 と段階 4a は済．残りを二つの流れに分ける．実装役の分類は handoff §5（**危険な段**と**規則の決まった小さな段**．モデルの名前は handoff §5 だけに書く）．設計は AGENTS §9 の step 0（Opus が草案，Fable がレビュー），risky な段の試験計画とコードは Sol がレビューする．

### 流れ 1（main の作業ツリー：`ezsdr-radio-uhd`・`ezsdr-mock-radio`・bench の道具）

同じ crate に触るので順に進める．

| 順 | 段 | 中身 | 難しさ | 分類・レビュー |
|---|---|---|---|---|
| 1 | **4b** | overflow の後の再開の gap（`overflow_restart_gap_ns`，今は推定 140 ms，RM-17）と #108（overflow の直後に予約した受信の再開が遅れる：uhd-rx が gap の間 `rx_recv` に止まる）．queue の深さ 7／8 | 高：Vocabulary の規則，両 Provider，受信のスレッドの並行性 | step 0 の設計 → owner の判断 → 危険な段（試験計画と Sol のレビュー，コードの Sol のレビュー） |
| 2 | 道具の修正 | S4 の 200 MS/s が RS-8a の 5 s に収まらない（capture の後片付け），S5 の 2 channel の pass が走らない（slot A だけの device を開く），C2 の前提（100・200 MS/s では host が遅れを取り戻せず，最後の chunk を期限前に渡せない） | 中：bench の道具だけ | 危険な段の実装役（手作業は規則の決まった段の実装役に出してよい），Opus の小段レビュー．測定の道具なので試験計画は省く |
| 3 | bt2 | C7 の道具：Module の水準で repeat の再開と m の掃引．production に hook が要る．#106 の実機の確認を兼ねる | 中〜高 | 危険な段，Sol |
| 4 | D13 の変換 | record suite の記録を `crates/ezsdr-radio/measurements/` の形に（`observations: []`，実際の `selector.args`） | 低 | 規則の決まった小さな段 |
| 5 | **bench（1 回）** | 4b の値の掃引，両盤の record suite を段階 4 を閉じる commit で（D13），C7，作り直した C2，TXP の B3 | — | 実機．送信の前に owner が att を確かめる |

### 流れ 2（worktree：Kernel と規則の sweep のコード側）

流れ 1 とファイルが重ならない．

| 順 | 段 | 中身 | 難しさ | 分類・レビュー |
|---|---|---|---|---|
| 1 | #105・#104（**済** 2026-11-18，`reviews/n105-*`） | Kernel の小さなずれ（EventSource の unit variant，撤回した規則の一覧，MA-20 の doc），PrepareReport と coercion の非 ASCII の key（SB-9a） | 低 | 規則の決まった小さな段 |
| 2 | #102（**済** 2026-11-18，`reviews/n102-*`．Sonnet が DeepSeek を率いる試し） | 両側で invalid な channel の ChannelGap が map の終わりまで落ちる（SC-31c） | 中：Stream Contract の読み | 危険な段の実装役，Opus のレビュー |
| 3 | #103（**済** 2026-11-18，`reviews/n103-*`） | device-paced の start の batch が受付の lock の外で admit される（KC-24a） | 高：Kernel の並行性 | 危険な段，Sol |
| 4 | #107（**済** 2026-11-19，#109 と一緒に，`reviews/n107-*`） | mt3 C4 の rehearsal の網羅の確認が t0 の位相に頼る（10 回に 2 回落ちる） | 低〜中：時刻に頼る試験 | 危険な段の実装役（小さい） |
| 5 | #75 の残り（**閉じた** 2026-11-19：棚卸しの結果，issue の項目は 79f55e1 と f8c9e2bf ですべて済．残りのコードの項目は無かった） | (3) 以外のコードの項目．着手の前に issue を棚卸しする | 中（INFERRED） | 危険な段，Sol |
| 6 | admission-per-origin の設計メモ（**済** 2026-11-19，`reviews/sb9a-*`．#104 で三振の 4 件目） | canonicaliser を全体にし（UTF-16 の key 順），SB-9a と 12 の入口の検査を撤回（owner D1），EA-5 の `malformed` を削除（owner D2）．報告された値の種類の検査を validate と prepare に（5 件目） | 中：Kernel の入口 | step 0 の設計（Opus 下書き，Fable），Sol |
| 7 | #112・#114（**済** 2026-11-19，`reviews/n112-*`） | Python から対でないサロゲートを送ると Session が終わる（クライアントで送る前に止める，owner）；`Violation.requested` に Module の報告値が入る（7 か所） | 低 | Sonnet が DeepSeek を率いる，Sol |

### 二つの流れの決まり

§1 のとおり：重い計算（gate，変異行の全件）は `simq` で直列，Sol は 1 本ずつ，各流れは自分の作業 branch で main の最新に載せ直してから gate．設計（読むだけ）はどちらの流れとも並べてよいので，4b の step 0 の間に流れ 2 の小さな段を進める．

### 最後（二つの流れが終わってから）

| 段 | 中身 |
|---|---|
| 締めの振り返り | コードとテストの組み立て，mutation の道具の選び方，feature `uhd` の mutation，持ち越し（handoff §2 の 2）．Opus の設計メモ → owner と決める |
| 出口の棚卸し | [00-overview](00-overview.md) §7 の出口条件 4：H1〜H5 と #56 は 10 月と 11-13 の bench に記録があるが，#56 と handoff §3 の行が開いたまま．記録を確かめて閉じる |
| **Gate X** | 出口条件の確認．owner に諮る：**条件 5 の BP4b（混在構成）と BP4c（XG）を Phase 8 に含めるか**（§3 の B は汎用性の点検へ回すことを推奨．XG は FPGA image の書き換えが要る） |

### owner の判断（出てくる順）

1. 4b の設計：overflow の gap の値と #108 の直し方（user に見える値）．
2. Gate X：出口条件 5 の扱い．

### 規模（INFERRED）

実装の段 8〜10，bench 1 回，振り返り．2 本並行で 1 日 1〜2 段として 1.5〜2 週間．延びやすいのは 4b（設計と Fable のレビュー）と #103（Kernel の並行性）．

## 3. 仕分け（2026-11-11）

### A. Phase 8 で必ずやる（推奨の順）

| 順 | 項目 | 流れ | 理由 |
|---|---|---|---|
| 1 | #96（s15 の parity の試験を観測した進みで） | 1 | 済（2026-11-12，記録 `reviews/n96-*`．受付の窓の 14 本は 2 で閉じる） |
| 2 | 試験の時刻の設計とその段（#91 を含む） | 1 | 設計は決定済み（[51](51-test-timing.md)，owner 2026-11-12）．段階：0（表，済）→ M（変異の判定，#91，済 2026-11-12）→ 1a〜1d（済 2026-11-12） → 2a・2b（済 2026-11-13，記録 `reviews/n51-2a-*`，`reviews/n51-2b-*`）→ 2c（済 2026-11-13）→ 3（済 2026-11-14，記録 `reviews/n51-3-*`）→ 1c-ii（済 2026-11-14，記録 `reviews/n51-1cii-*`）．**note 51 は完了**．gate の信頼 |
| 3 | #92（uhd-control の panic で Run が終わらない） | 2 | 済（2026-11-12，記録 `reviews/n92-*`．uhd-control の panic も DEVICE_LOST〈owner〉，KL は足さない〈owner〉） |
| 4 | #90（大きな capture が Manifest から落ちる） | 2 | 済（2026-11-12，記録 `reviews/n90-*`．hash は Kernel の `ContentHasher`〈owner 2026-11-11〉） |
| 5 | #95＋#94（時刻の overflow） | 2 | 済（2026-11-12，記録 `reviews/n95-*`．時計に収まらない送信の時刻と Stop は両 Provider で受付の拒否） |
| 6 | 49 の E2b（Configure と Reopen を操作に） | 2（worktree `Ez-SDRv4-e2b`） | 済（2026-11-12，記録 `reviews/e2b-*`．token は Send・Configure・Reopen の三つ〈owner〉．設定の途中の喪失で予約が消える件は #98） |
| 7 | 49 の E3（区切りの判断を `ezsdr_radio` へ） | 1 | E3a は済（2026-11-12，記録 `reviews/e3a-*`．区切りの算術を `segment.rs` の純粋な関数に，UHD と Mock がそれを呼ぶ，振る舞いは変えない）．E3b も済（2026-11-12，記録 `reviews/e3b-*`．形 S：算術は一つ，受付は記録の予測のまま〈owner〉．ずれ F4 は #99，済 2026-11-13，記録 `reviews/n99-*`：打ち切りは記録を閉じる〈owner：方向 B〉）．文書監査の D2（受信の報告の値）は送信の E3 の範囲の外：Phase 8 の締めの振り返りで確かめる |
| 8 | 49 の C2（KL-42：Stop の後の `applied`） | 1 | 済（2026-11-12，記録 `reviews/c2-*`．結果は境界のサンプルを送ったかで決まる，`uncertain` は Manifest だけ〈owner〉，喪失は装置の時刻で〈owner〉．#100 を立てた） |
| 9 | #75（確かめていない T3 の flush で復旧を終える） | 1 | P2．#91 と同じ機構なので，試験の時刻の設計の答えの後 |
| 10 | #67（Stop で uhd-rx が join の 1 s を越える） | 1 | streamer が漏れる．#56 の実機の後 |
| 11 | 受信の再開の先頭の 0 を FakeDevice と MockRadio に | 2（Mock の側） | 実機に合わせる．小さい |
| 12 | 実機をまとめて（2〜3 回）：#89 の後〈(i) `sleep` を `8dd1016` と `fe86629` で，(ii) `kill` で E9 → A1 を owner が決める．2026-11-12 に済，A1 も済〈`reviews/a1-*`〉〉，(b) T1 の回帰 9 試験，B1，#56，H1〜H5，受信の start lead，段階 4 の値（queue の深さ，throughput），TXP の B3 | — | Phase 8 の目的（測った値で Mock を縛る）の本体．機材が使えるときに順番によらず挟む．送信の前に毎回 att を確かめる |
| 13 | Phase 8 の締めのコードとテストの組み立ての振り返り | — | owner の決定済み（handoff §2 の 2）．`Frozen` の整理の残り，持ち越し，Provider が id を名乗る処理の重複もここで受ける |
| 14 | 保守表 3（実機に遅れて届いた burst） | 2 | 済（2026-11-14，記録 `reviews/mt3-*`）．実機の cell C1〈道具あり〉と C2〜C7〈Module の分け方の遅れ，10・100・200 MS/s，#2，32 sample の先頭，連鎖の境目，2 channel，負荷下の underflow の報告の順，repeat の再開〈m の掃引〉〉は 11-15 の次の bench（O7）．C2〜C6 の道具は済（2026-11-14，記録 `reviews/bt1-*`），C7 の道具は bt2 |

### B. Phase 8 の後へ回す（推奨）

| 項目 | 回す先 |
|---|---|
| 49 の C1（KL-38 の撤回） | 8b の前か凍結の前．Ceiling として書いてあり害がない（2026-11-08 の「やる」を後へ） |
| 49 の C4・C5・C6 | 元から owner が後で承認したときだけ |
| #71（timed な retune が多いと DUC の FIFO があふれる） | 8c（DDC/DUC） |
| ~~#77（Mock と oracle の故障の順）~~ | 済（2026-11-14，記録 `reviews/n77-*`） |
| #73・#74（ファイルなしの capture，`sc16`） | 予定どおり Phase 8 の後・8b の前 |
| #97（EA-13 の `read` が成果物を丸ごとメモリに読む） | #73 と一緒に（owner 2026-11-12）．**注意**：#73 で capture の既定はディスクに書かない（メモリ）に傾けると決めてある（owner 2026-10-30）ので，「ファイルのパスから直接読む」回避は既定では使えない．pipe で返す量の上限か少しずつ流す形を，#73 の設計で一緒に決める |
| ④ Module の形の設計メモ（49 §5.13） | Phase 8 の締めの振り返りに含める |
| 文書監査の D10，spec 22 の手順 5，Test 行の C（82 件） | 凍結の前の全規則の確認 |
| 混在構成（BP4b），XG（BP4c）の実機 | 汎用性の点検（Phase 8 の後）．要確認：Phase 8 の parity にこれらの board が要るか |

### C. やめる候補

| 項目 | 理由 |
|---|---|
| spec 22 の手順 3（spec 11 を 08 へ統合） | 元から取りやめの候補 |
| ~~handoff §3 の「負荷に弱いテスト」の行~~ | 消した（2026-11-14，note 51 の完了で） |

### D. owner の判断だけで済むもの（実装は A の中）

- `overflow_restart_gap_ns` の値（今は `unmeasured`）．
- ~~DDC/DUC に残った設定の扱い~~：届かない（E の仕分け，2026-11-14）．
- ~~feature `uhd` の mutation~~：A13 の変異の道具の選び方で（orchestrator，2026-11-14）．
- 保守表の 3・4．

### E. 要確認



- ~~D9b-1b の残りと実験の規則の取り込み~~ → 仕分け済み（2026-11-14）：(a)〈underflow の後の残りを新しい burst で続ける〉は **owner が取りやめ**（2026-11-14．今のまま burst を捨てる，KL-13）．R1〈#71〉・送信の R2′・D の「DDC/DUC に残った設定」は，note 47 で周波数・gain・antenna が `cold` になり本番が時刻つきの DSP 命令を出さない〈`Device::apply` は即時，`tune_dsp`/`tune_auto` は bench の probe だけ〉ので届かない：外す（orchestrator，#71 は閉じた）．実験の規則の取り込み〈45-report-e4 の候補〉と feature `uhd` の mutation は A13〈締めの振り返り〉で（orchestrator）．

## 4. 規模の見当（2026-11-11，INFERRED）

A は実装の段が 15 前後と実機 2〜3 回．2026-11-11 の速さ（1 日に 1〜2 段）で 1〜2 週間．49 の段と試験の時刻の段が延びる見込みがある．
