# Phase 8 計画と設計 33 第2版：Opus の独立レビュー（2026-10-16）

Claude Opus（サブエージェント，effort max，読むだけ，`plan/phase8/reviews/` は読まない条件）．

## 結論

**条件つきで Gate P に進めてよい。** owner が受理した P1〜P5・D1・D9・D13 を覆す指摘はありません。ただし、設計 33 の中で次の3つを先に直す必要があります。

- **P0（1件）**：D8 の ADC 利得の拘束。このままだと実行中に利得を変えられなくなる。
- **P1（11件）**：設計の欠け、段階の穴、文書一覧の漏れ、測定の定義。
- **owner の判断が要るもの（2件）**：P1-5 の antenna の update class と、P1-10 の D2 の Kernel 変更。

## 指摘

### P0

**P0-1 D8：ADC 利得の拘束で、Basic/LF の利得が実行中に変えられなくなる**
- 対象：設計 33 D8 の第1項（`plan/phase8/33-channels-and-streams.md:141`）
- 内容：
  - X300 の受信 ADC の digital gain は slot に1つで、chA と chB に同じ値を書きます。UHD は各受信 channel の gain group に "ADC-digital"（優先度 0）を入れます。
  - D8 は Basic/LF の利得 key を「ADC 利得だけ」にし、同じ slot の2 channel で値が違えば拒否するとしています。
  - 一方、実行中の更新は1回に1 key しか変えません（KC-25、KC-26）。同じ slot の BasicRX 2本は、どちらを先に変えても「両立しない」で拒否されます。Session でも schedule でも二度と変えられません。
- 推奨：Phase 8 では ADC 利得を出さない。front end の利得範囲は RF 段だけにする（UBX・OBX・CBX は今の 0〜31.5 dB、Basic/LF は送受とも 0 のみ）。UHD は優先度の高い RF 段から埋めるので、ADC は 0 のまま触られず、channel 間の拘束が消えます。
- 根拠：
  - `design/06-kernel-coordinator.md:331,333`
  - UHD v4.10.0.0：`host/lib/usrp/x300/x300_adc_ctrl.cpp:78-86`、`x300_radio_control.cpp:1302-1308, 1720-1737`、`host/lib/utils/gain_group.cpp:113-124, 179-185`
- 判定：コードは VERIFIED。拒否の連鎖は規則からの一段の推論（INFERRED）。

### P1

**P1-1 rate の違う stream を持つ resource では、root 宛ての schedule 項目が arm で拒否される**
- 内容：
  - SpecTime の clock は「宛先と同じ stream の clock」、それがなければ「全 stream が同じ比」のときだけ決まります（SB-16、`crates/ezsdr-kernel/src/coordinator/pipeline.rs:1010-1036`）。
  - D1・D11 では channel の設定はすべて root 宛てです。そのため、rate の違う stream があると、timed retune の `UpdateParameter` も root への `Stop` も `KC-16: ambiguous` で Failed{arm} になります。
  - stream の node を宛先にして避けようとしても、MockRadio は device 以外の宛先を拒否します（MR-18、`design/09-mock-radio.md:140`、`crates/ezsdr-mock-radio/src/lib.rs:1293-1296`）。
- 推奨：Kernel は変えない。radio Provider が `UpdateParameter` と `Stop` を `<r>/rx/s<k>` 宛てでも受けるようにする（何を変えるかは key が決め、node は SpecTime の clock を選ぶだけ）。RM-12、MR-18、UR-24/26 に書く。
- 判定：VERIFIED

**P1-2 流れていない DDC/DUC への timed command は装置を full restart が要る状態にする**
- 内容：
  - X3x0 の HG/XG image では、DDC/DUC は流れる packet の header から時刻を得ます。stream していないと timed command が溜まり、あふれると full restart が要ります（UHD `host/docs/devices/usrp_x3x0.dox:863`）。
  - 設計 33 は「どの stream にも入らない channel」（D3 の既定 -1）と「channel のない stream」（D4）を作ります。さらに UC-6 により、channel ごとの `set(frequency)` は常に timed です。
  - UR-24 が「装置に送らない」としているのは「channel なし」と「halt」だけで、停止中の stream と stream 外の channel は含みません（`design/18-uhd-radio.md:242-246`）。
  - 調査メモ（`x300-uhd-facts.md:60`）はこの記述を引いていますが、設計と計画には入っていません。Mock は模擬しないので、§59 でいう「Mock が緩い」に当たります。
