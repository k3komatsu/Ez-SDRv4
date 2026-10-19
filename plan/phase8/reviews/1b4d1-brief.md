# 指示書：Phase 8 段階 1b-4d-1 — stream ごとに device が持つものを 1 つの記録にする

リポジトリ：/home/komatsu/local_work/Ez-SDRv4（HEAD `b5596e8`，1b-4c まで commit 済み）．
この段階は **risky**（AGENTS.md §9：lifecycle と，fake と hardware の境界．owner と uhd-control のどちらが device を操作したか）．手順は §9 の 2 から：**まずテスト計画だけを書く**．コードは書かない．

## 1. なぜ（three strikes）

1b-4c の Sol の再レビュー 3 回は，同じ機構の major を 1 件ずつ見つけた（`plan/phase8/reviews/1b4c-opus.md` の「three strikes」，`plan/maintenance/failure-mechanisms.md` の 1b-4c の段落）．owner の thread が「device が今何を開いて何を当てているか」を自分の写しで覚え，uhd-control が device を変えた後もそれを当てにする．

- TX：`tx.rs` の `channels` と `verified`（`do_switch`）．
- RX：`rx.rs` の `opened` と，`StreamState.stale`（`core.rs`，uhd-control が立てる印）．
- 両方：segment の `configured`（`StreamState.configs` の bool，`Planned.configured`）．uhd-control の from-none が configure した segment を owner が開かずに始める近道．後の from-none が device を置き換えても残る（HEAD `278de14` からある．Sol の再レビュー 3 の指摘 1）．

## 2. 設計（orchestrator の裁定．計画はこれに沿って書く）

1. **1 つの記録**：`StreamState` に，その stream の streamer について device が実際に持つものを置く（line の lock の下）．
   - 開いている channel の集合：`Unknown`（分からない）か `Open(Channels)`．
   - 最後に成功した configure：どの segment のためか（その segment を始めた command の `seq`）．`Open` のときだけ意味を持つ．
   - 記録を書くのは，device を呼んだ thread だけ．呼び出しの**後**に lock を取って書く（device を呼ぶ間は line の lock を持たない．AGENTS の既存規則 UR-25）．
   - open の成功で集合を書き，configure の記録を消す．全 channel の configure と read-back が通った後にだけ configure を記録する．open の失敗・照合の拒否は `Unknown`．configure の失敗は集合を残し configure の記録を消す．
2. **操作の担当を lock の中で押さえる**：device を呼び始める前に，同じ lock の中で「操作中」を記録に立てる（uhd-control の from-none も owner も）．owner は「操作中」を見たら何もせず次の poll で読み直す．uhd-control の from-none は，今と同じく owner が idle のときだけ．
3. **owner の判断**（RX の `begin`，TX の `do_switch`）：次の segment について
   - 記録が `Open(その集合)` で，configure がその segment の `seq`：何もしない（今の `configured` の近道の代わり）．
   - 記録が `Open(その集合)` で configure が違う：configure だけ．
   - それ以外：開き直し → rate の照合（UR-13．保存した結果があれば再適用）→ configure．
   - 拒否された segment（`Planned.refused`）は今と同じく device を呼ばない（TX は clock を始めない，P358）．
4. **消すもの**：TX の `verified`，RX の `opened`，`StreamState.stale`，`StreamState.configs` の bool と `Planned.configured`．
   **残すもの**：実行中の segment の情報（RX の `Stream` の channel 数と block の解釈，TX の `channels` を burst の channel 数と `unacked` に使う部分），UR-13 の保存した照合の結果（`check_rates`）．
5. **`arm`（`mod.rs`）が最初に開く streamer も同じ記録に書く**．
6. **未決（計画で決めて Sol が見る）**：Sol の再レビュー 3 は「cold の `seq` だけでは設定の同一性にならない．timed 更新が後から投影を変える」と書いた．orchestrator の推奨は `seq` のまま：from-none の後に記帳された timed 更新は device の timed command として別に効くので，segment の始まりで configure し直す必要はない（今の `configured` の近道と同じ前提．INFERRED）．計画は S11 でこれを確かめ，違うと分かったら理由を書いて「版」を足す案を出す．

## 3. 順序と期待する結果（計画は各順序にテストを 1 本，assert は 1〜2 個）

device の呼び出しは FakeDevice の `calls()` で見る．順序は owner の thread を直接動かす（`rx.rs`・`tx.rs` の既存の unit test の形：`read`・`begin`・`step` を呼ぶ）か，manual clock と settle で作る．sleep で競合を作らない．

