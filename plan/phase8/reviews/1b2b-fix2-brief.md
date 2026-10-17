# 修正の指示書 2：Phase 8 段階 1b-2b（Sol の再レビュー後）

あなたは実装役です．リポジトリ /home/komatsu/local_work/Ez-SDRv4．前の指示書 `brief.md` と `fix-brief.md`（どちらも `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2b/`）の「守ること」はすべて有効です．Sol の再レビュー `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2b/sol-rereview.md`（新規 ①②③）を**全文読んでから**始めること．採否：**全部採用**．以下が決定です．

## 根（①と②は同じ）

`Item.ready` に2つの意味が混ざっている：VH-4 の「Provider が segment を始められる instant」と，送信の replay が `item.ready >= ended` で使う「記帳の順（End より後に記帳されたか）」．`settle` が移動元の cut を `ready` に書き込むと，後者が壊れる（②）．また release の床が移動の item にしか効かず，移動の後に移動先を始める別の item（後の `start_rx`，`cold` 変更）には効かない（①）．release を Provider の義務にしたのも同じ規則を2か所に書くことになる（設計 33 D3 の9）．

## 直すこと

1. **release を別の field に**：`Item` に `release: i64`（移動元が channel を離す instant．既定は `i64::MIN`）を足す．`ready` は今の意味（VH-4 と記帳の順）のまま，`settle` は**決して `ready` を書き換えない**．`State::begin` は segment の origin を `release + lead` 以後にもする（`earliest = earliest.max(release + lead)`．他の項は今のまま）．Provider は `release` を設定しない（常に既定値で記帳し，`Lines` だけが書く）．`Line::book` などで Provider が直接作る item は `release: i64::MIN`．
2. **移動先の全 item に床を（①）**：`settle` は，各 line について，その line を移動先とする各移動 `m` の移動元の今の `cut_by(m.seq)` を求め，その line の item のうち `(effective instant, seq) ≥ (m の instant, m.seq)` のもの（移動の item 自身を含む）の `release` を，それらの cut の最大にする（該当しない item は `i64::MIN`）．変化が無くなるまで繰り返す（上限は今のまま）．
3. **prune の後も床を残す**：`prune` で記録を捨てる移動について，その移動元の cut の instant を移動先の line の「prune 済みの release の床」として `Lines` に残し（line ごとに最大値），それ以後の移動先の item の `release` の下限にする（prune 後の item はすべて prune の instant 以後なので，全 item に効かせてよい）．
4. **Provider の義務の文を消す**：RM-34 から「移動先を後で始める item も cut を ready に含める（Provider の義務）」を消し，`Lines` が移動先の全 item に release の床を保つ，と書く（`release` は Provider が書かない）．
5. **RM-3（③）**：layout の description の受信 block の `channels` は stream が持つ集合の**要素数**（SC-10 の `u16`），block の位置 `i` はその集合の `i` 番目に小さい論理 channel．SampleClock の stream id の例に `<device>/rx/s<k>`・`<device>/tx/s<k>` を足す．
6. 前回の報告の「逆順の `seq` も while が救う」の説明は誤り（Sol）：P96 の equivalent の文面と RM-34／報告から，その主張を消し，「`seq` は instance の中で一意で到着順に増える（RM-34 の前提）」とだけ書く．

## テスト

- ①：両受信を停止 → 移動を記帳 → 移動先の `start_rx` を記帳 → 移動元に移動より前の `start_rx` を記帳（3 MS/s → 100 MS/s の条件）→ 移動先の segment の origin が新しい移動元の cut ＋ lead 以後．
- ②：両送信に将来の `End`（800 ms）→ 500 ms に 1000 ms の移動を記帳 → 移動先の送信 clock が記帳される（VH-2：End より前に記帳された clock は残る）．`ready` が書き換わっていないことも assert．
- prune の後の床：移動元の cut が prune の instant より後になる移動を prune した後，移動先に記帳した `start_rx`／`cold` 変更の segment がその cut ＋ lead 以後．
- 生成列のテストの `check_move` を広げ，移動先の line の移動以後の**全 segment**の origin ≥ 移動元の `cut_by(seq)` ＋ lead を主張する．
- mutation の行：`settle` が移動の item だけに床を置く；`release` でなく `ready` に書く（②が kill する）；`begin` が `release` を見ない；prune の床を残さない．`old` が変わる既存の行は当て直す．

## 確かめること・セルフレビュー・報告

前回と同じ（確かめること 1〜5 と，変わった行と新しい行の mutation；新しい読み取り専用レビュー役，最大3回．チェックリストに「①②③ が決定どおりか．`ready` を書き換える経路が残っていないか（grep）．移動先が移動元より先に channel を取る計画を作る列を自分で探す」）．報告は `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2b/fix2-report.md`．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．
