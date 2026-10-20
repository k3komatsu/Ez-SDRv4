## 1b-4d-1：Sol の計画レビュー（`sol-plan.md`）の裁定

Sol の major 1〜6 と minor 7〜9 をすべて採る．指示書 `brief.md` §2 の設計はこの裁定で次のとおり改める（ここが優先する）．

### 設計の改め

1. **設定の同一性は `(seq, 版)`**（Sol 1，指示書 §2 の 6 を改める）．
   - `ColdConfig` に版を持たせる．lock を取らずに読めるもの（`AtomicU64`）にする．
   - 版を上げるのは，投影が変わり，その変化を device が自分の timed command として受け取らないときだけ：channel が無いので記録だけになった release，投影に入っている更新の取消・移動．device に timed command として出した更新では上げない（全更新で configure し直す形にしない）．
   - 記録の configure の欄は `(seq, 版)`．版は `configure` が `updates` の lock の中で読んだ値（その投影で configure した版）．
   - owner の判断は，記録の `(seq, 版)` と，次の segment の `(by, 今の版)` を比べる．
2. **操作の担当（busy）の範囲**（Sol 2）．
   - busy は，その stream の streamer に触る device の呼び出し**すべて**を覆う：open，rate の照合，configure，`rx_start`，`rx_recv`，`tx_send`，`tx_async`（ACK と report の読み），Shutdown の ACK の読み．
   - owner は 1 回の begin・switch（open → 照合 → configure → start・clock の採用）の間，busy を 1 度だけ取って持ち続ける．途中で渡さない．
   - uhd-control の from-none は，owner が idle で busy が下りているときだけ busy を取る．owner は busy を見たら device を呼ばず，次の poll で読み直す（TX の `reports()` も呼ばない）．
   - busy を取る・下ろすのは line の lock の中．device を呼ぶ間は line の lock を持たない（今の規則のまま）．
   - 計画に，busy が覆う呼び出しの一覧と，下ろす境界（成功・失敗・lost・Shutdown）を表で書く．
3. **lock の順序**（Sol 6）．
   - 記録との照合は `streams` の lock の中で，版は atomic で読む．`streams` の中で `updates` の lock を取らない．
   - 成功の記録は configure が返った後に `streams` を取って書く．`updates` → `streams` → `rec` の順を変えない．
4. **minor 7**：`arm` の照合の拒否も `Unknown`．prune は記録を変えない（configure の欄も残す）．拒否された RX の segment は plan に入らないので S21 は「拒否の後に次の enable を記帳して戻る」系列に置き換える．「拒否された segment では device を呼ばない」は今の振る舞いではなく，この段階の新しい規則として書く（TX の今の `do_switch` は拒否の判定より先に open・configure しうる）．

### 計画に足す・直す系列（期待つき）

| # | 系列 | 期待 |
|---|---|---|
| S11a | from-none の configure の後，origin より前に効く timed 更新を記帳し，device に timed command として出る | configure し直さない．timed command は 1 回．origin の後の fake の設定の read-back（または周波数に依る loopback の出力）がその値 |
| S11b | channel が空で owner は idle → 将来の enable を from-none で configure → その effective instant より前に効く timed 更新を記帳 → release の時点で channel が無いので記録だけになる → owner が segment を始める（Sol 1） | 版が上がっているので owner が configure し直す．timed の device 呼び出しは 0 回．read-back がその値 |
| S11c | 投影に入った更新の取消・移動 | 版が上がり，owner が configure し直す |
| S9（作り直し） | S1 と同じく owner が始められる古い B を残し，後の C の from-none の `rx_open`（TX は `tx_open`）を止める．RX と TX の両方 | owner は B の device 呼び出しも採用もしない．owner の cursor・pending の switch は待つ間に進まない．busy の確認を消した実装ではこのテストが落ちること |
| S22 | control の `tx_open` を止めたまま TX の通常の poll | `tx_async` を含め，streamer の呼び出しを始めない |
| S23 | TX の `tx_async` を止めたまま control が enable を受け取る | read が終わるまで streamer を置き換えない（from-none は busy を待つか，owner が idle でないとして from-none にしない） |
| S24 | configure の成功から `rx_start`・TX の clock の採用まで | busy を途中で下ろさない |
| S25 | `{0,1}` で channel 0 の apply は成功，channel 1 の apply の失敗または read-back の拒否 | `Open({0,1})` のまま，configure の欄は一致しない．次の成功の segment は開き直さず両 channel を configure |
| S26 | control の open・configure を止めている間に，別の thread が lost を記録 | 戻った後，rate の照合・apply・start を始めない |
| S27 | busy の最中に Shutdown | busy を下ろして終わる．TX の ACK の待ちと RX の終わりの処理を妨げない．以後 segment を始めない |
| S28 | 同じ投影への timed の book・release と owner の configure を重ねる（Sol 6） | 完了する（deadlock しない）．照合した版と成功の記録が食い違わない．逆の lock の順を取る実装を落とす検査（LockSpy に `updates` の取得の記録を足すか，順序の assert） |

直すもの：
- **S17**：Stop の後も，既存の stream の終わりの呼び出し（RX の drain と untimed stop，TX の EOB と ACK の読み）は許す．新しい segment は始めない．
- **S5・S10 の観測**：TX の clock は記帳のときに登録される．期待は「owner が採用しない，burst を送らない，拒否された clock は origin で終わる」．
- **S1 の「落とす誤り」**：「成功の前に `Open` を書く」は S1 では落ちない．失敗の系列か，呼び出しの中での観測（S13b）に割り当てる．
- **S10 と P350**：S10 は P350（device の channel 番号と streamer の中の位置の区別）を検査しない．P350 の既存のテストを残す．
- **arm**：§1 の表と S20 の食い違いを `Unknown` にそろえる．
- **prune**：§1 の表と S16 の食い違いを「configure の欄も残す」にそろえる．
- **S21**：上の minor 7 のとおり置き換える．

