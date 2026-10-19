# 1b-4d-1c：Sol 6.1 のコードレビューと再レビュー（4 回）

扱いは [1b4d1c-opus.md](1b4d1c-opus.md)．

## sol-review

判定：FAIL

## 1. major：`targets` が空でも、更新を必要とする segment は残り得る

**場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:528,620–627`、`crates/ezsdr-radio-uhd/src/provider/core.rs:328–335`

**根拠：** targets は `effective` 以後の origin だけを探します。一方、空の分岐では投影の entry を元の `e` に残し、device に渡さず、成功記録も無効化しません。

**系列（時刻は ms）：**

1. RX は channels=0。from-none の enable を100に予約し、gain=0で configure 済み。
2. RX channels=0を100.2に予約する。100〜100.2の segment は残る。
3. 97.5で、live なTXの更新を100.5へ releaseする。
4. 97.6で、RX gain=3を99.7へ予約・releaseする。
5. `effective=100.5`。RXは既にchannelなし、その先のsegmentもないため、targetsは空。
6. RX ownerが100のsegmentをbeginする。

RXの投影は `99.7 <= origin=100` なのでgain=3。しかし成功記録のset・commandが一致し、ownerはconfigureせず、deviceはgain=0のままです。拒否・loss・容量超過はありません。

これは**ソースから導いた未実行の帰結（INFERRED）**です。HEADの記録だけの分岐には、この投影の版を進める処理がありました。今回、その保護がなくなっています。UR-36の配送説明（`design/18-uhd-radio.md:271`）も、この系列を満たしません。

**修正：** この系列を追加して配送・投影の扱いを裁定してから、版削除を確定してください。`targets.empty()` を「更新を必要とするsegmentが存在しない」と扱ってはいけません。元の`e`で投影を保持するなら、既にbeginしたownerにも頼らず値を届ける必要があります。成功記録の無条件な信頼だけでは閉じません。

## 2. major：S3は時計を固定しておらず、正しい実装でも時間依存で失敗する

**場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:2660–2694`

**根拠：** S3には`hold`も`Frozen`もepoch設定もありません。それでも`:2694`でdeviceの`effective == 50 ms`をassertします。FakeDeviceは`:1060`で `max(at, last_command, device_now)` を使います。

**系列：** rig作成からowner threadのparticipant登録までに50 ms以上遅延すると、device時計は50 msを越えます。ownerのprobeで時計が止まっても既に遅く、正しいtimed applyのログが期待と一致しません。**負荷下の発生は未実行（INFERRED）**ですが、時計制御の欠落はコードで確認できます。

報告の「S3にholdを追加した」（`report.md:23,73`）とも食い違います。

**修正：** 最初のbookingより前に、他の系列と同じ`hold(&core, &time, &device)`を保持してください。これは最新裁定§2で受理済みの修正です。

## 3. minor：TX主系列のread-backで二つの時計が一致していない

**場所：** `crates/ezsdr-radio-uhd/src/provider/tx.rs:1993–1995`

**根拠・系列：** Coreはrelease時の47 msのまま、deviceだけを`origin + 1 ms`へ進めてread-backしています。assertしているのもdevice時計だけです。RX側の`device_at`と違い、最新裁定§2の同期条件を満たしません。

**修正：** read-back前にCoreも同じ時刻へ進め、`(core.now(), device.time_now()) == (at, at)`をassertしてください。現状の検査は、同期したorigin後の観測ではなくdevice単体の観測です。

## 4. minor：UR-24の容量超過条件が、受理した制限を正しく記述していない

**場所：** `design/18-uhd-radio.md:253`

**根拠：** 文はcold changeがcommandのinstantとreleaseの間に「in effect」となる場合を挙げます。しかし、受理した反例は次です。

- channelなしで10 msの更新を17個予約。
- その後、100 msのenableを予約。
- 7.5 msにrelease。

100 msのcold changeはその区間で発効しませんが、pending targetsを増やして容量超過を起こします。

**修正：** 「timed booking後に予約されたcold changeが、release先のchannel集合を増やし得る」と記述してください。規範本文には動作、理由・既知の制限はChanges／裁定への参照として残す形が適切です。

---

