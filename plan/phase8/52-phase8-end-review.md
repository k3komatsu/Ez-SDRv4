# 52. Phase 8 の締め — コードとテストの組み立ての振り返り

| 項目 | 内容 |
|---|---|
| 状態 | **決定済み**（owner 2026-11-20，D1〜D7 すべて推奨どおり．D3 は追記つき．§4）．下書きは Opus（effort max） |
| 頼み | owner 2026-10-31 と 2026-10-19，`plan/phase8/50-remaining-work.md` §2「最後」，handoff §2 の 2．まず過去の段階の増え方を測り，それから比べて諮る |
| 根拠 | main `4679abc1`（worktree `~/local_work/Ez-SDRv4-n111`．origin/main と同じ）．bt2 は main に無い（branch `work/bt2`，記録は `~/.cache/ezsdr-fixes/2026-11-19-bt2/`）．owner の issue の線引き（2026-11-19：出口条件〈parity，実機の確認，gate の信頼〉と P1・P2 だけを Phase 8 で直し，P3 は 8b の先頭へ）は，まだ着地していない `b84acf11` の plan 50 にだけ書いてある |
| 測定 | [52-measure/](52-measure/)．`commands.sh` が全部を再実行する（付録 A）．段階ごとの全表は `52-measure/steps.md` |
| 印 | **VERIFIED**：この下書きのために走らせた命令か，読んだファイルで確かめた．**INFERRED**：推定（理由を書く） |

## 0. 結論

一つずつの推奨（理由と退けた案は §2，owner の判断は §3）．

- **a. mutation**：手書きの行は今のまま段階ごとの gate に残す．生成器（cargo-mutants）は gate にせず，凍結の前に production のコードを一度見渡す道具として使う．道具は 8b の前の小さな試し（owner 2026-11-20：bench 待ちの間に，Gate X の前へ前倒し）（`ezsdr-radio` と `uhd.rs`）で決める．見渡しは sim の container の中で，道具自身の写しの上でだけ流す（AGENTS の決まりに一文足す）．feature `uhd` は，mutation の image を `ezsdr-v4-dev:uhd4.10` に載せ替えて gate に入れる．
- **b. テストの組み立て**：作り直さない．今直すのは，gate の信頼にかかる一つ（kc_36 の TTL の 2 本）だけにする．`Direct` の `Drop` は 8b の掃除で直す．G10・G29 の killer は n51-3 の決定 E のまま残す．8b の rig は複数台の一つを最初から作る（fake の故障を stream の開始からの相対にする）．残りは捨てる．
- **c. レビュー**：Fable（設計），Sol（試験計画と最後のコード），major の直しの後の Opus の再確認は残す．変えるのは二つで，どちらも最近の運用を決まりにするだけである．紙の往復は Sol の 1 回と狭い再確認 1 回までにする．危険な段の中でも，振る舞いを変えない小段は Terra だけにする．
- **d. 負債**：36 件．済・閉じる 10，今 1，owner の判断 2（a・e），8b 7，凍結前の確認 3，後の phase 2，捨てる 11（§2d の表）．
- **e.** ④ Module の形は単独のメモにしない．8b の step 0 の最初の問いにする．stat `tx_close_past_record_front` は消す（Manifest の key なので owner の判断）．

## 1. 測定

### 1.1 数え方

- 範囲：`git rev-list --first-parent ec95e4b2..4679abc1`．Phase 7 の Gate X の merge（2026-10-01）から今の main まで，472 commit（VERIFIED）．
- 段階：commit が足した `plan/phase8/reviews/<段階>-dispositions*`・`-gate.json` の名前．それが無ければ他の記録の頭，それも無ければ件名で一行．
- 期間：**P0** は 10-01〜10-16（Phase 8 の前の保守）．**P8a** は 10-17〜11-06（一つの段階が複数の commit で入ることがあった）．**P8b** は 11-07〜11-20（段階ごとに squash した一つの commit．AGENTS §9）．
- 行の分類（`52-measure/churn.py`．各側の行は，その側の版のファイルでの位置で分ける．tx.rs・device.rs など 10 ファイルで境目を手で確かめた）：
  - `prod`：本体．
  - `uhdffi`：`uhd.rs`（feature `uhd`）．
  - `fake`：device.rs の FakeDevice（`pub enum FakeFault` から最初の `#[cfg(test)]` まで）と，feature `testing` の item．
  - `utest`：`#[cfg(test)]` の item と cfg(test) だけのファイル．
  - `itest`：`tests/`（`common/` を除く）．
  - `icommon`：`tests/common/`（rig，helper，bench の道具）と acceptance の `src`．
- 除いたもの：
  - `245dbf0f`（10-31 の木全体の rustfmt．テストで ±34k 行）．
  - `brain/`（設計メモ 45 の実験．足して消した．ただし mutation の行の +408／−405 は P8a の行の数に入っている）．
- 書式に依らない大きさ：空白を除いた文字数（kchar）．10-31 の rustfmt（`max_width 200`）で行数が変わったからである．たとえば Kernel は 18,116 行 → 15,364 行に減ったが，中身は 516 → 535 kchar に増えた（VERIFIED，`52-measure/totals-*.md`）．

### 1.2 今の大きさ（`4679abc1`，行．VERIFIED，`52-measure/totals-4679abc1.md`）

| crate | 本体 | FFI | fake | 内のテスト | `tests/` | `tests/common` | mutation の行 |
|---|---|---|---|---|---|---|---|
| ezsdr-radio-uhd | 7,115 | 1,176 | 1,922 | 30,955 | 15,342 | 24,050 | 1,855 |
| ezsdr-kernel | 15,364 | — | 641 | 444 | 25,436 | — | 423 |
| ezsdr-radio | 5,600 | — | — | 53 | 7,228 | — | 438 |
| ezsdr-mock-radio | 2,985 | — | — | 153 | 9,532 | — | 233 |
| ezsdr-acceptance | — | — | — | — | 8,542 | 812 | 101 |
| ezsdr-server | 1,735 | — | 25 | 50 | 2,576 | — | 90 |
| python（`ezsdr`／`tests`） | 1,214 | — | — | — | 1,996 | — | 121 |
| 残り 7 crate | 2,028 | — | — | 38 | 3,221 | — | 72 |
| **計** | **34,827**（＋py 1,214） | 1,176 | 2,588 | 31,693 | 71,877（＋py 1,996） | 24,862 | **3,333**（JSON 33,480 行） |

書式に依らない大きさの推移（kchar．VERIFIED，`52-measure/totals-{ec95e4b2,03b8f94d,4679abc1}.md`）：

| | Gate X（10-01） | Phase 8 の始め（10-17） | 今（11-20） |
|---|---|---|---|
| 全体：本体（＋FFI＋py） | 852 | 981 | 1,310 |
| 全体：テスト | 1,255 | 1,830 | 5,923 |
| 全体：テスト ÷ 本体 | 1.5 | 1.9 | **4.5** |
| UHD：本体＋FFI | 134 | 183 | 296 |
| UHD：fake | 25 | 26 | 64 |
| UHD：テスト | 195 | 413 | 3,214 |
| UHD：テスト ÷ 本体 | 1.4 | 2.3 | **10.8** |
| `ezsdr-radio`（純粋な規則）：テスト ÷ 本体 | 0.6 | 0.8 | 1.6 |

