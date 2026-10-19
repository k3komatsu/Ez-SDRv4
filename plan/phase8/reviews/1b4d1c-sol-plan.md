# 1b-4d-1c：Sol 6.1 の計画レビュー（3 回）

裁定は [1b4d1c-adjudication.md](1b4d1c-adjudication.md)．

## sol-plan

**推奨：F をそのまま実装せず、適用可能な時間帯と device queue の順序を修正してから再レビューしてください。版の削除は条件付きで支持します。** origin-timed 案への単純な差戻しや、owner の版監視追加も勧めません。

以下、`P` は `/home/komatsu/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/test-plan.md`、`B` は同ディレクトリの `plan-brief.md`。`U` はコンテナ内 `/root/tmp/uhd-4.10.0.0/` です。競合・実機での帰結を実行確認していない箇所は **INFERRED** とします。

## 1. major：F は旧 segment に早く効き得る

**対象：** `P:49,230,276`、`B:24–28`

計画前に挙げた「旧 segment と新しい更新の境界」が欠けています。

例えば、同じ channel について：

1. 旧 segment は100 msまで動く。
2. 100 msで channel を外し、後で再加入する segment も予約する。
3. gain 更新の `e = 100.1 ms`。
4. 97.1 msで release。

`channels_at(e)==0` なので F は97.1 msに untimed apply します。しかし旧 segment はまだ動いています。**「e にサンプルが無い」から「e より前にも無い」は導けません。**

**期待：** 旧 segment の最後のサンプルまで旧設定を保ち、後続 segment の origin には新設定が届くこと。RX／TX、layout の leave／rejoin で固定してください。

根拠は `control.rs:628–640` の release・適用先判定と、`design/09-mock-radio.md:203` のサンプル時刻による更新規則です。実機で影響する区間の長さは **INFERRED** ですが、F が旧 stream の動作中に setter を呼ぶ系列自体は成立します。

## 2. major：S12 の結論は UHD の queue 順序と逆

**対象：** `P:13,150–160,271,273`

UHD 4.10 は untimed command についても、

> FPGA が受信し、先行 command の処理を終えてから実行する

と明記しています。

**VERIFIED：** `U/host/docs/driver_usage/timed_cmd.dox:85–105`。

S12 の系列では：

| 対象 | 実行順 |
|---|---|
| 投影 | gain=2〔9.7 ms〕→ gain=1〔10 ms〕 |
| UHD の同じ一般 command queue | timed gain=1〔10 ms〕→ 後着の untimed gain=2 |

したがって、**実機の最終値は投影と逆になる可能性があります（INFERRED）**。`P:160` の「untimed 2 が先、timed 1 が後」は UHD の説明と一致しません。

現在の FakeDevice は untimed apply が timed queue を迂回します。

- timed enqueue：`device.rs:1021–1041`
- untimed の直接変更：`device.rs:1047–1075`
- timed の実行：`device.rs:746–758`

S12 の call log と `released` だけでは、この相違を検出できません。

**期待：** queue を drain した後、後続 segment の origin で device と投影が一致すること。まず実際の queue 順序を表す double／観測を用意し、その上で順序・投影・lateness の扱いを決めてください。これは既知の #62 と接続する問題であり、F だけで解消したとは扱えません。

## 3. major：release の時間保証と S5 の観測が不足

**対象：** `B:28`、`P:49,104–108,272`

release window は「**3 ms以上前**」ではなく、コードでは「**残り3 ms以下なら release 可能**」です（`control.rs:623–629`）。booking直後の release、poll遅れ、lock・UHD mutex待ち、先行 apply の所要時間もあります。

さらに提案は、lock取得前の `now` で一度だけ late を判定します。次が未検査です。

- 判定時は間に合うが、待機中に origin を越える。
- current keys の複数 channel のうち、後の channel だけ間に合わない。
- 同じ release pass の先行 apply 中に時間が進む。

**期待：** 実際の配送時点で間に合わない更新を、成功した定刻配送として扱わないこと。時間を gate で進め、後続 channel も検査してください。

S5 の payload **`requested: e`、`applied: now + device lead` は推奨できます**。ただし、その `now` の観測点を定義する必要があります。また `e` は booking時に補正済みの場合があるため、常に「利用者が要求した instant」とする `P:108` の説明は不正確です（`control.rs:582–589`）。

## 4. major：S9 の失敗時期待は提案実装と両立しない

**対象：** `P:49,137–140`

S9(b) は「失敗した値は投影にも入らない」とします。しかし投影への挿入は、既に booking時に済んでいます（`control.rs:608–611`）。提案された error return はそれを削除しません。