### mutation の行（Sol 8）

- P361〜P367，P317，P318 のそれぞれについて，その行が守っていた規則を，新しいどの行とどのテストに移すかを対応表にする．「コードが消えるので消す」だけにしない．
- P358（拒否の判定を前に移すなら当て直す），P351（from-none の書き換え）は「残す」と断定せず，実装のときに `old` の一致と filter を確かめる．

### 大きさ（Sol 9）

- 見積もりを出し直す．Stop・loss・arm・Shutdown の系列を次の段階へ送る分け方はとらない（この段階が変える lifecycle の安全の条件）．
- 超えるときは，まずテストの重なりを helper でまとめる．それでも超えるなら，この変更を安全に閉じられる単位での分け方を書く（orchestrator が決める）．

### 採らないもの

- 計画 §8 の 5（新しい機構のラベル `mech:device-state-copy`）：既存の `planned-state-read-as-actual` で数える（`plan/maintenance/failure-mechanisms.md` の 1b-4c の段落）．
- device の失敗の注入は計画の推奨どおり test 側の wrapper（`ScriptedDevice`）．`FakeFault` は増やさない．

---

STATUS: done

# Phase 8 段階 1b-4d-1 テスト計画（改訂版）

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `f487877`．
根拠：`brief.md`（指示書）と `adjudication.md`（orchestrator の裁定，指示書より優先）．
コードは書いていない．tree は変えていない．mutation・`cargo fmt`・commit・push・`git add` はしていない．

## 裁定の反映

| 裁定 | 計画での扱い（節） |
|---|---|
| 設計 1：設定の同一性は `(seq, 版)`．版は atomic | §1 の表，§3 の版の規則（S11a・S11b・S11c） |
| 設計 1：版の上げ方（record-only release，取消・移動の両方，device が timed で受けた更新では上げない） | §3 の版の規則，S11b・S11c |
| 設計 1：記録の configure の欄は `configure` が `updates` の中で読んだ版 | §3 の版の規則，§4 の lock 順 |
| 設計 1：owner の判断は `(by, 今の版)` と比べる | §2 の owner の判断，S11b |
| 設計 2：busy は device を呼ぶ streamer の呼び出しすべてを覆う（open・照合・configure・`rx_start`・`rx_recv`・`tx_send`・`tx_async`・Shutdown の ACK の読み） | §5 の busy 対応表 |
| 設計 2：owner は 1 回の begin・switch の間 busy を持ち続ける | §5，S24 |
| 設計 2：uhd-control の from-none は owner が idle かつ busy が下りているときだけ | §5，S9，S23 |
| 設計 2：busy を見た owner は device を呼ばず，次の poll で読む（TX の `reports()` も呼ばない） | §5，S9，S22 |
| 設計 2：busy を取る・下ろすのは line の lock の中 | §5，S13 |
| 設計 2：release 境界（成功・失敗・lost・Shutdown）の表 | §5 の表 |
| 設計 3：照合は `streams` の中で，版は atomic で読む．`streams` の中で `updates` を取らない | §4，S28 |
| 設計 3：成功の記録は configure が返った後に `streams`，次に `rec` の順 | §4，S28 |
| 設計 4（minor 7）：`arm` の照合の拒否も `Unknown` | §1 の表，S20 |
| 設計 4（minor 7）：prune は記録を変えない（configure の欄も残す） | §1 の表，S16 |
| 設計 4（minor 7）：拒否された RX の segment は plan に入らない．S21 を置き換える | S21（置き換え），§6 |
| 設計 4（minor 7）：「拒否された segment は device を呼ばない」は新しい規則として書く | §2，S10，S21 |
| 系列 S11a・S11b・S11c を追加 | S11a・S11b・S11c |
| S9 を作り直す（B を残し，C の from-none の open を止める．RX と TX） | S9 |
| S22・S23・S24・S25・S26・S27・S28 を追加 | 各系列 |
| 直す：S17（Stop の後も既存 stream の終わりの呼び出しは許す） | S17 |
| 直す：S5・S10 の観測（clock は記帳で登録．owner は採用しない，burst を送らない，拒否された clock は origin で終わる） | S5・S10 |
| 直す：S1 の「落とす誤り」（成功の前に `Open` を書く）は S1 では落ちない．S13b に割り当てる | S1，S13b |
| 直す：S10 と P350（device の channel 番号と streamer の位置の区別）．P350 の既存テストを残す | S10，§7 |
| mutation：P361〜P367，P317，P318，P358，P351 の対応表 | §7 |
| 大きさの見積もりを出し直す．Stop・loss・arm・Shutdown を次段階に送る分け方はしない | §8 |
| 超えるときの分け方（orchestrator が決める） | §8（分け方の候補） |
| 採らない：`mech:device-state-copy`．既存の `planned-state-read-as-actual` で数える | §9 |
| 採らない：`FakeFault` は増やさない．失敗の注入は test 側の `ScriptedDevice` | §9，§10 |

**Owner に上げる未決（orchestrator の判断が要る）**

- **D1（S11d）**：from-none の configure の後，owner が segment を始める前に，その origin より前に効く timed 更新が記帳されると，owner はその segment を始める時点で skip を決めてしまい，版の上がった後に record-only release が device に届かない（§S11d に詳しい）．既存の hazard（HEAD `f487877` の 1b-4c にもある）．選択肢：（A）record-only の更新を，次の segment が channel を持つときに device へ送る（UR-24 を変える）．（B）pending な from-none の cold change の origin で `e` を切り上げる（RM-25 が layout に行っていることと同じ．UR-24・RM-25 を変える）．（C）版だけに頼る（棄却：窓が残る）．**推奨は B**．
- **D2（busy と from-none）**：from-none の瞬間に busy が立っていたら，from-none は device を呼ばず，通常の cold change として記帳する．owner は次の poll で拾う（待たない）．現在の計画は D2 を採った前提で書く．orchestrator の確認を待つ．

