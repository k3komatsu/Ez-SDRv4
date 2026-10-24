# 38a：Sol 6.1 のレビュー

## テスト計画のレビュー（`sol-plan`）

判定：FAIL

## 1. 独自に抽出した失敗系列

1. **window 境界**：A の `e=10 ms`、B の受け取りを `7 ms`／その1 tick前に分ける。lead だけで判定する誤り、境界の `<`／`≤` の取り違えを検出する。
2. **予約履歴の範囲**：A と B の方向・channel を変え、A を channel 待ち／装置拒否／cold change 後にする。それでも A が床と枠に残ることを見る。
3. **満杯と期限**：満杯で late な予約、既存 command の instant ちょうど／直後、途中を飛ばした step を試す。判定順と枠の解放を固定する。
4. **同時刻の順序**：同じ `e` の異なる値・key を予約する。装置への順序、行の順序、最終設定を別々に見る。
5. **release と記録の分離**：装置には `e` 前に渡すが、行は `e` 前には存在しない。遅れた release も試す。
6. **終了の境界**：given／held／no-channel／refused を、停止・喪失それぞれの `S` 前・同時・後に配置する。Action の Stop は取り消さない。
7. **終了処理の遅延**：喪失・停止後に時計を進めてから確定する。さらに loss→stop、stop→loss、喪失の再通知を試し、最初の S と理由が変わらず二重記録しないことを見る。
8. **release 途中の喪失**：`targets` 後・apply 前、apply の失敗、apply 成功後に分ける。`e` を過ぎてからの release も含める。
9. **Ceiling の限定**：channel-less な先頭と live な後続を分け、後続の行だけを除外する。先頭の誤分類や、誤った `e` を除外で隠さない。
10. **profile の値の利用**：window 0／3 ms／非既定値で、床の計算と実際の release の両方が値を読むことを見る。

UHD の FIFO・late command の到着時実行は、[UHD 4.10 `multi_usrp.hpp`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/include/uhd/usrp/multi_usrp.hpp) の `set_command_time` で確認した。3 ms の妥当性は INFERRED のままとする。

## 2. 指摘

以下の `test-plan.md`、`plan-decisions.md` は `~/.cache/ezsdr-fixes/2026-10-24-note38/` 配下。

### 1. major — 語彙関数の期待値4行が D2 の式と一致しない

**根拠：VERIFIED。** `test-plan.md:95–98`。

`now=0, lead=20, window=30` では、earlier の `50`／`51` は窓の外であり、床には入らない。正しい期待値は次のとおり。

| 行 | 正しい `(e, late)` |
|---|---|
| 95：Some(45), earlier=[50] | `(45, false)` |
| 96：Some(50), earlier=[50] | `(50, false)` |
| 97：Some(50), earlier=[51] | `(50, false)` |
| 98：None, earlier=[50] | `(20, false)` |

現計画では正しい実装が落ち、「窓の外の予約も床にする」実装へ誘導される。

**修正：** 表を訂正する。床ちょうど・床より前・deadline 省略を検証したい行は、別途 `earlier ≤ now+window` になる入力にする。

### 2. major — Ceiling の除外条件が観測結果に依存し、誤分類を隠す

**根拠：VERIFIED。** `plan-decisions.md:24`（F1）、`test-plan.md:175–176`、設計メモ38:36。

F1 の「UHD の行が cancelled、`e<S`、apply が無い」は、Ceiling の十分条件ではない。例えば、本来 `no channel` と記録すべき先頭を誤って cancelled にすると、この条件を満たし、比較から消える。

また計画175行は `applied` **と `timed`** から除く。これはメモの「その command の行の比較だけを外し、`e` は残す」という制限を、そのままでは満たさない。

**修正：** 除外対象を、系列の期待 timeline と release 手順から独立に決める。command ごとの識別子で管理し、no-channel の先頭は除外しない。Ceiling 対象でも予約上の `e` は別の観測で必ず比較する。先頭を誤って cancelled にする変異が失敗する harness 試験を追加する。

### 3. major — O5 の「必ず `e>S`」は既存コードで成立しない