テストの関数の数（VERIFIED，`52-measure/test-counts.txt`）：Rust の `#[test]` は 922（10-01）→ 1,192（10-17）→ 2,240（10-31）→ 1,771（11-05．note 47 段 2 で消した）→ 3,293（11-20）．このうち `#[ignore]`（ほぼ実機のテスト）は 33 → 85．Python は 23 → 123．n110 の gate では `cargo test --workspace` が 3,218 本を通した．遅い binary は parity 101.8 s（73 本），`fake.rs` 71.7 s（725 本），UHD の lib 12.8 s（842 本）（VERIFIED，`~/.cache/ezsdr-fixes/gates/n110/test.log`）．

UHD のテストの置き場（VERIFIED）：
- 段階の名前の付いたファイル：`src/provider/tx/tests/{c2,e1,e2a,e2b,e3,e3b,mt3,mt3d,n75,n99,o1}.rs` で 8,766 行．
- `tests/fake.rs` 10,692 行，`hardware.rs` 4,270 行（実機のテスト 85 本）．
- bench の道具（`tests/common` の 17 module と `measurement/`）：約 21,800 行，mutation の行 784 本．bench の日ごとに一つずつ足してきた（`iii`，`bench1115`，`s4bench`，`s9bench`，`mt3*`，`n47*` …）．
  - 16 個は，runbook（phase7・phase8 の `bench.md`）が名指す実機のテストが今も使っている（`52-measure/bench-tools.txt`）．
  - 残る `raw`（233 行）は raw の probe 用である．
  - 実機のテスト 85 本のうち 16 本は，どちらの runbook にもテストの名前で出てこない．そのうち 4 本は design が証拠に引いている．

### 1.3 段階ごとの増え方（VERIFIED，`52-measure/steps.md`，`52-measure/crate-churn.txt`）

| 期間 | 段階（＋名前の無い commit） | 本体 +/− | fake +/− | テスト +/− | テスト ÷ 本体（足し引きの和） | 段階の中央値* | mutation の行 +/−/変 | テスト関数 +/−/本体を変えた |
|---|---|---|---|---|---|---|---|---|
| P0 | 90 | 8,814 / 5,820 | 182 / 27 | 19,559 / 6,712 | 1.8 | 1.4 | 430 / 38 / 220 | 347 / 70 / 1,282 |
| P8a | 102 | 13,974 / 9,063 | 1,446 / 719 | 68,310 / 35,340 | 4.5 | 3.2 | 1,942 / 970 / 919 | 1,964 / 1,195 / 1,763 |
| P8b | 60 | 6,945 / 3,059 | 706 / 147 | 60,588 / 8,609 | **6.9** | **4.5** | 1,753 / 160 / 580 | 1,563 / 146 / 1,571 |

\* 本体を 50 行以上変えた段階だけ．P8a の「行」には brain の +408／−405 が入っている．

- UHD だけで見ると，テスト ÷ 本体（足し引きの和）は P0 2.1 → P8a 6.6 → P8b **10.7**．P8b の UHD のテストの増分は，内のテスト +20,619，`tests/` +4,950，`tests/common` +14,371 行．同じ P8b で `ezsdr-radio` は 1.9，Mock は 4.0，Kernel は 4.9 だった．
- テストを一番足した段階：4b（+6,087），note 51 の 10 段（合計 +6,318），47s5（+4,038），1b-4c（+3,951），mt3（+3,044），bench1115（+2,754），o1（+2,424）．
- 本体を変えずにテストを 500 行以上足した段階は 15 ある（bench の道具，計測，note 37）．計 +23,779 行，行 +563．
- 本体の行 1 行あたりに足した mutation の行は，P8b で 0.25（1,753 ÷ 6,945）．既存の行を 3 本足すごとに約 1 本を当て直した（P8b で変えた 580 本のうち，anchor の変更が 468 本）．テストの改名は 660 本（`*-renames.txt`）で，うち 47s2 が 502 本．

### 1.4 mutation の行（VERIFIED）

- 本数：2,007（10-31）→ 3,333（11-19）．gate の mutation の段の時間は 1,268 s → 1,431 s（+13%）．47s2 の後は 1,672 本・722 s だった（`52-measure/gate-times.tsv`．どれも sim02:8＋sim03:8）．
  - **時間は行の数に比例して伸びていない**（行は +66%）．1 行あたりは平均で約 7 shard 秒（1,431 s × 16 ÷ 3,333）．shard が一度 build した後は行ごとの build が小さいからだと見ている（INFERRED．行ごとの時間は log に無い）．
  - テストの段は 141 s → 240 s．
  - 10-31 から 11-19 までの `gate.json` は 83 個（記録した gate の数で，走らせた回数はこれ以上）．1 日に約 4 個．
- 行が何を変えるか（`52-measure/rows-by-target-4679abc1.txt`．変える行の分類で数えた）：
  - production の Rust：2,114（63%．anchor を一行に定められなかった 61 本を含む）．
  - Python の本体：120．
  - FakeDevice と `testing`：152．
  - テスト側のコード：946（28%）．そのうち bench の道具（§1.2 の 17 module と `measurement/`）が 784．
  - UHD の crate だけで見ると，本体 837 に対して道具・テスト 873，fake 145 で，**本体より道具を変える行の方が多い**．
- feature `uhd`：`uhd.rs` を変える行は 0 本．
  - `uhd.rs` を通る実行のテストは 20 本ある．`uhd.rs` の中の unit test 15 本（C の関数を差し替えて試す）と，`tests/uhd_api.rs` の 5 本（libuhd に対して）．gate の `uhd-test` の段が `ezsdr-v4-dev:uhd4.10` の中で流している（`gate.py:334,358`．lib は feature なしで 842 本，ありで 857 本）．
  - mutation の image は `rust:1-bookworm` で libuhd が無い．`mutate.py` は feature を渡さない（`mutate.py:141`，`remote/Dockerfile`）．

### 1.5 レビュー（VERIFIED，`52-measure/review-counts.tsv`，`52-measure/plan-rounds.txt`）

対象は，dispositions に標準の数え表（Source・Major・Minor）がある 47 段階・191 行（下の表は，gate・owner・orchestrator の 5 行を除いて数えた）．P8b のほぼ全部と 48s2 である．P8a の多くは `*-opus.md` だけで表が無いので，失敗機構の段落（`failure-mechanisms.md`）でしか見られない．

| 種類 | 担当 | 回数 | major のあった回 | major | minor |
|---|---|---|---|---|---|
| 試験計画 | Sol | 30 | 26 | 89 | 82 |
| 試験計画の再確認 | Opus（ほかに Opus＋Terra 1 回，数なし） | 16 | 12 | 21 | 58 |
| 小段 | Opus | 27 | 10 | 17 | 110 |
| 小段 | Terra | 4 | 2 | 3 | 4 |
| 小段 | Haiku／Sonnet | 3 | 2 | 4 | 6 |
| 最後のコード | Sol（＋#102 の Opus 1 回） | 51 | 33 | 68 | 79 |
| 直しの後の再確認 | Opus | 37 | 3 | 3 | 76 |
| 直しの後の再確認 | Opus＋Terra（4b） | 2 | 1 | 3 | 5 |
| 直しの後の再確認 | Terra | 6 | 3 | 5 | 7 |
| 設計 | Fable | 9 | 5 | 14 | 15 |

担当は Source の欄の頭（最初の読点の前）で決めた．たとえば「Opus, re-check of Sol's fixes」は Opus である．

