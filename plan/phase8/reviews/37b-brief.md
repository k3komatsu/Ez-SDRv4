# 設計メモ 37 の実装 37b：実装の指示書（#69）

あなたは実装役です．`impl-brief.md`（§1 背景，§4 守ること），`impl2-brief.md`（§7 守ること，§4 確かめ方，§5 セルフレビュー），`table.md` の B-2・B-3・B-7 を先に読むこと．HEAD は `adbd3a0`（37a は commit 済み），tree はきれい．作業 dir は `~/.cache/ezsdr-fixes/2026-10-23-note37/`．37a で何をしたかは `plan/phase8/reviews/37a-opus.md` と，tests/README.md に足した規則（判定は試験のスレッドがいつ動くかに依らない，(1)〜(4)）．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`，`.*.orig`，`sol-*.log` など）は読むだけ．書き換えない．見張りのプロセスは止めない．`git stash` を使わない．**

## 1. 範囲（37b）

1. **B-2：raw device の 13 本**（`crates/ezsdr-radio-uhd/tests/fake.rs`，表の B-2 の一覧）を `crates/ezsdr-radio-uhd/src/device.rs` の tests に移す．使うのは既存の部品だけ：`manual(config)`，`participant(&device, f)`，`ms`，`set_time`，`transmitted`，`unended_bursts`（表の B-2 の行番号）．テストの名前は変えない．`reports_until` helper は一緒に移すか `participant` の中で読む．`ur_33_a_real_fake_s_time_starts_at_its_own_creation` は移さない（表のとおり）．37a の README の段落（`src/device.rs` の raw receive の case が既に unit test だと書く所）を，TX の raw も含むように直す．
2. **B-3：`Direct` 系**（`Direct::new`／`with_links` は実時間の時計）：表の 6 系列のうち，`:3238`（無害）以外の 5 本．時刻の窓が判定そのものなら (4)：`src/provider/control.rs` の制御された時計の unit test に移す（前例は `ur_24_the_device_queue_order_is_the_effective_order` と 37a の `ur_21_a_burst_before_start_up_is_refused`）．窓に依らない性質が残るなら fake の側はその形に直し，残らないなら消して代わりを名指しする．`Direct` の時計（実時間）は変えない．
3. **mutation**：表の B-7 のうち 37b の行（raw の移動で `fake` → `lib`：P204，P214，P215，P216，P205，P217，P207，P213，P230，P231，P232，B17，B30．Direct の移動で `lib`：U20，U21，U37，B01，B02，P264）．移したテストを `filter` に持つ行を全リストで grep し直す（表の一覧で全部とは限らない）．`old` が tree で 1 回だけ一致すること．
4. **spec 18**：`design/18-uhd-radio.md` のテストの参照で，場所や形の変化で偽になる所を直す（DA-1 の形式，Changes の 1 行）．

## 2. 確かめること・セルフレビュー・報告

- impl2-brief §4 のとおり（移した・直したテストを各 20 回，負荷なし．`remote_mutate.py` で当て直した行の killed．clippy，rules，`git diff --check`）．
- 移した各テストが目的の状態に着いたことを assert で見ている．照合が空でも通る形にしない．
- セルフレビュー：impl2-brief §5 のコマンドで，checklist は `checklist-37b.md`．P1 以上が無くなるか 3 回で止める．
- `report.md` を書き直す（最初の行 `STATUS: done` か `STATUS: blocked`）．中身は impl2-brief §6 と同じ項目．
- 守ること：impl2-brief §7 と同じ（本番のコードを変えない，新しい仕組み・rig の種類・時計の hold・fake の新しい fault を足さない，到達できる系列のテストを代わりなしで消さない，`cargo fmt` 禁止，ローカルで `mutate.py` 禁止，負荷をかけない，commit・`git add`・`git stash` 禁止）．**起きたことを書く．していないことは書かない．**

---

# 37b の修正の指示書（Sol のコードレビュー）

あなたは実装役です．`impl3-brief.md`，`impl2-brief.md`（§4・§5・§7），あなたの `report.md` と，Sol のコードレビュー `sol-37b-verdict.md` を読むこと．tree は 37b の未 commit の差分のまま．守ることは impl2-brief §7 と同じ（本番のコードを変えない，新しい仕組みを足さない，負荷をかけない，`cargo fmt` 禁止，ローカルで `mutate.py` 禁止，commit・`git add`・`git stash` 禁止，orchestrator のファイルは読むだけ）．

orchestrator は Sol の 2 件をどちらも採った．

1. **major 1（`src/device.rs` の `ur_33_the_fake_device_keeps_the_devices_queues`，`:2303-2314` 付近）**：合図は `apply` の前なので，200 ms の「終わらない」は 17 件目が queue で待ったことの証拠にならない（worker が合図の後で止まっただけでも通る）．200 ms の実時間で機能を判定しない．参加者の中で 17 件目の呼び出しの前と後の装置の時刻を読み，呼び出しの前は 0（最初の command が 20 ms まで残っている），戻った時刻が 20 ms 以後であることを assert する．実時間の上限はハングの見張りだけに残してよい．
2. **major 2（`src/provider/rx.rs` の `ur_25_a_stream_silent_before_e1_switches_before_e2`，`:1472-1494`）**：timeout の assert が「100 ms 未満」だけで，一律 99 ms を返す誤りも通る．旧テストの「1 ms 未満を渡さない」も落ちている．コードから期待値を導き（Sol の見立ては Core 10 ms で 5 ms，15 ms で下限の 1 ms．`recv_timeout` の式を読んで確かめる），`recv_timeout()` と `turn()` が装置に渡した値（`min_recv_timeout` など既存の観測）をその値で `assert_eq!` する．下限（1 ms 未満を渡さない）も assert する．待ちが cut＋quiet span に収まることが判定になるようにする．

どちらも，そのテストを `filter` に持つ mutation の行（全リストを grep．少なくとも B01，B02 と，queue のテストの行）を `remote_mutate.py`（impl2-brief §4 のコマンド，chmod 644 を先に）で流して killed を確かめる．`design/18-uhd-radio.md` の該当の説明が assert と合っているか見る．

確かめること：直した 2 本を各 20 回（負荷なし），`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD`，`git diff --check`．セルフレビューは 1 回（impl2-brief §5 のコマンド，checklist は `checklist-37b.md`，log は `review-fix-1.log`）．P1 が出たら直してもう 1 回まで（`review-fix-2.log`）．

`report.md` の最後に「## 修正の回（Sol）」を足す（何を変えたか，mutation の結果，20 回の結果，セルフレビューの指摘と扱い）．最初の行は `STATUS: done` のまま（止まったら `STATUS: blocked`）．**起きたことを書く．していないことは書かない．**