**根拠：VERIFIED。** `plan-decisions.md:11`、`crates/ezsdr-radio-uhd/src/provider/control.rs:3451–3467`。

既存の `ur_24_a_release_after_its_instant_applies_on_arrival` は、`e=50 ms` の command を `51 ms` に release する。したがって、その `before_timed_apply` で喪失を公開すれば `e<S` になる。`e=S` も作れる。

「渡っていないので保持へ戻す」という方針はよいが、未来の command だけを想定した8bでは、過去・同時刻の分類を固定できない。

**修正：** 8bを `e>S`、`e=S`、`e<S` に分ける。未送出の live command は、期限判定を通したうえで Ceiling／終了時取消として記録し、issued にしないことを確認する。

### 4. major — O7 の枠境界が D2 の規則を変更している

**根拠：VERIFIED。** `plan-decisions.md:13`、`test-plan.md:109–111`、設計メモ38:33・69。

D2 は「`e` がまだ来ていない」予約を数え、instant で枠を外す。一方 O7 は、受け取りが最早の `e` ちょうどなら必ず満杯とする。

さらに Mock は適用後に `updates` から消すため、同じ受け取り時刻でも、その直前に `step(e)` を済ませたかどうかで結果が変わりうる。これは予約履歴と instant で決まる規則ではない。

**修正：** D2 に合わせ、期限到達済みの予約を枠から除外する計画へ直す。`step(e)` を挟む場合と挟まない場合を比較し、処理済み状態に依存しないことを確認する。`e==r` を占有させる別規則を採るなら、orchestrator の解釈ではなく owner の承認が必要。

### 5. major — 非既定 window の試験は release 側の定数残存を殺せない

**根拠：VERIFIED。** `test-plan.md:85,209`、`control.rs:574,582`。

`ur_24_the_release_window_comes_from_the_description` の記載された assert は B の床と LATE のみ。床が description を読み、**release だけが旧3 ms定数を使う**実装でも通る。

そのため M15 を release 側へ適用した場合の killing assertion がない。

**修正：** window 10 ms、A の `e=12 ms` なら、2 ms直前には apply 無し、2 msには timed apply が1件、行は12 msまで無し、を確認する。床側と release 側の定数化を別々に殺す。

### 6. major — 終了時刻・同時刻優先順位の一部が、計画の setup では検証されない

**根拠：VERIFIED。** `test-plan.md:50,57`、`crates/ezsdr-mock-radio/src/lib.rs:2463–2479`。

- Mock の系列8は loss の S と `step` の時刻がともに8 msなので、誤って step 時刻を記録しても通る。
- Mock の9c停止側は先に `step(S)` で A を適用している。`stop()` 自身が同時刻の更新を先に確定する規則を壊しても、この試験は通る。
- O10 の「最初の S／理由」は宣言だけで、loss→stop、stop→loss、再通知の系列がない。

**修正：**
- loss を8 ms、処理する step を10.6 msとして、記録は8 msと確認する。
- A を未適用のまま Authority だけ S へ進め、直接 `mock.stop()` を呼ぶ。
- 終了順序2通りと喪失再通知を追加し、S・理由・行数が変わらないことを確認する。

### 7. major — 変更対象 spec に、実装と矛盾する規則が漏れている

**根拠：VERIFIED。** `test-plan.md:121–126` に対し：

- `design/07-radio-model.md:87` **RM-6** は TimingEnvelope を「six」とし、release window を含まない。
- 同 `:168` **RM-16** は、Provider stop でも既送出 command を issued として残す。D2 の未来の given command の取消記録と直接矛盾する。
- `design/09-mock-radio.md:52–72` **MR-3** の profile 値表に、x3xx の3 ms／ideal の0がない。
- `design/18-uhd-radio.md:502` の VH-8 試験表は、`timed` の約25系列を比較しないまま。

**修正：** 改訂一覧に加える。特に RM-16 の旧例外を残したまま、Provider spec だけを D2 に変えてはならない。

### 8. minor — `finish()` の新しい S 取得方法に対する既存試験の移行漏れ