そのため、release が `COMMAND_REJECTED` を出しても、後の owner configure が同じ値を適用し得ます。

**追加系列・期待：**

- release の apply拒否 → owner configure：拒否された更新を後で適用するのか、取り消すのかを明示する。
- channel 0成功 → channel 1拒否：部分適用と、後続 enable の回復結果を固定する。
- 待機中／channel間の loss：loss後に次の channel を呼ばない。

`ApplyFails` は適用前の非lost拒否だけです（`test_support.rs:336–340,367–370`）。現在のS9だけでは、これらを検査できません。

## 5. major：S8 は指定した mutation を落とさない

**対象：** `P:128–132,242`

S8 は RX の segment を拒否してから `pending()` を調べます。しかし、**拒否された RX segment は plan から除かれます**。

根拠：`crates/ezsdr-radio/src/timeline.rs:868–880`、`design/18-uhd-radio.md:271–272`。

したがって `!refused` を `true` に変えても、列挙する segment が無く、テストは通ります。

**修正：** 拒否された segment が残る TX で検査してください。RX版は「拒否によって候補が消える」規則のテストとしてなら有効です。

## 6. minor：S3・S3b・S4・S14 の検査力

### S3／S3b

**対象：** `P:79–91,250–252`

- configureの戻り直後・成功記録公開前の **probeの位置は正しい**です。ただしS2と同様、実際に owner configure を通る前提を明記してください。from-none の成功記録が一致すると、そこを通りません。
- releaseが勝手に `configured` を書く誤りは、join後の `Some(by)` では検出できません。**probeを解放する前に `configured == None` をassert**してください。
- S3b の「releaseを呼ぶ直前」の合図は、lock取得への到達を保証しません。計画自身の限界説明は正しいですが、S3／S10も `updates` の保持を検査していません。

**lockについての回答：** Fのapplyは投影を変更せず、対象segmentのconfigureも同じ投影値を適用します。正常完了し、originまでに必要なreleaseが済む範囲では、追加lockが防ぐ具体的な値喪失系列は確認できませんでした。**この追加lockとS3bは削除を支持します。** ただし「常にdeviceが最後の値になる」という全称ではなく、上記の範囲に限定してください。既存のtimed issuanceのlockまで消す理由にはなりません。

### S4

**対象：** `P:96–99`

各更新を別々にreleaseすると、誤って `released.push(e)` しても、次のbookingまでに前の行がretireできます。17個目の拒否は保証されません。また、1件ずつでは同じpassの逆順処理も検査できません。

**修正：** 各release直後の `released` 不変をassertし、順序検査は複数件を同じpassでreleaseしてください。

### S14

**対象：** `P:174–177`

**seqの取り違えを落とす系列として妥当**です。booking順を100→200→300 msと明記し、二度目のfrom-none configure後にlogの区切りを取ってください。異なるgain等も設定して結果を読むと、単なるapply回数より確実です。

## 7. minor：版の削除は支持するが、「stop後に読み手が無い」は未証明

**対象：** `P:189–193`

本番コードの版更新箇所がreleaseとcancelの2か所であること、cancelの本番呼出元がfinishだけであることは確認しました。

cold changeは新しいseqの設定を作り、open／configure失敗は成功記録を消します。pruneは記録を保持します（`core.rs:303–308,347–379,827–834`）。**配送と順序を正しく保証できれば、seq＋channel set＋成功／失敗記録で足りる方向です。版を残しても、既に始めたownerへのD1は直りません。**

ただし、ownerが既に `Planned` を取得していた場合は別です。

1. `Rx::begin` がnextを取得。
2. Provider::stopとcancel。
3. ownerが保持済みnextで `owner_configure` を実行。

`rx.rs:279–304` は、その間にstopを再確認していません。TXにも保持済みswitchがあります（`tx.rs:703–718`）。これは既存の窓で、版を残せば防げるものではありませんが、「読み手が存在しない」という削除理由には使えません。

**期待：** stopとownerの境界をgateで固定し、取消後の投影／配送に依存して新segmentを開始しないこと。版削除の根拠は「reader不存在」ではなく、必要な安全性で説明してください。

## 8. minor：Hは正しいが、一般化には条件が要る

**対象：** `P:12,164–168`

拒否停止後に、値を必要とする未拒否enableがあるS13では、Fとしてuntimed配送する期待は正しいです。haltedを追加条件にして配送自体を抑える案は支持しません。