**確認済み：** live経路のtimed apply・行・channelごとの会計、所有権／lock経路、構成版の参照削除、parityの`note`除外とdifferentialの`issued || note`。P405–P424と報告された既存13行は、置換元が各1か所に一致しています。ただし、これらに指摘1の系列はありません。

**未実施：** ビルド・テスト・mutation・実機確認。killed結果は再検証していません。ファイル変更なし。

## sol-rereview

判定：FAIL

元の4件は、指定された系列については修正済みです。ただし、`effective` への移動に隣接する2経路が残っています。以下の反例はソースから導いた未実行の帰結（**INFERRED**）です。

## 1. major：新しいsegmentは、移動した更新を早すぎる基底値として取り込む

**場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:435–451,482–489,628–636,852–855`

**根拠：** 既存投影のentryは`effective`へ移動しますが、直後に`self.config`へ値を無条件に書きます。以後の`segment_config()`はその値を基底設定に使います。更新は既に`held`から除かれているため、新しい投影には、その値を`effective`まで遅らせるentryがありません。layout側の`candidate`も同じ`config_at()`を読みます。

**系列：**

1. P425と同じ系列で、RX gain=3が`e=99.7 ms`から`effective=100.5 ms`へ移動し、記録だけになる。
2. その後、RXのenableを100.3 msに予約する。
3. 新しいsegmentの基底gainは3となり、origin=100.3 msで既に3を投影する。

既存の100 ms segmentは0、新設した100.3 ms segmentは3となります。同じ更新について、確定した100.5 msという適用時刻が新しい投影では失われます。ownerのconfigureもその3を読みます。実機での待ち・開始遅延の具体的結果は未検証です。

**修正：** 記録した更新も適用時刻付きで保持し、既存投影だけでなく、後から作る投影・from-noneのconfigureも、そのoriginに対応する値を読むようにしてください。`config.insert`を維持するなら、それを時刻非依存の基底として読む経路を直す必要があります。

## 2. major：移動前にconfigureしたownerとの不一致は、同じlockでも防げない

**場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:133–150,594–599,628–635`、`crates/ezsdr-radio-uhd/src/provider/core.rs:816–830`

**根拠：** booking時点で更新は元の`e`としてownerへ公開されます。`updates`のlockはconfigureと移動を直列化しますが、**configureが先に終わる順序**を禁止しません。後からentryだけ移動しても、既にdeviceへ渡した設定と成功記録は修正されません。

**系列：**

1. RXに100〜100.2 msのsegmentを予約する。今回はfrom-noneでconfigure済みにはせず、ownerがconfigureする状態にする。
2. TX commandを100.5 msへreleaseする。
3. RX gain=3を99.7 msへbookする。
4. RX ownerが先に`updates`を取り、100 msの設定としてgain=3をconfigureする。
5. RX releaseが後からlockを取り、entryを100.5 msへ移す。targetsは空なのでdeviceへは何も渡さない。

投影の100 ms時点は0へ戻りますが、deviceへ渡した値は3、成功記録はそのsegmentのままです。新テストは最初からgain=0でconfigure済みのownerだけを通るため、この順序を検出しません。

**修正：** 移動前の投影を使ったconfigureも整合させてください。例えば、ownerへ公開する前に適用時刻を確定するか、移動が既存configureへ与える影響を追跡して補正する必要があります。版を戻すだけでは、既にbeginしたownerまでは救えません。probeでこの順序を固定するテストが必要です。

## 元の4件の確認

| 元の指摘 | 結果 |
|---|---|
| major 1：記録だけの枝でentryが元の`e`に残る | **元の系列は解消**。`:628–635`で全取得投影へ`issued()`を適用。ただし上記1・2が残る |
| major 2：S3の時計固定なし | **解消**。`control.rs:2662`の`hold()`は最初のbooking前。helperはepochと両時計をassertする |
| minor 3：TX read-backの時計不一致 | **解消**。`tx.rs:1992`から共有`device_at()`を呼び、両時計の一致をassertする |
| minor 4：容量超過条件の説明 | **解消**。`design/18-uhd-radio.md:253`はbooking後のcold changeによるchannel増加を記述する |

