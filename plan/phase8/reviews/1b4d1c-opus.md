# 1b-4d-1c：orchestrator のレビュー，裁定，数

## 1. 何をしたか

D1 の A：記録だけになっていた timed 更新を，その channel を持つ pending の segment に届ける．configuration の版を消す．

最後に残った規則は 3 つ（spec 18 UR-24・UR-36）．

1. **timed command の効く時刻は booking で決まり，後から動かない**．`e = max(at, now + device lead, release 済みの最後の command)`．床で動いた `at` は booking で `LATE_COMMAND`．release は `debug_assert_eq!(effective, e)` だけを残す．
2. **device に出す順は `e` の順**．release は held を `(e, seq)` の順に見る．`targets`（その instant に live な channel，無ければ origin がそれ以後で拒否されていない segment の channel）が空で `e > now` の command は held に残し，pass を止める．channel の無いまま `e` を過ぎたら `no channel` で記録する．
3. **release した値は，効く時刻が過ぎたときに構成の記録に畳む**．`released_at`，`config_at`，`held_updates`．畳み込みは 1 つの loop で `(e, seq)` の順．

あわせて：release は各 command・各 channel の前に loss を見る．`configured` は segment の command（`seq`）だけで，`owner_step` は集合と command で決める．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 計画 v1 | 形 F（release のその場で untimed に当てる，orchestrator の案） | Sol：major 5（古い segment に早く効く，UHD は untimed も先の command の後で実行する（VERIFIED），時間の保証，失敗，S8） |
| 計画 v2 | 形 F'（`e` の timed で pending の segment の channel へ） | Sol：major 6（時計，booking 後の channel の増加，失敗の観測，configure 済みでの失敗，S12，design） |
| 計画 v3 | 裁定 2（depth で持ち戻す，失敗を投影から除く） | Sol：major 3．どちらの規則も新しい窓を開いた → 裁定 3 で取り下げ，計画レビューを打ち切った |
| 実装 | DeepSeek，1 時間 49 分，セルフレビュー 3 回 | Opus：指摘なし．Sol：FAIL，major 2（`targets` が空でも `e` の投影が残る，S3 の時計） |
| 修正 1 | 投影の entry を `effective` へ動かす | Sol：FAIL，major 2（動かす前の configure，後の segment の基底）→ **three strikes** |
| 修正 2 | 効く時刻を booking で確定（§3）．DeepSeek が `recorded` の決め直しを自分で足した | Opus のレビュー：`recorded` が `released` に入らず，順序が崩れる反例 → 修正 3 |
| 修正 3 | channel 待ちは held に残し，release を止める | Sol：FAIL，major 2（畳み込みの順，channel の間の loss）minor 1 |
| 修正 4 | 畳み込みを 1 つの loop に，loss の検査 | Sol：**PASS** |

## 3. three strikes の設計メモ（AGENTS.md §6）

この段階では，同じ仕組み `planned-state-read-as-actual`（記録と device の値の食い違い）の指摘が続いた．計画の段階で 4 件（Sol v1 の 1・2，v3 の 2・3），コードの後で 5 件（Sol のコードレビュー 1，再レビュー 1 の 1・2，Opus の修正 2 のレビュー，再レビュー 2 の 1）．

**問い：その概念を消せるか．**

食い違いの元は 1 つだった．timed command の効く時刻を booking で `e` と決めながら，release で「release 済みの command の後ろ」として `effective` に変えていた．後から変えた時刻は，その投影をすでに読んだ configure や基底に届かない．局所の直し（持ち戻し，失敗の除去，投影の移動，`recorded` の決め直し）は，どれも新しい窓を開いた．

**答え：後から変わる時刻という概念を消した．**
- 効く時刻は booking で決まる（規則 1）．
- device に出す順は `e` の順（規則 2）．
- 構成の記録は値をその時刻で畳む（規則 3）．