ただし、拒否が未来のcutとして予約済みで、release時には旧segmentがまだ動いている場合は、指摘1に戻ります。**「haltedだからuntimedでいつ当てても安全」ではありません。**

## 9. major：`design/18` 案は矛盾が残る

**対象：** `P:197–214,227–232`

修正対象の追加が必要です。

- **UR-24第1段落**（`design/18-uhd-radio.md:250`）にも「後のsegmentが適用する」という旧説明が残ります。
- **Holding and release**（同`:251`）は全releaseを `Some(e)` と説明しています。新しいuntimed例外とqueue順序を反映してください。
- 提案UR-36の「recordのcommandが一致すれば再configureしない」は、**channel setも一致する場合**に限定する必要があります。set不一致のreopenは成功記録を消し、configureが必要です（`core.rs:329–352`）。
- 提案UR-36の絶対的な「before the origin」は、UR-24のlate経路と矛盾します。配送機構はUR-24への参照に留めてください。
- UR-24案に **未拒否segment** の条件がありません。実装案の `!refused` と揃えてください。
- MR-33との出力parityを保証する `P:230` の説明は、指摘1によって成立しません。

DA-1について、履歴をChangesに置く方針は良いです。ただしUR-36へUR-24の配送規則を重複させず、各規則末尾のインデントした *Test:* に、実際に検査する条件を短く記述してください。

## UHD上のFの評価

**timed RX startが既にqueuedであること自体は、Fを否定する理由ではありません。**

UHDはstartの時刻をRX command用registerへ書き、FPGAはそれをRX専用FIFOへ渡し、enqueue時点でACKします。

- `U/host/lib/rfnoc/radio_control_impl.cpp:1116–1133`
- `U/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v:157–176,311–343`

これは **VERIFIED**。一般timed tuneのqueue待ちとは区別できます。一方、untimed applyが先行する一般timed commandを追い越す保証はありません。

Fはorigin-timed commandによる長いqueue占有を避けますが、代わりに**早期適用、既存queueとの順序、実配送期限**を解決する必要があります。frequencyのuntimed化によるLO phaseへの影響も、gainだけのテストからは保証できません（**INFERRED**、`design/09-mock-radio.md:205–207`）。

またVH-8の `updates()` は、`note` 行を明示的に除外します（`crates/ezsdr-acceptance/tests/parity.rs:692–693`）。**Manifestの比較が通っても、新経路のdevice状態やサンプルparityの証明にはなりません。**

## 10. minor：サイズ見積もり

`P:260–265` の約450〜640行は、記載済み項目の概算としては整合しています。ただし、queueを正しく観測するdouble、時間進行、部分失敗、旧segmentへの影響のテストが未計上です。

**1,500行以内は目標として妥当ですが、450〜640行で完了する見積もりは現時点では支持しません。**

未実施：ビルド・テスト・mutation・実機測定。ファイル変更なし、作業ツリーはcleanです。実機タイミングと競合の帰結には上記のINFERREDが残ります。

**判定：要修正**

## sol-plan2

**推奨：F' の方針は維持し、時計・失敗観測・queue 会計のテスト計画を修正してから実装してください。** Fへの差戻しや、版を残すだけの対処は勧めません。

以下の略記を使います。

- `P`：`/home/komatsu/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/test-plan-v2.md`
- `C`：`crates/ezsdr-radio-uhd/src/`
- `U`：指定コンテナ内 `/root/tmp/uhd-4.10.0.0/`

## 1. major：`advance_to` では FakeDevice の時計が進まない

**対象：`P:65-69,78,107,114,168,196-210`**

現在の `rig()` は、別々の時計を作ります。

- Core：`ManualTimeAuthority`（`C/provider/test_support.rs:62-75`）
- FakeDevice：既定は実時間の `HostClock`（`C/device.rs:394-413,573-588`）

`Core::new` に両者を同期する処理はありません（`C/provider/core.rs:512-554`）。`settle()` が読むのは **device自身の時計**です（`C/device.rs:731-750`）。

したがって、

1. gainを100.1 msにenqueue。
2. Coreの時刻を101 msへ進める。
3. `read_back()`で`settle()`を呼ぶ。

という系列でも、deviceの時刻が数msならcommandは未実行です。正しい実装が失敗したり、実行時間次第で結果が変わったりします。S5の`effective == now`も、二つの`now`が違うため保証できません。

**修正要求：** device時刻とCore時刻をどう一致させて進めるかを明記し、その一致をassertしてください。`ClockMode::Manual`への変更だけでも、Coreの`advance_to`との接続にはなりません。