**この表は，直しの後の再確認の当たりを小さく見せている．** 標準の表の無い段階（47s4，47s5，48s1，48s3，o1，s4a，stepB，n0，n106，stepC）を手で数えると，直しの後の Opus の再確認は約 16 回で，major は約 17 だった（47s5 の 3 回の FAIL で 6，s4a 4，48s3 3，o1 2，48s1 1，stepB 1）．INFERRED：表の無い記録を手で数えた．D′ の前後の大きな段階では，再確認が major をよく見つけている．

- **小段と最後のレビュー**：小段のレビューと最後のコードのレビューの両方がある段階は 12．小段で 24 の major が見つかり，最後のレビューはさらに 17 を見つけた．12 段階のうち 8 段階で，最後のレビューが major を出した．47s5 の振り返り（owner 2026-11-07）も同じことを書いている：安いレビュー 10 回で 57 件（本当の欠陥 5），Sol は部品をまたぐ順序 S1〜S4 を見つけ，それはどの安いレビューも捕まえていなかった．
- **紙の上の往復**：
  - 試験計画を 2 回以上レビューした 14 段階のうち 13 段階で，2 回目にも major が出た（n111 は表に数が無く，記録の文から数えた）．
  - n51-2c は 6 回目でやっと 0 になった（5，4，Fable 4，2，2，0）．
  - **bt2 は 3 回で収まらなかった**：
    - major は 7 → 4 → 8．計画は 48 → 64 → 76 KB に膨らんだ．
    - 指示書（20:18）から最初の小段の指示書（22:06）まで約 1 時間 50 分かかった．時刻は VERIFIED（ファイルの mtime）だが，それが全部紙の往復にかかった時間というのは INFERRED．
    - orchestrator は「plan v4 は作らない」と決めた（`decisions.md:46`）．その後の小段 B1〜B3 でも Terra は major を 1，3，4 件，Opus は 0，0，2 件出した．
- **費用**（記録にある値．VERIFIED．金額はどこにも記録が無い）：

  | 担当 | 1 回の時間 | token | 5 時間枠 |
  |---|---|---|---|
  | Sol の試験計画 | 8〜21 分 | 259 k（4b） | — |
  | Opus の再確認 | 3.6〜16.5 分 | 約 250 k | — |
  | Terra | 0.8〜1.8 分 | 76〜97 k | 2〜3% |
  | Fable | 約 20 分 | 231 k | — |

  段階の経過時間は `~/.cache/ezsdr-fixes/<段階>/` の mtime で見ると，危険な段階が 7〜25 時間（4b 24.7，mt3 14.5，e2a 13.1），小さい段階が 1〜3 時間（`52-measure/step-spans.txt`．INFERRED：段階の後に触ったファイルも入る）．

### 1.6 繰り返した機構（VERIFIED，`52-measure/issues-2026-11-20.json.txt`，`52-measure/mechanisms-phase8.txt`）

10-17 からの issue（#67〜#121）は 55 件．そのうち `mech:` のラベルが付いたのは 34 件．

| 機構 | 件数 | issue |
|---|---|---|
| **`wall-clock-test-oracle`** | **8（最多）** | #68，#69，#76，#83，#87，#96，#107，#109 |
| `planned-state-read-as-actual` | 6 | — |
| `supplied-callback-under-lock` | 4 | 11-19 に新しく立てた |
| `radio-rule-implemented-twice` | 4 | — |
| `stream-lifecycle-scattered` | 3 | — |
| `failure-collapsed-into-value` | 3 | うち #91 は `mutate.py` |
| 残りの機構 | 6 | — |

- gate の道具の issue は #91，#107，#109 の 3 件．
- テストと道具の側の issue は，合わせて 55 件中 9 件（16%）．
- review の表（Mechanism の欄）でも，一番多く名指された機構は `wall-clock-test-oracle` だった：17 行で，それらの行の major の計は 26（a1，e3a，e3b，n51 の各段．`52-measure/mech-findings.txt`．行の major には別の機構のものも混じる）．次は `planned-state-read-as-actual` の 7 行（c2，e3b，mt3，n99）．
- ledger の段落では，`planned-state-read-as-actual` と `radio-rule-implemented-twice` が P8a の段階ごとに何度も出た．どちらも設計メモ（49 の D′，22・36・38・40）で答えたあとは，その場の当てはめとして数えている．

### 1.7 数が示すことと示さないこと

示すこと：
- **テストの量が production の何倍の速さで増えたか．** UHD の P8b は約 11 倍だった．
- **増えた所．** 内のテスト，bench の道具，段階の名前の付いたファイルである．
- **mutation の費用は時間より手間に出ている．** 当て直しが新しい行の 3 本に 1 本，改名が 660 本あった．
- **レビューの種類ごとの当たりの率．**
- **紙の往復が収まらない例がある**（bt2，n51-2c）．

示さないこと：
- **テストの質や冗長さ．** 二つのテストが同じことを確かめているかどうかは，行数では分からない．
- **mutation の行一本あたりの値打ち．** 行が欠陥を防いだ数は，どこにも記録されていない．
- **「本体を変えた関数」の数え方の限界．** 空白を除いた本体の比較なので，helper の改名のような機械的な書き換えも数える．名前が変わり中身も変わったテストは，消したものと足したものに分かれる（改名として数えられたのは 6 本だけ）．
- **期間の線引きの限界．** P8a には一つの段階が複数の commit に分かれたものがある．
- **review の major の重さ．** 採らなかった major や，文面の直しで閉じた major も入っている（n120 の Terra など）．
- **お金の費用．** 記録が無い．

## 2. 問いごとの推奨

### 2a. 手書きの mutation の行と生成器

**cargo-mutants について分かっていること**（2026-11-20 に本 <https://mutants.rs/print.html> を読んだ．**この workspace では試していない**．ここに入れておらず，AGENTS §7・§9 は mutation を `remote_mutate.py` だけで流すと決めているため）：

できること（VERIFIED，docs）：
- 作る変異は次の 5 種類．
  - 関数の本体を型に合う値に置き換える．
  - 二項演算子を置き換える．
  - 単項演算子を消す．
  - wildcard のある match の腕を消す．
  - base のある struct literal の欄を消す．
- 変異ごとに，**変えた package のテストを全部**流す（`--test-workspace`／`--test-package` で広げられる）．**変異ごとにテストを選ぶ手段は無い**．`--` の後ろはすべての `cargo test` に渡る．
- `#[cfg(test)]` とテストの関数は変異させない．`#[mutants::skip]` と `.cargo/mutants.toml` の `exclude_re` で外せる．
- `--in-diff`，`--shard k/n`，`--features`，`--list`，nextest に対応している．timeout の既定は baseline の 5 倍で，下限は 20 s．

この workspace に当てはめると（INFERRED，上の docs と n110 の gate の数から）：
- **遅い．** UHD の変異 1 つで UHD の package のテストが約 85 s（lib 12.8 s＋`fake.rs` 71.7 s）と build がかかる．今の行は平均約 7 s である．
- **見逃す．** parity（ezsdr-acceptance，101.8 s）は別の package なので，parity だけが殺す変異は「生き残り」と報告される．
- **偽って殺す．** 実時間のテスト（`kill: bound` と宣言した 27 行の killer と，実時間が前提の fake のテスト 28 本）が shard の負荷で落ちると「殺した」と数える．#91 と同じ失敗の形で，`-- --skip` で外す必要がある．
- **作れない変異がある．** lock の範囲や順序の変異は作れない（例：G29「FreezeDispatch が admission の lock を取らない」）．

**推奨（役割の分担）．**

