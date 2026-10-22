# 設計メモ 35：UHD Module の stream ごとの印を，遷移の関数だけが変える状態にする

2026-10-22，orchestrator（Opus）．Phase 8 の 1b-5 の次の段階（handoff §2）．AGENTS.md §6 の three strikes による設計メモ．Sol 6.1 のレビュー（修正して採用，major 4・minor 2）を入れた版（§7）．

## 1. なぜ，何を

`mech:stream-lifecycle-scattered`（[failure-mechanisms](../maintenance/failure-mechanisms.md)）は，Phase 8 の UHD Module で 1b-4d-2 に 2 件，1b-4e-2 に 3 件出た．spec 22（VH-1〜VH-5）は MockRadio と timeline の側でこの機構に答えたが，UHD Module の stream ごとの状態には，1b-4d-1a で device の記録（`set`・`generation`・`configured`）を 1 つにした後も，次の印が残っていて，各 thread が直接読み書きしている．

| 印（`core::StreamState`） | 書く thread | 読む thread | 意味 |
|---|---|---|---|
| `busy`，`busy_seq` | owner の `owner_configure`，`release_unneeded`，uhd-control の from-none の enable．解くのは成功の節，呼び出し側（拒否を book した後），中断の節 | owner（`begin`，`pass`，`read`），uhd-control（from-none の判断），`move_source_busy` | 今 stream の device を呼んでいるのは誰か |
| `reading` | uhd-tx（`pass`，`read_reports_before_close`，`shutdown`） | uhd-control（`await_reading`） | uhd-tx の report の読みが進行中 |
| `idle` | owner（`Rx::begin`，`Tx::plan_section`），uhd-control の `start` | uhd-control（from-none の判断），`release_unneeded` | owner に始める segment が無い |
| `active` | owner（`begin`・`finish`，`plan_section`・`do_switch`），`Rx::new` | `release_unneeded`，`finish`・`do_switch`（`complete` へ渡す） | owner が処理中の segment |
| `delivered` | uhd-rx（`begin`，ブロックごと，`finish`），`Rx::new` | `Streams::book` の `End`（RM-16 の floor） | uhd-rx の segment でまだ渡していない最初の sample |

印の取り方・返し方の規則（誰がいつ立て，どの道で下ろすか）は spec 18 UR-25 に文で書かれ，コードでは production の 9 か所の `busy = false` と，`reading` を立てて下ろす 4 か所に散っている．1b-4d-2・1b-4e-2 の 5 件のうち，この形が直接の原因だったのは，返し方の道が増えるたびに直した所（拒否の後の返し方，loss の時の返し方）である．

**この段階で直すのは，これらの印の局所的な整合性と返し方の手順を，`Streams` の遷移の関数に閉じ込めることだけ**である．印の数を減らすことは目的にしない（必要な重なりを表せる形にする）．次のものは別の事実・寿命を表すので，この段階では変えない．

- device の記録（`set`・`generation`・`configured`，`record_*` の関数だけが書く），`completed`，`clocks`，instance の `stop`・`ended`．
- loss（`lost`，`device_gate`，device を呼ぶ道ごとの検査）：UR-29 の「別の thread が loss を publish した後に device を呼ばない」規則の形で，1b-4e-2 で open・close・rate の読みを `device_gate` の 1 つの直列化にした．5 件のうち 3 件（start の途中の loss の publish 漏れ，loss と open・close の競合，loss を報告する時の lock）はこちらの形で直してあり，この段階では防げない．
- **#67**（遠い origin の timed start が queue されたまま `Provider::stop` が来ると uhd-rx が join の 1 s で detach される）は未解決のまま残る．#56 の bench を待つ．

## 2. 推奨

### R1：device を呼ぶ者と，進行中の読みを，遷移の関数だけが変える

`busy`・`busy_seq`・`reading` を 1 つの struct の private な field にし，`Streams` の関数だけが変える．device を呼ぶ者は 1 人（`holder`），uhd-tx の report の読みはそれと別に重なりうる（`reading`）．

```rust
pub(crate) struct Claims { holder: Holder, reading: bool }   // field は private
pub(crate) enum Holder { Free, Owner { seq: Option<u64> }, Release, FromNone }
```

`holder` と `reading` を 1 つの enum にしない理由（Sol 1・2）：from-none は読みの最中に `holder` を取って読みの終わりを待ち，その待ちが `Provider::stop` か loss で終われば，読みは続いたまま `holder` だけを返す．`shutdown` の読みは `holder` を見ずに `reading` を立てる（detach された uhd-control が `FromNone` を返さないかもしれない，UR-25）ので，`FromNone` と読みが重なる．2 つの事実は寿命が違う．