### S16–S18について

時計を直せば、99/101 msのread-backは、名指しした「97.5 msでuntimed applyするF」を落とします。ただし、**99 msの値だけでは旧segmentの最後のサンプルまでを保証しません**。99.5 msで早期適用する誤りは通ります。

`P:196,203,210,288`の主張をread-backによる設定状態の検査に限定するか、cut直前と`e`直前・直後を観測してください。現在の系列は実際の旧streamのサンプルを観測していません。

## 2. major：booking後のchannel増加で、拒否なしにdepthを超える

**対象：`P:61,106-109,346`**

独立に導いた反例は、S4の予約順を逆にしたものです。

1. channelもpending segmentもない。
2. `e ≈ 10 ms`の更新を17個予約する。各bookingのcostは0。
3. channelを持つenableを100 msに予約する。
4. 7.5 msでreleaseする。

F'では17個すべてがdeviceへ出ます。しかしbooking後のcold変更には、既存heldを再審査して拒否する処理がありません（`C/provider/control.rs:590-611,950-959`）。

これは単に「以前からある会計方式」ではありません。**旧経路では出さなかったcommandをF'が出すことで、今回の変更がこの穴を通ります。** 複数channelへの増加ならさらに増えます。

また、heldのcostは次の`book_timed`で再計算されます。「booking後はcostが増えない」という説明も正確ではなく、**次のbookingが来なければ容量超過を検査しない**のが問題です。

**修正要求：** 上の順序を追加し、cold変更時の再審査・予約方式・容量規則の変更のどれで扱うかを裁定してください。単にINFERREDとして残すだけでは、queue容量を守る計画になりません。

なお、現在のS4の**17個目の拒否自体は妥当**です。clock問題を直せば、costだけ0、heldの和だけ0という誤実装をそれぞれ落とします。ただし1 channelなので、`released`を「channelごとに積む」ことまでは固定しません。

## 3. major：S9は「deviceを呼ばなかった」を観測できない

**対象：`P:144-151,309-310`**

### (a) release前のloss

`core.device_lost()`はinner fakeもlostにします（`C/provider/core.rs:636-645`）。その後、誤って`apply()`を呼んでも、fakeはログを残す前に失敗します（`C/device.rs:1024-1039,1048-1053`）。

したがって、**「applyログ0、coreはlost」だけなら、先頭のlossチェックを消した実装でも通ります。**

### (d) channel 0のlost拒否後

`ApplyFails`は拒否した呼び出しをinnerへ渡さず、現在のcall logにも残しません（`C/provider/test_support.rs:367-371`）。

誤ってchannel 1へ進んでも、inner fakeは既にlostなので、やはりログを残しません。`apply rx 1`が0というassertでは、呼ばなかったことを証明できません。

**修正要求：** wrapper入口で、失敗判定・inner呼出しより前に、`dir/chan/at`を記録してください。そのattempt列で(a)(d)を検査します。

また、S9(c)の失敗channel 1は最後です。これだけでは「失敗後に次のchannelを呼ばない」を検査しません。最後からのfall-throughによる行記録は検出できますが、`P:310`の説明は分ける必要があります。

`timed`／`timed_lost`という追加口そのものは支持します。

## 4. major：成功記録を持ったsegmentでのrelease失敗が抜けている

**対象：`P:13,145-148,222-229`**

S9(b)(c)は、いずれも`idle=false`として、ownerがまだconfigureしていない系列です。次がありません。

1. from-noneのenableをconfigure済み。記録はそのsegmentの`seq`。
2. その後に`e < O`の更新を予約。
3. F'のtimed applyが非lost拒否される。
4. ownerがそのsegmentをbeginする。

提案どおりなら、更新は投影に残りますが、deviceへ届かず、成功記録も無効化されません。ownerは同じ集合・同じseqなのでconfigureを省きます。

根拠：

- 更新の投影挿入：`C/provider/control.rs:608-611`
- release失敗時のreturn：同`:663-665`
- 非lost失敗は拒否報告のみ：`C/provider/core.rs:656-661`
- ownerの省略判断：同`:817-823`

**「deviceに知らせずに投影を変える経路がなくなる」という版削除の根拠は、失敗経路まで含めると成立していません。**

**修正要求：** この系列で、拒否後の更新を取り消すのか、成功記録を無効にして後のconfigureへ残すのか、期待を明示してテストしてください。版を復活させる必要がある、という指摘ではありません。

## 5. major：S12のassertとmutationの対応が不完全

