# 1b-5b：orchestrator のレビュー，裁定，数

## 1. 何をしたか

1b-5（今の key を消す切り替え）の 2 つ目．MockRadio の今の key のモードを消した．

- `profile.rs` の `Mode::Current` と `Profile` の今の key の腕（`ideal` の `DeviceDescription` の literal を含む），`lib.rs`・`profile.rs` の `is_layout()` の分岐，`channel.rs` の方向全体の channel model を消し，Mock はいつも layout の description で動く．本体は差し引き約 −350 行．
- selector の `keys` は，無いか `"channel"` なら受け付けて何もしない（UHD は 1b-5c まで `keys: "channel"` で layout を選ぶ）．他の値は拒否．`rx`・`tx` は常に受け付ける．
- Mock と acceptance のテスト 93 本の本体を layout の key と port `rx<k>` に移した．消したテストは無い．acceptance の layout の selector・profile の式は `crates/ezsdr-acceptance/src/layout.rs` に 1 つ．rig は `keys: "channel"` を付けて Mock と UHD の両方を layout にした．
- VH-8 の differential（`rm_26_the_timeline_against_mockradio`）は，方向全体の channel 数の操作を `radio.<d>.c<n>.stream` の変更（抜ける・戻る・RM-34 の move・空の stream への join）に置き換え，2 channel・2 stream の layout で timeline の `Lines` と比べる．1,000 seed で leave 2,634，return 1,495，move 2,148，empty join 2,161 を生成し，各数が 0 でないことを assert する．
- spec 09（MR-2，MR-3，MR-5〜MR-7，MR-18，MR-32，MR-34，MR-38，テストの表，Changes）．spec 07・18 は 1b-5c．

実装は Luna，セルフレビューは DeepSeek（2 回：P1 2 → なし）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | Luna，15:39〜19:47（OpenAI の 5 時間枠の上限で 17:43〜17:56 中断，同じ session で再開） | セルフレビュー 2 回 |
| Opus | 指示書への忠実さ，変えてはならない挙動 | VH-8 の範囲の懸念を Sol に回した |
| Sol のコードレビュー | 自分の失敗の筋書きから | **FAIL**，major 1・minor 2 |
| 修正の回 1 | Luna，19:53〜21:21 | P162 killed，differential 3 回とも同じ |
| Sol の再レビュー | 直したところと隣の系列 | **PASS** |
| gate | §4 | P528 が生き残り（§6），orchestrator が直した |

## 3. 指摘と扱い

- **Sol 1（major）：VH-8 の differential から channel の集合の変化が消えた**．Luna は今の key の方向全体の channel 数の操作を「layout に 1 対 1 の key が無い」として外し，代わりを足していなかった（Opus も同じ懸念を持って Sol に回した）．所属の操作で置き換えた（§1）．
- **Sol 2（minor）**：`mr_18_a_disable_that_arrives_after_a_later_change_follows_it` が逆向きの reentry の系列に書き換わっていた．元の系列（先の rate 変更の後へ disable が繰り下がり，新しい送信の clock が自分の origin で終わる）を元の名前で戻し，reentry のテストは別に残した．
- **Sol 3（minor）**：P162 の `old` は消えていなかった（消えたのは mutant の条件だけ）．行を戻し，`sim.channel` を一律に拒否する mutant に当て直した．
- **process**：Luna が orchestrator の見張り（`watchdog_proc.py`）を自分の背景の job と取り違えて止めた．修正の指示書に「見張りは止めない」と書き，その回は守られた．指示書は書き換えられなかった（1b-5a の後に足した禁止が効いた）．
- **1b-5c へ送るもの**：spec 07 が今の key の `radio.rx.coherent` を記述している（`design/07-radio-model.md:119`）．spec 07 が引く `mr_08_effective_holds_exactly_the_ten_configuration_keys` は，名前のまま layout の 5 つの key を確かめている．どちらも spec 07 を直すときに名前ごと直す．

## 4. gate

テスト 1570，clippy，`rules.py` 0 件，Python 3.14・3.9 各 56．docker の uhd は省いた（UHD の crate は変えていない）．mutation の全リスト：§6．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| セルフレビュー（DeepSeek，2 回） | P1 2 | P2・P3 数件 | VH-8 の削除，存在しない target．テストの範囲．機構なし |
| Sol | 1 | 2 | テストの範囲と mutation の行の扱い．機構なし |

`failure-mechanisms.md` に足すものはない．

## 6. mutation の全リスト

全リスト：maintenance 893/894，phase3〜phase6 全部，phase7 164/165（G09，既知）．生き残りは 1b-5a の **P528**（server の既定の profile の `"keys": "channel"` の行を消す mutant）：Mock が `keys` の無い selector も layout として受け付けるようになったので，Mock だけを使う既定の profile のその行は働かない．orchestrator が行と spec 16 の EA-9 の例からそれを消し，P528 を消した（`keys` は 1b-5c で全体から消える）．catalogue を `file` に持つ 7 行（P529，P530，V07，R09，B36〜B38）を流し直して 7/7 killed，server 43・Python 3.14・3.9 各 56・acceptance を流し直した．