| 関数 | 前 | 後 | 前が違うとき |
|---|---|---|---|
| `claim_owner(stream, seq) -> Option<Claim>` | `holder = Free` | `Owner{seq}` | `None`（owner は cursor も段階も動かさない） |
| `claim_release(stream) -> Option<Claim>` | `holder = Free`，段階が `Idle`，`active` なし | `Release` | `None` |
| `claim_from_none(stream) -> Option<Claim>` | `holder = Free`（`reading` はどちらでもよい） | `FromNone` | `None`（from-none を選ばない） |
| `may_call_from_none(stream) -> bool` | `holder = FromNone` | — | `reading` が立っていれば false（待つ） |
| `begin_read(stream) -> bool` | `holder = Free`，`reading = false` | `reading = true` | false（読まない） |
| `begin_shutdown_read(stream)` | どれでも | `reading = true` | — （UR-25 の例外：holder を見ない） |
| `end_read(stream)` | `reading = true` | `reading = false` | test build で panic |
| `give_back(claim)` | `claim` が取った `holder` | `holder = Free`（`reading` はそのまま） | test build で panic |

`Claim` は取った役割（stream と `Holder` の値）を持つ値で，`#[must_use]`．`give_back` だけが消費する．

- **返す順序は呼び出し側が決める**（Sol 4）：`owner_configure` は失敗したとき `Err((error, claim))` を返し，呼び出し側（`Rx::begin`，`Tx::do_switch`）が同じ lock の節で拒否を book してから `give_back` する．move の source が両方の line に拒否を記録するまで `Owner{seq}` を持ち続ける規則（UR-25）はこれで型に乗る．
- **Drop は返さない**：`Claim` の Drop は test build でだけ「返されずに落ちた」ことを panic で知らせ（返し忘れの道をテストが見つける），production では何もしない．lock を取り直さないので deadlock は無い．detach された thread の `Claim` は Drop されないので，その stream は `FromNone` のまま残る．これは今と同じで，`shutdown` の読みが holder を見ない例外がそれを扱う．
- `move_source_busy` は `holder == Owner{seq}` を読む関数のまま．

### R2：owner の段階を遷移の関数だけが変える

`idle`・`active`・`delivered` を 1 つの private な field にし，`Streams` の関数だけが変える．

```rust
pub(crate) enum Phase {
    Idle,                                          // 始める segment が無い
    Pending,                                       // 始める segment はあるが，まだ始めていない
    Active { segment: Option<(i64, Config, Option<u64>)>, delivered: Option<i64> },   // delivered は受信だけ
}
```

| 遷移 | 受信（uhd-rx） | 送信（uhd-tx） |
|---|---|---|
| 初期化 | `start` が T0 の stream を持てば `Active{delivered: 0}`．その segment が plan に一致するときだけ `segment` を持つ（`start` の途中に別の stream が T0 前の loss を publish すると，T0 の segment は plan から消える．今の `active = None`，`delivered = Some` と同じで，完了の記録もしない）．stream が無ければ `Idle` | `start` が clock を持てば `Active`，無ければ `Idle` |
| 次を探す | `begin`：cursor 以降の plan に segment があれば `Pending`，無ければ `Idle`（holder が取れない回は動かさない） | `plan_section`：clock も switch も無ければ `Idle`，clock があれば今の segment で `Active`，clock が無く switch があれば `Pending` |
| 始めた | `Active{segment, delivered: 0}` | `do_switch` の成功で `Active` |
| 始められなかった（拒否，loss） | `Pending` のまま（次の turn の `begin` が plan を読み直す） | `do_switch` は旧 `Active` を完了して `Pending` にし，最後の plan の読み直しで `Idle`（次が無い）・`Pending`・`Active` を決める |
| ブロックを渡した | `Active` の `delivered` を進める | — |
| 終えた | `finish(stream)`：自分の `Active` を `complete` へ渡して `Pending`（次は `begin` が決める） | `do_switch`：同じ |

読む側：from-none の判断と `release_unneeded` は `Idle` を，`Streams::book` の `End` は `Active` の `delivered`（その segment を切る End だけ，今の照合のまま）を読む．**`needed_before` は今のまま**（全 planned segment と `completed` を照合し，`active` を読まない．Sol 3）．`completed` は残す．finish の順序（完了の記録，段階の解除，prune）は今のまま．

**P472 を決定的にする**（Sol 5）：enum にしても「全 stream の段階を消す」mutant は書ける．決定的にするのはテストの側で，thread を使わない `Streams` の unit test で「受信 stream 0 と 1 を `Active` にする → stream 1 を `finish` → stream 0 の `delivered` が残る → Abort の `End` が stream 0 をその floor で切る」までを確かめ，P472 をそのテストに当て直す．

## 3. 消える書き方と変わる文

