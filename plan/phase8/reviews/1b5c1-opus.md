# 1b-5c-1：orchestrator のレビュー，裁定，数

## 1. 何をしたか

1b-5（今の key を消す切り替え）の 3 つ目．1b-5c を 2 つに分けた前半で，UHD Module の今の key のモードを消した．

- `provider/mode.rs` の `Mode::{Current, Layout}` と両方の façade を消し，`UhdRadio` と `Core` は binding の layout から作る `channels::Description` を直接持つ（要る操作だけを `DescriptionExt` に残した）．`control.rs`・`core.rs`・`mod.rs`・`tx.rs` の `is_layout()` の分岐，`current_key_direction`，今の key の channel 数の検査（UR-12），方向全体への timed update の fan-out と queue の会計，今の key の floor，`profile.rs` の今の builder を消した．front end の照合は layout の 1 つの腕だけになった（1b-4c の O5）．
- selector の `keys` は，無いか `"channel"` なら受け付けて何もしない．他の値は UR-5 で拒否．`keys` そのものは 1b-5c-2 で消す．
- UHD のテストを layout の key と stream の node，port `rx<k>` に移した（`tests/fake.rs` 66，`control.rs` 62，`tx.rs` 2，`tests/hardware.rs` 4 本の直接参照）．今の key だけの規則のテスト 11 本を消し，同じ規則を見る layout のテストを報告の表に挙げた．新しいテスト 3 本（実行時の番号の無い key の拒否，release の前に publish された loss，release の apply が返す loss）．
- control の differential は channel 数の操作を c0・c1 の所属の変更に展開した（1,000 seed で 2 channel になる変化 495，2 channel から減る変化 172，どちらも 0 でないことを assert）．
- spec 18（UR-5，9，12，15，17，20，23〜26，28，30，34，§6，Changes）．spec 07 と Vision は 1b-5c-2．

本体は差し引き減（`mode.rs` −192，`mod.rs` −57，`control.rs` の本体と `core.rs`・`tx.rs` も減）．diff 全体は約 3,900 行で，見積もり 1,500 行を大きく超えた．大半はテストの移し替えと mutation の行の当て直し．Luna は 2,000 行を超えた時点で報告に分割点を検討し，`Mode` を消すとテストを同時に移さないと green を保てないとして続けた（妥当）．

実装は Luna，セルフレビューは DeepSeek（2 回：P1 1 → なし）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | Luna，2026-10-21 21:36〜10-22 04:22（OpenAI の 5 時間枠の上限で 02:52〜03:57 中断，同じ session で再開） | セルフレビュー 2 回 |
| Opus | 指示書への忠実さ，変えてはならない挙動 | O1（major），O2（minor） |
| Sol のコードレビュー | 自分の失敗の筋書きから | **FAIL**，major 3・minor 1（O1・O2 と一致するもの 2） |
| 修正の回 1 | Luna，04:36〜05:19 | 変えた mutation の 7 行 7/7 killed |
| Sol の再レビュー | 直したところと隣の系列 | **PASS** |
| gate の 1 回目 | 全リスト | 1252/1262：この段階で壊れた 9 行（§6） |
| 修正の回 2 | Luna，05:31〜05:49 | 8 行を直して 7 killed，P424 は equivalent（orchestrator の判断） |
| gate の 2 回目 | §4，§6 | 1259/1261（P472 は負荷で揺れる kill，G09 は既知） |

Sol は 2 回とも 1 回目の起動で止まった（出力なし，session も作られない）．PID で止めて起動し直した．

## 3. 指摘と扱い