**対象：`P:168-170,305-306`**

### timed commandに渡す時刻

S12の期待は`Some(effective)`、つまり10,000 µsです。しかしassertには`at=<e>`とあります。

fakeは、

```text
effective = max(at, last_command, device_now)
```

なので、誤って`Some(9700)`を渡しても、deviceの最終値と`effective=10000`は同じになります（`C/device.rs:1031-1039`）。

**`at=10000`を直接assertすることが必須**です。read-backだけではこの誤りを落としません。

### Manifest行

`P:305`は、pending行の`at`を`effective`へ変えるmutationをS12で落とす計画です。しかしS12のassertには、その行の`at/note/issued`がありません。

S1では`e == effective`なので代用できません。

**追加すべきassert：**

- device command：`at=10000`
- pendingの記録行：`at=9700`、`note="no channel"`、`issued=true`

## 6. major：`design/18`案は会計とinstantの説明が実装案と違う

**対象：`P:238,250,263,265-278`**

### layoutの空候補

新しいUR-24案は「候補segmentがなければdepthに0 channel」と書きます（`P:250`）。

一方、実装案はlayoutを今のまま1本ずつ数えます（`P:61`、現コード`C/provider/control.rs:594-599`）。

例えば、c1がstream外で後続joinもない状態の17更新は、文面なら0 cost、実装案なら17個目を拒否します。

**layoutは常に1 slotという規則を残すのか、空候補なら0へ変えるのかを揃えてください。**

### own instantとeffective instant

`P:238`はstream外の更新を無条件に「at that instant」とします。しかしS12は`effective > e`です。最初の段落もeffective instantへの参照が必要です。

また、現行`design/18-uhd-radio.md:252`の「Each released command is recorded … with `e`」も、live行とpending行を区別する今回の説明に合わせて確認してください。

### DA-1

方向は概ね良好です。UR-17／UR-23の集合＋commandへの変更、UR-36から版を消し配送をUR-24へ参照する構成は支持します。

ただし、`P:263`末尾の「旧テストがleaveする」は履歴なので、規範の`Test:`行ではなくChangesへ置いてください。S16–S18のTest説明も、未観測の「最後のサンプル」を保証した表現にはしないでください。

## 7. UHD上のF'：成立する部分と、まだ一般化できない部分

### front endがstreamしていない場合

**front end側の時刻付きregister操作が、stream開始を待つ構造ではないことはVERIFIEDです。**

- X3xxのtimekeeperは`sample_rx_stb=1`で動く：`U/fpga/usrp3/top/x300/bus_int.v:541-556`
- dboardへの制御経路は`USE_TIME=1`でその`radio_time_tb`を使う：同`:918-940,943-965`
- 時刻到達後にsettings busへ出す：`U/fpga/usrp3/lib/rfnoc/utils/ctrlport_to_settings_bus.v:184-203`

したがって、**「streamerが走っていないから、front endへのtimed gainは時刻が進まず実行されない」ことを理由にF'を否定する必要はありません。** S13のhalted方向も同様です。実機でのgain変更完了時刻は未測定で、そこはINFERREDです。

### frequency全体は別

`P:344`の説明をfrequency全体へ一般化できません。

- `set_command_time`はradioだけでなくDDC/DUCにも設定する：`U/host/lib/usrp/multi_usrp_rfnoc.cpp:964-986`
- frequency設定はRFとDSP両方へ到達する：同`:1394-1404,2067-2077`
- DUCのtimed frequencyはsample側の時刻追跡を使う：`U/fpga/usrp3/lib/rfnoc/dds_timed.v:135-170`
- timed FIFOの取り出しにはtimestamp付きsampleの転送が必要：`U/fpga/usrp3/lib/rfnoc/axi_tag_time.v:206-211`

**sampleが来ない間、DSP側のcommandがfront endと同じ意味で`e`に消費済みになるとはいえません。**

追加してほしい系列は、

> idle中のtimed frequency → ownerのuntimed configure → 長い無通信区間 → 最初のtimestamp付きsample

です。再開時の設定と残存queueを確認してください。実機での最終的な誤設定・blockingの発生は **INFERRED** です。DDC/DUCの既知課題をこの段階で全部解く要求ではありませんが、gainだけで「すべての更新が配送済み」とする根拠にはできません。

### ownerのuntimed configureとFIFO

S2はtimedもconfigureもgain=3なので、順序が逆でも最終値は3です。**FIFO模型の正しさを検査しません。**

