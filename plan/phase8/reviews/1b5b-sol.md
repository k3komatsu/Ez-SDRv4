# 1b-5b：Sol 6.1 のコードレビュー

## 1. コードレビュー

判定：FAIL

本体の layout 専用化に明確な挙動不良は見つかりませんでした。ただし、移行で失われたテスト範囲と mutation の削除理由に修正が必要です。

1. **major — VH-8 から channel 集合の遷移が消えている**  
   **箇所:** `crates/ezsdr-mock-radio/tests/timeline.rs:50–55,399–409`
   - **根拠:** `project_to_layout()` が `Change::Channels` をすべて削除し、実行側も各方向1 channel に縮小しています。したがって、空集合への退出、再参加、複数 channel の増減は生成比較から完全に消えます。手書きの `mock_layout.rs` は、この比較の代替にはなりません。
   - **系列:** channel が退出した後に rate 変更・fault・Stop が届き、再参加する場合の clock、cut、fault の適用可否を、timeline と Mock の間で比較できなくなっています。
   - **修正:** count 操作を捨てず、layout 用の membership 操作へ置き換えてください。少なくとも `c<n>.stream = -1`／再参加を生成し、2 channel・2 stream で join／leave／RM-34 move も扱う形にします。期待側は channel 集合と各 stream の plan を追い、実際に各遷移が生成されたことも assert してください。spec 09 の「手書きケースで代替」の記述も合わせて直します。

2. **minor — disable の回帰テストが、逆向きの reentry テストに変わっている**  
   **箇所:** `crates/ezsdr-mock-radio/tests/mock_radio.rs:2220–2241`
   - **根拠:** 旧テストは「先に予約した cold 変更の後へ disable が繰り下がり、新しい TX clock が自身の origin で終了する」を assert していました。変更後は退出→再参加となり、TX clock の期待値も `Some(origin)` から `None` に変わっています。名前だけの移行ではありません。
   - **系列:** 先に届いた変更が segment を作り、後から届いた早い時刻の退出が同じ effective instant へ繰り下がる場合、新 segment が残ってしまう誤りを、このテストは検出しません。
   - **修正:** 現在の reentry ケースとは別に、最初を `s0.sample_rate_hz` の cold 更新、次を `c0.stream = -1` とし、旧テストの終了条件を残してください。

3. **minor — P162 の対象コードは消えていない**  
   **箇所:** `plan/maintenance/tools/mutations.json:5430–5432`、`crates/ezsdr-mock-radio/src/lib.rs:2256–2259`
   - **根拠:** 削除した P162 の `old` は、現在の source にも正確に1回一致します。消えたのは mutant の `is_layout()` 条件であって、SimulationChannel を受け付けるコードではありません。
   - **系列:** `sim.channel` を理由に Mock を一律拒否する誤りに対する、記録済み mutation が失われます。
   - **修正:** P162 を復元し、常に layout となった現在の意味に合わせ、`Some(spec)` を MR-41 の拒否へ置き換える mutant に当て直してください。

**確認結果**
- MockRadio 143 tests、acceptance 83 tests：すべて成功。未変更の parity 20 tests を含みます。
- 全6 mutation リスト、計1,272行の `old`：すべて source に正確に1回一致。
- P10・P12・P123 の対象コード消滅を確認。P162 のみ上記の例外。
- 指定された radio／UHD／server／Python の禁止範囲に差分なし。`git diff --check` 成功。

未確認：mutation は再実行していません。指定行の kill は既存ログ上で確認しましたが、現在の tree での再現性は INFERRED です。実機・Python・workspace 全体の再実行は対象外とし、ファイルは変更していません。

## 2. 修正の回 1 の再レビュー

判定：PASS

新規指摘なし（major 0 / minor 0）。前回の3件は修正されています。

1. **VH-8 の集合遷移と比較範囲**
   - `crates/ezsdr-mock-radio/tests/timeline.rs:116–156,277–313`：leave **2,634**、return **1,495**、move **2,148**、empty-stream join **2,161**。各遷移が非ゼロであることを assert しています。
   - `timeline.rs:377–459`：期待側は入力系列と `Lines` から計算され、Mock の出力には依存しません。`timeline.rs:675–704` の観測側とは分離されています。
   - RX/TX とも stream 0・1 の clock を比較し、RX は両 stream の block・cut・fault の影響も比較しています。比較から抜ける stream はありません。
   - 退出中の rate 変更・fault・Stop を経て再参加する系列も確認しました（例：seed 0／28／106）。

2. **disable の旧終了条件を復元**
   - `crates/ezsdr-mock-radio/tests/mock_radio.rs:2246–2272`：rate 変更後へ disable が繰り下がり、新しい TX clock が自身の origin で終了することを固定しています。
   - reentry テストも同ファイル `2220–2242` に残っています。`renames.txt` に旧名の改名はありません。

3. **P162 と mutation 行の整合**
   - `plan/maintenance/tools/mutations.json:5432–5439`：`sim.channel` を一律拒否する mutant に復元されています。対象テストは prepare 成功と受信100サンプルを要求するため、この拒否を検出します。
   - remote 実行ログで P162 の **killed** を確認しました。
   - 全6リスト、**1,273行**の `old` は、現在の source にそれぞれ正確に1回一致しました。

**再実行結果：** `mock_layout` 49、`mock_radio` 61、`timeline` 1、計111テスト成功。differential は追加の再実行も成功し、差異ゼロ。`git diff --check` 成功。

未確認：mutation 自体は再実行していないため、現在の tree での kill 再現性は **INFERRED**。実機・workspace 全体の gate は対象外です。ファイルは変更していません。