1. **手書きの行は，段階ごとの gate のまま残す．** 対象は，指示書と試験計画が名指すもの：規則の条件，拒否，lock の範囲と順序，bench の判定の枝．killer を名指すので速く，計画と行の対応をレビューが確かめられる．
2. **生成器は gate にしない．** v4.0 の凍結の前に一度（望むなら phase の終わりごとにも），production のコードを見渡すために流す．生き残った変異は三つに仕分けて記録する：テストを足す，等価として理由つきで `exclude_re` に入れる，受け入れる．段階は止めない．
   - **Safety core との関係**：見渡しは sim02・sim03 の container の中で，道具が作る写しの上でだけ流す．cargo-mutants は既定で写しの上で build し試す（1.0.2 から．target も写さない．25.1.0）．木そのものを変える `--in-place` は使わない（docs の changelog，VERIFIED）．手元では流さない．
   - 今の決まり（「mutation の行は `remote_mutate.py` だけで流す．手元で `mutate.py` を流さない．木に mutant を当てない」）は行のことしか書いていない．Safety core と §7 に「生成器の見渡しも同じ条件で，sim の container の中で写しの上だけ」と一文足す．これも D1 に含める．
3. **道具は試してから決める．** Gate X の後・8b の最初の段の前に，次の小さな試しをする．
   - 対象は `ezsdr-radio`（純粋な規則で，テストが数秒）と，`ezsdr-v4-dev:uhd4.10` の中の `uhd.rs`（`--features uhd`）の二つ．
   - まず `--list`（数秒）で変異の数を見てから流す．
   - 測るもの：変異の数，build できない数，生き残りの数，変異 1 つあたりの時間，実時間による偽の kill，今の行と同じ file:line に当たる割合．
   - 採る条件：workspace 全体を sim02＋sim03 で一晩以内に見渡せ，生き残りを仕分けきれること．そうでなければ生成器は使わず，凍結のチェックリストに「生成器なし」と書く．
4. **feature `uhd` を gate に入れる．** mutation の image を `ezsdr-v4-dev:uhd4.10` の上に作り直す（Rust stable，Python 3＋numpy，libuhd を持っている．VERIFIED `docker/uhd4.10/Dockerfile`）．行が `"features": ["uhd"]` を持てるようにし，`mutate.py` が `--features` を付ける．どの規則に行が要るかは，試しの `uhd.rs` の結果で決める．それまでは，今の 20 本と実機が `uhd.rs` を見る．

**退けた案．**
- **生成器を gate にして，手書きの行を置き換える．** 変異 1 つあたり約 10 倍遅い．別の package の killer を見逃し，実時間のテストで偽って殺す．並行性の変異も作れない．レビューが確かめている「計画の系列ごとに行がある」が失われる．
- **今のまま，見渡しもしない．** 誰も計画しなかった production の道が，テストされないまま凍結される．owner は凍結の前に道具を決めると決めている．
- **bench の道具の行（784 本）を別のリストにし，bench の前と道具を変えたときだけ流す．** 1 回の gate で約 5 分の節約（INFERRED：時間が行の数に比例すると仮定）．代わりにリストの決まりが一つ増える．標本 14 本のうち 12 本は判定・測定・設定の枝で，間違えば bench の結論を誤らせるものだった（出力の有無は 2 本）．今は割に合わない．
- **他の生成器．** 調べていない．

**後で不可能になること**：無い．見渡しは，道具の版を image に一つ固定するだけで，やめるのも自由である．行を gate に残すかぎり当て直しの手間（新しい行 3 本に 1 本）は続く．それが killer を名指す代価である．

**費用**（INFERRED）：小さな道具の段一つ．image を作り直し `features` の欄を足す（約 20 行）．試しは `--list` で範囲を決めてから流すので，sim の時間は 1 時間以内の見込み．生き残りの仕分けに実装役の半日．レビューは Terra（道具で危険でない）．

### 2b. テストの準備，rig，時刻に頼るテスト

**今の形**（VERIFIED）：
- UHD のテスト ÷ 本体は 10.8（kchar）．純粋な規則（`ezsdr-radio`）は 1.6 である．判断を純粋な関数にして（segment，staging の book，timeline）表で確かめる方向は効いている．UHD のテストの多くは，スレッドと FakeDevice の rig を通る．
- rig を作る関数は，UHD の内のテスト，Mock，acceptance に**約 40**（`52-measure/rig-builders.txt`．78 の名前からテストの名前を手で除いた）．
- `FakeConfig { … }` の literal は 368 か所（`fake.rs` 147，tx.rs 59，rx.rs 42 …）．`common::fake()` は `fake.rs` で 172 回．
- `Direct`（`tests/fake.rs:1376`，52 回使う）と `UhdRadio` に `Drop` が無い．
- 時刻に頼るものの残り：
  - `kill: bound` と宣言した実時間の行が 27（n110 の gate）．
  - 実時間が前提の fake のテストが 28（n51-3）．`n47s5_front_rehearsal_on_the_fake` はその一つで，負荷のある PC で落ちるのは設計どおりである．
  - **G10・G29 の killer**（`device_paced.rs` の `kg_03_a_fatal_event…`・`kg_04_no_action_is_dispatched_after_the_freeze`）．
    - G10 の killer は「radio:stop」を 10 s の実時間まで待つ．
    - G29 の killer は，stop の後 50 ms の drain の窓に，freeze の後の dispatch が現れないことを見る．
    - どちらも行は `kill` を宣言していない．これは n51-3 の決定 E（Kernel の行の期限切れは普通の kill．Kernel は `EZSDR-INFRA` の印を書かない．`n51-3-test-plan.md` §4.2・§4.6）による．
    - この形では，負荷で起きうるのは「生き残り」の報告（gate が止まる）であって，偽の kill ではない．変異の無い木は 10 s の 5〜25 分の 1 で通る（同 §4.2）．
  - kc_36 の TTL の 2 本．
  - #107 の根の原因：FakeDevice の手動の時計が零を host の時計から取る．
- 上の `wall-clock-test-oracle` は，Phase 8 の issue で一番多い機構である（§1.6）．

**推奨：作り直さない．gate の信頼にかかる一つだけ今直し，8b の rig は最初から一つにする．**

- **今**（テストのコードだけの小さな段．DeepSeek が直接書き，Terra がレビュー．owner の線引きの「gate の信頼」に当たる）：kc_36 の TTL の 2 本を手動の HostClock に移す．`Lease::detached` は HostClock を取る．今は負荷で落ちうるので（n51-3），gate を偽って止めうる．
- **8b の最初の掃除**：
  - `Direct` に `Drop` を付け，Run を終える．panic の途中でも stop と join をし，誤りは捨てる．
  - 今は `finish` の前の panic でスレッドが残り，同じ binary の後のテストを巻き込みうる（INFERRED）．ただし，それはテストがもう落ちたときだけの話なので P3 で，owner の線引きでは 8b に回す．
- **残す**：G10・G29 の killer は決定 E のままにする．変える理由になる系列（変異の無い木で偽の kill が出る順序）を，私は見つけられなかった．
- **8b の rig**：複数台の rig を一つだけ作り，8b のテストはすべてそれを使う．二つの FakeDevice に一つの手動の時刻を与え，故障は stream の開始からの相対で書く．これで #107 の根と 4b の n2 にも答えられる．今の約 40 の rig は一つにまとめない．
- **指示書の決まり**（AGENTS は変えない．Maintainability の 3 のとおり）：
  - 新しい判断は，まず `ezsdr_radio` に表のテストを足す．rig のテストは配線の道ごとに 1 本．
  - rig を作る関数を足すときは，報告に理由を書く．