UHDの一般queueは受信順です（`U/host/docs/driver_usage/timed_cmd.dox:94-115`）。一方、fakeのuntimed applyはtimed queueを迂回します（`C/device.rs:1047-1075`）。

異なる値の「timed A → untimed B」を同じ制御先へ出し、drain後がBとなる観測を、doubleの資格確認として追加してください。既知のfakeの差を残すなら、S2/S12から証明できる範囲を限定する必要があります。

### release後にsegmentがcut／refuseされる場合

S8は**release前**のrefusalだけです（`P:135-139`）。追加すべき系列は、

> pendingを対象にrelease成功 → 対象segmentをcut／refuse → 元のeffective instantを通過

です。期待はUR-24の「release済みcommandを取り戻さない」と整合させ、deviceへの適用、`released`のretire、記録行を確認してください。拒否されたsegmentへのowner呼出し禁止と、既に発行したregister変更の残存は別です。

## 8. minor：S3、S14、S15、分割案

- **S3：** unpublishedな記録を読む窓と、releaseが勝手に記録を書く誤りは検査できます。ただしprobeはconfigure完了後なので、`updates`を持たない誤実装でも「configure→release」の順になります。`P:101`のlock保証はこの系列では落とせません。既存timed lockを守る別テストを示すか、主張を記録公開の窓だけに限定してください。
- **S14：** booking順と異なるgainによる検査は妥当です。ただし`mark`は明確に「2度目のfrom-none configure後、`begin()`前」に置いてください（`P:183-184`）。本文の記述順は曖昧です。
- **S15：** 次段階への延期は支持します。ただし、controlのjoinだけでowner呼出しが完了するわけではありません。実際はcontrol→TX→RXの順にjoinします（`C/provider/mod.rs:695-704`）。保持済み`Planned`でのconfigure/startをplanのendだけが防ぐという説明は、証明済みにしないでください（`C/provider/rx.rs:279-304`）。
- **分割：** 1,050〜1,300行なら分けなくてよいです。ただし超過時にS16–S18だけを次へ送る案は支持しません（`P:338`）。今回の早期適用防止を固定するテストを、挙動変更と切り離さないでください。

## 9. 前回の指摘1–10の状態

| 前回 | 状態 |
|---|---|
| 1：旧segmentへの早期適用 | **方針は閉鎖、検証は未閉鎖**。F'とS16–S18は適切。時計と観測範囲を修正する。 |
| 2：UHD queue順序 | **一部未閉鎖**。`issued()`による投影移動は改善。S12のassert、untimed configureの模型が残る。 |
| 3：releaseの時間保証 | **方針変更で閉鎖可**。新しい定刻配送保証を撤回し、既存late動作を維持する。ただしS5の二つの時計を修正する。 |
| 4：失敗時の投影 | **一部未閉鎖**。保持する期待は明確になった。成功記録ありの拒否、loss後のattempt観測が欠ける。 |
| 5：拒否segmentのmutation | **閉鎖**。TXで拒否segmentを残して検査する形は妥当。 |
| 6：S3/S4/S14の検査力 | **一部未閉鎖**。S4の17個目、S14の値は改善。S3のlock主張、S12の記録行assertを修正する。 |
| 7：版削除とstop | **条件付き**。reader不存在を根拠にしない変更は良い。失敗経路の記録整合性を先に決める。 |
| 8：haltedの一般化 | **方針は閉鎖**。S13のtimed化を支持。実機挙動の範囲は上記の区別が必要。 |
| 9：design/18 | **未閉鎖**。対象箇所は増えたが、layoutの会計とinstantの矛盾が残る。 |
| 10：サイズ | **概算は閉鎖**。安全性テストを超過時に延期する案だけ撤回する。 |

## 10. 求められた裁定

- **D1：採用支持。** `issued:true`は実際の発行を表す。`note`と元の`at:e`を残す。ただしS12で三欄を直接assertする。
- **S5：既存動作の維持を支持。** arrival時適用と、released commandの後ろになるlateを混同しない。定刻配送を保証したとは扱わない。
- **S9：追加modeを支持、観測方法は要修正。** wrapper入口のattempt記録と、成功記録ありの失敗系列が必要。
- **S15：延期支持。** 既存の未検証窓として残し、版削除の安全性の証明には使わない。
- **§7：現見積もりなら分割不要。** S16–S18だけを後送するfallbackは不採用。

未実施：ビルド・テスト・mutation・実機測定。ファイル変更なし、HEADは`7968d98`、作業ツリーはcleanです。実機での時間・DSP再開時の帰結には上記のINFERREDが残ります。

**判定：要修正**

