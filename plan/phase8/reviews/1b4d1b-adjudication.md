# 1b-4d-1b：Sol の計画の差分レビュー（`sol-plan.log`）の裁定

Sol の major 1〜7 と minor 8〜11 をすべて採る．指示書 `plan-brief.md` §3（B1〜B6）と計画 `test-plan-1b.md` はこの裁定で次のとおり改める（ここが優先する）．

## 0. 分け方（orchestrator）

- **1b-4d-1b（この段階）**：B1・B2・B3・B5 と，loss・stop の出口．計画の S9，S13，S13b，S22，S23，S24，S26，S27，S28 のうち lock の順序の検査（`ur_25_a_configuration_taken_under_the_line_lock_is_refused`）．
- **1b-4d-1c（次の段階）**：B4（D1 の A）．Sol の major 1〜3 がここに集まる：owner が始めた後の系列，configure の完了と記録の公開の間の窓，device の queue の容量と過ぎた origin．計画の S11d の 2 本と S28 の重ね合わせ（`ur_25_a_release_overlapping_a_configure_…`）もこちら．この段階では `release` の記録だけの枝を変えない．
- 理由：B4 は B1〜B3 から独立している（計画 §8）．しかも B4 には，queue の会計という未決の設計がまだ残っている．小さく分けて，各段階を green で閉じる．

## 1. この段階の改め

1. **loss と stop の出口（Sol 5）**：from-none は，次のそれぞれで device の呼び出しをやめ，`busy` を下ろす．
   - `reading` を待っている間の loss．
   - open が返った後の loss（S26）．
   - rate の照合が返った後の loss．
   - configure の channel の apply の間の loss．`core.configure` は，各 channel の apply の前に `is_lost` を見る（owner の configure も同じ関数を通る）．
   - `reading` を待っている間の Provider の stop：uhd-control がもう見ている stop の印で待ちを終える．device を呼ばずに `busy` を下ろす．記帳は既存の stop の経路に従う．
   - `Provider::stop` の join が 1 s で uhd-control を切り離したとき：切り離された uhd-control が後で `busy` を下ろさなくても，uhd-tx の shutdown の読みが止まらないこと（shutdown の読みは `busy` を見ない，またはその回は読まずに終える）．期待をどちらかに決めて，テストを 1 本書く．
   - S27 の (e) は「`busy` を立てた**後**の loss」を主な検査にする（先に lost にしてから記帳する形はやめる）．
2. **S23 の観測（Sol 4）**：実時間 50 ms の観測をやめる．
   - `#[cfg(test)]` の hook で「`reading` を見て待つ枝に入った」ことを test と同期し，その時点の `tx_open` が 0 回であることを見る．
   - 待ちを消した実装は，hook に届かずに `tx_open` するので落ちる．
   - 実時間の timeout は，終わったかどうかの番人にだけ使う．
   - 「普通の cold 変更へ落ちなかった」ことは `ready` の時刻で見ない（rig の `Core::now()` は別の `ManualTimeAuthority`）．uhd-control が open・configure をし，記録に `(seq, 版)` が書かれたことで見る．
3. **S24 は brief の逆向きの競合にする（Sol 6）**：owner が `idle` を false にした後，その `rx_open`（TX は `tx_open`）を止める．その間に uhd-control が同じ stream の enable を受け取る．`idle` を強制的に true にする helper（`enable_from_none` など）は使わない．
   - 期待：uhd-control は open も configure もせず，普通の cold 変更として記帳する．
   - 計画の今の S24（呼び出しの中で `busy` が立っている）は残してよい．ただし，probe を booking の section まで延ばすか，「configure の後，booking の前に `busy` を下ろす」誤りを落とす別の観測を足す．
4. **LockSpy の検査（Sol 7）**：並行のテストでは，`try_lock` は別の thread が正しく `streams` を持っている瞬間にも失敗する．だから LockSpy の「device の呼び出しの間に line の lock を持たない」の検査を，B5 の thread-local の数え（呼んだ thread が `core::lock` の guard を持っていない）に置き換える．
   - 対照のテストを 1 本足す：別の thread が `streams` を持っていても，device を呼ぶ thread が持っていなければ通る．
   - 既存の `ur_25_the_line_lock_is_free_inside_every_*_device_call` は，新しい検査のまま通ること．
5. **B5 の範囲（Sol 8）**：この検査が見るのは「`core::lock` の guard を持ったまま `updates` を取る，または device を呼ぶ」ことだけ．`rec → streams` の逆は見ない．規則と計画にもそう書く．「lock の順序全体を検査する」とは書かない．
6. **assert と mutation（Sol 9）**：
   - S9 の assert に `idle` が変わらないことを入れる．
   - S22 は，run loop の 1 回分を test から呼べる関数（`run` の本体を `pass()` のように切り出す）で動かす．`step` の device 呼び出しを誤ってする変更も落とす．
   - `busy` を下ろす mutation の行は，成功の経路だけでなく，足した早い戻りのそれぞれに 1 行ずつ付ける．
7. **design/18（Sol 10，この段階の分）**：
   - UR-3（`design/18-uhd-radio.md:116` の「each streamer has one owning thread」）を直す．uhd-control が from-none の間だけ担当する．
   - 担当，`reading` の待ち，lock の順序（と B5 の検査の範囲）は，UR-25 に短い別の段落として書く．
   - UR-17・UR-23 は UR-25 を引くだけにし，同じ規則を 2 度書かない．
   - 理由（なぜ待つか，UHD の reconnect の例外）は Changes の行と decision record（この裁定と報告）へ．
   - UR-24・UR-36 は 1c で直す（この段階では触らない）．
8. **大きさ（Sol 11）**：この段階の分だけ数え直す．

## 2. 計画の §9 の扱い

- §9.1：正しい（Sol も確認した）．1c で扱う．
- §9.5：誤り．from-none と `book_timed` は同じ uhd-control の thread で順に動く（`control.rs:319-324,550,849-881`）．計画から消す．Sol の major 2 の窓（owner の thread の configure と release の競合）は別のもので，1c で扱う．
- §9.8：断定しない（stream command と retune の queue の関係は UR-24 でも INFERRED）．1c の計画で扱う．
- §9.9：plan 33 の行は orchestrator が段階を閉じるときに直す．

## 3. 1c へ送るもの（記録だけ．この段階では実装しない）

- Sol major 1：S11d は，owner が segment を始めた**後**に timed 更新を記帳・release する．実際の `segment.origin` を読む（RX の start lead で 50 ms 以上）．fake の時計がその origin を越えた後に設定を読む．
- Sol major 2：configure の snapshot から成功の記録の公開まで．orchestrator の推奨：記録を，configure した設定の `updates` を持ったまま `streams` を取って書く（`updates` → `streams` の順で許される）．release は `updates` を持ったまま記録を読んで決める．
- Sol major 3：B4 が出す command の queue の会計．orchestrator の推奨：`book_timed` が，`e` に channel が無くても pending の segment がその channel を持つなら，その segment の channel の数を cost として数える（UR-24 で実際に出る場所で数える．#62 の Mock の queue のモデルは Phase 8 段階 2）．過ぎた origin は，今の UR-24 の「instant が過ぎた release」と同じに扱う．UR-24 の文は「origin，または最後に出した command の後（UC-2）」にする．
- Sol minor 10 の UR-24・UR-36 の文の点検（layout の「後の segment が当てる」の全称を含む）．