| # | 順序 | 期待 |
|---|---|---|
| S1 | RX：owner は止めたまま．uhd-control が from-none で B=`{0,1}` を open・照合・configure → 後の cold で集合を空にし B を正の長さで終える → uhd-control が from-none で C=`{0}` を open・configure → owner を動かす（Sol 再レビュー 3 の系列） | owner は B を始める前に `{0,1}` を開き直して configure する．続く C でも開き直す |
| S2 | TX：S1 と同じ形 | 同じ．C の clock は C の segment で始まる |
| S3 | RX・TX：from-none が成功し，owner がその segment を始める | 追加の open も configure も無い（今と同じ呼び出しの数） |
| S4 | from-none の open が失敗 → 同じ集合の次の enable | owner が開き直して照合する |
| S5 | from-none の rate の照合が拒否 → 同じ集合の次の enable | 開き直しで保存した拒否が再適用され，clock も burst も始まらない |
| S6 | from-none の open と照合は成功，configure が失敗 → 同じ集合の次の segment | 開き直さず configure だけ |
| S7 | owner の open が失敗 → 同じ集合の次の segment | 開き直す |
| S8 | owner の configure が失敗 → 同じ集合の次の segment | configure だけ |
| S9 | uhd-control の from-none の device 呼び出しの最中に owner が poll する（device の wrapper が `rx_open` の中で止まる） | owner は device を呼ばず，何も始めない．uhd-control が記帳した後の poll で始める |
| S10 | TX：from-none で拒否された enable | clock を始めない（P358），`unacked` の対応は変わらない |
| S11 | from-none の後，owner が始める前に，その segment の origin より前に効く timed 更新が記帳される | §2 の 6 の推奨なら：configure し直さない．timed command は device に一度だけ出る |
| S12 | `arm` で開いた streamer の集合のまま，最初の segment が始まる | 開き直さない |
| S13 | 全経路 | device の呼び出しの間に line の lock を持たない（`test_support` の LockSpy） |

計画には，上に無い状態や失敗の経路を見つけたら足す（例：Stop，prune，`Shutdown`，halt した stream の次の enable）．

## 4. 計画に書くこと（テストごと）

- テストの名前（`ur_<規則>_...`），置く場所（unit か `tests/fake.rs`）．
- 何を assert するか（1〜2 個），どうやって観測するか（`calls()` の列，Manifest の section，event）．
- そのテストを落とす誤った実装（例：「`configured` の近道を残す」「失敗で記録を `Unknown` にしない」）．
- 既存のテストのうち，この変更で消えるもの・書き換えるものと，その理由．特に `verified`・`stale`・`configured` を見ているテスト（`tx.rs`・`rx.rs`・`control.rs` の unit，`tests/fake.rs` の `ur_13_*`・`ur_25_*`）．
- mutation の行の扱い（行は code と一緒に書くので，ここでは方針だけ）：P318，P344，P345，P358，P361〜P367 など，消すコードに当たっている行をどう当て直すか，消すか．
- **helper**（owner 2026-10-19）：テストで繰り返す式（Manifest の section を引く式，`COMMAND_REJECTED` の理由の取り出し，device の準備）を helper にする計画．
- **大きさ**：本体とテストで約 1,500 行に収まる見積もり．超えるなら分け方を書く．

## 5. 守ること

- コードを書かない（この回は計画だけ）．tree を変えない．
- `cargo fmt`・`rustfmt` を流さない．mutation を流さない．commit・push・`git add` をしない．
- 読むもの：`crates/ezsdr-radio-uhd/src/provider/{core,control,rx,tx,mod}.rs`，`crates/ezsdr-radio-uhd/src/device.rs`（FakeDevice），`design/18-uhd-radio.md` の UR-13・UR-17・UR-25，`plan/phase8/reviews/1b4c-sol-rereviews.md`．

## 6. 報告

`~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1/test-plan.md` に日本語で書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．

---

## 実装の指示書（改）：Phase 8 段階 1b-4d-1a

前の回の報告 `report-blocked.md` を受けて，orchestrator が次のように決めた．`impl-brief.md` の「守ること」「確かめること」「セルフレビュー」「報告」はそのまま当てはまる（報告のファイル名は `report.md`）．

### 分け方（採る）

- **1b-4d-1a（この回）**：stream ごとの device の記録（集合と `(seq, 版)`），版の規則，owner の判断（何もしない・configure だけ・開き直し），拒否された segment では device を呼ばない規則，`arm` が記録を書くこと，prune が記録を変えないこと．計画の S1〜S8，S10，S11a〜S11c，S12，S14，S16，S20，S21，S25 と，その既存テストの書き換え・mutation の行の対応表のうちこの範囲のもの．`verified`・`opened`・`stale`・`configured` の写しを消す．
- **1b-4d-1b（次の回）**：busy（操作の担当），D1 の A，lock の順序の検査．S9，S11d，S13・S13b，S15，S17，S18，S19，S22〜S24，S26〜S28．
- 1a の間は，uhd-control の from-none と owner の競合の窓は今（HEAD）と同じだけ残る．1a で窓を広げないこと（今の `idle` の判断と lock の取り方を変えない）．

### 前の回の「blocker」への答え

- `ColdConfig::configure` と release が `updates` の lock を device の呼び出しの間持つのは，§4.7 の clock lock の設計（`ClockMutex`）で，今も許されている．規則が禁じるのは **line の lock（`streams`）** を device の呼び出しの間に持つことだけ．計画の「`updates` → `streams` → `rec`」は取る順序の規則で，`updates` を device 呼び出しの間に持つことと矛盾しない．
- 1a は busy を入れないので，from-none が `streams` を離して device を呼ぶ今の形はそのまま．
- 版は `ColdConfig` の `AtomicU64` で，`streams` の中で `updates` を取らずに読める．`configure` は `updates` の lock の中で版を読み，その値を返して，呼んだ thread が `streams` を取って記録に書く．

### やり方

- この範囲は 1 回で閉じられる大きさ（本体 200 行前後とテスト）．`blocked` で止めず，最後まで実装する．判断に迷う点は，計画と adjudication に沿う側を選んで報告に書く．
- 既存テストの分類が計画に無いもの（`tx.rs`，`tests/fake.rs`）は，実装しながら分類して報告に表で書く．
- `design/18-uhd-radio.md` は，1a で偽になる文だけを直す（記録，owner の判断，拒否された segment，arm）．busy の文は 1b．