- 各 thread が `busy`・`busy_seq`・`reading`・`idle`・`active`・`delivered` を直接読み書きする所（production で約 60 か所）は，§2 の関数を呼ぶ形になる．field は private になる．
- spec 18 の UR-25 の「Ownership」の段落と UR-17 の delivered の文は印の名前（`busy`，`reading`，idle mark）で書かれている．holder・読み・段階の遷移の言葉に書き直す．規則（誰がいつ device を呼べるか，何を待つか，例外）は変えない．
- テストのうち印を直接読み書きするもの（unit test と `tests/fake.rs` で約 75 か所）は関数を通すように直す．

## 4. テストで固める系列（risky な段階の系列の表の元）

R1：
1. owner が `Free` の stream を取り，configure に成功して返す．
2. owner の configure が拒否され，呼び出し側が拒否を book してから返す．move の source なら両方の line に拒否が載るまで destination は開かない．
3. owner が `FromNone` の stream を見て何もしない（cursor，段階，device に触らない）．
4. uhd-tx の読みの最中に from-none が `FromNone` を取り，読みが終わってから最初の device の呼び出しをする．読みが先に終わる順と，from-none が先に取る順の両方．
5. from-none が読みを待つ間に `Provider::stop` か loss が来て，何も book せずに `holder` を返す．読みは続き，`reading` は uhd-tx の `end_read` で下りる．
6. `FromNone` のまま uhd-control が device の呼び出しで止まっている間に `shutdown` の読みが始まり，終わる（UR-25 の例外）．`shutdown` の読みの最中に uhd-control が戻って `give_back` する順（`reading` は残る）と，`end_read` が先の順（`holder` は残る）の両方で，最後に両方が解ける．
7. `release_unneeded` が `Idle` で `Free` の stream だけを閉じ，`Active`・`Pending`・他の holder の stream は閉じない．
8. 返し忘れ：各道（成功，拒否，loss，stop，待ちの中断）で `Claim` が `give_back` されることを，test build の Drop の panic が見張る．

R2：
9. 段階の各遷移（§2 R2 の表）を，受信と送信それぞれで．送信の拒否の後に次の segment が無い・有る場合，空への switch，長さ 0 の segment と拒否済みの segment の skip．受信の初期化で T0 の segment が plan に無い場合（`start` の途中の loss）．
10. 有限の move の鎖，同時の swap，source の拒否が確定する前の destination の待ち：`needed_before` と `completed` の判定が変わらない（1b-4e-2 の既存テストを挙げ，足りなければ足す）．
11. P472 の系列（§2 R2）．

## 5. 採らなかった案

- **device を 1 つの thread（actor）に持たせ，他の thread は message で頼む**：奪い合いそのものが無くなるが，UR-14〜UR-17・UR-25 の thread の形を作り替えることになり，1b-4 の bench の結果も前提から外れる．範囲が大きすぎる．
- **印を残して，組の不正を debug assert で見る**：各 thread が印を直接書く形は残る．
- **`holder` と `reading` を 1 つの enum にする**（この版の前の案）：読みの最中の from-none の中断と `shutdown` の読みが表せない（§2 R1）．
- **`Claim` の Drop で lock を取り直して返す**：lock を持ったまま Drop すると deadlock し，拒否を book する前に返してしまう．

## 6. 大きさと分け方

R1 と R2 は別の段階にする（それぞれ risky，約 1,000〜1,500 行）．R1 を先にする（取り方・返し方の規則の方が bug を出してきた）．各段階は AGENTS.md §9 の risky の手順（系列の表 → テスト計画 → Sol の計画レビュー → 実装）で進める．依る挙動はすべてこの Module の中のもので，UHD や実機の挙動に新しく依らない（VERIFIED：§1 の表は `crates/ezsdr-radio-uhd/src/provider/` の HEAD `7a025b9` を読んだもの）．

## 7. レビュー

Sol 6.1（2026-10-22）：修正して採用．major 4（from-none の中断で読みが消える，`shutdown` の読みとの重なり，`needed_before` は `active` を読まない，拒否の book と返す順序），minor 2（段階の遷移と P472 の観測，範囲と loss・#67 の明記）．すべて採用し，§1・§2・§4・§5 に入れた．R1 → R2 の順は Sol も勧めた．再レビュー（同日）：修正して採用．R1 は表せると確かめた．R2 の表の major 2（送信の拒否の後は `Idle` もある，受信の初期化で `delivered` があり `segment` が無い状態）と §4 の minor 1（`shutdown` の読みの最中に uhd-control が戻る順）を採用して入れた．R2 の残りはその段階の計画レビューで確かめる．

## 8. 実装

- R1：段階 1b-6a（2026-10-22，[reviews/1b6a-opus.md](reviews/1b6a-opus.md)）．
- R2：段階 1b-6b（2026-10-22，[reviews/1b6b-opus.md](reviews/1b6b-opus.md)）．`delivered` は §2 R2 の `Option<i64>` でなく今の組 `(origin, config, k)` を残した．