## 新テスト・P425・その他の隣接経路

- `control.rs:3054–3089`は、entryの新しいkey、100 msの投影gain=0、記録行、ownerのconfigure省略、deviceの旧値を実際にassertしています。ただし、他の投影・layout・新設segment・configure先行順序は通りません。
- `plan/maintenance/tools/mutations.json:7875–7882`のP425は置換元が**1か所**、対象テスト定義も**1か所**です。`effective`を`e`へ変えると、`:3080–3083`のkey・gainのassertを破ります。実行によるkillは再検証していません。
- `cancel_held()`が移動後のkeyを探さない点は問題ありません。移動したcommandは既に`held`から除かれており、取消対象ではありません。別のheld commandは固有の`seq`で区別されます。
- 現段階は各方向stream 0だけです。将来の複数stream対応までは保証していません。

## `design/18`の確認

UR-24の「記録だけの更新もeffective時点に投影する」という変更は、今回の修正意図と一致します。しかし上記1・2により、`:254`の適用時刻と`:271`のUR-36の配送説明を、全経路で満たしたとは確認できません。説明を弱めるのではなく、まず実装とテストを閉じるべきです。

**未実施：** ビルド・テスト・mutation・実機確認。ファイル変更なし。

## sol-rereview2

判定：FAIL

前回の major 2件は原因が除去されています。ただし、隣接経路に **major 2件、minor 1件**があります。以下の実行結果は未実行の推論（**INFERRED**）です。

## 1. major：畳み込みの順序が逆転し、新しい値を古い値で上書きする

**場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:637–658,664–665`

**根拠：** channel のない held command はその場で `of_record()` へ渡します。一方、過去に release した command は、held の処理後に `released_at` から畳みます。両者を通した `(e, seq)` 順になっていません。

**系列：**

1. RX に channel があり、101 ms で全 channel が抜ける変更を予約する。後続の enable はない。
2. gain=1、`e=100 ms` の A を 97.5 ms に release する。A は `released_at` に残る。
3. 同じ key に gain=2、`e=102 ms` の B を予約する。
4. 制御 thread の次の release が 102.5 ms になる。
5. B は channel がなく、`:639` で `config=2` になる。
6. 続いて A が `:658` で畳まれ、**`config=1` に戻る**。

この後に予約する enable の基底設定は1です。B の Manifest 行は2、既存投影も2ですが、新設 segment は1を configure します。layout の同一 channel key でも同じ構造です。

**修正：** channel なしの settlement と成功した release の settlement を、共通の `(e, seq)` 順で畳んでください。単に `released_at` の loop を前へ移すだけでは、同じ pass で新たに release する command との順序が残ります。上記系列で、最終 `config_at` と、その後に作る segment の configure を検査するテストが必要です。

## 2. major：別 thread が検出した loss を、release の channel 間で確認しない

**場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:617–619,675–685`  
**比較：** `crates/ezsdr-radio-uhd/src/provider/core.rs:730–737`

**根拠：** `is_lost()` は release の入口だけです。`give()` は、各 `apply()` 自身がエラーを返さない限り、残りの channel を呼び続けます。通常の `Core::configure()` が持つ channel ごとの loss 確認がありません。

**系列：**

1. 2 channel への timed release を開始する。
2. channel 0 の apply が成功する。
3. RX/TX thread が loss を検出し、`device_lost()` が loss を公開する。
4. `give()` が channel 1 の apply を新たに開始する。

`UhdDevice::apply()` も入口で lost を拒否せず、UHD 呼出しへ進みます（`crates/ezsdr-radio-uhd/src/uhd.rs:818–827`）。UR-29 の「loss 後は device を使わない」を満たしません。これは既存の欠落が今回の `give()` にも残ったものです。

**テストの不足：** `control.rs:3119–3135` は、channel 0 自身が lost error を返す系列です。成功した呼出しの間で**別 thread が公開する loss**は検査していません。

**修正：** 各 channel と次の command の呼出し前に loss を確認してください。channel 0 の成功後に loss を公開する gate/probe テストで、channel 1・後続 command が呼ばれないことを固定してください。

## 3. minor：UR-24 に追加した理由説明が DA-1 形式ではない