- 推奨：`ezsdr_radio` に1か所で書き、両 Provider が使う。内容は「効く時点で run していない stream の channel への `hardware_timed` 更新は、記録だけして装置に送らず、次に run するとき適用する」。測定スイートでこれを試してはいけません（装置を止めます）。
- 判定：文書は VERIFIED。AUTO tune が DDC/DUC へ timed write を出すかは INFERRED。

**P1-3 UHD Module の thread と lock の形が決まっていない**
- 内容：
  - spec 18 は uhd-rx と uhd-tx が1本ずつで、各 thread が streamer を1つ専有し（`design/18-uhd-radio.md:108`）、plan の lock も hot path の handle も1つ、という前提です（UR-20、UR-25：`:219,247`）。
  - stream が N 本になると、次が決まっていません。
    - stream ごとに thread を持つか。
    - D3 の「移動元が離してから移動先が取る」（`33-channels-and-streams.md:87`）を、thread 間でどう順序づけるか。
    - 2本の line への記帳と「両方 halt」を原子的にどう行うか。
- 推奨：設計 33 に1段落で決める。案は、stream ごとに data thread を1本、instance の全 line を1つの lock の下に置く（移動はその下で2 line に記帳）、hot path の handle は stream ごと。
- 判定：文書は VERIFIED、推奨は INFERRED。

**P1-4 D3 の移動は `ezsdr_radio` に1か所で実装すると明記する**
- 内容：本文は「Provider の中の1つの移動の操作として計画する」（`33-channels-and-streams.md:83`）で、Mock と UHD Module が別々に実装する読み方ができます。`mech:radio-rule-implemented-twice` は 9件あり、three strikes を超えています（`plan/maintenance/failure-mechanisms.md:10`）。
- 推奨：「同じ床の instant で2本の line に記帳し、拒否なら両方 halt する」関数を timeline に置き、両 Provider がそれを呼ぶと書く。
- 判定：VERIFIED

**P1-5 Session では antenna を変えられない（owner の判断が要る）**
- 内容：
  - antenna の key は update class を持たないので（RM-21、`design/07-radio-model.md:235`、`crates/ezsdr-radio/src/lib.rs:190`）、Session の `set` は KC-24 step 5 で拒否されます。
  - Session の暗黙の Spec は `requires` が空です（`design/03-spec-and-binding.md:239`）。
  - その結果、設計 33 の目標2（BasicRX の A/B を実数2系統で受ける、`:15`）も、D11 の `rx_channels[n].antenna`（`:171`）も、Python からは実現できません。Basic/LF では antenna が「実数 A/B か複素 AB か」を決めるので、混在構成の主目的に直接当たります。
  - EA-16 は今も、必ず拒否される setter を公開しています（`design/16-easy-api.md:83`）。
- 推奨：channel ごとの antenna を `cold` にする。代案は front end ごとの既定値（fe0→A、fe1→B）だけを決めること（複素 AB は Spec Run でしか使えなくなる）。
- 判定：VERIFIED

**P1-6 `ideal` profile の形が決まっていない**
- 内容：設計 33 は「残す」とだけ書いています（`:176`）。`ideal` は master clock なし、0〜64 channel、任意の整数 Hz です。次が決まっていません。
  - motherboard＋front end＋layout の形でどう書くか（layout が要るか、channel 数、既定の membership、port の数）。
  - `x3xx` の selector に `rx`/`tx` がないときの既定 layout。
- 影響：mock_radio.rs で 64か所、ほかに v58、sim-engine、Python のテスト（profile 名だけを替えるもの、`python/tests/test_easy_api.py:215-217`）が `ideal` を使っています。
- 判定：VERIFIED

