# E2a 設計レビュー（note 49 D′、`d9fddf3`、read-only）

**Verdict：direction holds with changes。** reopen は不要で、形（`TxBook` が `Bookings` と `TxList` を包み、結果を book に返す）より厳密に良い全体の代案は見つかりませんでした。直すべき点は major 2 件と、brief に入れる minor です。何も build・実行しておらず、列はすべてコードを読んだ推論（INFERRED）です。

## 1. Findings（重大度順）

### F1（major）取り消された差し替えの未読の payload が、新しい repeat として鳴る
E2a-3 は uhd-tx の読む時の取り込み（`install`）を消します。今はこれが「読む前に取り消された差し替え」を落とす唯一の関門です（`tx.rs:361-363`、`staging.rs:523`）。sketch の打ち切りは payload を `held` からしか除きません（§1.5 A「leave `held`」、S14）。

```text
R が鳴っている。uhd-tx は R の Send を発行し tx_send の中にいる
uhd-control：switch S を受付（pending(S, b)、playing = S、payload は list の unread）
tx_send が short か Err で戻る。complete が R を c で打ち切る（pending S は cancelled、playing = None）。held に S はない
次の turn：read() が Booking(S) を返し、held[b] = S になる（install はもうない）
装置の時刻が b に近づく：device burst は閉じているので S は新しい頭として on time（tx.rs:536-566、1098-1102）
結果：switches の行は cancelled なのに、新しい波形が b から Stop まで鳴る
```

- 記録に S の chain がないので、上に重なる予約が受け付けられ（RM-15）、後の `switch` は「repeat なし」で拒まれます。
- 今は "not taken" の `COMMAND_REJECTED` で落ちます。
- report の道（`read_list` と `reports` の間の受付）でも同じです。Stop は FIFO の順で守られ、loss と reopen の失敗は別の関門で守られます。穴は打ち切りだけです。
- **直し：** 差し替えの取り消しを book の一か所にまとめ、行の決着、`playing` の解除、payload の除去（`held` と `unread` の両方）、note を一度に行う。
- **テスト：** `book_switch` を drain せずに `tx.report(Underflow)` を呼ぶ形で作れます（`e1_q5` と同じ作り）。

### F2（major、決定の文のままなら）#86 の規則は untimed の差し替えを除く必要がある
- untimed の Action に要求の時刻はありません（RS-19、`design/04-run-and-session.md:181`）。
- Easy API の `switch(x)` は `aligned` の外では untimed です（`session.py:509`、EA-16 `design/16-easy-api.md:97`）。
- untimed の `at` は Kernel が決めた「今＋5 ms」で、境目の下限は「受付＋W＋lead = 12 ms」です。周期が約 7 ms より短い repeat では、ほぼすべての `switch(x)` が「押された」ことになります（profile の値からの推論）。
- 決定 3 の文のままだと、利用者が頼んでいない時刻が `requested_target` に入り、owner が見せたい timed のずれが埋もれます。
- **直し：** 条件に「timed（`!untimed`）」を入れ、RM-37 の最後の文に「untimed は同じ境目を取り、requested target は持たない」を足す。timed に限れば、この決定は sketch の「None」より良いと判断します。
- sketch の §5 と S16 は決定 3 より前の内容のままです。oracle の関数、両 Provider のテスト、Mock の変更（`lib.rs:1660,1769`）、MR-42 の文、mutation 行が移行表にありません。

### F3（minor、brief に必須）範囲を絞った打ち切りの、記録の結果が決まっていない
- 今の `halt(c, c+5)` は、記録で適用済みの差し替えの chain が c = b から始まる時、Stop の範囲 [b, b+5) を足します（`staging.rs:480,496-501`）。
- sketch の文（「R を c+5 で切る、適用済みの chain を除く」）どおりに作ると、R の front はすでに b なので [b, b+5) が空きます。そこへの timed の DropAndFlag は、受付の拒否から dispatch の drop に変わります。sketch 自身が挙げる「狭めすぎ」の具体例です。
- 今も pending の場合（R の chain が c+5 まで）と applied の場合（Stop の範囲）で、SendAsap の扱いが食い違っています（拒否と F36 の移動）。RM-15 `design/07-radio-model.md:163` の footprint にそろえ、名前のある変更にしてください。