**場所：** `design/18-uhd-radio.md:251`

booking floor の規則に続く「so that … a configuration projected … never has to move its instant」は、規範ではなく今回の設計変更の理由です。booking floor と `LATE_COMMAND` の説明も `:250–251` で重複しています。

**問題となる場面：** 次の変更時に、適用時刻の規則と今回の修正理由を同じ規範段落で保守することになります。

**修正：** floor・通知時点の規範は一度だけ記述し、投影を動かさなくてよい理由は Changes が指す decision record へ移してください。既存部分全体の書き直しは不要です。

## 指定項目の確認

- **前回 major 1・2：解消。** `e` は booking の floor で確定し、`ColdConfig::issued` と release 時の entry 移動は消えています。owner configure を booking→release の間に置くテストは `control.rs:3324–3348` にあります。
- **booking floor の不変性：** held の先頭待ち、拒否による途中 return、両方向を通して、後の command が先に release される経路は見つかりませんでした。部分成功分も `released` に残ります。ただし loss 後の呼出しは指摘2です。
- **待ち・会計・取消：** 待ちは held に残り、layout は一枠、current keys はその booking 時の `targets` 数で数えます。取消も同じ held を対象とします。待ちによる永久滞留・二重計上は見つかりませんでした。
- **`config` の読者：** 未畳み込みの released 値は `config_at()` と `held_updates()` が補います。UR-12 の candidate、layout、後続投影への入口はあります。ただし settlement 後の基底値が指摘1で壊れます。
- **新テスト：** `control.rs:1385–1502` は待ち・順序・取消を、`:3167–3348` は booking floor・投影時刻・configure 先行順序を観測しています。一方、**同じ key の released／no-channel 混在 settlement**はありません。
- **P426–P432：** 現存する P426・P427・P428・P430・P431・P432 は置換元が各1か所です。P429 は削除済みです。P430/P432 はどちらも channel なしのテストを使うため、指摘1の混在順序を固定しません。kill は再実行していません。
- **design/18：** UR-26・UR-30 の waiting／cancel／記録の説明は実装と整合します。UR-36 の後続 segment への適用保証は指摘1により未達です。仕様を弱めず実装を直すべきです。

未実施：ビルド、テスト、mutation、実機確認。ファイル変更なし。

## sol-rereview3

判定：PASS

`7968d98` からの差分を確認しました。前回3件は解消されており、追加の阻害指摘はありません。

1. **前回 major 1：解消 — settlement の順序**
   - `crates/ezsdr-radio-uhd/src/provider/control.rs:643–666`：channel なしも `released_at` に入り、唯一の最終 loop が `(e, seq)` 順に畳み込みます。
   - `control.rs:2956–2987`：指定の A＝100 ms／B＝102 ms の系列で、最終値2と後続 enable の device read-back＝2を検査しています。P432・P433も対応しています。

2. **前回 major 2：解消 — channel 間・command 間の loss**
   - `control.rs:626–628,688–690`：各 command と各 channel の前で loss を確認し、追加の device 呼出しを止めます。
   - `control.rs:3187–3251`：別 thread による loss 公開を gate で同期し、入口の attempt 数、後続呼出し・行の不在を検査しています。P434・P435がそれぞれの検査に対応しています。

3. **前回 minor 3：解消 — UR-24 の理由説明**
   - `design/18-uhd-radio.md:250–251,621`：指摘した理由は Changes に移り、floor と通知の重複も除去されています。UR-29の loss 規則・テスト参照も追加済みです。

**隣接経路の確認**
- refusal／loss による途中 return は、既存の `released_at` を失いません。未畳み込み分は `config_at()` と `held_updates()` が補います（`control.rs:449–455,488–495`）。
- lost 中の Action は実行 loop が booking せず、release も入口で停止します（`:301–305,617–619`）。
- 本番コードの `self.config` 更新は `of_record()` の1か所です。直接読む `segment_config()` も `held_updates()` を併用しています。
- mutation 2リスト・計959行の置換元は、すべて各1か所に一致しました。

未実施：ビルド、テスト、mutation、実機確認。実行時の成立は **INFERRED** であり、kill 結果は独立再検証していません。ファイル変更なし。
