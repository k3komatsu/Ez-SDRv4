# D1 の試し：cargo-mutants 27.1.0 を `ezsdr-radio` と `uhd.rs`（`--features uhd`）に当てた結果

実施：2026-11-20．道具は `plan/maintenance/tools/remote/sweep_mutants.py`（`remote_mutate.py` の `upload`・image・コンテナの分け方を使う）．変異は sim02・sim03 のコンテナの中で，cargo-mutants が作る写しの上にだけ当てた（`--in-place` なし，木には触れていない）．simq の下で，gate や全 mutation と重ねていない．生の出力は `sweep/{radio,uhd,radio-ws,radio-ws2}/`，ログは `sweep-*.log`，一覧は `list-*.log`．

## 1. 数字

| 対象 | 試験 | 変異数 | caught | missed | unviable | timeout | 壁時間 | 使った核 |
|---|---|---|---|---|---|---|---|---|
| `ezsdr-radio` 全部 | package のテストだけ（既定） | 1645 | 1240 | 261 | 126 | 18 | 448 s | 16 shard × 4 = 64 |
| `uhd.rs`（`--features uhd`，`--lib --test uhd_api`） | gate の `uhd-test` と同じ | 166 | 48 | 106 | 12 | 0 | 531 s | 8 shard × 4 = 32 |
| `radio` の `x3xx.rs`・`device.rs`・`lib.rs` | `--test-workspace`，timeout 20 s（下限の既定） | 382 | 140 | 0 | 37 | 205 | 777 s | 64 |
| 同じ | `--test-workspace`，`--minimum-test-timeout 400` | 382 | 307 | 0 | 37 | 38（未決：400 s で打ち切り） | 5483 s | 64 |

- 1 変異あたりの shard 時間（build + test）：`radio` の package テスト 平均 2.3 s（最大 22 s）．`uhd.rs` 平均 20.5 s．workspace のテストでは caught が平均 98 s，400 s の打ち切りに届いたものは未決（timeout．生き残りではない）．
- `radio` の timeout 18 は `Bookings::held_until` / `first_free` / `region_met` の変異（`staging.rs:1623,1642-1647,1244`）で，無限ループになる．失敗と同じに見てよい（INFERRED：ログは読んでいない．落とす変異は caught 側に入れてよい）．
- 4 行目：workspace のテストでは baseline が package のテストだけで 0.6 s と測られ，打ち切りが下限の 20 s になった（205 件の timeout は道具の設定の問題）．baseline はその package のテストしか走らせず，変異の方は workspace 全体を走らせる，という cargo-mutants の動きが原因（VERIFIED：`outcomes.json` の argv）．`--test-workspace` を使うなら `--minimum-test-timeout` を明示する．
- 4 行目の読み方：package テストで missed だった 48 件のうち，10 件は他の crate のテストが殺した．38 件は 400 s の打ち切りに達した（**未決（timeout）**：生き残りとは数えない）．package テストで caught だった 297 件は 1 件も timeout になっていない．workspace のテストは 1 回 約 550 s かかる（下の表）ので，400 s の打ち切りは全テストが終わる前である．38 件は，打ち切りまでのテストが通っただけで，後ろのテストが殺すかもしれず，止まっているだけかもしれない．**生き残りではなく未決**として扱う．決めるには，変異ごとの timeout を 550 s の全テストより上（例：900 s）にして，この 38 件だけ流し直す必要がある（38 × 約 600 s ≈ 2.3 万 shard 秒，16 shard で約 25 分）．
- 2 行目の missed 106 は `uhd.rs` の 64 %．実機が要る関数（`open`，`tune_*`，`rates`，`time_now` など）と，ダミーの C 関数が無い道が大半．

crate ごとのテスト秒（`sweep_mutants.py baselines sim02`，4 CPU，他に負荷なし）：

| crate | テスト秒 | production の変異数（`--list`，workspace 7145 の内訳） |
|---|---|---|
| ezsdr-radio-uhd | 346（feature なし） | 2251（うち `uhd.rs` 166） |
| ezsdr-kernel | 22 | 1810 |
| ezsdr-radio | 0（0.7 s） | 1645 |
| ezsdr-mock-radio | 10 | 652 |
| ezsdr-server | 13 | 224 |
| ezsdr-sim | 0 | 186 |
| ezsdr-sink-capture | 0 | 108 |
| ezsdr-acceptance | 157 | 105 |
| 残り 4 crate | 0 | 164 |

