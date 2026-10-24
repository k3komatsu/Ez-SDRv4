# #59：Sol 6.1 のレビュー

## テスト計画のレビュー（`sol-plan`）

判定：FAIL

## 1. 計画を読む前に立てたシナリオ

1. **選択と消費**：初期 segment は対象外。fault は対象の cold change まで待ち、一度だけ効く。同時刻は予約順で選ぶ。
2. **RX の拒否**：旧 segment の cut は残り、拒否された segment の block・SampleClock は生じない。
3. **復帰**：次の cold change／`start_rx` は拒否された設定値から再開する。無関係な stream は止めない。
4. **halt 中の timed 更新**：値は記録され、復帰後に実際のサンプルへ反映される。halt 自体は解けない。
5. **TX の拒否**：旧 burst を閉じ、拒否された clock の held burst を取り消す。後続 clock の予約は残し、halt 中の新規 burst は拒否する。
6. **move**：両 line を拒否し、移動後の設定を記録として保持する。各 line は個別に復帰できる。
7. **終了との競合**：loss／stop の前後・同時刻で、拒否件数、取消理由、未消費 fault の記録が一貫する。
8. **時刻の分離**：cold の effective instant と segment origin が異なる場合、後続予約で segment が消える場合を区別する。
9. **SimulationChannel**：拒否された TX の予約済み信号が medium に残らない。

以下、`test-plan.md` と `plan-decisions.md` は `~/.cache/ezsdr-fixes/2026-10-24-issue59/` 内を指す。

## 2. Findings

### 1. major — 複数の系列が、記載した時刻に予約・復帰できない

**根拠：VERIFIED**

`Harness::step(t)` は Authority を `t` へ進めてから Action を処理する。push 時刻は receipt ではない。

