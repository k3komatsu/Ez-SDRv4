**Verdict: ADOPT WITH CHANGES.** 趣旨は現コードと合うが，E4 の模型にするには失敗対象と完了条件を明記したい．P1 なし．

- **P2 — 所有者不在の範囲と静止の扱いが未確定**（`plan/phase8/45-uhd-decision-core.md:172`，同 `:134,143`）．
  最小系列：RX0 の生成成功 → RX1 の `rx_start` 成功 → RX1 の生成失敗 → Abort．現コードが `abandon_unowned` するのは **RX1 だけ**（`crates/ezsdr-radio-uhd/src/provider/mod.rs:648–650`）．
  現文の「開始をすでに出した stream」は RX0 まで含めて読める．また，所有者のない RX1 に §4.5 の epoch 確認を要求すれば，永久に届かず cleanup が進まない模型になる．失敗対象だけを放棄し，未生成者の確認を待たない例外を明記する．

- **P3 — TX・loss・handle の分岐を明示する**（`plan/phase8/45-uhd-decision-core.md:172`）．
  TX の生成失敗は RX の放棄遷移を起こさない（`crates/ezsdr-radio-uhd/src/provider/mod.rs:660–678`）．生成失敗自体は loss ではない．`rx_start` の lost エラーは停止より先に公表される（同 `:635–640`）．
  既生成者の終了は現文どおりだが，順序は control → TX → RX．cleanup は loss または detach なら streamer を閉じず，それ以外は閉じる（同 `:703–717,731–738`）．§4.5・§4.7 と矛盾はない．

**差し替え案（§4.9 の段落）：**
> 実行役の生成に失敗したら，後続の生成を打ち切り，`Provider::start` は Abort の停止処理を経て失敗する（UR-15）．生成に失敗した RX 自身について，初期 `rx_start` が成功済みの場合だけ `abandon_unowned` 相当の遷移を行う（装置は不確か，既存の Lost は保つ，segment なし，Abort）．開始していない RX と，TX の生成失敗にはこの遷移を行わない．生成失敗自体は loss としない．`rx_start` が loss を返した場合は，停止処理より先に失敗の印と native の `mark_lost` を公表する（§4.5 手順4）．
> 未生成の実行役には command を出さず，その実行役からの静止確認や完了通知も待たない．呼ばれなかった命令の終端は判断役が確定し，残る義務と記録の確定は §4.4・§4.5 に従って区別する．既生成者は通常の停止順序と join 上限に従う．cleanup は loss または detach があれば streamer を残し，それ以外は閉じる．native の loss 後の handle 保持は §4.5 手順4に従う．

`plan/phase8/45-report.md:130` は，改訂された K／D の要約として整合している．

Not checked: builds, tests, hardware, holdout．ファイル・git 状態の変更なし．