**根拠：VERIFIED／失敗可能性は INFERRED。** `crates/ezsdr-radio-uhd/src/provider/control.rs:2100–2112`。

`ur_30_the_recorded_plan_has_no_refused_segment` は `streams.end(...)` の後、`streams.stop` を設定せず `finish()` を呼ぶ。計画119行どおり S を `streams.stop`／`lost_at` だけから必須取得すると、この既存試験には S がない。§3に記載がない。

**修正：** 終了情報を `Streams::end` で一元保持するか、この試験に明示的な stop marker を設定する。未提示の S を現在時刻で黙って補わない。

### 9. minor — 一部の mutation に、記載された殺傷力がない

**根拠：VERIFIED。** `test-plan.md:35–36,66,210,212`。

- 系列3は window を大きくしても A の床は10 ms、B は11 msのまま。「window を大きくする」を必ず殺す試験ではない。
- M16 の `late = e > asked` は、提示された `e=max(asked,floor)` では `asked<floor` と同値。
- 系列10 UHD は release を10 msにしか呼ばないので、単独では「7 msに行を書く」を検出できない。
- M18 の終了行逆順は、通常適用だけの系列10では検出できない。9dの並びの assert が killing test になる。

**修正：** 同値 mutant を除き、各変異を実際に差が出る試験へ対応付ける。

---

変更は行っていない。テスト・mutation・実機測定は未実施。提示された判断ファイルには F1・F2 のみで、brief が挙げる F3・F4 は確認できなかった。なお最初のメモ読取りに§4も含まれたため、完全な非アンカー条件は満たしていない。

## コードレビュー（`sol-code`）

判定：FAIL

**major 2件、minor 1件。** 停止との競合と、Ceiling の除外範囲を修正してから再レビューすべきです。

## 1. major — 停止後の release が、取消すべき command を `issued` に確定する

