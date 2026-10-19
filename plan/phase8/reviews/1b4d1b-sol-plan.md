# 1b-4d-1b：Sol 6.1 のテスト計画の差分レビュー

指示書は [1b4d1b-brief.md](1b4d1b-brief.md) §3（Sol へ）．裁定は [1b4d1b-adjudication.md](1b4d1b-adjudication.md)．


**推奨：B1〜B3 の担当分離は維持し、B4 の競合条件とテストの観測方法を修正してから実装してください。** 全 streamer 呼び出しへ busy を戻す必要はありません。ただし、現計画では重要な系列が抜け、正しい実装でも失敗するテストがあります。

以下、`P` は `/home/komatsu/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1b/test-plan-1b.md`、`B` は同ディレクトリの `plan-brief.md` を指します。実行再現していない競合の帰結は **INFERRED** とします。

## 独立に立てた系列と照合結果

計画を読む前に、from-none の各失敗・loss・stop、owner との担当取得競合、idle TX の report 読取り、開始済み将来 segment への後着更新、lock 順序、manual clock の待機を挙げました。

S9・S22・S26・S27 は一部を扱っています。不足は次のとおりです。

## major

### 1. S11d は、依頼された「owner が既に始めた後」を通していない

**対象：** `B:60`、`P:67–70`

brief は「owner が segment を開始 → timed 更新を記帳・release」です。しかし計画は、release **後**に owner を begin しています。これでは「開始前なら再 configure で救えるが、開始後は更新が失われる」実装を区別できません。

さらに、5 ms の enable を指定しても、実際の origin は少なくとも ready＋RX start lead の **50 ms** です（`crates/ezsdr-radio/src/timeline.rs:937–955`、`design/18-uhd-radio.md:58`）。6 ms の host sleep 後に gain=3 を要求する観測は成立しません。

**修正・期待：**

- 実際の `segment.origin` を取得する。
- owner を先に begin させ、その後、origin 前の更新を記帳・release。
- 再 configure はなく、origin 宛ての timed apply が1回。
- fake 自身の時計が実際の origin を越えた後に設定を読む。`ManualTimeAuthority` の進行とは区別する。

### 2. configure の完了と成功記録の公開の間が未検査

**対象：** `P:158–163,173`、`crates/ezsdr-radio-uhd/src/provider/core.rs:751–754`

現コードは `ColdConfig::configure` が `updates` を解放してから、`configured` を書きます。次の順序が可能です。

1. owner が旧版で configure を完了し、`updates` を解放。
2. 成功記録の公開前に release が同じ `updates` を取得。
3. release は記録不一致として版を上げ、device には出さない。
4. owner が旧版の成功記録を書き、segment を開始。

**INFERRED：** 更新は device に届かず、開始した owner が再 configure する機会もありません。release 側だけ `updates → streams` にしても、この窓は閉じません。

**追加する系列・期待：** configure の snapshot が確定してから成功記録を公開する境界を直接制御し、その間に record-only release を置く。結果は「更新込みで configure 済み」か「timed apply で届く」のどちらかであり、更新を取りこぼして開始してはなりません。成功記録の公開まで含む同期方法を計画に明記してください。

### 3. B4 は、予約していなかった device queue の容量を消費する

**対象：** `B:55–59`、`P:173,230–231`

現行の current-keys 経路は、更新の effective instant に channel がなければ cost=0 として受理します（`crates/ezsdr-radio-uhd/src/provider/control.rs:591–597`）。B4 はそれを後から、将来 segment の channel 数分の device command に変えます。

**INFERRED：** channel 不在期間の更新を多数受理してから同じ将来 origin へ投入すると、宣言した queue depth を超え、control が back-pressure で待ち得ます。fake も queue が満杯なら待ちます（`crates/ezsdr-radio-uhd/src/device.rs:1021–1045`）。

**追加する系列・期待：**

- channel 不在中の複数更新を、configure 済みの将来 segment へ昇格。
- その後に、別のより早い更新を追加。
- depth を超えて device へ投入せず、順序・拒否・保持の扱いを明示する。