### F4（minor）`broken(None)` が未定義で、今の小さな不具合を引き継ぐ
- underflow の報告は、device burst が閉じていても `open` があれば打ち切ります（`tx.rs:1300`）。
- `open` があって何か送った後なら device burst は必ず開いているので（`tx.rs:811-828`）、この条件が足すのは「dispatch 済みで未送信」の場合だけです。
- そこへ遅れた報告が来ると、未送信の予約が note なしに消え、SOB のない EOB だけの send が出ます（`tx.rs:1016,1041-1045,1054`）。
- sketch の `broken(chain.next())` は、この時 `None` を受けます。「何も変えない」と決め、名前のある変更にすることを勧めます。

### F5（minor）(b1) は保てるが、sketch の根拠の一つが誤りで、文に直しが要る
- §4 (b) の「どの案でも Mock と違う」は誤りです。`ur_21_s4_a_finite_abandon_keeps_the_switch_of_the_repeat_it_has_not_started`（`tx.rs:7115-7133`）の SendAsap の枝は、今 UHD も Mock も `applied` です。(b1) は UHD だけ受付の拒否にします。
- uhd-tx がすでに遅れている時だけの差なので (b1) は保てます。ただし「決定的になる差」の五つ目として owner に伝え、KL に書いてください。
- (b2) は configure の遅れの間の `switch` を生かせる唯一の案ですが、二段の取り込みが戻るので、厳密に良くはありません。
- RM-37 の「playing」を「装置に発行した」に書き換えないでください。コードは `start ≤ now_k` も残し、Mock はそれだけで判じます。装置の条件は UR-38 に置きます。
- (b1) で届かなくなる文とテストが一覧にありません：RM-37 の「An abandon of a finite burst leaves the switch … pending」（`design/07-radio-model.md:350`）、UR-38 の「an abandon of a finite burst does not」（`design/18-uhd-radio.md:302`）、上のテスト。

### F6（minor）遷移の表に「`open` を結果なしで終える」がない
- UHD：`end_open` の 5 か所（`tx.rs:583,1174,1177,1191,1204`）、`abandon`（`:1054`）、`halt_failed_reopen`（`:984`）。
- Mock：`emit_tx_until` の誤りの出口二つ（tracker の Err、MR-24 の拒否、`lib.rs:1871-1901`）。これを `Outcome::Failed` に当てると、Mock が今は持たない打ち切り（記録の変更）を持ってしまいます。

### F7（minor）lock の節：再入と emit を test build で落とす
- plan を一つの節にすると、今の helper（`with_held`、`values_turn`、`device_holds` など）が中で `lock` を取り直し、同じスレッドで止まります。テストは失敗ではなく hang します。
- `core::lock` は test build で guard を数えています（`core.rs:59-64`、`test_support.rs:236`）。足す検査は二つです：同じ mutex の再入で panic、`Core::emit_at` に `assert_line_free`。
- E1 で Sol が見つけた「lock の下の emit」は、今も検査がありません（`e1-dispositions.md:56`）。

### F8（minor）plan の節の順が今と違う
sketch は「expiry → `broken`」を「`late_start` → `untake`」の前に書いていますが、今は逆です（`tx.rs:493-506`）。そのまま作ると、未送信の頭の開始が過ぎた時、late policy（SendAsap は動く）ではなく打ち切りになります。

### F9（minor）移行表の漏れと誤り
| 項目 | 事実 |
|---|---|
| `tx_step_guarded` の probe | Q2b だけでなく `e1_q14` も使う（`e1.rs:334`）。guard と plan が一つの節になると、この点は消える |
| N47S5-57 | killer は Q14 だけ。E2a-3 で guard を広げると、joined の節は uhd-tx から届かなくなる見込み。節と行を消すか、新しい killer を brief で決める |
| P650（`core.rs` の `late_by`） | killer は消えるテスト `ur_23_stop_before_the_moved_boundary…` |
| N47S5-153 | killer は (b1) で届かなくなる finite のテスト |
| N47S5-90（`if repeat && !switch`） | killer は書き直すテスト。mutation の中身が E2a-3 の意図した動きになる |
| 「O1-M7 unchanged」 | その行はない。あるのは O1-M7a・M7b（Mock の `channel.rs`） |
| `rejected` の note の順 | 打ち切りの note を book が返すと順が変わりうる。今の順を brief に書く |