**場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:651–662`  
**根拠：VERIFIED（コード追跡）。**

`run()` はループ先頭でだけ停止を確認します（`:314–324`）。通常の `release()` は喪失を再確認しますが、`streams.stop` は確認せず、`e ≤ now` の行を確定します。

到達可能な系列：

1. A（`e=10 ms`）を7 msに装置へ渡す。
2. uhd-control がループ先頭の停止確認を通過してから中断される。
3. `Provider::stop` が `S=9.9 ms` を公開する（`provider/mod.rs:703–710`）。
4. uhd-control が10.6 msに再開し、`release()` がAを `issued` と記録して `released_at` から除く。
5. 次の `finish()` にはAが残っておらず、取消へ変更できない。

D2 (4) の期待値は **`cancelled_at=9.9 ms`** です。既存の終了テストは停止後に直接 `finish()` を呼ぶため、この経路を通りません。

**修正：** 終端の公開後は通常の release／fold で確定せず、保存した `S` による settlement に残す。停止確認と release の間に停止を公開する、手動時計の競合テストと mutation 行を追加する。

## 2. major — Ceiling が channel-less な後続まで比較から外す

**場所：** `crates/ezsdr-radio-uhd/src/provider/control/differential.rs:321–328`  
**根拠：VERIFIED（コード追跡）。**

`channel_less` を計算していますが、除外条件は `stalled && e < loss` だけです。対象のcommand自身にchannelがなくても除外されます。

例えば明示系列（`:229–237`）を次の形にすると問題が出ます：

- RXのchannelを外す。
- A・BをRX、CをliveなTXへ、いずれも `e=10.4 ms` で予約。
- 最後のreleaseは10 ms、喪失は10.9 ms。

A・Bはともに `no channel` で比較すべきですが、現在は **BとCの両方** が除外されます。Bだけを誤ってcancelledにする実装を隠せます。

1,000系列のCeilingは実行結果でも0件でした。追加された明示系列は「channel-lessな先頭＋liveな後続」だけなので、この過剰除外を検出できません。

**修正：** 除外対象自身がchannel-lessでない条件を加える。上記の3-command系列で、除外集合がCだけになることと、Bの誤分類が比較を失敗させることを固定する。`timed`／`late` を全commandで比較する現状は維持する。

## 3. minor — 新しいspec本文に、実装と異なる定義がある

**場所：**
- `design/09-mock-radio.md:150`
- `design/07-radio-model.md:170`
- `design/18-uhd-radio.md:302`

**根拠：VERIFIED。**

- MR-18は「`e` is … floor」としています。しかし `at=11 ms`、floor=10 msなら、実装の `e` は11 msです（`crates/ezsdr-radio/src/timeline.rs:415–417`）。
- RM-16／UR-26のCeiling説明は、channel-lessな先頭による停止に限定されています。実装は、poll遅延やapply直前の喪失で渡せなかったlive commandもcancelledにします。系列8bの `e<S` がその例です（`provider/control.rs:5498–5518`）。

**修正：** MR-18では床の式を再記述せず、`e` とlate判定をRM-21へ委ねる。Ceilingは設計メモ38のとおり、poll・同期呼出しによるrelease遅延も含む定義にする。共通規則は語彙側に一度だけ置く。

## 同じtickの停止と喪失について

**「停止のsettlement完了後に届いた喪失では、確定済みの行を巻き戻さない」という裁定には同意します。**

`Streams::end()` は一度限り（`provider/core.rs:547–550`）で、settlementも対象をcollectionから取り除きます（`provider/control.rs:735–753`）。後から同じtickを読んだ喪失は、停止より前に観測された喪失ではありません。この限定で、38aにjoin順変更を求める必要はありません。

指摘1は **`S < e` のcommandを、停止公開後の通常releaseが確定する問題**であり、この裁定とは別です。

## 確認結果

実行したテストはすべて成功しました：

- UHD control unit：132件
- UHD differential／分類／RX differential：3件
- MockRadio：83件
- Mock timeline differential：1件
- 語彙の `rm_25_*`：2件

追加・再アンカーしたmutation行について、上記とは別の不備は確認しませんでした。J58・P424および `at=None` fallback の同値性の説明も妥当です。

ソース変更なし。mutation・実機・全workspace gateは未実施で、指摘1・2の追加再現テストも実行していません。

## 直し 1 回目の再レビュー（`sol-fix1`）

判定：FAIL

**前回の3件は修正済みですが、追加された終端後の再分類にmajorが2件あります。**

## 1. major — 拒否・喪失の早期returnが、同じpassの先行commandの再分類を飛ばす

**場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:629–667, 767–770`  
**根拠：VERIFIED（コード追跡。追加再現テストは未実施）。**

到達可能な系列：

1. liveなA・Bを、ともに `e=10 ms` で予約する。
2. 停止 `S=10.2 ms` を公開する。
3. 10.6 msのreleaseがAを渡し、`released_at` と `given_here` に入れる。
4. Bのapplyが拒否、または喪失を返す。

`:641`／`:644` のreturnで、`:660–665` のheldへの戻しを通りません。Aは停止後に渡されたのに、`finish()` が `e≤S` を見て **issued** にします。次のreleaseでもAはそのpassの `given_here` に入らず、修復されません。

期待値はAの `cancelled_at=10.2 ms`。Bは通常の拒否なら行なし、喪失なら終端で確定です。

**修正：** 拒否・喪失でも、既に渡したcommandの終端との照合を必ず通す。releaseの装置呼出し条件は変えず、上記2分岐の手動時計テストとmutation行を追加してください。

## 2. major — pass先頭のnowでは、途中でSを越えた後続commandまでissuedになる