- **捨てる**：`FakeConfig` の literal，`UhdRadio` の `Drop`，段階の名前の付いたテストのファイル名，HeaderTap（§2d）．

**退けた案．**
- **テストを規則ごとに並べ直し，rig をまとめる．** 数百本のテストと filter が動く．47s2 の改名 502 本がその代価を示している．直す欠陥が無い．agent はテストの名前の規則の ID で探せる．
- **実時間の rehearsal をすべて手動の時計へ．** 実時間で走る実機のテストの rehearsal なので，手動にすると rehearsal の中身が変わる（n51-3 の決定）．
- **古い bench の道具を消す．** 17 個のうち 16 個は runbook の実機のテストが使っており，実機の回帰の suite そのものである．runbook に無い実機のテストは §2d の #32 で扱う．

**後で不可能になること**：無い．rig をまとめる道は残る．**費用**：今の段が 1〜2 時間（INFERRED）．8b の rig は 8b の仕事の中に入る．

### 2c. レビューの費用と成果，8b に残すもの

**推奨．**

残すもの：
- step 0 の設計：Fable．9 回で major 14．
- 危険な段の試験計画：Sol．30 回中 26 回で major，計 89．力の無いテストは紙の上で直すのが一番安い．
- 最後のコード：Sol．すべての段（bench の道具を含む）．小段のレビューの後でも，12 段階中 8 で major 17 を出した．bench の道具でも，bench1115 7，s4bench 4，tools 2 の major を出した．
- 直しの後の再確認：Opus，必要なら Terra と並べる．今の手順 8 のまま（「After a fix for a major, the re-check reviewer re-reviews the fix」）．標準の表では 37 回で major 3 だが，表の無い大きな段階では約 16 回で約 17 を見つけた（§1.5）．

今の決まり（main の AGENTS.md §9．VERIFIED）：
- 手順 3：「Sol reviews the test plan … The orchestrator adjudicates and the plan is amended.」紙の再確認は決まりではない．
- Re-check reviewer：「runs narrow re-checks the orchestrator asks for, such as whether a corrected test plan closes the findings」．
- Second reviewer（Terra，owner 2026-11-19）：「Runs beside the Opus re-check reviewer … on every sub-step review and re-check of a risky step …; alone, it re-checks a small or text-only fix」．

実際には，標準の表のある 14 段階で，orchestrator が直した計画の紙の再確認を頼んだ．そのうち 5 段階（mt3，n51-2c，n75，4b，n111）と，main に無い bt2 で 3 回以上になった．

変えるもの（どちらも最近の運用を決まりにするだけ）：
1. **紙の往復は「Sol の 1 回＋狭い再確認 1 回」まで．** 3 回目は出さない．
   - 残った指摘は，先に落ちる形で書いたテストと，最初の小段のレビューがコードと突き合わせて確かめる．
   - 再確認がまだ設計の問いを出すなら（テストではなく振る舞いの決め方），計画を回し続けず step 0 に戻す．
   - 根拠：2 回目は 14 段階中 13 で major を出し，役に立っている．3 回目以降は収まらない例がある：bt2 は 3 回目で 8（7 → 4 → 8，計画は 48 → 76 KB），n51-2c は 6 回目でやっと 0．orchestrator は bt2 で「plan v4 は作らない」と決め（`decisions.md:46`），n120 は v2 から再確認なしでコードへ進んで同じ日に着地した．
   - 弱いところ（INFERRED）：3 回目の major も本物でありうる．それを紙の上より安くコードの側で捕まえられるかは，数では示せていない．AGENTS §9 の書き方にならい，危険な段階を 2〜3 回終えたら記録で見直す．
   - 文の変更：Re-check reviewer の「such as whether a corrected test plan closes the findings」に「at most once per plan」を足す．手順 3 に「a corrected plan is re-checked at most once; what remains goes to failing-first tests and the first sub-step review, or back to step 0 if it is a design question」を足す．
2. **危険な段の中でも，振る舞いを変えない小段（機械的な置き換え）は Terra だけ．**
   - n120 の B1 はこうした．
   - 並べた記録（4b，n111，bt2 の B1〜B3，n120）では，Opus の見落とした major を Terra が毎回出した（n120 の 2 件は文面の直しで閉じた）．n115 の再確認ではどちらも major を出さなかった．Opus は具体的な直し方と，正しいコードで落ちるテストを見つけた．並べる価値は振る舞いの変わる小段にある．
   - Terra は 1〜2 分・5 時間枠の 2〜3%．Opus は 4〜17 分で，token は n111 の 1 回で約 250 k．
   - 文の変更：Second reviewer の「on every sub-step review」に「that changes behaviour」を足す．

**退けた案．**
- **試験計画のレビューをやめる．** 89 の major は，紙の上で直すのが一番安かった．
- **最後の Sol のレビューをやめる．** 小段の後でも 8/12 段階で major が出た．
- **小段のレビューをやめる．** 早い段で 24 の major を出し，47s5 では実装と並んで本当の欠陥 5 を見つけた．
- **直しの後の再確認を減らす．** 表の無い大きな段階で約 17 の major を見つけた．
- **Fable をコードのレビューにも使う．** stepF の比較では，Fable 約 20 分で minor 5，Sol 約 5 分で test の観察の穴 3，重なりは 1 件だった．Fable は設計だけにする．

**後で不可能になること**：三度目の紙読みでしか見えない計画の誤りは，そのままコードへ進む．それは小段か Sol のコードのレビューが捕まえることになる．bt2 ではそうなった（INFERRED）．

**費用**：コードの変更は無い．AGENTS §9 の 2 か所に句を足す．往復が長引く危険な段階で 1〜2 時間減る見込み（INFERRED：bt2 と n51-2c から）．

### 2d. 集めた負債の仕分け

出典：`git grep -n -i "phase 8-end" origin/main -- plan`（日本語の言い方も含めた全件は `52-measure/debts-grep.txt`），handoff §2 の 2 の持ち越し，plan 50 §3 の A7・A13・E．状態はどれも `4679abc1` で確かめた（VERIFIED／INFERRED を付けた）．