- **O1・Sol 2（major）：送信の from-none の 2 本を代わりなしで消した**．`ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again` と `ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing` の系列（B = {0,1} → 空 → C = {0}）は layout でも起きる．Luna は「受信側が同じ判断を見ている」として消していた．元の名前で layout の所属の変更に移した．同じ時刻の leave 2 本で長さ 0 の中間 segment {1} ができ，送信の owner がそれを飛ばす `tx.step()` が 1 つ増えた（Sol が timeline と owner のコードで正しいと確かめた）．中身が同じだった `tx_channels_from_none` と `tx_enable_from_none` は channel の集合を受け取る 1 つの helper になった．Sol の 2 の続き：**P370 は P369 の重複ではなかった**（filter が受信と送信で違う）．戻して送信のテストに当てた．
- **O2・Sol 1（Sol は major）：本体に `#[cfg(test)]` だけの道ができた**．`ColdConfig` に `test_base`・`test_dir` と `settings_on` の `cfg(test)` の分岐が足され，P279 はテスト専用の射影を変異させていた．消して，テストも本番の `settings_on` → `configure` を通るようにし，P279 を本番の origin の境界に当て直した（device の read-back で kill）．
- **Sol 3（major）：control の differential が 2 channel への変更を何もしない操作に縮めた**．channel 数 1 と 2 をどちらも `c0.stream = 0` に写していた．c0・c1 の所属に展開し，2 channel の変化の数を assert した（§1）．何もしない channel 数の変更は，実行側でも何もしない `c0.stream` の更新として残し，instance 全体の cold の floor を期待側と揃えた．
- **Sol 4（minor）**：spec 18 の UR-9 の冒頭に `DeviceDescription` が残っていた．直した．
- **process**：指示書と checklist は書き換えられず，見張りも止められなかった（1b-5a・1b-5b の後に足した禁止が効いた）．
- **gate の mutation（§6）**：Luna は自分で変えた行しか流さず，`booked_stream` を `#[cfg(test)]` にしたことや `self.core.mode` が消えたことで compile できなくなった mutant，filter のテストがもう通らなくなった行を見落とした．指示書は「当て直した行」を流すとしか書いていなかった．次の指示書では，消したり `#[cfg(test)]` にしたりした名前を `new` に持つ行と，消したり移したりしたテストを `filter` に持つ行も grep して流すよう書く．
- **1b-5c-2 へ送るもの**：1b-5b からの 2 つ（spec 07 の `radio.rx.coherent`，`mr_08_effective_holds_exactly_the_ten_configuration_keys` の名前）はそのまま．UHD の `keys` の受け付け（`provider/mod.rs` の `channel_keys` の呼び出し）．

## 4. gate

テスト 1562（HEAD の 1570 から消した 11 を引き，新しい 3 を足した），clippy，`rules.py` 0 件，docker の uhd（clippy，lib 245，`uhd_api` 5），Python 3.14・3.9 各 56（修正の回 2 の後に Python 3.14 と残りを流し直した．回 2 は Python と server に触れていない）．`tests/hardware.rs` の 4 本は実機で流していない（docker の clippy で compile は確かめた）．mutation の全リスト：§6．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| セルフレビュー（DeepSeek，2 回） | P1 1 | P2 4・P3 数件 | テストの範囲．機構なし |
| Opus | 1 | 1 | テストの範囲（到達できる系列を消した），テスト専用の道．機構なし |
| Sol | 3（2 は Opus と同じ） | 1 | テストの範囲と differential の縮退．機構なし |
| gate（mutation） | 9 行 | — | 消した名前を `new` に持つ行の compile error 4，filter のテストが通らなくなった行 3，働かなくなったコード 2．機構なし |

「到達できる系列を消す」「differential から操作を外して代わりを足さない」は指示書 §5 に前の段階の誤りとして書いてあったが，また起きた（1b-5b の VH-8 と同じ形）．機構ではなく実装役の癖として，次の指示書でも残す．`failure-mechanisms.md` に足すものはない．

## 6. mutation の全リスト

1 回目（修正の回 1 の後）：1252/1262．G09 は既知．残る 9 行はこの段階で壊れた．
- compile error 4：P446，P450，P464 の mutant は `booked_stream`（`#[cfg(test)]` になった），P469 は `self.core.mode` を使っていた．`stream_key(<dir>, 0)` と `self.core.description.stream_channels(...)` に書き換えて killed．
- 生き残り 3：P265 は filter のテストがもうその行を通らなかった．同じ instant の timed update と from-none の enable を見る `ur_25_an_enable_at_a_timed_update_s_instant_applies_it` に当て直して killed．R04 は layout の cold が `layout_instant` を通るようになり，filter が `command_instant` を通らなくなった．`start_rx` の unit test に requested instant と `Kind::Start` の `e` の一致の assert を足して当て直し，killed．P424 は equivalent（行の `equivalent` に理由：timed command はいつも 1 channel を名指し，`give` の loop はちょうど 1 回回る）．
- 働かなくなったコード 2：P282（`channel_keys` の `None => Ok(true)`．UHD も Mock も戻り値を使わない．1b-5c-2 で関数ごと消える）は消した．R24（CBX の既定 2.45 GHz）は production から呼ばれなくなった `x3xx::description` から，layout の `channels.rs` の `Channel::of` に当て直して killed．

2 回目（修正の回 2 の後）：maintenance 889/890，phase3〜phase6 全部，phase7 161/162（G09，既知）．**P472**（`ur_26_two_receive_streams_end_at_their_own_stops`，1b-4d-2 から）は survived だったが，単独で 3 回流すと 3 回とも killed．これまでの全リストを合わせると 9 回中 1 回だけ survived．stream 0 の次の packet が 50 ms 遅れるのと abort の順に頼るので，8 shard の負荷で揺れる．この段階では変えていない．`delivered` は UHD の stream ごとの状態をまとめる段階で扱う field なので，そこへ送る（handoff）．
