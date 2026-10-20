判定：PASS

本体の変更に major は見つかりませんでした。テストの helper 化に非阻害の minor が1件あります。

### 指摘

1. **minor：helper 化で拒否件数の検査が弱くなっています**
   - **箇所**：`crates/ezsdr-radio-uhd/tests/fake.rs:1207`、`:6230–6234`
   - **証拠**：変更前は拒否イベント自体が2件であることを検査していました。変更後は `rejection_reasons` が `filter_map` で文字列でない `reason` を除外し、抽出後の件数を検査します。
   - **系列**：正常な拒否2件に、`reason` のない余分な拒否1件が混ざると、以前は失敗し、変更後は通り得ます。これはテストの検出力の差であり、現行本体がそのイベントを生成する不具合は確認していません。
   - **最小修正**：既存の検査に `assert_eq!(events.len(), 2)` を追加し、元の意味を保ってください。

### 確認結果

- `Streams::record` は実際の segment から stream・sequence・論理 channel 列を記録します。channel 列は streamer を開く際と同じ昇順です。
- 拒否後の sequence は `control.rs:1792–1812` で `[None, Some(2)]` を固定しています。membership の離脱後の `[1]` も実経路で検査されています。
- 拒否隔離の2テストは、予約済み item のある相手 line を比較し、RX の実際の設定拒否を通しています。
- **TX の `end: null` は今回閉じるべき穴ではありません。** 決定4の「今の規則」、RM-16、`timeline.rs:849–855` と整合しています。Provider の停止は burst を終え、進行中の TX clock は切りません。
- 2×2 の停止テストは Provider の実 thread・`stop`・`cleanup` を通ります。ただし Session／Manifest ファイルへの保存まで通すテストではありません。
- Python・acceptance・server に、変更した plan の `channels` を数として読む箇所は見つかりませんでした。Mock parity のコードは変更されていません。
- P480–P484 は対応する assertion を壊す内容です。変更対象の Rust ファイルを参照する既存行を含む **269 mutation 行すべてで `old` が厳密に1回一致**しました。
- `design/18` の今回の規範変更は実装と整合し、理由・履歴は Changes に分離されています。`git diff --check HEAD` も問題ありません。

未実施：ビルド・テスト・mutation の再実行、実機確認。mutation の実行結果は独立には再検証していません。ファイル変更はしていません。