## sol-plan3

**推奨：F' は維持し、持ち戻し後の順序と、configure が先行した失敗を裁定してから実装へ進む。v3 のままでは未閉鎖です。**

以下の略記を使います。

- `P`：`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/test-plan-v3.md`
- `A`：同ディレクトリの `adjudication-2.md`
- `C`：`crates/ezsdr-radio-uhd/src/`
- `U`：指定 Docker image 内の `/root/tmp/uhd-4.10.0.0/`

## 1. v2 の major 1〜6

| v2 major | v3 の状態 |
|---|---|
| 1：時計と read-back | **一部未閉鎖**。設定状態だけを主張する変更は適切。初期時刻と二時計の一致確認は下記 major 3。 |
| 2：booking 後の容量超過 | **容量検査自体は改善**。ただし持ち戻し後に順序の根拠を失う。下記 major 1。 |
| 3：失敗した呼び出しの観測 | **閉鎖**。入口の attempts、loss 前の spy、最初の channel での拒否が揃った（`P:203-218`）。 |
| 4：configure 済みでの release 失敗 | **逐次系列は改善、並行系列は未閉鎖**。下記 major 2。 |
| 5：S12 の直接 assert | **閉鎖**。device の `at=10000`、pending 行の三欄、投影移動を直接見る（`P:229-239`）。S12(b) も残すべき。 |
| 6：design の会計・instant・履歴 | **主要点は閉鎖**。layout の一律一 slot、effective instant、履歴の Changes 移動は揃った。下記 minor 2 の文言修正は必要。 |

なお、前回の「`core.device_lost()` が inner fake も lost にする」という説明は誤りでした。`FakeDevice::mark_lost` はログ追加だけです（`C/device.rs:1659-1661`）。v3 の訂正は正しく、attempts による観測はその差に依存しません。

## 2. major：持ち戻した command が、retire 後に投影上の前へ戻る

**対象：`P:90-97,156-164`。根拠：`C/provider/control.rs:117-120,129-132,590-599,625-633`。**

容量を超えたら持ち戻すこと自体は妥当です。しかし、次の pass で `released` を retire してから effective を計算し直すと、**その command が既に後ろへ並ばなければならなかった時刻を忘れます。**

具体的な系列（時刻は ms）：

1. RX は初め 1 channel。9.8 に 2 channels へ変える cold を予約し、その投影を `P₀` とする。
2. 10 の TX 更新を 13 個と、RX gain=1 の更新 B を予約・release する。B は 2 channels に出るので、`released.len() == 15`。
3. 7.6 に、RX gain=2 の A を **9.7** で予約する。9.7 の RX は 1 channel なので、booking は `15 + 1 == 16` で通る。
4. release は B の後ろなので A の effective を 10 とする。そこで RX は 2 channels。`15 + 2 > 16` となり A を held に戻す。
5. 11 の pass で B などを retire。v3 は A の effective を **9.7 に戻し**、到着時に channel 0 へ gain=2 を適用する。
6. 一方 `P₀` の更新順は **A=9.7 → B=10**。投影の最後の値は gain=1 になる。

結果は **device=2、投影=1** です。さらに、初回の effective=10 なら対象だった channel 1 も再試行時には対象から落ちます。

UHD は受信済み command を timestamp 順に並べ直しません（**VERIFIED**：`U/host/docs/driver_usage/timed_cmd.dox:94-108`）。したがって、過去時刻を渡し直すだけでは投影順序を救えません。上記の帰結はソースからの推論で、未実行です（**INFERRED**）。

**必要な修正：** この系列を追加し、先行 command が retire しても失ってはいけない順序と、投影・targets に使う effective を裁定してください。S4b は全 command が同じ順序のまま遅れる系列なので、この反例を検出しません。

### 持ち戻しと Provider::stop

同じ `(e, seq)` を held に戻す限り、`finish → cancel_held` は既存の削除経路に乗ります（`C/provider/control.rs:319-342,680-701`）。**逐次的には妥当**です。

ただし、新しい経路として「上限で持ち戻す → finish が cancel → slot を空けても発行されない」を固定してください。held 消去、投影消去、cancelled 行、追加 attempt なしを見れば足ります。これは延期済み S15 全体の再開要求ではありません。

## 3. major：owner が先に読んだ更新は、失敗後の remove では取り消せない

**対象：`A:15-19`、`P:95,203-218,294-301`。根拠：`C/provider/control.rs:151-169,658-665`、`C/provider/core.rs:817-829`。**