## 1. 設計：1 つの記録（`StreamState`）

`StreamState`（`core.rs`，lines ~99–117）に，その stream の streamer について device が実際に持つものを置く．すべて `streams` の lock の下で読み書きする．

| 項目 | 型 | 意味 | 書く場所 |
|---|---|---|---|
| `set` | `Unknown` または `Open(Channels)` | device が開いている channel の集合（分からないなら `Unknown`） | open の成功で `Open(集合)`．open の失敗・照合の拒否・arm の照合の拒否で `Unknown`．prune は変えない |
| `configured` | `Option<(seq, 版)>` | 最後に成功した configure の segment の `seq` と，その configure が読んだ版 | 全 channel の configure と read-back が通った後にだけ書く（呼び出し後に `streams` → `rec`）．configure の失敗は消す．open の成功・失敗では消す．prune は残す |
| `busy` | `bool` | device の呼び出しが進行中（§5） | 呼び出しの前に `streams` の中で立て，後に下ろす |

- 集合が `Open` で configure の欄が `None` の場合，その集合は device が開いていて，どの segment の configure も確かでないことを表す．
- configure の欄が `Some((seq, v))` でも，その後の版（`ColdConfig` の版）が `v` と違えば，その configure は今の投影のものではない．
- `StreamState.stale`，`StreamState.configs` の bool，`Planned.configured`，RX の `Rx.opened`，TX の `verified` を消す．`StreamState.configs` は `(seq, 版)` を持つ形に変える（§3）．
- 残すもの：実行中の segment の情報（RX の `Stream` の channel 数と block の解釈，TX の `channels` を burst の channel 数と `unacked` に使う部分），UR-13 の保存した照合の結果（`check_rates`，`rates` は最初の結果を保つ）．

### owner の判断（RX の `begin`，TX の `do_switch`）

次の segment について，`by` を from-none の seq（owner が始める segment の seq），`今の版` を `ColdConfig` の版（atomic，`updates` の lock なしで読む）とする．

| 記録 | 判断 | device の呼び出し |
|---|---|---|
| `Open(集合)` かつ `configured == Some((by, 今の版))` | 何もしない | なし |
| `Open(集合)` かつ集合が一致し，configure の欄が一致しない | configure だけ | `apply`（timed）の列 |
| それ以外（`Unknown`，集合が違う，`configured` が `None`） | 開き直し → rate の照合（保存した結果があれば再適用）→ configure | `rx_open`・`tx_open`，照合，`apply` |
| `Planned.refused` の segment | device を呼ばない（TX は clock を始めない） | なし |

- `by` が `None` のときは一致しないので，owner が configure する．
- 拒否された segment は plan に入らない（RX）か，clock を origin で終わらせる（TX）．どちらも device を呼ばない．これは今の振る舞いではなく，この段階で新しく書く規則である（TX の今の `do_switch` は拒否の判定より先に open や configure をしうる．INFERRED，実装で確かめる）．

### 版（`ColdConfig` の版）

- `ColdConfig` に `AtomicU64` の版を持たせる．読むのは lock なしでよい．
- **上げる場合**（投影が変わり，その変化を device が自分の timed command として受け取らないとき）：
  - channel が無くなり記録だけになった release（`release` の `chans.is_empty()` の枝．今は `continue` で `issued` の前に抜ける）．
  - 投影に入っている更新の取消（`cancel_held`）と移動．
- **上げない場合**：device に timed command として出した更新（全更新で configure し直す形にしない）．
- 版を上げるのは `updates` の lock の中．
- configure の記録に入る版は，`configure` が `updates` の中で読んだ値（その投影で configure した版）．

## 2. 順序とテストの形

- device の呼び出しは `FakeDevice::calls()` で見る．owner の thread は直接動かす（`read`・`begin`・`step`，`rx.rs`・`tx.rs` の既存 unit test と同じ形）か，manual clock と settle で作る．sleep で競合を作らない．
- 競合は `Barrier` で device の呼び出しを止めて作る．止める場所は test 側の wrapper（`ScriptedDevice`，§10）．
- 各 test の assert は 1〜2 個．
- 「落とす誤った実装」は，その系列の規則を壊す具体的な実装を書く．

## 3. 版の規則と記録の更新（系列の前提）

- 記録の `configured` は `(seq, 版)`．owner の判断は `(by, 今の版)` と一致するときだけ configure を省く（§1）．
- 版は device の timed command で届く更新では上がらない．record-only release と取消・移動では上がる（§1）．
- 「seq だけでは設定の同一性にならない」（Sol 再レビュー 3）への答え：版を足す．brief の推奨（seq のまま）は，record-only release と取消が版で拾えるので採らない．

## 4. lock の順序（規則）

- 順序：`updates → streams → rec`．この順以外で取らない．
- 照合（`configured == (by, 今の版)` の比較）は `streams` の中で行う．版は atomic で読むので `updates` を取らない．
- 成功の記録は configure が返った後に `streams` を取り，その中で `rec` を取って書く．
- device を呼ぶ間は，`updates`・`streams`・`rec` のどれも持たない（UR-25）．**注意**：現在の `ColdConfig::configure`（`control.rs` 132 以降）は `updates` を device の呼び出しの間持っている．この段階で `updates` の持ち方を変えるか，版を configure の前に読んで持たないかを実装で決める．どちらでも S13 と S28 が確かめる（§S13，§S28）．
- `rec` は `check_rates` の中の `configure` のループでも取られている（`configure` が channel ごとに `rec` を取る）．その取り方は変えない．

