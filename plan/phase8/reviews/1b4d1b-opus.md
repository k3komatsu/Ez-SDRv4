# 1b-4d-1b：orchestrator のレビューと裁定

読んだもの：`git diff b190918` の `provider/{core,control,rx,tx}.rs` の本体，`design/18` の差分，`report.md`．テストは名前と helper と S9・S22 の本文だけ読んだ（テストの力は Sol の 7 に任せる）．

## 指示書との一致

- B1（busy は from-none の間の uhd-control のものだけ）：`control.rs` の判断の section で `busy = true`，booking の section と早い戻りで false．RX の owner は `begin` の section で見る（`rx.rs:281`），TX は `pass`・`read` で見る．指示書どおり．
- B2（`reading`）：`Tx::pass` が busy の検査・plan の読み・`reading = true` を 1 つの section で行う．from-none の判断の section が `reading` を読むので，その後の uhd-tx の turn は busy を見て止まる．窓は無い．指示書（「同じ section」）のとおり．
- B3：from-none の条件は `idle && planned` のまま．
- B5：`Locked` は `MutexGuard` を包むだけで，test 以外では drop の順も寿命も変わらない．`assert_line_free` は `updates` を取る 5 か所すべての直前にある（`grep -n "updates.lock()" control.rs` で確かめた：`book`・`changed`・`configure`・release・cancel の 5 か所）．
- loss・stop の出口：裁定 §1.1 の 6 つとも実装にある．早い戻りは `busy` を下ろし，記帳しない．`record_opened` は呼び出しが出たときだけ書く．
- `--features uhd` の clippy：実装役の手元では流せなかった（libuhd 無し）．image `ezsdr-v4-dev:uhd4.10` の中で流して通った（gate の前に 1 回）．

## 変えてはならない振る舞い

- from-none の成功の経路：open → 照合 → configure → 記帳の順と回数は前と同じ．`failure` の組み立て（open の誤り → 照合の誤り → configure の誤り）も前の `opened.and(checked).and_then(configure)` と同じ結果．
- TX の `do_switch` の最後の `read(Some(..))` は戻り値を捨てる．switch がある間 `idle` は false なので，そこで `busy` が立っていることは無い．prune が飛ぶ経路は無い（Sol にも確かめさせる）．
- TX の `step` は clock も switch も無いとき device を呼ばない．`pass` の `end_reading` と `step` の間に from-none が判断しても，owner は idle なので `step` は何もしない．
- RX は stream がある間 `begin` を通らず `rx_recv` するが，そのとき `idle` は false なので from-none は起きない．

## 指摘（すべて minor）

1. **UR-17・UR-23 が UR-25 の規則を言い直している**（裁定 §1.7「引くだけ」）．「An owner whose stream's `busy` mark is set begins nothing that turn …」は UR-25 の Ownership の言い直し．引くだけにする．
2. **UR-25 の最後の文が読みにくい**：「the test build checks the first direction alone, a `rec` taken before `streams` included in its count」．言いたいのは「test build が見るのは state の guard を持ったまま `updates` を取ることだけで，`rec` と `streams` の順は見ない」．
3. **3 回目のセルフレビューの P2 の修正（`pass` への統合）は，どの reviewer にも見られていない**．Sol のレビューの範囲に入る．
4. 早い戻り（loss，呼び出しの間の stop）では `record_opened` は書くが `record_configured` は書かない．loss も Provider の stop も終わりなので，古い `configured` を読む owner は無いと見る（INFERRED．Sol の 2 で確かめる）．

## Sol のコードレビュー（[1b4d1b-sol.md](1b4d1b-sol.md)）の裁定

判定は PASS，major 0・minor 2．私の 4 件と合わせて，修正の回は 1 回（orchestrator が自分で直した．どれも数行で，渡す手間のほうが大きい）．

| 指摘 | 扱い |
|---|---|
| Sol 1：S13b に mutation の行が無い | 採る．owner の open の経路（`Core::open_streamer`）で記録を device の呼び出しの前に書く行 P403 を足した．今の probe の観測でこの誤りは落ちる．Sol の言う「probe と device の呼び出しの間で記録する」実装は，`open_device` がその間に何もしないので作為的と見て，観測は変えない |
| Sol 2：S9 の TX が `idle` の不変を見ない | 採る．busy の間の `read`・`step` の前後で `idle` が変わらないことを assert し，busy の枝で `idle` を下ろす行 P404 を足した |
| Opus 1：UR-17・UR-23 が UR-25 を言い直す | 採る．両方を「Its turns while an enable from no stream holds the stream are UR-25's.」にした |
| Opus 2：UR-25 の最後の文 | 採る．「`updates` is never taken under one of those, which the test build checks; it does not check the order among those four.」にした |
| Opus 3：`pass` への統合を誰も見ていない | Sol が見た（確認結果の「TX の `end_reading → step`」）．閉じる |
| Opus 4：早い戻りで古い `configured` が残る | Sol が確かめた：open を出した経路では `record_opened` が `configured` を消し，出す前に終わった経路では device は変わっておらず，loss と Provider の stop は全 line を終える．閉じる |

Sol の再レビューは要らない（major の修正が無い，AGENTS.md §9 の 8）．

## 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol の計画の差分レビュー | 7 | 4 | major 1〜3 は 1c へ（記録だけの release と始まった segment，記録を公開する窓，queue の会計：`planned-state-read-as-actual` の候補．1c の実装で数える）．4・6・7 と minor はテストの観測，5 は from-none の loss・stop の出口の欠落（計画の段階で入れた） |
| セルフレビュー（DeepSeek，3 回） | — | P2 7・P3 13 | テストの欠落と design の文．機構に数えない |
| Opus | 0 | 4 | 言い直した規則と文．数えない |
| Sol のコードレビュー | 0 | 2 | テストの欠落．数えない |

計画を先に書く手順（§9 の 2・3）の評価：コードの後に major は 0．計画の段階で Sol が出した major 7 のうち 4 はテストの観測で，コードを書く前に直せた．