## 2. workspace 全体を一晩で流せるか：見積りでは収まらない（約 14.7 時間，前提つき．測定で確定はしていない）

前提：sim02 + sim03 の 16 shard × 4 CPU = 64 核を gate などと重ねずに使う．

- **1 回目（package のテスト）．**
  - `radio` は測定どおり 1645 × 2.3 s = 3.8 千 shard 秒．
  - `uhd.rs` は 166 × 20.5 s = 3.4 千．
  - `ezsdr-radio-uhd` の残り 2085 件は，package のテストが 346 s かかる．caught は先に落ちるので速い．caught 8 割 × 40 s + 生き残り 2 割 × 350 s と置くと 平均 約 100 s で 21 万 shard 秒．全部 346 s かかるなら 74 万（INFERRED：caught の平均を測っていない）．
  - kernel 1810 × 約 17 s = 3.1 万．mock-radio 652 × 約 12 s = 0.8 万．server 224 × 15 s = 0.3 万．acceptance 105 × 160 s = 1.7 万．残り 約 0.2 万．
  - 合計 約 28 万 shard 秒 ÷ 16 shard = 約 17,600 s ≈ **4.9 時間**（幅 3.7〜13 時間．幅は主に `ezsdr-radio-uhd`）．
- **2 回目（workspace のテスト）が要る．**
  - 1 回目の生き残りは約 16 %（`radio` 実測 15.9 %，`uhd.rs` は 64 %．他の crate は INFERRED）で 7145 × 16 % ≈ 1140 件．
  - そのうち約 2 割（10/48）は他の crate のテストが殺す．残り 約 900 件が workspace のテストを最後まで走らせる．1 回 約 550 s（上の表の合計 157 + 22 + 10 + 346 + 13，負荷なし）．負荷の下では 600 s と置く．
  - 900 × 600 s = 54 万 shard 秒 ÷ 16 ≈ **9.4 時間**．caught の約 240 件（× 100 s）で +0.4 時間．
- 合計 **約 14.7 時間**（4.9 + 9.8 = (900 × 600 + 240 × 100) / 16．段取りと shard の偏りは含めない）．これは上の前提（他の crate の生き残り率 16 %，`ezsdr-radio-uhd` の平均 100 s，workspace テスト 600 s）の下での見積りで，測定した結論ではない：「一晩（約 12 時間）を超えうる」までが言える．
  - 分かれ目：他の前提を変えず，1 回目の生き残りの率が約 **11.5 % 以下**なら 12 時間に収まる（16 % では 14.7 時間）．`ezsdr-radio-uhd` の package 費用が下がる場合，または workspace を回し直さずに package ごとの生き残りだけを仕分ける場合も結論が変わる．
  - 決める測定：(1) `ezsdr-kernel` と `ezsdr-mock-radio` を 1 回目の形で流し，生き残りの率と 1 変異の時間を実測する．(2) `ezsdr-radio-uhd` の package テスト 1 crate の caught の平均秒を測る．(3) 変異ごとの timeout を 900 s にして 38 件を流し直し，未決の行方を決める．
- 仕分けの量も別の壁：生き残り 約 1140 件を人が見る．`UhdDevice::rates` 1 つで 27 個の変異のように，1 関数が束で出る（`#[mutants::skip]` や `exclude_re` で減らす手間が先に要る）．

**推奨：「workspace 全体の生成器の見渡し」は採らない**．凍結のチェックリストには「生成器なし」と書く．ただし道具は残し，**純粋な規則の crate だけを crate ごとに流す**使い方は価値がある：`ezsdr-radio` は 8 分で終わり（1 回目），生き残りの中身が実際のテストの穴だった（§3）．同じ形で `ezsdr-kernel`（1810 件，約 0.5 時間）を凍結の前に 1 回流すのは安い（INFERRED：kernel は未測定）．

## 3. 生き残りの仕分けの標本

`ezsdr-radio`（package のテストで missed 261 件の標本．「他の crate が殺す」は ws2 の結果で確かめたもの）：