## 5. busy（操作の担当）

### busy が覆う呼び出し（device を呼ぶ streamer の呼び出しすべて）

| 呼び出し | busy の取り方 | 場所 |
|---|---|---|
| `rx_open`・`tx_open` | owner の per-begin/switch busy（§5 の下段） | `rx.rs` `begin`，`tx.rs` `do_switch`，uhd-control の from-none（`control.rs` ~790–846） |
| rate の照合（`check_rates`） | owner の per-begin/switch busy | `core.rs` 553–570，`mod.rs` の `arm` 564–605 |
| `configure`（`apply` の列） | owner の per-begin/switch busy，または from-none の busy | `core.rs` 583 以降，`control.rs` 132 以降 |
| `rx_start`（begin と restart） | owner の per-begin busy（begin）．restart は per-call busy | `rx.rs` 314，443；`mod.rs` の `start()` 650 |
| `rx_recv` | per-call busy | `rx.rs` 247 |
| `rx_stop`（drain・untimed stop を含む） | per-call busy | `rx.rs` 620 |
| `tx_send`（EOB を含む） | per-call busy | `tx.rs` `close_device_burst` ~507 |
| `tx_async`（ACK と report の読み） | per-call busy | `tx.rs` `reports()`（run loop ~149，`tx_async` ~716） |
| Shutdown の ACK の読み（`reports(10 ms)` を 100 ms 繰り返す） | per-call busy | `tx.rs` `shutdown()` 789–800 |
| `cleanup` の `close_streams` | per-call busy | `mod.rs` 719 |

- owner は 1 回の begin・switch（open → 照合 → configure → start・clock の採用）の間，busy を 1 度だけ取って持ち続ける．途中で渡さない．
- uhd-control の from-none は，owner が idle かつ busy が下りているときだけ busy を取る（条件は `idle && !busy && planned`）．
- busy を見た owner は device を呼ばず，何も始めない．次の poll で読み直す．TX の `reports()` も呼ばない（D2 の前提）．
- busy を取る・下ろすのは `streams` の lock の中．device を呼ぶ間は，line の lock を持たない（UR-25）．

### 下ろす境界

| 境界 | 下ろす場所 |
|---|---|
| 成功 | 呼び出しの後に `streams` を取って下ろす（成功の記録と同じ lock の区間） |
| 失敗（device のエラー） | 同じ．失敗の記録（§1 の表）を同じ区間で書く |
| lost（RM-16） | 同じ．lost の後は segment を始めない．busy は呼び出しが返った後に下りる |
| Shutdown | ACK の読みが終わった後に下りる．Shutdown の後は segment を始めない |

## 6. 系列（S1–S28，S11d を含む）

各系列の形式：テスト名，置く場所（unit か `tests/fake.rs`），assert（1〜2 個），観測，落とす誤った実装．

**S1**：RX：owner は止める．uhd-control が from-none で B=`{0,1}` を open・照合・configure → 後の cold で集合を空にし B を正の長さで終える → uhd-control が from-none で C=`{0}` を open・configure → owner を動かす．
- テスト名：`ur_13_a_from_none_segment_the_owner_never_started_is_opened_again`．置く場所：`rx.rs` unit の隣（`ur_13_a_stale_receive_streamer_is_opened_again` の置き換え）．
- assert：（1）owner が B を始める前に `rx_open` が `{0,1}` で出る．（2）C の segment の前に `rx_open` が出る．
- 観測：`calls()` の列．
- 落とす誤り：`stale` を残す，または `configured` の近道を残す（owner が開き直さず B または C を始める）．

**S2**：TX：S1 と同じ形．
- テスト名：`ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again`．置く場所：`tx.rs` unit．
- assert：C の clock は C の segment の origin で始まる．`tx_open` が C の前に出る．
- 落とす誤り：`verified` を残す（`do_switch` が開き直さない）．

**S3**：RX・TX：from-none が成功し，owner がその segment を始める．
- テスト名：`ur_25_a_successful_enable_from_no_stream_is_started_without_a_second_open`．置く場所：`tests/fake.rs`（`ur_25_an_enable_from_no_stream_configures_once` の隣．既存のテストと統合してよい）．
- assert：`rx_open`・`tx_open`・`apply` の数が from-none の 1 回分だけ．
- 落とす誤り：owner が記録を見ずに開き直す（呼び出しが増える）．

**S4**：from-none の open が失敗 → 同じ集合の次の enable．
- テスト名：`ur_13_a_failed_open_from_no_stream_is_opened_again_by_the_owner`．置く場所：`control.rs` unit（`ScriptedDevice` で `rx_open` の 1 回目を失敗させる）．
- assert：記録の `set` は `Unknown`．次の segment で `rx_open` が出る．
- 落とす誤り：失敗の後に `Open(集合)` を書く．

**S5**：from-none の rate の照合が拒否 → 同じ集合の次の enable．
- テスト名：`ur_13_a_rate_refusal_from_no_stream_is_reapplied_and_starts_nothing`．置く場所：`control.rs` unit と `tx.rs` unit．
- assert：（1）開き直しで保存した拒否が再適用され，`apply` は出ない．（2）clock は登録されない（TX）または burst は始まらない（RX）．TX の clock は記帳のときに登録されるので，観測は「owner が採用しない，burst を送らない」．
- 落とす誤り：拒否の結果を捨て，照合をやり直して通す．

**S6**：from-none の open と照合は成功，configure が失敗 → 同じ集合の次の segment．
- テスト名：`ur_13_a_failed_configure_from_no_stream_is_configured_again_without_reopen`．置く場所：`control.rs` unit．
- assert：次の segment で `rx_open` は出ず，`apply` だけが出る．記録の `set` は `Open(集合)`，`configured` は `None`．
- 落とす誤り：configure の失敗で `Unknown` を書く（開き直しが起きる）．