`released.push(origin)` の有無だけでは足りません。origin が release 時点で既に過去の場合も未決です。UHD は過去の command time を到着時に実行します（UHD 4.10 `host/include/uhd/usrp/multi_usrp.hpp:335–337`）。その場合の lateness と記録を決めてください。

### 4. S23 の50 ms観測は誤実装を確実に落とさず、ready の観測も誤っている

**対象：** `P:122–127`

- busy=true を観測した直後に control が deschedule されれば、待ちを削除した誤実装でも50 ms以内に open しません。gate 解放後に open すれば最終回数も正解になります。「mutation の速度を仮定しない」という説明は成立しません。
- rig の `Core::now()` は独立した `ManualTimeAuthority` です（`crates/ezsdr-radio-uhd/src/provider/test_support.rs:60–74`）。host clock の待機だけでは進まないので、`ready > booked_at` は保証されません。

**修正：** 待機分岐への到達を test-only hook で同期し、reading を確認した結果として待機したことを観測してください。実時間 timeout は終了監視だけに使います。fallback しなかったことは、control による open・configure と記録で判定できます。

### 5. loss と shutdown の出口が不足している

**対象：** `P:137–153,172`、`B:83–85`

S26 は open 成功後の loss だけです。必要なのは次の系列も含みます。

- `reading` 待機中に loss → 待機を終了し、open せず busy を下ろす。
- rate 比較後の loss → configure しない。
- `{0,1}` の最初の apply 後に loss → 次 channel の apply をしない。
- from-none 中に Provider stop → 新 segment を開始せず、担当を解放する。

また、「control を join してから Shutdown」は、**join 成功を意味しません**。1秒で detach して先へ進みます（`crates/ezsdr-radio-uhd/src/provider/mod.rs:406–420,695–704`）。既存の wedged-thread テストだけでは、新しい busy／reading 待機の終了を保証しません。

S27(e) の「先に lost にしてから直接 book」は、担当取得後の解放漏れを十分に検査しません。実際に busy を立てた後の loss を主検査にしてください。

### 6. S24 が brief の逆向き競合を別のテストへ置き換えている

**対象：** `B:37`、`P:129–135`

要求は「owner の open を止め、その間 control が enable を受け取る」です。計画は control 自身の呼び出し中に busy を読むだけになっています。

**追加する系列・期待：**

- owner が idle=false にした後、その open を停止。
- **idle を強制 true にする既存 helper は使わず**、control が変更を受け取る。
- control は open／configure せず、通常の cold 変更として記帳する。

現 S24 は「configure 後、booking 前に busy を早く下ろす」誤りも落としません。probe は apply の**中**までしか見ていないためです。

### 7. 並行テストに既存 LockSpy の `try_lock` 判定をそのまま使えない

**対象：** `P:36,82–84,122,158`、`crates/ezsdr-radio-uhd/src/provider/test_support.rs:135–169`

既存 spy は単一スレッド前提です。並行テストでは、別 thread が正当に `streams` を保持した瞬間にも `try_lock()` は失敗します。それを「device caller が lock を持っている」と誤判定します。

**修正：** B5 の thread-local 検査を使って、**呼出元 thread の保持**を確認してください。別 thread が line を持っていても、device caller が持っていなければ許可する対照テストも必要です。

## minor

### 8. B5 の guard 案は妥当だが、lock 順序全体は検査しない

**対象：** `P:163,235`

`MutexGuard` を包み、取得成功後に加算・Drop で減算する方式は、小さい変更で実現できます。`updates` を取る前の assert と、意図的な逆順を落とすテストも妥当です。

ただし、全 `core::lock` を一つの深さで数える方式が検出するのは「それらを保持しての updates 取得」です。**`rec → streams` は検出しません。**

「lock 順序全体を検査する」とは書かず、検査範囲を限定するか、streams／rec の種類も区別してください。新しい D1 分岐の取得箇所も検査対象に含めます。

### 9. mutation と assert の対応に不足がある