| # | 項目（出典） | `4679abc1` での状態 | 仕分け | 理由 |
|---|---|---|---|---|
| 1 | feature `uhd` の mutation（1b4a-fix-reports §9，handoff §3） | 行 0 本，テスト 20 本（VERIFIED） | §2a の判断 D2 | — |
| 2 | 0 channel の `UhdDevice` の道の pin（同 §9） | channel の組を Run の中で変えなくなった（note 47）．`tx_open(0,&[])` に届く道は無い見込み（INFERRED） | 閉じる | 届かない |
| 3 | 同 §9 の残り：表の無い `ur_33`，採らなかった P3，D16 の antenna の読み戻し，`Scripted`，BP0 の項目 | spec 18 は 10-15 に DA-1 形式へ．値は `cold`（note 47・48）．BP0・BP5 は 10-20 に PASS（VERIFIED：handoff，00-overview．D16 の現状は INFERRED） | 閉じる | 後の段で置き換わった |
| 4 | 1b4e2：origin を越える待ち，close で読めない TX の report，`effective != e` | RM-34（stream の間の move）は撤回（`design/07:344`，VERIFIED） | 閉じる | 届かない |
| 5 | 4d-3：`rx_segment` の timing の行の `channels` が数のまま | `rx.rs:469` は数，`timing.plan` は列（`core.rs:777`）（VERIFIED） | 8b | P3．8b で行に装置を足すときに両方そろえる．Manifest の形なので，そのとき owner に出す |
| 6 | 4d-3：plan の e2e が Manifest のファイルまで通らない | 残っている（INFERRED） | 捨てる | 他の e2e が書いた Manifest を読む（INFERRED） |
| 7 | `Frozen` の整理の残り（plan 50 A13） | n51-1c-ii で消えた（crates に無い，VERIFIED） | 済 | — |
| 8 | Provider が id を名乗る処理の写し（約 15 か所ずつ．handoff §3 の「任意」） | 残っている | 後（汎用性の点検） | 三つ目の radio Provider を書くときに helper にする．今は二つが食い違った記録が無い |
| 9 | 文書監査の D2（`RX_OVERFLOW` の値が Mock と UHD で違う．plan 50 A7） | 1b-7b で payload を一か所で決めた．4b で RM-17 を一つの規則にした．parity の `compare_rx_overflow`（`parity.rs:909`）（VERIFIED） | 済 | — |
| 10 | 実験の規則の取り込み（45-report-e4 §5，plan 50 E） | 時刻の枠の解放はもう装置の時刻．生成の失敗は L1 の `abandon_unowned`（`mod.rs:674`）（VERIFIED）．join の順，受付の時刻，`u14c` は brain/ 用（INFERRED） | 捨てる（単独の項目として） | 8b の step 0 で形を決めるときに §4 を読み直す |
| 11 | stat `tx_close_past_record_front`（E3b K2） | §2e | §2e の判断 D7 | — |
| 12 | `Direct` に `Drop` が無い（n51-2b） | 無い（VERIFIED） | 8b（最初の掃除） | テストが落ちた後だけの漏れで P3．§2b |
| 13 | `UhdRadio` に `Drop` が無い（n51-2b） | 無い（VERIFIED） | 捨てる | Kernel の後片付けは必ず stop する．join する `Drop` は自分のスレッドで捨てられると止まる．テストの漏れは #12 で足りる |
| 14 | `FakeDevice` の literal と `common::fake()`（n51-3 の note 4） | 368 の literal（VERIFIED） | 捨てる | 欠陥が無い．時刻の決まりは Run の wrapper と governance の scan で守る（INFERRED） |
| 15 | kc_36 の TTL の 2 本が負荷で落ちうる（n51-3） | 残っている（VERIFIED：テストあり） | 今 | gate の信頼．§2b |
| 16 | scan の pin が関数の名前だけで決まる．捕まえない形（`elapsed() <`，`Wall::new`，計算した値）（n51-3） | scan の header に限界として書いてある | 捨てる | その後に該当が出ていない |
| 17 | `mutate.py` の `declared` の受け渡しにテストが無い．baseline の cache の key が `exact` を見ない（n51-3） | `exact` を持つ行は無い | 捨てる | 行が `exact` を持ったら直す |
| 18 | G10・G29 の実時間の killer（n111） | 10 s の待ちと 50 ms の窓，`kill` の宣言なし（VERIFIED） | 捨てる（決定 E のまま） | n51-3 の決定 E（Kernel の期限切れは普通の kill）．負荷で出るのは生き残りの報告で，偽の kill ではない．変える理由の系列が無い．§2b |
| 19 | `RunStateMachine::{move_to, begin_stopping, finish}` が HostClock を読む（n111） | HostClock を引数に取る（`run.rs:629,657,663`，VERIFIED）．lock の下で呼ぶかは確かめていない | 凍結前の全規則の確認（handoff §4） | KC-24a の範囲の確認．Kernel |
| 20 | FreezeDispatch の 5 s が全体の stop の列にかかる（n111） | 8b の一覧にある | 8b | 複数台 |
| 21 | `Stream::successor` と `Rx::successor`（4b n1） | 欄は一つ（`rx.rs:119`，VERIFIED） | 済 | — |
| 22 | probe-T0 と margin の重複，fake の手動の時計の零（4b n2，n107） | 残っている（INFERRED） | 8b | 8b の rig（§2b） |
| 23 | 受信の静かな終わりの手順が 8 回（4b n3） | helper `end_quietly`（`rx.rs:2067`，VERIFIED） | 済 | — |
| 24 | UHD に受信を再開させない streaming（4b-reviews:113） | 案のまま | 後（汎用性の点検か 8c の入力） | 測る値の意味が変わる．KL-8 と bench が要る．user に見える |
| 25 | HeaderTap：`tests/common` に DataLink の記録係が二つ．fold が capture Sink の `push_continuity` を写す（tools-recheck，`tools-dispositions.md:26`） | `measurement.rs:2902`，`s9bench.rs:64`（VERIFIED） | 捨てる | テストだけ．trait が変われば compiler が知らせる |
| 26 | n115 N1：Run 中の上書きの key が `Key` だけ | 受け入れ済み | 捨てる | 木の中の検査からは届かない |
| 27 | n115 N2：KC-24 の手順 10 と 12 で `requested` の意味が違う | 文が残っている | 凍結前の全規則の確認 | spec の文 |
| 28 | n115 N3：RF の検査の Run 中の枝が常に上書きされる．`Admitter::admit` は呼び手がテストだけ（`session.rs:209`，VERIFIED） | 残っている | 凍結前（MA-16a の公開 API の見直し） | Kernel の表面 |
| 29 | `rx_chain` の lock の使い手が一人（47s2 P1231） | まだ `core.rs:1562,1571`，`mod.rs:674` が取る（VERIFIED） | 捨てる | 争いのない lock．外すのは得のない並行性の変更 |
| 30 | TX の header の `contract` の take と restore が無駄（47s2 P1169） | `tx.rs:170,231,790,817`（VERIFIED） | 8b（残り物の掃除） | 小さい |
| 31 | `n47s5_front_rehearsal_on_the_fake` の host の遅れの印（bt1，mt3） | 設計どおり | 捨てる（負債ではない） | 実時間の rehearsal として宣言済み（n51-3） |
| 32 | runbook にテストの名前が出てこない実機のテスト 16 本（この下書きで見つけた） | 4 本は design が証拠に引く．`hw_m1`・`hw_m2`（front end を streamer の間で動かす）が試すのは，撤回した RM-34 の道（撤回は VERIFIED）．bench-results がその結果を引いているので，消してよいかは INFERRED | 8b（残り物の掃除） | `hw_m1`・`hw_m2` を消すか，その時に確かめる．結果の記録は bench-results に残る．残りは記録の証拠なので残す |
| 33 | #117，#118 | label `phase:8b`（VERIFIED） | 8b | owner の線引き |
| 34 | brain/ の `oracle.rs`・`cases.rs` を分ける話（`45e4a-test-plan.md:137`） | brain/ は 11-04 に消した（`26350523`，VERIFIED） | 閉じる | 対象が無い |
| 35 | 適用済みの switch の held burst が dispatch で落ちたときの `applied` の行．`cancel_switch_from` と `cancel_held_switches` の重複（`47s5-test-plan.md:1186`） | 名指された 3 関数は無い（`drop_piece` だけ残る，VERIFIED）．switch の結果は E2a の book と C2（境目の sample で決める）に移った（INFERRED：振る舞いは読んでいない） | 閉じる | 書き直しで消えた |
| 36 | n51-1c-ii を 4a で止める案 Q7（`n51-1cii-test-plan.md:346`） | 1c-ii は全部済（`Frozen` などは無い，VERIFIED） | 閉じる | 止めなかった |

`tools-dispositions.md:26` の「recorder の fold が capture Sink の `push_continuity` を写す」は #25 と同じ件なので，そこに含めた．