| 変異 | 仕分け | 理由 |
|---|---|---|
| `device.rs:58 Grid::contains` の `\|\|`→`&&`・`==`→`!=`（ws2：未決（timeout）） | テストを足す | RM-8 の規則そのもの．端（min/max）と非有限値の表が無い |
| `device.rs:99,107,124,132 Grid::at_or_above/at_or_below` の比較（ws2：未決（timeout）か他 crate が殺す．どちらでも package の表が無いことは変わらない） | テストを足す | 端の値（bound == hi）を返さない変異が生き残る．等価ではない |
| `device.rs:79,86 Grid::nearest` の `&&`→`\|\|`（ws2：timeout）．84:36 の `>`→`>=` は他 crate が殺す | テストを足す | 丸めと端の表を作る |
| `device.rs:229-247 rate_set_matches`（6 件，ws2：timeout） | テストを足す | `ranges` の展開の端．FakeDevice は singleton だけで，実機（range）の形をテストが見ない |
| `x3xx.rs:87 decimation` の `\|\|`→`&&`（ws2：timeout）．91:10 の `<`→`==`・`<=` は他 crate が殺す | テストを足す | 純粋関数．表のテストが package に無い（D3 の規則 3 の対象） |
| `x3xx.rs:266 FrontEnd::receives → true`（ws2：timeout） | テストを足す | RM-28 の表．受信しない前面板が受信と答える |
| `x3xx.rs:155 Board::uhd_name`（`""` は timeout，`"xyzzy"` は他 crate が殺す） | テストを足す | `""` は UHD の名前の前方一致をすべて通す．実機の取り違えにつながる |
| `x3xx.rs:397 delete arm "hg"`（他 crate が殺す） | 受け入れる | workspace のテストが殺す．package の表にも足せるが急がない |
| `lib.rs:417 addressed` の `<`→`<=`（ws2：timeout），`<`→`==`・`>`・412:13（他 crate が殺す） | 受け入れる（417 の `<=` だけテストを足す） | 範囲外の stream 番号は `Other` になる，の端．`k == streams`（`<=`）は timeout で未決．workspace のテストが殺すかは分からない（package の表には無い） |
| `lib.rs:295 RfEnvelopeCheck` の `>`→`>=`（ws2：timeout） | テストを足す | RM-19 は `lo_hz > hi_hz` だけを拒む．`lo == hi` の帯が通ることを pin する |
| `staging.rs:1623 region_met` の `<`→`<=`，`1342 halt_at` の `<`→`<=` | テストを足す | 区間が接するだけの端．半開区間の規則 |
| `staging.rs:1081 close_unjoined → Some(Some(±1))・None`，`1710 is_switch → false` | テストを足す（package に） | Provider の統合テストが殺す見込み（INFERRED：未確認）．規則は `ezsdr_radio` の純粋関数なので package の表に置く（D3 の規則 3） |
| `timeline.rs:373 lattice → Ok(0)`，`59 mask の <<→>>` | テストを足す | 直接のテストが無い．`mask` は channel 0 では同じ値になり，番号 1 以上のテストが要る |
| `timeline.rs:294 Lines::line → None`，`304 Lines::book → Ok(0/1)` | 受け入れる | 委譲だけ．Provider のテストが通る道（INFERRED：未確認） |
| `staging.rs:1642-1647 held_until` の timeout 12 件 | 受け入れる（失敗として数える） | 無限ループになる変異．テストは止まらず，打ち切りで落ちる |

等価と言えたものは標本の中に 0 件．生成器が余計に出すのは，委譲の関数，実機専用の関数，無限ループの変異．

`uhd.rs`（`--features uhd`，gate の `uhd-test` の範囲で missed 106 件の標本）：