**S7**：owner の open が失敗 → 同じ集合の次の segment．
- テスト名：`ur_13_a_failed_open_by_the_owner_is_opened_again`．置く場所：`rx.rs` unit．
- assert：`rx_open` が次に出る．記録の `set` は `Unknown`．
- 落とす誤り：`opened` を残す（`Rx` の写し）．

**S8**：owner の configure が失敗 → 同じ集合の次の segment．
- テスト名：`ur_13_a_failed_configure_by_the_owner_is_configured_again_without_reopen`．置く場所：`tx.rs` unit．
- assert：`tx_open` は出ず，`apply` だけが出る．
- 落とす誤り：configure の失敗で `Unknown` を書く．

**S9（作り直し）**：S1 と同じく owner が始められる古い B を残す．後の C の from-none の `rx_open`（TX は `tx_open`）を止める．RX と TX の両方．
- テスト名：`ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing`（RX と TX の 2 本）．置く場所：`tests/fake.rs`．
- 進め方：`ScriptedDevice` で C の `rx_open`（TX は `tx_open`）を `Barrier` で止める．owner の poll を 1 回動かす．
- assert：（1）その間，owner は device を呼ばない（`calls()` の数が増えない）．（2）owner の cursor と pending の switch は進まない．barrier を解いた後，次の poll で C を読み直して始める．
- 観測：`calls()` と owner の cursor（`Rx`・`Tx` の内部 state を test の中から読む helper）．
- 落とす誤り：busy の確認を消した実装（owner が B を始めてしまう）．この test はそれで落ちる．

**S10**：TX：from-none で拒否された enable．
- テスト名：`ur_25_a_transmit_refusal_from_no_stream_starts_no_clock`（既存．置き換えて残す）．置く場所：`tx.rs` unit．
- assert：（1）clock は origin で終わる（拒否された clock を採用しない，burst を送らない）．（2）`unacked` の対応は変わらない．
- 観測：`Lines` の clock の終わり，`calls()` の `tx_send`．
- 落とす誤り：`Ok(()) if to.refused => self.at += 1` を消す（P358 の対象）．S10 は P350（device の channel 番号と streamer の位置の区別）を検査しない．P350 の既存テストを残す（§7）．

**S11a**：from-none の configure の後，origin より前に効く timed 更新を記帳し，device に timed command として出る．
- テスト名：`ur_24_a_timed_update_after_a_from_none_configure_is_sent_without_configuring_again`．置く場所：`control.rs` unit．
- assert：（1）`apply` は timed の 1 回だけ．（2）configure は繰り返さない（版は上がらない）．
- 観測：`calls()` の `apply … at=<t> effective=…`，read-back（`st.settings`）がその値．
- 落とす誤り：版を上げる（device が受けた timed 更新で configure し直す）．

**S11b**：channel が空で owner は idle → 将来の enable を from-none で configure → その effective より前に効く timed 更新を記帳 → release の時点で channel が無いので記録だけになる → owner が segment を始める．
- テスト名：`ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again`．置く場所：`control.rs` unit．
- assert：（1）版が上がっている．（2）owner が configure し直す．（3）timed の device 呼び出しは 0 回．（4）read-back がその値．
- 落とす誤り：版を上げない（記録の `(seq, 版)` が一致して owner が configure を省く）．

**S11c**：投影に入った更新の取消・移動．
- テスト名：`ur_24_a_cancelled_projected_update_makes_the_owner_configure_again`．置く場所：`control.rs` unit（`cancel_held` と移動の両方）．
- assert：版が上がり，owner が configure し直す．
- 落とす誤り：取消で版を上げない．

**S11d（D1，未決）**：from-none の configure の後，owner が segment を始める前に，その origin より前に効く timed 更新が記帳される．その後 release が channel を空にする．
- 現象：owner は pending の segment を即座に始める（`rx.rs` `turn()` → `begin`，`tx.rs` `read()` / `retarget`，cut は MIN）．その時点で，owner は timed 更新の skip を決めてしまい，版は後で上がるが，record-only の release は device に届かない．
- 状態：既存の hazard（HEAD `f487877` の 1b-4c にもある）．
- テスト：D1 の裁定を待つ．選択肢ごとに形が変わる．
  - （A）`ur_24_a_record_only_release_is_sent_to_the_device_when_the_next_segment_holds_the_channel`：record-only の更新を device に出す．
  - （B）`ur_24_a_timed_update_is_floored_at_the_origin_of_a_pending_from_none_change`：pending な from-none の cold change の origin で `e` を切り上げる．RM-25 が layout に使っている形と同じ．
- 落とす誤り（B）：切り上げをしない実装（`e < origin` の更新が booked のまま，skip が決まる）．

**S12**：`arm` で開いた streamer の集合のまま，最初の segment が始まる．
- テスト名：`ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer`．置く場所：`mod.rs` の arm 経路を通す `tests/fake.rs`．
- assert：`rx_open`・`tx_open` が arm の 1 回だけ．
- 落とす誤り：arm が記録を書かない（owner が `Unknown` と見なして開き直す）．

**S13**：全経路で device の呼び出しの間に line の lock を持たない（`test_support` の `LockSpy`）．
- テスト名：`ur_25_the_line_lock_is_free_inside_every_device_call`（既存の `ur_25_the_line_lock_is_free_inside_every_control_device_call` と `…receive_device_call` を統合または拡張）．置く場所：`control.rs`，`rx.rs` の unit．
- assert：`LockSpy::enter()` が各呼び出しで `streams.try_lock().is_ok()` を満たす．
- 観測：`LockSpy`．
- 落とす誤り：busy を `streams` の lock の下で device を呼ぶ実装（`try_lock` が落ちる）．

