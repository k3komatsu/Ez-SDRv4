# 1b-4d-2：Sol の計画レビューの裁定（orchestrator）

Sol の報告：`sol-plan-report.md`（判定：要修正，major 7・追加 major 2・minor 5）．計画：`test-plan.md`．

## 1. 計画の §0 の裁定

- LATE_COMMAND の訂正（RM-11：device node）：採る．指示書 §3.4 の誤り．
- §1.2 の訂正 1〜10：採る（S12 は End の床，S3 の timed start は owner，S16 TX は unit）．
- R19〜R22：採る．
- P280・P308・P309 の削除，P313 の当て直し，P344・P345 の維持：採る（Sol も支持）．

## 2. Sol の指摘

1. **major 1（stop の再確認では保証にならない）：採る．指示書 §3.7 の再確認を取り下げる．**
   - 再確認は窓を狭めるが閉じない．しかも，残る状態（cut された segment に timed start が queue されている）は，**origin より前に begin した segment に `Provider::stop` が来れば，窓がなくても起きる**（owner は未来の segment をすぐ始める）．この状態の扱いは spec 22 VH-6（#56）が決めている：uhd-rx は origin の後で untimed に止め，届いたものを捨てる．再確認はこの保証を変えずに仕組みを 1 つ足すだけなので，足さない．
   - Sol の言う「origin が遠いと owner が join の 1 s を越え，detach され，cleanup が streamer を閉じない」は HEAD からある挙動で，この段階の窓に限らない．**この段階では直さず issue にする**（orchestrator が gate で起票：`Provider::stop` が遠い origin の queue された timed start に出会うと owner が detach される．#56 の bench の系列に足す）．
   - **S16 は今の挙動を固定するテストにする**（RX・TX とも）：owner が `Planned` を取る → configure から戻る所で gate → `Provider::stop` の記帳（stop の mark と End）→ gate を離す．RX の期待：owner は timed start を出し（今の挙動），その segment は End で origin に cut されているので，block は 1 つも配られず，clock は origin で終わり，origin の後で untimed の `rx_stop` が出る（VH-6）．origin は近く（stop から数十 ms）にして，join の 1 s の中で終わる形にする．TX の期待：owner は clock を採ってよいが，その segment は origin で終わり（`timeline.rs:892-895`），`tx_send` は出ない．
   - design/18 に「owner が stop を見直す」文は書かない．
2. **major 2（S16 TX が configure 後を検査しない）**：1 の書き直しで置き換える．TX も実際の `owner_configure` を通し，その戻りで gate する．
3. **major 3（`LockSpy` が `tests/fake.rs` から使えない）**：採る．integration test の中に，同じ `Arc<FakeDevice>` へ転送する小さな wrapper を置く．対象は RX・channel 1・untimed apply，device から戻った位置で止める．unit の補助を公開しない．
4. **major 4（S12 が `delivered` の書き先の混線を見逃す）**：採る．2 本の RX owner に違う長さの Samples を実際の経路で処理させ，Abort・Orderly の End がそれぞれ自分の stream の進捗で cut を決めること，一方の終わりで他方の `delivered` が消えないことを見る．手で値を置く unit は残してよい．
5. **major 5（TX の隔離と stream ごとの所有状態）**：採る．Sol の表の 4 系列（s1 の rate の cold 変更で s0 の burst と clock が続く；`Stop(tx/s1)` で s1 の burst だけ終わり，clock は残る；s1 の reopen の generation で s1 の古い相関だけ捨てる；s1 の `reading` 中の from-none で s0 の owner は device を呼び続ける）．
6. **major 6（S11 の操作と assert の矛盾）**：採る．s1 だけ送る区間と s0 を送る区間を分ける．
7. **major 7（design/18 の「halts the direction」など）**：採る．UR-17・UR-23・UR-25 の「halts the direction」を stream に，UR-23 の burst を止める宛先に `<id>/tx/s<k>`．UR-16 の cleanup の文：「1 本でも detach すれば `close_streams`（device 全体）を呼ばない」と書く．move の拒否は条件と結果だけを規範にし，段階と解除の予定は Changes へ．「configure は値だけを変える」の説明は規範に書かない．
8. **minor 8（S13 が control-first を検査しない）**：採る．control を先に join することは unit で固定する．gate は失敗時にも離し，worker の終わりまで待つ．
9. **minor 9（mutation の対応表）**：採る．`register(dir,k)` は TX の cold 変更の系列に当てる．S1 は s0・s1 に違う rate を与える．S10 は released と held が混ざる系列にする．join 順の mutant は RX-first．
10. **minor 10（H2・H3）**：採る．H2 は VERIFIED（UHD 4.10 `stream.hpp:242-247,391-396`）．H3 は「command time は motherboard ごとに設定・解除，queue は RFNoC block ごと（`timed_cmd.dox:94-115`）．UR-24 は instance で 1 列に数える近似（plan 33 D9）」と書き直す．
11. **追加 major（configure 中の loss）**：採る．owner は configure から戻った後，timed start（RX）と clock の採用（TX）の前に `core.is_lost()` を見る（`Core::configure` の channel ごとの検査と同じ形．1b-4d-1c の release も同じ）．loss の後に device を呼ばない．テスト：最後の apply の戻りで別 thread が loss を公開し，`rx_start` が出ないこと，TX は clock を採らないこと．
12. **追加 major（start の途中の失敗）**：採る．s0 の `rx_start` の後に s1 の `rx_start` が失敗する系列と，途中の spawn の失敗の系列．期待：`start` は `Rejected`，cleanup は立ち上がった owner を join してから `close_streams` を呼ぶ（device の呼び出しが残っている間に閉じない）．コードが要るなら最小に（今の `cleanup` の `stop(Abort)` の経路で足りるかをまず確かめる）．
13. **minor（off・空の on・halt の区別）**：採る．s1 を `Stop` した後に membership を変えても再開せず，`start_rx` で再開する；halt は成功した cold 変更で解ける（plan 33 D3 の 8）．

## 3. 大きさ

分けない．テストの繰り返しは RX・TX 共通の helper にする．機能とテストを別の commit に分けない．追加の系列を入れて見積もり直し，報告に実数を書く．
