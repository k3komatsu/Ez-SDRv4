STATUS: done

# 1b-4d-3：実装の報告

対象: HEAD `d58da80` からの未コミットの変更（7 ファイル）．commit・push・`git add` はしていない．

## 1. 直したこと

### 1.1 plan の行（決定 1）

`crates/ezsdr-radio-uhd/src/provider/core.rs:425-440` の `Streams::record` の各行を

`{ stream, sequence, origin, end, channels, ticks_per_sample }`

にした（`:431` `stream: k`，`:432` `sequence: segment.by`，`:435` `channels: segment.config.channels.iter().collect::<Vec<u32>>()`）．`channels` は `Channels::iter()` の論理 channel の列で，前の数（`count()`）は消した．方向の分け方（`rx`・`tx` の配列）と `rx`・`tx` をまたぐ並びは今のまま．doc comment（`:423-424`）も row の field に合わせた．Mock の MR-27 の `segments` と同じ名前（`stream`，`sequence`，`channels`）にしている．

### 1.2 `device_async` の行（決定 2）

`crates/ezsdr-radio-uhd/src/provider/tx.rs:791` の `Tx::report` の行に `"stream": self.k` を足した．`channel` は streamer の中の位置なので，2 本の送信 stream では `stream` が要る．

### 1.3 拒否の隔離のテスト（決定 3）