**場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:602, 629–635, 660`  
**規則：** `design/07-radio-model.md:169–170`  
**根拠：VERIFIED（コード追跡）。**

例えば：

- A・Bとも `e=10 ms`、release開始は9.9 ms。
- Aの同期apply中に停止 `S=10.2 ms` が公開され、Aが10.3 msに戻る。
- **Bのapplyは10.3 ms以降に初めて呼ばれ、成功する。**

Bは明確にSまでに渡されていません。しかし判定は古い `now=9.9 ms` のためheldへ戻らず、issuedになります。A自身の受理時刻が不明でも、Bの誤分類は確定できます。

**修正：** pass全体のnowをhandover時刻として使わず、commandごとの境界で終端と照合する。Aの待機中に停止を公開し、その後に呼ばれるBがcancelledになるテストとmutation行を追加してください。

### 同期apply自身の受理時刻についての裁定

**報告§9.5の棄却理由には、そのままでは同意しません。**

`Applied` に受理時刻がない点は正しいです（`crates/ezsdr-radio-uhd/src/device.rs:62–72`）。実際、FakeDeviceもqueue満杯で待ち、後から受理します（同`:1074–1097`）。したがって、**Sをまたぐ呼出し自身の正確な装置受理時刻を、現在の戻り値から復元できない**という説明は妥当です。

ただし、それは「pass先頭のnowで判定してよい」根拠にはなりません。D2 (4)／RM-16には、その例外がありません。呼出し自身の境界は、観測可能なhandoverの定義をorchestratorが明示する必要があります。少なくとも上記Bのような、S後に呼出しを始めるcommandは曖昧ではありません。

## 前回指摘と確認結果

- **停止後のfold：解消。** streams lock下の `ended_at()` 確認が停止・喪失の公開と行の確定を順序づけています（`control.rs:654–676`）。
- **Ceilingの過剰除外：解消。** `!channel_less` が入り、3-command系列の除外集合はliveなCだけです（`provider/control/differential.rs:323–328, 554–575`）。
- **spec本文：前回の不整合は解消。** MR-18はRM-21へ委譲し、Ceilingの共通定義はRM-16、UR-26はその参照になっています。
- heldへの戻し自体では、collectionからの消失・二重記録・release条件の変更は確認しませんでした。問題は、**戻しを通らない出口と、古い時刻による分類**です。

UHD control関連 **136テストを実行し、全件成功**しました。ただし上記の複合系列は既存テストにありません。

ソース変更なし。mutation・実機・全workspace gateは未実施です。

## 直し 2 回目の再レビュー（`sol-fix2`）

判定：PASS

今回指定された範囲では、major／minor の指摘はありません。前回の major 2 件は解消しています。

1. **早期 return による判定漏れ：解消（VERIFIED）**  
   `crates/ezsdr-radio-uhd/src/provider/control.rs:633–645, 655–657`  
   成功した command は直ちに `handed_at` 付きで `released_at` に保存されます。後続 command の拒否・喪失で return しても先行分は残り、終端の fold も記録せず戻るため、`settle` が判定できます。修正不要です。

2. **pass 先頭の時刻による誤分類：解消（VERIFIED）**  
   `crates/ezsdr-radio-uhd/src/provider/control.rs:707–731, 762–773`  
   各 `apply` の直前に時計を読み、最後の channel の値を保存しています。`settle` は command ごとに、due と hand-over の両方を停止 `≤ S`／喪失 `< S` で比較します。喪失した `apply` は `held` に残り、issued にはなりません。修正不要です。

3. **隣接系列・変更した期待値：整合（VERIFIED、静的確認）**  
   `crates/ezsdr-radio-uhd/src/provider/control.rs:5284–5432, 3838–3874, 5736–5765`  
   pass 途中の停止・喪失、先行 hand-over 後の拒否・lost apply、全体が停止後に走る pass を確認しました。既存2テストは `h = S` の喪失なので、cancelled への変更は新定義どおりです。追加テストは装置呼出しと最終行を観測しており、対象の誤分類を検出できます。修正不要です。

4. **定義・spec：整合（VERIFIED）**  
   `design/07-radio-model.md:169–170`、`design/18-uhd-radio.md:302`、`crates/ezsdr-radio-uhd/src/device.rs:62–72`  
   受理時刻を返さない API に対し、呼出し直前の読みを境界とする裁定は明確です。RM-16 が共通定義を持ち、UR-26 は参照と実装機構を記述しています。これは実際の装置受理時刻の保証ではありませんが、その制約込みでコードと一致しています。修正不要です。

ファイル変更なし。負荷を避け、テスト・mutation・実機・全体 gate は実行していません。報告された実行結果は独立には再確認していません。

