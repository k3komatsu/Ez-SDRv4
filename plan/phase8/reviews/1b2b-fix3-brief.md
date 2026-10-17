# 修正の指示書 3：Phase 8 段階 1b-2b（Sol の2回目の再レビュー後）

あなたは実装役です．リポジトリ /home/komatsu/local_work/Ez-SDRv4．前の指示書 `brief.md`・`fix-brief.md`・`fix2-brief.md`（どれも `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2b/`）の「守ること」はすべて有効です．Sol の2回目の再レビュー `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2b/sol-rereview2.md` を**全文読んでから**始めること．採否：**採用**．

## 根

release を「移動の item 自身が作った cut」（`cut_by(seq)`）だけから取っている．移動元が channel を手放す時刻は，移動の item が切る segment に限らない：移動の前に `Stop` や `End` で切られたが，その cut の instant が移動の instant より後になる segment（低い rate で sample に丸めた cut，Sol の例：1 Hz の stream を 100 ms に Stop → cut は 1 s，200 ms の移動で移動先が 200 ms に始まる）も，その channel を持っている．移動の item が cut を作らなければ `None` になり，床が無くなる．

## 決定：release の定義を計画から一般に読む

移動 `m`（`seq`，移動元 `from`，`channel`）の release は，**移動元の今の計画で，channel `m.channel` を持つ segment のうち，移動より前に始まったもの（その segment を始めた item の `seq` が `m.seq` より小さい，または初めの segment）の終わりの instant の最大**とする．終わり＝ cut の instant（`segment.instant(cut)`，RM-16）．開始待ちで origin で切られた segment，移動が切る segment，移動の前に `Stop`/`End`/拒否で切られた segment を全部含む．そのような segment が無ければ release なし（`i64::MIN`）．cut の無い（終わっていない）その種の segment は，移動が切るので無いはず：あれば内部の誤りとして panic（理由を doc に）．

1. `Line` に，この値を計画から返す関数を置く（例：`Line::released(channel, before_seq) -> Option<i64>`）．`settle_once` は `cut_by(m.seq)` の代わりにこれを使う．`cut_by` と replay の `cuts` の記録が他で使われなければ消す（使うなら理由を報告に）．
2. prune：計画（`Line::plan`）は prune 済みの item の segment も持つので，`released` は prune の後も同じ値を返すはず．それを確かめ，`Lines::floors`（prune の床）が要らなくなるなら消す（要るなら理由を報告に）．P103 の `equivalent` の扱いもそれに合わせる．
3. RM-34 の release の文を上の定義に書き換える（規範だけ）．

## テスト

- Sol の筋書き：受信，lead 0，移動元 channel 0 を 1 Hz，移動先は空で 1 MHz；100 ms に移動元を `Stop`（cut 1 s），200 ms に channel 0 を移動 → 移動先の segment の origin は 1 s 以後．
- 同じで `Stop` の代わりに送信の `End`（送信の line で）．
- 上の Stop 版を prune（prune の instant を 200 ms と 1 s の間）した後に移動先へ `start_rx`／`cold` 変更 → 1 s 以後．
- 生成列の `check_move`：release が None の場合も検査から外さず，移動先の移動以後の全 segment の origin ≥ 移動元の「移動より前に始まり channel を持つ segment の終わり」＋ lead を主張する（`None` なら主張は自明）．
- mutation の行：`released` が移動の item の cut だけを見る（前の実装）；channel を持たない segment も数える（過剰に遅くなる：これを捕まえるテストも要る．例：移動元の別の channel だけの segment の終わりが遅い場合に移動先が待たないこと）；「移動より前に始まった」の条件を外す（後の segment まで数える）．`old` が変わる既存の行は当て直す．

## 確かめること・セルフレビュー・報告

前回と同じ（確かめること 1〜5，変わった行と新しい行の mutation；新しい読み取り専用レビュー役，最大3回．チェックリストに「Sol の2回目の指摘が決定どおりか．移動先が移動元より先に channel を持つ計画を作る列（Stop，End，拒否，prune，連鎖，送受）を自分で探す」）．`ur_26_the_shutdown_s_last_burst_ack_waits_on_the_device_s_clock` の負荷による失敗は既知なので，workspace で落ちたら単独で流した結果と一緒に報告すればよい．報告は `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2b/fix3-report.md`．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．