- `crates/ezsdr-mock-radio/tests/mock_radio.rs:283–285`
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:300–302`
- `crates/ezsdr-mock-radio/src/lib.rs:2530–2536`

このため次が成立しない。

| 計画箇所 | 問題 |
|---|---|
| 系列2、`test-plan.md:68` | 系列1の最後は `T0+100 ms`。「続き」で `now=e` に戻れない。独立に組んでも、push 後の最初の step が `e+10 ms` なら receipt もそこなので、復帰 origin は `e+60 ms`。記載の `e+55 ms` ではない。 |
| 系列4、`:70` | timed gain を push して最初に `step(h)` すると receipt は `h`。x3xx の minimum lead は2 msなので、effective instant は `h+2 ms` となり `LATE_COMMAND` も出る。`h` の applied 行と「event は拒否1件だけ」は得られない。 |
| 系列7、`:78` | `step(e+100 ms)` の後に `step(e2+60 ms)`、ただし `e2=e+10 ms`。Authority を30 ms巻き戻すため `InPast`。 |

復帰の ready term は `crates/ezsdr-radio/src/timeline.rs:1000–1018`、巻き戻し拒否は `crates/ezsdr-kernel/src/time/authority.rs:210–217`。

**修正**：各系列に receipt 用の step を明記する。系列2は `step(e)` で次の cold を予約してから進める独立系列にする。系列4も `step(e)` で gain を予約する。系列7は復帰予約と halt の観測を、時刻が単調増加するよう並べ直す。

### 2. major — 初期 membership がなく、系列2・3・6の場面に到達しない

**根拠：VERIFIED**

既定で stream 0 に入るのは **RX c0 だけ**。RX c1 と全TX channel は `-1`。

- `crates/ezsdr-radio/src/channels.rs:514–533,957–958`
- 既存の確認：`crates/ezsdr-mock-radio/tests/mock_layout.rs:476–506`

したがって：

- **系列2**：c1 は最初から `-1`。`c1.stream=-1` は exit ではなく、cold の対象 stream がないため halt を解かない（`crates/ezsdr-mock-radio/src/lib.rs:1924–1926`）。
- **系列3**：`rx: count(2)` だけでは2 channel とも s0にならない。復帰後 `[0,1]` の期待が誤り。
- **系列6**：TX layout を宣言するだけでは TX clock はない。新旧 clock の取得に到達しない。

さらに系列6の旧 clock は、membership を直しても **arm 時刻0が origin**。sample `150_000` は150 msであり、`T0+100 ms` の cut 後ではなく receipt より過去になる（`crates/ezsdr-mock-radio/src/lib.rs:2375–2389`）。

**修正**：系列2・3に `radio.rx.c1.stream=0`、系列6に `radio.tx.c0.stream=0` の初期 constraint を追加する。旧TX burst の target は、その clock の origin・ratio から cut 後の index を計算する。拒否前に membership、clock、held 状態へ到達したことを assert する。

### 3. major — TX の重要な後続系列がなく、M9の殺傷も保証できない

**根拠：VERIFIED（既存の規則）、INFERRED（予定 mutation の生存可能性）**

系列6は「後続 change なし、open burst なし」の一例だけ。次を観測しない。

- halt 中に旧／拒否済み domain への新規 burst が拒否されること。
- 拒否後、次の cold change で正常に送信できること。
- 拒否された clock の burst だけを消し、後続 clock の burst を残すこと。
- 既に open な burst の cut と `BurstRecord`。
- SimulationChannel の予約済み信号の除去。

UHD は拒否後に clock を採用せず、後続 burst を clock ごとに選別する：

- `crates/ezsdr-radio-uhd/src/provider/tx.rs:242–254,718–769`

また、**拒否前に予約した次の change が既に clock を切っていれば、その end は残る**。計画の「拒否されたTX clockは常にoriginで終わる」は一般化し過ぎている。

- `crates/ezsdr-radio/tests/timeline.rs:292–296`
- `crates/ezsdr-radio/src/timeline.rs:846–853`

M9「拒否時にも `tx_switch` を呼ぶ」は、held burst を別途取り消していれば、系列6の既存assertだけでは current clock の誤採用を検出できない可能性がある。

**修正**：上記を短いTX系列として追加する。特に「cold A・Bを事前予約→各clockにburst→A拒否→Aだけ取消→Bで送信」を固定し、予約済み clock の origin/end が変わらないことも見る。channel mode の1例で、拒否したburstの信号が受信側に漏れないことを確認する。

### 4. major — D9の「未リンクRXも候補になる」という前提が既存コードと異なる

**根拠：VERIFIED**

`plan-decisions.md:5,11` は、plan を見るため未リンクRXの cold change も候補になるとしている。しかし MockRadio は未リンクRXの line に **T0の Stop を予約する**。

- `crates/ezsdr-mock-radio/src/lib.rs:2421–2427`
- `design/09-mock-radio.md:221`

off の line に cold change を入れても新 segment は作られない：

- `crates/ezsdr-radio/src/timeline.rs:919–947`

従って、通常の未リンクRXへの cold は、D1の「そのseqのsegmentがplanにある」という条件を満たさない。明示的な `start_rx` 後とは区別が必要。

**修正**：D9の事実認識を訂正し、「未リンクのまま cold→fault は未消費→別の稼働streamの coldで消費」を試験にする。未リンクでも明示的に start した場合を扱うなら、その系列を別に定義する。

### 5. major — D1・D3で変更した選択規則を識別する試験がない

**根拠：VERIFIED**

裁可後の規則は `(origin,seq)` ではなく **`(e,seq)`**、また `cut>0` は要求しない（`plan-decisions.md:5`）。

しかし：

- 系列8は ideal なので `e=origin`。`e<f≤origin` を除外する実装と、originで選ぶ誤実装を区別できない。
- 系列9も ideal・同時刻なので、effective順とorigin順の違いを見ない。
- RXで先行segmentが後続予約によりplanから消える場合、TXで `cut==0` のsegmentがplanに残る場合の試験がない。

M2・M13の修正だけでは、この裁可を固定できない。

**修正**：少なくとも以下を追加する。

1. x3xx の `e<f<origin`：その change は拒否せず、次の対象coldを拒否。
2. effective順とorigin順が異なる2 stream：effective順を採用。
3. 後続予約により先行RX segmentが消える場合と、TXのzero-sample segmentが残る場合：裁可どおりの消費対象を明示。

### 6. minor — spec変更一覧に、裁可後も残る矛盾がある

**根拠：VERIFIED**

`test-plan.md:109,112` は依然「`at_ns`以後に**始まる**segment」「`origin>=f`」「`cut>0`」をspecへ書く案であり、D1・D3の裁可と逆。

また `design/09-mock-radio.md:223` の MR-40 は「moveが拒否されたら applied もclockも変わらない」と読める。今回の**適用時の拒否**では、moveの値が記録に残り両lineがhaltする。既存の予約時拒否試験とは別である。

**修正**：spec差分を裁可後のeffective基準に統一する。MR-40の拒否を「予約できなかったmove」に限定し、予約後の拒否はRM-25／RM-34を参照する。Provider固有のfault選択規則はMR-20に一度だけ書き、他specは参照に留める。

### 7. minor — block期待値に誤記・観測時点の曖昧さがある

**根拠：VERIFIED**

- 系列1（`test-plan.md:67`）：`(first+len)·1000 == 3_000` は単位が不一致。1 Mspsで3 msなら、sample indexは3,000、root tick差は3,000,000。
- 系列8（`:79`）：3 msで `stop(Orderly)` した後まで読むなら、新clockの5,000 sampleのうち、既記載の2,000×2に加えて最後の1,000 sampleのblockも出る。stopは進行中blockをcutまで発行する（`crates/ezsdr-mock-radio/src/lib.rs:1175–1182`）。

**修正**：単位を訂正し、系列8のheader assertをstop前に行うか、stop後の末尾blockを期待値に含める。

---

D2・D4–D8・D10–D12は、上記修正を前提に概ね妥当。`applied`の形と拒否報告時刻の差は明記されている。ただしUHDの報告時刻はコード上 `core.now()` であり、厳密にRX origin／TX effective instantと等しい保証はない（`crates/ezsdr-radio-uhd/src/provider/core.rs:981–985`）。

ファイル変更、ビルド、試験、mutation実行はしていない。予定コードが未実装のため、mutationの最終的な殺傷判定は実装後に必要。

## コードレビュー（`sol-code`）

判定：FAIL

1. **major — halt 中、復帰用の将来 clock への正常な burst まで拒否する**
   - **箇所**：`crates/ezsdr-mock-radio/src/lib.rs:1542–1544`
   - **根拠：VERIFIED（コード追跡）**。cold A の拒否後、将来の cold B を予約すると、B の SampleClock は登録済みになる。しかし B の effective instant より前は `line.halted(now)` が真なので、B の clock を指定した burst も入口で拒否される。
   - UHD の `book_burst` は、最後の未終了 segment と登録済み clock を使ってこの予約を受理する（`crates/ezsdr-radio-uhd/src/provider/control.rs:1078–1093`）。したがって Mock が UHD より厳しい。brief S3(b) が求めた旧／拒否済み domain の拒否を超えている。
   - **修正**：halt 中でも、後続の有効な segment の clock への予約は許す。旧／拒否済み clock の拒否は維持する。`design/09-mock-radio.md:166` の「every transmit burst」も限定する。
   - **追加試験**：A 拒否 → B を将来時刻に予約 → **B の時刻より前に** B の clock へ burst を予約 → B で送信。既存の復帰試験は burst の受け取りを B の時刻まで進めるため、この穴を検出しない。

2. **minor — linkless RX は候補にならない、という新しい規定が広すぎる**
   - **箇所**：`design/09-mock-radio.md:166`
   - **根拠：VERIFIED（コード追跡）**。linkless stream に明示的な `start_rx` を出すと、リンクの有無を検査せず line に Start が予約される（`crates/ezsdr-mock-radio/src/lib.rs:1498–1508`）。その後の cold change は plan に segment を持ち、`refusal` の候補になる（同 `:2023–2027`）。
   - brief S4 はこの系列を範囲外としているが、spec は無条件に候補外と断定している。
   - **修正**：「T0 の Stop のままで、明示的に再開していない linkless RX」に限定する。範囲外の系列を実装する必要はない。

**確認結果**
- 対象：HEAD `230aba9` に対する未コミット差分。
- `ezsdr-mock-radio`：187 tests passed。
- `ezsdr-sim`：17 tests passed（schema freeze 含む）。
- P604–P621、F22、P589：置換元が各1回一致することを確認。

ファイル編集・mutation 実行はしていない。実機、workspace 全体、mutation の実際の殺傷は未確認。

## 直し 1 回目の再レビュー（`sol-fix1`）

判定：PASS

指摘なし（major 0、minor 0）。前回の2件は解消済み。

1. **halt 中の clock 判定 — VERIFIED**
   - `crates/ezsdr-mock-radio/src/lib.rs:1543–1548,1629–1636`
   - 後続 change B の clock への burst は、B の適用前でも受理される。旧 clock の cut 以降と、`SendAsap` が cut 以降へ動かした burst は拒否される。拒否された change の cut 0 の clock は MR-16 の lookup で拒否される。
   - 後続 clock を受理する点は UHD の `provider/control.rs:1078–1093` と整合する。

2. **隣接系列 — VERIFIED（コード追跡）**
   - `crates/ezsdr-mock-radio/src/lib.rs:1988–2011,2040–2050`
   - B の burst を予約後、2個目の fault が B を拒否すると、その burst は取消され、channel の予定からも除去される。
   - 旧 clock の cut 前を指定しても、halt 中では late policy が先に判定される。Drop／PlanViolation はそこで終了し、SendAsap で cut 以降へ移れば拒否される。

3. **spec・試験・mutation — VERIFIED**
   - `design/09-mock-radio.md:166`：linkless RX の候補外条件は、`start_rx` による再開までに限定された。
   - `crates/ezsdr-mock-radio/tests/mock_layout.rs:2663–2710`：B の適用10 ms前に予約を処理し、受理・BurstRecord・送信 block・受信 sample を確認している。
   - `plan/maintenance/tools/mutations.json:9481–9488,9571–9578`：P613 は新試験の受理 assertion、P622 は既存試験の拒否理由 assertion を破る。置換元は各1回一致。

実行確認：`mock_layout` の `mr_20_` 12試験すべて成功。

ファイル変更・mutation 実行なし。実機と隣接系列の追加実行は未確認。