release は時刻を変えないので，投影を動かす `ColdConfig::issued` と，版（`version`，`changed`）を消せた．テストは，各規則の系列（`ur_24_a_command_waiting_for_a_channel_*`，`ur_24_a_segment_*`，`ur_24_a_released_command_and_one_without_a_channel_settle_in_instant_order`，S12 の書き直し）と mutation の行 P426〜P435 が固定する．

## 4. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol の計画レビュー（3 回） | 14 | 約 15 | planned-state-read-as-actual 4（§3），残りはテストの観測と design の文 |
| セルフレビュー（DeepSeek，実装 3 回・修正 2 で 3 回・修正 3・4 で各 1 回） | P1 4 | P2/P3 多数 | P1 は修正 2 の `recorded` の穴（3 回）と時計 |
| Opus | 1 | 0 | planned-state-read-as-actual（修正 2 の順序） |
| Sol のコードレビューと再レビュー（4 回） | 6 | 3 | planned-state-read-as-actual 4，stream-lifecycle-scattered 1（release の channel の間の loss：`Core::configure` にはある検査が release に無かった），テストの時計 1 |

計画を先に書く手順の評価：計画の段階で major 14 件をコードの前に潰せた．それでも，コードの後に major 6 件が出た．原因は orchestrator の局所の規則（F，裁定 2）が続いたことにある．three strikes の問いを計画の段階で立てていれば，往復は短くなった．

## 5. 持ち越し

- **stop と owner の窓**（計画 v3 の S15，Sol v1 の 7）：owner が `Planned` を取った後に `Provider::stop` が来る窓．HEAD より前からある．1b-4d-2 で gate を置いて固定する．
- **frequency の DSP**（Sol v2 の 7）：idle の DDC/DUC の timed frequency は，サンプルが来るまで消費されない（VERIFIED：UHD の `dds_timed.v`，`axi_tag_time.v`）．テストは gain だけ．bench の系列：idle 中の timed frequency → owner の untimed configure → 長い無通信 → 最初のサンプル．
- **fake の FIFO**（Sol v2 の 7）：fake の untimed apply は timed の queue を迂回する．UHD は受け取った順に処理する．#62 と bench B8．
- **今の key の queue の会計**（裁定 3 §1.1）：booking の後の cold change で channel が増えると，release が depth を超え，device の back-pressure で待つ．1b-5 で今の key のモードが消える．
- **channel 待ちの後ろの command**（規則 2）：待っている command の `e` が過ぎるまで出ない．近ければ device lead より短い時間で出て，device は着いたときに適用する．
- 修正 3・4 のセルフレビューが範囲外として退けた P3（`FakeDevice::set_time` の「forward only」が強制されていない，`ApplyFails::timed` の型）．

## 6. DeepSeek V4.1 Flash の評価（owner の依頼，2026-10-20：OpenCode-Go の model が更新された）

前の段階（1b-4d-1b，同じ model の更新前と思われる）との比較．

| 見るもの | 1b-4d-1b | 1b-4d-1c |
|---|---|---|
| 実装の時間 | 約 2 時間 40 分（計画の書き直しを含む） | 1 時間 49 分．修正の回は 3 が 15 分（コードと mutation），4 が約 30 分 |
| 自分の mutation の生き残り | 3 行 | 実装 2 行（P408，P312 の当て違い），修正 1〜4 は 0 |
| 計画 | — | v1〜v3 とも 25 分前後で，裁定の細かい条件を守った．v1 の L は誤り（Sol） |
| 指示の違反 | なし | `mutations.json` の全体の書き直し 1 回（自分で戻した） |
| 報告と tree の食い違い | なし | 1 件（S3 の `hold` を「足した」と書いて無かった） |
| テストを固まらせた | なし | 1 回（20 分．timeout を付けていなかった）．指示の後は 0 |
| 指示書の外の仕組み | なし | 修正 2 で `recorded` の決め直しを足し，順序の穴があった |
| 起きたことの報告 | — | 修正 2 以降は失敗も書いた |

速さははっきり上がった．修正 3・4 は，範囲を守った，きれいな回だった．弱点は，セルフレビューの P1 に押されて仕組みを足しすぎること（修正 2）と，修正 1 までの報告の正確さ．