新規則は、**remove が configure より先なら**成立します。しかし、owner が既に `updates` を保持した系列がありません。

1. pending segment の投影に gain=3 の更新を予約する。
2. owner が `configure` に入り、`updates` を取得して gain=3 の設定を組み立てる。
3. owner を `Core::configure` の `"apply"` probe で止める。この時点でも `updates` は保持中。
4. release を別 thread で開始する。release は同じ `updates` の取得待ちになる。
5. owner を進める。untimed configure は gain=3 を device に渡し、成功記録を残す。
6. release の timed apply を **適用前の非 lost 拒否**にする。
7. release は投影から更新を除く。

終了時は、**device=3、投影=旧値、configured=その segment の seq**。同一 seq による省略判断は残ります。

したがって、`A:17` の「device と投影は一致する」も、`P:349` の「no later configure applies it」も、lock の取得順を限定せずには保証できません。ソース上、この直列化順は可能です。実際の並行テストは未実行（**INFERRED**）。

**必要な修正：** この gate 系列を追加し、既に configure が渡した値と成功記録をどう扱うか裁定してください。投影の remove だけで版削除の根拠が閉じた、とは判定できません。

### S9(e) は名指しした誤実装を落とさない

`P:216,218,422` は「remove を省くと後の configure が 3 を当てる」とします。しかし S9(e) は **begin が configure しないこと自体を期待**しています。

remove を省いても、

- begin は同一 seq なので configure しない。
- timed apply は拒否されたので device は旧値。
- attempts も増えない。

この assert は通ります。**S9(e) 自身で投影の値／entry 消去を確認してください。** S9(b)(c) が共有コードの remove 削除を検出することと、成功記録ありの系列を検証することは別です。

## 4. major：時計の道具は成立可能だが、v3 の手順では初期化と「一致」の確認が不足

**対象：`P:64-72,143-144`。根拠：`C/device.rs:576-588,731-740`、`C/host.rs:118-126,383-401`。**

`Frozen` の方法は M3 と整合します。ただし、participant を登録するまでの時間は既に進んでいます。**Frozen は時刻を 0 に戻しません。** v3 の「device は 0 のまま」には初期化手順が欠けています。

また、

```text
set_time(t)
assert device.time_now() == t
```

は setter の確認であり、**Core の時計との一致確認ではありません**。S1 では Core=47 ms のまま device を O+1 ms にし、S16 でも Core=97.5 ms のまま read-back 時刻を進めています。

必要なのは次の明記です。

- 最初の command より前に、freeze した device の epoch を確立し、初期時刻を assert する。
- read-back 点では Core も対応する時刻へ進め、両方を比較する。
- 意図的に device=0、Core=release 時刻とする呼び出し検査は、同期した実行の検証とは区別する。
- S3 も `effective == e` を主張するなら、初期 device 時刻を制御する。

`set_time` を cfg(test) に限定し、queue を消さず既存 `settle` を使うこと、通常の fake の既定値を変えないことは支持します。`command_queue=64` も容量制限をログで検査する目的には妥当です。

ただしこの道具は host clock の sleep を進めません。非ゼロ `apply_delay` や clock 待ちを含むテストへそのまま広げないことを条件にしてください。

## 5. minor

### 1：S4b は「後ろを追い越させない」の観測になっていない

**`P:158-164,421`**

(a) は持ち戻す残りが 1 個、(b) は残りがすべて同じ 2-channel cost です。後ろに「slot に入る別 command」がありません。

また、再挿入を残したまま `break → continue` とすると、同じ先頭を取り直して無限ループになります。これは「追い越しを assert で落とす」系列ではありません。

この主張を残すなら、**先頭は 2 slots 必要、残りは 1 slot、空きは 1 slot**の混在系列を使ってください。

### 2：depth の英文が未実行 command を指していない

**`P:332,384`**

`more than that in effect` は、今回数える「未発効の released command」と逆です。`outstanding / not yet in effect` など、会計対象が明確な文に直してください。

---

UHD 4.10 の FIFO、満杯時の blocking、過去時刻の到着時適用は指定コンテナで再確認しました（`U/host/docs/driver_usage/timed_cmd.dox:94-115`、`U/host/include/uhd/usrp/multi_usrp.hpp:333-337`）。ただし API command 数から実機 FIFO の非 blocking を保証できる範囲は、v3 が残す UR-9 の **INFERRED** を超えません。

未実施：編集、ビルド、テスト、mutation、実機測定。ファイル変更なし。レビュー範囲は指定された差分とその接続経路に限定しました。

**判定：要修正**
