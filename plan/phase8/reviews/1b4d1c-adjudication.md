# 1b-4d-1c：裁定

## adjudication


Sol の major 1〜5 と minor 6〜10 をすべて採る．major 1〜3 は，指示書 §3 の形 F（release のその場で untimed に当てる）から生じた．だから形を改める．**この裁定が `plan-brief.md` §3 と計画 `test-plan.md` より優先する．**

## 1. 形 F'（F を改める）

記録だけになる timed 更新も，device には **`e` を時刻にした timed command** で出す．出す先は，その instant に command が当たる channel である．

- **当たる channel** を 1 つの関数にする（仮名 `targets(dir, chan, t)`）．
  - その instant に live な channel があれば，今のとおりそれ（`channels_at`／`channel_live`）．
  - 無ければ，origin が `t` 以後の，拒否されていない segment が持つ channel の和．layout ではそのうち command の channel だけ，今の key では各 segment の集合の和．
  - それも無ければ空．
- **release**：`chans = targets(dir, chan, effective)`．空でなければ，今の live な枝と同じ経路で出す．`Some(effective)` で apply し，`released.push`，`issued()`，`late_command`（effective > e のとき）．空なら，今の記録だけの枝（device を呼ばない）．
- **`applied` の行**：
  - live な channel に出したときは今のまま（`issued: true`，`at: effective`）．
  - live な channel が無く，pending の segment の channel に出したときは，今の記録だけの行のまま（`at: e`，`note: "no channel"`）．`issued` の欄は足してよいが，VH-8（`parity.rs:685-703`）は `note` の行を比べないので，`note` は残す．
  - こうして Mock との比較（`key`・`claimed`・`at`）は変わらない．
- **queue の会計**（`book_timed`）：新しい command の cost も held の数え直しも，同じ `targets(...)` の長さで数える．layout は今も 1 本ずつ数えている（`control.rs:591-597`）ので，変わるのは今の key で channel の無い instant の command だけ．数えるのは booking のときであり，その後の cold change で当たる channel が変わるのは今と同じ．
- **版**：消す（Sol 7 は条件付きで支持）．消してよい理由は「読み手がいない」ではない．
  - 投影を device に知らせずに変える経路が，この形で無くなる．記録だけの更新も device に届く．
  - 残る `cancel_held` は `Provider::stop` の中だけにある．
  - 記録の `configured == Some(by)` は，集合も一致するときにだけ configure を省く理由になる（`owner_step` は集合の一致を先に見ている）．
- **退けた形**（計画に 1 行ずつ）：
  - F（release のその場で untimed）：古い segment が流れている間に効く（Sol 1）．UHD は untimed の command も先に queue に入った command の後で実行する（`timed_cmd.dox:85-105`，VERIFIED，Sol 2）ので，投影と device が逆になる．
  - origin を時刻にした timed：後ろの command を origin まで止める．
  - `e` を origin まで切り上げる：記録の instant が変わる．

## 2. 系列の改め