数：

| 仕分け | 件数 | # |
|---|---|---|
| 済・閉じる | 10 | 2，3，4，7，9，21，23，34，35，36 |
| 今 | 1 | 15 |
| owner の判断 | 2 | 1，11 |
| 8b | 7 | 5，12，20，22，30，32，33 |
| 凍結前の確認 | 3 | 19，27，28 |
| 後の phase | 2 | 8，24 |
| 捨てる | 11 | 6，10，13，14，16，17，18，25，26，29，31 |

「今」の 1 件は §2b の小さな段にする．

### 2e. ④ Module の形と，stat `tx_close_past_record_front`

**④ Module の形．**

- **出どころ．**
  - owner 2026-11-04 の順番：47 → 機能を減らす → 実機 → **Module の形の設計メモ** → …（`45-report-e4.md:3`）．
  - 48 §1 では範囲の外とした．
  - 49 の §5.8（H：stream ごとの actor）と §5.13 で，④ の入力を「状態の持ち主は D′ で一つ」とした．
  - plan 50 §3 B で締めの振り返りに含めた．
- **その後に決まったこと**（VERIFIED，各メモ）：
  - 判断は純粋な関数でスレッドの外にある：`segment.rs`，staging の `TxBook<P, T>` と `DeviceChain`（E3b），`timeline`．
  - 一つの事実に持ち主は一人：35 の `Streams` の遷移，44 の `RxChain`（L1），TxChain は book へ（E3b），device gate．
  - スレッドの形は変えていない：uhd-control，uhd-rx-s<k>，uhd-tx-s<k>．
- **退けた形．**
  - 一つの判断役のスレッド（brain/・DST）：E4 が前もって決めた二つの条件をどちらも満たさなかった（`45-report-e4.md` §1）．
  - stream ごとの actor（H）：`recv` で止まる．send は約 10 s 止まりうる（49 §5.8．INFERRED：算術）．
  - controller の一本（B）：Stop が遅れる（49 §5.14）．
- **まだ開いていて，答える相手が 8b にしかないこと．**
  - 8b の判断 1（owner 2026-10-10：10 MHz＋PPS を共有する装置は一つの instance・一つの root で，中に UHD の handle をいくつ持ってもよい．`28-forward-audit.md:11`）のもとで，スレッドと device gate が装置の数でどう増えるか．
  - RS-8a の 5 s と，装置ごとの stop（1 台約 3 s．n111 の Opus の note）．
  - KC-21a が uhd-control を待つこと．

**推奨**：単独のメモにしない．④ を 8b の step 0 の設計の最初の問いにして，上の事実を入力として渡す．plan 50 §3 B の ④ の行は閉じる．

退けた案：
- **今書く．** 使う相手が無い．8b の条件（台数，PPS で揃えた開始，KC-21a の待ち）がまだ決まっていない．8b が開き直す設計になりやすい．49 §5.13 は，D′ がどのスレッドの形にも移せると既に言っている．
- **やめる．** 8b は装置ごとのスレッドをどのみち決めなければならないので，問いは戻ってくる．

後で不可能になること：無い．費用：今は 0．8b の step 0 に問いが一つ増える．

**stat `tx_close_past_record_front`**（E3b K2．Manifest の `stats` の key なので owner の判断）．

証拠（VERIFIED）：
- 書く所は一か所：`close_unjoined`（`tx.rs:546`）．
- 読むのはテストだけ：helper `past_front`（`tx.rs:5132`）で 0 を確かめる 9 か所（tx.rs 8，e1.rs 1）と 1 を確かめる 1 か所（`tx.rs:3473`），`e3b.rs:1051`（無理に front を進めて `Some(1)` を確かめる）．
- 行は N47S5-60（maintenance）．その killer `ur_21_stat_counts_owner_closes_past_the_record_front`（`tx.rs:3457`）を，UR-23 の `*Test:*` の行（`design/18-uhd-radio.md:306`）が名指している．
- UR-30 の `stats` の key の一覧（`design/18:388`）には**入っていない**．schema，Python，KNOWN_LIMITATIONS にも無い．実機のテストも bench の道具も読まず，`bench/*/results` は Manifest の stats を記録しない．
- **確かめていないこと**：非公開の生のログ（`Ez-SDRv4-bench-raw`）で一度でも 0 でなかったか．
- key は初めて数えたときに作られる（lazy．`e2a-test-plan.md:58`）ので，普通の Run の Manifest には出てこない．
- 見ているのは front を書く所の一つだけで，F2・F6 は見えない（`e3b-test-plan.md` §A）．一致は E3b の関係のテスト（P-CF）が suite の中で固めている．

**推奨：消す**．コードの 1 文，テスト，行 N47S5-60，`past_front` の確かめ，UR-23 の `*Test:*` の名前，spec 18 の Changes に 1 行．KL は足さない．費用は小さな段一つ（DeepSeek が直接書き，Terra がレビュー）．

退けた案：残して UR-30 に診断として書き，bench の道具に表示させる．shape S が残した写し（記録の予測と装置）の実機の見張りになる．しかし front を書く所の一つしか見ず，誰も読んでいない．見張りが要るなら，front を書くすべての所の一般の検査にすべきで，それは owner が後へ回した L の形になる．

結果：記録と装置が close で食い違ったときだけ出ていた key が，Manifest から消える．普通の Run は何も変わらない．

## 3. owner に決めてもらうこと（一つずつ）

1. **D1 mutation の役割の分担（2a）．**
   - 推奨：手書きの行は段階ごとの gate に残す．cargo-mutants は凍結の前の見渡しにする．Gate X の後・8b の前に `ezsdr-radio` と `uhd.rs` で試し，その数で道具を決める．見渡しは sim の container の中で道具の写しの上だけで流す．Safety core と §7 に一文足す．
   - 代わり：生成器を gate にして行を置き換える．または何もしない．
   - 結果：image に道具が一つ増え，試しの段が一つ増える．凍結のチェックリストに「見渡し」が一項目増える．AGENTS の mutation の決まりが，行と見渡しの二つを書くようになる．
2. **D2 feature `uhd`（2a）．**
   - 推奨：mutation の image を `ezsdr-v4-dev:uhd4.10` の上に作り直す．行が feature を持てるようにし，`uhd.rs` の行は試しの結果で決める．
   - 代わり：`uhd.rs` を gate の外のままにして，20 本のテストと実機に任せる．
   - 結果：gate に穴が無くなる．代わりに image が大きくなり，二台の sim に dev の image が要る（今はある）．
   - これは道具の判断で orchestrator が決めてよいものだが，owner が「道具の選び方と一緒に決める」とした（handoff §3）ので，ここで出す．
3. **D3 テストの組み立て（2b）．**
   - 推奨：作り直さない．今直すのは gate の信頼の一つ（kc_36 の TTL の 2 本）だけ．`Direct` の `Drop` は 8b の掃除で．G10・G29 は決定 E のまま．8b の rig を一つ作る．
   - 代わり：規則ごとに並べ直し，rig をまとめる．
   - 結果：今の 40 の rig と段階の名前のファイルはそのまま残る．
4. **D4 8b のレビューの流れ（2c）．AGENTS §9 の 2 か所に句を足す．**
   - 推奨：
     - 直した試験計画の紙の再確認は 1 回まで．残りは先に落ちるテストと最初の小段のレビューへ，設計の問いなら step 0 へ．
     - 危険な段の中でも，振る舞いを変えない小段は Terra だけ．
     - 他（Fable の設計，Sol の計画と最後のコード，major の直しの後の再確認）は今のまま．
   - 代わり：今のまま（紙の再確認の回数は orchestrator がその都度決める）．
   - 結果：往復が長引く危険な段階で 1〜2 時間速くなる見込み．三度目の紙読みでしか見えない誤りは，小段と Sol に任せる．