- `crates/ezsdr-radio-uhd/src/provider/rx.rs:1490` `ur_25_a_refusal_leaves_the_transmit_stream_alone`: tx line に cold 変更を 1 つ予約し（`streams.book` で item 1，segment 2，clock 2 になる），その snapshot（items・plan・clocks）を RX の拒否の前後で比べる形に作り直した．空の tx line との比較はやめた．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:1525` `ur_25_a_refusal_of_a_layout_stream_leaves_the_other_stream_alone` を新設．layout の 2 本の受信 stream で，stream 0 の拒否が，cold 変更を 1 つ持つ stream 1 の line（items・plan・clocks）を変えないことを見る．stream 1 の集合は layout に合わせて `{1}`（`Channels::first(0).with(1)`）にした．
- `Held`（`rx.rs:1435-1438`）の plan の比較を `(origin, cut)` から `(origin, cut, channels, by)` に広げた（レビューの P3-3）．

### 1.4 複数 stream の `Provider::stop`（決定 4）

`crates/ezsdr-radio-uhd/tests/fake.rs:7347` `ur_30_the_plan_names_every_stream_at_the_stop` を新設．layout の 2×2（`rx_two()` + `tx_two()`，`layout_selector()`，rx0/rx1 の 2 link）を `Direct` で動かし，`finish()`（`Provider::stop(Orderly)` → `cleanup`）の後の `timing` の `plan` を読む:

- 各 row の `stream` がその stream の番号であること．
- 各 stream の最初の segment の `sequence` が `null`，変更で始まった segment は数（`is_u64()`）であること（RX と TX の両方）．
- 各 stream の `channels` がその stream の論理 channel の列（`[k]`）であること．
- RX の各 stream の最後の segment の `end` が stop の instant の sample（`ticks_per_sample` の `num`/`den` から `Segment::instant(sample_at_or_after(stop))` を計算）であること．
- TX の最後の segment の `end` は `null`（stop は burst を終え，clock を終えない＝今の規則）で，TX の最後の clock が `ended_at` を持たないこと．TX 側にも rate 変更を 1 つ入れ，TX row の `sequence` が command を名指すことを見る．

VH-8 の harness（`crates/ezsdr-acceptance/tests/parity.rs`）は触っていない．

### 1.5 テストの helper（決定 5）

`crates/ezsdr-radio-uhd/tests/fake.rs`:

- `:820` `section_of(instance, suffix)`: `instance.sections[&Namespace::parse("ezsdr.radio.uhd.<suffix>")]` の式を 1 つにした（46 か所）．
- `:825` `plan_row(timing)`: `timing` の `plan` 行を引く（既存の直接探索も含めて置換）．
- `:830` `plan_stream(plan, dir, k)`: plan の行を `stream` で選ぶ（新設の e2e で使用）．
- `:1201` `rejection_reason(events)` / `:1206` `rejection_reasons(events)`: `payload["reason"]` の取り出しを 1 つにした（12 か所）．

既存テストの意味は変えていない（`section_of` は clone を返すだけで，読みは同じ）．

### 1.6 sequence の値を固定（レビューの P3-1）

`crates/ezsdr-radio-uhd/src/provider/control.rs:1811` の `ur_30_the_recorded_plan_has_no_refused_segment` に，記録された plan row の `sequence` が `[None, Some(2)]` であることを足した（2 番目の cold 変更の arrival order が 2）．これで「segment を始めた command」の値が実経路で固定される．

## 2. design/18

- UR-30（`design/18-uhd-radio.md:304`）の `timing` の `plan` を「each segment's stream, the arrival order of the command that began it (`null` for a segment no command began), its origin, end, logical channels in block order and root ticks per sample」に直した．
- UR-28（`:292`）の `async` を「raw code, stream, channel and tick」に直し，Test 行（`:294`）に `ur_28_a_later_stream_s_reopen_discards_only_its_correlations`（row の `stream` が report の stream）を足した．
- UR-30 の Test 行（`:306`）に `ur_30_the_plan_names_every_stream_at_the_stop` を足した．
- UR-25 の Test 行（`:271`）に，既にあった `ur_25_a_refusal_leaves_the_transmit_stream_alone` と新設の `ur_25_a_refusal_of_a_layout_stream_leaves_the_other_stream_alone` を足した（1b-4b の記録からの持ち越し）．
- §6（`:532`）に新しいテストの行を 1 行足した．
- Changes（`:635`）に 1 行足した（DA-1）．引用は「plan 33's (8), design 33 D3's6 and RM-27's `segments`」．

## 3. テストと mutation の行

### 3.1 足した・直した・消したテスト

- 足した: `ur_25_a_refusal_of_a_layout_stream_leaves_the_other_stream_alone`（unit，`rx.rs:1525`），`ur_30_the_plan_names_every_stream_at_the_stop`（e2e，`fake.rs:7347`）．
- 直した: `ur_25_a_refusal_leaves_the_transmit_stream_alone`（`rx.rs:1490`，tx line に item と clock），`ur_25_a_layout_rate_change_cuts_its_stream_and_configures_every_channel`（`fake.rs:5284` 付近，`channels` を `[0,1]`，`stream`/`sequence` を追加），`ur_25_a_receive_membership_leave_opens_the_streamer_for_the_remaining_set`（`fake.rs:5547` 付近，`channels` を `[1]`），`ur_28_a_later_stream_s_reopen_discards_only_its_correlations`（`tx.rs:2241-2247`，`async` row の `stream` を stream 1 と stream 0 の両方で），`ur_30_the_recorded_plan_has_no_refused_segment`（`control.rs:1811`，`sequence` の値）．
- 消したテストはない．名前を変えたテストもない（`renames.txt` は不要）．

### 3.2 mutation の行（`plan/maintenance/tools/mutations.json`）

- `P480`（`:8324`）plan の `stream` → `0`
- `P481`（`:8334`）plan の `channels` の列 → `count()`
- `P482`（`:8344`）plan の `sequence` → `null`
- `P483`（`:8354`）`device_async` の `stream` → `0`
- `P484`（`:8364`）RX の拒否を次の受信 stream に記帳（layout の隔離テスト用）

既存の行は書き換えていない．P248・P250 の filter のテストを作り直したので，その 2 行も流した．`old` はすべて対象ファイル内で 1 回だけ一致することを確認した．

## 4. 確かめたこと

1. `cargo test --workspace --no-fail-fast`: exit 0，66 test binary すべて `test result: ok`（最後の実行は全変更後）．
2. `cargo clippy --workspace --all-targets -- -D warnings`: 通った．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: `0 problem(s) against HEAD`．
4. mutation（`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 ...`）:

| 行 | 結果 |
|---|---|
| P480 plan の `stream` | killed |
| P481 plan の `channels` の列 | killed |
| P482 plan の `sequence` | killed |
| P483 `device_async` の `stream` | killed |
| P484 RX の拒否を次の stream に | killed |
| P248 拒否を tx line に（作り直したテスト） | killed |
| P249 拒否を rx line に | killed |
| P250 拒否が全 stream を end | killed |
| P379 report の generation | killed |
| T06 / T32 / T33（`report` の tick の扱い） | killed |

12/12 killed，COMPILE ERROR なし．最初の実行はテスト名の変更前だったので，変更後にもう一度流し直した（最終結果は上のとおり）．

## 5. セルフレビュー

reviewer は DeepSeek V4.1 Flash（`opencode-go/deepseek-v4.1-flash#max`，`--agent plan`，回ごとに新しい起動），チェックリストは `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d3/checklist.md`．ログは同ディレクトリの `review-1.log`〜`review-3.log`．