### 検証した claim
- 行番号つきの claim は、上の誤りと些細な一つ（`send` は 645-838 行）を除いて一致しました：書き戻し 15＋2 か所、`playing` の書き手、テスト名 22 個、行の family の数、spec の行、`session.py`。
- 「差し替えは動かない」の論証（`control.rs:481,553`、`tx.rs:472-488,664-668`）は、広げた guard を前提に成り立ちます。
- `calls()` は `tx_async` を記録しないので、報告の読みが増えても trace は変わりません。

## 2. owner の決定と約束の照合

| 決定 | 判定 |
|---|---|
| 1. 形と sub-step | 保てる。E2a-2 は分けたい（§3 の A2） |
| 2. N2 = (b1) | 保てる。EA-11・EA-16 と合う（EA-16 の行と docstring に一句）。F5 の差と文の直しを入れる |
| 3. #86 | timed に限れば保てる（F2）。RM-14・SC-29a と合う |
| 4. `moved_to`・`installed`・`refuse`・`Refused` の削除 | 保てる。ただし F1 の直しが前提（読む時の関門を消すため）。schema と Python に読み手はない |
| 5. (d1) | 保てる。さらに id も要らない（A3） |
| 5. (d2)(d3)(d4) | 保てる。(d3) は RM-21 `design/07-radio-model.md:238` のとおり |
| 5. (d5) | KL-13 でも足りるが、AGENTS §2 の「one entry each」には独立の項目の方が合う |

RM-13、RM-14、RM-16、MR-25、UR-25、UR-26、UR-28、KL-36〜KL-39 に、sketch の表と違う点は見つかりませんでした。

## 3. 代案（推奨順）

| # | 代案 | 代価 | 後でできなくなること |
|---|---|---|---|
| A1 | 差し替えの取り消しを一か所にし、`unread` からも除く（F1） | 約 10 行、純粋テスト 1、rig テスト 1 | なし |
| A2 | E2a-2 を二つに分ける。2a：`open` の移動と send の分割だけ（pin と `calls()` は不変）。2b：#80、#85、`broken(None)` | sub-step のレビューと local commit が一回増える | なし |
| A3 | token は `inflight: Option<Send>` の有無だけにし、`OpId`、`ops`、`issued`、`receive_switch` の `cursor`・`started` 引数をやめる | なし（S7 のテストと行が一つ減る） | 8c の Replay で複数の op が並ぶ時に id を足す |
| A4 | 絞った打ち切りを純粋な関数として決め、「後ろにほかの chain がなければ今の `halt(c, c+5)`＋`stop` と同じ」を表のテストで固定する（F3） | 純粋テスト 1 | なし（C6 で消す） |
| A5 | `broken(None)` は何も変えない（F4） | テスト 1、名前のある変更 1 | なし |

- A2 の理由：sketch は「E2a-2 の diff は分割だけ」と書きますが、実際は挙動の直し二つが混ざり、trace の差の原因を切り分けられません。
- A3 の理由：実行役は一スレッドで、受付は発行した範囲を変えられないので、id が守るものは `Option::take` と同じです。`issued` は debug の不変条件しか読みません。

## 4. brief に足す列

1. S の payload が未読のまま R を打ち切る（F1）。
2. 適用済みの差し替えの境目 c = b で打ち切った後の、[b, b+5) への timed の DropAndFlag と SendAsap（F3）。
3. 未送信の頭があり device burst が閉じている時の、遅れた underflow（F4）。
4. 未送信の頭の開始が過ぎた時は late policy で判じる（F8）。
5. in-flight の間の loss：`complete` は一度、shutdown が note を出す。
6. 拒まれた switch（N2、pending あり、repeat なし）は `pending`・`playing`・行を変えない。
7. #86：timed で押された、timed で規則どおり、untimed で押された。両 Provider。
8. Mock の MR-24 と tracker の誤りの出口は記録を変えない（F6）。
9. finite の後ろの未開始の repeat への switch は受付で拒否（今は `applied`、F5）。

確かめていないこと：build と実行は一切なし（列は INFERRED）。FakeDevice が EOB だけの send をどう扱うか、sketch の行数と 74 行の見積もり、Python の suite、O1 のテスト本体、`e1-test-plan.md` は読んでいません。
危険：F1 は RF に出る誤りなので、E2a-3 の test plan に失敗するテストを先に入れてください。