**S13b**：`Open` の書き込み順（成功の前に `Open` を書かない）．
- テスト名：`ur_25_an_open_is_recorded_only_after_the_device_returns`．置く場所：`control.rs` unit（`ScriptedDevice` の `rx_open` の中で記録を読む）．
- assert：`rx_open` の呼び出しの中で，記録の `set` は `Unknown`．呼び出し後に `Open(集合)`．
- 落とす誤り：呼び出しの前に `Open` を書く．

**S14**：TX の clock は記帳で登録されるが，owner は採用しない（S5・S10 の観測の再定義）．
- テスト名：`ur_25_a_refused_transmit_clock_is_not_adopted`．置く場所：`tx.rs` unit．
- assert：`burst` が送られない．拒否された clock は origin で終わる．
- 落とす誤り：拒否された clock を採用する．

**S15**：RX の restart の `rx_start` の busy．
- テスト名：`ur_25_a_restart_is_started_under_the_busy_flag`．置く場所：`rx.rs` unit．
- assert：`rx_start` の呼び出しの中で `busy` は立っている．
- 落とす誤り：restart で busy を取らない（別の thread の `rx_stop` と重なる）．

**S16**：prune は記録を変えない（configure の欄も残す）．
- テスト名：`ur_17_a_prune_keeps_the_device_record_and_the_configure_mark`．置く場所：`control.rs` unit（既存の `ur_17_a_stream_s_prune_leaves_the_other_stream_alone` の隣）．
- assert：prune の後，`set` と `configured` は変わらない．
- 落とす誤り：prune が `configured` を消す（次の segment で configure が省かれない）．

**S17**：Stop の後も既存の stream の終わりの呼び出しは許す．新しい segment は始めない．
- テスト名：`ur_17_after_stop_only_the_ending_calls_of_an_open_stream_are_made`．置く場所：`rx.rs` unit と `tx.rs` unit．
- assert：（1）RX の drain と untimed stop は出る．TX の EOB と ACK の読みは出る．（2）新しい `rx_open`・`tx_open`・`apply`・`rx_start` は出ない．
- 落とす誤り：Stop で終わりの呼び出しまで止める（ストリームが閉じられない）．

**S18**：RX：拒否された segment（`Planned.refused`）は owner が始めない．device を呼ばない．
- テスト名：`ur_25_a_refused_receive_segment_makes_no_device_call`．置く場所：`rx.rs` unit．
- assert：拒否された segment の後の `calls()` に `rx_open`・`apply`・`rx_start` が出ない．
- 落とす誤り：拒否の判定より前に開き直しや configure をする（「拒否は device を呼ばない」規則の新しい部分）．

**S19**：TX：拒否された from-none の後，owner の次の switch が記録から判断する（`set` は `Open` のままか `Unknown`）．
- テスト名：`ur_25_a_refused_transmit_switch_decides_from_the_record_on_the_next_switch`．置く場所：`tx.rs` unit．
- assert：（1）記録が `Open(集合)` なら，次の判断は configure だけか何もしない．（2）記録が `Unknown` なら開き直す．`tx_open` の数で区別する．
- 落とす誤り：`verified` の代わりに拒否の結果を別の変数に持ち，記録と食い違わせる．

**S20**：`arm` の照合の拒否は `Unknown`．
- テスト名：`ur_25_a_refused_check_at_arm_leaves_the_set_unknown`．置く場所：`mod.rs` 経由の `tests/fake.rs`．
- assert：記録の `set` は `Unknown`．次の segment で開き直す．
- 落とす誤り：拒否の後も `Open(集合)` を残す．

**S21（置き換え）**：拒否された RX の segment は plan に入らない．代わりに，拒否の後に次の enable を記帳して戻る系列を置く．
- テスト名：`ur_25_a_refused_receive_enable_is_booked_and_the_next_enable_starts`．置く場所：`control.rs` unit と `tests/fake.rs`．
- assert：（1）拒否の後，記録は `Unknown` か `Open(前の集合)`．（2）次の enable が正しく始まる（`rx_open` の集合が次の enable のもの）．
- 落とす誤り：拒否の後に記録を書き換えず，次の enable を前の集合のまま始める．

**S22**：control の `tx_open` を止めたまま TX の通常の poll．
- テスト名：`ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call`．置く場所：`tx.rs` unit（`ScriptedDevice` で `tx_open` を `Barrier` で止める）．
- assert：`tx_async` を含め，streamer の呼び出しを始めない（`calls()` の数が増えない）．
- 落とす誤り：busy を見ずに `reports()` を呼ぶ．

**S23**：TX の `tx_async` を止めたまま control が enable を受け取る．
- テスト名：`ur_25_a_from_none_enable_waits_for_a_transmit_call_in_progress`．置く場所：`control.rs` unit．
- assert：read が終わるまで streamer を置き換えない．from-none は busy を待つか，owner が idle でないとして from-none にしない（D2）．
- 落とす誤り：busy を見ずに from-none で `tx_open` を出す（`tx_async` と並ぶ）．

**S24**：configure の成功から `rx_start`・TX の clock の採用まで．
- テスト名：`ur_25_busy_is_held_from_configure_to_the_start`．置く場所：`rx.rs` unit と `tx.rs` unit．
- assert：`configure` の返った後から `rx_start`（TX は clock の採用）まで，`busy` は下りない．
- 落とす誤り：configure の後で busy を下ろし，その間に from-none が割り込む．

**S25**：`{0,1}` で channel 0 の apply は成功，channel 1 の apply の失敗または read-back の拒否．
- テスト名：`ur_25_a_partly_applied_set_stays_open_and_is_configured_whole_next_time`．置く場所：`control.rs` unit（`FakeDevice` の `WrongRate` または `ScriptedDevice` の apply 失敗）．
- assert：（1）記録は `Open({0,1})`，`configured` は `None`．（2）次の成功 segment は開き直さず両 channel を configure する（`apply` が 2 つ）．
- 落とす誤り：部分的な成功で `configured` を書く．