5. **D5 負債の仕分け（2d）．**
   - 推奨：表のとおり．閉じる 10，今 1，8b 7，凍結前 3，後の phase 2，捨てる 11．
   - 代わり：捨てる 11 件を 8b へ回す．
   - 結果：捨てた項目は git の記録にだけ残る．
6. **D6 ④ Module の形（2e）．**
   - 推奨：8b の step 0 の最初の問いにして，plan 50 の ④ の行を閉じる．
   - 代わり：今，単独のメモを書く．
   - 結果：8b の設計が一つ大きくなる．
7. **D7 stat `tx_close_past_record_front`（2e）．**
   - 推奨：消す．
   - 代わり：UR-30 に書いて，bench で表示する．
   - 結果：Manifest の `stats` から，普通は出ない key が一つ消える．

**orchestrator が決めること**（着地のときに一行で報告する）：
- 「今」の 1 件（kc_36）を，一つの小さな段にする．
- D1 の試しの指示書と，その段のレビューを Terra にすること．
- 8b の最初の掃除（#12，#30，#32）．
- #5 を 8b で出す時期．

## 付録 A. 再実行の命令

`~/.cache/ezsdr-fixes/2026-11-20-p8end/m/commands.sh` が全部を流す（読むだけ．`PYTHONDONTWRITEBYTECODE=1`）．

| 命令 | 内容 | 出力 |
|---|---|---|
| `python3 -I churn.py ~/local_work/Ez-SDRv4-n111 ec95e4b2 4679abc1` | first-parent の commit ごとの分類した行，テスト関数，行の増減．約 1.5 分 | `churn.jsonl` |
| `python3 -I totals.py <repo> <rev>`（`ec95e4b2`，`03b8f94d`，`4679abc1`） | crate ごとの今の大きさ（行と kchar） | `totals-<rev>.md` |
| `python3 -I rows_by_target.py <repo> 4679abc1` | 行が何を変えるか | — |
| `review_counts.py` → `steps.py` → `review_compare.py` | レビューの数え表，段階・期間の表，往復の列 | — |
| `crate_churn.py`，`gate_times.py`，`bench_tools.py`，`mechanisms.py`，`mech_findings.py` | crate ごと，gate の時間，bench の道具，機構（ledger と review の表） | — |
| `git grep -c` の loop | テストの数 | `test-counts.txt` |
| `git grep "phase 8-end"` と日本語の言い方 | 負債の出典 | `debts-grep.txt` |
| `gh issue list --state all --json …` | issue と label | `issues-2026-11-20.json.txt` |

この下書きを `plan/` に入れるとき，`52-measure/` の要るものも一緒に入れる（記録は段階の置き場に一か所）．

## 4. owner の決定（2026-11-20）

- D1 (owner 2026-11-20 「ok」): hand-written rows stay in each step's gate; cargo-mutants is not a gate — one sweep of production code before the v4.0 freeze, survivors sorted (add a test / equivalent with reason / accept), steps not stopped; adopt only after a small trial after Gate X and before 8b (`ezsdr-radio` and `uhd.rs` with libuhd; adopt if the whole workspace can be swept overnight on sim02+sim03 and the survivors sorted, else write "no generator" on the freeze checklist); AGENTS Safety core and §7 gain one sentence: a generator's sweep also runs only in a sim container on the tool's own copy, never in a working tree.
- D1's trial (2026-11-20, records `reviews/tools2-*`): `ezsdr-radio` 1645 mutants in 448 s (1240 caught, 261 missed), `uhd.rs` with feature `uhd` 166 in 531 s (48 caught, 106 missed); the whole workspace estimated at ≈14.7 h on sim02+sim03 (conditional: a first-pass survivor rate ≤ ≈11.5 % would fit 12 h). Outcome (owner 2026-11-20 「D1は推奨でok」): no whole-workspace generator sweep on the freeze checklist; the tool stays for per-crate sweeps of pure-rule crates; the `ezsdr-radio` gaps it found (`Grid::contains`/`nearest`/`at_or_above`/`at_or_below` edges, `x3xx.rs` `decimation`/`receives`/`uhd_name`, `addressed`, `RfEnvelopeCheck` with `lo_hz == hi_hz`) go to 8b's first clean-up, and the `uhd.rs` row candidates (F4's `answers`/`reclaim`/`Drop`/`kept_count`, `rx_recv`'s Samples path, `tx_close`/`close_streams`/`mark_lost`, `made_subdev_spec`, `open`'s mcr check) to 8b with dummy C functions first.
- D2 (feature `uhd` into the mutation gate: the mutation image rebuilt on `ezsdr-v4-dev:uhd4.10`, rows may carry `features`): explained to the owner as item 4 of D1 and accepted with it.
- D3 (owner 2026-11-20 「ok」, after asking whether it holds long-term — the orchestrator answered that the problem will return): no rebuild now; fix only kc_36's two TTL tests (gate trust); `Direct`'s Drop in 8b's first clean-up; G10/G29 stay (decision E); 8b gets one multi-device rig from the start. Added: (1) existing tests move to the 8b rig when a step touches their file ("migrate on touch"), never a big-bang rebuild; (2) at the end of 8b the same measurement (`m/commands.sh`) is rerun, and if the test/production ratio is still growing a consolidation step is raised then, using D1's generator sweep as evidence; (3) briefs keep the rule: a new decision goes into `ezsdr_radio` pure functions with table tests first, one rig test per wiring path.
- D4 (owner 2026-11-20 「ok」): keep Fable (design), Sol (risky test plans; every step's final code), the re-check after a major fix (Opus, Terra beside it). Add to AGENTS §9: (1) a corrected test plan is re-checked at most once ("Sol once + one narrow re-check"); what remains goes to failing-first tests and the first sub-step review, or back to step 0 if it is a design question — reviewed against the records after 2–3 risky steps; (2) inside a risky step, a sub-step that changes no behaviour (a mechanical rewrite) is reviewed by Terra alone; Opus + Terra side by side only for sub-steps that change behaviour.
- D5 (owner 2026-11-20 「ok」): the 36 debts sorted as draft §2d's table — closed 10, now 1 (kc_36), owner 2 (D2, D7), 8b 7, before-freeze check 3, later phase 2, dropped 11 (they stay in git history).
- D6 (owner 2026-11-20 「ok」): ④ Module form is not a standalone note; it becomes the first question of 8b's step 0 with draft §2e's facts as input; plan 50 §3 B's ④ row closes.
- D7 (owner 2026-11-20 「ok」): remove the stat `tx_close_past_record_front` (the one statement in `close_unjoined`, its tests and `past_front` checks, row N47S5-60, UR-23's *Test:* name, a spec 18 Changes row; no KL). Not checked: whether it was ever non-zero in the private raw bench logs.

実施：D3 の「今」（kc_36）と D7（stat の削除）はこのメモと同じ着地．D4 と D1 の安全の一文は AGENTS.md §9 と Safety core に入れた．D1 の試しと D2（mutation の image）は 8b の前の道具の段（決定では Gate X の後．道具だけで Phase 8 のコードに触らないので，owner 2026-11-20 の了承で bench 待ちの間に前倒し）．D3 の測り直しは 8b の終わりに `52-measure/commands.sh` で．