**P1-7 段階 1b-2〜1b-5 で tree が緑のまま保てない**
- 内容：
  - 1b-3 で Mock が「新しい key で動く」一方、server、Python、fixture は 1b-5 まで古い key と port `rx` を使います（`33-channels-and-streams.md:207-214`）。
  - 1つの instance が新旧両方の key を受けると、1つの設定が2つの key で `effective` に載ります（fact-stated-twice と同じ形）。
  - 1b-2 で `tree()` を変えると、tree の完全一致を見る `rm_26_the_description_builds_the_tree_mockradio_built` が落ちます（`design/07-radio-model.md:330`）。
- 推奨：1b-2〜1b-4 は instance ごとに旧 key か新 key のどちらか一方だけ（selector に layout があれば新）。新しい tree は別の builder で作り、1b-5 で旧を消す。
- 判定：段階の文面からの INFERRED

**P1-8 書き換える文書の一覧が足りない（AGENTS.md §9 の checklist）**
- 内容：設計 33 で偽になるのに、一覧にない文があります。
  - Vision §8 の例、「one key per direction」の文、`instances: 2`（`design/vision/02-spec-binding-and-compilation.md:28,54,89`）
  - Vision §13 の grid の文（`03-simulation-mock-and-time.md:71`）
  - Vision §26 の「UBX/CBX/SBX は timed tune で位相差が一定」と2値の capability（`05-coherence-calibration-and-runtime-semantics.md:114,117`）
  - spec 03 の SB-3（`radio/rx/0` の例、`:148`）、SB-12（`:173`）、SB-23（`:266`）、SB-33（`:292`）
  - spec 07 の §2（`:23`）、§5 R1/R2（`:296-297`）、§7 の1〜2（`:337-338`）
  - 計画 P1 行の「Kernel は変えない見込み」（`00-overview.md:81`）
- CBX の文が偽である根拠：UHD の `host/docs/driver_usage/sync.dox:165-205` で timed LO 同期は SBX・UBX・OBX だけで、CBX は "others" です。`db_ubx.cpp:749` には `sync_phase` があり、`db_cbx.cpp` にはありません。
- 判定：VERIFIED

**P1-9 key の対応表が不完全**
- 内容：
  - D12 は設定 key 5系統の削除しか書いていません。
  - channel ごとになる capability（`phase_behavior`、step 系、RM-23 の path delay、`timed_tune`、`lo_group`）と、instance に残るものの区別がありません。
  - RM-9 の `radio.rx.coherent` の行き先もありません。stream の coherent は membership（`cold` で変わる設定）に依存するので、MA-10（Run 中 tree は変わらない、`design/05-module-api.md:161`）の下では静的な capability にできません。
  - front end ごとの既定値（RM-5、BasicRX は 1 GHz を既定にできない）も表にありません。
- 判定：VERIFIED

**P1-10 D2 を Gate P の独立した判断にする（owner）**
- 内容：D2 は Phase 8 で唯一の Kernel 変更です。それなのに、§4 の判断表は P1〜P5 だけで、P1 の受理文は「Kernel は変えない見込み」のままです（`00-overview.md:77-86`）。
- 推奨：代案「Vocabulary が c0〜cN を列挙する（Kernel 変更ゼロ、上限は凍結後も additive に上げられる）」と並べて1件として諮る。私の推奨は D2（下の「問題なし」参照）。
- 判定：VERIFIED

**P1-11 測定の境界を定義する（raw の値を profile に使わない）**
- 内容：
  - P3(4) の例「start lead 50 ms に対し実測 1 ms」は `hw_b8_raw_restart_lead`（Device を直接叩く試験）の値です。
  - Phase 7 の記録自体が「50 ms は装置の必要ではなく、遅れの原因は uhd-rx の loop」と書いています（`plan/phase7/bench-results.md:399-400`）。
  - profile の値は Module から見た値です（P1 (c)、RM-6、RM-25：`design/07-radio-model.md:103,251`）。
  - 段階3は `hw_b8_raw_*` をまとめ直すとあるので、raw の値が P3 に入ると Mock が緩くなります。
- 推奨：測定記録に「測った境界（Provider 経由か raw か）」を必須項目として入れ、比較テストと P3 は Provider 経由の値だけを使う。
- 判定：文書は VERIFIED、影響は INFERRED。

### P2（記録）