1. **S1・S1-TX**：owner が先に begin し（実際の `segment.origin` を読む），その後に `e < O` の更新を記帳して release する．device の log に `apply … at=<e>` の timed が 1 回出て，owner は configure し直さない．fake の時計が `e` を越えた後の read-back がその値．
2. **Sol 1 の系列（新）**：古い segment がまだ流れている間に release する．channel が `c` で抜け，`e` がその少し後，その後に再び入る segment がある．古い segment の最後のサンプルまでは古い値で，`e` からは新しい値（fake の read-back を，`c` の前と `e` の後で読む）．RX・TX・layout の leave／rejoin．
3. **S12（Sol 2）**：release 済みの command の後ろに付く記録だけの更新．`effective > e` は timed で出て，`issued()` で投影が動く．queue を drain した後，device と投影の最後の値が一致する．観測は fake の timed queue の実行順と read-back（`device.rs:746-758`）．
4. **S5（Sol 3）**：今の late release と同じ扱いにする．新しい late の規則は作らない．release が遅れたときは，今の live な枝の挙動を固定する．
5. **S9（Sol 4）**：失敗は今の live な枝と同じ（`device_failed` で return）．投影に入った値が後の configure で当たるかどうかは，今の live な枝の挙動と同じものを書き，テストで固定する．部分適用（channel 0 は成功し，1 で拒否）と，channel の間の loss を足す．`ApplyFails` で足りなければ，何を足すかを計画に書く．
6. **S8（Sol 5）**：拒否された segment が plan に残る TX で書く．RX は「拒否で候補が消える」テストとして残す．
7. **S3・S3b**：形 F' では release が記録を読まないので，S3 は「configure の戻りと記録の公開の間に release が来ても，device に timed で届く」を probe で固定する．probe を離す前に `configured == None` を assert する（Sol 6）．S3b と，F の `updates` の追加の lock は消す（Sol も支持）．timed の枝の既存の lock は残す．
8. **S4（Sol 6）**：今の key で channel の無い instant の更新を数える．release のたびに `released` を assert する．順序は同じ pass で複数を release して見る．17 個目が `COMMAND_QUEUE_FULL` になること（会計が数える），または数えない場合の理由を書く．
9. **S13（H）**：halted の方向に，拒否されていない pending の enable があるとき，`e` の timed で出る．既存のテストと design/18 §6 の表の期待を変える．
10. **S14（Sol 6）**：booking の順（100 → 200 → 300 ms）を明記する．2 度目の from-none の configure の後で log を区切る．gain に違う値を入れて，結果の値で読む．
11. **stop と owner の境界（Sol 7）**：owner が `Planned` を取った後に `Provider::stop` が来て，owner がそのまま configure する窓は，この段階より前からある．版が防ぐものでもない．この段階では系列とその期待を計画に書き，テストを 1 本足すか，次の段階に送るかを提案する（INFERRED の点を明記）．

## 3. design/18（Sol 9）

- UR-24 の第 1 段落（`design/18-uhd-radio.md:250`「後の segment が当てる」）と，Holding and release（`:251`）も直す．
- UR-24 の文に「拒否されていない segment」を入れる．
- UR-36 は UR-24 の届け方を引くだけにする．「集合も一致するとき」の条件を入れる．絶対的な「before the origin」は書かない．
- MR-33 との parity の説明（計画 `P:230`）は，Sol 1 の系列で確かめた形に書き直す．
- 各規則の末尾の `*Test:*` には，テストが実際に見る条件を短く書く．

## 4. 大きさ（Sol 10）

fake の queue の観測，時間の進め方，部分失敗，古い segment の系列を入れて見積もり直す．1,500 行を超えるなら分け方を提案する．

## adjudication-2


Sol の major 1〜6 と minor 8 をすべて採る．形 F' は保つ（Sol も支持）．**この裁定が `adjudication.md` と計画 v2 より優先する．**

## 1. 規則の改め（orchestrator の決定）

1. **release は device の queue を depth より多く埋めない（Sol 2）**．
   - release の pass で，`released.len()`（channel ごと，まだ effective を過ぎていないもの）と，これから出す channel の数の和が `command_queue_depth` を超えるなら，その command を held に戻し，この pass を終える．
   - 次の pass で slot が空いていれば出す．そのとき `e` を過ぎていれば，今の遅れた release と同じく `Some(effective)` で出し，device は着いたときに適用する．新しい late の規則は作らない．
   - これで，booking の後の cold change で当たる channel が増えた場合（F' の pending の channel も，HEAD からある live な channel の増加も）に，device の back-pressure で uhd-control が止まることは無くなる．
   - booking のときの会計（`targets` の長さ）は今のまま残す．利用者への `COMMAND_QUEUE_FULL` は booking のときだけ出る．
   - 系列：
     - Sol 2 の反例：pending の segment が無いまま 17 個を記帳し，その後に enable を予約して，同じ window で release する．16 個が出て，17 個目は held に残る．slot が空いた後に出る．
     - live な channel が 1 から 2 に増える同じ形．