**対象：** `P:60,70,78,114–117,211–218`

- S9 は要求された `idle` 不変を assert に含めていません。
- S22 は run loop 自体でなく、その一部の模倣です。`step` を誤って実行する変更は十分に固定しません。
- S11d は「記録の at が e」と「後続 command の queue 順序」を、列挙した主 assert で確認していません。
- 不一致側で常に timed にする誤りでも、集合不一致なら owner は依然 reopen・configure します。`P:78` の説明は誤りです。timed call=0 と版の変化で落としてください。
- busy 解放の mutation は成功経路だけでなく、追加した早期 return ごとに対応付けてください。

### 10. `design/18` 案は追加だけでは矛盾が残り、DA-1 としても長い

**対象：** `P:230–248`

修正が必要です。

- UR-24 の「applies nothing; enabling … applies it」は、後ろへの追記ではなく置換する。
- layout 側の「後の segment が適用する」という全称も点検する（`design/18-uhd-radio.md:247`）。
- ownership の旧文は UR-25 ではなく **UR-3** にあります（同 `:116`）。control の一時担当を反映する。
- UR-24 案の「最初の *記録が一致する* segment」と B4 の「最初の対象 segmentについて一致を判断」は意味が違います。後続の一致 segment を選んで先行 segment を飛ばさないよう統一する。
- `at=max(origin,effective)` という実装案（`P:173`）と「常に origin」という規範案を一致させる。
- 「owner が configure し直さないから」という説明は decision record へ移し、担当・待機・lock 順を短い別段落にする。
- UR-17／UR-23 は UR-25 を参照し、同じ busy 規則を重複記述しない。

UR-3 の一時担当、queue depth、過去 origin の扱いは、現在の「偽になる文」の一覧に不足しています。

### 11. 行数見積もりは目標としては妥当だが、1,050〜1,150行の根拠は弱い

**対象：** `P:185,252–256`

`P:185` のテスト内訳だけで約870行あり、本文の650〜750行と一致しません。今回不足している同期境界・終了系列・queue のテストも加わります。

**約1,500行を目標にすることは可能**ですが、現数字は再集計が必要です。分割するなら提示された「担当／待機」と「B4」を分ける案を支持します。loss・stop の安全系列を後回しにはしません。

## B1〜B4 と orchestrator の確認事項

- **B1〜B3：条件付きで妥当。** 通常 RUN では RX の begin、TX の read が device 操作前に idle=false にします。owner が保持中の recv／send／stop／restart に、呼出しごとの busy を追加する必要はありません。idle TX の report 読取りを reading で覆う例外処理は必要です。detach 後の shutdown は別途明記してください。
- **B2 の UHD 根拠：VERIFIED。** UHD 4.10 の `graph.cpp:188–197`、`multi_usrp_rfnoc.cpp:438–482`、`rfnoc_tx_streamer.cpp:98–101` と、ローカル `uhd.rs:1050–1052` を確認しました。旧 streamer を生かしたままの reopen を避ける理由は成立します。
- **B4：方向性は支持。ただし major 1〜3 の補完が必要。** 全 call の busy 化より、投影・成功記録の公開境界と queue 会計を揃える方を推奨します。
- **計画 §9.1：正しい。** 1a の既存テストは記録一致側です（`crates/ezsdr-radio-uhd/src/provider/control.rs:1223–1237`）。
- **計画 §9.5：誤り。** from-none の device 呼び出しと `book_timed` は同じ control thread 上で直列です（同 `:319–324,550,849–881`）。その呼び出し中に timed 更新を「記帳」する窓はありません。major 2 の owner thread との競合とは分けてください。
- **計画 §9.8：未確定。** stream command と retune の device queue 関係は既存 UR-24 でも INFERRED です（`design/18-uhd-radio.md:249`）。「release が必ず先でなければならない」とは現根拠から断定できません。

未実施：ビルド・テスト・mutation・実機。ファイル変更なし。競合の実行時帰結は上記のとおり INFERRED です。

**判定：要修正**