1. **KC-15 の T0 が大きく遅れうる。** T0 は全 SampleClock の格子の最小公倍数に乗ります。互いに素に近い decimation（例：N = 2040, 2032, 2024, 2016）だと、4 stream で T0 が 27.5 s 遅れ、6 stream では約2日になります（`design/06-kernel-coordinator.md:245`、計算で確認）。coerce で上限を設けるか、上限を記録する。
2. **D2 の細部。**
   - `check_constraint_kind` と `check_value_kind` は宣言側の key（`decl.key`）を報告します（`crates/ezsdr-kernel/src/binding.rs:303,321`）。D2 後は D2 自身が無効とする `radio.rx.c.frequency_hz` という形が報告されるので、具体の key を渡すようにする。
   - 重なりの検査は全 Vocabulary 横断で行う。今の registry は id の重複しか見ていません（`crates/ezsdr-kernel/src/module_api/registry.rs:47-72,194-199`）。
   - GPIO の bank＋pin は番号 segment が2つ要ります。bank を別の resource にする場合だけ1つで足ります（INFERRED）。
3. **読み手のない形（YAGNI）。** 次は Phase 8 で読むものがないので、形を文書に書くだけにして実装は見送るのを推します。
   - channel node（capability・設定・port・event のどれも持たない）
   - D6 の `timed_tune`（常に true）
   - D8 の clock 両立表（Phase 8 の board に衝突する組がない）
   - D13 の `lo_group`
   - selector の `image`/`links`（開くときの照合がない）
   - D9 の block ごとの queue。UR-24 自体が1列の queue なので、測った最悪値1つで §59 には足ります。
4. **D3 の補足。**
   - 5項は移動元の held burst しか書いていませんが、移動先も clock が替わるので、通常の cold change と同じく取り消されると明記する。
   - 6項の Manifest への記録は Provider の section に置く。Kernel の `SampleClockRecord` に channels を足すなら、2つ目の Kernel 変更になります（`schemas/sample_clock_record.v1.json`）。
5. **D13 の補足。**
   - coherent の条件が UBX＋Basic/LF の混在を外しています。
   - Basic/LF の `deterministic` は INFERRED なのに、bench に測る手順がありません。測るまでは弱い方の値を宣言する。
   - P2(2) の「厳しい向き」に phase の順序（random < random_unless_timed_tune < deterministic）を入れる。
6. **UBX・OBX の全二重では受信 antenna が RX2 に固定される**（UHD `host/docs/devices/dboards.dox:352-354,453`、VERIFIED）。bench で確かめるまでの間、Mock は緩いままです。保守的な既定値を置くのを推します。
7. **HG image の port 0 は 1 GbE。** 10 GbE の envelope のままだと、Mock が約8倍緩くなります（以前からある問題）。
8. **同じ装置への2つの binding。** UHD は同じ graph なら既存の multi_usrp を返します（`multi_usrp_rfnoc.cpp:2902-2908`）。slot ごとに binding を分けると（P1 で退けた案）、黙って共有され、subdev spec が上書きされます。serial で拒否するのを推します。
9. **開くときの照合。** 周波数と利得の範囲も比べられます（C API にある：`usrp.h:536,624`）。rate の集合は、chain が未接続だと `{1.0}` しか返らない可能性があります（`ddc_block_control.cpp:153-160`、INFERRED）。早めに FakeDevice か実機で確かめる。
10. **記録の誤り。**
    - 判断ログの「UR-3 の mcr < 2^28 は spec 18 にない」は誤りで、`design/18-uhd-radio.md:106` にあります。
    - 計画の 1a 行は「値は変えない」ですが、設計 33 では rate の格子を直す例外があります。
    - 1a に MR-34 の `random` と「同種の board だけの layout」が書いていません。
    - P3(6)「in-flight window は profile の値ではない」と P4(4)「Mock も in-flight window で判定する」が食い違います。
    - D3 の「-1 は使わない」は意味が二通りに読めます。
11. **計画の漏れ。**
    - NB-5（TimingEnvelope が受信の cold lead を公開していない）と、UR-17 の受信要求長の上限の測り直し（handoff §3）が入っていません。
    - BP0 は software が要らないので、機材が揃いしだい行う。
    - BP5（Phase 7 の機能 bench の回し直し）を BP2 より前に置く。書き直した Module で測る前に、まず動くことを確かめるため。
    - 出口条件に BP4b がありません。