**S26**：control の open・configure を止めている間に，別の thread が lost を記録する．
- テスト名：`ur_25_a_lost_stream_starts_no_rate_check_apply_or_start_after_the_wait`．置く場所：`control.rs` unit（`Barrier` で止める）．
- assert：戻った後，rate の照合・apply・start を始めない．
- 落とす誤り：lost の確認を戻った後に行わない．

**S27**：busy の最中に Shutdown．
- テスト名：`ur_25_a_shutdown_during_a_busy_call_ends_without_starting_a_segment`．置く場所：`tx.rs` unit と `mod.rs` の `cleanup` 経路．
- assert：（1）busy は下りて終わる．（2）TX の ACK の待ちと RX の終わりの処理を妨げない（終わりまで返る）．（3）以後 segment を始めない．
- 落とす誤り：Shutdown で busy を下ろさない（終わらない）．

**S28**：同じ投影への timed の book・release と owner の configure を重ねる．
- テスト名：`ur_25_a_book_and_release_overlapping_a_configure_neither_deadlock_nor_mismatch`．置く場所：`control.rs` unit（`Barrier`，`LockSpy` に `updates` の取得を記録する拡張，または順序の assert）．
- assert：（1）完了する（deadlock しない）．（2）照合した版と成功の記録が食い違わない（記録の `(seq, 版)` は configure が読んだ版と一致）．（3）`updates` → `streams` → `rec` の順以外を取らない．
- 落とす誤り：逆の順（`streams` の中で `updates` を取る）．

## 7. 既存のテストの扱い

| 既存のテスト（場所） | 扱い | 理由 |
|---|---|---|
| `ur_13_an_enable_from_no_stream_marks_the_receive_streamer_stale_whatever_it_gives`（`control.rs` 1095） | 書き換え（S1 の assert へ） | `stale` が消える |
| `ur_13_a_stale_receive_streamer_is_opened_again`（`rx.rs` 1123） | 置き換え（S1） | `stale` が消える |
| `ur_25_a_refused_segment_is_booked_by_uhd_rx`（`rx.rs` 1143） | 残す | 拒否の記帳 |
| `ur_17_an_ended_segment_prunes_the_items_before_it`（`rx.rs` 1167） | 残す | prune の範囲は変わらない |
| `ur_25_the_line_lock_is_free_inside_every_control_device_call`（`control.rs` 1687） | 拡張（S13） | busy を覆う呼び出しを増やす |
| `ur_17_a_stream_s_prune_leaves_the_other_stream_alone`（`control.rs` 1857） | 残す，S16 を足す | |
| `ur_25_the_line_lock_is_free_inside_every_receive_device_call`（`rx.rs` 1327） | 拡張（S13） | |
| `ur_25_an_enable_from_no_stream_configures_once`（`tests/fake.rs` 3973） | 残す（S3 と同じ観測） | 呼び出しの数 |
| `ur_25_a_transmit_refusal_from_no_stream_starts_no_clock`（`tx.rs` 1589 付近） | 残す（S10） | P358 の対象 |
| `ur_25_a_refused_enable_leaves_its_set_unopened_for_the_next_switch`（`tx.rs`） | 書き換え（S5） | `verified` が消える |
| `ur_13_a_refused_switch_to_another_set_leaves_the_old_set_unverified`（`tx.rs`） | 書き換え（S4・S5） | `verified` が消える |
| `ur_13_a_switch_to_the_verified_set_opens_nothing`（`tx.rs`） | 書き換え（S3 へ） | `verified` が消える |
| `ur_25_a_refused_enable_from_no_stream_halts_and_keeps_its_values`（`control.rs`） | 残す，P351 の対象 | |
| `tx.rs` の残りの unit（861・887・924・945・964 付近） | 実装時に 1 本ずつ中身を読んで分類する（この計画時点では未確認） | `verified` と `configured` を見ている可能性がある |
| `tests/fake.rs` の `ur_13_*`・`ur_25_*`（3719・5953・6008・6225・6278 付近） | 同上（未読．分類は実装時） | |

- 「テストの中身を未読」の行は，この計画の中で分類を確定していない．実装の前に，各 test が `verified`・`stale`・`configured`・`opened` を読むかを grep で確かめて，残す・書き換える・消すを決める．

## 8. 大きさ（見積もり）

- 本体（`core.rs`・`control.rs`・`rx.rs`・`tx.rs`・`mod.rs`）：約 350〜450 行の変更（消すものを含む）．
- テスト：S1–S28 と S11d・S13b を合わせた 30 本程度（S11d は D1 の結論で本数が決まる）と既存の書き換え，`ScriptedDevice` と helper（Manifest の section を引く式，`COMMAND_REJECTED` の理由の取り出し，device の準備，owner の cursor を読む helper）で約 600〜800 行．
- 合計：約 1,000〜1,300 行．D1（S11d）の選択（A か B）と D2 の結論で増減する．
- 不確かさ：D1 で（A）を採ると device に出す更新の経路が増え，約 100 行増える．D2 で busy の待ちを入れると，S9・S22・S23 の書き方が変わる．
- 超えるときの分け方（orchestrator が決める）：helper で重なりをまとめる．それでも超えるなら次の 2 つに分ける．（a）記録と版と owner の判断（S1–S8，S11a–S11c，S16，S20，S21）．（b）busy と lock の順（S9，S13，S13b，S22–S28）．Stop・loss・arm・Shutdown の系列は次の段階に送らない（この段階が変える lifecycle の安全の条件，S17・S20・S26・S27）．

## 9. 採らないもの