| 変異 | 仕分け | 理由 |
|---|---|---|
| `257,308 text/strings` の `-`→`+`・`/`（バッファ長） | テストを足す（低） | 「1 バイト短く渡す」NUL の保証．ダミーの `last_error` が渡された長さを検査すれば殺せる．1024 バイト以上の UHD の文が要る |
| `301 strings`，`317 find → Ok(vec![])` | 受け入れる | 結果は繋がっている実機に依る．装置が無い環境では `vec![]` が正しい答え（実機のテスト `hardware.rs` の領分） |
| `412 streamer_lifecycle → Ok(())` | 受け入れる | テストの本体が production にある．二重 free は abort で見るので，関数を空にすると見張りが消える．`#[mutants::skip]` の候補 |
| `429 made_subdev_spec` の `&&`→`\|\|` | **テストを足す（行の候補）** | 受理した spec まで free する．ダミーの setter は渡された handle を見ないので，null を渡しても通る．setter が非 null を受け取ることを検査すれば殺せる |
| `500 open` の mcr の検査（`<= 0`，`fract != 0`，`rate.is_err()`） | **テストを足す（行の候補）** | UR-3「mcr は正の整数」．`uhd_usrp_make`・`get_master_clock_rate`・`uhd_usrp_free` のダミーが要る |
| `639-660 answers/reclaim`，`680-685 Drop`，`633 kept_count` | **テストを足す（行の候補）** | F4：死んだ link の装置を free しない（process が落ちる）．`uhd_usrp_get_time_now` と `uhd_usrp_free` のダミーで，kept が増えることと reclaim の動きを検査できる．最も価値が高い |
| `1026-1058 rx_recv` の `received > 0`，`!has`，エラー文の長さ | **テストを足す（行の候補）** | `RxRecv::Samples` の道（受信の本道）にダミーのテストが無い．`uhd_rx_streamer_recv` のダミーは今は Timeout 側だけ |
| `1148 mark_lost`，`1165 tx_close`，`1174 close_streams → ()` | **テストを足す（行の候補）** | `rx_close` には「close が map の参照を落とす」テストがあるが `tx_close` には無い．close の後の `tx_send` が「閉じている」エラーになることを見る |
| `531 tune_dsp`，`564 tune_auto`，`812 rates`（27 件），`861 time_now`，`839/849 set_sources/set_time_zero`，`744/762 front_end/serial`，`702-709 describe` | 受け入れる | 実機が要る呼び出し．実機のテストと parity の測定が見る．`rates` は 1 関数で 27 変異なので，`exclude_re` の候補 |

## 4. D2 への推奨：`uhd.rs` で手書きの行にする候補

実演の行 `TL2-1`（`check_with` の `streaming && code == UHD_ERROR_RUNTIME`，殺すテスト `uhd_47_is_lost_only_at_the_two_stream_opens`）は `killed` だった．

次を行の候補にする（どれも「ダミーの C 関数を足す → テストを足す → 行」の順．今の段では足さない）：
1. F4 の道：`answers`/`reclaim`/`Drop`/`kept_count`（`uhd_usrp_get_time_now`・`uhd_usrp_free` のダミー）．process が落ちる道の見張りなので最優先．
2. `rx_recv` の Samples の道（`uhd_rx_streamer_recv` のダミーが samples と time spec を返す）．
3. `tx_close`・`close_streams`・`mark_lost`（`rx_close` のテストの写し．安い）．
4. `made_subdev_spec` の null（setter のダミーが handle を検査する）．
5. `open` の mcr（`uhd_usrp_make` のダミーが要るので，1 のダミーと一緒に）．

## 5. INFERRED と確かめていないこと

- 他の crate（kernel，mock-radio，server，acceptance）の生き残りの率と 1 変異の時間は測っていない（`radio` と `uhd.rs` からの当てはめ）．
- 2 回目の 600 s は，負荷の下の workspace テストの測定ではなく，負荷なしの 550 s からの置き換え．
- 「他の crate が殺す」10 件は ws2 の `CaughtMutant` で確かめた．`staging.rs`・`timeline.rs`・`channels.rs`・`measurement.rs` の生き残り（約 213 件）は workspace のテストで流していない．
- timeout 38 件は未決（400 s の打ち切り．生き残り・後ろのテストが殺す・停止のどれかは区別できない）．
- 実機が要る関数が missed になるのは，gate の `uhd-test` に実機のテストが無いため（`hardware.rs` は `#[ignore]`）．実機で見ている分は数えていない．
- 偽の kill（実時間のテストが負荷で落ちる，#91 と同じ形）は，`radio` の package のテストでは出ない見込み（INFERRED：実時間のテストを探していない）．workspace のテストでは 4 行目の caught に混じりうる．数えていない．
