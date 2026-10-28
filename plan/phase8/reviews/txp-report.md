STATUS: done

段階 TXP（worktree /home/komatsu/local_work/Ez-SDRv4-txp，未 commit）。詳細は plan/phase8/txprio-test-plan.md 末尾の「実装で足したこと」とセルフレビューへの対応。

- 実装：Device::raise_thread_priority（FakeDevice は Ok のみ，UhdDevice は uhd_set_thread_priority(0.5, true) で拒否は lost:false），Tx::run の最初の文で 1 回（拒否は rejected の 1 行，成功は stats.tx_realtime），send が窓に全量が入るまで待つ（最初の timed buffer だけ除外，待ちは 1 箇所，入口 2 つとも return self.send）。測定の包み役は転送（GuardedDevice は note を通さず，latch 済みなら拒否）。
- 試験：lib の M1〜M14（M4c，M14e 含む）・P1〜P6・M6・M8b，tests/fake.rs の P1b・G1〜G3，uhd.rs の F1・F2。既存の 7 試験は規則の更新（回帰なし）。
- 確認：cargo test --workspace ok，clippy -D warnings ok，docker feature の clippy と lib 456 ok，rules.py 0，git diff --check ok，時刻に依る試験を 20 回（失敗 0）。mutation：新しい P1100〜P1123 全部 killed，既存の tx.rs 行 85 本ほか killed（U19 は等価になり行を削除，P637/U17/R06/B31 を新しい本文に更新）。X8〜X10 は延期。
- 実機は流していない。bench.md (f) に B0〜B4 の手順（att の確認つき）。

---

STATUS: done

1. M8b: 上限を「3 つ目の payload send の host 時刻からの経過 < 12.5 ms」に変えた（絶対時刻をやめた。owner 側 snapshot はそのまま）。
2. bench.md B0: bursts[].blocks は数で列ではないと明記。send ごとの大きさは ProbeDevice の rows()（call == "tx_send"，sample_counts[0]，0 の行 = 空の EOB は除く）の有限の診断 run（5 s 程度）で読む。blocks/s は Manifest の総数。production は変えていない。
3. M14e: 新しい clock で 2 channel の burst を step し，最初の send が (2000, sob) で at が新 clock の instant(5000) と assert。
確認: M8b・M6・P を 20 回 (失敗 0)，workspace test ok，clippy ok，rules.py 0，git diff --check ok，P1111/P1112/P1118/P1108 killed。

---

STATUS: done

tests/hardware.rs に #[ignore] の hw_txp_b0_send_sizes を足した（usrp()，tx_session，ProbeDevice，gain 0 dB，31 000 sample の repeat（振幅 ≤ 0.5）を 1 と 5 MS/s で約 5 s，block 2 000 を assert，空の EOB を除いた sample_counts[0] の列を "B0 sizes rate <hz>:" と印字）。bench.md (f) B0 に docker／cargo の起動（(c) と同じ形，att の確認つき）を書いた。production は変えていない。
確認: --features uhd の docker clippy ok，clippy ok，cargo test --workspace ok，rules.py 0，git diff --check ok。実機は流していない。