- `mech:device-state-copy`（計画 §8 の 5）：採らない．既存の `planned-state-read-as-actual` で数える（`plan/maintenance/failure-mechanisms.md` の 1b-4c の段落）．
- `FakeFault` を増やすこと：採らない．失敗の注入は test 側の `ScriptedDevice` で行う．

## 10. `ScriptedDevice`（test 側の wrapper）

- `FakeDevice` を包む．`rx_open`・`tx_open` の n 回目を失敗させる．apply の n 回目を失敗させる．`rates()` の返す値を上書きする．`Barrier` で指定の呼び出しを止める．
- 呼び出し記録は `FakeDevice::calls()` に任せる．wrapper は失敗と停止だけを足す．
- 形は `test_support.rs` の `LockSpy` に倣う．

## 11. mutation の行（P361–P367，P317，P318，P358，P351 ほか）

行は code と一緒に書く（ここでは方針と対応だけ）．実装のとき各行の `old` が一致し，filter が 1 つの test を指すことを確かめる．行の `old` は code の変更で一致しなくなるので，この計画では「残す」と断定しない．

| 行 | 規則（現在の code） | 新しい場所 | 対応する新しい行・テスト | 実装時の確認 |
|---|---|---|---|---|
| P317（rx の `channels != self.opened`，RM-25 M1） | 集合の一致で開き直す（同じ件数の差し替え） | `begin` の判断：`Open(集合)` と集合の比較 | 新しい行（`set != record` の比較）．テスト `ur_25_a_same_size_membership_swap_reopens_the_streamer`（残す）と S1 | `opened` が消えるので old 文字列は一致しない．当て直す |
| P318（tx の `to.configured \|\| !self.verified \|\| to.set() != self.channels`） | 同上（TX） | `do_switch` の判断（§1） | 新しい行．テスト `ur_25_a_same_size_membership_swap_reopens_the_streamer`（残す）と S2 | `verified` が消える．当て直す |
| P358（`Ok(()) if to.refused => self.at += 1,`） | 拒否された TX の clock を始めない | 同じ位置（拒否の判定は `do_switch` に残す） | 残す．テスト `ur_25_a_transmit_refusal_from_no_stream_starts_no_clock`，S10 | 拒否の判定を移すなら当て直す |
| P351（from-none の `if let Err(error) = opened.and_then(...)`） | 配置できない configure を拒否として記帳する | from-none の block（`control.rs` ~790–846） | 残す．テスト `ur_25_a_refused_enable_from_no_stream_halts_and_keeps_its_values`，S6・S25 | from-none が busy を取るので old 文字列は変わる．当て直す |
| P361（tx の `verified` を無視） | 拒否された switch の未開の集合を開いていると見なす | 失敗の記録（`Unknown`）．新しい行：open の失敗で `Open(集合)` を書く | テスト `ur_25_a_refused_enable_leaves_its_set_unopened_for_the_next_switch`，S4，S5 | `verified` が消えるので行は消す．新しい行を書く |
| P362（switch が `verified` を先に落とさない） | 結果の前に未確認にする | 同じ：open の前に `set` を `Unknown` にする | テスト `ur_13_a_refused_switch_to_another_set_leaves_the_old_set_unverified`，S4，S7 | 行は消す．新しい行を書く |
| P363（成功の switch で `verified = true`） | 成功で確認済みにする | 成功の記録（`Open(集合)` と configure の欄） | テスト `ur_13_a_switch_to_the_verified_set_opens_nothing`，S3 | 行は消す．新しい行を書く |
| P364（`stale = dir == Dir::Rx` の `false` 化） | from-none の RX で stale を立てない | 記録が正しければ不要．S1 が落とす | テスト `ur_13_an_enable_from_no_stream_marks_the_receive_streamer_stale_whatever_it_gives`（書き換え），S1 | `stale` が消えるので行は消す |
| P365（`if stale { ... stale = true }` の除去） | stale を記録しない | 同上 | 同上 | 行は消す |
| P366（`std::mem::take(&mut ...stale)` の `&& false`） | 開き直しを止める | `begin` の判断（§1）．新しい行：`Open(集合)` と一致する記録で何もしない | テスト `ur_13_a_stale_receive_streamer_is_opened_again`（置き換え），S1，S3 | 行は消す．新しい行を書く |
| P367（`stale = opened.is_err() && dir == Dir::Rx`） | 元の UR-13 の条件（open の失敗だけ） | 同上 | 同上，S1 | 行は消す |

- P343，P344，P345，P350，P354，P359，P248–P256，J15，J16，J25，J49–J51，J62 は，この計画の時点では `old` と filter を確かめていない．実装で各行が一致し，一致しなくなった行は当て直すか，消して新しい行を書く．**P350 の既存テストは残す**（S10 は P350 を検査しない）．
- 新しい行（`set != record` の比較，`to.set() != record`，open 前の `Unknown`，成功の `Open(集合)`，lock の順）は，実装と一緒に書く．
- 規則：修正の round は触った行と新しい行だけを流す．全リストは段階の最後に 1 回．

## 12. 状態と残りの手順

- この計画の次は，orchestrator が D1 と D2 を決める．その後，実装者が test plan を基に実装する（§2 の 4）．
- 見積もりとの違いは，実装の中で `ur_13_*`・`ur_25_*` の既存テストを読んで分類する段階で出る．
- spec の改訂が要る：`design/18-uhd-radio.md` の UR-13（201），UR-14（203），UR-17（212），UR-23（238，「unless uhd-control did both for an enable from no stream」），UR-24（247–250），UR-25（252–253：「one lock that no thread holds across a device call」，「each streamer has one owning thread」），UR-26（257），UR-29（267），UR-30（274–275）．実装と一緒に改める．D1 で B を採ると RM-25 も改める．