| 回 | P1 | P2 | P3 | 扱い |
|---|---|---|---|---|
| 1 | なし | 2（helper の適用が触ったテストだけ，design の `null` の説明が 0 channel 開始の stream で偽） | 3（layout テストの mutation 行が無い／fixture が stream 1 の集合でない，async の stream 0 側が未検査，e2e のコメント） | すべて直した：section/reason の helper を全箇所へ，`null` の説明を「no command began」に，P484 と fixture `{1}`，stream 0 の `async` row，コメント |
| 2 | なし | なし | 7（TX の `sequence` 未観測，`sequence` の値が未固定，`Held` の plan が origin/cut だけ，doc の `ticks_per_sample` 落ち，e2e の end 計算が分母 1 前提，tx unit の追加 read が範囲外，Changes の citation） | すべて直した：TX の rate 変更と `sequence` の assert，control の unit test で `Some(2)`，`Held` に `channels`/`by`，doc，`num`/`den` で計算，追加 read にコメント，citation に RM-27 |
| 3 | なし | なし | 4（`sequence` の値，citation の出所，UR-25 の Test 行，helper の引用） | すべて直した：`Some(2)`，citation，UR-25 の Test 行に 2 テスト，helper の引用を RM-22 に |

P1 は 3 回とも無く，3 回で打ち切った（上限内）．すべての指摘を直し，理由をつけて退けたものはない（P3-6 の追加 read はレビュアーの「コメントを書くか削る」の前者を選んだ）．

## 6. 行数

- 本体: `core.rs` +5/−2，`tx.rs` +6/−1（合計 +11/−3）．
- テスト: `rx.rs` +55/−6，`control.rs` +3/−0，`tests/fake.rs` +176/−94（合計 +234/−100）．`fake.rs` の増減の大半は helper への置換（46 + 12 か所）．
- `design/18-uhd-radio.md` +7/−5，`plan/maintenance/tools/mutations.json` +50/−0．

## 7. 穴

- **TX の `end`**: `Provider::stop` は TX の burst を終えるが clock を終えないので，plan の TX の最後の segment の `end` は `null` のまま（今の規則）．決定 4 の「TX は今の規則のとおり」をこの意味に読み，e2e で `null` を固定した．stop の instant を TX の `end` にしたい場合は `Streams::record` が line の `ended` 状態も見る必要があり，今回の決定 1（field の追加）の範囲外なので触っていない．
- **e2e の範囲**: 決定 4 の「e2e」は `Direct`（Module の Provider の実 thread と `Provider::stop`）で見た．coordinator の `RunHandle`/Session を通す形ではない（layout で 2 本の受信 link を持つ profile を組む必要があり，VH-8 の harness は 1b-4e なので触らない指示だった）．Manifest に載る section の内容は同じ `instance()` の section で，`timing` の `plan` は `Provider::stop` 後の `cleanup` で書かれる．
- **`sequence` の一般値**: 実経路で `None`/`Some(2)` を固定したが，任意の command の arrival order はテストからは見えない（`Streams::seq` は非公開）．e2e は `is_u64()` まで．
- **`rx_segment` の timing 行**: まだ `"channels": channels.count()`（数）で stream 番号を持たない（`rx.rs:326`）．決定 1 は plan の行だけなので触っていない．レビュアーも範囲外と確認している．
- **mutation の範囲**: 新しい行・変えた行・触ったコードに当たる既存の行（12 行）だけを流した．全リストは orchestrator の gate．
- **実機**: 走らせていない（FakeDevice のみ）．