12. **用語のずれ。** 受理文は「timekeeper 1つ = device node 1つ」ですが、設計 33 では root（`radio.device`）が instance を表し、timekeeper は `t<i>`（`radio.timekeeper`）です。RM-2 の意味が変わり、2台を持つ instance の1台だけを Spec が bind することもできません。owner に明示する。

## 「見つからない」とした検索

| 調べたこと | 実行した検索 | 結果 |
|---|---|---|
| DDC/DUC の時刻の問題が計画に入っているか | `grep -rn -i "sense of time\|backing up\|without streaming\|derive their sense" plan/ design/ handoff.md` | 調査メモだけ |
| thread の設計 | `grep -n "thread\|スレッド\|uhd-rx\|uhd-tx" plan/phase8/33-channels-and-streams.md plan/phase8/00-overview.md` | 0件 |
| SpecTime・T0 の扱い | `grep -n "SpecTime\|SB-16\|KC-15\|KC-16" 同2ファイル` | 0件 |
| `ideal` の形 | `grep -n "ideal" 同2ファイル` | 33:176 の1件だけ |
| UR-17 の上限と NB-5 | `grep -n "UR-17\|受信要求長" plan/phase8/00-overview.md`；`grep -rn "NB-5" plan/ design/ handoff.md` | overview に0件；NB-5 は phase8 に0件 |
| CBX の timed 位相同期 | UHD v4.10.0.0 `db_cbx.cpp` に `grep -n -i "sync\|phase\|command_time"` | 0件 |
| 宣言側の key の使用 | `grep -rn "decl\.key\|\bd\.key\b" crates/ezsdr-kernel/src` | 3件（registry.rs:198、binding.rs:303、321） |
| KeyDecl を含む schema | `$defs/KeyDecl` を schemas で検索 | `vocabulary_descriptor.v1.json` だけ |

## 確認して問題なしとしたもの

- **D2 で全経路が覆える。** KeyDecl の参照はすべて `registry.key_decl` を通ります（validation.rs:125,543、matching.rs:100、admission.rs:202,423、session.rs:287、prepare.rs:151、spec.rs:1162）。したがって SB-2 の5位置、SB-45/46、KC-24 の step 5・6・11、MA-12 を D2 が覆います。D2 は最小で健全な Kernel 変更と判断します。
- **名前の文法。** SB-T0 は `c12`、`s3`、`rx0`、`radio/rx/s1` を受け付けます。
- **D9 の「合計 ≤ 1本」は実機より緩くならない。** UHD の link 選択（`graph_stream_manager.cpp:188-284`）では各 link の負荷が合計以下だからです（VERIFIED）。
- **DDC の decimation 集合と格子の修正。** N=257 を拒否し、258 と 2040 を受理、値は 639個（VERIFIED）。
- **multi_usrp は複数 streamer を同時に持てる。** streamer 内では rate を揃えます（`multi_usrp_rfnoc.cpp:325-395`）。
- **D3 は既存の規則と整合する。** RM-16・21・25、timeline の line ごとの形、SC-29 の clock ごとの tracker と整合します。D4 で port が固定なので invariant 40 も満たします。
- **Kernel 変更は不要。** Stream Contract、TM-13a、KC-23、KC-24 の step 7、SB-15、MA-10、SB-22c/RS-12 は、Provider 側の選択（P1-1、P0-1、P2-4）だけで足ります。
- **SimulationChannel** は rate の違いを扱えます（C3）。
- **intent と binding の分離。** slot・board 名は Spec に漏れず、EA-18 にも反しません。channel の番号の約束は §3 に書いてあります。
- **持ち越し項目は計画に入っている。** #51・#59・#62、監査 D2・D3・D10、計画32 の A2/B、H1〜H5、#56、方向別の start lead、ホスト設定の script、UR-32 はすべて計画にあります。
- **v3 の引用**（`v3/cpp/uhd_usrp/multiusrp.cpp:90-217,387-412`）は存在し、内容も合っています。