2. **release の失敗は，その command を投影から除く（Sol 4）**．
   - `apply` が lost でない `Err` を返したら，今のとおり `device_failed` で return する．その前に，その `(e, seq)` を全投影の `updates` から除く（`cancel_held` の `updates.remove` と同じ）．`config` には入れず，行も書かない（今のとおり）．
   - これで，device にも後の configure にもその値は入らない．記録（`configured`）が残っていても，device と投影は一致する．版は要らない．
   - 部分適用（channel 0 は成功し，1 で拒否）では，channel 0 に値が残る．これは device が command の一部を拒んだ結果で，`COMMAND_REJECTED` が報告する．期待をそう書き，テストで固定する（channel 0 の read-back は新しい値，投影は古い値）．
   - 系列：from-none で configure 済み（記録はその segment の seq）→ `e < O` の更新を記帳 → timed の apply が lost でない拒否 → owner が begin．owner は configure せず，device も投影も更新の前の値．
   - 同じ扱いを live な枝にも当てる（同じ経路なので 1 か所）．
3. **release の後に，宛先の segment が cut・拒否される（Sol 7）**．release 済みの command は取り戻さない（UR-24）．device は effective で適用し，`released` は effective を過ぎて retire し，行はそのまま．これを確かめるテストを 1 本書く．

## 2. テストの観測（Sol 1，3，5，7，8）

1. **時計（Sol 1）**：`rig()` の `Core` の `ManualTimeAuthority` と，`FakeDevice` の時計は別である．read-back や `effective` を見るテストでは，device の時計を Core の時計と同じに進める方法を計画に書く（既存のテストがどうしているかを grep し，無ければ test 専用の道具を足す）．さらに，テストの中で 2 つの時計が一致していることを assert する．
2. **S16〜S18（Sol 1）**：read-back は設定の状態を見るだけで，古い stream のサンプルは見ない．主張をそこまでに絞る．cut の直前・`e` の直前・`e` の直後の 3 点で読む．
3. **attempt の記録（Sol 3）**：`ApplyFails`（または LockSpy）の入口で，失敗の判定と inner の呼び出しより前に `dir/chan/at` を記録する．S9(a)(d) はその attempt の列で見る．S9(c) は，失敗する channel が最後にならない並び（channel 0 で拒否し，channel 1 を呼ばないこと）も足す．
4. **S12（Sol 5）**：device の command の `at=10000` を直接 assert する．pending の行は `at=9700`，`note="no channel"`，`issued=true` の 3 つの欄を assert する．
5. **fake の FIFO（Sol 7）**：fake の untimed の apply は timed の queue を迂回する．UHD は受け取った順に処理する（VERIFIED）．この段階では fake を直さない．S2・S12 で証明できる範囲を計画に限定して書く．差は INFERRED として記録する（#62 と bench B8）．
6. **frequency の DSP（Sol 7）**：idle の DDC/DUC の timed frequency は，サンプルが来るまで消費されない（VERIFIED：Sol の引用）．この段階のテストは gain で書く．frequency は「front end には `e` に効く．DSP 側は最初のサンプルまで queue に残りうる」と INFERRED に書く．Sol の系列（idle 中の timed frequency → owner の untimed configure → 長い無通信 → 最初のサンプル）は記録して，実機（bench）に送る．
7. **S3（Sol 8）**：主張を「記録を公開する前の窓」に限る．lock の保証の主張は外す．
8. **S14（Sol 8）**：`mark` は 2 度目の from-none の configure の後，`begin()` の前に置く，と明記する．
9. **S15（Sol 8）**：延期する．説明は「plan の end だけが守る」と断定せず，join の順（control → TX → RX，`mod.rs:695-704`）を書いて，INFERRED とする．
10. **分割（Sol 8）**：S16〜S18 だけを後に送る fallback は採らない．超えるなら，分け方を orchestrator に提案する．

## 3. design/18（Sol 6）

- **layout の会計**：layout は今のとおり常に 1 本ずつ数える．UR-24 の「候補が無ければ 0」は今の key だけに書く．
- **instant**：第 1 段落の「at that instant」は，effective instant（自分の instant，または前の command の後ろ）にする．`:252` の「Each released command is recorded … with `e`」を，live の行と pending の行の区別に合わせて確かめる．
- **UR-24 に足す**：上の §1 の 1（depth を超えない release）と，2（失敗は投影から除く）．
- **Test 行**：UR-36 の `*Test:*` の「旧テストが外れる」は履歴なので，Changes へ移す．S16〜S18 の説明は「最後のサンプル」と書かず，read-back で見るものだけを書く．

## 4. 次

計画 v3（`test-plan-v3.md`）にまとめる．v2 からの差分を §0 に書く．大きさを数え直す．

## adjudication-3


**この裁定が `adjudication-2.md` と計画 v3 より優先する．**

## 1. 裁定 2 の §1.1・§1.2 を取り下げる

Sol の major 2・3 は，どちらも裁定 2 で足した規則そのものから生じた．持ち戻しは順序を失い，失敗の除去は並行する configure と食い違う．規則を足すたびに新しい窓が開くので，単純な形に戻す．

1. **queue の会計（裁定 2 §1.1 を取り下げ）**：
   - release は depth の上限で command を持ち戻さない．会計は booking のとき（`targets` の長さ）だけにし，今と同じ形にする．
   - booking の後の cold change で，command が当たる channel が増えることがある．live な channel の増加は HEAD からある．F' では pending の segment の channel も増えうる．このとき release が device の queue の深さを超えうる．その結果は，device の back-pressure で uhd-control の apply が待つことである（UHD 4.10 の FIFO が満ちると，後の timed command は待つ：`timed_cmd.dox:94-115`，VERIFIED）．待つ長さは，queue の先頭の command の時刻までである．release は `e` の release window（3 ms）前にしか出さないので，その長さはおおよそ window に収まる（INFERRED）．
   - layout は今も常に 1 本ずつ数え，`targets` は 1 本以下なので，会計はちょうどである．この穴は今の key のモードだけにある．そのモードは 1b-5 で消える．
   - UR-24 に「booking の会計」と「その後の cold change で増えた分は device が back-pressure する」を 1 文で書く．既知の制限として Changes に記録する．
   - 計画の S4b（持ち戻し）は消す．S4（booking の会計，17 個目が `COMMAND_QUEUE_FULL`）は残す．
2. **失敗（裁定 2 §1.2 を取り下げ）**：
   - pending の枝の `apply` の失敗は，live な枝と同じに扱う（HEAD の挙動）．`device_failed` で return し，`config` にも行にも書かず，投影の entry は残す．
   - したがって，後の configure がその値を当てうる．すでに configure した segment の owner は configure し直さず，device はその値を持たない．`COMMAND_REJECTED` がそれを報告する．
   - 版を消しても，これは悪くならない．HEAD の版はこの失敗で上がらない．始めた owner は版があっても configure し直さない．
   - 計画の S9(e) は，この挙動（投影の entry が残る，begin が configure しない，device は旧値）を固定するテストにする．名指しする誤った実装は，失敗の後に行や `config` を書く実装．
3. 裁定 2 の §1.3（release の後に segment が cut・拒否される）は残す．

## 2. 時計の道具（Sol major 4）：採る

- `Frozen` にする前に進んだ時刻を戻さない．だから，最初の command の前に，freeze した device の時刻（epoch）を決めて，初期時刻を assert する．
- read-back を読む点では，Core の時計も同じ時刻へ進め，両方の時刻を比べて assert する．
- device を 0 のまま Core を進める「呼び出しの検査」は，時計を合わせた検査とは別と書く．
- S3 で `effective == e` を言うなら，device の初期時刻を決める．
- 道具は `#[cfg(test)]` だけ．queue は消さず，既存の `settle` を使う．fake の既定値は変えない．host clock の sleep は進めないので，`apply_delay` や clock の待ちを含むテストには広げない．

## 3. minor

- minor 1（S4b）：S4b ごと消す（§1.1）．
- minor 2（depth の英文）：§1.1 の文に置き換わる．会計の対象を「booking の時点で数えた，まだ効いていない command」と明確に書く．
- 持ち戻しを取り下げるので，「持ち戻しと Provider::stop」の系列は要らない．

## 4. 次

計画 v3 をこの裁定で直しながら実装する（計画 v3 に追記する）．もう一度の計画レビューは行わない．Sol は実装の後にコードをレビューする．
